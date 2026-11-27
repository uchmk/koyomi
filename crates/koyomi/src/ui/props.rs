//! The job property window: one window a job of a run (概要 / 定義 / ログ / 履歴), opened by a double click, Alt+Enter or
//! the job's menu. The windows are `ito_viewport` windows; their data is kept here under one lock and refreshed in the
//! root pass of the app ([`Browser::props_pass`]), so a window never asks the engine itself.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use egui::ViewportId;
use ito_a11y::Role;
use ito_i18n::Table;
use ito_viewport::{Closed, Opened, Spec, Windows, Words};
use koyomi_core::run::{Run, State};

use super::runfacts::{self, View};
use super::{jobhist, logview, Browser, RunTab};
use crate::show;

/// Most windows open at once; the next one closes the oldest.
pub const CAP: usize = 8;

/// A window is one job of one run.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PropsKey {
    pub run: String,
    pub job: String,
}

impl PropsKey {
    pub fn new(run: &str, job: &str) -> PropsKey {
        PropsKey { run: run.to_owned(), job: job.to_owned() }
    }
}

/// What a window asks of the app (done in the root pass, after the window is drawn).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Act {
    /// 「ページで開く」: the run page at this job (the ログ tab when `log`).
    Page { key: PropsKey, log: bool },
    /// 「エディターで開く」.
    Editor(PropsKey),
    /// 「履歴をページで見る」: the run page at this job's 履歴 tab.
    More(PropsKey),
    /// A row of the 履歴 tab: that run's same job in another window.
    OpenRun { from: PropsKey, run: String, log: bool },
}

impl Act {
    fn key(&self) -> &PropsKey {
        match self {
            Act::Page { key, .. } | Act::Editor(key) | Act::More(key) => key,
            Act::OpenRun { from, .. } => from,
        }
    }
}

/// What the root pass found for a window this time.
struct Snap {
    run: Run,
    def: View,
    hist: Vec<jobhist::Row>,
    range: String,
    local: bool,
    net_there: bool,
    can_page: bool,
}

/// Everything one window shows.
struct PropsData {
    run: Run,
    tab: RunTab,
    def: View,
    hist: Vec<jobhist::Row>,
    range: String,
    log: String,
    gen: u64,
    log_view: logview::LogView,
    sort: Option<(usize, bool)>,
    local: bool,
    net_there: bool,
    can_page: bool,
    /// The run is not in any list now: the last copy stays.
    gone: bool,
    /// The window was opened as the ninth: the oldest was closed for it.
    cap: bool,
    result: Option<Result<String, String>>,
    /// What the job was like when its log was last read once it was over (see [`settled_key`]); the log is read
    /// again when that changes. Cleared while the job runs.
    settled: Option<Settled>,
    /// The job of the window.
    job: String,
    /// The title last given to the window.
    title: String,
    /// Changes whenever what is drawn changes (the repaint's reason).
    stamp: u64,
}

impl PropsData {
    fn new(run: Run, job: &str, tab: RunTab, cap: bool) -> PropsData {
        PropsData {
            run,
            job: job.to_owned(),
            title: String::new(),
            tab,
            def: View::Loading,
            hist: Vec::new(),
            range: String::new(),
            log: String::new(),
            gen: 0,
            log_view: logview::LogView::new("props.log"),
            sort: None,
            local: true,
            net_there: false,
            can_page: false,
            gone: false,
            cap,
            result: None,
            settled: None,
            stamp: 0,
        }
    }

    /// Takes what the pass found; true when something changed.
    fn apply(&mut self, s: Snap) -> bool {
        let changed = self.gone
            || self.run != s.run
            || self.def != s.def
            || self.hist != s.hist
            || self.range != s.range
            || self.local != s.local
            || self.net_there != s.net_there
            || self.can_page != s.can_page;
        if changed {
            self.gone = false;
            self.run = s.run;
            if self.run.job(&self.job).is_some_and(|j| j.state == State::Running) {
                // It runs again: its end is read afresh.
                self.settled = None;
            }
            self.def = s.def;
            self.hist = s.hist;
            self.range = s.range;
            self.local = s.local;
            self.net_there = s.net_there;
            self.can_page = s.can_page;
            self.stamp += 1;
        }
        changed
    }

    /// Takes a log text; true when the text changed (`gen` moves only then).
    fn take_log(&mut self, text: &str) -> bool {
        if let Some(j) = self.run.job(&self.job).filter(|j| j.state != State::Running) {
            self.settled = Some(settled_key(&self.run, j));
        }
        if self.log == text {
            return false;
        }
        self.log = text.to_owned();
        self.gen += 1;
        self.stamp += 1;
        true
    }
}

struct Shared {
    data: BTreeMap<PropsKey, PropsData>,
    table: Arc<Table>,
    outbox: Vec<Act>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The property windows of one browser (one link).
pub struct Props {
    windows: Windows<PropsKey>,
    /// The number each open window carries in its name (`props.N`): the lowest free one, kept while it is open.
    slots: Vec<(PropsKey, usize)>,
    shared: Arc<Mutex<Shared>>,
    /// Windows asked for in this pass: the key and the tab; opened at the end of the root pass.
    pending: Vec<(PropsKey, RunTab)>,
    /// The language the words and the table are in.
    lang: String,
}

impl Props {
    pub fn new(state: Option<PathBuf>) -> Props {
        let shared =
            Arc::new(Mutex::new(Shared { data: BTreeMap::new(), table: Arc::new(crate::i18n::table("en").0), outbox: Vec::new() }));
        let body = shared.clone();
        let windows = Windows::new("props", state, move |ui, key| body_of(&body, ui, key));
        Props { windows, slots: Vec::new(), shared, pending: Vec::new(), lang: String::new() }
    }

    /// The open windows, oldest first.
    #[cfg(test)]
    pub fn keys(&self) -> Vec<PropsKey> {
        self.windows.keys()
    }

    /// The tab of an open window.
    #[cfg(test)]
    pub fn tab_of(&self, key: &PropsKey) -> Option<RunTab> {
        lock(&self.shared).data.get(key).map(|d| d.tab)
    }

    /// The log text and its generation of an open window.
    #[cfg(test)]
    pub fn log_of(&self, key: &PropsKey) -> Option<(String, u64)> {
        lock(&self.shared).data.get(key).map(|d| (d.log.clone(), d.gen))
    }

    /// What an open window says in its result line.
    #[cfg(test)]
    pub fn result_of(&self, key: &PropsKey) -> Option<Result<String, String>> {
        lock(&self.shared).data.get(key).and_then(|d| d.result.clone())
    }

    /// Whether an open window still carries the 8-window note.
    #[cfg(test)]
    pub fn cap_of(&self, key: &PropsKey) -> Option<bool> {
        lock(&self.shared).data.get(key).map(|d| d.cap)
    }

    /// How many rows of history an open window holds.
    #[cfg(test)]
    pub fn hist_len_of(&self, key: &PropsKey) -> Option<usize> {
        lock(&self.shared).data.get(key).map(|d| d.hist.len())
    }

    /// The definition an open window shows.
    #[cfg(test)]
    pub fn def_of(&self, key: &PropsKey) -> Option<View> {
        lock(&self.shared).data.get(key).map(|d| d.def.clone())
    }

    /// The title last given to an open window.
    #[cfg(test)]
    pub fn title_of_window(&self, key: &PropsKey) -> Option<String> {
        lock(&self.shared).data.get(key).map(|d| d.title.clone())
    }

    /// Closes every window and forgets all of it (the link changed).
    pub fn close_all(&mut self) {
        self.windows.close_all();
        let mut g = lock(&self.shared);
        g.data.clear();
        g.outbox.clear();
        self.slots.clear();
        self.pending.clear();
    }

    /// The logs the open windows want read: on the ログ tab, the run found, and the job going or not yet read since
    /// it ended. Oldest window first.
    fn log_list(&self) -> Vec<(String, String)> {
        let g = lock(&self.shared);
        self.windows
            .keys()
            .into_iter()
            .filter_map(|k| {
                let d = g.data.get(&k)?;
                let job = d.run.job(&k.job)?;
                wants_log(d.tab, d.gone, &d.run, job, d.settled).then(|| (k.run.clone(), k.job.clone()))
            })
            .collect()
    }

    /// Drops what belongs to windows that closed and are not open again; true for each one dropped.
    fn forget(&mut self, closed: &[Closed<PropsKey>]) -> usize {
        let mut dropped = 0;
        for c in closed {
            if self.windows.is_open(&c.key) {
                // Opened again before the report came: the data is the new window's.
                continue;
            }
            lock(&self.shared).data.remove(&c.key);
            self.slots.retain(|(k, _)| k != &c.key);
            dropped += 1;
        }
        dropped
    }

    /// What the windows asked for, without what the closed windows asked.
    fn take_acts(&mut self) -> Vec<Act> {
        let acts = std::mem::take(&mut lock(&self.shared).outbox);
        acts.into_iter().filter(|a| self.windows.is_open(a.key())).collect()
    }

    /// Closes the oldest window when `CAP` are open (its data and slot go at once). True when one was closed.
    fn make_room(&mut self) -> bool {
        let keys = self.windows.keys();
        if keys.len() < CAP {
            return false;
        }
        let oldest = keys[0].clone();
        self.windows.close(&oldest);
        lock(&self.shared).data.remove(&oldest);
        self.slots.retain(|(k, _)| k != &oldest);
        true
    }

    fn slot_for(&mut self, key: &PropsKey) -> Option<usize> {
        let used: Vec<usize> = self.slots.iter().map(|(_, n)| *n).collect();
        let n = lowest_free(&used)?;
        self.slots.push((key.clone(), n));
        Some(n)
    }

    /// The words of the caption buttons and the titles, in the language of `t`; once per language.
    fn sync_lang(&mut self, ctx: &egui::Context, t: &Table) {
        let lang = t.tr("app.lang");
        if self.lang == lang {
            return;
        }
        self.lang = lang.to_owned();
        lock(&self.shared).table = Arc::new(crate::i18n::table(lang).0);
        self.windows.set_words(Words {
            minimize: t.tr("title.minimize").to_owned(),
            maximize: t.tr("title.maximize").to_owned(),
            restore: t.tr("title.restore").to_owned(),
            close: t.tr("title.close").to_owned(),
        });
        // Every window draws its words again (the titles follow in the next snapshot step).
        for d in lock(&self.shared).data.values_mut() {
            d.stamp += 1;
        }
        self.windows.repaint_all(ctx);
    }
}

/// The lowest of 1..=CAP that is not in `used`.
pub fn lowest_free(used: &[usize]) -> Option<usize> {
    (1..=CAP).find(|n| !used.contains(n))
}

/// The job's state, the run's state and the job's `started`, `ended` and `retried`: a log read once all of these were
/// as they are now is the final one.
type Settled = (State, State, Option<chrono::NaiveDateTime>, Option<chrono::NaiveDateTime>, u32);

fn settled_key(run: &Run, job: &koyomi_core::run::JobRun) -> Settled {
    (job.state, run.state, job.started, job.ended, job.retried)
}

/// Whether the window's log is read this round. A going job is read every round; a finished one once, and again
/// when anything in its `Settled` key changes (a job that ran again).
fn wants_log(tab: RunTab, gone: bool, run: &Run, job: &koyomi_core::run::JobRun, settled: Option<Settled>) -> bool {
    tab == RunTab::Log && !gone && (job.state == State::Running || settled != Some(settled_key(run, job)))
}

/// The tab after (or before) `tab` in the order 概要, 定義, ログ, 履歴; it wraps.
pub fn cycle(tab: RunTab, forward: bool) -> RunTab {
    const ORDER: [RunTab; 4] = [RunTab::Facts, RunTab::Def, RunTab::Log, RunTab::Hist];
    let at = ORDER.iter().position(|t| *t == tab).unwrap_or(0);
    ORDER[if forward { (at + 1) % 4 } else { (at + 3) % 4 }]
}

/// Ctrl+Tab (`Some(true)`) and Ctrl+Shift+Tab (`Some(false)`), taken from the input. The Shift one is asked first:
/// egui matches a key logically, so `CTRL` alone would take Ctrl+Shift+Tab too. Not while a popup is open. A text
/// field that has the focus does not stop it (as in a browser).
fn tab_keys(i: &mut egui::InputState, popup: bool) -> Option<bool> {
    if popup {
        return None;
    }
    if i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Tab) {
        return Some(false);
    }
    i.consume_key(egui::Modifiers::CTRL, egui::Key::Tab).then_some(true)
}

fn title_of(t: &Table, run: &Run, key: &PropsKey) -> String {
    let label = run.job(&key.job).map_or(key.job.as_str(), |j| j.label());
    t.trf("run.props_title", &[label, &run.jobnet, &run.id])
}

fn spec_of(slot: usize, title: String) -> Spec {
    Spec {
        kind: "job".to_owned(),
        name: Some(format!("props.{slot}")),
        title,
        size: egui::vec2(720.0, 560.0),
        min_size: egui::vec2(420.0, 300.0),
        esc_closes: true,
    }
}

impl Browser {
    /// Asks for the window of one job of a run, at `tab` (opened at the end of this pass). An open one is brought
    /// forward and switched to the tab.
    pub(super) fn open_props(&mut self, run: &str, job: &str, tab: RunTab) {
        self.props.pending.push((PropsKey::new(run, job), tab));
        self.io.1.request_repaint();
    }

    /// Closes every window and forgets their data (the link changes).
    pub fn props_close_all(&mut self) {
        self.props.close_all();
        self.watch.lock().unwrap_or_else(|e| e.into_inner()).windows.clear();
    }

    /// A log text the thread read: given to the window of that job when it is open and wants it.
    pub(super) fn props_log(&mut self, run: &str, job: &str, text: &str) {
        let key = PropsKey::new(run, job);
        let listed = {
            let w = self.watch.lock().unwrap_or_else(|e| e.into_inner());
            w.windows.iter().any(|(r, j)| r == run && j == job)
        };
        if !listed {
            return;
        }
        let changed = {
            let mut g = lock(&self.props.shared);
            let Some(d) = g.data.get_mut(&key) else { return };
            if d.gone {
                return;
            }
            if d.run.job(job).is_none() {
                return;
            }
            d.take_log(text)
        };
        if changed {
            self.props.windows.repaint(&self.io.1, &key);
        }
    }

    /// What the window of `key` shows now; `None` when the run is in no list (it is gone).
    fn props_snap(&mut self, key: &PropsKey, t: &Table) -> Option<Snap> {
        let run = self.find_run(&key.run)?.clone();
        self.ask_definition(&run);
        self.ask_commands(&run);
        let def = runfacts::view(t, &run, &key.job, self.below_edit(), self.run_defs.get(&run.id), self.run_cmds.get(&run.id));
        let today = self.engine_today();
        let past = self.job_past.as_ref().filter(|p| p.is_for(self.link.far(), &run.jobnet, today));
        let hist = jobhist::job_history(
            std::iter::once(&run)
                .chain(self.runs.iter())
                .chain(self.hist.iter().flat_map(|h| h.rows.iter()))
                .chain(self.past_rows.iter().flatten())
                .chain(past.iter().flat_map(|p| p.runs.iter())),
            &run.jobnet,
            &key.job,
        );
        let read = past.map_or(if self.past_rows.is_some() { jobhist::READ_DAYS } else { 0 }, |p| p.read_to_days);
        let from = hist.iter().map(|r| r.date.clone()).chain(std::iter::once((today - chrono::Duration::days(read)).to_string())).min();
        let range =
            t.trf("run.jobhist_range", &[from.as_deref().unwrap_or_default(), &today.to_string(), &hist.len().to_string(), &run.jobnet]);
        let net_there = self.snap.as_ref().is_some_and(|s| s.defs.iter().any(|d| d.id == run.jobnet && d.net.is_some()));
        Some(Snap { run, def, hist, range, local: !self.link.is_far(), net_there, can_page: !self.props_blocked() })
    }

    /// Whether anything covers the page: a box, the card, the palette, or a screen or dialog of the app (settings,
    /// help, the bell's list, the guide). The windows' buttons that act on the page and Alt+Enter wait then.
    fn props_blocked(&self) -> bool {
        self.box_open() || self.covered() || self.page_covered
    }

    /// Alt+Enter on the run page (or the pane of 履歴) with a job picked: its window at 概要. Not while a text has the
    /// keyboard, a popup is open or a box is up.
    fn props_keys(&mut self, ctx: &egui::Context) {
        if self.editor.is_some() || self.gantt || self.props_blocked() {
            return;
        }
        let Some(run) = self.selected_run() else { return };
        let Some(job) = self.job_sel.clone().filter(|j| run.job(j).is_some()) else { return };
        if ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::Enter)) {
            let id = run.id.clone();
            self.open_props(&id, &job, RunTab::Facts);
        }
    }

    /// The last of the root pass: refreshes the windows' data, shows them, and does what they and the page asked.
    pub fn props_pass(&mut self, ctx: &egui::Context, t: &Table) {
        self.props.sync_lang(ctx, t);
        self.props_keys(ctx);
        // 1. What the windows show, found with no lock held; swapped in under the lock.
        let keys = self.props.windows.keys();
        let snaps: Vec<(PropsKey, Option<Snap>)> = keys.into_iter().map(|k| (k.clone(), self.props_snap(&k, t))).collect();
        let below = self.below_edit();
        let mut changed = Vec::new();
        let mut titles = Vec::new();
        {
            let mut g = lock(&self.props.shared);
            for (k, snap) in snaps {
                let Some(d) = g.data.get_mut(&k) else { continue };
                let moved = match snap {
                    Some(s) => d.apply(s),
                    None => {
                        let mut moved = !d.gone;
                        d.gone = true;
                        // The copies kept with the run are not shown to a caller who may not see them now.
                        let def = runfacts::view(t, &d.run, &k.job, true, None, None);
                        if below && d.def != def {
                            d.def = def;
                            moved = true;
                        }
                        if moved {
                            d.stamp += 1;
                        }
                        moved
                    }
                };
                let title = title_of(t, &d.run, &k);
                if d.title != title {
                    d.title.clone_from(&title);
                    titles.push((k.clone(), title));
                }
                // A save that finished while the tab was not drawn.
                if let Some(r) = d.log_view.poll_save(ctx) {
                    d.result = Some(saved_line(t, r));
                    d.stamp += 1;
                    changed.push(k.clone());
                } else if moved {
                    changed.push(k);
                }
            }
        }
        for (k, title) in titles {
            self.props.windows.set_title(&k, title);
        }
        for k in changed {
            self.props.windows.repaint(ctx, &k);
        }
        // 2. The windows.
        let closed = self.props.windows.show(ctx);
        // 3. What closed (and was not opened again) goes.
        self.props.forget(&closed);
        // 4. What the windows asked, from the windows still open.
        for act in self.props.take_acts() {
            self.props_act(ctx, act);
        }
        // 5. The windows asked for in this pass.
        for (key, tab) in std::mem::take(&mut self.props.pending) {
            self.props_begin(ctx, t, key, tab);
        }
        // The logs the windows want read.
        let list = self.props.log_list();
        let mut w = self.watch.lock().unwrap_or_else(|e| e.into_inner());
        if w.windows != list {
            w.windows = list;
        }
    }

    pub(super) fn props_act(&mut self, ctx: &egui::Context, act: Act) {
        let front = |ctx: &egui::Context| {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Focus);
        };
        // A click can come a pass late: what covers the page now decides, not what covered it when the button drew.
        let blocked = self.props_blocked();
        match act {
            Act::Page { key, log } => {
                if !blocked && self.find_run(&key.run).is_some() {
                    self.open_job_run(&key.run, &key.job, log);
                    front(ctx);
                }
            }
            Act::More(key) => {
                if !blocked && self.find_run(&key.run).is_some() {
                    self.open_job_run(&key.run, &key.job, false);
                    self.set_run_tab(RunTab::Hist);
                    front(ctx);
                }
            }
            Act::Editor(key) => {
                let Some(net) = self.find_run(&key.run).map(|r| r.jobnet.clone()).filter(|_| !blocked) else { return };
                let here = self.editor.as_ref().is_some_and(|e| e.id == net);
                if !here {
                    if self.editor.iter().chain(&self.editors).any(|e| e.dirty) {
                        // Nothing is opened over unsaved edits: the window says why.
                        let words = lock(&self.props.shared).table.tr("run.props_editor_busy").to_owned();
                        if let Some(d) = lock(&self.props.shared).data.get_mut(&key) {
                            d.result = Some(Err(words));
                            d.stamp += 1;
                        }
                        self.props.windows.repaint(ctx, &key);
                        return;
                    }
                    self.open_editor(&net);
                }
                if let Some(e) = self.editor.as_mut().filter(|e| e.id == net) {
                    e.open_part(&key.job);
                }
                self.focus = 0;
                front(ctx);
            }
            Act::OpenRun { from, run, log } => self.open_props(&run, &from.job, if log { RunTab::Log } else { RunTab::Facts }),
        }
    }

    /// Opens (or brings forward) the window of `key` at `tab`.
    fn props_begin(&mut self, ctx: &egui::Context, t: &Table, key: PropsKey, tab: RunTab) {
        if self.props.windows.is_open(&key) {
            let title = lock(&self.props.shared).data.get(&key).map(|d| title_of(t, &d.run, &key));
            if let Some(d) = lock(&self.props.shared).data.get_mut(&key) {
                d.tab = tab;
                d.cap = false;
                d.stamp += 1;
            }
            let slot = self.props.slots.iter().find(|(k, _)| k == &key).map_or(1, |(_, n)| *n);
            let spec = spec_of(slot, title.unwrap_or_default());
            if self.props.windows.open(key.clone(), spec, None) == Opened::Focused {
                self.props.windows.repaint(ctx, &key);
            }
            return;
        }
        // Not for a run no list holds.
        let Some(run) = self.find_run(&key.run).cloned() else { return };
        let cap = self.props.make_room();
        let Some(slot) = self.props.slot_for(&key) else { return };
        let mut data = PropsData::new(run.clone(), &key.job, tab, cap);
        if let Some(s) = self.props_snap(&key, t) {
            data.apply(s);
        }
        let title = title_of(t, &data.run, &key);
        data.title.clone_from(&title);
        lock(&self.props.shared).data.insert(key.clone(), data);
        self.props.windows.open(key, spec_of(slot, title), None);
        ctx.request_repaint();
    }
}

fn saved_line(t: &Table, r: Result<PathBuf, String>) -> Result<String, String> {
    match r {
        Ok(p) => Ok(t.trf("run.log_saved", &[&p.display().to_string()])),
        Err(e) => Err(t.trf("run.log_save_failed", &[&e])),
    }
}

/// A window's content, drawn on its own pass (or inline in the main window's, in the tests). The lock is held for
/// the drawing only; what the window asks goes to the outbox and the root pass is woken.
fn body_of(shared: &Mutex<Shared>, ui: &mut egui::Ui, key: &PropsKey) {
    let asked = {
        let mut g = lock(shared);
        let Shared { data, table, outbox } = &mut *g;
        let Some(d) = data.get_mut(key) else { return };
        let t = table.clone();
        let (before, tab) = (outbox.len(), d.tab);
        ui.push_id(key, |ui| window_ui(ui, &t, key, d, outbox));
        // A tab change moves what the thread reads: the root pass sets the list, so it is woken for it.
        if d.tab != tab {
            d.cap = false;
            d.stamp += 1;
        }
        outbox.len() != before || d.tab != tab
    };
    if asked {
        ui.ctx().request_repaint_of(ViewportId::ROOT);
    }
}

fn window_ui(ui: &mut egui::Ui, t: &Table, key: &PropsKey, d: &mut PropsData, out: &mut Vec<Act>) {
    // Ctrl+Tab / Ctrl+Shift+Tab: only in the window's own pass (the main window's Ctrl+Tab is its own).
    if ui.ctx().viewport_id() != ViewportId::ROOT {
        let popup = egui::Popup::is_any_open(ui.ctx());
        if let Some(forward) = ui.input_mut(|i| tab_keys(i, popup)) {
            d.tab = cycle(d.tab, forward);
        }
    }
    egui::Frame::NONE.inner_margin(10.0).show(ui, |ui| {
        head(ui, t, key, d, out);
        ui.add_space(4.0);
        tabs(ui, t, d);
        ui.add_space(4.0);
        match d.tab {
            RunTab::Facts => {
                egui::ScrollArea::vertical()
                    .id_salt("props_facts")
                    .auto_shrink(false)
                    .show(ui, |ui| Browser::facts_body(ui, t, &d.run, &key.job));
            }
            RunTab::Def => {
                egui::ScrollArea::vertical().id_salt("props_def").auto_shrink(false).show(ui, |ui| def_tab(ui, t, key, d, out));
            }
            RunTab::Log => log_tab(ui, t, key, d),
            RunTab::Hist => {
                egui::ScrollArea::vertical().id_salt("props_hist").auto_shrink(false).show(ui, |ui| hist_tab(ui, t, key, d, out));
            }
        }
    });
}

fn head(ui: &mut egui::Ui, t: &Table, key: &PropsKey, d: &PropsData, out: &mut Vec<Act>) {
    let c = ito_theme::colors();
    let label = d.run.job(&key.job).map_or(key.job.as_str(), |j| j.label());
    let line = match d.run.job(&key.job) {
        Some(j) => format!("{}  {label}", show::tag(t, j.state)),
        None => label.to_owned(),
    };
    let r = ui.label(egui::RichText::new(&line).strong().size(16.0));
    ito_a11y::say(&r, Role::Heading, &line);
    ito_a11y::test_id_in(ui, &r, "props.head", None);
    let small = format!("{} {}  {}", d.run.jobnet, d.run.id, show::tag(t, d.run.state));
    let r = ui.label(egui::RichText::new(&small).small().color(c.dim));
    ito_a11y::say(&r, Role::Label, &small);
    ui.horizontal_wrapped(|ui| {
        let words = t.tr("run.props_open_page");
        let b = ui.add_enabled(d.can_page && !d.gone, egui::Button::new(words));
        ito_a11y::say(&b, Role::Button, words);
        ito_a11y::test_id_in(ui, &b, "props.open_page", None);
        let b = if d.can_page && !d.gone { b } else { b.on_disabled_hover_text(t.tr("run.props_open_page_cannot")) };
        if b.clicked() {
            out.push(Act::Page { key: key.clone(), log: d.tab == RunTab::Log });
        }
        if d.net_there {
            let words = t.tr("run.props_open_editor");
            let b = ui.add_enabled(d.can_page, egui::Button::new(words));
            ito_a11y::say(&b, Role::Button, words);
            ito_a11y::test_id_in(ui, &b, "props.open_editor", None);
            let b = if d.can_page { b } else { b.on_disabled_hover_text(t.tr("run.props_open_editor_cannot")) };
            if b.clicked() {
                out.push(Act::Editor(key.clone()));
            }
        }
    });
    match &d.result {
        Some(Ok(m)) => {
            let line = format!("{} {m}", ito_a11y::State::Done.mark());
            let r = ui.add(egui::Label::new(egui::RichText::new(&line).color(c.done)).wrap());
            ito_a11y::say(&r, Role::Label, &line);
        }
        Some(Err(e)) => {
            let line = format!("{} {e}", ito_a11y::State::Error.mark());
            let r = ui.add(egui::Label::new(egui::RichText::new(&line).color(c.err)).wrap());
            ito_a11y::say(&r, Role::Label, &line);
        }
        None => {}
    }
    for (on, id, words) in [(d.gone, "props.gone", t.tr("run.props_gone")), (d.cap, "props.cap", t.tr("run.props_cap"))] {
        if on {
            let r = ui.add(egui::Label::new(egui::RichText::new(words).color(c.wait_text())).wrap());
            ito_a11y::say(&r, Role::Label, words);
            ito_a11y::test_id_in(ui, &r, id, None);
        }
    }
}

fn tabs(ui: &mut egui::Ui, t: &Table, d: &mut PropsData) {
    ui.horizontal(|ui| {
        for (tab, key, id) in [
            (RunTab::Facts, "run.tab_facts", "props.tab.facts"),
            (RunTab::Def, "run.tab_def", "props.tab.def"),
            (RunTab::Log, "run.tab_log", "props.tab.log"),
            (RunTab::Hist, "run.tab_hist", "props.tab.hist"),
        ] {
            let words = t.tr(key);
            let r = ui.selectable_label(d.tab == tab, words);
            ito_a11y::say(&r, Role::Button, words);
            ito_a11y::test_id_in(ui, &r, id, None);
            if r.clicked() {
                d.tab = tab;
            }
        }
    });
}

fn def_tab(ui: &mut egui::Ui, t: &Table, key: &PropsKey, d: &PropsData, out: &mut Vec<Act>) {
    ui.add_space(4.0);
    let head = t.tr("run.def_head");
    let r = ui.label(egui::RichText::new(head).strong());
    ito_a11y::say(&r, Role::Heading, head);
    ito_a11y::test_id_in(ui, &r, "props.def_head", None);
    if runfacts::definition_states(ui, t, d.def.clone(), d.net_there, d.local) {
        out.push(Act::Editor(key.clone()));
    }
}

fn log_tab(ui: &mut egui::Ui, t: &Table, key: &PropsKey, d: &mut PropsData) {
    let c = ito_theme::colors();
    let label = d.run.job(&key.job).map_or(key.job.as_str(), |j| j.label()).to_owned();
    let title = format!("{}: {label}", t.tr("run.log"));
    let r = ui.label(egui::RichText::new(&title).strong());
    ito_a11y::say(&r, Role::Heading, &title);
    ito_a11y::test_id_in(ui, &r, "props.log_head", None);
    if d.log.is_empty() {
        ui.label(egui::RichText::new(t.tr("run.log_none")).color(c.dim));
        return;
    }
    let words = super::log_words(t);
    let name = format!("{}_{}_{label}", d.run.jobnet, d.run.id);
    let out = d.log_view.show(ui, ito_prefs::Look { c, q: "" }, &words, &d.log, d.gen, &name);
    if let Some(said) = super::out_said(out) {
        let words = |(k, args): super::Said| t.trf(k, &args.iter().map(String::as_str).collect::<Vec<_>>());
        d.result = Some(said.map(words).map_err(words));
    }
}

fn hist_tab(ui: &mut egui::Ui, t: &Table, key: &PropsKey, d: &mut PropsData, out: &mut Vec<Act>) {
    let label = d.run.job(&key.job).map_or(key.job.as_str(), |j| j.label()).to_owned();
    super::jobhist_top(ui, t, &label, &d.range, "props.");
    let mut open = None;
    if d.hist.is_empty() {
        super::jobhist_none(ui, t);
    } else {
        open = Browser::jobhist_table(ui, t, &d.run.id, &d.hist, &mut d.sort, "props.");
        super::jobhist_foot(ui, t, &d.hist);
    }
    ui.add_space(4.0);
    let words = t.tr("run.props_more_on_page");
    let b = ui.button(words);
    ito_a11y::say(&b, Role::Button, words);
    ito_a11y::test_id_in(ui, &b, "props.jobhist.more", None);
    if b.clicked() {
        out.push(Act::More(key.clone()));
    }
    if let Some((run, log)) = open {
        out.push(Act::OpenRun { from: key.clone(), run, log });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(run: &str, job: &str) -> PropsKey {
        PropsKey::new(run, job)
    }

    #[test]
    fn the_lowest_free_slot_is_used_and_nine_have_none() {
        assert_eq!(lowest_free(&[]), Some(1));
        assert_eq!(lowest_free(&[1, 2, 4]), Some(3));
        assert_eq!(lowest_free(&[2, 3]), Some(1));
        assert_eq!(lowest_free(&[1, 2, 3, 4, 5, 6, 7, 8]), None);
    }

    fn wants(tab: RunTab, gone: bool, d: &PropsData) -> bool {
        wants_log(tab, gone, &d.run, d.run.job("a").unwrap(), d.settled)
    }

    #[test]
    fn the_log_is_wanted_while_the_job_goes_and_once_more_after_it_ended() {
        let mut d = PropsData::new(run_of("r1"), "a", RunTab::Log, false);
        let at = ito_common::now::now_local().naive_local();
        d.run.start("a", at);
        assert!(wants(RunTab::Log, false, &d), "a going job is read every round");
        d.take_log("one");
        assert!(d.settled.is_none() && wants(RunTab::Log, false, &d), "still going");
        d.run.finish("a", koyomi_core::run::Outcome::Exited(0), at);
        assert!(wants(RunTab::Log, false, &d), "ended, never read since");
        d.take_log(
            "one
two",
        );
        assert!(d.settled.is_some() && !wants(RunTab::Log, false, &d), "read once after it ended");
        assert!(!wants(RunTab::Facts, false, &d) && !wants(RunTab::Log, true, &d), "not on the log tab / a gone run");
        d.run.state = State::Error;
        assert!(wants(RunTab::Log, false, &d), "the run's state changed");
    }

    #[test]
    fn a_job_that_runs_again_is_read_again_at_its_end() {
        let mut d = PropsData::new(run_of("r1"), "a", RunTab::Log, false);
        let t0 = ito_common::now::now_local().naive_local();
        d.run.start("a", t0);
        d.run.finish("a", koyomi_core::run::Outcome::Exited(0), t0);
        d.take_log("first");
        assert!(!wants(RunTab::Log, false, &d));
        // It runs again (a re-run), ends in the same states, with a new `ended`.
        let t1 = t0 + chrono::Duration::seconds(30);
        d.run.start("a", t1);
        assert!(wants(RunTab::Log, false, &d));
        d.run.finish("a", koyomi_core::run::Outcome::Exited(0), t1);
        assert!(wants(RunTab::Log, false, &d), "a new end is read once more");
        d.take_log("second");
        assert!(!wants(RunTab::Log, false, &d), "and not again after the read");
    }

    fn snap_of(run: &Run) -> Snap {
        Snap { run: run.clone(), def: View::Loading, hist: Vec::new(), range: String::new(), local: true, net_there: false, can_page: true }
    }

    #[test]
    fn a_snapshot_of_a_running_job_clears_settled_and_its_end_is_read_once() {
        let mut d = PropsData::new(run_of("r1"), "a", RunTab::Log, false);
        let t0 = ito_common::now::now_local().naive_local();
        let mut run = run_of("r1");
        run.start("a", t0);
        run.finish("a", koyomi_core::run::Outcome::Exited(0), t0);
        assert!(d.apply(snap_of(&run)));
        d.take_log("first");
        assert!(d.settled.is_some() && !wants(RunTab::Log, false, &d), "read after it ended");
        // The job runs again: the snapshot says Running.
        let t1 = t0 + chrono::Duration::seconds(30);
        run.start("a", t1);
        assert!(d.apply(snap_of(&run)));
        assert!(d.settled.is_none(), "a running snapshot clears settled");
        assert!(wants(RunTab::Log, false, &d));
        // It ends: read once, then not again.
        run.finish("a", koyomi_core::run::Outcome::Exited(0), t1);
        assert!(d.apply(snap_of(&run)));
        assert!(wants(RunTab::Log, false, &d), "its end is not read yet");
        d.take_log("second");
        assert!(!wants(RunTab::Log, false, &d), "read once");
        assert!(!d.apply(snap_of(&run)) && !wants(RunTab::Log, false, &d), "an unchanged snapshot reads nothing more");
    }

    #[test]
    fn a_language_change_makes_every_window_draw_again() {
        let mut p = Props::new(None);
        let ctx = egui::Context::default();
        let (a, b) = (key("r1", "a"), key("r2", "a"));
        open(&mut p, &a, RunTab::Facts);
        open(&mut p, &b, RunTab::Log);
        p.sync_lang(&ctx, &crate::i18n::table("ja").0);
        let now = |p: &Props| {
            let g = lock(&p.shared);
            (g.data[&a].stamp, g.data[&b].stamp)
        };
        let before = now(&p);
        p.sync_lang(&ctx, &crate::i18n::table("ja").0);
        assert_eq!(now(&p), before, "once per language");
        p.sync_lang(&ctx, &crate::i18n::table("en").0);
        let after = now(&p);
        assert!(after.0 > before.0 && after.1 > before.1);
    }

    /// What `tab_keys` reads when `events` arrive (and a popup is open or not).
    fn keys_read(events: Vec<egui::Event>, popup: bool) -> Option<bool> {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0))),
            events,
            ..Default::default()
        };
        let mut got = None;
        let mut out = ctx.run_ui(input, |ui| got = ui.input_mut(|i| tab_keys(i, popup)));
        out.textures_delta.clear();
        got
    }

    fn tab_press(modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![egui::Event::Key { key: egui::Key::Tab, physical_key: Some(egui::Key::Tab), pressed: true, repeat: false, modifiers }]
    }

    #[test]
    fn ctrl_tab_goes_forward_and_ctrl_shift_tab_goes_back() {
        assert_eq!(keys_read(tab_press(egui::Modifiers::CTRL), false), Some(true));
        assert_eq!(keys_read(tab_press(egui::Modifiers::CTRL | egui::Modifiers::SHIFT), false), Some(false));
        assert_eq!(keys_read(tab_press(egui::Modifiers::NONE), false), None, "a plain Tab is the focus's");
        assert_eq!(keys_read(tab_press(egui::Modifiers::CTRL), true), None, "not while a popup is open");
        assert_eq!(keys_read(tab_press(egui::Modifiers::CTRL | egui::Modifiers::SHIFT), true), None);
    }

    #[test]
    fn the_tabs_cycle_both_ways() {
        assert_eq!(cycle(RunTab::Facts, true), RunTab::Def);
        assert_eq!(cycle(RunTab::Hist, true), RunTab::Facts);
        assert_eq!(cycle(RunTab::Facts, false), RunTab::Hist);
        assert_eq!(cycle(RunTab::Log, false), RunTab::Def);
    }

    fn open(p: &mut Props, k: &PropsKey, tab: RunTab) {
        let slot = p.slot_for(k).unwrap();
        let run = run_of(&k.run);
        lock(&p.shared).data.insert(k.clone(), PropsData::new(run, &k.job, tab, false));
        p.windows.open(k.clone(), spec_of(slot, "t".into()), None);
    }

    fn run_of(id: &str) -> Run {
        use koyomi_core::jobnet::{Job, Jobnet};
        let now = ito_common::now::now_local().naive_local();
        let net = Jobnet { jobs: vec![Job { id: "a".into(), ..Default::default() }], ..Default::default() };
        Run::new(id, "n", "n", "2026-10-30", koyomi_core::run::How::Manual, &net, now)
    }

    #[test]
    fn a_ninth_window_closes_the_oldest_and_takes_its_slot() {
        let mut p = Props::new(None);
        for i in 0..CAP {
            open(&mut p, &key(&format!("r{i}"), "a"), RunTab::Facts);
        }
        assert_eq!(p.windows.keys().len(), CAP);
        assert!(p.make_room(), "the cap is reached");
        assert!(!p.windows.is_open(&key("r0", "a")));
        assert!(!lock(&p.shared).data.contains_key(&key("r0", "a")));
        assert_eq!(p.slot_for(&key("r8", "a")), Some(1), "the oldest's slot is the lowest free");
        assert!(!p.make_room(), "room again after one closed");
    }

    #[test]
    fn a_closed_report_for_a_key_open_again_keeps_its_data_and_an_act_of_a_closed_key_is_dropped() {
        let mut p = Props::new(None);
        let (a, b) = (key("r1", "a"), key("r2", "a"));
        open(&mut p, &a, RunTab::Log);
        open(&mut p, &b, RunTab::Facts);
        // `a` closed, then opened again before the report was read.
        p.windows.close(&a);
        p.windows.open(a.clone(), spec_of(1, "t".into()), None);
        let closed = vec![Closed { key: a.clone(), why: ito_viewport::Why::Esc }];
        assert_eq!(p.forget(&closed), 0);
        assert!(lock(&p.shared).data.contains_key(&a), "the data stays");
        // `b` closed and not opened again: its data and slot go, and its act is dropped.
        p.windows.close(&b);
        lock(&p.shared).outbox.push(Act::Editor(b.clone()));
        lock(&p.shared).outbox.push(Act::Editor(a.clone()));
        assert_eq!(p.forget(&[Closed { key: b.clone(), why: ito_viewport::Why::Button }]), 1);
        assert!(!lock(&p.shared).data.contains_key(&b));
        assert!(p.slots.iter().all(|(k, _)| k != &b));
        assert_eq!(p.take_acts(), vec![Act::Editor(a)]);
    }

    #[test]
    fn the_window_log_list_has_only_the_log_tabs_of_running_jobs() {
        let mut p = Props::new(None);
        let (a, b) = (key("r1", "a"), key("r2", "a"));
        open(&mut p, &a, RunTab::Log);
        open(&mut p, &b, RunTab::Facts);
        for k in [&a, &b] {
            let mut g = lock(&p.shared);
            let d = g.data.get_mut(k).unwrap();
            d.run.start("a", ito_common::now::now_local().naive_local());
        }
        assert_eq!(p.log_list(), vec![("r1".to_owned(), "a".to_owned())]);
        lock(&p.shared).data.get_mut(&a).unwrap().gone = true;
        assert!(p.log_list().is_empty());
    }

    #[test]
    fn the_generation_moves_only_when_the_text_changes() {
        let mut d = PropsData::new(run_of("r1"), "a", RunTab::Log, false);
        let at = ito_common::now::now_local().naive_local();
        d.run.start("a", at);
        assert!(d.take_log("one"));
        let g = d.gen;
        assert!(!d.take_log("one"));
        assert_eq!(d.gen, g);
        assert!(d.take_log(
            "one
two"
        ));
        assert_eq!(d.gen, g + 1);
        assert!(d.settled.is_none(), "a going job is not settled");
        d.run.finish("a", koyomi_core::run::Outcome::Exited(0), at);
        d.take_log(
            "one
two",
        );
        assert!(d.settled.is_some());
    }
}
