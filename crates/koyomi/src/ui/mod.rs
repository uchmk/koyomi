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
pub mod gantt;
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
    /// File work done off the UI thread: a line for under the detail, or why not.
    Done(Result<Said, Said>),
    /// The file the 取り込む box was told to read: its text, or why it could not be read.
    PortText(String, Result<String, String>),
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
    /// The net the box renames or copies; `None` makes a new one.
    from: Option<(String, Re)>,
    /// 参照もいっしょに直す, for a rename of a net others name.
    fix: bool,
    /// A copy's 表示名 (`name =`); empty writes none.
    title: String,
    /// The 表示名 follows the file name as it is typed: the source's was its file name, or empty.
    follows: bool,
}

/// What the 新しいジョブネット box does to the net it was opened on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Re {
    Rename,
    Copy,
}

/// The box 今すぐ実行 opens for a net with `[params]`: each value, its default filled in.
struct Give {
    id: String,
    /// Name, default, what the field holds.
    values: Vec<(String, String, String)>,
    /// The first field takes the keyboard on the first frame.
    fresh: bool,
}

impl Give {
    /// The values that differ from their default: what the run is sent with.
    fn set(&self) -> BTreeMap<String, String> {
        self.values.iter().filter(|(_, d, v)| v != d).map(|(k, _, v)| (k.clone(), v.clone())).collect()
    }
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
    let id = format!("{g}/{n}");
    if bad_file_name(g) || bad_file_name(n) || !koyomi_core::jobnet::is_id(&id) {
        return Err(Why::Bad);
    }
    let low = id.to_lowercase();
    if ids.any(|i| i.to_lowercase() == low) {
        return Err(Why::Exists(id));
    }
    Ok(id)
}

/// What a file name on Windows cannot hold, and a trailing dot it would cut off.
fn bad_file_name(s: &str) -> bool {
    s.ends_with('.') || s.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
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

/// The 版を比べる box: the versions kept of a net, the one picked (0 is the newest) and the texts fetched so far.
struct Vers {
    id: String,
    list: Vec<String>,
    idx: usize,
    texts: BTreeMap<String, String>,
    asked: BTreeSet<String>,
    error: Option<String>,
}

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

/// What a thread off the UI thread tells: the key of the words and what goes into them, formatted
/// where the language table is.
type Said = (&'static str, Vec<String>);

pub struct Browser {
    rx: Receiver<Msg>,
    /// Where threads started by [`Browser::background`] report, and the window to wake.
    io: (Sender<Msg>, egui::Context),
    /// What [`Msg::Done`] brought, until [`Browser::show`] has the words for it.
    done: Option<Result<Said, Said>>,
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
    /// The run marked to be compared with: the run picked next is shown against it.
    compare_base: Option<String>,
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
    /// The net a delete is being asked about.
    deleting: Option<String>,
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
    /// The runs 対応済みにする was pressed for, until [`Browser::take_acks`].
    to_ack: Vec<String>,
    /// The runs already dealt with, as the app keeps them: the button says so.
    acked: BTreeSet<String>,
    /// The engine's notes about `[notify]` seen so far, and the ones not yet taken.
    seen_notes: BTreeSet<String>,
    notes: Vec<String>,
    /// A net runs on a schedule, but koyomi does not start at sign-in: the band says so.
    autostart_off: bool,
    /// The lines the cautions band held last frame (`None` until the first snapshot): it opens
    /// itself for a line that was not there, not for the ones already there at start.
    band_seen: Option<BTreeSet<String>>,
    /// The calendar picked on the left; when it is set, the detail is its month.
    cal_sel: Option<String>,
    /// Groups folded in the tree (this session only).
    folded: BTreeSet<String>,
    /// The selection the tree last showed: a new one unfolds its group.
    tree_seen: Option<String>,
    /// The month shown, when it is not this one.
    cal_month: Option<(i32, u32)>,
    /// What the last click on a day did to the file, shown under the month.
    cal_result: Option<Result<String, String>>,
    /// A day's name or a holiday table's path being typed under the month.
    cal_inline: Option<calendar::Inline>,
    /// The runs of the last 30 days, for the average time of a net, and the day they were asked for.
    past: std::collections::HashMap<String, chrono::Duration>,
    past_asked: Option<chrono::NaiveDate>,
    /// When the last ask failed: it is not repeated at every snapshot.
    past_failed: Option<std::time::Instant>,
    /// 履歴, when it is open: the detail is its rows.
    hist: Option<History>,
    /// ガント, when it is open: the detail is today's runs on a time axis.
    gantt: bool,
    /// The 版を比べる box, while it is open.
    vers: Option<Vers>,
    /// The editor, when a net is open in it: the detail is its canvas.
    editor: Option<canvas::Editor>,
    /// The 取り込む box, while it is open.
    port: Option<Port>,
    /// The 新しいジョブネット box, while it is open.
    make: Option<Make>,
    /// The 新しいカレンダー box.
    new_cal: Option<calendar::NewCal>,
    /// The calendar the カレンダーを消す box asks about.
    cal_deleting: Option<String>,
    /// The values box of 今すぐ実行, while it is open.
    give: Option<Give>,
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
        let io = (tx_msg.clone(), ctx.clone());
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
            io,
            tx,
            watch,
            snap: None,
            runs: Vec::new(),
            past: std::collections::HashMap::new(),
            past_asked: None,
            past_failed: None,
            down: None,
            starting: true,
            selected: None,
            run_sel: None,
            compare_base: None,
            job_sel: None,
            log_find: String::new(),
            log: None,
            vertical,
            flow,
            asking: None,
            killing: None,
            deleting: None,
            focus: 0,
            close_asked: false,
            result: None,
            over: None,
            ended: Vec::new(),
            to_ack: Vec::new(),
            acked: BTreeSet::new(),
            done: None,
            seen_notes: BTreeSet::new(),
            notes: Vec::new(),
            autostart_off: false,
            band_seen: None,
            cal_sel: None,
            folded: BTreeSet::new(),
            tree_seen: None,
            cal_month: None,
            cal_result: None,
            cal_inline: None,
            hist: None,
            gantt: true,
            vers: None,
            editor: None,
            editors: Vec::new(),
            port: None,
            make: None,
            new_cal: None,
            cal_deleting: None,
            give: None,
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

    /// The days the averages are made from: the last 30 days. `to` is tomorrow, so that it
    /// cannot be the range the 履歴 box asks for.
    fn past_range(today: chrono::NaiveDate) -> (String, String) {
        ((today - chrono::Duration::days(30)).to_string(), (today + chrono::Duration::days(1)).to_string())
    }

    /// Asks the engine for the last 30 days once a day.
    fn ask_past(&mut self) {
        let today = chrono::Local::now().date_naive();
        if self.past_asked != Some(today) && self.past_failed.is_none_or(|at| at.elapsed() > Duration::from_secs(60)) {
            self.past_asked = Some(today);
            let (from, to) = Self::past_range(today);
            let _ = self.tx.send(Request::History { from, to });
        }
    }

    pub fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Starting => self.starting = true,
                Msg::Snapshot(s) => {
                    self.ask_past();
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
                Msg::Done(r) => self.done = Some(r),
                Msg::PortText(path, r) => self.port_text(&path, r),
                Msg::Down(e) => {
                    self.starting = false;
                    self.down = Some(e);
                }
                Msg::Answer(Request::History { from, to }, r)
                    if self.past_asked.is_some_and(|d| Self::past_range(d) == (from.clone(), to.clone())) =>
                {
                    match r.and_then(|text| Runs::parse(&text)) {
                        Ok(rs) => {
                            self.past_failed = None;
                            // Averaged once here, not at every frame.
                            let nets: BTreeSet<&str> = rs.runs.iter().map(|r| r.jobnet.as_str()).collect();
                            self.past = nets
                                .into_iter()
                                .filter_map(|n| koyomi_core::run::average_duration(&rs.runs, n, 10).map(|a| (n.to_owned(), a)))
                                .collect();
                            // The 履歴 box may have been typed to this very range: it gets the rows too.
                            if let Some(h) = self.hist.as_mut().filter(|h| h.from == from && h.to == to) {
                                h.rows = rs.runs;
                                h.error = None;
                            }
                        }
                        // Asked again with a later snapshot, not at once.
                        Err(e) => {
                            self.past_asked = None;
                            self.past_failed = Some(std::time::Instant::now());
                            if let Some(h) = self.hist.as_mut().filter(|h| h.from == from && h.to == to) {
                                h.rows.clear();
                                h.error = Some(e);
                            }
                        }
                    }
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
                Msg::Answer(Request::Versions { id }, r) => {
                    if let Some(v) = self.vers.as_mut().filter(|v| v.id == id) {
                        match r.and_then(|text| proto::Versions::parse(&text)) {
                            Ok(l) if l.versions.is_empty() => v.error = Some("no versions kept".to_owned()),
                            Ok(l) => v.list = l.versions,
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::Answer(Request::VersionText { id, version }, r) => {
                    if let Some(v) = self.vers.as_mut().filter(|v| v.id == id) {
                        match r {
                            Ok(text) => {
                                v.texts.insert(version, text);
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::Answer(req, r) => {
                    // A start that went through opens the run.
                    if let (Request::Run { .. } | Request::Rerun { .. }, Ok(id)) = (&req, &r) {
                        self.run_sel = Some(id.clone());
                        self.job_sel = None;
                    }
                    // The tree's pick follows a net that moved, and lets go of one that went.
                    match (&req, &r) {
                        (Request::Rename { id, to, .. }, Ok(_)) if self.selected.as_ref() == Some(id) => self.selected = Some(to.clone()),
                        (Request::Delete { id }, Ok(_)) if self.selected.as_ref() == Some(id) => self.selected = None,
                        _ => {}
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

    /// How many definitions are not approved or do not read (the status bar's count).
    pub fn todo_count(&self) -> usize {
        self.snap.as_ref().map_or(0, |s| s.defs.iter().filter(|d| d.is_todo()).count())
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
                koyomi_core::quiet::next_starts(&net.when, &net.quiet, net.on_quiet, now, 1, cal).into_iter().next()
            })
            .min();
        (word, running, next)
    }

    /// The runs that ended since the last call, each once (the bell).
    pub fn take_ended(&mut self) -> Vec<Run> {
        self.drain();
        std::mem::take(&mut self.ended)
    }

    /// The runs already dealt with, as the app keeps them (from `Acked::take_changed`).
    pub fn set_acked(&mut self, ids: BTreeSet<String>) {
        self.acked = ids;
    }

    /// The runs the person marked 対応済み since the last call (the app keeps them in `acked.toml`).
    pub fn take_acks(&mut self) -> Vec<String> {
        std::mem::take(&mut self.to_ack)
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
        self.gantt = false;
        self.leave_editor();
    }

    /// Whether a box is up. The keys and `self.focus` belong to one box at a time, so another is not
    /// opened over it: an Enter meant for the new one would press a button of the old one.
    fn box_open(&self) -> bool {
        self.asking.is_some()
            || self.killing.is_some()
            || self.deleting.is_some()
            || self.vers.is_some()
            || self.port.is_some()
            || self.make.is_some()
            || self.new_cal.is_some()
            || self.cal_deleting.is_some()
            || self.give.is_some()
            || self.editor.as_ref().is_some_and(|e| e.conflict.is_some() || e.closing)
    }

    fn blocked(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| !s.warnings.is_empty())
    }

    fn selected_def(&self) -> Option<&Def> {
        let id = self.selected.as_deref()?;
        self.snap.as_ref()?.defs.iter().find(|d| d.id == id)
    }

    fn selected_run(&self) -> Option<&Run> {
        self.find_run(self.run_sel.as_deref()?)
    }

    /// The run `id`, from the runs of today or the history asked for.
    fn find_run(&self, id: &str) -> Option<&Run> {
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
        if let Some(g) = &self.give {
            let vals: Vec<String> = g.values.iter().map(|(k, _, v)| format!("{k}={v}")).collect();
            out.push_str(&format!("give: {} {}\n", g.id, vals.join(" ")));
        }
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
        self.result = Some(Ok(t.tr("run.writing").to_owned()));
        Self::background(&self.io, move || export_all().map(|(path, n)| ("port.exported", vec![path, n.to_string()])));
    }

    /// The palette's 取り込む: opens the box with the path field.
    pub fn import_box(&mut self) {
        if !self.box_open() {
            self.port = Some(Port { path: String::new(), text: None, plan: None, error: None, waiting: false, fresh: true });
            self.focus = 0;
        }
    }

    /// 新しいジョブネット: opens the box, with `group` filled in when it came from a group in the tree.
    pub fn new_box(&mut self, group: Option<&str>) {
        if !self.box_open() && self.snap.is_some() {
            self.make = Some(Make {
                group: group.unwrap_or_default().to_owned(),
                name: String::new(),
                error: None,
                fresh: true,
                from: None,
                fix: false,
                title: String::new(),
                follows: false,
            });
            self.focus = 0;
        }
    }

    /// 名前を変える・移す and 複製: the same box, with net `id`'s group and name filled in.
    fn re_box(&mut self, id: &str, re: Re) {
        let Some((group, name)) = id.split_once('/') else { return };
        if !self.box_open() && self.editor.is_none() {
            let name = if re == Re::Copy { format!("{name}-2") } else { name.to_owned() };
            // The source's own title: a copy keeps a custom one, and follows the file name otherwise.
            let own = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == id)).and_then(|d| d.title.clone());
            let follows = own.as_deref().is_none_or(|t| t.is_empty() || t == id.split_once('/').map_or("", |x| x.1));
            let title = if follows { name.clone() } else { own.unwrap_or_default() };
            self.make = Some(Make {
                group: group.to_owned(),
                name,
                error: None,
                fresh: true,
                from: Some((id.to_owned(), re)),
                fix: false,
                title,
                follows,
            });
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
            Ok(to) if m.from.is_some() => {
                // A rename or a copy: the engine does it, and its answer is the result line.
                let (id, re) = m.from.clone().expect("checked");
                let req = match re {
                    Re::Rename => Request::Rename { id, to, fix: m.fix },
                    // Still following the file name: leave it to the engine, so a file without a `name =` gains none.
                    Re::Copy => Request::Copy { id, to, title: (!m.follows).then(|| m.title.clone()) },
                };
                self.make = None;
                self.result = None;
                let _ = self.tx.send(req);
                return;
            }
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
                self.gantt = false;
            }
        }
    }

    /// The 新しいジョブネット box: a group (typed, or one there is) and a name.
    fn confirm_make(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let mut groups: Vec<String> = self.snap.iter().flat_map(|s| s.defs.iter().map(|d| d.group.clone())).collect();
        groups.dedup();
        let from = self.make.as_ref().and_then(|m| m.from.clone());
        // The nets that name the one renamed: the rename moves them along only when asked.
        let users = match (&from, &self.snap) {
            (Some((id, Re::Rename)), Some(s)) => koyomi_core::rename::referrers(&s.defs, id),
            _ => Vec::new(),
        };
        let Some(m) = &mut self.make else { return };
        let c = ito_theme::colors();
        let mut pressed = None;
        let (title, lead, go) = match &from {
            None => (t.tr("make.title").to_owned(), t.tr("make.lead").to_owned(), t.tr("make.go")),
            Some((id, Re::Rename)) => (t.tr("rename.title").to_owned(), t.trf("rename.lead", &[id]), t.tr("rename.go")),
            Some((id, Re::Copy)) => (t.tr("rename.copy_title").to_owned(), t.trf("rename.copy_lead", &[id]), t.tr("rename.copy_go")),
        };
        egui::Window::new(title)
            .id(egui::Id::new("make"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.add(egui::Label::new(lead).wrap());
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
                    if matches!(from, Some((_, Re::Copy))) {
                        if m.follows {
                            m.title = m.name.trim().to_owned();
                        }
                        ui.label(t.tr("make.shown"));
                        let r = ui.add(egui::TextEdit::singleline(&mut m.title).hint_text(t.tr("make.shown_hint")).desired_width(240.0));
                        ito_a11y::say(&r, Role::Label, t.tr("make.shown"));
                        if r.changed() {
                            m.follows = false;
                        }
                        ui.end_row();
                    }
                });
                if !users.is_empty() {
                    let text = t.trf("rename.users", &[&users.join(", ")]);
                    ui.add(egui::Label::new(egui::RichText::new(&text).color(c.dim)).wrap());
                    let label = t.tr("rename.fix");
                    // egui names a checkbox for the screen reader itself.
                    ui.checkbox(&mut m.fix, label);
                }
                if let Some(e) = &m.error {
                    ui.add(egui::Label::new(egui::RichText::new(e).color(c.err)).wrap());
                }
                ui.horizontal(|ui| {
                    for (i, label) in [go, t.tr("make.cancel")].into_iter().enumerate() {
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

    /// 今すぐ実行: starts the net, or first asks for its `[params]` when it has any.
    fn run_now(&mut self, id: String, params: Option<&BTreeMap<String, String>>) {
        if self.box_open() {
            return;
        }
        match params.filter(|p| !p.is_empty()) {
            Some(p) => {
                let values = p.iter().map(|(k, d)| (k.clone(), d.clone(), d.clone())).collect();
                self.give = Some(Give { id, values, fresh: true });
                self.focus = 0;
            }
            None => {
                let _ = self.tx.send(Request::Run { id, date: None, from: None, dry: false, params: BTreeMap::new() });
            }
        }
    }

    /// The values box of 今すぐ実行: one field a value, then 流す or やめる.
    fn confirm_give(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(g) = &mut self.give else { return };
        let c = ito_theme::colors();
        let mut pressed = None;
        egui::Window::new(t.tr("give.title"))
            .id(egui::Id::new("give"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.add(egui::Label::new(t.trf("give.lead", &[g.id.as_str()])).wrap());
                egui::Grid::new("give-fields").num_columns(2).show(ui, |ui| {
                    for (i, (name, default, value)) in g.values.iter_mut().enumerate() {
                        ui.label(name.as_str());
                        let r = ui.add(egui::TextEdit::singleline(value).hint_text(default.as_str()).desired_width(300.0));
                        ito_a11y::say(&r, Role::Label, name);
                        if i == 0 && g.fresh {
                            r.request_focus();
                            g.fresh = false;
                        }
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    for (i, label) in [t.tr("give.go"), t.tr("give.cancel")].into_iter().enumerate() {
                        let r = ui.button(label);
                        ito_a11y::say(&r, Role::Button, label);
                        if r.clicked() {
                            pressed = Some(i);
                        }
                    }
                });
                ui.label(egui::RichText::new(t.tr("give.hint")).color(c.dim).small());
            });
        match pressed.or(key) {
            Some(0) => {
                let params = g.set();
                let id = g.id.clone();
                self.give = None;
                let _ = self.tx.send(Request::Run { id, date: None, from: None, dry: false, params });
            }
            Some(_) => self.give = None,
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

    /// Runs file work on its own thread; the line it returns shows under the detail.
    fn background(io: &(Sender<Msg>, egui::Context), work: impl FnOnce() -> Result<Said, Said> + Send + 'static) {
        let (tx, ctx) = io.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Done(work()));
            ctx.request_repaint();
        });
    }

    /// Reads the file in the path field (off the UI thread); [`Browser::port_text`] gets the text.
    fn port_read(&mut self) {
        let Some(p) = &mut self.port else { return };
        let path = p.path.trim().trim_matches('"').to_owned();
        if path.is_empty() {
            return;
        }
        let (tx, ctx) = self.io.clone();
        std::thread::spawn(move || {
            let read = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| crate::cli::port_text(&b).ok_or_else(|| "UTF-8 でも UTF-16 でもない".to_owned()))
                .map_err(|e| format!("{path} を読めない: {e}"));
            let _ = tx.send(Msg::PortText(path, read));
            ctx.request_repaint();
        });
    }

    /// The file the box read: ask the engine what importing it would do.
    /// A read of a path the field no longer holds (or of a box since closed) is dropped.
    fn port_text(&mut self, path: &str, read: Result<String, String>) {
        let Some(p) = &mut self.port else { return };
        if p.path.trim().trim_matches('"') != path {
            return;
        }
        match read {
            Ok(text) => {
                p.error = None;
                p.text = Some(text.clone());
                p.waiting = true;
                self.focus = 0;
                let _ = self.tx.send(Request::Import { text, dry_run: true, on_conflict: None });
            }
            Err(e) => p.error = Some(e),
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
            canvas::Action::NewCalendar => self.new_cal_box_for_net(),
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
        let no_power = crate::proto::Power::default();
        let power = self.snap.as_ref().map(|s| &s.power).unwrap_or(&no_power);
        let depth = self.editors.len();
        let Some(e) = &mut self.editor else { return };
        let action = e.show(ui, t, self.vertical, calendars, power, depth);
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
        if let Some(done) = self.done.take() {
            let words = |(key, args): Said| t.trf(key, &args.iter().map(String::as_str).collect::<Vec<_>>());
            self.result = Some(done.map(words).map_err(words));
        }
        let ctx = ui.ctx().clone();
        let conflict = self.editor.as_ref().is_some_and(|e| e.conflict.is_some()) || self.vers.is_some();
        let port = self.port.as_ref().map(|p| (p.buttons(), p.stage() == Stage::Path));
        let making = self.make.is_some() || self.new_cal.is_some() || self.give.is_some();
        let closing = self.editor.as_ref().is_some_and(|e| e.closing);
        if closing && !self.close_asked {
            // The safe button first: Enter on a fresh box does not throw the changes away.
            self.focus = 1;
        }
        self.close_asked = closing;
        let modal = self.asking.is_some()
            || self.killing.is_some()
            || self.deleting.is_some()
            || self.cal_deleting.is_some()
            || conflict
            || closing
            || port.is_some()
            || making;
        let buttons = if self.vers.is_some() {
            4
        } else if conflict {
            3
        } else {
            port.map_or(2, |(n, _)| n)
        };
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
        // The lesser notes (autostart off; the calendars: a file that does not read, a holiday table
        // near its end) share one band that stays folded: nothing stops for these.
        let cautions: Vec<String> = self.snap.as_ref().map(|s| s.cautions.clone()).unwrap_or_default();
        let notes = usize::from(self.autostart_off) + cautions.len();
        let mut now: BTreeSet<String> = cautions.iter().map(|l| caution_key(l)).collect();
        if self.autostart_off {
            now.insert(t.tr("band.autostart").to_owned());
        }
        let grew = self.band_seen.as_ref().is_some_and(|seen| now.iter().any(|l| !seen.contains(l)));
        if self.snap.is_some() {
            self.band_seen = Some(now);
        }
        if notes > 0 {
            egui::Panel::top("cautions").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                let head = format!("{} {} {notes}", ito_a11y::State::Warning.mark(), t.tr("band.caution"));
                let mut fold = egui::CollapsingHeader::new(egui::RichText::new(head).strong().color(c.wait_text()))
                    .id_salt("cautions_fold")
                    .default_open(false);
                // A new caution opens the band once; after that it is the person's to fold.
                if grew {
                    fold = fold.open(Some(true));
                }
                fold.show(ui, |ui| {
                    let mut lines = Vec::new();
                    if self.autostart_off {
                        lines.push(t.tr("band.autostart").to_owned());
                    }
                    lines.extend(cautions.iter().cloned());
                    for l in &lines {
                        let r = ui.add(egui::Label::new(egui::RichText::new(l).color(c.wait_text())).wrap());
                        ito_a11y::say(&r, Role::Label, l);
                    }
                });
            });
        }
        egui::Panel::left("tree").default_size(200.0).show(ui, |ui| {
            if modal {
                ui.disable();
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.tree(ui, t);
                ui.add_space(10.0);
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
            if self.editor.is_none() {
                self.tabs(ui, t);
                ui.add_space(6.0);
            }
            if self.editor.is_some() {
                self.editor_view(ui, t);
            } else if self.gantt {
                egui::ScrollArea::vertical().id_salt("gantt_page").auto_shrink(false).show(ui, |ui| self.gantt_view(ui, t));
            } else if self.hist.is_some() {
                self.history_view(ui, t);
            } else if self.run_sel.is_some() {
                self.run_view(ui, t);
            } else if self.cal_sel.is_some() {
                self.calendar_view(ui, t);
            } else {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.detail(ui, t));
            }
        });
        // Boxes can overlap (a kill asked while a delete is open); one key answers only the first
        // open one, so a single Enter cannot decide two of them.
        let open = [
            self.asking.is_some(),
            self.killing.is_some(),
            self.deleting.is_some(),
            self.editor.as_ref().is_some_and(|e| e.conflict.is_some()) || closing,
            self.vers.is_some(),
            self.port.is_some(),
            self.make.is_some(),
            self.give.is_some(),
            self.new_cal.is_some(),
            self.cal_deleting.is_some(),
        ];
        let first = open.iter().position(|o| *o);
        let key_for = |i: usize| if first == Some(i) { key } else { None };
        self.confirm(&ctx, t, key_for(0));
        self.confirm_kill(&ctx, t, key_for(1));
        self.confirm_delete(&ctx, t, key_for(2));
        self.confirm_editor(&ctx, t, key_for(3));
        self.confirm_versions(&ctx, t, key_for(4));
        self.confirm_port(&ctx, t, key_for(5));
        self.confirm_make(&ctx, t, key_for(6));
        self.confirm_give(&ctx, t, key_for(7));
        self.confirm_new_cal(&ctx, t, key_for(8));
        self.confirm_cal_delete(&ctx, t, key_for(9));
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
        let mut re: Option<(String, Option<Re>)> = None;
        let editing = self.editor.is_some();
        if self.selected != self.tree_seen {
            // A net just made is not in the snapshot until the next scan: try again then.
            match snap.defs.iter().find(|d| Some(&d.id) == self.selected.as_ref()) {
                Some(d) => {
                    self.folded.remove(&d.group);
                    self.tree_seen.clone_from(&self.selected);
                }
                None if self.selected.is_none() => self.tree_seen = None,
                None => {}
            }
        }
        let mut todo: BTreeMap<&str, usize> = BTreeMap::new();
        for d in snap.defs.iter().filter(|d| d.is_todo()) {
            *todo.entry(d.group.as_str()).or_default() += 1;
        }
        let twins = review::twins(&snap.defs);
        let mut group: Option<&str> = None;
        for d in &snap.defs {
            if group != Some(d.group.as_str()) {
                group = Some(&d.group);
                let mark = if self.folded.contains(&d.group) { "▶" } else { "▼" };
                let todo = todo.get(d.group.as_str()).copied().unwrap_or(0);
                let mut head = egui::text::LayoutJob::default();
                let font = egui::TextStyle::Body.resolve(ui.style());
                let fmt = |color| egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
                head.append(&format!("{mark} {}", d.group), 0.0, fmt(ui.visuals().strong_text_color()));
                if todo > 0 {
                    head.append(&format!("  ✕{todo}"), 0.0, fmt(c.err));
                }
                let r = ui.add(egui::Label::new(head).sense(egui::Sense::click()));
                let state = t.tr(if self.folded.contains(&d.group) { "tree.folded" } else { "tree.open" });
                let said = if todo > 0 {
                    t.trf("tree.group_todo", &[&d.group, state, &t.trf("tree.todo", &[&todo.to_string()])])
                } else {
                    t.trf("tree.group", &[&d.group, state])
                };
                ito_a11y::say(&r, Role::Heading, &said);
                if r.clicked() && !self.folded.remove(&d.group) {
                    self.folded.insert(d.group.clone());
                }
                r.context_menu(|ui| {
                    if ui.button(t.tr("tree.new_here")).clicked() {
                        make_in = Some(d.group.clone());
                        ui.close();
                    }
                });
            }
            if self.folded.contains(&d.group) {
                continue;
            }
            let last = self.runs.iter().filter(|r| r.jobnet == d.id).max_by_key(|r| r.started).map(|r| r.state);
            let mut text = review::row_with(t, d, snap.held.contains(&d.id), last);
            if snap.held_jobs.iter().any(|h| h.net == d.id) {
                text = format!("{text}  {} {}", State::Held.mark(), t.tr("state.job_held"));
            }
            if twins.contains(&d.id) {
                text = review::twin_row(text, d);
            }
            let on = self.run_sel.is_none() && self.hist.is_none() && self.selected.as_deref() == Some(d.id.as_str());
            let r = ui.selectable_label(on, format!("  {text}"));
            // The mark is a picture; the word for the last run's state is what is said.
            let said = match last {
                Some(l) => {
                    format!("{}, {}", review::row(t, d, snap.held.contains(&d.id)), show::tag(t, l))
                        + &if snap.held_jobs.iter().any(|h| h.net == d.id) {
                            format!(", {}", t.tr("state.job_held"))
                        } else {
                            String::new()
                        }
                }
                None => text.clone(),
            };
            ito_a11y::say(&r, Role::Row, &said);
            if r.clicked() {
                pick = Some(d.id.clone());
            }
            r.context_menu(|ui| {
                // A net open in the editor would be saved back under its old name: close it first.
                for (key, what) in [("tree.rename", Some(Re::Rename)), ("tree.copy", Some(Re::Copy)), ("tree.delete", None)] {
                    let r = ui.add_enabled(!editing, egui::Button::new(t.tr(key))).on_disabled_hover_text(t.tr("tree.editing"));
                    if r.clicked() {
                        re = Some((d.id.clone(), what));
                        ui.close();
                    }
                }
            });
        }
        if let Some(g) = make_in {
            self.new_box(Some(&g));
        }
        match re {
            Some((id, Some(what))) => self.re_box(&id, what),
            Some((id, None)) if !self.box_open() => {
                self.deleting = Some(id);
                // The safe button first: Enter on a fresh box does not delete.
                self.focus = 1;
            }
            _ => {}
        }
        if pick.is_some() {
            self.selected = pick;
            self.run_sel = None;
            self.cal_sel = None;
            self.job_sel = None;
            self.result = None;
            self.hist = None;
            self.gantt = false;
            self.leave_editor();
        }
    }

    /// Today as a table: what is still to come (waiting), then the runs newest first, each with its
    /// state, net, planned time, start, end and duration. A click on a run opens it.
    fn today_table(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(snap) = &self.snap else { return };
        let now = chrono::Local::now().naive_local();
        let mut coming: Vec<(chrono::NaiveDateTime, String, String, Option<String>)> = Vec::new();
        for (d, at) in Self::starts_today(snap, now) {
            let wake = d.net.as_ref().is_some_and(|n| n.wake);
            let sleep = show::Sleep::of(&snap.power, wake);
            coming.push((at, d.id.clone(), d.label().to_owned(), show::sleep_mark(t, &sleep, at, now)));
        }
        coming.sort();
        // The next start of a net (by id) after `from`: quiet spans are seen, a held net has none.
        let by_id: std::collections::HashMap<&str, &Def> = snap
            .defs
            .iter()
            .filter(|d| d.standing == Standing::Confirmed && !snap.held.contains(&d.id))
            .map(|d| (d.id.as_str(), d))
            .collect();
        let average = |id: &str| self.past.get(id).copied();
        let next_of = |id: &str, from: chrono::NaiveDateTime| -> String {
            by_id
                .get(id)
                .and_then(|d| {
                    let net = d.net.as_ref()?;
                    let cal = d.calendar.as_deref().and_then(|c| snap.calendars.get(c));
                    koyomi_core::quiet::next_starts(&net.when, &net.quiet, net.on_quiet, from, 1, cal).into_iter().next()
                })
                .map(|at| if at.date() == now.date() { show::hms(at) } else { at.format("%m-%d %H:%M").to_string() })
                .unwrap_or_default()
        };
        let unconfirmed = snap.defs.iter().filter(|d| d.standing == Standing::Unconfirmed).count();
        if self.runs.is_empty() && coming.is_empty() {
            ui.label(egui::RichText::new(t.tr("run.none")).color(c.dim));
            return;
        }
        // The counts: what needs a person, what is going on, what is still to come, what is done.
        let count = |f: &dyn Fn(State) -> bool| self.runs.iter().filter(|r| f(r.state)).count();
        let parts = [
            (t.tr("today.unconfirmed").to_owned(), unconfirmed, c.wait_text()),
            (show::tag(t, State::Error).to_owned(), count(&|s| s.is_error()), graph::colour(&c, State::Error)),
            (
                show::tag(t, State::Running).to_owned(),
                count(&|s| matches!(s, State::Running | State::RunningError)),
                graph::colour(&c, State::Running),
            ),
            (show::tag(t, State::Waiting).to_owned(), coming.len(), graph::colour(&c, State::Waiting)),
            (show::tag(t, State::Ok).to_owned(), count(&|s| matches!(s, State::Ok | State::Warn)), graph::colour(&c, State::Ok)),
        ];
        ui.horizontal(|ui| {
            for (label, n, colour) in &parts {
                let text = format!("{label} {n}");
                let r = ui.label(egui::RichText::new(&text).strong().color(if *n == 0 { c.dim } else { *colour }));
                ito_a11y::say(&r, Role::Label, &text);
                ui.add_space(8.0);
            }
        });
        ui.add_space(4.0);
        let mut pick = None;
        let mut runs: Vec<&Run> = self.runs.iter().collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.started));
        {
            egui::Grid::new("today_grid").num_columns(7).spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
                for key in ["col_state", "col_net", "col_planned", "col_start", "col_end", "col_time", "col_next"] {
                    ui.label(egui::RichText::new(t.tr(&format!("today.{key}"))).small().color(c.dim));
                }
                ui.end_row();
                for (at, id, title, mark) in &coming {
                    let colour = graph::colour(&c, State::Waiting);
                    ui.label(egui::RichText::new(show::tag(t, State::Waiting)).color(colour));
                    let name = ui.label(title);
                    ito_a11y::say(&name, Role::Row, title);
                    let planned = match mark {
                        Some(m) => format!("{}  {m}", show::hms(*at)),
                        None => show::hms(*at),
                    };
                    ui.label(planned);
                    ui.label("");
                    ui.label("");
                    ui.label(average(id).map(|a| t.trf("today.average", &[&show::duration(t, a)])).unwrap_or_default());
                    ui.label(next_of(id, *at));
                    ui.end_row();
                }
                for run in runs {
                    let colour = graph::colour(&c, run.state);
                    let on = self.run_sel.as_deref() == Some(run.id.as_str());
                    ui.label(egui::RichText::new(show::tag(t, run.state)).color(colour));
                    let r = ui.selectable_label(on, egui::RichText::new(&run.title).color(colour));
                    let planned = run.planned.map(show::hms).unwrap_or_default();
                    let ended = run.ended.map(show::hms).unwrap_or_default();
                    let usual = average(&run.jobnet).map(|a| t.trf("today.average", &[&show::duration(t, a)]));
                    let took = match (run.duration(), &usual) {
                        (Some(d), _) => show::duration(t, d),
                        (None, Some(u)) => format!("{}… {u}", show::duration(t, now - run.started)),
                        (None, None) => format!("{}…", show::duration(t, now - run.started)),
                    };
                    let said = [show::tag(t, run.state).to_owned(), run.title.clone(), show::hms(run.started), ended.clone(), took.clone()];
                    let said: Vec<&str> = said.iter().map(String::as_str).filter(|x| !x.is_empty()).collect();
                    ito_a11y::say(&r, Role::Row, &said.join(" "));
                    ui.label(planned);
                    ui.label(show::hms(run.started));
                    ui.label(ended);
                    ui.label(took);
                    ui.label(next_of(&run.jobnet, now));
                    ui.end_row();
                    if r.clicked() {
                        pick = Some(run.id.clone());
                    }
                }
            });
        }
        if pick.is_some() {
            self.run_sel = pick;
            self.cal_sel = None;
            self.job_sel = None;
            self.result = None;
        }
    }

    /// The tabs over the detail, at the right: 今日 (the chart), ジョブネット (the net picked on the
    /// left), カレンダー (the first one, or the one picked) and 履歴.
    fn tabs(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        if self.snap.is_none() {
            return;
        }
        let first_cal = self.snap.as_ref().and_then(|s| s.calendars.keys().next().cloned());
        let (today, history, calendar) = (self.gantt, self.hist.is_some(), self.cal_sel.is_some() && !self.gantt && self.hist.is_none());
        let net = !today && !history && !calendar;
        let mut go = None;
        // One row high: a centred layout would take the whole height and push the view below off the screen.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Right to left: the last tab is added first.
            let tab = |ui: &mut egui::Ui, on: bool, key: &str, enabled: bool| {
                let label = t.tr(key);
                let r = ui.add_enabled(enabled, egui::Button::selectable(on, label));
                ito_a11y::say(&r, Role::Button, label);
                r.clicked()
            };
            if tab(ui, history, "tabs.history", true) {
                go = Some(3);
            }
            if tab(ui, calendar, "tabs.calendar", true) {
                go = Some(2);
            }
            if tab(ui, net, "tabs.net", true) {
                go = Some(1);
            }
            if tab(ui, today, "tabs.today", true) {
                go = Some(0);
            }
        });
        match go {
            Some(0) => self.open_gantt(),
            Some(1) => {
                self.hist = None;
                self.gantt = false;
                self.run_sel = None;
                self.cal_sel = None;
                self.job_sel = None;
                self.result = None;
            }
            Some(2) => match self.cal_sel.clone().or(first_cal) {
                // No calendar yet: the tab is where the first one is made.
                None => self.new_cal_box(),
                Some(name) => {
                    self.hist = None;
                    self.gantt = false;
                    self.run_sel = None;
                    self.job_sel = None;
                    self.result = None;
                    self.cal_month = None;
                    self.cal_result = None;
                    self.cal_sel = Some(name);
                }
            },
            Some(3) => self.open_history(),
            _ => {}
        }
    }

    fn open_gantt(&mut self) {
        self.hist = None;
        self.run_sel = None;
        self.cal_sel = None;
        self.job_sel = None;
        self.result = None;
        self.gantt = true;
    }

    /// The tray's 今日: ガント in the detail, unless a net is open in the editor or a box is (that work stays).
    pub fn show_today(&mut self) {
        if self.editor.is_some() || self.port.is_some() || self.make.is_some() || self.vers.is_some() || self.deleting.is_some() {
            return;
        }
        self.open_gantt();
    }

    /// Every start of an approved, not held net still to come today: the planned times as the quiet
    /// spans leave them (a skipped one is gone, a waiting one moves to the span's end).
    fn starts_today(snap: &Snapshot, now: chrono::NaiveDateTime) -> Vec<(&Def, chrono::NaiveDateTime)> {
        let end = now.date().and_hms_opt(23, 59, 59).unwrap_or(now);
        let mut out = Vec::new();
        for d in snap.defs.iter().filter(|d| d.standing == Standing::Confirmed && !snap.held.contains(&d.id)) {
            if let Some(net) = &d.net {
                let cal = d.calendar.as_deref().and_then(|c| snap.calendars.get(c));
                let times = when::between_in(&net.when, now, end, cal);
                out.extend(
                    koyomi_core::quiet::starts(&net.quiet, net.on_quiet, times).into_iter().filter(|at| *at <= end).map(|at| (d, at)),
                );
            }
        }
        out
    }

    /// Every start still to come today, as (net id, title, start).
    fn planned_today(&self, now: chrono::NaiveDateTime) -> Vec<(String, String, chrono::NaiveDateTime)> {
        let Some(snap) = &self.snap else { return Vec::new() };
        Self::starts_today(snap, now).into_iter().map(|(d, at)| (d.id.clone(), d.label().to_owned(), at)).collect()
    }

    /// ガント: today's nets, a lane each, with the runs and the starts still planned; a click on a lane opens the run under the chart.
    fn gantt_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        ui.heading(t.tr("run.today"));
        ui.add_space(6.0);
        self.today_table(ui, t);
        ui.add_space(8.0);
        let r = ui.label(egui::RichText::new(t.tr("run.gantt")).strong().color(c.dim));
        ito_a11y::say(&r, Role::Heading, t.tr("run.gantt"));
        let now = chrono::Local::now().naive_local();
        let planned = self.planned_today(now);
        if self.runs.is_empty() && planned.is_empty() {
            ui.label(egui::RichText::new(t.tr("run.none")).color(c.dim));
            return;
        }
        if self.runs.iter().any(|r| !r.is_over()) {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        let runs: Vec<&Run> = self.runs.iter().collect();
        let picked = self.run_sel.clone();
        let mut pick = None;
        egui::ScrollArea::horizontal().id_salt("gantt").show(ui, |ui| {
            pick = gantt::draw(ui, &runs, &planned, now, picked.as_deref(), t.tr("run.gantt_earlier"), t.tr("run.gantt_planned"));
        });
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

    /// Opens the 版を比べる box on the net open in the editor, or else the one picked.
    pub fn open_versions(&mut self) {
        let id = self.editor.as_ref().map(|e| e.id.clone()).or_else(|| self.selected.clone());
        let Some(id) = id else { return };
        if self.box_open() {
            return;
        }
        let _ = self.tx.send(Request::Versions { id: id.clone() });
        self.focus = 1;
        self.vers = Some(Vers { id, list: Vec::new(), idx: 0, texts: BTreeMap::new(), asked: BTreeSet::new(), error: None });
    }

    /// Asks for the texts the box compares, and shows it: the version picked against the next newer one
    /// (the file as it is, for the newest).
    fn confirm_versions(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(v) = &mut self.vers else { return };
        let newer = |v: &Vers| if v.idx == 0 { "current".to_owned() } else { v.list[v.idx - 1].clone() };
        let old = v.list.get(v.idx).cloned();
        let new = old.as_ref().map(|_| newer(v));
        for w in old.iter().chain(new.iter()) {
            if v.asked.insert(w.clone()) {
                let _ = self.tx.send(Request::VersionText { id: v.id.clone(), version: w.clone() });
            }
        }
        let texts = old.as_ref().zip(new.as_ref()).and_then(|(o, n)| Some((v.texts.get(o)?, v.texts.get(n)?)));
        let rows = [(t.tr("versions.net").to_owned(), v.id.clone())];
        let body = match (&v.error, &old, &new) {
            (Some(e), _, _) => vec![t.trf("versions.failed", &[e])],
            (None, Some(o), Some(n)) => {
                vec![t.trf("versions.compare", &[o, n, &(v.idx + 1).to_string(), &v.list.len().to_string()])]
            }
            _ => vec![t.tr("versions.loading").to_owned()],
        };
        let diff = texts.map(|(o, n)| ito_diff::diff(o, n, 3)).unwrap_or_default();
        let buttons = [
            t.tr("versions.restore").to_owned(),
            t.tr("canvas.cancel").to_owned(),
            t.tr("versions.older").to_owned(),
            t.tr("versions.newer").to_owned(),
        ];
        let q = ito_confirm::Question {
            title: t.tr("versions.title"),
            rows: &rows,
            body: &body,
            diff: &diff,
            buttons: &buttons,
            hint: t.tr("confirm.hint"),
            focus: Some(self.focus),
            primary: Some(1),
            danger: true,
            ..Default::default()
        };
        let answer = ito_confirm::show(ctx, &q, &Self::style()).or(key);
        let Some(v) = &mut self.vers else { return };
        match answer {
            Some(0) if v.list.get(v.idx).is_some() => {
                let req = Request::Restore { id: v.id.clone(), version: v.list[v.idx].clone() };
                self.vers = None;
                self.result = None;
                let _ = self.tx.send(req);
            }
            Some(2) if v.idx + 1 < v.list.len() => v.idx += 1,
            Some(3) if v.idx > 0 => v.idx -= 1,
            Some(1) => self.vers = None,
            _ => {}
        }
    }

    /// Opens 履歴 on the last 7 days and asks the engine for them.
    pub fn open_history(&mut self) {
        let h = History::week(chrono::Local::now().date_naive());
        let _ = self.tx.send(h.request());
        self.hist = Some(h);
        self.gantt = false;
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
        let (mut search, mut csv) = (false, false);
        ui.horizontal(|ui| {
            ui.label(t.tr("run.history_from"));
            ui.add(egui::TextEdit::singleline(&mut h.from).desired_width(90.0));
            ui.label(t.tr("run.history_to"));
            ui.add(egui::TextEdit::singleline(&mut h.to).desired_width(90.0));
            let all = t.tr("run.history_all");
            ui.label(t.tr("run.history_net"));
            let net_text = if h.net.is_empty() { all.to_owned() } else { h.net.clone() };
            egui::ComboBox::from_id_salt("history_net").selected_text(net_text).show_ui(ui, |ui| {
                ui.selectable_value(&mut h.net, String::new(), all);
                for id in &nets {
                    ui.selectable_value(&mut h.net, id.clone(), id);
                }
            });
            ui.label(t.tr("run.history_state"));
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
            let r = ui.button(t.tr("run.history_csv"));
            ito_a11y::say(&r, Role::Button, t.tr("run.history_csv"));
            csv = r.clicked();
        });
        if csv {
            let mut shown = h.filtered();
            shown.reverse();
            let text = crate::csv::runs(t, &shown.into_iter().cloned().collect::<Vec<_>>());
            self.result = Some(Ok(t.tr("run.writing").to_owned()));
            Self::background(&self.io, move || export_csv("history", &text).map(|path| ("run.history_csv_done", vec![path])));
        }
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
        if !rows.is_empty() {
            let owned: Vec<Run> = rows.iter().map(|r| (*r).clone()).collect();
            let lines = crate::report::lines(t, &koyomi_core::report::summarize(&owned));
            egui::CollapsingHeader::new(t.tr("report.heading")).id_salt("history_report").show(ui, |ui| {
                for l in &lines {
                    ui.label(l);
                }
            });
        }
        let mut pick = None;
        egui::Grid::new("history_grid").num_columns(6).spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
            for key in ["col_date", "col_state", "col_net", "col_start", "col_end", "col_time"] {
                ui.label(egui::RichText::new(t.tr(&format!("today.{key}"))).small().color(c.dim));
            }
            ui.end_row();
            for run in rows {
                let colour = graph::colour(&c, run.state);
                let ended = run.ended.map(show::hms).unwrap_or_default();
                let took = run.duration().map(|d| show::duration(t, d)).unwrap_or_default();
                let on = self.run_sel.as_deref() == Some(run.id.as_str());
                ui.label(run.started.date().to_string());
                ui.label(egui::RichText::new(show::tag(t, run.state)).color(colour));
                let r = ui.selectable_label(on, egui::RichText::new(&run.title).color(colour));
                let said = [
                    run.started.date().to_string(),
                    show::tag(t, run.state).to_owned(),
                    run.title.clone(),
                    show::hms(run.started),
                    ended.clone(),
                    took.clone(),
                ];
                let said = said.iter().map(String::as_str).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
                ito_a11y::say(&r, Role::Row, &said);
                ui.label(show::hms(run.started));
                ui.label(ended);
                ui.label(took);
                ui.end_row();
                if r.clicked() {
                    pick = Some(run.id.clone());
                }
            }
        });
        self.show_result(ui);
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

    /// Whether `job` of the net `net` is on hold (`held.toml`).
    fn job_held(&self, net: &str, job: &str) -> bool {
        self.snap.as_ref().is_some_and(|s| s.held_jobs.iter().any(|h| h.net == net && h.job == job))
    }

    /// 「保留 / 保留を解く」 for one job of a net.
    fn hold_job_button(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, net: &str, job: &str) {
        let held = self.job_held(net, job);
        let label = t.trf(if held { "run.release_job" } else { "run.hold_job" }, &[job]);
        let r = ui.add_enabled(!self.blocked(), egui::Button::new(&label));
        ito_a11y::say(&r, Role::Button, &label);
        if r.clicked() {
            let (id, job) = (net.to_owned(), Some(job.to_owned()));
            let _ = self.tx.send(if held { Request::Release { id, job } } else { Request::Hold { id, job } });
        }
    }

    /// The run: its line, the buttons, the picture, the jobs in words, the log.
    fn run_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(run) = self.selected_run().cloned() else {
            ui.label(t.tr("run.none"));
            return;
        };
        let (big, small) = show::run_head(t, &run);
        ui.heading(&big);
        ui.label(egui::RichText::new(small).small().color(c.dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            // Only the buttons that mean something for this run are drawn.
            if !run.is_over() {
                let r = ui.button(t.tr("run.kill"));
                ito_a11y::say(&r, Role::Button, t.tr("run.kill"));
                if r.clicked() && !self.box_open() {
                    self.killing = Some(run.id.clone());
                    // The safe button first: Enter on a fresh box does not kill.
                    self.focus = 1;
                }
            }
            // Again, once it is over, from one menu: what passed is carried over, or from the picked
            // job on. A run where everything passed can only go again from a picked job.
            let picked = self.job_sel.clone().filter(|p| run.jobs.iter().any(|j| &j.id == p));
            if run.is_over() {
                let head = format!("{} ▾", t.tr("run.rerun"));
                let tx = &self.tx;
                let r = ui.add_enabled_ui(!self.blocked(), |ui| {
                    ui.menu_button(&head, |ui| {
                        let rest = !run.jobs.iter().all(|j| j.state.passed());
                        let r = ui.add_enabled(rest, egui::Button::new(t.tr("run.rerun_rest")));
                        ito_a11y::say(&r, Role::Button, t.tr("run.rerun_rest"));
                        if r.clicked() {
                            let _ = tx.send(Request::Rerun { run: run.id.clone(), from: None, next: None });
                        }
                        let label = match &picked {
                            Some(j) => t.trf("run.rerun_from", &[j]),
                            None => t.tr("run.rerun_pick").to_owned(),
                        };
                        let r = ui.add_enabled(picked.is_some(), egui::Button::new(&label));
                        ito_a11y::say(&r, Role::Button, &label);
                        if r.clicked() {
                            let _ = tx.send(Request::Rerun { run: run.id.clone(), from: picked.clone(), next: None });
                        }
                        // From the jobs after the picked one, which ends as スキップ or 警告 without running.
                        for (mark, key) in [(State::Skipped, "run.rerun_next_skip"), (State::Warn, "run.rerun_next_warn")] {
                            let Some(j) = &picked else { break };
                            let label = t.trf(key, &[j]);
                            let r = ui.button(&label);
                            ito_a11y::say(&r, Role::Button, &label);
                            if r.clicked() {
                                let _ = tx.send(Request::Rerun { run: run.id.clone(), from: picked.clone(), next: Some(mark) });
                            }
                        }
                    })
                    .response
                });
                ito_a11y::say(&r.inner, Role::Button, &head);
            }
            // A failure dealt with: it leaves the count in the status bar.
            let done = self.acked.contains(&run.id);
            let words = t.tr(if done { "run.acked_already" } else { "run.ack" });
            if run.is_over() && run.state.is_error() && primary(ui, !done, words).clicked() {
                self.acked.insert(run.id.clone());
                self.to_ack.push(run.id.clone());
                self.result = Some(Ok(t.tr("run.acked").to_owned()));
            }
            // Skip the picked job before it starts: the jobs after it go on.
            let pending = picked.as_deref().and_then(|p| run.job(p)).is_some_and(|j| j.pending() && j.state != State::Running);
            let label = match &picked {
                Some(j) => t.trf("run.skip_from", &[j]),
                None => t.tr("run.skip_pick").to_owned(),
            };
            if !run.is_over() && pending {
                let r = ui.button(&label);
                ito_a11y::say(&r, Role::Button, &label);
                if r.clicked() {
                    if let Some(job) = picked.clone() {
                        let _ = self.tx.send(Request::Skip { run: run.id.clone(), job });
                    }
                }
            }
            // Hold the picked job of the net: a run that comes to it waits there (held.toml, every run).
            if let Some(job) = picked.as_deref().filter(|p| !p.contains('/')) {
                self.hold_job_button(ui, t, &run.jobnet, job);
            }
            // A 確認待ち part waits for the picked answer; with none picked, the only one that waits.
            let asking: Vec<&str> = run
                .jobs
                .iter()
                .filter(|j| j.state == State::Running && j.part == koyomi_core::jobnet::Part::Approval)
                .map(|j| j.id.as_str())
                .collect();
            let target =
                picked.as_deref().filter(|p| asking.contains(p)).or(if asking.len() == 1 { asking.first().copied() } else { None });
            for (yes, key) in [(true, "run.approve"), (false, "run.reject")].into_iter().filter(|_| target.is_some()) {
                let label = match target {
                    Some(j) => t.trf(&format!("{key}_from"), &[j]),
                    None => t.tr(key).to_owned(),
                };
                let r = ui.button(&label);
                ito_a11y::say(&r, Role::Button, &label);
                if r.clicked() {
                    if let Some(job) = target {
                        let _ = self.tx.send(Request::Answer { run: run.id.clone(), job: job.to_owned(), yes });
                    }
                }
            }
            // Mark this run, then pick another: the two are shown side by side under the buttons.
            let marked = self.compare_base.as_deref() == Some(run.id.as_str());
            let label = t.tr(if marked { "run.compare_clear" } else { "run.compare_mark" });
            let r = ui.button(label);
            ito_a11y::say(&r, Role::Button, label);
            if r.clicked() {
                self.compare_base = (!marked).then(|| run.id.clone());
            }
            if run.is_over() && picked.is_none() {
                ui.label(egui::RichText::new(t.tr("run.rerun_hint")).small().color(c.dim));
            }
        });
        self.show_result(ui);
        if let Some(base) = self.compare_base.as_deref().filter(|b| *b != run.id).and_then(|b| self.find_run(b)) {
            ui.add_space(6.0);
            let head = t.trf("run.compare_head", &[&base.id]);
            ui.label(egui::RichText::new(&head).strong());
            for l in crate::compare::lines(t, base, &run, &[]) {
                let r = ui.add(egui::Label::new(egui::RichText::new(&l).monospace()).wrap());
                ito_a11y::say(&r, Role::Label, &l);
            }
        }
        ui.add_space(6.0);
        let watched = Self::watched_job(&run, self.job_sel.as_deref());
        *self.watch.lock().unwrap_or_else(|e| e.into_inner()) = watched.clone().map(|j| (run.id.clone(), j));
        // The direction of the picture sits next to the picture.
        ui.horizontal(|ui| {
            for (vertical, key) in [(false, "run.horizontal"), (true, "run.vertical")] {
                let r = ui.selectable_label(self.vertical == vertical, t.tr(key));
                ito_a11y::say(&r, Role::Button, t.tr(key));
                if r.clicked() && self.vertical != vertical {
                    self.set_flow(vertical);
                }
            }
        });
        let mut pick = None;
        egui::ScrollArea::horizontal().id_salt("graph").auto_shrink([false, true]).show(ui, |ui| {
            pick = graph::draw(ui, &run, self.vertical, watched.as_deref());
        });
        graph::legend(ui, t, &c, &run);
        egui::Grid::new("jobs_grid").num_columns(8).spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
            for key in ["run.col_job", "run.state", "run.start", "run.end", "run.exit", "run.retried", "run.took", "run.reason"] {
                ui.label(egui::RichText::new(t.tr(key)).small().color(c.dim));
            }
            ui.end_row();
            for j in &run.jobs {
                let colour = graph::colour(&c, j.state);
                let on = watched.as_deref() == Some(j.id.as_str());
                let name = format!("{}{}", show::kind(t, &j.part), j.label());
                let r = ui.selectable_label(on, egui::RichText::new(&name).color(colour));
                ito_a11y::say(&r, Role::Row, &graph::words(t, j));
                if r.clicked() {
                    pick = Some(j.id.clone());
                }
                ui.label(egui::RichText::new(show::tag(t, j.state)).color(colour));
                ui.label(j.started.map(show::hms).unwrap_or_default());
                ui.label(j.ended.map(show::hms).unwrap_or_default());
                ui.label(j.exit.map(|x| x.to_string()).unwrap_or_default());
                ui.label(match (j.retried, j.retry) {
                    (0, None) => String::new(),
                    (n, Some(r)) => format!("{n}/{}", r.count),
                    (n, None) => n.to_string(),
                });
                ui.label(j.duration().map(|d| show::duration(t, d)).unwrap_or_default());
                ui.label(koyomi_core::run::tidy_reason(&j.reason));
                ui.end_row();
            }
        });
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
        let mut head = review::row(t, &def, held);
        if self.snap.as_ref().is_some_and(|s| review::twins(&s.defs).contains(&def.id)) {
            head = review::twin_row(head, &def);
        }
        ui.heading(&head);
        ui.label(egui::RichText::new(&def.file).color(c.dim));
        ui.add_space(6.0);
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
            if r.clicked() && !self.box_open() {
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
                self.run_now(def.id.clone(), def.net.as_ref().map(|n| &n.params));
            }
            // On hold: its times are written down as ⏸ and nothing runs until it is released.
            let key = if held { "run.release" } else { "run.hold" };
            let r = ui.add_enabled(!self.blocked(), egui::Button::new(t.tr(key)));
            ito_a11y::say(&r, Role::Button, t.tr(key));
            if r.clicked() {
                let id = def.id.clone();
                let _ = self.tx.send(if held { Request::Release { id, job: None } } else { Request::Hold { id, job: None } });
            }
        });
        if held {
            let text = format!("{} {}", State::Held.mark(), t.tr("run.held_note"));
            let r = ui.label(egui::RichText::new(&text).color(graph::colour(&c, State::Held)));
            ito_a11y::say(&r, Role::Label, &text);
        }
        self.show_result(ui);
        ui.add_space(6.0);
        for p in &def.problems {
            let line = p.in_file(&def.file);
            ui.label(egui::RichText::new(format!("{} {line}", ito_a11y::State::Error.mark())).color(c.err));
        }
        let section = |ui: &mut egui::Ui, key: &str| {
            ui.add_space(10.0);
            let r = ui.label(egui::RichText::new(t.tr(key)).strong().size(15.0));
            ito_a11y::say(&r, Role::Heading, t.tr(key));
        };
        if let Some(net) = def.net.as_ref().filter(|n| !n.jobs.is_empty()) {
            section(ui, "net.sec_graph");
            let now = chrono::Local::now().naive_local();
            let mut preview = Run::new("", &def.id, def.label(), "", koyomi_core::run::How::Manual, net, now);
            // A held job shows ⏸ on its station.
            let mut any_held = false;
            for j in preview.jobs.iter_mut().filter(|j| self.job_held(&def.id, &j.id)) {
                j.state = State::Held;
                any_held = true;
            }
            let picked = self.job_sel.clone().filter(|p| net.jobs.iter().any(|j| &j.id == p));
            let mut pick = None;
            egui::ScrollArea::horizontal().id_salt("net_graph").auto_shrink([false, true]).show(ui, |ui| {
                pick = graph::draw(ui, &preview, self.vertical, picked.as_deref());
            });
            if pick.is_some() {
                self.job_sel = if pick == picked { None } else { pick };
            }
            if any_held {
                let text = t.tr("run.job_held_note");
                let r = ui.label(egui::RichText::new(text).color(graph::colour(&c, State::Held)));
                ito_a11y::say(&r, Role::Label, text);
            }
            if let Some(job) = picked {
                ui.horizontal(|ui| self.hold_job_button(ui, t, &def.id, &job));
            }
        }
        if let Some(net) = &def.net {
            section(ui, "net.sec_timing");
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
                let power = self.snap.as_ref().map(|s| show::Sleep::of(&s.power, net.wake));
                let rows =
                    show::next_rows(t, &net.when, (&net.quiet, net.on_quiet), cal, chrono::Local::now().naive_local(), 10, power.as_ref());
                if rows.is_empty() {
                    ui.label(egui::RichText::new(t.tr("run.next_over")).color(c.dim));
                }
                for row in rows {
                    ui.label(format!("  {row}"));
                }
            }
        }
        let lines = review::settings(t, &def);
        if !lines.is_empty() {
            section(ui, "net.sec_settings");
            for l in lines {
                ui.label(l);
            }
        }
        if def.net.as_ref().is_some_and(|n| !n.jobs.is_empty()) {
            section(ui, "net.sec_jobs");
            if let Some(net) = &def.net {
                egui::Grid::new("net_jobs").striped(true).num_columns(4).show(ui, |ui| {
                    for h in ["net.col_id", "net.col_kind", "net.col_name", "net.col_after"] {
                        ui.label(egui::RichText::new(t.tr(h)).strong());
                    }
                    ui.end_row();
                    for j in &net.jobs {
                        ui.label(&j.id);
                        ui.label(t.tr(&format!("part.{}", j.part.key())));
                        ui.label(j.name.as_deref().unwrap_or(""));
                        let after: Vec<String> = j.after.iter().map(|l| l.text()).collect();
                        ui.label(after.join(", "));
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
            }
            for l in review::job_details(t, &def) {
                ui.label(l);
            }
        }
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

    /// The box that asks before a net is deleted: what names it (the engine refuses then), and
    /// that its file stays as a version.
    fn confirm_delete(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(id) = self.deleting.clone() else { return };
        let users = self.snap.as_ref().map(|s| koyomi_core::rename::referrers(&s.defs, &id)).unwrap_or_default();
        let rows = [(t.tr("versions.net").to_owned(), id.clone())];
        let mut body = vec![t.trf("delete.ask", &[&id])];
        if !users.is_empty() {
            body.push(t.trf("delete.users", &[&users.join(", ")]));
        }
        let buttons = [t.tr("delete.yes").to_owned(), t.tr("make.cancel").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("delete.title"),
            rows: &rows,
            body: &body,
            buttons: &buttons,
            hint: t.tr("confirm.hint"),
            focus: Some(self.focus),
            primary: Some(1),
            danger: true,
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &Self::style()).or(key) {
            Some(0) => {
                self.deleting = None;
                self.result = None;
                let _ = self.tx.send(Request::Delete { id });
            }
            Some(_) => self.deleting = None,
            None => {}
        }
    }
}

/// A screen's one main button (D13): filled with the theme's accent, greyed when not `enabled`.
pub(crate) fn primary(ui: &mut egui::Ui, enabled: bool, words: &str) -> egui::Response {
    let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
    let r = ui.add_enabled_ui(enabled, |ui| ito_prefs::button_with(ui, look, words, ito_prefs::Tone::Primary)).inner;
    ito_a11y::say(&r, Role::Button, words);
    r
}

/// Everything in the settings folder, as one file under `exports/` in the
/// state folder: its path and how many things it carries.
/// A caution without the part that moves each day, so the band opens for a new caution and not
/// for the same one a day older.
fn caution_key(line: &str) -> String {
    match (line.find("（あと"), line.find('）')) {
        (Some(a), Some(b)) if a < b => format!("{}{}", &line[..a], &line[b + '）'.len_utf8()..]),
        _ => line.to_owned(),
    }
}

fn export_all() -> Result<(String, usize), Said> {
    let paths = engine::Paths::find().map_err(plain)?;
    let src = koyomi_core::bundle::Source::read(&paths.app);
    let b = koyomi_core::bundle::export(&src, &koyomi_core::bundle::Pick { all: true, ..Default::default() }).map_err(plain)?;
    if b.is_empty() {
        return Err(("port.export_empty", Vec::new()));
    }
    let dir = paths.state.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| no_dir(&dir, e))?;
    let path = dir.join(format!("{}.toml", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    ito_common::write_atomic(&path, b.to_toml()).map_err(plain)?;
    Ok((path.display().to_string(), b.len()))
}

/// `text` as `exports/<what>-<YYYYMMDD-HHMMSS>.csv` in the state folder: its path.
fn export_csv(what: &str, text: &str) -> Result<String, Said> {
    let dir = engine::Paths::find().map_err(plain)?.state.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| no_dir(&dir, e))?;
    let path = dir.join(format!("{what}-{}.csv", chrono::Local::now().format("%Y%m%d-%H%M%S"))).display().to_string();
    crate::csv::write(&path, text).map_err(plain)?;
    Ok(path)
}

/// A failure from below, whose words are already made: told as they are.
fn plain(e: String) -> Said {
    ("port.plain", vec![e])
}

/// A folder that could not be made, worded when shown.
fn no_dir(dir: &std::path::Path, e: std::io::Error) -> Said {
    ("port.no_dir", vec![dir.display().to_string(), e.to_string()])
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
        let (tx_msg, rx) = mpsc::channel();
        let (tx, _) = mpsc::channel();
        let snap = Some(Snapshot { defs, ..Snapshot::default() });
        Browser {
            rx,
            io: (tx_msg, egui::Context::default()),
            tx,
            watch: Arc::default(),
            snap,
            runs: Vec::new(),
            past: std::collections::HashMap::new(),
            past_asked: None,
            past_failed: None,
            down: None,
            starting: false,
            selected: None,
            run_sel: None,
            compare_base: None,
            job_sel: None,
            log_find: String::new(),
            log: None,
            vertical: false,
            flow: None,
            asking: None,
            killing: None,
            deleting: None,
            focus: 0,
            close_asked: false,
            result: None,
            over: None,
            ended: Vec::new(),
            to_ack: Vec::new(),
            acked: BTreeSet::new(),
            done: None,
            seen_notes: BTreeSet::new(),
            notes: Vec::new(),
            autostart_off: false,
            band_seen: None,
            cal_sel: None,
            folded: BTreeSet::new(),
            tree_seen: None,
            cal_month: None,
            cal_result: None,
            cal_inline: None,
            hist: None,
            gantt: false,
            vers: None,
            editor: None,
            editors: Vec::new(),
            port: None,
            make: None,
            new_cal: None,
            cal_deleting: None,
            give: None,
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

    /// A screen with a pointer: what the screen reader is told (label and place of each widget
    /// that says something), so a test can press a widget by its words instead of a coordinate.
    struct Screen {
        ctx: egui::Context,
        size: egui::Vec2,
        nodes: BTreeMap<u64, (String, egui::Rect)>,
    }

    impl Screen {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            Screen { ctx, size: egui::vec2(1280.0, 800.0), nodes: BTreeMap::new() }
        }

        fn show(&mut self, events: Vec<egui::Event>, b: &mut Browser) {
            let t = crate::i18n::table("ja").0;
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size);
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| b.show(ui, &t));
            out.textures_delta.clear();
            if let Some(update) = out.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    let place = node
                        .bounds()
                        .map(|r| egui::Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32)));
                    match (node.label().or_else(|| node.value()), place) {
                        (Some(label), Some(place)) => drop(self.nodes.insert(id.0, (label.to_owned(), place))),
                        _ => drop(self.nodes.remove(&id.0)),
                    }
                }
            }
        }

        /// Frames until the layout has settled.
        fn settle(&mut self, b: &mut Browser) {
            for _ in 0..3 {
                self.show(Vec::new(), b);
            }
        }

        /// Presses the one widget whose words contain `words`, with the primary button.
        fn click(&mut self, words: &str, b: &mut Browser) {
            let exact: Vec<_> = self.nodes.values().filter(|(label, _)| label == words).collect();
            let found: Vec<_> =
                if exact.is_empty() { self.nodes.values().filter(|(label, _)| label.contains(words)).collect() } else { exact };
            assert_eq!(found.len(), 1, "one widget says {words:?}: {:?}", self.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
            let pos = found[0].1.center();
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.show(vec![egui::Event::PointerMoved(pos)], b);
            self.show(vec![button(true)], b);
            self.show(vec![button(false)], b);
            self.settle(b);
        }
    }

    /// A calendar's rename and delete name the files that still say `calendar =`: a net's own, or
    /// its group's `_group.toml` once; the delete box starts on the safe button.
    #[test]
    fn renaming_or_deleting_a_calendar_lists_what_names_it() {
        let mut d = defs();
        let own = koyomi_core::jobnet::parse(
            "format = 1
calendar = \"会社\"
[[job]]
id = \"a\"
run = [\"x.exe\"]
",
        )
        .unwrap();
        d[0].net = Some(own);
        d[0].calendar = Some("会社".into());
        d[1].calendar = Some("会社".into());
        d[2].calendar = Some("会社".into());
        assert_eq!(calendar::users(&d, "会社"), vec!["jobnets/試験/bom.toml".to_owned(), "jobnets/試験/_group.toml".to_owned()]);
        assert!(calendar::users(&d, "工場").is_empty());
        let mut b = quiet(d);
        b.snap
            .as_mut()
            .unwrap()
            .calendars
            .insert("会社".into(), koyomi_core::calendar::Calendar { name: "会社".into(), ..Default::default() });
        let mut s = Screen::new();
        s.settle(&mut b);
        b.cal_sel = Some("会社".into());
        s.settle(&mut b);
        s.click("名前を変える…", &mut b);
        assert_eq!(b.new_cal.as_ref().map(|m| m.name.as_str()), Some("会社"));
        let shown = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(shown(&s, "jobnets/試験/bom.toml, jobnets/試験/_group.toml"));
        assert!(shown(&s, "会社 のファイルの名前を変えます"), "a rename says the content stays");
        s.show(key(egui::Key::Enter), &mut b);
        assert_eq!(b.new_cal.as_ref().and_then(|m| m.error.as_deref()), Some("もうあります: 会社"), "the name it has is no rename");
        s.show(key(egui::Key::Escape), &mut b);
        assert!(b.new_cal.is_none());
        s.click("消す…", &mut b);
        assert_eq!(b.cal_deleting.as_deref(), Some("会社"));
        assert!(shown(&s, "versions/_calendars/会社/"));
        s.show(key(egui::Key::Enter), &mut b);
        assert!(b.cal_deleting.is_none() && b.cal_result.is_none(), "Enter on the fresh box deletes nothing");
    }

    /// The field under the month: a day's name opens with the name it has, Esc closes it, and
    /// 祝日の表を取り込む with no path writes nothing (the writing is in `calendar::tests`).
    #[test]
    fn a_day_is_named_and_a_holiday_table_brought_in_under_the_month() {
        let mut b = quiet(defs());
        let cal = koyomi_core::calendar::Calendar::parse("会社", "format = 1\nclosed = [{ date = 2026-11-20, name = \"創立記念日\" }]\n");
        b.snap.as_mut().unwrap().calendars.insert("会社".into(), cal.unwrap());
        let mut s = Screen::new();
        s.settle(&mut b);
        b.cal_sel = Some("会社".into());
        b.cal_month = Some((2026, 11));
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        b.cal_inline = Some(calendar::Inline::Name { cal: "会社".into(), day, text: "創立記念日".into(), fresh: true });
        s.settle(&mut b);
        let shown = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(shown(&s, "2026-11-20 の名前"));
        s.show(key(egui::Key::Escape), &mut b);
        assert!(b.cal_inline.is_none() && b.cal_result.is_none(), "Esc writes nothing");
        s.click("祝日の表を取り込む…", &mut b);
        assert!(matches!(b.cal_inline, Some(calendar::Inline::Table { .. })));
        s.click("決める", &mut b);
        assert_eq!(b.cal_result, Some(Err("パスを入れてください".to_owned())));
        assert!(b.cal_inline.is_none());
        b.cal_inline = Some(calendar::Inline::Name { cal: "工場".into(), day, text: String::new(), fresh: true });
        s.settle(&mut b);
        assert!(b.cal_inline.is_none(), "another calendar's field goes");
    }

    /// With no calendar the heading stays, says so, and its button opens the box; the box turns
    /// down an empty name and one a calendar has (a file is written only by the engine-free path,
    /// which `calendar::tests` checks).
    #[test]
    fn the_calendar_heading_stays_with_none_and_its_button_opens_the_box() {
        let mut b = quiet(Vec::new());
        let mut s = Screen::new();
        s.settle(&mut b);
        let shown = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(shown(&s, "カレンダーはまだありません"));
        s.click("＋ 新しいカレンダー", &mut b);
        assert!(b.new_cal.is_some(), "the button opens the box");
        assert!(shown(&s, "日本の祝日を休みにする"));
        s.show(key(egui::Key::Enter), &mut b);
        assert_eq!(b.new_cal.as_ref().and_then(|m| m.error.as_deref()), Some("名前を入れてください"));
        b.snap
            .as_mut()
            .unwrap()
            .calendars
            .insert("会社".into(), koyomi_core::calendar::Calendar { name: "会社".into(), ..Default::default() });
        b.new_cal.as_mut().unwrap().name = "会社".into();
        s.show(key(egui::Key::Enter), &mut b);
        assert_eq!(b.new_cal.as_ref().and_then(|m| m.error.as_deref()), Some("もうあります: 会社"));
        s.show(key(egui::Key::Escape), &mut b);
        assert!(b.new_cal.is_none(), "Esc closes it");
    }

    /// A line that was not in the band opens it once (not the ones there at start, and not only by
    /// count); folding it again keeps it folded until another new line comes.
    /// (The screen keeps the nodes it has seen, so only a label that is new proves the body was drawn.)
    #[test]
    fn a_new_caution_opens_the_band_once() {
        let set = |b: &mut Browser, lines: &[&str]| b.snap.as_mut().unwrap().cautions = lines.iter().map(|l| (*l).to_owned()).collect();
        let shown = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        let screen = || {
            let s = Screen::new();
            s.ctx.all_styles_mut(|st| st.animation_time = 0.0);
            s
        };
        // Already there at start: folded.
        let mut b = quiet(Vec::new());
        set(&mut b, &["既にある注意"]);
        let mut s = screen();
        s.settle(&mut b);
        assert!(!shown(&s, "既にある注意"), "folded at start");
        // Not there before: opens.
        let mut b = quiet(Vec::new());
        let mut s = screen();
        s.settle(&mut b);
        set(&mut b, &["注意その1"]);
        s.settle(&mut b);
        assert!(shown(&s, "注意その1"));
        s.click("注意 1", &mut b);
        // Folded by hand: a different line in its place (same count) opens it again.
        set(&mut b, &["別の注意"]);
        s.settle(&mut b);
        assert!(shown(&s, "別の注意"), "a replaced line opens it again");
        // The same caution with only its day count moved (each midnight) does not.
        s.click("注意 1", &mut b);
        set(&mut b, &["祝日の表は 2027-12-31 までの分です（あと 60 日）。置けば延びる"]);
        s.settle(&mut b);
        s.click("注意 1", &mut b);
        set(&mut b, &["祝日の表は 2027-12-31 までの分です（あと 59 日）。置けば延びる"]);
        s.settle(&mut b);
        assert!(!shown(&s, "あと 59 日"), "a day count alone keeps it folded");
    }

    /// The history filters say what each box narrows by, on the row of the 検索 button.
    #[test]
    fn the_history_filters_are_labelled() {
        let mut b = quiet(defs());
        b.open_history();
        let mut s = Screen::new();
        s.settle(&mut b);
        let row = |s: &Screen, w: &str| s.nodes.values().filter(|(l, _)| l == w).map(|(_, r)| r.center().y).collect::<Vec<_>>();
        let search = row(&s, "検索");
        assert_eq!(search.len(), 1);
        for w in ["ジョブネット", "状態"] {
            assert!(row(&s, w).iter().any(|y| (y - search[0]).abs() < 8.0), "{w} beside the filters");
        }
    }

    /// A net's page is split into 図, 実行のタイミング, 設定 and ジョブ, in that order.
    #[test]
    fn the_net_page_has_its_four_sections() {
        let dir = std::env::temp_dir().join(format!("koyomi-view-sections-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("夜間.toml");
        std::fs::write(&p, format!("wake = true\n{NET}")).unwrap();
        let def = read_def(&p, "試験", "夜間", &Store::default());
        let selected = Some(def.id.clone());
        let mut b = quiet(vec![def]);
        b.selected = selected;
        let mut s = Screen::new();
        s.settle(&mut b);
        let y = |w: &str| {
            let ys: Vec<f32> = s.nodes.values().filter(|(l, _)| l == w).map(|(_, r)| r.center().y).collect();
            assert!(!ys.is_empty(), "{w} is a heading: {:?}", s.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
            ys.into_iter().fold(f32::MIN, f32::max)
        };
        let ys = ["図", "実行のタイミング", "設定", "ジョブ"].map(y);
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "in order: {ys:?}");
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

    /// An over run has one 再実行 ▾ menu: from where it stopped, from the picked job, or from the
    /// jobs after it.
    #[test]
    fn rerun_is_one_menu() {
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
        run.finish("b", Outcome::Exited(1), now);
        assert!(run.is_over());
        let (tx, rx_req) = mpsc::channel();
        let mut b = quiet(defs());
        b.tx = tx;
        b.runs = vec![run];
        b.run_sel = Some("20261030-090000-01".into());
        let mut s = Screen::new();
        s.settle(&mut b);
        let menu_says = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(!menu_says(&s, "止まったところから"), "one button until opened");
        assert!(menu_says(&s, "ジョブを選ぶとそこから再実行"), "the hint with no job picked");
        s.click("再実行 ▾", &mut b);
        s.click("止まったところから", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Rerun { from: None, next: None, .. })));
        b.job_sel = Some("a".into());
        s.settle(&mut b);
        s.click("再実行 ▾", &mut b);
        s.click("a から再実行", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Rerun { from: Some(f), next: None, .. }) if f == "a"));
        s.click("再実行 ▾", &mut b);
        s.click("a の次から（a は警告終了）", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Rerun { from: Some(f), next: Some(State::Warn), .. }) if f == "a"));
        // The picked job is held for the net (H3); held, the button lets it go.
        s.click("a を保留", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Hold { id, job: Some(j) }) if id == "試験/bom" && j == "a"));
        b.snap.as_mut().unwrap().held_jobs.push(koyomi_core::held::HeldJob { net: "試験/bom".into(), job: "a".into() });
        s.settle(&mut b);
        s.click("a の保留を解く", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Release { id, job: Some(j) }) if id == "試験/bom" && j == "a"));
    }

    /// The kill box starts on the safe button: a plain Enter does not kill.
    /// 今すぐ実行 starts a net without `[params]` at once, and asks first for one with them,
    /// sending only the values that were changed.
    #[test]
    fn run_now_asks_for_params_and_sends_only_the_changed_ones() {
        let (tx, rx_req) = mpsc::channel();
        let mut b = quiet(defs());
        b.tx = tx;
        b.run_now("試験/bom".into(), None);
        match rx_req.try_recv() {
            Ok(Request::Run { params, .. }) => assert!(params.is_empty()),
            other => panic!("not a run: {other:?}"),
        }
        let params = BTreeMap::from([("month".to_owned(), "{month-1:%Y-%m}".to_owned()), ("dept".to_owned(), "all".to_owned())]);
        b.run_now("試験/bom".into(), Some(&params));
        assert!(rx_req.try_recv().is_err(), "the box asks first");
        assert!(b.report().contains("give: 試験/bom dept=all month={month-1:%Y-%m}"), "{}", b.report());
        let g = b.give.as_mut().unwrap();
        g.values.iter_mut().find(|(k, _, _)| k == "dept").unwrap().2 = "経理".into();
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(b.give.is_none(), "Enter answered the box");
        match rx_req.try_recv() {
            Ok(Request::Run { id, params, .. }) => {
                assert_eq!(id, "試験/bom");
                assert_eq!(params, BTreeMap::from([("dept".to_owned(), "経理".to_owned())]));
            }
            other => panic!("not a run: {other:?}"),
        }
    }

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

    /// The delete box starts on the safe button; a copy and a rename go to the engine, a rename
    /// onto a net there is does not.
    #[test]
    fn the_tree_deletes_on_ask_and_copies_and_renames_through_the_box() {
        let (tx, rx_req) = mpsc::channel();
        let mut b = quiet(defs());
        b.tx = tx;
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        b.deleting = Some("試験/bom".into());
        b.focus = 1;
        frame(&ctx, size, Vec::new(), &mut b);
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(b.deleting.is_none() && rx_req.try_recv().is_err(), "Enter on the fresh box deletes nothing");
        b.deleting = Some("試験/bom".into());
        b.focus = 0;
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Delete { id }) if id == "試験/bom"));
        b.re_box("試験/bom", Re::Copy);
        assert_eq!(b.make.as_ref().map(|m| m.name.as_str()), Some("bom-2"));
        {
            // The 表示名 follows the file name as it is typed, until it is edited by hand.
            let m = b.make.as_mut().unwrap();
            m.follows = true;
            m.name = "bom-9".into();
        }
        frame(&ctx, size, Vec::new(), &mut b);
        assert_eq!(b.make.as_ref().map(|m| m.title.as_str()), Some("bom-9"));
        b.make.as_mut().unwrap().name = "bom-2".into();
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(b.make.is_none());
        assert!(matches!(rx_req.try_recv(), Ok(Request::Copy { id, to, title: None }) if id == "試験/bom" && to == "試験/bom-2"));
        // A title typed by hand is sent as it is.
        b.re_box("試験/bom", Re::Copy);
        {
            let m = b.make.as_mut().unwrap();
            m.follows = false;
            m.title = "手で書いた".into();
        }
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Copy { title: Some(t), .. }) if t == "手で書いた"));
        b.re_box("試験/bom", Re::Rename);
        b.make.as_mut().unwrap().name = "月次締め".into();
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(b.make.as_ref().is_some_and(|m| m.error.is_some()), "a name there is stays in the box");
        b.make.as_mut().unwrap().name = "新".into();
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Rename { id, to, fix: false }) if id == "試験/bom" && to == "試験/新"));
    }

    /// Kill, approve and delete are asked only when no other box is up, whatever is behind it.
    #[test]
    fn no_question_opens_over_the_versions_box() {
        let mut b = quiet(defs());
        b.vers =
            Some(Vers { id: "試験/bom".into(), list: Vec::new(), idx: 0, texts: BTreeMap::new(), asked: BTreeSet::new(), error: None });
        assert!(b.box_open());
        b.give = None;
        let ctx = egui::Context::default();
        frame(&ctx, egui::vec2(1280.0, 800.0), Vec::new(), &mut b);
        assert!(b.killing.is_none() && b.asking.is_none() && b.deleting.is_none());
    }

    /// A box is not opened over another: an Enter meant for the new one would press the old one.
    #[test]
    fn no_box_opens_over_another() {
        let (tx, rx_req) = mpsc::channel();
        let mut b = quiet(defs());
        b.tx = tx;
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        b.deleting = Some("試験/bom".into());
        b.focus = 1;
        b.import_box();
        b.new_box(None);
        b.re_box("試験/bom", Re::Copy);
        assert!(b.port.is_none() && b.make.is_none(), "the delete box stays alone");
        frame(&ctx, size, Vec::new(), &mut b);
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(rx_req.try_recv().is_err(), "Enter on the safe button deleted nothing");
    }

    /// The status bar's facts: no word while the engine answers, and the unapproved count.
    #[test]
    fn the_status_facts_follow_the_snapshot() {
        let t = ito_i18n::Table::default();
        let mut b = quiet(defs());
        let (word, running, _) = b.engine_facts(&t);
        assert!(word.is_none() && running == 0, "a snapshot and no pause: all well");
        assert_eq!(b.todo_count(), 3, "the three test defs are not approved");
        b.snap.as_mut().unwrap().defs[0].standing = Standing::Confirmed;
        assert_eq!(b.todo_count(), 2, "an approved one is no longer counted");
        b.snap.as_mut().unwrap().defs[1].standing = Standing::Invalid;
        assert_eq!(b.todo_count(), 2, "one that does not read still is");
        b.snap.as_mut().unwrap().paused = true;
        assert!(b.engine_facts(&t).0.is_some(), "paused is a word");
        b.snap = None;
        assert!(b.engine_facts(&t).0.is_some(), "no snapshot: the engine is starting or down");
        assert_eq!(b.todo_count(), 0);
    }

    /// A selection made elsewhere (the today table) unfolds its group in the tree.
    #[test]
    fn a_new_selection_unfolds_its_group() {
        let mut b = quiet(defs());
        let d = defs().remove(0);
        b.folded.insert(d.group.clone());
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        assert!(b.folded.contains(&d.group), "nothing selected: stays folded");
        b.selected = Some(b.snap.as_ref().unwrap().defs[0].id.clone());
        frame(&ctx, size, Vec::new(), &mut b);
        assert!(!b.folded.contains(&b.snap.as_ref().unwrap().defs[0].group));
    }

    /// A press on a group's heading folds its nets away, and another opens them again.
    #[test]
    fn a_press_on_the_group_heading_folds_and_opens_it() {
        let mut b = quiet(defs());
        let mut s = Screen::new();
        s.settle(&mut b);
        assert!(b.folded.is_empty());
        s.click("試験、開いています", &mut b);
        assert!(b.folded.contains("試験"), "folded");
        s.click("試験、折りたたみ中", &mut b);
        assert!(b.folded.is_empty(), "open again");
    }

    /// The three questions are answered with the mouse: the safe button sends nothing, the other
    /// sends its request.
    #[test]
    fn a_press_on_a_question_button_answers_it() {
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
        let mut s = Screen::new();

        b.deleting = Some("試験/bom".into());
        s.settle(&mut b);
        s.click("やめる", &mut b);
        assert!(b.deleting.is_none() && rx_req.try_recv().is_err(), "the safe button deletes nothing");
        b.deleting = Some("試験/bom".into());
        s.settle(&mut b);
        s.click("消す", &mut b);
        assert!(b.deleting.is_none());
        assert!(matches!(rx_req.try_recv(), Ok(Request::Delete { id }) if id == "試験/bom"));

        b.run_sel = Some("r1".into());
        b.killing = Some("r1".into());
        s.settle(&mut b);
        s.click("やめる", &mut b);
        assert!(b.killing.is_none() && rx_req.try_recv().is_err(), "the safe button kills nothing");
        b.killing = Some("r1".into());
        s.settle(&mut b);
        s.click("強制終了する", &mut b);
        assert!(b.killing.is_none());
        assert!(matches!(rx_req.try_recv(), Ok(Request::Kill { .. })));

        let def = b.snap.as_ref().unwrap().defs[0].clone();
        b.selected = Some(def.id.clone());
        b.asking = Some(Approval { id: def.id.clone(), sha256: def.sha256.clone() });
        s.settle(&mut b);
        s.click("確かめた", &mut b);
        assert!(b.asking.is_none());
        assert!(matches!(rx_req.try_recv(), Ok(Request::Approve { id, .. }) if id == def.id));
    }

    /// What a thread off the UI thread tells is put into words where the language table is.
    #[test]
    fn a_done_message_is_worded_when_shown() {
        let mut b = quiet(defs());
        b.io.0.send(Msg::Done(Ok(("run.history_csv_done", vec!["C:/x.csv".to_owned()])))).unwrap();
        let mut s = Screen::new();
        s.settle(&mut b);
        assert!(matches!(&b.result, Some(Ok(m)) if m == "書いた: C:/x.csv"), "{:?}", b.result);
        // A failure is worded the same way.
        b.io.0.send(Msg::Done(Err(("port.no_dir", vec!["C:/e".to_owned(), "denied".to_owned()])))).unwrap();
        s.settle(&mut b);
        assert!(matches!(&b.result, Some(Err(m)) if m == "C:/e を作れない: denied"), "{:?}", b.result);
    }

    /// The 対応済みにする button hands over the run's id only for a run that ended in failure.
    #[test]
    fn a_press_on_ack_marks_only_a_failed_run() {
        use koyomi_core::jobnet::{Job, Jobnet};
        use koyomi_core::run::{How, State};
        let net = Jobnet { jobs: vec![Job { id: "a".into(), run: vec!["x".into()], ..Default::default() }], ..Default::default() };
        let now = chrono::Local::now().naive_local();
        let mut run = Run::new("r1", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.state = State::Error;
        run.ended = Some(now);
        let mut b = quiet(defs());
        b.runs = vec![run];
        b.run_sel = Some("r1".into());
        let mut s = Screen::new();
        s.settle(&mut b);
        // The run view's button row is inside the window, not pushed below it by the tab row.
        let inside = s.nodes.values().find(|n| n.0 == "対応済みにする").is_some_and(|n| n.1.max.y <= s.size.y);
        assert!(inside, "the button is on the screen");
        s.click("対応済みにする", &mut b);
        assert_eq!(b.take_acks(), vec!["r1".to_owned()]);
        assert!(matches!(&b.result, Some(Ok(m)) if m == "対応済みにした"));
        // Dealt with: the button says so and is off.
        s.settle(&mut b);
        assert!(s.nodes.values().any(|n| n.0 == "対応済み"));
        assert!(!s.nodes.values().any(|n| n.0 == "対応済みにする"));
        // A run that ended well: there is nothing to deal with, so no button.
        b.acked.clear();
        b.runs[0].state = State::Ok;
        s.settle(&mut b);
        assert!(!s.nodes.values().any(|n| n.0 == "対応済みにする"));
        assert!(b.take_acks().is_empty());
    }

    /// The net pane's calendar list ends in 新しいカレンダー…, which opens the box for the net
    /// with the editor kept; Escape leaves both as they were.
    #[test]
    fn the_net_pane_makes_a_calendar_for_the_net() {
        let mut b = quiet(defs());
        let id = defs()[0].id.clone();
        b.open_editor(&id);
        b.editor.as_mut().unwrap().open_net();
        let mut s = Screen::new();
        s.settle(&mut b);
        s.click("（なし）", &mut b);
        s.click("新しいカレンダー…", &mut b);
        assert_eq!(b.new_cal.as_ref().map(|m| m.for_net), Some(true));
        assert!(b.editing());
        s.show(key(egui::Key::Escape), &mut b);
        assert!(b.new_cal.is_none() && b.editing());
        assert!(!b.editor.as_ref().unwrap().dirty);
        // From the tree, the same box is not for a net.
        b.new_cal_box();
        assert_eq!(b.new_cal.as_ref().map(|m| m.for_net), Some(false));
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
