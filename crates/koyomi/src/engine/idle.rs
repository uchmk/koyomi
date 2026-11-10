//! Starts when the PC is left alone: `[[when]] idle = "10m"`.
//!
//! How long the PC has gone without a key or the mouse is read every [`POLL`].
//! A rule starts its net once when that reaches its length, and is armed again
//! only when someone uses the PC (the time falls under the length), as Task
//! Scheduler's idle trigger does. A rule starts unarmed until the time is seen
//! under its length, so a start or a reload of the engine while the PC is
//! already left alone does not start the net a second time in that stretch.
//!
//! The service runs apart from the person at the screen, so it reads their time
//! through ito-power's `user_idle_for` (the console session's). Where the time
//! cannot be known (a PC with no screen, nobody signed in there, or Windows not
//! keeping that time for the session) nothing starts, and that is said in the
//! cautions.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use koyomi_core::msg;
use koyomi_core::scan::{Def, Standing};
use koyomi_core::when::Kind;

use super::logs::Hit;

/// How often the time the PC has been left alone is read.
const POLL: Duration = Duration::from_secs(5);

/// How long the PC has gone without a key or the mouse; `None` when that cannot be known.
/// The service runs apart from the signed-in person, so it asks for the time of the PC's own
/// screen; Windows keeps that only for some sessions, and says nothing for the others.
pub fn idle_now() -> Option<Duration> {
    if crate::service::RUNNING.load(Ordering::Relaxed) {
        return ito_power::user_idle_for();
    }
    ito_power::idle_for()
}

/// `600` as `10 分`, for the record of why a net started.
pub fn span_text(secs: u64) -> String {
    if secs.is_multiple_of(60) {
        msg::note(msg::N_SPAN_MIN, &[&(secs / 60).to_string()])
    } else {
        msg::note(msg::N_SPAN_SEC, &[&secs.to_string()])
    }
}

struct Rule {
    def_id: String,
    kind: Kind,
    secs: u64,
    /// Set once the PC is seen in use (under `secs`); a start clears it.
    armed: bool,
}

pub struct Idle {
    rules: Vec<Rule>,
    last: Option<Instant>,
    /// The last read could not tell how long the PC has been left alone.
    unknown: bool,
}

impl Idle {
    pub fn new() -> Idle {
        Idle { rules: Vec::new(), last: None, unknown: false }
    }

    /// Takes the `idle` whens of the confirmed nets as the rules.
    /// A rule kept from before keeps whether it is armed; a new one waits to see the PC in use.
    pub fn set_rules(&mut self, defs: &[Def]) {
        let mut wanted: Vec<(String, Kind, u64)> = Vec::new();
        for d in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
            for w in d.net.iter().flat_map(|n| &n.when) {
                if let Kind::Idle { idle } = w.kind {
                    if !wanted.iter().any(|(id, k, _)| *id == d.id && *k == w.kind) {
                        wanted.push((d.id.clone(), w.kind.clone(), idle));
                    }
                }
            }
        }
        self.rules.retain(|r| wanted.iter().any(|(id, k, _)| *id == r.def_id && *k == r.kind));
        for (def_id, kind, secs) in wanted {
            if !self.rules.iter().any(|r| r.def_id == def_id && r.kind == kind) {
                self.rules.push(Rule { def_id, kind, secs, armed: false });
            }
        }
    }

    /// The rules the PC's idle time reached since the last read.
    pub fn due(&mut self) -> Vec<Hit> {
        if self.rules.is_empty() || self.last.is_some_and(|l| l.elapsed() < POLL) {
            return Vec::new();
        }
        self.last = Some(Instant::now());
        self.step(idle_now())
    }

    fn step(&mut self, idle: Option<Duration>) -> Vec<Hit> {
        let Some(idle) = idle else {
            self.unknown = true;
            return Vec::new();
        };
        self.unknown = false;
        let mut out = Vec::new();
        for r in &mut self.rules {
            if idle.as_secs() < r.secs {
                r.armed = true;
            } else if r.armed {
                r.armed = false;
                out.push(Hit { def_id: r.def_id.clone(), kind: r.kind.clone(), count: 1, first: String::new() });
            }
        }
        out
    }

    /// Why the `idle` whens do not start anything, while the idle time cannot be known; one line a net.
    pub fn problems(&self) -> Vec<String> {
        if !self.unknown {
            return Vec::new();
        }
        let mut ids: Vec<&str> = self.rules.iter().map(|r| r.def_id.as_str()).collect();
        ids.dedup();
        ids.into_iter().map(|id| msg::line(msg::IDLE_UNKNOWN, &[id])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, text: &str, standing: Standing) -> Def {
        let net = koyomi_core::jobnet::parse(text).unwrap();
        Def {
            id: id.to_owned(),
            group: "g".to_owned(),
            name: id.to_owned(),
            title: None,
            file: format!("{id}.toml"),
            standing,
            problems: Vec::new(),
            net: Some(net),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        }
    }

    const TEN: &str = "format = 1\n[[when]]\nidle = \"10m\"\n";
    const MIN: Option<Duration> = Some(Duration::from_secs(60));
    const HOUR: Option<Duration> = Some(Duration::from_secs(3600));

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.def_id.as_str()).collect()
    }

    #[test]
    fn it_starts_once_a_stretch_and_again_after_the_pc_is_used() {
        let mut idle = Idle::new();
        idle.set_rules(&[def("g/a", TEN, Standing::Confirmed)]);
        assert!(idle.step(MIN).is_empty());
        let hits = idle.step(Some(Duration::from_secs(600)));
        assert_eq!(ids(&hits), ["g/a"]);
        assert_eq!(hits[0].kind, Kind::Idle { idle: 600 });
        assert_eq!((hits[0].count, hits[0].first.as_str()), (1, ""));
        // Still left alone: not again.
        assert!(idle.step(HOUR).is_empty());
        // Used, then left alone again: once more.
        assert!(idle.step(Some(Duration::from_secs(3))).is_empty());
        assert_eq!(ids(&idle.step(HOUR)), ["g/a"]);
    }

    #[test]
    fn a_pc_already_left_alone_when_the_rule_comes_does_not_start_it() {
        let mut idle = Idle::new();
        idle.set_rules(&[def("g/a", TEN, Standing::Confirmed)]);
        assert!(idle.step(HOUR).is_empty());
        assert!(idle.step(MIN).is_empty());
        assert_eq!(ids(&idle.step(HOUR)), ["g/a"]);
    }

    #[test]
    fn a_reload_keeps_whether_a_rule_is_armed() {
        let mut idle = Idle::new();
        idle.set_rules(&[def("g/a", TEN, Standing::Confirmed)]);
        assert!(idle.step(MIN).is_empty());
        idle.set_rules(&[def("g/a", TEN, Standing::Confirmed), def("g/b", TEN, Standing::Confirmed)]);
        // g/a was armed before the reload; g/b is new and waits to see the PC in use.
        assert_eq!(ids(&idle.step(HOUR)), ["g/a"]);
        // A rule that is gone is dropped.
        idle.set_rules(&[def("g/b", TEN, Standing::Confirmed)]);
        assert!(idle.step(MIN).is_empty());
        assert_eq!(ids(&idle.step(HOUR)), ["g/b"]);
    }

    #[test]
    fn rules_of_each_length_start_when_their_own_time_is_reached() {
        let mut idle = Idle::new();
        let both = "format = 1\n[[when]]\nidle = \"10m\"\n[[when]]\nidle = \"30m\"\n";
        idle.set_rules(&[def("g/a", both, Standing::Confirmed)]);
        assert!(idle.step(MIN).is_empty());
        let hits = idle.step(Some(Duration::from_secs(700)));
        assert_eq!(hits.iter().map(|h| h.kind.clone()).collect::<Vec<_>>(), [Kind::Idle { idle: 600 }]);
        let hits = idle.step(Some(Duration::from_secs(1800)));
        assert_eq!(hits.iter().map(|h| h.kind.clone()).collect::<Vec<_>>(), [Kind::Idle { idle: 1800 }]);
    }

    #[test]
    fn an_unconfirmed_net_is_not_watched() {
        let mut idle = Idle::new();
        idle.set_rules(&[def("g/a", TEN, Standing::Unconfirmed)]);
        assert!(idle.step(MIN).is_empty());
        assert!(idle.step(HOUR).is_empty());
    }

    #[test]
    fn an_unknown_idle_time_starts_nothing_and_is_said() {
        let mut idle = Idle::new();
        idle.set_rules(&[def("g/a", TEN, Standing::Confirmed)]);
        assert!(idle.problems().is_empty());
        assert!(idle.step(None).is_empty());
        assert_eq!(idle.problems(), [msg::line(msg::IDLE_UNKNOWN, &["g/a"])]);
        assert!(idle.step(HOUR).is_empty());
        assert!(idle.problems().is_empty());
    }

    #[test]
    fn spans_are_said_in_minutes_when_whole() {
        assert_eq!(span_text(600), "10 分");
        assert_eq!(span_text(90), "90 秒");
    }
}
