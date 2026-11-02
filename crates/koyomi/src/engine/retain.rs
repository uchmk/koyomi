//! The sweep of old logs and history: once a day, and again when
//! `keep_days`, `archive` or `archive_days` changes,
//! `koyomi_core::retain::sweep` runs on a thread and what it removed (and
//! put into `archive/`) is kept in `retain.toml` (the engine has no log
//! file of its own), where `koyomi engine status` shows it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use chrono::NaiveDate;
use ito_common::kept;
use koyomi_core::retain::{cutoff, sweep, Archive, Swept};
use serde::{Deserialize, Serialize};

/// `retain.toml`: the last sweep.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Kept {
    /// When it ran, local time.
    #[serde(default)]
    at: String,
    /// What `keep_days` was then.
    #[serde(default)]
    keep_days: u32,
    /// How many day folders of `logs/` and day files of `history/` went.
    #[serde(default)]
    logs: usize,
    #[serde(default)]
    history: usize,
    /// Whether `archive` was on, how many days went into `archive/` and
    /// how many month zips `archive_days` removed.
    #[serde(default)]
    archive: bool,
    #[serde(default)]
    archived: usize,
    #[serde(default)]
    pruned: usize,
    /// What could not be removed.
    #[serde(default)]
    errors: Vec<String>,
}

/// `keep_days`, `archive` and `archive_days` of koyomi.toml.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub keep_days: u32,
    pub archive: bool,
    pub archive_days: u32,
}

impl Settings {
    pub fn of(c: &koyomi_core::config::Config) -> Settings {
        Settings { keep_days: c.keep_days, archive: c.archive, archive_days: c.archive_days }
    }
}

pub struct Retain {
    state_dir: PathBuf,
    /// The day and the settings the last sweep was started for.
    swept_for: Option<(NaiveDate, Settings)>,
    /// The sweep on its thread, until it answers.
    rx: Option<Receiver<(Settings, Swept)>>,
    last: Kept,
    writer: kept::Writer,
}

impl Retain {
    /// Reads `retain.toml` from `state_dir`.
    pub fn new(state_dir: &Path) -> Retain {
        let path = state_dir.join("retain.toml");
        let last: Kept = kept::read(&path).unwrap_or_default();
        Retain { state_dir: state_dir.to_owned(), swept_for: None, rx: None, last, writer: kept::Writer::new(path, true) }
    }

    /// Starts a sweep when today or the settings are not what the last one
    /// was for, and takes in the answer of the one running. The line to
    /// tell on the console, when a sweep ended.
    pub fn tick(&mut self, set: Settings) -> Option<String> {
        let today = ito_common::now::now_local().date_naive();
        if self.rx.is_none() && self.swept_for != Some((today, set)) {
            self.swept_for = Some((today, set));
            let (tx, rx) = mpsc::channel();
            let dir = self.state_dir.clone();
            std::thread::spawn(move || {
                // The runner still writes today's and yesterday's history.
                let busy = [today - chrono::Days::new(1), today];
                let archive = Archive::new(today, set.keep_days, set.archive, set.archive_days);
                let _ = tx.send((set, sweep(&dir, cutoff(today, set.keep_days), &busy, archive)));
            });
            self.rx = Some(rx);
        }
        let got = self.rx.as_ref().and_then(|rx| rx.try_recv().ok())?;
        self.rx = None;
        let (set, out) = got;
        self.last = Kept {
            at: ito_common::now::now_local().format("%Y-%m-%d %H:%M:%S").to_string(),
            keep_days: set.keep_days,
            logs: out.logs,
            history: out.history,
            archive: set.archive,
            archived: out.archived,
            pruned: out.pruned,
            errors: out.errors,
        };
        self.writer.save_toml(&self.last);
        Some(self.line())
    }

    /// The last sweep in a line, for `koyomi engine status`.
    pub fn line(&self) -> String {
        let k = &self.last;
        if k.at.is_empty() {
            return "記録の整理: まだ".to_owned();
        }
        let mut s = format!("記録の整理: {}、{} 日分を残す、ログ {} 日・履歴 {} 日を消した", k.at, k.keep_days, k.logs, k.history);
        if k.archive {
            s.push_str(&format!("、{} 日分を archive/ に入れた", k.archived));
        }
        if k.pruned > 0 {
            s.push_str(&format!("、古い zip {} 個を消した", k.pruned));
        }
        if !k.errors.is_empty() {
            s.push_str(&format!("、消せなかった {} 件: {}", k.errors.len(), k.errors.join(" / ")));
        }
        s
    }

    /// Writes the last sweep before the engine ends.
    pub fn flush(&self) {
        if let Ok(text) = toml::to_string(&self.last) {
            let _ = self.writer.save_now(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEEP_90: Settings = Settings { keep_days: 90, archive: false, archive_days: 0 };

    #[test]
    fn the_sweep_runs_once_a_day_and_again_when_keep_days_changes() {
        let dir = std::env::temp_dir().join(format!("koyomi-retain-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old = dir.join("logs").join("2020-01-01");
        std::fs::create_dir_all(&old).unwrap();
        let mut r = Retain::new(&dir);
        assert_eq!(r.line(), "記録の整理: まだ");
        let mut line = None;
        for _ in 0..200 {
            line = r.tick(KEEP_90);
            if line.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let line = line.expect("the sweep answers");
        assert!(line.contains("ログ 1 日・履歴 0 日"), "{line}");
        assert!(!line.contains("archive/"), "{line}");
        assert!(!old.exists());
        // The same day and days: nothing more.
        assert!(r.rx.is_none());
        assert_eq!(r.tick(KEEP_90), None);
        assert!(r.rx.is_none());
        // Other days: again.
        assert_eq!(r.tick(Settings { keep_days: 30, ..KEEP_90 }), None);
        assert!(r.rx.is_some());
        r.flush();
        let back = Retain::new(&dir);
        assert_eq!(back.last.logs, 1);
        assert_eq!(back.last.keep_days, 90);
    }

    #[test]
    fn with_archive_on_the_days_go_into_archive_first() {
        let dir = std::env::temp_dir().join(format!("koyomi-retain-archive-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old = dir.join("logs").join("2020-01-01").join("20200101-090000-00");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("copy.log"), "done\n").unwrap();
        let mut r = Retain::new(&dir);
        let set = Settings { archive: true, ..KEEP_90 };
        let mut line = None;
        for _ in 0..200 {
            line = r.tick(set);
            if line.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let line = line.expect("the sweep answers");
        assert!(line.contains("ログ 1 日・履歴 0 日を消した、1 日分を archive/ に入れた"), "{line}");
        assert!(!old.exists());
        assert!(dir.join("archive").join("2020-01.zip").exists());
        // Turning it off is another sweep.
        assert_eq!(r.tick(KEEP_90), None);
        assert!(r.rx.is_some());
    }
}
