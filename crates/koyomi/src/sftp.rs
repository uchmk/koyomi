//! `koyomi sftp`: an SFTP `send` or `fetch` job's work (`run`, which reads what to do
//! from the `KOYOMI_SFTP_*` the engine sets), and the host key to pin (`hostkey`), for
//! an `ssh` job too.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use koyomi_core::chore::{self, Sftp, Verify};
use koyomi_core::msg::{self, line, note};
use koyomi_core::run::Moved;

/// The exit code of a run that found nothing to move: the chore judges it a warning.
const NOTHING: i32 = chore::NOTHING;

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op, at] if op == "hostkey" => hostkey(at),
        [op] if op == "run" => run(),
        _ => {
            crate::say(
                "koyomi: sftp takes hostkey <host>[:<port>] (for send, fetch and ssh), or run (what a send or fetch job does)",
                true,
            );
            2
        }
    }
}

/// `koyomi sftp hostkey <host>[:<port>]` (or an `sftp://` or `ssh://` address): the line
/// to put in the job.
fn hostkey(at: &str) -> i32 {
    let addr = if chore::sftp(at).or_else(|| chore::ssh(at)).is_some() { at.to_owned() } else { format!("sftp://{at}") };
    let far = match chore::sftp(&addr)
        .or_else(|| chore::ssh(&addr))
        .unwrap_or_else(|| Err(line(msg::HOST_UNREADABLE, &[&chore::hide_password(at), &note(msg::N_HOST_SHAPE_SFTP, &[])])))
    {
        Ok(f) => f,
        Err(e) => {
            crate::say(&e, true);
            return 2;
        }
    };
    match fingerprint(&far.host, far.port.unwrap_or(22)) {
        Ok(fp) => {
            crate::say(&format!("host_key = \"{fp}\""), false);
            0
        }
        Err(e) => {
            // A coded reason already names the host.
            crate::say(&if msg::is_coded(&e) { format!("koyomi: {e}") } else { format!("koyomi: {}: {e}", far.host) }, true);
            1
        }
    }
}

/// The bare `SHA256:…` host key `host:port` shows, or why not (a catalogue sentence naming the host).
fn fingerprint(host: &str, port: u16) -> Result<String, String> {
    koyomi_sftp::fingerprint(host, port, Duration::from_secs(15)).map_err(|e| why(host, &e))
}

/// A `koyomi_sftp::Error` as the sentence a person reads: the catalogue's words for `host`, with the
/// library's English detail inside where there is one. Shared by `sftp`, `ssh` and the host-key peek.
pub(crate) fn why(host: &str, e: &koyomi_sftp::Error) -> String {
    use koyomi_sftp::Error as E;
    match e {
        E::Stopped => note(msg::N_STOPPED_SSH, &[]),
        E::Connect(m) => line(msg::SSH_CONNECT, &[host, &crate::ssh::connect_text(m)]),
        E::AuthRefused => line(msg::SSH_AUTH_REFUSED, &[host]),
        E::HostKeyMismatch { expected, actual } => line(msg::SSH_HOST_KEY_MISMATCH, &[host, expected, actual]),
        E::Lost(m) => line(msg::LINK_LOST, &[host, m]),
        E::Timeout => line(msg::LINK_TIMED_OUT, &[host]),
        E::Key(m) => line(msg::SFTP_KEY, &[host, m]),
        E::NotFound(p) => line(msg::SFTP_NOT_FOUND, &[host, p]),
        E::PermissionDenied(p) => line(msg::SFTP_PERMISSION_DENIED, &[host, p]),
        E::Exists(p) => line(msg::SFTP_EXISTS, &[host, p]),
        E::Local(m) => line(msg::SFTP_LOCAL, &[host, m]),
        E::Mismatch { name, ours, theirs } => line(msg::SFTP_MISMATCH, &[host, name, ours, theirs]),
        E::VerifyUnavailable(m) => line(msg::SFTP_VERIFY_UNAVAILABLE, &[host, m]),
        E::Other(m) => line(msg::SFTP_OTHER, &[host, m]),
    }
}

/// The SSH host key `host:port` shows, for the engine to hand to the settings: the bare `SHA256:…`; no
/// login is made. Err is a coded reason (which names the host), or `<host>:<port>: <error>`.
pub(crate) fn peek_host_key(host: &str, port: u16) -> Result<String, String> {
    fingerprint(host, port).map_err(|e| if msg::is_coded(&e) { e } else { format!("{host}:{port}: {e}") })
}

fn var(name: &str) -> Option<String> {
    env_var("KOYOMI_SFTP_", name)
}

/// The environment variable `<prefix><name>`, trimmed; `None` when it is empty.
pub(crate) fn env_var(prefix: &str, name: &str) -> Option<String> {
    std::env::var(format!("{prefix}{name}")).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// What a `send` or `fetch` job runs. Prints each file moved; exits 0 when done,
/// [`NOTHING`] when there was no file, 2 on an error (with the reason after `! `;
/// a check that failed leaves it in `KOYOMI_REASON` too, for the job's ✕).
fn run() -> i32 {
    match transfer() {
        Ok(0) => NOTHING,
        Ok(_) => 0,
        Err(e) => {
            // To stdout, so the job's log keeps it in order with the rest.
            crate::say_bad(&e);
            2
        }
    }
}

/// Moves the files and says how many.
fn transfer() -> Result<usize, String> {
    let op = var("OP").ok_or_else(|| line(msg::JOB_ENV_MISSING, &["KOYOMI_SFTP_OP", "kind = \"send\" or kind = \"fetch\"", "sftp"]))?;
    let (from, into) = (var("FROM").unwrap_or_default(), var("INTO").unwrap_or_default());
    let send = match op.as_str() {
        "send" => true,
        "fetch" => false,
        _ => return Err(line(msg::DEF_CHOICE_BAD, &["KOYOMI_SFTP_OP", "\"send\", \"fetch\""])),
    };
    let far = chore::sftp(if send { &into } else { &from })
        .ok_or_else(|| line(msg::SFTP_PEER_MISSING, &[&op, if send { "into" } else { "from" }]))??;
    let host_key = var("HOST_KEY").ok_or_else(|| line(msg::CHORE_HOST_KEY_MISSING, &["sftp://"]))?;
    let session = koyomi_sftp::Session::connect(&target(&far, host_key, "sftp")?).map_err(|e| why(&far.host, &e))?;
    let never = AtomicBool::new(false);
    let opts = koyomi_sftp::Options { overwrite: true, verify: verify(var("VERIFY").as_deref()) };
    if send {
        // A folder's files (not those below), the files a wildcard names, or one.
        let path = Path::new(&from);
        let pattern = if path.is_dir() { path.join("*").display().to_string() } else { from.clone() };
        let files = crate::engine::files::find_files(&pattern);
        if files.is_empty() {
            crate::say(&format!("no file to send: {from}"), false);
            return Ok(0);
        }
        let remote = format!("{}/", far.path.trim_end_matches('/'));
        for (local, size, _) in &files {
            let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let moved = session.put(local, &remote, opts, &never, &mut |_, _, _| {}).map_err(|e| failed(&far.host, &e, &name, *size))?;
            for m in moved {
                said("sent", m);
            }
        }
        Ok(files.len())
    } else {
        let (dir, pattern) = far.path.rsplit_once('/').map_or((".", far.path.as_str()), |(d, p)| (if d.is_empty() { "/" } else { d }, p));
        let found: Vec<(String, u64)> = session
            .list(dir)
            .map_err(|e| why(&far.host, &e))?
            .into_iter()
            .filter(|e| !e.is_dir && koyomi_sftp::glob_match(pattern, &e.name, false))
            .map(|e| (e.name, e.size))
            .collect();
        if found.is_empty() {
            crate::say(&format!("no file to fetch: {from}"), false);
            return Ok(0);
        }
        let into = PathBuf::from(&into);
        std::fs::create_dir_all(&into)
            .map_err(|e| msg::line(msg::DIR_CREATE_FAILED, &[&into.display().to_string(), &msg::io_words(&e)]))?;
        for (name, size) in &found {
            let remote = if dir == "/" { format!("/{name}") } else { format!("{dir}/{name}") };
            let moved =
                session.get(&remote, &into.join(name), opts, &never, &mut |_, _, _| {}).map_err(|e| failed(&far.host, &e, name, *size))?;
            for m in moved {
                said("fetched", m);
            }
        }
        Ok(found.len())
    }
}

/// How koyomi-sftp checks a file for the job's `verify`: `sha256` asks the far side for
/// the file's hash (`sha256sum`, or `Get-FileHash` on a Windows server), `readback`
/// reads it back.
fn verify(v: Option<&str>) -> koyomi_sftp::Verify {
    match v.and_then(Verify::of) {
        Some(Verify::Sha256) => koyomi_sftp::Verify::Exec,
        Some(Verify::Readback) => koyomi_sftp::Verify::Readback,
        None => koyomi_sftp::Verify::None,
    }
}

/// Logs a file moved (`sent` or `fetched`) and records it for the history.
fn said(verb: &str, m: koyomi_sftp::Moved) {
    let hash = m.sha256.as_deref().map(|h| format!(" (sha256 {h})")).unwrap_or_default();
    crate::say(&format!("{verb}: {} ({} bytes){hash}", m.name, m.bytes), false);
    keep(&record(m));
}

/// What the history keeps of a file moved: its hash came only from a check that agreed.
fn record(m: koyomi_sftp::Moved) -> Moved {
    let matched = m.sha256.is_some().then_some(true);
    Moved { name: m.name, bytes: m.bytes, sha256: m.sha256, matched }
}

/// Why the file `name` (`bytes` long) did not move on `host`, as the log says it. The log names
/// the file first, like `sent:`, since the coded reason names only the host for most errors.
/// A check that failed is recorded as not matched and left as the job's reason too.
fn failed(host: &str, e: &koyomi_sftp::Error, name: &str, bytes: u64) -> String {
    crate::say(&format!("failed: {name} ({bytes} bytes)"), false);
    let text = why(host, e);
    let (kept, why) = failure(e, name, bytes);
    if let Some(m) = kept {
        keep(&m);
    }
    if why {
        reason(&text);
    }
    text
}

/// For an error moving `name`: what to record, and whether it is the job's reason
/// (a check that failed, not the move itself).
fn failure(e: &koyomi_sftp::Error, name: &str, bytes: u64) -> (Option<Moved>, bool) {
    match e {
        koyomi_sftp::Error::Mismatch { ours, .. } => {
            (Some(Moved { name: name.to_owned(), bytes, sha256: Some(ours.clone()), matched: Some(false) }), true)
        }
        koyomi_sftp::Error::VerifyUnavailable(_) => (None, true),
        _ => (None, false),
    }
}

/// Adds `m` to the list `KOYOMI_MOVED` names, which the engine keeps with the job's
/// end; nothing when the job is run by hand.
pub(crate) fn keep(m: &Moved) {
    if let Some(path) = std::env::var_os("KOYOMI_MOVED") {
        let _ = m.append(Path::new(&path));
    }
}

/// Leaves `why` in the file `KOYOMI_REASON` names: the engine shows it as the job's ✕.
fn reason(why: &str) {
    if let Some(path) = std::env::var_os("KOYOMI_REASON") {
        let _ = std::fs::write(path, why);
    }
}

/// Who logs in and how, for `scheme` (`sftp`, or `ssh`, whose `KOYOMI_SSH_*` are read):
/// the address's user or this account's name; `key`, the password (read from the OS
/// credential store when it is still `secret:<name>`), or this account's own key in
/// `~/.ssh`.
pub(crate) fn target(far: &Sftp, host_key: String, scheme: &str) -> Result<koyomi_sftp::Target, String> {
    let prefix = format!("KOYOMI_{}_", scheme.to_ascii_uppercase());
    let user = far
        .user
        .clone()
        .or_else(|| std::env::var("USERNAME").ok())
        .or_else(|| std::env::var("USER").ok())
        .ok_or_else(|| line(msg::SFTP_USER_UNKNOWN, &[scheme]))?;
    let auth = if let Some(k) = env_var(&prefix, "KEY") {
        koyomi_sftp::Auth::Key { path: PathBuf::from(k), passphrase: None }
    } else if let Some(p) = env_var(&prefix, "PASSWORD") {
        koyomi_sftp::Auth::Password(password(p)?)
    } else {
        let ssh = ito_common::home_dir().map(|h| h.join(".ssh")).ok_or_else(|| line(msg::HOME_UNKNOWN, &[]))?;
        let key = ["id_ed25519", "id_ecdsa"]
            .iter()
            .map(|n| ssh.join(n))
            .find(|p| p.is_file())
            .ok_or_else(|| line(msg::DEFAULT_KEY_MISSING, &[&ssh.join("id_ed25519").display().to_string()]))?;
        koyomi_sftp::Auth::Key { path: key, passphrase: None }
    };
    let mut t = koyomi_sftp::Target::new(far.host.clone(), user, auth, host_key);
    if let Some(p) = far.port {
        t.port = p;
    }
    Ok(t)
}

/// A job's `password`: read from the OS credential store when it is `secret:<name>`.
pub(crate) fn password(p: String) -> Result<String, String> {
    match p.strip_prefix("secret:") {
        Some(name) => ito_secret::get("koyomi", name.trim())
            .map_err(|e| line(msg::SECRET_UNREADABLE, &[name, &e.to_string()]))?
            .ok_or_else(|| line(msg::SECRET_MISSING, &[name])),
        None => Ok(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A port that nothing listens on (bound and dropped on the loopback), so no network is touched.
    fn refused_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn a_refused_loopback_port_fails_with_an_error_naming_host_and_port() {
        let port = refused_port();
        let e = peek_host_key("127.0.0.1", port).unwrap_err();
        assert!(msg::SSH_CONNECT.is_in(&e) && e.contains("127.0.0.1"), "{e}");
    }

    #[test]
    fn verify_maps_to_how_ito_sftp_checks() {
        assert_eq!(verify(None), koyomi_sftp::Verify::None);
        assert_eq!(verify(Some("sha256")), koyomi_sftp::Verify::Exec);
        assert_eq!(verify(Some("readback")), koyomi_sftp::Verify::Readback);
        assert_eq!(verify(Some("md5")), koyomi_sftp::Verify::None);
    }

    #[test]
    fn a_failed_check_is_recorded_and_is_the_reason() {
        let e = koyomi_sftp::Error::Mismatch { name: "a.csv".into(), ours: "11".into(), theirs: "22".into() };
        assert_eq!(
            failure(&e, "a.csv", 5),
            (Some(Moved { name: "a.csv".into(), bytes: 5, sha256: Some("11".into()), matched: Some(false) }), true)
        );
        assert_eq!(failure(&koyomi_sftp::Error::VerifyUnavailable("no exec".into()), "x", 0), (None, true));
        // Any other error is the log's only.
        assert_eq!(failure(&koyomi_sftp::Error::NotFound("/in/d.csv".into()), "d.csv", 0), (None, false));
        assert_eq!(
            record(koyomi_sftp::Moved { name: "b.csv".into(), bytes: 3, sha256: Some("33".into()) }),
            Moved { name: "b.csv".into(), bytes: 3, sha256: Some("33".into()), matched: Some(true) }
        );
        assert_eq!(
            record(koyomi_sftp::Moved { name: "c.csv".into(), bytes: 4, sha256: None }),
            Moved { name: "c.csv".into(), bytes: 4, sha256: None, matched: None }
        );
    }
}
