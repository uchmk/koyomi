//! `--csv <file>`: what `list`, `history`, `report` and `plan` print, as a table for a spreadsheet.
//!
//! The rows are built here; ito-table writes them: CSV (UTF-8 with a BOM, CRLF, RFC 4180, a cell
//! that looks like a formula kept as text), or Excel when the file name ends in `.xlsx` (numbers
//! as numbers, times as real dates). `-` is the CSV on standard output, without the BOM. The
//! heads and words are the language table's.

use std::path::Path;

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use ito_i18n::Table;
use ito_table::{Cell, Stamp};
use koyomi_core::run::{How, Run};
use koyomi_core::scan::Def;

/// A sheet with a row of `heads` (keys of `[csv]`), named after `name`.
fn sheet(t: &Table, name: &str, heads: &[&str]) -> ito_table::Table {
    let head: Vec<String> = heads.iter().map(|h| t.tr(&format!("csv.{h}")).to_owned()).collect();
    ito_table::Table::new(&head.iter().map(String::as_str).collect::<Vec<_>>()).sheet(name)
}

fn day(d: NaiveDate) -> Cell {
    Stamp::date(d.year() as u16, d.month() as u8, d.day() as u8).into()
}

fn at(d: Option<NaiveDateTime>) -> Cell {
    d.map_or_else(
        || Cell::Text(String::new()),
        |d| Stamp::new(d.year() as u16, d.month() as u8, d.day() as u8, d.hour() as u8, d.minute() as u8, d.second() as u8).into(),
    )
}

fn number<N: Into<Cell>>(n: Option<N>) -> Cell {
    n.map_or_else(|| Cell::Text(String::new()), Into::into)
}

/// One row per job of every definition that reads (a definition that does not: one row, no job).
pub fn jobs(t: &Table, defs: &[Def]) -> ito_table::Table {
    let mut s = sheet(t, "jobs", &["group", "jobnet", "title", "standing", "calendar", "job", "job_name", "kind", "run", "after"]);
    for d in defs {
        let net = |job: [String; 5]| {
            let head = [
                d.group.clone(),
                d.id.clone(),
                d.label().to_owned(),
                crate::review::word(t, d.standing),
                d.calendar.clone().unwrap_or_default(),
            ];
            head.into_iter().chain(job).map(Cell::from).collect::<Vec<_>>()
        };
        let Some(n) = d.net.as_ref().filter(|n| !n.jobs.is_empty()) else {
            s.row(net(Default::default()));
            continue;
        };
        for j in &n.jobs {
            let run = match (&j.shell, &j.script) {
                (Some(sh), Some(sc)) => format!("{sh}: {sc}"),
                _ => j.run.join(" "),
            };
            let after: Vec<&str> = j.after.iter().map(|l| l.job.as_str()).collect();
            let kind = t.tr(&format!("part.{}", j.part.key())).to_owned();
            s.row(net([j.id.clone(), j.name.clone().unwrap_or_default(), kind, run, after.join(" ")]));
        }
    }
    s
}

/// The key of `how`'s word (`csv.how_<key>`).
fn how(h: How) -> &'static str {
    match h {
        How::Manual => "how_manual",
        How::Schedule => "how_schedule",
        How::Rerun => "how_rerun",
        How::File => "how_file",
        How::Start => "how_start",
        How::After => "how_after",
        How::Log => "how_log",
        How::Mail => "how_mail",
        How::Idle => "how_idle",
    }
}

/// One row per run, oldest first as given; the jobs that ended 異常 by id.
pub fn runs(t: &Table, rs: &[Run]) -> ito_table::Table {
    let mut s = sheet(
        t,
        "runs",
        &["run_id", "jobnet", "title", "date", "how", "planned", "started", "ended", "seconds", "state", "reason", "failed"],
    );
    for r in rs {
        let failed: Vec<&str> = r.jobs.iter().filter(|j| j.state.is_error()).map(|j| j.id.as_str()).collect();
        let date = NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").map_or_else(|_| r.date.clone().into(), day);
        s.row([
            r.id.clone().into(),
            r.jobnet.clone().into(),
            r.title.clone().into(),
            date,
            t.tr(&format!("csv.{}", how(r.how))).to_owned().into(),
            at(r.planned),
            at(Some(r.started)),
            at(r.ended),
            number(r.duration().map(|d| d.num_seconds())),
            t.tr(&format!("state.{}", r.state.key())).to_owned().into(),
            r.reason.clone().into(),
            failed.join(" ").into(),
        ]);
    }
    s
}

/// One row per file a `send` or `fetch` moved, the runs oldest first as given; `verified` is
/// empty when the far side's hash was not compared.
pub fn files(t: &Table, rs: &[Run]) -> ito_table::Table {
    let mut s = sheet(t, "files", &["run_id", "jobnet", "job", "ended", "file", "bytes", "sha256", "verified"]);
    for r in rs {
        for j in &r.jobs {
            for m in &j.moved {
                let verified = match m.matched {
                    Some(true) => t.tr("csv.verified_yes"),
                    Some(false) => t.tr("csv.verified_no"),
                    None => "",
                };
                s.row([
                    r.id.clone().into(),
                    r.jobnet.clone().into(),
                    j.id.clone().into(),
                    at(j.ended),
                    m.name.clone().into(),
                    m.bytes.into(),
                    m.sha256.clone().unwrap_or_default().into(),
                    verified.to_owned().into(),
                ]);
            }
        }
    }
    s
}

/// One row per net: its runs, how many passed and failed, the share that passed, the usual time
/// of the good ones and the jobs that failed most.
pub fn report(t: &Table, rows: &[koyomi_core::report::Row]) -> ito_table::Table {
    let mut s = sheet(t, "report", &["jobnet", "title", "runs", "passed", "failed_runs", "other", "rate", "average", "worst"]);
    for r in rows {
        s.row([
            r.jobnet.clone().into(),
            r.title.clone().into(),
            r.runs.into(),
            r.passed.into(),
            r.failed.into(),
            r.other.into(),
            number(r.rate),
            number(r.average),
            crate::report::worst(r).into(),
        ]);
    }
    s
}

/// One row per start; a day with none as one row with only its date and weekday.
pub fn plan(t: &Table, days: &[crate::plan::Day]) -> ito_table::Table {
    let mut s = sheet(t, "plan", &["day", "weekday", "time", "jobnet"]);
    for (d, list) in days {
        let wd = t.tr(&format!("csv.wd{}", d.weekday().num_days_from_monday())).to_owned();
        if list.is_empty() {
            s.row([day(*d), wd.clone().into()]);
        }
        for (at, name) in list {
            s.row([day(*d), wd.clone().into(), at.format("%H:%M").to_string().into(), name.clone().into()]);
        }
    }
    s
}

/// `table` to the file `to` (Excel when it ends in `.xlsx`, else CSV), or as CSV to standard
/// output when it is `-`. It blocks: the window calls it off the UI thread.
pub fn write(to: &str, table: &ito_table::Table) -> Result<(), String> {
    if to == "-" {
        use std::io::Write;
        let text = table.csv_string();
        return std::io::stdout().write_all(text.trim_start_matches('\u{feff}').as_bytes()).map_err(|e| e.to_string());
    }
    let path = Path::new(to);
    let xlsx = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xlsx"));
    if xlsx { table.xlsx(path) } else { table.csv(path) }.map_err(|e| format!("{to}: 書けない: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(s: &ito_table::Table) -> String {
        s.csv_string().trim_start_matches('\u{feff}').to_owned()
    }

    #[test]
    fn a_plan_is_one_row_per_start_and_an_empty_day_keeps_its_date() {
        let (t, _) = crate::i18n::table("ja");
        let d = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let start = d.and_hms_opt(2, 0, 0).unwrap();
        let s = plan(&t, &[(d, vec![(start, "月次, 締め".into())]), (d.succ_opt().unwrap(), vec![])]);
        assert_eq!(csv(&s), "日付,曜日,時刻,ジョブネット\r\n2026-11-01,日,02:00,\"月次, 締め\"\r\n2026-11-02,月,,\r\n");
        let (en, _) = crate::i18n::table("en");
        assert!(csv(&plan(&en, &[])).starts_with("Day,Weekday,Time,Jobnet\r\n"));
        assert!(s.xlsx_bytes().is_ok());
    }

    #[test]
    fn a_run_is_one_row_with_its_failed_jobs() {
        use koyomi_core::jobnet::{Job, Jobnet};
        let job = |id: &str| Job { id: id.into(), run: vec!["x".into()], ..Default::default() };
        let net = Jobnet { jobs: vec![job("a"), job("=b")], ..Default::default() };
        let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let mut r = Run::new("20261030-020000-01", "g/n", "n", "2026-10-30", How::Schedule, &net, at("2026-10-30 02:00:00"));
        r.ended = Some(at("2026-10-30 02:01:05"));
        r.state = koyomi_core::run::State::Error;
        r.jobs[1].state = koyomi_core::run::State::Error;
        let (t, _) = crate::i18n::table("ja");
        let s = runs(&t, &[r]);
        let row = csv(&s).lines().nth(1).unwrap().to_owned();
        assert_eq!(row, "20261030-020000-01,g/n,n,2026-10-30,スケジュール,,2026-10-30 02:00:00,2026-10-30 02:01:05,65,異常終了,,'=b");
        assert!(s.xlsx_bytes().is_ok());
    }

    #[test]
    fn a_moved_file_is_one_row_with_its_hash_and_check() {
        use koyomi_core::jobnet::{Job, Jobnet};
        use koyomi_core::run::Moved;
        let job = |id: &str| Job { id: id.into(), run: vec!["x".into()], ..Default::default() };
        let net = Jobnet { jobs: vec![job("send"), job("other")], ..Default::default() };
        let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let mut r = Run::new("20261030-020000-01", "g/n", "n", "2026-10-30", How::Schedule, &net, at("2026-10-30 02:00:00"));
        r.jobs[0].ended = Some(at("2026-10-30 02:00:09"));
        r.jobs[0].moved = vec![
            Moved { name: "売上 1.csv".into(), bytes: 3, sha256: Some("ab".repeat(32)), matched: Some(true) },
            Moved { name: "s3://b/x.csv".into(), bytes: 0, sha256: None, matched: Some(false) },
            Moved { name: "plain.csv".into(), bytes: 7, sha256: None, matched: None },
        ];
        let (t, _) = crate::i18n::table("ja");
        let s = files(&t, &[r]);
        let text = csv(&s);
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows[0], "実行,ジョブネット,ジョブ,終了,ファイル,バイト,SHA-256,照合");
        assert_eq!(rows[1], format!("20261030-020000-01,g/n,send,2026-10-30 02:00:09,売上 1.csv,3,{},合っていた", "ab".repeat(32)));
        assert_eq!(rows[2], "20261030-020000-01,g/n,send,2026-10-30 02:00:09,s3://b/x.csv,0,,違っていた");
        assert_eq!(rows[3], "20261030-020000-01,g/n,send,2026-10-30 02:00:09,plain.csv,7,,");
        assert_eq!(rows.len(), 4, "the job that moved nothing has no row");
        assert!(s.xlsx_bytes().is_ok());
    }
}
