//! Talking to the engine from the window and the CLI, starting it when it is not up.

use std::time::{Duration, Instant};

use ito_ipc::door::{ask, Reply};
use koyomi_core::msg::{self, line};

use crate::proto::{self, Request};

/// How long a start is waited for before it is called failed.
const START_WAIT: Duration = Duration::from_secs(10);

#[cfg(test)]
thread_local! {
    /// How many times [`start`] was called on this thread, so a test can prove a path never starts the engine.
    pub static STARTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether an engine answers.
pub fn running() -> bool {
    ask(&proto::door(), &Request::Status).is_ok()
}

/// Starts `koyomi engine` on its own (no window, no console, not a child that dies with this
/// process) and waits until it answers.
pub fn start() -> Result<(), String> {
    #[cfg(test)]
    STARTS.with(|c| c.set(c.get() + 1));
    if running() {
        return Ok(());
    }
    // A service engine is started by Windows; another one beside it would fight it for the door.
    match decide(cfg!(windows), crate::service::query()) {
        Step::Spawn => spawn()?,
        Step::Wait => {}
        Step::Refuse(why) => return Err(why),
    }
    let until = Instant::now() + START_WAIT;
    while Instant::now() < until {
        if running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(line(msg::ENGINE_NOT_STARTED, &[]))
}

/// What `start` does once the engine does not answer.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Spawn,
    Wait,
    Refuse(String),
}

/// The decision from the service query. On Linux a failed query starts nothing: the unit may be
/// installed, and an engine started outside it would fight it for the door.
fn decide(windows: bool, queried: Result<Option<ito_service::State>, String>) -> Step {
    use ito_service::State;
    match queried {
        Ok(None) => Step::Spawn,
        Ok(Some(State::Stopped | State::Stopping)) => {
            Step::Refuse(line(if windows { msg::ENGINE_SERVICE_STOPPED_WINDOWS } else { msg::ENGINE_SERVICE_STOPPED_LINUX }, &[]))
        }
        Ok(Some(_)) => Step::Wait,
        Err(_) if windows => Step::Spawn,
        Err(e) => Step::Refuse(line(msg::SERVICE_STATE_UNREADABLE, &[&e])),
    }
}

/// One request, starting the engine first when it is stopped.
pub fn call(request: &Request) -> Reply {
    start()?;
    ask(&proto::door(), request)
}

fn spawn() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| msg::io_words(&e))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("engine");
    ito_proc::spawn_detached(cmd).map_err(|e| line(msg::ENGINE_SPAWN_FAILED, &[&msg::io_words(&e)]))
}

#[cfg(test)]
mod tests {
    use ito_service::State;

    use super::*;

    #[test]
    fn no_service_spawns_on_both() {
        assert_eq!(decide(true, Ok(None)), Step::Spawn);
        assert_eq!(decide(false, Ok(None)), Step::Spawn);
    }

    #[test]
    fn a_stopped_service_is_refused_with_the_command_of_the_platform() {
        for s in [State::Stopped, State::Stopping] {
            assert_eq!(decide(true, Ok(Some(s))), Step::Refuse(line(msg::ENGINE_SERVICE_STOPPED_WINDOWS, &[])));
            assert_eq!(decide(false, Ok(Some(s))), Step::Refuse(line(msg::ENGINE_SERVICE_STOPPED_LINUX, &[])));
        }
    }

    #[test]
    fn a_service_that_is_up_is_waited_for() {
        for s in [State::Running, State::Starting] {
            assert_eq!(decide(true, Ok(Some(s))), Step::Wait);
            assert_eq!(decide(false, Ok(Some(s))), Step::Wait);
        }
    }

    #[test]
    fn a_failed_query_spawns_on_windows_and_refuses_on_linux() {
        assert_eq!(decide(true, Err("boom".into())), Step::Spawn);
        assert_eq!(decide(false, Err("boom".into())), Step::Refuse(line(msg::SERVICE_STATE_UNREADABLE, &["boom"])));
    }
}
