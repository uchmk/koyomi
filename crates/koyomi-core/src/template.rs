//! `{date}` and friends in a job's `run`, `script`, `dir` and `env` values: the business
//! day of the run, moved and written the way the job wants it, so a monthly
//! job gets its month from koyomi and not from its own arithmetic.
//!
//! - `{date}` is `2026-10-30`; `{date:%Y%m%d}` writes it with a chrono format.
//! - `{date-1d}`, `{date+7d}`, `{date-1m}`, `{date-1y}` move it by days, months or years;
//!   `{date-1bd}` by business days (the net's calendar; without one it is an error).
//! - `{month}` is `202610`, `{month-1}` the month before (`{month-1:%Y-%m}` to write it another way).
//!
//! A `{` that does not start one of these is left as it is (PowerShell scripts are full of braces).

use chrono::format::{Item, StrftimeItems};
use chrono::{Months, NaiveDate};

use crate::calendar::Calendar;
use crate::jobnet::Jobnet;

/// One `{…}` found in a text.
struct Token {
    month: bool,
    n: i32,
    unit: Unit,
    format: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    Day,
    Business,
    Month,
    Year,
}

/// Whether every `{date…}` and `{month…}` in `text` is well formed (the day is not needed).
pub fn check(text: &str) -> Result<(), String> {
    walk(text, &mut |_, _| Ok(String::new())).map(|_| ())
}

/// Whether `text` has a business-day move, which needs the net to have a calendar.
pub fn needs_calendar(text: &str) -> bool {
    let mut found = false;
    let _ = walk(text, &mut |t, _| {
        found |= t.unit == Unit::Business;
        Ok(String::new())
    });
    found
}

/// `text` with every token written out for the business day `date`.
pub fn expand(text: &str, date: NaiveDate, cal: Option<&Calendar>) -> Result<String, String> {
    walk(text, &mut |t, src| {
        let moved = match (t.unit, t.n) {
            (_, 0) => Some(date),
            (Unit::Day, n) => date.checked_add_signed(chrono::Duration::days(n.into())),
            (Unit::Month, n) => shift_months(date, n),
            (Unit::Year, n) => n.checked_mul(12).and_then(|m| shift_months(date, m)),
            (Unit::Business, n) => {
                let cal = cal.ok_or_else(|| format!("{src}: 営業日で動かすには calendar が要る"))?;
                cal.shift(date, n)
            }
        };
        let moved = moved.ok_or_else(|| format!("{src}: {date} から動かした日がない"))?;
        let format = t.format.as_deref().unwrap_or(if t.month { "%Y%m" } else { "%Y-%m-%d" });
        Ok(moved.format(format).to_string())
    })
}

/// The net with every job's `run`, `script` and `dir` expanded for `date`.
pub fn expand_net(net: &Jobnet, date: NaiveDate, cal: Option<&Calendar>) -> Result<Jobnet, String> {
    let mut out = net.clone();
    for j in &mut out.jobs {
        let id = j.id.clone();
        let at = |what: &str, e: String| format!("{id} の {what}: {e}");
        for a in &mut j.run {
            *a = expand(a, date, cal).map_err(|e| at("run", e))?;
        }
        if let Some(s) = &mut j.script {
            *s = expand(s, date, cal).map_err(|e| at("script", e))?;
        }
        for (k, v) in &mut j.env {
            if !v.starts_with("secret:") {
                *v = expand(v, date, cal).map_err(|e| at(&format!("env {k}"), e))?;
            }
        }
        if let Some(d) = &mut j.dir {
            *d = expand(d, date, cal).map_err(|e| at("dir", e))?;
        }
    }
    Ok(out)
}

fn shift_months(date: NaiveDate, n: i32) -> Option<NaiveDate> {
    let m = Months::new(n.unsigned_abs());
    if n >= 0 {
        date.checked_add_months(m)
    } else {
        date.checked_sub_months(m)
    }
}

/// Calls `f` for each token with its source text and puts what it returns in its place.
fn walk(text: &str, f: &mut dyn FnMut(&Token, &str) -> Result<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find('}') else {
            out.push_str(tail);
            return Ok(out);
        };
        let src = &tail[..=end];
        match parse(&src[1..end]) {
            None => {
                out.push('{');
                rest = &tail[1..];
            }
            Some(Err(e)) => return Err(format!("{src}: {e}")),
            Some(Ok(t)) => {
                out.push_str(&f(&t, src)?);
                rest = &tail[end + 1..];
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// The inside of a `{…}`: `None` when it is not ours.
fn parse(inner: &str) -> Option<Result<Token, String>> {
    let (head, format) = match inner.split_once(':') {
        Some((h, f)) => (h, Some(f)),
        None => (inner, None),
    };
    let (month, rest) = if let Some(r) = head.strip_prefix("date") { (false, r) } else { (true, head.strip_prefix("month")?) };
    if !(rest.is_empty() || rest.starts_with(['+', '-'])) {
        return None;
    }
    Some(token(month, rest, format))
}

fn token(month: bool, rest: &str, format: Option<&str>) -> Result<Token, String> {
    let (n, unit) = if rest.is_empty() {
        (0, if month { Unit::Month } else { Unit::Day })
    } else {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let body = &rest[1..];
        let digits = body.chars().take_while(char::is_ascii_digit).count();
        let n: i32 = body[..digits].parse().map_err(|_| "動かす数がない（{date-1d} のように書く）".to_owned())?;
        let unit = match (&body[digits..], month) {
            ("", true) | ("m", _) => Unit::Month,
            ("d", false) => Unit::Day,
            ("bd", false) => Unit::Business,
            ("y", false) => Unit::Year,
            ("", false) => return Err("単位がない（d・bd・m・y）".to_owned()),
            (u, _) => return Err(format!("単位が読めない: {u}（{{month}} は数だけか m、{{date}} は d・bd・m・y）")),
        };
        (sign * n, unit)
    };
    if let Some(f) = format {
        if f.is_empty() || StrftimeItems::new(f).any(|i| matches!(i, Item::Error)) {
            return Err(format!("書式が読めない: {f}"));
        }
    }
    Ok(Token { month, n, unit, format: format.map(str::to_owned) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn cal() -> Calendar {
        Calendar { weekly_off: vec![chrono::Weekday::Sat, chrono::Weekday::Sun], ..Calendar::default() }
    }

    #[test]
    fn date_and_month_default_forms() {
        let day = d("2026-10-30");
        assert_eq!(expand("a-{date}-{month}", day, None).unwrap(), "a-2026-10-30-202610");
        assert_eq!(expand("{date:%Y%m%d}", day, None).unwrap(), "20261030");
    }

    #[test]
    fn months_cross_the_year_and_clamp() {
        assert_eq!(expand("{month-1}", d("2026-01-15"), None).unwrap(), "202512");
        assert_eq!(expand("{month+2:%Y-%m}", d("2026-11-30"), None).unwrap(), "2027-01");
        assert_eq!(expand("{date-1m}", d("2026-03-31"), None).unwrap(), "2026-02-28");
        assert_eq!(expand("{date-1y:%Y}", d("2026-03-31"), None).unwrap(), "2025");
        assert_eq!(expand("{date+7d}", d("2026-10-30"), None).unwrap(), "2026-11-06");
    }

    #[test]
    fn business_days_use_the_calendar() {
        let c = cal();
        // 2026-11-02 is a Monday: one business day back is Friday the 30th.
        assert_eq!(expand("{date-1bd:%m%d}", d("2026-11-02"), Some(&c)).unwrap(), "1030");
        assert!(expand("{date-1bd}", d("2026-11-02"), None).unwrap_err().contains("calendar が要る"));
        assert!(needs_calendar("x {date+2bd}") && !needs_calendar("{date-1d}"));
    }

    #[test]
    fn other_braces_are_left_alone() {
        let ps = "if ($x) { $h = @{ a = 1 }; \"{0}\" -f $h }";
        assert_eq!(expand(ps, d("2026-10-30"), None).unwrap(), ps);
        assert_eq!(expand("{dated} {months} {", d("2026-10-30"), None).unwrap(), "{dated} {months} {");
    }

    #[test]
    fn bad_tokens_are_named() {
        for bad in ["{date-}", "{date-1}", "{date-1x}", "{month-1d}", "{date:}", "{date:%Q}"] {
            assert!(check(bad).is_err(), "{bad}");
        }
        assert!(check("{date-1bd:%Y%m} {month}").is_ok());
    }
}
