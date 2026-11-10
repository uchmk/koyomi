//! The planned stops (`[[blackout]]` in `koyomi.toml`): a stretch of days or hours in which no net
//! starts by its schedule or an event, for maintenance, a power cut or the year's end. A time that
//! comes in one is written down as スキップ with why (`計画停止（…）`), and it is not caught up
//! afterwards. A run already going when one begins goes on; a net run by hand still runs.
//!
//! ```toml
//! [[blackout]]
//! from = "2026-12-29"          # 00:00 of that day
//! to = "2027-01-03"            # through that day (up to 00:00 of the next)
//! why = "年末年始"
//!
//! [[blackout]]
//! from = "2026-11-14 22:00"
//! to = "2026-11-15 06:00"      # up to, not at
//! why = "電源工事"
//! ```

use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::msg;

/// One planned stop, as written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blackout {
    /// `"2026-12-29"` (from its 00:00) or `"2026-12-29 22:00"`.
    pub from: String,
    /// `"2027-01-03"` (through that day) or `"2027-01-03 06:00"` (up to, not at).
    pub to: String,
    /// What it is for, shown with every time it stops.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// A day alone, or a day and a time of it; `None` when it is neither.
fn moment(text: &str) -> Option<(NaiveDateTime, bool)> {
    let text = text.trim();
    if let Ok(d) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some((d.and_time(NaiveTime::MIN), false));
    }
    ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"].iter().find_map(|f| NaiveDateTime::parse_from_str(text, f).ok()).map(|t| (t, true))
}

impl Blackout {
    /// From when, up to (not at) when.
    pub fn span(&self) -> Result<(NaiveDateTime, NaiveDateTime), String> {
        let bad = |what: &str, text: &str| msg::line(msg::BLACKOUT_TIME_BAD, &[what, text]);
        let (from, _) = moment(&self.from).ok_or_else(|| bad("from", &self.from))?;
        let (to, timed) = moment(&self.to).ok_or_else(|| bad("to", &self.to))?;
        // A day alone is through that day.
        let to = if timed { to } else { to.checked_add_days(Days::new(1)).ok_or_else(|| bad("to", &self.to))? };
        if to <= from {
            return Err(msg::line(msg::BLACKOUT_ORDER, &[self.to.trim(), self.from.trim()]));
        }
        Ok((from, to))
    }

    /// The file's check: both ends read and `to` is after `from`.
    pub fn check(&self) -> Result<(), String> {
        self.span().map(|_| ())
    }

    /// Whether `at` is in it.
    pub fn contains(&self, at: NaiveDateTime) -> bool {
        self.span().is_ok_and(|(from, to)| from <= at && at < to)
    }

    /// Whether any of `day` is in it.
    pub fn touches(&self, day: NaiveDate) -> bool {
        let start = day.and_time(NaiveTime::MIN);
        let end = start + chrono::Duration::days(1);
        self.span().is_ok_and(|(from, to)| from < end && start < to)
    }

    /// Whether all of `day` is in it.
    pub fn covers(&self, day: NaiveDate) -> bool {
        let start = day.and_time(NaiveTime::MIN);
        let end = start + chrono::Duration::days(1);
        self.span().is_ok_and(|(from, to)| from <= start && end <= to)
    }

    /// `2026-12-29〜2027-01-03` or `2026-11-14 22:00〜2026-11-15 06:00`, as written.
    pub fn when(&self) -> String {
        format!("{}〜{}", self.from.trim(), self.to.trim())
    }

    /// Why a time in it does not start, for the record: `計画停止（年末年始、2026-12-29〜2027-01-03）`.
    pub fn reason(&self) -> String {
        match self.why.trim() {
            "" => msg::note(msg::N_BLACKOUT_REASON, &[&self.when()]),
            why => msg::note(msg::N_BLACKOUT_REASON_WHY, &[why, &self.when()]),
        }
    }
}

/// The stop `at` is in, if any.
pub fn find(list: &[Blackout], at: NaiveDateTime) -> Option<&Blackout> {
    list.iter().find(|b| b.contains(at))
}

/// The stop any of `day` is in, if any.
pub fn on_day(list: &[Blackout], day: NaiveDate) -> Option<&Blackout> {
    list.iter().find(|b| b.touches(day))
}

/// The stop going on at `now`, or else the soonest to begin within `days` days of it.
pub fn near(list: &[Blackout], now: NaiveDateTime, days: u64) -> Option<&Blackout> {
    if let Some(b) = find(list, now) {
        return Some(b);
    }
    let horizon = now + chrono::Duration::days(days as i64);
    list.iter()
        .filter_map(|b| b.span().ok().map(|(from, _)| (from, b)))
        .filter(|(from, _)| *from > now && *from <= horizon)
        .min_by_key(|(from, _)| *from)
        .map(|(_, b)| b)
}

/// The first `n` times after `after` that are in none of `list`, from `next` (the next `k` times
/// after a moment, in order: `quiet::next_starts` and the like). A stop is stepped over whole,
/// so a long one with many times in it costs one round.
pub fn outside(
    list: &[Blackout],
    after: NaiveDateTime,
    n: usize,
    mut next: impl FnMut(NaiveDateTime, usize) -> Vec<NaiveDateTime>,
) -> Vec<NaiveDateTime> {
    if list.is_empty() {
        return next(after, n);
    }
    let mut out: Vec<NaiveDateTime> = Vec::new();
    let mut cursor = after;
    for _ in 0..64 {
        let times = next(cursor, n - out.len());
        let Some(&last) = times.last() else { break };
        for t in times {
            if find(list, t).is_none() && out.last().is_none_or(|l| *l < t) {
                out.push(t);
            }
        }
        if out.len() >= n {
            break;
        }
        // On from the end of the stop the last time is in (a moment before it, as `next` is after).
        cursor = match find(list, last).and_then(|b| b.span().ok()) {
            Some((_, to)) => (to - chrono::Duration::seconds(1)).max(last),
            None => last,
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(from: &str, to: &str, why: &str) -> Blackout {
        Blackout { from: from.into(), to: to.into(), why: why.into() }
    }

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    /// A day alone is the whole day at either end; a time is up to, not at.
    #[test]
    fn days_are_whole_and_times_are_up_to() {
        let y = b("2026-12-29", "2027-01-03", "年末年始");
        assert_eq!(y.span().unwrap(), (at("2026-12-29 00:00"), at("2027-01-04 00:00")));
        assert!(y.contains(at("2026-12-29 00:00")));
        assert!(y.contains(at("2027-01-03 23:59")));
        assert!(!y.contains(at("2027-01-04 00:00")));
        assert!(!y.contains(at("2026-12-28 23:59")));
        let w = b("2026-11-14 22:00", "2026-11-15T06:00", "");
        assert!(w.contains(at("2026-11-14 22:00")));
        assert!(w.contains(at("2026-11-15 05:59")));
        assert!(!w.contains(at("2026-11-15 06:00")));
        // One day alone, from and to the same.
        assert!(b("2026-11-03", "2026-11-03", "").covers(day("2026-11-03")));
    }

    #[test]
    fn a_bad_end_or_a_backward_span_is_an_error() {
        let e = b("2026/12/29", "2027-01-03", "").check().unwrap_err();
        assert!(
            msg::BLACKOUT_TIME_BAD.is_in(&e)
                && e.contains("from には、\"2026-12-29\" または \"2026-12-29 22:00\" の形式で指定してください"),
            "{e}"
        );
        let e = b("2026-12-29", "あした", "").check().unwrap_err();
        assert!(msg::BLACKOUT_TIME_BAD.is_in(&e) && e.contains("to には"), "{e}");
        let e = b("2026-11-15 06:00", "2026-11-14 22:00", "").check().unwrap_err();
        assert!(
            msg::BLACKOUT_ORDER.is_in(&e) && e.contains("to（2026-11-14 22:00）が、from（2026-11-15 06:00）より前になっています"),
            "{e}"
        );
        assert!(b("2026-11-14 22:00", "2026-11-14 22:00", "").check().is_err());
        // A whole day ends after a time on it.
        assert!(b("2026-11-14 22:00", "2026-11-14", "").check().is_ok());
    }

    /// A day the stop reaches into is touched; one it fills is covered.
    #[test]
    fn touches_and_covers_days() {
        let w = b("2026-11-14 22:00", "2026-11-15 06:00", "");
        assert!(w.touches(day("2026-11-14")) && w.touches(day("2026-11-15")));
        assert!(!w.touches(day("2026-11-13")) && !w.touches(day("2026-11-16")));
        assert!(!w.covers(day("2026-11-14")) && !w.covers(day("2026-11-15")));
        let y = b("2026-12-29", "2027-01-03", "");
        assert!(y.covers(day("2027-01-01")) && !y.touches(day("2027-01-04")));
        assert!(!b("2026-11-14 00:00", "2026-11-15 00:00", "").touches(day("2026-11-15")));
    }

    #[test]
    fn the_reason_says_why_and_when() {
        assert_eq!(b("2026-12-29", "2027-01-03", " 年末年始 ").reason(), "計画停止（年末年始、2026-12-29〜2027-01-03）");
        assert_eq!(b("2026-11-14 22:00", "2026-11-15 06:00", "").reason(), "計画停止（2026-11-14 22:00〜2026-11-15 06:00）");
    }

    /// The next times step over a stop, however many times it holds.
    #[test]
    fn outside_steps_over_a_stop() {
        // Every five minutes.
        let every = |after: NaiveDateTime, k: usize| -> Vec<NaiveDateTime> {
            let first = after.and_utc().timestamp() / 300 * 300 + 300;
            (0..k as i64).map(|i| chrono::DateTime::from_timestamp(first + i * 300, 0).unwrap().naive_utc()).collect()
        };
        let list = [b("2026-12-29", "2027-01-03", "年末年始")];
        let got = outside(&list, at("2026-12-28 23:50"), 3, every);
        assert_eq!(got, vec![at("2026-12-28 23:55"), at("2027-01-04 00:00"), at("2027-01-04 00:05")]);
        assert_eq!(outside(&[], at("2026-12-28 23:50"), 2, every), vec![at("2026-12-28 23:55"), at("2026-12-29 00:00")]);
        // Nothing after the stop: none, and no endless looking.
        let none = |_: NaiveDateTime, _: usize| Vec::new();
        assert!(outside(&list, at("2026-12-30 00:00"), 1, none).is_empty());
    }

    /// The band: the stop going on, or the soonest to come within the days given.
    #[test]
    fn near_is_the_one_now_or_the_soonest() {
        let list = [b("2026-12-29", "2027-01-03", "年末年始"), b("2026-11-14 22:00", "2026-11-15 06:00", "電源工事")];
        assert_eq!(near(&list, at("2026-11-14 23:00"), 7).map(|b| b.why.as_str()), Some("電源工事"));
        assert_eq!(near(&list, at("2026-11-10 09:00"), 7).map(|b| b.why.as_str()), Some("電源工事"));
        assert_eq!(near(&list, at("2026-11-01 09:00"), 7), None);
        assert_eq!(near(&list, at("2026-12-25 09:00"), 7).map(|b| b.why.as_str()), Some("年末年始"));
        assert_eq!(find(&list, at("2027-01-01 12:00")).map(|b| b.why.as_str()), Some("年末年始"));
        assert_eq!(on_day(&list, day("2026-11-15")).map(|b| b.why.as_str()), Some("電源工事"));
        assert_eq!(on_day(&list, day("2026-11-16")), None);
    }
}
