//! A run's end as a line in the system log (Windows: the Application log, source `koyomi`), so the
//! monitoring tool that watches the PC (Zabbix, JP1/IM, SCOM) sees a failure without reading
//! koyomi's own screens. One event id per state the monitor tells apart.

use ito_eventlog::Level;
use koyomi_core::run::State;

/// The source name the monitor filters on.
pub const SOURCE: &str = "koyomi";

/// The event id of an abnormal run that went well again.
pub const RECOVERED: u32 = 1100;
/// The event id of times that passed with the PC off or koyomi stopped.
pub const MISSED: u32 = 3000;

/// The level and event id for a run that ended in `state`.
pub fn of(state: State) -> (Level, u32) {
    match state {
        State::Ok => (Level::Info, 1000),
        State::Warn => (Level::Warning, 1001),
        State::Timeout => (Level::Error, 2001),
        State::Killed => (Level::Error, 2002),
        State::Locked => (Level::Error, 2003),
        State::Missed => (Level::Warning, MISSED),
        s if s.is_error() => (Level::Error, 2000),
        _ => (Level::Info, 1002),
    }
}

/// Writes one line; a PC with no log to write to is not an error for the run.
pub fn write(level: Level, id: u32, title: &str, body: &str) {
    // The tests end runs too: they must not write to the PC's own log.
    if cfg!(test) {
        return;
    }
    if let Ok(log) = ito_eventlog::Log::open(SOURCE) {
        let _ = log.write(
            level,
            id,
            &format!(
                "{title}
{body}"
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_ending_state_has_its_own_id_and_failures_are_errors() {
        assert_eq!(of(State::Ok), (Level::Info, 1000));
        assert_eq!(of(State::Error), (Level::Error, 2000));
        assert_eq!(of(State::Timeout).1, 2001);
        assert_eq!(of(State::Killed).1, 2002);
        assert_eq!(of(State::Warn).0, Level::Warning);
        assert_eq!(of(State::Missed).1, MISSED);
    }
}
