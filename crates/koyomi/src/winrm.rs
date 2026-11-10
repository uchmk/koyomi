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
use koyomi_core::msg::{self, line, note};

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
            crate::say(&e, true);
            return 2;
        }
    };
    // The connection is tried here first: a refused port must not wait out the time limit
    // (Windows takes about a second to refuse, longer than the short tries below).
    let place = format!("{host}:{port}");
    if let Err(why) = probe(&place, &host, port) {
        crate::say(&why, true);
        return 1;
    }
    match ito_winrm::peek_fingerprint(&host, port, Duration::from_secs(15), &AtomicBool::new(false)) {
        Ok(f) => {
            crate::say(&format!("cert = \"{}\"", ito_winrm::fingerprint::format(&f)), false);
            0
        }
        Err(e) => {
            crate::say(&reason(&e, &place, WinrmAuth::Basic), true);
            1
        }
    }
}

/// Whether `host:port` takes a connection, and if not, why as a coded line naming `place`.
fn probe(place: &str, host: &str, port: u16) -> Result<(), String> {
    use std::net::ToSocketAddrs;
    let refused = |detail: &str| line(msg::WINRM_CONNECT, &[place, detail]);
    let addrs: Vec<_> =
        (host, port).to_socket_addrs().map_err(|e| refused(&note(msg::N_NET_HOST_NOT_FOUND_WITH, &[&plain(&e.to_string())])))?.collect();
    let mut last = None;
    for a in &addrs {
        match std::net::TcpStream::connect_timeout(a, Duration::from_secs(5)) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        None => refused(&note(msg::N_NET_NO_ADDRESS, &[])),
        Some(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => refused(&note(msg::N_NET_REFUSED, &[])),
        Some(e) if e.kind() == std::io::ErrorKind::TimedOut => line(msg::LINK_TIMED_OUT, &[place]),
        Some(e) => refused(&plain(&e.to_string())),
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
        Some("10061" | "111") => note(msg::N_NET_REFUSED, &[]),
        Some("10060" | "110") => note(msg::N_NET_NO_ANSWER, &[]),
        Some("10054" | "10053" | "104") => note(msg::N_NET_CUT, &[]),
        Some("10051" | "10065" | "101" | "113") => note(msg::N_NET_NO_ROUTE, &[]),
        Some("11001" | "11004") => note(msg::N_NET_HOST_NOT_FOUND, &[]),
        Some(c) if !c.is_empty() => note(msg::N_NET_OS_ERROR, &[c]),
        _ => note(msg::N_NET_FAILED, &[]),
    }
}

/// The host and port of `koyomi winrm cert`'s `<host>[:<port>]` or
/// `winrm://[user@]host[:port]`; [`WINRM_PORT`] when no port is written.
fn place(at: &str) -> Result<(String, u16), String> {
    let t = at.trim();
    let rest = t.get(..8).filter(|p| p.eq_ignore_ascii_case("winrm://")).map_or(t, |_| &t[8..]);
    match chore::sftp(&format!("sftp://{rest}")) {
        Some(Ok(far)) if !rest.is_empty() => Ok((far.host, far.port.unwrap_or(WINRM_PORT))),
        _ => Err(line(msg::HOST_UNREADABLE, &[at, &note(msg::N_HOST_SHAPE_WINRM, &[])])),
    }
}

/// Runs the command; its exit code, or why there is none. `stop` set: this ends
/// without an exit code, at any step, and a far command that has started is asked
/// to end.
fn exec(stop: &AtomicBool) -> Result<i32, String> {
    let on = var("ON").ok_or_else(|| line(msg::JOB_ENV_MISSING, &["KOYOMI_WINRM_ON", "kind = \"winrm\"", "winrm"]))?;
    // As it is written: not trimmed.
    let command =
        std::env::var("KOYOMI_WINRM_COMMAND").ok().filter(|c| !c.trim().is_empty()).ok_or_else(|| line(msg::CHORE_COMMAND_EMPTY, &[]))?;
    let cert = var("CERT");
    let (group_name, max, pass) = crate::ssh::fan_of("KOYOMI_WINRM_", &on)?;
    // Read once, before a group's PCs start.
    let password = crate::sftp::password(var("PASSWORD").ok_or_else(|| line(msg::CHORE_PASSWORD_MISSING, &["winrm://"]))?)?;
    let auth = match var("AUTH") {
        None => WinrmAuth::Basic,
        Some(a) => WinrmAuth::of(&a).ok_or_else(|| line(msg::DEF_CHOICE_BAD, &["auth", "\"basic\", \"ntlm\""]))?,
    };
    if let Some(name) = group_name {
        let members = crate::ssh::members(name, "winrm")?;
        return crate::ssh::group(name, &members, max, pass, stop, &say, |m, out| {
            let far = chore::winrm(&m.on).ok_or_else(|| line(msg::GROUP_MEMBER_SCHEME, &[&m.on, "winrm://"]))??;
            let cert = m
                .cert
                .as_deref()
                .or(cert.as_deref())
                .ok_or_else(|| line(msg::GROUP_MEMBER_PIN_MISSING, &[&m.on, "cert", "koyomi winrm cert <host>"]))?;
            one(&far, cert, &password, auth, &command, stop, out)
        });
    }
    let far = chore::winrm(&on).ok_or_else(|| line(msg::CHORE_ON_BAD, &["winrm://user@host[:port]", &on]))??;
    let cert = cert.ok_or_else(|| line(msg::CHORE_WINRM_CERT_MISSING, &[]))?;
    // A coded reason already names the host and keeps its ID at the head.
    one(&far, &cert, &password, auth, &command, stop, &say).map_err(|e| if msg::is_coded(&e) { e } else { format!("{}: {e}", far.host) })
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
    let user = far.user.clone().ok_or_else(|| line(msg::CHORE_WINRM_USER, &[&far.host]))?;
    let pin = ito_winrm::fingerprint::parse(cert).ok_or_else(|| line(msg::CHORE_CERT_BAD, &[cert]))?;
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
    let exit = result.map_err(|e| reason(&e, &far.host, auth))?;
    // A Windows code such as 0xC0000005 goes through as the same bits.
    exit.status.map(|code| code as i32).ok_or_else(|| line(msg::FAR_NO_EXIT_CODE, &[]))
}

/// Why it failed, as the job's ✕ reason: a coded line naming `host` (`host:port` for `cert`).
fn reason(e: &ito_winrm::Error, host: &str, auth: WinrmAuth) -> String {
    use ito_winrm::Error as E;
    match e {
        E::Stopped => note(msg::N_STOPPED_WINRM, &[]),
        E::AuthRefused if auth == WinrmAuth::Ntlm => line(msg::WINRM_NTLM_REFUSED, &[host]),
        E::AuthRefused => line(msg::WINRM_BASIC_REFUSED, &[host]),
        E::Fingerprint(got) => line(msg::WINRM_CERT_MISMATCH, &[host, got]),
        E::Connect(e) => line(msg::WINRM_CONNECT, &[host, &plain(e)]),
        E::Tls(e) => line(msg::WINRM_TLS, &[host, &plain(e)]),
        E::Lost(e) => line(msg::LINK_LOST, &[host, &plain(e)]),
        E::Timeout => line(msg::LINK_TIMED_OUT, &[host]),
        E::Fault(e) => line(msg::WINRM_FAULT, &[host, &plain(e)]),
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
            assert!(msg::HOST_UNREADABLE.is_in(&place(bad).unwrap_err()), "{bad}");
        }
    }

    #[test]
    fn each_failure_says_what_to_look_at() {
        use ito_winrm::Error as E;
        let basic = |e: &E| reason(e, "app01", WinrmAuth::Basic);
        let b = basic(&E::AuthRefused);
        assert!(msg::WINRM_BASIC_REFUSED.is_in(&b) && b.contains("app01") && b.contains("auth = \"ntlm\""), "{b}");
        let ntlm = reason(&E::AuthRefused, "app01", WinrmAuth::Ntlm);
        assert!(msg::WINRM_NTLM_REFUSED.is_in(&ntlm) && ntlm.contains("CORP\\名前") && !ntlm.contains("Basic"), "{ntlm}");
        let f = basic(&E::Fingerprint("AB:CD".into()));
        assert!(msg::WINRM_CERT_MISMATCH.is_in(&f) && f.contains("AB:CD"), "{f}");
        let c = basic(&E::Connect("refused".into()));
        assert!(msg::WINRM_CONNECT.is_in(&c) && c.contains("5986") && c.contains("refused"), "{c}");
        assert_eq!(basic(&E::Stopped), note(msg::N_STOPPED_WINRM, &[]));
        assert_eq!(basic(&E::Other("x".into())), "x");
        assert!(msg::LINK_TIMED_OUT.is_in(&basic(&E::Timeout)));
        assert!(msg::LINK_LOST.is_in(&basic(&E::Lost("x".into()))));
        assert!(msg::WINRM_TLS.is_in(&basic(&E::Tls("x".into()))));
        assert!(msg::WINRM_FAULT.is_in(&basic(&E::Fault("x".into()))));
        let dump = "HandlerError(IO(Os { code: 10061, kind: ConnectionRefused, message: \"x\" }))";
        for e in [E::Connect(dump.into()), E::Tls(dump.into()), E::Lost(dump.into()), E::Other(dump.into())] {
            let r = basic(&e);
            assert!(!r.contains("HandlerError") && !r.contains("Os {") && !r.contains('\n'), "{r}");
        }
        assert!(basic(&E::Connect(dump.into())).contains(&note(msg::N_NET_REFUSED, &[])));
    }
}
