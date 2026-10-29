//! The clock: which `[[when]]` times came due since the engine last looked,
//! and what to do about each.
//!
//! The engine keeps the instant it last looked in `schedule.toml` (`checked`,
//! UTC). Each second it asks `koyomi_core::when::due` for the times between
//! that instant and now, in the local zone, and judges each one: start it,
//! or record why it did not start (時刻を過ぎた while the PC was off,
//! スキップ while the last run still goes, 未確認で止めた). A net's `missed =
//! "once"` runs the last passed time instead, marked 遅れて実行; its `overlap =
//! "queue"` makes a time wait for the last run to end. A time is never
//! judged twice: `checked` moves on as soon as it is judged, and is written
//! at once when something happened.
//!
//! The first look of a fresh install starts from now: nothing before koyomi
//! existed is reported as passed.

use std::path::Path;
use std::time::Instant;

use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use ito_common::kept;
use koyomi_core::jobnet::{Missed, Overlap};
use koyomi_core::run::State;
use koyomi_core::scan::{Def, Standing};
use serde::{Deserialize, Serialize};

/// A time that came due this much after its instant is 時刻を過ぎた: the PC
/// (or the engine) was not running at it.
pub const LATE: Duration = Duration::seconds(60);
/// How often `checked` is written while nothing happens.
const WRITE_EVERY: Duration = Duration::seconds(10);

/// `schedule.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Kept {
    /// The instant last looked at, RFC 3339 in UTC.
    #[serde(default)]
    checked: String,
}

pub struct Schedule {
    checked: DateTime<Utc>,
    writer: kept::Writer,
    written: DateTime<Utc>,
    looked: Option<Instant>,
}

impl Schedule {
    /// Reads `schedule.toml` from `state_dir`; without one, starts from now.
    pub fn new(state_dir: &Path) -> Schedule {
        let path = state_dir.join("schedule.toml");
        let now = Utc::now();
        let kept: Kept = kept::read(&path).unwrap_or_default();
        let checked = DateTime::parse_from_rfc3339(&kept.checked).map(|d| d.to_utc()).unwrap_or(now);
        Schedule { checked, writer: kept::Writer::new(path, true), written: now, looked: None }
    }

    /// The instant last looked at, when a second passed since the last look
    /// (else `None`: nothing to do yet). A clock set back is taken as a new
    /// start: nothing between is due.
    pub fn look(&mut self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.looked.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) {
            return None;
        }
        self.looked = Some(Instant::now());
        if now < self.checked {
            self.checked = now;
            self.write(now);
        }
        Some(self.checked)
    }

    /// Moves the look on to `now`. `acted`: something was started or
    /// recorded, so the file is written now rather than with the next batch.
    pub fn looked(&mut self, now: DateTime<Utc>, acted: bool) {
        self.checked = now;
        if acted || now - self.written >= WRITE_EVERY {
            self.write(now);
        }
    }

    fn write(&mut self, now: DateTime<Utc>) {
        self.written = now;
        self.writer.save_toml(&Kept { checked: self.checked.to_rfc3339() });
    }

    /// Writes `checked` before the engine ends.
    pub fn flush(&self) {
        if let Ok(text) = toml::to_string(&Kept { checked: self.checked.to_rfc3339() }) {
            let _ = self.writer.save_now(text);
        }
    }

    #[cfg(test)]
    pub fn checked(&self) -> DateTime<Utc> {
        self.checked
    }
}

/// What the engine does about one time of `def` that came due.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Starts now; `late` when its time passed (`missed = "once"`), which the run says.
    Start { late: bool },
    /// Waits for the net's last run to end (`overlap = "queue"`).
    Queue { late: bool },
    /// Recorded as a run that did not start, in this state, for this reason.
    NotRun(State, String),
}

/// Judges the time `planned` of `def`, due `late` ago. `warnings` and
/// `bad_config` are the engine's reasons nothing starts; `running` says the
/// same net's last run is not over; `last` that no later time of the net
/// came due in the same look (the one time `missed = "once"` runs).
pub fn judge(
    def: &Def,
    planned: NaiveDateTime,
    late: Duration,
    warnings: &[String],
    bad_config: Option<&str>,
    running: bool,
    last: bool,
) -> Verdict {
    let (missed, overlap) = def.net.as_ref().map_or((Missed::Skip, Overlap::Skip), |n| (n.missed, n.overlap));
    let passed = late > LATE;
    if passed && !(missed == Missed::Once && last) {
        return Verdict::NotRun(State::Missed, format!("{} に PC が止まっていました", planned.format("%-H:%M")));
    }
    if !warnings.is_empty() {
        return Verdict::NotRun(State::Locked, format!("流さなかった: {}", warnings.join(" / ")));
    }
    if let Some(e) = bad_config {
        return Verdict::NotRun(State::Locked, format!("流さなかった: {e}"));
    }
    match def.standing {
        Standing::Invalid => return Verdict::NotRun(State::Locked, "定義の誤りがあり、流さなかった".to_owned()),
        Standing::Unconfirmed => return Verdict::NotRun(State::Locked, format!("未確認のため流さなかった（koyomi approve {}）", def.id)),
        Standing::Confirmed => {}
    }
    if running {
        return match overlap {
            Overlap::Queue => Verdict::Queue { late: passed },
            Overlap::Skip => Verdict::NotRun(State::Skipped, "前の回がまだ動いていました".to_owned()),
        };
    }
    Verdict::Start { late: passed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::approved::hash;
    use std::path::PathBuf;

    fn def(standing: Standing) -> Def {
        def_with(standing, "")
    }

    /// A definition with `head` (`missed = "once"` …) under its `format`.
    fn def_with(standing: Standing, head: &str) -> Def {
        let text = &format!(
            "format = 1\nname = \"月次締め\"\n{head}[[when]]\nevery = \"day\"\nat = [\"02:00\"]\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"
        );
        Def {
            id: "経理/月次締め".into(),
            group: "経理".into(),
            name: "月次締め".into(),
            title: Some("月次締め".into()),
            file: "経理/月次締め.toml".into(),
            standing,
            problems: Vec::new(),
            net: koyomi_core::jobnet::parse(text).ok(),
            sha256: hash(text.as_bytes()),
            text: text.into(),
            previous: None,
        }
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koyomi-schedule-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_time_on_a_confirmed_net_starts_and_each_reason_not_to_is_named() {
        let p = at("2026-10-30 02:00:00");
        let none: &[String] = &[];
        let start = Verdict::Start { late: false };
        assert_eq!(judge(&def(Standing::Confirmed), p, Duration::seconds(3), none, None, false, true), start);
        assert_eq!(
            judge(&def(Standing::Confirmed), p, Duration::seconds(3), none, None, false, false),
            start,
            "last means nothing unless once"
        );
        assert_eq!(
            judge(&def(Standing::Confirmed), p, Duration::seconds(61), none, None, false, true),
            Verdict::NotRun(State::Missed, "2:00 に PC が止まっていました".into())
        );
        assert_eq!(
            judge(&def(Standing::Confirmed), p, Duration::seconds(3), none, None, true, true),
            Verdict::NotRun(State::Skipped, "前の回がまだ動いていました".into())
        );
        let w = judge(&def(Standing::Confirmed), p, Duration::zero(), &["x が書ける".to_owned()], None, false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, "流さなかった: x が書ける".into()));
        let w = judge(&def(Standing::Confirmed), p, Duration::zero(), none, Some("koyomi.toml が読めない"), false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, "流さなかった: koyomi.toml が読めない".into()));
        let w = judge(&def(Standing::Unconfirmed), p, Duration::zero(), none, None, false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, "未確認のため流さなかった（koyomi approve 経理/月次締め）".into()));
        let w = judge(&def(Standing::Invalid), p, Duration::zero(), none, None, false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, "定義の誤りがあり、流さなかった".into()));
        // A time the PC slept through is 時刻を過ぎた whatever else is wrong: that is what happened.
        let w = judge(&def(Standing::Unconfirmed), p, Duration::minutes(5), &["w".to_owned()], None, true, true);
        assert!(matches!(w, Verdict::NotRun(State::Missed, _)), "{w:?}");
    }

    #[test]
    fn once_runs_the_last_passed_time_late_and_queue_waits_for_the_last_run() {
        let p = at("2026-10-30 02:00:00");
        let none: &[String] = &[];
        let once = def_with(Standing::Confirmed, "missed = \"once\"\n");
        assert_eq!(judge(&once, p, Duration::minutes(5), none, None, false, true), Verdict::Start { late: true });
        assert_eq!(judge(&once, p, Duration::seconds(3), none, None, false, true), Verdict::Start { late: false });
        // Only the last passed time runs; the ones before it are 時刻を過ぎた.
        assert!(matches!(judge(&once, p, Duration::minutes(5), none, None, false, false), Verdict::NotRun(State::Missed, _)));
        // Everything that stops a run on time stops a late one too.
        let w = judge(&def_with(Standing::Unconfirmed, "missed = \"once\"\n"), p, Duration::minutes(5), none, None, false, true);
        assert!(matches!(w, Verdict::NotRun(State::Locked, _)), "{w:?}");
        let w = judge(&once, p, Duration::minutes(5), &["w".to_owned()], None, false, true);
        assert!(matches!(w, Verdict::NotRun(State::Locked, _)), "{w:?}");
        let w = judge(&once, p, Duration::minutes(5), none, None, true, true);
        assert_eq!(w, Verdict::NotRun(State::Skipped, "前の回がまだ動いていました".into()));
        let queue = def_with(Standing::Confirmed, "overlap = \"queue\"\n");
        assert_eq!(judge(&queue, p, Duration::seconds(3), none, None, true, true), Verdict::Queue { late: false });
        assert_eq!(judge(&queue, p, Duration::seconds(3), none, None, false, true), Verdict::Start { late: false });
        assert!(matches!(judge(&queue, p, Duration::minutes(5), none, None, true, true), Verdict::NotRun(State::Missed, _)));
        let w = judge(&def_with(Standing::Unconfirmed, "overlap = \"queue\"\n"), p, Duration::seconds(3), none, None, true, true);
        assert!(matches!(w, Verdict::NotRun(State::Locked, _)), "{w:?}");
        let both = def_with(Standing::Confirmed, "missed = \"once\"\noverlap = \"queue\"\n");
        assert_eq!(judge(&both, p, Duration::minutes(5), none, None, true, true), Verdict::Queue { late: true });
    }

    #[test]
    fn the_last_look_is_kept_across_a_restart_and_a_fresh_start_is_now() {
        let dir = scratch("kept");
        let before = Utc::now();
        let mut s = Schedule::new(&dir);
        assert!(s.checked() >= before && s.checked() <= Utc::now());
        let looked = DateTime::parse_from_rfc3339("2026-10-28T17:00:00Z").unwrap().to_utc();
        s.looked(looked, true);
        s.flush();
        let s = Schedule::new(&dir);
        assert_eq!(s.checked(), looked);
        let text = std::fs::read_to_string(dir.join("schedule.toml")).unwrap();
        assert!(text.contains("checked = \"2026-10-28T17:00:00+00:00\""), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clock_set_back_starts_over_from_now() {
        let dir = scratch("back");
        let mut s = Schedule::new(&dir);
        let later = Utc::now() + Duration::days(2);
        s.looked(later, false);
        let now = Utc::now();
        assert_eq!(s.look(now), Some(now));
        assert!(s.look(now).is_none(), "a second look within the second does nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
