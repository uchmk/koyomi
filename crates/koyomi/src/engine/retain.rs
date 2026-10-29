//! The sweep of old logs and history: once a day, and again when
//! `keep_days` changes, `koyomi_core::retain::sweep` runs on a thread and
//! what it removed is kept in `retain.toml` (the engine has no log file of
//! its own), where `koyomi engine status` shows it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use chrono::{Local, NaiveDate};
use ito_common::kept;
use koyomi_core::retain::{cutoff, sweep, Swept};
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
    /// What could not be removed.
    #[serde(default)]
    errors: Vec<String>,
}

pub struct Retain {
    state_dir: PathBuf,
    /// The day and `keep_days` the last sweep was started for.
    swept_for: Option<(NaiveDate, u32)>,
    /// The sweep on its thread, until it answers.
    rx: Option<Receiver<(NaiveDate, u32, Swept)>>,
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

    /// Starts a sweep when today or `keep_days` is not what the last one
    /// was for, and takes in the answer of the one running. The line to
    /// tell on the console, when a sweep ended.
    pub fn tick(&mut self, keep_days: u32) -> Option<String> {
        let today = Local::now().date_naive();
        if self.rx.is_none() && self.swept_for != Some((today, keep_days)) {
            self.swept_for = Some((today, keep_days));
            let (tx, rx) = mpsc::channel();
            let dir = self.state_dir.clone();
            std::thread::spawn(move || {
                // The runner still writes today's and yesterday's history.
                let busy = [today - chrono::Days::new(1), today];
                let _ = tx.send((today, keep_days, sweep(&dir, cutoff(today, keep_days), &busy)));
            });
            self.rx = Some(rx);
        }
        let got = self.rx.as_ref().and_then(|rx| rx.try_recv().ok())?;
        self.rx = None;
        let (_, keep_days, out) = got;
        self.last = Kept {
            at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            keep_days,
            logs: out.logs,
            history: out.history,
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
            line = r.tick(90);
            if line.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let line = line.expect("the sweep answers");
        assert!(line.contains("ログ 1 日・履歴 0 日"), "{line}");
        assert!(!old.exists());
        // The same day and days: nothing more.
        assert!(r.rx.is_none());
        assert_eq!(r.tick(90), None);
        assert!(r.rx.is_none());
        // Other days: again.
        assert_eq!(r.tick(30), None);
        assert!(r.rx.is_some());
        r.flush();
        let back = Retain::new(&dir);
        assert_eq!(back.last.logs, 1);
        assert_eq!(back.last.keep_days, 90);
    }
}
