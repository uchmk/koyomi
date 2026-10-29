//! What is too old to keep: the day folders under `logs/` and the day files
//! under `history/` from before `keep_days` of koyomi.toml.
//!
//! A day is read off the name alone (`logs/2026-07-31/`,
//! `history/2026/07/31.toml`): nothing is opened, and a name that is not a
//! day is not touched -- what a person put there is theirs. `keep_days`
//! counts today: 90 keeps today and the 89 days before it.

use std::path::{Path, PathBuf};

use chrono::{Days, NaiveDate};

/// What one sweep removed, and what it could not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// Day folders removed under `logs/`.
    pub logs: usize,
    /// Day files removed under `history/`.
    pub history: usize,
    /// Each path that could not be removed, with why.
    pub errors: Vec<String>,
}

/// The oldest day kept: `today` and the `keep_days - 1` days before it. A
/// `keep_days` of 0 keeps today (nothing of today is ever swept).
pub fn cutoff(today: NaiveDate, keep_days: u32) -> NaiveDate {
    today.checked_sub_days(Days::new(u64::from(keep_days.saturating_sub(1)))).unwrap_or(NaiveDate::MIN)
}

/// The day a `logs/` folder is of, from its name: `2026-07-31`.
pub fn log_day(name: &str) -> Option<NaiveDate> {
    (name.len() == 10).then(|| NaiveDate::parse_from_str(name, "%Y-%m-%d").ok()).flatten()
}

/// The day a `history/` file is of, from its folders and name: `2026`,
/// `07`, `31.toml`.
pub fn history_day(year: &str, month: &str, file: &str) -> Option<NaiveDate> {
    let day = file.strip_suffix(".toml")?;
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    NaiveDate::parse_from_str(&format!("{year}-{month}-{day}"), "%Y-%m-%d").ok()
}

/// The names in `dir`, in no order; none when it cannot be read.
fn names(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.filter_map(Result::ok).filter_map(|e| Some((e.file_name().to_str()?.to_owned(), e.path()))).collect()
}

/// The day folders under `logs` from before `cutoff`, oldest first.
pub fn old_logs(logs: &Path, cutoff: NaiveDate) -> Vec<(NaiveDate, PathBuf)> {
    let mut out: Vec<_> = names(logs).into_iter().filter_map(|(n, p)| log_day(&n).map(|d| (d, p))).filter(|(d, _)| *d < cutoff).collect();
    out.sort();
    out
}

/// The day files under `history` from before `cutoff`, oldest first.
pub fn old_history(history: &Path, cutoff: NaiveDate) -> Vec<(NaiveDate, PathBuf)> {
    let mut out = Vec::new();
    for (year, ydir) in names(history) {
        if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        for (month, mdir) in names(&ydir) {
            if month.len() != 2 || !month.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            for (file, path) in names(&mdir) {
                if let Some(d) = history_day(&year, &month, &file) {
                    if d < cutoff {
                        out.push((d, path));
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Removes what is older than `cutoff` under `state_dir`'s `logs/` and
/// `history/`, except the days in `busy` (the ones the engine still writes).
/// A month or year folder left empty goes with its last file. Blocks on the
/// disk: call it off the engine's loop.
pub fn sweep(state_dir: &Path, cutoff: NaiveDate, busy: &[NaiveDate]) -> Swept {
    let mut out = Swept::default();
    let at = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    for (day, path) in old_logs(&state_dir.join("logs"), cutoff) {
        if busy.contains(&day) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => out.logs += 1,
            Err(e) => out.errors.push(at(&path, e)),
        }
    }
    for (day, path) in old_history(&state_dir.join("history"), cutoff) {
        if busy.contains(&day) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                out.history += 1;
                // The month, then the year, when nothing is left in them.
                let mut dir = path.parent();
                for _ in 0..2 {
                    match dir {
                        Some(d) if std::fs::remove_dir(d).is_ok() => dir = d.parent(),
                        _ => break,
                    }
                }
            }
            Err(e) => out.errors.push(at(&path, e)),
        }
    }
    out
}

/// How many days' records would go at once if `keep_days` became
/// `new_days`: the days with a log folder or a history file from before the
/// new cutoff that are not before the old one. Zero when `new_days` is not
/// shorter.
pub fn days_lost(state_dir: &Path, today: NaiveDate, old_days: u32, new_days: u32) -> usize {
    if new_days >= old_days {
        return 0;
    }
    let (was, now) = (cutoff(today, old_days), cutoff(today, new_days));
    let mut days: Vec<NaiveDate> = old_logs(&state_dir.join("logs"), now)
        .into_iter()
        .chain(old_history(&state_dir.join("history"), now))
        .map(|(d, _)| d)
        .filter(|d| *d >= was)
        .collect();
    days.sort();
    days.dedup();
    days.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn fresh(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koyomi-retain-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(state: &Path, logs: &[&str], history: &[&str], others: &[&str]) {
        for d in logs {
            std::fs::create_dir_all(state.join("logs").join(d).join("r1")).unwrap();
            std::fs::write(state.join("logs").join(d).join("r1").join("a.log"), "x").unwrap();
        }
        for d in history {
            let p = crate::history::path(&state.join("history"), day(d));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        for o in others {
            let p = state.join(o);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "mine").unwrap();
        }
    }

    #[test]
    fn the_cutoff_counts_today() {
        assert_eq!(cutoff(day("2026-10-30"), 90), day("2026-08-02"));
        assert_eq!(cutoff(day("2026-10-30"), 1), day("2026-10-30"));
        assert_eq!(cutoff(day("2026-10-30"), 0), day("2026-10-30"));
    }

    #[test]
    fn days_are_read_off_the_names_only() {
        assert_eq!(log_day("2026-07-31"), Some(day("2026-07-31")));
        assert_eq!(log_day("2026-7-31"), None);
        assert_eq!(log_day("2026-07-31-old"), None);
        assert_eq!(log_day("notes"), None);
        assert_eq!(history_day("2026", "07", "31.toml"), Some(day("2026-07-31")));
        assert_eq!(history_day("2026", "07", "31.toml.bak"), None);
        assert_eq!(history_day("2026", "7", "31.toml"), None);
        assert_eq!(history_day("2026", "13", "01.toml"), None);
    }

    #[test]
    fn the_sweep_removes_old_days_and_leaves_the_rest() {
        let state = fresh("sweep");
        put(
            &state,
            &["2026-07-30", "2026-08-01", "2026-08-02", "2026-10-30"],
            &["2026-07-30", "2026-08-01", "2026-08-02", "2026-10-30"],
            &["logs/notes/keep.txt", "logs/2026-07-30-old/a.log", "history/2026/07/notes.txt", "history/readme.txt"],
        );
        let out = sweep(&state, day("2026-08-02"), &[day("2026-08-01")]);
        assert_eq!(out, Swept { logs: 1, history: 1, errors: vec![] });
        assert!(!state.join("logs/2026-07-30").exists());
        assert!(state.join("logs/2026-08-01").exists(), "busy: left");
        assert!(state.join("logs/2026-08-02").exists());
        assert!(state.join("logs/notes/keep.txt").exists());
        assert!(state.join("logs/2026-07-30-old").exists());
        assert!(!state.join("history/2026/07/30.toml").exists());
        assert!(state.join("history/2026/07/notes.txt").exists(), "the month stays with a stranger in it");
        assert!(state.join("history/2026/08/01.toml").exists());
        assert!(state.join("history/readme.txt").exists());
        // A month left empty goes; so does a year.
        let state = fresh("sweep-empty");
        put(&state, &[], &["2025-12-31", "2026-10-30"], &[]);
        assert_eq!(sweep(&state, day("2026-08-02"), &[]).history, 1);
        assert!(!state.join("history/2025").exists());
        assert!(state.join("history/2026/10/30.toml").exists());
        // Nothing there: nothing done, no error.
        assert_eq!(sweep(&fresh("sweep-none"), day("2026-08-02"), &[]), Swept::default());
    }

    #[test]
    fn shortening_counts_the_days_that_would_go() {
        let state = fresh("lost");
        put(&state, &["2026-07-30", "2026-08-02", "2026-09-01", "2026-10-30"], &["2026-08-02", "2026-09-15"], &[]);
        let today = day("2026-10-30");
        // 90 → 30: the cutoff moves from 08-02 to 10-01; 08-02 (twice), 09-01 and 09-15 go, 07-30 was already gone.
        assert_eq!(days_lost(&state, today, 90, 30), 3);
        assert_eq!(days_lost(&state, today, 30, 90), 0);
        assert_eq!(days_lost(&state, today, 90, 90), 0);
    }
}
