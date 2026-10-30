//! The standby's decision for HA (TODO 87, DECISIONS「複数の PC・Web UI・利用者とロール・HA」).
//!
//! The standby polls the primary's `GET /health` every `interval`. After `misses` answers in a
//! row that do not come, it takes over: from then on its own engine runs the nets' schedules. It
//! does not hand back by itself when the primary answers again; that is a person's
//! `koyomi standby giveback`, so the two never flip back and forth on a flaky line. While it has
//! taken over and the primary answers, it says once that both are running.

use std::time::Duration;

/// How often the primary is polled when `[standby]` does not say.
pub const INTERVAL: Duration = Duration::from_secs(20);
/// How many misses in a row before taking over when `[standby]` does not say.
pub const MISSES: u32 = 3;

/// Where the standby is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Watching the primary; `missed` answers in a row have not come.
    Watching { missed: u32 },
    /// Took over; `both_said` once the primary was seen answering again.
    Active { both_said: bool },
}

/// What the standby does after a poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing changes.
    Stay,
    /// Start running the schedules here, and say so.
    TakeOver,
    /// The primary answers while this one is active: say so once (a person hands back).
    BothActive,
}

/// The standby's watch on one primary.
#[derive(Clone, Debug)]
pub struct Watch {
    misses: u32,
    state: State,
}

impl Watch {
    /// `misses` below 1 is taken as 1.
    pub fn new(misses: u32) -> Watch {
        Watch { misses: misses.max(1), state: State::Watching { missed: 0 } }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn active(&self) -> bool {
        matches!(self.state, State::Active { .. })
    }

    /// One poll of the primary: `answered` is whether its `/health` said ok.
    pub fn poll(&mut self, answered: bool) -> Step {
        match (self.state, answered) {
            (State::Watching { .. }, true) => {
                self.state = State::Watching { missed: 0 };
                Step::Stay
            }
            (State::Watching { missed }, false) if missed + 1 >= self.misses => {
                self.state = State::Active { both_said: false };
                Step::TakeOver
            }
            (State::Watching { missed }, false) => {
                self.state = State::Watching { missed: missed + 1 };
                Step::Stay
            }
            (State::Active { both_said: false }, true) => {
                self.state = State::Active { both_said: true };
                Step::BothActive
            }
            (State::Active { .. }, false) => {
                self.state = State::Active { both_said: false };
                Step::Stay
            }
            (State::Active { both_said: true }, true) => Step::Stay,
        }
    }

    /// `koyomi standby giveback`: stop running the schedules here and watch again. False when this
    /// one had not taken over.
    pub fn give_back(&mut self) -> bool {
        let was = self.active();
        self.state = State::Watching { missed: 0 };
        was
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// it takes over only after the set misses in a row; an answer between starts the count again.
    #[test]
    fn it_takes_over_after_the_misses_in_a_row() {
        let mut w = Watch::new(3);
        assert_eq!([false, false, true, false, false].map(|a| w.poll(a)), [Step::Stay; 5]);
        assert_eq!(w.state(), State::Watching { missed: 2 });
        assert_eq!(w.poll(false), Step::TakeOver);
        assert!(w.active());
        assert_eq!(w.poll(false), Step::Stay);
    }

    /// once active it does not hand back by itself; it says once that both run, again after a miss.
    #[test]
    fn it_stays_active_and_says_both_once() {
        let mut w = Watch::new(1);
        assert_eq!(w.poll(false), Step::TakeOver);
        assert_eq!(w.poll(true), Step::BothActive);
        assert_eq!(w.poll(true), Step::Stay);
        assert!(w.active());
        assert_eq!(w.poll(false), Step::Stay);
        assert_eq!(w.poll(true), Step::BothActive);
    }

    /// giveback watches again from zero; on a watching one it says there was nothing to give.
    #[test]
    fn giveback_watches_again() {
        let mut w = Watch::new(0);
        assert!(!w.give_back());
        assert_eq!(w.poll(false), Step::TakeOver);
        assert!(w.give_back());
        assert_eq!(w.state(), State::Watching { missed: 0 });
        assert_eq!(w.poll(true), Step::Stay);
    }
}
