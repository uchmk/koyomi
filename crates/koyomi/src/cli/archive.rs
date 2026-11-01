//! `koyomi archive [list]`, `koyomi archive show <YYYY-MM-DD>` and `koyomi archive log <run> <job>`:
//! what the sweep put into `archive/YYYY-MM.zip` (koyomi.toml's `archive = true`), read straight
//! from the zips, so the engine need not be running. The engine reads the days and the logs from
//! there too (`History`, `Log`): for `koyomi history`, `koyomi log`, the window and the Web UI.

use std::path::PathBuf;

use chrono::NaiveDate;
use koyomi_core::retain;
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
    match log_text(run, job) {
        Ok(text) => {
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

/// One job's whole log as `archive/` keeps it (`koyomi archive log`, and `koyomi log` with no
/// engine to ask).
pub(super) fn log_text(run: &str, job: &str) -> Result<String, String> {
    let bytes = retain::archived_log(&state()?, run, job)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_usage_and_the_commands_know_archive() {
        assert!(super::super::COMMANDS.contains(&"archive"));
        assert!(super::super::USAGE.contains("koyomi archive show <YYYY-MM-DD>"));
        assert!(super::super::USAGE.contains("koyomi log <run> <job>        the tail of one job's log (gone from logs/: from archive/)"));
    }
}
