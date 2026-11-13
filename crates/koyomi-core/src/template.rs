//! `{date}` and friends in a job's `run`, `script`, `dir` and `env` values: the business
//! day of the run, moved and written the way the job wants it, so a monthly
//! job gets its month from koyomi and not from its own arithmetic.
//!
//! - `{date}` is `2026-10-30`; `{date:%Y%m%d}` writes it with a chrono format.
//! - `{date-1d}`, `{date+7d}`, `{date-1m}`, `{date-1y}` move it by days, months or years;
//!   `{date-1bd}` by business days (the net's calendar; without one it is an error).
//! - `{month}` is `202610`, `{month-1}` the month before (`{month-1:%Y-%m}` to write it another way).
//!
//! - `{param:month}` is the value of `month` in the net's `[params]`: its default, or what
//!   `koyomi run --set month=2026-09` (or the window's box) gave. It is put in first, so a
//!   default may itself be `{month-1:%Y-%m}`.
//! - `{var:share}` is the value of `share` in koyomi.toml's `[vars]`, the same for every net.
//!   It is put in after the `{param:…}`, so a default may be `{var:share}\in`, and before
//!   the day's tokens, so a value may hold `{date}`.
//!
//! A `{` that does not start one of these is left as it is (PowerShell scripts are full of braces).

use std::collections::{BTreeMap, BTreeSet};

use chrono::format::{Item, StrftimeItems};
use chrono::{Months, NaiveDate};

use crate::calendar::Calendar;
use crate::jobnet::{Jobnet, Part};
use crate::msg;

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
                let cal = cal.ok_or_else(|| msg::line(msg::TEMPLATE_NO_CALENDAR, &[src]))?;
                cal.shift(date, n)
            }
        };
        let moved = moved.ok_or_else(|| msg::line(msg::TEMPLATE_NO_DAY, &[src, &date.to_string()]))?;
        let format = t.format.as_deref().unwrap_or(if t.month { "%Y%m" } else { "%Y-%m-%d" });
        Ok(moved.format(format).to_string())
    })
}

const PARAM: &str = "{param:";
const VAR: &str = "{var:";

/// The names of the `{param:…}` in `text`, in order (an empty one as `""`).
pub fn param_names(text: &str) -> Vec<&str> {
    names(text, PARAM)
}

/// The names of the `{var:…}` in `text`, in order (an empty one as `""`).
pub fn var_names(text: &str) -> Vec<&str> {
    names(text, VAR)
}

fn names<'a>(text: &'a str, open: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(open) {
        let tail = &rest[at + open.len()..];
        let Some(end) = tail.find('}') else { break };
        out.push(&tail[..end]);
        rest = &tail[end + 1..];
    }
    out
}

/// Whether every `{var:…}` in `text` has a name `[vars]` can hold.
pub fn check_vars(text: &str) -> Result<(), String> {
    match var_names(text).into_iter().find(|n| !crate::jobnet::plain_name(n)) {
        Some("") => Err(msg::line(msg::VAR_NAME_EMPTY, &[])),
        Some(n) => Err(msg::line(msg::VAR_NAME_BAD, &[n])),
        None => Ok(()),
    }
}

/// `text` with every `{param:…}` replaced by its value in `values`.
pub fn fill(text: &str, values: &BTreeMap<String, String>) -> Result<String, String> {
    put(text, PARAM, values, msg::PARAM_MISSING)
}

/// `text` with every `{var:…}` replaced by its value in koyomi.toml's `[vars]`.
pub fn fill_vars(text: &str, vars: &BTreeMap<String, String>) -> Result<String, String> {
    put(text, VAR, vars, msg::VAR_MISSING)
}

fn put(text: &str, open: &str, values: &BTreeMap<String, String>, missing: msg::Id) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(open) {
        let tail = &rest[at + open.len()..];
        let Some(end) = tail.find('}') else { break };
        let name = &tail[..end];
        let v = values.get(name).ok_or_else(|| msg::line(missing, &[name]))?;
        out.push_str(&rest[..at]);
        out.push_str(v);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The net's `[params]` with `set` laid over them: a name `set` gives that the net does not have is an error.
pub fn values(net: &Jobnet, set: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, String> {
    let mut out = net.params.clone();
    for (k, v) in set {
        match out.get_mut(k) {
            Some(slot) => *slot = v.clone(),
            None if net.params.is_empty() => return Err(msg::line(msg::PARAMS_NONE, &[k])),
            None => {
                let known: Vec<&str> = net.params.keys().map(String::as_str).collect();
                return Err(msg::line(msg::PARAM_UNKNOWN, &[k, &known.join(&msg::note(msg::N_LIST_SEP, &[]))]));
            }
        }
    }
    Ok(out)
}

/// The net with every job's `run`, `script`, `env`, `dir`, `judge.ok_file` and its part's paths
/// written out: first the `{param:…}` from `values` ([`values`]), then the `{var:…}` from
/// koyomi.toml's `vars`, then the day's tokens for `date`.
pub fn expand_net(
    net: &Jobnet,
    date: NaiveDate,
    cal: Option<&Calendar>,
    values: &BTreeMap<String, String>,
    vars: &BTreeMap<String, String>,
) -> Result<Jobnet, String> {
    let mut out = net.clone();
    each_text(&mut out, &mut |id, what, t| {
        *t = fill(t, values)
            .and_then(|t| fill_vars(&t, vars))
            .and_then(|t| expand(&t, date, cal))
            .map_err(|e| msg::line(msg::TEMPLATE_EXPAND, &[id, what, msg::uncoded(&e)]))?;
        Ok(())
    })?;
    Ok(out)
}

/// The names of the `{var:…}` the net reads, in its jobs and its `[params]`' defaults.
pub fn vars_of(net: &Jobnet) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = net.params.values().flat_map(|v| var_names(v)).map(str::to_owned).collect();
    let _ = each_text(&mut net.clone(), &mut |_, _, t| {
        out.extend(var_names(t).into_iter().map(str::to_owned));
        Ok(())
    });
    out
}

/// Calls `f` with the job's id, the field's name and each text [`expand_net`] writes out
/// (an `env` value that is `secret:…` is not one).
fn each_text(net: &mut Jobnet, f: &mut dyn FnMut(&str, &str, &mut String) -> Result<(), String>) -> Result<(), String> {
    for j in &mut net.jobs {
        let id = j.id.as_str();
        for a in &mut j.run {
            f(id, "run", a)?;
        }
        if let Some(s) = &mut j.script {
            f(id, "script", s)?;
        }
        for (k, v) in &mut j.env {
            if !v.starts_with("secret:") {
                f(id, &format!("env {k}"), v)?;
            }
        }
        if let Some(d) = &mut j.dir {
            f(id, "dir", d)?;
        }
        if let Some(o) = j.judge.as_mut().and_then(|g| g.ok_file.as_mut()) {
            f(id, "judge ok_file", o)?;
        }
        if let Part::Chore(c) = &mut j.part {
            for (name, p) in c.paths_mut() {
                f(id, name, p)?;
            }
        }
        if let Part::File { file, .. } = &mut j.part {
            f(id, "file", file)?;
        }
    }
    Ok(())
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
        match parse(&src[1..end], src) {
            None => {
                out.push('{');
                rest = &tail[1..];
            }
            Some(Err(e)) => return Err(e),
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
fn parse(inner: &str, src: &str) -> Option<Result<Token, String>> {
    let (head, format) = match inner.split_once(':') {
        Some((h, f)) => (h, Some(f)),
        None => (inner, None),
    };
    let (month, rest) = if let Some(r) = head.strip_prefix("date") { (false, r) } else { (true, head.strip_prefix("month")?) };
    if !(rest.is_empty() || rest.starts_with(['+', '-'])) {
        return None;
    }
    Some(token(month, rest, format, src))
}

fn token(month: bool, rest: &str, format: Option<&str>, src: &str) -> Result<Token, String> {
    let (n, unit) = if rest.is_empty() {
        (0, if month { Unit::Month } else { Unit::Day })
    } else {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let body = &rest[1..];
        let digits = body.chars().take_while(char::is_ascii_digit).count();
        let n: i32 = body[..digits].parse().map_err(|_| msg::line(msg::TOKEN_NO_NUMBER, &[src]))?;
        let unit = match (&body[digits..], month) {
            ("", true) | ("m", _) => Unit::Month,
            ("d", false) => Unit::Day,
            ("bd", false) => Unit::Business,
            ("y", false) => Unit::Year,
            ("", false) => return Err(msg::line(msg::TOKEN_NO_UNIT, &[src])),
            (u, _) => return Err(msg::line(msg::TOKEN_UNIT_BAD, &[src, u])),
        };
        (sign * n, unit)
    };
    if let Some(f) = format {
        if f.is_empty() || StrftimeItems::new(f).any(|i| matches!(i, Item::Error)) {
            return Err(msg::line(msg::TOKEN_FORMAT_BAD, &[src, f]));
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
        assert!(expand("{date-1bd}", d("2026-11-02"), None).is_err_and(|e| msg::TEMPLATE_NO_CALENDAR.is_in(&e)));
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

    #[test]
    fn params_go_in_first_and_may_hold_date_tokens() {
        let net = Jobnet { params: BTreeMap::from([("m".to_owned(), "{month-1:%Y-%m}".to_owned())]), ..Jobnet::default() };
        let day = d("2026-10-30");
        let dflt = values(&net, &BTreeMap::new()).unwrap();
        assert_eq!(expand(&fill("x{param:m}.csv", &dflt).unwrap(), day, None).unwrap(), "x2026-09.csv");
        let set = values(&net, &BTreeMap::from([("m".to_owned(), "2026-07".to_owned())])).unwrap();
        assert_eq!(fill("{param:m} {0}", &set).unwrap(), "2026-07 {0}");
        assert!(msg::PARAM_MISSING.is_in(&fill("{param:n}", &set).unwrap_err()));
        assert!(values(&net, &BTreeMap::from([("n".to_owned(), "1".to_owned())])).unwrap_err().contains("定義されているのは m"));
        assert_eq!(param_names("a {param:m} {param:} {date}"), ["m", ""]);
    }

    /// `{var:…}`: koyomi.toml's `[vars]`, after the `{param:…}` and before the day's tokens.
    #[test]
    fn vars_go_in_after_params_and_before_the_day() {
        let vars = BTreeMap::from([("share".to_owned(), r"\\nas\backup\{date:%Y%m}".to_owned())]);
        let params = BTreeMap::from([("into".to_owned(), r"{var:share}\in".to_owned())]);
        let mut net = Jobnet { params, ..Jobnet::default() };
        let run = vec!["copy".to_owned(), "{param:into}".to_owned(), "{var:share}".to_owned()];
        net.jobs.push(crate::jobnet::Job { id: "a".into(), run, ..crate::jobnet::Job::default() });
        let dflt = values(&net, &BTreeMap::new()).unwrap();
        let out = expand_net(&net, d("2026-11-02"), None, &dflt, &vars).unwrap();
        assert_eq!(out.jobs[0].run, ["copy", r"\\nas\backup\202611\in", r"\\nas\backup\202611"]);
        let e = expand_net(&net, d("2026-11-02"), None, &dflt, &BTreeMap::new()).unwrap_err();
        assert!(
            msg::TEMPLATE_EXPAND.is_in(&e)
                && e.contains("ジョブ a の run を展開できません")
                && e.contains("{var:share} の値が、koyomi.toml の [vars] にありません"),
            "{e}"
        );
        assert_eq!(vars_of(&net).into_iter().collect::<Vec<_>>(), ["share"]);
        assert_eq!(var_names("{var:a} {var:} {param:b}"), ["a", ""]);
        assert!(check_vars("{var:a_1} {date}").is_ok());
        assert!(msg::VAR_NAME_EMPTY.is_in(&check_vars("{var:}").unwrap_err()));
        assert!(msg::VAR_NAME_BAD.is_in(&check_vars("{var:共有}").unwrap_err()));
    }

    #[test]
    fn a_file_waits_path_is_expanded_and_its_vars_are_listed() {
        use crate::jobnet::{Job, Part};
        let job = Job { id: "w".into(), part: Part::File { file: r"{var:in}\{date:%Y%m%d}.csv".into(), settle: None }, ..Job::default() };
        let net = Jobnet { jobs: vec![job], ..Jobnet::default() };
        assert_eq!(vars_of(&net).into_iter().collect::<Vec<_>>(), ["in"]);
        let vars = BTreeMap::from([("in".to_owned(), r"C:\in".to_owned())]);
        let out = expand_net(&net, d("2026-10-30"), None, &BTreeMap::new(), &vars).unwrap();
        assert_eq!(out.jobs[0].part, Part::File { file: r"C:\in\20261030.csv".into(), settle: None });
        let e = expand_net(&net, d("2026-10-30"), None, &BTreeMap::new(), &BTreeMap::new()).unwrap_err();
        assert!(msg::TEMPLATE_EXPAND.is_in(&e) && e.contains("file"), "{e}");
    }
}
