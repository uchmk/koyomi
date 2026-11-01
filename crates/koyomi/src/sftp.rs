//! `koyomi sftp`: an SFTP `send` or `fetch` job's work (`run`, which reads what to do
//! from the `KOYOMI_SFTP_*` the engine sets), and the host key to pin (`hostkey`), for
//! an `ssh` job too.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use koyomi_core::chore::{self, Sftp};

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
/// [`NOTHING`] when there was no file, 2 on an error (with the reason after `! `).
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
    let opts = ito_sftp::Options { overwrite: true };
    if send {
        let Some(local) = sendable(Path::new(&from)) else {
            crate::say(&format!("no file to send: {from}"), false);
            return Ok(0);
        };
        let remote = format!("{}/", far.path.trim_end_matches('/'));
        let moved = match session.put(&local, &remote, opts, &never, &mut |_, _, _| {}) {
            Err(ito_sftp::Error::NotFound(p)) if p == local.display().to_string() => {
                crate::say(&format!("no file to send: {from}"), false);
                return Ok(0);
            }
            r => r.map_err(|e| format!("{remote}: {e}"))?,
        };
        for m in &moved {
            crate::say(&format!("sent: {} ({} bytes)", m.name, m.bytes), false);
        }
        Ok(moved.len())
    } else {
        let (dir, pattern) = far.path.rsplit_once('/').map_or((".", far.path.as_str()), |(d, p)| (if d.is_empty() { "/" } else { d }, p));
        let names: Vec<String> = session
            .list(dir)
            .map_err(|e| format!("{dir}: {e}"))?
            .into_iter()
            .filter(|e| !e.is_dir && ito_sftp::glob_match(pattern, &e.name, false))
            .map(|e| e.name)
            .collect();
        if names.is_empty() {
            crate::say(&format!("no file to fetch: {from}"), false);
            return Ok(0);
        }
        let into = PathBuf::from(&into);
        std::fs::create_dir_all(&into).map_err(|e| format!("{}: {e}", into.display()))?;
        for name in &names {
            let remote = if dir == "/" { format!("/{name}") } else { format!("{dir}/{name}") };
            for m in session.get(&remote, &into.join(name), opts, &never, &mut |_, _, _| {}).map_err(|e| format!("{remote}: {e}"))? {
                crate::say(&format!("fetched: {} ({} bytes)", m.name, m.bytes), false);
            }
        }
        Ok(names.len())
    }
}

/// What `put` takes for `from`: a folder's files as `<folder>\*`, a wildcard or a file as
/// it is; `None` when there is no such file.
fn sendable(from: &Path) -> Option<PathBuf> {
    if from.is_dir() {
        return Some(from.join("*"));
    }
    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (ito_sftp::has_wildcard(&name) || from.is_file()).then(|| from.to_path_buf())
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
        let p = match p.strip_prefix("secret:") {
            Some(name) => ito_secret::get("koyomi", name.trim())
                .map_err(|e| format!("秘密を読めない: {name}: {e}"))?
                .ok_or_else(|| format!("秘密がない: {name}（koyomi secret set {name}）"))?,
            None => p,
        };
        ito_sftp::Auth::Password(p)
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
