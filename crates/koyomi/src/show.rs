//! How a run and its jobs read on the console and on the screen: the state's
//! symbol and word, the times, the duration. The console and the window say
//! the same thing in the same words.

use chrono::{Datelike, NaiveDateTime};
use ito_i18n::Table;
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

/// The next `n` times `whens` come after `now`, one row each: what the window
/// and `koyomi next` both show, from the same calculation the engine starts by.
pub fn next_rows(t: &Table, whens: &[When], now: NaiveDateTime, n: usize) -> Vec<String> {
    when::next_times(whens, now, n).into_iter().map(|at| when_row(t, at)).collect()
}

/// One line for a job: tag, name, times, duration, the reason when there is one.
pub fn job_line(t: &Table, j: &JobRun) -> String {
    let mut line = format!("{}  {}", tag(t, j.state), j.label());
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
        let rows = next_rows(&ja, &whens, now, 3);
        assert_eq!(rows, vec!["2026-10-31（土）02:00", "2026-11-01（日）02:00", "2026-11-02（月）02:00"]);
        let (en, _) = crate::i18n::table("en");
        assert_eq!(next_rows(&en, &whens, now, 1), vec!["2026-10-31 (Sat) 02:00"]);
        assert!(next_rows(&en, &[], now, 10).is_empty());
    }

    #[test]
    fn a_duration_reads_as_h_mm_ss() {
        assert_eq!(duration(chrono::Duration::seconds(3723)), "1:02:03");
        assert_eq!(duration(chrono::Duration::seconds(5)), "0:00:05");
    }
}
