//! Time-remaining estimate from how fast the battery has been dropping.
//!
//! The displayed percentage (see `estimate`) only ever steps down by 1% while
//! discharging, so each step is a clean timestamp. The rate is the drop since the
//! first step of this discharge (within the last day) divided by the time since
//! then. Measuring to "now" rather than to the last step means a mouse left
//! asleep or unused stretches the estimate out, which is what it'll really do.

/// Ignore steps older than this, so the rate follows recent use.
const WINDOW_SECS: u64 = 24 * 3600;
/// Need at least this many percent of drop...
const MIN_DROP: u8 = 2;
/// ...over at least this long before guessing.
const MIN_SPAN_SECS: u64 = 20 * 60;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Discharge {
    /// (unix time, percent) of each step down. The first entry may be the level
    /// we started watching at rather than a real step; `first_is_step` says which.
    points: Vec<(u64, u8)>,
    first_is_step: bool,
}

impl Discharge {
    /// Feeds the displayed percentage. Returns true if the history changed (so the
    /// caller can save it).
    pub fn observe(&mut self, now: u64, percent: u8, charging: bool) -> bool {
        if charging {
            let had = !self.points.is_empty();
            self.points.clear();
            return had;
        }
        match self.points.last().copied() {
            None => {
                self.points.push((now, percent));
                self.first_is_step = false;
            }
            Some((_, last)) if percent < last => {
                self.points.push((now, percent));
                self.prune(now);
            }
            // Higher than before: it was charged while we weren't looking. Start over.
            Some((_, last)) if percent > last => {
                self.points.clear();
                self.points.push((now, percent));
                self.first_is_step = false;
            }
            _ => return false,
        }
        true
    }

    fn prune(&mut self, now: u64) {
        let before = self.points.len();
        // Always keep the latest point so a long idle stretch still has an anchor.
        while self.points.len() > 1 && now.saturating_sub(self.points[0].0) > WINDOW_SECS {
            self.points.remove(0);
        }
        if self.points.len() != before {
            self.first_is_step = true;
        }
    }

    /// Hours until 0% at the recent rate, or None if there isn't enough history.
    pub fn hours_left(&self, now: u64, percent: u8) -> Option<f64> {
        // Anchor on a real step: the first point is only a starting level otherwise.
        let anchor = if self.first_is_step { self.points.first() } else { self.points.get(1) }?;
        let drop = anchor.1.checked_sub(percent)?;
        let span = now.checked_sub(anchor.0)?;
        if drop < MIN_DROP || span < MIN_SPAN_SECS {
            return None;
        }
        let per_hour = drop as f64 / (span as f64 / 3600.0);
        Some(percent as f64 / per_hour)
    }

    /// One line per point, for saving between runs.
    pub fn to_text(&self) -> String {
        let mut s = format!("first_is_step={}\n", self.first_is_step as u8);
        for (t, p) in &self.points {
            s.push_str(&format!("{t} {p}\n"));
        }
        s
    }

    pub fn from_text(text: &str) -> Discharge {
        let mut d = Discharge::default();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("first_is_step=") {
                d.first_is_step = v.trim() == "1";
            } else if let Some((t, p)) = line.split_once(' ') {
                if let (Ok(t), Ok(p)) = (t.trim().parse(), p.trim().parse()) {
                    d.points.push((t, p));
                }
            }
        }
        d
    }
}

/// "~12h", "~4h 30m", "~45m", "~3d 4h". Under ten hours it rounds to 10 minutes.
pub fn format_hours(h: f64) -> String {
    let mins = (h * 60.0).round().max(0.0) as u64;
    if mins >= 48 * 60 {
        let (d, hh) = (mins / (24 * 60), (mins % (24 * 60)) / 60);
        return if hh == 0 { format!("~{d}d") } else { format!("~{d}d {hh}h") };
    }
    if mins >= 10 * 60 {
        return format!("~{}h", (mins + 30) / 60);
    }
    let mins = ((mins + 5) / 10) * 10;
    match (mins / 60, mins % 60) {
        (0, 0) => "~10m".to_string(),
        (0, m) => format!("~{m}m"),
        (h, 0) => format!("~{h}h"),
        (h, m) => format!("~{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3600;

    #[test]
    fn needs_some_history() {
        let mut d = Discharge::default();
        d.observe(0, 80, false);
        assert_eq!(d.hours_left(10 * H, 80), None); // no steps yet
        d.observe(H, 79, false);
        d.observe(2 * H, 78, false);
        assert_eq!(d.hours_left(2 * H, 78), None); // only 1% since the first step
        d.observe(3 * H, 77, false);
        // 2% in 2 h since the first step at 79% => 1%/h => 77 h left.
        assert_eq!(d.hours_left(3 * H, 77).map(|h| h.round()), Some(77.0));
    }

    #[test]
    fn idle_time_stretches_the_estimate() {
        let mut d = Discharge::default();
        for (i, p) in [50u8, 49, 48, 47].into_iter().enumerate() {
            d.observe(i as u64 * H, p, false);
        }
        // From the first step (49% at 1 h): 2% over 2 h => 1%/h => 47 h left.
        assert_eq!(d.hours_left(3 * H, 47).map(|h| h.round()), Some(47.0));
        // Four idle hours later, still 47%: 2% over 6 h => 141 h left.
        assert_eq!(d.hours_left(7 * H, 47).map(|h| h.round()), Some(141.0));
    }

    #[test]
    fn charging_or_a_jump_up_starts_over() {
        let mut d = Discharge::default();
        d.observe(0, 60, false);
        d.observe(H, 59, false);
        assert!(d.observe(2 * H, 59, true));
        assert_eq!(d, Discharge::default());
        d.observe(3 * H, 90, false);
        d.observe(4 * H, 89, false);
        d.observe(5 * H, 95, false); // charged while the tray was closed
        assert_eq!(d.points, vec![(5 * H, 95)]);
    }

    #[test]
    fn round_trips_through_text() {
        let mut d = Discharge::default();
        d.observe(100, 70, false);
        d.observe(200, 69, false);
        assert_eq!(Discharge::from_text(&d.to_text()), d);
    }

    #[test]
    fn formats_like_the_tooltip() {
        assert_eq!(format_hours(12.2), "~12h");
        assert_eq!(format_hours(4.5), "~4h 30m");
        assert_eq!(format_hours(4.48), "~4h 30m");
        assert_eq!(format_hours(0.74), "~40m");
        assert_eq!(format_hours(2.0), "~2h");
        assert_eq!(format_hours(76.0), "~3d 4h");
        assert_eq!(format_hours(0.01), "~10m");
    }
}
