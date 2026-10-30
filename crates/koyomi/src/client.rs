//! Talking to the engine from the window and the CLI, starting it when it is not up.

use std::time::{Duration, Instant};

use ito_ipc::door::{ask, Reply};

use crate::proto::{self, Request};

/// How long a start is waited for before it is called failed.
const START_WAIT: Duration = Duration::from_secs(10);

/// Whether an engine answers.
pub fn running() -> bool {
    ask(&proto::door(), &Request::Status).is_ok()
}

/// Starts `koyomi engine` on its own (no window, no console, not a child that dies with this
/// process) and waits until it answers.
pub fn start() -> Result<(), String> {
    if running() {
        return Ok(());
    }
    // A service engine is started by Windows; another one beside it would fight it for the door.
    match crate::service::state() {
        None => spawn()?,
        Some(ito_service::State::Stopped | ito_service::State::Stopping) => {
            return Err("サービスのエンジンが止まっている。`koyomi service start` で始める".to_owned())
        }
        Some(_) => {}
    }
    let until = Instant::now() + START_WAIT;
    while Instant::now() < until {
        if running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("エンジンが起動しなかった".to_owned())
}

/// One request, starting the engine first when it is stopped.
pub fn call(request: &Request) -> Reply {
    start()?;
    ask(&proto::door(), request)
}

fn spawn() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("engine");
    ito_proc::spawn_detached(cmd).map_err(|e| format!("エンジンを起動できない: {e}"))
}
