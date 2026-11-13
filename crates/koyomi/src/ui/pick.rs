//! Fields that used to be typed and can be picked instead: a net from the engine's list, the weekdays, a
//! number out of a short range, and a path from a file chooser. Typing stays for what a list cannot hold
//! (a glob, `{date}`, a far engine's path, a net the list does not know): a text that does not read as one of the
//! choices keeps its typed field, and the picker comes back as soon as the text reads.
//!
//! Everything that decides something is a plain function here (the text of a field in, the text out), so it is
//! tested without a window. Writing happens only when the person picks.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use ito_a11y::Role;
use ito_filepick::{Done, Listing, Mode, Picker, Style, Ticket, Words};
use ito_i18n::Table;
use koyomi_core::scan::Def;
use koyomi_core::when;

use crate::proto::{PlacesReply, Request};

/// One net of the engine, as a list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetChoice {
    /// `経理/締め`: what the field holds.
    pub id: String,
    /// What the list shows: the id, then the net's title when it has one.
    pub label: String,
}

/// The nets of `defs` as choices, in the order the net list sorts them (by group, then name).
pub fn net_choices(defs: &[Def]) -> Vec<NetChoice> {
    let mut sorted: Vec<&Def> = defs.iter().collect();
    sorted.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
    sorted
        .into_iter()
        .map(|d| match d.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(title) => NetChoice { id: d.id.clone(), label: format!("{}  {title}", d.id) },
            None => NetChoice { id: d.id.clone(), label: d.id.clone() },
        })
        .collect()
}

/// The words a net list needs besides the nets.
pub struct NetWords<'a> {
    /// The choice that writes nothing (an optional field).
    pub none: &'a str,
    /// What an empty required field shows.
    pub choose: &'a str,
    /// Put after a value the list does not hold.
    pub missing: &'a str,
    /// Put after a value that names the net being edited.
    pub own: &'a str,
}

/// The entries of a net list as (value written, words shown), and which one is picked now.
///
/// The net being edited (`own`) is not offered. A value the list does not hold (a net that is gone, or one a
/// narrowed user cannot see) stays as the first choice, as written, with `missing` after it.
pub fn net_entries(nets: &[NetChoice], own: &str, value: &str, optional: bool, w: &NetWords) -> (Vec<(String, String)>, usize) {
    let mut out: Vec<(String, String)> = Vec::new();
    let shown = value.trim();
    if optional {
        out.push((String::new(), w.none.to_owned()));
    } else if shown.is_empty() {
        out.push((String::new(), w.choose.to_owned()));
    }
    let held = nets.iter().any(|n| n.id == shown && n.id != own);
    if !shown.is_empty() && !held {
        let suffix = if shown == own { w.own } else { w.missing };
        out.push((value.to_owned(), format!("{value}{suffix}")));
    }
    out.extend(nets.iter().filter(|n| n.id != own).map(|n| (n.id.clone(), n.label.clone())));
    let now = if shown.is_empty() {
        0
    } else if held {
        out.iter().position(|(v, _)| v == shown).unwrap_or(0)
    } else {
        out.iter().position(|(v, _)| v == value).unwrap_or(0)
    };
    (out, now)
}

/// The weekdays as the `[[when]]` writes them.
pub const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// The days a `days` text names, as seven switches in week order; `None` when some word is not a weekday.
/// An empty text names none.
pub fn days_set(text: &str) -> Option<[bool; 7]> {
    let mut set = [false; 7];
    for word in text.split([',', ' ', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
        let day = when::weekday(word).ok()?;
        set[day.num_days_from_monday() as usize] = true;
    }
    Some(set)
}

/// The text of a set of days: week order, `mon, tue` (what the `[[when]]` writes). No day: empty.
pub fn days_text(set: &[bool; 7]) -> String {
    DAYS.iter().zip(set).filter(|(_, on)| **on).map(|(d, _)| *d).collect::<Vec<_>>().join(", ")
}

/// `set` with day `i` turned the other way.
pub fn days_toggle(set: &[bool; 7], i: usize) -> [bool; 7] {
    let mut out = *set;
    out[i] = !out[i];
    out
}

/// Every day: nothing named (what an empty `days` of `minutes` and `hours` means).
pub fn every_day(set: &[bool; 7]) -> bool {
    set.iter().all(|on| !on)
}

/// How many days month `month` (1 to 12) can have; February has 29 (`02-29` is a date).
pub fn days_in_month(month: u32) -> u32 {
    match month {
        2 => 29,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `04-01` as (month, day), when it reads.
pub fn month_day_parts(text: &str) -> Option<(u32, u32)> {
    when::month_day(text).ok()
}

/// `MM-DD`, with the day kept inside the month.
pub fn month_day_text(month: u32, day: u32) -> String {
    format!("{month:02}-{:02}", day.min(days_in_month(month)).max(1))
}

/// A whole number inside `lo..=hi` that the text is, when it is one.
pub fn number_in(text: &str, lo: u32, hi: u32) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|n| (lo..=hi).contains(n))
}

/// The weekday (0 for Monday) a text names, when it names one.
pub fn weekday_index(text: &str) -> Option<usize> {
    when::weekday(text).ok().map(|d| d.num_days_from_monday() as usize)
}

/// What a chooser is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// One folder.
    Folder,
    /// One file that is there to read (or a folder or a glob, which the person types).
    File,
    /// A file to write.
    Save,
    /// Files to attach, one path a line.
    Files,
}

/// The folder a chooser starts in: the one the field names when it is there (for a file, the folder the file is in).
/// A relative path or one with a glob or a `{date}` in it starts nowhere in particular.
pub fn start_dir(text: &str, want: Want) -> Option<PathBuf> {
    // The attachments are one a line: the last one is what the person was at.
    let line = if want == Want::Files { text.lines().map(str::trim).rfind(|l| !l.is_empty())? } else { text.trim() };
    if line.is_empty() || line.contains(['*', '?', '{']) {
        return None;
    }
    let path = Path::new(line);
    if !path.is_absolute() {
        return None;
    }
    if path.is_dir() {
        return Some(path.to_path_buf());
    }
    path.parent().filter(|p| p.is_dir()).map(Path::to_path_buf)
}

/// The text a chooser's answer makes: one path replaces the field; files are added a line each after what is there.
pub fn browsed_text(now: &str, want: Want, paths: &[String]) -> Option<String> {
    let first = paths.first()?;
    if want != Want::Files {
        return Some(first.clone());
    }
    let mut out = now.trim_end_matches(['\r', '\n']).to_owned();
    for p in paths {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(p);
    }
    Some(out)
}

/// Whether `root` (as `Style::root_of` writes it) is a drive root such as `C:\`.
fn is_drive_root(root: &str) -> bool {
    let b = root.as_bytes();
    b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// Where a far chooser starts and which name it selects (both on the engine's disk), read from the field's text. Lexical
/// only: this PC's disk is never touched. It starts only from a complete path (not empty, no glob, no `{date}`, a root the
/// engine's style reads; on Windows a drive, not a share): anything else starts at the picker's first place.
pub fn far_start(text: &str, want: Want, style: Style) -> (Option<String>, Option<String>) {
    let line = if want == Want::Files {
        match text.lines().map(str::trim).rfind(|l| !l.is_empty()) {
            Some(l) => l,
            None => return (None, None),
        }
    } else {
        text.trim()
    };
    if line.is_empty() || line.contains(['*', '?', '{']) {
        return (None, None);
    }
    let Some(root) = style.root_of(line) else { return (None, None) };
    if style == Style::Windows && !is_drive_root(&root) {
        return (None, None);
    }
    match want {
        Want::Folder => (Some(line.to_owned()), None),
        Want::File | Want::Save => match (style.parent(line), style.last_part(line)) {
            (Some(dir), Some(name)) => (Some(dir), Some(name)),
            _ => (Some(root), None),
        },
        Want::Files => (Some(style.parent(line).unwrap_or(root)), None),
    }
}

/// The next ticket a far chooser's request carries (the engine's answer is matched by it).
fn next_ticket() -> u64 {
    static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
    NEXT_TICKET.fetch_add(1, Ordering::Relaxed)
}

/// What the window gives a far chooser each frame.
pub struct FarLook<'a> {
    pub clock: &'a ito_common::Clock,
    pub lang: &'a str,
}

/// The file chooser that is open (one for the whole window) and the field it was opened for.
#[derive(Default)]
pub struct Browse {
    open: Option<Open>,
    /// The request a far chooser wants sent (taken by [`Browse::request`]).
    out: Option<Request>,
}

struct Open {
    /// The part (or the net) and the field.
    owner: String,
    want: Want,
    how: How,
    /// What the person chose, until the field it was for takes it.
    answer: Option<Vec<String>>,
}

enum How {
    /// The system's own dialog on this PC.
    Local(Receiver<Vec<PathBuf>>),
    /// The engine was asked for its places; the picker is built when they come.
    Places {
        ticket: u64,
        title: String,
        text: String,
        /// Whether the engine runs on Windows, as the snapshot said (for the style when the places do not come).
        windows: Option<bool>,
        /// The words of the waiting box (the picker's own, so the cancel button reads like the picker's).
        asking: String,
        words: Words,
    },
    /// The picker of ito-filepick, listing the engine's disk through `DirList`.
    Far { picker: Box<Picker>, words: Words, asked: Option<(u64, Ticket)> },
}

impl Browse {
    /// A chooser that stays open, for a test.
    #[cfg(test)]
    pub fn pending(owner: &str) -> Browse {
        let (tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(tx);
        Browse { open: Some(Open { owner: owner.to_owned(), want: Want::File, how: How::Local(rx), answer: None }), out: None }
    }

    /// A far chooser that waits for the engine's places, for a test (its `DirPlaces` is in [`Browse::request`]).
    #[cfg(test)]
    pub fn waiting_far(owner: &str, want: Want) -> Browse {
        let mut b = Browse::default();
        b.start_far(owner.to_owned(), want, "path".to_owned(), String::new(), Some(true), "ja", "asking".to_owned());
        b
    }

    /// The far picker's status as text (`Loading`, `Ready`, `Failed("…")`), for a test.
    #[cfg(test)]
    pub fn far_status(&self) -> String {
        match self.open.as_ref().map(|o| &o.how) {
            Some(How::Far { picker, .. }) => format!("{:?}", picker.status()),
            _ => panic!("no far picker"),
        }
    }

    /// Whether a chooser is open.
    pub fn busy(&self) -> bool {
        self.open.is_some()
    }

    /// Whether a far chooser is open and has no answer yet (the window's keys are the picker's then).
    pub fn picking(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.answer.is_none() && matches!(o.how, How::Places { .. } | How::Far { .. }))
    }

    /// Closes a far chooser without an answer; what the engine answers for it later is ignored.
    pub fn cancel(&mut self) {
        if self.open.as_ref().is_some_and(|o| matches!(o.how, How::Places { .. } | How::Far { .. })) {
            self.open = None;
        }
    }

    /// The request a far chooser wants sent, once.
    pub fn request(&mut self) -> Option<Request> {
        self.out.take()
    }

    /// The engine's answer to `req`; true when it was this chooser's.
    pub fn answered(&mut self, req: &Request, r: Result<String, String>) -> bool {
        match req {
            Request::DirPlaces { ticket } => self.places_answered(*ticket, r),
            Request::DirList { ticket, .. } => self.list_answered(*ticket, r),
            _ => false,
        }
    }

    /// `req` could not be sent (`why`): the same as an answer that failed.
    pub fn unsent(&mut self, req: &Request, why: String) {
        self.answered(req, Err(why));
    }

    fn places_answered(&mut self, ticket: u64, r: Result<String, String>) -> bool {
        let Some(open) = &mut self.open else { return false };
        let want = open.want;
        let How::Places { ticket: live, title, text, windows, words, .. } = &open.how else { return false };
        if *live != ticket {
            return false;
        }
        // Without the places the picker still opens, in the style the snapshot hints, and its first listing shows the
        // engine's reason (a role that is not admin, for one).
        let reply = r.ok().and_then(|s| serde_json::from_str::<PlacesReply>(&s).ok());
        let (style, places) = match reply {
            Some(p) => (p.style, p.places),
            None => (if *windows == Some(false) { Style::Posix } else { Style::Windows }, Vec::new()),
        };
        let (start, name) = far_start(text, want, style);
        let mode = match want {
            Want::Folder => Mode::OpenFolder,
            Want::Files => Mode::OpenFiles,
            Want::File => Mode::OpenFile,
            // Only a path is picked: the engine writes the file when the job runs (and checks it again then).
            Want::Save => Mode::SaveFile,
        };
        let mut picker = Picker::new("koyomi.browse", mode, style).title(title).places(places);
        if let Some(s) = start {
            picker = picker.start(&s);
        }
        if let Some(n) = name {
            picker = picker.file_name(&n);
        }
        open.how = How::Far { picker: Box::new(picker), words: words.clone(), asked: None };
        true
    }

    fn list_answered(&mut self, ticket: u64, r: Result<String, String>) -> bool {
        let Some(open) = &mut self.open else { return false };
        let How::Far { picker, asked, .. } = &mut open.how else { return false };
        let Some((n, live)) = *asked else { return false };
        if n != ticket {
            return false;
        }
        *asked = None;
        let listing = r.and_then(|s| serde_json::from_str::<Listing>(&s).map_err(|e| e.to_string()));
        picker.set_listing(live, listing);
        true
    }

    /// Looks for the answer of the open chooser and draws a far one; asks for another frame while it is still open.
    pub fn poll(&mut self, ctx: &egui::Context, look: &FarLook) {
        let Some(open) = &mut self.open else { return };
        if open.answer.is_some() {
            return;
        }
        let mut close = false;
        match &mut open.how {
            How::Local(rx) => match rx.try_recv() {
                Ok(paths) if paths.is_empty() => close = true,
                Ok(paths) => open.answer = Some(paths.iter().map(|p| p.display().to_string()).collect()),
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(TryRecvError::Disconnected) => close = true,
            },
            How::Places { title, asking, words, .. } => {
                let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
                let shown = egui::Modal::new(egui::Id::new("koyomi.browse.asking")).show(ctx, |ui| {
                    ui.set_width(320.0);
                    ui.label(title.as_str());
                    ui.label(asking.as_str());
                    let r = ito_prefs::button_with(ui, look, &words.cancel, ito_prefs::Tone::Plain);
                    ito_a11y::say(&r, Role::Button, &words.cancel);
                    r.clicked()
                });
                close = shown.inner || shown.should_close();
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            How::Far { picker, words, asked } => {
                let shown = picker.show(ctx, &ito_filepick::Look { words, clock: look.clock, lang: look.lang });
                if let Some(ito_filepick::Want::List { ticket, path }) = shown.want {
                    let n = next_ticket();
                    *asked = Some((n, ticket));
                    self.out = Some(Request::DirList { path, ticket: n });
                }
                match shown.done {
                    Some(Done::Picked(p)) if !p.is_empty() => open.answer = Some(p),
                    Some(_) => close = true,
                    None => ctx.request_repaint_after(Duration::from_millis(100)),
                }
            }
        }
        if close {
            self.open = None;
        }
    }

    /// The answer, when it is for `owner`.
    fn take(&mut self, owner: &str) -> Option<Vec<String>> {
        let open = self.open.as_mut().filter(|o| o.owner == owner)?;
        let answer = open.answer.take()?;
        self.open = None;
        Some(answer)
    }

    /// An answer no field took (its job or its `[[when]]` is not the one open any more) is dropped.
    pub fn drop_answer(&mut self) {
        if self.open.as_ref().is_some_and(|o| o.answer.is_some()) {
            self.open = None;
        }
    }

    fn start(&mut self, owner: String, want: Want, title: String, dir: Option<PathBuf>) {
        let rx = ito_dialog::spawn(move || {
            let mut d = ito_dialog::Dialog::new().title(&title);
            if let Some(dir) = dir {
                d = d.directory(dir);
            }
            match want {
                Want::Folder => d.open_folder().into_iter().collect(),
                Want::File => d.open_file().into_iter().collect(),
                Want::Save => d.save_file().into_iter().collect(),
                Want::Files => d.open_files(),
            }
        });
        self.open = Some(Open { owner, want, how: How::Local(rx), answer: None });
    }

    /// Opens a far chooser: asks the engine for its places first (the request is in [`Browse::request`]).
    #[allow(clippy::too_many_arguments)]
    fn start_far(&mut self, owner: String, want: Want, title: String, text: String, windows: Option<bool>, lang: &str, asking: String) {
        let ticket = next_ticket();
        self.out = Some(Request::DirPlaces { ticket });
        let words = Words::for_lang(lang);
        self.open = Some(Open { owner, want, how: How::Places { ticket, title, text, windows, asking, words }, answer: None });
    }
}

/// The connection names the engine's snapshot lists, for the selects of the job form and the `[[when]]` form. They
/// are passed together (one small struct) so that `drawer` and `Ctx` do not grow an argument for each list.
#[derive(Clone, Copy, Debug, Default)]
pub struct Conns<'a> {
    pub hosts: &'a [crate::proto::HostName],
    pub buckets: &'a [String],
    pub mailboxes: &'a [String],
    pub groups: &'a [crate::proto::GroupName],
    pub mails: &'a [String],
}

/// What the fields of one editor frame share: the nets, the net being edited, whether the engine is this PC, the
/// part (or the net) the pane is about, and the chooser.
pub struct Ctx<'a> {
    pub nets: &'a [NetChoice],
    pub own: String,
    pub local: bool,
    /// The names of the shells the engine has (all five while it has not said).
    shells: Vec<(String, String)>,
    /// The engine's default shell ("" until it has said).
    default: String,
    /// The `[[account]]` names and the `[[database]]`s (name and kind) the engine has registered.
    accounts: Vec<String>,
    databases: Vec<crate::proto::DatabaseName>,
    /// Whether the engine runs on Windows (`None`: it has not said).
    windows: Option<bool>,
    /// The `[[host]]`, `[[bucket]]` and `[[mailbox]]` names, the `[groups]` and the `[[notify.mails]]` the engine has.
    conns: Conns<'a>,
    /// A link asked for this settings page (taken by the drawer after the frame).
    settings: std::cell::Cell<Option<usize>>,
    scope: String,
    browse: RefCell<Browse>,
    /// An engine on another PC whose disk the person may browse (an admin of a far engine).
    far_browse: bool,
}

impl<'a> Ctx<'a> {
    pub fn new(nets: &'a [NetChoice], own: &str, local: bool, scope: &str, browse: Browse) -> Self {
        Ctx {
            nets,
            own: own.to_owned(),
            local,
            shells: Vec::new(),
            default: String::new(),
            accounts: Vec::new(),
            databases: Vec::new(),
            windows: None,
            conns: Conns::default(),
            settings: std::cell::Cell::new(None),
            scope: scope.to_owned(),
            browse: RefCell::new(browse),
            far_browse: false,
        }
    }

    /// With 参照… for the disk of an engine on another PC (the engine's disk is only listed; a file to write is only named).
    pub fn with_far_browse(mut self, on: bool) -> Self {
        self.far_browse = on;
        self
    }

    /// Whether a path field has a 参照… button: on this PC always, on another PC's engine for an admin.
    pub fn can_browse(&self) -> bool {
        self.local || self.far_browse
    }

    /// With the accounts and databases the engine has registered, and whether it runs on Windows.
    pub fn with_registry(mut self, accounts: &[String], databases: &[crate::proto::DatabaseName], windows: Option<bool>) -> Self {
        self.accounts = accounts.to_vec();
        self.databases = databases.to_vec();
        self.windows = windows;
        self
    }

    /// With the hosts, buckets, mailboxes, groups and notify mails the engine has (an engine that lacks them has none).
    pub fn with_conns(mut self, conns: Conns<'a>) -> Self {
        self.conns = conns;
        self
    }

    /// The registered hosts, buckets and mailboxes, the groups and the notify mails.
    pub fn conns(&self) -> Conns<'a> {
        self.conns
    }

    /// The registered account names.
    pub fn accounts(&self) -> &[String] {
        &self.accounts
    }

    /// The registered databases.
    pub fn databases(&self) -> &[crate::proto::DatabaseName] {
        &self.databases
    }

    /// Whether the engine runs on Windows (`None`: it has not said).
    pub fn windows(&self) -> Option<bool> {
        self.windows
    }

    /// A link of the form asks for the settings page `page`.
    pub fn ask_settings(&self, page: usize) {
        self.settings.set(Some(page));
    }

    /// The settings page a link asked for, once.
    pub fn take_settings(&self) -> Option<usize> {
        self.settings.take()
    }

    /// With the shells the engine has (none: it has not said, so every shell koyomi knows is offered).
    pub fn with_shells(mut self, found: &[crate::proto::Shell], default: &str) -> Self {
        self.shells = found.iter().map(|s| (s.name.clone(), s.path.clone())).collect();
        self.default = default.to_owned();
        self
    }

    /// The engine's default shell ("" when it has not said).
    pub fn shell_default(&self) -> &str {
        &self.default
    }

    /// Where the engine found `shell` ("" when it has not said or lacks it).
    pub fn shell_path(&self, shell: &str) -> &str {
        self.shells.iter().find(|(n, _)| n == shell).map_or("", |(_, p)| p.as_str())
    }

    /// The shells a job's `shell` may be picked from.
    pub fn shell_names(&self) -> Vec<String> {
        if self.shells.is_empty() {
            koyomi_core::shell::SHELLS.iter().map(|s| (*s).to_owned()).collect()
        } else {
            self.shells.iter().map(|(n, _)| n.clone()).collect()
        }
    }

    /// A chooser is open (anything that renumbers the `[[when]]`s waits for it).
    pub fn busy(&self) -> bool {
        self.browse.borrow().busy()
    }

    /// The chooser state, to keep for the next frame.
    pub fn into_browse(self) -> Browse {
        self.browse.into_inner()
    }

    /// A select of the nets for the field `key`; `label` is its name for a screen reader. True when one was picked.
    pub fn net_select(&self, ui: &mut egui::Ui, t: &Table, key: &str, label: &str, value: &mut String, optional: bool) -> bool {
        let words = NetWords { none: t.tr("pick.none"), choose: t.tr("pick.choose"), missing: t.tr("pick.missing"), own: t.tr("pick.own") };
        let (entries, now) = net_entries(self.nets, &self.own, value, optional, &words);
        let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
        match select_index(ui, key, label, now, &labels) {
            Some(i) if entries[i].0 != *value => {
                value.clone_from(&entries[i].0);
                true
            }
            _ => false,
        }
    }

    /// 参照… for the field `key` (in the part this frame is about): this PC's chooser, or for an admin the far engine's
    /// picker. The answer of the chooser lands in `text` when this field is the one it was opened for. True when `text`
    /// changed.
    pub fn browse_button(&self, ui: &mut egui::Ui, t: &Table, key: &str, name: &str, want: Want, text: &mut String) -> bool {
        if !self.can_browse() {
            return false;
        }
        let owner = format!("{}\u{1}{key}", self.scope);
        let mut changed = false;
        let answer = self.browse.borrow_mut().take(&owner);
        if let Some(new) = answer.and_then(|paths| browsed_text(text, want, &paths)) {
            if new != *text {
                *text = new;
                changed = true;
            }
        }
        let busy = self.browse.borrow().busy();
        let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
        let r = ui.add_enabled_ui(!busy, |ui| ito_prefs::button_with(ui, look, t.tr("pick.browse"), ito_prefs::Tone::Plain)).inner;
        ito_a11y::say(&r, Role::Button, &format!("{name}: {}", t.tr("pick.browse")));
        if busy {
            r.on_disabled_hover_text(t.tr("pick.browsing"));
        } else if self.local {
            if r.on_hover_text(t.tr("pick.browse_hover")).clicked() {
                let dir = start_dir(text, want);
                self.browse.borrow_mut().start(owner, want, name.to_owned(), dir);
            }
        } else if r.on_hover_text(t.tr("pick.browse_far_hover")).clicked() {
            let asking = t.tr("pick.far_asking").to_owned();
            self.browse.borrow_mut().start_far(owner, want, name.to_owned(), text.clone(), self.windows, t.tr("app.lang"), asking);
        }
        changed
    }
}

/// A select over `labels` (an index is the value); `label` is its name for a screen reader. The index picked, when
/// it changed.
pub fn select_index(ui: &mut egui::Ui, id: &str, label: &str, now: usize, labels: &[String]) -> Option<usize> {
    let options: Vec<(usize, &str)> = labels.iter().enumerate().map(|(i, l)| (i, l.as_str())).collect();
    let out = ui.scope(|ui| ito_prefs::select(ui, id, now, &options));
    ito_a11y::say(&out.response, Role::Label, label);
    out.inner
}

/// The seven days as switches (and 毎日 first when `every` is a word): a switch turned writes `text` in week order.
/// `text` must read (see [`days_set`]). True when one was pressed.
pub fn day_toggles(ui: &mut egui::Ui, t: &Table, label: &str, text: &mut String, every: Option<&str>) -> bool {
    let Some(set) = days_set(text) else { return false };
    let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
    let lang = t.tr("app.lang");
    let mut next = None;
    ui.horizontal_wrapped(|ui| {
        let one = |ui: &mut egui::Ui, words: &str, on: bool| -> bool {
            let tone = if on { ito_prefs::Tone::Primary } else { ito_prefs::Tone::Plain };
            let r = ito_prefs::button_with(ui, look, words, tone);
            let spoken = if on { ito_a11y::State::spoken(words, &[ito_a11y::State::Selected], lang) } else { words.to_owned() };
            ito_a11y::say(&r, Role::Button, &format!("{label}: {spoken}"));
            r.clicked()
        };
        if let Some(words) = every {
            if one(ui, words, every_day(&set)) {
                next = Some([false; 7]);
            }
        }
        for (i, day) in DAYS.iter().enumerate() {
            if one(ui, t.tr(&format!("weekday.{day}")), set[i]) {
                next = Some(days_toggle(&set, i));
            }
        }
    });
    match next {
        Some(new) if new != set || days_text(&new) != text.trim() => {
            *text = days_text(&new);
            true
        }
        _ => false,
    }
}

/// The S3 regions the drawer's and the settings page's region field offer, each with the key of its note.
pub const S3_REGIONS: [(&str, &str); 5] = [
    ("ap-northeast-1", "pick.region_ap_northeast_1"),
    ("ap-northeast-3", "pick.region_ap_northeast_3"),
    ("us-east-1", "pick.region_us_east_1"),
    ("us-west-2", "pick.region_us_west_2"),
    ("eu-west-1", "pick.region_eu_west_1"),
];

/// [`S3_REGIONS`] as `(value, note)` rows for a `ComboField`, the notes in the language in force.
pub fn region_rows(t: &Table) -> Vec<(&'static str, &str)> {
    S3_REGIONS.iter().map(|(v, k)| (*v, t.tr(k))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets() -> Vec<NetChoice> {
        let def = |group: &str, name: &str, title: Option<&str>| Def {
            id: format!("{group}/{name}"),
            group: group.into(),
            name: name.into(),
            title: title.map(str::to_owned),
            file: String::new(),
            standing: koyomi_core::scan::Standing::Confirmed,
            problems: Vec::new(),
            net: None,
            sha256: String::new(),
            text: String::new(),
            previous: None,
            calendar: None,
        };
        net_choices(&[def("人事", "b", None), def("経理", "締め", Some("月次締め")), def("人事", "a", None)])
    }

    fn words() -> NetWords<'static> {
        NetWords { none: "none", choose: "choose", missing: " (gone)", own: " (self)" }
    }

    #[test]
    fn nets_are_sorted_with_their_title() {
        let n = nets();
        let ids: Vec<&str> = n.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["人事/a", "人事/b", "経理/締め"]);
        assert_eq!(n[2].label, "経理/締め  月次締め");
        assert_eq!(n[0].label, "人事/a");
    }

    #[test]
    fn the_own_net_is_left_out_and_a_missing_one_stays_marked() {
        let n = nets();
        // Required and empty: the placeholder first, nothing else written.
        let (e, now) = net_entries(&n, "人事/a", "", false, &words());
        assert_eq!((e[0].0.as_str(), e[0].1.as_str(), now), ("", "choose", 0));
        assert!(e.iter().all(|(v, _)| v != "人事/a"));
        // Optional: 「なし」 first and picked while empty.
        let (e, now) = net_entries(&n, "", "", true, &words());
        assert_eq!((e[0].1.as_str(), now, e.len()), ("none", 0, 4));
        // A value the list holds is picked, with no empty choice once it is set.
        let (e, now) = net_entries(&n, "", "人事/b", false, &words());
        assert_eq!(e[now].0, "人事/b");
        assert!(e.iter().all(|(v, _)| !v.is_empty()));
        // A value the list does not hold stays as written, marked, and is picked.
        let (e, now) = net_entries(&n, "", "旧/消えた", false, &words());
        assert_eq!((e[now].0.as_str(), e[now].1.as_str()), ("旧/消えた", "旧/消えた (gone)"));
        assert_eq!(e.len(), 4);
        // The net itself named: kept, and said so (it is not offered).
        let (e, now) = net_entries(&n, "人事/a", "人事/a", false, &words());
        assert_eq!(e[now].1, "人事/a (self)");
        assert_eq!(e.iter().filter(|(v, _)| v == "人事/a").count(), 1);
    }

    #[test]
    fn days_read_and_write_in_week_order() {
        let set = days_set("fri, mon,tue").unwrap();
        assert_eq!(set, [true, true, false, false, true, false, false]);
        assert_eq!(days_text(&set), "mon, tue, fri");
        assert_eq!(days_set("").unwrap(), [false; 7]);
        assert!(every_day(&[false; 7]) && !every_day(&set));
        assert_eq!(days_set("mon, funday"), None);
        // The canonical text round trips.
        for text in ["", "mon", "mon, tue, wed, thu, fri", "sat, sun"] {
            assert_eq!(days_text(&days_set(text).unwrap()), text);
        }
        // Turning the last day off goes back to every day.
        let one = days_set("wed").unwrap();
        assert!(every_day(&days_toggle(&one, 2)));
        assert!(!every_day(&days_toggle(&[false; 7], 0)));
    }

    #[test]
    fn a_month_has_the_days_it_has() {
        assert_eq!([1, 2, 4, 12].map(days_in_month), [31, 29, 30, 31]);
        assert_eq!(month_day_parts("02-29"), Some((2, 29)));
        assert_eq!(month_day_parts("13-40"), None);
        assert_eq!(month_day_parts("04-31"), None);
        assert_eq!(month_day_text(4, 31), "04-30");
        assert_eq!(month_day_text(2, 29), "02-29");
        assert_eq!(month_day_text(11, 5), "11-05");
        assert_eq!(number_in(" 31 ", 1, 31), Some(31));
        assert_eq!(number_in("32", 1, 31), None);
        assert_eq!(number_in("1,15", 1, 31), None);
        assert_eq!(weekday_index("wed"), Some(2));
        assert_eq!(weekday_index("x"), None);
    }

    #[test]
    fn a_chooser_starts_where_the_field_is() {
        let dir = std::env::temp_dir();
        let text = dir.display().to_string();
        assert_eq!(start_dir(&text, Want::Folder), Some(dir.clone()));
        let file = dir.join("no-such-file-for-koyomi.txt");
        assert_eq!(start_dir(&file.display().to_string(), Want::File), Some(dir.clone()));
        let glob = dir.join("*.csv");
        assert_eq!(start_dir(&glob.display().to_string(), Want::File), None);
        assert_eq!(start_dir("", Want::Folder), None);
        assert_eq!(start_dir("relative/dir", Want::Folder), None);
        assert_eq!(start_dir("{date}", Want::Folder), None);
        let lines = format!("nowhere-x\n{text}\n");
        assert_eq!(start_dir(&lines, Want::Files), Some(dir));
    }

    #[test]
    fn a_chosen_path_fills_the_field() {
        let p = |s: &str| s.to_owned();
        assert_eq!(browsed_text("old", Want::File, &[p("C:/a.txt")]), Some(p("C:/a.txt")));
        assert_eq!(browsed_text("x", Want::Folder, &[]), None);
        let out = browsed_text("C:/a.txt\n", Want::Files, &[p("C:/b.txt"), p("C:/c.txt")]).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), ["C:/a.txt", "C:/b.txt", "C:/c.txt"]);
        assert_eq!(browsed_text("", Want::Files, &[p("C:/b.txt")]), Some(p("C:/b.txt")));
    }

    fn start(text: &str, want: Want, style: Style) -> (Option<String>, Option<String>) {
        far_start(text, want, style)
    }

    fn some(a: &str, b: Option<&str>) -> (Option<String>, Option<String>) {
        (Some(a.to_owned()), b.map(str::to_owned))
    }

    #[test]
    fn a_far_chooser_starts_from_a_complete_windows_path_only() {
        let w = Style::Windows;
        assert_eq!(start(r"C:\data\in", Want::Folder, w), some(r"C:\data\in", None));
        assert_eq!(start(r"  C:\data\in  ", Want::Folder, w), some(r"C:\data\in", None));
        assert_eq!(start(r"C:\data\in\a.csv", Want::File, w), some(r"C:\data\in", Some("a.csv")));
        assert_eq!(start(r"C:\data\in\a.csv", Want::Files, w), some(r"C:\data\in", None));
        assert_eq!(start("x\n C:\\data\\a.csv \n\n", Want::Files, w), some(r"C:\data", None), "the last line is what the person was at");
        assert_eq!(start("C:\\data\\a.csv\nnowhere", Want::Files, w), (None, None));
        // A root has no parent: the file chooser starts at the root, selecting nothing.
        assert_eq!(start(r"C:\", Want::File, w), some(r"C:\", None));
        assert_eq!(start(r"C:\", Want::Files, w), some(r"C:\", None));
        assert_eq!(start(r"C:\", Want::Folder, w), some(r"C:\", None));
        assert_eq!(start(r"C:\data\", Want::Folder, w), some(r"C:\data\", None));
        // Not a complete path on a drive: the picker starts at its first place.
        for text in [
            r"\x",
            "C:foo",
            r"\\?\C:\x",
            r"\\server\share\x",
            r"\\server\share",
            "relative",
            r"a\b",
            "",
            "   ",
            "C:\\a\\*.csv",
            "C:\\a?",
            "C:\\{date}\\x",
        ] {
            for want in [Want::Folder, Want::File, Want::Files] {
                assert_eq!(start(text, want, w), (None, None), "{text:?} {want:?}");
            }
        }
        assert_eq!(start("", Want::Files, w), (None, None));
    }

    #[test]
    fn a_far_chooser_reads_a_posix_path_by_the_engines_style() {
        let p = Style::Posix;
        assert_eq!(start("/srv/data", Want::Folder, p), some("/srv/data", None));
        assert_eq!(start("/srv/data/a.csv", Want::File, p), some("/srv/data", Some("a.csv")));
        assert_eq!(start("/srv/data/a.csv", Want::Files, p), some("/srv/data", None));
        assert_eq!(start("/", Want::File, p), some("/", None));
        assert_eq!(start("/", Want::Folder, p), some("/", None));
        // A name with a tab or a backslash is kept as it is.
        assert_eq!(start("/srv/a\tb", Want::File, p), some("/srv", Some("a\tb")));
        assert_eq!(start("/srv/a\\b", Want::File, p), some("/srv", Some("a\\b")));
        for text in ["relative/dir", "", r"C:\data", "/srv/{date}", "/srv/*.csv", "/srv/a?"] {
            assert_eq!(start(text, Want::File, p), (None, None), "{text:?}");
        }
    }

    fn places_json() -> String {
        let reply = PlacesReply { style: Style::Posix, places: vec![ito_filepick::Place::new("Root", "/", ito_filepick::PlaceKind::Root)] };
        serde_json::to_string(&reply).unwrap()
    }

    fn waiting() -> (Browse, u64) {
        let mut b = Browse::waiting_far("owner", Want::Folder);
        let Some(Request::DirPlaces { ticket }) = b.request() else { panic!("a far chooser asks for the places first") };
        assert!(b.request().is_none(), "a request is taken once");
        (b, ticket)
    }

    fn is_far(b: &Browse) -> bool {
        matches!(b.open.as_ref().map(|o| &o.how), Some(How::Far { .. }))
    }

    /// One frame of the chooser's `poll`.
    fn poll_frame(ctx: &egui::Context, b: &mut Browse) {
        let clock = ito_common::Clock::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| b.poll(ui.ctx(), &FarLook { clock: &clock, lang: "ja" }));
        out.textures_delta.clear();
    }

    #[test]
    fn the_places_build_the_picker_and_a_stale_ticket_is_ignored() {
        let (mut b, ticket) = waiting();
        assert!(b.picking() && b.busy() && !is_far(&b));
        assert!(!b.answered(&Request::DirPlaces { ticket: ticket + 1000 }, Ok(places_json())), "an old ticket is not this chooser's");
        assert!(!b.answered(&Request::Status, Ok(String::new())));
        assert!(!is_far(&b));
        assert!(b.answered(&Request::DirPlaces { ticket }, Ok(places_json())));
        assert!(is_far(&b) && b.picking());
        assert!(!b.answered(&Request::DirPlaces { ticket }, Ok(places_json())), "answered once");
    }

    #[test]
    fn places_that_fail_or_do_not_read_still_build_the_picker() {
        type Fail<'a> = &'a dyn Fn(&mut Browse, &Request);
        let failing: [Fail; 3] = [
            &|b, req| assert!(b.answered(req, Err("refused".into()))),
            &|b, req| assert!(b.answered(req, Ok("not json".into()))),
            &|b, req| b.unsent(req, "link down".into()),
        ];
        for fail in failing {
            let (mut b, ticket) = waiting();
            fail(&mut b, &Request::DirPlaces { ticket });
            assert!(is_far(&b) && b.picking());
        }
    }

    /// A far chooser whose picker is up and has asked for its first listing: the chooser, the path and the ticket.
    fn listing_asked(ctx: &egui::Context) -> (Browse, String, u64) {
        let (mut b, ticket) = waiting();
        assert!(b.answered(&Request::DirPlaces { ticket }, Ok(places_json())));
        // The picker asks for its first listing on its first frames.
        for _ in 0..4 {
            poll_frame(ctx, &mut b);
            if let Some(Request::DirList { path, ticket }) = b.request() {
                return (b, path, ticket);
            }
        }
        panic!("the picker did not ask for a listing");
    }

    #[test]
    fn a_listing_answer_is_taken_only_for_the_live_ticket() {
        let ctx = egui::Context::default();
        let (mut b, path, n) = listing_asked(&ctx);
        assert!(!path.is_empty());
        assert_eq!(b.far_status(), "Loading");
        let listing = serde_json::to_string(&ito_filepick::Listing::new(vec![ito_filepick::Entry::dir("a")])).unwrap();
        assert!(!b.answered(&Request::DirList { path: path.clone(), ticket: n + 1000 }, Ok(listing.clone())), "an old ticket");
        assert_eq!(b.far_status(), "Loading");
        assert!(b.answered(&Request::DirList { path: path.clone(), ticket: n }, Ok(listing)));
        assert_eq!(b.far_status(), "Ready");
        assert!(!b.answered(&Request::DirList { path, ticket: n }, Ok("{}".into())), "answered once");
        assert!(b.picking());
    }

    #[test]
    fn a_listing_that_fails_or_is_not_sent_shows_in_the_picker() {
        let ctx = egui::Context::default();
        let (mut b, path, n) = listing_asked(&ctx);
        assert!(b.answered(&Request::DirList { path, ticket: n }, Err("no such folder".into())));
        assert_eq!(b.far_status(), r#"Failed("no such folder")"#);
        assert!(b.picking(), "the picker stays up to show why");
        let (mut b, path, n) = listing_asked(&ctx);
        b.unsent(&Request::DirList { path, ticket: n }, "link down".into());
        assert_eq!(b.far_status(), r#"Failed("link down")"#);
        let (mut b, path, n) = listing_asked(&ctx);
        assert!(b.answered(&Request::DirList { path, ticket: n }, Ok("not json".into())));
        assert!(b.far_status().starts_with("Failed("), "{}", b.far_status());
        assert!(b.picking());
    }

    #[test]
    fn a_cancelled_chooser_ignores_what_the_engine_answers_later() {
        let (mut b, ticket) = waiting();
        b.cancel();
        assert!(!b.picking() && !b.busy());
        assert!(!b.answered(&Request::DirPlaces { ticket }, Ok(places_json())));
        assert!(!b.busy(), "nothing opens");
        // Cancelling a built picker drops it too; a local chooser is not cancelled (the system's dialog is its own).
        let (mut b, ticket) = waiting();
        assert!(b.answered(&Request::DirPlaces { ticket }, Ok(places_json())));
        b.cancel();
        assert!(!b.picking() && !b.busy());
        let mut local = Browse::pending("o");
        assert!(!local.picking());
        local.cancel();
        assert!(local.busy());
    }

    #[test]
    fn the_waiting_box_closes_on_escape() {
        let ctx = egui::Context::default();
        let (mut b, _) = waiting();
        poll_frame(&ctx, &mut b);
        assert!(b.picking(), "the box is up");
        let esc =
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let clock = ito_common::Clock::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            events: vec![esc],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| b.poll(ui.ctx(), &FarLook { clock: &clock, lang: "ja" }));
        out.textures_delta.clear();
        assert!(!b.picking() && !b.busy(), "Escape closes the chooser");
    }

    #[test]
    fn the_browse_button_is_for_this_pc_or_a_far_admin() {
        let ctx = |local: bool, far: bool| Ctx::new(&[], "a", local, "", Browse::default()).with_far_browse(far);
        assert!(ctx(true, false).can_browse(), "this PC always");
        assert!(!ctx(false, false).can_browse(), "another PC's engine, not an admin");
        assert!(ctx(false, true).can_browse(), "another PC's engine, an admin");
    }

    #[test]
    fn a_far_picker_opens_in_the_mode_the_field_wants() {
        for (want, mode) in
            [(Want::Folder, Mode::OpenFolder), (Want::File, Mode::OpenFile), (Want::Files, Mode::OpenFiles), (Want::Save, Mode::SaveFile)]
        {
            let mut b = Browse::waiting_far("owner", want);
            let Some(Request::DirPlaces { ticket }) = b.request() else { panic!("a far chooser asks for the places first") };
            assert!(b.answered(&Request::DirPlaces { ticket }, Ok(places_json())));
            match b.open.as_ref().map(|o| &o.how) {
                Some(How::Far { picker, .. }) => assert_eq!(picker.mode(), mode, "{want:?}"),
                _ => panic!("no far picker for {want:?}"),
            }
        }
    }

    #[test]
    fn the_region_list_has_a_note_in_both_languages() {
        for lang in [include_str!("../../lang/en.toml"), include_str!("../../lang/ja.toml")] {
            let table: toml::Table = lang.parse().unwrap();
            for (value, key) in S3_REGIONS {
                let (section, name) = key.split_once('.').unwrap();
                let note = table[section][name].as_str().unwrap_or_default();
                assert!(!note.is_empty(), "{value}: {key}");
            }
        }
    }
}
