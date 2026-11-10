//! The hours a net does not start in (`quiet = ["12:00-13:00"]`): a time of
//! its `[[when]]` that comes then waits until the hours are over
//! (`on_quiet = "wait"`, the default) or is written down as skipped
//! (`on_quiet = "skip"`). A span whose end is before its start goes past
//! midnight (`"22:00-02:00"`).

use chrono::{Duration, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::msg;

/// One span of quiet hours: from `from`, up to but not at `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub from: NaiveTime,
    pub to: NaiveTime,
}

impl Span {
    /// `"12:00-13:00"`.
    pub fn parse(text: &str) -> Result<Span, String> {
        let bad = || msg::line(msg::QUIET_SPAN_BAD, &[text]);
        let (a, b) = text.split_once('-').ok_or_else(bad)?;
        let (from, to) = (crate::when::time(a).map_err(|_| bad())?, crate::when::time(b).map_err(|_| bad())?);
        if from == to {
            return Err(msg::line(msg::QUIET_SPAN_SAME, &[text]));
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

/// When a time `at` of a net starts under its quiet hours: at their end
/// (`wait`), not at all (`skip`), or at `at` when it is in none of them.
pub fn start(spans: &[Span], on: OnQuiet, at: NaiveDateTime) -> Option<NaiveDateTime> {
    match until(spans, at) {
        None => Some(at),
        Some(end) => (on == OnQuiet::Wait).then_some(end),
    }
}

/// The starts of `times` (earliest first) under the quiet hours, as the engine
/// makes them: each moved or dropped by [`start`], and the times that wait for
/// the same end counted once (the later ones in the same hours are skipped).
pub fn starts(spans: &[Span], on: OnQuiet, times: impl IntoIterator<Item = NaiveDateTime>) -> Vec<NaiveDateTime> {
    let mut out: Vec<NaiveDateTime> = Vec::new();
    for s in times.into_iter().filter_map(|at| start(spans, on, at)) {
        if out.last() != Some(&s) {
            out.push(s);
        }
    }
    out
}

/// The next `n` starts after `after` of a net's `[[when]]` under its quiet
/// hours ([`starts`]). Times are read in rounds, so hours that drop many of
/// them still give `n` when there are that many; a net that never leaves its
/// quiet hours gives none.
pub fn next_starts(
    whens: &[crate::when::When],
    spans: &[Span],
    on: OnQuiet,
    after: NaiveDateTime,
    n: usize,
    cal: Option<&crate::calendar::Calendar>,
) -> Vec<NaiveDateTime> {
    if spans.is_empty() {
        return crate::when::next_times_in(whens, after, n, cal);
    }
    let mut out: Vec<NaiveDateTime> = Vec::new();
    let mut cursor = after;
    for _ in 0..64 {
        let times = crate::when::next_times_in(whens, cursor, n.max(16), cal);
        let Some(&last) = times.last() else { break };
        for s in starts(spans, on, times) {
            if out.last().is_none_or(|l| *l < s) {
                out.push(s);
            }
        }
        if out.len() >= n {
            break;
        }
        cursor = last;
    }
    out.truncate(n);
    out
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
    fn the_starts_wait_once_per_hours_or_are_dropped() {
        let s = spans(&["12:00-13:00"]);
        let times = ["11:30", "12:00", "12:30", "13:30"].map(|h| at(&format!("2026-10-30 {h}")));
        let wait = starts(&s, OnQuiet::Wait, times);
        assert_eq!(wait, ["11:30", "13:00", "13:30"].map(|h| at(&format!("2026-10-30 {h}"))));
        let skip = starts(&s, OnQuiet::Skip, times);
        assert_eq!(skip, ["11:30", "13:30"].map(|h| at(&format!("2026-10-30 {h}"))));
        assert_eq!(start(&[], OnQuiet::Skip, times[1]), Some(times[1]));
    }

    #[test]
    fn the_next_starts_look_past_hours_that_drop_many_times() {
        let net = crate::jobnet::parse(
            "format = 1
quiet = [\"00:00-23:00\"]
on_quiet = \"skip\"
[[when]]
every = \"minutes\"
n = 10
from = \"00:00\"
to = \"23:59\"
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap();
        let got = next_starts(&net.when, &net.quiet, net.on_quiet, at("2026-10-30 00:00"), 3, None);
        assert_eq!(got, ["23:00", "23:10", "23:20"].map(|h| at(&format!("2026-10-30 {h}"))));
        let all = spans(&["00:00-12:00", "12:00-00:00"]);
        assert!(next_starts(&net.when, &all, OnQuiet::Skip, at("2026-10-30 00:00"), 3, None).is_empty());
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
        assert!(msgs.iter().all(|m| crate::msg::is_coded(m) && m.contains("quiet")), "{msgs:?}");
        // The bad time, the bad element and the bad `on_quiet` are each told, `on_quiet` by its own name.
        assert!(msgs.iter().any(|m| crate::msg::DEF_CHOICE_BAD.is_in(m) && m.contains("on_quiet")), "{msgs:?}");
        assert!(msgs.iter().any(|m| crate::msg::DEF_VALUE_UNREADABLE.is_in(m) && m.contains("12:00")), "{msgs:?}");
    }
}
