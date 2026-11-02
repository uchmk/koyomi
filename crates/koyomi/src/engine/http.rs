//! `[http]` of koyomi.toml: while the engine runs it listens on this PC's loopback
//! (ito-http), so another program can start a net and follow its run without the CLI.
//! With `lan = true` it listens on the network instead, over TLS only, with a self-signed
//! certificate kept in `app_dir`'s `tls` folder; another PC pins its fingerprint
//! (`koyomi http fingerprint`, TODO 80).
//!
//! - `POST /runs` with `{"net": "<group>/<name>", "date"?, "from"?, "params"?}` is
//!   `koyomi run`: 202 and `{"run": "<id>"}`, or 422 and the engine's reason.
//! - `GET /runs/<id>` is the run as `koyomi status` / `history` know it, with `"over"`.
//! - Read only, for another PC's window, the Web UI and an HA standby (TODO 80, 81, 87):
//!   `GET /health` (the engine's status line, and `"ha"` with `[standby]` or `[primary]`), `GET /nets` (each definition's id, name,
//!   standing, hold and the hash of its file), `GET /runs` (today's and the running ones; `?from=&to=` for days, `?running=1` only those not over)
//!   and `GET /runs/<id>/log?job=<job>` (the tail of a job's log, secrets already `***`).
//! - `POST /runs/<id>/stop` stops the run's jobs (`koyomi kill`).
//! - `POST /nets/skip-next` with `{"net", "cancel"?}` skips the net's next scheduled run
//!   (`koyomi skip-next`), for the `run` role like hold and release.
//! - `POST /standby/release`, for the `run` role (TODO 96): a primary with `[primary]` takes its
//!   schedules back from this standby: 200 once it let go, 409 with why not.
//! - Definitions, for the `edit` role (TODO 83): `GET /nets/text?net=<group>/<name>` is the
//!   file's text and hash; `POST /nets/propose` with `{"net", "text", "base"?}` writes it as
//!   the editor's save does, but leaves it unconfirmed: it runs only once this PC's window
//!   confirms it. `base` is the hash read with the text; a file changed since gets 409.
//! - The Web UI (TODO 81): `GET /` and its script and style sheet, kept in the exe, need no
//!   token; the page signs in with a token at ito-http's `POST /session` and goes on with the
//!   cookie (a change also sends `X-Ito-Csrf: 1`). `GET /me` says who and with what role.
//!
//! Every request carries a bearer token kept as `secret:<name>` in the OS credential
//! store (read once when the listener starts); ito-http turns a missing or wrong one away
//! with 401 and never logs it. `[http]`'s own token has the `run` role; each
//! `[[http.user]]` has its own token and role (TODO 83), and a path the role does not reach
//! gets 403. The requests go through the engine's own door as `Request::Over`, so they are
//! what the CLI may do and are written down as by-hand ops with who sent them.
//!
//! A `[[http.user]]` with `nets = ["経理/*", …]` (TODO 94) reaches only those nets: every
//! answer is cut to them in [`narrow`], a net or run outside answers as one that is not there,
//! and a proposal outside or anything about every net gets 403. `GET /me` says the patterns.
//! An HA standby copies every net, so its user has no `nets`.
//!
//! A `[[http.user]]` with `roles = { "経理/*" = "run", "基盤/*" = "view" }` (TODO 101) has a role
//! per net, as JP1's resource groups do. The gate in front of every path lets through what their
//! strongest role reaches; [`narrow`] then checks their role on the net the request is about, and
//! what is no one net's (an HA primary taking its schedules back) against `role`. `GET /nets`
//! says their role on each net, and `GET /me` the patterns, so the page shows each net's buttons.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ito_http::{Response, Server};
use koyomi_core::config;
use koyomi_core::role::{self, Access, Nets, Role};
use koyomi_core::run::{Run, State};
use serde::Deserialize;

use crate::proto::{self, Request, Runs, Snapshot};

/// How long after a failed start the listener is tried again.
const RETRY: Duration = Duration::from_secs(60);

pub struct Http {
    /// The `[http]` the listener is (or is being) started for.
    cfg: Option<config::Http>,
    server: Option<Server>,
    /// Why the last start failed.
    note: Option<String>,
    /// When a start was last asked for.
    tried: Option<Instant>,
    starting: bool,
    /// Whether the tailnet port was open when last looked at, so its opening is logged once.
    tailnet_open: bool,
    /// Which `[http]` a start was for; an answer for an older one is dropped.
    gen: u64,
    tx: mpsc::Sender<(u64, Result<Server, String>)>,
    rx: mpsc::Receiver<(u64, Result<Server, String>)>,
}

impl Default for Http {
    fn default() -> Http {
        let (tx, rx) = mpsc::channel();
        Http { cfg: None, server: None, note: None, tried: None, starting: false, tailnet_open: false, gen: 0, tx, rx }
    }
}

impl Http {
    /// Starts, restarts or stops the listener to match `cfg`. The token is read and the
    /// port taken on a thread (the store can be slow); a failed start is tried again after
    /// [`RETRY`].
    pub fn tick(&mut self, cfg: Option<&config::Http>) {
        while let Ok((gen, r)) = self.rx.try_recv() {
            if gen != self.gen {
                continue;
            }
            self.starting = false;
            match r {
                Ok(s) => {
                    crate::say(&format!("koyomi: {}", listening(self.cfg.as_ref(), &s)), false);
                    if let Some(why) = tailnet_note(self.cfg.as_ref(), &s) {
                        crate::say(&why, false);
                    }
                    self.tailnet_open = s.tailnet_port().is_some();
                    self.server = Some(s);
                    self.note = None;
                }
                Err(e) => self.note = Some(format!("koyomi.toml: [http] で待ち受けられない（{e}）。{} 秒ごとに試す", RETRY.as_secs())),
            }
        }
        // ito-http keeps trying the tailnet port behind the open main one (Tailscale not up yet).
        if let Some(p) = self.server.as_ref().filter(|_| !self.tailnet_open).and_then(Server::tailnet_port) {
            self.tailnet_open = true;
            crate::say(&format!("koyomi: HTTP: tailnet の :{p} を開いた（Tailscale の相手は token なし）"), false);
        }
        if self.cfg.as_ref() != cfg {
            self.stop();
            self.cfg = cfg.cloned();
        }
        let Some(cfg) = &self.cfg else { return };
        if self.server.is_some() || self.starting || self.tried.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        self.tried = Some(Instant::now());
        self.starting = true;
        let (gen, tx, cfg) = (self.gen, self.tx.clone(), cfg.clone());
        let _ = std::thread::Builder::new().name("http".into()).spawn(move || {
            let _ = tx.send((gen, start(&cfg)));
        });
    }

    /// Stops listening (a changed `[http]`, or the engine ending).
    pub fn stop(&mut self) {
        self.gen += 1;
        self.starting = false;
        self.tailnet_open = false;
        self.tried = None;
        self.note = None;
        if let Some(s) = self.server.take() {
            s.stop();
        }
    }

    /// What a person should see about `[http]`.
    pub fn notes(&self) -> Vec<String> {
        let tailnet = self.server.as_ref().and_then(|s| tailnet_note(self.cfg.as_ref(), s));
        self.note.iter().cloned().chain(tailnet).collect()
    }

    /// The status line while it listens.
    pub fn line(&self) -> Option<String> {
        self.server.as_ref().map(|s| listening(self.cfg.as_ref(), s))
    }
}

fn listening(cfg: Option<&config::Http>, s: &Server) -> String {
    let port = s.port();
    let tailnet = match (s.tailnet_port(), cfg.and_then(|c| c.tailnet)) {
        (Some(p), _) => format!("。tailnet の :{p} でも（Tailscale の相手は token なし）"),
        (None, Some(p)) => format!("。tailnet の :{p} はまだ開けない"),
        (None, None) => String::new(),
    };
    if let Some(fp) = s.fingerprint() {
        return format!("HTTP: LAN の :{port} で TLS で待ち受けている（指紋 {}）{tailnet}", ito_http::tls::format(&fp));
    }
    let v6 = if cfg.is_some_and(|c| c.ipv6) { format!("・[::1]:{port}") } else { String::new() };
    format!("HTTP: 127.0.0.1:{port}{v6} で待ち受けている{tailnet}")
}

/// Why the tailnet port is not open yet (Tailscale not running, no tailnet address, the port
/// taken). ito-http has opened the other listeners and keeps trying this one behind.
fn tailnet_note(cfg: Option<&config::Http>, s: &Server) -> Option<String> {
    let port = cfg?.tailnet?;
    if s.tailnet_port().is_some() {
        return None;
    }
    let why = s.tailnet_error().map(|e| format!("（{e}）")).unwrap_or_default();
    Some(format!("koyomi.toml: [http] の tailnet の :{port} がまだ開けない{why}。Tailscale が動けば開く"))
}

/// Where the LAN listener's certificate and key are kept: `app_dir`'s `tls`, beside the
/// person's other secrets.
pub fn cert_dir() -> Option<std::path::PathBuf> {
    ito_common::app_dir("koyomi").map(|d| d.join("tls"))
}

/// `koyomi http fingerprint`: this PC's certificate's fingerprint, made if there is none yet.
/// What another PC writes in its `[[remote]]`.
pub fn fingerprint() -> Result<String, String> {
    let dir = cert_dir().ok_or("設定のフォルダーが分からない")?;
    ito_http::tls::fingerprint(&dir).map(|fp| ito_http::tls::format(&fp)).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Reads a token kept as `secret:<name>`.
fn token(name: &str) -> Result<String, String> {
    ito_http::token_from_secret("koyomi", name).map_err(|e| format!("token の秘密 {name}: {e}（koyomi secret set {name}）"))
}

/// Who may do what at most: `default` for `[http]`'s token, then each `[[http.user]]` with the
/// strongest of `role` and their `roles` (TODO 101), which [`within`] narrows per net.
fn roles(cfg: &config::Http) -> BTreeMap<String, Role> {
    let mut roles = BTreeMap::new();
    if !cfg.secret().is_empty() {
        roles.insert("default".to_owned(), Role::Run);
    }
    for u in &cfg.user {
        roles.insert(u.name.trim().to_owned(), u.access().top());
    }
    roles
}

/// The `[[http.user]]` narrowed with `nets` (TODO 94) or `roles` (TODO 101); the others have one
/// role on every net.
fn access(cfg: &config::Http) -> BTreeMap<String, Access> {
    cfg.user.iter().map(|u| (u.name.trim().to_owned(), u.access())).filter(|(_, a)| a.narrowed()).collect()
}

/// The tailnet listener (TODO 105): each `[[http.user]]`'s Tailscale login or tag is let in
/// as that person, so their role, `nets` and `roles` hold as with a token.
fn tailnet(cfg: &config::Http, port: u16) -> ito_http::Tailnet {
    cfg.user.iter().fold(ito_http::Tailnet::new(port), |t, u| match u.tailscale() {
        Some(tag) if tag.starts_with("tag:") => t.tag(tag, u.name.trim()),
        Some(login) => t.login(login, u.name.trim()),
        None => t,
    })
}

fn start(cfg: &config::Http) -> Result<Server, String> {
    let mut c = ito_http::Config::new(cfg.port, if cfg.secret().is_empty() { String::new() } else { token(cfg.secret())? });
    for u in cfg.user.iter().filter(|u| !u.secret().is_empty()) {
        c = c.token_named(u.name.trim(), token(u.secret())?);
    }
    c.tailnet = cfg.tailnet.map(|port| tailnet(cfg, port));
    for path in PAGES.iter().map(|(p, ..)| *p).chain([THEME_CSS]) {
        c = c.public_path(path);
    }
    c.sessions = true;
    c.ipv6 = cfg.ipv6;
    let at = if cfg.lan {
        let dir = cert_dir().ok_or("設定のフォルダーが分からない")?;
        c.bind = ito_http::Bind::Lan { addr: None, tls: ito_http::TlsCfg::new(dir) };
        format!("LAN :{}", cfg.port)
    } else {
        format!("127.0.0.1:{}", cfg.port)
    };
    let at = match cfg.tailnet {
        Some(port) => format!("{at}・tailnet :{port}"),
        None => at,
    };
    let (roles, access) = (roles(cfg), access(cfg));
    let serve = move |req: ito_http::Request| {
        let by = req.who.clone().unwrap_or_default();
        let ask = |r: &Request| ito_ipc::door::ask(&proto::door(), &Request::Over { by: by.clone(), request: Box::new(r.clone()) });
        page(&req).or_else(|| allowed(&req, &roles)).or_else(|| me(&req, &roles, &access)).unwrap_or_else(|| match access.get(&by) {
            Some(a) => within(&req, a, ask),
            None => handle(&req, ask),
        })
    };
    Server::start(c, serve, log).map_err(|e| format!("{at}: {e}"))
}

/// The Web UI's files, kept in the exe: path, content type, body.
const PAGES: [(&str, &str, &str); 3] = [
    ("/", "text/html; charset=utf-8", include_str!("../../web/index.html")),
    ("/app.js", "text/javascript; charset=utf-8", include_str!("../../web/app.js")),
    ("/app.css", "text/css; charset=utf-8", include_str!("../../web/app.css")),
];

/// The colours of the Web UI: every colour in `app.css` is a variable of this sheet.
const THEME_CSS: &str = "/theme.css";

/// One of the Web UI's files for a `GET`. Nothing but these files and the API: no other
/// origin, no inline script, no frames.
fn page(req: &ito_http::Request) -> Option<Response> {
    if !matches!(req.method.as_str(), "GET" | "HEAD") {
        return None;
    }
    let (kind, body) = match PAGES.iter().find(|(p, ..)| *p == req.path) {
        Some((_, kind, body)) => (*kind, body.to_string()),
        None if req.path == THEME_CSS => ("text/css; charset=utf-8", theme_css(ito_common::base_dir().as_deref())),
        None => return None,
    };
    Some(
        Response::bytes(200, kind, body)
            .with_header(
                "Content-Security-Policy",
                "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
            )
            .with_header("X-Content-Type-Options", "nosniff")
            .with_header("Referrer-Policy", "no-referrer")
            .with_header("Cache-Control", "no-store"),
    )
}

/// `/theme.css`: the theme common.toml under `base` picks, as the window picks it (the window's
/// own themes and the files in `themes/`), so the page has the window's colours. `system` follows
/// the browser's light or dark rather than this PC's. Read on each request: a theme changed in the
/// window shows at the next reload. A theme that is not found gives the first built-in one, as the
/// window does.
fn theme_css(base: Option<&std::path::Path>) -> String {
    let mut themes = ito_theme::builtin();
    let mut common = ito_common::Common::default();
    if let Some(b) = base {
        themes.extend(ito_theme::load_dir(&ito_common::themes_dir(b)).0);
        if let Ok((c, _)) = ito_common::Common::read(b) {
            common = c;
        }
    }
    let (choice, dark, light) = common.theme_choice(crate::settings::THEMES);
    let high = common.high_contrast_or_off();
    ito_theme::stylesheet(&themes, &choice, &dark, &light, high).unwrap_or_else(|_| {
        let c = if high { themes[0].colors.high_contrast() } else { themes[0].colors };
        format!(
            ":root {{
{}}}
",
            c.css_vars()
        )
    })
}

/// `GET /me`: who signed in, their role, the nets they are narrowed to (`null`: every net) and
/// their role per net (`roles`, `null`: `role` on every net), so the page shows only the buttons
/// it may use.
fn me(req: &ito_http::Request, roles: &BTreeMap<String, Role>, access: &BTreeMap<String, Access>) -> Option<Response> {
    if req.path.trim_end_matches('/') != "/me" {
        return None;
    }
    if req.method != "GET" {
        return Some(Response::error(405, "GET で読む"));
    }
    let who = req.who.as_deref().unwrap_or_default();
    let a = access.get(who);
    let role = a.map(|a| a.role).or_else(|| roles.get(who).copied()).map_or("view", Role::name);
    let nets = a.and_then(|a| a.nets.as_ref()).map(Nets::patterns);
    let per: Option<BTreeMap<&str, &str>> =
        a.filter(|a| !a.roles.is_empty()).map(|a| a.roles.iter().map(|(p, r)| (p.as_str(), r.name())).collect());
    Some(Response::ok(serde_json::json!({ "who": who, "role": role, "nets": nets, "roles": per })))
}

/// One request of a caller narrowed by `nets` (TODO 94) or `roles` (TODO 101): every question
/// to the engine goes through [`narrow`], and one it refuses makes the answer 403. `GET /nets`
/// also says their role on each net.
fn within(req: &ito_http::Request, access: &Access, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let needs = role::needs(&req.method, &req.path);
    let refused = RefCell::new(None);
    let narrowed = |r: &Request| {
        narrow(r, access, needs, &ask).unwrap_or_else(|e| {
            *refused.borrow_mut() = Some(e.clone());
            Err(e)
        })
    };
    let out = if req.method == "GET" && req.path.trim_end_matches('/') == "/nets" {
        nets(narrowed, |id| Some(access.role_on(id)))
    } else {
        handle(req, narrowed)
    };
    match refused.into_inner() {
        Some(e) => Response::error(403, &e),
        None => out,
    }
}

/// What a caller narrowed by `access` gets for `r`, sent for an HTTP request that needs `needs`:
/// the engine's answer cut to the nets they see, or `Err` when it is refused. Deny by default:
/// what is not named here is refused to a caller with `nets`, as is what is about every net. A
/// net they do not see, and a run of one, answer as ones that are not there, so nothing says they
/// exist; a proposal outside is refused, whether the net is there or not. On a net they see, their
/// role there must reach `needs`; what is no one net's needs it of `role`.
fn narrow(
    r: &Request,
    access: &Access,
    needs: Role,
    ask: &impl Fn(&Request) -> Result<String, String>,
) -> Result<Result<String, String>, String> {
    let may = |id: &str| {
        let has = access.role_on(id);
        if has.allows(needs) {
            Ok(())
        } else {
            Err(format!("{id} では {has} のロール。これには {needs} が要る"))
        }
    };
    let all = access.nets.is_none();
    let sees = |id: &str| access.sees(id);
    Ok(match r {
        Request::Status | Request::List | Request::Runs | Request::History { .. } if all => ask(r),
        Request::Status => ask(r).map(|s| s.lines().next().and_then(|l| l.split(':').next()).unwrap_or_default().to_owned()),
        Request::Handover { release: false } => ask(r),
        Request::List => ask(r).and_then(|t| Snapshot::parse(&t)).map(|s| {
            Snapshot {
                defs: s.defs.into_iter().filter(|d| sees(&d.id)).collect(),
                held: s.held.into_iter().filter(|id| sees(id)).collect(),
                held_jobs: s.held_jobs.into_iter().filter(|h| sees(&h.net)).collect(),
                skips: s.skips.into_iter().filter(|k| sees(&k.id)).collect(),
                paused: s.paused,
                // The planned stops are the whole PC's, not a net's: the times they stop show as such.
                blackouts: s.blackouts,
                ..Snapshot::default()
            }
            .to_toml()
        }),
        Request::Runs | Request::History { .. } => ask(r).and_then(|t| Runs::parse(&t)).map(|mut runs| {
            runs.runs.retain(|run| sees(&run.jobnet));
            runs.to_toml()
        }),
        Request::Run { id, .. }
        | Request::Show { id }
        | Request::Hold { id, .. }
        | Request::Release { id, .. }
        | Request::SkipNext { id, .. } => {
            if sees(id) {
                may(id)?;
                ask(r)
            } else {
                Err(format!("定義がない: {id}"))
            }
        }
        Request::Propose { id, .. } if sees(id) => {
            may(id)?;
            ask(r)
        }
        Request::Propose { id, .. } => {
            let within = access.nets.as_ref().map(|n| n.patterns().join("・")).unwrap_or_default();
            return Err(format!("nets の外なので出せない: {id}（出せるのは {within}）"));
        }
        Request::Kill { run, .. } | Request::Rerun { run, .. } | Request::Skip { run, .. } | Request::Log { run, .. } => {
            match run_net(run, ask) {
                Some(net) if sees(&net) => {
                    may(&net)?;
                    ask(r)
                }
                _ => Err(format!("実行がない: {run}")),
            }
        }
        _ if !all => return Err("nets で絞った利用者にはできない（全部のネットにかかる）".to_owned()),
        _ if access.role.allows(needs) => ask(r),
        _ => return Err(format!("ネットごとでないものは {} のロール。これには {needs} が要る", access.role)),
    })
}

/// The net of the run `id`: one of today or still going, then one of the day its id names.
fn run_net(id: &str, ask: &impl Fn(&Request) -> Result<String, String>) -> Option<String> {
    let find = |r: &Request| Runs::parse(&ask(r).ok()?).ok()?.runs.into_iter().find(|x| x.id == id).map(|x| x.jobnet);
    find(&Request::Runs).or_else(|| {
        let day = started_on(id)?;
        find(&Request::History { from: day.clone(), to: day })
    })
}

/// A refusal when the caller's role does not reach what the request needs; `None` lets it through.
fn allowed(req: &ito_http::Request, roles: &BTreeMap<String, Role>) -> Option<Response> {
    let Some(who) = req.who.as_deref() else {
        return Some(Response::error(401, "token が要る"));
    };
    let has = roles.get(who).copied();
    let needs = role::needs(&req.method, &req.path);
    match has {
        Some(has) if has.allows(needs) => None,
        Some(has) => Some(Response::error(403, &format!("{who} は {has} のロール。これには {needs} が要る"))),
        None => Some(Response::error(403, &format!("{who} は [[http.user]] にない"))),
    }
}

fn log(l: &ito_http::LogLine) {
    let by = match (l.via, &l.who) {
        (ito_http::Via::Tailscale, Some(who)) => format!("（{who}、Tailscale）"),
        _ => String::new(),
    };
    crate::say(&format!("http: {} {} {} {}ms{by}", l.method, l.path, l.status, l.millis), false);
}

/// What `POST /runs` takes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    net: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
}

/// What `POST /runs/<id>/rerun` takes: nothing (from the first job not passed), `from`, or `next`
/// (that job ends as ⊘, or as △ with `warn`, and the ones after it go), as `koyomi rerun` does.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Again {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    warn: bool,
}

/// What `POST /runs/<id>/skip` takes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pass {
    job: String,
}

/// What `POST /nets/hold` and `POST /nets/release` take: the net, and a job of it to hold alone.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hold {
    net: String,
    #[serde(default)]
    job: Option<String>,
}

/// What `POST /nets/skip-next` takes: the net, and `cancel` to take the skip back.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkipNext {
    net: String,
    #[serde(default)]
    cancel: bool,
}

/// What `POST /nets/propose` takes: the net, its whole text, and the hash `GET /nets/text` gave
/// (none for a new net).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    net: String,
    text: String,
    #[serde(default)]
    base: Option<String>,
}

/// The run id of `/runs/<id>/<what>`.
fn run_part<'a>(path: &'a str, what: &str) -> Option<&'a str> {
    path.strip_prefix("/runs/")?.strip_suffix(what)?.strip_suffix('/').filter(|id| !id.is_empty() && !id.contains('/'))
}

/// One request (the token already checked), asking the engine through `ask`.
fn handle(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let path = req.path.trim_end_matches('/');
    let one = path.strip_prefix("/runs/").filter(|id| !id.is_empty() && !id.contains('/'));
    let log = run_part(path, "log");
    let act = ["stop", "rerun", "skip"].into_iter().find_map(|w| run_part(path, w).map(|id| (w, id)));
    match (req.method.as_str(), path, one, log) {
        ("POST", "/runs", ..) => start_run(req, ask),
        ("POST", "/nets/hold", ..) => hold(req, true, ask),
        ("POST", "/nets/release", ..) => hold(req, false, ask),
        ("POST", "/nets/skip-next", ..) => skip_next(req, ask),
        ("POST", "/nets/propose", ..) => propose(req, ask),
        ("GET", "/nets/text", ..) => def_text(req, ask),
        ("POST", "/standby/release", ..) => release_standby(ask),
        ("POST", ..) if act.is_some() => match act.unwrap_or_default() {
            ("stop", id) => stop_run(id, ask),
            ("rerun", id) => rerun(req, id, ask),
            (_, id) => skip(req, id, ask),
        },
        ("GET", "/runs", ..) => list_runs(req, ask),
        ("GET", "/health", ..) => health(ask),
        ("GET", "/nets", ..) => nets(ask, |_| None),
        ("GET", _, Some(id), _) => look(id, ask),
        ("GET", _, _, Some(id)) => job_log(req, id, ask),
        (_, "/runs", ..) => Response::error(405, "POST /runs で流す・GET /runs で読む"),
        (_, "/health" | "/nets", ..) | (_, _, Some(_), _) | (_, _, _, Some(_)) => Response::error(405, "GET で読む"),
        (_, "/nets/hold" | "/nets/release", ..) => Response::error(405, "POST で保留にする・外す"),
        (_, "/nets/skip-next", ..) => Response::error(405, "POST で次の回を飛ばす・取り消す"),
        (_, "/nets/propose", ..) => Response::error(405, "POST で定義を出す"),
        (_, "/nets/text", ..) => Response::error(405, "GET で読む"),
        (_, "/standby/release", ..) => Response::error(405, "POST で引き取る"),
        _ if act.is_some() => Response::error(405, "POST で止める・流し直す・飛ばす"),
        _ => Response::error(
            404,
            "POST /runs・GET /runs・GET /runs/<id>・GET /runs/<id>/log・POST /runs/<id>/stop|rerun|skip・GET /nets・GET /nets/text・POST /nets/hold|release|skip-next|propose・POST /standby/release・GET /health だけ",
        ),
    }
}

/// `GET /health`: the engine answers, with its status line. What an HA standby polls; with
/// `[standby]` or `[primary]`, `"ha"` says which and whether it has the schedules (TODO 96).
fn health(ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    match ask(&Request::Status) {
        Ok(line) => {
            let mut out = serde_json::json!({
                "ok": true,
                "version": env!("CARGO_PKG_VERSION"),
                "proto": proto::PROTO,
                "status": line.trim(),
            });
            let ha = ask(&Request::Handover { release: false }).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
            if let Some(ha) = ha {
                out["ha"] = ha;
            }
            Response::ok(out)
        }
        Err(e) => Response::error(503, &e),
    }
}

/// `POST /standby/release`: the primary takes its schedules back. 200 once this standby let
/// go, 409 with why not (not ready yet, or a time is near).
fn release_standby(ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    match ask(&Request::Handover { release: true }).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string())) {
        Ok(mut v) => {
            v["released"] = serde_json::Value::Bool(true);
            Response::ok(v)
        }
        Err(e) => Response::error(409, &e),
    }
}

/// `GET /nets`: each definition's id, name, standing, hold, `[params]` (name and default, so a
/// caller can ask for the values before `POST /runs`) and the hash of its file, so a standby
/// fetches only what changed (TODO 87). Not the text: that is `GET /nets/text`, for `edit`.
/// With them, as the window's 今日 counts them: `today`, the starts still to come today, and
/// `next`, the next one (none for a net not approved or on hold), `then`, the one after the last
/// of `today` (each start's 次の回); `jobs`, each job's `id`, `name` and whether it is `held`
/// (`POST /nets/hold` with `job`); and `skips`, the times `koyomi skip-next` skips that have not come.
/// For a caller with a role per net, `role`: theirs on that net (`role_of`, TODO 101).
fn nets(ask: impl Fn(&Request) -> Result<String, String>, role_of: impl Fn(&str) -> Option<Role>) -> Response {
    match ask(&Request::List).and_then(|t| Snapshot::parse(&t)) {
        Ok(snap) => Response::ok(nets_at(&snap, chrono::Local::now().naive_local(), role_of)),
        Err(e) => Response::error(500, &e),
    }
}

/// What `GET /nets` answers at `now`.
fn nets_at(snap: &Snapshot, now: chrono::NaiveDateTime, role_of: impl Fn(&str) -> Option<Role>) -> serde_json::Value {
    let mut today: BTreeMap<&str, Vec<chrono::NaiveDateTime>> = BTreeMap::new();
    for (d, at) in snap.starts_today(now) {
        today.entry(d.id.as_str()).or_default().push(at);
    }
    let nets: Vec<_> = snap
        .defs
        .iter()
        .map(|d| {
            let mut starts = today.remove(d.id.as_str()).unwrap_or_default();
            starts.sort();
            let then = snap.next_start(d, starts.last().copied().unwrap_or(now));
            let mut net = serde_json::json!({
                "id": d.id,
                "group": d.group,
                "name": d.label(),
                "standing": d.standing,
                "sha256": d.sha256,
                "held": snap.held.contains(&d.id),
                "jobs": d.net.as_ref().map(|n| n.jobs.iter().map(|j| serde_json::json!({
                    "id": j.id,
                    "name": j.name,
                    "held": snap.held_jobs.iter().any(|h| h.net == d.id && h.job == j.id),
                })).collect::<Vec<_>>()).unwrap_or_default(),
                "params": d.net.as_ref().map(|n| n.params.clone()).unwrap_or_default(),
                "today": starts,
                "next": snap.next_start(d, now),
                "then": then,
                "skips": snap.skips.iter().filter(|s| s.id == d.id && s.at > now).map(|s| s.at).collect::<Vec<_>>(),
            });
            if let Some(r) = role_of(&d.id) {
                net["role"] = r.name().into();
            }
            net
        })
        .collect();
    serde_json::json!({ "nets": nets, "paused": snap.paused, "warnings": snap.warnings })
}

/// `GET /runs`: today's and the running ones, or with `from` and `to` (`YYYY-MM-DD`) the days'.
/// With `running=1`, only the ones not over: what an HA standby reads to take them over.
fn list_runs(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let r = match (req.query("from"), req.query("to")) {
        (None, None) => Request::Runs,
        (Some(from), to) => Request::History { from: from.to_owned(), to: to.unwrap_or(from).to_owned() },
        (None, Some(_)) => return Response::error(400, "to には from も要る"),
    };
    let days = matches!(r, Request::History { .. });
    match ask(&r).and_then(|t| Runs::parse(&t)) {
        Ok(runs) => {
            let running = req.query("running") == Some("1");
            let runs: Vec<_> = runs.runs.iter().filter(|r| !running || !r.is_over()).map(run_json).collect();
            Response::ok(serde_json::json!({ "runs": runs }))
        }
        Err(e) if days => Response::error(400, &e),
        Err(e) => Response::error(500, &e),
    }
}

/// `GET /runs/<id>/log?job=<job>`: the tail of the job's log, as `koyomi log` shows it.
fn job_log(req: &ito_http::Request, run: &str, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let Some(job) = req.query("job").filter(|j| !j.is_empty()) else {
        return Response::error(400, "job が要る: /runs/<id>/log?job=<ジョブ>");
    };
    match ask(&Request::Log { run: run.to_owned(), job: job.to_owned() }) {
        Ok(text) => Response::ok(serde_json::json!({ "run": run, "job": job, "log": text })),
        Err(e) => Response::error(404, &e),
    }
}

/// A run as JSON: `koyomi status`'s fields and `"over"`.
fn run_json(run: &Run) -> serde_json::Value {
    let mut v = serde_json::to_value(run).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("over".into(), run.is_over().into());
    }
    v
}

fn start_run(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let s = match req.json::<Start>() {
        Ok(Some(s)) if !s.net.trim().is_empty() => s,
        Ok(_) => return Response::error(400, "net が要る: {\"net\": \"<グループ>/<名前>\"}"),
        Err(e) => return Response::error(400, &format!("JSON を読めない: {e}")),
    };
    let r = Request::Run { id: s.net.trim().to_owned(), date: s.date, from: s.from, dry: false, params: s.params };
    match ask(&r) {
        Ok(id) => Response::new(202, serde_json::json!({ "run": id.trim() })),
        Err(e) => Response::error(422, &e),
    }
}

/// `POST /runs/<id>/stop`: stops the run's jobs, as `koyomi kill <id>` does.
fn stop_run(run: &str, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    match ask(&Request::Kill { run: run.to_owned(), job: None }) {
        Ok(text) => Response::ok(serde_json::json!({ "run": run, "said": text.trim() })),
        Err(e) => Response::error(422, &e),
    }
}

/// `POST /runs/<id>/rerun`: starts the run again, as `koyomi rerun` does. 202 with the new run's id.
fn rerun(req: &ito_http::Request, run: &str, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let a = match req.json::<Again>() {
        Ok(a) => a.unwrap_or_default(),
        Err(e) => return Response::error(400, &format!("JSON を読めない: {e}")),
    };
    let named = |j: Option<String>| j.map(|j| j.trim().to_owned()).filter(|j| !j.is_empty());
    let r = match (named(a.from), named(a.next)) {
        (Some(_), Some(_)) => return Response::error(400, "from と next はどちらか 1 つ"),
        (_, None) if a.warn => return Response::error(400, "warn は next と書く"),
        (from, None) => Request::Rerun { run: run.to_owned(), from, next: None },
        (None, next) => Request::Rerun { run: run.to_owned(), from: next, next: Some(if a.warn { State::Warn } else { State::Skipped }) },
    };
    match ask(&r) {
        Ok(id) => Response::new(202, serde_json::json!({ "run": id.trim() })),
        Err(e) => Response::error(422, &e),
    }
}

/// `POST /runs/<id>/skip`: skips a job before it starts (⊘), as `koyomi skip` does.
fn skip(req: &ito_http::Request, run: &str, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let job = match req.json::<Pass>() {
        Ok(Some(p)) if !p.job.trim().is_empty() => p.job.trim().to_owned(),
        Ok(_) => return Response::error(400, "job が要る: {\"job\": \"<ジョブ>\"}"),
        Err(e) => return Response::error(400, &format!("JSON を読めない: {e}")),
    };
    match ask(&Request::Skip { run: run.to_owned(), job: job.clone() }) {
        Ok(text) => Response::ok(serde_json::json!({ "run": run, "job": job, "said": text.trim() })),
        Err(e) => Response::error(422, &e),
    }
}

/// `POST /nets/hold` and `/nets/release`: puts the net (or one job of it) on hold or takes it
/// off, as `koyomi hold` and `koyomi release` do.
fn hold(req: &ito_http::Request, on: bool, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let h = match req.json::<Hold>() {
        Ok(Some(h)) if !h.net.trim().is_empty() => h,
        Ok(_) => return Response::error(400, "net が要る: {\"net\": \"<グループ>/<名前>\"}"),
        Err(e) => return Response::error(400, &format!("JSON を読めない: {e}")),
    };
    let (id, job) = (h.net.trim().to_owned(), h.job.map(|j| j.trim().to_owned()).filter(|j| !j.is_empty()));
    let r = if on { Request::Hold { id: id.clone(), job: job.clone() } } else { Request::Release { id: id.clone(), job: job.clone() } };
    match ask(&r) {
        Ok(text) => Response::ok(serde_json::json!({ "net": id, "job": job, "held": on, "said": text.trim() })),
        Err(e) => Response::error(422, &e),
    }
}

/// `POST /nets/skip-next`: skips the net's next scheduled run (or, with `cancel`, takes the skip
/// back), as `koyomi skip-next` does.
fn skip_next(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let s = match req.json::<SkipNext>() {
        Ok(Some(s)) if !s.net.trim().is_empty() => s,
        Ok(_) => return Response::error(400, "net が要る: {\"net\": \"<グループ>/<名前>\"}"),
        Err(e) => return Response::error(400, &format!("JSON を読めない: {e}")),
    };
    let id = s.net.trim().to_owned();
    match ask(&Request::SkipNext { id: id.clone(), cancel: s.cancel }) {
        Ok(text) => Response::ok(serde_json::json!({ "net": id, "skip": !s.cancel, "said": text.trim() })),
        Err(e) => Response::error(422, &e),
    }
}

/// `GET /nets/text?net=<group>/<name>`: the definition's file as it is, with the hash to send
/// back as `base` and whether it is confirmed.
fn def_text(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let Some(id) = req.query("net").map(str::trim).filter(|n| !n.is_empty()) else {
        return Response::error(400, "net が要る: /nets/text?net=<グループ>/<名前>");
    };
    let def = match ask(&Request::Show { id: id.to_owned() }) {
        Ok(t) => toml::from_str::<koyomi_core::scan::Def>(&t).map_err(|e| e.to_string()),
        Err(e) => return Response::error(404, &e),
    };
    match def {
        Ok(d) => Response::ok(serde_json::json!({
            "net": d.id,
            "text": d.text,
            "sha256": d.sha256,
            "standing": d.standing,
        })),
        Err(e) => Response::error(500, &e),
    }
}

/// `POST /nets/propose`: writes the definition but leaves it unconfirmed (TODO 83). 409 when the
/// file changed since `base`, 422 when it cannot be written as it is.
fn propose(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let p = match req.json::<Proposal>() {
        Ok(Some(p)) if !p.net.trim().is_empty() && !p.text.trim().is_empty() => p,
        Ok(_) => {
            return Response::error(400, "net と text が要る: {\"net\": \"<グループ>/<名前>\", \"text\": \"…\", \"base\"?: \"<sha256>\"}")
        }
        Err(e) => return Response::error(400, &format!("JSON を読めない: {e}")),
    };
    let id = p.net.trim().to_owned();
    let base = p.base.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty());
    match ask(&Request::Propose { id: id.clone(), text: p.text, base_sha256: base }) {
        Ok(text) => Response::ok(serde_json::json!({ "net": id, "confirmed": false, "said": text.trim() })),
        Err(e) if e.starts_with("ほかで変わっています") => Response::error(409, &e),
        Err(e) => Response::error(422, &e),
    }
}

/// A run of today or still going, then one of the day its id says it started.
fn look(id: &str, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let Some(day) = started_on(id) else {
        return Response::error(404, &format!("実行の id は YYYYMMDD-HHMMSS-NN: {id}"));
    };
    let find = |req: &Request| -> Result<Option<Run>, String> { Ok(Runs::parse(&ask(req)?)?.runs.into_iter().find(|r| r.id == id)) };
    let found = find(&Request::Runs).and_then(|r| match r {
        Some(r) => Ok(Some(r)),
        None => find(&Request::History { from: day.clone(), to: day }),
    });
    match found {
        Ok(Some(run)) => Response::ok(run_json(&run)),
        Ok(None) => Response::error(404, &format!("その実行はない: {id}")),
        Err(e) => Response::error(500, &e),
    }
}

/// `YYYY-MM-DD` from a run id's `YYYYMMDD-…`.
fn started_on(id: &str) -> Option<String> {
    let d = id.get(..8).filter(|d| d.bytes().all(|b| b.is_ascii_digit()) && id[8..].starts_with('-'))?;
    chrono::NaiveDate::parse_from_str(d, "%Y%m%d").ok().map(|d| d.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::{Read, Write};

    fn req(method: &str, path: &str, body: &str) -> ito_http::Request {
        ito_http::Request {
            who: Some("default".into()),
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn runs(ids: &[&str]) -> String {
        runs_of(&ids.iter().map(|id| (*id, "g/a")).collect::<Vec<_>>())
    }

    /// Runs as the engine answers them, each `(id, net)`.
    fn runs_of(runs: &[(&str, &str)]) -> String {
        let text: String = runs
            .iter()
            .map(|(id, net)| {
                format!(
                    "[[run]]\nid = \"{id}\"\njobnet = \"{net}\"\ntitle = \"a\"\ndate = \"2026-10-31\"\nhow = \"manual\"\nstarted = \"2026-10-31T09:00:00\"\nended = \"2026-10-31T09:01:00\"\nstate = \"ok\"\njobs = []\n"
                )
            })
            .collect();
        Runs::parse(&text).expect("runs").to_toml()
    }

    fn def(id: &str) -> koyomi_core::scan::Def {
        let (group, name) = id.split_once('/').expect("group/name");
        koyomi_core::scan::Def {
            id: id.into(),
            group: group.into(),
            name: name.into(),
            title: None,
            file: format!("{id}.toml"),
            standing: koyomi_core::scan::Standing::Confirmed,
            problems: Vec::new(),
            net: None,
            sha256: "abc".into(),
            text: "format = 1\n".into(),
            previous: None,
            calendar: None,
        }
    }

    /// `nets` narrows a user (TODO 94): the nets outside and their runs are as if they were not
    /// there and never reach the engine, a proposal outside is refused, and so is whatever this
    /// does not name.
    #[test]
    fn a_user_narrowed_to_some_nets_reaches_only_those() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::Status => Ok("koyomi engine pid 7: 定義 2 件（確認済み 2）、動いている実行 1\n保留中: 人事/b\n".to_owned()),
                Request::List => Ok(Snapshot {
                    defs: vec![def("経理/a"), def("人事/b")],
                    held: vec!["経理/a".into(), "人事/b".into()],
                    warnings: vec!["jobnets/人事/b.toml は x が書き換えられる".into()],
                    ..Default::default()
                }
                .to_toml()),
                Request::Runs => Ok(runs_of(&[("20261031-090000-01", "経理/a"), ("20261031-090000-02", "人事/b")])),
                Request::History { .. } => Ok(runs_of(&[("20261030-230000-01", "人事/b")])),
                Request::Show { id } => Ok(toml::to_string(&def(id)).expect("def")),
                Request::Run { .. } | Request::Rerun { .. } => Ok("20261031-100000-01".to_owned()),
                _ => Ok("ok".to_owned()),
            }
        };
        let nets = Access { role: Role::Edit, nets: Some(Nets::parse(&["経理/*".to_owned()]).unwrap()), roles: Vec::new() };
        let go = |method: &str, path: &str, body: &str, q: &[(&str, &str)]| within(&with_query(req(method, path, body), q), &nets, ask);

        let n = go("GET", "/nets", "", &[]);
        assert_eq!((n.status, n.json["nets"].as_array().map(Vec::len), n.json["nets"][0]["id"].as_str()), (200, Some(1), Some("経理/a")));
        assert_eq!(n.json["nets"][0]["role"].as_str(), Some("edit"));
        assert_eq!((n.json["nets"][0]["held"].as_bool(), n.json["warnings"].as_array().map(Vec::len)), (Some(true), Some(0)));
        assert_eq!(go("GET", "/health", "", &[]).json["status"].as_str(), Some("koyomi engine pid 7"));
        let r = go("GET", "/runs", "", &[]);
        assert_eq!((r.json["runs"].as_array().map(Vec::len), r.json["runs"][0]["jobnet"].as_str()), (Some(1), Some("経理/a")));
        assert_eq!(go("GET", "/runs", "", &[("from", "2026-10-30")]).json["runs"].as_array().map(Vec::len), Some(0));
        assert_eq!(go("GET", "/runs/20261031-090000-01", "", &[]).status, 200);
        assert_eq!(go("GET", "/runs/20261031-090000-02", "", &[]).status, 404);
        assert_eq!(go("GET", "/runs/20261030-230000-01", "", &[]).status, 404);
        assert_eq!(go("GET", "/runs/20261031-090000-02/log", "", &[("job", "x")]).status, 404);
        assert_eq!(go("GET", "/runs/20261031-090000-01/log", "", &[("job", "x")]).status, 200);

        // A run or a net outside answers as one that is not there.
        assert_eq!(go("POST", "/runs/20261031-090000-02/stop", "", &[]).status, 422);
        assert_eq!(go("POST", "/runs/20261030-230000-01/rerun", "", &[]).status, 422);
        assert_eq!(go("POST", "/runs/20261031-090000-02/skip", r#"{"job": "x"}"#, &[]).status, 422);
        assert_eq!(go("POST", "/runs", r#"{"net": "人事/b"}"#, &[]).status, 422);
        assert_eq!(go("POST", "/nets/hold", r#"{"net": "人事/b"}"#, &[]).status, 422);
        assert_eq!(go("POST", "/nets/release", r#"{"net": "人事/b"}"#, &[]).status, 422);
        assert_eq!(go("POST", "/nets/skip-next", r#"{"net": "人事/b"}"#, &[]).status, 422);
        assert_eq!(go("GET", "/nets/text", "", &[("net", "人事/b")]).status, 404);
        // A proposal outside is refused, whether that net is there or not.
        assert_eq!(go("POST", "/nets/propose", r#"{"net": "人事/b", "text": "x"}"#, &[]).status, 403);
        assert_eq!(go("POST", "/nets/propose", r#"{"net": "人事/new", "text": "x"}"#, &[]).status, 403);
        let outside = |a: &String| a.contains("人事") || a.contains("-02\"") || a.contains("20261030-230000-01");
        let touched: Vec<_> = asked.borrow().iter().filter(|a| outside(a)).cloned().collect();
        assert!(touched.is_empty(), "{touched:?}");

        // Inside, as before.
        assert_eq!(go("POST", "/runs/20261031-090000-01/stop", "", &[]).status, 200);
        assert_eq!(go("POST", "/runs", r#"{"net": "経理/a"}"#, &[]).status, 202);
        assert_eq!(go("POST", "/nets/hold", r#"{"net": "経理/a"}"#, &[]).status, 200);
        assert_eq!(go("GET", "/nets/text", "", &[("net", "経理/a")]).status, 200);
        assert_eq!(go("POST", "/nets/propose", r#"{"net": "経理/new", "text": "x"}"#, &[]).status, 200);
        assert!(asked.borrow().iter().any(|a| a.starts_with("Propose") && a.contains("経理/new")));

        // What this does not name, or what is about every net, is refused.
        for r in [Request::Pause, Request::Stop, Request::Delete { id: "経理/a".into() }, Request::Ops { from: "a".into(), to: "b".into() }]
        {
            assert!(narrow(&r, &nets, Role::Edit, &ask).is_err(), "{r:?}");
        }
        // With no `nets`, every net, as before, and no role per net.
        let all = handle(&req("GET", "/nets", ""), ask);
        assert_eq!((all.json["nets"].as_array().map(Vec::len), all.json["nets"][0].get("role")), (Some(2), None));
        assert_eq!(handle(&req("POST", "/nets/propose", r#"{"net": "人事/new", "text": "x"}"#), ask).status, 200);
    }

    /// On the tailnet a login or a tag is the person who wrote it, so their role and `nets` hold.
    #[test]
    fn a_tailscale_login_or_tag_is_let_in_as_its_person() {
        use ito_http::tailnet::Identity;
        let cfg = config::Config::parse(
            "[http]\nport = 8790\ntailnet = 8791\n[[http.user]]\nname = \"sato\"\nrole = \"view\"\ntailscale = \"sato@example.com\"\nnets = [\"経理/*\"]\n[[http.user]]\nname = \"ha\"\nrole = \"run\"\ntailscale = \"tag:koyomi-ha\"\ntoken = \"secret:ha\"\n[[http.user]]\nname = \"ito\"\nrole = \"edit\"\ntoken = \"secret:i\"\n",
        )
        .unwrap();
        let http = cfg.http.as_ref().unwrap();
        let t = tailnet(http, 8791);
        assert_eq!((t.port, t.logins.len(), t.tags.len()), (8791, 1, 1));
        let id = |login: Option<&str>, tags: &[&str]| Identity {
            login: login.map(str::to_owned),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            node: "pc.tail1234.ts.net".into(),
        };
        assert_eq!(t.who(&id(Some("SATO@example.com"), &[])).as_deref(), Some("sato"));
        assert_eq!(t.who(&id(None, &["tag:koyomi-ha"])).as_deref(), Some("ha"));
        // A tagged node is judged by its tags, an unknown login is nobody.
        assert_eq!(t.who(&id(Some("sato@example.com"), &["tag:other"])), None);
        assert_eq!(t.who(&id(Some("ito@example.com"), &[])), None);
        // The names are the ones the roles and `nets` are kept under.
        let (known, narrowed) = (roles(http), access(http));
        assert_eq!((known["sato"], known["ha"], known.get("default")), (Role::View, Role::Run, None));
        assert!(narrowed["sato"].sees("経理/a") && !narrowed["sato"].sees("人事/b"));
    }

    /// Tailscale being down leaves the other listeners open; the person is told why the tailnet
    /// port is not.
    #[test]
    fn a_tailnet_port_that_cannot_open_yet_is_a_note() {
        struct Down;
        impl ito_http::tailnet::Whois for Down {
            fn whois(&self, _: std::net::SocketAddr) -> std::io::Result<Option<ito_http::tailnet::Identity>> {
                Ok(None)
            }
            fn status(&self) -> std::io::Result<ito_http::tailnet::Status> {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "tailscaled is not running"))
            }
        }
        let cfg = config::Config::parse(
            "[http]\nport = 8790\ntailnet = 8791\n[[http.user]]\nname = \"sato\"\nrole = \"view\"\ntailscale = \"sato@example.com\"\n",
        )
        .unwrap();
        let http = cfg.http.as_ref().unwrap();
        let mut t = tailnet(http, 0);
        t.whois = std::sync::Arc::new(Down);
        let mut c = ito_http::Config::new(0, "tok");
        c.tailnet = Some(t);
        let s = Server::start(c, |_| ito_http::Response::ok(serde_json::Value::Null), |_| {}).unwrap();
        assert_eq!(s.tailnet_port(), None);
        let note = tailnet_note(Some(http), &s).unwrap();
        assert!(note.contains(":8791 がまだ開けない（") && note.contains("tailscaled is not running"), "{note}");
        assert!(listening(Some(http), &s).ends_with("tailnet の :8791 はまだ開けない"));
        // Without `tailnet` there is nothing to say.
        assert_eq!(tailnet_note(None, &s), None);
        s.stop();
    }

    /// only the users with `nets` or `roles` are narrowed, and `/me` says to what.
    #[test]
    fn the_narrowed_users_are_those_with_nets() {
        let cfg = config::Config::parse(
            "[http]\nport = 8790\ntoken = \"secret:t\"\n[[http.user]]\nname = \"sato\"\nrole = \"edit\"\ntoken = \"secret:s\"\nnets = [\"経理/*\", \"共通/日次\"]\n[[http.user]]\nname = \"ito\"\nrole = \"view\"\ntoken = \"secret:i\"\n[[http.user]]\nname = \"kato\"\nrole = \"view\"\ntoken = \"secret:k\"\nroles = { \"経理/*\" = \"run\" }\n",
        )
        .unwrap();
        let http = cfg.http.as_ref().unwrap();
        let (known, narrowed) = (roles(http), access(http));
        assert_eq!(narrowed.keys().collect::<Vec<_>>(), ["kato", "sato"]);
        assert!(narrowed["sato"].sees("共通/日次") && !narrowed["sato"].sees("共通/週次"));
        // The gate lets kato through to what run reaches; the net decides the rest.
        assert_eq!((known["kato"], known["ito"]), (Role::Run, Role::View));
        let as_who = |who: &str| {
            let mut r = req("GET", "/me", "");
            r.who = Some(who.into());
            me(&r, &known, &narrowed).expect("me").json
        };
        assert_eq!(as_who("sato")["nets"], serde_json::json!(["経理/*", "共通/日次"]));
        assert_eq!((as_who("sato")["role"].as_str(), as_who("ito")["nets"].is_null()), (Some("edit"), true));
        assert_eq!((as_who("sato")["roles"].is_null(), as_who("ito")["roles"].is_null()), (true, true));
        let kato = as_who("kato");
        assert_eq!(
            (kato["role"].as_str(), kato["nets"].is_null(), &kato["roles"]),
            (Some("view"), true, &serde_json::json!({ "経理/*": "run" }))
        );
    }

    /// `roles` gives a user a role per net (TODO 101): what a request does to a net needs that
    /// net's role, a net no pattern names keeps `role`, and so does what is no one net's.
    #[test]
    fn a_user_has_a_role_per_net() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::List => Ok(Snapshot { defs: vec![def("経理/a"), def("経理/b"), def("人事/b")], ..Default::default() }.to_toml()),
                Request::Runs => Ok(runs_of(&[("20261031-090000-01", "経理/a"), ("20261031-090000-02", "人事/b")])),
                Request::History { .. } => Ok(runs_of(&[])),
                Request::Show { id } => Ok(toml::to_string(&def(id)).expect("def")),
                Request::Run { .. } => Ok("20261031-100000-01".to_owned()),
                Request::Handover { .. } => Ok(r#"{"role": "standby"}"#.to_owned()),
                _ => Ok("ok".to_owned()),
            }
        };
        let per = |base: Role, nets: Option<&[&str]>, roles: &[(&str, &str)]| Access {
            role: base,
            nets: nets.map(|n| Nets::parse(&n.iter().map(|p| p.to_string()).collect::<Vec<_>>()).unwrap()),
            roles: role::per_net(&roles.iter().map(|&(p, r)| (p.to_owned(), r.to_owned())).collect()).unwrap(),
        };
        let go =
            |a: &Access, method: &str, path: &str, body: &str, q: &[(&str, &str)]| within(&with_query(req(method, path, body), q), a, ask);

        // sato: view, but run on 経理.
        let sato = per(Role::View, None, &[("経理/*", "run")]);
        assert_eq!(go(&sato, "POST", "/runs", r#"{"net": "経理/a"}"#, &[]).status, 202);
        let r = go(&sato, "POST", "/runs", r#"{"net": "人事/b"}"#, &[]);
        assert_eq!(r.status, 403, "{:?}", r.json);
        assert!(r.json["error"].as_str().unwrap_or_default().contains("人事/b では view のロール"), "{:?}", r.json);
        assert_eq!(go(&sato, "GET", "/nets/text", "", &[("net", "経理/a")]).status, 403);
        assert_eq!(go(&sato, "POST", "/nets/hold", r#"{"net": "人事/b"}"#, &[]).status, 403);
        assert_eq!(go(&sato, "POST", "/runs/20261031-090000-02/stop", "", &[]).status, 403);
        assert_eq!(go(&sato, "POST", "/runs/20261031-090000-01/stop", "", &[]).status, 200);
        assert_eq!(go(&sato, "GET", "/runs/20261031-090000-02/log", "", &[("job", "x")]).status, 200);
        assert_eq!(go(&sato, "GET", "/runs", "", &[]).json["runs"].as_array().map(Vec::len), Some(2));
        let n = go(&sato, "GET", "/nets", "", &[]);
        let role_of = |i: usize| n.json["nets"][i]["role"].as_str().map(str::to_owned);
        assert_eq!(n.json["nets"].as_array().map(Vec::len), Some(3));
        assert_eq!((role_of(0), role_of(2)), (Some("run".into()), Some("view".into())), "{:?}", n.json);
        assert_eq!(go(&sato, "POST", "/standby/release", "{}", &[]).status, 403);
        // What was refused never reached the engine.
        let sent = |a: &String| {
            ((a.starts_with("Run {") || a.starts_with("Hold {")) && a.contains("人事")) || (a.starts_with("Kill {") && a.contains("-02\""))
        };
        assert!(!asked.borrow().iter().any(sent), "{:?}", asked.borrow());

        // ito: run, but only view on 人事; what is no one net's goes by run.
        let ito = per(Role::Run, None, &[("人事/*", "view")]);
        assert_eq!(go(&ito, "POST", "/runs", r#"{"net": "経理/b"}"#, &[]).status, 202);
        assert_eq!(go(&ito, "POST", "/runs", r#"{"net": "人事/b"}"#, &[]).status, 403);
        assert_eq!(go(&ito, "POST", "/standby/release", "{}", &[]).status, 200);
        assert_eq!(go(&ito, "GET", "/health", "", &[]).status, 200);

        // Both: a net they do not see is not there, whatever its role.
        let both = per(Role::View, Some(&["経理/*"]), &[("経理/a", "run")]);
        assert_eq!(go(&both, "POST", "/runs", r#"{"net": "経理/a"}"#, &[]).status, 202);
        assert_eq!(go(&both, "POST", "/runs", r#"{"net": "経理/b"}"#, &[]).status, 403);
        assert_eq!(go(&both, "POST", "/runs", r#"{"net": "人事/b"}"#, &[]).status, 422);
        assert_eq!(go(&both, "GET", "/nets", "", &[]).json["nets"].as_array().map(Vec::len), Some(2));
    }

    /// rerun, skip, hold and release ask as `koyomi rerun`, `skip`, `hold` and `release` do.
    #[test]
    fn a_run_is_rerun_and_skipped_and_a_net_held() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::Rerun { .. } => Ok("20261031-100000-01\n".to_owned()),
                Request::Skip { job, .. } | Request::Hold { job: Some(job), .. } if job == "x" => Err("そのジョブはない".to_owned()),
                _ => Ok("ok\n".to_owned()),
            }
        };
        let id = "20261031-090000-01";
        let r = handle(&req("POST", &format!("/runs/{id}/rerun"), ""), ask);
        assert_eq!((r.status, r.json["run"].as_str()), (202, Some("20261031-100000-01")));
        assert!(handle(&req("POST", &format!("/runs/{id}/rerun/"), r#"{"next": "b", "warn": true}"#), ask).status == 202);
        assert!(asked.borrow()[0].contains("from: None, next: None"), "{:?}", asked.borrow());
        assert!(asked.borrow()[1].contains(r#"from: Some("b"), next: Some(Warn)"#), "{:?}", asked.borrow());
        assert_eq!(handle(&req("POST", &format!("/runs/{id}/rerun"), r#"{"from": "a", "next": "b"}"#), ask).status, 400);
        assert_eq!(handle(&req("POST", &format!("/runs/{id}/rerun"), r#"{"from": "a", "warn": true}"#), ask).status, 400);

        let r = handle(&req("POST", &format!("/runs/{id}/skip"), r#"{"job": "b"}"#), ask);
        assert_eq!((r.status, r.json["job"].as_str()), (200, Some("b")));
        assert_eq!(handle(&req("POST", &format!("/runs/{id}/skip"), ""), ask).status, 400);
        assert_eq!(handle(&req("POST", &format!("/runs/{id}/skip"), r#"{"job": "x"}"#), ask).status, 422);
        assert_eq!(handle(&req("GET", &format!("/runs/{id}/skip"), ""), ask).status, 405);

        let r = handle(&req("POST", "/nets/hold", r#"{"net": "g/a"}"#), ask);
        assert_eq!((r.status, r.json["held"].as_bool()), (200, Some(true)));
        let r = handle(&req("POST", "/nets/release", r#"{"net": "g/a", "job": "b"}"#), ask);
        assert_eq!((r.status, r.json["held"].as_bool(), r.json["job"].as_str()), (200, Some(false), Some("b")));
        assert!(asked.borrow().last().is_some_and(|a| a.contains(r#"Release { id: "g/a", job: Some("b") }"#)));
        assert_eq!(handle(&req("POST", "/nets/hold", r#"{"net": "g/a", "job": "x"}"#), ask).status, 422);
        assert_eq!(handle(&req("POST", "/nets/hold", "{}"), ask).status, 400);
        assert_eq!(handle(&req("GET", "/nets/hold", ""), ask).status, 405);

        let r = handle(&req("POST", "/nets/skip-next", r#"{"net": "g/a"}"#), ask);
        assert_eq!((r.status, r.json["skip"].as_bool()), (200, Some(true)));
        let r = handle(&req("POST", "/nets/skip-next", r#"{"net": "g/a", "cancel": true}"#), ask);
        assert_eq!((r.status, r.json["skip"].as_bool()), (200, Some(false)));
        assert!(asked.borrow().last().is_some_and(|a| a.contains(r#"SkipNext { id: "g/a", cancel: true }"#)));
        assert_eq!(handle(&req("POST", "/nets/skip-next", r#"{"net": "g/a", "job": "b"}"#), ask).status, 400);
        assert_eq!(handle(&req("POST", "/nets/skip-next", "{}"), ask).status, 400);
        assert_eq!(handle(&req("GET", "/nets/skip-next", ""), ask).status, 405);
    }

    /// A definition is read with its hash and sent back as a proposal, which the engine leaves unconfirmed.
    #[test]
    fn a_definition_is_read_and_proposed() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::Show { id } if id == "g/a" => {
                    let def = koyomi_core::scan::Def {
                        id: "g/a".into(),
                        group: "g".into(),
                        name: "a".into(),
                        title: None,
                        file: "g/a.toml".into(),
                        standing: koyomi_core::scan::Standing::Confirmed,
                        problems: Vec::new(),
                        net: None,
                        sha256: "abc".into(),
                        text: "format = 1\n".into(),
                        previous: None,
                        calendar: None,
                    };
                    Ok(toml::to_string(&def).expect("def"))
                }
                Request::Propose { base_sha256: Some(b), .. } if b == "old" => Err("ほかで変わっています: g/a".to_owned()),
                Request::Propose { text, .. } if text.contains("bad") => Err("保存できない:\nx".to_owned()),
                Request::Propose { .. } => Ok("置いた（未確認。この PC の窓で確認するまで動かない）: g/a\n".to_owned()),
                _ => Err("定義がない".to_owned()),
            }
        };
        let mut r = req("GET", "/nets/text", "");
        r.query = vec![("net".into(), "g/a".into())];
        let got = handle(&r, ask);
        assert_eq!((got.status, got.json["sha256"].as_str(), got.json["standing"].as_str()), (200, Some("abc"), Some("confirmed")));
        assert_eq!(got.json["text"].as_str(), Some("format = 1\n"));
        r.query = vec![("net".into(), "g/x".into())];
        assert_eq!(handle(&r, ask).status, 404);
        assert_eq!(handle(&req("GET", "/nets/text", ""), ask).status, 400);

        let got = handle(&req("POST", "/nets/propose", r#"{"net": "g/a", "text": "format = 1\n", "base": "abc"}"#), ask);
        assert_eq!((got.status, got.json["confirmed"].as_bool()), (200, Some(false)));
        assert!(asked.borrow().last().is_some_and(|a| a.contains(r#"base_sha256: Some("abc")"#)), "{:?}", asked.borrow());
        assert_eq!(handle(&req("POST", "/nets/propose", r#"{"net": "g/a", "text": "x", "base": "old"}"#), ask).status, 409);
        assert_eq!(handle(&req("POST", "/nets/propose", r#"{"net": "g/a", "text": "bad"}"#), ask).status, 422);
        assert_eq!(handle(&req("POST", "/nets/propose", r#"{"net": "g/a"}"#), ask).status, 400);
        assert_eq!(handle(&req("POST", "/nets/propose", r#"{"net": "g/a", "text": "x", "force": true}"#), ask).status, 400);
        assert_eq!(handle(&req("GET", "/nets/propose", ""), ask).status, 405);
        assert_eq!(handle(&req("POST", "/nets/text", ""), ask).status, 405);
    }

    #[test]
    fn a_post_starts_the_net_as_koyomi_run_does() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::Run { id, .. } if id == "g/a" => Ok("20261031-090000-01".to_owned()),
                _ => Err("定義がない".to_owned()),
            }
        };
        let r = handle(&req("POST", "/runs", r#"{"net": "g/a", "params": {"day": "1"}}"#), ask);
        assert_eq!((r.status, r.json["run"].as_str()), (202, Some("20261031-090000-01")));
        assert!(asked.borrow()[0].contains("dry: false") && asked.borrow()[0].contains("\"day\""), "{:?}", asked.borrow());
        assert_eq!(handle(&req("POST", "/runs/", r#"{"net": "g/x"}"#), ask).status, 422);
        assert_eq!(handle(&req("POST", "/runs", ""), ask).status, 400);
        assert_eq!(handle(&req("POST", "/runs", r#"{"net": "g/a", "dry": true}"#), ask).status, 400);
        assert_eq!(handle(&req("POST", "/runs", "{"), ask).status, 400);
        assert_eq!(handle(&req("PUT", "/runs", ""), ask).status, 405);
        assert_eq!(handle(&req("DELETE", "/runs/20261031-090000-01", ""), ask).status, 405);
        assert_eq!(handle(&req("POST", "/nets", ""), ask).status, 405);
        assert_eq!(handle(&req("GET", "/defs", ""), ask).status, 404);
    }

    #[test]
    fn a_get_finds_the_run_today_then_on_the_day_its_id_names() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            Ok(match r {
                Request::Runs => runs(&["20261031-090000-01"]),
                Request::History { from, to } if from == "2026-10-30" && to == "2026-10-30" => runs(&["20261030-230000-01"]),
                _ => runs(&[]),
            })
        };
        let r = handle(&req("GET", "/runs/20261031-090000-01", ""), ask);
        assert_eq!((r.status, r.json["jobnet"].as_str(), r.json["over"].as_bool()), (200, Some("g/a"), Some(true)));
        assert_eq!(asked.borrow().len(), 1);
        assert_eq!(handle(&req("GET", "/runs/20261030-230000-01", ""), ask).status, 200);
        assert_eq!(handle(&req("GET", "/runs/20261029-230000-01", ""), ask).status, 404);
        assert_eq!(handle(&req("GET", "/runs/nope", ""), ask).status, 404);
        assert_eq!(handle(&req("GET", "/runs/20261399-000000-01", ""), ask).status, 404);
    }

    fn with_query(mut r: ito_http::Request, q: &[(&str, &str)]) -> ito_http::Request {
        r.query = q.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        r
    }

    /// the read-only paths answer from the same door requests as the CLI.
    #[test]
    fn the_handshake_goes_through_health_and_release() {
        let ask = |r: &Request| match r {
            Request::Status => Ok("動いている\n".to_owned()),
            Request::Handover { release: false } => Ok(r#"{"role":"standby","active":true,"ready":true}"#.to_owned()),
            Request::Handover { release: true } => Err("時刻が近い（g/a 09:00:00）。少しあとで".to_owned()),
            Request::Runs => {
                let two = runs(&["20261031-090000-01", "20261031-090000-02"]);
                Ok(two.replacen("ended = \"2026-10-31T09:01:00\"\nstate = \"ok\"", "state = \"running\"", 1))
            }
            _ => Err("no".to_owned()),
        };
        let h = handle(&req("GET", "/health", ""), ask);
        assert_eq!((h.status, h.json["ha"]["role"].as_str(), h.json["ha"]["ready"].as_bool()), (200, Some("standby"), Some(true)));
        let r = handle(&req("POST", "/standby/release", "{}"), ask);
        assert_eq!((r.status, r.json["error"].as_str()), (409, Some("時刻が近い（g/a 09:00:00）。少しあとで")));
        assert_eq!(handle(&req("GET", "/standby/release", ""), ask).status, 405);
        let r = handle(&with_query(req("GET", "/runs", ""), &[("running", "1")]), ask);
        assert_eq!(
            r.json["runs"].as_array().map(|a| a.iter().map(|r| r["id"].as_str()).collect::<Vec<_>>()),
            Some(vec![Some("20261031-090000-01")])
        );
        assert_eq!(handle(&req("GET", "/runs", ""), ask).json["runs"].as_array().map(Vec::len), Some(2));
        let let_go = |r: &Request| match r {
            Request::Handover { release: true } => Ok(r#"{"role":"standby","active":false}"#.to_owned()),
            _ => Err("no".to_owned()),
        };
        let r = handle(&req("POST", "/standby/release", "{}"), let_go);
        assert_eq!((r.status, r.json["released"].as_bool(), r.json["active"].as_bool()), (200, Some(true), Some(false)));
        // Neither [standby] nor [primary]: no "ha".
        let plain = |r: &Request| match r {
            Request::Status => Ok("動いている\n".to_owned()),
            _ => Err("no".to_owned()),
        };
        assert!(handle(&req("GET", "/health", ""), plain).json.get("ha").is_none());
        // A user narrowed to some nets reads "ha" but cannot take the schedules back.
        let nets = Access { role: Role::Edit, nets: Some(Nets::parse(&["経理/*".to_owned()]).unwrap()), roles: Vec::new() };
        assert_eq!(within(&req("GET", "/health", ""), &nets, ask).json["ha"]["role"].as_str(), Some("standby"));
        assert_eq!(within(&req("POST", "/standby/release", "{}"), &nets, let_go).status, 403);
    }

    #[test]
    fn the_read_paths_answer_from_the_door() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::Status => Ok("動いている\n".to_owned()),
                Request::Runs => Ok(runs(&["20261031-090000-01"])),
                Request::History { from, .. } if from == "bad" => Err("日付は YYYY-MM-DD で書く: bad".to_owned()),
                Request::History { .. } => Ok(runs(&["20261030-230000-01", "20261030-235000-01"])),
                Request::Log { job, .. } if job == "copy" => Ok("line 1\n***\n".to_owned()),
                Request::Log { .. } => Err("ログがない".to_owned()),
                Request::List => {
                    let net = koyomi_core::jobnet::Jobnet {
                        params: [("month".to_owned(), "{month-1:%Y-%m}".to_owned())].into(),
                        ..Default::default()
                    };
                    let def = koyomi_core::scan::Def {
                        id: "g/a".into(),
                        group: "g".into(),
                        name: "a".into(),
                        title: None,
                        file: "g/a.toml".into(),
                        standing: koyomi_core::scan::Standing::Confirmed,
                        problems: Vec::new(),
                        net: Some(net),
                        sha256: String::new(),
                        text: String::new(),
                        previous: None,
                        calendar: None,
                    };
                    Ok(Snapshot { defs: vec![def], held: vec!["g/b".into()], ..Default::default() }.to_toml())
                }
                _ => Err("no".to_owned()),
            }
        };
        let h = handle(&req("GET", "/health", ""), ask);
        assert_eq!(
            (h.status, h.json["status"].as_str(), h.json["proto"].as_u64()),
            (200, Some("動いている"), Some(u64::from(proto::PROTO)))
        );
        let r = handle(&req("GET", "/runs", ""), ask);
        assert_eq!((r.status, r.json["runs"][0]["over"].as_bool()), (200, Some(true)));
        let r = handle(&with_query(req("GET", "/runs", ""), &[("from", "2026-10-30")]), ask);
        assert_eq!(r.json["runs"].as_array().map(Vec::len), Some(2));
        assert!(asked.borrow().last().is_some_and(|a| a.contains("to: \"2026-10-30\"")), "{:?}", asked.borrow());
        assert_eq!(handle(&with_query(req("GET", "/runs", ""), &[("from", "bad")]), ask).status, 400);
        assert_eq!(handle(&with_query(req("GET", "/runs", ""), &[("to", "2026-10-30")]), ask).status, 400);
        let l = handle(&with_query(req("GET", "/runs/20261031-090000-01/log", ""), &[("job", "copy")]), ask);
        assert_eq!((l.status, l.json["log"].as_str()), (200, Some("line 1\n***\n")));
        assert_eq!(handle(&req("GET", "/runs/20261031-090000-01/log", ""), ask).status, 400);
        assert_eq!(handle(&with_query(req("GET", "/runs/20261031-090000-01/log", ""), &[("job", "x")]), ask).status, 404);
        let n = handle(&req("GET", "/nets", ""), ask);
        assert_eq!((n.status, n.json["nets"].as_array().map(Vec::len), n.json["paused"].as_bool()), (200, Some(1), Some(false)));
        // The values a run can be given, with their defaults: what a caller asks for before POST /runs.
        assert_eq!(n.json["nets"][0]["params"]["month"].as_str(), Some("{month-1:%Y-%m}"));
    }

    /// The Web UI's 今日 from `GET /nets`: the starts still to come today, the next one, the one
    /// after today's last, and the skips not yet come; nothing for a net on hold.
    #[test]
    fn the_nets_carry_todays_starts() {
        let def = |id: &str| {
            let text = "format = 1\n[[when]]\nevery = \"minutes\"\nn = 360\nfrom = \"00:00\"\nto = \"23:59\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
            koyomi_core::scan::Def {
                id: id.into(),
                group: "g".into(),
                name: id.trim_start_matches("g/").into(),
                title: None,
                file: format!("{id}.toml"),
                standing: koyomi_core::scan::Standing::Confirmed,
                problems: Vec::new(),
                net: Some(koyomi_core::jobnet::parse(text).unwrap()),
                sha256: String::new(),
                text: String::new(),
                previous: None,
                calendar: None,
            }
        };
        let at = |t: &str| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M").unwrap();
        let snap = Snapshot {
            defs: vec![def("g/a"), def("g/b")],
            held: vec!["g/b".into()],
            held_jobs: vec![koyomi_core::held::HeldJob { net: "g/a".into(), job: "a".into() }],
            skips: vec![
                koyomi_core::held::Skip { id: "g/a".into(), at: at("2026-10-31 18:00") },
                koyomi_core::held::Skip { id: "g/a".into(), at: at("2026-10-31 06:00") },
            ],
            ..Default::default()
        };
        let v = nets_at(&snap, at("2026-10-31 07:30"), |_| None);
        let a = &v["nets"][0];
        assert_eq!(a["today"], serde_json::json!(["2026-10-31T12:00:00", "2026-10-31T18:00:00"]));
        assert_eq!((a["next"].as_str(), a["then"].as_str()), (Some("2026-10-31T12:00:00"), Some("2026-11-01T00:00:00")));
        assert_eq!(a["skips"], serde_json::json!(["2026-10-31T18:00:00"]));
        // The jobs, and which of them is held on its own (not the net).
        assert_eq!(a["jobs"], serde_json::json!([{ "id": "a", "name": null, "held": true }]));
        assert_eq!(v["nets"][1]["jobs"][0]["held"].as_bool(), Some(false));
        let b = &v["nets"][1];
        assert_eq!((b["held"].as_bool(), b["today"].as_array().map(Vec::len), b["next"].is_null()), (Some(true), Some(0), true));
    }

    /// The listener as the engine starts it: the token first, then the handler.
    #[test]
    fn a_request_without_the_token_is_turned_away() {
        let ask = |_: &Request| Ok::<_, String>("20261031-090000-01".to_owned());
        let server = Server::start(ito_http::Config::new(0, "tok"), move |r| handle(&r, ask), |_| {}).expect("listen");
        let send = |auth: &str| {
            let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port())).expect("connect");
            let body = r#"{"net":"g/a"}"#;
            write!(
                c,
                "POST /runs HTTP/1.1\r\nHost: x\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .expect("send");
            let mut out = String::new();
            let _ = c.read_to_string(&mut out);
            out
        };
        assert!(send("").starts_with("HTTP/1.1 401"), "{}", send(""));
        assert!(send("Authorization: Bearer nope\r\n").starts_with("HTTP/1.1 401"));
        let ok = send("Authorization: Bearer tok\r\n");
        assert!(ok.starts_with("HTTP/1.1 202") && ok.contains("20261031-090000-01"), "{ok}");
        server.stop();
    }

    /// each token gets its role: view reads only, run also starts runs, and a request with no
    /// token never reaches the engine.
    #[test]
    fn a_role_reaches_only_its_paths() {
        let cfg = config::Config::parse(
            "[http]\nport = 8790\ntoken = \"secret:t\"\n[[http.user]]\nname = \"sato\"\nrole = \"view\"\ntoken = \"secret:s\"\n[[http.user]]\nname = \"ito\"\nrole = \"edit\"\ntoken = \"secret:i\"\n",
        )
        .unwrap();
        let known = roles(cfg.http.as_ref().unwrap());
        let as_who = |who: Option<&str>, method: &str, path: &str| {
            let mut r = req(method, path, "");
            r.who = who.map(str::to_owned);
            allowed(&r, &known).map(|r| r.status)
        };
        assert_eq!(as_who(Some("sato"), "GET", "/runs"), None);
        assert_eq!(as_who(Some("sato"), "POST", "/runs"), Some(403));
        assert_eq!(as_who(Some("default"), "POST", "/runs"), None);
        assert_eq!(as_who(Some("default"), "POST", "/nets"), Some(403));
        assert_eq!(as_who(Some("ito"), "POST", "/nets"), None);
        assert_eq!(as_who(Some("gone"), "GET", "/runs"), Some(403));
        assert_eq!(as_who(None, "GET", "/runs"), Some(401));
        let only_users =
            config::Config::parse("[http]\nport = 8790\n[[http.user]]\nname = \"sato\"\nrole = \"run\"\ntoken = \"secret:s\"\n").unwrap();
        assert!(!roles(only_users.http.as_ref().unwrap()).contains_key("default"));
    }

    /// on the LAN another PC reaches the same paths over TLS, only with the pinned certificate.
    #[test]
    fn another_pc_reads_over_tls_with_the_pinned_certificate() {
        let dir = std::env::temp_dir().join(format!("koyomi-http-tls-{}", std::process::id()));
        let mut cfg = ito_http::Config::new(0, "tok");
        cfg.bind = ito_http::Bind::Lan { addr: Some([127, 0, 0, 1].into()), tls: ito_http::TlsCfg::new(&dir) };
        let ask = |r: &Request| match r {
            Request::Status => Ok("動いている".to_owned()),
            _ => Err("no".to_owned()),
        };
        let server = Server::start(cfg, move |r| handle(&r, ask), |_| {}).expect("listen");
        let fp = ito_http::tls::format(&server.fingerprint().expect("a LAN bind has a certificate"));
        let url = format!("https://127.0.0.1:{}", server.port());
        assert_eq!(ito_http::client::peek_fingerprint(&url, Duration::from_secs(5)).as_deref(), Ok(fp.as_str()));
        let c = ito_http::client::Client::new(&url, &fp, "tok").expect("client");
        let h = c.get("/health").expect("health");
        assert_eq!((h.status, h.json["status"].as_str()), (200, Some("動いている")));
        let other = ito_http::tls::format(&[7; 32]);
        let wrong = ito_http::client::Client::new(&url, &other, "tok").expect("client").get("/health");
        assert!(matches!(wrong, Err(ito_http::client::Error::Fingerprint(ref got)) if *got == fp), "{wrong:?}");
        assert_eq!(ito_http::client::Client::new(&url, &fp, "nope").expect("client").get("/health").map(|r| r.status), Ok(401));
        server.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_day_comes_from_the_run_id() {
        assert_eq!(started_on("20261031-090000-01").as_deref(), Some("2026-10-31"));
        assert_eq!(started_on("2026103-090000-01"), None);
        assert_eq!(started_on("20261031"), None);
        assert_eq!(started_on("２０２６1031-0"), None);
    }

    /// `/theme.css` is the theme common.toml picks: one `:root` for a fixed theme, the light one
    /// under `prefers-color-scheme` for `system`, the stronger colours with `high_contrast`, and
    /// the first built-in theme for a name not found.
    #[test]
    fn the_theme_follows_common_toml() {
        let dir = std::env::temp_dir().join(format!("koyomi-http-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let write = |text: &str| std::fs::write(ito_common::common_path(&dir), text).expect("common.toml");
        let themes = ito_theme::builtin();
        let named = |n: &str| themes.iter().find(|t| t.name == n).expect("a built-in theme").colors;
        write(
            "theme = \"system\"
dark_theme = \"Nord\"
light_theme = \"Catppuccin Latte\"
",
        );
        let css = theme_css(Some(&dir));
        assert!(
            css.starts_with(&format!(
                ":root {{
{}}}
",
                named("Nord").css_vars()
            )),
            "{css}"
        );
        assert!(
            css.contains("@media (prefers-color-scheme: light)")
                && css.contains(named("Catppuccin Latte").css_vars().lines().next().unwrap())
        );
        write(
            "theme = \"Catppuccin Latte\"
",
        );
        assert_eq!(
            theme_css(Some(&dir)),
            format!(
                ":root {{
{}}}
",
                named("Catppuccin Latte").css_vars()
            )
        );
        write(
            "theme = \"Catppuccin Latte\"
high_contrast = true
",
        );
        assert_eq!(
            theme_css(Some(&dir)),
            format!(
                ":root {{
{}}}
",
                named("Catppuccin Latte").high_contrast().css_vars()
            )
        );
        write(
            "theme = \"nope\"
",
        );
        assert_eq!(
            theme_css(Some(&dir)),
            format!(
                ":root {{
{}}}
",
                themes[0].colors.css_vars()
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Web UI's files need no token and forbid other origins and inline script; `/me` says
    /// the role; a stop is `koyomi kill`.
    #[test]
    fn the_page_is_served_and_a_stop_is_a_kill() {
        let p = page(&req("GET", "/", "")).expect("the page");
        assert_eq!(p.status, 200);
        assert!(p.headers.iter().any(|(k, v)| k == "Content-Security-Policy" && v.contains("script-src 'self'") && !v.contains("unsafe")));
        assert!(page(&req("GET", "/app.js", "")).is_some() && page(&req("GET", "/app.css", "")).is_some());
        let css = page(&req("GET", "/theme.css", "")).expect("the theme");
        assert!(css.headers.iter().any(|(k, _)| k == "Content-Security-Policy"));
        assert!(page(&req("POST", "/", "")).is_none() && page(&req("GET", "/runs", "")).is_none());
        for (_, _, body) in PAGES {
            assert!(!body.contains("innerHTML") && !body.contains("onclick=") && !body.contains("http://") && !body.contains("https://"));
        }
        let known = BTreeMap::from([("sato".to_owned(), Role::View)]);
        let mut r = req("GET", "/me", "");
        r.who = Some("sato".into());
        let m = me(&r, &known, &BTreeMap::new()).expect("me");
        assert_eq!(m.status, 200);
        assert_eq!((m.json["who"].as_str(), m.json["role"].as_str()), (Some("sato"), Some("view")));
        assert!(me(&req("GET", "/runs", ""), &known, &BTreeMap::new()).is_none());
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            Ok("止めた".to_owned())
        };
        assert_eq!(handle(&req("POST", "/runs/20261031-090000-01/stop", ""), ask).status, 200);
        assert!(asked.borrow()[0].starts_with("Kill") && asked.borrow()[0].contains("job: None"), "{:?}", asked.borrow());
        assert_eq!(handle(&req("GET", "/runs/20261031-090000-01/stop", ""), ask).status, 405);
    }
}
