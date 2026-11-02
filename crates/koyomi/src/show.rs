//! How a run and its jobs read on the console and on the screen: the state's
//! symbol and word, the times, the duration. The console and the window say
//! the same thing in the same words.

use chrono::{Datelike, NaiveDateTime};
use ito_i18n::Table;
use koyomi_core::blackout::Blackout;
use koyomi_core::calendar::{Calendar, Day};
use koyomi_core::jobnet::Part;
use koyomi_core::quiet::{OnQuiet, Span};
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

/// `41分`, `1時間5分`, `12秒` (`41 min` … in English): the two largest units.
pub fn duration(t: &Table, d: chrono::Duration) -> String {
    ito_common::text::duration_words(d.to_std().unwrap_or_default(), t.tr("app.lang"))
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
    format!("{}  {}  {}  {}{}", tag(t, r.state), r.id, r.title, r.date, run_tail(t, r))
}

/// The run's heading in two parts: the state and the name to set big (`✓ 正常終了  月次締め`),
/// and the rest for one small line under it (`#12  2026-10-30  09:30:00–09:31:02  1分2秒`).
pub fn run_head(t: &Table, r: &Run) -> (String, String) {
    (format!("{}  {}", tag(t, r.state), r.title), format!("{}  {}{}", r.id, r.date, run_tail(t, r)))
}

/// The time, how long it took, the delays and the reason, each led by two spaces.
fn run_tail(t: &Table, r: &Run) -> String {
    let mut out = String::new();
    if r.not_started() {
        if let Some(p) = r.planned {
            out.push_str(&format!("  {}", hms(p)));
        }
    } else {
        out.push_str(&format!("  {}", span(Some(r.started), r.ended)));
        if let Some(d) = r.duration() {
            out.push_str(&format!("  {}", duration(t, d)));
        }
    }
    for d in &r.delays {
        out.push_str(&format!("  ⏰{}", t.tr(d.key())));
    }
    if !r.reason.is_empty() {
        out.push_str(&format!("  {}", koyomi_core::run::one_line(&r.reason)));
    }
    out
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
        when::Kind::Log { log, contains } => Some(t.trf("run.event_log", &[log, contains])),
        when::Kind::EventLog { eventlog, ids, source } => {
            let mut what: Vec<String> = Vec::new();
            if !ids.is_empty() {
                what.push(format!("ID {}", ids.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")));
            }
            what.extend(source.iter().cloned());
            Some(t.trf("run.event_eventlog", &[eventlog, &what.join(" / ")]))
        }
        when::Kind::Mail { mail, sender, subject, .. } => {
            let what: Vec<String> = [("from", sender), ("subject", subject)]
                .into_iter()
                .filter_map(|(k, v)| v.as_ref().map(|v| format!("{}「{v}」", t.tr(&format!("run.mail_{k}")))))
                .collect();
            Some(t.trf("run.event_mail", &[mail, &what.join(" / ")]))
        }
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
///
/// With `sleep`, a time that comes after this PC would have gone to sleep is
/// marked too: asleep, woken for it (`wake = true`), or why it cannot be woken.
/// A time in a planned stop (`stops`) is marked with it instead: it will not start.
#[allow(clippy::too_many_arguments)]
pub fn next_rows(
    t: &Table,
    whens: &[When],
    quiet: (&[Span], OnQuiet),
    cal: Option<&Calendar>,
    now: NaiveDateTime,
    n: usize,
    sleep: Option<&Sleep>,
    stops: &[Blackout],
) -> Vec<String> {
    let mut rows: Vec<String> = whens.iter().filter_map(|w| event_row(t, w)).collect();
    rows.extend(koyomi_core::quiet::next_starts(whens, quiet.0, quiet.1, now, n, cal).into_iter().map(|at| {
        let mut row = when_row(t, at);
        if let Some(b) = koyomi_core::blackout::find(stops, at) {
            return format!("{row}  {}", stop_mark(t, b));
        }
        if cal.is_some_and(|c| c.stale(at.date())) {
            row = format!("{row}  {}", t.tr("calendar.stale"));
        }
        if let Some(mark) = sleep.and_then(|s| sleep_mark(t, s, at, now)) {
            row = format!("{row}  {mark}");
        }
        row
    }));
    rows
}

/// What a planned stop is for, or that nothing is written.
pub fn stop_why<'a>(t: &'a Table, b: &'a Blackout) -> &'a str {
    match b.why.trim() {
        "" => t.tr("blackout.none"),
        why => why,
    }
}

/// The mark for a time in a planned stop: `⊘ 計画停止（年末年始）`.
pub fn stop_mark(t: &Table, b: &Blackout) -> String {
    t.trf("blackout.mark", &[stop_why(t, b)])
}

/// What the power settings mean for a net's times: how long this PC idles
/// before it sleeps, whether the net wakes it, and why it cannot (a
/// `power.why_*` key's tail), all from the engine's [`crate::proto::Power`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sleep {
    /// Seconds; `None`: the PC never sleeps (nothing is marked).
    pub after: Option<u64>,
    pub wake: bool,
    pub cannot: Option<String>,
}

impl Sleep {
    pub fn of(power: &crate::proto::Power, wake: bool) -> Sleep {
        Sleep { after: power.sleep_after, wake, cannot: power.cannot_wake.clone() }
    }
}

/// `30分` / `2時間` / `1時間5分`, rounded up to whole minutes.
pub fn idle(t: &Table, secs: u64) -> String {
    let m = secs.div_ceil(60).max(1);
    ito_common::text::duration_words(std::time::Duration::from_secs(m * 60), t.tr("app.lang"))
}

/// The mark for a time `at` that comes later than the PC would stay awake from `now`.
pub fn sleep_mark(t: &Table, s: &Sleep, at: NaiveDateTime, now: NaiveDateTime) -> Option<String> {
    let after = s.after?;
    if at <= now + chrono::Duration::seconds(i64::try_from(after).unwrap_or(i64::MAX / 2)) {
        return None;
    }
    Some(match (&s.cannot, s.wake) {
        (_, false) => t.trf("power.asleep", &[&idle(t, after)]),
        (Some(why), true) => t.trf("power.cannot", &[t.tr(&format!("power.why_{why}"))]),
        (None, true) => t.tr("power.wakes").to_owned(),
    })
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
        Day::Holiday(n) => Some(n.clone()),
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
        line.push_str(&format!("  {}", duration(t, d)));
    }
    if !j.reason.is_empty() {
        line.push_str(&format!("  {}", koyomi_core::run::one_line(&koyomi_core::run::tidy_reason(&j.reason))));
    }
    line
}

/// The lines of `log` that match `query` (smart-case regex), and how many of how many.
/// `None` for an empty query: the log as it is.
pub fn filter_log(log: &str, query: &str) -> Result<Option<(String, usize, usize)>, String> {
    if query.is_empty() {
        return Ok(None);
    }
    let m = ito_match::Matcher::new(query)?;
    let all = log.lines().count();
    let hits: Vec<&str> = log.lines().filter(|l| m.is_match(l)).collect();
    Ok(Some((
        hits.join(
            "
",
        ),
        hits.len(),
        all,
    )))
}

/// The picked job's facts, one `label: value` per line: start, end, took, exit code, retries, command.
pub fn job_facts(t: &Table, j: &JobRun) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut add = |k: &str, v: String| rows.push((t.tr(k).to_owned(), v));
    add("run.state", tag(t, j.state));
    if let Some(s) = j.started {
        add("run.start", s.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    if let Some(e) = j.ended {
        add("run.end", e.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    if let Some(d) = j.duration() {
        add("run.took", duration(t, d));
    }
    if let Some(x) = j.exit {
        add("run.exit", x.to_string());
    }
    if j.retried > 0 || j.retry.is_some() {
        let max = j.retry.map_or(0, |r| r.count);
        add("run.retried", format!("{} / {max}", j.retried));
    }
    if !j.reason.is_empty() {
        add("run.reason", koyomi_core::run::tidy_reason(&j.reason));
    }
    if j.part.runs_process() {
        // The script as written (not the encoded form it is run with).
        let cmd = match (&j.shell, &j.script) {
            (Some(shell), Some(script)) => format!("{shell}: {script}"),
            _ => j.run.join(" "),
        };
        if !cmd.trim().is_empty() {
            add("run.command", cmd);
        }
    }
    if !j.env.is_empty() {
        // A `secret:<name>` value is a reference to the credential store; it is never shown.
        let env: Vec<String> = j.env.iter().map(|(k, v)| format!("{k}={}", if v.starts_with("secret:") { "***" } else { v })).collect();
        add("run.env", env.join("  "));
    }
    rows
}

/// `【分岐】` and the like before a part's name; nothing before a job's.
pub fn kind(t: &Table, p: &Part) -> String {
    if p.is_job() {
        String::new()
    } else {
        part_name(t, p)
    }
}

/// `【ジョブ】`, `【分岐】` …: what a part is, a job included (the shelf and the pane's heading).
pub fn part_name(t: &Table, p: &Part) -> String {
    format!("【{}】", t.tr(&format!("part.{}", p.key())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_search_keeps_matching_lines_with_smart_case() {
        let log = "start
ERROR disk
error low
end";
        assert_eq!(filter_log(log, "").unwrap(), None);
        let (text, n, all) = filter_log(log, "error").unwrap().unwrap();
        assert_eq!(
            (text.as_str(), n, all),
            (
                "ERROR disk
error low",
                2,
                4
            )
        );
        let (text, n, _) = filter_log(log, "ERROR").unwrap().unwrap();
        assert_eq!((text.as_str(), n), ("ERROR disk", 1));
        assert!(filter_log(log, "(").is_err());
    }

    #[test]
    fn job_facts_list_exit_code_and_retries() {
        let (t, _) = crate::i18n::table("en");
        let mut j =
            JobRun::from_job(&koyomi_core::jobnet::Job { id: "a".into(), run: vec!["x.exe".into(), "-v".into()], ..Default::default() });
        j.state = State::Error;
        j.exit = Some(3);
        j.retried = 2;
        j.retry = Some(koyomi_core::jobnet::Retry { count: 3, wait: 5 });
        let rows = job_facts(&t, &j);
        let get = |k: &str| rows.iter().find(|r| r.0 == t.tr(k)).map(|r| r.1.clone());
        assert_eq!(get("run.exit").as_deref(), Some("3"));
        assert_eq!(get("run.retried").as_deref(), Some("2 / 3"));
        assert_eq!(get("run.command").as_deref(), Some("x.exe -v"));
        assert_eq!(get("run.start"), None);
    }

    #[test]
    fn job_facts_show_the_script_and_hide_secret_env() {
        let (t, _) = crate::i18n::table("en");
        let job = koyomi_core::jobnet::Job {
            id: "a".into(),
            shell: Some("powershell".into()),
            script: Some("Get-Date".into()),
            env: [("DB_PASS".to_owned(), "secret:db".to_owned()), ("MODE".to_owned(), "full".to_owned())].into(),
            ..Default::default()
        };
        let rows = job_facts(&t, &JobRun::from_job(&job));
        let get = |k: &str| rows.iter().find(|r| r.0 == t.tr(k)).map(|r| r.1.clone());
        assert_eq!(get("run.command").as_deref(), Some("powershell: Get-Date"));
        let env = get("run.env").unwrap();
        assert!(env.contains("DB_PASS=***") && env.contains("MODE=full") && !env.contains("secret:db"), "{env}");
    }

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
        let rows = next_rows(&ja, &whens, (&[], OnQuiet::Wait), None, now, 3, None, &[]);
        assert_eq!(rows, vec!["2026-10-31（土）02:00", "2026-11-01（日）02:00", "2026-11-02（月）02:00"]);
        let (en, _) = crate::i18n::table("en");
        assert_eq!(next_rows(&en, &whens, (&[], OnQuiet::Wait), None, now, 1, None, &[]), vec!["2026-10-31 (Sat) 02:00"]);
        assert!(next_rows(&en, &[], (&[], OnQuiet::Wait), None, now, 10, None, &[]).is_empty());
        // A time in a planned stop is marked with it, and nothing else.
        let stop = [Blackout { from: "2026-11-01".into(), to: "2026-11-01".into(), why: "点検".into() }];
        let rows = next_rows(&ja, &whens, (&[], OnQuiet::Wait), None, now, 2, None, &stop);
        assert_eq!(rows, vec!["2026-10-31（土）02:00", "2026-11-01（日）02:00  ⊘ 計画停止（点検）"]);
        let stop = [Blackout { why: String::new(), ..stop[0].clone() }];
        assert_eq!(
            next_rows(&en, &whens, (&[], OnQuiet::Wait), None, now, 2, None, &stop)[1],
            "2026-11-01 (Sun) 02:00  ⊘ planned stop (no reason given)"
        );
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
        let rows = next_rows(&ja, &whens, (&[], OnQuiet::Wait), Some(&cal), now, 2, None, &[]);
        assert_eq!(rows, vec!["2026-11-02（月）02:00", "2026-11-04（水）02:00"], "the weekend and 文化の日 are skipped");
        let late = NaiveDateTime::parse_from_str("2027-12-30 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let rows = next_rows(&ja, &whens, (&[], OnQuiet::Wait), Some(&cal), late, 2, None, &[]);
        assert_eq!(rows, vec!["2027-12-31（金）02:00", "2028-01-03（月）02:00  祝日の表が古い"]);
    }

    /// a time later than the PC stays awake is marked: asleep, woken, or why it cannot be.
    #[test]
    fn times_after_the_pc_sleeps_are_marked() {
        let whens = koyomi_core::jobnet::parse(
            "format = 1
[[when]]
every = \"day\"
at = [\"09:10\", \"10:00\"]
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap()
        .when;
        let (ja, _) = crate::i18n::table("ja");
        let now = NaiveDateTime::parse_from_str("2026-10-30 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let mut s = Sleep { after: Some(1800), wake: false, cannot: None };
        let rows = next_rows(&ja, &whens, (&[], OnQuiet::Wait), None, now, 2, Some(&s), &[]);
        assert_eq!(rows[0], "2026-10-30（金）09:10", "within 30 minutes: awake still");
        assert_eq!(rows[1], "2026-10-30（金）10:00  この PC は 30分で眠る設定。起こさなければ動かない");
        s.wake = true;
        assert!(next_rows(&ja, &whens, (&[], OnQuiet::Wait), None, now, 2, Some(&s), &[])[1].ends_with("PC を起こして流す"));
        s.cannot = Some("battery".into());
        let row = &next_rows(&ja, &whens, (&[], OnQuiet::Wait), None, now, 2, Some(&s), &[])[1];
        assert!(row.contains("起こせない") && row.contains("バッテリー"), "{row}");
        s.after = None;
        assert_eq!(
            next_rows(&ja, &whens, (&[], OnQuiet::Wait), None, now, 2, Some(&s), &[])[1],
            "2026-10-30（金）10:00",
            "never sleeps: no mark"
        );
        assert_eq!(idle(&ja, 7200), "2時間");
        assert_eq!(idle(&ja, 3870), "1時間5分");
        let (en, _) = crate::i18n::table("en");
        for why in ["timers_off", "battery", "unsupported", "failed"] {
            let key = format!("power.why_{why}");
            assert_ne!(ja.tr(&key), key);
            assert_ne!(en.tr(&key), key);
        }
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
    fn a_duration_reads_in_words() {
        let (ja, _) = crate::i18n::table("ja");
        let (en, _) = crate::i18n::table("en");
        assert_eq!(duration(&ja, chrono::Duration::seconds(3723)), "1時間2分");
        assert_eq!(duration(&ja, chrono::Duration::seconds(2460)), "41分");
        assert_eq!(duration(&ja, chrono::Duration::seconds(12)), "12秒");
        assert_eq!(duration(&ja, chrono::Duration::seconds(-3)), "0秒");
        assert_eq!(duration(&en, chrono::Duration::seconds(3900)), "1 h 5 min");
    }
}
