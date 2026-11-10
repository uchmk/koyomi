//! `koyomi service`: the engine as a Windows service that logs on as the person's own account, so
//! nets run without anyone signed in (TODO 86, DECISIONS「Windows のサービスとして動かす」).
//!
//! The account is the person's, so the credential store, `state_dir`, `app_dir` and the engine's
//! door stay where they are. The password goes to the Service Control Manager and nowhere else.
//! A service has no screen, so what the engine says goes to `app_dir`'s `engine.log`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ito_ipc::door::ask;
use ito_service::{Error, Install, Start, State};
use koyomi_core::msg::{self, line};

use crate::proto::{self, Request};

/// How long a stop is waited for: the engine ends the processes of the jobs still running first.
const STOP_WAIT: Duration = Duration::from_secs(60);
/// How long a start is waited for, until the engine answers at its door.
const START_WAIT: Duration = Duration::from_secs(30);
/// `engine.log` is set aside as `engine.old.log` when the service starts and it is bigger than this.
const LOG_KEEP: u64 = 1024 * 1024;

/// Set while the engine runs as the service. The service runs apart from the
/// signed-in person, so how long they have left the PC alone is not seen there.
pub static RUNNING: AtomicBool = AtomicBool::new(false);

/// The service's name: one per person on the PC, like the engine's door.
pub fn name() -> String {
    name_on(cfg!(windows), &std::env::var("USERNAME").unwrap_or_default())
}

/// Windows has one service per person; a Linux machine has the one unit.
fn name_on(windows: bool, user: &str) -> String {
    if windows {
        service_name(user)
    } else {
        "koyomi".to_owned()
    }
}

fn service_name(user: &str) -> String {
    format!("koyomi-{user}")
}

/// The account the service logs on as: the one running this command.
fn account() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{d}\\{user}"),
        _ => format!(".\\{user}"),
    }
}

/// Whether the service is installed, and where it is.
pub fn state() -> Option<State> {
    ito_service::state(&name()).ok().flatten().map(|s| s.state)
}

/// Like [`state`], but a failed query is an error (its text) and not "no service".
pub fn query() -> Result<Option<State>, String> {
    ito_service::state(&name()).map(|o| o.map(|s| s.state)).map_err(|e| e.to_string())
}

/// `koyomi service [install|uninstall|start|stop|status|run]`.
pub fn cmd(args: &[String]) -> i32 {
    let res = match args {
        [] => status(),
        [a] => match a.as_str() {
            "status" => status(),
            "install" => install(),
            "uninstall" => uninstall(),
            "start" => start(),
            "stop" => stop(),
            "run" => return run(),
            _ => return usage(),
        },
        _ => return usage(),
    };
    match res {
        Ok(text) => {
            crate::say(&format!("koyomi: {text}"), false);
            0
        }
        Err(e) => {
            crate::say(&format!("koyomi: {e}"), true);
            1
        }
    }
}

fn usage() -> i32 {
    crate::say("koyomi: service takes install, uninstall, start, stop or status", true);
    2
}

fn status() -> Result<String, String> {
    let s = ito_service::state(&name()).map_err(|e| why(&e))?;
    Ok(status_line(&name(), s))
}

fn status_line(name: &str, s: Option<ito_service::Status>) -> String {
    match s {
        None => msg::note(msg::N_SERVICE_NONE, &[]),
        Some(s) => {
            let state = match s.state {
                State::Running => msg::note(msg::N_SERVICE_RUNNING, &[&s.pid.to_string()]),
                State::Starting => msg::note(msg::N_SERVICE_STARTING, &[]),
                State::Stopping => msg::note(msg::N_SERVICE_STOPPING, &[]),
                State::Stopped => msg::note(msg::N_SERVICE_STOPPED, &[]),
            };
            msg::note(msg::N_SERVICE_LINE, &[name, &state])
        }
    }
}

fn install() -> Result<String, String> {
    if state().is_some() {
        return Err(line(msg::SERVICE_INSTALLED_ALREADY, &[&name()]));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    crate::cli::refuse_temp(&exe)?;
    let account = account();
    let password = ito_service::read_password(&msg::note(msg::N_SERVICE_PASSWORD_ASK, &[&account])).map_err(|e| why(&e))?;
    if password.is_empty() {
        return Err(line(msg::SERVICE_PASSWORD_EMPTY, &[]));
    }
    let user = std::env::var("USERNAME").unwrap_or_default();
    ito_service::install(&Install {
        name: name(),
        display: format!("koyomi ({user})"),
        description: msg::note(msg::N_SERVICE_DESCRIPTION, &[]),
        exe,
        args: vec!["service".to_owned(), "run".to_owned()],
        account,
        password,
        start: Start::AutoDelayed,
    })
    .map_err(|e| why(&e))?;
    let mut text = msg::note(msg::N_SERVICE_INSTALLED, &[&name()]);
    if restart_entry() == Ok(true) {
        text.push(' ');
        text.push_str(&msg::note(msg::N_SERVICE_INSTALLED_TRAY, &[]));
    }
    if crate::client::running() {
        text.push(' ');
        text.push_str(&msg::note(msg::N_SERVICE_INSTALLED_BUSY, &[]));
    } else {
        text.push('\n');
        text.push_str(&start()?);
    }
    Ok(text)
}

fn uninstall() -> Result<String, String> {
    if state().is_none() {
        return Err(line(msg::SERVICE_NOT_INSTALLED, &[&name()]));
    }
    ito_service::stop(&name(), STOP_WAIT).map_err(|e| why(&e))?;
    ito_service::uninstall(&name()).map_err(|e| why(&e))?;
    let then = match restart_entry() {
        Ok(true) => msg::note(msg::N_SERVICE_THEN_ENGINE, &[]),
        Ok(false) => msg::note(msg::N_SERVICE_THEN_SCREEN, &[]),
        // The entry still starts only the tray: the exe that would take its place is in Temp.
        Err(_) => msg::note(msg::N_SERVICE_THEN_TEMP, &[]),
    };
    Ok(format!("{} {then}", msg::note(msg::N_SERVICE_REMOVED, &[&name()])))
}

fn start() -> Result<String, String> {
    if state().is_none() {
        return Err(line(msg::SERVICE_NOT_INSTALLED, &[&name()]));
    }
    if state() != Some(State::Running) && crate::client::running() {
        return Err(line(msg::SERVICE_SIGNIN_ENGINE_RUNNING, &[]));
    }
    ito_service::start(&name()).map_err(|e| why(&e))?;
    let until = Instant::now() + START_WAIT;
    while Instant::now() < until {
        if crate::client::running() {
            return Ok(msg::note(msg::N_SERVICE_STARTED, &[&name()]));
        }
        if state() == Some(State::Stopped) {
            return Err(line(msg::SERVICE_STOPPED_AT_START, &[&name(), &log_path().map(|p| p.display().to_string()).unwrap_or_default()]));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(line(msg::SERVICE_NO_ANSWER, &[&name(), &START_WAIT.as_secs().to_string()]))
}

fn stop() -> Result<String, String> {
    if state().is_none() {
        return Err(line(msg::SERVICE_NOT_INSTALLED, &[&name()]));
    }
    ito_service::stop(&name(), STOP_WAIT).map_err(|e| why(&e))?;
    Ok(msg::note(msg::N_SERVICE_STOPPED_DONE, &[&name()]))
}

/// Points the sign-in entry, when there is one, at what it should start now: the tray beside a
/// service, the engine without one. True when there was one. An exe in Temp is refused, and the
/// entry is left as it was.
fn restart_entry() -> Result<bool, String> {
    if !ito_common::startup::enabled("koyomi") {
        return Ok(false);
    }
    let Ok(exe) = std::env::current_exe() else {
        return Ok(false);
    };
    crate::cli::refuse_temp(&exe)?;
    Ok(ito_common::startup::set("koyomi", &exe, &crate::cli::startup_args(), true).is_ok())
}

/// What the Service Control Manager runs: the engine, until the service is asked to stop.
fn run() -> i32 {
    let res = ito_service::run(&name(), |stop| {
        if let Some(p) = log_path() {
            crate::log_to(&p, LOG_KEEP);
        }
        std::thread::spawn(move || {
            while !stop.wait(Duration::from_secs(3600)) {}
            // The same end as `koyomi engine stop`: the jobs' process trees are ended. The door may
            // not be up yet when the stop comes right after the start.
            let until = Instant::now() + START_WAIT;
            while ask(&proto::door(), &Request::Stop).is_err() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        RUNNING.store(true, Ordering::Relaxed);
        crate::engine::run(false)
    });
    match res {
        Ok(()) => 0,
        Err(Error::NotService) => {
            crate::say(&format!("koyomi: {}", line(msg::SERVICE_NOT_RUN_BY_WINDOWS, &[])), true);
            2
        }
        Err(e) => {
            crate::say(&format!("koyomi: {}", why(&e)), true);
            1
        }
    }
}

/// `app_dir`'s `engine.log`: where the service's engine writes what it would say on stderr.
pub fn log_path() -> Option<PathBuf> {
    ito_common::app_dir("koyomi").map(|d| d.join("engine.log"))
}

/// The person's words for why a service call failed.
fn why(e: &Error) -> String {
    match e {
        Error::Unsupported => line(msg::SERVICE_UNSUPPORTED, &[]),
        Error::NotService => line(msg::SERVICE_NOT_RUN_BY_WINDOWS, &[]),
        Error::NeedsAdmin => line(msg::SERVICE_NEEDS_ADMIN, &[]),
        Error::NotInstalled => line(msg::SERVICE_NOT_INSTALLED, &[&name()]),
        Error::AlreadyExists => line(msg::SERVICE_INSTALLED_ALREADY, &[&name()]),
        Error::BadName => line(msg::SERVICE_NAME_BAD, &[&name()]),
        Error::BadAccount => line(msg::SERVICE_ACCOUNT_BAD, &[&account()]),
        Error::LogonFailed => line(msg::SERVICE_LOGON_FAILED, &[]),
        Error::Timeout => line(msg::SERVICE_TIMEOUT, &[]),
        Error::NoConsole => line(msg::SERVICE_NO_CONSOLE, &[]),
        Error::Failed(n) => line(msg::SERVICE_REFUSED, &[&n.to_string()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the service is one per person, so two people on one PC each have their own.
    #[test]
    fn the_name_follows_the_person() {
        assert_eq!(service_name("yuu"), "koyomi-yuu");
        assert!(ito_service::valid_name(&service_name("yuu")));
    }

    #[test]
    fn linux_has_the_one_unit_and_windows_one_per_person() {
        assert_eq!(name_on(true, "yuu"), "koyomi-yuu");
        assert_eq!(name_on(false, "yuu"), "koyomi");
        assert_eq!(name_on(false, ""), "koyomi");
    }

    /// status says not installed, running with the pid, and how to start a stopped one.
    #[test]
    fn the_status_line_says_where_it_is() {
        assert_eq!(status_line("koyomi-a", None), msg::note(msg::N_SERVICE_NONE, &[]));
        assert!(status_line("koyomi-a", None).contains("koyomi service install"));
        let s = |state, pid| Some(ito_service::Status { state, pid });
        assert_eq!(
            status_line("koyomi-a", s(State::Running, 42)),
            msg::note(msg::N_SERVICE_LINE, &["koyomi-a", &msg::note(msg::N_SERVICE_RUNNING, &["42"])])
        );
        assert!(status_line("koyomi-a", s(State::Running, 42)).contains("pid 42"));
        assert!(status_line("koyomi-a", s(State::Stopped, 0)).contains("koyomi service start"));
    }

    /// the reasons are the person's words, and the admin one says what to do.
    #[test]
    fn the_reasons_say_what_to_do() {
        assert!(msg::SERVICE_NEEDS_ADMIN.is_in(&why(&Error::NeedsAdmin)));
        assert!(why(&Error::NeedsAdmin).contains("管理者として実行"));
        assert!(msg::SERVICE_LOGON_FAILED.is_in(&why(&Error::LogonFailed)));
        assert!(why(&Error::LogonFailed).contains("パスワードを確認"));
        assert!(msg::SERVICE_REFUSED.is_in(&why(&Error::Failed(5))));
        assert!(why(&Error::Failed(5)).contains("5"));
    }
}
