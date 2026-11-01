//! `koyomi winrm run`: what a `winrm` job runs. As `koyomi ssh run` does, it
//! reads what to do from the `KOYOMI_WINRM_*` the engine sets, runs the command
//! on the far Windows PC (WinRM over HTTPS, its certificate pinned, Basic), passes
//! its output on line by line as it comes (the far standard error after `! `) and
//! ends with the far command's exit code; without one, with [`NO_EXIT`] and why in
//! the file `KOYOMI_REASON` names. The certificate to pin comes from `koyomi winrm
//! cert`.
//!
//! The engine stops it as it does an `ssh` (see `ssh.rs`). ito-winrm looks at the
//! flag at every step: before the command is sent nothing starts there; after, the
//! far command is asked to end and its shell deleted before this ends.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use koyomi_core::chore::{self, WINRM_PORT};

use crate::ssh::{cut, say, text};

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "run" => crate::ssh::run(exec),
        [op, at] if op == "cert" => cert(at),
        _ => {
            crate::say("koyomi: winrm takes run (what a winrm job does), or cert <host>[:<port>] (the certificate to pin)", true);
            2
        }
    }
}

fn var(name: &str) -> Option<String> {
    crate::sftp::env_var("KOYOMI_WINRM_", name)
}

/// `koyomi winrm cert <host>[:<port>]` (or a `winrm://` address): the line to put
/// in the job.
fn cert(at: &str) -> i32 {
    let (host, port) = match place(at) {
        Ok(x) => x,
        Err(e) => {
            crate::say(&format!("koyomi: {e}"), true);
            return 2;
        }
    };
    match ito_winrm::peek_fingerprint(&host, port, Duration::from_secs(15), &AtomicBool::new(false)) {
        Ok(f) => {
            crate::say(&format!("cert = \"{}\"", ito_winrm::fingerprint::format(&f)), false);
            0
        }
        Err(e) => {
            crate::say(&format!("koyomi: {host}:{port}: {}", reason(&e)), true);
            1
        }
    }
}

/// The host and port of `koyomi winrm cert`'s `<host>[:<port>]` or
/// `winrm://[user@]host[:port]`; [`WINRM_PORT`] when no port is written.
fn place(at: &str) -> Result<(String, u16), String> {
    let t = at.trim();
    let rest = t.get(..8).filter(|p| p.eq_ignore_ascii_case("winrm://")).map_or(t, |_| &t[8..]);
    match chore::sftp(&format!("sftp://{rest}")) {
        Some(Ok(far)) if !rest.is_empty() => Ok((far.host, far.port.unwrap_or(WINRM_PORT))),
        _ => Err(format!("ホストを読めない: {at}（<ホスト>[:<ポート>] か winrm://ユーザー@ホスト[:ポート]）")),
    }
}

/// Runs the command; its exit code, or why there is none. `stop` set: this ends
/// without an exit code, at any step, and a far command that has started is asked
/// to end.
fn exec(stop: &AtomicBool) -> Result<i32, String> {
    let on = var("ON").ok_or("KOYOMI_WINRM_ON がない（koyomi winrm run は kind = \"winrm\" のジョブが動かす）")?;
    let far = chore::winrm(&on).ok_or_else(|| format!("on は winrm://ユーザー@ホスト[:ポート] で書く: {on}"))??;
    let user = far.user.clone().ok_or("winrm:// にはユーザーが要る")?;
    // As it is written: not trimmed.
    let command = std::env::var("KOYOMI_WINRM_COMMAND").ok().filter(|c| !c.trim().is_empty()).ok_or("command がない")?;
    let cert = var("CERT").ok_or("cert がない（koyomi winrm cert <ホスト> で調べる）")?;
    let pin = ito_winrm::fingerprint::parse(&cert).ok_or_else(|| format!("cert が読めない（AB:CD:… の形）: {cert}"))?;
    let password = crate::sftp::password(var("PASSWORD").ok_or("password がない（secret:<名前> で書く）")?)?;
    let mut target = ito_winrm::Target::https(&far.host, &user, &password, Some(pin));
    target.port = far.port.unwrap_or(WINRM_PORT);
    let opts = ito_winrm::ExecOptions { env: crate::ssh::forwarded("KOYOMI_WINRM_ENV"), timeout: None };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = ito_winrm::exec(&target, &command, &opts, stop, &mut |stream, bytes| {
        let (buf, stderr) = match stream {
            ito_winrm::Stream::Stdout => (&mut out, false),
            ito_winrm::Stream::Stderr => (&mut err, true),
        };
        for line in cut(buf, bytes) {
            say(&line, stderr);
        }
    });
    for (buf, stderr) in [(out, false), (err, true)] {
        if !buf.is_empty() {
            say(&text(&buf), stderr);
        }
    }
    let exit = result.map_err(|e| format!("{}: {}", far.host, reason(&e)))?;
    // A Windows code such as 0xC0000005 goes through as the same bits.
    exit.status.map(|code| code as i32).ok_or_else(|| format!("{}: 終了コードを受け取れなかった", far.host))
}

/// Why it failed, as the job's ✕ reason: what to look at on which side.
fn reason(e: &ito_winrm::Error) -> String {
    use ito_winrm::Error as E;
    match e {
        E::Stopped => "止めた（コマンドが始まっていれば止まるよう頼み、相手のシェルを消した）".into(),
        E::AuthRefused => "ユーザーかパスワードを断られた（相手の WinRM で Basic が有効か: winrm set winrm/config/service/auth @{Basic=\"true\"}。Basic で入れるのは相手の PC のローカルのアカウントだけ）".into(),
        E::Fingerprint(got) => format!("証明書が cert と違う（相手が出したのは {got}。証明書を入れ替えたのなら koyomi winrm cert で確かめて cert を直す）"),
        E::Connect(e) => format!("つながらない（WinRM の HTTPS のリスナーとファイアウォールの 5986）: {e}"),
        E::Tls(e) => format!("TLS で失敗した: {e}"),
        E::Lost(e) => format!("接続が切れた: {e}"),
        E::Timeout => "時間切れ".into(),
        E::Fault(e) => format!("相手の WinRM が断った: {e}"),
        E::Other(e) => e.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_takes_a_host_a_port_or_a_winrm_address() {
        assert_eq!(place("app01"), Ok(("app01".into(), 5986)));
        assert_eq!(place(" app01:5999 "), Ok(("app01".into(), 5999)));
        assert_eq!(place("WinRM://CORP\\ops@10.0.0.5:5986"), Ok(("10.0.0.5".into(), 5986)));
        assert_eq!(place("winrm://app01"), Ok(("app01".into(), 5986)));
        for bad in ["", "winrm://", "app01:x", "a b", "-h"] {
            assert!(place(bad).unwrap_err().contains("ホストを読めない"), "{bad}");
        }
    }

    #[test]
    fn each_failure_says_what_to_look_at() {
        use ito_winrm::Error as E;
        assert!(reason(&E::AuthRefused).contains("Basic=\"true\""));
        let fp = "AB:CD";
        assert!(reason(&E::Fingerprint(fp.into())).contains("相手が出したのは AB:CD"));
        assert!(reason(&E::Connect("refused".into())).contains("5986"));
        assert!(reason(&E::Stopped).starts_with("止めた"));
        assert_eq!(reason(&E::Other("x".into())), "x");
    }
}
