//! `koyomi ssh run`: what an `ssh` job runs. It reads what to do from the
//! `KOYOMI_SSH_*` the engine sets, runs the command on the far PC, passes its
//! output on line by line as it comes (the far standard error after `! `), and
//! ends with the far command's exit code. Without one (no connection, no login,
//! a signal, a lost connection) it ends with [`NO_EXIT`] and leaves why in the
//! file `KOYOMI_REASON` names, which the engine shows as the job's ✕ reason.
//! The host key to pin comes from `koyomi sftp hostkey`.
//!
//! The engine runs this straight, not through PowerShell, and stops an `ssh`
//! job by asking first (`ito_proc::run_lines_soft`): the ask, a CTRL_BREAK or
//! the event named in `ITO_PROC_STOP_EVENT` (which needs no console), sets
//! [`ito_proc::stop_requested`]. ito-sftp looks at it from the start: before
//! the far command is sent, the connection is given up and nothing starts
//! there; after, the far command is sent TERM and the channel closed before
//! this ends, so it does not run on alone.

use std::sync::atomic::AtomicBool;

use koyomi_core::chore::{self, NO_EXIT};

/// The longest piece of output kept waiting for its line's end: past it, it is
/// passed on as a line of its own.
const LINE_MAX: usize = 64 * 1024;

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "run" => run(),
        _ => {
            crate::say("koyomi: ssh takes run (what an ssh job does); the host key: koyomi sftp hostkey <host>[:<port>]", true);
            2
        }
    }
}

fn var(name: &str) -> Option<String> {
    crate::sftp::env_var("KOYOMI_SSH_", name)
}

fn run() -> i32 {
    // First of all, before connecting: the handler and the watcher of the stop
    // event are what turn the engine's ask into the flag.
    let stop = ito_proc::stop_requested();
    match exec(stop) {
        Ok(code) => code,
        Err(reason) => {
            // To stdout, so the job's log keeps it in order with the rest.
            crate::say(&format!("! {reason}"), false);
            if let Some(path) = std::env::var_os("KOYOMI_REASON") {
                let _ = std::fs::write(path, &reason);
            }
            NO_EXIT
        }
    }
}

/// Runs the command; its exit code, or why there is none. `stop` set: this
/// ends without an exit code, at any step (connecting too), and a far command
/// that has started is asked to stop too.
fn exec(stop: &AtomicBool) -> Result<i32, String> {
    let on = var("ON").ok_or("KOYOMI_SSH_ON がない（koyomi ssh run は kind = \"ssh\" のジョブが動かす）")?;
    let far = chore::ssh(&on).ok_or_else(|| format!("on は ssh://[ユーザー@]ホスト[:ポート] で書く: {on}"))??;
    // As it is written: not trimmed.
    let command = std::env::var("KOYOMI_SSH_COMMAND").ok().filter(|c| !c.trim().is_empty()).ok_or("command がない")?;
    let host_key = var("HOST_KEY").ok_or("host_key がない（koyomi sftp hostkey <ホスト> で調べる）")?;
    let target = crate::sftp::target(&far, host_key, "ssh")?;
    let opts = ito_sftp::ExecOptions { env: forwarded(), timeout: None };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = ito_sftp::exec(&target, &command, &opts, stop, &mut |stream, bytes| {
        let (buf, stderr) = match stream {
            ito_sftp::Stream::Stdout => (&mut out, false),
            ito_sftp::Stream::Stderr => (&mut err, true),
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
    let exit = result.map_err(|e| match e {
        ito_sftp::Error::Stopped => stopped(&far.host),
        e => format!("{}: {e}", far.host),
    })?;
    if !exit.env_refused.is_empty() {
        let names = exit.env_refused.join(", ");
        crate::say(&format!("! {}: env を受け付けなかった: {names}（相手の sshd_config の AcceptEnv）", far.host), false);
    }
    outcome(exit.status, exit.signal).map_err(|e| format!("{}: {e}", far.host))
}

/// Why there is no exit code after a stop: the engine shows it after 強制終了 or
/// 打ち切り, so it says what was done on the far side. ito-sftp does not say
/// whether the command had been sent yet (a stop while connecting sends none),
/// so this holds either way.
fn stopped(host: &str) -> String {
    format!("{host}: 止めた（コマンドが始まっていれば TERM で止まるよう頼んだ）")
}

/// The exit code to end with, or why the command has none.
fn outcome(status: Option<u32>, signal: Option<String>) -> Result<i32, String> {
    match (status, signal) {
        // A Windows code such as 0xC0000005 goes through as the same bits.
        (Some(code), _) => Ok(code as i32),
        (None, Some(sig)) => Err(format!("相手のコマンドがシグナル {sig} で終わった")),
        (None, None) => Err("終了コードを受け取れなかった".into()),
    }
}

/// The names in `KOYOMI_SSH_ENV` (the job's own `env`) with their values here, to
/// ask the far side for.
fn forwarded() -> Vec<(String, String)> {
    let names = std::env::var("KOYOMI_SSH_ENV").unwrap_or_default();
    names.split(',').map(str::trim).filter(|n| !n.is_empty()).filter_map(|n| Some((n.to_owned(), std::env::var(n).ok()?))).collect()
}

/// A far line to the job's log: standard error after `! `, as the engine writes a
/// job's own.
fn say(line: &str, stderr: bool) {
    match stderr {
        true => crate::say(&format!("! {line}"), false),
        false => crate::say(line, false),
    }
}

/// `bytes` added to what waits in `buf`, and the lines that are whole now.
fn cut(buf: &mut Vec<u8>, bytes: &[u8]) -> Vec<String> {
    buf.extend_from_slice(bytes);
    let mut lines = Vec::new();
    while let Some(end) = buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buf.drain(..=end).collect();
        lines.push(text(&line));
    }
    if buf.len() > LINE_MAX {
        lines.push(text(&std::mem::take(buf)));
    }
    lines
}

/// One line as text: UTF-8, or else the Windows code page (a Japanese Windows's
/// `cmd` writes CP932); without its line end.
fn text(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    ito_proc::Encoding::Auto.decode(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_output_is_cut_into_lines_whatever_the_chunks() {
        let mut buf = Vec::new();
        assert_eq!(cut(&mut buf, b"one\r\ntw"), vec!["one".to_owned()]);
        assert_eq!(cut(&mut buf, b"o\n\nthr"), vec!["two".to_owned(), String::new()]);
        assert_eq!(buf, b"thr");
        // UTF-8 split between chunks, and CP932 on its own line.
        let kana = "かな".as_bytes();
        assert!(cut(&mut buf, &kana[..2]).is_empty());
        assert_eq!(cut(&mut buf, &[&kana[2..], b"\n"].concat()), vec!["thrかな".to_owned()]);
        // Not UTF-8 (あ in CP932): read as the code page, still a line.
        assert_eq!(cut(&mut buf, &[0x82, 0xa0, b'\n']).len(), 1);
        // A line without an end does not wait without bound.
        assert_eq!(cut(&mut buf, &vec![b'x'; LINE_MAX + 1]).len(), 1);
        assert!(buf.is_empty());
    }

    #[test]
    fn a_stop_says_what_was_done_on_the_far_side() {
        let reason = stopped("web01");
        assert_eq!(reason, "web01: 止めた（コマンドが始まっていれば TERM で止まるよう頼んだ）");
    }

    #[test]
    fn no_exit_code_gives_a_reason() {
        assert_eq!(outcome(Some(0), None), Ok(0));
        assert_eq!(outcome(Some(3), Some("TERM".into())), Ok(3));
        assert_eq!(outcome(Some(0xC000_0005), None), Ok(0xC000_0005_u32 as i32));
        assert!(outcome(None, Some("KILL".into())).unwrap_err().contains("シグナル KILL"));
        assert!(outcome(None, None).unwrap_err().contains("終了コード"));
    }
}
