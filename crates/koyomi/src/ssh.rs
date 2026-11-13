//! `koyomi ssh run`: what an `ssh` job runs. It reads what to do from the
//! `KOYOMI_SSH_*` the engine sets, runs the command on the far PC, passes its
//! output on line by line as it comes (the far standard error after `! `), and
//! ends with the far command's exit code. Without one (no connection, no login,
//! a signal, a lost connection) it ends with [`NO_EXIT`] and leaves why in the
//! file `KOYOMI_REASON` names, which the engine shows as the job's ✕ reason.
//! The host key to pin comes from `koyomi sftp hostkey`.
//!
//! `on = "group:web"` runs it on each PC of the group `web` in koyomi.toml's
//! `[groups]`, `KOYOMI_SSH_MAX` at a time (one when not written), each PC's lines
//! marked `[web1] `; it ends as the worst PC does, or with `pass = "any"` as the
//! best does ([`group`]).
//!
//! The engine runs this straight, not through PowerShell, and stops an `ssh`
//! job by asking first (`ito_proc::run_lines_soft`): the ask, a CTRL_BREAK or
//! the event named in `ITO_PROC_STOP_EVENT` (which needs no console), sets
//! [`ito_proc::stop_requested`]. koyomi-sftp looks at it from the start: before
//! the far command is sent, the connection is given up and nothing starts
//! there; after, the far command is sent TERM and the channel closed before
//! this ends, so it does not run on alone.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use koyomi_core::chore::{self, Pass, Sftp, GROUP_MAX, NO_EXIT};
use koyomi_core::config::Member;
use koyomi_core::msg::{self, line, note};

/// The longest piece of output kept waiting for its line's end: past it, it is
/// passed on as a line of its own.
const LINE_MAX: usize = 64 * 1024;

/// How many failed PCs of a group the job's reason names; the rest are counted.
const SHOWN: usize = 3;

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "run" => run(exec),
        _ => {
            crate::say("koyomi: ssh takes run (what an ssh job does); the host key: koyomi sftp hostkey <host>[:<port>]", true);
            2
        }
    }
}

fn var(name: &str) -> Option<String> {
    crate::sftp::env_var("KOYOMI_SSH_", name)
}

/// Runs `exec` (an `ssh`'s or a `winrm`'s) and ends as [the module](self) says:
/// its exit code, or [`NO_EXIT`] with why.
pub(crate) fn run(exec: impl FnOnce(&AtomicBool) -> Result<i32, String>) -> i32 {
    // First of all, before connecting: the handler and the watcher of the stop
    // event are what turn the engine's ask into the flag.
    let stop = ito_proc::stop_requested();
    match exec(stop) {
        Ok(code) => code,
        Err(reason) => {
            // To stdout, so the job's log keeps it in order with the rest.
            crate::say_bad(&reason);
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
    let on = var("ON").ok_or_else(|| line(msg::JOB_ENV_MISSING, &["KOYOMI_SSH_ON", "kind = \"ssh\"", "ssh"]))?;
    // As it is written: not trimmed.
    let command =
        std::env::var("KOYOMI_SSH_COMMAND").ok().filter(|c| !c.trim().is_empty()).ok_or_else(|| line(msg::CHORE_COMMAND_EMPTY, &[]))?;
    let host_key = var("HOST_KEY");
    let (group_name, max, pass) = fan_of("KOYOMI_SSH_", &on)?;
    if let Some(name) = group_name {
        let members = members(name, "ssh")?;
        return group(name, &members, max, pass, stop, &say, |m, out| {
            let far = chore::ssh(&m.on).ok_or_else(|| line(msg::GROUP_MEMBER_SCHEME, &[&chore::hide_password(&m.on), "ssh://"]))??;
            let host_key = m
                .host_key
                .clone()
                .or_else(|| host_key.clone())
                .ok_or_else(|| line(msg::GROUP_MEMBER_PIN_MISSING, &[&m.on, "host_key", "koyomi sftp hostkey <host>"]))?;
            one(&far, host_key, &command, stop, out)
        });
    }
    let far = chore::ssh(&on).ok_or_else(|| line(msg::CHORE_ON_BAD, &["ssh://[user@]host[:port]", &chore::hide_password(&on)]))??;
    let host_key = host_key.ok_or_else(|| line(msg::CHORE_HOST_KEY_MISSING, &["ssh://"]))?;
    // A coded reason already names the host and keeps its ID at the head.
    one(&far, host_key, &command, stop, &say).map_err(|e| if msg::is_coded(&e) { e } else { format!("{}: {e}", far.host) })
}

/// Runs `command` on `far`, its lines to `out`: its exit code, or why there is
/// none (without the host, which the caller adds).
fn one(far: &Sftp, host_key: String, command: &str, stop: &AtomicBool, out: &dyn Fn(&str, bool)) -> Result<i32, String> {
    let target = crate::sftp::target(far, host_key, "ssh")?;
    let opts = koyomi_sftp::ExecOptions { env: forwarded("KOYOMI_SSH_ENV"), timeout: None };
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let result = koyomi_sftp::exec(&target, command, &opts, stop, &mut |stream, bytes| {
        let (buf, stderr) = match stream {
            koyomi_sftp::Stream::Stdout => (&mut o, false),
            koyomi_sftp::Stream::Stderr => (&mut e, true),
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
    let exit = result.map_err(|e| crate::sftp::why(&far.host, &e))?;
    if !exit.env_refused.is_empty() {
        let names = exit.env_refused.join(", ");
        out(&line(msg::SSH_ENV_REFUSED, &[&far.host, &names]), true);
    }
    outcome(exit.status, exit.signal)
}

/// A connect failure in a few plain words, for the hole of E2150. koyomi-sftp passes the library's
/// Debug text for a refused connection (`Io(Os { code: 10061, kind: ConnectionRefused, .. })`); that is not shown.
pub(crate) fn connect_text(m: &str) -> String {
    let has = |k: &str| m.contains(k);
    if has("ConnectionRefused") || has("10061") || has("os error 111") {
        note(msg::N_NET_REFUSED, &[])
    } else if has("TimedOut") || has("10060") || has("timed out") {
        note(msg::N_NET_NO_ANSWER, &[])
    } else if has("Unreachable") || has("10051") || has("10065") {
        note(msg::N_NET_NO_ROUTE, &[])
    } else if has("NotFound") || has("failed to lookup") || has("11001") {
        note(msg::N_NET_HOST_NOT_FOUND, &[])
    } else if has("Io(") || has("Os {") || has("kind:") {
        note(msg::N_NET_FAILED, &[])
    } else {
        m.to_owned()
    }
}

/// The group named in `on` (`group:web`) with `max` and `pass` of the job
/// (`prefix`: `KOYOMI_SSH_` or `KOYOMI_WINRM_`), which only a group may have.
pub(crate) fn fan_of<'a>(prefix: &str, on: &'a str) -> Result<(Option<&'a str>, usize, Pass), String> {
    let max = crate::sftp::env_var(prefix, "MAX");
    let pass = crate::sftp::env_var(prefix, "PASS");
    let Some(name) = chore::group(on) else {
        return match max.is_some() || pass.is_some() {
            true => Err(line(msg::CHORE_GROUP_ONLY, &[if max.is_some() { "max" } else { "pass" }])),
            false => Ok((None, 1, Pass::All)),
        };
    };
    let max = match max {
        None => 1,
        Some(m) => m
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=GROUP_MAX).contains(n))
            .ok_or_else(|| line(msg::CHORE_MAX_RANGE, &[&GROUP_MAX.to_string()]))?,
    };
    let pass = match pass {
        None => Pass::All,
        Some(p) => Pass::of(&p).ok_or_else(|| line(msg::DEF_CHOICE_BAD, &["pass", "\"all\", \"any\""]))?,
    };
    Ok((Some(name), max, pass))
}

/// The PCs of the group `name` in koyomi.toml's `[groups]`, for an `ssh` or a
/// `winrm` (`kind`).
pub(crate) fn members(name: &str, kind: &str) -> Result<Vec<Member>, String> {
    let paths = crate::engine::Paths::find()?;
    let config = koyomi_core::config::Config::load(&paths.config)?;
    config.members(name, kind)
}

/// Runs `job` on each PC of the group `name`, `max` at a time in the group's
/// order. Each PC's lines go to `log` (the job's log: [`say`]) marked with
/// [`labels`], and a line when it ends; one not started yet when `stop` is set
/// does not start. Ends as [`verdict`] says.
pub(crate) fn group(
    name: &str,
    members: &[Member],
    max: usize,
    pass: Pass,
    stop: &AtomicBool,
    log: &(dyn Fn(&str, bool) + Sync),
    job: impl Fn(&Member, &dyn Fn(&str, bool)) -> Result<i32, String> + Sync,
) -> Result<i32, String> {
    let labels = labels(members);
    let at_once = max.clamp(1, members.len().max(1));
    let ends = match pass {
        Pass::All => note(msg::N_GROUP_WORST, &[]),
        Pass::Any => note(msg::N_GROUP_BEST, &[]),
    };
    log(&note(msg::N_GROUP_START, &[name, &members.len().to_string(), &at_once.to_string(), &ends]), false);
    let next = AtomicUsize::new(0);
    let mut results: Vec<Option<Result<i32, String>>> = vec![None; members.len()];
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..at_once)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        let (Some(m), Some(tag)) = (members.get(i), labels.get(i)) else { break };
                        let r = match stop.load(Ordering::SeqCst) {
                            true => Err(note(msg::N_GROUP_NOT_STARTED, &[])),
                            false => job(m, &|line: &str, stderr: bool| log(&format!("[{tag}] {line}"), stderr)),
                        };
                        match &r {
                            Ok(code) => log(&format!("[{tag}] {}", note(msg::N_RUN_EXIT, &[&code.to_string()])), false),
                            Err(e) => log(&format!("[{tag}] {e}"), true),
                        }
                        done.push((i, r));
                    }
                    done
                })
            })
            .collect();
        for w in workers {
            // A worker that panicked leaves its PCs without a result.
            for (i, r) in w.join().unwrap_or_default() {
                results[i] = Some(r);
            }
        }
    });
    let results = results.into_iter().map(|r| r.unwrap_or_else(|| Err(line(msg::GROUP_WORKER_LOST, &[])))).collect();
    verdict(&labels, results, pass)
}

/// What each PC's lines are marked with: its host, and port when written; the
/// whole address when another PC of the group has the same.
fn labels(members: &[Member]) -> Vec<String> {
    let address = |on: &str| on.split_once("://").map_or(on, |(_, rest)| rest).to_owned();
    let short: Vec<String> = members
        .iter()
        .map(|m| {
            let a = address(&m.on);
            a.rsplit_once('@').map_or(a.clone(), |(_, host)| host.to_owned())
        })
        .collect();
    short
        .iter()
        .zip(members)
        .map(|(s, m)| match short.iter().filter(|x| x.eq_ignore_ascii_case(s)).count() {
            1 => s.clone(),
            _ => address(&m.on),
        })
        .collect()
}

/// What a group's job ends with, its PCs' `results` in the group's order: the
/// worst (`all`) or the best (`any`) of them, a larger code being worse than a
/// smaller and no exit code worst of all. No exit code: why, naming the PCs that
/// ended so ([`SHOWN`] of them, then how many more).
fn verdict(labels: &[String], results: Vec<Result<i32, String>>, pass: Pass) -> Result<i32, String> {
    let rank = |r: &&Result<i32, String>| match r {
        // A Windows code such as 0xC0000005 is a large one.
        Ok(code) => u64::from(*code as u32),
        Err(_) => u64::MAX,
    };
    let pick = match pass {
        Pass::All => results.iter().max_by_key(rank),
        Pass::Any => results.iter().min_by_key(rank),
    };
    if let Some(Ok(code)) = pick {
        return Ok(*code);
    }
    let failed: Vec<String> =
        labels.iter().zip(&results).filter_map(|(l, r)| r.as_ref().err().map(|e| format!("{l}: {}", msg::uncoded(e)))).collect();
    let mut who = failed.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(" / ");
    if failed.len() > SHOWN {
        who += &format!(" {}", note(msg::N_GROUP_MORE, &[&(failed.len() - SHOWN).to_string()]));
    }
    Err(line(msg::GROUP_NO_EXIT, &[&results.len().to_string(), &failed.len().to_string(), &who]))
}

/// The exit code to end with, or why the command has none.
fn outcome(status: Option<u32>, signal: Option<String>) -> Result<i32, String> {
    match (status, signal) {
        // A Windows code such as 0xC0000005 goes through as the same bits.
        (Some(code), _) => Ok(code as i32),
        (None, Some(sig)) => Err(line(msg::FAR_SIGNAL, &[&sig])),
        (None, None) => Err(line(msg::FAR_NO_EXIT_CODE, &[])),
    }
}

/// The names in `list` (`KOYOMI_SSH_ENV`: the job's own `env`) with their values
/// here, to ask the far side for.
pub(crate) fn forwarded(list: &str) -> Vec<(String, String)> {
    let names = std::env::var(list).unwrap_or_default();
    names.split(',').map(str::trim).filter(|n| !n.is_empty()).filter_map(|n| Some((n.to_owned(), std::env::var(n).ok()?))).collect()
}

/// A far line to the job's log: standard error after `! `, as the engine writes a
/// job's own.
pub(crate) fn say(line: &str, stderr: bool) {
    match stderr {
        true => crate::say(&format!("! {line}"), false),
        false => crate::say(line, false),
    }
}

/// `bytes` added to what waits in `buf`, and the lines that are whole now.
pub(crate) fn cut(buf: &mut Vec<u8>, bytes: &[u8]) -> Vec<String> {
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
pub(crate) fn text(line: &[u8]) -> String {
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

    fn pc(on: &str) -> Member {
        Member { on: on.into(), host_key: None, cert: None }
    }

    #[test]
    fn a_group_marks_each_pc_by_its_host_or_whole_address() {
        let m = [pc("ssh://ops@web1"), pc("ssh://web2:2222"), pc("ssh://ops@web3"), pc("SSH://root@WEB3"), pc("winrm://CORP\\ops@app01")];
        assert_eq!(labels(&m), ["web1", "web2:2222", "ops@web3", "root@WEB3", "app01"]);
    }

    #[test]
    fn a_group_ends_as_its_worst_or_best_pc() {
        let l: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        let ok = |c: i32| Ok::<i32, String>(c);
        let no = |e: &str| Err::<i32, String>(e.into());
        assert_eq!(verdict(&l, vec![ok(0), ok(2), ok(1)], Pass::All), Ok(2));
        assert_eq!(verdict(&l, vec![ok(0), ok(2), ok(1)], Pass::Any), Ok(0));
        // A Windows code is worse than a small one.
        assert_eq!(verdict(&l, vec![ok(0xC000_0005_u32 as i32), ok(1)], Pass::All), Ok(0xC000_0005_u32 as i32));
        assert_eq!(verdict(&l, vec![ok(3), no("x")], Pass::Any), Ok(3));
        let all = verdict(&l, vec![ok(0), no("つながらない"), ok(0)], Pass::All).unwrap_err();
        assert_eq!(all, line(msg::GROUP_NO_EXIT, &["3", "1", "b: つながらない"]));
        let any = verdict(&l, ["1", "2", "3", "4", "5"].map(no).to_vec(), Pass::Any).unwrap_err();
        let more = note(msg::N_GROUP_MORE, &["2"]);
        assert_eq!(any, line(msg::GROUP_NO_EXIT, &["5", "5", &format!("a: 1 / b: 2 / c: 3 {more}")]));
    }

    #[test]
    fn a_group_runs_each_pc_once_at_most_max_at_a_time_and_a_stop_starts_no_more() {
        let m: Vec<Member> = (0..7).map(|i| pc(&format!("ssh://h{i}"))).collect();
        let (now, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let seen = std::sync::Mutex::new(Vec::new());
        let lines = std::sync::Mutex::new(Vec::new());
        let log = |l: &str, stderr: bool| lines.lock().unwrap().push(format!("{}{l}", if stderr { "! " } else { "" }));
        let r = group("web", &m, 3, Pass::All, &AtomicBool::new(false), &log, |pc, out| {
            let n = now.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(n, Ordering::SeqCst);
            out("x", false);
            std::thread::sleep(std::time::Duration::from_millis(20));
            seen.lock().unwrap().push(pc.on.clone());
            now.fetch_sub(1, Ordering::SeqCst);
            Ok(pc.on.ends_with('4') as i32)
        });
        assert_eq!(r, Ok(1));
        assert!(most.load(Ordering::SeqCst) <= 3);
        let mut seen = seen.into_inner().unwrap();
        seen.sort();
        assert_eq!(seen, m.iter().map(|p| p.on.clone()).collect::<Vec<_>>());
        {
            let lines = lines.lock().unwrap();
            assert_eq!(lines[0], note(msg::N_GROUP_START, &["web", "7", "3", &note(msg::N_GROUP_WORST, &[])]));
            let exit1 = format!("[h4] {}", note(msg::N_RUN_EXIT, &["1"]));
            assert!(lines.contains(&"[h2] x".to_owned()) && lines.contains(&exit1), "{lines:?}");
        }
        // Stopped: none starts.
        lines.lock().unwrap().clear();
        let r = group("web", &m, 2, Pass::Any, &AtomicBool::new(true), &log, |_, _| Ok(0));
        assert!(msg::GROUP_NO_EXIT.is_in(&r.unwrap_err()), "stopped");
        assert!(lines.lock().unwrap().contains(&format!("! [h6] {}", note(msg::N_GROUP_NOT_STARTED, &[]))));
    }

    #[test]
    fn max_and_pass_are_only_for_a_group() {
        let p = "KOYOMI_TEST_FAN_";
        assert_eq!(fan_of(p, "ssh://h"), Ok((None, 1, Pass::All)));
        assert_eq!(fan_of(p, "group:web"), Ok((Some("web"), 1, Pass::All)));
        std::env::set_var(format!("{p}MAX"), "4");
        std::env::set_var(format!("{p}PASS"), "any");
        assert_eq!(fan_of(p, " Group:web "), Ok((Some("web"), 4, Pass::Any)));
        assert_eq!(fan_of(p, "ssh://h").unwrap_err(), line(msg::CHORE_GROUP_ONLY, &["max"]));
        std::env::set_var(format!("{p}MAX"), "65");
        assert_eq!(fan_of(p, "group:web").unwrap_err(), line(msg::CHORE_MAX_RANGE, &[&GROUP_MAX.to_string()]));
        std::env::set_var(format!("{p}MAX"), "2");
        std::env::set_var(format!("{p}PASS"), "most");
        assert_eq!(fan_of(p, "group:web").unwrap_err(), line(msg::DEF_CHOICE_BAD, &["pass", "\"all\", \"any\""]));
        std::env::remove_var(format!("{p}MAX"));
        std::env::remove_var(format!("{p}PASS"));
    }

    #[test]
    fn a_refused_connection_is_said_in_plain_words() {
        let debug = "Io(Os { code: 10061, kind: ConnectionRefused, message: \"x\" })";
        let t = connect_text(debug);
        assert_eq!(t, note(msg::N_NET_REFUSED, &[]));
        assert_eq!(connect_text("something odd"), "something odd");
        assert_eq!(connect_text("Io(Weird { kind: Other })"), note(msg::N_NET_FAILED, &[]));
    }

    #[test]
    fn no_exit_code_gives_a_reason() {
        assert_eq!(outcome(Some(0), None), Ok(0));
        assert_eq!(outcome(Some(3), Some("TERM".into())), Ok(3));
        assert_eq!(outcome(Some(0xC000_0005), None), Ok(0xC000_0005_u32 as i32));
        assert_eq!(outcome(None, Some("KILL".into())).unwrap_err(), line(msg::FAR_SIGNAL, &["KILL"]));
        assert_eq!(outcome(None, None).unwrap_err(), line(msg::FAR_NO_EXIT_CODE, &[]));
    }
}
