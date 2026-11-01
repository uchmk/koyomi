//! `koyomi service`: the engine as a Windows service that logs on as the person's own account, so
//! nets run without anyone signed in (TODO 86, DECISIONS「Windows のサービスとして動かす」).
//!
//! The account is the person's, so the credential store, `state_dir`, `app_dir` and the engine's
//! door stay where they are. The password goes to the Service Control Manager and nowhere else.
//! A service has no screen, so what the engine says goes to `app_dir`'s `engine.log`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ito_ipc::door::ask;
use ito_service::{Error, Install, Start, State};

use crate::proto::{self, Request};

/// How long a stop is waited for: the engine ends the processes of the jobs still running first.
const STOP_WAIT: Duration = Duration::from_secs(60);
/// How long a start is waited for, until the engine answers at its door.
const START_WAIT: Duration = Duration::from_secs(30);
/// `engine.log` is set aside as `engine.old.log` when the service starts and it is bigger than this.
const LOG_KEEP: u64 = 1024 * 1024;

/// The service's name: one per person on the PC, like the engine's door.
pub fn name() -> String {
    service_name(&std::env::var("USERNAME").unwrap_or_default())
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
        None => "サービスは入っていない（`koyomi service install` で入れる）".to_owned(),
        Some(s) => {
            let state = match s.state {
                State::Running => format!("動いている（pid {}）", s.pid),
                State::Starting => "始めているところ".to_owned(),
                State::Stopping => "止めているところ".to_owned(),
                State::Stopped => "止まっている（`koyomi service start` で始める）".to_owned(),
            };
            format!("サービス {name}: {state}")
        }
    }
}

fn install() -> Result<String, String> {
    if state().is_some() {
        return Err(format!("サービス {} はもう入っている", name()));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    crate::cli::refuse_temp(&exe)?;
    let account = account();
    let password = ito_service::read_password(&format!("{account} のパスワード（Windows のサインインのもの）: ")).map_err(|e| why(&e))?;
    if password.is_empty() {
        return Err("パスワードが空。パスワードの無い（PIN だけの）アカウントでは入れられない".to_owned());
    }
    let user = std::env::var("USERNAME").unwrap_or_default();
    ito_service::install(&Install {
        name: name(),
        display: format!("koyomi ({user})"),
        description: "koyomi のエンジン。サインインしていなくてもネットを流す。".to_owned(),
        exe,
        args: vec!["service".to_owned(), "run".to_owned()],
        account,
        password,
        start: Start::AutoDelayed,
    })
    .map_err(|e| why(&e))?;
    let mut text = format!("サービス {} を入れた（PC が起きたら少し遅れて始まる）", name());
    if restart_entry() == Ok(true) {
        text.push_str("。サインイン時にはトレイだけを起こす");
    }
    if crate::client::running() {
        text.push_str("。今はサインインで起こしたエンジンが動いている。`koyomi engine stop` で止めてから `koyomi service start` で始める");
    } else {
        text.push('\n');
        text.push_str(&start()?);
    }
    Ok(text)
}

fn uninstall() -> Result<String, String> {
    if state().is_none() {
        return Err(format!("サービス {} は入っていない", name()));
    }
    ito_service::stop(&name(), STOP_WAIT).map_err(|e| why(&e))?;
    ito_service::uninstall(&name()).map_err(|e| why(&e))?;
    let then = match restart_entry() {
        Ok(true) => "サインイン時にはまたエンジンを起こす",
        Ok(false) => "エンジンは窓を開けば起きる（サインイン時に起こすなら `koyomi startup on`）",
        // The entry still starts only the tray: the exe that would take its place is in Temp.
        Err(_) => {
            "サインイン時の起動はトレイだけを起こすまま（この exe は Temp にあるので向け直さなかった）。置いた場所の koyomi で `koyomi startup on` を回す"
        }
    };
    Ok(format!("サービス {} を外した。{then}", name()))
}

fn start() -> Result<String, String> {
    if state().is_none() {
        return Err(format!("サービス {} は入っていない（`koyomi service install` で入れる）", name()));
    }
    if state() != Some(State::Running) && crate::client::running() {
        return Err("サインインで起こしたエンジンが動いている。`koyomi engine stop` で止めてから始める".to_owned());
    }
    ito_service::start(&name()).map_err(|e| why(&e))?;
    let until = Instant::now() + START_WAIT;
    while Instant::now() < until {
        if crate::client::running() {
            return Ok(format!("サービス {} を始めた", name()));
        }
        if state() == Some(State::Stopped) {
            return Err(format!(
                "サービス {} が始まってすぐ止まった。{} を見る",
                name(),
                log_path().map(|p| p.display().to_string()).unwrap_or_default()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(format!("サービス {} のエンジンが {} 秒たっても答えない", name(), START_WAIT.as_secs()))
}

fn stop() -> Result<String, String> {
    if state().is_none() {
        return Err(format!("サービス {} は入っていない", name()));
    }
    ito_service::stop(&name(), STOP_WAIT).map_err(|e| why(&e))?;
    Ok(format!("サービス {} を止めた（動いていたジョブのプロセスは終えた）", name()))
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
        crate::engine::run(false)
    });
    match res {
        Ok(()) => 0,
        Err(Error::NotService) => {
            crate::say(
                "koyomi: `koyomi service run` は Windows のサービスとして起こされたときだけ動く（手で流すなら `koyomi engine`）",
                true,
            );
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
        Error::Unsupported => "サービスは Windows でだけ動く".to_owned(),
        Error::NotService => "Windows のサービスとして起こされていない".to_owned(),
        Error::NeedsAdmin => "管理者の権限が要る。「管理者として実行」で開いたターミナルで回す".to_owned(),
        Error::NotInstalled => format!("サービス {} は入っていない", name()),
        Error::AlreadyExists => format!("サービス {} はもう入っている", name()),
        Error::BadName => format!("サービスの名前 {} が使えない", name()),
        Error::BadAccount => format!("アカウント {} が無いか、パスワードが無い（PIN だけのアカウントは使えない）", account()),
        Error::LogonFailed => {
            "サービスがサインインできなかった。パスワードが違う（変えたら `koyomi service uninstall` してから入れ直す）".to_owned()
        }
        Error::Timeout => "サービスが時間内に切り替わらなかった。`koyomi service status` で見る".to_owned(),
        Error::NoConsole => {
            "パスワードを打つ所が無い。ターミナルから `koyomi.com service install` で回すか、パスワードをパイプで渡す".to_owned()
        }
        Error::Failed(n) => format!("Windows が断った（エラー {n}）"),
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

    /// status says not installed, running with the pid, and how to start a stopped one.
    #[test]
    fn the_status_line_says_where_it_is() {
        assert!(status_line("koyomi-a", None).contains("入っていない"));
        let s = |state, pid| Some(ito_service::Status { state, pid });
        assert_eq!(status_line("koyomi-a", s(State::Running, 42)), "サービス koyomi-a: 動いている（pid 42）");
        assert!(status_line("koyomi-a", s(State::Stopped, 0)).contains("koyomi service start"));
    }

    /// the reasons are the person's words, and the admin one says what to do.
    #[test]
    fn the_reasons_say_what_to_do() {
        assert!(why(&Error::NeedsAdmin).contains("管理者として実行"));
        assert!(why(&Error::LogonFailed).contains("パスワードが違う"));
        assert!(why(&Error::Failed(5)).contains("5"));
    }
}
