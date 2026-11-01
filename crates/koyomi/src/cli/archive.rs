//! `koyomi archive [list]`, `koyomi archive show <YYYY-MM-DD>` and `koyomi archive log <run> <job>`:
//! what the sweep put into `archive/YYYY-MM.zip` (koyomi.toml's `archive = true`), read straight
//! from the zips, so the engine need not be running. `koyomi history` reads the days from here too.

use std::path::PathBuf;

use chrono::{Datelike, Days, NaiveDate};
use koyomi_core::retain;
use koyomi_core::run::Run;
use serde::Serialize;

use super::{emit, fail, out, say_runs, table, usage, Log};
use crate::proto::Runs;

pub(super) fn cmd(args: &[String]) -> i32 {
    match args {
        [] => list(),
        [op] if op == "list" => list(),
        [op, day] if op == "show" => match NaiveDate::parse_from_str(day, "%Y-%m-%d") {
            Ok(d) => show(d),
            Err(_) => usage("archive show takes YYYY-MM-DD"),
        },
        [op, run, job] if op == "log" => log(run, job),
        _ => usage("archive takes list, show <YYYY-MM-DD> or log <run> <job>"),
    }
}

fn state() -> Result<PathBuf, String> {
    crate::engine::Paths::find().map(|p| p.state)
}

/// `koyomi archive --toml`: the folder, each month's zip and what could not be read.
#[derive(Serialize)]
struct Archive {
    dir: String,
    zip: Vec<Zip>,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct Zip {
    month: String,
    path: String,
    bytes: u64,
    /// The days with a history file, and those with logs.
    history: Vec<String>,
    logs: Vec<String>,
}

/// The month zips, oldest first: how many days of history and of logs each holds, and its size.
fn list() -> i32 {
    let state = match state() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let days = |ds: Vec<NaiveDate>| ds.iter().map(NaiveDate::to_string).collect::<Vec<_>>();
    let (mut zip, mut errors) = (Vec::new(), Vec::new());
    for (month, path) in retain::zips(&state) {
        match retain::days_in(&path) {
            Ok((history, logs)) => zip.push(Zip {
                month: month.format("%Y-%m").to_string(),
                path: path.display().to_string(),
                bytes: std::fs::metadata(&path).map_or(0, |m| m.len()),
                history: days(history),
                logs: days(logs),
            }),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    let code = i32::from(!errors.is_empty());
    let dir = retain::archive_dir(&state).display().to_string();
    if super::toml() {
        emit(&toml::to_string(&Archive { dir, zip, errors }).unwrap_or_default());
        return code;
    }
    let t = table();
    if zip.is_empty() && errors.is_empty() {
        out(t.tr("archive.none"), false);
    }
    for z in &zip {
        let size = ito_common::text::bytes_iec(z.bytes);
        out(&t.trf("archive.line", &[&z.month, &z.history.len().to_string(), &z.logs.len().to_string(), &size]), false);
    }
    for e in &errors {
        out(&format!("koyomi: {e}"), true);
    }
    out(&dir, false);
    code
}

/// The day's runs as the archived history file leaves them, as `koyomi history` prints them.
fn show(day: NaiveDate) -> i32 {
    let state = match state() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let t = table();
    let runs = match retain::archived_day(&state, day) {
        Ok(Some(d)) => d.runs(),
        Ok(None) => return fail(&t.trf("archive.day_none", &[&day.to_string()])),
        Err(e) => return fail(&e),
    };
    if super::toml() {
        emit(&Runs { runs }.to_toml());
        return 0;
    }
    if runs.is_empty() {
        out(t.tr("run.history_none"), false);
    }
    say_runs(&t, &runs);
    0
}

/// One job's whole log from the archive.
fn log(run: &str, job: &str) -> i32 {
    let state = match state() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    match retain::archived_log(&state, run, job) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if super::toml() {
                emit(&toml::to_string(&Log { run: run.to_owned(), job: job.to_owned(), text }).unwrap_or_default());
            } else {
                out(text.trim_end_matches('\n'), false);
            }
            0
        }
        Err(e) => fail(&e),
    }
}

/// For `koyomi history`: the runs of the days from `from` to `to` that the engine had none of
/// (`have`) and whose month has a zip in `archive/`. What could not be read is said and left out.
pub(super) fn runs_between(from: NaiveDate, to: NaiveDate, have: &[Run]) -> Vec<Run> {
    let Ok(state) = state() else { return Vec::new() };
    let months: Vec<NaiveDate> = retain::zips(&state).into_iter().map(|(m, _)| m).collect();
    let mut out_runs = Vec::new();
    let mut day = from;
    while day <= to {
        let first = day.with_day(1).unwrap_or(day);
        let known = have.iter().any(|r| run_day(r) == Some(day));
        if !known && months.contains(&first) {
            match retain::archived_day(&state, day) {
                Ok(Some(d)) => out_runs.extend(d.runs()),
                Ok(None) => {}
                Err(e) => out(&format!("koyomi: {e}"), true),
            }
        }
        let Some(next) = day.checked_add_days(Days::new(1)) else { break };
        day = next;
    }
    out_runs
}

/// The day a run started on, from its id (`20260731-090000-00`).
pub(super) fn run_day(r: &Run) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(r.id.get(..8)?, "%Y%m%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_s_day_comes_from_its_id() {
        let at = NaiveDate::from_ymd_opt(2026, 7, 31).unwrap().and_hms_opt(9, 0, 0).unwrap();
        let how = koyomi_core::run::How::Manual;
        let run = |id: &str| Run::with_jobs(id, "g/n", "n", "2026-07-31", how, Vec::new(), at);
        assert_eq!(run_day(&run("20260731-090000-00")), NaiveDate::from_ymd_opt(2026, 7, 31));
        assert_eq!(run_day(&run("2026")), None);
        assert_eq!(run_day(&run("x0260731-090000-00")), None);
    }

    #[test]
    fn the_usage_and_the_commands_know_archive() {
        assert!(super::super::COMMANDS.contains(&"archive"));
        assert!(super::super::USAGE.contains("koyomi archive show <YYYY-MM-DD>"));
    }
}
