//! Settings, stored as a tiny key=value file in %APPDATA%\HitscanTray\settings.ini.
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Notify when the battery drops to this percent. 0 turns low-battery alerts off.
    pub threshold: u8,
    /// Notify once the battery reaches 100% while charging.
    pub notify_full: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { threshold: 10, notify_full: true }
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("HitscanTray").join("settings.ini"))
}

impl Config {
    /// Returns the saved settings and whether a settings file already existed.
    pub fn load() -> (Config, bool) {
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return (Config::default(), false);
        };
        (Config::parse(&text), true)
    }

    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "threshold" => {
                    if let Ok(n) = v.parse::<u8>() {
                        c.threshold = n.min(100);
                    }
                }
                "notify_full" => c.notify_full = matches!(v, "1" | "true" | "yes"),
                _ => {}
            }
        }
        c
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(
            p,
            format!(
                "threshold={}\nnotify_full={}\n",
                self.threshold, self.notify_full as u8
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ignores_unknown_keys() {
        // interval_min was a setting in an early build; old files still load.
        let c = Config::parse("threshold=20\nnotify_full=0\ninterval_min=5\njunk\n");
        assert_eq!(c, Config { threshold: 20, notify_full: false });
    }
}
