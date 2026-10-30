//! The window's left side (the tree, then today's runs and 履歴, then the calendars),
//! the detail beside it (a definition, a run as a picture with its log under it, the
//! runs of past days, or a calendar's month) and the boxes that ask before approving or killing.
//!
//! Nothing here touches a process: a thread keeps the engine up and asks it for the
//! list, the runs of today and the log being watched, and the window reads what that
//! thread sent. An approval, a start and a kill are sent the same way.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ito_a11y::Role;
use ito_common::kept;
use koyomi_core::run::{Run, State};
use koyomi_core::scan::{Def, Standing};
use koyomi_core::when;
use serde::{Deserialize, Serialize};

use crate::proto::{self, Request, Runs, Snapshot};
use crate::{client, engine, review, show};

pub mod calendar;
pub mod canvas;
pub mod graph;
pub mod whens;

/// How often the engine is asked for the list and the runs.
const POLL: Duration = Duration::from_secs(1);

enum Msg {
    Starting,
    Snapshot(Snapshot),
    Runs(Runs),
    /// The tail of one job's log: run, job, text.
    Log(String, String, String),
    Down(String),
    /// The engine's answer to a request the window sent.
    Answer(Request, Result<String, String>),
}

pub struct Approval {
    id: String,
    sha256: String,
}

/// The 取り込む box: a path to type, then the engine's plan for that file to confirm.
struct Port {
    path: String,
    /// The file's text once it was read: the plan is about it.
    text: Option<String>,
    plan: Option<proto::Plan>,
    error: Option<String>,
    /// A plan or the import itself is on the way to the engine.
    waiting: bool,
    /// The path field takes the keyboard on the first frame.
    fresh: bool,
}

/// The 新しいジョブネット box: the group and the name of the net to make.
struct Make {
    group: String,
    name: String,
    error: Option<String>,
    /// The name field takes the keyboard on the first frame.
    fresh: bool,
}

/// Why the 新しいジョブネット box cannot make its id yet.
#[derive(Debug, PartialEq, Eq)]
enum Why {
    Empty,
    Bad,
    Exists(String),
}

/// The id the 新しいジョブネット box makes from `group` and `name`, when no net in `ids` has it.
/// Windows does not tell `A` from `a` in a file name, so neither does this.
fn new_id<'a>(group: &str, name: &str, mut ids: impl Iterator<Item = &'a str>) -> Result<String, Why> {
    let (g, n) = (group.trim(), name.trim());
    if g.is_empty() || n.is_empty() {
        return Err(Why::Empty);
    }
    // What a file name on Windows cannot hold, and a trailing dot it would cut off.
    let bad = |s: &str| s.ends_with('.') || s.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c));
    let id = format!("{g}/{n}");
    if bad(g) || bad(n) || !koyomi_core::jobnet::is_id(&id) {
        return Err(Why::Bad);
    }
    let low = id.to_lowercase();
    if ids.any(|i| i.to_lowercase() == low) {
        return Err(Why::Exists(id));
    }
    Ok(id)
}

/// What the box shows at this point, and so which buttons it has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// The path field: 読む / やめる.
    Path,
    /// Nothing to choose: one button closes it.
    Close,
    /// The plan can be done as it is: 取り込む / やめる.
    Go,
    /// Something differs: 上書き / やめる / 名前を変える / 飛ばす.
    Pick,
}

impl Port {
    fn stage(&self) -> Stage {
        if self.text.is_none() {
            return Stage::Path;
        }
        if self.waiting || self.error.is_some() {
            return Stage::Close;
        }
        match &self.plan {
            Some(p) if p.problems == 0 && p.writes && p.undecided => Stage::Pick,
            Some(p) if p.problems == 0 && p.writes => Stage::Go,
            _ => Stage::Close,
        }
    }

    fn buttons(&self) -> usize {
        match self.stage() {
            Stage::Path | Stage::Go => 2,
            Stage::Close => 1,
            Stage::Pick => 4,
        }
    }
}

/// What `state.toml` keeps for the window.
#[derive(Default, Serialize, Deserialize)]
struct Kept {
    /// `horizontal` (the default) or `vertical`: which way a run's picture flows.
    #[serde(default)]
    flow: Option<String>,
}

/// The job whose log the thread fetches: `(run, job)`.
type Watch = Arc<Mutex<Option<(String, String)>>>;

/// 履歴: the days asked for, what narrows the rows, and the rows the engine sent for those days.
struct History {
    from: String,
    to: String,
    /// The net the rows are narrowed to; empty for all of them.
    net: String,
    state: Option<State>,
    rows: Vec<Run>,
    /// Why there are no rows: a date that does not read, or the engine's word.
    error: Option<String>,
}

impl History {
    /// The last 7 days up to `today`, nothing narrowed.
    fn week(today: chrono::NaiveDate) -> History {
        let from = today - chrono::Duration::days(7);
        History { from: from.to_string(), to: today.to_string(), net: String::new(), state: None, rows: Vec::new(), error: None }
    }

    fn request(&self) -> Request {
        Request::History { from: self.from.clone(), to: self.to.clone() }
    }

    /// The rows that match the net and the state, newest first.
    fn filtered(&self) -> Vec<&Run> {
        let mut rows: Vec<&Run> =
            self.rows.iter().filter(|r| (self.net.is_empty() || r.jobnet == self.net) && self.state.is_none_or(|s| r.state == s)).collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.started));
        rows
    }
}

pub struct Browser {
    rx: Receiver<Msg>,
    tx: Sender<Request>,
    watch: Watch,
    snap: Option<Snapshot>,
    runs: Vec<Run>,
    down: Option<String>,
    starting: bool,
    /// The definition picked in the tree.
    selected: Option<String>,
    /// The run picked under 今日; when it is set, the detail is the run.
    run_sel: Option<String>,
    /// The job whose log is shown, when one was picked by hand.
    job_sel: Option<String>,
    /// What the log search box holds (a regular expression, smart case).
    log_find: String,
    /// The last log tail fetched: run, job, text.
    log: Option<(String, String, String)>,
    vertical: bool,
    flow: Option<kept::Writer>,
    asking: Option<Approval>,
    /// The language when the plain wording is on (`easy = true` in common.toml).
    pub easy: Option<String>,
    /// The run a kill is being asked about.
    killing: Option<String>,
    focus: usize,
    /// The close question was up last frame: a new one starts on its safe button.
    close_asked: bool,
    /// The last answer to an approval or a start, shown under the detail.
    result: Option<Result<String, String>>,
    /// The runs that were over at the last poll; `None` before the first, so
    /// what was over before the window opened is not told of again.
    over: Option<BTreeSet<String>>,
    /// The runs that ended since [`Browser::take_ended`], oldest first.
    ended: Vec<Run>,
    /// The engine's notes about `[notify]` seen so far, and the ones not yet taken.
    seen_notes: BTreeSet<String>,
    notes: Vec<String>,
    /// A net runs on a schedule, but koyomi does not start at sign-in: the band says so.
    autostart_off: bool,
    /// The calendar picked on the left; when it is set, the detail is its month.
    cal_sel: Option<String>,
    /// The month shown, when it is not this one.
    cal_month: Option<(i32, u32)>,
    /// What the last click on a day did to the file, shown under the month.
    cal_result: Option<Result<String, String>>,
    /// 履歴, when it is open: the detail is its rows.
    hist: Option<History>,
    /// The editor, when a net is open in it: the detail is its canvas.
    editor: Option<canvas::Editor>,
    /// The 取り込む box, while it is open.
    port: Option<Port>,
    /// The 新しいジョブネット box, while it is open.
    make: Option<Make>,
    /// The editors a child net was entered from, outermost first.
    editors: Vec<canvas::Editor>,
}

impl Browser {
    /// Starts the thread. `state` is the state folder for `state.toml`; `None` keeps nothing.
    pub fn start(ctx: &egui::Context, state: Option<PathBuf>) -> Browser {
        let (tx_msg, rx) = mpsc::channel::<Msg>();
        let (tx, rx_req) = mpsc::channel::<Request>();
        let watch: Watch = Arc::default();
        let seen = watch.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || loop {
            let send = |m: Msg| {
                let alive = tx_msg.send(m).is_ok();
                ctx.request_repaint();
                alive
            };
            if !client::running() {
                if !send(Msg::Starting) {
                    return;
                }
                if let Err(e) = client::start() {
                    if !send(Msg::Down(e)) {
                        return;
                    }
                    std::thread::sleep(POLL * 3);
                    continue;
                }
            }
            match rx_req.recv_timeout(POLL) {
                Ok(request) => {
                    let answer = client::call(&request);
                    if !send(Msg::Answer(request, answer)) {
                        return;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            let m = match client::call(&Request::List).and_then(|t| Snapshot::parse(&t)) {
                Ok(s) => Msg::Snapshot(s),
                Err(e) => Msg::Down(e),
            };
            if !send(m) {
                return;
            }
            if let Ok(r) = client::call(&Request::Runs).and_then(|t| Runs::parse(&t)) {
                if !send(Msg::Runs(r)) {
                    return;
                }
            }
            let target = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some((run, job)) = target {
                if let Ok(text) = client::call(&Request::Log { run: run.clone(), job: job.clone() }) {
                    if !send(Msg::Log(run, job, text)) {
                        return;
                    }
                }
            }
        });
        let path = state.map(|d| d.join("state.toml"));
        let kept: Kept = path.as_deref().and_then(kept::read).unwrap_or_default();
        let vertical = kept.flow.as_deref() == Some("vertical");
        let flow = path.map(|p| kept::Writer::new(p, true));
        Browser {
            rx,
            tx,
            watch,
            snap: None,
            runs: Vec::new(),
            down: None,
            starting: true,
            selected: None,
            run_sel: None,
            job_sel: None,
            log_find: String::new(),
            log: None,
            vertical,
            flow,
            asking: None,
            killing: None,
            focus: 0,
            close_asked: false,
            result: None,
            over: None,
            ended: Vec::new(),
            seen_notes: BTreeSet::new(),
            notes: Vec::new(),
            autostart_off: false,
            cal_sel: None,
            cal_month: None,
            cal_result: None,
            hist: None,
            editor: None,
            editors: Vec::new(),
            port: None,
            make: None,
            easy: None,
        }
    }

    /// Whether the engine has answered once.
    pub fn snapshot_known(&self) -> bool {
        self.snap.is_some()
    }

    /// Whether the engine answered and has no definition at all.
    pub fn tree_is_empty(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.defs.is_empty())
    }

    pub fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Starting => self.starting = true,
                Msg::Snapshot(s) => {
                    self.starting = false;
                    self.down = None;
                    for n in &s.notes {
                        if self.seen_notes.insert(n.clone()) {
                            self.notes.push(n.clone());
                        }
                    }
                    let timed =
                        s.defs.iter().any(|d| d.standing == Standing::Confirmed && d.net.as_ref().is_some_and(|n| !n.when.is_empty()));
                    self.autostart_off = timed && !ito_common::startup::enabled("koyomi");
                    self.snap = Some(s);
                }
                Msg::Runs(r) => {
                    let now: BTreeSet<String> = r.runs.iter().filter(|r| r.is_over()).map(|r| r.id.clone()).collect();
                    if let Some(before) = &self.over {
                        self.ended.extend(r.runs.iter().filter(|r| r.is_over() && !before.contains(&r.id)).cloned());
                    }
                    self.over = Some(now);
                    self.runs = r.runs;
                }
                Msg::Log(run, job, text) => self.log = Some((run, job, text)),
                Msg::Down(e) => {
                    self.starting = false;
                    self.down = Some(e);
                }
                Msg::Answer(Request::History { .. }, r) => {
                    // The days asked for: the rows of 履歴, or why there are none.
                    if let Some(h) = &mut self.hist {
                        match r.and_then(|text| Runs::parse(&text)) {
                            Ok(rs) => {
                                h.rows = rs.runs;
                                h.error = None;
                            }
                            Err(e) => {
                                h.rows.clear();
                                h.error = Some(e);
                            }
                        }
                    }
                }
                Msg::Answer(Request::Save { text, .. }, r) => {
                    if let Some(e) = &mut self.editor {
                        match r {
                            Ok(said) => e.saved(&text, said),
                            // A net made here met a file made elsewhere: not this one's to overwrite.
                            Err(m) if m.starts_with("ほかで変わっています") && e.base.is_none() => {
                                e.result = Some(Err(format!("もうある（ほかで作られた）: {}。閉じて、別の名前で作り直す", e.id)))
                            }
                            Err(m) if m.starts_with("ほかで変わっています") => e.conflict = Some(text),
                            Err(m) => e.result = Some(Err(m)),
                        }
                    }
                }
                Msg::Answer(Request::Import { dry_run, .. }, r) => self.imported(dry_run, r),
                Msg::Answer(req, r) => {
                    // A start that went through opens the run.
                    if let (Request::Run { .. } | Request::Rerun { .. }, Ok(id)) = (&req, &r) {
                        self.run_sel = Some(id.clone());
                        self.job_sel = None;
                    }
                    self.result = Some(r);
                }
            }
        }
    }

    /// The ids of the loaded runs that ended abnormally (the status bar's count).
    pub fn failed_ids(&self) -> impl Iterator<Item = &str> {
        self.runs.iter().filter(|r| r.state.is_error()).map(|r| r.id.as_str())
    }

    /// The status bar's engine facts: `(engine word, running count, next time)`.
    /// The word is `None` while all is well and the engine answers.
    pub fn engine_facts(&self, t: &ito_i18n::Table) -> (Option<String>, usize, Option<chrono::NaiveDateTime>) {
        let running = self.runs.iter().filter(|r| !r.is_over()).count();
        let Some(snap) = &self.snap else {
            let word = match (&self.down, self.starting) {
                (Some(_), _) => t.tr("status.engine_down"),
                (None, _) => t.tr("status.engine_starting"),
            };
            return (Some(word.to_owned()), running, None);
        };
        let word = snap.paused.then(|| t.tr("status.paused").to_owned());
        let now = chrono::Local::now().naive_local();
        let next = snap
            .defs
            .iter()
            .filter(|d| !snap.held.contains(&d.id))
            .filter_map(|d| {
                let net = d.net.as_ref()?;
                let cal = d.calendar.as_deref().and_then(|c| snap.calendars.get(c));
                koyomi_core::when::next_times_in(&net.when, now, 1, cal).into_iter().next()
            })
            .min();
        (word, running, next)
    }

    /// The runs that ended since the last call, each once (the bell).
    pub fn take_ended(&mut self) -> Vec<Run> {
        self.drain();
        std::mem::take(&mut self.ended)
    }

    /// The engine's notes about `[notify]` not taken before, each once (the bell).
    pub fn take_notes(&mut self) -> Vec<String> {
        self.drain();
        std::mem::take(&mut self.notes)
    }

    /// Shows run `id` in the detail (a click under the bell).
    pub fn open_run(&mut self, id: &str) {
        self.run_sel = Some(id.to_owned());
        self.job_sel = None;
        self.hist = None;
        self.leave_editor();
    }

    fn blocked(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| !s.warnings.is_empty())
    }

    fn selected_def(&self) -> Option<&Def> {
        let id = self.selected.as_deref()?;
        self.snap.as_ref()?.defs.iter().find(|d| d.id == id)
    }

    fn selected_run(&self) -> Option<&Run> {
        let id = self.run_sel.as_deref()?;
        self.runs.iter().find(|r| r.id == id).or_else(|| self.hist.as_ref()?.rows.iter().find(|r| r.id == id))
    }

    /// The job whose log is shown: the one picked, else the one running, else the last that started.
    fn watched_job(run: &Run, picked: Option<&str>) -> Option<String> {
        if let Some(j) = picked.and_then(|p| run.jobs.iter().find(|j| j.id == p)) {
            return Some(j.id.clone());
        }
        let running = run.jobs.iter().find(|j| j.state == koyomi_core::run::State::Running);
        let last = run.jobs.iter().filter(|j| j.started.is_some()).max_by_key(|j| j.started);
        running.or(last).map(|j| j.id.clone())
    }

    fn set_flow(&mut self, vertical: bool) {
        self.vertical = vertical;
        if let Some(w) = &self.flow {
            w.save_toml(&Kept { flow: Some(if vertical { "vertical" } else { "horizontal" }.to_owned()) });
        }
    }

    /// Draws the warning band, the tree and the runs, the detail and, when asked for, a box.
    ///
    /// While a box is up it is the only thing that answers: its keys are taken before anything
    /// behind it is drawn, what is behind is disabled, and no widget keeps the keyboard focus.
    /// `<State:name>` under `--keys`: the engine, the counts and what is
    /// picked, one `key: value` line each.
    pub fn report(&self) -> String {
        let engine = match (&self.down, self.starting, &self.snap) {
            (Some(e), _, _) => format!("down: {e}"),
            (None, true, _) => "starting".to_owned(),
            (None, false, Some(_)) => "up".to_owned(),
            (None, false, None) => "waiting".to_owned(),
        };
        let mut out = format!("engine: {engine}\n");
        if let Some(s) = &self.snap {
            out.push_str(&format!("nets: {}\nwarnings: {}\n", s.defs.len(), s.warnings.len()));
        }
        out.push_str(&format!("runs: {}\n", self.runs.len()));
        out.push_str(&format!("selected: {}\n", self.selected.as_deref().unwrap_or("-")));
        out.push_str(&format!("run: {}\n", self.run_sel.as_deref().unwrap_or("-")));
        out.push_str(&format!("job: {}\n", self.job_sel.as_deref().unwrap_or("-")));
        let history = match &self.hist {
            Some(h) => {
                let net = if h.net.is_empty() { "*" } else { h.net.as_str() };
                format!("{}..{} {net} {} {}", h.from, h.to, h.state.map_or("*", |s| s.key()), h.filtered().len())
            }
            None => "-".to_owned(),
        };
        out.push_str(&format!("history: {history}\n"));
        let asking = match (&self.asking, &self.killing) {
            (Some(a), _) => format!("approve {}", a.id),
            (None, Some(r)) => format!("kill {r}"),
            (None, None) => "-".to_owned(),
        };
        out.push_str(&format!("asking: {asking}\nflow: {}\n", if self.vertical { "vertical" } else { "horizontal" }));
        if let Some(m) = &self.make {
            out.push_str(&format!("make: {}/{}{}\n", m.group, m.name, m.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default()));
        }
        if let Some(e) = &self.editor {
            out.push_str(&e.report());
            out.push('\n');
        }
        out
    }

    /// Opens net `id` in the editor (the 編集 button; a net that does not read says why instead).
    pub fn open_editor(&mut self, id: &str) {
        let Some(def) = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == id)) else { return };
        match canvas::Editor::open(def) {
            Ok(e) => {
                self.editor = Some(e);
                self.editors.clear();
                self.result = None;
            }
            Err(m) => self.result = Some(Err(m)),
        }
    }

    /// The palette's 書き出す: everything in the settings folder, to
    /// `exports/<YYYYMMDD-HHMMSS>.toml` in the state folder. The path is told
    /// under the detail.
    pub fn export(&mut self, t: &ito_i18n::Table) {
        self.result = Some(export_all().map(|(path, n)| t.trf("port.exported", &[&path, &n.to_string()])));
    }

    /// The palette's 取り込む: opens the box with the path field.
    pub fn import_box(&mut self) {
        if self.port.is_none() {
            self.port = Some(Port { path: String::new(), text: None, plan: None, error: None, waiting: false, fresh: true });
            self.focus = 0;
        }
    }

    /// 新しいジョブネット: opens the box, with `group` filled in when it came from a group in the tree.
    pub fn new_box(&mut self, group: Option<&str>) {
        if self.make.is_none() && self.port.is_none() && self.snap.is_some() {
            self.make = Some(Make { group: group.unwrap_or_default().to_owned(), name: String::new(), error: None, fresh: true });
            self.focus = 0;
        }
    }

    /// The box's 作る: the template (the one `koyomi new` writes) in a new editor. Nothing is
    /// written until it is saved.
    fn make_go(&mut self, t: &ito_i18n::Table) {
        let Some(m) = &mut self.make else { return };
        let unsaved = self.editor.as_ref().is_some_and(|e| e.dirty) || self.editors.iter().any(|e| e.dirty);
        let ids = self.snap.iter().flat_map(|s| s.defs.iter().map(|d| d.id.as_str()));
        let made = match new_id(&m.group, &m.name, ids) {
            Err(Why::Empty) => Err(t.tr("make.empty").to_owned()),
            Err(Why::Bad) => Err(t.tr("make.bad").to_owned()),
            Err(Why::Exists(id)) => Err(t.trf("make.exists", &[id.as_str()])),
            Ok(_) if unsaved => Err(t.tr("make.unsaved").to_owned()),
            Ok(id) => koyomi_core::jobnet::parse(&koyomi_core::write::template(m.name.trim()))
                .map(|net| (id, net))
                .map_err(|p| p.iter().map(|p| p.message.clone()).collect::<Vec<_>>().join("\n")),
        };
        match made {
            Err(e) => m.error = Some(e),
            Ok((id, net)) => {
                self.make = None;
                self.editor = Some(canvas::Editor::new_net(&id, net));
                self.editors.clear();
                self.selected = None;
                self.run_sel = None;
                self.cal_sel = None;
                self.job_sel = None;
                self.result = None;
                self.hist = None;
            }
        }
    }

    /// The 新しいジョブネット box: a group (typed, or one there is) and a name.
    fn confirm_make(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let mut groups: Vec<String> = self.snap.iter().flat_map(|s| s.defs.iter().map(|d| d.group.clone())).collect();
        groups.dedup();
        let Some(m) = &mut self.make else { return };
        let c = ito_theme::colors();
        let mut pressed = None;
        egui::Window::new(t.tr("make.title"))
            .id(egui::Id::new("make"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.add(egui::Label::new(t.tr("make.lead")).wrap());
                egui::Grid::new("make-fields").num_columns(2).show(ui, |ui| {
                    ui.label(t.tr("make.group"));
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut m.group).hint_text(t.tr("make.group_hint")).desired_width(240.0));
                        ito_a11y::say(&r, Role::Label, t.tr("make.group"));
                        if !groups.is_empty() {
                            egui::ComboBox::from_id_salt("make-group").selected_text(t.tr("make.pick")).show_ui(ui, |ui| {
                                for g in &groups {
                                    if ui.selectable_label(m.group.trim() == g, g).clicked() {
                                        m.group = g.clone();
                                    }
                                }
                            });
                        }
                    });
                    ui.end_row();
                    ui.label(t.tr("make.name"));
                    let r = ui.add(egui::TextEdit::singleline(&mut m.name).hint_text(t.tr("make.name_hint")).desired_width(240.0));
                    ito_a11y::say(&r, Role::Label, t.tr("make.name"));
                    if m.fresh {
                        r.request_focus();
                        m.fresh = false;
                    }
                    ui.end_row();
                });
                if let Some(e) = &m.error {
                    ui.add(egui::Label::new(egui::RichText::new(e).color(c.err)).wrap());
                }
                ui.horizontal(|ui| {
                    for (i, label) in [t.tr("make.go"), t.tr("make.cancel")].into_iter().enumerate() {
                        let r = ui.button(label);
                        ito_a11y::say(&r, Role::Button, label);
                        if r.clicked() {
                            pressed = Some(i);
                        }
                    }
                });
                ui.label(egui::RichText::new(t.tr("make.hint")).color(c.dim).small());
            });
        match pressed.or(key) {
            Some(0) => self.make_go(t),
            Some(_) => self.make = None,
            None => {}
        }
    }

    /// The engine's answer to an import: a plan to show, or what it did.
    fn imported(&mut self, dry_run: bool, r: Result<String, String>) {
        let Some(p) = &mut self.port else {
            if !dry_run {
                self.result = Some(r.map(|_| "取り込んだ（ジョブネットは未確認）".to_owned()));
            }
            return;
        };
        p.waiting = false;
        match r.and_then(|text| proto::Plan::parse(&text)) {
            Ok(plan) if plan.applied => {
                self.port = None;
                self.result = Some(Ok("取り込んだ（ジョブネットは未確認。確かめてから流す）".to_owned()));
            }
            Ok(plan) => {
                // Not done: the plan to confirm (or, after a choice, what changed meanwhile).
                self.focus = if plan.undecided { 1 } else { 0 };
                p.plan = Some(plan);
            }
            Err(e) => p.error = Some(e),
        }
    }

    /// Reads the file in the path field and asks the engine what it would do.
    fn port_read(&mut self) {
        let Some(p) = &mut self.port else { return };
        let path = p.path.trim().trim_matches('"').to_owned();
        if path.is_empty() {
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                p.error = None;
                p.text = Some(text.clone());
                p.waiting = true;
                self.focus = 0;
                let _ = self.tx.send(Request::Import { text, dry_run: true, on_conflict: None });
            }
            Err(e) => p.error = Some(format!("{path} を読めない: {e}")),
        }
    }

    /// Sends the import itself, with the choice for what differs.
    fn port_go(&mut self, on_conflict: Option<&str>) {
        let Some(p) = &mut self.port else { return };
        let Some(text) = p.text.clone() else { return };
        p.plan = None;
        p.waiting = true;
        self.focus = 0;
        let _ = self.tx.send(Request::Import { text, dry_run: false, on_conflict: on_conflict.map(str::to_owned) });
    }

    /// The 取り込む box: the path field, then the plan with its buttons.
    fn confirm_port(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(p) = &mut self.port else { return };
        let c = ito_theme::colors();
        let stage = p.stage();
        if stage == Stage::Path {
            let mut pressed = None;
            egui::Window::new(t.tr("port.title"))
                .id(egui::Id::new("port-path"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_width(480.0);
                    ui.add(egui::Label::new(t.tr("port.lead")).wrap());
                    let r = ui.add(egui::TextEdit::singleline(&mut p.path).hint_text(t.tr("port.path_hint")).desired_width(f32::INFINITY));
                    ito_a11y::say(&r, Role::Label, t.tr("port.file"));
                    if p.fresh {
                        r.request_focus();
                        p.fresh = false;
                    }
                    if let Some(e) = &p.error {
                        ui.add(egui::Label::new(egui::RichText::new(e).color(c.err)).wrap());
                    }
                    ui.horizontal(|ui| {
                        for (i, label) in [t.tr("port.read"), t.tr("port.cancel")].into_iter().enumerate() {
                            let r = ui.button(label);
                            ito_a11y::say(&r, Role::Button, label);
                            if r.clicked() {
                                pressed = Some(i);
                            }
                        }
                    });
                    ui.label(egui::RichText::new(t.tr("port.hint_path")).color(c.dim).small());
                });
            match pressed.or(key) {
                Some(0) => self.port_read(),
                Some(_) => self.port = None,
                None => {}
            }
            return;
        }
        const MOST: usize = 30;
        let rows = [(t.tr("port.file").to_owned(), p.path.trim().to_owned())];
        let mut body: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        if p.waiting {
            body.push(t.tr(if p.plan.is_none() && p.text.is_some() { "port.doing" } else { "port.reading" }).to_owned());
        }
        if let Some(e) = &p.error {
            notes.push(e.clone());
        }
        if let Some(plan) = &p.plan {
            body.extend(plan.lines.iter().take(MOST).cloned());
            if plan.lines.len() > MOST {
                body.push(t.trf("port.more", &[&(plan.lines.len() - MOST).to_string()]));
            }
            if plan.problems > 0 {
                notes.push(t.trf("port.problems", &[&plan.problems.to_string()]));
            } else if !plan.writes {
                body.push(t.tr("port.nothing").to_owned());
            } else if plan.undecided {
                notes.push(t.trf("port.pick", &[&plan.conflicts.to_string()]));
            }
        }
        let buttons: Vec<String> = match stage {
            Stage::Close => vec![t.tr("port.close").to_owned()],
            Stage::Go => vec![t.tr("port.go").to_owned(), t.tr("port.cancel").to_owned()],
            _ => vec![
                t.tr("port.overwrite").to_owned(),
                t.tr("port.cancel").to_owned(),
                t.tr("port.rename").to_owned(),
                t.tr("port.skip").to_owned(),
            ],
        };
        let q = ito_confirm::Question {
            title: t.tr("port.title"),
            rows: &rows,
            body: &body,
            notes: &notes,
            buttons: &buttons,
            hint: t.tr(if stage == Stage::Pick { "port.hint_pick" } else { "confirm.hint" }),
            focus: Some(self.focus),
            primary: if stage == Stage::Go { Some(0) } else { None },
            ..Default::default()
        };
        let Some(i) = ito_confirm::show(ctx, &q, &Self::style()).or(key) else { return };
        match (stage, i) {
            (Stage::Go, 0) => self.port_go(None),
            (Stage::Pick, 0) => self.port_go(Some("overwrite")),
            (Stage::Pick, 2) => self.port_go(Some("rename")),
            (Stage::Pick, 3) => self.port_go(Some("skip")),
            _ => self.port = None,
        }
    }

    /// Whether the editor is open (the palette shows its commands then).
    pub fn editing(&self) -> bool {
        self.editor.is_some()
    }

    /// A command from the palette or a key, for the editor.
    pub fn edit(&mut self, cmd: canvas::Cmd) {
        let Some(e) = &mut self.editor else { return };
        if let Some(a) = e.edit(cmd) {
            self.act(a);
        }
    }

    /// Leaves the editor for another view; with changes anywhere it asks first.
    fn leave_editor(&mut self) {
        let Some(e) = &mut self.editor else { return };
        if e.dirty || self.editors.iter().any(|e| e.dirty) {
            e.closing = true;
        } else {
            self.editor = None;
            self.editors.clear();
        }
    }

    /// What the editor asked for.
    fn act(&mut self, a: canvas::Action) {
        match a {
            canvas::Action::Save { force } => {
                if let Some(e) = &mut self.editor {
                    e.result = None;
                    let _ = self.tx.send(Request::Save { id: e.id.clone(), text: e.text(), base_sha256: e.base.clone(), force });
                }
            }
            canvas::Action::Flow => self.set_flow(!self.vertical),
            canvas::Action::Close => self.editor = self.editors.pop(),
            canvas::Action::Enter(id) => {
                let def = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == id)).cloned();
                let opened = def.as_ref().ok_or_else(|| format!("ジョブネットがない: {id}")).and_then(canvas::Editor::open);
                match (opened, &mut self.editor) {
                    (Ok(inner), Some(outer)) => {
                        let outer = std::mem::replace(outer, inner);
                        self.editors.push(outer);
                    }
                    (Err(m), Some(e)) => e.result = Some(Err(m)),
                    (_, None) => {}
                }
            }
        }
    }

    fn editor_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let empty = BTreeMap::new();
        let calendars = self.snap.as_ref().map(|s| &s.calendars).unwrap_or(&empty);
        let depth = self.editors.len();
        let Some(e) = &mut self.editor else { return };
        let action = e.show(ui, t, self.vertical, calendars, depth);
        if let Some(a) = action {
            self.act(a);
        }
    }

    /// The boxes over the editor: a save that met another change, and leaving with changes.
    fn confirm_editor(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(e) = &self.editor else { return };
        if let Some(text) = e.conflict.clone() {
            let Some(def) = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == e.id)).cloned() else {
                if let Some(e) = &mut self.editor {
                    e.conflict = None;
                }
                return;
            };
            let diff = ito_diff::diff(&e.base_text, &def.text, 3);
            let rows = [(t.tr("review.file").to_owned(), def.file.clone())];
            let body = [t.tr("canvas.conflict_body").to_owned()];
            let buttons = [t.tr("canvas.reload").to_owned(), t.tr("canvas.cancel").to_owned(), t.tr("canvas.overwrite").to_owned()];
            let q = ito_confirm::Question {
                title: t.tr("canvas.conflict_title"),
                rows: &rows,
                body: &body,
                diff: &diff,
                buttons: &buttons,
                hint: t.tr("confirm.hint"),
                focus: Some(self.focus),
                primary: Some(0),
                danger: true,
                ..Default::default()
            };
            let answer = ito_confirm::show(ctx, &q, &Self::style()).or(key);
            let Some(e) = &mut self.editor else { return };
            match answer {
                Some(0) => e.reload(&def),
                Some(2) => {
                    e.conflict = None;
                    e.result = None;
                    let _ = self.tx.send(Request::Save { id: e.id.clone(), text, base_sha256: e.base.clone(), force: true });
                }
                Some(_) => e.conflict = None,
                None => {}
            }
            return;
        }
        if e.closing {
            let body = [t.tr("canvas.close_body").to_owned()];
            let buttons = [t.tr("canvas.close_yes").to_owned(), t.tr("canvas.close_no").to_owned()];
            let q = ito_confirm::Question {
                title: t.tr("canvas.close_title"),
                body: &body,
                buttons: &buttons,
                hint: t.tr("confirm.hint"),
                focus: Some(self.focus),
                primary: Some(1),
                danger: true,
                ..Default::default()
            };
            let answer = ito_confirm::show(ctx, &q, &Self::style()).or(key);
            let Some(e) = &mut self.editor else { return };
            match answer {
                Some(0) => {
                    e.closing = false;
                    self.editor = None;
                    self.editors.clear();
                }
                Some(_) => e.closing = false,
                None => {}
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        self.drain();
        let ctx = ui.ctx().clone();
        let conflict = self.editor.as_ref().is_some_and(|e| e.conflict.is_some());
        let port = self.port.as_ref().map(|p| (p.buttons(), p.stage() == Stage::Path));
        let making = self.make.is_some();
        let closing = self.editor.as_ref().is_some_and(|e| e.closing);
        if closing && !self.close_asked {
            // The safe button first: Enter on a fresh box does not throw the changes away.
            self.focus = 1;
        }
        self.close_asked = closing;
        let modal = self.asking.is_some() || self.killing.is_some() || conflict || closing || port.is_some() || making;
        let buttons = if conflict { 3 } else { port.map_or(2, |(n, _)| n) };
        let key = if modal { self.box_keys(&ctx, buttons, making) } else { None };
        let c = ito_theme::colors();
        if let Some(snap) = self.snap.as_ref().filter(|s| !s.warnings.is_empty()) {
            let lines: Vec<String> = snap.warnings.clone();
            egui::Panel::top("band").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                let mark = ito_a11y::State::Warning.mark();
                let head = format!("{mark} {}", t.tr("band.warning"));
                let r = ui.label(egui::RichText::new(head.clone()).strong().color(c.wait_text()));
                ito_a11y::say(&r, Role::Label, &head);
                for l in &lines {
                    ui.label(egui::RichText::new(l).color(c.wait_text()));
                }
            });
        }
        if self.snap.as_ref().is_some_and(|s| s.paused) {
            egui::Panel::top("paused").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let head = format!("{} {}", State::Held.mark(), t.tr("band.paused"));
                    let r = ui.label(egui::RichText::new(head.clone()).strong().color(c.wait_text()));
                    ito_a11y::say(&r, Role::Label, &head);
                    let r = ui.add_enabled(!modal, egui::Button::new(t.tr("run.resume")));
                    ito_a11y::say(&r, Role::Button, t.tr("run.resume"));
                    if r.clicked() {
                        let _ = self.tx.send(Request::Resume);
                    }
                });
            });
        }
        if self.autostart_off {
            egui::Panel::top("autostart").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                let head = format!("{} {}", ito_a11y::State::Warning.mark(), t.tr("band.autostart"));
                let r = ui.add(egui::Label::new(egui::RichText::new(head.clone()).color(c.wait_text())).wrap());
                ito_a11y::say(&r, Role::Label, &head);
            });
        }
        // The calendars: a file that does not read, a holiday table near its end. Nothing stops for these.
        if let Some(snap) = self.snap.as_ref().filter(|s| !s.cautions.is_empty()) {
            let lines: Vec<String> = snap.cautions.clone();
            egui::Panel::top("cautions").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                let head = format!("{} {}", ito_a11y::State::Warning.mark(), t.tr("band.caution"));
                let r = ui.label(egui::RichText::new(head.clone()).strong().color(c.wait_text()));
                ito_a11y::say(&r, Role::Label, &head);
                for l in &lines {
                    ui.add(egui::Label::new(egui::RichText::new(l).color(c.wait_text())).wrap());
                }
            });
        }
        egui::Panel::left("tree").default_size(320.0).show(ui, |ui| {
            if modal {
                ui.disable();
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.tree(ui, t);
                ui.add_space(10.0);
                self.today(ui, t);
                self.history_link(ui, t);
                self.calendar_list(ui, t);
                // The one switch over the whole schedule, below what it stops.
                let paused = self.snap.as_ref().is_some_and(|s| s.paused);
                if !paused && self.snap.is_some() {
                    ui.add_space(10.0);
                    let r = ui.button(t.tr("run.pause"));
                    ito_a11y::say(&r, Role::Button, t.tr("run.pause"));
                    if r.clicked() {
                        let _ = self.tx.send(Request::Pause);
                    }
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if modal {
                ui.disable();
            }
            if self.editor.is_some() {
                self.editor_view(ui, t);
            } else if self.hist.is_some() {
                self.history_view(ui, t);
            } else if self.run_sel.is_some() {
                self.run_view(ui, t);
            } else if self.cal_sel.is_some() {
                self.calendar_view(ui, t);
            } else {
                self.detail(ui, t);
            }
        });
        self.confirm(&ctx, t, key);
        self.confirm_kill(&ctx, t, key);
        self.confirm_editor(&ctx, t, key);
        self.confirm_port(&ctx, t, key);
        self.confirm_make(&ctx, t, key);
        if modal && !making && !port.is_some_and(|(_, typing)| typing) {
            // Tab moves egui's focus before any of this runs; drop it so the next Enter or Space
            // cannot press a button behind the box (or in it: the box's own focus is `self.focus`).
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }

    /// Esc (cancel) and Enter (the button the keyboard is on) answer the box; Tab and Shift+Tab move between its two buttons,
    /// or, in a box of `fields`, between the fields.
    fn box_keys(&mut self, ctx: &egui::Context, buttons: usize, fields: bool) -> Option<usize> {
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                Some(1)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                Some(self.focus)
            } else {
                if fields {
                    return None;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) || i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
                    self.focus = (self.focus + 1) % buttons;
                }
                None
            }
        })
    }

    fn tree(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        if self.snap.is_some() {
            let label = match &self.easy {
                Some(lang) => t.trf("tree.new_easy", &[&crate::glossary::GLOSSARY.label("jobnet", lang, true, "")]),
                None => t.tr("tree.new").to_owned(),
            };
            let r = ui.button(&label);
            ito_a11y::say(&r, Role::Button, &label);
            if let Some(lang) = &self.easy {
                ito_glossary::hover(ui, ui.painter(), r.rect, &crate::glossary::GLOSSARY, "jobnet", lang, "tree");
            }
            if r.clicked() {
                self.new_box(None);
            }
        }
        let Some(snap) = &self.snap else {
            let text = match (&self.down, self.starting) {
                (Some(e), _) => t.trf("tree.failed", &[e]),
                (None, _) => t.tr("tree.starting").to_owned(),
            };
            ui.label(text);
            return;
        };
        if snap.defs.is_empty() {
            ui.add(egui::Label::new(t.tr("tree.empty")).wrap());
            return;
        }
        if let Some(e) = &self.down {
            ui.label(egui::RichText::new(t.trf("tree.failed", &[e])).color(c.err));
        }
        let mut pick = None;
        let mut make_in = None;
        let mut group: Option<&str> = None;
        for d in &snap.defs {
            if group != Some(d.group.as_str()) {
                group = Some(&d.group);
                let r = ui.add(egui::Label::new(egui::RichText::new(&d.group).strong().color(c.dim)).sense(egui::Sense::click()));
                ito_a11y::say(&r, Role::Heading, &d.group);
                r.context_menu(|ui| {
                    if ui.button(t.tr("tree.new_here")).clicked() {
                        make_in = Some(d.group.clone());
                        ui.close();
                    }
                });
            }
            let text = review::row(t, d, snap.held.contains(&d.id));
            let on = self.run_sel.is_none() && self.hist.is_none() && self.selected.as_deref() == Some(d.id.as_str());
            let r = ui.selectable_label(on, format!("  {text}"));
            ito_a11y::say(&r, Role::Row, &text);
            if r.clicked() {
                pick = Some(d.id.clone());
            }
        }
        if let Some(g) = make_in {
            self.new_box(Some(&g));
        }
        if pick.is_some() {
            self.selected = pick;
            self.run_sel = None;
            self.cal_sel = None;
            self.job_sel = None;
            self.result = None;
            self.hist = None;
            self.leave_editor();
        }
    }

    /// Today's runs, newest first: symbol and state, net, planned or started, ended, duration.
    fn today(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let r = ui.label(egui::RichText::new(t.tr("run.today")).strong().color(c.dim));
        ito_a11y::say(&r, Role::Heading, t.tr("run.today"));
        let Some(snap) = &self.snap else { return };
        // What is still to come today, soonest first: a time, not a run, so not a row to open.
        let now = chrono::Local::now().naive_local();
        let end = now.date().and_hms_opt(23, 59, 59).unwrap_or(now);
        let mut coming: Vec<(chrono::NaiveDateTime, String)> = Vec::new();
        for d in snap.defs.iter().filter(|d| d.standing == Standing::Confirmed) {
            if let Some(net) = &d.net {
                let cal = d.calendar.as_deref().and_then(|c| snap.calendars.get(c));
                coming.extend(when::between_in(&net.when, now, end, cal).into_iter().map(|at| (at, d.label().to_owned())));
            }
        }
        coming.sort();
        if self.runs.is_empty() && coming.is_empty() {
            ui.add(egui::Label::new(t.tr("run.none")).wrap());
            return;
        }
        for (at, title) in &coming {
            let text = format!("{}  {title}  {}", show::tag(t, State::Waiting), show::hms(*at));
            let r = ui.label(egui::RichText::new(format!("  {text}")).color(graph::colour(&c, State::Waiting)));
            ito_a11y::say(&r, Role::Row, &text);
        }
        let mut pick = None;
        let mut runs: Vec<&Run> = self.runs.iter().collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.started));
        for run in runs {
            let planned = run.planned.map(show::hms).unwrap_or_else(|| show::hms(run.started));
            let mut text = format!("{}  {}  {}", show::tag(t, run.state), run.title, planned);
            if let Some(e) = run.ended {
                text.push_str(&format!("  {}", show::hms(e)));
            }
            if let Some(d) = run.duration() {
                text.push_str(&format!("  {}", show::duration(d)));
            }
            let on = self.run_sel.as_deref() == Some(run.id.as_str());
            let colour = graph::colour(&c, run.state);
            let r = ui.selectable_label(on, egui::RichText::new(format!("  {text}")).color(colour));
            ito_a11y::say(&r, Role::Row, &text);
            if r.clicked() {
                pick = Some(run.id.clone());
            }
        }
        if pick.is_some() {
            self.run_sel = pick;
            self.cal_sel = None;
            self.job_sel = None;
            self.result = None;
            self.hist = None;
            self.leave_editor();
        }
    }

    /// The 履歴 row under today's runs: a click opens the last 7 days.
    fn history_link(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        if self.snap.is_none() {
            return;
        }
        let r = ui.selectable_label(self.hist.is_some(), format!("  {}", t.tr("run.history")));
        ito_a11y::say(&r, Role::Row, t.tr("run.history"));
        if r.clicked() {
            self.open_history();
        }
    }

    /// Opens 履歴 on the last 7 days and asks the engine for them.
    pub fn open_history(&mut self) {
        let h = History::week(chrono::Local::now().date_naive());
        let _ = self.tx.send(h.request());
        self.hist = Some(h);
        self.run_sel = None;
        self.cal_sel = None;
        self.job_sel = None;
        self.result = None;
    }

    /// 履歴: the days and what narrows the rows, the rows that match (newest
    /// first, each the day it started, the state, the net, the times), and under
    /// them the run picked, as under 今日 (its picture, its log, 再実行).
    fn history_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let nets: Vec<String> = self.snap.as_ref().map(|s| s.defs.iter().map(|d| d.id.clone()).collect()).unwrap_or_default();
        let Some(h) = self.hist.as_mut() else { return };
        ui.heading(t.tr("run.history"));
        ui.add_space(6.0);
        let mut search = false;
        ui.horizontal(|ui| {
            ui.label(t.tr("run.history_from"));
            ui.add(egui::TextEdit::singleline(&mut h.from).desired_width(90.0));
            ui.label(t.tr("run.history_to"));
            ui.add(egui::TextEdit::singleline(&mut h.to).desired_width(90.0));
            let all = t.tr("run.history_all");
            let net_text = if h.net.is_empty() { all.to_owned() } else { h.net.clone() };
            egui::ComboBox::from_id_salt("history_net").selected_text(net_text).show_ui(ui, |ui| {
                ui.selectable_value(&mut h.net, String::new(), all);
                for id in &nets {
                    ui.selectable_value(&mut h.net, id.clone(), id);
                }
            });
            let state_text = match h.state {
                Some(s) => show::tag(t, s),
                None => all.to_owned(),
            };
            egui::ComboBox::from_id_salt("history_state").selected_text(state_text).show_ui(ui, |ui| {
                ui.selectable_value(&mut h.state, None, all);
                for s in State::ALL {
                    ui.selectable_value(&mut h.state, Some(s), show::tag(t, s));
                }
            });
            let r = ui.button(t.tr("run.history_search"));
            ito_a11y::say(&r, Role::Button, t.tr("run.history_search"));
            search = r.clicked();
        });
        if search {
            let day = |s: &str| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok();
            match (day(&h.from), day(&h.to)) {
                (Some(from), Some(to)) => {
                    h.from = from.to_string();
                    h.to = to.to_string();
                    h.error = None;
                    let _ = self.tx.send(h.request());
                }
                _ => h.error = Some(t.tr("run.history_bad_date").to_owned()),
            }
        }
        if let Some(e) = &h.error {
            ui.label(egui::RichText::new(e).color(c.err));
        }
        ui.add_space(6.0);
        let rows = h.filtered();
        ui.label(egui::RichText::new(t.trf("run.history_count", &[&rows.len().to_string()])).color(c.dim));
        if rows.is_empty() && h.error.is_none() {
            ui.label(t.tr("run.history_none"));
        }
        let mut pick = None;
        for run in rows {
            let mut text = format!("{}  {}  {}  {}", run.started.date(), show::tag(t, run.state), run.title, show::hms(run.started));
            if let Some(e) = run.ended {
                text.push_str(&format!("  {}", show::hms(e)));
            }
            if let Some(d) = run.duration() {
                text.push_str(&format!("  {}", show::duration(d)));
            }
            let on = self.run_sel.as_deref() == Some(run.id.as_str());
            let r = ui.selectable_label(on, egui::RichText::new(format!("  {text}")).color(graph::colour(&c, run.state)));
            ito_a11y::say(&r, Role::Row, &text);
            if r.clicked() {
                pick = Some(run.id.clone());
            }
        }
        if pick.is_some() {
            self.run_sel = pick;
            self.job_sel = None;
            self.result = None;
        }
        if self.run_sel.is_some() {
            ui.add_space(10.0);
            ui.separator();
            self.run_view(ui, t);
        }
    }

    /// The run: its line, the buttons, the picture, the jobs in words, the log.
    fn run_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(run) = self.selected_run().cloned() else {
            ui.label(t.tr("run.none"));
            return;
        };
        let head = show::run_line(t, &run);
        ui.heading(&head);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let r = ui.add_enabled(!run.is_over(), egui::Button::new(t.tr("run.kill")));
            ito_a11y::say(&r, Role::Button, t.tr("run.kill"));
            if r.clicked() {
                self.killing = Some(run.id.clone());
                // The safe button first: Enter on a fresh box does not kill.
                self.focus = 1;
            }
            // Again, once it is over: what passed is carried over. From the picked job when one is picked.
            let can = run.is_over() && !self.blocked();
            let r = ui.add_enabled(can, egui::Button::new(t.tr("run.rerun")));
            ito_a11y::say(&r, Role::Button, t.tr("run.rerun"));
            if r.clicked() {
                let _ = self.tx.send(Request::Rerun { run: run.id.clone(), from: None });
            }
            let picked = self.job_sel.clone().filter(|p| run.jobs.iter().any(|j| &j.id == p));
            let label = match &picked {
                Some(j) => t.trf("run.rerun_from", &[j]),
                None => t.tr("run.rerun_pick").to_owned(),
            };
            let r = ui.add_enabled(can && picked.is_some(), egui::Button::new(&label));
            ito_a11y::say(&r, Role::Button, &label);
            if r.clicked() {
                let _ = self.tx.send(Request::Rerun { run: run.id.clone(), from: picked.clone() });
            }
            // Skip the picked job before it starts: the jobs after it go on.
            let pending = picked.as_deref().and_then(|p| run.job(p)).is_some_and(|j| j.pending() && j.state != State::Running);
            let label = match &picked {
                Some(j) => t.trf("run.skip_from", &[j]),
                None => t.tr("run.skip_pick").to_owned(),
            };
            let r = ui.add_enabled(!run.is_over() && pending, egui::Button::new(&label));
            ito_a11y::say(&r, Role::Button, &label);
            if r.clicked() {
                if let Some(job) = picked.clone() {
                    let _ = self.tx.send(Request::Skip { run: run.id.clone(), job });
                }
            }
            ui.separator();
            for (vertical, key) in [(false, "run.horizontal"), (true, "run.vertical")] {
                let r = ui.selectable_label(self.vertical == vertical, t.tr(key));
                ito_a11y::say(&r, Role::Button, t.tr(key));
                if r.clicked() && self.vertical != vertical {
                    self.set_flow(vertical);
                }
            }
        });
        self.show_result(ui);
        ui.add_space(6.0);
        let watched = Self::watched_job(&run, self.job_sel.as_deref());
        *self.watch.lock().unwrap_or_else(|e| e.into_inner()) = watched.clone().map(|j| (run.id.clone(), j));
        let mut pick = None;
        egui::ScrollArea::horizontal().id_salt("graph").auto_shrink([false, true]).show(ui, |ui| {
            pick = graph::draw(ui, &run, self.vertical, watched.as_deref());
        });
        for j in &run.jobs {
            let text = graph::words(t, j);
            let on = watched.as_deref() == Some(j.id.as_str());
            let r = ui.selectable_label(on, egui::RichText::new(&text).color(graph::colour(&c, j.state)));
            ito_a11y::say(&r, Role::Row, &text);
            if r.clicked() {
                pick = Some(j.id.clone());
            }
        }
        if pick.is_some() {
            self.job_sel = pick;
        }
        if let Some(job) = watched.as_deref().and_then(|w| run.job(w)) {
            ui.add_space(6.0);
            egui::Grid::new("job_facts").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                for (k, v) in show::job_facts(t, job) {
                    ui.label(egui::RichText::new(&k).color(c.dim));
                    let r = ui.add(egui::Label::new(&v).wrap());
                    ito_a11y::say(&r, Role::Label, &format!("{k}: {v}"));
                    ui.end_row();
                }
            });
        }
        ui.add_space(8.0);
        let title = match &watched {
            Some(j) => format!("{}: {j}", t.tr("run.log")),
            None => t.tr("run.log").to_owned(),
        };
        let r = ui.label(egui::RichText::new(&title).strong());
        ito_a11y::say(&r, Role::Heading, &title);
        let text = match (&self.log, &watched) {
            (Some((r, j, text)), Some(w)) if *r == run.id && j == w => text.clone(),
            _ => String::new(),
        };
        if text.is_empty() {
            ui.label(egui::RichText::new(t.tr("run.log_none")).color(c.dim));
        } else {
            ui.add(egui::TextEdit::singleline(&mut self.log_find).hint_text(t.tr("run.log_find")).desired_width(240.0));
            let text = match show::filter_log(&text, &self.log_find) {
                Ok(None) => text,
                Ok(Some((hits, shown, all))) => {
                    ui.label(egui::RichText::new(t.trf("run.log_hits", &[&shown.to_string(), &all.to_string()])).color(c.dim));
                    hits
                }
                Err(e) => {
                    ui.label(egui::RichText::new(format!("{} {e}", ito_a11y::State::Error.mark())).color(c.err));
                    text
                }
            };
            egui::ScrollArea::vertical().id_salt("log").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                let r = ui.add(egui::Label::new(egui::RichText::new(&text).monospace()).wrap());
                ito_a11y::say(&r, Role::Label, &text);
            });
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(def) = self.selected_def().cloned() else { return };
        let held = self.snap.as_ref().is_some_and(|s| s.held.contains(&def.id));
        let head = review::row(t, &def, held);
        ui.heading(&head);
        ui.label(egui::RichText::new(&def.file).color(c.dim));
        ui.add_space(6.0);
        for p in &def.problems {
            let line = p.in_file(&def.file);
            ui.label(egui::RichText::new(format!("{} {line}", ito_a11y::State::Error.mark())).color(c.err));
        }
        for l in review::body(t, &def) {
            ui.label(l);
        }
        if let Some(net) = &def.net {
            ui.add_space(6.0);
            if net.when.is_empty() {
                ui.label(egui::RichText::new(t.tr("run.next_none")).color(c.dim));
            } else {
                let head = t.trf("run.next", &["10"]);
                let r = ui.label(egui::RichText::new(&head).strong().color(c.dim));
                ito_a11y::say(&r, Role::Heading, &head);
                if def.standing != Standing::Confirmed {
                    ui.label(egui::RichText::new(t.tr("run.next_unconfirmed")).color(c.wait_text()));
                }
                let cal = self.snap.as_ref().and_then(|s| def.calendar.as_deref().and_then(|c| s.calendars.get(c)));
                let rows = show::next_rows(t, &net.when, cal, chrono::Local::now().naive_local(), 10);
                if rows.is_empty() {
                    ui.label(egui::RichText::new(t.tr("run.next_over")).color(c.dim));
                }
                for row in rows {
                    ui.label(format!("  {row}"));
                }
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can = def.standing == Standing::Unconfirmed && !self.blocked();
            let label = match &self.easy {
                Some(lang) => format!("{}…", crate::glossary::GLOSSARY.label("approve", lang, true, "")),
                None => t.tr("confirm.approve").to_owned(),
            };
            let r = ui.add_enabled(can, egui::Button::new(&label));
            ito_a11y::say(&r, Role::Button, &label);
            if let Some(lang) = &self.easy {
                ito_glossary::hover(ui, ui.painter(), r.rect, &crate::glossary::GLOSSARY, "approve", lang, "confirm");
            }
            if r.clicked() {
                self.asking = Some(Approval { id: def.id.clone(), sha256: def.sha256.clone() });
                self.focus = 0;
            }
            let r = ui.add_enabled(def.net.is_some(), egui::Button::new(t.tr("canvas.edit")));
            ito_a11y::say(&r, Role::Button, t.tr("canvas.edit"));
            if r.clicked() {
                self.open_editor(&def.id);
                self.focus = 0;
            }
            let can = def.standing == Standing::Confirmed && !self.blocked();
            let r = ui.add_enabled(can && !held, egui::Button::new(t.tr("run.now")));
            ito_a11y::say(&r, Role::Button, t.tr("run.now"));
            if r.clicked() {
                let _ = self.tx.send(Request::Run { id: def.id.clone(), date: None, from: None, dry: false });
            }
            // On hold: its times are written down as ⏸ and nothing runs until it is released.
            let key = if held { "run.release" } else { "run.hold" };
            let r = ui.add_enabled(!self.blocked(), egui::Button::new(t.tr(key)));
            ito_a11y::say(&r, Role::Button, t.tr(key));
            if r.clicked() {
                let id = def.id.clone();
                let _ = self.tx.send(if held { Request::Release { id } } else { Request::Hold { id } });
            }
        });
        if held {
            let text = format!("{} {}", State::Held.mark(), t.tr("run.held_note"));
            let r = ui.label(egui::RichText::new(&text).color(graph::colour(&c, State::Held)));
            ito_a11y::say(&r, Role::Label, &text);
        }
        self.show_result(ui);
    }

    /// The engine's answer to the last request (approve, run, rerun), under the buttons.
    fn show_result(&self, ui: &mut egui::Ui) {
        let c = ito_theme::colors();
        match &self.result {
            Some(Ok(m)) => {
                ui.label(egui::RichText::new(format!("{} {m}", ito_a11y::State::Done.mark())).color(c.done));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(format!("{} {e}", ito_a11y::State::Error.mark())).color(c.err));
            }
            None => {}
        }
    }

    /// The confirm boxes' colours, from the theme.
    pub(crate) fn style() -> ito_confirm::Style {
        let c = ito_theme::colors();
        ito_confirm::Style {
            panel: c.raised(),
            border: c.border,
            accent: Some(c.wait),
            title: c.strong(),
            text: c.fg,
            dim: c.dim,
            warn: c.wait_text(),
            button_bg: c.chosen(),
            button_text: c.fg,
        }
    }

    /// The box that shows what would run, with the change since the last approval.
    fn confirm(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(ask) = &self.asking else { return };
        let Some(def) = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == ask.id)).cloned() else {
            self.asking = None;
            return;
        };
        // What was looked at must be what is approved: a change since closes the box.
        if def.sha256 != ask.sha256 || def.standing != Standing::Unconfirmed || self.blocked() {
            self.asking = None;
            return;
        }
        let mut body = review::body(t, &def);
        let cal = self.snap.as_ref().and_then(|s| def.calendar.as_deref().and_then(|c| s.calendars.get(c)));
        let week = review::week(t, &def, cal, chrono::Local::now().naive_local());
        if !week.is_empty() {
            body.push(String::new());
            body.extend(week);
        }
        let diff = review::diff(&def);
        let rows = [(t.tr("review.file").to_owned(), def.file.clone())];
        let buttons = [t.tr("confirm.yes").to_owned(), t.tr("confirm.no").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("confirm.title"),
            rows: &rows,
            body: &body,
            diff: &diff,
            buttons: &buttons,
            hint: t.tr("confirm.hint"),
            focus: Some(self.focus),
            primary: Some(0),
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &Self::style()).or(key) {
            Some(0) => {
                let (id, sha256) = (ask.id.clone(), ask.sha256.clone());
                let _ = self.tx.send(Request::Approve { id, sha256 });
                self.asking = None;
            }
            Some(_) => self.asking = None,
            None => {}
        }
    }

    /// The box that asks before a run is killed. A run that ended meanwhile closes it.
    fn confirm_kill(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(id) = self.killing.clone() else { return };
        let Some(run) = self.runs.iter().find(|r| r.id == id && !r.is_over()).cloned() else {
            self.killing = None;
            return;
        };
        let rows = [(t.tr("run.state").to_owned(), show::run_line(t, &run))];
        let body = [t.trf("run.kill_ask", &[&run.title])];
        let buttons = [t.tr("run.kill_yes").to_owned(), t.tr("run.kill_no").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("run.kill"),
            rows: &rows,
            body: &body,
            buttons: &buttons,
            hint: t.tr("confirm.hint"),
            focus: Some(self.focus),
            primary: Some(0),
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &Self::style()).or(key) {
            Some(0) => {
                let _ = self.tx.send(Request::Kill { run: id, job: None });
                self.killing = None;
            }
            Some(_) => self.killing = None,
            None => {}
        }
    }
}

/// Everything in the settings folder, as one file under `exports/` in the
/// state folder: its path and how many things it carries.
fn export_all() -> Result<(String, usize), String> {
    let paths = engine::Paths::find()?;
    let src = koyomi_core::bundle::Source::read(&paths.app);
    let b = koyomi_core::bundle::export(&src, &koyomi_core::bundle::Pick { all: true, ..Default::default() })?;
    if b.is_empty() {
        return Err("書き出すものがない".to_owned());
    }
    let dir = paths.state.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} を作れない: {e}", dir.display()))?;
    let path = dir.join(format!("{}.toml", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    ito_common::write_atomic(&path, b.to_toml())?;
    Ok((path.display().to_string(), b.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::approved::Store;
    use koyomi_core::scan::read_def;

    const NET: &str = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";

    fn defs() -> Vec<Def> {
        // A folder per call: the tests run side by side, and one that reads a
        // file another is still writing would get a def that is not Unconfirmed.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("koyomi-view-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        ["bom", "バックアップ", "月次締め"]
            .iter()
            .map(|n| {
                let p = dir.join(format!("{n}.toml"));
                std::fs::write(&p, NET).unwrap();
                read_def(&p, "試験", n, &Store::default())
            })
            .collect()
    }

    #[test]
    fn a_new_id_needs_a_free_name_a_file_can_hold() {
        let ids = ["試験/bom", "試験/月次締め"];
        let id = |g: &str, n: &str| new_id(g, n, ids.iter().copied());
        assert_eq!(id(" 経理 ", " 締め "), Ok("経理/締め".into()));
        assert_eq!(id("", "x"), Err(Why::Empty));
        assert_eq!(id("経理", "  "), Err(Why::Empty));
        assert_eq!(id("経理", "a/b"), Err(Why::Bad));
        assert_eq!(id("経理", ".."), Err(Why::Bad));
        assert_eq!(id("経理", "a:b"), Err(Why::Bad));
        assert_eq!(id("経理", "締め."), Err(Why::Bad));
        assert_eq!(id("試験", "BOM"), Err(Why::Exists("試験/BOM".into())));
        assert_eq!(id("試験", "月次締め"), Err(Why::Exists("試験/月次締め".into())));
    }

    #[test]
    fn a_new_net_is_unsaved_and_can_be_saved() {
        let net = koyomi_core::jobnet::parse(&koyomi_core::write::template("締め")).unwrap();
        let e = canvas::Editor::new_net("経理/締め", net);
        assert!(e.dirty && e.can_save() && e.base.is_none());
        assert!(e.report().ends_with("new=true"), "{}", e.report());
    }

    /// A browser on a fixed list with no engine behind it.
    #[test]
    fn the_history_narrows_by_net_and_state_newest_first() {
        use koyomi_core::run::How;
        let net = koyomi_core::jobnet::Jobnet::default();
        let at = |d: u32, h: u32| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap().and_hms_opt(h, 0, 0).unwrap();
        let run = |id: &str, jobnet: &str, at, state| {
            let mut r = Run::new(id, jobnet, "t", "2026-10-28", How::Manual, &net, at);
            r.state = state;
            r
        };
        let mut h = History::week(chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap());
        assert_eq!((h.from.as_str(), h.to.as_str()), ("2026-10-23", "2026-10-30"));
        assert!(matches!(h.request(), Request::History { from, to } if from == "2026-10-23" && to == "2026-10-30"));
        h.rows = vec![
            run("r1", "g/a", at(28, 9), State::Ok),
            run("r2", "g/b", at(29, 9), State::Error),
            run("r3", "g/a", at(29, 10), State::Ok),
        ];
        let ids = |h: &History| h.filtered().iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&h), ["r3", "r2", "r1"]);
        h.net = "g/a".into();
        assert_eq!(ids(&h), ["r3", "r1"]);
        h.net.clear();
        h.state = Some(State::Error);
        assert_eq!(ids(&h), ["r2"]);
        h.state = Some(State::Held);
        assert!(ids(&h).is_empty());
    }

    fn quiet(defs: Vec<Def>) -> Browser {
        let (_, rx) = mpsc::channel();
        let (tx, _) = mpsc::channel();
        let snap = Some(Snapshot { defs, ..Snapshot::default() });
        Browser {
            rx,
            tx,
            watch: Arc::default(),
            snap,
            runs: Vec::new(),
            down: None,
            starting: false,
            selected: None,
            run_sel: None,
            job_sel: None,
            log_find: String::new(),
            log: None,
            vertical: false,
            flow: None,
            asking: None,
            killing: None,
            focus: 0,
            close_asked: false,
            result: None,
            over: None,
            ended: Vec::new(),
            seen_notes: BTreeSet::new(),
            notes: Vec::new(),
            autostart_off: false,
            cal_sel: None,
            cal_month: None,
            cal_result: None,
            hist: None,
            editor: None,
            editors: Vec::new(),
            port: None,
            make: None,
            easy: None,
        }
    }

    #[test]
    fn an_empty_tree_is_known_only_after_the_engine_answered() {
        let b = quiet(Vec::new());
        assert!(b.snapshot_known() && b.tree_is_empty());
        assert!(!quiet(defs()).tree_is_empty());
    }

    /// A browser asking about `bom`.
    fn asking() -> Browser {
        let defs = defs();
        let ask = Approval { id: defs[0].id.clone(), sha256: defs[0].sha256.clone() };
        let selected = Some(defs[0].id.clone());
        let mut b = quiet(defs);
        b.selected = selected;
        b.asking = Some(ask);
        b
    }

    fn frame(ctx: &egui::Context, size: egui::Vec2, events: Vec<egui::Event>, b: &mut Browser) {
        let t = crate::i18n::table("ja").0;
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| b.show(ui, &t));
        out.textures_delta.clear();
    }

    fn key(k: egui::Key) -> Vec<egui::Event> {
        let e = |pressed| egui::Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        vec![e(true), e(false)]
    }

    /// The box's title, rows and buttons are on the screen, however many frames it is up.
    #[test]
    fn the_confirm_box_stays_inside_the_window() {
        for size in [egui::vec2(1280.0, 800.0), egui::vec2(640.0, 360.0)] {
            let ctx = egui::Context::default();
            let mut b = asking();
            for _ in 0..30 {
                frame(&ctx, size, Vec::new(), &mut b);
            }
            let r = ctx.memory(|m| m.area_rect(egui::Id::new("ito-confirm"))).expect("drawn");
            assert!(r.top() >= 0.0 && r.bottom() <= size.y, "{size:?}: {r:?}");
        }
    }

    /// A box after a taller one is as tall as what it holds.
    #[test]
    fn a_short_box_after_a_tall_one_is_short() {
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        let mut b = asking();
        let height = |ctx: &egui::Context| ctx.memory(|m| m.area_rect(egui::Id::new("ito-confirm"))).expect("drawn").height();
        for _ in 0..3 {
            frame(&ctx, size, Vec::new(), &mut b);
        }
        let short = height(&ctx);
        let long: String = (0..40)
            .map(|i| {
                format!(
                    "# line {i}
"
                )
            })
            .collect();
        b.snap.as_mut().unwrap().defs[0].previous = Some(long);
        for _ in 0..3 {
            frame(&ctx, size, Vec::new(), &mut b);
        }
        let tall = height(&ctx);
        b.snap.as_mut().unwrap().defs[0].previous = None;
        for _ in 0..10 {
            frame(&ctx, size, Vec::new(), &mut b);
        }
        assert!(height(&ctx) <= short + 1.0, "short {short} tall {tall} again {}", height(&ctx));
    }

    /// 4.5: Tab and Enter answer the box and reach nothing behind it.
    #[test]
    fn keys_do_not_reach_the_tree_behind_the_box() {
        let ctx = egui::Context::default();
        let mut b = asking();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        assert!(b.asking.is_some(), "the box is up");
        frame(&ctx, size, key(egui::Key::Tab), &mut b);
        assert_eq!(b.focus, 1);
        assert!(ctx.memory(|m| m.focused()).is_none(), "nothing behind the box takes the focus");
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(b.asking.is_none(), "Enter on the cancel button closes the box");
        assert_eq!(b.selected.as_deref(), Some("試験/bom"), "the tree did not pick another row");
    }

    /// A run of today that is still going draws, with its picture and its jobs, and a frame later too.
    #[test]
    fn a_running_run_draws_its_picture_and_watches_the_running_job() {
        use koyomi_core::jobnet::{Job, Jobnet, Line};
        use koyomi_core::run::{How, Outcome};
        let job = |id: &str, after: &[&str]| Job {
            id: id.into(),
            run: vec!["x".into()],
            after: after.iter().map(|s| Line::ok(s)).collect(),
            ..Default::default()
        };
        let net = Jobnet { name: None, jobs: vec![job("a", &[]), job("b", &["a"])], when: Vec::new(), ..Default::default() };
        let now = chrono::Local::now().naive_local();
        let mut run = Run::new("20261030-090000-01", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.start("a", now);
        run.finish("a", Outcome::Exited(0), now);
        run.start("b", now);
        let mut b = quiet(defs());
        b.runs = vec![run];
        b.run_sel = Some("20261030-090000-01".into());
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        for _ in 0..3 {
            frame(&ctx, size, Vec::new(), &mut b);
        }
        let watched = b.watch.lock().unwrap().clone();
        assert_eq!(watched, Some(("20261030-090000-01".into(), "b".into())), "the running job's log is the one fetched");
        assert!(b.killing.is_none());
    }

    /// The kill box starts on the safe button: a plain Enter does not kill.
    #[test]
    fn the_kill_box_starts_on_no() {
        use koyomi_core::jobnet::{Job, Jobnet};
        use koyomi_core::run::How;
        let net = Jobnet {
            name: None,
            jobs: vec![Job { id: "a".into(), run: vec!["x".into()], ..Default::default() }],
            when: Vec::new(),
            ..Default::default()
        };
        let now = chrono::Local::now().naive_local();
        let mut run = Run::new("r1", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.start("a", now);
        let (tx, rx_req) = mpsc::channel();
        let mut b = quiet(defs());
        b.tx = tx;
        b.runs = vec![run];
        b.run_sel = Some("r1".into());
        b.killing = Some("r1".into());
        b.focus = 1;
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(b.killing.is_none(), "Enter answered the box");
        assert!(rx_req.try_recv().is_err(), "and sent no kill");
    }

    /// The editor opens from a definition, reports itself, and asks before changes are dropped.
    #[test]
    fn the_editor_asks_before_dropping_changes() {
        let mut b = quiet(defs());
        let id = defs()[0].id.clone();
        b.open_editor(&id);
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        assert!(b.editing());
        assert!(b.report().contains(&format!("editor: {id} dirty=false")), "{}", b.report());
        // Clean: closes at once.
        b.edit(canvas::Cmd::Close);
        assert!(!b.editing());
        // Dirty: the close asks, and Escape keeps the editor.
        b.open_editor(&id);
        b.editor.as_mut().unwrap().add(koyomi_core::jobnet::Part::Notify, Some("a"), None);
        assert!(b.report().contains("dirty=true"), "{}", b.report());
        b.focus = 0;
        b.edit(canvas::Cmd::Close);
        assert!(b.editing());
        assert!(b.editor.as_ref().unwrap().closing);
        frame(&ctx, size, Vec::new(), &mut b);
        assert_eq!(b.focus, 1, "the close question starts on やめる");
        frame(&ctx, size, key(egui::Key::Escape), &mut b);
        assert!(b.editing());
        assert!(!b.editor.as_ref().unwrap().closing);
        // A click in the tree asks the same way; a save goes to the engine.
        b.leave_editor();
        assert!(b.editor.as_ref().unwrap().closing);
        b.editor.as_mut().unwrap().closing = false;
        let (tx, rx_req) = mpsc::channel();
        b.tx = tx;
        b.edit(canvas::Cmd::Save);
        match rx_req.try_recv() {
            Ok(Request::Save { id: saved, force, .. }) => {
                assert_eq!(saved, id);
                assert!(!force);
            }
            other => panic!("{other:?}"),
        }
    }
}
