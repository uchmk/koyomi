//! `calendars/<name>.toml`: which days are business days. A net names one
//! (`calendar = "会社"`, or its group's `_group.toml` does), and its
//! `[[when]]` then counts business days (`business_day = 5`, `day = 20,
//! shift = -3`) and says what a time on a closed day does (`on_closed`).
//!
//! A day is open or closed by, in this order: the file's `open`, its
//! `closed`, Japan's holidays (`holidays = "japan"`), `weekly_off`. The
//! holiday table is a set of rules that hold until [`japan::UNTIL`]: past
//! it nothing is computed as if there were no holidays -- the day is "the
//! table is old" and the engine does not run it.
//!
//! The Cabinet Office's own table, `syukujitsu.csv`, put in `calendars/`
//! as it is downloaded, takes the place of the rules for the years it
//! covers ([`Table`]): a newer one carries the table on without a new
//! version of koyomi.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{Datelike, Days, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

use crate::msg::{self, line};
use crate::when::OnClosed;

/// The `format` this version reads.
pub const FORMAT: i64 = 1;

/// How far [`Calendar::adjust`] and [`Calendar::shift`] walk for a business
/// day before giving up: past any run of closed days a person would write.
const WALK: usize = 60;

/// The file name of the Cabinet Office's table in `calendars/`.
pub const TABLE_FILE: &str = "syukujitsu.csv";

/// The Cabinet Office's table of holidays (`syukujitsu.csv`: a header, then
/// `2027/1/1,元日` a line, in Shift_JIS or UTF-8). For the years it covers,
/// a day is a holiday only when the table has it; the rules do the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    /// The first and the last year it has a day in.
    pub years: (i32, i32),
    pub days: BTreeMap<NaiveDate, String>,
}

impl Table {
    /// Reads the table's text. The message (coded) names the line that is wrong.
    pub fn parse(text: &str) -> Result<Table, String> {
        let mut days = BTreeMap::new();
        for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (date, name) = line.split_once(',').unwrap_or((line, ""));
            let date = date.trim().trim_matches('"');
            let read = NaiveDate::parse_from_str(date, "%Y/%m/%d").or_else(|_| NaiveDate::parse_from_str(date, "%Y-%m-%d"));
            let Ok(d) = read else {
                // The first line is the header.
                if i == 0 {
                    continue;
                }
                return Err(msg::line(msg::TABLE_DATE_UNREADABLE, &[&(i + 1).to_string(), date]));
            };
            let name = name.trim().trim_matches('"');
            days.insert(d, if name.is_empty() { msg::note(msg::N_HOLIDAY_DEFAULT, &[]) } else { name.to_owned() });
        }
        let (Some(first), Some(last)) = (days.keys().next(), days.keys().next_back()) else {
            return Err(line(msg::TABLE_EMPTY, &[]));
        };
        Ok(Table { years: (first.year(), last.year()), days })
    }

    /// The file's bytes as text: UTF-8 when they are, else Shift_JIS (as the Cabinet Office writes it).
    pub fn decode(bytes: &[u8]) -> String {
        match std::str::from_utf8(bytes) {
            Ok(t) => t.to_owned(),
            Err(_) => ito_proc::Encoding::ShiftJis.decode(bytes),
        }
    }

    /// Reads the file at `path`; `None` when there is none.
    pub fn load(path: &Path) -> Result<Option<Table>, String> {
        match std::fs::read(path) {
            Ok(b) => Table::parse(&Table::decode(&b)).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(line(msg::FILE_READ_FAILED, &[&path.display().to_string(), &msg::io_words(&e)])),
        }
    }

    /// Whether `date` is in a year the table covers.
    pub fn covers(&self, date: NaiveDate) -> bool {
        (self.years.0..=self.years.1).contains(&date.year())
    }

    /// The last day it covers: the end of its last year.
    pub fn until(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.years.1, 12, 31).unwrap_or(japan::UNTIL)
    }
}

/// The last day the holidays are known: the rules', or the Cabinet Office's table's when it goes further.
pub fn until(table: Option<&Table>) -> NaiveDate {
    table.map_or(japan::UNTIL, |t| t.until().max(japan::UNTIL))
}

/// The caution (coded) about the age of a holiday table that lasts `until`, when there is one.
pub fn caution(until: NaiveDate, today: NaiveDate, warn_days: i64) -> Option<String> {
    let left = (until - today).num_days();
    if left < 0 {
        Some(line(msg::TABLE_STALE, &[&until.to_string()]))
    } else if left < warn_days {
        Some(line(msg::TABLE_ENDING_SOON, &[&until.to_string(), &left.to_string()]))
    } else {
        None
    }
}

/// One calendar file, read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    /// The file stem: what `calendar = "…"` names.
    pub name: String,
    /// `holidays = "japan"`: Japan's public holidays are closed days.
    pub holidays: bool,
    /// `weekly_off = ["sat", "sun"]`.
    pub weekly_off: Vec<Weekday>,
    /// `closed = [2026-11-20, { date = 2026-12-29, name = "年末" }]`: the name is `""` when none was written.
    pub closed: BTreeMap<NaiveDate, String>,
    /// `open = [...]`: days that are business days whatever the rest says.
    pub open: BTreeMap<NaiveDate, String>,
    /// `calendars/syukujitsu.csv`, when there is one and it read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<Table>,
}

/// What one day is, with why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Day {
    /// A business day by the rules.
    Business,
    /// A business day by `open`, with its name.
    Open(String),
    /// A closed day by `closed`, with its name.
    Closed(String),
    /// A public holiday, with its name.
    Holiday(String),
    /// A `weekly_off` day.
    WeeklyOff,
}

impl Day {
    pub fn is_business(&self) -> bool {
        matches!(self, Day::Business | Day::Open(_))
    }

    /// The name written or the holiday's; `None` for a plain day.
    pub fn name(&self) -> Option<&str> {
        match self {
            Day::Business | Day::WeeklyOff => None,
            Day::Open(n) | Day::Closed(n) => Some(n.as_str()).filter(|n| !n.is_empty()),
            Day::Holiday(n) => Some(n.as_str()),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: Option<i64>,
    holidays: Option<String>,
    weekly_off: Option<Vec<String>>,
    closed: Option<Vec<toml::Value>>,
    open: Option<Vec<toml::Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    date: toml::Value,
    #[serde(default)]
    name: String,
}

impl Calendar {
    /// Reads a calendar file's text. The message (coded) names the calendar and what is wrong.
    pub fn parse(name: &str, text: &str) -> Result<Calendar, String> {
        let raw: Raw = toml::from_str(text).map_err(|e| {
            let m = e.message();
            match m.strip_prefix("unknown field `").and_then(|r| r.split('`').next()) {
                Some(f) => line(msg::CAL_UNKNOWN_FIELD, &[name, f]),
                None => line(msg::CAL_TOML_SYNTAX, &[name, m]),
            }
        })?;
        match raw.format {
            None => return Err(line(msg::CAL_FORMAT_MISSING, &[name])),
            Some(f) if f != FORMAT => return Err(line(msg::CAL_FORMAT_UNSUPPORTED, &[name])),
            Some(_) => {}
        }
        let holidays = match raw.holidays.as_deref() {
            None | Some("none") => false,
            Some("japan") => true,
            Some(_) => return Err(line(msg::CAL_HOLIDAYS_VALUE, &[name])),
        };
        let weekly_off = raw
            .weekly_off
            .unwrap_or_default()
            .iter()
            .map(|s| crate::when::weekday(s).map_err(|e| line(msg::CAL_DAY_VALUE, &[name, "weekly_off", msg::uncoded(&e)])))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Calendar {
            name: name.to_owned(),
            holidays,
            weekly_off,
            closed: days(name, raw.closed, "closed")?,
            open: days(name, raw.open, "open")?,
            table: None,
        })
    }

    /// Reads the file at `path`, named by its stem.
    pub fn load(path: &Path) -> Result<Calendar, String> {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let text =
            std::fs::read_to_string(path).map_err(|e| line(msg::FILE_READ_FAILED, &[&path.display().to_string(), &msg::io_words(&e)]))?;
        Calendar::parse(&name, &text)
    }

    /// What `date` is.
    pub fn day(&self, date: NaiveDate) -> Day {
        if let Some(n) = self.open.get(&date) {
            return Day::Open(n.clone());
        }
        if let Some(n) = self.closed.get(&date) {
            return Day::Closed(n.clone());
        }
        if self.holidays {
            let name = match &self.table {
                Some(t) if t.covers(date) => t.days.get(&date).cloned(),
                _ => japan::holiday(date),
            };
            if let Some(n) = name {
                return Day::Holiday(n);
            }
        }
        if self.weekly_off.contains(&date.weekday()) {
            return Day::WeeklyOff;
        }
        Day::Business
    }

    pub fn is_business(&self, date: NaiveDate) -> bool {
        self.day(date).is_business()
    }

    /// Whether `date` is past what the holiday table knows.
    pub fn stale(&self, date: NaiveDate) -> bool {
        self.until().is_some_and(|u| date > u)
    }

    /// The last day the holiday table knows, when it is used.
    pub fn until(&self) -> Option<NaiveDate> {
        self.holidays.then(|| until(self.table.as_ref()))
    }

    /// The `n`th business day of the month (`n` < 0: from the end, -1 the
    /// last). `None` when the month has fewer.
    pub fn nth_business_day(&self, year: i32, month: u32, n: i32) -> Option<NaiveDate> {
        if n == 0 {
            return None;
        }
        let first = NaiveDate::from_ymd_opt(year, month, 1)?;
        let mut days: Vec<NaiveDate> =
            (0..31).filter_map(|i| first.checked_add_days(Days::new(i))).filter(|d| d.month() == month).collect();
        if n < 0 {
            days.reverse();
        }
        days.into_iter().filter(|d| self.is_business(*d)).nth(n.unsigned_abs() as usize - 1)
    }

    /// `date` moved `n` business days (forward when `n` > 0, back when < 0;
    /// `date` itself is not counted). `None` when no business day is found
    /// within the walk.
    pub fn shift(&self, date: NaiveDate, n: i32) -> Option<NaiveDate> {
        if n == 0 {
            return Some(date);
        }
        let mut d = date;
        let mut left = n.unsigned_abs();
        for _ in 0..WALK + n.unsigned_abs() as usize * 7 {
            d = if n > 0 { d.succ_opt()? } else { d.pred_opt()? };
            if self.is_business(d) {
                left -= 1;
                if left == 0 {
                    return Some(d);
                }
            }
        }
        None
    }

    /// The day a time written for `date` runs on under `on_closed`: the day
    /// itself when it is a business day; else the business day before or
    /// after, `None` (nothing runs) for `skip`, or the day as it is for `run`.
    pub fn adjust(&self, date: NaiveDate, on_closed: OnClosed) -> Option<NaiveDate> {
        if on_closed == OnClosed::Run || self.is_business(date) {
            return Some(date);
        }
        match on_closed {
            OnClosed::Prev => self.shift(date, -1),
            OnClosed::Next => self.shift(date, 1),
            _ => None,
        }
    }

    /// The days whose times run on `date` under `on_closed`: `date` itself,
    /// and the closed days that `prev`/`next` move onto it.
    pub fn sources(&self, date: NaiveDate, on_closed: OnClosed) -> Vec<NaiveDate> {
        if on_closed == OnClosed::Run {
            return vec![date];
        }
        if !self.is_business(date) {
            return Vec::new();
        }
        let mut out = vec![date];
        let step: fn(&NaiveDate) -> Option<NaiveDate> = match on_closed {
            OnClosed::Prev => NaiveDate::succ_opt,
            OnClosed::Next => NaiveDate::pred_opt,
            _ => return out,
        };
        let mut d = date;
        for _ in 0..WALK {
            let Some(n) = step(&d) else { break };
            if self.is_business(n) {
                break;
            }
            out.push(n);
            d = n;
        }
        out.sort();
        out
    }

    /// `closed = [...]` or `open = [...]` as TOML, for writing the file back.
    pub fn list_text(days: &BTreeMap<NaiveDate, String>) -> String {
        let items: Vec<String> = days
            .iter()
            .map(|(d, n)| if n.is_empty() { d.to_string() } else { format!("{{ date = {d}, name = {} }}", ito_quote(n)) })
            .collect();
        format!("[{}]", items.join(", "))
    }
}

/// A TOML basic string.
fn ito_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn days(name: &str, list: Option<Vec<toml::Value>>, field: &str) -> Result<BTreeMap<NaiveDate, String>, String> {
    let mut out = BTreeMap::new();
    for v in list.unwrap_or_default() {
        let (date, name) = match &v {
            toml::Value::Table(_) => {
                let n: Named = v.clone().try_into().map_err(|_| line(msg::CAL_ENTRY_SHAPE, &[name, field]))?;
                (crate::when::date(&n.date).map_err(|e| line(msg::CAL_DAY_VALUE, &[name, field, msg::uncoded(&e)]))?, n.name)
            }
            _ => (crate::when::date(&v).map_err(|e| line(msg::CAL_DAY_VALUE, &[name, field, msg::uncoded(&e)]))?, String::new()),
        };
        out.insert(date, name);
    }
    Ok(out)
}

/// Whether `name` may name a calendar file (`calendars/<name>.toml`).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('_')
        && !name.ends_with(['.', ' '])
        && !crate::jobnet::reserved(name)
        && !name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
}

/// A file in `calendars/` that did not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bad {
    /// The calendar it would have been, or `None` for the holiday table.
    pub calendar: Option<String>,
    /// Why, as a coded line.
    pub line: String,
}

/// Every calendar in `calendars/`, by name, and the files that did not read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Calendars {
    pub map: BTreeMap<String, Calendar>,
    /// The files that did not read, in name order, the holiday table's last.
    pub bad: Vec<Bad>,
    /// `calendars/syukujitsu.csv`, when there is one and it read.
    pub table: Option<Table>,
}

impl Calendars {
    /// Reads `<dir>/*.toml`. A missing folder is no calendars.
    pub fn load(dir: &Path) -> Calendars {
        let mut out = Calendars::default();
        let Ok(rd) = std::fs::read_dir(dir) else { return out };
        let mut files: Vec<_> =
            rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "toml")).collect();
        files.sort();
        for p in files {
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match Calendar::load(&p) {
                Ok(c) => {
                    out.map.insert(name, c);
                }
                Err(e) => out.bad.push(Bad { calendar: Some(name), line: e }),
            }
        }
        match Table::load(&dir.join(TABLE_FILE)) {
            Ok(table) => out.set_table(table),
            Err(e) => out.bad.push(Bad { calendar: None, line: e }),
        }
        out
    }

    /// Use `table` for every calendar's holidays.
    pub fn set_table(&mut self, table: Option<Table>) {
        for c in self.map.values_mut() {
            c.table = table.clone();
        }
        self.table = table;
    }

    pub fn get(&self, name: &str) -> Option<&Calendar> {
        self.map.get(name)
    }

    /// Why `name` cannot be used, when it cannot (a coded line): the file is missing or did not read.
    pub fn problem(&self, name: &str) -> Option<String> {
        if self.map.contains_key(name) {
            return None;
        }
        match self.bad.iter().find(|b| b.calendar.as_deref() == Some(name)) {
            Some(b) => Some(b.line.clone()),
            None => Some(line(msg::CAL_MISSING, &[name])),
        }
    }

    /// What `koyomi check` and the window's band say about the holiday
    /// table's age, for the calendars that use it: a line when fewer than
    /// `warn_days` days are left, another when it is past. Each once.
    pub fn cautions(&self, today: NaiveDate, warn_days: i64) -> Vec<String> {
        let mut out: Vec<String> = self.bad.iter().map(|b| b.line.clone()).collect();
        if self.map.values().any(|c| c.holidays) {
            out.extend(caution(until(self.table.as_ref()), today, warn_days));
        }
        out
    }

    /// The same lines by the calendar they are about, for that calendar's own page: a file that
    /// did not read under its name, the holiday table's age under each calendar that uses it.
    pub fn cautions_by_calendar(&self, today: NaiveDate, warn_days: i64) -> std::collections::BTreeMap<String, Vec<String>> {
        let mut out: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for b in &self.bad {
            if let Some(name) = &b.calendar {
                out.entry(name.clone()).or_default().push(b.line.clone());
            }
        }
        if let Some(line) = caution(until(self.table.as_ref()), today, warn_days) {
            for (name, cal) in &self.map {
                if cal.holidays {
                    out.entry(name.clone()).or_default().push(line.clone());
                }
            }
        }
        out
    }
}

/// Japan's public holidays, by the rules in force, up to [`UNTIL`].
pub mod japan {
    use crate::msg;
    use chrono::{Datelike, NaiveDate, Weekday};

    /// The last day the rules are known to hold: the Cabinet Office has
    /// announced the holidays through this year. Raise it with each year's
    /// announcement (and check the rules did not change).
    pub const UNTIL: NaiveDate = match NaiveDate::from_ymd_opt(2027, 12, 31) {
        Some(d) => d,
        None => unreachable!(),
    };

    /// The holiday's name on `date`, by the rules, when it is one. Past
    /// [`UNTIL`] the rules are applied as they are: the caller says whether
    /// to trust them.
    pub fn holiday(date: NaiveDate) -> Option<String> {
        Some(msg::note(name_of(date)?, &[]))
    }

    /// Which holiday `date` is.
    fn name_of(date: NaiveDate) -> Option<msg::Note> {
        if let Some(n) = fixed(date) {
            return Some(n);
        }
        // 振替休日: a holiday on a Sunday is made up on the first day after it that is not one.
        let mut p = date.pred_opt()?;
        while fixed(p).is_some() {
            if p.weekday() == Weekday::Sun {
                return Some(msg::N_HOLIDAY_SUBSTITUTE);
            }
            p = p.pred_opt()?;
        }
        // 国民の休日: a day between two holidays (not a Sunday).
        if date.weekday() != Weekday::Sun && fixed(date.pred_opt()?).is_some() && fixed(date.succ_opt()?).is_some() {
            return Some(msg::N_HOLIDAY_CITIZENS);
        }
        None
    }

    /// The holidays on their own days (not the made-up ones).
    fn fixed(date: NaiveDate) -> Option<msg::Note> {
        let (y, m, d) = (date.year(), date.month(), date.day());
        let nth_monday = |n: u32| date.weekday() == Weekday::Mon && (d - 1) / 7 + 1 == n;
        Some(match (m, d) {
            (1, 1) => msg::N_HOLIDAY_NEW_YEAR,
            (1, _) if nth_monday(2) => msg::N_HOLIDAY_COMING_OF_AGE,
            (2, 11) => msg::N_HOLIDAY_FOUNDATION,
            (2, 23) => msg::N_HOLIDAY_EMPEROR_BIRTHDAY,
            (3, _) if d == equinox(y, 20.8431) => msg::N_HOLIDAY_VERNAL_EQUINOX,
            (4, 29) => msg::N_HOLIDAY_SHOWA,
            (5, 3) => msg::N_HOLIDAY_CONSTITUTION,
            (5, 4) => msg::N_HOLIDAY_GREENERY,
            (5, 5) => msg::N_HOLIDAY_CHILDRENS,
            (7, _) if nth_monday(3) => msg::N_HOLIDAY_MARINE,
            (8, 11) => msg::N_HOLIDAY_MOUNTAIN,
            (9, _) if nth_monday(3) => msg::N_HOLIDAY_RESPECT_AGED,
            (9, _) if d == equinox(y, 23.2488) => msg::N_HOLIDAY_AUTUMN_EQUINOX,
            (10, _) if nth_monday(2) => msg::N_HOLIDAY_SPORTS,
            (11, 3) => msg::N_HOLIDAY_CULTURE,
            (11, 23) => msg::N_HOLIDAY_LABOR_THANKS,
            _ => return None,
        })
    }

    /// The day of the equinox in `year` (the usual approximation, good for
    /// 1980–2099), from the base for spring or autumn.
    fn equinox(year: i32, base: f64) -> u32 {
        let y = f64::from(year - 1980);
        (base + 0.242194 * y - (y / 4.0).floor()).floor() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn company() -> Calendar {
        Calendar::parse(
            "会社",
            "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [2026-11-20, { date = 2026-12-29, name = \"年末\" }]\nopen = [{ date = 2026-11-21, name = \"休日出勤\" }]\n",
        )
        .unwrap()
    }

    #[test]
    fn japans_holidays_in_2026_and_2027() {
        let h = |s: &str| japan::holiday(d(s));
        assert_eq!(h("2026-01-01"), Some("元日".to_owned()));
        assert_eq!(h("2026-01-12"), Some("成人の日".to_owned()));
        assert_eq!(h("2026-02-11"), Some("建国記念の日".to_owned()));
        assert_eq!(h("2026-02-23"), Some("天皇誕生日".to_owned()));
        assert_eq!(h("2026-03-20"), Some("春分の日".to_owned()));
        assert_eq!(h("2026-05-06"), Some("振替休日".to_owned()), "憲法記念日 on a Sunday, made up after こどもの日");
        assert_eq!(h("2026-07-20"), Some("海の日".to_owned()));
        assert_eq!(h("2026-09-21"), Some("敬老の日".to_owned()));
        assert_eq!(h("2026-09-22"), Some("国民の休日".to_owned()));
        assert_eq!(h("2026-09-23"), Some("秋分の日".to_owned()));
        assert_eq!(h("2026-10-12"), Some("スポーツの日".to_owned()));
        assert_eq!(h("2026-11-03"), Some("文化の日".to_owned()));
        assert_eq!(h("2026-11-23"), Some("勤労感謝の日".to_owned()));
        assert_eq!(h("2026-11-24"), None);
        assert_eq!(h("2027-03-21"), Some("春分の日".to_owned()));
        assert_eq!(h("2027-03-22"), Some("振替休日".to_owned()));
        assert_eq!(h("2027-09-20"), Some("敬老の日".to_owned()));
        assert_eq!(h("2027-09-23"), Some("秋分の日".to_owned()));
        assert_eq!(h("2027-09-22"), None, "a Wednesday between a plain day and a holiday");
        assert_eq!(h("2027-08-11"), Some("山の日".to_owned()));
    }

    #[test]
    fn open_beats_closed_beats_holidays_beats_the_week() {
        let c = company();
        assert_eq!(c.day(d("2026-11-19")), Day::Business);
        assert_eq!(c.day(d("2026-11-20")), Day::Closed(String::new()));
        assert_eq!(c.day(d("2026-11-21")), Day::Open("休日出勤".into()));
        assert_eq!(c.day(d("2026-11-22")), Day::WeeklyOff);
        assert_eq!(c.day(d("2026-11-23")), Day::Holiday("勤労感謝の日".into()));
        assert_eq!(c.day(d("2026-12-29")), Day::Closed("年末".into()));
        assert_eq!(c.day(d("2026-12-29")).name(), Some("年末"));
        assert_eq!(c.day(d("2026-11-20")).name(), None);
        assert!(c.is_business(d("2026-11-21")));
        let plain = Calendar::parse("x", "format = 1\n").unwrap();
        assert!(plain.is_business(d("2026-11-22")), "no rules: every day is a business day");
        assert!(!plain.holidays && plain.weekly_off.is_empty());
    }

    #[test]
    fn business_days_are_counted_and_shifted() {
        let c = company();
        // November 2026: the 1st is a Sunday; the 3rd and 23rd are holidays, the 20th is closed, the 21st (Sat) is open.
        assert_eq!(c.nth_business_day(2026, 11, 1), Some(d("2026-11-02")));
        assert_eq!(c.nth_business_day(2026, 11, 2), Some(d("2026-11-04")));
        assert_eq!(c.nth_business_day(2026, 11, 5), Some(d("2026-11-09")));
        assert_eq!(c.nth_business_day(2026, 11, -1), Some(d("2026-11-30")));
        assert_eq!(c.nth_business_day(2026, 11, -5), Some(d("2026-11-24")));
        assert_eq!(c.nth_business_day(2026, 11, 31), None);
        assert_eq!(c.nth_business_day(2026, 11, 0), None);
        assert_eq!(c.shift(d("2026-11-20"), -3), Some(d("2026-11-17")));
        assert_eq!(c.shift(d("2026-11-19"), 1), Some(d("2026-11-21")));
        assert_eq!(c.shift(d("2026-11-21"), 1), Some(d("2026-11-24")));
        assert_eq!(c.shift(d("2026-11-21"), 0), Some(d("2026-11-21")));
        assert_eq!(c.shift(d("2026-11-21"), 2), Some(d("2026-11-25")));
    }

    #[test]
    fn on_closed_moves_skips_or_keeps_a_time() {
        let c = company();
        let sat = d("2026-11-28");
        assert_eq!(c.adjust(sat, OnClosed::Prev), Some(d("2026-11-27")));
        assert_eq!(c.adjust(sat, OnClosed::Next), Some(d("2026-11-30")));
        assert_eq!(c.adjust(sat, OnClosed::Skip), None);
        assert_eq!(c.adjust(sat, OnClosed::Run), Some(sat));
        assert_eq!(c.adjust(d("2026-11-27"), OnClosed::Skip), Some(d("2026-11-27")));
        // The business day before a weekend takes the weekend's times under prev; the one after under next.
        assert_eq!(c.sources(d("2026-11-27"), OnClosed::Prev), [d("2026-11-27"), d("2026-11-28"), d("2026-11-29")]);
        assert_eq!(c.sources(d("2026-11-30"), OnClosed::Next), [d("2026-11-28"), d("2026-11-29"), d("2026-11-30")]);
        assert_eq!(c.sources(d("2026-11-27"), OnClosed::Skip), [d("2026-11-27")]);
        assert!(c.sources(sat, OnClosed::Skip).is_empty());
        assert_eq!(c.sources(sat, OnClosed::Run), [sat]);
    }

    #[test]
    fn the_table_has_an_end_and_says_so() {
        let c = company();
        assert!(!c.stale(d("2027-12-31")));
        assert!(c.stale(d("2028-01-01")));
        assert_eq!(c.until(), Some(japan::UNTIL));
        assert_eq!(Calendar::parse("x", "format = 1\n").unwrap().until(), None);
        let u = japan::UNTIL;
        assert_eq!(caution(u, d("2027-01-01"), 90), None);
        assert_eq!(caution(u, d("2027-11-01"), 90), Some(line(msg::TABLE_ENDING_SOON, &["2027-12-31", "60"])));
        assert_eq!(caution(u, d("2028-01-02"), 90), Some(line(msg::TABLE_STALE, &["2027-12-31"])));
        assert!(msg::TABLE_ENDING_SOON.is_warning() && msg::TABLE_STALE.is_warning());
    }

    #[test]
    fn a_file_that_does_not_read_says_why() {
        let p = |t: &str| Calendar::parse("x", t).unwrap_err();
        // Each is coded, with the calendar's name ("x") and what is wrong in the holes.
        assert_eq!(p("holidays = \"japan\"\n"), line(msg::CAL_FORMAT_MISSING, &["x"]));
        assert_eq!(p("format = 2\n"), line(msg::CAL_FORMAT_UNSUPPORTED, &["x"]));
        assert_eq!(p("format = 1\nholidays = \"us\"\n"), line(msg::CAL_HOLIDAYS_VALUE, &["x"]));
        let bad_day = p("format = 1\nweekly_off = [\"土\"]\n");
        assert!(msg::CAL_DAY_VALUE.is_in(&bad_day) && bad_day.contains("weekly_off") && bad_day.contains("\"土\""), "{bad_day}");
        let bad_closed = p("format = 1\nclosed = [1]\n");
        assert!(msg::CAL_DAY_VALUE.is_in(&bad_closed) && bad_closed.contains("closed"), "{bad_closed}");
        assert_eq!(p("format = 1\nopen = [{ day = 2026-11-20 }]\n"), line(msg::CAL_ENTRY_SHAPE, &["x", "open"]));
        assert_eq!(p("format = 1\nfoo = 1\n"), line(msg::CAL_UNKNOWN_FIELD, &["x", "foo"]));
        assert!(msg::CAL_TOML_SYNTAX.is_in(&p("format = [\n")));
        assert!(valid_name("会社") && valid_name("本社 2026"));
        assert!(!valid_name("") && !valid_name("..") && !valid_name("a/b") && !valid_name("a\\b") && !valid_name("_x"));
        assert!(!valid_name("NUL") && !valid_name("com1") && !valid_name("a.") && !valid_name("a "));
    }

    #[test]
    fn the_lists_write_back_as_toml() {
        let c = company();
        assert_eq!(Calendar::list_text(&c.closed), "[2026-11-20, { date = 2026-12-29, name = \"年末\" }]");
        assert_eq!(Calendar::list_text(&c.open), "[{ date = 2026-11-21, name = \"休日出勤\" }]");
        assert_eq!(Calendar::list_text(&BTreeMap::new()), "[]");
        let text = format!("format = 1\nclosed = {}\n", Calendar::list_text(&c.closed));
        assert_eq!(Calendar::parse("x", &text).unwrap().closed, c.closed);
    }

    #[test]
    fn a_folder_of_calendars_names_the_bad_ones() {
        let dir = std::env::temp_dir().join(format!("koyomi-core-cals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Calendars::load(&dir).map.is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("会社.toml"), "format = 1\nholidays = \"japan\"\n").unwrap();
        std::fs::write(dir.join("壊れ.toml"), "format = 1\nholidays = \"x\"\n").unwrap();
        std::fs::write(dir.join("memo.txt"), "x").unwrap();
        let cals = Calendars::load(&dir);
        assert_eq!(cals.map.keys().collect::<Vec<_>>(), ["会社"]);
        assert_eq!(cals.bad, [Bad { calendar: Some("壊れ".to_owned()), line: line(msg::CAL_HOLIDAYS_VALUE, &["壊れ"]) }]);
        assert_eq!(cals.problem("会社"), None);
        assert_eq!(cals.problem("壊れ").unwrap(), line(msg::CAL_HOLIDAYS_VALUE, &["壊れ"]));
        assert_eq!(cals.problem("本社").unwrap(), line(msg::CAL_MISSING, &["本社"]));
        assert_eq!(cals.cautions(d("2027-12-01"), 90).len(), 2);
        let by = cals.cautions_by_calendar(d("2027-12-01"), 90);
        assert_eq!(by.keys().collect::<Vec<_>>(), ["会社", "壊れ"]);
        assert!(by["会社"].len() == 1 && by["壊れ"][0] == line(msg::CAL_HOLIDAYS_VALUE, &["壊れ"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cabinet_offices_table_takes_over_its_years_and_carries_the_table_on() {
        let csv = "国民の祝日・休日月日,国民の祝日・休日名称\r\n2028/1/1,元日\r\n2028/1/10,成人の日\r\n2028/5/8,臨時の休日\r\n";
        let t = Table::parse(csv).unwrap();
        assert_eq!(t.years, (2028, 2028));
        assert_eq!(t.until(), d("2028-12-31"));
        // As the Cabinet Office writes it: Shift_JIS.
        let sjis = b"2028/1/1,\x8c\xb3\x93\xfa\r\n";
        assert_eq!(Table::parse(&Table::decode(sjis)).unwrap().days[&d("2028-01-01")], "元日");
        assert!(
            Table::parse("日付,名前\n2028/1/1,元日\n2028/13/1,x\n").unwrap_err()
                == msg::line(msg::TABLE_DATE_UNREADABLE, &["3", "2028/13/1"])
        );
        assert_eq!(Table::parse("日付,名前\n").unwrap_err(), line(msg::TABLE_EMPTY, &[]));
        assert_eq!(Table::parse("\u{feff}2028-01-01,\n").unwrap().days[&d("2028-01-01")], "祝日");

        let mut c = Calendar::parse("会社", "format = 1\nholidays = \"japan\"\n").unwrap();
        assert!(c.stale(d("2028-01-04")));
        c.table = Some(t);
        assert!(!c.stale(d("2028-12-31")) && c.stale(d("2029-01-01")));
        assert_eq!(c.until(), Some(d("2028-12-31")));
        // In its years only the table counts; the rules still give the years before.
        assert_eq!(c.day(d("2028-05-08")), Day::Holiday("臨時の休日".into()));
        assert_eq!(c.day(d("2028-02-11")), Day::Business);
        assert_eq!(c.day(d("2026-11-23")), Day::Holiday("勤労感謝の日".into()));
        assert_eq!(caution(c.until().unwrap(), d("2027-12-01"), 90), None);
    }

    #[test]
    fn a_table_in_the_folder_goes_to_every_calendar_and_a_bad_one_is_named() {
        let dir = std::env::temp_dir().join(format!("koyomi-core-table-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("会社.toml"), "format = 1\nholidays = \"japan\"\n").unwrap();
        std::fs::write(dir.join(TABLE_FILE), "日付,名前\n2028/1/1,元日\n").unwrap();
        let cals = Calendars::load(&dir);
        assert!(cals.bad.is_empty());
        assert_eq!(cals.map["会社"].until(), Some(d("2028-12-31")));
        assert!(cals.cautions(d("2027-12-01"), 90).is_empty());
        std::fs::write(dir.join(TABLE_FILE), "日付,名前\nx,y\n").unwrap();
        let cals = Calendars::load(&dir);
        assert_eq!(cals.bad, [Bad { calendar: None, line: msg::line(msg::TABLE_DATE_UNREADABLE, &["2", "x"]) }]);
        assert_eq!(cals.map["会社"].until(), Some(japan::UNTIL));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_calendar_that_is_not_there_is_said_in_words() {
        let dir = std::env::temp_dir().join(format!("koyomi-cal-words-{}", std::process::id()));
        let path = dir.join("none.toml");
        let err = Calendar::load(&path).unwrap_err();
        assert!(msg::FILE_READ_FAILED.is_in(&err), "{err}");
        assert!(!err.contains("os error"), "{err}");
        let why = msg::io_words_in(&msg::language(), &std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(err.contains(&why), "{err} / {why}");
    }
}
