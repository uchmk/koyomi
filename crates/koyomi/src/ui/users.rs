//! The 利用者 page of the settings: the engine's HTTPS users (`[[http.user]]` of koyomi.toml), listed, added,
//! changed and removed through the door's `Users`, `UserSet`, `UserRemove`, `UserAdd` and `UserReset`.
//!
//! The state here is plain data, so the rules are tested without a window: what the page may do (`access`), how
//! a listing is taken (the worker answers in order, so only the last `Users` asked counts), what an answer does
//! (`answered`), and what a click asks for (`act`). The drawing is `show`, and the box that shows a new token
//! once is `token_show`.
//!
//! A token lives only in [`Issued`], in the box's state: it is not in a log, a result line, a state file, a test
//! ID or any `{:?}` (its `Debug` prints `***`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use koyomi_core::msg;
use koyomi_core::role::Role;
use koyomi_core::users::{Listed, Listing, Token};

use crate::proto::Request;

/// The role and the name the caller has at the engine: what a listing was read under.
pub(super) type Under = (Option<Role>, Option<String>);

const ROLES: [Role; 4] = [Role::View, Role::Run, Role::Edit, Role::Admin];

/// What the page may do now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Access {
    /// Another PC's engine has not told the role yet: a loading line, nothing sent.
    Wait,
    /// The connection is down: its error, as it is.
    Down(String),
    /// Another PC's engine, and a role that is not `admin`.
    NotAdmin,
    /// The listing may be asked for and changed.
    Open,
}

/// The page's access. `far`: the window shows another PC's engine. Only an `admin` manages the users there.
pub(super) fn access(far: bool, role: Option<Role>, down: Option<&str>) -> Access {
    if !far {
        return Access::Open;
    }
    if role.is_some_and(|r| r != Role::Admin) {
        return Access::NotAdmin;
    }
    if let Some(e) = down.filter(|e| !e.is_empty()) {
        return Access::Down(e.to_owned());
    }
    match role {
        None => Access::Wait,
        Some(_) => Access::Open,
    }
}

/// The answer to `UserAdd` or `UserReset`: the token is shown once and kept nowhere else.
pub(super) struct Issued {
    pub name: String,
    token: String,
    pub note: String,
}

impl fmt::Debug for Issued {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Issued").field("name", &self.name).field("token", &"***").finish()
    }
}

impl Issued {
    /// The reply's `name`, `token` and `note`; an unparsable reply, or one with no token, is `None`, and the
    /// parser's text (which could quote the reply) is dropped.
    pub(super) fn parse(reply: &str) -> Option<Issued> {
        #[derive(serde::Deserialize)]
        struct Raw {
            name: String,
            token: String,
            #[serde(default)]
            note: String,
        }
        let raw: Raw = toml::from_str(reply).ok()?;
        (!raw.token.is_empty()).then_some(Issued { name: raw.name, token: raw.token, note: raw.note })
    }

    #[cfg(test)]
    pub(super) fn token(&self) -> &str {
        &self.token
    }
}

/// A line under the list, kept as what it is so it is worded when it is drawn (the answer is taken where there
/// is no word table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Line {
    /// The engine's own words, as they came.
    Raw(String),
    /// koyomi.toml was changed since the listing was read.
    BaseChanged,
    /// The engine said yes, but its reply could not be read.
    ReplyBad(String),
    /// The link broke while a user was being added: it may have been.
    UnsureAdd(String),
    /// The link broke while a token was being made: it may have changed.
    UnsureReset(String),
    /// A request the thread did not take: why.
    Unsent(String),
}

impl Line {
    pub(super) fn text(&self, t: &ito_i18n::Table) -> String {
        match self {
            Line::Raw(s) | Line::Unsent(s) => s.clone(),
            Line::BaseChanged => t.tr("users.base_changed").to_owned(),
            Line::ReplyBad(n) => t.trf("users.reply_bad", &[n]),
            Line::UnsureAdd(n) => t.trf("users.unsure_add", &[n]),
            Line::UnsureReset(n) => t.trf("users.unsure_reset", &[n]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Set,
    Remove,
    Add,
    Reset,
}

/// The page's write that has not been answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Write {
    pub kind: Kind,
    pub name: String,
}

/// A user's nets as the person picks them: every net, or the chosen patterns (`<group>/<name>`, where `*` is any
/// run inside one part). The saved form is `None` (every net) or `Some(chosen)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NetPick {
    /// Every net: the request carries `nets: None`. Then `chosen` is empty.
    pub all: bool,
    /// The patterns, in the order they will be saved.
    pub chosen: Vec<String>,
}

impl NetPick {
    /// Every net.
    fn every() -> NetPick {
        NetPick { all: true, chosen: Vec::new() }
    }

    /// The pick a listed user has: nothing is tidied.
    fn of(nets: Option<&[String]>) -> NetPick {
        match nets {
            None => NetPick::every(),
            Some(n) => NetPick { all: false, chosen: n.to_vec() },
        }
    }

    /// The one normal form: `None` for every net, else the list as it is.
    fn norm(&self) -> Option<Vec<String>> {
        if self.all {
            None
        } else {
            Some(self.chosen.clone())
        }
    }

    /// 「選んだジョブネットだけ」 with nothing ticked: it cannot be saved.
    fn missing(&self) -> bool {
        !self.all && self.chosen.is_empty()
    }

    fn choose_all(&mut self) {
        self.all = true;
        self.chosen.clear();
    }

    fn has(&self, entry: &str) -> bool {
        self.chosen.iter().any(|c| c == entry)
    }

    /// Ticks an entry that is not ticked yet: it goes last.
    fn tick(&mut self, entry: &str) {
        if !self.has(entry) {
            self.chosen.push(entry.to_owned());
        }
    }

    /// Unticks an entry: every copy of it goes.
    fn untick(&mut self, entry: &str) {
        self.chosen.retain(|c| c != entry);
    }

    /// Ticks a group's `<group>/*`; the single ids of that group are covered now, so they go.
    fn tick_group(&mut self, group: &str) {
        let pattern = group_pattern(group);
        let prefix = format!("{group}/");
        self.chosen.retain(|c| *c == pattern || !c.starts_with(&prefix) || c.contains('*'));
        self.tick(&pattern);
    }
}

/// Whether two nets lists reach the same nets as the engine reads them: the order and repeats do not count.
fn same_nets(a: &Option<Vec<String>>, b: &Option<Vec<String>>) -> bool {
    fn set(v: &Option<Vec<String>>) -> Option<BTreeSet<&String>> {
        v.as_ref().map(|v| v.iter().collect())
    }
    set(a) == set(b)
}

/// The pattern that covers every net of a group, now and later.
fn group_pattern(group: &str) -> String {
    format!("{group}/*")
}

/// The engine's net ids as the chooser shows them: each group, and its ids, sorted. An id with no group is not offered.
fn offered(ids: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for id in ids {
        if let Some((g, _)) = id.split_once('/') {
            groups.entry(g.to_owned()).or_default().push(id.clone());
        }
    }
    groups
        .into_iter()
        .map(|(g, mut v)| {
            v.sort();
            v.dedup();
            (g, v)
        })
        .collect()
}

/// Why a chosen entry is not one of the checkboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kept {
    /// It has a `*` (written in koyomi.toml).
    Pattern,
    /// The engine's list has come and the net is not in it.
    Gone,
    /// The engine's list has not come.
    Unchecked,
}

/// What `chosen` holds that the list does not offer, in `chosen`'s order, each text once.
fn kept_entries(chosen: &[String], ids: &[String]) -> Vec<(String, Kept)> {
    let groups = offered(ids);
    let mut out: Vec<(String, Kept)> = Vec::new();
    for c in chosen {
        let is_group = c.strip_suffix("/*").is_some_and(|g| groups.iter().any(|(n, _)| n == g));
        let is_id = groups.iter().any(|(_, v)| v.contains(c));
        if is_group || is_id || out.iter().any(|(t, _)| t == c) {
            continue;
        }
        let why = if c.contains('*') {
            Kept::Pattern
        } else if ids.is_empty() {
            Kept::Unchecked
        } else {
            Kept::Gone
        };
        out.push((c.clone(), why));
    }
    out
}

/// A text as a part of a test ID: lowercase hex of its UTF-8 bytes, so two texts never share one.
fn net_code(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

/// 役割を変える…: one user's editor.
#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub name: String,
    pub role: String,
    pub nets: NetPick,
    pub roles: Option<BTreeMap<String, String>>,
    /// What the editor opened with.
    orig: (String, Option<Vec<String>>, Option<BTreeMap<String, String>>),
    /// The role or the name changed under the page: 保存 waits until the person has looked at the new listing.
    pub stale: bool,
    /// The user was changed elsewhere while this was open: what is typed is merged with the new listing, and the
    /// save waits until the person has looked at it.
    pub changed: bool,
}

impl Edit {
    fn of(l: &Listed) -> Edit {
        let roles = l.user.roles.clone().filter(|r| !r.is_empty());
        let nets = NetPick::of(l.user.nets.as_deref());
        Edit {
            name: l.user.name.clone(),
            role: l.user.role.clone(),
            orig: (l.user.role.clone(), nets.norm(), roles.clone()),
            nets,
            roles,
            stale: false,
            changed: false,
        }
    }

    /// A newer listing of the same user while this has typed changes: a field the person did not change takes the
    /// listed value, a changed one keeps what was typed, and the per-net roles (not editable here) are the listed ones.
    fn merge(&mut self, fresh: Edit) {
        if self.role == self.orig.0 {
            self.role = fresh.role.clone();
        }
        if same_nets(&self.nets.norm(), &self.orig.1) {
            self.nets = fresh.nets.clone();
        }
        self.roles = fresh.roles.clone();
        self.orig = fresh.orig;
        self.normalize();
        self.changed = true;
    }

    fn is_admin(&self) -> bool {
        self.role == Role::Admin.name()
    }

    /// Admin has every net and no per-net roles.
    fn normalize(&mut self) {
        if self.is_admin() {
            self.nets = NetPick::every();
            self.roles = None;
        }
    }

    /// The nets are 「選んだジョブネットだけ」 with nothing ticked (an admin never is).
    pub(super) fn nets_missing(&self) -> bool {
        !self.is_admin() && self.nets.missing()
    }

    pub(super) fn dirty(&self) -> bool {
        self.role != self.orig.0 || !same_nets(&self.nets.norm(), &self.orig.1) || self.roles != self.orig.2
    }

    fn request(&self, base: &str) -> Request {
        let admin = self.is_admin();
        Request::UserSet {
            name: self.name.clone(),
            role: self.role.clone(),
            nets: if admin { None } else { self.nets.norm() },
            roles: if admin { None } else { self.roles.clone() },
            base: base.to_owned(),
        }
    }
}

/// 追加…: the form at the top.
#[derive(Clone, Debug)]
pub(super) struct Add {
    pub name: String,
    pub role: String,
    pub nets: NetPick,
    pub stale: bool,
    /// Why the name was refused, in the process language.
    pub error: Option<String>,
}

impl Add {
    fn new() -> Add {
        Add { name: String::new(), role: Role::View.name().to_owned(), nets: NetPick::every(), stale: false, error: None }
    }

    pub(super) fn dirty(&self) -> bool {
        !self.name.trim().is_empty() || self.role != Role::View.name() || !self.nets.all
    }

    fn is_admin(&self) -> bool {
        self.role == Role::Admin.name()
    }

    /// The nets are 「選んだジョブネットだけ」 with nothing ticked (an admin never is).
    pub(super) fn nets_missing(&self) -> bool {
        !self.is_admin() && self.nets.missing()
    }

    /// The trimmed name, or why it is refused (before anything is sent).
    fn check(&self, names: &[String]) -> Result<String, String> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().any(char::is_whitespace) || name == "default" {
            return Err(msg::note(msg::N_USER_ASK_NAME_BAD, &[]));
        }
        if names.iter().any(|n| n == name) {
            return Err(msg::note(msg::N_USER_ASK_NAME_EXISTS, &[name]));
        }
        Ok(name.to_owned())
    }

    fn request(&self, name: String, base: &str) -> Request {
        Request::UserAdd {
            name,
            role: self.role.clone(),
            nets: if self.is_admin() { None } else { self.nets.norm() },
            roles: None,
            base: base.to_owned(),
        }
    }
}

/// The question before a token is made again or a user is removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Confirm {
    pub kind: Kind,
    pub name: String,
    /// 0: go on, 1: cancel. A fresh question starts on cancel.
    pub focus: usize,
}

/// What the page needs to know about the window each frame.
pub(super) struct View<'a> {
    /// The `[[remote]]` name of the PC shown; `None` for this PC.
    pub far: Option<&'a str>,
    /// The caller's name at the engine (`/me`'s `who`); `None` for this PC.
    pub who: Option<&'a str>,
    /// The access is [`Access::Open`].
    pub open: bool,
    /// The ids of the engine's nets: what the nets chooser offers.
    pub nets: &'a [String],
}

/// What a click on the page asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Click {
    Reload,
    OpenEdit(String),
    CloseEdit,
    Save,
    OpenAdd,
    CloseAdd,
    Create,
    /// 「新しい一覧を見ました」: the forms may be sent again.
    Seen,
    AskReset(String),
    AskRemove(String),
    Yes,
    No,
    /// Esc: the question, else the add form, else the editor.
    Esc,
    MoveFocus,
}

/// What the browser does after a click.
// `Request` grew with the typed password fields; these are short-lived UI actions, one at a time, so boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(super) enum Do {
    Nothing,
    /// Ask `Users` (a read).
    Ask,
    Send(Request),
}

/// What an answer did to the page, for the browser.
pub(super) struct Done {
    /// The list is asked for again.
    pub reload: bool,
    /// A line for the result line (never a token).
    pub result: Option<Result<String, String>>,
}

#[derive(Default)]
pub(super) struct Users {
    /// The last frame the page was drawn in.
    seen: Option<u64>,
    pub(super) listing: Option<Listing>,
    /// The role and name last told, to see a change.
    ident: Option<Under>,
    /// `Users` asked and not yet answered.
    asked: usize,
    /// The answers to throw away (asked before the role or the name changed), oldest first.
    discard: usize,
    /// The newest answer taken while others are still to come.
    got: Option<Result<Listing, String>>,
    /// The list is to be asked for when the page can.
    pub(super) need: bool,
    error: Option<String>,
    pub(super) edit: Option<Edit>,
    pub(super) add: Option<Add>,
    write: Option<Write>,
    refusal: Option<Line>,
    pub(super) confirm: Option<Confirm>,
    token: Option<Issued>,
    tokens_opened: u64,
}

impl Users {
    /// The page is drawn this frame. A frame that does not follow the last one it was drawn in is a new opening:
    /// the list is asked for again and forms that hold nothing typed are closed.
    pub(super) fn touch(&mut self, frame: u64) {
        let fresh = self.seen.is_none_or(|s| frame > s + 1);
        self.seen = Some(frame);
        if fresh {
            self.need = true;
            self.refusal = None;
            self.confirm = None;
            if self.edit.as_ref().is_some_and(|e| !e.dirty()) {
                self.edit = None;
            }
            if self.add.as_ref().is_some_and(|a| !a.dirty()) {
                self.add = None;
            }
        }
    }

    /// Whether the page was drawn in the frame before `frame` (or in it).
    pub(super) fn is_open(&self, frame: u64) -> bool {
        self.seen.is_some_and(|s| frame <= s + 1)
    }

    /// `Users` is being sent.
    pub(super) fn ask(&mut self) {
        self.need = false;
        self.asked += 1;
        self.error = None;
    }

    /// One `Users` answer. Only when none is left to come is a listing taken: the newest wins.
    pub(super) fn listing_answer(&mut self, r: Result<String, String>) {
        self.asked = self.asked.saturating_sub(1);
        if self.discard > 0 {
            self.discard -= 1;
            return;
        }
        self.got = Some(r.and_then(|t| Listing::parse(&t)));
        if self.asked == 0 {
            match self.got.take() {
                Some(Ok(l)) => {
                    self.listing = Some(l);
                    self.error = None;
                    self.sync_edit();
                }
                Some(Err(e)) => self.error = Some(e),
                None => {}
            }
        }
    }

    /// An open editor against the user as the new listing has it.
    fn sync_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
        let listed = self.listing.as_ref().and_then(|l| l.user.iter().find(|u| u.user.name == e.name));
        let Some(listed) = listed else { return };
        let fresh = Edit::of(listed);
        self.edit = Some(if fresh.orig == e.orig {
            e
        } else if e.dirty() {
            let mut e = e;
            e.merge(fresh);
            e
        } else {
            fresh
        });
    }

    /// The role or the name at the engine is told. A change from the last told one drops the listing.
    /// Returns whether it did.
    pub(super) fn role_told(&mut self, now: Under) -> bool {
        if self.ident.as_ref() == Some(&now) {
            return false;
        }
        let first = self.ident.is_none();
        self.ident = Some(now);
        if first {
            return false;
        }
        self.dropped();
        true
    }

    /// What was read under another role or name is not kept: the listing goes, forms with nothing typed close,
    /// and the others wait until the new listing is looked at.
    fn dropped(&mut self) {
        self.listing = None;
        self.got = None;
        self.error = None;
        self.refusal = None;
        self.confirm = None;
        self.discard = self.asked;
        self.need = true;
        match &mut self.edit {
            Some(e) if e.dirty() => e.stale = true,
            _ => self.edit = None,
        }
        match &mut self.add {
            Some(a) if a.dirty() => a.stale = true,
            _ => self.add = None,
        }
    }

    fn ready(&self, v: &View) -> bool {
        v.open && self.write.is_none() && self.asked == 0 && self.error.is_none() && self.listing.is_some()
    }

    fn can_write(&self, v: &View) -> bool {
        self.ready(v) && self.confirm.is_none()
    }

    fn locked(v: &View, name: &str) -> bool {
        v.far.is_some() && v.who == Some(name)
    }

    fn base(&self) -> String {
        self.listing.as_ref().map(|l| l.base.clone()).unwrap_or_default()
    }

    fn names(&self) -> Vec<String> {
        self.listing.iter().flat_map(|l| l.user.iter().map(|u| u.user.name.clone())).collect()
    }

    /// What a click asks for.
    pub(super) fn act(&mut self, click: Click, v: &View) -> Do {
        match click {
            Click::Reload => {
                if v.open && self.asked == 0 && self.write.is_none() {
                    self.need = true;
                    return Do::Ask;
                }
            }
            Click::OpenEdit(name) => {
                if self.can_write(v) && !Self::locked(v, &name) && !self.edit.as_ref().is_some_and(Edit::dirty) {
                    let listed = self.listing.as_ref().and_then(|l| l.user.iter().find(|u| u.user.name == name));
                    self.edit = listed.map(Edit::of);
                    self.refusal = None;
                }
            }
            Click::CloseEdit => self.edit = None,
            Click::Save => {
                let ok = self.can_write(v)
                    && self
                        .edit
                        .as_ref()
                        .is_some_and(|e| !e.stale && !e.changed && e.dirty() && !e.nets_missing() && !Self::locked(v, &e.name));
                if ok {
                    let req = self.edit.as_ref().map(|e| (e.name.clone(), e.request(&self.base())));
                    if let Some((name, req)) = req {
                        self.refusal = None;
                        self.write = Some(Write { kind: Kind::Set, name });
                        return Do::Send(req);
                    }
                }
            }
            Click::OpenAdd => {
                if self.add.is_none() && v.open && self.write.is_none() {
                    self.add = Some(Add::new());
                }
            }
            Click::CloseAdd => self.add = None,
            Click::Create => {
                if self.can_write(v) {
                    let names = self.names();
                    let base = self.base();
                    if let Some(a) = self.add.as_mut().filter(|a| !a.stale) {
                        match a.check(&names) {
                            Err(why) => a.error = Some(why),
                            Ok(_) if a.nets_missing() => a.error = None,
                            Ok(name) => {
                                a.error = None;
                                let req = a.request(name.clone(), &base);
                                self.refusal = None;
                                self.write = Some(Write { kind: Kind::Add, name });
                                return Do::Send(req);
                            }
                        }
                    }
                }
            }
            Click::Seen => {
                if self.ready(v) {
                    if let Some(e) = &mut self.edit {
                        e.stale = false;
                        e.changed = false;
                    }
                    if let Some(a) = &mut self.add {
                        a.stale = false;
                    }
                }
            }
            Click::AskReset(name) => {
                if self.can_write(v) && !Self::locked(v, &name) {
                    self.confirm = Some(Confirm { kind: Kind::Reset, name, focus: 1 });
                }
            }
            Click::AskRemove(name) => {
                if self.can_write(v) && !Self::locked(v, &name) {
                    self.confirm = Some(Confirm { kind: Kind::Remove, name, focus: 1 });
                }
            }
            Click::MoveFocus => {
                if let Some(c) = &mut self.confirm {
                    c.focus = 1 - c.focus.min(1);
                }
            }
            Click::No => self.confirm = None,
            Click::Yes => {
                let c = self.confirm.take();
                if let Some(c) = c.filter(|c| self.ready(v) && !Self::locked(v, &c.name)) {
                    if self.listing.as_ref().is_some_and(|l| l.user.iter().any(|u| u.user.name == c.name)) {
                        let base = self.base();
                        let req = match c.kind {
                            Kind::Reset => Request::UserReset { name: c.name.clone(), base },
                            _ => Request::UserRemove { name: c.name.clone(), base },
                        };
                        self.refusal = None;
                        self.write = Some(Write { kind: c.kind, name: c.name });
                        return Do::Send(req);
                    }
                }
            }
            Click::Esc => {
                if self.confirm.is_some() {
                    self.confirm = None;
                } else if self.add.is_some() {
                    self.add = None;
                } else {
                    self.edit = None;
                }
            }
        }
        Do::Nothing
    }

    /// The answer to one of the page's writes (`UserSet`, `UserRemove`, `UserAdd`, `UserReset`).
    pub(super) fn answered(&mut self, req: &Request, r: Result<String, String>) -> Done {
        let (kind, name) = match req {
            Request::UserSet { name, .. } => (Kind::Set, name),
            Request::UserRemove { name, .. } => (Kind::Remove, name),
            Request::UserAdd { name, .. } => (Kind::Add, name),
            Request::UserReset { name, .. } => (Kind::Reset, name),
            _ => return Done { reload: false, result: None },
        };
        let name = name.trim().to_owned();
        self.write = None;
        match r {
            Ok(reply) => {
                let result = match kind {
                    Kind::Set => {
                        self.edit = None;
                        Some(Ok(reply))
                    }
                    Kind::Remove => {
                        if self.edit.as_ref().is_some_and(|e| e.name == name) {
                            self.edit = None;
                        }
                        Some(Ok(reply))
                    }
                    Kind::Add | Kind::Reset => {
                        if kind == Kind::Add {
                            self.add = None;
                        }
                        match Issued::parse(&reply) {
                            Some(issued) => {
                                self.token = Some(issued);
                                self.tokens_opened += 1;
                                Some(Ok(match (kind, req) {
                                    (Kind::Add, Request::UserAdd { role, nets, roles, .. }) => {
                                        let nets = nets_word(nets.as_deref());
                                        let roles = roles_word(roles.as_ref());
                                        msg::note(msg::N_USER_ADD_DONE, &[&name, role, &nets, &roles])
                                    }
                                    _ => msg::note(msg::N_USER_RESET_DONE, &[&name]),
                                }))
                            }
                            None => {
                                self.refusal = Some(Line::ReplyBad(name.clone()));
                                None
                            }
                        }
                    }
                };
                Done { reload: true, result }
            }
            Err(e) => {
                self.refusal = Some(if e.contains(&msg::USER_BASE_CHANGED.code()) {
                    Line::BaseChanged
                } else if msg::MAY_HAVE_ARRIVED.is_in(&e) && matches!(kind, Kind::Add | Kind::Reset) {
                    if kind == Kind::Add {
                        Line::UnsureAdd(name)
                    } else {
                        Line::UnsureReset(name)
                    }
                } else if msg::is_coded(&e) {
                    Line::Raw(e)
                } else {
                    match kind {
                        Kind::Add => Line::UnsureAdd(name),
                        Kind::Reset => Line::UnsureReset(name),
                        _ => Line::Raw(e),
                    }
                });
                Done { reload: true, result: None }
            }
        }
    }

    /// A request the thread did not take: the page does not wait for an answer that will not come. What was
    /// typed stays.
    pub(super) fn unsent(&mut self, read: bool, why: String) {
        if read {
            self.asked = self.asked.saturating_sub(1);
            self.error = Some(why);
        } else {
            self.write = None;
            self.refusal = Some(Line::Unsent(why));
        }
    }

    /// Whether something here would be lost by a switch of the link: a token not yet seen, or typed changes.
    pub(super) fn unsaved(&self) -> bool {
        self.token.is_some() || self.forms_dirty()
    }

    /// Whether an editor or the add form holds typed changes.
    pub(super) fn forms_dirty(&self) -> bool {
        self.edit.as_ref().is_some_and(Edit::dirty) || self.add.as_ref().is_some_and(Add::dirty)
    }

    pub(super) fn holds_token(&self) -> bool {
        self.token.is_some()
    }

    pub(super) fn tokens_opened(&self) -> u64 {
        self.tokens_opened
    }

    /// 閉じる: the token is dropped.
    pub(super) fn close_token(&mut self) {
        self.token = None;
    }

    /// A question, the add form or an editor is up (Esc closes it before it closes the settings).
    pub(super) fn wants_esc(&self) -> bool {
        self.confirm.is_some() || self.edit.is_some() || self.add.is_some()
    }

    pub(super) fn token(&self) -> Option<&Issued> {
        self.token.as_ref()
    }

    #[cfg(test)]
    pub(super) fn write(&self) -> Option<&Write> {
        self.write.as_ref()
    }

    #[cfg(test)]
    pub(super) fn refusal(&self) -> Option<&Line> {
        self.refusal.as_ref()
    }

    #[cfg(test)]
    pub(super) fn asked(&self) -> usize {
        self.asked
    }

    #[cfg(test)]
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

fn nets_word(nets: Option<&[String]>) -> String {
    match nets {
        Some(n) if !n.is_empty() => n.join(", "),
        _ => msg::note(msg::N_USER_NETS_ALL, &[]),
    }
}

fn roles_word(roles: Option<&BTreeMap<String, String>>) -> String {
    match roles {
        Some(r) if !r.is_empty() => r.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", "),
        _ => msg::note(msg::N_CFG_NONE, &[]),
    }
}

/// The token's state in the words `koyomi user list` uses.
fn token_word(l: &Listed) -> String {
    let secret = l.user.secret.as_deref().unwrap_or_default();
    match l.token {
        Token::Ok => msg::note(msg::N_USER_TOKEN_OK, &[]),
        Token::Hash => msg::note(msg::N_USER_TOKEN_HASH, &[]),
        Token::Missing => msg::note(msg::N_USER_TOKEN_MISSING, &[secret]),
        Token::Unreadable => msg::note(msg::N_USER_TOKEN_UNREADABLE, &[secret]),
        Token::None => msg::note(msg::N_USER_TOKEN_NONE, &[]),
    }
}

/// A role's one-liner without its leading number.
fn meaning(role: Role) -> String {
    let n = match role {
        Role::View => msg::N_USER_ROLE_VIEW,
        Role::Run => msg::N_USER_ROLE_RUN,
        Role::Edit => msg::N_USER_ROLE_EDIT,
        Role::Admin => msg::N_USER_ROLE_ADMIN,
    };
    let s = msg::note(n, &[]);
    s.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start().to_owned()
}

/// A user's name as a part of a test ID: lowercase letters, digits, `-` and `_`.
pub(super) fn tid(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "user".to_owned()
    } else {
        s
    }
}

pub(super) fn say_line(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    let r = ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap());
    ito_a11y::say(&r, ito_a11y::Role::Label, text);
}

/// A button that may be off; when it is off for a reason, the reason shows on hover.
pub(super) fn button(
    ui: &mut egui::Ui,
    on: bool,
    why: Option<&str>,
    mark: ito_prefs::Mark,
    words: &str,
    tone: ito_prefs::Tone,
    id: &str,
) -> bool {
    let r = ui.add_enabled_ui(on, |ui| ito_prefs::IconButton::new(mark, words).tone(tone).show(ui)).inner;
    ito_a11y::test_id(&r, id);
    if !on {
        if let Some(w) = why {
            r.on_disabled_hover_text(w);
        }
        return false;
    }
    r.clicked()
}

/// Draws the page and returns the one click of this frame.
pub(super) fn show(u: &mut Users, ui: &mut egui::Ui, t: &ito_i18n::Table, v: &View, access: &Access) -> Option<Click> {
    let c = ito_theme::colors();
    let mut click: Option<Click> = None;
    // A token on the screen owns the keys: only its box reads Esc.
    let (esc, enter, tab) = if !u.holds_token() && (u.confirm.is_some() || u.wants_esc()) {
        ui.ctx().input_mut(|i| {
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            let asking = u.confirm.is_some();
            let enter = asking && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let tab =
                asking && (i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) || i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
            (esc, enter, tab)
        })
    } else {
        (false, false, false)
    };
    if esc {
        click = Some(Click::Esc);
    } else if enter {
        click = u.confirm.as_ref().map(|q| if q.focus == 0 { Click::Yes } else { Click::No });
    } else if tab {
        click = Some(Click::MoveFocus);
    }
    let heading = match v.far {
        Some(n) => t.trf("users.heading_far", &[n]),
        None => t.tr("users.heading_this").to_owned(),
    };
    let page = ui.vertical(|ui| {
        ui.label(egui::RichText::new(&heading).strong());
        ui.add_space(4.0);
        match access {
            Access::Wait => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t.tr("users.loading_far"));
                });
                return;
            }
            Access::NotAdmin => {
                say_line(ui, t.tr("users.only_admin"), c.dim);
                return;
            }
            Access::Down(e) => say_line(ui, e, c.err),
            Access::Open => {}
        }
        // The toolbar.
        let idle = v.open && u.write.is_none() && u.asked == 0;
        ui.horizontal(|ui| {
            if button(ui, idle, None, ito_prefs::Mark::Refresh, t.tr("users.reload"), ito_prefs::Tone::Plain, "users.reload") {
                click.get_or_insert(Click::Reload);
            }
            let can_add = u.add.is_none() && v.open && u.write.is_none();
            if button(ui, can_add, None, ito_prefs::Mark::Plus, t.tr("users.add"), ito_prefs::Tone::Plain, "users.add.open") {
                click.get_or_insert(Click::OpenAdd);
            }
            if u.asked > 0 {
                ui.spinner();
                ui.label(t.tr("users.loading"));
            }
        });
        if let Some(e) = &u.error {
            say_line(ui, e, c.err);
        }
        let can = u.can_write(v);
        if u.add.is_some() {
            add_form(u, ui, t, (can, v.nets), &mut click);
        }
        let mut drawn = false;
        let users: Vec<Listed> = u.listing.as_ref().map(|l| l.user.clone()).unwrap_or_default();
        if u.listing.is_some() && users.is_empty() && u.error.is_none() && u.asked == 0 {
            say_line(ui, &msg::note(msg::N_USER_NONE, &[]), c.dim);
        }
        for l in &users {
            let name = &l.user.name;
            let locked = Users::locked(v, name);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let head = if v.who == Some(name.as_str()) { format!("{name} {}", t.tr("users.you")) } else { name.clone() };
                ui.label(egui::RichText::new(head).strong());
                let nets = nets_word(l.user.nets.as_deref());
                let roles = roles_word(l.user.roles.as_ref());
                for (label, text) in [
                    (msg::N_USER_LABEL_ROLE, l.user.role.clone()),
                    (msg::N_USER_LABEL_NETS, nets),
                    (msg::N_USER_LABEL_ROLES, roles),
                    (msg::N_USER_LABEL_TOKEN, token_word(l)),
                ] {
                    let line = format!("{}: {text}", msg::note(label, &[]));
                    let r = ui.add(egui::Label::new(line.as_str()).wrap());
                    ito_a11y::say(&r, ito_a11y::Role::Label, &line);
                }
                let editing = u.edit.as_ref().is_some_and(|e| &e.name == name);
                if editing {
                    drawn = true;
                    edit_form(u, ui, t, (can, v.nets), &mut click);
                } else {
                    let id = tid(name);
                    let why = locked.then(|| t.tr("users.self_locked"));
                    let on = can && !locked && !u.edit.as_ref().is_some_and(Edit::dirty);
                    ui.horizontal_wrapped(|ui| {
                        if button(
                            ui,
                            on,
                            why,
                            ito_prefs::Mark::Pencil,
                            t.tr("users.edit"),
                            ito_prefs::Tone::Plain,
                            &format!("users.edit.{id}"),
                        ) {
                            click.get_or_insert(Click::OpenEdit(name.clone()));
                        }
                        let on = can && !locked;
                        if button(
                            ui,
                            on,
                            why,
                            ito_prefs::Mark::Lock,
                            t.tr("users.reset"),
                            ito_prefs::Tone::Plain,
                            &format!("users.reset.{id}"),
                        ) {
                            click.get_or_insert(Click::AskReset(name.clone()));
                        }
                        if button(
                            ui,
                            on,
                            why,
                            ito_prefs::Mark::Trash,
                            t.tr("users.remove"),
                            ito_prefs::Tone::Danger,
                            &format!("users.remove.{id}"),
                        ) {
                            click.get_or_insert(Click::AskRemove(name.clone()));
                        }
                    });
                }
            });
        }
        // An editor with typed changes whose card is not there (the listing changed): it still shows.
        if u.edit.is_some() && !drawn {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                edit_form(u, ui, t, (can, v.nets), &mut click);
            });
        }
        if let Some(line) = &u.refusal {
            ui.add_space(6.0);
            say_line(ui, &line.text(t), c.err);
        }
    });
    ito_a11y::test_id(&page.response, "users.page");
    if let Some(q) = &u.confirm {
        let (title, body, yes) = match q.kind {
            Kind::Reset => (t.tr("users.reset_title"), t.trf("users.reset_body", &[&q.name]), t.tr("users.reset_yes")),
            _ => (t.tr("users.remove_title"), t.trf("users.remove_body", &[&q.name]), t.tr("users.remove_yes")),
        };
        let buttons = [yes.to_owned(), t.tr("users.cancel").to_owned()];
        let mark = if q.kind == Kind::Reset { ito_prefs::Mark::Lock } else { ito_prefs::Mark::Trash };
        let body = [body];
        let question = ito_confirm::Question {
            title,
            body: &body,
            buttons: &buttons,
            marks: &[Some(mark), Some(ito_prefs::Mark::Close)],
            focus: Some(q.focus),
            primary: Some(0),
            danger: true,
            ..Default::default()
        };
        if let Some(choice) = ito_confirm::show(ui.ctx(), &question, &super::Browser::style()) {
            click = Some(if choice == 0 { Click::Yes } else { Click::No });
        }
    }
    click
}

/// One checkbox of the nets chooser: `ticked` is what it shows, `on` whether it can be clicked. Returns whether it
/// was clicked.
fn net_check(ui: &mut egui::Ui, (ticked, on): (bool, bool), words: &str, hover: &str, test_id: &str, salt: &str) -> bool {
    let mut value = ticked;
    let r = ui
        .push_id(salt, |ui| ui.add_enabled(on, egui::Checkbox::new(&mut value, words)))
        .inner
        .on_hover_text(hover)
        .on_disabled_hover_text(hover);
    ito_a11y::test_id(&r, test_id);
    r.changed()
}

fn nets_row(ui: &mut egui::Ui, t: &ito_i18n::Table, nets: &mut NetPick, ids: &[String], admin: bool, id: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(t.tr("users.nets_field"));
        if admin {
            // An admin has every net: both radios are off.
            let why = t.tr("users.nets_admin");
            let r = ui.add_enabled(false, egui::RadioButton::new(false, t.tr("users.nets_all"))).on_disabled_hover_text(why);
            ito_a11y::test_id(&r, &format!("{id}.all"));
            let r = ui.add_enabled(false, egui::RadioButton::new(false, t.tr("users.nets_only"))).on_disabled_hover_text(why);
            ito_a11y::test_id(&r, &format!("{id}.only"));
            ui.label(egui::RichText::new(nets_word(None)).color(ito_theme::colors().dim)).on_hover_text(why);
            return;
        }
        let r = ui.radio(nets.all, t.tr("users.nets_all"));
        ito_a11y::test_id(&r, &format!("{id}.all"));
        if r.clicked() && !nets.all {
            nets.choose_all();
        }
        let r = ui.radio(!nets.all, t.tr("users.nets_only"));
        ito_a11y::test_id(&r, &format!("{id}.only"));
        if r.clicked() && nets.all {
            nets.all = false;
        }
    });
    if admin || nets.all {
        return;
    }
    let dim = ito_theme::colors().dim;
    let groups = offered(ids);
    egui::ScrollArea::vertical().id_salt(format!("{id}.scroll")).max_height(240.0).show(ui, |ui| {
        // What is chosen but not offered here: kept until it is unticked.
        for (text, why) in kept_entries(&nets.chosen, ids) {
            let note = match why {
                Kept::Pattern => t.tr("users.nets_kept_pattern"),
                Kept::Gone => t.tr("users.nets_kept_gone"),
                Kept::Unchecked => t.tr("users.nets_kept_unchecked"),
            };
            ui.horizontal(|ui| {
                if net_check(ui, (true, true), &text, &text, &format!("{id}.kept.{}", net_code(&text)), &format!("{id}.kept.{text}")) {
                    nets.untick(&text);
                }
                ui.label(egui::RichText::new(note).color(dim));
            });
        }
        for (group, members) in &groups {
            let pattern = group_pattern(group);
            let covered = nets.has(&pattern);
            let words = t.trf("users.nets_group", &[group]);
            if net_check(ui, (covered, true), &words, &pattern, &format!("{id}.group.{}", net_code(group)), &format!("{id}.group.{group}"))
            {
                if covered {
                    nets.untick(&pattern);
                } else {
                    nets.tick_group(group);
                }
            }
            ui.indent(format!("{id}.members.{group}"), |ui| {
                for member in members {
                    let name = member.split_once('/').map_or(member.as_str(), |(_, n)| n);
                    let ticked = covered || nets.has(member);
                    let test_id = format!("{id}.net.{}", net_code(member));
                    if net_check(ui, (ticked, !covered), name, member, &test_id, &format!("{id}.net.{member}")) {
                        if ticked {
                            nets.untick(member);
                        } else {
                            nets.tick(member);
                        }
                    }
                }
            });
        }
        if groups.is_empty() {
            ui.label(egui::RichText::new(t.tr("users.nets_no_list")).color(dim));
        }
    });
    if nets.missing() {
        ui.label(egui::RichText::new(t.tr("users.nets_need")).color(ito_theme::colors().err));
    }
}

fn role_radios(ui: &mut egui::Ui, current: &mut String, id: &str) -> bool {
    let mut changed = false;
    for r in ROLES {
        let resp = ui.radio(*current == r.name(), meaning(r));
        ito_a11y::test_id(&resp, &format!("{id}.role_{}", r.name()));
        if resp.clicked() && *current != r.name() {
            *current = r.name().to_owned();
            changed = true;
        }
    }
    changed
}

fn stale_row(ui: &mut egui::Ui, t: &ito_i18n::Table, note: &str, id: &str, can_see: bool, click: &mut Option<Click>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(t.tr(note)).color(ito_theme::colors().err));
        if button(ui, can_see, None, ito_prefs::Mark::Check, t.tr("users.seen"), ito_prefs::Tone::Plain, id) {
            click.get_or_insert(Click::Seen);
        }
    });
}

fn edit_form(u: &mut Users, ui: &mut egui::Ui, t: &ito_i18n::Table, (can, ids): (bool, &[String]), click: &mut Option<Click>) {
    let can_see = can;
    let busy = u.write.is_some();
    let Some(e) = u.edit.as_mut() else { return };
    let id = format!("users.edit.{}", tid(&e.name));
    ui.separator();
    // While a write is out the inputs are off: the answer closes the editor that was sent.
    ui.add_enabled_ui(!busy, |ui| {
        if role_radios(ui, &mut e.role, &id) {
            e.normalize();
        }
        let admin = e.is_admin();
        nets_row(ui, t, &mut e.nets, ids, admin, &format!("{id}.nets"));
    });
    let roles = roles_word(e.roles.as_ref());
    ui.label(format!("{}: {roles}", msg::note(msg::N_USER_LABEL_ROLES, &[])));
    ui.add(egui::Label::new(egui::RichText::new(t.tr("users.edit_roles_note")).color(ito_theme::colors().dim)).wrap());
    let (dirty, stale, changed) = (e.dirty(), e.stale, e.changed);
    let need = e.nets_missing();
    if stale {
        stale_row(ui, t, "users.stale_note", "users.seen", can_see, click);
    }
    if changed {
        stale_row(ui, t, "users.changed_note", "users.edit.changed_seen", can_see, click);
    }
    ui.horizontal(|ui| {
        if button(
            ui,
            can && dirty && !stale && !changed && !need,
            need.then(|| t.tr("users.nets_need")),
            ito_prefs::Mark::Save,
            t.tr("users.save"),
            ito_prefs::Tone::Primary,
            &format!("{id}.save"),
        ) {
            click.get_or_insert(Click::Save);
        }
        if button(ui, true, None, ito_prefs::Mark::Close, t.tr("users.cancel"), ito_prefs::Tone::Plain, &format!("{id}.cancel")) {
            click.get_or_insert(Click::CloseEdit);
        }
    });
}

fn add_form(u: &mut Users, ui: &mut egui::Ui, t: &ito_i18n::Table, (can, ids): (bool, &[String]), click: &mut Option<Click>) {
    let busy = u.write.is_some();
    let Some(a) = u.add.as_mut() else { return };
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // While a write is out the inputs are off: the answer closes the form that was sent.
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                ui.label(t.tr("users.name_field"));
                let r = ui.add(egui::TextEdit::singleline(&mut a.name).desired_width(200.0));
                ito_a11y::say(&r, ito_a11y::Role::Label, t.tr("users.name_field"));
                ito_a11y::test_id(&r, "users.add.name");
            });
            if role_radios(ui, &mut a.role, "users.add") && a.is_admin() {
                a.nets = NetPick::every();
            }
            let admin = a.is_admin();
            nets_row(ui, t, &mut a.nets, ids, admin, "users.add.nets");
        });
        if let Some(why) = &a.error {
            say_line(ui, why, ito_theme::colors().err);
        }
        if a.stale {
            stale_row(ui, t, "users.stale_note", "users.seen", can, click);
        }
        let stale = a.stale;
        let need = a.nets_missing();
        ui.horizontal(|ui| {
            if button(
                ui,
                can && !stale && !need,
                need.then(|| t.tr("users.nets_need")),
                ito_prefs::Mark::Plus,
                t.tr("users.create"),
                ito_prefs::Tone::Primary,
                "users.add.create",
            ) {
                click.get_or_insert(Click::Create);
            }
            if button(ui, true, None, ito_prefs::Mark::Close, t.tr("users.cancel"), ito_prefs::Tone::Plain, "users.add.cancel") {
                click.get_or_insert(Click::CloseAdd);
            }
        });
    });
}

/// The box that shows a new token once. Returns `true` when it is to close (閉じる, or Esc).
pub(super) fn token_show(issued: &Issued, ctx: &egui::Context, t: &ito_i18n::Table) -> bool {
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut close = esc;
    egui::Window::new(t.tr("users.token_title"))
        .id(egui::Id::new("users-token"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(t.trf("users.token_for", &[&issued.name]));
            ui.add_space(6.0);
            // No test ID and no accessible name of our own on the token: only the label that shows it.
            ui.add(egui::Label::new(egui::RichText::new(&issued.token).monospace().strong()).selectable(true).wrap());
            ui.add_space(6.0);
            if !issued.note.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(&issued.note).color(ito_theme::colors().dim)).wrap());
                ui.add_space(6.0);
            }
            ui.horizontal(|ui| {
                if button(ui, true, None, ito_prefs::Mark::Copy, t.tr("users.copy"), ito_prefs::Tone::Plain, "users.token.copy") {
                    ctx.copy_text(issued.token.clone());
                }
                if button(ui, true, None, ito_prefs::Mark::Close, t.tr("users.close"), ito_prefs::Tone::Primary, "users.token.close") {
                    close = true;
                }
            });
        });
    close
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = "k3y-FAKE-TOKEN-0123456789";

    fn user(name: &str, role: &str) -> Listed {
        Listed {
            user: koyomi_core::users::User { name: name.into(), role: role.into(), nets: None, roles: None, tailscale: None, secret: None },
            token: Token::Hash,
        }
    }

    fn listing_text(base: &str, names: &[(&str, &str)]) -> String {
        Listing { base: base.into(), user: names.iter().map(|(n, r)| user(n, r)).collect() }.to_toml()
    }

    fn view<'a>(far: Option<&'a str>, who: Option<&'a str>) -> View<'a> {
        View { far, who, open: true, nets: &[] }
    }

    /// A page that asked for the list and was answered.
    fn loaded(base: &str, names: &[(&str, &str)]) -> Users {
        let mut u = Users::default();
        u.touch(1);
        u.ask();
        u.listing_answer(Ok(listing_text(base, names)));
        u
    }

    fn reply(name: &str) -> String {
        format!("name = \"{name}\"\ntoken = \"{FAKE}\"\nnote = \"shown once\"\n")
    }

    fn sent(d: Do) -> Request {
        match d {
            Do::Send(r) => r,
            Do::Ask => panic!("a read where a write was expected"),
            Do::Nothing => panic!("nothing was sent"),
        }
    }

    #[test]
    fn access_follows_the_link_and_the_role() {
        assert_eq!(access(false, None, None), Access::Open);
        assert_eq!(access(true, None, None), Access::Wait);
        assert_eq!(access(true, Some(Role::Admin), None), Access::Open);
        assert_eq!(access(true, Some(Role::Edit), None), Access::NotAdmin);
        assert_eq!(access(true, Some(Role::Admin), Some("gone")), Access::Down("gone".into()));
        assert_eq!(access(true, Some(Role::Run), Some("gone")), Access::NotAdmin);
        assert_eq!(access(true, Some(Role::Admin), Some("")), Access::Open);
    }

    #[test]
    fn an_older_answer_does_not_overwrite_a_newer_one() {
        let mut u = Users::default();
        u.touch(1);
        u.ask();
        u.ask();
        assert_eq!(u.asked(), 2);
        u.listing_answer(Ok(listing_text("old", &[("a", "view")])));
        assert!(u.listing.is_none(), "taken only when none is left to come");
        u.listing_answer(Ok(listing_text("new", &[("a", "view"), ("b", "run")])));
        assert_eq!(u.listing.as_ref().map(|l| l.base.as_str()), Some("new"));
    }

    #[test]
    fn a_failed_answer_keeps_the_old_listing_and_says_why() {
        let mut u = loaded("b1", &[("a", "admin")]);
        u.ask();
        u.listing_answer(Err("KOYOMI-E1111 nope".into()));
        assert_eq!(u.error(), Some("KOYOMI-E1111 nope"));
        assert_eq!(u.listing.as_ref().map(|l| l.base.as_str()), Some("b1"));
        assert!(!u.can_write(&view(None, None)), "no write while the last read failed");
    }

    #[test]
    fn the_state_goes_from_loading_to_list_to_editor_to_saving_and_back() {
        let mut u = Users::default();
        u.touch(1);
        assert!(u.need);
        u.ask();
        assert!(!u.ready(&view(None, None)), "loading");
        u.listing_answer(Ok(listing_text("b1", &[("kei", "view"), ("mei", "admin")])));
        let v = view(None, None);
        assert!(u.ready(&v));
        assert!(matches!(u.act(Click::OpenEdit("kei".into()), &v), Do::Nothing));
        assert!(u.edit.is_some());
        // Nothing changed: nothing to save.
        assert!(matches!(u.act(Click::Save, &v), Do::Nothing));
        u.edit.as_mut().unwrap().role = "run".into();
        let req = sent(u.act(Click::Save, &v));
        assert!(matches!(&req, Request::UserSet { name, role, base, .. } if name == "kei" && role == "run" && base == "b1"));
        assert_eq!(u.write().map(|w| w.kind), Some(Kind::Set));
        assert!(!u.ready(&v), "saving");
        let done = u.answered(&req, Ok("Changed kei".into()));
        assert!(done.reload && matches!(done.result, Some(Ok(_))));
        assert!(u.edit.is_none() && u.write().is_none());
    }

    fn pick(chosen: &[&str]) -> NetPick {
        NetPick { all: false, chosen: chosen.iter().map(|s| (*s).to_owned()).collect() }
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_net_pick_comes_from_none_or_a_list_and_goes_back_to_the_same_request() {
        assert_eq!(NetPick::of(None), NetPick::every());
        assert_eq!(NetPick::of(None).norm(), None);
        let listed = strs(&["b/y", "a/x", "b/y"]);
        let p = NetPick::of(Some(&listed));
        assert!(!p.all);
        assert_eq!(p.norm(), Some(listed), "order and duplicates are kept");
        let mut l = user("kei", "view");
        l.user.nets = Some(strs(&["b/y", "a/x", "b/y"]));
        let e = Edit::of(&l);
        assert!(!e.dirty(), "an untouched edit is not dirty");
        assert!(matches!(e.request("b"), Request::UserSet { nets: Some(n), .. } if n == ["b/y", "a/x", "b/y"]));
    }

    #[test]
    fn all_sends_none_and_only_with_a_list_sends_it_in_order() {
        let mut e = Edit::of(&user("kei", "view"));
        assert!(!e.dirty());
        e.nets = pick(&["b/y", "a/x"]);
        assert!(e.dirty());
        assert!(matches!(e.request("b"), Request::UserSet { nets: Some(n), .. } if n == ["b/y", "a/x"]));
        e.nets.choose_all();
        assert!(e.nets.chosen.is_empty(), "すべて keeps nothing aside");
        assert!(!e.dirty(), "going back to the original すべて is not dirty");
        assert!(matches!(e.request("b"), Request::UserSet { nets: None, .. }));
    }

    #[test]
    fn choosing_all_clears_the_chosen_and_a_listed_list_is_dirty_against_all() {
        let mut l = user("kei", "view");
        l.user.nets = Some(strs(&["a/x"]));
        let mut e = Edit::of(&l);
        e.nets.choose_all();
        assert!(e.nets.chosen.is_empty() && e.dirty());
        assert!(matches!(e.request("b"), Request::UserSet { nets: None, .. }));
    }

    #[test]
    fn admin_forces_all_and_drops_the_nets_and_per_net_roles() {
        let mut e = Edit::of(&user("kei", "view"));
        e.nets = pick(&["a/x"]);
        e.roles = Some(BTreeMap::from([("a/x".into(), "run".into())]));
        e.role = "admin".into();
        e.normalize();
        assert!(e.nets == NetPick::every() && e.roles.is_none());
        assert!(!e.nets_missing());
        assert!(matches!(e.request("b"), Request::UserSet { nets: None, roles: None, .. }));
        let mut a = Add::new();
        a.nets = pick(&["a/x"]);
        a.role = "admin".into();
        assert!(!a.nets_missing());
        assert!(matches!(a.request("mei".into(), "b"), Request::UserAdd { nets: None, .. }));
    }

    #[test]
    fn only_with_nothing_ticked_is_refused_by_save_and_create() {
        let v = view(None, None);
        let mut u = loaded("b1", &[("kei", "view")]);
        opened(&mut u, "kei");
        u.edit.as_mut().unwrap().nets = pick(&[]);
        assert!(u.edit.as_ref().unwrap().dirty() && u.edit.as_ref().unwrap().nets_missing());
        assert!(matches!(u.act(Click::Save, &v), Do::Nothing));
        assert!(u.write().is_none(), "no write starts");
        u.edit.as_mut().unwrap().nets = pick(&["a/x"]);
        assert!(matches!(sent(u.act(Click::Save, &v)), Request::UserSet { nets: Some(_), .. }));
        let mut u = loaded("b1", &[("kei", "view")]);
        u.act(Click::OpenAdd, &v);
        let a = u.add.as_mut().unwrap();
        a.name = "mei".into();
        assert!(matches!(a.request("mei".into(), "b1"), Request::UserAdd { nets: None, .. }), "all sends none");
        a.nets = pick(&[]);
        assert!(a.dirty());
        assert!(matches!(u.act(Click::Create, &v), Do::Nothing));
        assert!(u.write().is_none(), "no write starts");
        u.add.as_mut().unwrap().nets = pick(&["a/*"]);
        assert!(matches!(sent(u.act(Click::Create, &v)), Request::UserAdd { nets: Some(n), .. } if n == ["a/*"]));
    }

    #[test]
    fn ticking_a_new_group_drops_its_single_ids_and_unticking_keeps_a_hand_written_one() {
        let mut p = pick(&["経理/日次", "人事/a", "経理/日*"]);
        p.tick_group("経理");
        assert_eq!(p.chosen, ["人事/a", "経理/日*", "経理/*"], "the group's single ids go; patterns and other groups stay");
        // A hand-written id next to the group stays when the group is unticked.
        let mut p = pick(&["経理/*", "経理/日次"]);
        p.untick("経理/*");
        assert_eq!(p.chosen, ["経理/日次"]);
        assert!(p.has("経理/日次"));
        // Untouched, `["経理/*", "経理/日次"]` is not dirty.
        let mut l = user("kei", "view");
        l.user.nets = Some(strs(&["経理/*", "経理/日次"]));
        assert!(!Edit::of(&l).dirty());
    }

    #[test]
    fn ticking_a_net_appends_and_unticking_removes_every_copy() {
        let mut p = pick(&["a/x", "b/y", "a/x"]);
        p.tick("c/z");
        p.tick("c/z");
        assert_eq!(p.chosen, ["a/x", "b/y", "a/x", "c/z"]);
        p.untick("a/x");
        assert_eq!(p.chosen, ["b/y", "c/z"]);
    }

    #[test]
    fn unticking_and_ticking_again_is_not_a_change() {
        let mut l = user("kei", "view");
        l.user.nets = Some(strs(&["a/x", "b/y"]));
        let mut e = Edit::of(&l);
        e.nets.untick("a/x");
        assert!(e.dirty());
        e.nets.tick("a/x");
        assert_eq!(e.nets.chosen, ["b/y", "a/x"]);
        assert!(!e.dirty(), "the same nets in another order reach the same nets");
    }

    #[test]
    fn what_the_list_does_not_offer_is_kept_with_its_note_until_unticked() {
        let ids = strs(&["経理/日次", "経理/月次", "人事/a"]);
        let chosen = strs(&["*/日次", "経理/*", "経理/gone", "人事/a", "*/日次", "経理/月次"]);
        assert_eq!(kept_entries(&chosen, &ids), [("*/日次".to_owned(), Kept::Pattern), ("経理/gone".to_owned(), Kept::Gone)]);
        // Before the list came, a plain id is not yet checked; a pattern is still a pattern.
        assert_eq!(
            kept_entries(&strs(&["経理/日次", "*/日次"]), &[]),
            [("経理/日次".to_owned(), Kept::Unchecked), ("*/日次".to_owned(), Kept::Pattern)]
        );
        // A group that is not in the list is a pattern with a `*`.
        assert_eq!(kept_entries(&strs(&["gone/*"]), &ids), [("gone/*".to_owned(), Kept::Pattern)]);
        // The list arriving changes nothing in `chosen`: the pick is not touched by the listing of nets.
        let p = pick(&["経理/gone"]);
        let before = p.clone();
        let _ = (kept_entries(&p.chosen, &[]), kept_entries(&p.chosen, &ids));
        assert_eq!(p, before);
        let mut p = p;
        p.untick("経理/gone");
        assert!(kept_entries(&p.chosen, &ids).is_empty());
    }

    #[test]
    fn the_offered_list_is_sorted_by_group_and_by_id_and_skips_ids_with_no_group() {
        let ids = strs(&["経理/b", "人事/a", "経理/a", "plain"]);
        assert_eq!(offered(&ids), [("人事".to_owned(), strs(&["人事/a"])), ("経理".to_owned(), strs(&["経理/a", "経理/b"]))]);
    }

    #[test]
    fn the_test_id_code_tells_groups_apart_where_tid_does_not() {
        assert_eq!(tid("経理"), tid("総務"));
        assert_ne!(net_code("経理"), net_code("総務"));
        assert_eq!(net_code("ab"), "6162");
        assert_ne!(net_code("a/x"), net_code("a/y"));
    }

    #[test]
    fn per_net_roles_are_kept_when_the_role_or_nets_change() {
        let mut l = user("kei", "run");
        l.user.roles = Some(BTreeMap::from([("a/x".into(), "edit".into())]));
        let mut e = Edit::of(&l);
        e.role = "view".into();
        e.nets = pick(&["a/x"]);
        assert!(matches!(e.request("b"), Request::UserSet { roles: Some(r), .. } if r.get("a/x").map(String::as_str) == Some("edit")));
    }

    #[test]
    fn the_card_of_who_has_its_buttons_off_on_another_pc_and_on_for_this_one() {
        let mut u = loaded("b1", &[("kei", "admin"), ("mei", "view")]);
        let far = view(Some("lab"), Some("kei"));
        assert!(matches!(u.act(Click::OpenEdit("kei".into()), &far), Do::Nothing) && u.edit.is_none());
        assert!(matches!(u.act(Click::AskReset("kei".into()), &far), Do::Nothing) && u.confirm.is_none());
        assert!(matches!(u.act(Click::AskRemove("kei".into()), &far), Do::Nothing) && u.confirm.is_none());
        // Another user's card works.
        assert!(matches!(u.act(Click::AskRemove("mei".into()), &far), Do::Nothing) && u.confirm.is_some());
        u.confirm = None;
        // This PC: `who` is None there, and a name is not locked.
        let here = view(None, None);
        assert!(matches!(u.act(Click::AskRemove("kei".into()), &here), Do::Nothing) && u.confirm.is_some());
        assert!(Users::locked(&far, "kei") && !Users::locked(&here, "kei") && !Users::locked(&far, "mei"));
    }

    #[test]
    fn remove_and_reset_ask_first_and_send_with_the_base() {
        let mut u = loaded("b9", &[("kei", "run"), ("mei", "admin")]);
        let v = view(None, None);
        u.act(Click::AskRemove("kei".into()), &v);
        assert_eq!(u.confirm.as_ref().map(|c| c.focus), Some(1), "starts on cancel");
        assert!(matches!(u.act(Click::No, &v), Do::Nothing) && u.confirm.is_none());
        u.act(Click::AskRemove("kei".into()), &v);
        let req = sent(u.act(Click::Yes, &v));
        assert!(matches!(&req, Request::UserRemove { name, base } if name == "kei" && base == "b9"));
        u.answered(&req, Ok("Removed".into()));
        u.ask();
        u.listing_answer(Ok(listing_text("b10", &[("mei", "admin")])));
        u.act(Click::AskReset("mei".into()), &v);
        let req = sent(u.act(Click::Yes, &v));
        assert!(matches!(&req, Request::UserReset { name, base } if name == "mei" && base == "b10"));
    }

    #[test]
    fn the_add_form_refuses_a_bad_name_before_sending() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::OpenAdd, &v);
        for bad in ["", "   ", "a b", "default", "kei"] {
            u.add.as_mut().unwrap().name = bad.into();
            assert!(matches!(u.act(Click::Create, &v), Do::Nothing), "{bad:?}");
            assert!(u.add.as_ref().unwrap().error.is_some(), "{bad:?}");
            assert!(u.write().is_none());
        }
        u.add.as_mut().unwrap().name = "  mei ".into();
        u.add.as_mut().unwrap().nets = pick(&["a/x"]);
        let req = sent(u.act(Click::Create, &v));
        assert!(matches!(&req, Request::UserAdd { name, role, nets: Some(n), roles: None, base }
            if name == "mei" && role == "view" && n == &["a/x"] && base == "b1"));
        assert_eq!(u.write().map(|w| w.kind), Some(Kind::Add));
    }

    #[test]
    fn a_new_token_opens_the_box_and_closing_it_drops_the_token() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::OpenAdd, &v);
        u.add.as_mut().unwrap().name = "mei".into();
        let req = sent(u.act(Click::Create, &v));
        assert!(!u.holds_token() && u.tokens_opened() == 0);
        let done = u.answered(&req, Ok(reply("mei")));
        assert!(done.reload && u.add.is_none());
        assert!(u.holds_token() && u.unsaved() && u.tokens_opened() == 1);
        assert_eq!(u.token().map(Issued::token), Some(FAKE));
        let said = match done.result {
            Some(Ok(s)) => s,
            _ => panic!("a success line"),
        };
        assert!(said.contains("mei") && !said.contains(FAKE), "{said}");
        u.close_token();
        assert!(!u.holds_token() && !u.unsaved());
        assert_eq!(u.tokens_opened(), 1, "the count only grows");
    }

    #[test]
    fn a_reset_reply_opens_the_box_too() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::AskReset("kei".into()), &v);
        let req = sent(u.act(Click::Yes, &v));
        let done = u.answered(&req, Ok(reply("kei")));
        assert!(u.holds_token());
        assert!(matches!(&done.result, Some(Ok(s)) if s.contains("kei") && !s.contains(FAKE)));
    }

    #[test]
    fn an_unparsable_reply_holds_no_token_anywhere() {
        let mut u = loaded("b1", &[]);
        let v = view(None, None);
        u.act(Click::OpenAdd, &v);
        u.add.as_mut().unwrap().name = "mei".into();
        let req = sent(u.act(Click::Create, &v));
        let bad = format!("name = \"mei\"\ntoken = \"{FAKE}\nnote = ");
        let done = u.answered(&req, Ok(bad));
        assert!(!u.holds_token());
        assert!(done.result.is_none());
        let line = u.refusal().expect("a line").clone();
        assert_eq!(line, Line::ReplyBad("mei".into()));
        let shown = format!("{line:?} {:?}", u.refusal());
        assert!(!shown.contains(FAKE) && !shown.contains("expected"), "{shown}");
        // A reply with no token is the same.
        assert!(Issued::parse("name = \"x\"\ntoken = \"\"\n").is_none());
    }

    #[test]
    fn issued_prints_no_token() {
        let i = Issued::parse(&reply("mei")).unwrap();
        let shown = format!("{i:?}");
        assert!(shown.contains("mei") && shown.contains("***") && !shown.contains(FAKE), "{shown}");
    }

    #[test]
    fn a_broken_link_on_add_or_reset_says_it_may_have_been_done_and_does_not_retry() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::OpenAdd, &v);
        u.add.as_mut().unwrap().name = "mei".into();
        let req = sent(u.act(Click::Create, &v));
        let done = u.answered(&req, Err("connection reset".into()));
        assert!(done.reload && done.result.is_none() && !u.holds_token());
        assert_eq!(u.refusal(), Some(&Line::UnsureAdd("mei".into())));
        assert!(u.write().is_none());
        let req = Request::UserReset { name: "kei".into(), base: "b1".into() };
        u.answered(&req, Err("connection reset".into()));
        assert_eq!(u.refusal(), Some(&Line::UnsureReset("kei".into())));
        // A refusal of the engine is its own words.
        u.answered(&req, Err("KOYOMI-E2101 refused".into()));
        assert_eq!(u.refusal(), Some(&Line::Raw("KOYOMI-E2101 refused".into())));
    }

    #[test]
    fn after_the_file_changed_elsewhere_the_editor_stays_and_nothing_is_resent() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::OpenEdit("kei".into()), &v);
        u.edit.as_mut().unwrap().role = "run".into();
        let req = sent(u.act(Click::Save, &v));
        let err = format!("{} changed", msg::USER_BASE_CHANGED.code());
        let done = u.answered(&req, Err(err));
        assert!(done.reload, "the list is read again");
        assert_eq!(u.refusal(), Some(&Line::BaseChanged));
        assert!(u.edit.as_ref().is_some_and(|e| e.role == "run"), "what was typed stays");
        assert!(u.write().is_none());
        assert!(!u.need || u.asked() == 0, "nothing is sent by the page itself");
        // The new listing comes, and a click is needed to send again.
        u.ask();
        u.listing_answer(Ok(listing_text("b2", &[("kei", "view")])));
        assert!(u.write().is_none());
        let req = sent(u.act(Click::Save, &v));
        assert!(matches!(&req, Request::UserSet { base, .. } if base == "b2"));
    }

    #[test]
    fn buttons_are_off_while_a_write_is_in_flight() {
        let mut u = loaded("b1", &[("kei", "view"), ("mei", "run")]);
        let v = view(None, None);
        u.act(Click::OpenEdit("kei".into()), &v);
        u.edit.as_mut().unwrap().role = "run".into();
        let _ = sent(u.act(Click::Save, &v));
        assert!(u.write().is_some());
        for c in [
            Click::Save,
            Click::Create,
            Click::AskReset("mei".into()),
            Click::AskRemove("mei".into()),
            Click::OpenEdit("mei".into()),
            Click::Reload,
        ] {
            assert!(matches!(u.act(c.clone(), &v), Do::Nothing), "{c:?}");
        }
        assert!(u.confirm.is_none());
    }

    #[test]
    fn unsaved_follows_the_token_box_and_typed_forms() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        assert!(!u.unsaved());
        u.act(Click::OpenEdit("kei".into()), &v);
        assert!(!u.unsaved(), "an editor with nothing typed asks nothing");
        u.edit.as_mut().unwrap().nets = pick(&["a/x"]);
        assert!(u.unsaved() && u.forms_dirty());
        u.edit.as_mut().unwrap().nets.choose_all();
        assert!(!u.unsaved());
        u.act(Click::OpenAdd, &v);
        assert!(!u.unsaved());
        u.add.as_mut().unwrap().name = "x".into();
        assert!(u.unsaved());
        u.act(Click::CloseAdd, &v);
        u.token = Issued::parse(&reply("x"));
        assert!(u.unsaved() && !u.forms_dirty());
    }

    #[test]
    fn a_change_of_role_or_name_drops_the_listing_and_holds_typed_forms_until_seen() {
        let mut u = loaded("b1", &[("kei", "admin"), ("mei", "view")]);
        assert!(!u.role_told((Some(Role::Admin), Some("kei".into()))), "the first telling is not a change");
        let v = view(Some("lab"), Some("kei"));
        u.act(Click::OpenEdit("mei".into()), &v);
        u.edit.as_mut().unwrap().role = "run".into();
        u.act(Click::OpenAdd, &v);
        assert!(!u.role_told((Some(Role::Admin), Some("kei".into()))), "the same is no change");
        assert!(u.role_told((Some(Role::Admin), Some("sho".into()))));
        assert!(u.listing.is_none() && u.need);
        assert!(u.edit.as_ref().is_some_and(|e| e.stale), "typed changes stay, held");
        assert!(u.add.is_none(), "a pristine form closes");
        // The new listing arrives; the held editor still waits to be looked at.
        u.ask();
        u.listing_answer(Ok(listing_text("b2", &[("mei", "view")])));
        let v = view(Some("lab"), Some("sho"));
        assert!(matches!(u.act(Click::Save, &v), Do::Nothing), "off until seen");
        u.act(Click::Seen, &v);
        assert!(!u.edit.as_ref().unwrap().stale);
        assert!(matches!(u.act(Click::Save, &v), Do::Send(_)));
    }

    #[test]
    fn an_answer_read_under_the_old_identity_is_thrown_away() {
        let mut u = Users::default();
        u.role_told((Some(Role::Admin), Some("kei".into())));
        u.touch(1);
        u.ask();
        assert!(u.role_told((Some(Role::Admin), Some("sho".into()))));
        u.ask();
        u.listing_answer(Ok(listing_text("old", &[("a", "view")])));
        assert!(u.listing.is_none());
        u.listing_answer(Ok(listing_text("new", &[("b", "view")])));
        assert_eq!(u.listing.as_ref().map(|l| l.base.as_str()), Some("new"));
    }

    #[test]
    fn a_new_opening_asks_again_and_closes_pristine_forms() {
        let mut u = loaded("b1", &[("kei", "view")]);
        u.need = false;
        let v = view(None, None);
        u.act(Click::OpenEdit("kei".into()), &v);
        u.touch(2);
        assert!(!u.need && u.edit.is_some(), "the next frame is the same opening");
        assert!(u.is_open(3) && !u.is_open(4));
        u.touch(9);
        assert!(u.need && u.edit.is_none(), "a gap is a new opening");
    }

    #[test]
    fn an_unsent_request_does_not_leave_the_page_waiting_and_keeps_what_was_typed() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::OpenEdit("kei".into()), &v);
        u.edit.as_mut().unwrap().role = "run".into();
        let _ = sent(u.act(Click::Save, &v));
        u.unsent(false, "switching".into());
        assert!(u.write().is_none() && u.edit.as_ref().is_some_and(|e| e.role == "run"));
        assert_eq!(u.refusal(), Some(&Line::Unsent("switching".into())));
        u.ask();
        u.unsent(true, "down".into());
        assert_eq!(u.asked(), 0);
        assert_eq!(u.error(), Some("down"));
    }

    #[test]
    fn esc_closes_the_question_then_the_add_form_then_the_editor() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::OpenEdit("kei".into()), &v);
        u.act(Click::OpenAdd, &v);
        u.act(Click::AskRemove("kei".into()), &v);
        u.act(Click::Esc, &v);
        assert!(u.confirm.is_none() && u.add.is_some() && u.edit.is_some());
        u.act(Click::Esc, &v);
        assert!(u.add.is_none() && u.edit.is_some());
        u.act(Click::Esc, &v);
        assert!(u.edit.is_none() && !u.wants_esc());
    }

    #[test]
    fn test_ids_are_clean_for_any_name() {
        for n in ["kei", "Kei Sato", "ケイ", "", "a.b", "x/y"] {
            let id = format!("users.edit.{}", tid(n));
            assert!(ito_a11y::is_test_id(&id), "{id}");
        }
    }

    #[test]
    fn role_meanings_lose_their_number() {
        for r in ROLES {
            let m = meaning(r);
            assert!(m.starts_with(r.name()), "{m}");
        }
    }

    fn rich(name: &str, role: &str, nets: &[&str], roles: &[(&str, &str)]) -> Listed {
        let mut l = user(name, role);
        l.user.nets = (!nets.is_empty()).then(|| nets.iter().map(|s| (*s).to_owned()).collect());
        l.user.roles = (!roles.is_empty()).then(|| roles.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect());
        l
    }

    /// Reads a listing the way the page does: asked, then answered.
    fn relist(u: &mut Users, base: &str, users: Vec<Listed>) {
        u.ask();
        u.listing_answer(Ok(Listing { base: base.into(), user: users }.to_toml()));
    }

    fn opened(u: &mut Users, name: &str) {
        u.act(Click::OpenEdit(name.into()), &view(None, None));
        assert!(u.edit.is_some());
    }

    #[test]
    fn a_new_listing_closes_replaces_or_merges_the_open_editor() {
        let v = view(None, None);
        let base = || rich("kei", "view", &["a"], &[("a", "run")]);
        // Gone: closed.
        let mut u = loaded("b1", &[("kei", "view")]);
        relist(&mut u, "b1", vec![rich("kei", "view", &["a"], &[("a", "run")])]);
        u.edit = Some(Edit::of(&base()));
        relist(&mut u, "b2", vec![user("mei", "view")]);
        assert!(u.edit.is_none());
        // Same: untouched, and not flagged.
        u.edit = Some(Edit::of(&base()));
        u.edit.as_mut().unwrap().nets = pick(&["a", "b"]);
        relist(&mut u, "b3", vec![base()]);
        assert!(u.edit.as_ref().is_some_and(|e| !e.changed && e.nets == pick(&["a", "b"])));
        // Different and pristine: replaced.
        u.edit = Some(Edit::of(&base()));
        relist(&mut u, "b4", vec![rich("kei", "run", &["a"], &[])]);
        let e = u.edit.as_ref().unwrap();
        assert!(!e.changed && !e.dirty() && e.role == "run" && e.roles.is_none());
        // Different and dirty: merged, flagged, save off until seen.
        u.edit = Some(Edit::of(&base()));
        u.edit.as_mut().unwrap().nets = pick(&["a", "b"]);
        relist(&mut u, "b5", vec![rich("kei", "run", &["a"], &[("a", "edit")])]);
        let e = u.edit.as_ref().unwrap();
        assert!(e.changed && e.dirty());
        assert!(matches!(u.act(Click::Save, &v), Do::Nothing), "save waits for Seen");
        u.act(Click::Seen, &v);
        let Request::UserSet { role, nets, roles, base, .. } = sent(u.act(Click::Save, &v)) else { panic!("not a set") };
        assert_eq!((role.as_str(), base.as_str()), ("run", "b5"), "the role changed elsewhere is not carried back");
        assert_eq!(nets, Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(roles.and_then(|r| r.get("a").cloned()).as_deref(), Some("edit"), "the listing's per-net roles");
    }

    #[test]
    fn merge_takes_an_unchanged_pick_keeps_a_changed_one_and_a_fresh_admin_drops_it() {
        // Unchanged pick: the fresh listing's pick is taken.
        let mut u = loaded("b1", &[("kei", "view")]);
        relist(&mut u, "b1", vec![rich("kei", "view", &["a"], &[])]);
        u.edit = Some(Edit::of(&rich("kei", "view", &["a"], &[])));
        u.edit.as_mut().unwrap().role = "run".into();
        relist(&mut u, "b2", vec![rich("kei", "view", &["a", "b"], &[])]);
        let e = u.edit.as_ref().unwrap();
        assert!(e.changed && e.nets == pick(&["a", "b"]) && e.role == "run");
        // Changed pick: kept.
        u.edit = Some(Edit::of(&rich("kei", "view", &["a"], &[])));
        u.edit.as_mut().unwrap().nets = pick(&["z"]);
        relist(&mut u, "b3", vec![rich("kei", "view", &["a", "b"], &[])]);
        assert_eq!(u.edit.as_ref().unwrap().nets, pick(&["z"]));
        // A fresh listing that makes the user admin drops a changed pick: the one intended loss.
        u.edit = Some(Edit::of(&rich("kei", "view", &["a"], &[])));
        u.edit.as_mut().unwrap().nets = pick(&["z"]);
        relist(&mut u, "b4", vec![rich("kei", "admin", &[], &[])]);
        let e = u.edit.as_ref().unwrap();
        assert!(e.is_admin() && e.nets == NetPick::every());
    }

    #[test]
    fn a_typed_role_beats_a_role_changed_elsewhere() {
        let mut u = loaded("b1", &[("kei", "view")]);
        opened(&mut u, "kei");
        u.edit.as_mut().unwrap().role = "edit".into();
        relist(&mut u, "b2", vec![rich("kei", "run", &[], &[])]);
        let e = u.edit.as_ref().unwrap();
        assert!(e.changed);
        assert_eq!(e.role, "edit");
        assert_eq!(e.orig.0, "run");
    }

    #[test]
    fn a_write_in_flight_blocks_a_new_add_form_and_a_down_link_does() {
        let mut u = loaded("b1", &[("kei", "view"), ("mei", "run")]);
        let v = view(None, None);
        u.act(Click::OpenAdd, &v);
        u.add.as_mut().unwrap().name = "alice".into();
        let req = sent(u.act(Click::Create, &v));
        u.act(Click::CloseAdd, &v);
        assert!(u.add.is_none());
        u.act(Click::OpenAdd, &v);
        assert!(u.add.is_none(), "no new form while the write is out");
        // The answer closes the form that was sent.
        let mut u2 = loaded("b1", &[("kei", "view")]);
        u2.act(Click::OpenAdd, &v);
        u2.add.as_mut().unwrap().name = "alice".into();
        let req2 = sent(u2.act(Click::Create, &v));
        u2.answered(&req2, Ok(reply("alice")));
        assert!(u2.add.is_none());
        let _ = req;
        // A link that is down opens no form.
        let mut u3 = loaded("b1", &[("kei", "view")]);
        u3.act(Click::OpenAdd, &View { far: Some("pc"), who: None, open: false, nets: &[] });
        assert!(u3.add.is_none());
    }

    #[test]
    fn a_far_may_have_arrived_line_says_it_may_have_been_done() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let line = msg::line(msg::MAY_HAVE_ARRIVED, &["timed out"]);
        assert!(msg::is_coded(&line));
        let add = Request::UserAdd { name: "mei".into(), role: "view".into(), nets: None, roles: None, base: "b1".into() };
        u.answered(&add, Err(line.clone()));
        assert_eq!(u.refusal(), Some(&Line::UnsureAdd("mei".into())));
        let reset = Request::UserReset { name: "kei".into(), base: "b1".into() };
        u.answered(&reset, Err(line.clone()));
        assert_eq!(u.refusal(), Some(&Line::UnsureReset("kei".into())));
        let remove = Request::UserRemove { name: "kei".into(), base: "b1".into() };
        u.answered(&remove, Err(line.clone()));
        assert_eq!(u.refusal(), Some(&Line::Raw(line)));
    }

    #[test]
    fn a_token_on_the_screen_keeps_esc_from_the_add_form() {
        let mut u = loaded("b1", &[("kei", "view")]);
        let v = view(None, None);
        u.act(Click::OpenAdd, &v);
        u.add.as_mut().unwrap().name = "mei".into();
        u.token = Issued::parse(&reply("kei"));
        assert!(u.holds_token());
        let t = crate::i18n::table("en").0;
        let ctx = egui::Context::default();
        let esc = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput { events: vec![esc(true), esc(false)], ..Default::default() };
        let mut click = None;
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..input };
        let mut out = ctx.run_ui(input, |ui| click = show(&mut u, ui, &t, &v, &Access::Open));
        out.textures_delta.clear();
        assert!(click.is_none(), "the page took no key");
        assert!(u.add.is_some());
        let left = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let _ = left;
    }
}
