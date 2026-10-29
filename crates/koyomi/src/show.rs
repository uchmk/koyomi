//! How a run and its jobs read on the console and on the screen: the state's
//! symbol and word, the times, the duration. The console and the window say
//! the same thing in the same words.

use chrono::{Datelike, NaiveDateTime};
use ito_i18n::Table;
use koyomi_core::calendar::{Calendar, Day};
use koyomi_core::jobnet::Part;
use koyomi_core::run::{JobRun, Run, State};
use koyomi_core::when::{self, When};

/// The state's word: `正常終了`, `実行中` …
pub fn word(t: &Table, s: State) -> String {
    t.tr(&format!("state.{}", s.key())).to_owned()
}

/// Symbol and word: `✓ 正常終了`. The symbol never stands alone.
pub fn tag(t: &Table, s: State) -> String {
    format!("{} {}", s.mark(), word(t, s))
}

pub fn hms(at: NaiveDateTime) -> String {
    at.format("%H:%M:%S").to_string()
}

/// `h:mm:ss`.
pub fn duration(d: chrono::Duration) -> String {
    let s = d.num_seconds().max(0);
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// `09:30:00–09:31:02` or `09:30:00–` while it goes on.
pub fn span(started: Option<NaiveDateTime>, ended: Option<NaiveDateTime>) -> String {
    match (started, ended) {
        (Some(s), Some(e)) => format!("{}–{}", hms(s), hms(e)),
        (Some(s), None) => format!("{}–", hms(s)),
        (None, _) => String::new(),
    }
}

/// One line for the run: tag, id, net, business day, times, duration.
pub fn run_line(t: &Table, r: &Run) -> String {
    let mut line = format!("{}  {}  {}  {}", tag(t, r.state), r.id, r.title, r.date);
    if r.not_started() {
        if let Some(p) = r.planned {
            line.push_str(&format!("  {}", hms(p)));
        }
    } else {
        line.push_str(&format!("  {}", span(Some(r.started), r.ended)));
        if let Some(d) = r.duration() {
            line.push_str(&format!("  {}", duration(d)));
        }
    }
    if !r.reason.is_empty() {
        line.push_str(&format!("  {}", r.reason));
    }
    line
}

/// `2026-11-02（月）02:00`: a day, its weekday in the table's words, a time.
pub fn when_row(t: &Table, at: NaiveDateTime) -> String {
    let day = at.format("%Y-%m-%d").to_string();
    let wd = t.tr(&format!("weekday.{}", at.weekday().to_string().to_lowercase())).to_owned();
    t.trf("run.when_row", &[&day, &wd, &at.format("%H:%M").to_string()])
}

/// An event `when` as a row (`ファイルが来たとき: D:\受信\*.csv`); `None` for a time.
pub fn event_row(t: &Table, w: &When) -> Option<String> {
    match &w.kind {
        when::Kind::File { file, settle: None } => Some(t.trf("run.event_file", &[file])),
        when::Kind::File { file, settle: Some(s) } => Some(t.trf("run.event_file_settle", &[file, &koyomi_core::duration::text(*s)])),
        when::Kind::OnStart => Some(t.tr("run.event_start").to_owned()),
        when::Kind::AfterJobnet { jobnet, on } => {
            Some(t.trf("run.event_after", &[jobnet.as_str(), t.tr(&format!("run.after_{}", on.name()))]))
        }
        _ => None,
    }
}

/// The events `whens` start by, then the next `n` times they come after `now`,
/// one row each: what the window and `koyomi next` both show, from the same
/// calculation the engine starts by. With a calendar, a day past its holiday
/// table is marked: the engine will not run it.
pub fn next_rows(t: &Table, whens: &[When], cal: Option<&Calendar>, now: NaiveDateTime, n: usize) -> Vec<String> {
    let mut rows: Vec<String> = whens.iter().filter_map(|w| event_row(t, w)).collect();
    rows.extend(when::next_times_in(whens, now, n, cal).into_iter().map(|at| {
        let row = when_row(t, at);
        if cal.is_some_and(|c| c.stale(at.date())) {
            format!("{row}  {}", t.tr("calendar.stale"))
        } else {
            row
        }
    }));
    rows
}

/// The days of a month, in order, with what each is under `cal`.
pub fn month_days(cal: &Calendar, year: i32, month: u32) -> Vec<(chrono::NaiveDate, Day)> {
    let mut out = Vec::new();
    let Some(mut d) = chrono::NaiveDate::from_ymd_opt(year, month, 1) else { return out };
    while d.month() == month {
        out.push((d, cal.day(d)));
        let Some(n) = d.succ_opt() else { break };
        d = n;
    }
    out
}

/// `会社 2026-11（営業日 19 日）`: the month's heading.
pub fn month_head(t: &Table, cal: &Calendar, year: i32, month: u32) -> String {
    let business = month_days(cal, year, month).iter().filter(|(_, d)| d.is_business()).count();
    t.trf("calendar.month", &[&cal.name, &format!("{year}-{month:02}"), &business.to_string()])
}

/// The day's name for the list under the month: what was written, the holiday's, or the kind.
pub fn day_name(t: &Table, day: &Day) -> Option<String> {
    match day {
        Day::Business | Day::WeeklyOff => None,
        Day::Holiday(n) => Some((*n).to_owned()),
        Day::Open(n) => Some(if n.is_empty() { t.tr("calendar.open").to_owned() } else { n.clone() }),
        Day::Closed(n) => Some(if n.is_empty() { t.tr("calendar.closed").to_owned() } else { n.clone() }),
    }
}

/// `祝日の表: 2027-12-31 まで` when the calendar uses it; marked when `today` is past it.
pub fn until_line(t: &Table, cal: &Calendar, today: chrono::NaiveDate) -> Option<String> {
    let until = cal.until()?;
    let line = t.trf("calendar.until", &[&until.to_string()]);
    Some(if cal.stale(today) { format!("{line}  {}", t.tr("calendar.stale")) } else { line })
}

/// What `koyomi calendar` prints: the heading, a week per row (a closed day
/// in `( )`, an `open` day in `[ ]`), the named days, the holiday table's end.
pub fn month_lines(t: &Table, cal: &Calendar, year: i32, month: u32, today: chrono::NaiveDate) -> Vec<String> {
    let mut out = vec![month_head(t, cal, year, month)];
    let days = month_days(cal, year, month);
    let wd = |w: chrono::Weekday| t.tr(&format!("weekday.{}", w.to_string().to_lowercase())).to_owned();
    let week = [
        chrono::Weekday::Mon,
        chrono::Weekday::Tue,
        chrono::Weekday::Wed,
        chrono::Weekday::Thu,
        chrono::Weekday::Fri,
        chrono::Weekday::Sat,
        chrono::Weekday::Sun,
    ];
    out.push(week.iter().map(|w| format!("{:>4}", wd(*w))).collect::<Vec<_>>().join(""));
    let mut row = String::new();
    let Some((first, _)) = days.first() else { return out };
    for _ in 0..first.weekday().num_days_from_monday() {
        row.push_str("    ");
    }
    for (d, day) in &days {
        let cell = match day {
            Day::Business => format!(" {:>2} ", d.day()),
            Day::Open(_) => format!("[{:>2}]", d.day()),
            Day::Closed(_) | Day::Holiday(_) | Day::WeeklyOff => format!("({:>2})", d.day()),
        };
        row.push_str(&cell);
        if d.weekday() == chrono::Weekday::Sun {
            out.push(row.trim_end().to_owned());
            row.clear();
        }
    }
    if !row.is_empty() {
        out.push(row.trim_end().to_owned());
    }
    for (d, day) in &days {
        if let Some(n) = day_name(t, day) {
            out.push(format!("{} {n}", d.day()));
        }
    }
    out.extend(until_line(t, cal, today));
    out
}

/// One line for a job: tag, name, times, duration, the reason when there is one.
pub fn job_line(t: &Table, j: &JobRun) -> String {
    let mut line = format!("{}  {}{}", tag(t, j.state), kind(t, &j.part), j.label());
    let times = span(j.started, j.ended);
    if !times.is_empty() {
        line.push_str(&format!("  {times}"));
    }
    if let Some(d) = j.duration() {
        line.push_str(&format!("  {}", duration(d)));
    }
    if !j.reason.is_empty() {
        line.push_str(&format!("  {}", j.reason));
    }
    line
}

/// `【分岐】` and the like before a part's name; nothing before a job's.
pub fn kind(t: &Table, p: &Part) -> String {
    if p.is_job() {
        String::new()
    } else {
        format!("【{}】", t.tr(&format!("part.{}", p.key())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_word_in_both_languages() {
        for code in ["en", "ja"] {
            let (t, _) = crate::i18n::table(code);
            for s in State::ALL {
                let w = word(&t, s);
                assert!(!w.starts_with("state."), "{code}: no word for {}", s.key());
                assert!(!w.contains("逃した") && !w.contains("遅延") && !w.contains("時刻超過"), "{w}");
            }
        }
    }

    #[test]
    fn the_next_times_read_as_day_weekday_time_in_both_languages() {
        let whens = koyomi_core::jobnet::parse(
            "format = 1
[[when]]
every = \"day\"
at = [\"02:00\"]
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap()
        .when;
        let now = NaiveDateTime::parse_from_str("2026-10-30 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let (ja, _) = crate::i18n::table("ja");
        let rows = next_rows(&ja, &whens, None, now, 3);
        assert_eq!(rows, vec!["2026-10-31（土）02:00", "2026-11-01（日）02:00", "2026-11-02（月）02:00"]);
        let (en, _) = crate::i18n::table("en");
        assert_eq!(next_rows(&en, &whens, None, now, 1), vec!["2026-10-31 (Sat) 02:00"]);
        assert!(next_rows(&en, &[], None, now, 10).is_empty());
    }

    /// with a calendar the closed days are skipped (`on_closed = "skip"`), and a day past the holiday table is marked.
    #[test]
    fn a_calendar_skips_closed_days_and_marks_days_past_its_table() {
        let whens = koyomi_core::jobnet::parse(
            "format = 1
calendar = \"会社\"
[[when]]
every = \"day\"
at = [\"02:00\"]
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap()
        .when;
        let cal = Calendar::parse("会社", "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\n").unwrap();
        let (ja, _) = crate::i18n::table("ja");
        let now = NaiveDateTime::parse_from_str("2026-10-30 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let rows = next_rows(&ja, &whens, Some(&cal), now, 2);
        assert_eq!(rows, vec!["2026-11-02（月）02:00", "2026-11-04（水）02:00"], "the weekend and 文化の日 are skipped");
        let late = NaiveDateTime::parse_from_str("2027-12-30 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let rows = next_rows(&ja, &whens, Some(&cal), late, 2);
        assert_eq!(rows, vec!["2027-12-31（金）02:00", "2028-01-03（月）02:00  祝日の表が古い"]);
    }

    #[test]
    fn a_month_reads_as_weeks_with_closed_days_in_brackets() {
        let cal = Calendar::parse(
            "会社",
            "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [{ date = 2026-11-20, name = \"創立記念日\" }]\nopen = [2026-11-28]\n",
        )
        .unwrap();
        let (ja, _) = crate::i18n::table("ja");
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let lines = month_lines(&ja, &cal, 2026, 11, today);
        assert_eq!(lines[0], "会社 2026-11（営業日 19 日）");
        assert_eq!(lines[1], "   月   火   水   木   金   土   日");
        assert_eq!(lines[2], "                        ( 1)");
        assert_eq!(lines[3], "  2 ( 3)  4   5   6 ( 7)( 8)");
        assert_eq!(lines[5], " 16  17  18  19 (20)(21)(22)");
        assert_eq!(lines[6], "(23) 24  25  26  27 [28](29)");
        assert_eq!(lines[7], " 30");
        assert_eq!(&lines[8..], ["3 文化の日", "20 創立記念日", "23 勤労感謝の日", "28 休日出勤", "祝日の表: 2027-12-31 まで"]);
        let (en, _) = crate::i18n::table("en");
        let late = chrono::NaiveDate::from_ymd_opt(2028, 1, 5).unwrap();
        assert_eq!(
            month_lines(&en, &cal, 2026, 11, late).last().map(String::as_str),
            Some("Holiday table: until 2027-12-31  holiday table out of date")
        );
    }

    #[test]
    fn a_duration_reads_as_h_mm_ss() {
        assert_eq!(duration(chrono::Duration::seconds(3723)), "1:02:03");
        assert_eq!(duration(chrono::Duration::seconds(5)), "0:00:05");
    }
}
