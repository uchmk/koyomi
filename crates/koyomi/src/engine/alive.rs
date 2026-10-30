//! When the engine was last alive. It writes `alive.toml` in the state folder every
//! half minute and once more, marked clean, when it is stopped. The next start reads
//! it: a file that is not clean means the engine died (or the machine went off), and a
//! long gap after a clean stop means nobody started it. Either is told once.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime};
use ito_common::kept;
use serde::{Deserialize, Serialize};

const FORMAT: &str = "%Y-%m-%d %H:%M:%S";
/// How often the heartbeat is written.
const BEAT: Duration = Duration::from_secs(30);
/// A crash shorter than this is a restart, not news.
const CRASH_MIN: i64 = 60;
/// A stop shorter than this is a restart too.
const STOP_MIN: i64 = 10 * 60;

/// `alive.toml`.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Kept {
    /// The last time the engine wrote, local time.
    #[serde(default)]
    at: String,
    /// The engine was stopped on purpose (`koyomi engine stop`).
    #[serde(default)]
    clean: bool,
}

pub struct Alive {
    writer: kept::Writer,
    last_beat: Instant,
    /// What the previous run left behind, told once: the line and whether it was a crash.
    pub outage: Option<(String, bool)>,
}

impl Alive {
    /// Reads the file the previous engine left and starts beating.
    pub fn new(state_dir: &Path) -> Alive {
        let path = state_dir.join("alive.toml");
        let prev: Kept = kept::read(&path).unwrap_or_default();
        let now = Local::now().naive_local();
        let mut a = Alive { writer: kept::Writer::new(path, true), last_beat: Instant::now(), outage: outage(&prev, now) };
        a.write(false);
        a
    }

    /// Writes the heartbeat when it is due.
    pub fn tick(&mut self) {
        if self.last_beat.elapsed() >= BEAT {
            self.last_beat = Instant::now();
            self.write(false);
        }
    }

    /// The last write, marked clean: the engine was stopped, not lost.
    pub fn stopped(&mut self) {
        let kept = Kept { at: Local::now().format(FORMAT).to_string(), clean: true };
        if let Ok(text) = toml::to_string(&kept) {
            let _ = self.writer.save_now(text);
        }
    }

    fn write(&mut self, clean: bool) {
        self.writer.save_toml(&Kept { at: Local::now().format(FORMAT).to_string(), clean });
    }
}

/// The line to tell about the time between the previous engine's last write and `now`;
/// `true` when that engine died instead of being stopped.
fn outage(prev: &Kept, now: NaiveDateTime) -> Option<(String, bool)> {
    let at = NaiveDateTime::parse_from_str(&prev.at, FORMAT).ok()?;
    let secs = (now - at).num_seconds();
    if secs < if prev.clean { STOP_MIN } else { CRASH_MIN } {
        return None;
    }
    let how = if prev.clean { "止まっていた" } else { "落ちていた" };
    let span = koyomi_core::duration::text(secs as u64);
    let line = format!(
        "エンジンは {} から {} ({span}) {how}。この間の時刻のジョブは流れていない",
        at.format("%-m/%-d %-H:%M"),
        now.format("%-m/%-d %-H:%M")
    );
    Some((line, !prev.clean))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, FORMAT).unwrap()
    }

    fn kept(at: &str, clean: bool) -> Kept {
        Kept { at: at.to_owned(), clean }
    }

    #[test]
    fn a_long_gap_is_told_and_a_restart_is_not() {
        let now = t("2026-10-30 09:00:00");
        assert_eq!(outage(&Kept::default(), now), None);
        assert_eq!(outage(&kept("2026-10-30 08:59:30", false), now), None);
        assert_eq!(outage(&kept("2026-10-30 08:55:00", true), now), None);
        let (line, crashed) = outage(&kept("2026-10-30 08:00:00", false), now).unwrap();
        assert!(crashed && line.contains("10/30 8:00 から 10/30 9:00") && line.contains("落ちていた"), "{line}");
        let (line, crashed) = outage(&kept("2026-10-29 18:00:00", true), now).unwrap();
        assert!(!crashed && line.contains("止まっていた"), "{line}");
    }
}
