//! The `[[remote]]` names of koyomi.toml, and the two boxes that change them: the add box (a new connection, checked
//! before anything is written) and the manage box (rename and delete). The status-bar list (`Link::Far`) is what
//! drives the window on another PC; this file holds the names it lists and the threads that write koyomi.toml.
//!
//! Every call runs on its own thread; the answers come back on a channel and are taken at the top of each frame.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use ito_a11y::Role;
use koyomi_core::msg;

use super::manage::{self, Manage, Pressed};

/// What a thread brought back.
enum Got {
    Names(Vec<String>),
    /// The add box: its ticket, and the fingerprint that PC shows (or why not).
    Peeked(u64, Result<String, String>),
    /// The add box: its ticket, the name, and whether the check passed and the connection was written.
    Joined(u64, String, Result<(), String>),
    /// The manage box: its ticket, what was asked, and the outcome line (or why not).
    Managed(u64, Change, Result<String, String>),
}

/// What the manage box asked a thread to do to koyomi.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Change {
    /// The name it had, and the one it is to have.
    Rename(String, String),
    Delete(String),
}

/// Where the add-a-connection box is. Nothing is written before the last step has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AddStep {
    /// The name and the URL are typed.
    Form,
    /// The fingerprint is being fetched.
    Peeking,
    /// The fingerprint is shown, to be compared with what the other PC says. There is no way on without the
    /// person saying it is the same.
    Compare,
    /// The token is typed.
    Token,
    /// The check runs and, once it passed, the connection is written.
    Joining,
}

/// The add-a-connection box.
pub(super) struct Add {
    /// Which box this is: a result from a thread of an earlier box with the same name does not belong to this one.
    ticket: u64,
    pub(super) step: AddStep,
    pub(super) name: String,
    pub(super) url: String,
    /// The typed token: handed to the thread and emptied as soon as it has it.
    pub(super) token: String,
    /// The fingerprint the other PC showed.
    pub(super) fp: Option<String>,
    pub(super) error: Option<String>,
}

impl Add {
    fn new() -> Add {
        static TICKETS: AtomicU64 = AtomicU64::new(1);
        Add {
            ticket: TICKETS.fetch_add(1, Ordering::Relaxed),
            step: AddStep::Form,
            name: String::new(),
            url: String::new(),
            token: String::new(),
            fp: None,
            error: None,
        }
    }
}

/// The check, then the writing: the PC answers, the token drives a window, and only then the token goes to the
/// credential store and the `[[remote]]` to koyomi.toml. Nothing is written when the check fails, and the token is
/// not stored until koyomi.toml is known to take the connection; a koyomi.toml that cannot be written gives it back.
/// The check stays outside the lock (a slow PC must not hold up the others); the writing is under it.
fn join(name: &str, url: &str, fingerprint: &str, token: &str) -> Result<(), String> {
    crate::link::probe(name, url, fingerprint, token)?;
    let secret = format!("{name}-token");
    let new = koyomi_core::config::Remote {
        name: name.to_owned(),
        url: url.to_owned(),
        fingerprint: fingerprint.to_owned(),
        token: format!("secret:{secret}"),
    };
    let _held = crate::remote::edits();
    let (path, text) = crate::remote::prepare_add(&new).map_err(|e| format!("{name}: {e}"))?;
    crate::remote::finish_add(name, &secret, token, &path, text, &crate::remote::Keyring, ito_common::write_atomic)
}

/// Whether Enter presses the button that goes on at this step. Not at the fingerprint: 同じだった is the person
/// saying they compared it, and the Enter that sent the form, pressed again or held, must not say that for them.
fn enter_goes_on(step: AddStep) -> bool {
    step != AddStep::Compare
}

pub struct Far {
    /// The `[[remote]]` names, once read.
    names: Vec<String>,
    tx: Sender<Got>,
    rx: Receiver<Got>,
    ctx: egui::Context,
    /// The add-a-connection box, while it is open.
    pub(super) add: Option<Add>,
    /// The connection just added: the window goes to it (taken by [`Far::take_joined`]).
    joined: Option<String>,
    /// The manage box (rename and delete), while it is open.
    pub(super) manage: Option<Manage>,
    /// The state folder: a rename moves the acknowledgement file there.
    state: Option<std::path::PathBuf>,
}

impl Far {
    /// Nothing read: no names.
    pub fn idle(ctx: &egui::Context) -> Far {
        let (tx, rx) = mpsc::channel();
        Far { names: Vec::new(), tx, rx, ctx: ctx.clone(), add: None, joined: None, manage: None, state: None }
    }

    /// Reads the `[[remote]]` names off the UI thread. `state` is the state folder (`None`: nothing is moved there).
    pub fn start(ctx: &egui::Context, state: Option<std::path::PathBuf>) -> Far {
        let mut far = Far::idle(ctx);
        far.state = state;
        let (tx, ctx) = (far.tx.clone(), far.ctx.clone());
        std::thread::spawn(move || {
            if let Ok(names) = crate::remote::names() {
                let _ = tx.send(Got::Names(names));
                ctx.request_repaint();
            }
        });
        far
    }

    /// Test only: what the name thread would send.
    #[cfg(test)]
    pub(super) fn send_names(&self, names: Vec<String>) {
        let _ = self.tx.send(Got::Names(names));
    }

    /// Whether any `[[remote]]` is set.
    pub fn any(&self) -> bool {
        !self.names.is_empty()
    }

    /// The `[[remote]]` names, for 別の PC に送る (TODO 111).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    #[cfg(test)]
    pub(super) fn set_names(&mut self, names: Vec<String>) {
        self.names = names;
    }

    fn spawn(&mut self, work: impl FnOnce() -> Got + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }

    /// Takes what the threads sent.
    pub(super) fn drain(&mut self) {
        while let Ok(got) = self.rx.try_recv() {
            self.take(got);
        }
    }

    fn take(&mut self, got: Got) {
        match got {
            Got::Names(names) => self.names = names,
            Got::Peeked(ticket, r) => {
                if let Some(a) = self.add.as_mut().filter(|a| a.ticket == ticket && a.step == AddStep::Peeking) {
                    match r {
                        Ok(fp) => {
                            a.fp = Some(fp);
                            a.step = AddStep::Compare;
                        }
                        Err(e) => {
                            a.step = AddStep::Form;
                            a.error = Some(e);
                        }
                    }
                }
            }
            Got::Joined(ticket, name, r) => {
                let waiting = self.add.as_ref().is_some_and(|a| a.ticket == ticket && a.step == AddStep::Joining);
                match r {
                    Ok(()) => {
                        // It is written, so it is a name now, even if the box was given up meanwhile.
                        if !self.names.contains(&name) {
                            self.names.push(name.clone());
                        }
                        if waiting {
                            self.add = None;
                            self.joined = Some(name);
                        }
                    }
                    Err(e) if waiting => {
                        if let Some(a) = &mut self.add {
                            a.step = AddStep::Token;
                            a.error = Some(e);
                        }
                    }
                    Err(_) => {}
                }
            }
            Got::Managed(ticket, change, r) => {
                // It is written, so it is so, even if the box was given up meanwhile.
                if r.is_ok() {
                    match &change {
                        Change::Rename(old, new) => {
                            if let Some(n) = self.names.iter_mut().find(|n| *n == old) {
                                n.clone_from(new);
                            }
                        }
                        Change::Delete(name) => self.names.retain(|n| n != name),
                    }
                }
                if let Some(m) = self.manage.as_mut().filter(|m| m.ticket == ticket && m.busy) {
                    m.busy = false;
                    match r {
                        Ok(line) => {
                            m.editing = None;
                            m.asking = None;
                            m.line = Some(Ok(line));
                        }
                        Err(e) => m.line = Some(Err(e)),
                    }
                }
            }
        }
    }

    /// Opens the add-a-connection box (and closes the manage box, unless it is busy).
    pub fn open_add(&mut self) {
        if self.manage_busy() {
            return;
        }
        self.manage = None;
        if self.add.is_none() {
            self.add = Some(Add::new());
        }
    }

    /// Opens the manage box (and closes the add box).
    pub fn open_manage(&mut self) {
        self.add = None;
        if self.manage.is_none() {
            self.manage = Some(Manage::new());
        }
    }

    /// Closes the manage box, unless a change is being carried out. A pick of another link does this: the box goes
    /// with this browser anyway, and its Enter must not rename anything while the switch waits behind a question.
    pub fn close_manage(&mut self) {
        if !self.manage_busy() {
            self.manage = None;
        }
    }

    /// Whether the manage box is open.
    #[cfg(test)]
    pub fn managing(&self) -> bool {
        self.manage.is_some()
    }

    /// Whether a rename or a delete is being carried out: the link is not switched and the box not closed meanwhile.
    pub fn manage_busy(&self) -> bool {
        self.manage.as_ref().is_some_and(|m| m.busy)
    }

    /// 変える: the typed name is judged against the other names, and a free one is written on a thread. The same
    /// name writes nothing. `link` is the window's current link (`None`: this PC).
    pub(super) fn manage_rename(&mut self, _t: &ito_i18n::Table, link: Option<&str>, old: &str, new: &str) {
        use crate::remote::NameWhy;
        let Some(m) = self.manage.as_mut().filter(|m| !m.busy) else { return };
        let (old, new) = (old.trim().to_owned(), new.trim().to_owned());
        if new == old {
            m.editing = None;
            m.line = None;
            return;
        }
        let others: Vec<String> = self.names.iter().filter(|n| **n != old).cloned().collect();
        let bad = match crate::remote::check_name(&new, &others) {
            Err(NameWhy::Empty) => Some(msg::line(msg::NAME_EMPTY, &[])),
            Err(NameWhy::Taken) => Some(msg::line(msg::NAME_TAKEN, &[&new])),
            Err(NameWhy::Bad) => Some(msg::line(msg::NAME_UNUSABLE, &[&new])),
            Ok(()) if manage::in_use(link, &old) => Some(msg::line(msg::CONNECTION_IN_USE, &[&old])),
            Ok(()) => None,
        };
        if bad.is_some() {
            m.line = bad.map(Err);
            return;
        }
        m.busy = true;
        m.line = None;
        let (ticket, state) = (m.ticket, self.state.clone());
        self.spawn(move || {
            let r = crate::remote::rename(&old, &new, state.as_deref());
            Got::Managed(ticket, Change::Rename(old, new), r)
        });
    }

    /// 消す on the question: written on a thread, unless the connection is in use.
    pub(super) fn manage_delete(&mut self, _t: &ito_i18n::Table, link: Option<&str>, name: &str) {
        let Some(m) = self.manage.as_mut().filter(|m| !m.busy) else { return };
        let name = name.trim().to_owned();
        if manage::in_use(link, &name) {
            m.line = Some(Err(msg::line(msg::CONNECTION_IN_USE, &[&name])));
            return;
        }
        m.busy = true;
        m.line = None;
        let ticket = m.ticket;
        self.spawn(move || {
            let r = crate::remote::remove(&name);
            Got::Managed(ticket, Change::Delete(name), r)
        });
    }

    /// The manage box. `link` is the window's current link (`None`: this PC).
    pub fn manage_box(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, link: Option<&str>) {
        let Some(m) = self.manage.as_mut() else { return };
        let used = |n: &str| manage::in_use(link, n);
        match manage::show(m, ctx, t, &self.names, &used) {
            Pressed::Nothing => {}
            Pressed::Close => {
                if !m.busy {
                    self.manage = None;
                }
            }
            Pressed::Add => self.open_add(),
            Pressed::Rename(old, new) => self.manage_rename(t, link, &old, &new),
            Pressed::Delete(name) => self.manage_delete(t, link, &name),
        }
    }

    /// Whether the add box is open.
    #[cfg(test)]
    pub fn adding(&self) -> bool {
        self.add.is_some()
    }

    /// Closes the add box. A check that is still running ends on its own; what it brings back is dropped.
    pub fn add_cancel(&mut self) {
        self.add = None;
    }

    /// The name and the URL are checked: what cannot be used is said in the box, else the fingerprint is fetched.
    pub(super) fn add_name_go(&mut self, _t: &ito_i18n::Table) {
        use crate::remote::NameWhy;
        let Some(a) = self.add.as_mut().filter(|a| a.step == AddStep::Form) else { return };
        let (name, url) = (a.name.trim().to_owned(), a.url.trim().to_owned());
        let bad = match crate::remote::check_name(&name, &self.names) {
            Err(NameWhy::Empty) => Some(msg::line(msg::NAME_EMPTY, &[])),
            Err(NameWhy::Taken) => Some(msg::line(msg::NAME_TAKEN, &[&name])),
            Err(NameWhy::Bad) => Some(msg::line(msg::NAME_UNUSABLE, &[&name])),
            Ok(()) if url.len() <= "https://".len() || !url.starts_with("https://") || url.contains(char::is_whitespace) => {
                Some(msg::line(msg::URL_BAD, &[]))
            }
            Ok(()) => None,
        };
        if bad.is_some() {
            a.error = bad;
            return;
        }
        a.error = None;
        a.step = AddStep::Peeking;
        let ticket = a.ticket;
        self.spawn(move || Got::Peeked(ticket, crate::remote::peek_fingerprint(&url)));
    }

    /// 同じだった: the fingerprint is what the other PC says. Only from the step that shows it.
    pub(super) fn add_same(&mut self) {
        if let Some(a) = self.add.as_mut().filter(|a| a.step == AddStep::Compare) {
            a.step = AddStep::Token;
            a.error = None;
        }
    }

    /// つなぐ: the typed token leaves the box here (the box is left holding none) with what the check needs, and
    /// the box goes to its waiting step. `None`: the token is empty, or this is not the step for it.
    pub(super) fn add_hand_off(&mut self, _t: &ito_i18n::Table) -> Option<(String, String, String, String)> {
        let a = self.add.as_mut().filter(|a| a.step == AddStep::Token)?;
        if a.token.trim().is_empty() {
            a.error = Some(msg::line(msg::TOKEN_EMPTY, &[]));
            return None;
        }
        let fingerprint = a.fp.clone()?;
        let token = std::mem::take(&mut a.token).trim().to_owned();
        a.error = None;
        a.step = AddStep::Joining;
        Some((a.name.trim().to_owned(), a.url.trim().to_owned(), fingerprint, token))
    }

    /// つなぐ: the check and then the writing, on a thread.
    pub(super) fn add_join(&mut self, t: &ito_i18n::Table) {
        let ticket = self.add.as_ref().map_or(0, |a| a.ticket);
        let Some((name, url, fingerprint, token)) = self.add_hand_off(t) else { return };
        self.spawn(move || {
            let r = join(&name, &url, &fingerprint, &token);
            Got::Joined(ticket, name, r)
        });
    }

    /// The name of the connection just added, once: the window goes to it.
    pub fn take_joined(&mut self) -> Option<String> {
        self.joined.take()
    }

    /// The add-a-connection box. Esc closes it; Enter presses the button that goes on, except at the fingerprint.
    /// `under_card`: the troubleshooting card is open over the page, so the box does not take the focus from its field.
    pub fn add_box(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, under_card: bool) {
        let Some(step) = self.add.as_ref().map(|a| a.step) else { return };
        let (esc, enter) = ctx.input_mut(|i| {
            (i.consume_key(egui::Modifiers::NONE, egui::Key::Escape), i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        });
        if esc {
            self.add_cancel();
            return;
        }
        let c = ito_theme::colors();
        let mut pressed = (enter && enter_goes_on(step)).then_some(0);
        if let Some(a) = &mut self.add {
            let (lead, go) = match step {
                AddStep::Form | AddStep::Peeking => (t.tr("far.add_lead"), t.tr("far.add_next")),
                AddStep::Compare => (t.tr("far.add_compare"), t.tr("far.add_same")),
                AddStep::Token | AddStep::Joining => (t.tr("far.add_token_lead"), t.tr("far.add_join")),
            };
            egui::Window::new(t.tr("far.add_title"))
                .id(egui::Id::new("far-add"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_width(480.0);
                    ui.add(egui::Label::new(lead).wrap());
                    let busy = matches!(step, AddStep::Peeking | AddStep::Joining);
                    match step {
                        AddStep::Form | AddStep::Peeking => {
                            ui.add_enabled_ui(!busy, |ui| {
                                egui::Grid::new("far-add-fields").num_columns(2).show(ui, |ui| {
                                    ui.label(t.tr("far.add_name"));
                                    let r = ui.add(
                                        egui::TextEdit::singleline(&mut a.name).hint_text(t.tr("far.add_name_hint")).desired_width(300.0),
                                    );
                                    ito_a11y::say(&r, Role::Label, t.tr("far.add_name"));
                                    if a.name.is_empty() && a.url.is_empty() && !under_card {
                                        r.request_focus();
                                    }
                                    ui.end_row();
                                    ui.label(t.tr("far.add_url"));
                                    let r =
                                        ui.add(egui::TextEdit::singleline(&mut a.url).hint_text("https://pc:8443").desired_width(300.0));
                                    ito_a11y::say(&r, Role::Label, t.tr("far.add_url"));
                                    ui.end_row();
                                });
                            });
                        }
                        AddStep::Compare => {
                            let fp = a.fp.clone().unwrap_or_default();
                            let r = ui.add(egui::Label::new(egui::RichText::new(&fp).monospace().strong()).wrap().selectable(true));
                            ito_a11y::say(&r, Role::Label, &fp);
                        }
                        AddStep::Token | AddStep::Joining => {
                            ui.add_enabled_ui(!busy, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(t.tr("far.add_token"));
                                    let r = ui.add(egui::TextEdit::singleline(&mut a.token).password(true).desired_width(300.0));
                                    ito_a11y::say(&r, Role::Label, t.tr("far.add_token"));
                                    if step == AddStep::Token && a.token.is_empty() && !under_card {
                                        r.request_focus();
                                    }
                                });
                            });
                        }
                    }
                    if busy {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(t.tr(if step == AddStep::Peeking { "far.add_peeking" } else { "far.add_joining" }));
                        });
                    }
                    if let Some(e) = &a.error {
                        ui.add(egui::Label::new(egui::RichText::new(e).color(c.err)).wrap());
                    }
                    if let Some(n) = super::box_answer_on(ui, !busy, ito_prefs::Mark::Plus, go, t.tr("far.add_cancel")) {
                        pressed = Some(n);
                    }
                });
        }
        match (pressed, step) {
            (Some(1), _) => self.add_cancel(),
            (Some(_), AddStep::Form) => self.add_name_go(t),
            (Some(_), AddStep::Compare) => self.add_same(),
            (Some(_), AddStep::Token) => self.add_join(t),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adding() -> Far {
        let mut f = Far::idle(&egui::Context::default());
        f.names = vec!["old".to_owned()];
        f.open_add();
        f
    }

    fn ticket(f: &Far) -> u64 {
        f.add.as_ref().unwrap().ticket
    }

    fn ja() -> ito_i18n::Table {
        crate::i18n::table("ja").0
    }

    /// Puts the box at a step as if the earlier ones had passed (no thread, no network).
    fn at(f: &mut Far, step: AddStep) {
        let a = f.add.as_mut().unwrap();
        a.name = "new".into();
        a.url = "https://pc:8443".into();
        a.step = step;
    }

    #[test]
    fn the_add_box_refuses_a_bad_name_or_address_in_the_box_and_starts_nothing() {
        let t = ja();
        for (name, url) in [
            ("", "https://pc"),
            ("old", "https://pc"),
            ("a b", "https://pc"),
            ("a:b", "https://pc"),
            ("ok", "http://pc"),
            ("ok", "https://"),
            ("ok", "https://a b"),
        ] {
            let mut f = adding();
            let a = f.add.as_mut().unwrap();
            (a.name, a.url) = (name.into(), url.into());
            f.add_name_go(&t);
            let a = f.add.as_ref().unwrap();
            assert_eq!(a.step, AddStep::Form, "{name:?} {url:?}");
            assert!(a.error.is_some(), "{name:?} {url:?}");
        }
    }

    #[test]
    fn there_is_no_token_step_without_the_same_press() {
        let t = ja();
        let mut f = adding();
        at(&mut f, AddStep::Peeking);
        f.take(Got::Peeked(ticket(&f), Ok("ab".repeat(32))));
        assert_eq!(f.add.as_ref().unwrap().step, AddStep::Compare);
        // Nothing but the press gets past it.
        assert!(f.add_hand_off(&t).is_none());
        f.add.as_mut().unwrap().token = "t".into();
        assert!(f.add_hand_off(&t).is_none());
        assert_eq!(f.add.as_ref().unwrap().step, AddStep::Compare);
        f.add_same();
        assert_eq!(f.add.as_ref().unwrap().step, AddStep::Token);
        // The press does nothing from any other step.
        let mut g = adding();
        at(&mut g, AddStep::Form);
        g.add_same();
        assert_eq!(g.add.as_ref().unwrap().step, AddStep::Form);
    }

    #[test]
    fn enter_does_not_say_the_fingerprint_is_the_same() {
        assert!(!enter_goes_on(AddStep::Compare));
        assert!(enter_goes_on(AddStep::Form));
        assert!(enter_goes_on(AddStep::Token));
    }

    #[test]
    fn a_failed_peek_goes_back_to_the_form_with_the_words() {
        let mut f = adding();
        at(&mut f, AddStep::Peeking);
        f.take(Got::Peeked(ticket(&f), Err("つながらない".into())));
        let a = f.add.as_ref().unwrap();
        assert_eq!((a.step, a.error.as_deref(), a.fp.is_none()), (AddStep::Form, Some("つながらない"), true));
        // An answer after the box moved on changes nothing.
        let mut f = adding();
        at(&mut f, AddStep::Compare);
        f.take(Got::Peeked(ticket(&f), Ok("x".into())));
        assert_eq!(f.add.as_ref().unwrap().step, AddStep::Compare);
        assert!(f.add.as_ref().unwrap().fp.is_none());
    }

    #[test]
    fn a_late_peek_of_an_earlier_box_with_the_same_name_changes_nothing() {
        let mut f = adding();
        at(&mut f, AddStep::Peeking);
        let old = ticket(&f);
        f.add_cancel();
        f.open_add();
        at(&mut f, AddStep::Peeking);
        assert_ne!(ticket(&f), old);
        f.take(Got::Peeked(old, Ok("aa".repeat(32))));
        let a = f.add.as_ref().unwrap();
        assert_eq!((a.step, a.fp.is_none(), a.error.is_none()), (AddStep::Peeking, true, true));
        f.take(Got::Peeked(old, Err("late".into())));
        assert_eq!(f.add.as_ref().unwrap().step, AddStep::Peeking);
        assert!(f.add.as_ref().unwrap().error.is_none());
    }

    #[test]
    fn a_late_join_of_an_earlier_box_adds_the_name_but_leaves_a_new_box_alone() {
        let mut f = adding();
        at(&mut f, AddStep::Joining);
        let old = ticket(&f);
        f.add_cancel();
        f.open_add();
        at(&mut f, AddStep::Joining);
        f.take(Got::Joined(old, "new".into(), Ok(())));
        assert_eq!(f.names, ["old", "new"], "it was written, so it is a name");
        assert_eq!(f.add.as_ref().unwrap().step, AddStep::Joining, "the new box is as it was");
        assert!(f.take_joined().is_none());
        let mut f = adding();
        at(&mut f, AddStep::Joining);
        let old = ticket(&f);
        f.add_cancel();
        f.open_add();
        at(&mut f, AddStep::Joining);
        f.take(Got::Joined(old, "new".into(), Err("late".into())));
        assert_eq!(f.names, ["old"]);
        let a = f.add.as_ref().unwrap();
        assert_eq!((a.step, a.error.is_none()), (AddStep::Joining, true));
    }

    #[test]
    fn a_failed_write_says_why_and_whether_the_token_is_left_and_never_the_token() {
        let w = crate::remote::write_failed("pc", "読み取り専用", Ok(()));
        assert!(msg::CONFIG_WRITE_FAILED.is_in(&w) && w.contains("pc") && w.contains("読み取り専用"), "{w}");
        let w = crate::remote::write_failed("pc", "読み取り専用", Err("アクセス拒否".into()));
        assert!(msg::CONFIG_WRITE_FAILED_TOKEN_LEFT.is_in(&w) && w.contains("読み取り専用") && w.contains("アクセス拒否"), "{w}");
    }

    #[test]
    fn the_token_is_gone_from_the_box_once_handed_off_and_an_empty_one_goes_nowhere() {
        let t = ja();
        let mut f = adding();
        at(&mut f, AddStep::Token);
        f.add.as_mut().unwrap().fp = Some("ab".repeat(32));
        assert!(f.add_hand_off(&t).is_none(), "empty token");
        assert!(f.add.as_ref().unwrap().error.is_some());
        assert_eq!(f.add.as_ref().unwrap().step, AddStep::Token);
        f.add.as_mut().unwrap().token = "  secret  ".into();
        let (name, url, fp, token) = f.add_hand_off(&t).unwrap();
        assert_eq!((name.as_str(), url.as_str(), token.as_str()), ("new", "https://pc:8443", "secret"));
        assert_eq!(fp, "ab".repeat(32));
        let a = f.add.as_ref().unwrap();
        assert!(a.token.is_empty(), "the box holds no token");
        assert_eq!(a.step, AddStep::Joining);
        // A second press cannot hand off again.
        assert!(f.add_hand_off(&t).is_none());
    }

    #[test]
    fn a_failed_check_stays_in_the_box_and_gives_no_connection_to_go_to() {
        let t = ja();
        let mut f = adding();
        at(&mut f, AddStep::Token);
        f.add.as_mut().unwrap().fp = Some("ab".repeat(32));
        f.add.as_mut().unwrap().token = "bad".into();
        f.add_hand_off(&t).unwrap();
        f.take(Got::Joined(ticket(&f), "new".into(), Err("new: 401".into())));
        let a = f.add.as_ref().expect("still open");
        assert_eq!((a.step, a.error.as_deref()), (AddStep::Token, Some("new: 401")));
        assert!(a.token.is_empty());
        assert!(f.take_joined().is_none());
        assert_eq!(f.names, ["old"], "the name is not added");
    }

    #[test]
    fn a_passed_check_closes_the_box_adds_the_name_and_names_the_link_once() {
        let t = ja();
        let mut f = adding();
        at(&mut f, AddStep::Token);
        f.add.as_mut().unwrap().fp = Some("ab".repeat(32));
        f.add.as_mut().unwrap().token = "ok".into();
        f.add_hand_off(&t).unwrap();
        f.take(Got::Joined(ticket(&f), "new".into(), Ok(())));
        assert!(f.add.is_none());
        assert_eq!(f.names, ["old", "new"]);
        assert_eq!(f.take_joined().as_deref(), Some("new"));
        assert!(f.take_joined().is_none());
    }

    #[test]
    fn a_check_that_passes_after_the_box_was_given_up_adds_the_name_but_does_not_switch() {
        let mut f = adding();
        at(&mut f, AddStep::Joining);
        f.add_cancel();
        assert!(!f.adding());
        f.take(Got::Joined(0, "new".into(), Ok(())));
        assert_eq!(f.names, ["old", "new"]);
        assert!(f.take_joined().is_none());
    }

    fn managing() -> Far {
        let mut f = Far::idle(&egui::Context::default());
        f.names = vec!["kei".to_owned(), "works".to_owned()];
        f.open_manage();
        f
    }

    fn mticket(f: &Far) -> u64 {
        f.manage.as_ref().unwrap().ticket
    }

    fn mline(f: &Far) -> Option<Result<String, String>> {
        f.manage.as_ref().unwrap().line.clone()
    }

    #[test]
    fn a_pick_closes_the_manage_box_unless_it_is_busy() {
        let mut f = managing();
        f.manage.as_mut().unwrap().busy = true;
        f.close_manage();
        assert!(f.managing(), "a box that is writing stays");
        f.manage.as_mut().unwrap().busy = false;
        f.close_manage();
        assert!(!f.managing(), "an idle box goes with the pick");
    }

    #[test]
    fn one_box_at_a_time_and_a_busy_manage_box_stays() {
        let mut f = adding();
        f.open_manage();
        assert!(f.managing() && !f.adding(), "opening the manage box closes the add box");
        f.open_add();
        assert!(f.adding() && !f.managing(), "opening the add box closes the manage box");
        f.open_manage();
        f.manage.as_mut().unwrap().busy = true;
        f.open_add();
        assert!(f.managing() && !f.adding(), "a busy manage box is not closed");
        f.manage.as_mut().unwrap().busy = false;
    }

    #[test]
    fn a_rename_to_the_same_name_writes_nothing() {
        let t = ja();
        let mut f = managing();
        f.manage.as_mut().unwrap().edit("kei");
        f.manage_rename(&t, None, "kei", "  kei ");
        let m = f.manage.as_ref().unwrap();
        assert!(m.editing.is_none() && !m.busy && m.line.is_none());
    }

    #[test]
    fn a_refused_name_is_said_on_the_status_line_and_the_editing_stays() {
        let t = ja();
        for (new, why) in [("", msg::NAME_EMPTY), ("   ", msg::NAME_EMPTY), ("works", msg::NAME_TAKEN), ("a b", msg::NAME_UNUSABLE)] {
            let mut f = managing();
            f.manage.as_mut().unwrap().edit("kei");
            f.manage_rename(&t, None, "kei", new);
            let m = f.manage.as_ref().unwrap();
            assert!(m.editing.is_some() && !m.busy, "{new:?}");
            let want = match why {
                msg::NAME_TAKEN => msg::line(why, &["works"]),
                msg::NAME_UNUSABLE => msg::line(why, &["a b"]),
                _ => msg::line(why, &[]),
            };
            assert_eq!(m.line, Some(Err(want)), "{new:?}");
        }
    }

    #[test]
    fn a_connection_in_use_is_neither_renamed_nor_deleted() {
        let t = ja();
        // The window's link, even when it does not answer.
        let mut f = managing();
        f.manage_rename(&t, Some("kei"), "kei", "keiba");
        assert_eq!(mline(&f), Some(Err(msg::line(msg::CONNECTION_IN_USE, &["kei"]))));
        f.manage_delete(&t, Some("kei"), "kei");
        assert_eq!(mline(&f), Some(Err(msg::line(msg::CONNECTION_IN_USE, &["kei"]))));
        assert!(!f.manage_busy());
    }

    #[test]
    fn an_answer_changes_the_names_and_the_box_that_asked() {
        let mut f = managing();
        f.manage.as_mut().unwrap().edit("kei");
        f.manage.as_mut().unwrap().busy = true;
        let t = mticket(&f);
        f.take(Got::Managed(t, Change::Rename("kei".into(), "keiba".into()), Ok("kei の名前を keiba に変えた".into())));
        assert_eq!(f.names, ["keiba", "works"], "renamed in place");
        let m = f.manage.as_ref().unwrap();
        assert!(!m.busy && m.editing.is_none());
        assert_eq!(m.line, Some(Ok("kei の名前を keiba に変えた".into())));
        // A failure keeps the editing and shows the error; the names stay.
        f.manage.as_mut().unwrap().edit("works");
        f.manage.as_mut().unwrap().busy = true;
        f.take(Got::Managed(t, Change::Rename("works".into(), "x".into()), Err("書けない".into())));
        let m = f.manage.as_ref().unwrap();
        assert!(!m.busy && m.editing.is_some());
        assert_eq!(m.line, Some(Err("書けない".into())));
        assert_eq!(f.names, ["keiba", "works"]);
        // A delete leaves the list.
        f.manage.as_mut().unwrap().busy = true;
        f.take(Got::Managed(t, Change::Delete("keiba".into()), Ok("keiba を消した".into())));
        assert_eq!(f.names, ["works"]);
        f.manage.as_mut().unwrap().busy = true;
        f.take(Got::Managed(t, Change::Delete("works".into()), Ok("works を消した".into())));
        assert!(f.names.is_empty() && !f.any());
    }

    #[test]
    fn a_late_answer_of_an_earlier_box_changes_the_names_but_not_a_new_box() {
        let mut f = managing();
        f.manage.as_mut().unwrap().busy = true;
        let old = mticket(&f);
        f.manage = None;
        f.open_manage();
        f.manage.as_mut().unwrap().busy = true;
        f.take(Got::Managed(old, Change::Delete("kei".into()), Ok("kei を消した".into())));
        assert_eq!(f.names, ["works"], "it was written, so it is so");
        let m = f.manage.as_ref().unwrap();
        assert!(m.busy && m.line.is_none(), "the new box waits for its own answer");
    }

    #[test]
    fn the_manage_box_draws_in_each_state() {
        let t = ja();
        let ctx = egui::Context::default();
        let mut f = managing();
        let frame = |f: &mut Far, link: Option<&str>| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f.manage_box(ui.ctx(), &t, link));
            out.textures_delta.clear();
        };
        frame(&mut f, Some("kei"));
        f.manage.as_mut().unwrap().edit("works");
        frame(&mut f, None);
        f.manage.as_mut().unwrap().ask("works");
        frame(&mut f, None);
        f.names.clear();
        frame(&mut f, None);
        assert!(f.managing());
    }
}
