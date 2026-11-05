//! Reading and writing the mouse's settings. See PROTOCOL.md for the full map.
//!
//! The mouse keeps its configuration in a 192-byte block, read 10 bytes at a time
//! with command `0x08`. Most values are stored as a pair `[value, 0x55 - value]`;
//! DPI values and colours as quads `[a, b, c, 0x55 - (a + b + c)]`.
//!
//! Offsets were confirmed by watching the block while toggling each setting in the
//! official utility. Items marked "inferred" are educated guesses that still need a
//! test on real hardware.

use crate::hid::Device;

pub const CMD_READ_FLASH: u8 = 0x08;
pub const CMD_WRITE_FLASH: u8 = 0x07;
pub const CMD_GET_LONG_RANGE: u8 = 0x17;
/// Confirmed on hardware via the web configurator; not used by the Rust code.
pub const CMD_SET_LONG_RANGE: u8 = 0x16;
pub const BLOCK_LEN: usize = 0xC0;
const CHUNK: usize = 10;

pub const OFF_POLLING: usize = 0x00;
pub const OFF_STAGE_COUNT: usize = 0x02;
pub const OFF_CURRENT_STAGE: usize = 0x04;
pub const OFF_LOD: usize = 0x0A;
pub const OFF_DPI: usize = 0x0C; // 8 quads
pub const OFF_COLORS: usize = 0x2C; // 8 quads
pub const OFF_BUTTONS: usize = 0x60; // 16 quads
pub const OFF_DEBOUNCE: usize = 0xA9;
pub const OFF_MOTION_SYNC: usize = 0xAB;
pub const OFF_ANGLE_SNAP: usize = 0xAF;
pub const OFF_RIPPLE: usize = 0xB1;
pub const OFF_SLEEP_ENABLED: usize = 0xB5;
pub const OFF_SLEEP_TIME: usize = 0xB7;
pub const OFF_HIGH_PERF: usize = 0xB9;

impl Device {
    /// Reads `len` bytes of the settings block starting at `addr`.
    pub fn read_flash(&self, addr: u16, len: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        let mut a = addr as usize;
        while out.len() < len {
            let n = CHUNK.min(len - out.len());
            let args = [0x00, (a >> 8) as u8, a as u8, n as u8];
            // Another app may be reading other addresses; keep only our reply.
            let reply = (0..3).find_map(|_| {
                self.transact(CMD_READ_FLASH, &args)
                    .filter(|r| r[3] == (a >> 8) as u8 && r[4] == a as u8)
            })?;
            out.extend_from_slice(&reply[6..6 + n]);
            a += n;
        }
        Some(out)
    }

    /// Writes `data` at `addr`, 10 bytes per request, as the official DLL does.
    /// The mouse applies changes immediately; there is no separate save step.
    /// Returns false if any chunk isn't acknowledged.
    pub fn write_flash(&self, addr: u16, data: &[u8]) -> bool {
        let mut a = addr as usize;
        for chunk in data.chunks(CHUNK) {
            let mut args = vec![0x00, (a >> 8) as u8, a as u8, chunk.len() as u8];
            args.extend_from_slice(chunk);
            let acked = (0..3).any(|_| {
                self.transact(CMD_WRITE_FLASH, &args)
                    .is_some_and(|r| r[3] == (a >> 8) as u8 && r[4] == a as u8)
            });
            if !acked {
                return false;
            }
            a += chunk.len();
        }
        true
    }

    /// Writes a pair-encoded value and confirms it by reading it back.
    pub fn set_pair(&self, off: usize, value: u8) -> bool {
        self.write_flash(off as u16, &encode_pair(value))
            && self.read_flash(off as u16, 2).is_some_and(|r| pair(&r, 0) == Some(value))
    }

    /// Long Distance Mode is a receiver setting, not part of the block.
    pub fn long_range_mode(&self) -> Option<u8> {
        self.transact(CMD_GET_LONG_RANGE, &[]).map(|r| r[6])
    }
}

/// A value checked against its stored check byte.
pub fn pair(raw: &[u8], off: usize) -> Option<u8> {
    let v = *raw.get(off)?;
    (*raw.get(off + 1)? == 0x55u8.wrapping_sub(v)).then_some(v)
}

pub fn encode_pair(v: u8) -> [u8; 2] {
    [v, 0x55u8.wrapping_sub(v)]
}

pub fn encode_quad(q: [u8; 3]) -> [u8; 4] {
    [q[0], q[1], q[2], 0x55u8.wrapping_sub(q[0].wrapping_add(q[1]).wrapping_add(q[2]))]
}

pub fn quad(raw: &[u8], off: usize) -> Option<[u8; 3]> {
    let q = raw.get(off..off + 4)?;
    let sum = q[0].wrapping_add(q[1]).wrapping_add(q[2]);
    (q[3] == 0x55u8.wrapping_sub(sum)).then_some([q[0], q[1], q[2]])
}

/// Hz to report-rate code (inverse of `polling_hz`).
pub fn polling_code(hz: u32) -> Option<u8> {
    [1u8, 2, 4, 8].into_iter().find(|&c| polling_hz(c) == Some(hz))
}

/// Report-rate code to Hz. All four are confirmed on hardware.
pub fn polling_hz(code: u8) -> Option<u32> {
    match code {
        1 => Some(1000),
        2 => Some(500),
        4 => Some(250),
        8 => Some(125),
        _ => None,
    }
}

/// DPI from a stored step. Only the low byte is decoded so far; the third byte of
/// each DPI quad presumably carries high bits for values above 12,800.
pub fn dpi_from_step(step: u8) -> u32 {
    (step as u32 + 1) * 50
}

pub fn describe_button(b: [u8; 3]) -> String {
    match b {
        [1, 0x01, 0] => "Left click".into(),
        [1, 0x02, 0] => "Right click".into(),
        [1, 0x04, 0] => "Middle click".into(),
        [1, 0x08, 0] => "Back".into(),
        [1, 0x10, 0] => "Forward".into(),
        [2, 0x01, 0] => "DPI loop".into(),
        [2, 0x02, 0] => "DPI up (inferred)".into(),
        [2, 0x03, 0] => "DPI down (inferred)".into(),
        [0, 0, 0] => "(empty)".into(),
        [t, a, c] => format!("unknown ({t:02x} {a:02x} {c:02x})"),
    }
}

/// Human-readable dump of a settings block, for the probe tool.
pub fn describe(raw: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    let show = |v: Option<u8>, f: &dyn Fn(u8) -> String| match v {
        Some(v) => f(v),
        None => "bad check byte".to_string(),
    };
    let on = |v: u8| if v != 0 { "on".to_string() } else { "off".to_string() };

    lines.push(format!(
        "Polling rate     : {}",
        show(pair(raw, OFF_POLLING), &|v| polling_hz(v)
            .map(|hz| format!("{hz} Hz"))
            .unwrap_or_else(|| format!("unknown code {v}")))
    ));
    let stages = pair(raw, OFF_STAGE_COUNT).unwrap_or(0).min(8);
    lines.push(format!("DPI stages       : {stages}"));
    lines.push(format!(
        "Current stage    : {}",
        show(pair(raw, OFF_CURRENT_STAGE), &|v| format!("{}", v + 1))
    ));
    for i in 0..8 {
        let dpi = quad(raw, OFF_DPI + i * 4);
        let col = quad(raw, OFF_COLORS + i * 4);
        let dpi = match dpi {
            Some([x, y, 0]) if x == y => format!("{}", dpi_from_step(x)),
            Some([x, y, 0]) => format!("X {} / Y {}", dpi_from_step(x), dpi_from_step(y)),
            Some([x, y, hi]) => format!("raw {x:02x} {y:02x} {hi:02x} (high bits not decoded)"),
            None => "bad check byte".into(),
        };
        let col = col.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")).unwrap_or("bad check byte".into());
        let mark = if (i as u8) < stages { " " } else { "(unused)" };
        lines.push(format!("  Stage {}{mark:9}: {dpi:>6} DPI, colour {col}", i + 1));
    }
    lines.push(format!(
        "LOD              : {}",
        show(pair(raw, OFF_LOD), &|v| format!("{v} mm"))
    ));
    lines.push(format!(
        "Debounce         : {}",
        show(pair(raw, OFF_DEBOUNCE), &|v| format!("{v} ms"))
    ));
    lines.push(format!("Motion Sync      : {}", show(pair(raw, OFF_MOTION_SYNC), &on)));
    lines.push(format!("Ripple Control   : {}", show(pair(raw, OFF_RIPPLE), &on)));
    lines.push(format!("Angle Snapping   : {}", show(pair(raw, OFF_ANGLE_SNAP), &on)));
    lines.push(format!("High Performance : {}", show(pair(raw, OFF_HIGH_PERF), &on)));
    lines.push(format!("Auto Sleep       : {}", show(pair(raw, OFF_SLEEP_ENABLED), &on)));
    lines.push(format!(
        "Auto Sleep time  : {}",
        show(pair(raw, OFF_SLEEP_TIME), &|v| {
            let s = v as u32 * 10;
            if s % 60 == 0 { format!("{} min", s / 60) } else { format!("{s} s") }
        })
    ));
    lines.push("Buttons (slot order in memory):".into());
    for i in 0..16 {
        match quad(raw, OFF_BUTTONS + i * 4) {
            Some([0, 0, 0]) => {}
            Some(b) => lines.push(format!("  Slot {:2}       : {}", i + 1, describe_button(b))),
            None => lines.push(format!("  Slot {:2}       : bad check byte", i + 1)),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block read from a Hyperlight on 2026-11-05 (3 stages at 1600 DPI, 1000 Hz,
    /// LOD 1 mm, debounce 0, all toggles off, sleep after 2 min).
    const SAMPLE: &str = "01 54 03 52 02 53 00 55 00 55 01 54 1f 1f 00 17 1f 1f 00 17 1f 1f 00 17 3f 3f 00 d7 3f 3f 00 d7 3f 3f 00 d7 3f 3f 00 d7 3f 3f 00 d7 00 fb 02 58 f2 22 66 db 0a 0a ff 42 82 04 ff d0 82 04 ff d0 82 04 ff d0 82 04 ff d0 82 04 ff d0 02 53 80 d5 03 52 00 55 ff 00 ff 57 00 55 80 d5 03 52 00 55 01 01 00 53 01 02 00 52 01 04 00 50 01 08 00 4c 01 10 00 44 02 01 00 52 02 02 00 51 02 03 00 50 07 00 00 4e 08 04 00 49 04 0a 03 44 00 00 00 55 00 00 00 55 00 00 00 55 00 00 00 55 00 00 00 55 01 ff 00 ff 07 09 46 00 55 00 55 00 55 06 4f 00 55 00 55 00 55 01 54 0c 49 00 55 0a 4b ff ff ff";

    fn sample() -> Vec<u8> {
        SAMPLE.split(' ').map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    #[test]
    fn encoders_round_trip() {
        let raw = sample();
        assert_eq!(encode_pair(1), [0x01, 0x54]);
        assert_eq!(&encode_quad([0x1f, 0x1f, 0x00])[..], &raw[OFF_DPI..OFF_DPI + 4]);
        assert_eq!(&encode_quad([0xf2, 0x22, 0x66])[..], &raw[OFF_COLORS + 4..OFF_COLORS + 8]);
        assert_eq!(polling_code(500), Some(2));
    }

    #[test]
    fn decodes_known_block() {
        let raw = sample();
        assert_eq!(raw.len(), BLOCK_LEN);
        assert_eq!(pair(&raw, OFF_POLLING).and_then(polling_hz), Some(1000));
        assert_eq!(pair(&raw, OFF_STAGE_COUNT), Some(3));
        assert_eq!(pair(&raw, OFF_CURRENT_STAGE), Some(2));
        assert_eq!(quad(&raw, OFF_DPI).map(|q| dpi_from_step(q[0])), Some(1600));
        assert_eq!(quad(&raw, OFF_COLORS + 4), Some([0xf2, 0x22, 0x66]));
        assert_eq!(pair(&raw, OFF_LOD), Some(1));
        assert_eq!(pair(&raw, OFF_DEBOUNCE), Some(0));
        assert_eq!(pair(&raw, OFF_SLEEP_ENABLED), Some(1));
        assert_eq!(pair(&raw, OFF_SLEEP_TIME), Some(12));
        for off in [OFF_MOTION_SYNC, OFF_ANGLE_SNAP, OFF_RIPPLE, OFF_HIGH_PERF] {
            assert_eq!(pair(&raw, off), Some(0));
        }
        assert_eq!(quad(&raw, OFF_BUTTONS + 5 * 4).map(describe_button).as_deref(), Some("DPI loop"));
    }
}
