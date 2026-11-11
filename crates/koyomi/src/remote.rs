//! Other PCs' engines (TODO 80, DECISIONS「複数の PC・Web UI・利用者とロール・HA」): each PC
//! runs its own engine, and this one calls another's `[http]` over HTTPS with its certificate
//! pinned by the fingerprint written in `[[remote]]` of koyomi.toml. No central server.
//!
//! - `koyomi http fingerprint`: this PC's fingerprint, to write in the other PC's `[[remote]]`.
//! - `koyomi remote peek <url>`: the fingerprint a PC shows, to compare with what it says there
//!   before trusting it (the only time a certificate is looked at unpinned).
//! - `koyomi remote [list]`, `status <name>`, `nets <name>`, `runs <name>`, `run <name> <net>`.
//! - The window switches to one of them with [`names`] and [`ask`] (`ui::far`).
//! - A net goes to another PC (TODO 111) with [`there`] (what it has now) and [`propose`]
//!   (`POST /nets/propose`, where it lands unconfirmed): the window's 別の PC に送る and
//!   `koyomi remote send <name> <net>`.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ito_http::client::{self, Client, Reply};
use koyomi_core::config::Remote;
use koyomi_core::msg::{self, line};
use koyomi_core::scan::{Def, Standing};

use crate::engine::Paths;
use crate::proto::Request;

const PEEK_WAIT: Duration = Duration::from_secs(10);

/// `koyomi http fingerprint`.
pub fn http_cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "fingerprint" => {
            finish(crate::engine::http::fingerprint().map(|fp| format!("{fp}\n{}", msg::note(msg::N_REMOTE_FP_HINT, &[]))))
        }
        _ => usage("http takes fingerprint"),
    }
}

/// `koyomi remote …`.
pub fn cmd(args: &[String]) -> i32 {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = match a.as_slice() {
        [] | ["list"] => list(),
        ["peek", url] => peek(url),
        ["status", name] => with(name, status),
        ["nets", name] => with(name, nets),
        ["runs", name] => with(name, runs),
        ["run", name, net] => with(name, |r| run(r, net)),
        ["send", name, net] => send(name, &net.replace('\\', "/")),
        _ => return usage("remote takes list, peek <url>, status|nets|runs <name>, run <name> <group/name>, or send <name> <group/name>"),
    };
    finish(res)
}

fn usage(text: &str) -> i32 {
    crate::say(&format!("koyomi: {text}"), true);
    2
}

fn finish(res: Result<String, String>) -> i32 {
    match res {
        Ok(text) => {
            crate::say(&text, false);
            0
        }
        Err(e) => {
            // A coded line keeps its ID at the head.
            crate::say(&if msg::is_coded(&e) { e } else { format!("koyomi: {e}") }, true);
            1
        }
    }
}

/// `s` for a path segment or a query value: anything but a letter, a digit and `-._~` as `%XX`.
pub fn query(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn remotes() -> Result<Vec<Remote>, String> {
    Ok(Paths::find()?.config()?.remote)
}

/// Why a name cannot be a new `[[remote]]`'s.
#[derive(Debug, PartialEq, Eq)]
pub enum NameWhy {
    Empty,
    /// A `[[remote]]` has it already.
    Taken,
    /// Not usable as part of the secret's name `<name>-token`, or as a `[[remote]]` name.
    Bad,
}

/// Whether `name` may be a new `[[remote]]`'s: not empty, not one of `have`, and made of what a secret's name
/// and a `[[remote]]` name may hold (no space, no path separator, no control character).
pub fn check_name(name: &str, have: &[String]) -> Result<(), NameWhy> {
    let name = name.trim();
    if name.is_empty() {
        Err(NameWhy::Empty)
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '"' | '\'' | ':')) {
        Err(NameWhy::Bad)
    } else if have.iter().any(|h| h.trim().eq_ignore_ascii_case(name)) {
        Err(NameWhy::Taken)
    } else {
        Ok(())
    }
}

/// `text` (koyomi.toml) with `new` added to its `[[remote]]` after every one of `remotes` (those already read
/// from it, so none is lost). Comments and the other tables stay. The result must read as a koyomi.toml.
/// It refuses a name the file has already (the box's own list may be old) and a secret another `[[remote]]` uses
/// (writing the token there would overwrite theirs).
pub fn with_remote(text: &str, remotes: &[Remote], new: &Remote) -> Result<String, String> {
    let have: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
    match check_name(&new.name, &have) {
        Ok(()) => {}
        Err(NameWhy::Taken) => return Err(line(msg::NAME_TAKEN, &[new.name.trim()])),
        Err(NameWhy::Empty) => return Err(line(msg::NAME_EMPTY, &[])),
        Err(NameWhy::Bad) => return Err(line(msg::NAME_UNUSABLE, &[new.name.trim()])),
    }
    let secret = new.secret();
    if !secret.is_empty() && remotes.iter().any(|r| r.secret().eq_ignore_ascii_case(secret)) {
        return Err(line(msg::SECRET_IN_USE, &[secret]));
    }
    let quote = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let block = |r: &Remote| -> Vec<(&'static str, String)> {
        vec![("name", quote(&r.name)), ("url", quote(&r.url)), ("fingerprint", quote(&r.fingerprint)), ("token", quote(&r.token))]
    };
    let blocks: Vec<Vec<(&str, String)>> = remotes.iter().chain(std::iter::once(new)).map(block).collect();
    let out = ito_common::set_tables(text, "remote", &blocks)?;
    koyomi_core::config::Config::parse(&out)?;
    Ok(out)
}

/// Where this PC's koyomi.toml is and what it would hold with `new` added. It writes nothing, and it blocks:
/// not on the UI thread.
pub fn prepare_add(new: &Remote) -> Result<(PathBuf, String), String> {
    let paths = Paths::find()?;
    let path = paths.config.clone();
    let text = ito_common::read_or_empty(&path)?;
    let remotes = koyomi_core::config::Config::parse(&text)?.remote;
    let out = with_remote(&text, &remotes, new)?;
    Ok((path, out))
}

/// The one lock for this process's changes to koyomi.toml (add, rename, delete, and a settings save). `write_atomic`
/// replaces a whole file but does not serialise a read-modify-write, and each press runs on a thread of its own that
/// outlives the box.
static EDITS: Mutex<()> = Mutex::new(());

/// Takes [`EDITS`]; held from the read of koyomi.toml through the last clean-up. A poisoned lock is taken anyway:
/// what it guards is not in memory.
pub fn edits() -> MutexGuard<'static, ()> {
    EDITS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The OS credential store as far as a change to `[[remote]]` needs it (a fake stands in for it in tests).
pub trait SecretStore {
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// koyomi's own secrets in the OS credential store.
pub struct Keyring;

impl SecretStore for Keyring {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        ito_secret::get("koyomi", name).map_err(|e| e.to_string())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        ito_secret::set("koyomi", name, value).map_err(|e| e.to_string())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        ito_secret::delete("koyomi", name).map_err(|e| e.to_string())
    }
}

/// Every remote of `remotes` as the `[[remote]]` blocks `set_tables` takes.
fn blocks_of(remotes: &[&Remote]) -> Vec<Vec<(&'static str, String)>> {
    let quote = |s: &str| toml::Value::String(s.to_owned()).to_string();
    remotes
        .iter()
        .map(|r| vec![("name", quote(&r.name)), ("url", quote(&r.url)), ("fingerprint", quote(&r.fingerprint)), ("token", quote(&r.token))])
        .collect()
}

/// Refuses when `[standby] watch` or `[primary] standby` names `name` (compared as `Config::parse` does).
fn not_named_by_tables(cfg: &koyomi_core::config::Config, name: &str) -> Result<(), String> {
    if cfg.standby.as_ref().is_some_and(|s| s.watch.trim() == name) {
        return Err(line(msg::NAMED_BY_TABLE, &[name, "[standby] watch"]));
    }
    if cfg.primary.as_ref().is_some_and(|p| p.standby.trim() == name) {
        return Err(line(msg::NAMED_BY_TABLE, &[name, "[primary] standby"]));
    }
    Ok(())
}

/// `set_tables` with `after`, then the result must read as a koyomi.toml holding exactly those remotes.
fn rewrite(text: &str, after: &[&Remote]) -> Result<String, String> {
    let out = ito_common::set_tables(text, "remote", &blocks_of(after))?;
    let now = koyomi_core::config::Config::parse(&out)?.remote;
    let same = now.len() == after.len()
        && now.iter().zip(after).all(|(a, b)| a.name == b.name && a.url == b.url && a.fingerprint == b.fingerprint && a.token == b.token);
    if !same {
        return Err(line(msg::REWRITE_MISMATCH, &[]));
    }
    Ok(out)
}

/// `text` (koyomi.toml) with the `[[remote]]` called `old` called `new` instead. Only its `name` changes: the token
/// line stays, and so does the credential store (a name-derived secret may be used by another connection, a job or a
/// mail setting). Comments and the other tables stay.
pub fn plan_rename(text: &str, old: &str, new: &str) -> Result<String, String> {
    let (old, new) = (old.trim(), new.trim());
    let cfg = koyomi_core::config::Config::parse(text)?;
    if !cfg.remote.iter().any(|r| r.name.trim() == old) {
        return Err(line(msg::REMOTE_GONE, &[old]));
    }
    not_named_by_tables(&cfg, old)?;
    let others: Vec<String> = cfg.remote.iter().filter(|r| r.name.trim() != old).map(|r| r.name.clone()).collect();
    match check_name(new, &others) {
        Ok(()) => {}
        Err(NameWhy::Taken) => return Err(line(msg::NAME_TAKEN, &[new])),
        Err(NameWhy::Empty) => return Err(line(msg::NAME_EMPTY, &[])),
        Err(NameWhy::Bad) => return Err(line(msg::NAME_UNUSABLE, &[new])),
    }
    let renamed: Vec<Remote> =
        cfg.remote.iter().map(|r| if r.name.trim() == old { Remote { name: new.to_owned(), ..r.clone() } } else { r.clone() }).collect();
    rewrite(text, &renamed.iter().collect::<Vec<_>>())
}

/// What becomes of the secret of a connection that is deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretFate {
    /// koyomi made it (`<name>-token`) and nothing else names it.
    Delete(String),
    /// Left in the credential store (empty when the connection named none).
    Keep(String),
}

/// koyomi.toml without a `[[remote]]`, and what to do with its secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovePlan {
    pub text: String,
    pub secret: SecretFate,
}

/// Whether `secret` is the one `koyomi` generates for the connection `name`: `<name>-token`, ASCII case ignored.
/// The test `plan_remove` uses to decide whether a delete takes the secret along.
pub fn generated_secret(name: &str, secret: &str) -> bool {
    let secret = secret.trim();
    !secret.is_empty() && secret.eq_ignore_ascii_case(&format!("{}-token", name.trim()))
}

/// `text` (koyomi.toml) without the `[[remote]]` called `name`. The secret is `Delete` only when it is the generated
/// `<name>-token` and no string left in the file refers to it.
pub fn plan_remove(text: &str, name: &str) -> Result<RemovePlan, String> {
    let name = name.trim();
    let cfg = koyomi_core::config::Config::parse(text)?;
    let Some(gone) = cfg.remote.iter().find(|r| r.name.trim() == name) else {
        return Err(line(msg::REMOTE_GONE, &[name]));
    };
    not_named_by_tables(&cfg, name)?;
    let secret = gone.secret().to_owned();
    let left: Vec<&Remote> = cfg.remote.iter().filter(|r| r.name.trim() != name).collect();
    let out = rewrite(text, &left)?;
    let generated = generated_secret(name, &secret);
    let fate = if generated && !refers(&out, &secret)? { SecretFate::Delete(secret) } else { SecretFate::Keep(secret) };
    Ok(RemovePlan { text: out, secret: fate })
}

/// Whether any string in `toml_text` (read as TOML, so `secret: x` and `\u` escapes count as what they spell) refers
/// to `secret`: with its whitespace removed and in ASCII lower case it contains `secret:<secret>`. Over-matching
/// is fine, under-matching is not.
pub fn refers(toml_text: &str, secret: &str) -> Result<bool, String> {
    fn squash(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase()
    }
    fn walk(v: &toml::Value, want: &str) -> bool {
        match v {
            toml::Value::String(s) => squash(s).contains(want),
            toml::Value::Array(a) => a.iter().any(|v| walk(v, want)),
            toml::Value::Table(t) => t.iter().any(|(k, v)| squash(k).contains(want) || walk(v, want)),
            _ => false,
        }
    }
    let value: toml::Value = toml::from_str(toml_text).map_err(|e| e.to_string())?;
    Ok(walk(&value, &format!("secret:{}", squash(secret))))
}

/// Whether a file on this PC other than koyomi.toml may refer to `secret`: every `*.toml` under the settings folder
/// (job nets, calendars, …) and the staged `next.toml`. A file that cannot be read or parsed counts as a yes: it cannot
/// be ruled out. Only this PC's definitions matter, a credential store is per PC.
pub fn used_elsewhere(paths: &Paths, secret: &str) -> bool {
    fn files(dir: &std::path::Path, out: &mut Vec<PathBuf>) -> bool {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
        };
        for entry in entries {
            let Ok(entry) = entry else { return false };
            let (path, Ok(kind)) = (entry.path(), entry.file_type()) else { return false };
            if kind.is_dir() {
                if !files(&path, out) {
                    return false;
                }
            } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) {
                out.push(path);
            }
        }
        true
    }
    let mut list = Vec::new();
    if !files(&paths.app, &mut list) {
        return true;
    }
    list.retain(|p| !p.as_os_str().eq_ignore_ascii_case(paths.config.as_os_str()));
    if paths.next.exists() && !list.iter().any(|p| p.as_os_str().eq_ignore_ascii_case(paths.next.as_os_str())) {
        list.push(paths.next.clone());
    }
    list.iter().any(|p| match std::fs::read_to_string(p) {
        Ok(text) => refers(&text, secret).unwrap_or(true),
        Err(_) => true,
    })
}

/// Renames in `text` (the read of koyomi.toml at `path`), writes it with `write`, and moves the acknowledgement file
/// in `state`. The caller holds [`edits`]. The token and the credential store are not touched.
fn carry_rename(
    path: &std::path::Path,
    text: &str,
    old: &str,
    new: &str,
    state: Option<&std::path::Path>,
    write: impl FnOnce(&std::path::Path, String) -> Result<(), String>,
) -> Result<String, String> {
    let (old, new) = (old.trim(), new.trim());
    let out = plan_rename(text, old, new)?;
    write(path, out)?;
    let mut line = msg::note(msg::N_REMOTE_RENAMED, &[old, new]);
    if let Some(dir) = state {
        if let Err(e) = crate::acked::move_file(dir, old, new) {
            line = msg::note(msg::N_REMOTE_RENAMED_ACK, &[old, new, &e.to_string()]);
        }
    }
    Ok(line)
}

/// 名前を変える: `old` is `new` in this PC's koyomi.toml. It blocks: not on the UI thread.
pub fn rename(old: &str, new: &str, state: Option<&std::path::Path>) -> Result<String, String> {
    let _held = edits();
    let paths = Paths::find()?;
    let text = ito_common::read_or_empty(&paths.config)?;
    carry_rename(&paths.config, &text, old, new, state, ito_common::write_atomic)
}

/// Deletes in `text`: `elsewhere` says whether a file other than koyomi.toml refers to a secret that would go. Writes
/// first, and only then touches the credential store. The caller holds [`edits`].
fn carry_remove(
    path: &std::path::Path,
    text: &str,
    name: &str,
    elsewhere: impl FnOnce(&str) -> bool,
    store: &dyn SecretStore,
    write: impl FnOnce(&std::path::Path, String) -> Result<(), String>,
) -> Result<String, String> {
    let name = name.trim();
    let plan = plan_remove(text, name)?;
    let fate = match plan.secret {
        SecretFate::Delete(s) if elsewhere(&s) => SecretFate::Keep(s),
        other => other,
    };
    write(path, plan.text)?;
    let mut line = msg::note(msg::N_REMOTE_REMOVED, &[name]);
    match fate {
        SecretFate::Delete(s) => {
            if let Err(e) = store.delete(&s) {
                line = msg::note(msg::N_REMOTE_REMOVED_SECRET_LEFT, &[name, &s, &e.to_string()]);
            }
        }
        SecretFate::Keep(s) if !s.is_empty() => {
            line = msg::note(msg::N_REMOTE_REMOVED_SECRET_KEPT, &[name, &s]);
        }
        SecretFate::Keep(_) => {}
    }
    Ok(line)
}

/// 消す: the `[[remote]]` called `name` leaves this PC's koyomi.toml. It blocks: not on the UI thread.
pub fn remove(name: &str) -> Result<String, String> {
    let _held = edits();
    let paths = Paths::find()?;
    let text = ito_common::read_or_empty(&paths.config)?;
    carry_remove(&paths.config, &text, name, |s| used_elsewhere(&paths, s), &Keyring, ito_common::write_atomic)
}

/// What the add box says when koyomi.toml could not be written: the write's reason, and, if the token that was
/// stored for it could not be taken out again, that it is still in the credential store. Never the token.
pub(crate) fn write_failed(name: &str, write: &str, undone: Result<(), String>) -> String {
    match undone {
        Ok(()) => line(msg::CONFIG_WRITE_FAILED, &[name, write]),
        Err(d) => line(msg::CONFIG_WRITE_FAILED_TOKEN_LEFT, &[name, write, &d]),
    }
}

/// The last steps of adding a connection, after the check passed and `prepare_add` made `text`: the token goes to the
/// credential store, then koyomi.toml is written; a koyomi.toml that cannot be written gives the token back. A secret
/// that is there already is never overwritten (a delete may have left `<name>-token` on purpose). The caller holds
/// [`edits`].
pub(crate) fn finish_add(
    name: &str,
    secret: &str,
    token: &str,
    path: &std::path::Path,
    text: String,
    store: &dyn SecretStore,
    write: impl FnOnce(&std::path::Path, String) -> Result<(), String>,
) -> Result<(), String> {
    match store.get(secret) {
        Err(e) => return Err(line(msg::TOKEN_SECRET_UNREADABLE, &[name, secret, &e])),
        Ok(Some(_)) => return Err(line(msg::SECRET_EXISTS, &[secret])),
        Ok(None) => {}
    }
    store.set(secret, token).map_err(|e| line(msg::TOKEN_STORE_FAILED, &[name, &e]))?;
    write(path, text).map_err(|e| write_failed(name, &e, store.delete(secret)))
}

/// Each `[[remote]]`'s trimmed name and its secret name, in the order of koyomi.toml. A koyomi.toml with a token
/// written as is does not read (`KOYOMI-E1023`), so the secret is never empty. It reads koyomi.toml: not on the UI thread.
pub fn listed() -> Result<Vec<(String, String)>, String> {
    Ok(remotes()?.iter().map(|r| (r.name.trim().to_owned(), r.secret().to_owned())).collect())
}

/// One call to the `[[remote]]` named `name`, cut off after `deadline`, counted from when it goes out. It blocks.
pub fn ask_within(
    name: &str,
    deadline: Duration,
    f: impl FnOnce(&Client) -> Result<Reply, client::Error>,
) -> Result<serde_json::Value, String> {
    with(name, |r| {
        let mut c = client(r)?;
        c.deadline = Some(deadline);
        answer(r, f(&c).map_err(|e| why(r, &e))?)
    })
}

pub(crate) fn with<T>(name: &str, f: impl FnOnce(&Remote) -> Result<T, String>) -> Result<T, String> {
    let all = remotes()?;
    let r = all.iter().find(|r| r.name.trim() == name).ok_or_else(|| {
        let names: Vec<_> = all.iter().map(|r| r.name.trim()).collect();
        if names.is_empty() {
            line(msg::NO_REMOTE, &[name])
        } else {
            line(msg::UNKNOWN_REMOTE, &[name, &names.join(", ")])
        }
    })?;
    f(r)
}

pub(crate) fn client(r: &Remote) -> Result<Client, String> {
    let name = r.secret();
    let token = ito_secret::get("koyomi", name)
        .map_err(|e| line(msg::TOKEN_SECRET_UNREADABLE, &[&r.name, name, &e.to_string()]))?
        .ok_or_else(|| line(msg::TOKEN_SECRET_MISSING, &[&r.name, name]))?;
    Client::new(r.url.trim(), r.fingerprint.trim(), token).map_err(|e| why(r, &e))
}

/// One call, with what went wrong in the person's words.
fn call(r: &Remote, f: impl FnOnce(&Client) -> Result<Reply, client::Error>) -> Result<serde_json::Value, String> {
    let reply = f(&client(r)?).map_err(|e| why(r, &e))?;
    answer(r, reply)
}

/// A reply's JSON when it is a yes, else what went wrong in the person's words.
fn answer(r: &Remote, reply: Reply) -> Result<serde_json::Value, String> {
    match reply.status {
        200..=299 => Ok(reply.json),
        401 => Err(line(msg::TOKEN_REFUSED, &[&r.name, r.secret()])),
        s => Err(unexpected_status(&r.name, s, &reply.json)),
    }
}

/// What a status the caller does not know is, in the person's words. The engine's own coded line (a 403 for a
/// role) goes through as it is.
pub(crate) fn unexpected_status(name: &str, status: u16, json: &serde_json::Value) -> String {
    let error = json["error"].as_str().unwrap_or_default();
    if msg::is_coded(error) {
        return error.to_owned();
    }
    line(msg::UNEXPECTED_STATUS, &[name, format!("{status} {error}").trim_end()])
}

pub(crate) fn why(r: &Remote, e: &client::Error) -> String {
    reason(&r.name, e)
}

/// What went wrong with a call to `name` (a `[[remote]]` name, or the URL when none is registered yet).
pub(crate) fn reason(name: &str, e: &client::Error) -> String {
    match e {
        client::Error::Fingerprint(got) => line(msg::CERT_MISMATCH, &[name, got]),
        client::Error::Connect(s) => line(msg::CONNECT_FAILED, &[name, s]),
        client::Error::Deadline => line(msg::TIMED_OUT, &[name]),
        client::Error::Tls(s) => line(msg::TLS_FAILED, &[name, s]),
        client::Error::Url(s) => line(msg::ADDRESS_UNREADABLE, &[s]),
        client::Error::Bad(s) => line(msg::REPLY_UNREADABLE, &[name, s]),
    }
}

fn list() -> Result<String, String> {
    let all = remotes()?;
    if all.is_empty() {
        return Ok(msg::note(msg::N_REMOTE_NONE, &[]));
    }
    let lines: Vec<String> = all
        .iter()
        .map(|r| match call(r, |c| c.get("/health")) {
            Ok(j) => format!(
                "{}  {}  {}",
                r.name,
                r.url,
                j["status"].as_str().map_or_else(|| msg::note(msg::N_REMOTE_ANSWERED, &[]), str::to_owned)
            ),
            // A coded line already names the connection and keeps its ID at the head.
            Err(e) if msg::is_coded(&e) => e,
            Err(e) => format!("{}  {}  ✕ {}", r.name, r.url, e),
        })
        .collect();
    Ok(lines.join("\n"))
}

/// The fingerprint the PC at `url` shows, or why not in the person's words. It blocks: not on the UI thread.
pub(crate) fn peek_fingerprint(url: &str) -> Result<String, String> {
    client::peek_fingerprint(url.trim(), PEEK_WAIT).map_err(|e| reason(url.trim(), &e))
}

fn peek(url: &str) -> Result<String, String> {
    let fp = peek_fingerprint(url)?;
    Ok(format!("fingerprint = \"{fp}\"\n{}", msg::note(msg::N_REMOTE_PEEK_HINT, &[])))
}

fn status(r: &Remote) -> Result<String, String> {
    let j = call(r, |c| c.get("/health"))?;
    Ok(msg::note(msg::N_REMOTE_STATUS, &[&r.name, j["status"].as_str().unwrap_or_default(), j["version"].as_str().unwrap_or("?")]))
}

fn nets(r: &Remote) -> Result<String, String> {
    let j = call(r, |c| c.get("/nets"))?;
    Ok(nets_text(&j))
}

fn nets_text(j: &serde_json::Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if j["paused"].as_bool() == Some(true) {
        lines.push(msg::note(msg::N_REMOTE_PAUSED, &[]));
    }
    for n in j["nets"].as_array().into_iter().flatten() {
        let held = if n["held"].as_bool() == Some(true) { format!("  {}", msg::note(msg::N_REMOTE_HELD, &[])) } else { String::new() };
        lines.push(format!(
            "{}  {}  {}{held}",
            n["id"].as_str().unwrap_or("?"),
            n["name"].as_str().unwrap_or(""),
            n["standing"].as_str().unwrap_or("")
        ));
    }
    if lines.is_empty() {
        lines.push(msg::note(msg::N_REMOTE_NO_NETS, &[]));
    }
    lines.join("\n")
}

fn runs(r: &Remote) -> Result<String, String> {
    let j = call(r, |c| c.get("/runs"))?;
    Ok(runs_text(&j))
}

fn runs_text(j: &serde_json::Value) -> String {
    let lines: Vec<String> = j["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let over =
                if r["over"].as_bool() == Some(true) { String::new() } else { format!("  {}", msg::note(msg::N_REMOTE_RUNNING, &[])) };
            format!(
                "{}  {}  {}{over}",
                r["id"].as_str().unwrap_or("?"),
                r["jobnet"].as_str().unwrap_or(""),
                r["title"].as_str().unwrap_or("")
            )
        })
        .collect();
    if lines.is_empty() {
        msg::note(msg::N_REMOTE_NO_RUNS, &[])
    } else {
        lines.join("\n")
    }
}

fn run(r: &Remote, net: &str) -> Result<String, String> {
    let j = call(r, |c| c.post_json("/runs", &serde_json::json!({ "net": net })))?;
    Ok(msg::note(msg::N_REMOTE_RUN_DONE, &[&r.name, net, j["run"].as_str().unwrap_or("?")]))
}

/// What `GET /nets/text` says of a net on another PC (TODO 111).
#[derive(Debug, Clone, PartialEq)]
pub struct There {
    pub text: String,
    /// Sent back as `base`: a file changed there since it was read is not overwritten.
    pub sha256: String,
    /// `None` when that PC does not say.
    pub standing: Option<Standing>,
}

impl There {
    fn from_json(j: &serde_json::Value) -> There {
        There {
            text: j["text"].as_str().unwrap_or_default().to_owned(),
            sha256: j["sha256"].as_str().unwrap_or_default().to_owned(),
            standing: serde_json::from_value(j["standing"].clone()).ok(),
        }
    }
}

/// The net `net` as the `[[remote]]` named `name` has it, `None` when it has no such net.
/// That PC's token needs the `edit` role. It blocks: not on the UI thread.
pub fn there(name: &str, net: &str) -> Result<Option<There>, String> {
    with(name, |r| {
        let reply = client(r)?.get(&format!("/nets/text?net={}", query(net))).map_err(|e| why(r, &e))?;
        if reply.status == 404 {
            return Ok(None);
        }
        answer(r, reply).map(|j| Some(There::from_json(&j)))
    })
}

/// Sends `text` as the net `net` to the `[[remote]]` named `name` (`POST /nets/propose`), where it
/// lands unconfirmed until that PC's window confirms it. `base` is the hash [`there`] read, `None`
/// for a net that PC does not have. `Ok(true)` when the text is the one already confirmed there.
/// It blocks: not on the UI thread.
pub fn propose(name: &str, net: &str, text: &str, base: Option<&str>) -> Result<bool, String> {
    with(name, |r| {
        let mut body = serde_json::json!({ "net": net, "text": text });
        if let Some(b) = base {
            body["base"] = b.into();
        }
        let reply = client(r)?.post_json("/nets/propose", &body).map_err(|e| why(r, &e))?;
        if reply.status == 409 {
            return Err(line(msg::REMOTE_NET_CHANGED, &[&r.name, net]));
        }
        answer(r, reply).map(|j| j["confirmed"].as_bool() == Some(true))
    })
}

/// `koyomi remote send <name> <group/name>`: the change from what that PC has to this PC's net,
/// then `y` sends it.
fn send(name: &str, id: &str) -> Result<String, String> {
    let def: Def = crate::client::call(&Request::Show { id: id.to_owned() }).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string()))?;
    if def.standing == Standing::Invalid {
        return Err(line(msg::CMD_SEND_INVALID, &[id]));
    }
    let there = there(name, id)?;
    if there.as_ref().is_some_and(|t| t.text == def.text) {
        return Ok(msg::note(msg::N_SEND_SAME, &[name, id]));
    }
    let head = match &there {
        Some(_) => msg::note(msg::N_SEND_DIFF, &[name, id]),
        None => msg::note(msg::N_SEND_NEW, &[name, id]),
    };
    crate::say(&head, false);
    let old = there.as_ref().map_or("", |t| t.text.as_str());
    for l in crate::review::diff_text(&ito_diff::diff(old, &def.text, 3)) {
        crate::say(&l, false);
    }
    if def.standing != Standing::Confirmed {
        crate::say(&msg::note(msg::N_SEND_UNCONFIRMED_HERE, &[]), false);
    }
    crate::say(&msg::note(msg::N_SEND_ASK, &[]), false);
    let mut typed = String::new();
    let _ = std::io::stdin().read_line(&mut typed);
    if !typed.trim().eq_ignore_ascii_case("y") {
        return Err(line(msg::CMD_SEND_DECLINED, &[id]));
    }
    let same = propose(name, id, &def.text, there.as_ref().map(|t| t.sha256.as_str()))?;
    Ok(sent_line(name, id, same))
}

/// What a send says once that PC took it.
fn sent_line(name: &str, net: &str, same: bool) -> String {
    msg::note(if same { msg::N_SEND_DONE_SAME } else { msg::N_SEND_DONE }, &[name, net])
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_peek_error_is_coded_and_names_the_address() {
        let t = super::reason("https://h:8443", &ito_http::client::Error::Connect("os error 10061".into()));
        assert!(msg::CONNECT_FAILED.is_in(&t) && t.contains("https://h:8443") && t.contains("os error 10061"), "{t}");
    }

    use super::*;

    /// A name goes into a query as it is when it can, and percent-encoded when it cannot.
    #[test]
    fn a_job_is_sent_as_a_query_value() {
        assert_eq!(query("20261030-020000-01"), "20261030-020000-01");
        assert_eq!(query("a b&c=日"), "a%20b%26c%3D%E6%97%A5");
    }

    /// the lists say paused, held and running in a line each, and say when there is nothing.
    #[test]
    fn the_lists_read_as_lines() {
        let j = serde_json::json!({ "paused": true, "nets": [
            { "id": "g/a", "name": "A", "standing": "confirmed", "held": false },
            { "id": "g/b", "name": "B", "standing": "unconfirmed", "held": true },
        ]});
        let (paused, held) = (msg::note(msg::N_REMOTE_PAUSED, &[]), msg::note(msg::N_REMOTE_HELD, &[]));
        assert_eq!(nets_text(&j), format!("{paused}\ng/a  A  confirmed\ng/b  B  unconfirmed  {held}"));
        assert_eq!(nets_text(&serde_json::json!({ "nets": [] })), msg::note(msg::N_REMOTE_NO_NETS, &[]));
        let j = serde_json::json!({ "runs": [
            { "id": "1", "jobnet": "g/a", "title": "A", "over": true },
            { "id": "2", "jobnet": "g/b", "title": "B", "over": false },
        ]});
        assert_eq!(runs_text(&j), format!("1  g/a  A\n2  g/b  B  {}", msg::note(msg::N_REMOTE_RUNNING, &[])));
        assert_eq!(runs_text(&serde_json::json!({ "runs": [] })), msg::note(msg::N_REMOTE_NO_RUNS, &[]));
    }

    /// What another PC has of a net is read from `GET /nets/text`; one that does not say its
    /// standing reads as none. A send says whether it waits for that PC's window.
    #[test]
    fn a_net_there_is_read_and_a_send_is_said() {
        let j = serde_json::json!({ "net": "g/a", "text": "format = 1\n", "sha256": "abc", "standing": "unconfirmed" });
        let want = There { text: "format = 1\n".into(), sha256: "abc".into(), standing: Some(Standing::Unconfirmed) };
        assert_eq!(There::from_json(&j), want);
        assert_eq!(There::from_json(&serde_json::json!({ "text": "x", "sha256": "s" })).standing, None);
        assert_eq!(sent_line("本番", "g/a", false), msg::note(msg::N_SEND_DONE, &["本番", "g/a"]));
        assert_eq!(sent_line("本番", "g/a", true), msg::note(msg::N_SEND_DONE_SAME, &["本番", "g/a"]));
        // Unconfirmed says whose screen confirms it; the same-content send does not wait.
        assert!(sent_line("本番", "g/a", false).matches("本番").count() == 2);
        assert!(sent_line("本番", "g/a", false) != sent_line("本番", "g/a", true));
    }

    fn remote_of(name: &str) -> Remote {
        Remote {
            name: name.into(),
            url: format!("https://{name}:8443"),
            fingerprint: "ab".repeat(32),
            token: format!("secret:{name}-token"),
        }
    }

    #[test]
    fn with_remote_adds_one_and_keeps_comments_other_tables_and_every_existing_remote() {
        let text = "# my settings\nkeep_days = 30 # keep\n\n[vars]\nplace = \"x\"\n\n# the factory\n[[remote]]\nname = \"fact\"\nurl = \"https://fact:8443\"\nfingerprint = \"".to_owned()
            + &"cd".repeat(32)
            + "\"\ntoken = \"secret:fact-token\"\n\n[[remote]]\nname = \"office\"\nurl = \"https://office:8443\"\nfingerprint = \""
            + &"ef".repeat(32)
            + "\"\ntoken = \"secret:office-token\"\n";
        let have = koyomi_core::config::Config::parse(&text).unwrap().remote;
        assert_eq!(have.len(), 2);
        let out = with_remote(&text, &have, &remote_of("new")).unwrap();
        for kept in ["# my settings", "keep_days = 30 # keep", "[vars]", "place = \"x\"", "# the factory"] {
            assert!(out.contains(kept), "{kept} lost:\n{out}");
        }
        let now = koyomi_core::config::Config::parse(&out).unwrap().remote;
        let names: Vec<&str> = now.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["fact", "office", "new"]);
        assert_eq!(now[0].fingerprint, "cd".repeat(32));
        assert_eq!(now[2].token, "secret:new-token");
        assert_eq!(now[2].url, "https://new:8443");
    }

    #[test]
    fn with_remote_works_on_an_empty_text() {
        let out = with_remote("", &[], &remote_of("new")).unwrap();
        let now = koyomi_core::config::Config::parse(&out).unwrap().remote;
        assert_eq!(now.len(), 1);
        assert_eq!(now[0].name, "new");
    }

    #[test]
    fn with_remote_refuses_a_name_the_file_has_in_any_case_and_a_secret_another_remote_uses() {
        let have = vec![remote_of("Fact")];
        let text = with_remote("", &[], &have[0]).unwrap();
        let e = with_remote(&text, &have, &remote_of("fact")).unwrap_err();
        assert!(msg::NAME_TAKEN.is_in(&e) && e.contains("fact"), "{e}");
        assert!(with_remote(&text, &have, &remote_of("FACT")).is_err());
        let mut other = remote_of("new");
        other.token = "secret:FACT-token".into();
        let e = with_remote(&text, &have, &other).unwrap_err();
        assert!(msg::SECRET_IN_USE.is_in(&e) && e.contains("FACT-token"), "{e}");
        // Nothing of either was written, and a fresh name and secret still go in.
        assert!(with_remote(&text, &have, &remote_of("new")).is_ok());
    }

    #[test]
    fn a_remote_that_does_not_read_back_is_not_written() {
        let mut bad = remote_of("new");
        bad.fingerprint = "short".into();
        assert!(with_remote("", &[], &bad).is_err());
    }

    #[test]
    fn a_remote_name_is_checked_for_empty_taken_and_secret_name_characters() {
        let have = vec!["Fact".to_owned()];
        assert_eq!(check_name("", &have), Err(NameWhy::Empty));
        assert_eq!(check_name("  ", &have), Err(NameWhy::Empty));
        assert_eq!(check_name("fact", &have), Err(NameWhy::Taken));
        assert_eq!(check_name(" Fact ", &have), Err(NameWhy::Taken));
        for bad in ["a b", "a/b", "a\\b", "a\"b", "a'b", "a:b", "a\tb", "a\nb"] {
            assert_eq!(check_name(bad, &have), Err(NameWhy::Bad), "{bad:?}");
        }
        assert_eq!(check_name("office-2", &have), Ok(()));
        assert_eq!(check_name("工場", &have), Ok(()));
    }

    const FP: &str = "abababababababababababababababababababababababababababababababab";

    /// A koyomi.toml with a comment, another table, and the given `[[remote]]` names (token `secret:<name>-token`).
    fn file_with(names: &[&str], tail: &str) -> String {
        let mut t = "# my settings\nkeep_days = 30 # keep\n\n[vars]\nplace = \"x\"\n\n".to_owned();
        for n in names {
            t += &format!("# the {n}\n[[remote]]\nname = \"{n}\"\nurl = \"https://{n}:8443\"\nfingerprint = \"{FP}\"\ntoken = \"secret:{n}-token\"\n\n");
        }
        t + tail
    }

    fn names_in(text: &str) -> Vec<String> {
        koyomi_core::config::Config::parse(text).unwrap().remote.into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn a_rename_changes_the_name_only_and_keeps_the_rest() {
        let text = file_with(&["fact", "office"], "");
        let out = plan_rename(&text, "fact", "works").unwrap();
        for kept in ["# my settings", "keep_days = 30 # keep", "[vars]", "place = \"x\"", "# the fact", "# the office"] {
            assert!(out.contains(kept), "{kept} lost:\n{out}");
        }
        let now = koyomi_core::config::Config::parse(&out).unwrap().remote;
        assert_eq!(now.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["works", "office"]);
        assert_eq!(now[0].token, "secret:fact-token", "the token line stays as it was");
        assert_eq!(now[0].url, "https://fact:8443");
        assert_eq!(now[1].token, "secret:office-token");
    }

    #[test]
    fn a_change_of_case_only_is_allowed_and_leaves_the_token() {
        let text = file_with(&["fact"], "");
        let out = plan_rename(&text, "fact", "Fact").unwrap();
        let now = koyomi_core::config::Config::parse(&out).unwrap().remote;
        assert_eq!((now[0].name.as_str(), now[0].token.as_str()), ("Fact", "secret:fact-token"));
    }

    #[test]
    fn a_rename_is_refused_for_a_taken_bad_or_empty_name_or_one_not_in_the_file() {
        let text = file_with(&["fact", "office"], "");
        for new in ["OFFICE", "office", "", "  ", "a b", "a:b"] {
            assert!(plan_rename(&text, "fact", new).is_err(), "{new:?}");
        }
        let e = plan_rename(&text, "ghost", "x").unwrap_err();
        assert!(msg::REMOTE_GONE.is_in(&e) && e.contains("ghost"), "{e}");
    }

    #[test]
    fn a_connection_named_by_standby_or_primary_cannot_be_renamed_or_removed() {
        let standby = file_with(&["fact", "office"], "[standby]\nwatch = \"fact\"\n");
        let primary = file_with(&["fact", "office"], "[primary]\nstandby = \"fact\"\n");
        for (text, table) in [(&standby, "[standby]"), (&primary, "[primary]")] {
            let e = plan_rename(text, "fact", "works").unwrap_err();
            assert!(msg::NAMED_BY_TABLE.is_in(&e) && e.contains(table), "{e}");
            let e = plan_remove(text, "fact").unwrap_err();
            assert!(msg::NAMED_BY_TABLE.is_in(&e) && e.contains(table), "{e}");
            // The other one is free.
            assert!(plan_rename(text, "office", "works").is_ok());
            assert!(plan_remove(text, "office").is_ok());
        }
    }

    #[test]
    fn removing_the_last_one_leaves_a_file_that_parses_with_the_rest() {
        let text = file_with(&["fact"], "");
        let plan = plan_remove(&text, "fact").unwrap();
        assert!(names_in(&plan.text).is_empty());
        assert!(!plan.text.contains("[[remote]]"), "{}", plan.text);
        for kept in ["# my settings", "keep_days = 30 # keep", "[vars]", "place = \"x\""] {
            assert!(plan.text.contains(kept), "{kept} lost:\n{}", plan.text);
        }
        assert_eq!(plan.secret, SecretFate::Delete("fact-token".into()));
    }

    #[test]
    fn removing_a_middle_one_keeps_the_others_and_their_comments() {
        let text = file_with(&["a", "b", "c"], "");
        let plan = plan_remove(&text, "b").unwrap();
        assert_eq!(names_in(&plan.text), ["a", "c"]);
        assert!(plan.text.contains("# the a") && plan.text.contains("# the c") && plan.text.contains("[vars]"));
        assert!(plan_remove(&text, "ghost").is_err());
    }

    #[test]
    fn generated_secret_is_the_name_token_in_any_ascii_case() {
        assert!(generated_secret("works", "works-token"));
        assert!(generated_secret("works", "Works-Token"));
        assert!(generated_secret(" works ", " works-token "));
        assert!(!generated_secret("works", ""));
        assert!(!generated_secret("works", "   "));
        assert!(!generated_secret("factory", "works-token"));
        assert!(!generated_secret("works", "shared"));
    }

    #[test]
    fn a_generated_secret_goes_a_custom_one_stays_and_one_still_named_stays() {
        let text = file_with(&["fact", "office"], "");
        assert_eq!(plan_remove(&text, "fact").unwrap().secret, SecretFate::Delete("fact-token".into()));
        let custom = text.replace("secret:fact-token", "secret:shared");
        assert_eq!(plan_remove(&custom, "fact").unwrap().secret, SecretFate::Keep("shared".into()));
        // Another remote names the generated secret: plainly, with a space after the colon, and by a `\u` escape.
        for spelling in ["secret:fact-token", "secret: fact-token", "secret:\\u0066act-token", "secret:Fact-Token"] {
            let other = text.replace("secret:office-token", spelling);
            assert_eq!(plan_remove(&other, "fact").unwrap().secret, SecretFate::Keep("fact-token".into()), "{spelling}");
        }
        // Another table names it.
        let table = file_with(&["fact"], "[ping]\nurl = \"secret: fact-token\"\n");
        assert_eq!(plan_remove(&table, "fact").unwrap().secret, SecretFate::Keep("fact-token".into()));
        // The generated name in another case still counts as generated.
        let upper = file_with(&["fact"], "").replace("secret:fact-token", "secret:FACT-token");
        assert_eq!(plan_remove(&upper, "fact").unwrap().secret, SecretFate::Delete("FACT-token".into()));
    }

    #[test]
    fn refers_reads_values_through_nested_tables_and_arrays() {
        let t = "[a.b]\nlist = [1, [\"x\", { deep = \"use secret: Tok-1 here\" }]]\n";
        assert!(refers(t, "tok-1").unwrap());
        assert!(refers(t, "TOK-1").unwrap());
        assert!(!refers(t, "tok-2").unwrap());
        assert!(!refers("a = \"tok-1\"\n", "tok-1").unwrap(), "the word alone is not a reference");
        assert!(refers("a = \"secret:\\u0074ok\"\n", "tok").unwrap());
        assert!(refers("not toml [", "x").is_err());
    }

    fn paths_in(dir: &std::path::Path) -> Paths {
        let (app, state) = (dir.join("app"), dir.join("state"));
        std::fs::create_dir_all(app.join("jobnets/g")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        Paths {
            jobnets: app.join("jobnets"),
            calendars: app.join("calendars"),
            config: app.join("koyomi.toml"),
            approved: state.join("approved.toml"),
            held: state.join("held.toml"),
            next: state.join("next.toml"),
            app,
            state,
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("koyomi-remote-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_secret_used_by_a_file_other_than_koyomi_toml_is_found() {
        let d = temp("elsewhere");
        let p = paths_in(&d);
        std::fs::write(&p.config, "[mail]\npassword = \"secret:fact-token\"\n").unwrap();
        assert!(!used_elsewhere(&p, "fact-token"), "koyomi.toml itself is left out, and a next file that is not there is fine");
        let net = p.jobnets.join("g/a.toml");
        std::fs::write(&net, "token = \"secret:other\"\n").unwrap();
        assert!(!used_elsewhere(&p, "fact-token"));
        std::fs::write(&net, "token = \"secret: fact-token\"\n").unwrap();
        assert!(used_elsewhere(&p, "fact-token"), "a space after the colon counts");
        std::fs::write(&net, "this does not parse [\n").unwrap();
        assert!(used_elsewhere(&p, "fact-token"), "a file that does not read cannot be ruled out");
        std::fs::remove_file(&net).unwrap();
        std::fs::write(&p.next, "x = \"secret:fact-token\"\n").unwrap();
        assert!(used_elsewhere(&p, "fact-token"), "the staged definitions count");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Records the calls made; never a value.
    #[derive(Default)]
    struct Fake {
        calls: std::cell::RefCell<Vec<String>>,
        has: Option<String>,
        get_fails: bool,
        delete_fails: bool,
    }

    impl SecretStore for Fake {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            self.calls.borrow_mut().push(format!("get {name}"));
            if self.get_fails {
                Err("store down".into())
            } else {
                Ok(self.has.clone())
            }
        }
        fn set(&self, name: &str, _value: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("set {name}"));
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("delete {name}"));
            if self.delete_fails {
                Err("denied".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn a_rename_writes_the_file_and_moves_the_acknowledgements_and_makes_no_store_call() {
        let d = temp("rename");
        let text = file_with(&["fact"], "");
        let path = d.join("koyomi.toml");
        std::fs::write(d.join(crate::acked::file_name(Some("fact"))), "acked = [\"r1\"]\n").unwrap();
        let wrote = std::cell::RefCell::new(None);
        let line = carry_rename(&path, &text, "fact", "works", Some(&d), |p, t| {
            *wrote.borrow_mut() = Some((p.to_path_buf(), t));
            Ok(())
        })
        .unwrap();
        assert_eq!(line, msg::note(msg::N_REMOTE_RENAMED, &["fact", "works"]));
        let (p, t) = wrote.into_inner().expect("written");
        assert_eq!(p, path);
        assert_eq!(names_in(&t), ["works"]);
        assert!(d.join(crate::acked::file_name(Some("works"))).exists());
        assert!(!d.join(crate::acked::file_name(Some("fact"))).exists());
        // A refused plan or a failed write changes nothing: no move.
        std::fs::write(d.join(crate::acked::file_name(Some("fact"))), "acked = []\n").unwrap();
        assert!(carry_rename(&path, &text, "fact", "a b", Some(&d), |_, _| panic!("no write")).is_err());
        let e = carry_rename(&path, &text, "fact", "works2", Some(&d), |_, _| Err("read only".into())).unwrap_err();
        assert_eq!(e, "read only");
        assert!(d.join(crate::acked::file_name(Some("fact"))).exists(), "the file did not move");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_delete_writes_before_it_touches_the_store_and_keeps_what_it_should() {
        let text = file_with(&["fact", "office"], "");
        let path = std::path::Path::new("koyomi.toml");
        let order = std::cell::RefCell::new(Vec::<String>::new());
        let store = Fake::default();
        let write = |_: &std::path::Path, _: String| {
            order.borrow_mut().push("write".into());
            Ok(())
        };
        let line = carry_remove(path, &text, "fact", |_| false, &store, write).unwrap();
        order.borrow_mut().extend(store.calls.borrow().iter().cloned());
        assert_eq!(*order.borrow(), ["write", "delete fact-token"]);
        assert_eq!(line, msg::note(msg::N_REMOTE_REMOVED, &["fact"]));

        // A write error leaves the secret, and makes no call.
        let store = Fake::default();
        let e = carry_remove(path, &text, "fact", |_| false, &store, |_, _| Err("read only".into())).unwrap_err();
        assert_eq!(e, "read only");
        assert!(store.calls.borrow().is_empty());

        // A delete error is a success with a note that names the secret.
        let store = Fake { delete_fails: true, ..Fake::default() };
        let line = carry_remove(path, &text, "fact", |_| false, &store, |_, _| Ok(())).unwrap();
        assert!(
            line == msg::note(msg::N_REMOTE_REMOVED_SECRET_LEFT, &["fact", "fact-token", "denied"])
                && line.contains("fact-token")
                && line.contains("denied"),
            "{line}"
        );

        // Used elsewhere, or not generated: Keep, no delete call, and it says how to remove it.
        let store = Fake::default();
        let line = carry_remove(path, &text, "fact", |_| true, &store, |_, _| Ok(())).unwrap();
        assert!(store.calls.borrow().is_empty());
        assert_eq!(line, msg::note(msg::N_REMOTE_REMOVED_SECRET_KEPT, &["fact", "fact-token"]));
        assert!(line.contains("koyomi secret delete fact-token"));
        let custom = text.replace("secret:fact-token", "secret:shared");
        let line = carry_remove(path, &custom, "fact", |_| false, &store, |_, _| Ok(())).unwrap();
        assert!(store.calls.borrow().is_empty());
        assert!(line.contains("shared"), "{line}");
    }

    #[test]
    fn the_edit_lock_makes_a_second_taker_wait() {
        use std::sync::mpsc;
        use std::time::Duration;
        let first = edits();
        let (tx, rx) = mpsc::channel::<()>();
        let t = std::thread::spawn(move || {
            let _second = edits();
            tx.send(()).unwrap();
        });
        // Without the lock the thread would answer long before this runs out.
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "the second taker is held up while the first holds it");
        drop(first);
        rx.recv_timeout(Duration::from_secs(10)).expect("the second taker gets it once the first lets go");
        t.join().unwrap();
    }

    /// A new connection's secret: the add box's last steps (`finish_add`).
    #[test]
    fn adding_never_overwrites_a_secret_that_is_there_and_takes_its_own_back() {
        let path = std::path::Path::new("koyomi.toml");
        // Free: get, set, write, in that order.
        let store = Fake::default();
        finish_add("kei", "kei-token", "tok", path, "text".into(), &store, |_, _| Ok(())).unwrap();
        assert_eq!(*store.calls.borrow(), ["get kei-token", "set kei-token"]);
        // A value already there: no set, no write.
        let store = Fake { has: Some("old".into()), ..Fake::default() };
        let e = finish_add("kei", "kei-token", "tok", path, "text".into(), &store, |_, _| panic!("no write")).unwrap_err();
        assert!(msg::SECRET_EXISTS.is_in(&e) && e.contains("kei-token") && e.contains("koyomi secret delete kei-token"), "{e}");
        assert_eq!(*store.calls.borrow(), ["get kei-token"]);
        // A get that fails: no set, no write.
        let store = Fake { get_fails: true, ..Fake::default() };
        let e = finish_add("kei", "kei-token", "tok", path, "text".into(), &store, |_, _| panic!("no write")).unwrap_err();
        assert!(msg::TOKEN_SECRET_UNREADABLE.is_in(&e) && e.contains("store down"), "{e}");
        assert_eq!(*store.calls.borrow(), ["get kei-token"]);
        // A write that fails takes the new secret back.
        let store = Fake::default();
        let e = finish_add("kei", "kei-token", "tok", path, "text".into(), &store, |_, _| Err("read only".into())).unwrap_err();
        assert!(msg::CONFIG_WRITE_FAILED.is_in(&e) && e.contains("kei") && e.contains("read only"), "{e}");
        assert_eq!(*store.calls.borrow(), ["get kei-token", "set kei-token", "delete kei-token"]);
        // ... and says so when it cannot.
        let store = Fake { delete_fails: true, ..Fake::default() };
        let e = finish_add("kei", "kei-token", "tok", path, "text".into(), &store, |_, _| Err("read only".into())).unwrap_err();
        assert!(msg::CONFIG_WRITE_FAILED_TOKEN_LEFT.is_in(&e) && e.contains("denied") && !e.contains("tok)"), "{e}");
    }
}
