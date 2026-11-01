//! The decisions of HA (TODO 87 and 96, DECISIONS「複数の PC・Web UI・利用者とロール・HA」).
//!
//! The standby (`[standby]`) polls the primary's `GET /health` every `interval`. After `misses`
//! answers in a row that do not come, it takes over: from then on its own engine runs the nets'
//! schedules. With `give_back = "manual"` (the default) it does not hand back by itself when the
//! primary answers again; that is a person's `koyomi standby giveback`, so the two never flip back
//! and forth on a flaky line. With `give_back = "auto"` it hands back once the primary has
//! answered `misses` polls in a row: to a primary that waits (`[primary]`) when it asks, and to
//! one that runs on its own by stepping down.
//!
//! The primary (`[primary]`) asks the standby before it runs the schedules, and every `every`
//! after. While the standby says it has taken over, the primary waits ([`Hold::Held`]) and asks it
//! to let go; the standby lets go only when it is ready ([`Watch::release`]), so the schedules
//! move over in one step: never on both PCs, never on neither.
//!
//! When it takes over, the runs the primary was running at the last poll are started again here
//! only for a net that says `takeover = "rerun"` ([`resume`]); the others are written down as
//! not taken over, for a person to decide (the primary may only be cut off and still run them).

use std::time::Duration;

use crate::jobnet::{Jobnet, Takeover};
use crate::run::Run;

/// How often the primary is polled when `[standby]` does not say.
pub const INTERVAL: Duration = Duration::from_secs(20);
/// How many misses in a row before taking over when `[standby]` does not say.
pub const MISSES: u32 = 3;

/// What one poll of the primary found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// No answer: no connection, or TLS that does not finish.
    Nobody,
    /// It answers and runs the schedules itself.
    Running,
    /// It answers and waits for this one to let go (`[primary]`).
    Waiting,
    /// It answers and runs the schedules, but asks this one before it goes on (`[primary]`):
    /// it waits once it hears this one has taken over, so this one never steps down from it.
    Asks,
}

/// Where the standby is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Watching the primary; `missed` answers in a row have not come.
    Watching { missed: u32 },
    /// Took over. `back`: the polls in a row the primary has answered since; `told`: what was
    /// last said of its answers (each once, again after a miss).
    Active { back: u32, told: Option<Seen> },
}

/// What the standby does after a poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing changes.
    Stay,
    /// Start running the schedules here, and say so.
    TakeOver,
    /// The primary runs the schedules too: say so once (a person hands back).
    BothActive,
    /// The primary is back and waits to be handed the schedules: say so once.
    Waiting,
    /// `give_back = "auto"`: the primary has run on its own for `misses` polls in a row. Stop
    /// running the schedules here and watch again.
    StepDown,
}

/// Why the standby does not let go when the primary asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// It has not taken over.
    Idle,
    /// `give_back = "manual"`, and nobody has run `koyomi standby giveback`.
    Manual,
    /// `give_back = "auto"`, but the primary has answered only `back` of the `misses` polls in a row.
    NotYet { back: u32, misses: u32 },
}

/// The standby's watch on one primary.
#[derive(Clone, Debug)]
pub struct Watch {
    misses: u32,
    auto: bool,
    state: State,
    /// A person ran `koyomi standby giveback` while the primary waits: let go when it asks.
    giving: bool,
}

impl Watch {
    /// `misses` below 1 is taken as 1. `auto`: `give_back = "auto"`.
    pub fn new(misses: u32, auto: bool) -> Watch {
        Watch { misses: misses.max(1), auto, state: State::Watching { missed: 0 }, giving: false }
    }

    /// Resumes a takeover kept over a restart, so it does not wait for the misses again.
    pub fn resumed(misses: u32, auto: bool, giving: bool) -> Watch {
        Watch { misses: misses.max(1), auto, state: State::Active { back: 0, told: None }, giving }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn active(&self) -> bool {
        matches!(self.state, State::Active { .. })
    }

    /// A person said to hand back, and the primary has not come for it yet.
    pub fn giving(&self) -> bool {
        self.giving
    }

    /// One poll of the primary.
    pub fn poll(&mut self, seen: Seen) -> Step {
        match (self.state, seen) {
            (State::Watching { missed }, Seen::Nobody) if missed + 1 >= self.misses => {
                self.state = State::Active { back: 0, told: None };
                Step::TakeOver
            }
            (State::Watching { missed }, Seen::Nobody) => {
                self.state = State::Watching { missed: missed + 1 };
                Step::Stay
            }
            (State::Watching { .. }, _) => {
                self.state = State::Watching { missed: 0 };
                Step::Stay
            }
            (State::Active { .. }, Seen::Nobody) => {
                self.state = State::Active { back: 0, told: None };
                Step::Stay
            }
            (State::Active { back, told }, seen) => {
                let back = back + 1;
                // Only one that runs on its own is stepped down from: one that asks waits after
                // its next ask, and stepping down then would leave the schedules to nobody.
                if self.auto && seen == Seen::Running && back >= self.misses {
                    self.watch_again();
                    return Step::StepDown;
                }
                self.state = State::Active { back, told: Some(seen) };
                match seen {
                    _ if told == Some(seen) => Step::Stay,
                    Seen::Waiting => Step::Waiting,
                    _ => Step::BothActive,
                }
            }
        }
    }

    /// Whether it would let go if the primary asked now.
    pub fn ready(&self) -> Result<(), Refusal> {
        match self.state {
            State::Watching { .. } => Err(Refusal::Idle),
            State::Active { .. } if self.giving => Ok(()),
            State::Active { .. } if !self.auto => Err(Refusal::Manual),
            State::Active { back, .. } if back < self.misses => Err(Refusal::NotYet { back, misses: self.misses }),
            State::Active { .. } => Ok(()),
        }
    }

    /// The primary asks for the schedules back (`POST /standby/release`): when [`Watch::ready`],
    /// stop running them here and watch again.
    pub fn release(&mut self) -> Result<(), Refusal> {
        self.ready()?;
        self.watch_again();
        Ok(())
    }

    /// `koyomi standby giveback` while the primary waits: let go when it next asks. False when
    /// this one had not taken over.
    pub fn hand_over(&mut self) -> bool {
        self.giving = self.active();
        self.giving
    }

    /// `koyomi standby giveback`: stop running the schedules here and watch again, at once.
    /// False when this one had not taken over.
    pub fn give_back(&mut self) -> bool {
        let was = self.active();
        self.watch_again();
        was
    }

    fn watch_again(&mut self) {
        self.state = State::Watching { missed: 0 };
        self.giving = false;
    }
}

/// Where the primary is (`[primary]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Asking the standby before running the schedules; `missed` asks in a row got no answer.
    /// The times that come due meanwhile wait, and are judged once it knows.
    Asking { missed: u32 },
    /// The standby has taken over: the schedules and the events run there, not here.
    Held,
    /// Running the schedules here.
    Running,
}

/// What one ask of the standby heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// It has taken over and has not let go.
    Active,
    /// It stands by, or it let go just now.
    Idle,
    /// No answer: no connection, or TLS that does not finish.
    Nobody,
    /// It answers, but cannot say (no `[standby]` there, an old koyomi, a wrong token …).
    Unclear,
}

/// The primary's asks of its standby.
#[derive(Clone, Debug)]
pub struct Ask {
    misses: u32,
    hold: Hold,
}

impl Ask {
    /// Starts by asking. `misses` below 1 is taken as 1.
    pub fn new(misses: u32) -> Ask {
        Ask { misses: misses.max(1), hold: Hold::Asking { missed: 0 } }
    }

    pub fn hold(&self) -> Hold {
        self.hold
    }

    /// Takes in one ask's answer; the new hold when it changed.
    ///
    /// Running, only "taken over" stops it (a split line healed: the standby runs, this one
    /// waits). Asking, an answer that cannot say runs here, as without `[primary]`; held, it
    /// keeps waiting rather than run the schedules twice. A held one that gets no answer asks
    /// again, the times waiting, and runs after `misses` in a row.
    pub fn heard(&mut self, h: Heard) -> Option<Hold> {
        let next = match (self.hold, h) {
            (_, Heard::Active) => Hold::Held,
            (_, Heard::Idle) | (Hold::Running, _) | (Hold::Asking { .. }, Heard::Unclear) => Hold::Running,
            (Hold::Held, Heard::Unclear) => Hold::Held,
            (Hold::Asking { missed }, Heard::Nobody) => self.missed(missed),
            (Hold::Held, Heard::Nobody) => self.missed(0),
        };
        let changed = next != self.hold;
        self.hold = next;
        changed.then_some(next)
    }

    fn missed(&self, before: u32) -> Hold {
        if before + 1 >= self.misses {
            Hold::Running
        } else {
            Hold::Asking { missed: before + 1 }
        }
    }
}

/// What the standby does with a run the primary was running when it took over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resume {
    /// Start it again here from the jobs that had not passed ([`Run::taken_over`]).
    Rerun,
    /// Write it down as not taken over ([`Run::left_over`]), with why.
    Leave(String),
}

/// `theirs`, a run the primary was running at the last poll; `net`, the definition here that
/// may run (confirmed), if any. Only a net that says `takeover = "rerun"` runs again: under a
/// split line the primary may still be running it, and only the owner knows which nets bear that.
pub fn resume(theirs: &Run, net: Option<&Jobnet>) -> Resume {
    match net {
        None => Resume::Leave("この PC に流せる定義がない".to_owned()),
        Some(n) if n.takeover != Takeover::Rerun => Resume::Leave("takeover = \"rerun\" ではない".to_owned()),
        Some(_) if theirs.jobs.iter().all(|j| j.state.passed()) => Resume::Leave("ジョブは全部済んでいた".to_owned()),
        Some(_) => Resume::Rerun,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::jobnet::{Job, Line};
    use crate::run::{How, State as RunState};

    /// it takes over only after the set misses in a row; an answer between starts the count again.
    #[test]
    fn it_takes_over_after_the_misses_in_a_row() {
        let mut w = Watch::new(3, false);
        let seen = [Seen::Nobody, Seen::Nobody, Seen::Running, Seen::Nobody, Seen::Waiting, Seen::Nobody, Seen::Nobody];
        assert_eq!(seen.map(|s| w.poll(s)), [Step::Stay; 7]);
        assert_eq!(w.state(), State::Watching { missed: 2 });
        assert_eq!(w.poll(Seen::Nobody), Step::TakeOver);
        assert!(w.active());
        assert_eq!(w.poll(Seen::Nobody), Step::Stay);
    }

    /// manual: once active it does not hand back by itself; it says once that both run, again after a miss.
    #[test]
    fn it_stays_active_and_says_both_once() {
        let mut w = Watch::new(1, false);
        assert_eq!(w.poll(Seen::Nobody), Step::TakeOver);
        assert_eq!(w.poll(Seen::Running), Step::BothActive);
        assert_eq!([Seen::Running; 5].map(|s| w.poll(s)), [Step::Stay; 5]);
        assert!(w.active());
        assert_eq!(w.poll(Seen::Nobody), Step::Stay);
        assert_eq!(w.poll(Seen::Running), Step::BothActive);
        assert_eq!(w.poll(Seen::Waiting), Step::Waiting);
        assert_eq!(w.poll(Seen::Waiting), Step::Stay);
        assert_eq!(w.release(), Err(Refusal::Manual));
        assert!(w.active());
    }

    /// giveback watches again from zero; on a watching one it says there was nothing to give.
    #[test]
    fn giveback_watches_again() {
        let mut w = Watch::new(0, false);
        assert!(!w.give_back());
        assert!(!w.hand_over());
        assert_eq!(w.poll(Seen::Nobody), Step::TakeOver);
        assert!(w.give_back());
        assert_eq!(w.state(), State::Watching { missed: 0 });
        assert_eq!(w.poll(Seen::Running), Step::Stay);
        let mut w = Watch::resumed(3, false, false);
        assert!(w.active());
        assert_eq!(w.poll(Seen::Running), Step::BothActive);
    }

    /// a person's giveback while the primary waits lets go when it asks, and not before; it
    /// holds over a restart.
    #[test]
    fn a_person_hands_over_when_the_primary_asks() {
        let mut w = Watch::new(1, false);
        assert_eq!(w.poll(Seen::Nobody), Step::TakeOver);
        assert_eq!(w.poll(Seen::Waiting), Step::Waiting);
        assert!(w.hand_over());
        assert!(w.giving() && w.active());
        assert_eq!(w.ready(), Ok(()));
        let mut kept = Watch::resumed(1, false, w.giving());
        assert_eq!(kept.release(), Ok(()));
        assert!(!kept.active() && !kept.giving());
        assert_eq!(w.release(), Ok(()));
        assert_eq!(w.state(), State::Watching { missed: 0 });
        assert_eq!(w.release(), Err(Refusal::Idle));
    }

    /// auto: it lets a waiting primary have the schedules once it answered `misses` polls in a
    /// row (a miss starts the count again), and steps down from one that runs on its own.
    #[test]
    fn auto_hands_back_after_the_misses_answered() {
        let mut w = Watch::new(3, true);
        assert_eq!([Seen::Nobody; 3].map(|s| w.poll(s)), [Step::Stay, Step::Stay, Step::TakeOver]);
        assert_eq!(w.release(), Err(Refusal::NotYet { back: 0, misses: 3 }));
        assert_eq!(w.poll(Seen::Waiting), Step::Waiting);
        assert_eq!(w.poll(Seen::Waiting), Step::Stay);
        assert_eq!(w.poll(Seen::Nobody), Step::Stay);
        assert_eq!([Seen::Waiting; 2].map(|s| w.poll(s)), [Step::Waiting, Step::Stay]);
        assert_eq!(w.release(), Err(Refusal::NotYet { back: 2, misses: 3 }));
        assert_eq!(w.poll(Seen::Waiting), Step::Stay);
        assert_eq!(w.ready(), Ok(()));
        assert_eq!(w.release(), Ok(()));
        assert!(!w.active());

        let mut w = Watch::resumed(2, true, false);
        assert_eq!(w.poll(Seen::Running), Step::BothActive);
        assert_eq!(w.poll(Seen::Running), Step::StepDown);
        assert_eq!(w.state(), State::Watching { missed: 0 });
        // A waiting primary is never stepped down from: it asks.
        let mut w = Watch::resumed(1, true, false);
        assert_eq!(w.poll(Seen::Waiting), Step::Waiting);
        assert!(w.active());
        // Nor one that runs but asks: it waits after its next ask.
        let mut w = Watch::resumed(1, true, false);
        assert_eq!([Seen::Asks; 3].map(|s| w.poll(s)), [Step::BothActive, Step::Stay, Step::Stay]);
        assert_eq!(w.poll(Seen::Waiting), Step::Waiting);
        assert_eq!(w.release(), Ok(()));
        let mut w = Watch::new(2, true);
        assert_eq!([Seen::Nobody, Seen::Asks, Seen::Nobody].map(|s| w.poll(s)), [Step::Stay; 3]);
    }

    /// the primary asks first, waits while the standby has taken over, and runs once it lets
    /// go or after `misses` asks in a row get no answer.
    #[test]
    fn the_primary_asks_before_it_runs() {
        let mut a = Ask::new(2);
        assert_eq!(a.hold(), Hold::Asking { missed: 0 });
        assert_eq!(a.heard(Heard::Nobody), Some(Hold::Asking { missed: 1 }));
        assert_eq!(a.heard(Heard::Nobody), Some(Hold::Running));
        let mut a = Ask::new(2);
        assert_eq!(a.heard(Heard::Idle), Some(Hold::Running));
        assert_eq!(a.heard(Heard::Nobody), None);
        assert_eq!(a.heard(Heard::Unclear), None);
        // A split line healed: the standby had taken over, so this one waits.
        assert_eq!(a.heard(Heard::Active), Some(Hold::Held));
        assert_eq!(a.heard(Heard::Active), None);
        assert_eq!(a.heard(Heard::Unclear), None, "held, an unclear answer keeps it waiting");
        assert_eq!(a.heard(Heard::Nobody), Some(Hold::Asking { missed: 1 }));
        assert_eq!(a.heard(Heard::Active), Some(Hold::Held));
        assert_eq!(a.heard(Heard::Idle), Some(Hold::Running));
        let mut a = Ask::new(3);
        assert_eq!(a.heard(Heard::Unclear), Some(Hold::Running), "asking, an unclear answer runs as without [primary]");
        let mut a = Ask::new(1);
        assert_eq!(a.heard(Heard::Active), Some(Hold::Held));
        assert_eq!(a.heard(Heard::Nobody), Some(Hold::Running));
    }

    fn net(takeover: Takeover) -> Jobnet {
        let job = |id: &str, after: &[&str]| Job {
            id: id.into(),
            run: vec!["x".into()],
            after: after.iter().map(|s| Line::ok(s)).collect(),
            ..Default::default()
        };
        Jobnet { jobs: vec![job("a", &[]), job("b", &["a"])], takeover, ..Default::default() }
    }

    fn theirs(a: RunState, b: RunState) -> Run {
        let at = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(2, 0, 0).unwrap();
        let mut r = Run::new("20261101-020000-00", "g/n", "n", "2026-11-01", How::Schedule, &net(Takeover::Leave), at);
        for (j, s) in r.jobs.iter_mut().zip([a, b]) {
            j.state = s;
            j.ended = s.is_final().then_some(at);
        }
        r.state = RunState::Running;
        r
    }

    /// only a net here that says `takeover = "rerun"` runs again, and only when a job is left.
    #[test]
    fn only_a_rerun_net_is_resumed() {
        let r = theirs(RunState::Ok, RunState::Running);
        assert_eq!(resume(&r, Some(&net(Takeover::Rerun))), Resume::Rerun);
        assert!(matches!(resume(&r, Some(&net(Takeover::Leave))), Resume::Leave(w) if w.contains("takeover")));
        assert!(matches!(resume(&r, None), Resume::Leave(w) if w.contains("定義がない")));
        let done = theirs(RunState::Ok, RunState::Skipped);
        assert!(matches!(resume(&done, Some(&net(Takeover::Rerun))), Resume::Leave(w) if w.contains("済んでいた")));
    }
}
