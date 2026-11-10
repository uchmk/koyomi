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
use koyomi_core::msg;
use koyomi_core::quiet::{self, OnQuiet};
use koyomi_core::run::State;
use koyomi_core::scan::{Def, Standing};
use serde::{Deserialize, Serialize};

/// A time that came due this much after its instant is 時刻を過ぎた: the PC
/// (or the engine) was not running at it.
pub const LATE: Duration = Duration::seconds(60);
/// How far back the clock may be set and still be waited out.
const BACK: Duration = Duration::days(1);
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
        let now = ito_common::now::now();
        let kept: Kept = kept::read(&path).unwrap_or_default();
        let checked = DateTime::parse_from_rfc3339(&kept.checked).map(|d| d.to_utc()).unwrap_or(now);
        Schedule { checked, writer: kept::Writer::new(path, true), written: now, looked: None }
    }

    /// The instant last looked at, when a second passed since the last look
    /// (else `None`: nothing to do yet). A clock set back by up to `BACK` waits
    /// until it catches up, so the times it passes again do not start twice;
    /// set back further, it is taken as a new start: nothing between is due.
    pub fn look(&mut self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.looked.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) {
            return None;
        }
        self.looked = Some(Instant::now());
        if now < self.checked && self.checked - now <= BACK {
            return None;
        }
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
    judge_known(def, planned, late, warnings, bad_config, running, last, true)
}

/// [`judge`], where `known` says the engine could have known this time when it came: a time
/// of a net that was written after it (a time added while the engine was stopped) was never
/// missed, and the net is judged as it stands (an unconfirmed one is 止めた, not 時刻を過ぎた).
#[allow(clippy::too_many_arguments)]
pub fn judge_known(
    def: &Def,
    planned: NaiveDateTime,
    late: Duration,
    warnings: &[String],
    bad_config: Option<&str>,
    running: bool,
    last: bool,
    known: bool,
) -> Verdict {
    let (missed, overlap) = def.net.as_ref().map_or((Missed::Skip, Overlap::Skip), |n| (n.missed, n.overlap));
    let passed = late > LATE;
    if !known && def.standing != Standing::Confirmed {
        let on_time = judge_known(def, planned, Duration::zero(), warnings, bad_config, running, last, true);
        return on_time;
    }
    if passed && !(missed == Missed::Once && last) {
        return Verdict::NotRun(State::Missed, msg::note(msg::N_MISSED_PC_STOPPED, &[&planned.format("%-H:%M").to_string()]));
    }
    if !warnings.is_empty() {
        return Verdict::NotRun(
            State::Locked,
            msg::note(msg::N_NOT_RUN_BECAUSE, &[&warnings.iter().map(|w| msg::uncoded(w)).collect::<Vec<_>>().join(" / ")]),
        );
    }
    if let Some(e) = bad_config {
        return Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_BECAUSE, &[msg::uncoded(e)]));
    }
    match def.standing {
        Standing::Invalid => return Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_INVALID, &[])),
        Standing::Unconfirmed => return Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_UNCONFIRMED, &[&def.id])),
        Standing::Confirmed => {}
    }
    if running {
        return match overlap {
            Overlap::Queue => Verdict::Queue { late: passed },
            Overlap::Skip => Verdict::NotRun(State::Skipped, msg::note(msg::N_NOT_RUN_PREVIOUS, &[])),
        };
    }
    Verdict::Start { late: passed }
}

/// What a time of `def` that would start does when it falls in the net's `quiet` hours.
#[derive(Debug, PartialEq, Eq)]
pub enum Quiet {
    /// Recorded as スキップ, for this reason (`on_quiet = "skip"`).
    Skip(String),
    /// Starts at this time, when the hours are over.
    Wait(NaiveDateTime),
}

/// Whether the time `planned` of `def` is held back by its `quiet` hours at
/// `now`; `None` when it is in none, or they were over by now (it passed
/// while the engine was off, and `missed` has had its say).
pub fn quiet(def: &Def, planned: NaiveDateTime, now: NaiveDateTime) -> Option<Quiet> {
    let net = def.net.as_ref()?;
    let until = quiet::until(&net.quiet, planned).filter(|u| *u > now)?;
    Some(match net.on_quiet {
        OnQuiet::Wait => Quiet::Wait(until),
        OnQuiet::Skip => Quiet::Skip(msg::note(msg::N_QUIET_SKIP, &[&quiet::named(&net.quiet, planned).unwrap_or_default()])),
    })
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
            calendar: None,
        }
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn a_time_in_the_quiet_hours_waits_or_is_skipped() {
        let wait = def_with(Standing::Confirmed, "quiet = [\"01:30-03:00\"]\n");
        let p = at("2026-10-30 02:00:00");
        assert_eq!(quiet(&wait, p, p), Some(Quiet::Wait(at("2026-10-30 03:00:00"))));
        assert_eq!(quiet(&wait, p, at("2026-10-30 03:00:00")), None, "over by now: it starts");
        assert_eq!(quiet(&wait, at("2026-10-30 03:00:00"), at("2026-10-30 03:00:00")), None);
        let skip = def_with(Standing::Confirmed, "quiet = [\"01:30-03:00\"]\non_quiet = \"skip\"\n");
        assert_eq!(quiet(&skip, p, p), Some(Quiet::Skip(msg::note(msg::N_QUIET_SKIP, &["01:30-03:00"]))));
        assert_eq!(quiet(&def(Standing::Confirmed), p, p), None);
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
            Verdict::NotRun(State::Missed, msg::note(msg::N_MISSED_PC_STOPPED, &["2:00"]))
        );
        assert_eq!(
            judge(&def(Standing::Confirmed), p, Duration::seconds(3), none, None, true, true),
            Verdict::NotRun(State::Skipped, msg::note(msg::N_NOT_RUN_PREVIOUS, &[]))
        );
        let w = judge(&def(Standing::Confirmed), p, Duration::zero(), &["x が書ける".to_owned()], None, false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_BECAUSE, &["x が書ける"])));
        let w = judge(&def(Standing::Confirmed), p, Duration::zero(), none, Some("koyomi.toml が読めない"), false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_BECAUSE, &["koyomi.toml が読めない"])));
        let w = judge(&def(Standing::Unconfirmed), p, Duration::zero(), none, None, false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_UNCONFIRMED, &["経理/月次締め"])));
        let w = judge(&def(Standing::Invalid), p, Duration::zero(), none, None, false, true);
        assert_eq!(w, Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_INVALID, &[])));
        // A time the PC slept through is 時刻を過ぎた whatever else is wrong: that is what happened.
        let w = judge(&def(Standing::Unconfirmed), p, Duration::minutes(5), &["w".to_owned()], None, true, true);
        assert!(matches!(w, Verdict::NotRun(State::Missed, _)), "{w:?}");
        // But not a time the engine never knew: one added to an unconfirmed net after it passed.
        let w = judge_known(&def(Standing::Unconfirmed), p, Duration::minutes(5), none, None, false, true, false);
        assert_eq!(w, Verdict::NotRun(State::Locked, msg::note(msg::N_NOT_RUN_UNCONFIRMED, &["経理/月次締め"])));
        let w = judge_known(&def(Standing::Unconfirmed), p, Duration::minutes(5), none, None, false, true, true);
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
        assert_eq!(w, Verdict::NotRun(State::Skipped, msg::note(msg::N_NOT_RUN_PREVIOUS, &[])));
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
        let before = ito_common::now::now();
        let mut s = Schedule::new(&dir);
        assert!(s.checked() >= before && s.checked() <= ito_common::now::now());
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
    fn a_clock_set_back_a_little_waits_for_the_time_to_pass_again() {
        let dir = scratch("back-a-little");
        let mut s = Schedule::new(&dir);
        let before = ito_common::now::now() + Duration::hours(1);
        s.looked(before, false);
        assert!(s.look(ito_common::now::now()).is_none());
        assert_eq!(s.checked(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clock_set_back_starts_over_from_now() {
        let dir = scratch("back");
        let mut s = Schedule::new(&dir);
        let later = ito_common::now::now() + Duration::days(2);
        s.looked(later, false);
        let now = ito_common::now::now();
        assert_eq!(s.look(now), Some(now));
        assert!(s.look(now).is_none(), "a second look within the second does nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
