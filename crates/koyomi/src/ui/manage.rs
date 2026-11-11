//! The box that renames, deletes and moves a `[[remote]]` of koyomi.toml (TODO 80): one row per connection with a
//! rename, a delete and a change of its URL. This file holds the box's state and its drawing; what a press does (the
//! checks, the thread that writes koyomi.toml) is `Far`'s.
//!
//! A connection the window is using (its current link, or the PC its page shows) offers neither a rename nor a delete:
//! renaming it would cut the link the window stands on. Its URL can be changed all the same: a PC that moved is
//! exactly when the current link is down. Esc closes the innermost thing: the question, else the editing, else the box.
//! While a press is carried out the box does not close and takes no new press.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use ito_a11y::Role;

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
}

impl Manage {
    pub(super) fn new() -> Manage {
        static TICKETS: AtomicU64 = AtomicU64::new(1);
        Manage { ticket: TICKETS.fetch_add(1, Ordering::Relaxed), editing: None, asking: None, moving: None, busy: false, line: None }
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

/// Draws the box and returns what was pressed. `used` says whether a row is in use.
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
            ui.set_width(520.0);
            ui.add_enabled_ui(!busy, |ui| {
                if let Some(e) = unread {
                    // koyomi.toml did not read: any row would fail to rename or delete, so none is drawn.
                    let text = t.tr("far.manage_unread");
                    let r = ui.label(egui::RichText::new(text).color(c.dim));
                    ito_a11y::say(&r, Role::Label, text);
                    let r = ui.add(egui::Label::new(egui::RichText::new(e).color(c.err)).wrap());
                    ito_a11y::say(&r, Role::Label, e);
                } else {
                    if names.is_empty() {
                        ui.label(egui::RichText::new(t.tr("far.manage_none")).color(c.dim));
                    }
                    egui::ScrollArea::vertical().id_salt("far-manage-rows").max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                        for name in names {
                            row(ui, t, m, name, used(name), &mut press, &mut to_edit, &mut to_ask, &mut to_url);
                        }
                    });
                    compare(ui, t, m, &mut press);
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

/// One row: the name and its two buttons, or the field being typed in.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    m: &mut Manage,
    name: &str,
    used: bool,
    press: &mut Pressed,
    to_edit: &mut Option<String>,
    to_ask: &mut Option<String>,
    to_url: &mut Option<String>,
) {
    let c = ito_theme::colors();
    let editing = m.editing.as_ref().is_some_and(|e| e.from == name);
    let moving = m.moving.as_ref().is_some_and(|v| v.from == name);
    let form = m.moving.as_ref().is_some_and(|v| v.from == name && v.step == UrlStep::Form);
    let (mut go, mut back, mut rename, mut delete, mut url) = (false, false, false, false, false);
    egui::Sides::new().height(32.0).show(
        ui,
        |ui| {
            if editing {
                if let Some(e) = &mut m.editing {
                    let r = ui.add(egui::TextEdit::singleline(&mut e.text).desired_width(220.0));
                    ito_a11y::say(&r, Role::Label, &t.trf("far.manage_name", &[name]));
                    if e.fresh {
                        r.request_focus();
                        e.fresh = false;
                    }
                }
            } else if moving {
                ui.label(egui::RichText::new(name).strong());
                if let Some(v) = &mut m.moving {
                    let r = ui.add_enabled(v.step == UrlStep::Form, egui::TextEdit::singleline(&mut v.text).desired_width(260.0));
                    ito_a11y::say(&r, Role::Label, &t.trf("far.manage_url_name", &[name]));
                    if v.fresh {
                        r.request_focus();
                        v.fresh = false;
                    }
                }
            } else {
                ui.label(egui::RichText::new(name).strong());
                if used {
                    ui.label(egui::RichText::new(t.tr("far.manage_in_use")).color(c.dim));
                }
            }
        },
        |ui| {
            if moving {
                back = ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("far.manage_back"))
                    .said(&format!("{} {name}", t.tr("far.manage_back")))
                    .show(ui)
                    .clicked();
                let said = format!("{} {name}", t.tr("far.manage_url_go"));
                let b =
                    ito_prefs::IconButton::new(ito_prefs::Mark::Link, t.tr("far.manage_url_go")).said(&said).tone(ito_prefs::Tone::Primary);
                go = ui.add_enabled(form, |ui: &mut egui::Ui| b.show(ui)).clicked();
            } else if editing {
                back = ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("far.manage_back"))
                    .said(&format!("{} {name}", t.tr("far.manage_back")))
                    .show(ui)
                    .clicked();
                go = ito_prefs::IconButton::new(ito_prefs::Mark::Pencil, t.tr("far.manage_go"))
                    .said(&format!("{} {name}", t.tr("far.manage_go")))
                    .tone(ito_prefs::Tone::Primary)
                    .show(ui)
                    .clicked();
            } else {
                let said = format!("{} {name}", t.tr("far.manage_delete"));
                let del = ito_prefs::IconButton::new(ito_prefs::Mark::Trash, t.tr("far.manage_delete")).said(&said);
                delete = ui.add_enabled(!used, |ui: &mut egui::Ui| del.show(ui)).clicked();
                let said = format!("{} {name}", t.tr("far.manage_rename"));
                let ren = ito_prefs::IconButton::new(ito_prefs::Mark::Pencil, t.tr("far.manage_rename")).said(&said);
                rename = ui.add_enabled(!used, |ui: &mut egui::Ui| ren.show(ui)).clicked();
                // Allowed on the connection in use: a PC that moved is exactly when the current link is down.
                let said = format!("{} {name}", t.tr("far.manage_url"));
                url = ito_prefs::IconButton::new(ito_prefs::Mark::Link, t.tr("far.manage_url")).said(&said).show(ui).clicked();
            }
        },
    );
    if go {
        if let Some(v) = m.moving.as_ref().filter(|v| v.from == name) {
            *press = Pressed::Url(v.from.clone(), v.text.clone());
        } else if let Some(e) = &m.editing {
            *press = Pressed::Rename(e.from.clone(), e.text.clone());
        }
    }
    if back {
        m.editing = None;
        if moving {
            m.moving = None;
        }
    }
    if url {
        *to_url = Some(name.to_owned());
    }
    if rename {
        *to_edit = Some(name.to_owned());
    }
    if delete {
        *to_ask = Some(name.to_owned());
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
        let frame = |m: &mut Manage, names: &[String]| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(show(m, ui.ctx(), &t, names, &BTreeMap::new(), &BTreeMap::new(), None, &used), Pressed::Nothing);
            });
            out.textures_delta.clear();
        };
        let mut m = Manage::new();
        frame(&mut m, &names);
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
                show(&mut m, ui.ctx(), &t, &names, &BTreeMap::new(), &BTreeMap::new(), Some("KOYOMI-E1023 x"), &used),
                Pressed::Nothing
            );
        });
        out.textures_delta.clear();
    }
}
