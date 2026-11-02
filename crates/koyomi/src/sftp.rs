//! `koyomi sftp`: an SFTP `send` or `fetch` job's work (`run`, which reads what to do
//! from the `KOYOMI_SFTP_*` the engine sets), and the host key to pin (`hostkey`), for
//! an `ssh` job too.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use koyomi_core::chore::{self, Sftp, Verify};
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
    let far = match chore::sftp(&addr).or_else(|| chore::ssh(&addr)).unwrap_or_else(|| Err(format!("ホストを読めない: {at}"))) {
        Ok(f) => f,
        Err(e) => {
            crate::say(&format!("koyomi: {e}"), true);
            return 2;
        }
    };
    match ito_sftp::fingerprint(&far.host, far.port.unwrap_or(22), Duration::from_secs(15)) {
        Ok(fp) => {
            crate::say(&format!("host_key = \"{fp}\""), false);
            0
        }
        Err(e) => {
            crate::say(&format!("koyomi: {}: {e}", far.host), true);
            1
        }
    }
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
            crate::say(&format!("! {e}"), false);
            2
        }
    }
}

/// Moves the files and says how many.
fn transfer() -> Result<usize, String> {
    let op = var("OP").ok_or("KOYOMI_SFTP_OP がない（koyomi sftp run は send か fetch のジョブが動かす）")?;
    let (from, into) = (var("FROM").unwrap_or_default(), var("INTO").unwrap_or_default());
    let send = match op.as_str() {
        "send" => true,
        "fetch" => false,
        _ => return Err(format!("KOYOMI_SFTP_OP は send か fetch: {op}")),
    };
    let far = chore::sftp(if send { &into } else { &from }).ok_or("sftp:// の相手がない")??;
    let host_key = var("HOST_KEY").ok_or("host_key がない")?;
    let session = ito_sftp::Session::connect(&target(&far, host_key, "sftp")?).map_err(|e| format!("{}: {e}", far.host))?;
    let never = AtomicBool::new(false);
    let opts = ito_sftp::Options { overwrite: true, verify: verify(var("VERIFY").as_deref()) };
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
            let moved = session
                .put(local, &remote, opts, &never, &mut |_, _, _| {})
                .map_err(|e| failed(&format!("{remote}{name}"), &e, &name, *size))?;
            for m in moved {
                said("sent", m);
            }
        }
        Ok(files.len())
    } else {
        let (dir, pattern) = far.path.rsplit_once('/').map_or((".", far.path.as_str()), |(d, p)| (if d.is_empty() { "/" } else { d }, p));
        let found: Vec<(String, u64)> = session
            .list(dir)
            .map_err(|e| format!("{dir}: {e}"))?
            .into_iter()
            .filter(|e| !e.is_dir && ito_sftp::glob_match(pattern, &e.name, false))
            .map(|e| (e.name, e.size))
            .collect();
        if found.is_empty() {
            crate::say(&format!("no file to fetch: {from}"), false);
            return Ok(0);
        }
        let into = PathBuf::from(&into);
        std::fs::create_dir_all(&into).map_err(|e| format!("{}: {e}", into.display()))?;
        for (name, size) in &found {
            let remote = if dir == "/" { format!("/{name}") } else { format!("{dir}/{name}") };
            let moved =
                session.get(&remote, &into.join(name), opts, &never, &mut |_, _, _| {}).map_err(|e| failed(&remote, &e, name, *size))?;
            for m in moved {
                said("fetched", m);
            }
        }
        Ok(found.len())
    }
}

/// How ito-sftp checks a file for the job's `verify`: `sha256` asks the far side for
/// the file's hash (`sha256sum`, or `Get-FileHash` on a Windows server), `readback`
/// reads it back.
fn verify(v: Option<&str>) -> ito_sftp::Verify {
    match v.and_then(Verify::of) {
        Some(Verify::Sha256) => ito_sftp::Verify::Exec,
        Some(Verify::Readback) => ito_sftp::Verify::Readback,
        None => ito_sftp::Verify::None,
    }
}

/// Logs a file moved (`sent` or `fetched`) and records it for the history.
fn said(verb: &str, m: ito_sftp::Moved) {
    let hash = m.sha256.as_deref().map(|h| format!(" (sha256 {h})")).unwrap_or_default();
    crate::say(&format!("{verb}: {} ({} bytes){hash}", m.name, m.bytes), false);
    keep(&record(m));
}

/// What the history keeps of a file moved: its hash came only from a check that agreed.
fn record(m: ito_sftp::Moved) -> Moved {
    let matched = m.sha256.is_some().then_some(true);
    Moved { name: m.name, bytes: m.bytes, sha256: m.sha256, matched }
}

/// Why the file `name` (`bytes` long) did not move, as the log says it. A check that
/// failed is recorded as not matched and left as the job's reason too.
fn failed(at: &str, e: &ito_sftp::Error, name: &str, bytes: u64) -> String {
    let text = format!("{at}: {e}");
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
fn failure(e: &ito_sftp::Error, name: &str, bytes: u64) -> (Option<Moved>, bool) {
    match e {
        ito_sftp::Error::Mismatch { ours, .. } => {
            (Some(Moved { name: name.to_owned(), bytes, sha256: Some(ours.clone()), matched: Some(false) }), true)
        }
        ito_sftp::Error::VerifyUnavailable(_) => (None, true),
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
pub(crate) fn target(far: &Sftp, host_key: String, scheme: &str) -> Result<ito_sftp::Target, String> {
    let prefix = format!("KOYOMI_{}_", scheme.to_ascii_uppercase());
    let user = far
        .user
        .clone()
        .or_else(|| std::env::var("USERNAME").ok())
        .ok_or_else(|| format!("ユーザーがわからない（{scheme}://ユーザー@ホスト と書く）"))?;
    let auth = if let Some(k) = env_var(&prefix, "KEY") {
        ito_sftp::Auth::Key { path: PathBuf::from(k), passphrase: None }
    } else if let Some(p) = env_var(&prefix, "PASSWORD") {
        ito_sftp::Auth::Password(password(p)?)
    } else {
        let ssh = std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".ssh")).ok_or("USERPROFILE がない")?;
        let key = ["id_ed25519", "id_ecdsa"]
            .iter()
            .map(|n| ssh.join(n))
            .find(|p| p.is_file())
            .ok_or_else(|| format!("鍵がない: {}（key か password を書く）", ssh.join("id_ed25519").display()))?;
        ito_sftp::Auth::Key { path: key, passphrase: None }
    };
    let mut t = ito_sftp::Target::new(far.host.clone(), user, auth, host_key);
    if let Some(p) = far.port {
        t.port = p;
    }
    Ok(t)
}

/// A job's `password`: read from the OS credential store when it is `secret:<name>`.
pub(crate) fn password(p: String) -> Result<String, String> {
    match p.strip_prefix("secret:") {
        Some(name) => ito_secret::get("koyomi", name.trim())
            .map_err(|e| format!("秘密を読めない: {name}: {e}"))?
            .ok_or_else(|| format!("秘密がない: {name}（koyomi secret set {name}）")),
        None => Ok(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_maps_to_how_ito_sftp_checks() {
        assert_eq!(verify(None), ito_sftp::Verify::None);
        assert_eq!(verify(Some("sha256")), ito_sftp::Verify::Exec);
        assert_eq!(verify(Some("readback")), ito_sftp::Verify::Readback);
        assert_eq!(verify(Some("md5")), ito_sftp::Verify::None);
    }

    #[test]
    fn a_failed_check_is_recorded_and_is_the_reason() {
        let e = ito_sftp::Error::Mismatch { name: "a.csv".into(), ours: "11".into(), theirs: "22".into() };
        assert_eq!(
            failure(&e, "a.csv", 5),
            (Some(Moved { name: "a.csv".into(), bytes: 5, sha256: Some("11".into()), matched: Some(false) }), true)
        );
        assert_eq!(failure(&ito_sftp::Error::VerifyUnavailable("no exec".into()), "x", 0), (None, true));
        // Any other error is the log's only.
        assert_eq!(failure(&ito_sftp::Error::NotFound("/in/d.csv".into()), "d.csv", 0), (None, false));
        assert_eq!(
            record(ito_sftp::Moved { name: "b.csv".into(), bytes: 3, sha256: Some("33".into()) }),
            Moved { name: "b.csv".into(), bytes: 3, sha256: Some("33".into()), matched: Some(true) }
        );
        assert_eq!(
            record(ito_sftp::Moved { name: "c.csv".into(), bytes: 4, sha256: None }),
            Moved { name: "c.csv".into(), bytes: 4, sha256: None, matched: None }
        );
    }
}
