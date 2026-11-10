//! Another PC's engine in the window (TODO 80): the PC list at the top switches the window to
//! one of the `[[remote]]` of koyomi.toml, and the page shows that engine's nets and today's
//! runs, a run's jobs and a job's log, with 今すぐ流す and 強制終了 behind a yes or no, and a
//! net's hold and its next run skipped (`POST /nets/hold|release|skip-next`) the same way. A net
//! with `[params]` asks for its values in the same yes or no, as 今すぐ実行 on this PC does.
//!
//! Every call goes over HTTPS with the certificate pinned (`crate::remote`) on its own thread;
//! the answers come back on a channel and are kept only while that PC is still the one shown.
//! What the other PC lets this one do is its `[[user]]` role's: a refusal shows as its words.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use ito_a11y::Role;
use koyomi_core::run::{Run, State};
use serde::Deserialize;

use super::graph;
use super::manage::{self, Manage, Pressed};
use crate::remote::query;
use crate::show;

/// How often the shown PC is asked again for its nets and runs.
const EVERY: Duration = Duration::from_secs(5);

/// One net as `GET /nets` gives it.
#[derive(Debug, Clone, Deserialize)]
struct Net {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    standing: String,
    #[serde(default)]
    held: bool,
    /// `[params]`: name and default.
    #[serde(default)]
    params: BTreeMap<String, String>,
}

/// What a thread brought back, with the PC it asked.
enum Got {
    Names(Vec<String>),
    Nets(String, Result<serde_json::Value, String>),
    Runs(String, Result<serde_json::Value, String>),
    /// PC, run, job, the log's text.
    Log(String, String, String, Result<serde_json::Value, String>),
    /// PC, the line to show under the heading.
    Did(String, Result<String, String>),
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

/// A press waiting for its yes.
#[derive(Debug, Clone, PartialEq)]
enum Act {
    /// Run the net (its id) now, with its `[params]` as name, default, what the field holds.
    Run(String, Vec<(String, String, String)>),
    /// Kill the run (its id).
    Stop(String),
    /// Run the ended run (its id) again from where it stopped.
    Rerun(String),
    /// Hold the net (its id), or take the hold off (`false`).
    Hold(String, bool),
    /// Skip the net's (its id) next scheduled run, or take the skips back (`false`).
    SkipNext(String, bool),
}

impl Act {
    /// The mark on the button that asks for it, and on the answer that does it.
    fn mark(&self) -> ito_prefs::Mark {
        match self {
            Act::Run(..) | Act::Hold(_, false) => ito_prefs::Mark::Play,
            Act::Stop(_) => ito_prefs::Mark::Stop,
            Act::Rerun(_) => ito_prefs::Mark::Rerun,
            Act::Hold(_, true) => ito_prefs::Mark::Pause,
            Act::SkipNext(_, true) => ito_prefs::Mark::Skip,
            Act::SkipNext(_, false) => ito_prefs::Mark::Undo,
        }
    }
}

pub struct Far {
    /// The `[[remote]]` names, once read; empty hides the PC list.
    names: Vec<String>,
    /// The PC shown; `None` is this one.
    on: Option<String>,
    nets: Option<Result<Vec<Net>, String>>,
    paused: bool,
    runs: Option<Result<Vec<Run>, String>>,
    run_sel: Option<String>,
    job_sel: Option<String>,
    log: Option<Result<String, String>>,
    asking: Option<Act>,
    said: Option<Result<String, String>>,
    /// When the nets and runs were last asked for.
    asked: Option<Instant>,
    /// Calls not back yet: no new round starts over them.
    pending: usize,
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
    /// Nothing read: the PC list stays hidden.
    pub fn idle(ctx: &egui::Context) -> Far {
        let (tx, rx) = mpsc::channel();
        Far {
            names: Vec::new(),
            on: None,
            nets: None,
            paused: false,
            runs: None,
            run_sel: None,
            job_sel: None,
            log: None,
            asking: None,
            said: None,
            asked: None,
            pending: 0,
            tx,
            rx,
            ctx: ctx.clone(),
            add: None,
            joined: None,
            manage: None,
            state: None,
        }
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

    /// Whether there is another PC to switch to.
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

    /// Whether another PC is shown (the window's own page is not).
    pub fn shown(&self) -> bool {
        self.on.is_some()
    }

    /// Back to this PC's page.
    pub fn home(&mut self) {
        self.switch(None);
    }

    fn spawn(&mut self, work: impl FnOnce() -> Got + Send + 'static) {
        self.pending += 1;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }

    /// Takes what the threads sent; an answer from a PC no longer shown is dropped.
    pub(super) fn drain(&mut self) {
        while let Ok(got) = self.rx.try_recv() {
            self.take(got);
        }
    }

    fn take(&mut self, got: Got) {
        if !matches!(got, Got::Names(_)) {
            self.pending = self.pending.saturating_sub(1);
        }
        let here = |pc: &str| self.on.as_deref() == Some(pc);
        match got {
            Got::Names(names) => self.names = names,
            Got::Nets(pc, j) if here(&pc) => {
                self.paused = j.as_ref().is_ok_and(|j| j["paused"].as_bool() == Some(true));
                self.nets = Some(j.and_then(|j| serde_json::from_value(j["nets"].clone()).map_err(|e| e.to_string())));
            }
            Got::Runs(pc, j) if here(&pc) => {
                self.runs = Some(j.and_then(|j| serde_json::from_value(j["runs"].clone()).map_err(|e| e.to_string())));
            }
            Got::Log(pc, run, job, j)
                if here(&pc) && self.run_sel.as_deref() == Some(run.as_str()) && self.job_sel.as_deref() == Some(job.as_str()) =>
            {
                self.log = Some(j.map(|j| j["log"].as_str().unwrap_or_default().to_owned()));
            }
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
            Got::Did(pc, said) if here(&pc) => {
                self.said = Some(said);
                // What was done shows at once, not at the next round.
                self.asked = None;
            }
            _ => {}
        }
    }

    /// Shows `pc` (`None`: this one), from a clean page.
    fn switch(&mut self, pc: Option<String>) {
        // A change to koyomi.toml is being carried out: the page stays where it is until it is done.
        if pc == self.on || self.manage_busy() {
            return;
        }
        let names = std::mem::take(&mut self.names);
        let (tx, rx, ctx) = (self.tx.clone(), std::mem::replace(&mut self.rx, mpsc::channel().1), self.ctx.clone());
        let (add, joined, manage, state) = (self.add.take(), self.joined.take(), self.manage.take(), self.state.take());
        *self = Far { names, on: pc, tx, rx, add, joined, manage, state, ..Far::idle(&ctx) };
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
    pub(super) fn manage_rename(&mut self, t: &ito_i18n::Table, link: Option<&str>, old: &str, new: &str) {
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
            Err(NameWhy::Empty) => Some(t.tr("far.add_name_empty").to_owned()),
            Err(NameWhy::Taken) => Some(t.trf("far.add_name_taken", &[&new])),
            Err(NameWhy::Bad) => Some(t.tr("far.add_name_bad").to_owned()),
            Ok(()) if manage::in_use(link, self.on.as_deref(), &old) => Some(t.tr("far.manage_in_use").to_owned()),
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
    pub(super) fn manage_delete(&mut self, t: &ito_i18n::Table, link: Option<&str>, name: &str) {
        let Some(m) = self.manage.as_mut().filter(|m| !m.busy) else { return };
        let name = name.trim().to_owned();
        if manage::in_use(link, self.on.as_deref(), &name) {
            m.line = Some(Err(t.tr("far.manage_in_use").to_owned()));
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
        let on = self.on.clone();
        let used = |n: &str| manage::in_use(link, on.as_deref(), n);
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
    pub(super) fn add_name_go(&mut self, t: &ito_i18n::Table) {
        use crate::remote::NameWhy;
        let Some(a) = self.add.as_mut().filter(|a| a.step == AddStep::Form) else { return };
        let (name, url) = (a.name.trim().to_owned(), a.url.trim().to_owned());
        let bad = match crate::remote::check_name(&name, &self.names) {
            Err(NameWhy::Empty) => Some(t.tr("far.add_name_empty").to_owned()),
            Err(NameWhy::Taken) => Some(t.trf("far.add_name_taken", &[&name])),
            Err(NameWhy::Bad) => Some(t.tr("far.add_name_bad").to_owned()),
            Ok(()) if url.len() <= "https://".len() || !url.starts_with("https://") || url.contains(char::is_whitespace) => {
                Some(t.tr("far.add_url_bad").to_owned())
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
    pub(super) fn add_hand_off(&mut self, t: &ito_i18n::Table) -> Option<(String, String, String, String)> {
        let a = self.add.as_mut().filter(|a| a.step == AddStep::Token)?;
        if a.token.trim().is_empty() {
            a.error = Some(t.tr("far.add_token_empty").to_owned());
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
    pub fn add_box(&mut self, ctx: &egui::Context, t: &ito_i18n::Table) {
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
                                    if a.name.is_empty() && a.url.is_empty() {
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
                                    if step == AddStep::Token && a.token.is_empty() {
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

    /// Asks the shown PC for its nets and runs (and the log looked at) when a round is due.
    fn poll(&mut self) {
        let Some(pc) = self.on.clone() else { return };
        if self.pending > 0 || self.asked.is_some_and(|a| a.elapsed() < EVERY) {
            return;
        }
        self.asked = Some(Instant::now());
        let p = pc.clone();
        self.spawn(move || Got::Nets(p.clone(), crate::remote::ask(&p, |c| c.get("/nets"))));
        let p = pc.clone();
        self.spawn(move || Got::Runs(p.clone(), crate::remote::ask(&p, |c| c.get("/runs"))));
        if let (Some(run), Some(job)) = (self.run_sel.clone(), self.job_sel.clone()) {
            self.ask_log(pc, run, job);
        }
        // The next round comes without anyone moving the mouse.
        self.ctx.request_repaint_after(EVERY);
    }

    fn ask_log(&mut self, pc: String, run: String, job: String) {
        let path = format!("/runs/{}/log?job={}", query(&run), query(&job));
        self.spawn(move || {
            let j = crate::remote::ask(&pc, |c| c.get(&path));
            Got::Log(pc, run, job, j)
        });
    }

    fn act(&mut self, a: Act) {
        let Some(pc) = self.on.clone() else { return };
        self.spawn(move || {
            let said = match &a {
                Act::Run(net, values) => crate::remote::ask(&pc, |c| c.post_json("/runs", &run_body(net, values)))
                    .map(|j| format!("{net}: {}", j["run"].as_str().unwrap_or("?"))),
                Act::Stop(run) => crate::remote::ask(&pc, |c| c.post_json(&format!("/runs/{}/stop", query(run)), &serde_json::json!({})))
                    .map(|j| j["said"].as_str().unwrap_or(run).to_owned()),
                Act::Rerun(run) => crate::remote::ask(&pc, |c| c.post_json(&format!("/runs/{}/rerun", query(run)), &serde_json::json!({})))
                    .map(|j| format!("{run} → {}", j["run"].as_str().unwrap_or("?"))),
                Act::Hold(net, on) => {
                    let path = if *on { "/nets/hold" } else { "/nets/release" };
                    crate::remote::ask(&pc, |c| c.post_json(path, &serde_json::json!({ "net": net }))).map(|j| said(net, &j))
                }
                Act::SkipNext(net, on) => {
                    crate::remote::ask(&pc, |c| c.post_json("/nets/skip-next", &serde_json::json!({ "net": net, "cancel": !on })))
                        .map(|j| said(net, &j))
                }
            };
            Got::Did(pc, said)
        });
    }

    /// The PC list: this PC, then each `[[remote]]`.
    pub fn picker(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        self.drain();
        let here = t.tr("far.here").to_owned();
        let mut pick = self.on.clone();
        ui.horizontal(|ui| {
            ui.label(t.tr("far.pc"));
            let r = egui::ComboBox::from_id_salt("far_pc")
                .selected_text(pick.clone().unwrap_or_else(|| here.clone()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, None, &here);
                    for n in &self.names {
                        ui.selectable_value(&mut pick, Some(n.clone()), n);
                    }
                })
                .response;
            ito_a11y::say(&r, Role::Button, &format!("{} {}", t.tr("far.pc"), pick.as_deref().unwrap_or(&here)));
        });
        self.switch(pick);
    }

    /// The page of the PC shown.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        self.drain();
        self.poll();
        let Some(pc) = self.on.clone() else { return };
        let c = ito_theme::colors();
        ui.horizontal(|ui| {
            ui.heading(t.trf("far.title", &[&pc]));
            let refresh = ito_prefs::IconButton::new(ito_prefs::Mark::Refresh, t.tr("far.refresh"));
            if ui.add_enabled(self.pending == 0, |ui: &mut egui::Ui| refresh.show(ui)).clicked() {
                self.asked = None;
            }
            if self.pending > 0 {
                ui.spinner();
            }
        });
        if self.paused {
            ui.label(egui::RichText::new(format!("{} {}", State::Held.mark(), t.tr("far.paused"))).color(c.wait_text()));
        }
        match &self.said {
            Some(Ok(s)) => {
                ui.label(s);
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).color(graph::colour(&c, State::Error)));
            }
            None => {}
        }
        if let Some(Act::Run(_, values)) = &mut self.asking {
            if !values.is_empty() {
                ui.add_space(4.0);
                egui::Grid::new("far_give").num_columns(2).show(ui, |ui| {
                    for (name, default, value) in values.iter_mut() {
                        ui.label(name.as_str());
                        let r = ui.add(egui::TextEdit::singleline(value).hint_text(default.as_str()).desired_width(300.0));
                        ito_a11y::say(&r, Role::Label, name);
                        ui.end_row();
                    }
                });
            }
        }
        if let Some(a) = self.asking.clone() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let (text, yes) = match &a {
                    Act::Run(net, _) => (t.trf("far.ask_run", &[&pc, net]), t.tr("run.now")),
                    Act::Stop(run) => (t.trf("far.ask_stop", &[&pc, run]), t.tr("run.kill_yes")),
                    Act::Rerun(run) => (t.trf("far.ask_rerun", &[&pc, run]), t.tr("run.rerun")),
                    Act::Hold(net, true) => (t.trf("far.ask_hold", &[&pc, net]), t.tr("run.hold")),
                    Act::Hold(net, false) => (t.trf("far.ask_release", &[&pc, net]), t.tr("run.release")),
                    Act::SkipNext(net, true) => (t.trf("far.ask_skip_next", &[&pc, net]), t.tr("far.skip_next")),
                    Act::SkipNext(net, false) => (t.trf("far.ask_unskip", &[&pc, net]), t.tr("far.unskip")),
                };
                let r = ui.label(egui::RichText::new(&text).strong());
                ito_a11y::say(&r, Role::Label, &text);
                // The answer that does it, in the tone of what it does: a stop cannot be taken back.
                let tone = if matches!(a, Act::Stop(_)) { ito_prefs::Tone::Danger } else { ito_prefs::Tone::Primary };
                let y = ito_prefs::IconButton::new(a.mark(), yes).tone(tone).show(ui);
                let n = ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("run.kill_no")).show(ui);
                if y.clicked() {
                    self.asking = None;
                    self.said = None;
                    self.act(a.clone());
                } else if n.clicked() {
                    self.asking = None;
                }
            });
        }
        egui::ScrollArea::vertical().id_salt("far_page").auto_shrink([false, false]).show(ui, |ui| {
            self.nets_view(ui, t);
            ui.add_space(10.0);
            self.runs_view(ui, t);
            ui.add_space(10.0);
            self.run_view(ui, t);
        });
    }

    fn nets_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        ui.strong(t.tr("far.nets"));
        let nets = match &self.nets {
            None => return loading(ui, t),
            Some(Err(e)) => return error(ui, e),
            Some(Ok(n)) if n.is_empty() => {
                ui.label(egui::RichText::new(t.tr("far.no_nets")).color(c.dim));
                return;
            }
            Some(Ok(n)) => n.clone(),
        };
        let mut press = None;
        egui::Grid::new("far_nets").num_columns(5).spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
            for n in &nets {
                let title = if n.name.is_empty() { &n.id } else { &n.name };
                let r = ui.label(title);
                ito_a11y::say(&r, Role::Row, &format!("{title} {}", n.id));
                ui.label(egui::RichText::new(&n.id).color(c.dim));
                let mut standing = t.tr(&format!("standing.{}", n.standing)).to_owned();
                if n.held {
                    standing = format!("{standing}  {}", t.tr("far.held"));
                }
                ui.label(standing);
                let free = self.asking.is_none();
                let said = format!("{} {title}", t.tr("run.now"));
                let now = ito_prefs::IconButton::new(ito_prefs::Mark::Play, t.tr("run.now")).said(&said);
                if ui.add_enabled(free, |ui: &mut egui::Ui| now.show(ui)).clicked() {
                    let values = n.params.iter().map(|(k, d)| (k.clone(), d.clone(), d.clone())).collect();
                    press = Some(Act::Run(n.id.clone(), values));
                }
                ui.horizontal(|ui| {
                    let hold = if n.held { "run.release" } else { "run.hold" };
                    let acts = [
                        (hold, Act::Hold(n.id.clone(), !n.held)),
                        ("far.skip_next", Act::SkipNext(n.id.clone(), true)),
                        ("far.unskip", Act::SkipNext(n.id.clone(), false)),
                    ];
                    for (key, act) in acts {
                        let said = format!("{} {title}", t.tr(key));
                        let button = ito_prefs::IconButton::new(act.mark(), t.tr(key)).said(&said);
                        if ui.add_enabled(free, |ui: &mut egui::Ui| button.show(ui)).clicked() {
                            press = Some(act);
                        }
                    }
                });
                ui.end_row();
            }
        });
        if press.is_some() {
            self.asking = press;
        }
    }

    fn runs_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        ui.strong(t.tr("far.runs"));
        let mut runs = match &self.runs {
            None => return loading(ui, t),
            Some(Err(e)) => return error(ui, e),
            Some(Ok(r)) if r.is_empty() => {
                ui.label(egui::RichText::new(t.tr("run.none")).color(c.dim));
                return;
            }
            Some(Ok(r)) => r.clone(),
        };
        runs.sort_by_key(|r| std::cmp::Reverse(r.started));
        let (mut pick, mut press) = (None, None);
        egui::Grid::new("far_runs").num_columns(5).spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
            for key in ["col_state", "col_net", "col_start", "col_end"] {
                ui.label(egui::RichText::new(t.tr(&format!("today.{key}"))).small().color(c.dim));
            }
            ui.end_row();
            for run in &runs {
                let colour = graph::colour(&c, run.state);
                ui.label(egui::RichText::new(show::tag(t, run.state)).color(colour));
                let on = self.run_sel.as_deref() == Some(run.id.as_str());
                let r = ui.selectable_label(on, egui::RichText::new(&run.title).color(colour));
                let ended = run.ended.map(show::hms).unwrap_or_default();
                ito_a11y::say(&r, Role::Row, &format!("{} {} {} {ended}", show::tag(t, run.state), run.title, show::hms(run.started)));
                if r.clicked() {
                    pick = Some(run.id.clone());
                }
                ui.label(show::hms(run.started));
                ui.label(ended);
                let (key, act, tone) = if run.is_over() {
                    ("run.rerun", Act::Rerun(run.id.clone()), ito_prefs::Tone::Plain)
                } else {
                    ("run.kill", Act::Stop(run.id.clone()), ito_prefs::Tone::Danger)
                };
                let said = format!("{} {}", t.tr(key), run.title);
                let button = ito_prefs::IconButton::new(act.mark(), t.tr(key)).said(&said).tone(tone);
                if ui.add_enabled(self.asking.is_none(), |ui: &mut egui::Ui| button.show(ui)).clicked() {
                    press = Some(act);
                }
                ui.end_row();
            }
        });
        if pick.is_some() && pick != self.run_sel {
            self.run_sel = pick;
            self.job_sel = None;
            self.log = None;
        }
        if press.is_some() {
            self.asking = press;
        }
    }

    /// The run picked: its jobs; a press on one shows the tail of its log.
    fn run_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(run) = self.run_sel.as_deref().and_then(|id| self.runs.as_ref()?.as_ref().ok()?.iter().find(|r| r.id == id)).cloned()
        else {
            return;
        };
        ui.strong(format!("{}  {}", run.title, run.id));
        let mut pick = None;
        ui.horizontal_wrapped(|ui| {
            for j in &run.jobs {
                let on = self.job_sel.as_deref() == Some(j.id.as_str());
                let text = format!("{} {}", j.state.mark(), j.id);
                let r = ui.selectable_label(on, egui::RichText::new(&text).color(graph::colour(&c, j.state)));
                ito_a11y::say(&r, Role::Button, &format!("{} {}", j.id, show::word(t, j.state)));
                if r.clicked() {
                    pick = Some(j.id.clone());
                }
            }
        });
        if let Some(job) = pick.filter(|p| self.job_sel.as_ref() != Some(p)) {
            self.job_sel = Some(job.clone());
            self.log = None;
            if let Some(pc) = self.on.clone() {
                self.ask_log(pc, run.id.clone(), job);
            }
        }
        if self.job_sel.is_none() {
            ui.label(egui::RichText::new(t.tr("far.pick_job")).color(c.dim));
            return;
        }
        match &self.log {
            None => loading(ui, t),
            Some(Err(e)) => error(ui, e),
            Some(Ok(text)) => {
                egui::ScrollArea::vertical().id_salt("far_log").max_height(320.0).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace()).wrap());
                });
            }
        }
    }
}

fn loading(ui: &mut egui::Ui, t: &ito_i18n::Table) {
    ui.label(egui::RichText::new(t.tr("far.loading")).color(ito_theme::colors().dim));
}

fn error(ui: &mut egui::Ui, e: &str) {
    let c = ito_theme::colors();
    let r = ui.add(egui::Label::new(egui::RichText::new(e).color(graph::colour(&c, State::Error))).wrap());
    ito_a11y::say(&r, Role::Label, e);
}

/// The line shown after a hold or a skip: what the engine said (it names the net), else the net.
fn said(net: &str, j: &serde_json::Value) -> String {
    j["said"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(net).to_owned()
}

/// What `POST /runs` is sent: the net, and only the values changed from their default (as on this PC).
fn run_body(net: &str, values: &[(String, String, String)]) -> serde_json::Value {
    let params: BTreeMap<&str, &str> = values.iter().filter(|(_, d, v)| v != d).map(|(k, _, v)| (k.as_str(), v.as_str())).collect();
    if params.is_empty() {
        serde_json::json!({ "net": net })
    } else {
        serde_json::json!({ "net": net, "params": params })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn far_on(pc: &str) -> Far {
        let mut f = Far::idle(&egui::Context::default());
        f.names = vec![pc.to_owned(), "other".to_owned()];
        f.switch(Some(pc.to_owned()));
        f
    }

    /// what comes back is kept for the PC shown and dropped for one switched away from.
    #[test]
    fn an_answer_counts_only_for_the_pc_shown() {
        let mut f = far_on("b");
        f.pending = 2;
        let nets = serde_json::json!({ "paused": true, "nets": [{ "id": "g/a", "name": "A", "standing": "confirmed", "held": true }] });
        f.take(Got::Nets("b".into(), Ok(nets.clone())));
        let n = f.nets.as_ref().unwrap().as_ref().unwrap();
        assert_eq!((n[0].id.as_str(), n[0].held, f.paused), ("g/a", true, true));
        f.take(Got::Runs("other".into(), Ok(serde_json::json!({ "runs": [] }))));
        assert!(f.runs.is_none());
        assert_eq!(f.pending, 0);
        f.switch(Some("other".into()));
        assert!(f.nets.is_none() && !f.paused && f.names.len() == 2);
        f.take(Got::Nets("b".into(), Ok(nets)));
        assert!(f.nets.is_none());
        f.take(Got::Did("other".into(), Err("other: 403 edit が要る".into())));
        assert_eq!(f.said, Some(Err("other: 403 edit が要る".into())));
    }

    /// 今すぐ実行 on another PC sends only the values changed, and no `params` when none is.
    #[test]
    fn a_run_is_sent_with_the_changed_values() {
        let values = vec![
            ("dept".to_owned(), "all".to_owned(), "経理".to_owned()),
            ("month".to_owned(), "{month-1:%Y-%m}".to_owned(), "{month-1:%Y-%m}".to_owned()),
        ];
        assert_eq!(run_body("g/a", &values), serde_json::json!({ "net": "g/a", "params": { "dept": "経理" } }));
        assert_eq!(run_body("g/a", &values[1..]), serde_json::json!({ "net": "g/a" }));
        let n: Net = serde_json::from_value(serde_json::json!({ "id": "g/a", "params": { "month": "x" } })).unwrap();
        assert_eq!(n.params.get("month").map(String::as_str), Some("x"));
    }

    /// a hold's or a skip's answer shows as the engine's words, or the net when it said nothing.
    #[test]
    fn a_hold_and_a_skip_say_what_the_engine_said() {
        assert_eq!(
            said(
                "g/a",
                &serde_json::json!({ "said": "保留を解いた: g/a
" })
            ),
            "保留を解いた: g/a"
        );
        assert_eq!(said("g/a", &serde_json::json!({ "said": "" })), "g/a");
    }

    #[test]
    fn a_job_is_sent_as_a_query_value() {
        assert_eq!(query("20261030-020000-01"), "20261030-020000-01");
        assert_eq!(query("a b&c=日"), "a%20b%26c%3D%E6%97%A5");
    }

    /// the page draws with nothing come back yet, with the list, and with a refusal.
    #[test]
    fn the_page_draws_in_each_state() {
        let (t, _) = crate::i18n::table("ja");
        let mut f = far_on("b");
        f.asked = Some(Instant::now());
        let ctx = egui::Context::default();
        let frame = |f: &mut Far| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                f.picker(ui, &t);
                f.show(ui, &t);
            });
            out.textures_delta.clear();
        };
        frame(&mut f);
        f.take(Got::Nets("b".into(), Ok(serde_json::json!({ "nets": [{ "id": "g/a", "standing": "unconfirmed" }] }))));
        f.take(Got::Runs("b".into(), Err("b: つながらない".into())));
        f.asking = Some(Act::Run("g/a".into(), vec![("month".into(), "{month-1:%Y-%m}".into(), "2026-09".into())]));
        frame(&mut f);
        for a in [
            Act::Hold("g/a".into(), true),
            Act::Hold("g/a".into(), false),
            Act::SkipNext("g/a".into(), true),
            Act::SkipNext("g/a".into(), false),
        ] {
            f.asking = Some(a);
            frame(&mut f);
        }
        assert_eq!(f.on.as_deref(), Some("b"));
    }

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
            assert_eq!(f.pending, 0, "no thread for {name:?} {url:?}");
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
        assert_eq!(crate::remote::write_failed("pc", "読み取り専用", Ok(())), "pc: 読み取り専用");
        let w = crate::remote::write_failed("pc", "読み取り専用", Err("アクセス拒否".into()));
        assert!(w.starts_with("pc: 読み取り専用") && w.contains("資格情報ストア") && w.contains("アクセス拒否"), "{w}");
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

    #[test]
    fn the_add_box_survives_a_switch_of_the_page() {
        let mut f = adding();
        f.switch(Some("old".into()));
        assert!(f.adding());
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
        f.switch(Some("kei".into()));
        assert!(!f.shown(), "the page does not switch while busy");
        f.manage.as_mut().unwrap().busy = false;
        f.switch(Some("kei".into()));
        assert!(f.shown() && f.managing(), "the box is kept through a switch");
    }

    #[test]
    fn a_rename_to_the_same_name_writes_nothing() {
        let t = ja();
        let mut f = managing();
        f.manage.as_mut().unwrap().edit("kei");
        f.manage_rename(&t, None, "kei", "  kei ");
        let m = f.manage.as_ref().unwrap();
        assert!(m.editing.is_none() && !m.busy && m.line.is_none());
        assert_eq!(f.pending, 0);
    }

    #[test]
    fn a_refused_name_is_said_on_the_status_line_and_the_editing_stays() {
        let t = ja();
        for (new, why) in
            [("", "far.add_name_empty"), ("   ", "far.add_name_empty"), ("works", "far.add_name_taken"), ("a b", "far.add_name_bad")]
        {
            let mut f = managing();
            f.manage.as_mut().unwrap().edit("kei");
            f.manage_rename(&t, None, "kei", new);
            let m = f.manage.as_ref().unwrap();
            assert!(m.editing.is_some() && !m.busy, "{new:?}");
            let want = if why == "far.add_name_taken" { t.trf(why, &["works"]) } else { t.tr(why).to_owned() };
            assert_eq!(m.line, Some(Err(want)), "{new:?}");
            assert_eq!(f.pending, 0, "no thread for {new:?}");
        }
    }

    #[test]
    fn a_connection_in_use_is_neither_renamed_nor_deleted() {
        let t = ja();
        // The window's link, even when it does not answer.
        let mut f = managing();
        f.manage_rename(&t, Some("kei"), "kei", "keiba");
        assert_eq!(mline(&f), Some(Err(t.tr("far.manage_in_use").to_owned())));
        f.manage_delete(&t, Some("kei"), "kei");
        assert_eq!(mline(&f), Some(Err(t.tr("far.manage_in_use").to_owned())));
        assert!(!f.manage_busy());
        assert_eq!(f.pending, 0);
        // The PC its page shows.
        let mut f = managing();
        f.on = Some("works".into());
        f.manage_delete(&t, None, "works");
        assert_eq!(mline(&f), Some(Err(t.tr("far.manage_in_use").to_owned())));
        assert_eq!(f.pending, 0);
    }

    #[test]
    fn an_answer_changes_the_names_and_the_box_that_asked() {
        let mut f = managing();
        f.manage.as_mut().unwrap().edit("kei");
        f.manage.as_mut().unwrap().busy = true;
        f.pending = 1;
        let t = mticket(&f);
        f.take(Got::Managed(t, Change::Rename("kei".into(), "keiba".into()), Ok("kei の名前を keiba に変えた".into())));
        assert_eq!(f.names, ["keiba", "works"], "renamed in place");
        let m = f.manage.as_ref().unwrap();
        assert!(!m.busy && m.editing.is_none());
        assert_eq!(m.line, Some(Ok("kei の名前を keiba に変えた".into())));
        assert_eq!(f.pending, 0);
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
