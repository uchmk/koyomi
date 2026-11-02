//! `[[when]]`: the times a net starts on its own, and the sums on them: the
//! times of one day, the next ones after a moment, and the ones that came
//! due between two looks at the clock.
//!
//! The sums are on the wall clock (`NaiveDateTime`), as a person writes
//! them. Only [`due`] touches the zone: a time the clock skipped (the spring
//! change) runs once, at the first time that exists after it; a time the
//! clock shows twice (the autumn change) runs once; a clock set back runs
//! nothing a second time.
//!
//! The reading of the fields is `jobnet`'s; here the shape, the leaf values
//! and the sums.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Days, Duration, MappedLocalTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

use crate::calendar::Calendar;
use crate::run::State;

/// One `[[when]]`: a kind of time, within `start`..=`end`, minus `except`.
/// With a calendar, `on_closed` says what a time on a closed day does;
/// `start`, `end` and `except` are on the day the time runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct When {
    pub kind: Kind,
    #[serde(default)]
    pub except: Vec<NaiveDate>,
    #[serde(default)]
    pub start: Option<NaiveDate>,
    #[serde(default)]
    pub end: Option<NaiveDate>,
    /// `on_closed`; none written: `skip` when the net has a calendar.
    #[serde(default)]
    pub on_closed: Option<OnClosed>,
    /// `wait_for`: a time of this net waits for the end of that net's run on
    /// the same business day (when it is scheduled that day).
    #[serde(default)]
    pub wait_for: Option<String>,
    /// `wait_until`: the time of day after which the wait gives up (abnormal).
    #[serde(default)]
    pub wait_until: Option<NaiveTime>,
}

/// `on_closed`: what a time written on a closed day of the net's calendar does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnClosed {
    /// Runs on the business day before.
    Prev,
    /// Runs on the business day after.
    Next,
    /// Does not run.
    Skip,
    /// Runs on the day as written.
    Run,
}

/// The kinds of time, named as `every` names them (`once` and `dates` have no `every`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "every", rename_all = "snake_case")]
pub enum Kind {
    /// `once = 2026-11-30T21:00:00`.
    Once { at: NaiveDateTime },
    /// `every = "day"`, `at = ["02:00"]`.
    Day { at: Vec<NaiveTime> },
    /// `every = "minutes"`, `n = 15`, `from`..=`to` (inclusive), `days` (empty: every day).
    Minutes { n: u32, from: NaiveTime, to: NaiveTime, days: Vec<Weekday> },
    /// `every = "hours"`, the same fields.
    Hours { n: u32, from: NaiveTime, to: NaiveTime, days: Vec<Weekday> },
    /// `every = "week"`, `days = ["mon", "fri"]`, `at`.
    Week { days: Vec<Weekday>, at: Vec<NaiveTime> },
    /// `every = "month"`, `day = 25` / `day = "last"` / `nth = 2, weekday = "tue"`, `at`.
    Month { rule: MonthRule, at: Vec<NaiveTime> },
    /// `every = "year"`, `date = "04-01"`, `at`. `02-29` runs in leap years only.
    Year { month: u32, day: u32, at: Vec<NaiveTime> },
    /// `dates = [2026-12-30, 2027-01-04]`, `at`.
    Dates { dates: Vec<NaiveDate>, at: Vec<NaiveTime> },
    /// `file = 'D:\受信\*.csv'`, `settle = "10s"`: a file arriving (an event, not a time).
    /// `file` is an absolute path; `*` / `?` only in its name. `settle`: the seconds
    /// the file must stay the same size before it counts as written.
    File {
        file: String,
        #[serde(default)]
        settle: Option<u64>,
    },
    /// `log = 'D:\app\app.log'`, `contains = "ERROR"`: a line with that text
    /// written to the file (an event). Only lines written while the engine
    /// watches count; each poll with such lines starts the net once.
    Log { log: String, contains: String },
    /// `eventlog = "Security"`, `ids = [4625]`, `source = "…"`: a record
    /// written to that Windows event log (an event). `ids` empty: any id of
    /// `source`; one of the two is always written.
    EventLog {
        eventlog: String,
        #[serde(default)]
        ids: Vec<u32>,
        #[serde(default)]
        source: Option<String>,
    },
    /// `mail = "imaps://user@host/INBOX"`, `password = "secret:<name>"`,
    /// `sender = "…"`, `subject = "…"`: a mail with that sender / subject
    /// arriving in the mailbox (an event). Only mail that comes while the
    /// engine watches counts; each poll with such mail starts the net once.
    /// `secret` is the name in ito-secret, never the password itself.
    Mail {
        mail: String,
        secret: String,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        subject: Option<String>,
    },
    /// `on_start = true`: when the engine starts (the PC's start or sign-in).
    OnStart,
    /// `idle = "10m"`: when the PC has been left alone that long (an event), in
    /// seconds. Once each time it is left alone: it starts again only after
    /// someone has used the PC.
    Idle { idle: u64 },
    /// `after_jobnet = "経理/売上取込"`, `on = "ok"`: when a run of that net ends
    /// so (an event). The run takes the business day of the run it follows.
    AfterJobnet {
        jobnet: String,
        #[serde(default)]
        on: AfterOn,
    },
}

impl Kind {
    /// An event (a file arriving, the engine's start, the PC left alone, another
    /// net's end), not a time: it has no next time, and runs when it happens on a day
    /// `start` / `end` / `except` allow.
    pub fn is_event(&self) -> bool {
        matches!(
            self,
            Kind::File { .. }
                | Kind::Log { .. }
                | Kind::EventLog { .. }
                | Kind::Mail { .. }
                | Kind::OnStart
                | Kind::Idle { .. }
                | Kind::AfterJobnet { .. }
        )
    }
}

/// Which end of the run before lets an `after_jobnet` net start: `on = "ok"`
/// (正常終了 or 警告終了), `"error"` (異常終了, 打ち切り, 強制終了) or `"any"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterOn {
    #[default]
    Ok,
    Error,
    Any,
}

impl AfterOn {
    /// Whether a run that ended as `state` lets the net after it start. The
    /// record of a time that did not run (スキップ, 時刻を過ぎた, 止めた, 保留)
    /// is no end of a run: nothing follows it.
    pub fn lets(self, state: State) -> bool {
        match self {
            AfterOn::Ok => matches!(state, State::Ok | State::Warn),
            AfterOn::Error => matches!(state, State::Error | State::Timeout | State::Killed),
            AfterOn::Any => matches!(state, State::Ok | State::Warn | State::Error | State::Timeout | State::Killed),
        }
    }

    /// `on` as it is written.
    pub fn name(self) -> &'static str {
        match self {
            AfterOn::Ok => "ok",
            AfterOn::Error => "error",
            AfterOn::Any => "any",
        }
    }
}

/// Which day of the month. `day = 31` in a shorter month is no day: nothing runs that month.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum MonthRule {
    /// `day = 20`, moved `shift` business days (`shift = -3`: three before; needs a calendar when not 0).
    Day {
        day: u32,
        #[serde(default)]
        shift: i32,
    },
    Last,
    /// The `nth` (1..=5) `weekday` of the month.
    Nth {
        nth: u32,
        weekday: Weekday,
    },
    /// `business_day = 5`: the 5th business day; `-1`: the last. Needs a calendar.
    BusinessDay {
        n: i32,
    },
}

impl MonthRule {
    pub fn matches(&self, date: NaiveDate) -> bool {
        self.matches_in(date, None)
    }

    /// Whether `date` is the rule's day, counting business days by `cal`
    /// (without one, a rule that counts them matches no day).
    pub fn matches_in(&self, date: NaiveDate, cal: Option<&Calendar>) -> bool {
        match self {
            MonthRule::Day { day, shift: 0 } => date.day() == *day,
            MonthRule::Day { day, shift } => cal.is_some_and(|c| {
                months_around(date).any(|(y, m)| NaiveDate::from_ymd_opt(y, m, *day).and_then(|base| c.shift(base, *shift)) == Some(date))
            }),
            MonthRule::Last => date.succ_opt().is_none_or(|n| n.month() != date.month()),
            MonthRule::Nth { nth, weekday } => date.weekday() == *weekday && (date.day() - 1) / 7 + 1 == *nth,
            MonthRule::BusinessDay { n } => cal.is_some_and(|c| c.nth_business_day(date.year(), date.month(), *n) == Some(date)),
        }
    }

    /// Whether the rule counts business days, and so needs a calendar.
    pub fn needs_calendar(&self) -> bool {
        match self {
            MonthRule::Day { shift, .. } => *shift != 0,
            MonthRule::BusinessDay { .. } => true,
            MonthRule::Last | MonthRule::Nth { .. } => false,
        }
    }
}

/// The months a `shift` of up to 31 business days can land in `date`'s from: three either side.
fn months_around(date: NaiveDate) -> impl Iterator<Item = (i32, u32)> {
    let n = date.year() * 12 + date.month() as i32 - 1;
    (n - 3..=n + 3).map(|k| (k.div_euclid(12), (k.rem_euclid(12) + 1) as u32))
}

/// How far [`When::next_after`] looks: past the longest gap a yearly `02-29` can have.
const HORIZON: u64 = 366 * 8 + 7;

impl When {
    /// An event (`file`, `on_start`), not a time.
    pub fn is_event(&self) -> bool {
        self.kind.is_event()
    }

    /// Whether `date` is within `start`..=`end` and not in `except`.
    pub fn runs_on(&self, date: NaiveDate) -> bool {
        self.start.is_none_or(|s| date >= s) && self.end.is_none_or(|e| date <= e) && !self.except.contains(&date)
    }

    /// `on_closed` as it applies with `cal`: `skip` unless written; `run` without a calendar.
    pub fn on_closed_in(&self, cal: Option<&Calendar>) -> OnClosed {
        match cal {
            Some(_) => self.on_closed.unwrap_or(OnClosed::Skip),
            None => OnClosed::Run,
        }
    }

    /// The times the kind writes on `date`: no calendar's move, no `start`/`end`/`except`.
    fn written_on(&self, date: NaiveDate, cal: Option<&Calendar>) -> Vec<NaiveTime> {
        let on = |yes: bool, at: &Vec<NaiveTime>| if yes { at.clone() } else { Vec::new() };
        match &self.kind {
            Kind::Once { at } => on(at.date() == date, &vec![at.time()]),
            Kind::Day { at } => at.clone(),
            Kind::Minutes { n, from, to, days } => steps(date, i64::from(*n), *from, *to, days),
            Kind::Hours { n, from, to, days } => steps(date, i64::from(*n) * 60, *from, *to, days),
            Kind::Week { days, at } => on(days.contains(&date.weekday()), at),
            Kind::Month { rule, at } => on(rule.matches_in(date, cal), at),
            Kind::Year { month, day, at } => on(date.month() == *month && date.day() == *day, at),
            Kind::Dates { dates, at } => on(dates.contains(&date), at),
            Kind::File { .. }
            | Kind::Log { .. }
            | Kind::EventLog { .. }
            | Kind::Mail { .. }
            | Kind::OnStart
            | Kind::Idle { .. }
            | Kind::AfterJobnet { .. } => Vec::new(),
        }
    }

    /// The times on `date` with no calendar, in order, each once.
    pub fn times_on(&self, date: NaiveDate) -> Vec<NaiveTime> {
        self.times_on_in(date, None)
    }

    /// The times that run on `date`, in order, each once: the ones written
    /// on it and, with a calendar, the ones `on_closed` moves onto it from
    /// the closed days beside it.
    pub fn times_on_in(&self, date: NaiveDate, cal: Option<&Calendar>) -> Vec<NaiveTime> {
        if !self.runs_on(date) {
            return Vec::new();
        }
        let mut times = match cal {
            None => self.written_on(date, None),
            Some(c) => c.sources(date, self.on_closed_in(cal)).into_iter().flat_map(|s| self.written_on(s, cal)).collect(),
        };
        times.sort();
        times.dedup();
        times
    }

    /// The first time after `after` (not at it) with no calendar, if any within the horizon.
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        self.next_after_in(after, None)
    }

    /// The first time after `after` (not at it), if any within the horizon.
    pub fn next_after_in(&self, after: NaiveDateTime, cal: Option<&Calendar>) -> Option<NaiveDateTime> {
        if self.is_event() {
            return None;
        }
        if let Kind::Once { at } = &self.kind {
            let day = match cal {
                Some(c) => c.adjust(at.date(), self.on_closed_in(cal))?,
                None => at.date(),
            };
            let t = day.and_time(at.time());
            return (t > after && self.runs_on(day)).then_some(t);
        }
        let first = self.start.map_or(after.date(), |s| s.max(after.date()));
        let far = first + Days::new(HORIZON);
        let last = self.end.map_or(far, |e| e.min(far));
        let mut d = first;
        while d <= last {
            if let Some(t) = self.times_on_in(d, cal).into_iter().find(|t| d.and_time(*t) > after) {
                return Some(d.and_time(t));
            }
            d = d.succ_opt()?;
        }
        None
    }
}

/// `from`, `from + step`, … while `<= to` (`step` in minutes), on the `days` (all when empty).
fn steps(date: NaiveDate, step: i64, from: NaiveTime, to: NaiveTime, days: &[Weekday]) -> Vec<NaiveTime> {
    if !days.is_empty() && !days.contains(&date.weekday()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut t = from;
    while t <= to {
        out.push(t);
        let next = t + Duration::minutes(step.max(1));
        if next <= t {
            // Past midnight: the day is done.
            break;
        }
        t = next;
    }
    out
}

/// Every time of `whens` after `after` and up to `until`, in order, each once (no calendar).
pub fn between(whens: &[When], after: NaiveDateTime, until: NaiveDateTime) -> Vec<NaiveDateTime> {
    between_in(whens, after, until, None)
}

/// Every time of `whens` after `after` and up to `until`, in order, each once.
pub fn between_in(whens: &[When], after: NaiveDateTime, until: NaiveDateTime, cal: Option<&Calendar>) -> Vec<NaiveDateTime> {
    let mut out = BTreeSet::new();
    let mut d = after.date();
    let last = until.date().min(after.date() + Days::new(400));
    while d <= last {
        for w in whens {
            for t in w.times_on_in(d, cal) {
                let dt = d.and_time(t);
                if dt > after && dt <= until {
                    out.insert(dt);
                }
            }
        }
        let Some(n) = d.succ_opt() else { break };
        d = n;
    }
    out.into_iter().collect()
}

/// The next `n` times of `whens` after `after` with no calendar.
pub fn next_times(whens: &[When], after: NaiveDateTime, n: usize) -> Vec<NaiveDateTime> {
    next_times_in(whens, after, n, None)
}

/// The next `n` times of `whens` after `after`, in order, each once (two
/// `[[when]]` that name the same time are one time).
pub fn next_times_in(whens: &[When], after: NaiveDateTime, n: usize, cal: Option<&Calendar>) -> Vec<NaiveDateTime> {
    let mut out = BTreeSet::new();
    for w in whens {
        let mut cursor = after;
        for _ in 0..n {
            match w.next_after_in(cursor, cal) {
                Some(t) => {
                    out.insert(t);
                    cursor = t;
                }
                None => break,
            }
        }
    }
    out.into_iter().take(n).collect()
}

/// How far on either side of the two looks the wall-clock times are listed:
/// past any change of the clock's offset.
const SLACK: Duration = Duration::hours(2);

/// The moments of `whens` that came due after `checked` and by `now` (the two
/// looks at the clock, as instants), each with the wall-clock time it was
/// written as, in order. Each instant once: a wall-clock time the zone shows
/// twice is its first showing; one the zone skips is the first time after it.
pub fn due<Tz: TimeZone>(tz: &Tz, whens: &[When], checked: DateTime<Utc>, now: DateTime<Utc>) -> Vec<(DateTime<Utc>, NaiveDateTime)> {
    due_in(tz, whens, None, checked, now)
}

/// [`due`] with the net's calendar.
pub fn due_in<Tz: TimeZone>(
    tz: &Tz,
    whens: &[When],
    cal: Option<&Calendar>,
    checked: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, NaiveDateTime)> {
    if now <= checked || whens.is_empty() {
        return Vec::new();
    }
    let lo = checked.with_timezone(tz).naive_local() - SLACK;
    let hi = now.with_timezone(tz).naive_local() + SLACK;
    let mut out: BTreeMap<DateTime<Utc>, NaiveDateTime> = BTreeMap::new();
    for naive in between_in(whens, lo, hi, cal) {
        let Some(instant) = resolve(tz, naive) else { continue };
        if checked < instant && instant <= now {
            out.entry(instant).or_insert(naive);
        }
    }
    out.into_iter().collect()
}

/// The instant a wall-clock time is in `tz`: the first of two, or the first
/// time after a skipped one. `None` only when the clock skips over three hours.
pub fn resolve<Tz: TimeZone>(tz: &Tz, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    let mut t = naive;
    for _ in 0..=180 {
        match tz.from_local_datetime(&t) {
            MappedLocalTime::Single(d) | MappedLocalTime::Ambiguous(d, _) => return Some(d.to_utc()),
            MappedLocalTime::None => t += Duration::minutes(1),
        }
    }
    None
}

/// `"02:00"` or `"02:00:30"`.
pub fn time(s: &str) -> Result<NaiveTime, String> {
    let s = s.trim();
    NaiveTime::parse_from_str(s, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M"))
        .map_err(|_| format!("時刻は \"02:00\" のように書く: \"{s}\""))
}

/// `mon` … `sun` (or the whole word), any case.
pub fn weekday(s: &str) -> Result<Weekday, String> {
    let w = s.trim().to_ascii_lowercase();
    let w = w.get(..3).unwrap_or(&w);
    match w {
        "mon" => Ok(Weekday::Mon),
        "tue" => Ok(Weekday::Tue),
        "wed" => Ok(Weekday::Wed),
        "thu" => Ok(Weekday::Thu),
        "fri" => Ok(Weekday::Fri),
        "sat" => Ok(Weekday::Sat),
        "sun" => Ok(Weekday::Sun),
        _ => Err(format!("曜日は mon・tue・wed・thu・fri・sat・sun のどれか: \"{}\"", s.trim())),
    }
}

/// A TOML date (`2026-11-30`) or a string of one.
pub fn date(v: &toml::Value) -> Result<NaiveDate, String> {
    let s = match v {
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::String(s) => s.trim().to_owned(),
        _ => return Err("日付は 2026-11-30 のように書く".to_owned()),
    };
    let day = s.get(..10).unwrap_or(&s);
    NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|_| format!("日付は 2026-11-30 のように書く: {s}"))
}

/// A TOML date-time (`2026-11-30T21:00:00`, no offset) or a string of one.
pub fn datetime(v: &toml::Value) -> Result<NaiveDateTime, String> {
    let s = match v {
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::String(s) => s.trim().to_owned(),
        _ => return Err("日時は 2026-11-30T21:00:00 のように書く".to_owned()),
    };
    let s = s.replace(' ', "T");
    NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M"))
        .map_err(|_| format!("日時は 2026-11-30T21:00:00 のように書く（時差はつけない）: {s}"))
}

/// `"04-01"` as (month, day); `02-29` allowed.
pub fn month_day(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("date は \"04-01\" のように月-日で書く: \"{}\"", s.trim());
    let (m, d) = s.trim().split_once('-').ok_or_else(bad)?;
    let (m, d): (u32, u32) = (m.parse().map_err(|_| bad())?, d.parse().map_err(|_| bad())?);
    // A leap year: 02-29 is a day.
    NaiveDate::from_ymd_opt(2024, m, d).ok_or_else(bad)?;
    Ok((m, d))
}

/// The mailbox a `mail` start watches, read from `imaps://user@host[:port][/folder]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub folder: String,
}

/// `imaps://user@host[:port][/folder]`: only IMAP over TLS; the port is 993
/// and the folder `INBOX` when left out. The user may be a mail address
/// (`me@example.com@imap.example.com`): the last `@` before the folder ends it.
pub fn mailbox(s: &str) -> Result<Mailbox, String> {
    const SHAPE: &str = "mail は \"imaps://ユーザー@サーバー/フォルダー\" で書く（例: imaps://me@example.com@imap.example.com/INBOX）";
    let s = s.trim();
    let rest = s.strip_prefix("imaps://").ok_or_else(|| {
        if s.starts_with("imap://") {
            "mail は imaps:// だけ（暗号化しない imap:// は使えない）".to_owned()
        } else {
            SHAPE.to_owned()
        }
    })?;
    let (auth, folder) = rest.split_once('/').unwrap_or((rest, ""));
    let (user, server) = auth.rsplit_once('@').ok_or_else(|| SHAPE.to_owned())?;
    let (host, port) = match server.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| format!("mail のポートは 1〜65535: \"{p}\""))?),
        None => (server, 993),
    };
    let bad = |t: &str| t.is_empty() || t.chars().any(|c| c.is_whitespace() || c == '"');
    if bad(user) || bad(host) {
        return Err(SHAPE.to_owned());
    }
    let folder = if folder.trim().is_empty() { "INBOX" } else { folder.trim() };
    Ok(Mailbox { user: user.to_owned(), host: host.to_owned(), port, folder: folder.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn mailbox_reads_user_host_port_and_folder() {
        let m = mailbox("imaps://me@example.com@imap.example.com/INBOX/請求").unwrap();
        assert_eq!(m, Mailbox { user: "me@example.com".into(), host: "imap.example.com".into(), port: 993, folder: "INBOX/請求".into() });
        let m = mailbox("imaps://batch@mail.corp.local:1993").unwrap();
        assert_eq!((m.user.as_str(), m.port, m.folder.as_str()), ("batch", 1993, "INBOX"));
        assert!(mailbox("imap://a@b").unwrap_err().contains("imaps://"));
        for bad in ["imaps://host", "imaps://@host", "imaps://a@", "imaps://a@b:0", "imaps://a@b:x", "https://a@b", "imaps://a b@c"] {
            assert!(mailbox(bad).is_err(), "{bad}");
        }
    }
    fn t(s: &str) -> NaiveTime {
        time(s).unwrap()
    }
    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }
    fn when(kind: Kind) -> When {
        When { kind, except: Vec::new(), start: None, end: None, on_closed: None, wait_for: None, wait_until: None }
    }
    fn company() -> Calendar {
        Calendar::parse("会社", "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [2026-11-20]\n").unwrap()
    }
    fn at(times: &[&str]) -> Vec<NaiveTime> {
        times.iter().map(|s| t(s)).collect()
    }
    fn texts(v: &[NaiveDateTime]) -> Vec<String> {
        v.iter().map(|x| x.format("%Y-%m-%d %H:%M").to_string()).collect()
    }

    #[test]
    fn after_jobnet_takes_the_ends_its_on_names_and_no_record_of_a_time_not_run() {
        assert!(AfterOn::Ok.lets(State::Ok) && AfterOn::Ok.lets(State::Warn) && !AfterOn::Ok.lets(State::Error));
        assert!(AfterOn::Error.lets(State::Error) && AfterOn::Error.lets(State::Timeout) && AfterOn::Error.lets(State::Killed));
        assert!(!AfterOn::Error.lets(State::Warn));
        assert!(AfterOn::Any.lets(State::Ok) && AfterOn::Any.lets(State::Killed));
        for s in [State::Skipped, State::Missed, State::Locked, State::Held, State::NotRun, State::Running, State::Waiting] {
            assert!(!AfterOn::Any.lets(s), "{s:?}");
        }
        let w = when(Kind::AfterJobnet { jobnet: "経理/売上取込".to_owned(), on: AfterOn::Any });
        assert!(w.is_event());
        assert_eq!(w.next_after(dt("2026-10-30 23:00")), None);
        assert_eq!(AfterOn::default().name(), "ok");
    }

    #[test]
    fn a_day_has_its_times_in_order_and_the_next_crosses_midnight() {
        let w = when(Kind::Day { at: at(&["23:00", "02:00", "02:00"]) });
        assert_eq!(w.times_on(d("2026-10-30")), at(&["02:00", "23:00"]));
        assert_eq!(w.next_after(dt("2026-10-30 23:00")), Some(dt("2026-10-31 02:00")));
        assert_eq!(w.next_after(dt("2026-10-30 22:59")), Some(dt("2026-10-30 23:00")));
    }

    #[test]
    fn month_end_and_a_day_a_short_month_lacks() {
        let last = when(Kind::Month { rule: MonthRule::Last, at: at(&["21:00"]) });
        assert_eq!(last.next_after(dt("2026-02-01 00:00")), Some(dt("2026-02-28 21:00")));
        assert_eq!(last.next_after(dt("2028-02-01 00:00")), Some(dt("2028-02-29 21:00")));
        assert_eq!(last.next_after(dt("2026-04-01 00:00")), Some(dt("2026-04-30 21:00")));
        let d31 = when(Kind::Month { rule: MonthRule::Day { day: 31, shift: 0 }, at: at(&["21:00"]) });
        assert!(d31.times_on(d("2026-04-30")).is_empty());
        assert_eq!(d31.next_after(dt("2026-04-01 00:00")), Some(dt("2026-05-31 21:00")));
        let d25 = when(Kind::Month { rule: MonthRule::Day { day: 25, shift: 0 }, at: at(&["09:00"]) });
        assert_eq!(d25.next_after(dt("2026-10-25 09:00")), Some(dt("2026-11-25 09:00")));
    }

    #[test]
    fn a_calendar_counts_business_days() {
        let c = company();
        // November 2026: the 1st is a Sunday, the 3rd and 23rd holidays, the 20th closed.
        let fifth = when(Kind::Month { rule: MonthRule::BusinessDay { n: 5 }, at: at(&["09:00"]) });
        assert_eq!(fifth.next_after_in(dt("2026-11-01 00:00"), Some(&c)), Some(dt("2026-11-09 09:00")));
        assert_eq!(fifth.next_after(dt("2026-11-01 00:00")), None, "no calendar: no business days");
        let last = when(Kind::Month { rule: MonthRule::BusinessDay { n: -1 }, at: at(&["21:00"]) });
        assert_eq!(last.next_after_in(dt("2026-11-01 00:00"), Some(&c)), Some(dt("2026-11-30 21:00")));
        assert_eq!(last.next_after_in(dt("2026-12-01 00:00"), Some(&c)), Some(dt("2026-12-31 21:00")));
        let shifted = when(Kind::Month { rule: MonthRule::Day { day: 20, shift: -3 }, at: at(&["09:00"]) });
        assert_eq!(shifted.next_after_in(dt("2026-11-01 00:00"), Some(&c)), Some(dt("2026-11-17 09:00")));
        assert!(shifted.times_on_in(d("2026-11-20"), Some(&c)).is_empty());
        assert!(shifted.times_on(d("2026-11-20")).is_empty() && shifted.times_on(d("2026-11-17")).is_empty());
        // A shift past the month's start lands in the month before.
        let early = when(Kind::Month { rule: MonthRule::Day { day: 1, shift: -1 }, at: at(&["09:00"]) });
        assert_eq!(early.times_on_in(d("2026-11-30"), Some(&c)), at(&["09:00"]));
        assert!(MonthRule::BusinessDay { n: 1 }.needs_calendar() && MonthRule::Day { day: 1, shift: 1 }.needs_calendar());
        assert!(!MonthRule::Day { day: 1, shift: 0 }.needs_calendar() && !MonthRule::Last.needs_calendar());
    }

    #[test]
    fn on_closed_moves_or_skips_times() {
        let c = company();
        let mut daily = when(Kind::Day { at: at(&["02:00"]) });
        // With a calendar, a closed day's time skips unless told otherwise.
        assert!(daily.times_on_in(d("2026-11-21"), Some(&c)).is_empty());
        assert_eq!(daily.times_on_in(d("2026-11-21"), None), at(&["02:00"]));
        assert_eq!(daily.next_after_in(dt("2026-11-20 00:00"), Some(&c)), Some(dt("2026-11-24 02:00")));
        daily.on_closed = Some(OnClosed::Run);
        assert_eq!(daily.next_after_in(dt("2026-11-20 00:00"), Some(&c)), Some(dt("2026-11-20 02:00")));
        let mut m = when(Kind::Month { rule: MonthRule::Day { day: 21, shift: 0 }, at: at(&["09:00"]) });
        m.on_closed = Some(OnClosed::Prev);
        // The 21st is a Saturday, the 20th closed: the time runs on the 19th.
        assert_eq!(m.times_on_in(d("2026-11-19"), Some(&c)), at(&["09:00"]));
        assert!(m.times_on_in(d("2026-11-21"), Some(&c)).is_empty());
        assert_eq!(m.next_after_in(dt("2026-11-01 00:00"), Some(&c)), Some(dt("2026-11-19 09:00")));
        m.on_closed = Some(OnClosed::Next);
        assert_eq!(m.next_after_in(dt("2026-11-01 00:00"), Some(&c)), Some(dt("2026-11-24 09:00")));
        // `except` is on the day the time runs.
        m.except = vec![d("2026-11-24")];
        assert_eq!(m.next_after_in(dt("2026-11-01 00:00"), Some(&c)), Some(dt("2026-12-21 09:00")));
        let once = when(Kind::Once { at: dt("2026-11-22 10:00") });
        assert_eq!(once.next_after_in(dt("2026-11-01 00:00"), Some(&c)), None);
        assert_eq!(once.next_after(dt("2026-11-01 00:00")), Some(dt("2026-11-22 10:00")));
        let mut once_prev = once.clone();
        once_prev.on_closed = Some(OnClosed::Prev);
        assert_eq!(once_prev.next_after_in(dt("2026-11-01 00:00"), Some(&c)), Some(dt("2026-11-19 10:00")));
        assert_eq!(between_in(&[once_prev], dt("2026-11-01 00:00"), dt("2026-12-01 00:00"), Some(&c)), [dt("2026-11-19 10:00")]);
        assert_eq!(next_times_in(&[daily.clone()], dt("2026-11-20 00:00"), 2, Some(&c)), [dt("2026-11-20 02:00"), dt("2026-11-21 02:00")]);
    }

    #[test]
    fn the_nth_weekday_of_a_month() {
        let w = when(Kind::Month { rule: MonthRule::Nth { nth: 2, weekday: Weekday::Tue }, at: at(&["10:00"]) });
        // 2026-11-01 is a Sunday: the Tuesdays are the 3rd and the 10th.
        assert_eq!(w.next_after(dt("2026-11-01 00:00")), Some(dt("2026-11-10 10:00")));
        assert!(w.times_on(d("2026-11-03")).is_empty());
        let fifth = when(Kind::Month { rule: MonthRule::Nth { nth: 5, weekday: Weekday::Fri }, at: at(&["10:00"]) });
        // November 2026 has four Fridays; the next fifth is 2027-01-29.
        assert_eq!(fifth.next_after(dt("2026-11-01 00:00")), Some(dt("2027-01-29 10:00")));
    }

    #[test]
    fn except_start_and_end_cut_days_out() {
        let mut w = when(Kind::Day { at: at(&["02:00"]) });
        w.except = vec![d("2026-10-31")];
        w.start = Some(d("2026-10-30"));
        w.end = Some(d("2026-11-02"));
        assert!(w.times_on(d("2026-10-29")).is_empty());
        assert!(w.times_on(d("2026-10-31")).is_empty());
        assert!(w.times_on(d("2026-11-03")).is_empty());
        assert_eq!(w.next_after(dt("2026-10-01 00:00")), Some(dt("2026-10-30 02:00")));
        assert_eq!(w.next_after(dt("2026-10-30 02:00")), Some(dt("2026-11-01 02:00")));
        assert_eq!(w.next_after(dt("2026-11-02 02:00")), None);
    }

    #[test]
    fn minutes_and_hours_step_within_from_and_to_inclusive_on_the_days_named() {
        let w = when(Kind::Minutes { n: 15, from: t("09:00"), to: t("10:00"), days: vec![Weekday::Mon] });
        // 2026-11-02 is a Monday.
        assert_eq!(w.times_on(d("2026-11-02")), at(&["09:00", "09:15", "09:30", "09:45", "10:00"]));
        assert!(w.times_on(d("2026-11-03")).is_empty());
        let h = when(Kind::Hours { n: 2, from: t("22:00"), to: t("23:59"), days: Vec::new() });
        assert_eq!(h.times_on(d("2026-11-03")), at(&["22:00"]));
        let all = when(Kind::Hours { n: 6, from: t("00:00"), to: t("23:59"), days: Vec::new() });
        assert_eq!(all.times_on(d("2026-11-03")), at(&["00:00", "06:00", "12:00", "18:00"]));
    }

    #[test]
    fn a_week_a_year_and_dates() {
        let w = when(Kind::Week { days: vec![Weekday::Mon, Weekday::Fri], at: at(&["08:30"]) });
        assert_eq!(w.next_after(dt("2026-10-30 08:30")), Some(dt("2026-11-02 08:30")));
        let leap = when(Kind::Year { month: 2, day: 29, at: at(&["00:00"]) });
        assert_eq!(leap.next_after(dt("2026-03-01 00:00")), Some(dt("2028-02-29 00:00")));
        let apr = when(Kind::Year { month: 4, day: 1, at: at(&["09:00"]) });
        assert_eq!(apr.next_after(dt("2026-04-01 09:00")), Some(dt("2027-04-01 09:00")));
        let ds = when(Kind::Dates { dates: vec![d("2026-12-30"), d("2027-01-04")], at: at(&["09:00"]) });
        assert_eq!(ds.next_after(dt("2026-12-30 09:00")), Some(dt("2027-01-04 09:00")));
        assert_eq!(ds.next_after(dt("2027-01-04 09:00")), None);
        let once = when(Kind::Once { at: dt("2026-11-30 21:00") });
        assert_eq!(once.next_after(dt("2026-11-30 20:59")), Some(dt("2026-11-30 21:00")));
        assert_eq!(once.next_after(dt("2026-11-30 21:00")), None);
    }

    #[test]
    fn the_next_times_merge_the_whens_and_name_a_time_once() {
        let whens =
            vec![when(Kind::Day { at: at(&["02:00"]) }), when(Kind::Week { days: vec![Weekday::Mon], at: at(&["02:00", "12:00"]) })];
        let next = next_times(&whens, dt("2026-11-01 00:00"), 4);
        assert_eq!(texts(&next), ["2026-11-01 02:00", "2026-11-02 02:00", "2026-11-02 12:00", "2026-11-03 02:00"]);
        assert!(next_times(&[], dt("2026-11-01 00:00"), 10).is_empty());
        let span = between(&whens, dt("2026-11-01 02:00"), dt("2026-11-02 12:00"));
        assert_eq!(texts(&span), ["2026-11-02 02:00", "2026-11-02 12:00"]);
    }

    #[test]
    fn the_leaf_values_read_and_refuse() {
        assert_eq!(time("2:00").unwrap(), t("02:00"));
        assert_eq!(time("02:00:30").unwrap().format("%H:%M:%S").to_string(), "02:00:30");
        assert!(time("25:00").unwrap_err().contains("02:00"));
        assert_eq!(weekday("Tuesday").unwrap(), Weekday::Tue);
        assert!(weekday("火").unwrap_err().contains("mon"));
        let v: toml::Value = toml::from_str("a = 2026-11-30\nb = \"2026-11-30\"\nc = 2026-11-30T21:00:00\nd = 1\n").unwrap();
        assert_eq!(date(&v["a"]).unwrap(), d("2026-11-30"));
        assert_eq!(date(&v["b"]).unwrap(), d("2026-11-30"));
        assert_eq!(datetime(&v["c"]).unwrap(), dt("2026-11-30 21:00"));
        assert!(date(&v["d"]).is_err());
        assert!(datetime(&v["a"]).is_err());
        assert_eq!(month_day("04-01").unwrap(), (4, 1));
        assert_eq!(month_day("02-29").unwrap(), (2, 29));
        assert!(month_day("13-01").is_err());
        assert!(month_day("0401").is_err());
    }

    #[test]
    fn a_when_survives_toml() {
        let w = When {
            kind: Kind::Month { rule: MonthRule::Nth { nth: 2, weekday: Weekday::Tue }, at: at(&["10:00"]) },
            except: vec![d("2026-12-08")],
            start: Some(d("2026-11-01")),
            end: None,
            on_closed: Some(OnClosed::Prev),
            wait_for: Some("経理/売上取込".to_owned()),
            wait_until: Some(at(&["09:30"])[0]),
        };
        let text = toml::to_string(&vec![w.clone()].into_iter().map(|w| ("w", w)).collect::<BTreeMap<_, _>>()).unwrap();
        let back: BTreeMap<String, When> = toml::from_str(&text).unwrap();
        assert_eq!(back["w"], w);
    }

    #[test]
    fn an_event_has_no_next_time_and_survives_toml() {
        let file = When {
            kind: Kind::File { file: "D:\\in\\*.csv".to_owned(), settle: None },
            except: Vec::new(),
            start: None,
            end: None,
            on_closed: Some(OnClosed::Run),
            wait_for: None,
            wait_until: None,
        };
        let start = When { kind: Kind::OnStart, ..file.clone() };
        let log = When { kind: Kind::Log { log: "D:\\app\\a.log".to_owned(), contains: "ERROR".to_owned() }, ..file.clone() };
        let eventlog = When { kind: Kind::EventLog { eventlog: "Security".to_owned(), ids: vec![4625], source: None }, ..file.clone() };
        let idle = When { kind: Kind::Idle { idle: 600 }, ..file.clone() };
        for w in [&file, &start, &log, &eventlog, &idle] {
            assert!(w.is_event());
            assert_eq!(w.next_after(dt("2026-11-30 00:00")), None);
            assert!(w.times_on(d("2026-11-30")).is_empty());
            let text = toml::to_string(&BTreeMap::from([("w", w.clone())])).unwrap();
            let back: BTreeMap<String, When> = toml::from_str(&text).unwrap();
            assert_eq!(&back["w"], w);
        }
        assert!(toml::to_string(&BTreeMap::from([("w", file)])).unwrap().contains("every = \"file\""));
        assert!(next_times(&[start], dt("2026-11-30 00:00"), 3).is_empty());
    }

    /// UTC−5, an hour ahead between 2026-03-08 02:00 and 2026-11-01 02:00 on
    /// the wall clock (as the east coast of the US).
    #[derive(Clone, Debug)]
    struct Coast;
    const STD: i32 = -5 * 3600;
    const DST: i32 = -4 * 3600;

    fn summer(utc: &NaiveDateTime) -> bool {
        (dt("2026-03-08 07:00")..dt("2026-11-01 06:00")).contains(utc)
    }

    impl TimeZone for Coast {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Coast {
            Coast
        }
        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> MappedLocalTime<FixedOffset> {
            let fits: Vec<FixedOffset> = [DST, STD]
                .into_iter()
                .filter(|o| summer(&(*local - Duration::seconds(i64::from(*o)))) == (*o == DST))
                .map(|o| FixedOffset::east_opt(o).unwrap())
                .collect();
            match fits[..] {
                [o] => MappedLocalTime::Single(o),
                [a, b] => MappedLocalTime::Ambiguous(a, b),
                _ => MappedLocalTime::None,
            }
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            FixedOffset::east_opt(if summer(utc) { DST } else { STD }).unwrap()
        }
    }

    fn utc(s: &str) -> DateTime<Utc> {
        dt(s).and_utc()
    }

    #[test]
    fn the_test_zone_skips_and_repeats_an_hour() {
        assert!(matches!(Coast.from_local_datetime(&dt("2026-03-08 02:30")), MappedLocalTime::None));
        assert!(matches!(Coast.from_local_datetime(&dt("2026-11-01 01:30")), MappedLocalTime::Ambiguous(..)));
        assert_eq!(Coast.from_local_datetime(&dt("2026-07-01 12:00")).unwrap().to_utc(), utc("2026-07-01 16:00"));
        assert_eq!(Coast.from_local_datetime(&dt("2026-12-01 12:00")).unwrap().to_utc(), utc("2026-12-01 17:00"));
    }

    #[test]
    fn a_skipped_time_runs_once_at_the_first_time_after_it() {
        let whens = vec![when(Kind::Day { at: at(&["02:30"]) })];
        // 01:00 standard time, then 04:00 summer time: the clock jumped from 02:00 to 03:00.
        let got = due(&Coast, &whens, utc("2026-03-08 06:00"), utc("2026-03-08 08:00"));
        assert_eq!(got, vec![(utc("2026-03-08 07:00"), dt("2026-03-08 02:30"))]);
        // Looked at every minute across the jump: still once.
        let mut all = Vec::new();
        let mut checked = utc("2026-03-08 06:00");
        while checked < utc("2026-03-08 08:00") {
            let now = checked + Duration::minutes(1);
            all.extend(due(&Coast, &whens, checked, now));
            checked = now;
        }
        assert_eq!(all, vec![(utc("2026-03-08 07:00"), dt("2026-03-08 02:30"))]);
    }

    #[test]
    fn a_repeated_time_runs_once_and_a_clock_set_back_never_runs_twice() {
        let whens = vec![when(Kind::Day { at: at(&["01:30"]) })];
        // 01:00 summer time to 02:00 standard time: 01:30 came twice on the wall.
        let got = due(&Coast, &whens, utc("2026-11-01 05:00"), utc("2026-11-01 07:00"));
        assert_eq!(got, vec![(utc("2026-11-01 05:30"), dt("2026-11-01 01:30"))]);
        // Looked at after the first 01:30, at 01:45 summer time, then at 01:15 standard time: nothing.
        assert!(due(&Coast, &whens, utc("2026-11-01 05:45"), utc("2026-11-01 06:15")).is_empty());
        let mut all = Vec::new();
        let mut checked = utc("2026-11-01 05:00");
        while checked < utc("2026-11-01 07:00") {
            let now = checked + Duration::minutes(1);
            all.extend(due(&Coast, &whens, checked, now));
            checked = now;
        }
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn every_time_a_stopped_pc_slept_through_is_listed_in_order() {
        let whens =
            vec![when(Kind::Day { at: at(&["02:00"]) }), when(Kind::Week { days: vec![Weekday::Thu], at: at(&["02:00", "09:00"]) })];
        // Off from Tuesday 2026-10-27 20:00 to Friday 2026-10-30 01:00 (summer time, UTC−4).
        let got = due(&Coast, &whens, utc("2026-10-28 00:00"), utc("2026-10-30 05:00"));
        let planned: Vec<String> = got.iter().map(|(_, p)| p.format("%m-%d %H:%M").to_string()).collect();
        assert_eq!(planned, ["10-28 02:00", "10-29 02:00", "10-29 09:00"]);
        assert_eq!(got[0].0, utc("2026-10-28 06:00"));
        assert!(due(&Coast, &whens, utc("2026-10-30 05:00"), utc("2026-10-30 05:00")).is_empty());
        assert!(due(&Coast, &whens, utc("2026-10-30 05:00"), utc("2026-10-30 04:00")).is_empty());
    }
}
