//! The box that renames, deletes and moves a `[[remote]]` of koyomi.toml (TODO 80): one row per connection with a
//! rename, a delete and a change of its URL. This file holds the box's state and its drawing; what a press does (the
//! checks, the thread that writes koyomi.toml) is `Far`'s.
//!
//! A connection the window is using (its current link, or the PC its page shows) offers neither a rename nor a delete:
//! renaming it would cut the link the window stands on. Its URL can be changed all the same: a PC that moved is
//! exactly when the current link is down. Esc closes the innermost thing: the question, else the editing, else the box.
//! While a press is carried out the box does not close and takes no new press.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use ito_a11y::Role;

use super::listtable;

/// A row being renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Editing {
    /// The name the row had.
    pub from: String,
    /// What the field holds.
    pub text: String,
    /// The field takes the focus once, when it appears.
    fresh: bool,
}

/// Where a change of a URL is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UrlStep {
    /// The new URL is typed.
    Form,
    /// The fingerprint it shows is being fetched.
    Peeking,
    /// The fingerprint differs from the stored one and is shown, to be compared with what the other PC says. There
    /// is no way on without the person saying it is the same.
    Compare,
    /// The check runs (and, once it passed, koyomi.toml is written).
    Checking,
}

/// A row whose URL is being changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Moving {
    /// The connection's name.
    pub from: String,
    /// What the field holds.
    pub text: String,
    pub step: UrlStep,
    /// The fingerprint the new URL shows, once fetched.
    pub fp: Option<String>,
    /// The field takes the focus once, when it appears.
    fresh: bool,
}

/// A row being asked about before it is deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Asking {
    pub name: String,
    /// The button the keyboard is on: 0 is 消す, 1 is やめる. It starts on やめる, so Enter on a fresh question deletes
    /// nothing.
    pub focus: usize,
}

/// The manage box.
pub(super) struct Manage {
    /// Which box this is: an answer of an earlier box does not belong to this one.
    pub ticket: u64,
    pub editing: Option<Editing>,
    pub asking: Option<Asking>,
    pub moving: Option<Moving>,
    /// A press is being carried out: nothing else is pressed and the box stays until the answer comes.
    pub busy: bool,
    /// What the last press came to: the error, or the outcome.
    pub line: Option<Result<String, String>>,
    /// The sort and the picked row of the table.
    table: listtable::Memory,
}

/// What a frame of the box came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pressed {
    Nothing,
    Close,
    /// つなぎ先を足す…
    Add,
    /// 変える: the row's name and the typed one.
    Rename(String, String),
    /// 消す on the question.
    Delete(String),
    /// The row's name and the typed URL.
    Url(String, String),
    /// 同じでした on the fingerprint of the new URL.
    UrlSame,
    /// The 見張る switch of a row: the row's name and whether the window is to watch it now.
    Watch(String, bool),
}

impl Manage {
    pub(super) fn new() -> Manage {
        static TICKETS: AtomicU64 = AtomicU64::new(1);
        Manage {
            ticket: TICKETS.fetch_add(1, Ordering::Relaxed),
            editing: None,
            asking: None,
            moving: None,
            busy: false,
            line: None,
            table: listtable::Memory::default(),
        }
    }

    /// 名前を変える on a row: the field holds the current name.
    pub(super) fn edit(&mut self, name: &str) {
        self.editing = Some(Editing { from: name.to_owned(), text: name.to_owned(), fresh: true });
        self.moving = None;
        self.line = None;
    }

    /// 消す on a row: the question, on やめる.
    pub(super) fn ask(&mut self, name: &str) {
        self.asking = Some(Asking { name: name.to_owned(), focus: 1 });
        self.moving = None;
        self.line = None;
    }

    /// The URL change on a row: the field holds the current URL (empty when it is not known).
    pub(super) fn move_url(&mut self, name: &str, url: &str) {
        self.moving = Some(Moving { from: name.to_owned(), text: url.to_owned(), step: UrlStep::Form, fp: None, fresh: true });
        self.editing = None;
        self.asking = None;
        self.line = None;
    }

    /// What the keys do. Esc closes the innermost thing: the question, else the editing, else the box. Enter presses
    /// the focused button of the question, 変える while a name is being typed, and does nothing otherwise. Tab and
    /// Shift+Tab move the question's focus. While busy every key does nothing.
    pub(super) fn route(&mut self, esc: bool, enter: bool, tab: bool) -> Pressed {
        if self.busy {
            return Pressed::Nothing;
        }
        if let Some(a) = &mut self.asking {
            if esc {
                self.asking = None;
            } else if enter {
                let a = a.clone();
                self.asking = None;
                if a.focus == 0 {
                    return Pressed::Delete(a.name);
                }
            } else if tab {
                a.focus = (a.focus + 1) % 2;
            }
            return Pressed::Nothing;
        }
        if let Some(v) = &self.moving {
            if esc {
                self.moving = None;
            } else if enter && v.step == UrlStep::Form {
                return Pressed::Url(v.from.clone(), v.text.clone());
            }
            return Pressed::Nothing;
        }
        if let Some(e) = &self.editing {
            if esc {
                self.editing = None;
            } else if enter {
                return Pressed::Rename(e.from.clone(), e.text.clone());
            }
            return Pressed::Nothing;
        }
        if esc {
            Pressed::Close
        } else {
            Pressed::Nothing
        }
    }
}

/// Whether the window is using the connection `name`: its current link.
pub(super) fn in_use(link: Option<&str>, name: &str) -> bool {
    link.is_some_and(|l| l.trim() == name.trim())
}

/// The delete question's text, by what the delete does with the connection's own secret. `secret` is the secret the
/// token names (`None`: not known).
fn ask_body(t: &ito_i18n::Table, name: &str, secret: Option<&str>) -> String {
    match secret {
        None => t.trf("far.manage_ask_body_unknown", &[name]),
        Some(s) if crate::remote::generated_secret(name, s) => t.trf("far.manage_ask_body", &[name, s]),
        Some(s) => t.trf("far.manage_ask_body_kept", &[name, s]),
    }
}

/// Draws the box and returns what was pressed. `used` says whether a row is in use. `off` holds the rows whose own
/// `watch` is false (their 見張る switch shows off), and `all_off` is `watch_remotes = false`: the rows still show
/// their own flag, and one line above them says that all watching is off.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    m: &mut Manage,
    ctx: &egui::Context,
    t: &ito_i18n::Table,
    names: &[String],
    secrets: &BTreeMap<String, String>,
    urls: &BTreeMap<String, String>,
    unread: Option<&str>,
    used: &dyn Fn(&str) -> bool,
    off: &BTreeSet<String>,
    all_off: bool,
) -> Pressed {
    let (esc, enter, tab) = ctx.input_mut(|i| {
        let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
        // Tab moves the question's focus; elsewhere it is the widgets' own.
        let tab = m.asking.is_some()
            && !m.busy
            && (i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) || i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
        (esc, enter, tab)
    });
    // A row that is gone from the list it was read from cannot be renamed or moved: its form closes before a key can send it
    // (a URL check that is already running is left alone).
    if unread.is_none() && !m.busy {
        if m.editing.as_ref().is_some_and(|e| !names.contains(&e.from)) {
            m.editing = None;
        }
        if m.moving.as_ref().is_some_and(|v| v.step == UrlStep::Form && !names.contains(&v.from)) {
            m.moving = None;
        }
    }
    let key = m.route(esc, enter, tab);
    let c = ito_theme::colors();
    let busy = m.busy;
    let mut press = Pressed::Nothing;
    let (mut to_edit, mut to_ask, mut to_url) = (None, None, None);
    egui::Window::new(t.tr("far.manage_title"))
        .id(egui::Id::new("far-manage"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(360.0, 760.0));
            ui.add_enabled_ui(!busy, |ui| {
                if let Some(e) = unread {
                    // koyomi.toml did not read: any row would fail to rename or delete, so none is drawn.
                    let text = t.tr("far.manage_unread");
                    let r = ui.label(egui::RichText::new(text).color(c.dim));
                    ito_a11y::say(&r, Role::Label, text);
                    let r = ui.add(egui::Label::new(egui::RichText::new(e).color(c.err)).wrap());
                    ito_a11y::say(&r, Role::Label, e);
                } else {
                    if all_off {
                        let text = t.tr("far.manage_watch_all_off");
                        let r = ui.add(egui::Label::new(egui::RichText::new(text).color(c.dim)).wrap());
                        ito_a11y::say(&r, Role::Label, text);
                    }
                    if names.is_empty() {
                        ui.label(egui::RichText::new(t.tr("far.manage_none")).color(c.dim));
                    }
                    m.table.sync(None, Some(names));
                    if !names.is_empty() {
                        let rows = rows(t, names, urls, used, off);
                        egui::ScrollArea::vertical().id_salt("far-manage-rows").max_height(360.0).auto_shrink([false, true]).show(
                            ui,
                            |ui| {
                                table(ui, t, m, &rows, off, (&mut press, &mut to_edit, &mut to_ask, &mut to_url));
                            },
                        );
                    }
                    if names.iter().any(|n| used(n)) {
                        // Also in words, not only on the hover of the off buttons.
                        let text = t.tr("far.manage_in_use_note");
                        let r = ui.add(egui::Label::new(egui::RichText::new(text).color(c.dim)).wrap());
                        ito_a11y::say(&r, Role::Label, text);
                    }
                    form(ui, t, m, &mut press);
                }
            });
            if let Some(line) = &m.line {
                ui.add_space(6.0);
                let (text, colour) = match line {
                    Ok(s) => (s.as_str(), c.fg),
                    Err(e) => (e.as_str(), c.err),
                };
                let r = ui.add(egui::Label::new(egui::RichText::new(text).color(colour)).wrap());
                ito_a11y::say(&r, Role::Label, text);
            }
            if busy {
                let words = match m.moving.as_ref().map(|v| v.step) {
                    Some(UrlStep::Peeking) => "far.add_peeking",
                    Some(UrlStep::Checking) => "far.manage_url_checking",
                    _ => "far.manage_busy",
                };
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t.tr(words));
                });
            }
            ui.add_space(8.0);
            let (mut add, mut close) = (false, false);
            ui.add_enabled_ui(!busy, |ui| {
                egui::Sides::new().show(
                    ui,
                    |ui| {
                        if ito_prefs::IconButton::new(ito_prefs::Mark::Plus, t.tr("far.manage_add")).show(ui).clicked() {
                            add = true;
                        }
                    },
                    |ui| {
                        if ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("far.manage_close")).show(ui).clicked() {
                            close = true;
                        }
                    },
                );
            });
            if add {
                press = Pressed::Add;
            } else if close {
                press = Pressed::Close;
            }
        });
    if let Some(n) = to_edit {
        m.edit(&n);
    }
    if let Some(n) = to_ask {
        m.ask(&n);
    }
    if let Some(n) = to_url {
        m.move_url(&n, urls.get(n.trim()).map_or("", String::as_str));
    }
    if let Some(a) = m.asking.clone() {
        let body = [ask_body(t, &a.name, secrets.get(a.name.trim()).map(String::as_str))];
        let buttons = [t.tr("far.manage_delete").to_owned(), t.tr("far.manage_back").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("far.manage_ask_title"),
            body: &body,
            buttons: &buttons,
            marks: &[Some(ito_prefs::Mark::Trash), Some(ito_prefs::Mark::Close)],
            focus: Some(a.focus),
            primary: Some(0),
            danger: true,
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &super::Browser::style()) {
            Some(0) => {
                m.asking = None;
                press = Pressed::Delete(a.name);
            }
            Some(_) => m.asking = None,
            None => {}
        }
    }
    if key != Pressed::Nothing {
        key
    } else {
        press
    }
}

/// A URL without a `user:password@` part (a password is never shown).
pub(super) fn strip_userinfo(url: &str) -> String {
    // Everything up to the LAST '@' after the scheme goes: a password may hold an unencoded '/', '?' or '#', and a far URL
    // is a base address, so showing too little beats showing a password.
    let (scheme, rest) = url.split_once("://").map_or(("", url), |(s, r)| (s, r));
    let host = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    if url.contains("://") {
        format!("{scheme}://{host}")
    } else {
        host.to_owned()
    }
}

/// The rows of the table: name, URL (as far as it is known) and state (in use, watched), with the three things a row
/// can do. A connection in use is neither renamed nor deleted (the reason is the hover of the two).
fn rows(
    t: &ito_i18n::Table,
    names: &[String],
    urls: &BTreeMap<String, String>,
    used: &dyn Fn(&str) -> bool,
    off: &BTreeSet<String>,
) -> Vec<listtable::Row> {
    use ito_prefs::{Mark, Tone};
    names
        .iter()
        .map(|name| {
            let in_use = used(name);
            let watching = !off.contains(name.trim());
            let state = [in_use.then(|| t.tr("far.manage_in_use_short")), watching.then(|| t.tr("far.manage_watch"))]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\u{30fb}");
            // The screen reader hears why the row's rename and delete are off (the hover is not reached by a keyboard).
            let said = [in_use.then(|| t.tr("far.manage_in_use_said").to_owned()), watching.then(|| t.tr("far.manage_watch").to_owned())]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\u{30fb}");
            let url = urls.get(name.trim()).map_or_else(String::new, |u| strip_userinfo(u));
            let cells = vec![listtable::Cell::new(name), listtable::Cell::new(url), listtable::Cell::new(state).said(said)];
            let why = in_use.then(|| t.tr("far.manage_in_use"));
            let said = |key: &str| format!("{} {name}", t.tr(key));
            let acts = vec![
                // Allowed on the connection in use: a PC that moved is exactly when the current link is down.
                listtable::Act::new("url", t.tr("far.manage_url"), Mark::Link, Tone::Plain).said(said("far.manage_url")),
                listtable::Act::new("rename", t.tr("far.manage_rename"), Mark::Pencil, Tone::Plain)
                    .said(said("far.manage_rename"))
                    .on(!in_use, why)
                    .edit(),
                listtable::Act::new("delete", t.tr("far.manage_delete"), Mark::Trash, Tone::Danger)
                    .said(said("far.manage_delete"))
                    .on(!in_use, why),
            ];
            listtable::Row::new(name, cells, acts)
        })
        .collect()
}

/// The table of connections; the 見張る switch of each row sits in its 操作 cell, left of the buttons.
fn table(
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    m: &mut Manage,
    rows: &[listtable::Row],
    off: &BTreeSet<String>,
    (press, to_edit, to_ask, to_url): (&mut Pressed, &mut Option<String>, &mut Option<String>, &mut Option<String>),
) {
    let c = ito_theme::colors();
    let busy = m.busy;
    let word = t.tr("far.manage_watch");
    let heads: Vec<String> = ["name", "url", "state", "ops"].iter().map(|k| t.tr(&format!("far.manage_col_{k}")).to_owned()).collect();
    let word_w = ui.painter().layout_no_wrap(word.to_owned(), egui::TextStyle::Body.resolve(ui.style()), c.dim).size().x;
    // The switch is `ito_prefs::switch`, which is `ito_theme::zoomed(ctx, 40.0)` wide: the cell grows with the letter zoom.
    let extra_w = word_w + ito_theme::zoomed(ui.ctx(), 40.0) + ui.spacing().item_spacing.x * 2.0;
    let rows: Vec<listtable::Row> = rows
        .iter()
        .cloned()
        .map(|mut r| {
            r.extra_w = extra_w;
            r
        })
        .collect();
    let mut flip: Option<(String, bool)> = None;
    let mut draw = |ui: &mut egui::Ui, i: usize| {
        let name = &rows[i].name;
        let watching = !off.contains(name.trim());
        let look = ito_prefs::Look { c, q: "" };
        ui.label(egui::RichText::new(word).color(c.dim));
        // The scope holds the switch alone, so a press at its centre (`<Click:見張る kei>`, `<ClickId:>`) lands on the switch.
        let mut now = false;
        let scope = ui.add_enabled_ui(!busy, |ui| {
            now = ito_prefs::switch(ui, look, watching);
        });
        let said = t.trf("far.manage_watch_name", &[name]);
        let enabled = scope.response.enabled();
        scope.response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, watching, &said));
        let id = format!("manage.watch.{name}");
        if ito_a11y::is_test_id(&id) {
            ito_a11y::test_id(&scope.response, &id);
        }
        if now {
            flip = Some((name.clone(), !watching));
        }
    };
    let spec = listtable::Spec { prefix: "manage", heads: &heads, hint: t.tr("today.sort"), link: None };
    let hit = listtable::show(ui, &mut m.table, &spec, &rows, Some(&mut draw));
    if let Some((name, on)) = flip {
        *press = Pressed::Watch(name, on);
    }
    if let Some((i, key)) = hit {
        let name = Some(rows[i].name.clone());
        match key {
            "url" => *to_url = name,
            "rename" => *to_edit = name,
            _ => *to_ask = name,
        }
    }
}

/// The form of the row being renamed or moved, below the table: its field and the two buttons, and for a URL that
/// differs from the stored one the fingerprint to compare.
fn form(ui: &mut egui::Ui, t: &ito_i18n::Table, m: &mut Manage, press: &mut Pressed) {
    let (name, moving) = match (&m.moving, &m.editing) {
        (Some(v), _) => (v.from.clone(), true),
        (None, Some(e)) => (e.from.clone(), false),
        _ => {
            listtable::form_closed(&mut m.table);
            return;
        }
    };
    let (mut go, mut back) = (false, false);
    let heading = t.trf(if moving { "far.manage_url_heading" } else { "far.manage_rename_heading" }, &[&name]);
    let mut mem = std::mem::take(&mut m.table);
    listtable::form_below(ui, &mut mem, &heading, listtable::form_token(if moving { "url" } else { "rename" }, &name), |ui| {
        let form_step = m.moving.as_ref().is_some_and(|v| v.step == UrlStep::Form);
        egui::Sides::new().height(32.0).show(
            ui,
            |ui| {
                if moving {
                    if let Some(v) = &mut m.moving {
                        let r = ui.add_enabled(v.step == UrlStep::Form, egui::TextEdit::singleline(&mut v.text).desired_width(320.0));
                        ito_a11y::say(&r, Role::Label, &t.trf("far.manage_url_name", &[&name]));
                        if v.fresh {
                            r.request_focus();
                            v.fresh = false;
                        }
                    }
                } else if let Some(e) = &mut m.editing {
                    let r = ui.add(egui::TextEdit::singleline(&mut e.text).desired_width(320.0));
                    ito_a11y::say(&r, Role::Label, &t.trf("far.manage_name", &[&name]));
                    if e.fresh {
                        r.request_focus();
                        e.fresh = false;
                    }
                }
            },
            |ui| {
                back = ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("far.manage_back"))
                    .said(&format!("{} {name}", t.tr("far.manage_back")))
                    .show(ui)
                    .clicked();
                if moving {
                    let said = format!("{} {name}", t.tr("far.manage_url_go"));
                    let b = ito_prefs::IconButton::new(ito_prefs::Mark::Link, t.tr("far.manage_url_go"))
                        .said(&said)
                        .tone(ito_prefs::Tone::Primary);
                    go = ui.add_enabled(form_step, |ui: &mut egui::Ui| b.show(ui)).clicked();
                } else {
                    go = ito_prefs::IconButton::new(ito_prefs::Mark::Pencil, t.tr("far.manage_go"))
                        .said(&format!("{} {name}", t.tr("far.manage_go")))
                        .tone(ito_prefs::Tone::Primary)
                        .show(ui)
                        .clicked();
                }
            },
        );
        compare(ui, t, m, press);
    });
    m.table = mem;
    if go {
        if let Some(v) = m.moving.as_ref().filter(|v| v.from == name) {
            *press = Pressed::Url(v.from.clone(), v.text.clone());
        } else if let Some(e) = &m.editing {
            *press = Pressed::Rename(e.from.clone(), e.text.clone());
        }
    }
    if back {
        m.editing = None;
        m.moving = None;
    }
}

/// The fingerprint of a new URL that differs from the stored one, and the two buttons that say what to do with it.
fn compare(ui: &mut egui::Ui, t: &ito_i18n::Table, m: &mut Manage, press: &mut Pressed) {
    let Some(v) = m.moving.as_ref().filter(|v| v.step == UrlStep::Compare) else { return };
    let (name, fp) = (v.from.clone(), v.fp.clone().unwrap_or_default());
    ui.separator();
    ui.add(egui::Label::new(t.trf("far.manage_url_compare", &[&name])).wrap());
    let r = ui.add(egui::Label::new(egui::RichText::new(&fp).monospace().strong()).wrap().selectable(true));
    ito_a11y::say(&r, Role::Label, &fp);
    match super::box_answer_on(ui, true, ito_prefs::Mark::Check, t.tr("far.add_same"), t.tr("far.manage_back")) {
        Some(0) => *press = Pressed::UrlSame,
        Some(_) => m.moving = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_in_use_when_it_is_the_link() {
        assert!(in_use(Some("kei"), "kei"));
        assert!(in_use(Some(" kei "), "kei"));
        assert!(!in_use(Some("a"), "kei"));
        assert!(!in_use(None, "kei"));
    }

    #[test]
    fn esc_closes_the_innermost_thing_first() {
        let mut m = Manage::new();
        m.edit("kei");
        m.ask("kei");
        assert_eq!(m.route(true, false, false), Pressed::Nothing);
        assert!(m.asking.is_none() && m.editing.is_some(), "the question went first");
        assert_eq!(m.route(true, false, false), Pressed::Nothing);
        assert!(m.editing.is_none(), "then the editing");
        assert_eq!(m.route(true, false, false), Pressed::Close, "then the box");
    }

    #[test]
    fn the_first_enter_on_a_fresh_question_deletes_nothing() {
        let mut m = Manage::new();
        m.ask("kei");
        assert_eq!(m.asking.as_ref().unwrap().focus, 1);
        assert_eq!(m.route(false, true, false), Pressed::Nothing);
        assert!(m.asking.is_none(), "Enter on やめる closes the question");
        // Tab moves to 消す, and Enter presses it.
        m.ask("kei");
        assert_eq!(m.route(false, false, true), Pressed::Nothing);
        assert_eq!(m.asking.as_ref().unwrap().focus, 0);
        assert_eq!(m.route(false, true, false), Pressed::Delete("kei".into()));
        assert!(m.asking.is_none());
        // Tab again comes back.
        m.ask("kei");
        m.route(false, false, true);
        m.route(false, false, true);
        assert_eq!(m.asking.as_ref().unwrap().focus, 1);
    }

    #[test]
    fn enter_presses_henkaeru_while_typing_and_does_nothing_otherwise() {
        let mut m = Manage::new();
        assert_eq!(m.route(false, true, false), Pressed::Nothing);
        m.edit("kei");
        m.editing.as_mut().unwrap().text = "keiba".into();
        assert_eq!(m.route(false, true, false), Pressed::Rename("kei".into(), "keiba".into()));
        assert!(m.editing.is_some(), "the editing stays until the answer comes");
    }

    #[test]
    fn while_busy_no_key_does_anything() {
        let mut m = Manage::new();
        m.edit("kei");
        m.busy = true;
        for (esc, enter, tab) in [(true, false, false), (false, true, false), (false, false, true)] {
            assert_eq!(m.route(esc, enter, tab), Pressed::Nothing);
        }
        assert!(m.editing.is_some());
        m.editing = None;
        assert_eq!(m.route(true, false, false), Pressed::Nothing, "Esc does not close the box");
    }

    #[test]
    fn the_question_names_the_secret_the_delete_will_use() {
        let (t, _) = crate::i18n::table("ja");
        let today = "works を削除します。この PC の koyomi.toml から [[remote]] を削除します。koyomi が作成したシークレット works-token は、ほかから使われていなければ削除します。接続先の PC には何も行いません。";
        assert_eq!(ask_body(&t, "works", Some("works-token")), today);
        assert!(ask_body(&t, "works", Some("Works-Token")).contains("シークレット Works-Token は、"));
        let kept = ask_body(&t, "factory", Some("works-token"));
        assert!(kept.contains("シークレット works-token は削除しません"), "{kept}");
        assert!(!kept.contains("factory-token"));
        let unknown = ask_body(&t, "works", None);
        assert!(!unknown.contains("-token") && unknown.contains("works を削除します"), "{unknown}");
    }

    /// The box draws with names, with none, with a row being renamed, with the question, and while busy.
    #[test]
    fn the_box_draws_in_each_state() {
        let (t, _) = crate::i18n::table("ja");
        let ctx = egui::Context::default();
        let names = vec!["kei".to_owned(), "works".to_owned()];
        let used = |n: &str| n == "works";
        let frame_with = |m: &mut Manage, names: &[String], off: &BTreeSet<String>, all_off: bool| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(show(m, ui.ctx(), &t, names, &BTreeMap::new(), &BTreeMap::new(), None, &used, off, all_off), Pressed::Nothing);
            });
            out.textures_delta.clear();
        };
        let frame = |m: &mut Manage, names: &[String]| frame_with(m, names, &BTreeSet::new(), false);
        let mut m = Manage::new();
        frame(&mut m, &names);
        // A switch per row in each state: on, off (own flag), and the all-off line above rows that keep their flag.
        let off = BTreeSet::from(["kei".to_owned()]);
        frame_with(&mut m, &names, &off, false);
        frame_with(&mut m, &names, &off, true);
        frame_with(&mut m, &names, &BTreeSet::new(), true);
        // A press while busy does nothing (the switch is disabled and the keys are ignored).
        m.busy = true;
        frame_with(&mut m, &names, &off, false);
        m.busy = false;
        frame(&mut m, &[]);
        m.edit("kei");
        frame(&mut m, &names);
        frame(&mut m, &names);
        m.ask("kei");
        frame(&mut m, &names);
        m.asking = None;
        m.busy = true;
        m.line = Some(Err("kei: つながらない".into()));
        frame(&mut m, &names);
        m.busy = false;
        m.line = Some(Ok("kei を消した".into()));
        frame(&mut m, &names);
        m.line = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(
                show(
                    &mut m,
                    ui.ctx(),
                    &t,
                    &names,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    Some("KOYOMI-E1023 x"),
                    &used,
                    &BTreeSet::new(),
                    false
                ),
                Pressed::Nothing
            );
        });
        out.textures_delta.clear();
    }

    #[test]
    fn a_password_in_a_url_is_never_shown() {
        assert_eq!(strip_userinfo("https://kei:secret@pc.example:8443/x?a=b"), "https://pc.example:8443/x?a=b");
        assert_eq!(strip_userinfo("https://kei:pa/ss@pc.example/"), "https://pc.example/");
        assert_eq!(strip_userinfo("https://kei:p?w#d@pc.example:8443/x"), "https://pc.example:8443/x");
        assert_eq!(strip_userinfo("kei:pw@pc:8443"), "pc:8443");
        assert_eq!(strip_userinfo("https://pc.example:8443/x"), "https://pc.example:8443/x");
        assert_eq!(strip_userinfo("pc.example"), "pc.example");
    }

    #[test]
    fn a_row_in_use_is_neither_renamed_nor_deleted_but_may_move() {
        let (t, _) = crate::i18n::table("ja");
        let names = vec!["kei".to_owned(), "works".to_owned()];
        let urls = BTreeMap::from([("works".to_owned(), "https://u:p@w.example/".to_owned())]);
        let off = BTreeSet::from(["kei".to_owned()]);
        let rows = rows(&t, &names, &urls, &|n| n == "works", &off);
        let texts = |i: usize| rows[i].cells.iter().map(|c| c.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(0), ["kei", "", ""], "not watched and not in use: nothing to say");
        assert_eq!(texts(1)[1], "https://w.example/");
        let on = |i: usize| rows[i].acts.iter().map(|a| (a.key, a.on)).collect::<Vec<_>>();
        assert_eq!(on(0), [("url", true), ("rename", true), ("delete", true)]);
        assert_eq!(on(1), [("url", true), ("rename", false), ("delete", false)]);
        assert_eq!(rows[1].acts[1].why.as_deref(), Some(t.tr("far.manage_in_use")));
        assert!(rows[0].acts[1].edit, "a double-click renames");
        // The screen reader hears why the row in use cannot be renamed or deleted.
        let heads: Vec<String> = ["名前", "URL", "状態", "操作"].map(str::to_owned).to_vec();
        assert_eq!(
            rows[1].words(&heads),
            "名前: works, URL: https://w.example/, 状態: 使用中（接続中のため、名前の変更と削除はできません）・見張る"
        );
        assert_eq!(rows[0].words(&heads), "名前: kei");
    }

    /// The line under the table that says a connection in use is not renamed or deleted shows only when a row is in use.
    #[test]
    fn the_in_use_note_shows_only_when_a_row_is_in_use() {
        use super::super::listtable::pad::Pad;
        let (t, _) = crate::i18n::table("ja");
        let names = vec!["kei".to_owned(), "works".to_owned()];
        let note = t.tr("far.manage_in_use_note");
        let mut pad = Pad::new();
        let mut m = Manage::new();
        pad.settle(&mut |ui| {
            show(&mut m, ui.ctx(), &t, &names, &BTreeMap::new(), &BTreeMap::new(), None, &|n| n == "works", &BTreeSet::new(), false);
        });
        assert!(pad.nodes.values().any(|(l, _)| l == note), "a row is in use: the line is there");
        let mut pad = Pad::new();
        pad.settle(&mut |ui| {
            show(&mut m, ui.ctx(), &t, &names, &BTreeMap::new(), &BTreeMap::new(), None, &|_| false, &BTreeSet::new(), false);
        });
        assert!(!pad.nodes.values().any(|(l, _)| l == note), "none is in use: no line");
    }

    #[test]
    fn a_form_whose_row_is_gone_closes_before_a_key_can_send_it() {
        let (t, _) = crate::i18n::table("ja");
        let ctx = egui::Context::default();
        let mut m = Manage::new();
        m.edit("kei");
        m.editing.as_mut().unwrap().text = "keiba".into();
        let run = |m: &mut Manage, names: &[String], enter: bool| {
            let mut input = egui::RawInput::default();
            if enter {
                input.events.push(egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut got = Pressed::Nothing;
            let mut out = ctx.run_ui(input, |ui| {
                got = show(m, ui.ctx(), &t, names, &BTreeMap::new(), &BTreeMap::new(), None, &|_| false, &BTreeSet::new(), false);
            });
            out.textures_delta.clear();
            got
        };
        let both = vec!["kei".to_owned(), "works".to_owned()];
        run(&mut m, &both, false);
        assert!(m.editing.is_some(), "the row is there: the form stays");
        // The list no longer has it, and Enter in the same frame sends nothing.
        assert_eq!(run(&mut m, &["works".to_owned()], true), Pressed::Nothing);
        assert!(m.editing.is_none());
        // A URL form closes the same way at its form step, but a check that is running is left alone.
        m.move_url("works", "https://w.example/");
        assert_eq!(run(&mut m, &both, false), Pressed::Nothing);
        run(&mut m, &["kei".to_owned()], false);
        assert!(m.moving.is_none());
    }
}
