//! Battery percentage as the official Hitscan Utility shows it.
//!
//! The firmware's own percent jumps around, especially while charging, when the
//! voltage rises. The official `HIDUsb.dll` ("BatteryOptimize") instead:
//!
//! 1. converts the reported voltage to a percent with a fixed 21-point curve,
//! 2. moves the displayed value toward that estimate by at most 1% per tick
//!    (every 10 s; every 5 min near full), only down while discharging and only up
//!    while charging (and then only while the estimate is more than 10 points
//!    ahead), and
//! 3. seeds the displayed value at startup from the last value it saved, so it
//!    doesn't jump when the app restarts.
//!
//! Everything here mirrors that code, including its odd corners.

use crate::hid::Battery;

/// Voltage (mV) at 0%, 5%, 10% … 100%, from the official DLL's defaults.
pub const CURVE_MV: [u16; 21] = [
    3050, 3420, 3480, 3540, 3600, 3660, 3720, 3760, 3800, 3840, 3880, 3920, 3940, 3960, 3980,
    4000, 4020, 4040, 4060, 4080, 4110,
];

/// Seconds between steps normally, and near full charge.
pub const TICK_SECS: u64 = 10;
pub const SLOW_TICK_SECS: u64 = 300;

/// A displayed value saved earlier, with the Unix time it was saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    pub percent: u8,
    pub unix: u64,
}

/// Percent from voltage, interpolated within each 5% band of the curve.
pub fn percent_from_mv(mv: u16, charging: bool) -> u8 {
    if mv >= CURVE_MV[20] {
        return if charging { 99 } else { 100 };
    }
    let Some(i) = CURVE_MV.iter().position(|&v| mv < v) else { return 0 };
    if i == 0 {
        return 1;
    }
    let lo = CURVE_MV[i - 1] as i32;
    let step = (CURVE_MV[i] as i32 - lo) / 5;
    let p = (5 * (i as i32 - 1) + (mv as i32 - lo) / step) as u8;
    // The DLL nudges exactly 0 and 15 up by one.
    if p == 0 || p == 15 {
        p + 1
    } else {
        p
    }
}

/// The estimate the displayed value is steered toward.
pub fn target(b: &Battery) -> u8 {
    if b.millivolts > 0 {
        percent_from_mv(b.millivolts, b.charging)
    } else {
        b.percent
    }
}

/// Initial displayed value for a new session.
pub fn seed(target: u8, saved: Option<Saved>, now: u64) -> u8 {
    let Some(s) = saved else { return target };
    if now < s.unix {
        return target;
    }
    let elapsed = now - s.unix;
    if elapsed > 1800 {
        return target;
    }
    if elapsed < 60 {
        return s.percent;
    }
    let rise = (elapsed as f64 * 0.028) as u32;
    let up = (s.percent as u32 + rise).min(100) as u8;
    let fall = (elapsed as f64 * 0.014) as u8;
    let down = s.percent.saturating_sub(fall);
    if target > up {
        up
    } else if target < down {
        down
    } else {
        s.percent
    }
}

/// One tick. Returns the seconds until the next one.
fn step(shown: &mut u8, target: u8, b: &Battery) -> u64 {
    let mut next = TICK_SECS;
    if b.charging {
        if target >= 95 && *shown >= 85 {
            next = SLOW_TICK_SECS;
            if *shown < 99 {
                *shown += 1;
            }
        } else if target as i32 - *shown as i32 > 10 {
            *shown += 1;
        }
        // The DLL jumps to 100 once the firmware has reported 100% for about a second.
        if b.percent >= 100 {
            *shown = 100;
        }
    } else if *shown > target {
        if b.percent == 0 {
            *shown = 0;
        } else {
            *shown -= 1;
            if b.percent <= 15 && *shown > 15 {
                *shown = 15;
            }
        }
    }
    *shown = (*shown).min(100);
    next
}

/// Tracks the displayed percent across polls. Polls can be far apart; the ticks
/// that would have happened in between are replayed against the latest reading.
#[derive(Clone, Debug, Default)]
pub struct Estimator {
    shown: Option<u8>,
    last_tick: u64,
    interval: u64,
}

impl Estimator {
    pub fn shown(&self) -> Option<u8> {
        self.shown
    }

    /// Feeds a reading taken at Unix time `now`; returns the percent to display.
    /// `saved` is only used for the first reading.
    pub fn update(&mut self, b: &Battery, now: u64, saved: Option<Saved>) -> u8 {
        let target = target(b);
        let mut shown = match self.shown {
            Some(s) => s,
            None => {
                // First reading: seed, then the DLL runs a tick straight away.
                let mut s = seed(target, saved, now);
                self.interval = step(&mut s, target, b);
                self.last_tick = now;
                self.shown = Some(s);
                return s;
            }
        };
        if now < self.last_tick {
            self.last_tick = now; // clock went backwards
        }
        let mut ticks = 0;
        while now - self.last_tick >= self.interval && ticks < 10_000 {
            self.last_tick += self.interval;
            self.interval = step(&mut shown, target, b);
            ticks += 1;
        }
        if ticks == 10_000 {
            self.last_tick = now;
        }
        // Applied as soon as the DLL sees the firmware report, not on a tick.
        if b.percent == 0 {
            shown = 0;
        } else if b.percent <= 15 && shown > 15 {
            shown = 15;
        }
        if b.charging && b.percent >= 100 {
            shown = 100;
        }
        self.shown = Some(shown);
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(percent: u8, charging: bool, millivolts: u16) -> Battery {
        Battery { percent, charging, millivolts, device_id: [0; 3] }
    }

    #[test]
    fn curve_matches_dll() {
        assert_eq!(percent_from_mv(3000, false), 1);
        assert_eq!(percent_from_mv(3729, false), 31);
        assert_eq!(percent_from_mv(3954, true), 63);
        assert_eq!(percent_from_mv(3999, true), 74);
        assert_eq!(percent_from_mv(4110, true), 99);
        assert_eq!(percent_from_mv(4110, false), 100);
        assert_eq!(percent_from_mv(3540, false), 16); // 15 is nudged to 16
    }

    #[test]
    fn seeding() {
        assert_eq!(seed(70, None, 1000), 70);
        assert_eq!(seed(70, Some(Saved { percent: 40, unix: 990 }), 1000), 40);
        assert_eq!(seed(70, Some(Saved { percent: 40, unix: 0 }), 5000), 70);
        // 500 s: may rise 14 points or fall 7.
        assert_eq!(seed(70, Some(Saved { percent: 40, unix: 500 }), 1000), 54);
        assert_eq!(seed(45, Some(Saved { percent: 40, unix: 500 }), 1000), 40);
        assert_eq!(seed(20, Some(Saved { percent: 40, unix: 500 }), 1000), 33);
    }

    #[test]
    fn charging_lags_ten_points_behind() {
        let mut e = Estimator::default();
        let b = bat(55, true, 3954); // target 63
        assert_eq!(e.update(&b, 1000, Some(Saved { percent: 40, unix: 995 })), 41);
        // Every 10 s it gains 1% until it's within 10 of the target.
        assert_eq!(e.update(&b, 1030, None), 44);
        assert_eq!(e.update(&b, 2000, None), 53);
    }

    #[test]
    fn discharging_only_goes_down() {
        let mut e = Estimator::default();
        assert_eq!(e.update(&bat(25, false, 3729), 0, Some(Saved { percent: 35, unix: 0 })), 34);
        assert_eq!(e.update(&bat(25, false, 3729), 100, None), 31);
        assert_eq!(e.update(&bat(25, false, 3800), 200, None), 31);
        // Firmware says 15% or less: clamp straight to 15.
        assert_eq!(e.update(&bat(14, false, 3700), 210, None), 15);
    }

    #[test]
    fn full_when_firmware_says_full() {
        let mut e = Estimator::default();
        e.update(&bat(97, true, 4100), 0, Some(Saved { percent: 90, unix: 0 }));
        assert_eq!(e.update(&bat(100, true, 4120), 10, None), 100);
    }
}
