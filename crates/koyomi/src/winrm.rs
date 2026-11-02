//! `koyomi winrm run`: what a `winrm` job runs. As `koyomi ssh run` does, it
//! reads what to do from the `KOYOMI_WINRM_*` the engine sets, runs the command
//! on the far Windows PC (WinRM over HTTPS, its certificate pinned, Basic or NTLM), passes
//! its output on line by line as it comes (the far standard error after `! `) and
//! ends with the far command's exit code; without one, with [`NO_EXIT`] and why in
//! the file `KOYOMI_REASON` names. The certificate to pin comes from `koyomi winrm
//! cert`. `on = "group:app"` runs it on each PC of a group of koyomi.toml as an
//! `ssh` does (`crate::ssh::group`), each PC with its own `cert` or else the job's.
//!
//! The engine stops it as it does an `ssh` (see `ssh.rs`). ito-winrm looks at the
//! flag at every step: before the command is sent nothing starts there; after, the
//! far command is asked to end and its shell deleted before this ends.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use koyomi_core::chore::{self, Sftp, WinrmAuth, WINRM_PORT};

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
    // The connection is tried here first: a refused port must not wait out the time limit
    // (Windows takes about a second to refuse, longer than the short tries below).
    if let Err(why) = probe(&host, port) {
        crate::say(&format!("koyomi: {host}:{port}: {why}"), true);
        return 1;
    }
    match ito_winrm::peek_fingerprint(&host, port, Duration::from_secs(15), &AtomicBool::new(false)) {
        Ok(f) => {
            crate::say(&format!("cert = \"{}\"", ito_winrm::fingerprint::format(&f)), false);
            0
        }
        Err(e) => {
            let why = match e {
                ito_winrm::Error::Timeout => format!("時間切れ（15 秒たっても証明書が来なかった。{LISTENER}）"),
                e => reason(&e, WinrmAuth::Basic),
            };
            crate::say(&format!("koyomi: {host}:{port}: {why}"), true);
            1
        }
    }
}

const LISTENER: &str = "相手の WinRM の HTTPS のリスナーとファイアウォールのポートを確かめる";

/// Whether `host:port` takes a connection, and if not, why in one line of Japanese.
fn probe(host: &str, port: u16) -> Result<(), String> {
    use std::net::ToSocketAddrs;
    let addrs: Vec<_> =
        (host, port).to_socket_addrs().map_err(|e| format!("つながらない（ホストが見つからない）: {}", plain(&e.to_string())))?.collect();
    let mut last = None;
    for a in &addrs {
        match std::net::TcpStream::connect_timeout(a, Duration::from_secs(5)) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        None => "つながらない（ホストにアドレスがない）".to_owned(),
        Some(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            format!("つながらない（相手がこのポートで待ち受けていない。{LISTENER}）")
        }
        Some(e) if e.kind() == std::io::ErrorKind::TimedOut => format!("時間切れ（5 秒たっても答えがない。{LISTENER}）"),
        Some(e) => format!("つながらない（{}）", plain(&e.to_string())),
    })
}

/// An error text on one line, with a Rust debug dump (`HandlerError(IO(Os { code: …`) turned
/// into what the OS error code means.
fn plain(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !one.contains("HandlerError") && !one.contains("Os {") {
        return one;
    }
    let code = one.split("code:").nth(1).map(|r| r.trim_start().chars().take_while(char::is_ascii_digit).collect::<String>());
    match code.as_deref() {
        Some("10061" | "111") => "相手がこのポートで待ち受けていない".to_owned(),
        Some("10060" | "110") => "相手から答えがない".to_owned(),
        Some("10054" | "10053" | "104") => "接続が相手に切られた".to_owned(),
        Some("10051" | "10065" | "101" | "113") => "相手までの道がない".to_owned(),
        Some("11001" | "11004") => "ホストが見つからない".to_owned(),
        Some(c) if !c.is_empty() => format!("OS のエラー {c}"),
        _ => "接続で失敗した".to_owned(),
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
    // As it is written: not trimmed.
    let command = std::env::var("KOYOMI_WINRM_COMMAND").ok().filter(|c| !c.trim().is_empty()).ok_or("command がない")?;
    let cert = var("CERT");
    let (group_name, max, pass) = crate::ssh::fan_of("KOYOMI_WINRM_", &on)?;
    // Read once, before a group's PCs start.
    let password = crate::sftp::password(var("PASSWORD").ok_or("password がない（secret:<名前> で書く）")?)?;
    let auth = match var("AUTH") {
        None => WinrmAuth::Basic,
        Some(a) => WinrmAuth::of(&a).ok_or_else(|| format!("auth は \"basic\" か \"ntlm\": {a}"))?,
    };
    if let Some(name) = group_name {
        let members = crate::ssh::members(name, "winrm")?;
        return crate::ssh::group(name, &members, max, pass, stop, &say, |m, out| {
            let far = chore::winrm(&m.on).ok_or_else(|| format!("winrm:// の PC ではない: {}", m.on))??;
            let cert =
                m.cert.as_deref().or(cert.as_deref()).ok_or(
                    "cert がない（koyomi.toml の [groups] でこの PC に書くか、ジョブに書く。koyomi winrm cert <ホスト> で調べる）",
                )?;
            one(&far, cert, &password, auth, &command, stop, out)
        });
    }
    let far = chore::winrm(&on).ok_or_else(|| format!("on は winrm://ユーザー@ホスト[:ポート] か group:組の名前 で書く: {on}"))??;
    let cert = cert.ok_or("cert がない（koyomi winrm cert <ホスト> で調べる）")?;
    one(&far, &cert, &password, auth, &command, stop, &say).map_err(|e| format!("{}: {e}", far.host))
}

/// Runs `command` on `far`, its lines to `out`: its exit code, or why there is
/// none (without the host, which the caller adds).
fn one(
    far: &Sftp,
    cert: &str,
    password: &str,
    auth: WinrmAuth,
    command: &str,
    stop: &AtomicBool,
    out: &dyn Fn(&str, bool),
) -> Result<i32, String> {
    let user = far.user.clone().ok_or("winrm:// にはユーザーが要る")?;
    let pin = ito_winrm::fingerprint::parse(cert).ok_or_else(|| format!("cert が読めない（AB:CD:… の形）: {cert}"))?;
    let mut target = ito_winrm::Target::https(&far.host, &user, password, Some(pin)).with_auth(match auth {
        WinrmAuth::Basic => ito_winrm::Auth::Basic,
        WinrmAuth::Ntlm => ito_winrm::Auth::Ntlm,
    });
    target.port = far.port.unwrap_or(WINRM_PORT);
    let opts = ito_winrm::ExecOptions { env: crate::ssh::forwarded("KOYOMI_WINRM_ENV"), timeout: None };
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let result = ito_winrm::exec(&target, command, &opts, stop, &mut |stream, bytes| {
        let (buf, stderr) = match stream {
            ito_winrm::Stream::Stdout => (&mut o, false),
            ito_winrm::Stream::Stderr => (&mut e, true),
        };
        for line in cut(buf, bytes) {
            out(&line, stderr);
        }
    });
    for (buf, stderr) in [(o, false), (e, true)] {
        if !buf.is_empty() {
            out(&text(&buf), stderr);
        }
    }
    let exit = result.map_err(|e| reason(&e, auth))?;
    // A Windows code such as 0xC0000005 goes through as the same bits.
    exit.status.map(|code| code as i32).ok_or_else(|| "終了コードを受け取れなかった".to_owned())
}

/// Why it failed, as the job's ✕ reason: what to look at on which side.
fn reason(e: &ito_winrm::Error, auth: WinrmAuth) -> String {
    use ito_winrm::Error as E;
    match e {
        E::Stopped => "止めた（コマンドが始まっていれば止まるよう頼み、相手のシェルを消した）".into(),
        E::AuthRefused if auth == WinrmAuth::Ntlm => "ユーザーかパスワードを断られた（NTLM。ドメインのアカウントは on に winrm://CORP\\名前@ホスト と書く。相手の WinRM で Negotiate が有効か: winrm get winrm/config/service/auth）".into(),
        E::AuthRefused => "ユーザーかパスワードを断られた（相手の WinRM で Basic が有効か: winrm set winrm/config/service/auth @{Basic=\"true\"}。Basic で入れるのは相手の PC のローカルのアカウントだけ。ドメインのアカウントなら auth = \"ntlm\"）".into(),
        E::Fingerprint(got) => format!("証明書が cert と違う（相手が出したのは {got}。証明書を入れ替えたのなら koyomi winrm cert で確かめて cert を直す）"),
        E::Connect(e) => format!("つながらない（WinRM の HTTPS のリスナーとファイアウォールの 5986）: {}", plain(e)),
        E::Tls(e) => format!("TLS で失敗した: {}", plain(e)),
        E::Lost(e) => format!("接続が切れた: {}", plain(e)),
        E::Timeout => "時間切れ".into(),
        E::Fault(e) => format!("相手の WinRM が断った: {}", plain(e)),
        E::Other(e) => plain(e),
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
        let basic = |e: &E| reason(e, WinrmAuth::Basic);
        assert!(basic(&E::AuthRefused).contains("Basic=\"true\"") && basic(&E::AuthRefused).contains("auth = \"ntlm\""));
        let ntlm = reason(&E::AuthRefused, WinrmAuth::Ntlm);
        assert!(ntlm.contains("CORP\\名前") && !ntlm.contains("Basic"), "{ntlm}");
        let fp = "AB:CD";
        assert!(basic(&E::Fingerprint(fp.into())).contains("相手が出したのは AB:CD"));
        assert!(basic(&E::Connect("refused".into())).contains("5986"));
        assert!(basic(&E::Stopped).starts_with("止めた"));
        assert_eq!(basic(&E::Other("x".into())), "x");
        let dump = "HandlerError(IO(Os { code: 10061, kind: ConnectionRefused, message: \"x\" }))";
        for e in [E::Connect(dump.into()), E::Tls(dump.into()), E::Lost(dump.into()), E::Other(dump.into())] {
            let r = basic(&e);
            assert!(!r.contains("HandlerError") && !r.contains("Os {") && !r.contains('\n'), "{r}");
        }
        assert!(basic(&E::Connect(dump.into())).contains("待ち受けていない"));
    }
}
