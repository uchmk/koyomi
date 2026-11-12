//! Which engine the window talks to: this PC's (over the door) or another PC's (over `POST /door`).
//!
//! [`Link::Local`] is what the window always did: it starts the engine when it is down. [`Link::Far`]
//! never starts anything: it reads the `[[remote]]` of that name, checks with `GET /me` that the token may
//! drive a window, and then sends each request as `POST /door`.

use std::time::{Duration, Instant};

use ito_http::client::{self, Client, Reply};
use koyomi_core::config::Remote;
use koyomi_core::msg::{self, line};
use koyomi_core::role::Role;

use crate::proto::{self, Request};
use crate::{client as local, remote};

/// How often the engine of this PC is asked for the list and the runs.
const POLL_LOCAL: Duration = Duration::from_secs(1);
/// How often another PC is asked: a request costs a TLS handshake there.
pub const POLL_FAR: Duration = Duration::from_secs(3);
/// The longest one call to another PC may take, from name lookup to the last byte of the answer. When it
/// fires, the far side may still have acted on the request.
const CALL_DEADLINE: Duration = Duration::from_secs(30);
/// The largest answer taken from another PC: a year of `History` is more than the 4 MiB default.
const MAX_REPLY: usize = 64 * 1024 * 1024;
/// How long a read that could not reach another PC waits before it is sent once more. The engine there restarts its
/// listener when `[http]` changes (a user added or a token made again on 利用者), and a read that comes in at that
/// moment is refused or cut off.
const READ_AGAIN: Duration = Duration::from_secs(1);
/// A read is sent once more only when it failed sooner than this: an address no one answers or a network that drops
/// the packets fails slowly, and waiting for it twice would only make the person wait longer. A PC whose engine is
/// stopped refuses at once, so its read is sent once more too, and fails again a second later.
const QUICK_FAIL: Duration = Duration::from_secs(3);

/// Which engine a window talks to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// The engine of this PC, started when it is down.
    Local,
    /// The engine of the `[[remote]]` of this name.
    Far(String),
}

/// A link that has started: what it needs to send requests.
pub struct Conn {
    kind: Kind,
    role: Option<Role>,
    /// The `[[http.user]]` name the engine knows this connection by (`/me`'s `who`); `None` for this PC.
    who: Option<String>,
}

enum Kind {
    Local,
    Far { remote: Remote, client: Box<Client> },
}

impl Link {
    /// The `[[remote]]` name, `None` for this PC.
    pub fn far(&self) -> Option<&str> {
        match self {
            Link::Local => None,
            Link::Far(name) => Some(name),
        }
    }

    pub fn is_far(&self) -> bool {
        matches!(self, Link::Far(_))
    }

    /// How often the list is asked for.
    pub fn poll(&self) -> Duration {
        match self {
            Link::Local => POLL_LOCAL,
            Link::Far(_) => POLL_FAR,
        }
    }

    /// Connects. Another PC's engine is never started from here; a name that is not a `[[remote]]` is an
    /// error and does not fall back to this PC.
    pub fn start(&self) -> Result<Conn, String> {
        match self {
            Link::Local => local::start().map(|()| Conn { kind: Kind::Local, role: None, who: None }),
            Link::Far(name) => remote::with(name, connect),
        }
    }
}

/// Connects to a `[[remote]]`: the client, then `GET /me`, which says whether this token may drive a window.
/// It never starts an engine.
fn connect(r: &Remote) -> Result<Conn, String> {
    let client = build(r)?;
    let reply = client.get("/me").map_err(|e| remote::why(r, &e))?;
    let role = match reply.status {
        200..=299 => admit(&r.name, &reply.json)?,
        s => return Err(read_status(r, s, &reply.json)),
    };
    let who = who_of(&reply.json);
    Ok(Conn { kind: Kind::Far { remote: r.clone(), client: Box::new(client) }, role: Some(role), who })
}

/// The one place a client for another PC is made.
fn build(r: &Remote) -> Result<Client, String> {
    Ok(limited(remote::client(r)?))
}

/// `client` with the deadline and the reply limit every call to another PC has.
fn limited(mut client: Client) -> Client {
    client.deadline = Some(CALL_DEADLINE);
    client.max_reply = MAX_REPLY;
    client
}

/// `client` made to wait [`proto::PEEK_CALL_WAIT`] for a read and for the whole call.
fn slowed(client: &mut Client) {
    client.timeout = proto::PEEK_CALL_WAIT;
    client.deadline = Some(proto::PEEK_CALL_WAIT);
}

/// The check the add-a-connection box makes before it writes anything: the PC at `url` (its certificate pinned
/// to `fingerprint`) answers `GET /health`, and `token` is one a window may be driven with (`GET /me`). The
/// client is built from what was typed: no secret is read, and nothing is written. The token is never put in a
/// message.
pub fn probe(name: &str, url: &str, fingerprint: &str, token: &str) -> Result<Role, String> {
    let r = Remote {
        name: name.to_owned(),
        url: url.to_owned(),
        fingerprint: fingerprint.to_owned(),
        token: format!("secret:{name}-token"),
        watch: true,
    };
    let client = limited(Client::new(url.trim(), fingerprint.trim(), token).map_err(|e| remote::why(&r, &e))?);
    check(&r, &client)
}

/// The check the manage box makes before it changes the URL of `r`: as [`probe`], with `url` and `fingerprint` in
/// place of the stored ones and the token read from the credential store as a connection reads it. The token is
/// never put in a message, and nothing is written.
pub fn probe_stored(r: &Remote, url: &str, fingerprint: &str) -> Result<Role, String> {
    let moved = Remote { url: url.to_owned(), fingerprint: fingerprint.to_owned(), ..r.clone() };
    check(&moved, &build(&moved)?)
}

/// `GET /health`, then `GET /me`, which says whether the token may drive a window.
fn check(r: &Remote, client: &Client) -> Result<Role, String> {
    let health = client.get("/health").map_err(|e| remote::why(r, &e))?;
    if !(200..=299).contains(&health.status) {
        return Err(read_status(r, health.status, &health.json));
    }
    let me = client.get("/me").map_err(|e| remote::why(r, &e))?;
    match me.status {
        200..=299 => admit(&r.name, &me.json),
        s => Err(read_status(r, s, &me.json)),
    }
}

impl Conn {
    /// The caller's role over there; `None` for this PC.
    pub fn role(&self) -> Option<Role> {
        self.role
    }

    /// The user name this connection is, over there; `None` for this PC, or when the engine named none.
    pub fn who(&self) -> Option<&str> {
        self.who.as_deref()
    }

    /// Whether the engine still answers. Another PC counts as up once started: a failed call says otherwise.
    pub fn running(&self) -> bool {
        match self.kind {
            Kind::Local => local::running(),
            Kind::Far { .. } => true,
        }
    }

    /// Lets this connection wait [`proto::PEEK_CALL_WAIT`] for one answer, as a peek at a host needs (the
    /// engine answers within [`proto::PEEK_ANSWER_WAIT`]). Another PC's client otherwise gives up on a quiet read
    /// after its per-step timeout; this PC's pipe has no such limit.
    pub fn slow(&mut self) {
        if let Kind::Far { client, .. } = &mut self.kind {
            slowed(client);
        }
    }

    /// One request. It blocks: not on the UI thread.
    pub fn call(&self, request: &Request) -> Result<String, String> {
        match &self.kind {
            Kind::Local => local::call(request),
            Kind::Far { remote, client } => {
                let body = serde_json::json!({ "proto": proto::PROTO, "request": request });
                match send(request.is_read(), || client.post_json("/door", &body), std::thread::sleep) {
                    Ok(reply) => read_reply(remote, request, &reply),
                    Err(e) => Err(unsure(remote, request, &e)),
                }
            }
        }
    }
}

/// Calls `post`; when `read` and it could not reach the other PC quickly ([`send_again`]), `wait`s [`READ_AGAIN`] and
/// calls it once more. A request that is not a read is called once whatever happens: it may have arrived ([`unsure`]).
fn send<T>(read: bool, mut post: impl FnMut() -> Result<T, client::Error>, wait: impl FnOnce(Duration)) -> Result<T, client::Error> {
    let started = Instant::now();
    let sent = post();
    if read && send_again(&sent, started.elapsed()) {
        wait(READ_AGAIN);
        return post();
    }
    sent
}

/// Whether a call that ended with `sent` after `took` failed in the way a restarting listener makes it fail: it could
/// not reach the other PC (refused or cut off), and quickly. Only [`send`] asks, and only for a read.
fn send_again<T>(sent: &Result<T, client::Error>, took: Duration) -> bool {
    matches!(sent, Err(client::Error::Connect(_))) && took < QUICK_FAIL
}

/// The `who` of a `GET /me` answer: only a non-empty string is a name.
pub fn who_of(me: &serde_json::Value) -> Option<String> {
    me["who"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// The role of a `GET /me` answer when a window may be driven with it: `edit` or `admin`, not narrowed.
pub fn admit(name: &str, me: &serde_json::Value) -> Result<Role, String> {
    let role = me["role"].as_str().and_then(|s| Role::parse(s).ok());
    match role {
        Some(r) if r >= Role::Edit && me["nets"].is_null() && me["roles"].is_null() => Ok(r),
        _ => Err(line(msg::NOT_ADMITTED, &[name])),
    }
}

/// What a `/door` answer says, in the person's words: the engine's own text for a yes, its own error for a no.
pub fn read_reply(r: &Remote, request: &Request, reply: &Reply) -> Result<String, String> {
    match reply.status {
        200 => {
            let j = &reply.json;
            match (j["ok"].as_bool(), j["text"].as_str(), j["error"].as_str()) {
                (Some(true), Some(text), _) => Ok(text.to_owned()),
                (Some(false), _, Some(error)) if !error.is_empty() => Err(error.to_owned()),
                // A 200 that says neither yes nor no is not a yes: nothing is confirmed.
                _ => Err(if request.is_read() {
                    line(msg::REPLY_UNREADABLE, &[&r.name, "neither ok nor error in the answer"])
                } else {
                    line(msg::MAY_HAVE_ARRIVED, &["neither ok nor error in the answer"])
                }),
            }
        }
        s => Err(read_status(r, s, &reply.json)),
    }
}

fn read_status(r: &Remote, status: u16, json: &serde_json::Value) -> String {
    match status {
        401 => line(msg::TOKEN_REFUSED, &[&r.name, r.secret()]),
        // The engine's own line (a role that is too low, a protocol that differs) goes through as it is.
        403 | 409 => json["error"].as_str().unwrap_or_default().to_owned(),
        404 => line(msg::ENGINE_TOO_OLD, &[&r.name]),
        s => remote::unexpected_status(&r.name, s, json),
    }
}

/// Why a call to another PC failed. A request which is not a read, on a connection that broke or timed out or
/// on an answer that did not read, may have arrived: that is E2102 with the reason, and it is never sent again
/// on its own.
fn unsure(r: &Remote, request: &Request, e: &client::Error) -> String {
    if !request.is_read() && matches!(e, client::Error::Connect(_) | client::Error::Bad(_) | client::Error::Deadline) {
        line(msg::MAY_HAVE_ARRIVED, &[&e.to_string()])
    } else {
        remote::why(r, e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn remote() -> Remote {
        Remote { name: "kei".into(), url: "https://h:1".into(), fingerprint: "ab".into(), token: "secret:kei-token".into(), watch: true }
    }

    fn reply(status: u16, json: serde_json::Value) -> Reply {
        Reply { status, json }
    }

    #[test]
    fn a_slowed_client_waits_for_a_peek_answer_in_every_step_and_in_all() {
        let mut client = limited(Client::new("https://127.0.0.1:1", &"ab".repeat(32), "t").unwrap());
        assert!(client.timeout < proto::PEEK_ANSWER_WAIT);
        slowed(&mut client);
        assert!(client.timeout > proto::PEEK_ANSWER_WAIT && client.deadline.unwrap() > proto::PEEK_ANSWER_WAIT);
        assert_eq!((client.timeout, client.deadline), (proto::PEEK_CALL_WAIT, Some(proto::PEEK_CALL_WAIT)));
    }

    #[test]
    fn who_is_only_a_non_empty_string() {
        assert_eq!(who_of(&json!({"who": "kei", "role": "admin"})), Some("kei".to_owned()));
        assert_eq!(who_of(&json!({"role": "admin"})), None);
        assert_eq!(who_of(&json!({"who": 3})), None);
        assert_eq!(who_of(&json!({"who": ""})), None);
        assert_eq!(who_of(&json!({"who": null})), None);
    }

    #[test]
    fn far_never_starts_the_engine() {
        let before = local::STARTS.with(std::cell::Cell::get);
        let r = Link::Far("no-such-pc-for-this-test".into()).start();
        assert!(r.is_err());
        // A remote that is found but cannot be reached (no such secret, nothing listening) is an error too.
        let found = Remote {
            name: "kei".into(),
            url: "https://127.0.0.1:1".into(),
            fingerprint: "ab".repeat(32),
            token: "secret:no-such-secret-for-this-test".into(),
            watch: true,
        };
        assert!(connect(&found).is_err());
        assert_eq!(local::STARTS.with(std::cell::Cell::get), before);
    }

    #[test]
    fn a_200_that_is_not_a_clear_yes_or_no_is_an_error() {
        let r = remote();
        let save = Request::Save { id: "g/a".into(), text: "x".into(), base_sha256: None, force: false };
        for body in [json!({"ok": true}), json!({"ok": false}), json!({"ok": false, "error": ""}), json!(null)] {
            let e = read_reply(&r, &save, &reply(200, body.clone())).unwrap_err();
            assert!(msg::MAY_HAVE_ARRIVED.is_in(&e), "{body}: {e}");
            let e = read_reply(&r, &Request::List, &reply(200, body.clone())).unwrap_err();
            assert!(msg::REPLY_UNREADABLE.is_in(&e) && e.contains("kei"), "{body}: {e}");
        }
    }

    #[test]
    fn an_unknown_name_is_an_error_not_this_pc() {
        let e = Link::Far("no-such-pc-for-this-test".into()).start().err().expect("an error");
        assert!(e.contains("no-such-pc-for-this-test"), "{e}");
    }

    #[test]
    fn a_probe_with_an_address_that_does_not_read_fails_without_the_network() {
        let e = probe("kei", "http://not-https", &"ab".repeat(32), "tok").unwrap_err();
        assert!(msg::ADDRESS_UNREADABLE.is_in(&e) && !e.contains("tok"), "{e}");
    }

    #[test]
    fn the_polls_differ() {
        assert_eq!(Link::Local.poll(), Duration::from_secs(1));
        assert_eq!(Link::Far("x".into()).poll(), Duration::from_secs(3));
    }

    #[test]
    fn a_yes_is_the_engines_text() {
        assert_eq!(
            read_reply(&remote(), &Request::List, &reply(200, json!({"ok": true, "text": "保存した: g/a"}))),
            Ok("保存した: g/a".to_owned())
        );
    }

    #[test]
    fn a_no_is_the_engines_error_verbatim() {
        let no = reply(200, json!({"ok": false, "error": "ほかで変わっています: g/a"}));
        assert_eq!(read_reply(&remote(), &Request::List, &no), Err("ほかで変わっています: g/a".to_owned()));
    }

    #[test]
    fn the_statuses_have_their_words() {
        let r = remote();
        let e = |s, j| read_reply(&r, &Request::List, &reply(s, j)).unwrap_err();
        assert!(msg::TOKEN_REFUSED.is_in(&e(401, json!({}))));
        assert!(e(401, json!({})).contains("kei-token"));
        assert_eq!(e(403, json!({"error": "kei は view のロール。これには edit が要る"})), "kei は view のロール。これには edit が要る");
        assert!(msg::ENGINE_TOO_OLD.is_in(&e(404, json!({}))));
        assert_eq!(
            e(409, json!({"error": "窓とエンジンの版が違う（窓 45、エンジン 44）"})),
            "窓とエンジンの版が違う（窓 45、エンジン 44）"
        );
        let five = e(500, json!({"error": "x"}));
        assert!(msg::UNEXPECTED_STATUS.is_in(&five) && five.contains("500 x"), "{five}");
        // An engine's coded line is not put behind a status.
        let coded = line(msg::ROLE_TOO_LOW, &["kei", "view", "edit"]);
        assert_eq!(e(500, json!({"error": coded.clone()})), coded);
    }

    #[test]
    fn me_is_admitted_for_edit_and_admin_only_and_not_narrowed() {
        let me =
            |role: &str, nets: serde_json::Value, roles: serde_json::Value| json!({"who": "u", "role": role, "nets": nets, "roles": roles});
        assert_eq!(admit("kei", &me("edit", json!(null), json!(null))), Ok(Role::Edit));
        assert_eq!(admit("kei", &me("admin", json!(null), json!(null))), Ok(Role::Admin));
        for (role, nets, roles) in [
            ("view", json!(null), json!(null)),
            ("run", json!(null), json!(null)),
            ("admin", json!(["a/*"]), json!(null)),
            ("edit", json!(null), json!({"a/*": "view"})),
        ] {
            let e = admit("kei", &me(role, nets, roles)).unwrap_err();
            assert!(msg::NOT_ADMITTED.is_in(&e) && e.contains("kei"), "{e}");
        }
    }

    #[test]
    fn only_a_request_that_is_not_a_read_is_called_unsure() {
        let r = remote();
        let e = client::Error::Connect("timed out".into());
        let save = Request::Save { id: "g/a".into(), text: "x".into(), base_sha256: None, force: false };
        let t = unsure(&r, &save, &e);
        assert!(msg::MAY_HAVE_ARRIVED.is_in(&t) && t.contains("timed out"), "{t}");
        assert!(msg::CONNECT_FAILED.is_in(&unsure(&r, &Request::List, &e)));
        let late = client::Error::Deadline;
        assert!(msg::MAY_HAVE_ARRIVED.is_in(&unsure(&r, &save, &late)));
        assert!(msg::TIMED_OUT.is_in(&unsure(&r, &Request::List, &late)));
        for e in [client::Error::Fingerprint("f".into()), client::Error::Url("u".into()), client::Error::Tls("t".into())] {
            assert!(!msg::MAY_HAVE_ARRIVED.is_in(&unsure(&r, &save, &e)));
        }
    }

    #[test]
    fn a_deadline_is_coded_without_the_unsure_tail() {
        let r = remote();
        let t = remote::why(&r, &client::Error::Deadline);
        assert!(msg::TIMED_OUT.is_in(&t) && t.contains("kei"), "{t}");
        assert!(!msg::MAY_HAVE_ARRIVED.is_in(&t), "{t}");
    }

    #[test]
    fn only_a_quick_failure_to_reach_the_pc_is_sent_again() {
        let quick = Duration::from_millis(80);
        let cut: Result<(), _> = Err(client::Error::Connect("os error 10054".into()));
        assert!(send_again(&cut, quick), "the listener was restarting");
        assert!(!send_again(&cut, QUICK_FAIL), "a PC that is off fails slowly");
        assert!(!send_again(&Ok(()), quick));
        for e in [
            client::Error::Deadline,
            client::Error::Bad("b".into()),
            client::Error::Tls("t".into()),
            client::Error::Fingerprint("f".into()),
            client::Error::Url("u".into()),
        ] {
            assert!(!send_again(&Err::<(), _>(e.clone()), quick), "{e}");
        }
    }

    #[test]
    fn only_a_read_is_sent_again_and_only_once() {
        let cut = || Err::<u16, _>(client::Error::Connect("os error 10054".into()));
        // A read cut off at once: one wait of READ_AGAIN, then the second answer is the answer.
        let (mut calls, mut waited) = (0, Vec::new());
        let r = send(
            true,
            || {
                calls += 1;
                if calls == 1 {
                    cut()
                } else {
                    Ok(200)
                }
            },
            |d| waited.push(d),
        );
        assert_eq!((r, calls, waited), (Ok(200), 2, vec![READ_AGAIN]));
        // Cut off twice: the second failure is the answer, and there is no third call.
        let (mut calls, mut waited) = (0, Vec::new());
        let r = send(
            true,
            || {
                calls += 1;
                cut()
            },
            |d| waited.push(d),
        );
        assert_eq!((r, calls, waited.len()), (cut(), 2, 1));
        // Not a read: one call, no wait, whatever the failure.
        let (mut calls, mut waited) = (0, Vec::new());
        let r = send(
            false,
            || {
                calls += 1;
                cut()
            },
            |d| waited.push(d),
        );
        assert_eq!((r, calls, waited.len()), (cut(), 1, 0));
        // A read that worked, or failed in another way: one call.
        let mut calls = 0;
        let r = send(
            true,
            || {
                calls += 1;
                Err::<u16, _>(client::Error::Deadline)
            },
            |_| panic!("no wait"),
        );
        assert_eq!((r, calls), (Err(client::Error::Deadline), 1));
    }
}
