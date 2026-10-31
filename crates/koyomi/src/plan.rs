//! `koyomi plan`: what starts on each day of a month, as text, a PDF or a sheet for the printer.
//!
//! Only the definitions the person approved count (the same ones that start by themselves), and
//! each counts by its own calendar, so the sheet shows what the engine would do.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use koyomi_core::calendar::Calendar;
use koyomi_core::scan::{Def, Standing};
use koyomi_core::when;

/// One day: its date and the times that start, earliest first.
pub type Day = (NaiveDate, Vec<(NaiveDateTime, String)>);

/// A4 in CSS pixels, and how many lines a page holds.
const PAGE: (u32, u32) = (794, 1123);
const LINE: u32 = 20;
const TOP: u32 = 110;
const PER_PAGE: u32 = (PAGE.1 - TOP - 60) / LINE;

/// Every day of the month with what starts on it. Days with nothing are kept (an empty list).
pub fn month(defs: &[Def], calendars: &BTreeMap<String, Calendar>, year: i32, m: u32) -> Vec<Day> {
    let Some(first) = NaiveDate::from_ymd_opt(year, m, 1) else { return Vec::new() };
    let mut days: Vec<Day> = Vec::new();
    let mut d = first;
    while d.month() == m {
        days.push((d, Vec::new()));
        let Some(n) = d.succ_opt() else { break };
        d = n;
    }
    fill(defs, calendars, days)
}

/// One day with what starts on it, as [`month`] counts it.
pub fn day(defs: &[Def], calendars: &BTreeMap<String, Calendar>, d: NaiveDate) -> Day {
    fill(defs, calendars, vec![(d, Vec::new())]).pop().unwrap_or((d, Vec::new()))
}

/// The days given, in order and with none missing, with the approved starts on each.
fn fill(defs: &[Def], calendars: &BTreeMap<String, Calendar>, mut days: Vec<Day>) -> Vec<Day> {
    let Some((first, _)) = days.first() else { return days };
    let start = first.and_hms_opt(0, 0, 0).unwrap_or_default() - chrono::Duration::seconds(1);
    let end = days.last().map_or(start, |(d, _)| d.and_hms_opt(23, 59, 59).unwrap_or_default());
    for def in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
        let Some(net) = &def.net else { continue };
        let cal = def.calendar.as_deref().and_then(|c| calendars.get(c));
        for at in koyomi_core::quiet::starts(&net.quiet, net.on_quiet, when::between_in(&net.when, start, end, cal)) {
            if let Some((_, list)) = days.iter_mut().find(|(d, _)| *d == at.date()) {
                list.push((at, def.label().to_owned()));
            }
        }
    }
    for (_, list) in &mut days {
        list.sort();
    }
    days
}

/// The month as lines: `YYYY-MM-DD  HH:MM  name`, a day without a start as one line alone.
pub fn text(days: &[Day]) -> String {
    let mut s = String::new();
    for (d, list) in days {
        if list.is_empty() {
            let _ = writeln!(s, "{d}  -");
        }
        for (at, name) in list {
            let _ = writeln!(s, "{d}  {}  {name}", at.format("%H:%M"));
        }
    }
    s
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The month as A4 pages of SVG, the title on each.
pub fn svg(days: &[Day], year: i32, m: u32) -> Vec<String> {
    let mut lines: Vec<(String, bool)> = Vec::new();
    for (d, list) in days {
        let wd = ["月", "火", "水", "木", "金", "土", "日"][d.weekday().num_days_from_monday() as usize];
        if list.is_empty() {
            lines.push((format!("{:02}({wd})   -", d.day()), false));
        }
        for (i, (at, name)) in list.iter().enumerate() {
            let day = if i == 0 { format!("{:02}({wd})", d.day()) } else { "      ".to_owned() };
            lines.push((format!("{day}   {}   {name}", at.format("%H:%M")), true));
        }
    }
    let chunks: Vec<&[(String, bool)]> = lines.chunks(PER_PAGE as usize).collect();
    let total = chunks.len().max(1);
    let mut pages = Vec::new();
    for n in 0..total {
        let mut s = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n<rect width=\"100%\" height=\"100%\" fill=\"#fff\"/>\n",
            PAGE.0, PAGE.1, PAGE.0, PAGE.1
        );
        let _ = writeln!(
            s,
            "<text x=\"48\" y=\"64\" font-family=\"sans-serif\" font-size=\"24\" fill=\"#000\">{year}年{m}月 実行予定表</text>"
        );
        let _ = writeln!(s, "<line x1=\"48\" y1=\"80\" x2=\"{}\" y2=\"80\" stroke=\"#000\"/>", PAGE.0 - 48);
        for (i, (line, on)) in chunks.get(n).copied().unwrap_or_default().iter().enumerate() {
            let fill = if *on { "#000" } else { "#777" };
            let y = TOP + i as u32 * LINE;
            let _ = writeln!(
                s,
                "<text x=\"48\" y=\"{y}\" xml:space=\"preserve\" font-family=\"monospace\" font-size=\"14\" fill=\"{fill}\">{}</text>",
                esc(line)
            );
        }
        let _ = writeln!(
            s,
            "<text x=\"{}\" y=\"{}\" text-anchor=\"end\" font-family=\"sans-serif\" font-size=\"12\" fill=\"#555\">{} / {total}</text>",
            PAGE.0 - 48,
            PAGE.1 - 32,
            n + 1
        );
        s.push_str("</svg>\n");
        pages.push(s);
    }
    pages
}

/// The PDF of the month written to `out`.
pub fn pdf(days: &[Day], year: i32, m: u32, out: &std::path::Path) -> Result<(), String> {
    ito_export::pdf(&svg(days, year, m), &ito_export::Fonts::system(), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32, times: &[(u32, &str)]) -> Day {
        let date = NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
        (date, times.iter().map(|(h, n)| (date.and_hms_opt(*h, 0, 0).unwrap(), (*n).to_owned())).collect())
    }

    #[test]
    fn text_lists_each_start_and_marks_empty_days() {
        let t = text(&[day(1, &[(9, "a"), (17, "b")]), day(2, &[])]);
        assert_eq!(t, "2026-01-01  09:00  a\n2026-01-01  17:00  b\n2026-01-02  -\n");
    }

    #[test]
    fn svg_escapes_and_paginates() {
        let many: Vec<Day> = (1..=31).map(|d| day(d, &[(9, "a&b"), (10, "c"), (11, "d")])).collect();
        let pages = svg(&many, 2026, 1);
        assert!(pages.len() >= 2);
        assert!(pages[0].contains("a&amp;b"));
        assert!(pages[0].contains("1 / "));
        assert_eq!(svg(&[], 2026, 1).len(), 1);
    }

    #[test]
    fn month_without_nets_has_every_day() {
        let days = month(&[], &BTreeMap::new(), 2026, 2);
        assert_eq!(days.len(), 28);
        assert!(days.iter().all(|(_, l)| l.is_empty()));
    }

    #[test]
    fn one_day_holds_what_the_month_holds_for_it_and_only_the_approved() {
        let text = "format = 1
[[when]]
every = \"day\"
at = [\"17:00\", \"02:00\"]
[[job]]
id = \"a\"
run = [\"x\"]
";
        let net = |id: &str, standing| Def {
            id: id.to_owned(),
            group: "g".to_owned(),
            name: id.to_owned(),
            title: None,
            file: format!("{id}.toml"),
            standing,
            problems: Vec::new(),
            net: Some(koyomi_core::jobnet::parse(text).unwrap()),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        };
        let defs = [net("yes", Standing::Confirmed), net("no", Standing::Unconfirmed)];
        let d = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let one = super::day(&defs, &BTreeMap::new(), d);
        let times: Vec<String> = one.1.iter().map(|(at, n)| format!("{} {n}", at.format("%H:%M"))).collect();
        assert_eq!(times, ["02:00 yes", "17:00 yes"]);
        let from_month = month(&defs, &BTreeMap::new(), 2026, 11).into_iter().find(|(x, _)| *x == d).unwrap();
        assert_eq!(one, from_month);
    }
}

#[cfg(test)]
mod pdf_tests {
    use super::*;

    #[test]
    fn writes_a_pdf() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let days = vec![(date, vec![(date.and_hms_opt(9, 0, 0).unwrap(), "月次".to_owned())])];
        let out = std::env::temp_dir().join(format!("koyomi-plan-test-{}.pdf", std::process::id()));
        pdf(&days, 2026, 1, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        assert!(bytes.starts_with(b"%PDF"));
    }
}
