//! What is too old to keep: the day folders under `logs/` and the day files
//! under `history/` from before `keep_days` of koyomi.toml.
//!
//! A day is read off the name alone (`logs/2026-07-31/`,
//! `history/2026/07/31.toml`): nothing is opened, and a name that is not a
//! day is not touched -- what a person put there is theirs. `keep_days`
//! counts today: 90 keeps today and the 89 days before it.
//!
//! With `archive = true` a day goes into its month's zip first
//! (`archive/2026-07.zip`, holding `logs/2026-07-31/…` and
//! `history/2026/07/31.toml` under the same names), and only a month whose
//! zip was written loses its originals. `archive_days` ages the zips out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Days, Months, NaiveDate};
use ito_zip::{OnExisting, Source};

use crate::history::Day;
use crate::run::Run;

/// What one sweep removed, and what it could not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// Day folders removed under `logs/`.
    pub logs: usize,
    /// Day files removed under `history/`.
    pub history: usize,
    /// Days put into `archive/` before they were removed.
    pub archived: usize,
    /// Month zips removed from `archive/` by `archive_days`.
    pub pruned: usize,
    /// Each path that could not be removed, with why.
    pub errors: Vec<String>,
}

/// What the sweep does with what it lets go: `archive` and `archive_days`
/// of koyomi.toml.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Archive {
    /// Into `archive/YYYY-MM.zip` first; a day that does not go in is not removed.
    pub on: bool,
    /// Month zips whose last day is before this leave `archive/`; `None` keeps them.
    pub drop_before: Option<NaiveDate>,
}

impl Archive {
    /// From koyomi.toml. A zip is kept at least as long as `keep_days`
    /// keeps the days themselves, so a short `archive_days` cannot throw a
    /// month away the day it was put in.
    pub fn new(today: NaiveDate, keep_days: u32, archive: bool, archive_days: u32) -> Archive {
        Archive { on: archive, drop_before: (archive_days > 0).then(|| cutoff(today, archive_days.max(keep_days))) }
    }
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
/// A month or year folder left empty goes with its last file. With
/// `archive.on` the days go into their months' zips first, and a month whose
/// zip could not be written keeps its days (the error says why). Blocks on
/// the disk: call it off the engine's loop.
pub fn sweep(state_dir: &Path, cutoff: NaiveDate, busy: &[NaiveDate], archive: Archive) -> Swept {
    let mut out = Swept::default();
    let at = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let logs: Vec<_> = old_logs(&state_dir.join("logs"), cutoff).into_iter().filter(|(d, _)| !busy.contains(d)).collect();
    let history: Vec<_> = old_history(&state_dir.join("history"), cutoff).into_iter().filter(|(d, _)| !busy.contains(d)).collect();
    let failed = if archive.on { put_away(state_dir, &logs, &history, &mut out) } else { BTreeSet::new() };
    let kept = |d: &NaiveDate| failed.contains(&first_of(*d));
    for (day, path) in logs {
        if kept(&day) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => out.logs += 1,
            Err(e) => out.errors.push(at(&path, e)),
        }
    }
    for (day, path) in history {
        if kept(&day) {
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
    if let Some(before) = archive.drop_before {
        for (month, path) in zips(state_dir) {
            if last_of(month) < before {
                match std::fs::remove_file(&path) {
                    Ok(()) => out.pruned += 1,
                    Err(e) => out.errors.push(at(&path, e)),
                }
            }
        }
    }
    out
}

/// The first day of `day`'s month.
fn first_of(day: NaiveDate) -> NaiveDate {
    day.with_day(1).unwrap_or(day)
}

/// The last day of the month that starts on `first`.
fn last_of(first: NaiveDate) -> NaiveDate {
    first.checked_add_months(Months::new(1)).and_then(|n| n.pred_opt()).unwrap_or(NaiveDate::MAX)
}

/// Writes the days into their months' zips, one write per month (all of a
/// month or none of it). The months that did not go in.
fn put_away(state_dir: &Path, logs: &[(NaiveDate, PathBuf)], history: &[(NaiveDate, PathBuf)], out: &mut Swept) -> BTreeSet<NaiveDate> {
    let mut months: BTreeMap<NaiveDate, (Vec<Source>, BTreeSet<NaiveDate>)> = BTreeMap::new();
    let mut failed = BTreeSet::new();
    for (day, path) in logs {
        let m = months.entry(first_of(*day)).or_default();
        let before = m.0.len();
        if let Err(e) = files(path, &day.format("logs/%Y-%m-%d").to_string(), &mut m.0) {
            out.errors.push(e);
            failed.insert(first_of(*day));
        }
        if m.0.len() > before {
            m.1.insert(*day);
        }
    }
    for (day, path) in history {
        let m = months.entry(first_of(*day)).or_default();
        m.0.push(Source::new(path, history_entry(*day)));
        m.1.insert(*day);
    }
    if months.values().all(|(sources, _)| sources.is_empty()) {
        return failed;
    }
    let dir = archive_dir(state_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        out.errors.push(format!("{}: {e}", dir.display()));
        return months.into_keys().collect();
    }
    for (month, (sources, days)) in months {
        if failed.contains(&month) || sources.is_empty() {
            continue;
        }
        let zip = dir.join(month.format("%Y-%m.zip").to_string());
        match ito_zip::add(&zip, &sources, OnExisting::Replace) {
            Ok(_) => out.archived += days.len(),
            Err(e) => {
                out.errors.push(format!("{}: {e}", zip.display()));
                failed.insert(month);
            }
        }
    }
    failed
}

/// Every file under `dir`, named `<prefix>/<path under dir>`, in name order.
/// A name that is not UTF-8 is an error: the zip would not hold it as it is.
fn files(dir: &Path, prefix: &str, out: &mut Vec<Source>) -> Result<(), String> {
    let at = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let mut entries: Vec<_> = std::fs::read_dir(dir).and_then(Iterator::collect).map_err(|e| at(dir, e))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let path = e.path();
        let Some(name) = e.file_name().to_str().map(|n| format!("{prefix}/{n}")) else {
            return Err(format!("{}: 名前が UTF-8 でないので zip に入れられない", path.display()));
        };
        let meta = std::fs::metadata(&path).map_err(|e| at(&path, e))?;
        if meta.is_dir() {
            files(&path, &name, out)?;
        } else if meta.is_file() {
            out.push(Source::new(path, name));
        }
    }
    Ok(())
}

/// `archive/` in the state folder.
pub fn archive_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("archive")
}

/// The month a zip of `archive/` is of, from its name: `2026-07.zip` is 2026-07-01.
pub fn zip_month(name: &str) -> Option<NaiveDate> {
    let m = name.strip_suffix(".zip").filter(|m| m.len() == 7)?;
    NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d").ok()
}

/// The month zips in `archive/`, oldest first.
pub fn zips(state_dir: &Path) -> Vec<(NaiveDate, PathBuf)> {
    let mut out: Vec<_> = names(&archive_dir(state_dir)).into_iter().filter_map(|(n, p)| zip_month(&n).map(|m| (m, p))).collect();
    out.sort();
    out
}

/// A day's history file inside its month's zip: `history/2026/07/31.toml`.
pub fn history_entry(day: NaiveDate) -> String {
    day.format("history/%Y/%m/%d.toml").to_string()
}

/// A job's log inside its month's zip, `logs/2026-07-31/<run>/<job>.log`,
/// and the day: the run's id starts with it.
pub fn log_entry(run: &str, job: &str) -> Option<(NaiveDate, String)> {
    let day = NaiveDate::parse_from_str(run.get(..8)?, "%Y%m%d").ok()?;
    Some((day, format!("logs/{}/{run}/{job}.log", day.format("%Y-%m-%d"))))
}

/// The days a month's zip holds: those with a history file, and those with logs.
pub fn days_in(zip: &Path) -> Result<(Vec<NaiveDate>, Vec<NaiveDate>), String> {
    let (mut history, mut logs) = (BTreeSet::new(), BTreeSet::new());
    for e in ito_zip::list(zip).map_err(|e| e.to_string())? {
        let parts: Vec<&str> = e.name.split('/').collect();
        match parts.as_slice() {
            ["history", y, m, f] => history.extend(history_day(y, m, f)),
            ["logs", d, _, ..] => logs.extend(log_day(d)),
            _ => {}
        }
    }
    Ok((history.into_iter().collect(), logs.into_iter().collect()))
}

/// A day's history read back from `archive/`; `None` when its month's zip
/// does not hold it.
pub fn archived_day(state_dir: &Path, day: NaiveDate) -> Result<Option<Day>, String> {
    let zip = archive_dir(state_dir).join(day.format("%Y-%m.zip").to_string());
    match ito_zip::read(&zip, &history_entry(day)) {
        Ok(bytes) => Day::parse(&String::from_utf8_lossy(&bytes)).map(Some).map_err(|e| format!("{}: {e}", zip.display())),
        Err(ito_zip::Error::NotFound(_)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// A job's whole log read back from `archive/`.
pub fn archived_log(state_dir: &Path, run: &str, job: &str) -> Result<Vec<u8>, String> {
    let (day, name) = log_entry(run, job).ok_or_else(|| format!("実行の ID でない: {run}"))?;
    let zip = archive_dir(state_dir).join(day.format("%Y-%m.zip").to_string());
    ito_zip::read(&zip, &name).map_err(|e| e.to_string())
}

/// Adds to `runs` the runs of the days from `from` to `to` that it has none of
/// and whose month has a zip in `archive/`, read back from there, and keeps
/// `runs` by the day they started (each day's order stays). What `History`
/// answers with for the days the sweep took. Each zip is read only for those
/// days; what could not be read is left out and said in what comes back.
pub fn add_archived(state_dir: &Path, from: NaiveDate, to: NaiveDate, runs: &mut Vec<Run>) -> Vec<String> {
    let months: BTreeSet<NaiveDate> = zips(state_dir).into_iter().map(|(m, _)| m).collect();
    let have: BTreeSet<NaiveDate> = runs.iter().map(|r| r.started.date()).collect();
    let (mut added, mut errors) = (Vec::new(), Vec::new());
    let mut day = from;
    while day <= to && !months.is_empty() {
        if !have.contains(&day) && months.contains(&first_of(day)) {
            match archived_day(state_dir, day) {
                Ok(Some(d)) => added.extend(d.runs()),
                Ok(None) => {}
                Err(e) => errors.push(e),
            }
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }
    if !added.is_empty() {
        runs.extend(added);
        runs.sort_by_key(|r| r.started.date());
    }
    errors
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
        let out = sweep(&state, day("2026-08-02"), &[day("2026-08-01")], Archive::default());
        assert_eq!(out, Swept { logs: 1, history: 1, ..Swept::default() });
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
        assert_eq!(sweep(&state, day("2026-08-02"), &[], Archive::default()).history, 1);
        assert!(!state.join("history/2025").exists());
        assert!(state.join("history/2026/10/30.toml").exists());
        // Nothing there: nothing done, no error.
        assert_eq!(sweep(&fresh("sweep-none"), day("2026-08-02"), &[], Archive::default()), Swept::default());
        // Archive off: no archive/ is made.
        assert!(!state.join("archive").exists());
    }

    const ON: Archive = Archive { on: true, drop_before: None };

    #[test]
    fn the_archive_takes_the_days_before_they_go() {
        let state = fresh("archive");
        put(&state, &["2026-07-30", "2026-07-31", "2026-08-01", "2026-10-30"], &["2026-07-30", "2026-08-01", "2026-10-30"], &[]);
        std::fs::remove_dir_all(state.join("logs/2026-07-31/r1")).unwrap();
        std::fs::create_dir_all(state.join("logs/2026-07-31/r2")).unwrap();
        std::fs::write(state.join("history/2026/07/30.toml"), "event = []\n").unwrap();
        let out = sweep(&state, day("2026-08-02"), &[], ON);
        // 07-31 has only an empty folder: removed, nothing to put away.
        assert_eq!(out, Swept { logs: 3, history: 2, archived: 2, ..Swept::default() });
        let july = state.join("archive/2026-07.zip");
        let names: Vec<String> = ito_zip::list(&july).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["logs/2026-07-30/r1/a.log", "history/2026/07/30.toml"]);
        assert_eq!(ito_zip::read(&july, "history/2026/07/30.toml").unwrap(), b"event = []\n");
        let august = state.join("archive/2026-08.zip");
        assert_eq!(days_in(&august).unwrap(), (vec![day("2026-08-01")], vec![day("2026-08-01")]));
        assert_eq!(zips(&state).iter().map(|(m, _)| *m).collect::<Vec<_>>(), [day("2026-07-01"), day("2026-08-01")]);
        assert!(!state.join("logs/2026-07-30").exists() && !state.join("history/2026/07").exists());
        assert!(state.join("logs/2026-10-30").exists() && state.join("history/2026/10/30.toml").exists());
        // Read back: the day's runs, a job's log; a day not there is none.
        assert_eq!(archived_day(&state, day("2026-07-30")).unwrap(), Some(Day::default()));
        assert_eq!(archived_day(&state, day("2026-08-02")).unwrap(), None);
        assert_eq!(archived_day(&state, day("2026-06-01")).unwrap(), None);
        assert!(archived_log(&state, "20260730-090000-00", "a").unwrap_err().contains("logs/2026-07-30/20260730-090000-00/a.log"));
        std::fs::create_dir_all(state.join("logs/2026-07-29/20260729-090000-00")).unwrap();
        std::fs::write(state.join("logs/2026-07-29/20260729-090000-00/copy.log"), "done\n").unwrap();
        // A later sweep adds to the month's zip.
        assert_eq!(sweep(&state, day("2026-08-02"), &[], ON).archived, 1);
        assert_eq!(archived_log(&state, "20260729-090000-00", "copy").unwrap(), b"done\n");
        assert_eq!(ito_zip::list(&july).unwrap().len(), 3);
    }

    /// A history file for day `d` with one started run for each id, an hour apart from 9:00.
    fn put_runs(state: &Path, d: &str, ids: &[&str]) {
        let mut events = Vec::new();
        for (h, id) in (9..).zip(ids) {
            events.push(crate::history::run_start(&run(id, d, h)));
        }
        let p = crate::history::path(&state.join("history"), day(d));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, Day { events }.to_toml()).unwrap();
    }

    fn run(id: &str, d: &str, hour: u32) -> Run {
        let at = day(d).and_hms_opt(hour, 0, 0).unwrap();
        Run::with_jobs(id, "g/n", "n", d, crate::run::How::Manual, Vec::new(), at)
    }

    #[test]
    fn archived_days_fill_only_the_days_with_no_runs() {
        let state = fresh("fill");
        put_runs(&state, "2026-07-29", &["20260729-090000-00"]);
        put_runs(&state, "2026-07-30", &["20260730-090000-00", "20260730-100000-00"]);
        put_runs(&state, "2026-08-01", &["20260801-090000-00"]);
        assert_eq!(sweep(&state, day("2026-08-02"), &[], ON).archived, 3);
        // The engine has 07-30 (a run of its own) and 08-03; 07-28 and 07-31 have nothing anywhere.
        let mut runs = vec![run("20260730-120000-00", "2026-07-30", 12), run("20260803-090000-00", "2026-08-03", 9)];
        assert!(add_archived(&state, day("2026-07-28"), day("2026-08-03"), &mut runs).is_empty());
        let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["20260729-090000-00", "20260730-120000-00", "20260801-090000-00", "20260803-090000-00"]);
        // Again: every day has its runs now, so nothing comes twice.
        assert!(add_archived(&state, day("2026-07-28"), day("2026-08-03"), &mut runs).is_empty());
        assert_eq!(runs.len(), 4);
        // Outside the range: nothing.
        let mut none = Vec::new();
        assert!(add_archived(&state, day("2026-08-02"), day("2026-08-31"), &mut none).is_empty());
        assert!(none.is_empty());
        // No archive/ at all, and a zip that cannot be read (said, and left out).
        assert!(add_archived(&fresh("fill-none"), day("2026-07-28"), day("2026-08-03"), &mut none).is_empty());
        std::fs::write(state.join("archive/2026-06.zip"), "not a zip").unwrap();
        let errors = add_archived(&state, day("2026-06-30"), day("2026-07-29"), &mut none);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(none.len(), 1, "July's day still comes");
    }

    #[test]
    fn a_month_that_does_not_go_in_keeps_its_days() {
        let state = fresh("archive-fail");
        put(&state, &["2026-07-30", "2026-08-01"], &["2026-07-30", "2026-08-01"], &[]);
        // July's zip is not a zip: nothing of July goes; August does.
        std::fs::create_dir_all(state.join("archive")).unwrap();
        std::fs::write(state.join("archive/2026-07.zip"), "not a zip").unwrap();
        let out = sweep(&state, day("2026-08-02"), &[], ON);
        assert_eq!((out.logs, out.history, out.archived, out.errors.len()), (1, 1, 1, 1), "{out:?}");
        assert!(out.errors[0].contains("2026-07.zip"), "{:?}", out.errors);
        assert!(state.join("logs/2026-07-30/r1/a.log").exists() && state.join("history/2026/07/30.toml").exists());
        assert!(!state.join("logs/2026-08-01").exists());
        assert_eq!(std::fs::read(state.join("archive/2026-07.zip")).unwrap(), b"not a zip");
    }

    #[test]
    fn archive_days_ages_the_month_zips_out() {
        let state = fresh("archive-prune");
        std::fs::create_dir_all(state.join("archive")).unwrap();
        for f in ["2025-12.zip", "2026-06.zip", "2026-07.zip", "notes.zip", "2026-7.zip"] {
            std::fs::write(state.join("archive").join(f), "x").unwrap();
        }
        // 365 days from 2027-07-15 keeps 2026-07-16 on: June goes, July (ends 07-31) stays.
        let a = Archive::new(day("2027-07-15"), 90, false, 365);
        assert_eq!(a, Archive { on: false, drop_before: Some(day("2026-07-16")) });
        assert_eq!(sweep(&state, day("2027-04-17"), &[], a).pruned, 2);
        let mut left: Vec<String> = names(&state.join("archive")).into_iter().map(|(n, _)| n).collect();
        left.sort();
        assert_eq!(left, ["2026-07.zip", "2026-7.zip", "notes.zip"]);
        // archive_days 0 keeps them; one shorter than keep_days is keep_days.
        assert_eq!(Archive::new(day("2027-07-15"), 90, true, 0).drop_before, None);
        assert_eq!(Archive::new(day("2027-07-15"), 90, true, 30).drop_before, Some(cutoff(day("2027-07-15"), 90)));
        assert_eq!(zip_month("2026-07.zip"), Some(day("2026-07-01")));
        assert_eq!(zip_month("2026-13.zip"), None);
        let entry = log_entry("20260731-090000-00", "copy");
        assert_eq!(entry, Some((day("2026-07-31"), "logs/2026-07-31/20260731-090000-00/copy.log".to_owned())));
        assert_eq!(log_entry("nope", "copy"), None);
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
