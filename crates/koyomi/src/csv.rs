//! `--csv <file>`: what `list`, `history` and `plan` print, as a table for a spreadsheet.
//!
//! RFC 4180: CRLF ends each row; a field with a comma, a quote or a line end is quoted, its
//! quotes doubled. A file starts with the UTF-8 BOM so Excel on a Japanese Windows reads it as
//! UTF-8; `-` (standard output) gets no BOM. The heads and words are the language table's.

use std::borrow::Cow;

use ito_i18n::Table;
use koyomi_core::run::{How, Run};
use koyomi_core::scan::Def;

/// Times as a spreadsheet reads them.
const TIME: &str = "%Y-%m-%d %H:%M:%S";

/// `s` as one field.
fn field(s: &str) -> Cow<'_, str> {
    if s.contains([',', '"', '\r', '\n']) {
        Cow::Owned(format!("\"{}\"", s.replace('"', "\"\"")))
    } else {
        Cow::Borrowed(s)
    }
}

/// The table: a row of `heads` (keys of `[csv]`), then `rows`.
fn text(t: &Table, heads: &[&str], rows: &[Vec<String>]) -> String {
    let mut s = String::new();
    let head: Vec<String> = heads.iter().map(|h| t.tr(&format!("csv.{h}")).to_owned()).collect();
    for row in std::iter::once(&head).chain(rows) {
        let fields: Vec<Cow<str>> = row.iter().map(|f| field(f)).collect();
        s.push_str(&fields.join(","));
        s.push_str("\r\n");
    }
    s
}

/// One row per job of every definition that reads (a definition that does not: one row, no job).
pub fn jobs(t: &Table, defs: &[Def]) -> String {
    let mut rows = Vec::new();
    for d in defs {
        let net = |job: [String; 5]| {
            let [id, name, kind, run, after] = job;
            vec![
                d.group.clone(),
                d.id.clone(),
                d.label().to_owned(),
                crate::review::word(t, d.standing),
                d.calendar.clone().unwrap_or_default(),
                id,
                name,
                kind,
                run,
                after,
            ]
        };
        let Some(n) = d.net.as_ref().filter(|n| !n.jobs.is_empty()) else {
            rows.push(net(Default::default()));
            continue;
        };
        for j in &n.jobs {
            let run = match (&j.shell, &j.script) {
                (Some(sh), Some(sc)) => format!("{sh}: {sc}"),
                _ => j.run.join(" "),
            };
            let after: Vec<&str> = j.after.iter().map(|l| l.job.as_str()).collect();
            let kind = t.tr(&format!("part.{}", j.part.key())).to_owned();
            rows.push(net([j.id.clone(), j.name.clone().unwrap_or_default(), kind, run, after.join(" ")]));
        }
    }
    text(t, &["group", "jobnet", "title", "standing", "calendar", "job", "job_name", "kind", "run", "after"], &rows)
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
    }
}

/// One row per run, oldest first as given; the jobs that ended 異常 by id.
pub fn runs(t: &Table, rs: &[Run]) -> String {
    let rows: Vec<Vec<String>> = rs
        .iter()
        .map(|r| {
            let at = |d: Option<chrono::NaiveDateTime>| d.map(|d| d.format(TIME).to_string()).unwrap_or_default();
            let failed: Vec<&str> = r.jobs.iter().filter(|j| j.state.is_error()).map(|j| j.id.as_str()).collect();
            vec![
                r.id.clone(),
                r.jobnet.clone(),
                r.title.clone(),
                r.date.clone(),
                t.tr(&format!("csv.{}", how(r.how))).to_owned(),
                at(r.planned),
                at(Some(r.started)),
                at(r.ended),
                r.duration().map(|d| d.num_seconds().to_string()).unwrap_or_default(),
                t.tr(&format!("state.{}", r.state.key())).to_owned(),
                r.reason.clone(),
                failed.join(" "),
            ]
        })
        .collect();
    text(t, &["run_id", "jobnet", "title", "date", "how", "planned", "started", "ended", "seconds", "state", "reason", "failed"], &rows)
}

/// One row per start; a day with none as one row with only its date and weekday.
pub fn plan(t: &Table, days: &[crate::plan::Day]) -> String {
    use chrono::Datelike;
    let mut rows = Vec::new();
    for (d, list) in days {
        let wd = t.tr(&format!("csv.wd{}", d.weekday().num_days_from_monday())).to_owned();
        if list.is_empty() {
            rows.push(vec![d.to_string(), wd.clone(), String::new(), String::new()]);
        }
        for (at, name) in list {
            rows.push(vec![d.to_string(), wd.clone(), at.format("%H:%M").to_string(), name.clone()]);
        }
    }
    text(t, &["day", "weekday", "time", "jobnet"], &rows)
}

/// `text` to the file `to` (BOM first), or to standard output when it is `-`.
pub fn write(to: &str, text: &str) -> Result<(), String> {
    if to == "-" {
        use std::io::Write;
        return std::io::stdout().write_all(text.as_bytes()).map_err(|e| e.to_string());
    }
    let mut bytes = "\u{feff}".as_bytes().to_vec();
    bytes.extend_from_slice(text.as_bytes());
    std::fs::write(to, bytes).map_err(|e| format!("{to}: 書けない: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn fields_with_a_comma_quote_or_line_end_are_quoted_and_rows_end_in_crlf() {
        assert_eq!(field("a"), "a");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(field("1\n2"), "\"1\n2\"");
        let (t, _) = crate::i18n::table("ja");
        let d = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let at = d.and_hms_opt(2, 0, 0).unwrap();
        let s = plan(&t, &[(d, vec![(at, "月次, 締め".into())]), (d.succ_opt().unwrap(), vec![])]);
        assert_eq!(s, "日付,曜日,時刻,ジョブネット\r\n2026-11-01,日,02:00,\"月次, 締め\"\r\n2026-11-02,月,,\r\n");
        let (en, _) = crate::i18n::table("en");
        assert!(plan(&en, &[]).starts_with("Day,Weekday,Time,Jobnet\r\n"));
    }

    #[test]
    fn a_run_is_one_row_with_its_failed_jobs() {
        use koyomi_core::jobnet::{Job, Jobnet};
        let job = |id: &str| Job { id: id.into(), run: vec!["x".into()], ..Default::default() };
        let net = Jobnet { jobs: vec![job("a"), job("b")], ..Default::default() };
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let mut r = Run::new("20261030-020000-01", "g/n", "n", "2026-10-30", How::Schedule, &net, at("2026-10-30 02:00:00"));
        r.ended = Some(at("2026-10-30 02:01:05"));
        r.state = koyomi_core::run::State::Error;
        r.jobs[1].state = koyomi_core::run::State::Error;
        let (t, _) = crate::i18n::table("ja");
        let s = runs(&t, &[r]);
        let row = s.lines().nth(1).unwrap();
        assert_eq!(row, "20261030-020000-01,g/n,n,2026-10-30,スケジュール,,2026-10-30 02:00:00,2026-10-30 02:01:05,65,異常終了,,b");
    }
}
