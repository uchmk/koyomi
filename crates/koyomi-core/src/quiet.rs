//! The hours a net does not start in (`quiet = ["12:00-13:00"]`): a time of
//! its `[[when]]` that comes then waits until the hours are over
//! (`on_quiet = "wait"`, the default) or is written down as skipped
//! (`on_quiet = "skip"`). A span whose end is before its start goes past
//! midnight (`"22:00-02:00"`).

use chrono::{Duration, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

/// One span of quiet hours: from `from`, up to but not at `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub from: NaiveTime,
    pub to: NaiveTime,
}

impl Span {
    /// `"12:00-13:00"`.
    pub fn parse(text: &str) -> Result<Span, String> {
        let bad = || format!("\"12:00-13:00\" のように書く: \"{text}\"");
        let (a, b) = text.split_once('-').ok_or_else(bad)?;
        let (from, to) = (crate::when::time(a).map_err(|_| bad())?, crate::when::time(b).map_err(|_| bad())?);
        if from == to {
            return Err(format!("始まりと終わりが同じ: \"{text}\""));
        }
        Ok(Span { from, to })
    }

    /// As the file writes it.
    pub fn text(&self) -> String {
        let t = |t: NaiveTime| crate::late::Deadline::At(t).text();
        format!("{}-{}", t(self.from), t(self.to))
    }

    /// Whether the time of day `t` is in it.
    pub fn contains(&self, t: NaiveTime) -> bool {
        if self.from < self.to {
            self.from <= t && t < self.to
        } else {
            t >= self.from || t < self.to
        }
    }

    /// The end of it for `at`, which is in it.
    fn end(&self, at: NaiveDateTime) -> NaiveDateTime {
        let same = at.date().and_time(self.to);
        if same > at {
            same
        } else {
            same + Duration::days(1)
        }
    }
}

/// `on_quiet`: what a time that comes in the quiet hours does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnQuiet {
    /// Starts when the hours are over.
    #[default]
    Wait,
    /// Written down as スキップ.
    Skip,
}

/// When the quiet hours `at` falls in are over (spans that meet or overlap count as one);
/// `None` when `at` is in none of them.
pub fn until(spans: &[Span], at: NaiveDateTime) -> Option<NaiveDateTime> {
    let mut end = spans.iter().find(|s| s.contains(at.time()))?.end(at);
    // Each turn moves the end into the next span, if it is in one; one turn per span at the most.
    for _ in spans {
        match spans.iter().find(|s| s.contains(end.time())) {
            Some(s) => end = s.end(end),
            None => break,
        }
    }
    Some(end)
}

/// The span `at` falls in, for the record (`12:00-13:00`).
pub fn named(spans: &[Span], at: NaiveDateTime) -> Option<String> {
    spans.iter().find(|s| s.contains(at.time())).map(Span::text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn spans(list: &[&str]) -> Vec<Span> {
        list.iter().map(|s| Span::parse(s).unwrap()).collect()
    }

    #[test]
    fn a_span_reads_and_writes_back() {
        assert_eq!(Span::parse("12:00-13:00").unwrap().text(), "12:00-13:00");
        assert_eq!(Span::parse(" 22:00 - 02:30 ").unwrap().text(), "22:00-02:30");
        assert!(Span::parse("12:00").is_err());
        assert!(Span::parse("12:00-25:00").is_err());
        assert!(Span::parse("12:00-12:00").is_err());
    }

    #[test]
    fn a_time_in_the_hours_waits_until_their_end() {
        let s = spans(&["12:00-13:00"]);
        assert_eq!(until(&s, at("2026-10-30 11:59")), None);
        assert_eq!(until(&s, at("2026-10-30 12:00")), Some(at("2026-10-30 13:00")));
        assert_eq!(until(&s, at("2026-10-30 12:30")), Some(at("2026-10-30 13:00")));
        assert_eq!(until(&s, at("2026-10-30 13:00")), None, "the end is not in it");
    }

    #[test]
    fn a_span_past_midnight_ends_the_next_morning() {
        let s = spans(&["22:00-02:00"]);
        assert_eq!(until(&s, at("2026-10-30 23:00")), Some(at("2026-10-31 02:00")));
        assert_eq!(until(&s, at("2026-10-31 01:00")), Some(at("2026-10-31 02:00")));
        assert_eq!(until(&s, at("2026-10-31 12:00")), None);
    }

    #[test]
    fn spans_that_meet_are_one() {
        let s = spans(&["12:00-13:00", "13:00-13:30", "18:00-19:00"]);
        assert_eq!(until(&s, at("2026-10-30 12:10")), Some(at("2026-10-30 13:30")));
        assert_eq!(named(&s, at("2026-10-30 13:10")).as_deref(), Some("13:00-13:30"));
        let round = spans(&["00:00-12:00", "12:00-00:00"]);
        assert!(until(&round, at("2026-10-30 08:00")).is_some(), "the whole day ends somewhere, not in a loop");
    }

    #[test]
    fn the_keys_read_and_survive_a_round_trip() {
        let text = "format = 1
quiet = [\"12:00-13:00\", \"22:00-02:00\"]
on_quiet = \"skip\"

[[job]]
id = \"a\"
run = [\"x\"]
";
        let n = crate::jobnet::parse(text).unwrap();
        assert_eq!(n.quiet, spans(&["12:00-13:00", "22:00-02:00"]));
        assert_eq!(n.on_quiet, OnQuiet::Skip);
        assert_eq!(crate::jobnet::parse(&crate::write::text(&n)).unwrap(), n);
        let plain = crate::jobnet::parse(
            "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap();
        assert_eq!((plain.quiet.len(), plain.on_quiet), (0, OnQuiet::Wait));
        let bad = crate::jobnet::parse(
            "format = 1
quiet = [\"12:00\", 3]
on_quiet = \"later\"
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap_err();
        let msgs: Vec<_> = bad.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(msgs.len(), 3, "{msgs:?}");
        assert!(msgs.iter().all(|m| m.starts_with("quiet: ") || m.starts_with("on_quiet ")), "{msgs:?}");
    }
}
