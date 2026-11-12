//! The window's left side (the tree, then today's runs and 履歴, then the calendars),
//! the detail beside it (a definition, a run as a picture with its log under it, the
//! runs of past days, or a calendar's month) and the boxes that ask before approving or killing.
//!
//! Nothing here touches a process: a thread keeps the engine up and asks it for the
//! list, the runs of today and the log being watched, and the window reads what that
//! thread sent. An approval, a start and a kill are sent the same way.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ito_a11y::Role;
use ito_common::kept;
use koyomi_core::run::{Run, State};
use koyomi_core::scan::{Def, Standing};
use serde::{Deserialize, Serialize};

use crate::link::{Conn, Link};
use crate::proto::{self, Request, Runs, Snapshot};
use crate::{engine, review, show};

pub mod calendar;
pub mod canvas;
pub mod far;
pub mod gantt;
pub mod graph;
pub mod whens;

mod far_config;
mod ha;
mod hosts;
mod manage;
mod net_side;
pub(crate) mod paths;
mod users;

/// How long a window waits to ask another PC again after its list could not be read.
const FAR_RETRY: Duration = Duration::from_secs(10);
/// How often it is asked which PC runs the schedules (`[standby]` or `[primary]`).
const HA_POLL: Duration = Duration::from_secs(30);

/// What the thread is asked to do.
enum Job {
    Ask(Request),
    /// A change to one calendar: the text is fetched first, the change is made on what was fetched, and the
    /// write carries the hash of what was fetched. It is the same path for this PC and another.
    Calendar {
        name: String,
        change: CalendarChange,
    },
}

/// What a calendar edit does to the text the engine has: the new text, or why it cannot.
pub(super) type EditFn = Box<dyn FnOnce(&str) -> Result<String, String> + Send>;

/// What a calendar job does to the fetched text.
pub(super) enum CalendarChange {
    /// Makes the new text from the fetched one; a failure writes nothing.
    Edit(EditFn),
    Delete,
}

/// The words every failure before the write ends with: nothing was changed. A coded line goes through as it is,
/// so its ID stays at the head; it comes from before the write, so nothing was written either way.
fn unchanged(why: &str) -> String {
    if koyomi_core::msg::is_coded(why) {
        why.to_owned()
    } else {
        koyomi_core::msg::note(koyomi_core::msg::N_CALENDAR_UNCHANGED, &[why])
    }
}

/// One calendar change: ask for the calendar's text, change it, and write it back with the hash it was read
/// at. Any failure before the write says nothing was changed. The write's own answer comes back as it is: an
/// engine refusal verbatim, a broken connection with its 「届いたか分からない」. Never retried on its own.
fn calendar_job(name: &str, change: CalendarChange, call: &dyn Fn(&Request) -> Result<String, String>) -> Result<String, String> {
    let body =
        call(&Request::CalendarText { name: name.to_owned() }).and_then(|t| proto::CalendarBody::parse(&t)).map_err(|e| unchanged(&e))?;
    match change {
        CalendarChange::Edit(f) => {
            let text = f(&body.text).map_err(|e| unchanged(&e))?;
            call(&Request::CalendarPut { name: name.to_owned(), text, base_sha256: Some(body.sha256) })
        }
        CalendarChange::Delete => call(&Request::CalendarDelete { name: name.to_owned(), base_sha256: body.sha256 }),
    }
}

/// One request that is in the worker's hands and is not a read: it counts while it is alive, and it is
/// dropped by the worker after its answer was sent, or with the envelope when it was never taken.
/// A switch of the link waits until none is left.
struct Pending(Arc<AtomicUsize>);

impl Pending {
    fn new(count: &Arc<AtomicUsize>) -> Pending {
        count.fetch_add(1, Ordering::SeqCst);
        Pending(count.clone())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The envelope the window sends to the thread: the job and, for a request that is not a read, its count.
struct Work {
    job: Job,
    pending: Option<Pending>,
}

/// The window's side of the thread's queue. It is its own field so that a request can be sent while another
/// field of the browser is borrowed.
struct Outbox {
    tx: Sender<Work>,
    /// How many requests that are not reads the thread has not finished.
    pending: Arc<AtomicUsize>,
    /// A switch of the link is waiting for the pending requests: no write is sent (a read still is, and is not counted).
    switching: Cell<bool>,
    /// A request was refused because of `switching`; the result line says so once.
    refused: Cell<bool>,
}

impl Outbox {
    /// Whether the request was sent (`false`: refused while a switch of the link waits).
    fn send(&self, request: Request) -> bool {
        if self.switching.get() && !request.is_read() {
            self.refused.set(true);
            return false;
        }
        let pending = (!request.is_read()).then(|| Pending::new(&self.pending));
        // A failed send gives the envelope back inside the error, and its drop takes the count off again.
        self.tx.send(Work { job: Job::Ask(request), pending }).is_ok()
    }

    /// A calendar job is never a read: it always counts, and it is refused while a switch waits.
    fn send_calendar(&self, name: String, change: CalendarChange) -> bool {
        if self.switching.get() {
            self.refused.set(true);
            return false;
        }
        let pending = Some(Pending::new(&self.pending));
        self.tx.send(Work { job: Job::Calendar { name, change }, pending }).is_ok()
    }
}

/// This PC's offset from UTC now, in minutes (what the engine's `utc_offset_min` is compared with).
fn local_utc_offset_min() -> i32 {
    ito_common::now::now_local().offset().local_minus_utc() / 60
}

/// The wall clock of a PC whose UTC offset is `offset_min`, when it is `now_utc` (UTC).
fn now_at(now_utc: chrono::DateTime<chrono::Utc>, offset_min: i32) -> chrono::NaiveDateTime {
    (now_utc + chrono::Duration::minutes(i64::from(offset_min))).naive_utc()
}

/// The request that saves the editor's text: `edit` may only propose (it waits for a confirmation in the
/// engine's own window), `admin` and this PC save.
fn save_request(role: Option<koyomi_core::role::Role>, id: String, text: String, base_sha256: Option<String>, force: bool) -> Request {
    if role == Some(koyomi_core::role::Role::Edit) {
        Request::Propose { id, text, base_sha256 }
    } else {
        Request::Save { id, text, base_sha256, force }
    }
}

/// The forced save the conflict box's third button (index 2) asks for. Another PC's engine takes none.
fn force_save(far: bool, answer: usize, id: &str, text: String, base_sha256: Option<String>) -> Option<Request> {
    (answer == 2 && !far).then(|| Request::Save { id: id.to_owned(), text, base_sha256, force: true })
}

/// The conflict box's body: on another PC's engine there is no overwrite button, so its text does not offer one.
fn conflict_body(far: bool) -> String {
    conflict_body_in(&koyomi_core::msg::language(), far)
}

/// [`conflict_body`] in the language `code`.
fn conflict_body_in(code: &str, far: bool) -> String {
    let id = if far { koyomi_core::msg::CONFLICT_FAR } else { koyomi_core::msg::CONFLICT_NEAR };
    koyomi_core::msg::say_in(code, id, &[]).line()
}

/// The list of jobnets that could not be fetched: a coded cause goes through, a bare one is told as the list's.
fn list_unavailable(e: &str) -> String {
    if koyomi_core::msg::is_coded(e) {
        e.to_owned()
    } else {
        koyomi_core::msg::line(koyomi_core::msg::LIST_UNAVAILABLE, &[e])
    }
}

/// The request that stages a net for a day, with what the window saw: the file it started from (`base`) and the
/// staged one over it, if any. Without a `base` the net was never saved, so there is nothing to switch from.
fn stage_request(id: &str, base: Option<String>, staged: Option<String>, text: String, from: String) -> Result<Request, String> {
    match base {
        Some(live) => Ok(Request::Stage { id: id.to_owned(), text, from, expect: Some(proto::StageExpect { live, staged }) }),
        None => Err(koyomi_core::msg::line(koyomi_core::msg::STAGE_NO_LIVE, &[id])),
    }
}

/// The link `state.toml` keeps: this PC's when it keeps none (or no file).
pub fn kept_link(state: Option<&std::path::Path>) -> Link {
    let kept: Kept = state.and_then(|d| kept::read(&d.join("state.toml"))).unwrap_or_default();
    kept.link.filter(|n| !n.trim().is_empty()).map_or(Link::Local, Link::Far)
}

/// Whether the rest of a round is skipped after the list could not be read, and how long the next wait is.
/// Another PC is left alone for a while; this PC's engine is asked for the rest as it always was.
fn after_list_failure(link: &Link) -> Option<Duration> {
    link.is_far().then_some(FAR_RETRY)
}

/// One job is answered, and only then is its envelope dropped (the window counts on that order). Whether the
/// window is still there to hear it comes back.
fn answer(work: Work, reply: &dyn Fn(&Request) -> Result<String, String>, send: &dyn Fn(Msg) -> bool) -> bool {
    let Work { job, pending } = work;
    let alive = match job {
        Job::Ask(request) => {
            let r = reply(&request);
            send(Msg::Answer(request, r))
        }
        Job::Calendar { name, change } => send(Msg::CalendarDone(calendar_job(&name, change, reply))),
    };
    drop(pending);
    alive
}

/// The thread: keeps the link up, answers the window's requests and asks for the list, the runs, the log being
/// watched and the handover. It ends when the window drops its side of a channel.
fn worker(link: &Link, rx_req: &Receiver<Work>, tx_msg: &Sender<Msg>, ctx: &egui::Context, seen: &Watch) {
    let send = |m: Msg| {
        let alive = tx_msg.send(m).is_ok();
        ctx.request_repaint();
        alive
    };
    let poll = link.poll();
    let mut conn: Option<Conn> = None;
    let mut wait = poll;
    let mut ha_at: Option<std::time::Instant> = None;
    loop {
        if conn.as_ref().is_none_or(|c| !c.running()) {
            conn = None;
            if !link.is_far() && !send(Msg::Starting) {
                return;
            }
            match link.start() {
                Ok(c) => {
                    if !send(Msg::Role(c.role(), c.who().map(str::to_owned))) {
                        return;
                    }
                    conn = Some(c);
                }
                Err(e) => {
                    for work in rx_req.try_iter() {
                        if !answer(work, &|_| Err(e.clone()), &send) {
                            return;
                        }
                    }
                    if !send(Msg::Down(e.clone())) {
                        return;
                    }
                    match rx_req.recv_timeout(after_list_failure(link).unwrap_or(poll * 3)) {
                        Ok(work) => {
                            if !answer(work, &|_| Err(e.clone()), &send) {
                                return;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                    continue;
                }
            }
        }
        let Some(c) = conn.as_ref() else { continue };
        match rx_req.recv_timeout(wait) {
            Ok(work) => {
                if !answer(work, &|r| c.call(r), &send) {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        wait = poll;
        let listed = c.call(&Request::List).and_then(|t| Snapshot::parse(&t));
        let failed = listed.is_err();
        let m = match listed {
            Ok(s) => Msg::Snapshot(Box::new(s)),
            Err(e) => Msg::Down(e),
        };
        if !send(m) {
            return;
        }
        if failed {
            if let Some(w) = after_list_failure(link) {
                // The next round connects afresh, so a new token, certificate or address and the role are read again.
                conn = None;
                wait = w;
                continue;
            }
        }
        if let Ok(r) = c.call(&Request::Runs).and_then(|t| Runs::parse(&t)) {
            if !send(Msg::Runs(r)) {
                return;
            }
        }
        let target = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some((run, job)) = target {
            if let Ok(text) = c.call(&Request::Log { run: run.clone(), job: job.clone() }) {
                if !send(Msg::Log(run, job, text)) {
                    return;
                }
            }
        }
        if ha_at.is_none_or(|at| at.elapsed() >= HA_POLL) {
            ha_at = Some(std::time::Instant::now());
            let ha = c.call(&Request::Handover { release: false }).ok().and_then(|t| serde_json::from_str(&t).ok());
            if !send(Msg::Ha(ha)) {
                return;
            }
        }
    }
}

enum Msg {
    Starting,
    /// The caller's role and name at the other PC (`None` for this one), once the link started (every start, a
    /// reconnect too).
    Role(Option<koyomi_core::role::Role>, Option<String>),
    Snapshot(Box<Snapshot>),
    Runs(Runs),
    /// The tail of one job's log: run, job, text.
    Log(String, String, String),
    Down(String),
    /// The engine's answer to a request the window sent.
    Answer(Request, Result<String, String>),
    /// The engine's answer to a calendar job: its own text, or why nothing was changed.
    CalendarDone(Result<String, String>),
    /// File work done off the UI thread: a line for under the detail, or why not.
    Done(Result<Said, Said>),
    /// The same, for work that made a file on this PC: the line, and the file's path (told with buttons).
    DoneAt(Result<(Said, PathBuf), Said>),
    /// The file the 取り込む box was told to read: its text, or why it could not be read.
    PortText(String, Result<String, String>),
    /// What a `[[remote]]` PC has of the net 別の PC に送る asked for: PC, net, what it has.
    There(String, String, Result<Option<crate::remote::There>, String>),
    /// The engine's `"ha"`, every `HA_POLL`: `None` without `[standby]` or `[primary]`.
    Ha(Option<serde_json::Value>),
}

pub struct Approval {
    id: String,
    sha256: String,
}

/// 別の PC に送る (TODO 111): this PC's net, the PC it goes to, and what that PC has of it once
/// read (`Some(None)`: no such net there). The box is up once it is read.
struct Sending {
    id: String,
    pc: String,
    text: String,
    there: Option<Option<crate::remote::There>>,
}

/// The calls 別の PC に送る makes, so a test can stand in for the other PC.
#[derive(Clone, Copy)]
struct Wire {
    there: fn(&str, &str) -> Result<Option<crate::remote::There>, String>,
    propose: fn(&str, &str, &str, Option<&str>) -> Result<bool, String>,
}

impl Default for Wire {
    fn default() -> Self {
        Wire { there: crate::remote::there, propose: crate::remote::propose }
    }
}

/// The 取り込む box: a path to type, then the engine's plan for that file to confirm.
struct Port {
    path: String,
    /// The file's text once it was read: the plan is about it.
    text: Option<String>,
    plan: Option<proto::Plan>,
    error: Option<String>,
    /// The path last read: `error` is about this file, whatever the field holds now.
    read: Option<String>,
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

/// One net of the 次の 1 回だけスキップ box: its next time not yet skipped, and the times it skips.
struct SkipRow {
    id: String,
    name: String,
    next: Option<chrono::NaiveDateTime>,
    skipped: Vec<chrono::NaiveDateTime>,
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
    /// `false` once the inspector at the right of 今日 was folded.
    #[serde(default)]
    inspector: Option<bool>,
    /// `false` once the pane at the right of 履歴 was folded.
    #[serde(default)]
    history_pane: Option<bool>,
    /// `false` once the pane at the right of the calendar was folded.
    #[serde(default)]
    calendar_pane: Option<bool>,
    /// The calendar's view: `"day"`, `"week"` or `"month"`.
    #[serde(default)]
    calendar_view: Option<String>,
    /// `true` once the calendar was switched to its week, as files before `calendar_view` said.
    #[serde(default, skip_serializing)]
    calendar_week: Option<bool>,
    /// `false` once the pane at the right of the net page was folded.
    #[serde(default)]
    net_pane: Option<bool>,
    /// The `[[remote]]` name of the PC the window is connected to; none is this PC.
    #[serde(default)]
    link: Option<String>,
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
#[derive(Clone)]
struct History {
    from: String,
    to: String,
    /// The net the rows are narrowed to; empty for all of them.
    net: String,
    state: Option<State>,
    /// The column the rows are sorted by, and whether down.
    sort: Option<(usize, bool)>,
    rows: Vec<Run>,
    /// Why there are no rows: a date that does not read, or the engine's word.
    error: Option<String>,
}

impl History {
    /// The last 7 days up to `today`, nothing narrowed.
    fn week(today: chrono::NaiveDate) -> History {
        let from = today - chrono::Duration::days(7);
        History {
            from: from.to_string(),
            to: today.to_string(),
            net: String::new(),
            state: None,
            sort: None,
            rows: Vec::new(),
            error: None,
        }
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

/// How many places 戻る keeps; the oldest goes first.
const BACK_MAX: usize = 20;

/// Where the detail was before a link left it: the view and what was picked on it, so 戻る comes
/// back to the same row.
#[derive(Clone)]
struct Place {
    gantt: bool,
    hist: Option<History>,
    selected: Option<String>,
    nets_pick: Option<String>,
    run_sel: Option<String>,
    today_net: Option<String>,
    job_sel: Option<String>,
    cal_sel: Option<String>,
    cal_month: Option<(i32, u32)>,
    cal_day: Option<chrono::NaiveDate>,
}

impl Place {
    /// The same view with the same row picked; a history's rows aside, as the engine sends them again.
    fn same(&self, o: &Place) -> bool {
        let hist = match (&self.hist, &o.hist) {
            (None, None) => true,
            (Some(a), Some(b)) => a.from == b.from && a.to == b.to && a.net == b.net && a.state == b.state,
            _ => false,
        };
        hist && self.gantt == o.gantt
            && self.selected == o.selected
            && self.nets_pick == o.nets_pick
            && self.run_sel == o.run_sel
            && self.today_net == o.today_net
            && self.job_sel == o.job_sel
            && self.cal_sel == o.cal_sel
            && self.cal_month == o.cal_month
            && self.cal_day == o.cal_day
    }
}

/// What a thread off the UI thread tells: the key of the words and what goes into them, formatted
/// where the language table is.
type Said = (&'static str, Vec<String>);

/// A tab of the row over the detail.
#[derive(Clone, Copy)]
enum Tab {
    Net,
    Calendar,
    Today,
    History,
}

pub struct Browser {
    rx: Receiver<Msg>,
    /// Where threads started by [`Browser::background`] report, and the window to wake.
    io: (Sender<Msg>, egui::Context),
    /// What [`Msg::Done`] brought, until [`Browser::show`] has the words for it.
    done: Option<Result<Said, Said>>,
    /// The file [`Msg::DoneAt`] brought, taken with `done`.
    done_path: Option<PathBuf>,
    out: Outbox,
    /// The link the person picked, until [`Browser::take_link_wish`].
    want_link: Option<Link>,
    /// Which engine the window talks to.
    link: Link,
    /// The caller's role at the other PC; `None` for this one.
    role: Option<koyomi_core::role::Role>,
    /// The caller's name at the other PC (`/me`'s `who`); `None` for this one or when it told none.
    who: Option<String>,
    /// The 利用者 page of the settings. Its state is here, so a switch of the link empties it.
    users: users::Users,
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
    /// The file the result line is about, with the line's text; shown only while that line is the result.
    result_path: Option<paths::Made>,
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
    /// The engine's runs were taken once (`has_runs`).
    runs_known: bool,
    /// Connections whose URL moved, until [`Browser::take_moved_urls`] (the watch connects to them afresh).
    moved_urls: Vec<String>,
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
    /// The day picked in the month, whose starts show beside it; today when none is.
    cal_day: Option<chrono::NaiveDate>,
    /// What the last click on a day did to the file, shown under the month.
    cal_result: Option<Result<String, String>>,
    /// A day's name or a holiday table's path being typed under the month.
    cal_inline: Option<calendar::Inline>,
    /// The holiday table's file chooser while it is open, with the name of the calendar it was opened for. The OS
    /// chooser outlives the field, so it is kept here; its answer (`None` for a cancel) comes on the channel.
    cal_picking: Option<(String, std::sync::mpsc::Receiver<Option<PathBuf>>)>,
    /// The runs of the last 30 days, for the average time of a net, and the day they were asked for.
    past: std::collections::HashMap<String, chrono::Duration>,
    past_asked: Option<chrono::NaiveDate>,
    /// When the last ask failed: it is not repeated at every snapshot.
    past_failed: Option<std::time::Instant>,
    /// 履歴, when it is open: the detail is its rows.
    hist: Option<History>,
    /// ガント, when it is open: the detail is today's runs on a time axis.
    gantt: bool,
    /// The today table's chip that is on: only its rows show.
    today_kind: Option<TodayKind>,
    /// The net of the row picked on 今日 (a start still to come has no run); the inspector shows it.
    today_net: Option<String>,
    /// The inspector at the right of 今日 is open (not folded to its strip).
    inspector: bool,
    /// The pane at the right of 履歴 is open (not folded to its strip).
    hist_pane: bool,
    /// 履歴 was opened on a far link before its first snapshot: the days are the engine's, so the week is built and
    /// asked when the snapshot has come.
    hist_waiting: bool,
    /// The pane at the right of the calendar is open (not folded to its strip).
    cal_pane: bool,
    /// The calendar shows the picked day, its week or its month.
    cal_view: calendar::CalView,
    /// The pane at the right of the net page is open (not folded to its strip).
    net_pane: bool,
    /// The today table's sort: the column and whether it goes down. `None`: still to come, then newest first.
    today_sort: Option<(usize, bool)>,
    /// The jobs table's sort on the net page, as `today_sort`.
    net_sort: Option<(usize, bool)>,
    /// The run's jobs table's sort, as `net_sort` (`None`: the run's own order).
    run_sort: Option<(usize, bool)>,
    /// The nets overview's sort (the ジョブネット tab with no net picked), as `today_sort`.
    nets_sort: Option<(usize, bool)>,
    /// The net picked (not opened) in the nets overview by a single click; the tree marks it too.
    nets_pick: Option<String>,
    /// Where each link came from, newest last: 戻る goes to the last one. A tab or a pick in the tree
    /// forgets them.
    back: Vec<Place>,
    /// 戻る just came back: the picked row is scrolled into sight on the next frame.
    reveal: bool,
    /// The troubleshooting card is open over the page: the page takes no key, and no box takes egui's focus
    /// from the card's field. Set by the window every frame.
    pub under_card: bool,
    /// The command palette is open over the page: no box takes egui's focus from its field. Set by the window every
    /// frame.
    pub under_palette: bool,
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
    /// The 次の 1 回だけスキップ box the tray opens, while it is up.
    skip_pick: bool,
    /// The 次の定義 box: the net whose staged definition it shows.
    staged_box: Option<String>,
    /// 別の PC に送る, from the pick of the PC until its box is answered.
    sending: Option<Sending>,
    wire: Wire,
    /// The editors a child net was entered from, outermost first.
    editors: Vec<canvas::Editor>,
    /// The `[[remote]]` names and the add and manage boxes (the status-bar list drives the window on another PC).
    far: far::Far,
    /// The box that edits the koyomi.toml of the PC this window shows (an `admin` on another PC's engine).
    config_edit: Option<far_config::ConfigEdit>,
    /// The list of links in the status bar.
    hosts: hosts::Hosts,
    /// The engine's `"ha"` (the status bar's HA words).
    ha: Option<serde_json::Value>,
}

impl Browser {
    /// Starts the thread. `state` is the state folder for `state.toml`; `None` keeps nothing.
    pub fn start(ctx: &egui::Context, state: Option<PathBuf>, link: Link, writer: Option<kept::Writer>) -> Browser {
        let (tx_msg, rx) = mpsc::channel::<Msg>();
        let (tx, rx_req) = mpsc::channel::<Work>();
        let pending: Arc<AtomicUsize> = Arc::default();
        let watch: Watch = Arc::default();
        let seen = watch.clone();
        let io = (tx_msg.clone(), ctx.clone());
        let ctx = ctx.clone();
        let thread_link = link.clone();
        std::thread::spawn(move || worker(&thread_link, &rx_req, &tx_msg, &ctx, &seen));
        let path = state.as_ref().map(|d| d.join("state.toml"));
        let kept: Kept = path.as_deref().and_then(kept::read).unwrap_or_default();
        let vertical = kept.flow.as_deref() == Some("vertical");
        let inspector = kept.inspector != Some(false);
        let (hist_pane, cal_pane) = (kept.history_pane != Some(false), kept.calendar_pane != Some(false));
        let cal_view = calendar::CalView::kept(kept.calendar_view.as_deref(), kept.calendar_week);
        let net_pane = kept.net_pane != Some(false);
        let flow = writer;
        let far = far::Far::start(&io.1, state);
        Browser {
            far,
            config_edit: None,
            hosts: hosts::Hosts::new(),
            rx,
            io,
            want_link: None,
            out: Outbox { tx, pending, switching: Cell::new(false), refused: Cell::new(false) },
            link,
            role: None,
            who: None,
            users: users::Users::default(),
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
            result_path: None,
            over: None,
            ended: Vec::new(),
            to_ack: Vec::new(),
            acked: BTreeSet::new(),
            done: None,
            done_path: None,
            seen_notes: BTreeSet::new(),
            notes: Vec::new(),
            runs_known: false,
            moved_urls: Vec::new(),
            autostart_off: false,
            band_seen: None,
            cal_sel: None,
            folded: BTreeSet::new(),
            tree_seen: None,
            cal_month: None,
            cal_day: None,
            cal_result: None,
            cal_inline: None,
            cal_picking: None,
            hist: None,
            gantt: true,
            today_kind: None,
            today_net: None,
            inspector,
            hist_pane,
            hist_waiting: false,
            cal_pane,
            cal_view,
            net_pane,
            today_sort: None,
            net_sort: None,
            run_sort: None,
            nets_sort: None,
            nets_pick: None,
            back: Vec::new(),
            reveal: false,
            under_card: false,
            under_palette: false,
            vers: None,
            editor: None,
            editors: Vec::new(),
            port: None,
            make: None,
            new_cal: None,
            cal_deleting: None,
            give: None,
            skip_pick: false,
            staged_box: None,
            sending: None,
            wire: Wire::default(),
            easy: None,
            ha: None,
        }
    }

    /// Which engine the window talks to.
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// The other PC's name and the caller's role there, for the title and the status bar; `None` on this PC.
    pub fn link_label(&self) -> Option<String> {
        let name = self.link.far()?;
        Some(match self.role {
            Some(r) => format!("{name} ({r})"),
            None => name.to_owned(),
        })
    }

    /// How many requests that are not reads the thread has not finished.
    pub fn pending(&self) -> usize {
        self.out.pending.load(Ordering::SeqCst)
    }

    /// One look at a switch of the link that is waiting: from now on no write is sent (see [`Outbox::send`]),
    /// and how many requests are still in the thread's hands is returned (`0`: the switch may go on). The count
    /// is read before the thread's answers are taken: an answer is sent before its request stops counting, so a
    /// zero means every answer is in the channel, and the drain here has them.
    pub fn switch_step(&mut self) -> usize {
        self.out.switching.set(true);
        let left = self.pending();
        self.drain();
        left
    }

    /// 「やめる」: sends again.
    pub fn cancel_switch(&self) {
        self.out.switching.set(false);
    }

    /// Whether an editor holds changes that are not saved, or a new token is on the screen (it is shown once).
    pub fn has_unsaved(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.dirty)
            || self.editors.iter().any(|e| e.dirty)
            || self.config_edit.as_ref().is_some_and(far_config::ConfigEdit::dirty)
            || self.users.unsaved()
    }

    /// Whether a new token is on the screen: a switch of the link would lose it, and its question says so.
    pub fn holds_token(&self) -> bool {
        self.users.holds_token()
    }

    /// How many times the token box has opened. A switch that waits for a write notes it at its start: if it moved,
    /// a token came during the wait and the switch is called off.
    pub fn tokens_opened(&self) -> u64 {
        self.users.tokens_opened()
    }

    /// Whether a token box opened since `started` (what [`Browser::tokens_opened`] said when the switch began): the
    /// switch is then called off and the box stays.
    pub fn switch_called_off(&self, started: u64) -> bool {
        self.users.tokens_opened() != started
    }

    /// A question, the add form or an editor of the users page is up: its Esc is for them, not for the settings.
    pub fn users_wants_esc(&self) -> bool {
        self.users.wants_esc()
    }

    /// Whether the unsaved changes are only in the koyomi.toml box: the switch's question then names that box.
    pub fn unsaved_is_config(&self) -> bool {
        !(self.editor.as_ref().is_some_and(|e| e.dirty) || self.editors.iter().any(|e| e.dirty))
            && !self.users.forms_dirty()
            && self.config_edit.as_ref().is_some_and(far_config::ConfigEdit::dirty)
    }

    /// The link the person picked in a picker, once.
    pub fn take_link_wish(&mut self) -> Option<Link> {
        self.want_link.take()
    }

    /// Writes `state.toml` (the link included).
    pub fn save_state(&self) {
        self.save_kept();
    }

    /// The link button and its list: this PC and every `[[remote]]`. A pick is taken by [`Browser::take_link_wish`].
    /// `facts` is what the watch found out about each connection, `unwatched` the names it does not watch. Returns whether the list was opened just now.
    pub fn link_picker(
        &mut self,
        ui: &mut egui::Ui,
        t: &ito_i18n::Table,
        facts: &std::collections::HashMap<String, crate::watch::Fact>,
        unwatched: &std::collections::BTreeSet<String>,
    ) -> bool {
        let out = self.hosts.show(ui, t, &self.link, self.down.is_some(), self.far.names(), facts, unwatched);
        if out.manage && !self.own_box_open() {
            self.far.open_manage();
        }
        // A change to koyomi.toml is being carried out: the link stays where it is until it is done.
        if let Some(l) = out.pick.filter(|l| *l != self.link).filter(|_| !self.far.manage_busy()) {
            self.far.close_manage();
            self.want_link = Some(l);
        }
        out.opened
    }

    /// Hands a request to the thread. A request that is not a read is counted until the thread has finished
    /// it. While a switch of the link waits, a write is refused (and said so); a read still goes, and its answer
    /// goes to this browser, which goes away with the switch.
    fn send(&self, request: Request) -> bool {
        self.out.send(request)
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

    /// The engine's wall clock, for what the window means by now: on a far link with a snapshot, this PC's UTC clock
    /// moved by the offset the last snapshot carried; otherwise this PC's own clock. It promises no more than that:
    /// a clock that is off between the two PCs, or a change of summer time on the engine's side while no snapshot
    /// came, is not corrected. Before the first snapshot of a far link the offset is not known and this PC's clock
    /// is used, which is fine for what is only drawn (it is drawn again when the snapshot comes) but not for a date
    /// that goes into a request: see [`Browser::engine_date_known`].
    pub(crate) fn engine_now(&self) -> chrono::NaiveDateTime {
        match self.far_offset() {
            Some(offset) => now_at(ito_common::now::now(), offset),
            None => ito_common::now::now_local().naive_local(),
        }
    }

    /// The engine's date: see [`Browser::engine_now`].
    pub(crate) fn engine_today(&self) -> chrono::NaiveDate {
        self.engine_now().date()
    }

    /// The engine's UTC offset when the window is on another PC and a snapshot has told it.
    fn far_offset(&self) -> Option<i32> {
        if self.link.is_far() {
            self.snap.as_ref().map(|s| s.utc_offset_min)
        } else {
            None
        }
    }

    /// Whether the engine's date is known: always on this PC, and on a far link once the first snapshot has come.
    fn engine_date_known(&self) -> bool {
        !self.link.is_far() || self.snap.is_some()
    }

    /// Asks the engine for the last 30 days once a day. On a far link nothing is asked before the first snapshot:
    /// the date that goes into the request is the engine's.
    fn ask_past(&mut self) {
        if !self.engine_date_known() {
            return;
        }
        let today = self.engine_today();
        if self.past_asked != Some(today) && self.past_failed.is_none_or(|at| at.elapsed() > Duration::from_secs(60)) {
            self.past_asked = Some(today);
            let (from, to) = Self::past_range(today);
            self.send(Request::History { from, to });
        }
    }

    /// Whether the window shows another PC's engine as an `admin`: the one role that may edit its koyomi.toml.
    pub fn far_admin(&self) -> bool {
        self.link.is_far() && self.role == Some(koyomi_core::role::Role::Admin)
    }

    /// Whether the window shows another PC's engine and a role that is not `admin` has been told.
    pub fn far_not_admin(&self) -> bool {
        self.link.is_far() && self.role.is_some_and(|r| r != koyomi_core::role::Role::Admin)
    }

    /// Whether the box that edits the other PC's koyomi.toml is open.
    pub fn config_open(&self) -> bool {
        self.config_edit.is_some()
    }

    /// Asks the engine for the users (a read). A request the thread did not take puts the page into its error line.
    fn users_ask(&mut self) {
        self.users.ask();
        if !self.out.send(Request::Users) {
            let why = self.unsent_why();
            self.users.unsent(true, why);
        }
    }

    /// Asks again at once when the page is open and may ask; else the page asks when it is drawn.
    fn users_reask(&mut self) {
        let frame = self.io.1.cumulative_frame_nr();
        // A `down` that is there is not a reason to wait: the reconnect that brought a new role or name is the end of it.
        let open = users::access(self.link.is_far(), self.role, None) == users::Access::Open;
        if self.users.need && open && self.users.is_open(frame) {
            self.users_ask();
        }
    }

    /// The users page of the settings.
    pub fn users_page(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        self.users.touch(ui.ctx().cumulative_frame_nr());
        let access = users::access(self.link.is_far(), self.role, self.down.as_deref());
        let open = access == users::Access::Open;
        if self.users.need && open {
            self.users_ask();
        }
        let (far, who) = (self.link.far().map(str::to_owned), self.who.clone());
        let view = users::View { far: far.as_deref(), who: who.as_deref(), open };
        let Some(click) = users::show(&mut self.users, ui, t, &view, &access) else {
            return;
        };
        match self.users.act(click, &view) {
            users::Do::Nothing => {}
            users::Do::Ask => self.users_ask(),
            users::Do::Send(request) => {
                if !self.out.send(request) {
                    let why = self.unsent_why();
                    self.users.unsent(false, why);
                }
            }
        }
    }

    /// Draws the box that shows a new token (if there is one) over the whole window. It stays when the settings
    /// close or another page is picked, until 閉じる or Esc.
    pub fn token_box(&mut self, ctx: &egui::Context, t: &ito_i18n::Table) {
        let Some(issued) = self.users.token() else {
            return;
        };
        if users::token_show(issued, ctx, t) {
            self.users.close_token();
        }
    }

    /// Opens the box that edits the other PC's koyomi.toml and asks for its text.
    pub fn open_config_edit(&mut self) {
        if self.config_edit.is_none() && self.far_admin() {
            self.config_edit = Some(far_config::ConfigEdit::new());
            self.config_read();
        }
    }

    /// Asks for the other PC's koyomi.toml; a request the thread did not take puts the box into Failed with why.
    fn config_read(&mut self) {
        if !self.out.send(Request::ConfigText) {
            let why = self.unsent_why();
            if let Some(c) = &mut self.config_edit {
                c.unsent(why);
            }
        }
    }

    /// Why a request was not taken: the connection's own failure if it has one, else the words of a refused write.
    fn unsent_why(&self) -> String {
        self.down.clone().unwrap_or_else(|| koyomi_core::msg::line(koyomi_core::msg::LINK_SWITCHING, &[]))
    }

    /// Draws the koyomi.toml box (if open) and carries out what it asks for.
    pub fn config_box(&mut self, ctx: &egui::Context, t: &ito_i18n::Table) {
        let Some(c) = &mut self.config_edit else {
            return;
        };
        match far_config::show(c, ctx, t) {
            far_config::Act::Nothing => {}
            far_config::Act::Close => self.config_edit = None,
            far_config::Act::Read => self.config_read(),
            far_config::Act::Write { text, base } => {
                if !self.out.send(Request::ConfigPut { text, base_sha256: base }) {
                    let why = self.unsent_why();
                    if let Some(c) = &mut self.config_edit {
                        c.unsent(why);
                    }
                }
            }
        }
    }

    /// Opens the box that adds a `[[remote]]` (the command palette uses it).
    pub fn add_remote_box(&mut self) {
        if self.own_box_open() {
            return;
        }
        self.far.open_add();
    }

    /// Opens the box that renames and deletes a `[[remote]]` (the command palette uses it).
    pub fn manage_remotes_box(&mut self) {
        if self.own_box_open() {
            return;
        }
        self.far.open_manage();
    }

    /// The reconnect after the URL of the current connection moved: when this browser is on the same link as `old`,
    /// the manage box of `old` comes along (see [`far::Far::carry_manage_from`]), so the outcome is still read.
    /// A different link, or a box that is busy, leaves it with `old`, which goes.
    /// On any switch, what `old` has for the app and the app has not taken yet comes along: the connections whose
    /// URL moved (the watch connects to them afresh) and a write of koyomi.toml (the prefs read it again). The
    /// switch can come in the frame after the drain that found them, before the app takes them.
    pub fn keep_from(&mut self, old: &mut Browser) {
        self.moved_urls.append(&mut old.moved_urls);
        self.far.carry_written_from(&mut old.far);
        if self.link == old.link {
            self.far.carry_manage_from(&mut old.far);
        }
    }

    pub fn drain(&mut self) {
        self.far.drain();
        if let Some(name) = self.far.take_joined() {
            // The one way the window changes its link: the pick is taken by the app.
            self.want_link = Some(Link::Far(name));
        }
        if let Some(name) = self.far.take_moved() {
            self.moved_urls.push(name.clone());
            // The window is on the connection whose URL moved: it reconnects at the new one. The box stays, so the
            // outcome (and the reconnecting) is still read: `switch_link` carries it into the new browser (`keep_from`).
            if self.link.far().is_some_and(|l| l.trim() == name.trim()) {
                self.want_link = Some(Link::Far(name));
                if let Some(Ok(line)) = self.far.manage.as_mut().and_then(|m| m.line.as_mut()) {
                    line.push(' ');
                    line.push_str(&koyomi_core::msg::note(koyomi_core::msg::N_REMOTE_URL_RECONNECT, &[]));
                }
            }
        }
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Starting => self.starting = true,
                Msg::Role(r, who) => {
                    self.role = r;
                    self.who.clone_from(&who);
                    // What the users page holds was read under the earlier role and name.
                    if self.users.role_told((r, who)) {
                        self.users_reask();
                    }
                }
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
                    self.snap = Some(*s);
                    // After the snapshot is kept: on a far link the day asked for is the engine's.
                    self.ask_past();
                    self.history_when_known();
                }
                Msg::Runs(r) => {
                    let now: BTreeSet<String> = r.runs.iter().filter(|r| r.is_over()).map(|r| r.id.clone()).collect();
                    if let Some(before) = &self.over {
                        self.ended.extend(r.runs.iter().filter(|r| r.is_over() && !before.contains(&r.id)).cloned());
                    }
                    self.over = Some(now);
                    self.runs = r.runs;
                    self.runs_known = true;
                }
                Msg::Log(run, job, text) => self.log = Some((run, job, text)),
                Msg::Done(r) => {
                    self.done = Some(r);
                    self.done_path = None;
                }
                Msg::DoneAt(r) => match r {
                    Ok((said, path)) => {
                        self.done = Some(Ok(said));
                        self.done_path = Some(path);
                    }
                    Err(e) => {
                        self.done = Some(Err(e));
                        self.done_path = None;
                    }
                },
                Msg::PortText(path, r) => self.port_text(&path, r),
                Msg::There(pc, id, got) => self.send_read(pc, id, got),
                Msg::Ha(v) => self.ha = v,
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
                Msg::Answer(req @ (Request::Save { .. } | Request::Propose { .. }), r) => {
                    // A save and a proposal (a role that may not confirm) end the same way.
                    let text = match req {
                        Request::Save { text, .. } | Request::Propose { text, .. } => text,
                        _ => unreachable!("matched above"),
                    };
                    if let Some(e) = &mut self.editor {
                        match r {
                            Ok(said) => e.saved(&text, said),
                            // A net made here met a file made elsewhere: not this one's to overwrite.
                            Err(m) if is_changed_elsewhere(&m) && e.base.is_none() => {
                                e.result = Some(Err(koyomi_core::msg::line(koyomi_core::msg::CREATED_ELSEWHERE, &[&e.id])))
                            }
                            Err(m) if is_changed_elsewhere(&m) => e.conflict = Some(text),
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
                Msg::Answer(Request::Stage { id, .. }, r) => {
                    // Staged from the editor: it goes back to the net as it is; the button by the title shows the day.
                    let def = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == id)).cloned();
                    match &mut self.editor {
                        Some(e) if e.id == id => {
                            if let (Ok(_), Some(def)) = (&r, &def) {
                                e.reload(def);
                            }
                            e.result = Some(r);
                        }
                        _ => self.result = Some(r),
                    }
                }
                Msg::Answer(Request::Unstage { id }, r) => match &mut self.editor {
                    Some(e) if e.id == id => e.result = Some(r),
                    _ => self.result = Some(r),
                },
                Msg::CalendarDone(r) => self.cal_result = Some(r),
                Msg::Answer(Request::CalendarPut { name, .. } | Request::CalendarRename { to: name, .. }, r) => {
                    if self.new_cal.as_ref().is_some_and(|m| m.waiting) {
                        self.new_cal_answer(name, r);
                    } else {
                        self.cal_result = Some(r);
                    }
                }
                Msg::Answer(Request::HolidayTable { .. }, r) => self.cal_result = Some(r),
                // The users page: its own state's, never the result line's. A new token goes to its box only.
                Msg::Answer(Request::Users, r) => self.users.listing_answer(r),
                Msg::Answer(
                    req @ (Request::UserSet { .. } | Request::UserRemove { .. } | Request::UserAdd { .. } | Request::UserReset { .. }),
                    r,
                ) => {
                    let done = self.users.answered(&req, r);
                    if let Some(result) = done.result {
                        self.result = Some(result);
                    }
                    if done.reload {
                        self.users.need = true;
                        self.users_reask();
                    }
                }
                // The other PC's koyomi.toml: the box's, never the result line's (and its text goes nowhere else).
                Msg::Answer(Request::ConfigText, r) => {
                    if let Some(c) = &mut self.config_edit {
                        c.loaded(r);
                    }
                }
                Msg::Answer(Request::ConfigPut { .. }, r) => {
                    if let Some(note) = self.config_edit.as_mut().and_then(|c| c.saved(r)) {
                        self.config_edit = None;
                        self.result = Some(Ok(note));
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
        crate::watch::failed_ids(&self.runs)
    }

    /// How many definitions are not approved or do not read (the status bar's count).
    pub fn todo_count(&self) -> usize {
        self.snap.as_ref().map_or(0, |s| s.defs.iter().filter(|d| d.is_todo()).count())
    }

    /// The status bar's engine facts: `(engine word, running count, next time)`.
    /// The word is `None` while all is well and the engine answers.
    pub fn engine_facts(&self, t: &ito_i18n::Table) -> (Option<String>, usize, Option<chrono::NaiveDateTime>) {
        if self.down.is_some() {
            return (Some(t.tr("status.engine_down").to_owned()), 0, None);
        }
        let running = self.runs.iter().filter(|r| !r.is_over()).count();
        let Some(snap) = &self.snap else {
            let word = match (&self.down, self.starting) {
                (Some(_), _) => t.tr("status.engine_down"),
                (None, _) => t.tr("status.engine_starting"),
            };
            return (Some(word.to_owned()), running, None);
        };
        let word = snap.paused.then(|| t.tr("status.paused").to_owned());
        let now = self.engine_now();
        let next = snap
            .defs
            .iter()
            .filter(|d| !snap.held.contains(&d.id))
            .filter_map(|d| {
                let net = d.net.as_ref()?;
                let cal = d.calendar.as_deref().and_then(|c| snap.calendars.get(c));
                koyomi_core::blackout::outside(&snap.blackouts, now, 1, |after, k| {
                    koyomi_core::quiet::next_starts(&net.when, &net.quiet, net.on_quiet, after, k, cal)
                })
                .into_iter()
                .next()
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

    /// Whether the engine's runs have been taken once (a notice's run can be opened from then on).
    pub fn has_runs(&self) -> bool {
        self.runs_known
    }

    /// The connections whose URL was changed, each once: the app's watch connects to them afresh.
    pub fn take_moved_urls(&mut self) -> Vec<String> {
        self.drain();
        std::mem::take(&mut self.moved_urls)
    }

    /// What the manage box's switches show (see [`far::Far::set_watching`]).
    pub fn set_watching(&mut self, off: std::collections::BTreeSet<String>, all_off: bool) {
        self.far.set_watching(off, all_off);
    }

    /// Whether this window wrote koyomi.toml since the last call (the prefs read it again at once).
    pub fn take_config_written(&mut self) -> bool {
        self.drain();
        self.far.take_config_written()
    }

    /// Shows run `id` in the detail (a click under the bell).
    pub fn open_run(&mut self, id: &str) {
        // Mark its net in the tree too, so the run shows where it belongs.
        if let Some(net) = self.runs.iter().find(|r| r.id == id).map(|r| r.jobnet.clone()) {
            self.selected = Some(net);
        }
        self.run_sel = Some(id.to_owned());
        self.cal_sel = None;
        self.result = None;
        self.job_sel = None;
        self.hist = None;
        self.gantt = false;
        self.leave_editor();
    }

    /// Whether a box is up, the browser's own or a connection box. The keys and `self.focus` belong to one box at a
    /// time, so another is not opened over it: an Enter meant for the new one would press a button of the old one.
    fn box_open(&self) -> bool {
        self.own_box_open() || self.far.boxed()
    }

    /// Whether one of the browser's own boxes is up (the connection boxes are not counted).
    fn own_box_open(&self) -> bool {
        self.asking.is_some()
            || self.killing.is_some()
            || self.deleting.is_some()
            || self.vers.is_some()
            || self.port.is_some()
            || self.make.is_some()
            || self.new_cal.is_some()
            || self.cal_deleting.is_some()
            || self.give.is_some()
            || self.skip_pick
            || self.staged_box.is_some()
            || self.sending.is_some()
            || self.config_edit.is_some()
            || self.users.holds_token()
            || self.editor.as_ref().is_some_and(|e| e.conflict.is_some() || e.closing || e.stage_ask.is_some())
    }

    /// Whether the troubleshooting card or the command palette is over the page.
    fn covered(&self) -> bool {
        self.under_card || self.under_palette
    }

    fn blocked(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| !s.warnings.is_empty())
    }

    /// Whether a warning bears on `def` itself (approving it is refused); another net's file does not count.
    fn blocked_for(&self, def: &Def) -> bool {
        // The engine says which nets a warning bears on: this PC's files and another PC's are read the same way.
        self.snap.as_ref().is_some_and(|s| s.blocked.contains(&def.id))
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
        self.save_kept();
    }

    /// Writes the picture's flow and the right panes' folds to `state.toml`.
    fn save_kept(&self) {
        if let Some(w) = &self.flow {
            let flow = Some(if self.vertical { "vertical" } else { "horizontal" }.to_owned());
            w.save_toml(&Kept {
                flow,
                inspector: Some(self.inspector),
                history_pane: Some(self.hist_pane),
                calendar_pane: Some(self.cal_pane),
                calendar_view: Some(self.cal_view.word().to_owned()),
                calendar_week: None,
                net_pane: Some(self.net_pane),
                link: self.link.far().map(str::to_owned),
            });
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
        let link = self.link.clone();
        Self::background_file(&self.io, move || {
            export_all(&link).map(|(path, n)| (("port.exported", vec![path.display().to_string(), n.to_string()]), path))
        });
    }

    /// The palette's 取り込む: opens the box with the path field.
    pub fn import_box(&mut self) {
        if !self.box_open() {
            self.port = Some(Port { path: String::new(), text: None, plan: None, error: None, read: None, waiting: false, fresh: true });
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
    fn make_go(&mut self) {
        let Some(m) = &mut self.make else { return };
        let unsaved = self.editor.as_ref().is_some_and(|e| e.dirty) || self.editors.iter().any(|e| e.dirty);
        let ids = self.snap.iter().flat_map(|s| s.defs.iter().map(|d| d.id.as_str()));
        let made = match new_id(&m.group, &m.name, ids) {
            Err(Why::Empty) => Err(koyomi_core::msg::line(koyomi_core::msg::JOBNET_NAME_EMPTY, &[])),
            Err(Why::Bad) => Err(koyomi_core::msg::line(koyomi_core::msg::JOBNET_NAME_BAD, &[])),
            Err(Why::Exists(id)) => Err(koyomi_core::msg::line(koyomi_core::msg::JOBNET_EXISTS, &[id.as_str()])),
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
                self.send(req);
                return;
            }
            Ok(_) if unsaved => Err(koyomi_core::msg::line(koyomi_core::msg::UNSAVED_EDITS, &[])),
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
        let (title, lead, go, mark) = match &from {
            None => (t.tr("make.title").to_owned(), t.tr("make.lead").to_owned(), t.tr("make.go"), ito_prefs::Mark::Plus),
            Some((id, Re::Rename)) => {
                (t.tr("rename.title").to_owned(), t.trf("rename.lead", &[id]), t.tr("rename.go"), ito_prefs::Mark::Pencil)
            }
            Some((id, Re::Copy)) => {
                (t.tr("rename.copy_title").to_owned(), t.trf("rename.copy_lead", &[id]), t.tr("rename.copy_go"), ito_prefs::Mark::Copy)
            }
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
                pressed = box_answer(ui, mark, go, t.tr("make.cancel"));
            });
        match pressed.or(key) {
            Some(0) => self.make_go(),
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
                self.send(Request::Run { id, date: None, from: None, dry: false, params: BTreeMap::new() });
            }
        }
    }

    /// The values box of 今すぐ実行: one field a value, then 流す or やめる.
    fn confirm_give(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(g) = &mut self.give else { return };
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
                pressed = box_answer(ui, ito_prefs::Mark::Play, t.tr("give.go"), t.tr("give.cancel"));
            });
        match pressed.or(key) {
            Some(0) => {
                let params = g.set();
                let id = g.id.clone();
                self.give = None;
                self.send(Request::Run { id, date: None, from: None, dry: false, params });
            }
            Some(_) => self.give = None,
            None => {}
        }
    }

    /// The engine's answer to an import: a plan to show, or what it did.
    fn imported(&mut self, dry_run: bool, r: Result<String, String>) {
        let Some(p) = &mut self.port else {
            if !dry_run {
                self.result = Some(r.map(|_| koyomi_core::msg::note(koyomi_core::msg::N_IMPORT_DONE, &[])));
            }
            return;
        };
        p.waiting = false;
        match r.and_then(|text| proto::Plan::parse(&text)) {
            Ok(plan) if plan.applied => {
                self.port = None;
                self.result = Some(Ok(koyomi_core::msg::note(koyomi_core::msg::N_IMPORT_DONE, &[])));
            }
            Ok(plan) => {
                // Not done: the plan to confirm (or, after a choice, what changed meanwhile).
                self.focus = if plan.undecided { 1 } else { 0 };
                p.plan = Some(plan);
            }
            Err(e) => p.error = Some(e),
        }
    }

    /// Looks for the answer of the holiday table's file chooser, every frame (the chooser outlives the field). A
    /// picked file fills the field only when the table field of the same calendar is open now; a cancel changes nothing.
    fn poll_picking(&mut self, ctx: &egui::Context) {
        let Some((name, rx)) = &self.cal_picking else { return };
        let chosen = match rx.try_recv() {
            Ok(chosen) => chosen,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        if let (Some(file), Some(calendar::Inline::Table { cal, path, .. })) = (chosen, &mut self.cal_inline) {
            if cal == name {
                *path = file.display().to_string();
            }
        }
        self.cal_picking = None;
    }

    /// Runs file work on its own thread; the line it returns shows under the detail.
    fn background(io: &(Sender<Msg>, egui::Context), work: impl FnOnce() -> Result<Said, Said> + Send + 'static) {
        let (tx, ctx) = io.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Done(work()));
            ctx.request_repaint();
        });
    }

    /// [`Browser::background`] for work that makes a file on this PC: the line and the file's path.
    fn background_file(io: &(Sender<Msg>, egui::Context), work: impl FnOnce() -> Result<(Said, PathBuf), Said> + Send + 'static) {
        let (tx, ctx) = io.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::DoneAt(work()));
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
                .map_err(|e| koyomi_core::msg::line(koyomi_core::msg::FILE_READ_FAILED, &[&path, &e.to_string()]))
                .and_then(|b| {
                    crate::cli::port_text(&b).ok_or_else(|| koyomi_core::msg::line(koyomi_core::msg::FILE_ENCODING_UNKNOWN, &[&path]))
                });
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
        p.read = Some(path.to_owned());
        match read {
            Ok(text) => {
                p.error = None;
                p.text = Some(text.clone());
                p.waiting = true;
                self.focus = 0;
                self.send(Request::Import { text, dry_run: true, on_conflict: None });
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
        self.send(Request::Import { text, dry_run: false, on_conflict: on_conflict.map(str::to_owned) });
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
                        // The error is about the file last read, not the field (which may have been edited since):
                        // it is on this PC (the window read it).
                        if let Some(file) = p.read.as_deref().filter(|f| !f.is_empty()) {
                            paths::path_buttons(ui, t, std::path::Path::new(file), paths::Place::Here, true);
                        }
                    }
                    pressed = box_answer(ui, ito_prefs::Mark::Import, t.tr("port.read"), t.tr("port.cancel"));
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
                notes.push(koyomi_core::msg::line(koyomi_core::msg::IMPORT_PROBLEMS, &[&plan.problems.to_string()]));
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
        use ito_prefs::Mark;
        let marks: &[Option<Mark>] = match stage {
            Stage::Close => &[Some(Mark::Close)],
            Stage::Go => &[Some(Mark::Import), Some(Mark::Close)],
            _ => &[Some(Mark::Save), Some(Mark::Close), Some(Mark::Pencil), Some(Mark::Skip)],
        };
        let q = ito_confirm::Question {
            title: t.tr("port.title"),
            rows: &rows,
            body: &body,
            notes: &notes,
            buttons: &buttons,
            marks,
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
                    self.out.send(save_request(self.role, e.id.clone(), e.text(), e.base.clone(), force));
                }
            }
            canvas::Action::Flow => self.set_flow(!self.vertical),
            canvas::Action::Close => self.editor = self.editors.pop(),
            canvas::Action::NewCalendar => self.new_cal_box_for_net(),
            canvas::Action::Stage(from) => {
                if let Some(e) = &mut self.editor {
                    e.result = None;
                    e.stage_ask = Some(from);
                    self.focus = 0;
                }
            }
            canvas::Action::Staged => match self.editor.as_ref().map(|e| e.id.clone()) {
                Some(id) if !self.box_open() => {
                    self.staged_box = Some(id);
                    self.focus = 0;
                }
                _ => {}
            },
            canvas::Action::Enter(id) => {
                let def = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == id)).cloned();
                let opened = def
                    .as_ref()
                    .ok_or_else(|| koyomi_core::msg::line(koyomi_core::msg::JOBNET_MISSING, &[&id]))
                    .and_then(canvas::Editor::open);
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
        let defs = self.snap.as_ref().map(|s| s.defs.as_slice()).unwrap_or(&[]);
        let stops = self.snap.as_ref().map(|s| s.blackouts.as_slice()).unwrap_or(&[]);
        let staged = self.snap.as_ref().map(|s| s.staged.as_slice()).unwrap_or(&[]);
        let now = self.engine_now();
        let around = canvas::Around { calendars, power, stops, depth: self.editors.len(), defs, staged, now };
        let Some(e) = &mut self.editor else { return };
        let action = e.show(ui, t, self.vertical, &around);
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
            // Another PC's engine takes no forced save: the box only reloads or cancels.
            let far = self.link.is_far();
            let body = [conflict_body(far)];
            let buttons = [t.tr("canvas.reload").to_owned(), t.tr("canvas.cancel").to_owned(), t.tr("canvas.overwrite").to_owned()];
            let buttons = &buttons[..if far { 2 } else { 3 }];
            let q = ito_confirm::Question {
                title: t.tr("canvas.conflict_title"),
                rows: &rows,
                body: &body,
                diff: &diff,
                buttons,
                marks: &[Some(ito_prefs::Mark::Refresh), Some(ito_prefs::Mark::Close), Some(ito_prefs::Mark::Save)][..buttons.len()],
                focus: Some(self.focus),
                primary: Some(0),
                danger: true,
                ..Default::default()
            };
            let answer = ito_confirm::show(ctx, &q, &Self::style()).or(key);
            let Some(e) = &mut self.editor else { return };
            let force = answer.and_then(|a| force_save(far, a, &e.id, text.clone(), e.base.clone()));
            match answer {
                Some(0) => e.reload(&def),
                Some(_) => {
                    e.conflict = None;
                    if let Some(r) = force {
                        e.result = None;
                        self.out.send(r);
                    }
                }
                None => {}
            }
            return;
        }
        if let Some(from) = e.stage_ask.clone() {
            let Some(def) = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == e.id)).cloned() else {
                if let Some(e) = &mut self.editor {
                    e.stage_ask = None;
                }
                return;
            };
            let text = e.text();
            let diff = ito_diff::diff(&def.text, &text, 3);
            let rows = [(t.tr("review.file").to_owned(), def.file.clone()), (t.tr("canvas.stage_from").to_owned(), from.clone())];
            let body = [t.tr("canvas.stage_body").to_owned()];
            let buttons = [t.tr("canvas.stage_yes").to_owned(), t.tr("canvas.cancel").to_owned()];
            let title = t.trf("canvas.stage_title", &[&from]);
            let q = ito_confirm::Question {
                title: &title,
                rows: &rows,
                body: &body,
                diff: &diff,
                buttons: &buttons,
                marks: &[Some(ito_prefs::Mark::CalendarCheck), Some(ito_prefs::Mark::Close)],
                focus: Some(self.focus),
                primary: Some(0),
                ..Default::default()
            };
            let answer = ito_confirm::show(ctx, &q, &Self::style()).or(key);
            let Some(e) = &mut self.editor else { return };
            match answer {
                Some(0) => {
                    e.stage_ask = None;
                    e.result = None;
                    // What this window saw: the file it started from, and the staged one over it (if any).
                    let staged =
                        self.snap.as_ref().and_then(|s| s.staged.iter().find(|n| n.id == e.id)).map(koyomi_core::next::Entry::seal);
                    match stage_request(&e.id, e.base.clone(), staged, text, from) {
                        Ok(r) => {
                            self.out.send(r);
                        }
                        Err(m) => e.result = Some(Err(m)),
                    }
                }
                Some(_) => e.stage_ask = None,
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
                marks: &[Some(ito_prefs::Mark::Trash), Some(ito_prefs::Mark::Close)],
                focus: Some(self.focus),
                primary: Some(0),
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
            let result = done.map(words).map_err(words);
            // A file this window wrote is on this PC, whichever engine it talks to.
            self.result_path = match (&result, self.done_path.take()) {
                (Ok(text), Some(path)) => Some(paths::Made { text: text.clone(), path, place: paths::Place::Here }),
                _ => None,
            };
            self.result = Some(result);
        }
        if self.out.refused.take() {
            self.result = Some(Err(koyomi_core::msg::line(koyomi_core::msg::LINK_SWITCHING, &[])));
        }
        let ctx = ui.ctx().clone();
        self.far.add_box(&ctx, t, self.covered());
        self.far.manage_box(&ctx, t, self.link.far());
        self.config_box(&ctx, t);
        self.token_box(&ctx, t);
        let conflict = self.editor.as_ref().is_some_and(|e| e.conflict.is_some()) || self.vers.is_some();
        let port = self.port.as_ref().map(|p| (p.buttons(), p.stage() == Stage::Path));
        let making = self.make.is_some() || self.new_cal.is_some() || self.give.is_some() || self.skip_pick;
        let closing = self.editor.as_ref().is_some_and(|e| e.closing);
        let asking_stage = self.editor.as_ref().is_some_and(|e| e.stage_ask.is_some());
        if closing && !self.close_asked {
            // The safe button first: Enter on a fresh box does not throw the changes away.
            self.focus = 1;
        }
        self.close_asked = closing;
        let keys_modal = self.asking.is_some()
            || self.killing.is_some()
            || self.deleting.is_some()
            || self.cal_deleting.is_some()
            || conflict
            || closing
            || port.is_some()
            || making
            || asking_stage
            || self.staged_box.is_some()
            || self.sending.as_ref().is_some_and(|s| s.there.is_some());
        // The token box reads its own keys: it blocks the page like a box, but `box_keys` leaves it alone.
        let modal = keys_modal || self.users.holds_token();
        let buttons = if self.vers.is_some() {
            4
        } else if conflict || self.staged_box.is_some() {
            // Another PC's save conflict only reloads or cancels.
            if self.link.is_far() && self.staged_box.is_none() {
                2
            } else {
                3
            }
        } else if asking_stage {
            2
        } else {
            port.map_or(2, |(n, _)| n)
        };
        let key = if keys_modal { self.box_keys(&ctx, buttons, making) } else { None };
        let c = ito_theme::colors();
        if let Some(snap) = self.snap.as_ref().filter(|s| !s.warnings.is_empty()) {
            // The writable-by warning keeps its header; the files that did not read have their own.
            let (writable, problems): (Vec<String>, Vec<String>) =
                snap.warnings.iter().cloned().partition(|l| koyomi_core::msg::FILE_WRITABLE_BY.is_in(l));
            egui::Panel::top("band").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                let mark = ito_a11y::State::Warning.mark();
                for (key, lines) in [("band.warning", &writable), ("band.problem", &problems)] {
                    if lines.is_empty() {
                        continue;
                    }
                    let head = format!("{mark} {}", t.tr(key));
                    let r = ui.label(egui::RichText::new(head.clone()).strong().color(c.wait_text()));
                    ito_a11y::say(&r, Role::Label, &head);
                    for l in lines {
                        ui.label(egui::RichText::new(l).color(c.wait_text()));
                    }
                }
            });
        }
        // The engine's day is not this window's day: what 今日 shows is off.
        if self.link.is_far() && self.snap.as_ref().is_some_and(|s| s.utc_offset_min != local_utc_offset_min()) {
            egui::Panel::top("zone").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                ui.label(egui::RichText::new(t.tr("band.zone")).color(c.wait_text()));
            });
        }
        if self.snap.as_ref().is_some_and(|s| s.paused) {
            egui::Panel::top("paused").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                // Centred on the row, so the words and the button's words sit on one line.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                    let head = format!("{} {}", State::Held.mark(), t.tr("band.paused"));
                    let r = ui.label(egui::RichText::new(head.clone()).strong().color(c.wait_text()));
                    ito_a11y::say(&r, Role::Label, &head);
                    // Filled with the warn colour of the band: it undoes the pause, in the words' own size.
                    let resume = ito_prefs::IconButton::new(ito_prefs::Mark::Play, t.tr("run.resume")).tone(ito_prefs::Tone::Warn);
                    let r = ui.add_enabled(!modal, |ui: &mut egui::Ui| resume.show(ui));
                    if r.clicked() {
                        self.send(Request::Resume);
                    }
                });
            });
        }
        // A planned stop going on, or one to begin within the week: its time comes and nothing starts.
        let stop_line = self.snap.as_ref().and_then(|s| {
            let now = self.engine_now();
            let b = koyomi_core::blackout::near(&s.blackouts, now, 7)?;
            let why = show::stop_why(t, b);
            Some(if b.contains(now) {
                t.trf("band.blackout_now", &[why, b.to.trim()])
            } else {
                t.trf("band.blackout_soon", &[why, b.from.trim(), b.to.trim()])
            })
        });
        if let Some(line) = stop_line {
            egui::Panel::top("blackout").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                let text = format!("{} {line}", State::Skipped.mark());
                let r = ui.add(egui::Label::new(egui::RichText::new(text.clone()).color(c.wait_text())).wrap());
                ito_a11y::say(&r, Role::Label, &text);
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
            ito_a11y::test_id(&ui.response(), "main.tree");
            if modal {
                ui.disable();
            }
            one_size(ui.style_mut());
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.tree(ui, t);
                ui.add_space(10.0);
                self.calendar_list(ui, t);
                // The one switch over the whole schedule, below what it stops.
                let paused = self.snap.as_ref().is_some_and(|s| s.paused);
                if !paused && self.snap.is_some() {
                    ui.add_space(10.0);
                    let r = ito_prefs::icon_button(ui, ito_prefs::Mark::Pause, t.tr("run.pause"), t.tr("run.pause"));
                    if r.clicked() {
                        self.send(Request::Pause);
                    }
                }
            });
        });
        // No room at the right and bottom, so a pane there meets the window's edges; each page keeps
        // its own room with `page`.
        let central = egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin { left: 8, top: 8, right: 0, bottom: 0 });
        egui::CentralPanel::default().frame(central).show(ui, |ui| {
            ito_a11y::test_id(&ui.response(), "main.central");
            if modal {
                ui.disable();
            }
            if self.editor.is_none() {
                egui::Frame::new().inner_margin(egui::Margin { right: 8, ..Default::default() }).show(ui, |ui| self.tabs(ui, t));
                // A pane at the right meets the tabs' line; the page keeps its own room under it.
                ui.add_space(-ui.spacing().item_spacing.y);
            }
            if self.editor.is_some() {
                // The editor's fields pane meets the window's right and bottom edges, as the inspector does;
                // its bars and canvas keep their own room.
                egui::Frame::new().inner_margin(egui::Margin { top: 12, ..Default::default() }).show(ui, |ui| self.editor_view(ui, t));
            } else if self.gantt {
                // The picked row's net at the right, folded to a strip when not wanted.
                self.inspector_pane(ui, t);
                page().show(ui, |ui| {
                    // The chart in a pane at the bottom, its top edge dragged to size it; the table above.
                    egui::Panel::bottom("gantt_pane")
                        .resizable(true)
                        .default_size(320.0)
                        .size_range(120.0..=(ui.available_height() - 80.0).max(120.0))
                        .frame(egui::Frame::new().inner_margin(egui::Margin { top: 8, ..Default::default() }))
                        .show(ui, |ui| {
                            one_size(ui.style_mut());
                            // The panel keeps the size its content took, so fill it (an empty first frame would shrink it).
                            ui.set_min_height(ui.available_height());
                            self.gantt_view(ui, t);
                        });
                    // Above the scrolling table, so an answer (the palette's 書き出す …) stays in sight.
                    // The open inspector says it under the picked net's buttons instead.
                    if !(self.inspector && self.inspected().is_some()) {
                        self.show_result(ui, t);
                    }
                    // Both ways: with the inspector open the table may be wider than what is left.
                    egui::ScrollArea::both().id_salt("gantt_page").auto_shrink(false).show(ui, |ui| {
                        one_size(ui.style_mut());
                        self.today_table(ui, t);
                    });
                });
            } else if self.hist.is_some() {
                self.history_view(ui, t);
            } else if self.run_sel.is_some() {
                page().show(ui, |ui| self.run_page(ui, t, true));
            } else if self.cal_sel.is_some() {
                self.calendar_view(ui, t);
            } else {
                // The net open, or the overview's picked row, at the right; folded to a strip when not wanted.
                self.net_pane(ui, t);
                page().show(ui, |ui| {
                    // A net's jobs in a pane at the bottom, its top edge dragged to size it; the page above.
                    if self.selected_def().is_some_and(|d| d.net.as_ref().is_some_and(|n| !n.jobs.is_empty())) {
                        egui::Panel::bottom("jobs_pane")
                            .resizable(true)
                            .default_size(320.0)
                            .size_range(120.0..=(ui.available_height() - 80.0).max(120.0))
                            .frame(egui::Frame::new().inner_margin(egui::Margin { top: 8, ..Default::default() }))
                            .show(ui, |ui| {
                                ui.set_min_height(ui.available_height());
                                self.jobs_pane(ui, t);
                            });
                    }
                    if self.selected_def().is_some() {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.detail(ui, t));
                    } else {
                        // The pane shows the answer under the picked net's buttons; otherwise it is said here.
                        if !(self.net_pane && self.side_net().is_some()) {
                            self.show_result(ui, t);
                        }
                        self.nets_overview(ui, t);
                    }
                });
            }
            // The row 戻る came back to is in sight now.
            self.reveal = false;
        });
        self.poll_picking(&ctx);
        // Boxes can overlap (a kill asked while a delete is open); one key answers only the first
        // open one, so a single Enter cannot decide two of them.
        let open = [
            self.asking.is_some(),
            self.killing.is_some(),
            self.deleting.is_some(),
            self.editor.as_ref().is_some_and(|e| e.conflict.is_some() || e.stage_ask.is_some()) || closing,
            self.vers.is_some(),
            self.port.is_some(),
            self.make.is_some(),
            self.give.is_some(),
            self.new_cal.is_some(),
            self.cal_deleting.is_some(),
            self.skip_pick,
            self.staged_box.is_some(),
            self.sending.as_ref().is_some_and(|s| s.there.is_some()),
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
        self.confirm_skip_next(&ctx, t, key_for(10));
        self.confirm_staged(&ctx, t, key_for(11));
        self.confirm_send(&ctx, t, key_for(12));
        if keys_modal && !making && !self.covered() && !port.is_some_and(|(_, typing)| typing) {
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
            // The list's title, with a small ＋ for a new net at the right (its words on hover and to a reader).
            let r = ui
                .horizontal(|ui| {
                    ui.label(egui::RichText::new(t.tr("tree.heading")).small().strong().color(c.dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ito_prefs::mark_button(ui, ito_prefs::Mark::Plus, &label, 22.0)
                    })
                    .inner
                })
                .inner;
            if let Some(lang) = &self.easy {
                ito_glossary::hover(ui, ui.painter(), r.rect, &crate::glossary::GLOSSARY, "jobnet", lang, "tree");
            }
            if r.clicked() {
                self.new_box(None);
            }
            ui.add_space(4.0);
        }
        let Some(snap) = &self.snap else {
            let text = match (&self.down, self.starting) {
                (Some(e), _) => list_unavailable(e),
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
            ui.label(egui::RichText::new(list_unavailable(e)).color(c.err));
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
                let r = tree_group(ui, &c, &format!("{mark} {}", d.group), todo);
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
            // The label (shortened when the tree is narrow), then what to know about it, kept whole at the right.
            let (mut head, word) = review::row_parts(t, d, snap.held.contains(&d.id));
            if twins.contains(&d.id) {
                head = review::twin_row(head, d);
            }
            let mut tail: Vec<String> = word.into_iter().collect();
            if snap.held_jobs.iter().any(|h| h.net == d.id) {
                tail.push(format!("{} {}", State::Held.mark(), t.tr("state.job_held")));
            }
            // A next definition staged for a day: the day, short, and in full to a reader.
            let staged = snap
                .staged
                .iter()
                .find(|e| e.id == d.id)
                .map(|e| (format!("→{}", e.from.format("%-m/%-d")), t.trf("staged.note", &[&e.from.to_string()])));
            if let Some((short, _)) = &staged {
                tail.push(short.clone());
            }
            let tail = tail.join("  ");
            let text = if tail.is_empty() { head.clone() } else { format!("{head}  {tail}") };
            // On the overview (no net open) the row picked there is marked here too.
            let shown = if self.selected.is_none() && !self.gantt && self.cal_sel.is_none() {
                self.nets_pick.as_deref()
            } else {
                self.selected.as_deref()
            };
            let on = self.run_sel.is_none() && self.hist.is_none() && shown == Some(d.id.as_str());
            let r = tree_row(ui, &c, on, last, &head, &tail);
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
                None => text,
            };
            let said = match &staged {
                Some((short, long)) if said.contains(short.as_str()) => said.replace(short.as_str(), long),
                Some((_, long)) => format!("{said}, {long}"),
                None => said,
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
        if let Some(id) = pick {
            // A pick in the tree is not a link: there is nothing to go back to.
            self.forget_back();
            self.open_net(&id);
        }
    }

    /// Opens the net `id` on the ジョブネット tab, as a pick in the tree does.
    fn open_net(&mut self, id: &str) {
        self.nets_pick = Some(id.to_owned());
        self.selected = Some(id.to_owned());
        self.run_sel = None;
        self.cal_sel = None;
        self.job_sel = None;
        self.result = None;
        self.hist = None;
        self.gantt = false;
        self.leave_editor();
    }

    /// Where the detail is now, for 戻る.
    fn here(&self) -> Place {
        Place {
            gantt: self.gantt,
            hist: self.hist.clone(),
            selected: self.selected.clone(),
            nets_pick: self.nets_pick.clone(),
            run_sel: self.run_sel.clone(),
            today_net: self.today_net.clone(),
            job_sel: self.job_sel.clone(),
            cal_sel: self.cal_sel.clone(),
            cal_month: self.cal_month,
            cal_day: self.cal_day,
        }
    }

    /// Back to `p`, its picked row scrolled into sight; a history asks the engine for its days again.
    fn put(&mut self, p: Place) {
        if let Some(h) = &p.hist {
            self.send(h.request());
        }
        self.gantt = p.gantt;
        self.hist = p.hist;
        self.selected = p.selected;
        self.nets_pick = p.nets_pick;
        self.run_sel = p.run_sel;
        self.today_net = p.today_net;
        self.job_sel = p.job_sel;
        self.cal_sel = p.cal_sel;
        self.cal_month = p.cal_month;
        self.cal_day = p.cal_day;
        self.result = None;
        self.cal_result = None;
        self.cal_inline = None;
        self.reveal = true;
        self.leave_editor();
    }

    /// Follows a link: `go` moves the detail, and where it was is kept for 戻る (not when it stayed).
    fn follow(&mut self, go: impl FnOnce(&mut Self)) {
        let from = self.here();
        go(self);
        if !self.here().same(&from) {
            self.back.push(from);
            if self.back.len() > BACK_MAX {
                self.back.remove(0);
            }
        }
        self.reveal = false;
    }

    /// 戻る: to where the last link came from.
    fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            self.put(p);
        }
    }

    /// A tab, a pick in the tree or the tray moves on its own: 戻る goes away.
    fn forget_back(&mut self) {
        self.back.clear();
        self.reveal = false;
    }

    /// Opens run `id` as a link does (the bell's): 戻る comes back to where the detail was.
    pub fn link_run(&mut self, id: &str) {
        self.follow(|b| b.open_run(id));
    }

    /// What 戻る goes back to, in words: the tab, or the run, calendar or net that was open.
    fn place_name(&self, t: &ito_i18n::Table, p: &Place) -> String {
        if p.gantt {
            t.tr("tabs.today").to_owned()
        } else if p.hist.is_some() {
            t.tr("tabs.history").to_owned()
        } else if let Some(id) = &p.run_sel {
            self.runs.iter().find(|r| &r.id == id).map_or_else(|| id.clone(), |r| r.title.clone())
        } else if let Some(name) = &p.cal_sel {
            name.clone()
        } else if let Some(id) = &p.selected {
            id.clone()
        } else {
            t.tr("tabs.net").to_owned()
        }
    }

    /// The ジョブネット tab with no net picked: every net in one table, in the tree's order, with its last
    /// run's state, start and time, its next start and whether it waits for a person. A click picks a row
    /// (the tree marks it too), a double click opens the net; a head sorts.
    fn nets_overview(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(snap) = &self.snap else { return };
        section_head(ui, t.tr("nets.heading"));
        ui.add_space(4.0);
        if snap.defs.is_empty() {
            ui.label(egui::RichText::new(t.tr("nets.empty")).color(c.dim));
            return;
        }
        let now = self.engine_now();
        let when = |at: chrono::NaiveDateTime| at.format("%m-%d %H:%M").to_string();
        let twins = review::twins(&snap.defs);
        struct Row {
            order: usize,
            id: String,
            state: Option<State>,
            cells: Vec<(String, bool)>,
            keys: [Option<i64>; 7],
        }
        let mut rows: Vec<Row> = snap
            .defs
            .iter()
            .enumerate()
            .map(|(order, d)| {
                let held = snap.held.contains(&d.id);
                let last = self.runs.iter().filter(|r| r.jobnet == d.id).max_by_key(|r| r.started);
                // The next time that starts: quiet hours and planned stops seen.
                let next = snap.next_start(d, now);
                let took = last.map(|r| r.duration().unwrap_or(now - r.started));
                let note = match d.standing {
                    Standing::Confirmed if held => t.tr("state.held").to_owned(),
                    Standing::Confirmed => String::new(),
                    Standing::Unconfirmed => t.tr("standing.unconfirmed").to_owned(),
                    Standing::Invalid => t.tr("standing.invalid").to_owned(),
                };
                let state = last.map(|r| r.state);
                // Two nets of one name read apart as the tree tells them: with the file's name after it.
                let name = if twins.contains(&d.id) { format!("{} · {}", d.label(), d.name) } else { d.label().to_owned() };
                let cells = vec![
                    (state.map(|s| show::tag(t, s).to_owned()).unwrap_or_default(), false),
                    (name, false),
                    (d.group.clone(), false),
                    (last.map(|r| when(r.started)).unwrap_or_default(), true),
                    (took.map(|d| show::duration(t, d)).unwrap_or_default(), true),
                    (next.map(when).unwrap_or_default(), true),
                    (note, false),
                ];
                let keys = [None, None, None, last.map(|r| stamp(r.started)), took.map(|d| d.num_milliseconds()), next.map(stamp), None];
                Row { order, id: d.id.clone(), state, cells, keys }
            })
            .collect();
        if let Some((col, down)) = self.nets_sort {
            rows.sort_by(|a, b| {
                let o = match (a.keys[col], b.keys[col], a.cells[col].0.is_empty(), b.cells[col].0.is_empty()) {
                    (_, _, true, false) => return std::cmp::Ordering::Greater,
                    (_, _, false, true) => return std::cmp::Ordering::Less,
                    (Some(x), Some(y), _, _) => x.cmp(&y),
                    _ => ito_common::text::natural_cmp(&a.cells[col].0, &b.cells[col].0, false),
                };
                let o = o.then(a.order.cmp(&b.order));
                if down {
                    o.reverse()
                } else {
                    o
                }
            });
        }
        let heads: Vec<String> = ["col_state", "col_net", "col_group", "col_last", "col_time", "col_next", "col_note"]
            .map(|k| t.tr(&format!("nets.{k}")).to_owned())
            .to_vec();
        let picked = self.nets_pick.clone();
        let reveal = self.reveal;
        let (mut pick, mut open) = (None, None);
        // The header band stays put; only the rows scroll up and down. Sideways they scroll together.
        egui::ScrollArea::horizontal().id_salt("nets_scroll").auto_shrink(false).show(ui, |ui| {
            one_size(ui.style_mut());
            let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
            let cells: Vec<Vec<(String, bool)>> = rows.iter().map(|r| r.cells.clone()).collect();
            let mut widths = TodayTable::fit(ui, &heads, &cells);
            // The net, its group and the note give way (ending in `…`) before the times are cut off.
            TodayTable::squeeze(&mut widths, ui.available_width(), &[1, 2, 6], 72.0);
            let mut table = TodayTable::with(&c, &widths, ui.available_width());
            if let Some(col) = table.head(ui, &heads, self.nets_sort, t.tr("today.sort")) {
                self.nets_sort = TodayTable::next_sort(self.nets_sort, col);
            }
            egui::ScrollArea::vertical().id_salt("nets_rows").auto_shrink([true, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in &rows {
                    let on = picked.as_deref() == Some(row.id.as_str());
                    let r = table.cells_of(ui, &row.cells, on, row.state).on_hover_text(t.tr("nets.open_hint"));
                    if on && reveal {
                        reveal_row(ui, &r);
                    }
                    let said: Vec<&str> = row.cells.iter().map(|(t, _)| t.as_str()).filter(|t| !t.is_empty()).collect();
                    ito_a11y::say(&r, Role::Row, &said.join(" "));
                    if r.double_clicked() {
                        open = Some(row.id.clone());
                    } else if r.clicked() {
                        pick = Some(row.id.clone());
                    }
                }
            });
            ui.spacing_mut().item_spacing.y = gap;
        });
        if let Some(id) = open {
            self.follow(|b| b.open_net(&id));
        } else if pick.is_some() {
            self.nets_pick = pick;
        }
    }

    /// Today as a table: what is still to come (waiting, or スキップ with why in a planned stop), then
    /// the runs newest first, each with its state, net, planned time, start, end and duration. A
    /// click on a run opens it.
    fn today_table(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(snap) = &self.snap else { return };
        let now = self.engine_now();
        let mut coming: Vec<(chrono::NaiveDateTime, String, String, Option<String>, State)> = Vec::new();
        for (d, at) in snap.starts_today(now) {
            let wake = d.net.as_ref().is_some_and(|n| n.wake);
            let sleep = show::Sleep::of(&snap.power, wake);
            coming.push((at, d.id.clone(), d.label().to_owned(), show::sleep_mark(t, &sleep, at, now), State::Waiting));
        }
        // A time in a planned stop is shown, as the スキップ it will be, with why.
        for (d, at, b) in snap.stopped_today(now) {
            coming.push((at, d.id.clone(), d.label().to_owned(), Some(show::stop_mark(t, b)), State::Skipped));
        }
        coming.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        // The next start of a net (by id) after `from`: quiet spans are seen, a held net has none.
        let by_id: std::collections::HashMap<&str, &Def> = snap.defs.iter().map(|d| (d.id.as_str(), d)).collect();
        let average = |id: &str| self.past.get(id).copied();
        let next_at = |id: &str, from: chrono::NaiveDateTime| by_id.get(id).and_then(|d| snap.next_start(d, from));
        let next_words = |at: Option<chrono::NaiveDateTime>| {
            at.map(|at| if at.date() == now.date() { show::hms(at) } else { at.format("%m-%d %H:%M").to_string() }).unwrap_or_default()
        };
        if self.runs.is_empty() && coming.is_empty() {
            ui.label(egui::RichText::new(t.tr("run.none")).color(c.dim));
            return;
        }
        // The rows: what is still to come, then the runs, newest first.
        let mut rows: Vec<TodayRow> = Vec::new();
        for (at, id, title, mark, state) in &coming {
            let planned = match mark {
                Some(m) => format!("{}  {m}", show::hms(*at)),
                None => show::hms(*at),
            };
            let usual = average(id).filter(|_| *state == State::Waiting);
            let next = next_at(id, *at);
            // Read out as a run's row is: its state, the net and when it starts.
            let said = format!("{} {title} {planned}", show::tag(t, *state));
            let cells = [
                planned,
                String::new(),
                String::new(),
                usual.map(|a| t.trf("today.average", &[&show::duration(t, a)])).unwrap_or_default(),
                next_words(next),
            ];
            rows.push(TodayRow {
                state: *state,
                title: title.clone(),
                said,
                cells,
                run: None,
                net: id.clone(),
                kinds: vec![TodayKind::Is(*state)],
                keys: (Some(*at), None, None, usual.map(|a| a.num_milliseconds()), next.map(stamp)),
            });
        }
        let mut runs: Vec<&Run> = self.runs.iter().collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.started));
        for run in runs {
            let ended = run.ended.map(show::hms).unwrap_or_default();
            let usual = average(&run.jobnet).map(|a| t.trf("today.average", &[&show::duration(t, a)]));
            let took = match (run.duration(), &usual) {
                (Some(d), _) => show::duration(t, d),
                (None, Some(u)) => format!("{}… {u}", show::duration(t, now - run.started)),
                (None, None) => format!("{}…", show::duration(t, now - run.started)),
            };
            let said = [show::tag(t, run.state).to_owned(), run.title.clone(), show::hms(run.started), ended.clone(), took.clone()];
            let said: Vec<&str> = said.iter().map(String::as_str).filter(|x| !x.is_empty()).collect();
            let next = next_at(&run.jobnet, now);
            let cells = [run.planned.map(show::hms).unwrap_or_default(), show::hms(run.started), ended, took, next_words(next)];
            let took = run.duration().unwrap_or(now - run.started).num_milliseconds();
            rows.push(TodayRow {
                state: run.state,
                title: run.title.clone(),
                said: said.join(" "),
                cells,
                run: Some(run.id.clone()),
                net: run.jobnet.clone(),
                kinds: TodayKind::of(run.state, self.acked.contains(&run.id)),
                keys: (run.planned, Some(run.started), run.ended, Some(took), next.map(stamp)),
            });
        }
        // The chips: what needs a person, then every state a run can be in (a state with none is dim).
        // A click shows only its rows; a second click shows them all again.
        let count = |k: TodayKind| rows.iter().filter(|r| r.kinds.contains(&k)).count();
        let mut parts = vec![(TodayKind::Unhandled, t.tr("today.unhandled").to_owned(), graph::colour(&c, State::Error))];
        parts.extend(
            State::ALL.into_iter().filter(|s| *s != State::NotRun).map(|s| (TodayKind::Is(s), show::tag(t, s), graph::mark_colour(&c, s))),
        );
        let mut kind = self.today_kind;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
            let look = ito_prefs::Look { c, q: "" };
            for (k, label, colour) in &parts {
                let n = count(*k);
                let on = kind == Some(*k);
                // Only what today has: a state with no run is no chip (the one in use stays, to turn it off).
                if n == 0 && !on {
                    continue;
                }
                let text = format!("{label} {n}");
                let r = ito_prefs::tag(ui, look, ito_prefs::Tag::new(&text).color(*colour).on(on))
                    .on_hover_text(t.tr(if on { "today.filter_off" } else { "today.filter_on" }))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                ito_a11y::say(&r, Role::Button, &text);
                if r.clicked() {
                    kind = if on { None } else { Some(*k) };
                }
            }
        });
        self.today_kind = kind;
        if let Some(k) = kind {
            rows.retain(|r| r.kinds.contains(&k));
        }
        if let Some((col, down)) = self.today_sort {
            TodayRow::sort(&mut rows, col, down);
        }
        ui.add_space(8.0);
        let (mut pick, mut open) = (None, None);
        let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
        let mut table = TodayTable::new(ui, &c);
        let heads: Vec<String> = ["col_state", "col_net", "col_planned", "col_start", "col_end", "col_time", "col_next"]
            .map(|k| t.tr(&format!("today.{k}")).to_owned())
            .to_vec();
        if let Some(col) = table.head(ui, &heads, self.today_sort, t.tr("today.sort")) {
            // A column's first click sorts it up, the second down, the third puts the order back.
            self.today_sort = TodayTable::next_sort(self.today_sort, col);
        }
        if rows.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(t.tr("today.none_here")).color(c.dim));
        }
        // A right click holds or releases the row's net (and releases a job of it held), without opening it.
        let (held, held_jobs) = match &self.snap {
            Some(s) => (s.held.clone(), s.held_jobs.clone()),
            None => Default::default(),
        };
        let blocked = self.blocked();
        let (mut ask, mut go_net) = (None, None);
        for row in &rows {
            let on = match &row.run {
                Some(_) => self.run_sel == row.run,
                None => self.run_sel.is_none() && self.today_net.as_deref() == Some(row.net.as_str()),
            };
            let r =
                table.row(ui, row.state, &show::tag(t, row.state), &row.title, &row.cells, on, true).on_hover_text(t.tr("today.open_hint"));
            if on && self.reveal {
                reveal_row(ui, &r);
            }
            ito_a11y::say(&r, Role::Row, &row.said);
            if r.double_clicked() {
                // A start still to come has no run yet: its net opens instead.
                open = Some(row.run.clone().ok_or_else(|| row.net.clone()));
            } else if r.clicked() {
                pick = Some((row.run.clone(), row.net.clone()));
            }
            r.context_menu(|ui| {
                let net_held = held.contains(&row.net);
                let key = if net_held { "run.release" } else { "run.hold" };
                if ui.add_enabled(!blocked, egui::Button::new(t.tr(key))).clicked() {
                    let id = row.net.clone();
                    ask = Some(if net_held { Request::Release { id, job: None } } else { Request::Hold { id, job: None } });
                    ui.close();
                }
                for h in held_jobs.iter().filter(|h| h.net == row.net) {
                    if ui.add_enabled(!blocked, egui::Button::new(t.trf("run.release_job", &[&h.job]))).clicked() {
                        ask = Some(Request::Release { id: h.net.clone(), job: Some(h.job.clone()) });
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(t.tr("today.open_net")).clicked() {
                    go_net = Some(row.net.clone());
                    ui.close();
                }
            });
        }
        ui.spacing_mut().item_spacing.y = gap;
        if let Some(req) = ask {
            self.send(req);
        }
        if let Some(id) = go_net {
            self.follow(|b| b.open_net(&id));
            return;
        }
        // A click picks the row here (marked in the chart below too); a double click opens its run on the
        // net page, or the net of a start still to come.
        match (open, pick) {
            (Some(Ok(run)), _) => self.follow(|b| b.open_run(&run)),
            (Some(Err(net)), _) => self.follow(|b| b.open_net(&net)),
            (None, Some((run, net))) => {
                self.run_sel = run;
                self.today_net = Some(net);
            }
            (None, None) => {}
        }
    }

    /// The tabs over the detail, at the left: ジョブネット (the net picked on the left), カレンダー (the
    /// first one, or the one picked), 今日 (the chart) and 履歴.
    fn tabs(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        if self.snap.is_none() {
            return;
        }
        let first_cal = self.snap.as_ref().and_then(|s| s.calendars.keys().next().cloned());
        let (today, history, calendar) = (self.gantt, self.hist.is_some(), self.cal_sel.is_some() && !self.gantt && self.hist.is_none());
        let net = !today && !history && !calendar;
        let mut go = None;
        let mut back = false;
        let back_to = self.back.last().map(|p| t.trf("nav.back_to", &[&self.place_name(t, p)]));
        let c = ito_theme::colors();
        // One row high: a centred layout would take the whole height and push the view below off the screen.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y + 6.0);
        let bar = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (tab, on, key) in [
                (Tab::Net, net, "tabs.net"),
                (Tab::Calendar, calendar, "tabs.calendar"),
                (Tab::Today, today, "tabs.today"),
                (Tab::History, history, "tabs.history"),
            ] {
                let label = t.tr(key);
                let r = pill_tab(ui, &c, on, label);
                ito_a11y::say(&r, Role::Button, label);
                if r.clicked() {
                    go = Some(tab);
                }
            }
            // The day the chart is about, at the right end.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let day = self.engine_today();
                let wd = ["weekday.mon", "weekday.tue", "weekday.wed", "weekday.thu", "weekday.fri", "weekday.sat", "weekday.sun"]
                    [chrono::Datelike::weekday(&day).num_days_from_monday() as usize];
                let text = format!("{}({})", day.format("%Y-%m-%d"), t.tr(wd));
                let r = ui.label(egui::RichText::new(&text).monospace().color(c.dim));
                ito_a11y::say(&r, Role::Label, &text);
                // Where a link came from, beside the day: back to the same row.
                if let Some(to) = &back_to {
                    ui.add_space(8.0);
                    // Its hover and its spoken name say where it goes.
                    back = ito_prefs::IconButton::new(ito_prefs::Mark::ChevronLeft, t.tr("nav.back")).said(to).show(ui).clicked();
                }
            });
        });
        // A hairline right under the tabs, as the page's top edge; a pane at the right starts on it.
        let y = bar.response.rect.bottom();
        let x = ui.max_rect().x_range();
        ui.painter().hline(x, y, egui::Stroke::new(1.0, c.border));
        if back {
            self.go_back();
            return;
        }
        if go.is_some() {
            // A tab is not a link: there is nothing to go back to.
            self.forget_back();
        }
        match go {
            Some(Tab::Today) => self.open_gantt(),
            Some(Tab::Net) => {
                // Pressed again on a net's page: back to every net (the tree lets go of it too).
                if net && self.run_sel.is_none() && self.editor.is_none() && self.selected.is_some() {
                    // The net just left stays picked in the overview.
                    self.nets_pick = self.selected.take();
                }
                self.hist = None;
                self.gantt = false;
                self.run_sel = None;
                self.cal_sel = None;
                self.job_sel = None;
                self.result = None;
            }
            Some(Tab::Calendar) => match self.cal_sel.clone().or(first_cal) {
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
            Some(Tab::History) => self.open_history(),
            None => {}
        }
    }

    fn open_gantt(&mut self) {
        self.hist = None;
        self.run_sel = None;
        self.cal_sel = None;
        self.job_sel = None;
        self.result = None;
        self.today_net = None;
        self.gantt = true;
    }

    /// The tray's 今日: ガント in the detail, unless a net is open in the editor or a box is (that work stays).
    pub fn show_today(&mut self) {
        if self.editor.is_some() || self.port.is_some() || self.make.is_some() || self.vers.is_some() || self.deleting.is_some() {
            return;
        }
        self.forget_back();
        self.open_gantt();
    }

    /// The tray's 未対応を見る: the newest of today's failures not marked 対応済み, else 今日; as
    /// [`Self::show_today`], not while a net is open in the editor or a box is.
    pub fn show_failed(&mut self) {
        if self.editor.is_some() || self.port.is_some() || self.make.is_some() || self.vers.is_some() || self.deleting.is_some() {
            return;
        }
        self.forget_back();
        let newest = self.runs.iter().filter(|r| r.state.is_error() && !self.acked.contains(&r.id)).max_by_key(|r| r.started);
        match newest.map(|r| r.id.clone()) {
            Some(id) => self.open_run(&id),
            None => self.open_gantt(),
        }
    }

    /// A click on a notice of run `id`: the run in the detail, or in 履歴's pane when it is not one of
    /// the runs the window holds (an older day's); as [`Self::show_today`], not while a net is open in
    /// the editor or a box is.
    pub fn show_run(&mut self, id: &str) {
        if self.editor.is_some() || self.port.is_some() || self.make.is_some() || self.vers.is_some() || self.deleting.is_some() {
            return;
        }
        self.forget_back();
        if self.runs.iter().any(|r| r.id == id) {
            self.open_run(id);
        } else {
            self.open_history();
            self.run_sel = Some(id.to_owned());
            self.hist_pane = true;
        }
    }

    /// The tray's 次の 1 回だけスキップ…: the box that picks the net, unless another box or the editor is up.
    pub fn show_skip_next(&mut self) {
        if self.editor.is_some() || self.box_open() {
            return;
        }
        self.result = None;
        self.skip_pick = true;
    }

    /// The approved, not held nets that run on a schedule, each with its next time not yet skipped
    /// and the times it skips: the rows of the 次の 1 回だけスキップ box.
    fn skip_rows(&self, t: &ito_i18n::Table, now: chrono::NaiveDateTime) -> Vec<SkipRow> {
        let Some(snap) = &self.snap else { return Vec::new() };
        snap.defs
            .iter()
            .filter(|d| d.standing == Standing::Confirmed && !snap.held.contains(&d.id))
            .filter_map(|d| {
                let net = d.net.as_ref()?;
                let cal = d.calendar.as_deref().and_then(|c| snap.calendars.get(c));
                let skipped: Vec<_> = snap.skips.iter().filter(|s| s.id == d.id && s.at > now).map(|s| s.at).collect();
                let next = koyomi_core::held::next_open(&net.when, cal, now, |t| skipped.contains(&t));
                (next.is_some() || !skipped.is_empty()).then(|| SkipRow {
                    id: d.id.clone(),
                    name: d.title.as_ref().map_or(d.id.clone(), |title| t.trf("skipnext.named", &[title, &d.id])),
                    next,
                    skipped,
                })
            })
            .collect()
    }

    /// The 次の 1 回だけスキップ box: a row a net, 飛ばす sends `SkipNext` (the engine says which
    /// time it took), 戻す takes the net's skips back. Esc or 閉じる closes it.
    fn confirm_skip_next(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        if !self.skip_pick {
            return;
        }
        let c = ito_theme::colors();
        let rows = self.skip_rows(t, self.engine_now());
        let mut send = None;
        let mut close = key.is_some();
        egui::Window::new(t.tr("skipnext.title"))
            .id(egui::Id::new("skipnext"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.add(egui::Label::new(t.tr("skipnext.lead")).wrap());
                ui.add_space(4.0);
                if rows.is_empty() {
                    ui.label(egui::RichText::new(t.tr("skipnext.none")).color(c.dim));
                }
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    egui::Grid::new("skipnext-rows").num_columns(4).striped(true).show(ui, |ui| {
                        for h in ["skipnext.net", "skipnext.next", "skipnext.skipped"] {
                            ui.label(egui::RichText::new(t.tr(h)).strong());
                        }
                        ui.label("");
                        ui.end_row();
                        for SkipRow { id, name, next, skipped } in &rows {
                            ui.label(name.as_str());
                            ui.label(next.map(|n| n.format("%m-%d %H:%M").to_string()).unwrap_or_default());
                            let gone = skipped.iter().map(|s| s.format("%m-%d %H:%M").to_string()).collect::<Vec<_>>().join(", ");
                            ui.label(egui::RichText::new(gone).color(c.dim));
                            ui.horizontal(|ui| {
                                if next.is_some() {
                                    let label = format!("{} {}", t.tr("skipnext.skip"), name);
                                    let r = ito_prefs::IconButton::new(ito_prefs::Mark::Skip, t.tr("skipnext.skip")).said(&label).show(ui);
                                    if r.clicked() {
                                        send = Some(Request::SkipNext { id: id.clone(), cancel: false });
                                    }
                                }
                                if !skipped.is_empty() {
                                    let label = format!("{} {}", t.tr("skipnext.undo"), name);
                                    let r = ito_prefs::IconButton::new(ito_prefs::Mark::Undo, t.tr("skipnext.undo")).said(&label).show(ui);
                                    if r.clicked() {
                                        send = Some(Request::SkipNext { id: id.clone(), cancel: true });
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                close |= box_foot(ui, |ui| ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("skipnext.close")).show(ui).clicked());
            });
        if let Some(r) = send {
            self.result = None;
            self.send(r);
        }
        if close {
            self.skip_pick = false;
        }
    }

    /// The 次の定義 box: the change from the net now to what is staged for a day, loaded into the
    /// editor to change it, or dropped.
    fn confirm_staged(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(id) = self.staged_box.clone() else { return };
        let found = self.snap.as_ref().and_then(|s| {
            let entry = s.staged.iter().find(|e| e.id == id)?.clone();
            let def = s.defs.iter().find(|d| d.id == id)?.clone();
            Some((entry, def))
        });
        let Some((entry, def)) = found else {
            self.staged_box = None;
            return;
        };
        let from = entry.from.to_string();
        let diff = ito_diff::diff(&def.text, &entry.text, 3);
        let standing = if entry.approved() { t.tr("staged.approved") } else { t.tr("staged.hand") };
        let rows = [
            (t.tr("review.file").to_owned(), def.file.clone()),
            (t.tr("staged.from").to_owned(), from.clone()),
            (t.tr("staged.standing").to_owned(), standing.to_owned()),
        ];
        let body = [t.tr("staged.body").to_owned()];
        let buttons = [t.tr("staged.edit").to_owned(), t.tr("staged.close").to_owned(), t.tr("staged.cancel").to_owned()];
        let title = t.trf("staged.title", &[&from]);
        let q = ito_confirm::Question {
            title: &title,
            rows: &rows,
            body: &body,
            diff: &diff,
            buttons: &buttons,
            marks: &[Some(ito_prefs::Mark::Pencil), Some(ito_prefs::Mark::Close), Some(ito_prefs::Mark::Trash)],
            focus: Some(self.focus),
            primary: Some(0),
            ..Default::default()
        };
        let answer = ito_confirm::show(ctx, &q, &Self::style()).or(key);
        match answer {
            Some(0) => self.take_staged(t, &entry),
            Some(2) => {
                self.result = None;
                self.send(Request::Unstage { id });
            }
            _ => {}
        }
        if answer.is_some() {
            self.staged_box = None;
        }
    }

    /// 編集に読み込む: the staged text into the editor (opened on the net when it is not), the day
    /// row open on its day; undo goes back to the net as it is.
    fn take_staged(&mut self, t: &ito_i18n::Table, entry: &koyomi_core::next::Entry) {
        let here = self.editor.as_ref().is_some_and(|e| e.id == entry.id);
        if !here {
            if self.editor.iter().chain(&self.editors).any(|e| e.dirty) {
                let busy = Err(t.tr("staged.busy").to_owned());
                match &mut self.editor {
                    Some(e) => e.result = Some(busy),
                    None => self.result = Some(busy),
                }
                return;
            }
            self.open_editor(&entry.id);
        }
        if let Some(e) = self.editor.as_mut().filter(|e| e.id == entry.id) {
            if let Err(m) = e.take_next(&entry.text, &entry.from.to_string()) {
                e.result = Some(Err(m));
            }
        }
    }

    /// Under the net's buttons: the day a next definition takes over, and a button to see it.
    fn staged_note(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str) {
        let Some(from) = self.snap.as_ref().and_then(|s| s.staged.iter().find(|e| e.id == id)).map(|e| e.from.to_string()) else {
            return;
        };
        let c = ito_theme::colors();
        let see = ui
            .horizontal_wrapped(|ui| {
                let text = format!("→ {}", t.trf("staged.note", &[&from]));
                let r = ui.label(egui::RichText::new(&text).color(c.dim));
                ito_a11y::say(&r, Role::Label, &text);
                ito_prefs::IconButton::new(ito_prefs::Mark::Calendar, t.tr("staged.see")).show(ui).clicked()
            })
            .inner;
        if see && !self.box_open() {
            self.staged_box = Some(id.to_owned());
            self.focus = 0;
        }
    }

    /// 別の PC に送る ▾: the net to one of the `[[remote]]` PCs, where it lands unconfirmed (TODO 111).
    fn send_button(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def) {
        if !self.far.any() {
            return;
        }
        let ok = def.standing != Standing::Invalid && !self.box_open();
        let head = format!("{} ▾", t.tr("send.button"));
        let button = ito_prefs::IconButton::new(ito_prefs::Mark::Send, &head);
        let mut r = ui.add_enabled(ok, |ui: &mut egui::Ui| button.show(ui));
        if def.standing == Standing::Invalid {
            r = r.on_disabled_hover_text(t.tr("send.fix_first"));
        }
        let mut pick = None;
        egui::Popup::menu(&r).show(|ui| {
            for pc in self.far.names() {
                let label = t.trf("send.to", &[pc]);
                let r = ui.button(&label);
                ito_a11y::say(&r, Role::Button, &label);
                if r.clicked() {
                    pick = Some(pc.clone());
                }
            }
        });
        if let Some(pc) = pick {
            self.send_to(def, pc);
        }
    }

    /// Reads what `pc` has of the net, off the UI thread; [`Browser::send_read`] gets it.
    fn send_to(&mut self, def: &Def, pc: String) {
        self.result = None;
        self.sending = Some(Sending { id: def.id.clone(), pc: pc.clone(), text: def.text.clone(), there: None });
        let (tx, ctx) = self.io.clone();
        let (there, id) = (self.wire.there, def.id.clone());
        std::thread::spawn(move || {
            let got = there(&pc, &id);
            let _ = tx.send(Msg::There(pc, id, got));
            ctx.request_repaint();
        });
    }

    /// What the other PC has of the net: the box opens on it, or a line says why not. The same
    /// text is not sent. An answer for a send since stopped is dropped.
    fn send_read(&mut self, pc: String, id: String, got: Result<Option<crate::remote::There>, String>) {
        let Some(s) = self.sending.as_mut().filter(|s| s.pc == pc && s.id == id && s.there.is_none()) else { return };
        match got {
            Ok(Some(there)) if there.text == s.text => {
                self.sending = None;
                self.done = Some(Ok(("send.same", vec![pc, id])));
            }
            Ok(there) => {
                s.there = Some(there);
                self.focus = 0;
            }
            Err(e) => {
                self.sending = None;
                self.result = Some(Err(e));
            }
        }
    }

    /// While the other PC is read: which, and a button to stop waiting.
    fn sending_note(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let Some(s) = self.sending.as_ref().filter(|s| s.there.is_none()) else { return };
        let text = t.trf("send.reading", &[&s.pc, &s.id]);
        let c = ito_theme::colors();
        let stop = ui
            .horizontal_wrapped(|ui| {
                ui.spinner();
                let r = ui.label(egui::RichText::new(&text).color(c.dim));
                ito_a11y::say(&r, Role::Label, &text);
                ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("send.stop")).show(ui).clicked()
            })
            .inner;
        if stop {
            self.sending = None;
        }
    }

    /// The 別の PC に送る box: what that PC has of the net against this PC's, and 送る.
    fn confirm_send(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(s) = &self.sending else { return };
        let Some(there) = &s.there else { return };
        let word = |st: Option<Standing>| st.map_or_else(|| "?".to_owned(), |st| review::word(t, st));
        let here = self.snap.as_ref().and_then(|snap| snap.defs.iter().find(|d| d.id == s.id)).map(|d| d.standing);
        let rows = [
            (t.tr("send.pc").to_owned(), s.pc.clone()),
            (t.tr("send.there").to_owned(), there.as_ref().map_or_else(|| t.tr("send.there_none").to_owned(), |x| word(x.standing))),
            (t.tr("send.here").to_owned(), word(here)),
        ];
        let diff = ito_diff::diff(there.as_ref().map_or("", |x| x.text.as_str()), &s.text, 3);
        let body = [t.tr("send.body").to_owned()];
        let buttons = [t.tr("send.go").to_owned(), t.tr("send.cancel").to_owned()];
        let title = t.trf("send.title", &[&s.pc, &s.id]);
        let q = ito_confirm::Question {
            title: &title,
            rows: &rows,
            body: &body,
            diff: &diff,
            buttons: &buttons,
            marks: &[Some(ito_prefs::Mark::Send), Some(ito_prefs::Mark::Close)],
            focus: Some(self.focus),
            primary: Some(0),
            ..Default::default()
        };
        let answer = ito_confirm::show(ctx, &q, &Self::style()).or(key);
        if answer == Some(0) {
            let (pc, id, text) = (s.pc.clone(), s.id.clone(), s.text.clone());
            let base = there.as_ref().map(|x| x.sha256.clone());
            let propose = self.wire.propose;
            self.result = None;
            Self::background(&self.io, move || match propose(&pc, &id, &text, base.as_deref()) {
                Ok(same) => Ok((if same { "send.done_same" } else { "send.done" }, vec![pc, id])),
                Err(e) => Err(("send.failed", vec![e])),
            });
        }
        if answer.is_some() {
            self.sending = None;
        }
    }

    /// Every start still to come today, as (net id, title, start).
    fn planned_today(&self, now: chrono::NaiveDateTime) -> Vec<(String, String, chrono::NaiveDateTime)> {
        let Some(snap) = &self.snap else { return Vec::new() };
        snap.starts_today(now).into_iter().map(|(d, at)| (d.id.clone(), d.label().to_owned(), at)).collect()
    }

    /// ガント: today's nets, a lane each, with the runs and the starts still planned; a click on a lane opens the run under the chart.
    fn gantt_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let now = self.engine_now();
        let planned = self.planned_today(now);
        if self.runs.is_empty() && planned.is_empty() {
            return;
        }
        if self.runs.iter().any(|r| !r.is_over()) {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        let runs: Vec<&Run> = self.runs.iter().collect();
        let picked = self.run_sel.clone();
        let mut pick = None;
        // The chart in a card: its title and what the bars mean, the key at the right, then the lanes.
        egui::Frame::new()
            .fill(c.side)
            .stroke(egui::Stroke::new(1.0, c.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                // The key at the right when it fits, else on its own line under the title.
                ui.horizontal_wrapped(|ui| {
                    let r = ui.label(egui::RichText::new(t.tr("run.gantt")).strong().color(c.fg));
                    ito_a11y::say(&r, Role::Heading, t.tr("run.gantt"));
                    ui.label(egui::RichText::new(t.tr("run.gantt_note")).small().color(c.dim));
                    let mut shown: Vec<State> = Vec::new();
                    for run in &runs {
                        if !shown.contains(&run.state) {
                            shown.push(run.state);
                        }
                    }
                    gantt::legend(ui, t, &shown, !planned.is_empty());
                });
                ui.add_space(6.0);
                egui::ScrollArea::both().id_salt("gantt").auto_shrink(false).show(ui, |ui| {
                    pick = gantt::draw(
                        ui,
                        &runs,
                        &planned,
                        now,
                        picked.as_deref(),
                        t.tr("run.gantt_earlier"),
                        t.tr("run.gantt_planned"),
                        t.tr("today.open_hint"),
                    );
                });
            });
        // As a row of the table: a click picks the run, a double click opens it on the net page.
        match pick {
            Some((id, true)) => self.follow(|b| b.open_run(&id)),
            Some((id, false)) => {
                self.today_net = None;
                self.run_sel = Some(id);
            }
            None => {}
        }
    }

    /// Opens the 版を比べる box on the net open in the editor, or else the one picked.
    pub fn open_versions(&mut self) {
        let id = self.editor.as_ref().map(|e| e.id.clone()).or_else(|| self.selected.clone());
        let Some(id) = id else { return };
        if self.box_open() {
            return;
        }
        self.send(Request::Versions { id: id.clone() });
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
                self.out.send(Request::VersionText { id: v.id.clone(), version: w.clone() });
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
            marks: &[
                Some(ito_prefs::Mark::Undo),
                Some(ito_prefs::Mark::Close),
                Some(ito_prefs::Mark::ChevronLeft),
                Some(ito_prefs::Mark::ChevronRight),
            ],
            focus: Some(self.focus),
            primary: Some(0),
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
                self.send(req);
            }
            Some(2) if v.idx + 1 < v.list.len() => v.idx += 1,
            Some(3) if v.idx > 0 => v.idx -= 1,
            Some(1) => self.vers = None,
            _ => {}
        }
    }

    /// Opens 履歴 on the last 7 days and asks the engine for them.
    /// On a far link before the first snapshot nothing is asked yet: the days are the engine's, and the week is
    /// built and asked when the snapshot has come.
    pub fn open_history(&mut self) {
        let known = self.engine_date_known();
        let h = History::week(self.engine_today());
        if known {
            self.send(h.request());
        }
        self.hist_waiting = !known;
        self.hist = Some(h);
        self.gantt = false;
        self.run_sel = None;
        self.cal_sel = None;
        self.job_sel = None;
        self.result = None;
    }

    /// The first snapshot of a far link has come: the 履歴 opened before it gets the engine's week and asks for it.
    fn history_when_known(&mut self) {
        if !self.hist_waiting || !self.engine_date_known() {
            return;
        }
        self.hist_waiting = false;
        let week = History::week(self.engine_today());
        if let Some(h) = self.hist.as_mut() {
            h.from = week.from;
            h.to = week.to;
            h.error = None;
            let req = h.request();
            self.send(req);
        }
    }

    /// 履歴: a card of what narrows the rows (the days, a preset, the net, the state, the search and
    /// the CSV), the chips that count each state and narrow to one, and the rows that match (newest
    /// first unless a column is sorted). The run picked opens in a pane at the right, as under 今日.
    fn history_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        if self.hist.is_none() {
            return;
        }
        // The pane at the right is always there, as on 今日 and the calendar: the run picked, else a
        // word on how to pick one.
        let mut edge = ui.max_rect().right();
        if !self.hist_pane {
            if folded_pane(ui, "history_run_strip", t.tr("run.history_pane"), t.tr("inspect.open")) {
                self.hist_pane = true;
                self.save_kept();
            }
        } else {
            let wide = ui.available_width();
            let (mut close, mut fold) = (false, false);
            let picked = self.selected_run().is_some();
            let pane = egui::Panel::right("history_run")
                .resizable(true)
                .default_size(460.0_f32.min(wide * 0.4))
                .size_range(280.0..=(wide * 0.7).max(280.0))
                .frame(egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.border)).inner_margin(egui::Margin::same(12)))
                .show(ui, |ui| {
                    one_size(ui.style_mut());
                    // The panel keeps the size its content took, so fill it.
                    ui.set_min_height(ui.available_height());
                    // The head: what the pane is, the fold mark and, with a run picked, ✕ at the right.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            fold = ito_prefs::mark_button(ui, ito_prefs::Mark::ChevronRight, t.tr("inspect.fold"), 22.0).clicked();
                            if picked {
                                close = ito_prefs::mark_button(ui, ito_prefs::Mark::Close, t.tr("run.history_close"), 22.0).clicked();
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let r = ui.label(egui::RichText::new(t.tr("run.history_pane")).strong().color(c.dim));
                                ito_a11y::say(&r, Role::Heading, t.tr("run.history_pane"));
                            });
                        });
                    });
                    ui.add_space(8.0);
                    if !picked {
                        let r = ui.add(egui::Label::new(egui::RichText::new(t.tr("run.history_pick")).color(c.dim)).wrap());
                        ito_a11y::say(&r, Role::Label, t.tr("run.history_pick"));
                        return;
                    }
                    egui::ScrollArea::vertical().id_salt("history_run_scroll").auto_shrink(false).show(ui, |ui| self.run_view(ui, t));
                });
            edge = edge.min(pane.response.rect.left());
            if close {
                self.run_sel = None;
                self.job_sel = None;
            }
            if fold {
                self.hist_pane = false;
                self.save_kept();
            }
        }
        page().show(ui, |ui| {
            let nets: Vec<String> = self.snap.as_ref().map(|s| s.defs.iter().map(|d| d.id.clone()).collect()).unwrap_or_default();
            // What is left of the page up to the pane (the page still reports the width it had before it).
            let room = (edge.min(ui.max_rect().right()) - ui.cursor().left() - 12.0).max(200.0);
            let (waiting, today) = (self.hist_waiting, self.engine_today());
            let Some(h) = self.hist.as_mut() else { return };
            let r = ui.heading(egui::RichText::new(t.tr("run.history")).strong());
            ito_a11y::say(&r, Role::Heading, t.tr("run.history"));
            ui.add_space(6.0);
            if waiting {
                let r = ui.label(egui::RichText::new(t.tr("run.history_waiting")).color(c.dim));
                ito_a11y::say(&r, Role::Label, t.tr("run.history_waiting"));
            }
            let (mut search, mut csv) = (false, false);
            let all = t.tr("run.history_all");
            // Off (greyed) while the engine's date is awaited: a press would otherwise be dropped without a word.
            ui.add_enabled_ui(!waiting, |ui| {
                egui::Frame::new()
                    .fill(c.panel)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(room - 20.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                            let small = |ui: &mut egui::Ui, key: &str| ui.label(egui::RichText::new(t.tr(key)).small().color(c.dim));
                            small(ui, "run.history_from");
                            // Typed, or picked on one month grid: the first click the start, the second the end.
                            let words = ito_prefs::DateWords::for_lang(t.tr("app.lang"));
                            ito_prefs::RangeField::new("history", words)
                                .names(t.tr("run.history_from"), t.tr("run.history_to"))
                                .width(124.0)
                                .show(ui, ito_prefs::Look { c, q: "" }, &mut h.from, &mut h.to);
                            // The presets ask at once for the days up to today.
                            let from = |back: i64| (today - chrono::Duration::days(back)).to_string();
                            let now = [0, 7, 30].into_iter().find(|b| h.from == from(*b) && h.to == today.to_string());
                            let options = [
                                (Some(0), t.tr("run.history_today")),
                                (Some(7), t.tr("run.history_7")),
                                (Some(30), t.tr("run.history_30")),
                            ];
                            if let Some(Some(back)) = ito_prefs::seg(ui, ito_prefs::Look { c, q: "" }, "history-presets", now, &options) {
                                h.from = from(back);
                                h.to = today.to_string();
                                search = true;
                            }
                        });
                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                            let small = |ui: &mut egui::Ui, key: &str| ui.label(egui::RichText::new(t.tr(key)).small().color(c.dim));
                            small(ui, "run.history_net");
                            let net_text = if h.net.is_empty() { all.to_owned() } else { h.net.clone() };
                            egui::ComboBox::from_id_salt("history_net").width(96.0).selected_text(net_text).show_ui(ui, |ui| {
                                ui.selectable_value(&mut h.net, String::new(), all);
                                for id in &nets {
                                    ui.selectable_value(&mut h.net, id.clone(), id);
                                }
                            });
                            small(ui, "run.history_state");
                            let state_text = match h.state {
                                Some(s) => show::tag(t, s),
                                None => all.to_owned(),
                            };
                            egui::ComboBox::from_id_salt("history_state").width(86.0).selected_text(state_text).show_ui(ui, |ui| {
                                ui.selectable_value(&mut h.state, None, all);
                                for s in State::ALL {
                                    ui.selectable_value(&mut h.state, Some(s), show::tag(t, s));
                                }
                            });
                            ui.add_space(4.0);
                            search |=
                                ito_prefs::icon_button(ui, ito_prefs::Mark::Search, t.tr("run.history_search"), t.tr("run.history_search"))
                                    .clicked();
                            csv = ito_prefs::icon_button(ui, ito_prefs::Mark::Download, t.tr("run.history_csv"), t.tr("run.history_csv"))
                                .clicked();
                        });
                    });
            });
            if csv {
                let mut shown = h.filtered();
                shown.reverse();
                let table = crate::sheet::runs(t, &shown.into_iter().cloned().collect::<Vec<_>>());
                self.result = Some(Ok(t.tr("run.writing").to_owned()));
                Self::background_file(&self.io, move || {
                    export_csv("history", &table).map(|path| (("run.history_csv_done", vec![path.display().to_string()]), path))
                });
            }
            // Nothing is asked before the engine's date is known: the days would be this PC's.
            if search && !waiting {
                let day = |s: &str| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok();
                match (day(&h.from), day(&h.to)) {
                    (Some(from), Some(to)) => {
                        h.from = from.to_string();
                        h.to = to.to_string();
                        h.error = None;
                        self.out.send(h.request());
                    }
                    _ => h.error = Some(t.tr("run.history_bad_date").to_owned()),
                }
            }
            if let Some(e) = &h.error {
                ui.label(egui::RichText::new(e).color(c.err));
            }
            ui.add_space(8.0);
            // The chips: all the rows of the net, then each state some row is in. A click narrows to that
            // state; a second click, or the first chip, shows them all again.
            let look = ito_prefs::Look { c, q: "" };
            let mut state = h.state;
            {
                let of_net: Vec<&Run> = h.rows.iter().filter(|r| h.net.is_empty() || r.jobnet == h.net).collect();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                    let text = t.trf("run.history_count", &[&of_net.len().to_string()]);
                    let r = ito_prefs::tag(ui, look, ito_prefs::Tag::new(&text).color(c.fg).on(state.is_none()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    ito_a11y::say(&r, Role::Button, &text);
                    if r.clicked() {
                        state = None;
                    }
                    for s in State::ALL {
                        let n = of_net.iter().filter(|r| r.state == s).count();
                        let on = state == Some(s);
                        if n == 0 && !on {
                            continue;
                        }
                        let text = format!("{} {n}", show::tag(t, s));
                        let r = ito_prefs::tag(ui, look, ito_prefs::Tag::new(&text).color(graph::colour(&c, s)).on(on))
                            .on_hover_text(t.tr(if on { "today.filter_off" } else { "today.filter_on" }))
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        ito_a11y::say(&r, Role::Button, &text);
                        if r.clicked() {
                            state = if on { None } else { Some(s) };
                        }
                    }
                });
            }
            h.state = state;
            let mut rows = h.filtered();
            if !rows.is_empty() {
                ui.add_space(4.0);
                let owned: Vec<Run> = rows.iter().map(|r| (*r).clone()).collect();
                let lines = crate::report::lines(t, &koyomi_core::report::summarize(&owned));
                egui::CollapsingHeader::new(t.tr("report.heading")).id_salt("history_report").show(ui, |ui| {
                    for l in &lines {
                        ui.label(l);
                    }
                });
            }
            // State, net, date, start, end, time; a sorted column orders the rows, else the newest first.
            if let Some((col, down)) = h.sort {
                let never = i64::MAX;
                rows.sort_by(|a, b| {
                    let key = |r: &Run| -> i64 {
                        match col {
                            0 => State::ALL.iter().position(|s| *s == r.state).unwrap_or(0) as i64,
                            4 => r.ended.map_or(never, |e| e.and_utc().timestamp_millis()),
                            5 => r.duration().map_or(never, |d| d.num_milliseconds()),
                            _ => r.started.and_utc().timestamp_millis(),
                        }
                    };
                    if col == 1 {
                        a.title.cmp(&b.title)
                    } else {
                        key(a).cmp(&key(b))
                    }
                });
                if down {
                    rows.reverse();
                }
            }
            let rows: Vec<Run> = rows.into_iter().cloned().collect();
            ui.add_space(6.0);
            let mut pick = None;
            // The net takes what the fixed columns leave; a page narrower than that scrolls sideways.
            let width = room - 14.0;
            let fixed = [100.0, 0.0, 100.0, 84.0, 84.0, 120.0];
            let net = (width - fixed.iter().sum::<f32>()).max(120.0);
            let widths: Vec<f32> = fixed.iter().enumerate().map(|(i, w)| if i == 1 { net } else { *w }).collect();
            let mut table = TodayTable::with(&c, &widths, width);
            let heads: Vec<String> = ["col_state", "col_net", "col_date", "col_start", "col_end", "col_time"]
                .iter()
                .map(|k| t.tr(&format!("today.{k}")).to_owned())
                .collect();
            let sort = h.sort;
            let reveal = self.reveal;
            let none = rows.is_empty() && h.error.is_none();
            let mut sorted = None;
            let tall = ui.available_height();
            ui.allocate_ui(egui::vec2(room, tall), |ui| {
                egui::ScrollArea::both().id_salt("history_rows").auto_shrink(false).show(ui, |ui| {
                    let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
                    sorted = table.head(ui, &heads, sort, t.tr("today.sort"));
                    for run in &rows {
                        let ended = run.ended.map(show::hms).unwrap_or_default();
                        let took = run.duration().map(|d| show::duration(t, d)).unwrap_or_default();
                        let on = self.run_sel.as_deref() == Some(run.id.as_str());
                        let date = run.started.date().to_string();
                        let word = show::tag(t, run.state);
                        let cells = [date.clone(), show::hms(run.started), ended.clone(), took.clone()];
                        let r = table.row(ui, run.state, &word, &run.title, &cells, on, true);
                        if on && reveal {
                            reveal_row(ui, &r);
                        }
                        let said = [date, show::tag(t, run.state).to_owned(), run.title.clone(), show::hms(run.started), ended, took];
                        let said = said.iter().map(String::as_str).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
                        ito_a11y::say(&r, Role::Row, &said);
                        if r.clicked() {
                            pick = Some(run.id.clone());
                        }
                    }
                    ui.spacing_mut().item_spacing.y = gap;
                    if none {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(t.tr("run.history_none")).color(c.dim));
                    }
                    ui.add_space(8.0);
                    self.show_result(ui, t);
                });
            });
            if let (Some(col), Some(h)) = (sorted, self.hist.as_mut()) {
                h.sort = TodayTable::next_sort(h.sort, col);
            }
            if pick.is_some() {
                self.run_sel = pick;
                self.job_sel = None;
                self.result = None;
                ui.ctx().request_repaint();
            }
        });
    }

    /// Whether `job` of the net `net` is on hold (`held.toml`).
    fn job_held(&self, net: &str, job: &str) -> bool {
        self.snap.as_ref().is_some_and(|s| s.held_jobs.iter().any(|h| h.net == net && h.job == job))
    }

    /// 「保留 / 保留を解く」 for one job of a net.
    fn hold_job_button(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, net: &str, job: &str) {
        let held = self.job_held(net, job);
        let label = t.trf(if held { "run.release_job" } else { "run.hold_job" }, &[job]);
        let mark = if held { ito_prefs::Mark::Play } else { ito_prefs::Mark::Pause };
        let r = ui.add_enabled(!self.blocked(), |ui: &mut egui::Ui| ito_prefs::IconButton::new(mark, &label).show(ui));
        if r.clicked() {
            let (id, job) = (net.to_owned(), Some(job.to_owned()));
            self.send(if held { Request::Release { id, job } } else { Request::Hold { id, job } });
        }
    }

    /// The run: its line, the buttons, the picture, the jobs in words, the log — one under the other.
    fn run_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        self.run_page(ui, t, false);
    }

    /// The run. `paned`: the jobs in words and the log go in a pane at the bottom (its top edge dragged to size
    /// it), the jobs at its left and the picked job's facts and log at its right; the rest scrolls above.
    fn run_page(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, paned: bool) {
        let Some(run) = self.selected_run().cloned() else {
            ui.label(t.tr("run.none"));
            return;
        };
        let watched = Self::watched_job(&run, self.job_sel.as_deref());
        *self.watch.lock().unwrap_or_else(|e| e.into_inner()) = watched.clone().map(|j| (run.id.clone(), j));
        let watched = watched.as_deref();
        if !paned {
            self.run_top(ui, t, &run, watched);
            self.run_jobs(ui, t, &run, watched, false);
            ui.add_space(6.0);
            self.run_detail(ui, t, &run, watched);
            return;
        }
        egui::Panel::bottom("run_pane")
            .resizable(true)
            .default_size(360.0)
            .size_range(120.0..=(ui.available_height() - 80.0).max(120.0))
            .frame(egui::Frame::new().inner_margin(egui::Margin { top: 8, ..Default::default() }))
            .show(ui, |ui| {
                // The panel keeps the size its content took, so fill it (an empty first frame would shrink it).
                ui.set_min_height(ui.available_height());
                let wide = ui.available_width();
                egui::Panel::right("run_log_pane")
                    .resizable(true)
                    .default_size(wide * 0.45)
                    .size_range(200.0..=(wide - 200.0).max(200.0))
                    .frame(egui::Frame::new().inner_margin(egui::Margin { left: 10, ..Default::default() }))
                    .show(ui, |ui| {
                        ui.set_min_height(ui.available_height());
                        egui::ScrollArea::vertical().id_salt("run_detail").auto_shrink(false).show(ui, |ui| {
                            self.run_detail(ui, t, &run, watched);
                        });
                    });
                self.run_jobs(ui, t, &run, watched, true);
            });
        egui::ScrollArea::vertical().id_salt("run_page").auto_shrink(false).show(ui, |ui| self.run_top(ui, t, &run, watched));
    }

    /// The run's line, the buttons on it, a run to compare it with, and the picture.
    fn run_top(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, run: &Run, watched: Option<&str>) {
        let c = ito_theme::colors();
        let run = run.clone();
        let (big, small) = show::run_head(t, &run);
        ui.heading(&big);
        ui.label(egui::RichText::new(small).small().color(c.dim));
        ui.add_space(6.0);
        // The actions on this run, as one bar: which run they act on at the left, then the buttons.
        egui::Frame::new()
            .fill(c.panel)
            .stroke(egui::Stroke::new(1.0, c.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    let chip = t.trf("run.selected", &[&run.title]);
                    let r =
                        ito_prefs::tag(ui, ito_prefs::Look { c, q: "" }, ito_prefs::Tag::new(&chip).color(graph::colour(&c, run.state)));
                    ito_a11y::say(&r, Role::Label, &chip);
                    ui.add_space(6.0);
                    // Only the buttons that mean something for this run are drawn.
                    if !run.is_over() && danger(ui, ito_prefs::Mark::Stop, t.tr("run.kill")).clicked() && !self.box_open() {
                        self.killing = Some(run.id.clone());
                        // The safe button first: Enter on a fresh box does not kill.
                        self.focus = 1;
                    }
                    // Again, once it is over, from one menu: what passed is carried over, or from the picked
                    // job on. A run where everything passed can only go again from a picked job.
                    let picked = self.job_sel.clone().filter(|p| run.jobs.iter().any(|j| &j.id == p));
                    if run.is_over() {
                        let head = format!("{} ▾", t.tr("run.rerun"));
                        let rerun = std::cell::RefCell::new(None::<Request>);
                        let button = ito_prefs::IconButton::new(ito_prefs::Mark::Rerun, &head);
                        let r = ui.add_enabled(!self.blocked(), |ui: &mut egui::Ui| button.show(ui));
                        egui::Popup::menu(&r).show(|ui| {
                            let rest = !run.jobs.iter().all(|j| j.state.passed());
                            let r = ui.add_enabled(rest, egui::Button::new(t.tr("run.rerun_rest")));
                            ito_a11y::say(&r, Role::Button, t.tr("run.rerun_rest"));
                            if r.clicked() {
                                *rerun.borrow_mut() = Some(Request::Rerun { run: run.id.clone(), from: None, next: None });
                            }
                            let label = match &picked {
                                Some(j) => t.trf("run.rerun_from", &[j]),
                                None => t.tr("run.rerun_pick").to_owned(),
                            };
                            let r = ui.add_enabled(picked.is_some(), egui::Button::new(&label));
                            ito_a11y::say(&r, Role::Button, &label);
                            if r.clicked() {
                                *rerun.borrow_mut() = Some(Request::Rerun { run: run.id.clone(), from: picked.clone(), next: None });
                            }
                            // From the jobs after the picked one, which ends as スキップ or 警告 without running.
                            for (mark, key) in [(State::Skipped, "run.rerun_next_skip"), (State::Warn, "run.rerun_next_warn")] {
                                let Some(j) = &picked else { break };
                                let label = t.trf(key, &[j]);
                                let r = ui.button(&label);
                                ito_a11y::say(&r, Role::Button, &label);
                                if r.clicked() {
                                    *rerun.borrow_mut() =
                                        Some(Request::Rerun { run: run.id.clone(), from: picked.clone(), next: Some(mark) });
                                }
                            }
                        });
                        if let Some(r) = rerun.take() {
                            self.send(r);
                        }
                    }
                    // A failure dealt with: it leaves the count in the status bar.
                    let done = self.acked.contains(&run.id);
                    let words = t.tr(if done { "run.acked_already" } else { "run.ack" });
                    if run.is_over() && run.state.is_error() && primary(ui, !done, ito_prefs::Mark::Check, words).clicked() {
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
                    let skip = !run.is_over() && pending && ito_prefs::IconButton::new(ito_prefs::Mark::Skip, &label).show(ui).clicked();
                    if let Some(job) = picked.clone().filter(|_| skip) {
                        self.send(Request::Skip { run: run.id.clone(), job });
                    }
                    // Hold the picked job of the net: a run that comes to it waits there (held.toml, every run).
                    if let Some(job) = picked.as_deref().filter(|p| !p.contains('/')) {
                        self.hold_job_button(ui, t, &run.jobnet, job);
                    }
                    // A 確認待ち part waits for the picked answer; with none picked, the only one that waits.
                    let asking: Vec<&str> = run
                        .jobs
                        .iter()
                        .filter(|j| j.state == State::Running && matches!(j.part, koyomi_core::jobnet::Part::Approval { .. }))
                        .map(|j| j.id.as_str())
                        .collect();
                    let target =
                        picked.as_deref().filter(|p| asking.contains(p)).or(if asking.len() == 1 { asking.first().copied() } else { None });
                    let answers = [(true, "run.approve", ito_prefs::Mark::Check), (false, "run.reject", ito_prefs::Mark::Close)];
                    for (yes, key, mark) in answers.into_iter().filter(|_| target.is_some()) {
                        let label = match target {
                            Some(j) => t.trf(&format!("{key}_from"), &[j]),
                            None => t.tr(key).to_owned(),
                        };
                        if ito_prefs::IconButton::new(mark, &label).show(ui).clicked() {
                            if let Some(job) = target {
                                self.send(Request::Answer { run: run.id.clone(), job: job.to_owned(), yes });
                            }
                        }
                    }
                    // Mark this run, then pick another: the two are shown side by side under the buttons.
                    let marked = self.compare_base.as_deref() == Some(run.id.as_str());
                    let (label, mark) = if marked {
                        (t.tr("run.compare_clear"), ito_prefs::Mark::Close)
                    } else {
                        (t.tr("run.compare_mark"), ito_prefs::Mark::Compare)
                    };
                    if ito_prefs::IconButton::new(mark, label).show(ui).clicked() {
                        self.compare_base = (!marked).then(|| run.id.clone());
                    }
                    if run.is_over() && picked.is_none() {
                        ui.label(egui::RichText::new(t.tr("run.rerun_hint")).small().color(c.dim));
                    }
                });
            });
        self.show_result(ui, t);
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
        // The direction of the picture sits next to the picture.
        let options = [(false, t.tr("run.horizontal")), (true, t.tr("run.vertical"))];
        let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
        if let Some(vertical) = ui.horizontal(|ui| ito_prefs::seg(ui, look, "run-flow", self.vertical, &options)).inner {
            self.set_flow(vertical);
        }
        // Where the editor put the jobs, from the net as it is now.
        let saved = self
            .snap
            .as_ref()
            .and_then(|s| s.defs.iter().find(|d| d.id == run.jobnet))
            .and_then(|d| d.net.as_ref())
            .map(|n| n.jobs.clone());
        let mut pick = None;
        egui::ScrollArea::horizontal().id_salt("graph").auto_shrink([false, true]).show(ui, |ui| {
            pick = graph::draw(ui, &run, self.vertical, watched, saved.as_deref());
        });
        graph::legend(ui, t, &c, &run);
        if pick.is_some() {
            self.job_sel = pick;
        }
    }

    /// The run's jobs in words, one row a job, drawn as the today table. A click picks the job (as on the
    /// picture). `paned`: the heading and the header band stay put and only the rows scroll.
    fn run_jobs(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, run: &Run, watched: Option<&str>, paned: bool) {
        let c = ito_theme::colors();
        if paned {
            section_head(ui, t.tr("net.sec_jobs"));
        }
        let mut rows: Vec<(usize, Vec<(String, bool)>)> = run
            .jobs
            .iter()
            .enumerate()
            .map(|(i, j)| {
                let retried = match (j.retried, j.retry) {
                    (0, None) => String::new(),
                    (n, Some(r)) => format!("{n}/{}", r.count),
                    (n, None) => n.to_string(),
                };
                let cells = vec![
                    (format!("{}{}", show::kind(t, &j.part), j.label()), false),
                    (show::tag(t, j.state).to_string(), false),
                    (j.started.map(show::hms).unwrap_or_default(), true),
                    (j.ended.map(show::hms).unwrap_or_default(), true),
                    (j.exit.map(|x| x.to_string()).unwrap_or_default(), true),
                    (retried, true),
                    (j.duration().map(|d| show::duration(t, d)).unwrap_or_default(), false),
                    (koyomi_core::run::tidy_reason(&j.reason).to_string(), false),
                ];
                (i, cells)
            })
            .collect();
        sort_rows(&mut rows, self.run_sort);
        let heads: Vec<String> = ["run.col_job", "run.state", "run.start", "run.end", "run.exit", "run.retried", "run.took", "run.reason"]
            .map(|k| t.tr(k).to_owned())
            .to_vec();
        let mut pick = None;
        let mut body = |ui: &mut egui::Ui, table: &mut TodayTable| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, cells) in &rows {
                let j = &run.jobs[*i];
                let on = watched == Some(j.id.as_str());
                let r = table.cells_of(ui, cells, on, Some(j.state));
                ito_a11y::say(&r, Role::Row, &graph::words(t, j));
                if r.clicked() {
                    pick = Some(j.id.clone());
                }
            }
        };
        egui::ScrollArea::horizontal().id_salt("run_jobs_scroll").auto_shrink([false, true]).show(ui, |ui| {
            one_size(ui.style_mut());
            let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
            let cells: Vec<Vec<(String, bool)>> = rows.iter().map(|(_, r)| r.clone()).collect();
            let mut widths = TodayTable::fit(ui, &heads, &cells);
            // The job and the reason give way (ending in `…`) before the times are cut off.
            TodayTable::squeeze(&mut widths, ui.available_width(), &[0, 7], 72.0);
            let mut table = TodayTable::with(&c, &widths, ui.available_width());
            if let Some(col) = table.head(ui, &heads, self.run_sort, t.tr("today.sort")) {
                self.run_sort = TodayTable::next_sort(self.run_sort, col);
            }
            if paned {
                egui::ScrollArea::vertical().id_salt("run_jobs_rows").auto_shrink([true, false]).show(ui, |ui| body(ui, &mut table));
            } else {
                body(ui, &mut table);
            }
            ui.spacing_mut().item_spacing.y = gap;
        });
        if pick.is_some() {
            self.job_sel = pick;
        }
    }

    /// The picked job's facts and its log (the job running, else the last that started, when none is picked).
    fn run_detail(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, run: &Run, watched: Option<&str>) {
        let c = ito_theme::colors();
        if let Some(job) = watched.and_then(|w| run.job(w)) {
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

    /// The net the inspector on 今日 shows: that of the row picked in the table, else of the run picked
    /// on the chart.
    fn inspected(&self) -> Option<Def> {
        let id = self.today_net.clone().or_else(|| self.selected_run().map(|r| r.jobnet.clone()))?;
        self.snap.as_ref()?.defs.iter().find(|d| d.id == id).cloned()
    }

    /// The pane at the right of 今日: the picked net's name, its main buttons, its picture, its next
    /// starts, its jobs one card each and the end of its last log. Folded, it is a thin strip with ‹.
    fn inspector_pane(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        if !self.inspector {
            if folded_pane(ui, "inspector_strip", t.tr("inspect.title"), t.tr("inspect.open")) {
                self.inspector = true;
                self.save_kept();
            }
            return;
        }
        let wide = ui.available_width();
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(300.0_f32.min(wide * 0.4))
            .size_range(220.0..=(wide * 0.7).max(220.0))
            .frame(egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.border)).inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| {
                one_size(ui.style_mut());
                egui::ScrollArea::vertical().id_salt("inspector_scroll").auto_shrink(false).show(ui, |ui| self.inspector_body(ui, t));
            });
    }

    fn inspector_body(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let look = ito_prefs::Look { c, q: "" };
        let def = self.inspected();
        // The head: the name (a click opens the net's page), the fold mark at the right.
        let mut fold = false;
        let mut go = None;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                fold = ito_prefs::mark_button(ui, ito_prefs::Mark::ChevronRight, t.tr("inspect.fold"), 22.0).clicked();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let Some(def) = &def else {
                        let r = ui.label(egui::RichText::new(t.tr("inspect.title")).strong().color(c.dim));
                        ito_a11y::say(&r, Role::Heading, t.tr("inspect.title"));
                        return;
                    };
                    section_head(ui, def.label());
                    let open = ito_prefs::IconButton::new(ito_prefs::Mark::ArrowRight, t.tr("canvas.enter")).said(t.tr("today.open_net"));
                    if open.show(ui).clicked() {
                        go = Some(def.id.clone());
                    }
                });
            });
        });
        if fold {
            self.inspector = false;
            self.save_kept();
        }
        let Some(def) = def else {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(t.tr("inspect.none")).color(c.dim));
            return;
        };
        // The net's newest run today, if any: its state is the badge, its log the excerpt at the end.
        let last = self.runs.iter().filter(|r| r.jobnet == def.id).max_by_key(|r| r.started).cloned();
        let shown = self.selected_run().filter(|r| r.jobnet == def.id).cloned().or(last);
        ui.horizontal_wrapped(|ui| {
            if let Some(run) = &shown {
                let text = t.trf("inspect.last", &[&show::tag(t, run.state)]);
                let r = ito_prefs::tag(ui, look, ito_prefs::Tag::new(&text).color(graph::colour(&c, run.state)));
                ito_a11y::say(&r, Role::Label, &text);
            }
            ui.label(egui::RichText::new(&def.file).small().color(c.dim));
            self.net_file_buttons(ui, t, &def.file);
        });
        let held = self.snap.as_ref().is_some_and(|s| s.held.contains(&def.id));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            self.run_now_button(ui, t, &def, held);
            self.approve_button(ui, t, &def);
            self.edit_button(ui, t, &def);
            self.hold_button(ui, t, &def, held);
            self.send_button(ui, t, &def);
        });
        if held {
            let text = format!("{} {}", State::Held.mark(), t.tr("run.held_note"));
            let r = ui.label(egui::RichText::new(&text).color(graph::colour(&c, State::Held)));
            ito_a11y::say(&r, Role::Label, &text);
        }
        self.staged_note(ui, t, &def.id);
        self.sending_note(ui, t);
        self.show_result(ui, t);
        let section = |ui: &mut egui::Ui, key: &str| {
            ui.add_space(12.0);
            let r = ui.label(egui::RichText::new(t.tr(key)).strong().color(c.fg));
            ito_a11y::say(&r, Role::Heading, t.tr(key));
            ui.add_space(2.0);
        };
        let code = |ui: &mut egui::Ui, lines: &[(String, egui::Color32)]| {
            egui::Frame::new()
                .fill(c.bg)
                .stroke(egui::Stroke::new(1.0, c.border))
                .corner_radius(4)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    for (l, colour) in lines {
                        let r = ui.add(egui::Label::new(egui::RichText::new(l).monospace().color(*colour)).wrap());
                        ito_a11y::say(&r, Role::Label, l);
                    }
                });
        };
        if let Some(net) = def.net.as_ref().filter(|n| !n.jobs.is_empty()) {
            section(ui, "inspect.graph");
            self.net_graph(ui, t, &def, "inspector_graph");
            section(ui, "net.sec_timing");
            self.timing_card(ui, t, &def);
            section(ui, "inspect.jobs");
            let rows = review::job_rows(t, &def);
            for (i, j) in net.jobs.iter().enumerate() {
                let after: Vec<String> = j.after.iter().map(|l| l.text()).collect();
                let next: Vec<String> = net.jobs.iter().filter(|k| k.after.iter().any(|l| l.job == j.id)).map(|k| k.id.clone()).collect();
                card(&c).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        let r = ui.label(egui::RichText::new(&j.id).monospace().strong());
                        ito_a11y::say(&r, Role::Heading, &j.id);
                        ui.label(egui::RichText::new(t.tr(&format!("part.{}", j.part.key()))).color(c.dim));
                    });
                    let mut fact = |key: &str, v: String| {
                        if !v.is_empty() {
                            let text = format!("{}: {v}", t.tr(key));
                            let r = ui.label(egui::RichText::new(&text).color(c.dim));
                            ito_a11y::say(&r, Role::Label, &text);
                        }
                    };
                    fact("inspect.name", j.name.clone().unwrap_or_default());
                    fact("inspect.before", after.join(", "));
                    fact("inspect.after", next.join(", "));
                    if let Some(cmd) = rows.get(i).and_then(|r| r.command.clone()) {
                        code(ui, &[(cmd, c.fg)]);
                    }
                });
                ui.add_space(4.0);
            }
        } else if def.net.is_some() {
            section(ui, "net.sec_timing");
            self.timing_card(ui, t, &def);
        }
        // The end of the newest run's log: the job that failed, else the one running or the last.
        if let Some(run) = shown {
            let job = run.jobs.iter().find(|j| j.state.is_error()).map(|j| j.id.clone()).or_else(|| Self::watched_job(&run, None));
            if let Some(job) = job {
                *self.watch.lock().unwrap_or_else(|e| e.into_inner()) = Some((run.id.clone(), job.clone()));
                section(ui, "inspect.log");
                ui.label(egui::RichText::new(format!("{} / {job}", run.id)).small().color(c.dim));
                let text = self.log.as_ref().filter(|(r, j, _)| r == &run.id && j == &job).map(|(_, _, s)| s.as_str()).unwrap_or("");
                let colour = if run.state.is_error() { c.err } else { c.fg };
                let lines: Vec<&str> = text.lines().collect();
                let tail: Vec<(String, egui::Color32)> =
                    lines[lines.len().saturating_sub(8)..].iter().map(|l| ((*l).to_owned(), colour)).collect();
                if tail.is_empty() {
                    ui.label(egui::RichText::new(t.tr("inspect.no_log")).color(c.dim));
                } else {
                    code(ui, &tail);
                }
            }
        }
        if let Some(id) = go {
            self.follow(|b| b.open_net(&id));
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
        self.net_file_buttons(ui, t, &def.file);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            self.approve_button(ui, t, &def);
            self.edit_button(ui, t, &def);
            self.run_now_button(ui, t, &def, held);
            self.hold_button(ui, t, &def, held);
            self.send_button(ui, t, &def);
            // The job picked on the graph or in the jobs table is held from here too, next to the net's own hold.
            let job = self.job_sel.clone().filter(|p| def.net.as_ref().is_some_and(|n| n.jobs.iter().any(|j| &j.id == p)));
            if let Some(job) = job {
                self.hold_job_button(ui, t, &def.id, &job);
            }
        });
        if held {
            let text = format!("{} {}", State::Held.mark(), t.tr("run.held_note"));
            let r = ui.label(egui::RichText::new(&text).color(graph::colour(&c, State::Held)));
            ito_a11y::say(&r, Role::Label, &text);
        }
        self.staged_note(ui, t, &def.id);
        self.sending_note(ui, t);
        self.show_result(ui, t);
        ui.add_space(6.0);
        for p in &def.problems {
            let line = p.in_file(&def.file);
            ui.label(egui::RichText::new(format!("{} {line}", ito_a11y::State::Error.mark())).color(c.err));
        }
        let section = |ui: &mut egui::Ui, key: &str| {
            ui.add_space(10.0);
            section_head(ui, t.tr(key));
        };
        if def.net.as_ref().is_some_and(|n| !n.jobs.is_empty()) {
            section(ui, "net.sec_graph");
            self.net_graph(ui, t, &def, "net_graph");
        }
        // Its next starts are in the pane at the right.
        let lines = review::settings(t, &def);
        if !lines.is_empty() {
            section(ui, "net.sec_settings");
            for l in lines {
                ui.label(l);
            }
        }
    }

    /// 確かめる…: opens the box that marks the net 確認済み (only while it is not).
    fn approve_button(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def) {
        let can = def.standing == Standing::Unconfirmed && !self.blocked_for(def);
        let label = match &self.easy {
            Some(lang) => format!("{}…", crate::glossary::GLOSSARY.label("approve", lang, true, "")),
            None => t.tr("confirm.approve").to_owned(),
        };
        let r = ui.add_enabled(can, |ui: &mut egui::Ui| ito_prefs::IconButton::new(ito_prefs::Mark::Check, &label).show(ui));
        if let Some(lang) = &self.easy {
            ito_glossary::hover(ui, ui.painter(), r.rect, &crate::glossary::GLOSSARY, "approve", lang, "confirm");
        }
        if r.clicked() && !self.box_open() {
            self.asking = Some(Approval { id: def.id.clone(), sha256: def.sha256.clone() });
            self.focus = 0;
        }
    }

    /// 編集: the net in the editor.
    fn edit_button(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def) {
        let edit = ito_prefs::IconButton::new(ito_prefs::Mark::Pencil, t.tr("canvas.edit"));
        let r = ui.add_enabled(def.net.is_some(), |ui: &mut egui::Ui| edit.show(ui));
        if r.clicked() {
            self.open_editor(&def.id);
            self.focus = 0;
        }
    }

    /// 今すぐ流す: the page's main action, filled with the accent.
    fn run_now_button(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def, held: bool) {
        let can = def.standing == Standing::Confirmed && !self.blocked() && !held;
        let run = ito_prefs::IconButton::new(ito_prefs::Mark::Play, t.tr("run.now")).tone(ito_prefs::Tone::Primary);
        if ui.add_enabled(can, |ui: &mut egui::Ui| run.show(ui)).clicked() && can {
            self.run_now(def.id.clone(), def.net.as_ref().map(|n| &n.params));
        }
    }

    /// Holds the net, or releases it: on hold its times are written down as ⏸ and nothing runs.
    fn hold_button(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def, held: bool) {
        let (key, mark) = if held { ("run.release", ito_prefs::Mark::Play) } else { ("run.hold", ito_prefs::Mark::Pause) };
        let r = ui.add_enabled(!self.blocked(), |ui: &mut egui::Ui| ito_prefs::IconButton::new(mark, t.tr(key)).show(ui));
        if r.clicked() {
            let id = def.id.clone();
            self.send(if held { Request::Release { id, job: None } } else { Request::Hold { id, job: None } });
        }
    }

    /// The net's picture, laid out as the editor placed it; a click on a job picks it.
    fn net_graph(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def, salt: &str) {
        let c = ito_theme::colors();
        if let Some(net) = def.net.as_ref().filter(|n| !n.jobs.is_empty()) {
            let now = self.engine_now();
            let mut preview = Run::new("", &def.id, def.label(), "", koyomi_core::run::How::Manual, net, now);
            // A held job shows ⏸ on its station.
            let mut any_held = false;
            for j in preview.jobs.iter_mut().filter(|j| self.job_held(&def.id, &j.id)) {
                j.state = State::Held;
                any_held = true;
            }
            let picked = self.job_sel.clone().filter(|p| net.jobs.iter().any(|j| &j.id == p));
            let mut pick = None;
            egui::ScrollArea::horizontal().id_salt(salt).auto_shrink([false, true]).show(ui, |ui| {
                pick = graph::draw(ui, &preview, self.vertical, picked.as_deref(), Some(&net.jobs));
            });
            if pick.is_some() {
                self.job_sel = if pick == picked { None } else { pick };
            }
            if any_held {
                let text = t.tr("run.job_held_note");
                let r = ui.label(egui::RichText::new(text).color(graph::colour(&c, State::Held)));
                ito_a11y::say(&r, Role::Label, text);
            }
        }
    }

    /// The net's next ten starts, in figures of one width; the first, the one coming, is bold.
    /// The net's next times in the inspector, in a card as each of its jobs is.
    fn timing_card(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def) {
        card(&ito_theme::colors()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            self.net_timing(ui, t, def);
        });
    }

    fn net_timing(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def) {
        let c = ito_theme::colors();
        if let Some(net) = &def.net {
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
                let stops = self.snap.as_ref().map(|s| s.blackouts.as_slice()).unwrap_or(&[]);
                let now = self.engine_now();
                let rows = show::next_rows(t, &net.when, (&net.quiet, net.on_quiet), cal, now, 10, power.as_ref(), stops);
                if rows.is_empty() {
                    ui.label(egui::RichText::new(t.tr("run.next_over")).color(c.dim));
                }
                for (i, row) in rows.iter().enumerate() {
                    let text = egui::RichText::new(format!("  {row}")).monospace();
                    let r = ui.label(if i == 0 { text.strong() } else { text });
                    ito_a11y::say(&r, Role::Label, row);
                }
            }
        }
    }

    /// The net's jobs, one row a job: the pane under the net's page.
    fn jobs_pane(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(def) = self.selected_def().cloned() else { return };
        {
            section_head(ui, t.tr("net.sec_jobs"));
            if let Some(net) = def.net.as_ref().filter(|n| !n.jobs.is_empty()) {
                // One row a job, drawn as the today table: what it is, what comes before and after it, what it
                // runs and where, and the rest of its settings. A click picks the job (as on the graph).
                let details = review::job_rows(t, &def);
                let mut rows: Vec<(usize, Vec<(String, bool)>)> = net
                    .jobs
                    .iter()
                    .enumerate()
                    .map(|(i, j)| {
                        let row = details.get(i);
                        let after: Vec<String> = j.after.iter().map(|l| l.text()).collect();
                        // The jobs that wait for this one, written as their own `after` names it.
                        let next: Vec<String> = net
                            .jobs
                            .iter()
                            .flat_map(|k| {
                                k.after.iter().filter(|l| l.job == j.id).map(move |l| format!("{}{}", k.id, &l.text()[l.job.len()..]))
                            })
                            .collect();
                        let cells = vec![
                            (j.id.clone(), true),
                            (t.tr(&format!("part.{}", j.part.key())).to_owned(), false),
                            (j.name.clone().unwrap_or_default(), false),
                            (after.join(", "), true),
                            (next.join(", "), true),
                            (row.and_then(|r| r.command.clone()).unwrap_or_default(), true),
                            (row.and_then(|r| r.dir.clone()).unwrap_or_default(), true),
                            (
                                row.map(|r| {
                                    r.rest.join(
                                        "
",
                                    )
                                })
                                .unwrap_or_default(),
                                false,
                            ),
                        ];
                        (i, cells)
                    })
                    .collect();
                sort_rows(&mut rows, self.net_sort);
                let heads: Vec<String> = ["col_id", "col_kind", "col_name", "col_after", "col_next", "col_run", "col_dir", "col_more"]
                    .map(|k| t.tr(&format!("net.{k}")).to_owned())
                    .to_vec();
                let picked = self.job_sel.clone();
                let mut pick = None;
                // The heading above and the header band stay put; only the rows scroll up and down. Sideways the
                // header and the rows scroll together.
                egui::ScrollArea::horizontal().id_salt("net_jobs_scroll").auto_shrink(false).show(ui, |ui| {
                    one_size(ui.style_mut());
                    let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
                    let cells: Vec<Vec<(String, bool)>> = rows.iter().map(|(_, r)| r.clone()).collect();
                    let mut widths = TodayTable::fit(ui, &heads, &cells);
                    // The words (name, the jobs before and after, the command, its folder, the rest) end in
                    // `…` before the table runs off the side; the ID and the kind stay whole.
                    TodayTable::squeeze(&mut widths, ui.available_width(), &[2, 3, 4, 5, 6, 7], 72.0);
                    let mut table = TodayTable::with(&c, &widths, ui.available_width());
                    if let Some(col) = table.head(ui, &heads, self.net_sort, t.tr("today.sort")) {
                        self.net_sort = TodayTable::next_sort(self.net_sort, col);
                    }
                    egui::ScrollArea::vertical().id_salt("net_jobs_rows").auto_shrink([true, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, cells) in &rows {
                            let id = &net.jobs[*i].id;
                            let on = picked.as_deref() == Some(id.as_str());
                            let r = table.cells(ui, cells, on);
                            let said: Vec<&str> = cells.iter().map(|(t, _)| t.as_str()).filter(|t| !t.is_empty()).collect();
                            ito_a11y::say(&r, Role::Button, &said.join(" "));
                            if r.clicked() {
                                pick = Some(if on { None } else { Some(id.clone()) });
                            }
                        }
                    });
                    ui.spacing_mut().item_spacing.y = gap;
                });
                if let Some(p) = pick {
                    self.job_sel = p;
                }
            }
        }
    }

    /// The open and copy buttons for a net's definition file; only when the engine is on this PC, whose
    /// `jobnets` folder holds it.
    pub(super) fn net_file_buttons(&self, ui: &mut egui::Ui, t: &ito_i18n::Table, file: &str) {
        if self.link.is_far() {
            return;
        }
        if let Some(path) = paths::net_file(file) {
            paths::path_buttons(ui, t, &path, paths::Place::Engine, true);
        }
    }

    /// The engine's answer to the last request (approve, run, rerun), under the buttons.
    fn show_result(&self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        match &self.result {
            Some(Ok(m)) => {
                ui.label(egui::RichText::new(format!("{} {m}", ito_a11y::State::Done.mark())).color(c.done));
                if let Some(made) = paths::Made::for_result(&self.result_path, &self.result) {
                    paths::path_buttons(ui, t, &made.path, made.place, !self.link.is_far());
                }
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
        if def.sha256 != ask.sha256 || def.standing != Standing::Unconfirmed || self.blocked_for(&def) {
            self.asking = None;
            return;
        }
        let mut body = review::body(t, &def);
        let none = std::collections::BTreeMap::new();
        let week = review::week_in(t, &def, self.snap.as_ref().map_or(&none, |s| &s.calendars), self.engine_now());
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
            marks: &[Some(ito_prefs::Mark::Check), Some(ito_prefs::Mark::Close)],
            focus: Some(self.focus),
            primary: Some(0),
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &Self::style()).or(key) {
            Some(0) => {
                let (id, sha256) = (ask.id.clone(), ask.sha256.clone());
                self.send(Request::Approve { id, sha256 });
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
            marks: &[Some(ito_prefs::Mark::Stop), Some(ito_prefs::Mark::Close)],
            focus: Some(self.focus),
            primary: Some(0),
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &Self::style()).or(key) {
            Some(0) => {
                self.send(Request::Kill { run: id, job: None });
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
            body.push(koyomi_core::msg::line(koyomi_core::msg::DELETE_BLOCKED, &[&users.join(", ")]));
        }
        let buttons = [t.tr("delete.yes").to_owned(), t.tr("make.cancel").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("delete.title"),
            rows: &rows,
            body: &body,
            buttons: &buttons,
            marks: &[Some(ito_prefs::Mark::Trash), Some(ito_prefs::Mark::Close)],
            focus: Some(self.focus),
            primary: Some(0),
            danger: true,
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &Self::style()).or(key) {
            Some(0) => {
                self.deleting = None;
                self.result = None;
                self.send(Request::Delete { id });
            }
            Some(_) => self.deleting = None,
            None => {}
        }
    }
}

/// A screen's one main button (D13): a mark and words, filled with the theme's accent, greyed when not `enabled`.
pub(crate) fn primary(ui: &mut egui::Ui, enabled: bool, mark: ito_prefs::Mark, words: &str) -> egui::Response {
    ui.add_enabled(enabled, |ui: &mut egui::Ui| ito_prefs::IconButton::new(mark, words).tone(ito_prefs::Tone::Primary).show(ui))
}

/// A button that does something hard to take back (kill), in the theme's danger tone.
pub(crate) fn danger(ui: &mut egui::Ui, mark: ito_prefs::Mark, words: &str) -> egui::Response {
    ito_prefs::IconButton::new(mark, words).tone(ito_prefs::Tone::Danger).show(ui)
}

/// The foot of a box: a blank line under what it asks, then its buttons at its right edge, as ito's confirm box lays
/// them. `buttons` draws them right to left: the rightmost first.
pub(crate) fn box_foot<R>(ui: &mut egui::Ui, buttons: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
    // Sized to its own height: a bare `with_layout` would take all the room left and grow the box every frame.
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 0.0), layout, buttons).inner
}

/// A box's two buttons at its foot: what it is for (filled, with `mark`), then one that closes it doing nothing,
/// rightmost. `Some(0)` or `Some(1)` for the one clicked, as Enter and Esc give.
pub(crate) fn box_answer(ui: &mut egui::Ui, mark: ito_prefs::Mark, go: &str, cancel: &str) -> Option<usize> {
    box_answer_on(ui, true, mark, go, cancel)
}

/// [`box_answer`] with the go button off while `enabled` is false (a request is with the engine).
pub(crate) fn box_answer_on(ui: &mut egui::Ui, enabled: bool, mark: ito_prefs::Mark, go: &str, cancel: &str) -> Option<usize> {
    box_foot(ui, |ui| {
        let cancel = ito_prefs::IconButton::new(ito_prefs::Mark::Close, cancel).show(ui).clicked();
        let go = primary(ui, enabled, mark, go).clicked();
        if go {
            Some(0)
        } else {
            cancel.then_some(1)
        }
    })
}

/// A row's two buttons: what it is for (filled, with `mark`), then one that closes it doing nothing. `Some(0)` or
/// `Some(1)` for the one clicked, as Enter and Esc give.
pub(crate) fn go_or_cancel(ui: &mut egui::Ui, mark: ito_prefs::Mark, go: &str, cancel: &str) -> Option<usize> {
    let go = primary(ui, true, mark, go).clicked();
    let cancel = ito_prefs::IconButton::new(ito_prefs::Mark::Close, cancel).show(ui).clicked();
    if go {
        Some(0)
    } else {
        cancel.then_some(1)
    }
}

/// What a chip over today's table picks out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TodayKind {
    /// Failed and not marked handled yet.
    Unhandled,
    /// In this state (a start still to come counts as 待ち).
    Is(State),
}

impl TodayKind {
    /// The chips a run of `state` counts under; `acked` when someone marked it handled.
    fn of(state: State, acked: bool) -> Vec<Self> {
        let mut kinds = vec![Self::Is(state)];
        if state.is_error() && !acked {
            kinds.push(Self::Unhandled);
        }
        kinds
    }
}

/// A clock time as a number to sort by.
fn stamp(at: chrono::NaiveDateTime) -> i64 {
    at.and_utc().timestamp_millis()
}

/// One row of today's table, with what it sorts by.
struct TodayRow {
    state: State,
    title: String,
    said: String,
    cells: [String; 5],
    /// The run's id; `None` for a start still to come.
    run: Option<String>,
    /// The net's id.
    net: String,
    kinds: Vec<TodayKind>,
    /// planned, start, end (clock times); time (ms); next (as `stamp`).
    keys: TodayKeys,
}

type TodayKeys = (Option<chrono::NaiveDateTime>, Option<chrono::NaiveDateTime>, Option<chrono::NaiveDateTime>, Option<i64>, Option<i64>);

impl TodayRow {
    /// Failures first, then what runs, what waits, what is held, what is done.
    fn rank(state: State) -> u8 {
        use koyomi_core::run::Tone;
        match state.tone() {
            Tone::Error => 0,
            Tone::Running => 1,
            Tone::Warn | Tone::Held => 2,
            Tone::Dim => 3,
            Tone::Ok => 4,
        }
    }

    /// Sorts by column `col` (state, net, planned, start, end, time, next), down when `down`.
    /// Rows with nothing in the column go last either way; ties keep their order.
    fn sort(rows: &mut [TodayRow], col: usize, down: bool) {
        fn by<T: Ord>(a: Option<T>, b: Option<T>, down: bool) -> std::cmp::Ordering {
            match (a, b) {
                (Some(a), Some(b)) => {
                    if down {
                        b.cmp(&a)
                    } else {
                        a.cmp(&b)
                    }
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        }
        rows.sort_by(|a, b| match col {
            0 => by(Some(Self::rank(a.state)), Some(Self::rank(b.state)), down),
            1 => by(Some(a.title.to_lowercase()), Some(b.title.to_lowercase()), down),
            2 => by(a.keys.0.map(stamp), b.keys.0.map(stamp), down),
            3 => by(a.keys.1.map(stamp), b.keys.1.map(stamp), down),
            4 => by(a.keys.2.map(stamp), b.keys.2.map(stamp), down),
            5 => by(a.keys.3, b.keys.3, down),
            _ => by(a.keys.4, b.keys.4, down),
        });
    }
}

/// Today's table, drawn row by row: a header band, then rows with the state (mark and word) in its
/// colour, the net, and the times in a fixed-width face. A picked row is filled with a stripe at the
/// left in its state's colour.
struct TodayTable {
    c: ito_theme::Colors,
    /// The left edge of each column, from the table's left.
    cols: Vec<f32>,
    width: f32,
}

impl TodayTable {
    const ROW: f32 = 26.0;
    const PAD: f32 = 10.0;

    fn new(ui: &egui::Ui, c: &ito_theme::Colors) -> Self {
        let width = ui.available_width().max(640.0);
        // State, net (the rest), planned, start, end, time, next.
        let fixed = [110.0, 0.0, 110.0, 80.0, 80.0, 170.0, 100.0];
        let net = (width - fixed.iter().sum::<f32>()).max(160.0);
        let widths: Vec<f32> = fixed.iter().enumerate().map(|(i, w)| if i == 1 { net } else { *w }).collect();
        Self::with(c, &widths, width)
    }

    /// A table of these column widths, `width` wide at least (the last column takes what is over).
    fn with(c: &ito_theme::Colors, widths: &[f32], width: f32) -> Self {
        let mut cols = Vec::with_capacity(widths.len());
        let mut x = 0.0;
        for w in widths {
            cols.push(x);
            x += w;
        }
        Self { c: *c, cols, width: width.max(x) }
    }

    /// The sort after a click on column `col`: up first, then down, then back to none.
    fn next_sort(was: Option<(usize, bool)>, col: usize) -> Option<(usize, bool)> {
        match was {
            Some((w, false)) if w == col => Some((col, true)),
            Some((w, true)) if w == col => None,
            _ => Some((col, false)),
        }
    }

    fn cell_right(&self, i: usize) -> f32 {
        self.cols.get(i + 1).copied().unwrap_or(self.width)
    }

    /// The header band. A click on a column's name gives its index; the sorted column shows ▲ or ▼.
    fn head(&mut self, ui: &mut egui::Ui, heads: &[String], sort: Option<(usize, bool)>, hint: &str) -> Option<usize> {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(self.width, Self::ROW - 2.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, self.c.panel);
        ui.painter().hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0, self.c.border));
        let font = egui::TextStyle::Small.resolve(ui.style());
        let mut hit = None;
        for (i, h) in heads.iter().enumerate() {
            let cell = egui::Rect::from_x_y_ranges(rect.left() + self.cols[i]..=rect.left() + self.cell_right(i), rect.y_range());
            let r = ui.interact(cell, ui.id().with(("table-head", i)), egui::Sense::click());
            let arrow = match sort {
                Some((col, false)) if col == i => " ▲",
                Some((col, true)) if col == i => " ▼",
                _ => "",
            };
            let ink = if !arrow.is_empty() || r.hovered() { self.c.fg } else { self.c.dim };
            if r.hovered() {
                ui.painter().rect_filled(cell, 0.0, self.c.hover());
            }
            // A squeezed column's head ends in `…` (the whole of it on hover) rather than running into the next.
            let g = one_line(ui, &format!("{h}{arrow}"), font.clone(), ink, cell.width() - Self::PAD - 4.0);
            let cut = g.elided;
            ui.painter().galley(egui::pos2(cell.left() + Self::PAD, cell.center().y - g.size().y / 2.0), g, ink);
            let r = r
                .on_hover_text(if cut {
                    format!(
                        "{h}
{hint}"
                    )
                } else {
                    hint.to_owned()
                })
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let said = match arrow {
                " ▲" => format!("{h} ▲"),
                " ▼" => format!("{h} ▼"),
                _ => h.clone(),
            };
            ito_a11y::say(&r, Role::Button, &said);
            if r.clicked() {
                hit = Some(i);
            }
        }
        hit
    }

    /// One row: `cells` follow the state and the net (on 今日 planned, start, end, time and next; the
    /// sixth column is in words). Empty cells show a dim dash.
    #[allow(clippy::too_many_arguments)]
    fn row(&mut self, ui: &mut egui::Ui, state: State, word: &str, title: &str, cells: &[String], on: bool, click: bool) -> egui::Response {
        let sense = if click { egui::Sense::click() } else { egui::Sense::hover() };
        let (rect, r) = ui.allocate_exact_size(egui::vec2(self.width, Self::ROW), sense);
        if !ui.is_rect_visible(rect) {
            return r;
        }
        let c = &self.c;
        let colour = graph::colour(c, state);
        let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        // Each row is tinted with its state's colour (a failure reads red at a glance); a hover deepens
        // the tint, a picked row is filled and striped at the left.
        let tint = |t: f32| ito_theme::mix(c.bg, colour, t);
        if on {
            p.rect_filled(rect, 0.0, c.chosen());
            p.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, colour);
        } else if click && r.hovered() {
            p.rect_filled(rect, 0.0, tint(0.22));
        } else {
            p.rect_filled(rect, 0.0, tint(0.1));
        }
        p.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, c.border));
        let body = egui::TextStyle::Body.resolve(ui.style());
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        let y = rect.center().y;
        let at = |i: usize| rect.left() + self.cols[i] + Self::PAD;
        let clip =
            |i: usize| egui::Rect::from_x_y_ranges(rect.left() + self.cols[i]..=rect.left() + self.cell_right(i) - 4.0, rect.y_range());
        p.with_clip_rect(clip(0)).text(egui::pos2(at(0), y), egui::Align2::LEFT_CENTER, word, body.clone(), graph::mark_colour(c, state));
        let ink = if on { c.strong() } else { c.fg };
        p.with_clip_rect(clip(1)).text(egui::pos2(at(1), y), egui::Align2::LEFT_CENTER, title, body.clone(), ink);
        for (k, cell) in cells.iter().enumerate() {
            let i = k + 2;
            let (text, ink) = if cell.is_empty() { ("—", c.dim) } else { (cell.as_str(), c.fg) };
            // The time column carries words ("12分… (平均 10分)"); the rest are clock times.
            let font = if i == 5 { body.clone() } else { mono.clone() };
            p.with_clip_rect(clip(i)).text(egui::pos2(at(i), y), egui::Align2::LEFT_CENTER, text, font, ink);
        }
        if click {
            r.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            r
        }
    }
}

impl TodayTable {
    /// A row of plain cells (`(text, fixed-width)`; empty shows a dim dash, a line break makes the row
    /// taller), without a state: hover tints it, a picked row is filled and striped at the left in `run`.
    fn cells(&mut self, ui: &mut egui::Ui, cells: &[(String, bool)], on: bool) -> egui::Response {
        self.cells_of(ui, cells, on, None)
    }

    /// [`Self::cells`] for a job of a run: the row is tinted with its state's colour (as the today table) and
    /// its first two cells (the job and its state) are written in that colour.
    fn cells_of(&mut self, ui: &mut egui::Ui, cells: &[(String, bool)], on: bool, state: Option<State>) -> egui::Response {
        let body = egui::TextStyle::Body.resolve(ui.style());
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        let lines = cells.iter().map(|(t, _)| t.lines().count().max(1)).max().unwrap_or(1);
        let line = ui.fonts_mut(|f| f.row_height(&body));
        let height = Self::ROW.max(lines as f32 * line + 10.0);
        let (rect, r) = ui.allocate_exact_size(egui::vec2(self.width, height), egui::Sense::click());
        if !ui.is_rect_visible(rect) {
            return r;
        }
        let c = &self.c;
        let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let colour = state.map(|s| graph::colour(c, s));
        if on {
            p.rect_filled(rect, 0.0, c.chosen());
            p.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, colour.unwrap_or(c.run));
        } else if let Some(colour) = colour {
            p.rect_filled(rect, 0.0, ito_theme::mix(c.bg, colour, if r.hovered() { 0.22 } else { 0.1 }));
        } else if r.hovered() {
            p.rect_filled(rect, 0.0, c.hover());
        }
        p.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, c.border));
        let ink = if on { c.strong() } else { c.fg };
        // One line sits in the middle as in the today table; a taller row starts its cells at the top.
        let (y, align) = if lines == 1 {
            (rect.center().y, egui::Align2::LEFT_CENTER)
        } else {
            (rect.top() + (Self::ROW - line) / 2.0, egui::Align2::LEFT_TOP)
        };
        let mut cut = Vec::new();
        for (i, (text, fixed)) in cells.iter().enumerate() {
            let left = rect.left() + self.cols[i] + Self::PAD;
            let right = rect.left() + self.cell_right(i) - 4.0;
            let ink = match colour {
                Some(colour) if i < 2 => colour,
                _ => ink,
            };
            let (text, ink) = if text.is_empty() { ("—", c.dim) } else { (text.as_str(), ink) };
            // The dash is the same in every column, fixed-width or not.
            let font = if *fixed && text != "—" { mono.clone() } else { body.clone() };
            if lines == 1 {
                // A column narrowed by [`Self::squeeze`] ends its words in `…`; the whole reads on hover.
                let g = one_line(ui, text, font, ink, right - left);
                if g.elided {
                    cut.push(text);
                }
                p.galley(egui::pos2(left, y - g.size().y / 2.0), g, ink);
            } else {
                let clip = egui::Rect::from_x_y_ranges(rect.left() + self.cols[i]..=right, rect.y_range());
                p.with_clip_rect(clip).text(egui::pos2(left, y), align, text, font, ink);
            }
        }
        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
        if cut.is_empty() {
            r
        } else {
            r.on_hover_text(cut.join(
                "
",
            ))
        }
    }

    /// Narrows the columns in `flex` (words, which can end in `…`) until `widths` fit `room`, each by
    /// its share of what it has over `min`; the others (clock times) keep theirs. A head that no longer
    /// fits ends in `…` as well. What still does not fit scrolls sideways.
    fn squeeze(widths: &mut [f32], room: f32, flex: &[usize], min: f32) {
        let over = widths.iter().sum::<f32>() - room;
        let spare: f32 = flex.iter().filter_map(|&i| widths.get(i)).map(|w| (w - min).max(0.0)).sum();
        if over <= 0.0 || spare <= 0.0 {
            return;
        }
        let k = (over / spare).min(1.0);
        for &i in flex {
            if let Some(w) = widths.get_mut(i) {
                *w -= (*w - min).max(0.0) * k;
            }
        }
    }

    /// Column widths that fit `heads` and `rows` (each cell's longest line), padded.
    fn fit(ui: &egui::Ui, heads: &[String], rows: &[Vec<(String, bool)>]) -> Vec<f32> {
        let body = egui::TextStyle::Body.resolve(ui.style());
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        let small = egui::TextStyle::Small.resolve(ui.style());
        let p = ui.painter();
        let w = |t: &str, f: &egui::FontId| p.layout_no_wrap(t.to_owned(), f.clone(), egui::Color32::WHITE).size().x;
        heads
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let cells = rows.iter().filter_map(|r| r.get(i)).map(|(t, fixed)| w(t, if *fixed { &mono } else { &body }));
                // Room for the sort arrow next to the head; the pad at the left, and a gap before the next
                // column's pad (its text then stands 18 points off).
                cells.fold(w(&format!("{h} ▼"), &small), f32::max) + Self::PAD + 8.0
            })
            .collect()
    }
}

/// Sorts a jobs table's `(order, cells)` rows by `sort`. Column 0 (the job) sorts in its own order; the rest by
/// their words with numbers as numbers (job2 before job10), the empty ones last.
fn sort_rows(rows: &mut [(usize, Vec<(String, bool)>)], sort: Option<(usize, bool)>) {
    let Some((col, down)) = sort else { return };
    rows.sort_by(|(ia, a), (ib, b)| {
        let o = if col == 0 {
            ia.cmp(ib)
        } else {
            let (x, y) = (&a[col].0, &b[col].0);
            match (x.is_empty(), y.is_empty()) {
                (true, false) => return std::cmp::Ordering::Greater,
                (false, true) => return std::cmp::Ordering::Less,
                _ => ito_common::text::natural_cmp(x, y, false),
            }
        };
        if down {
            o.reverse()
        } else {
            o
        }
    });
}

/// The room around a page under the tabs. The central panel keeps none at the right and bottom, so
/// a pane there meets the window's edges; the page's own content keeps this room instead.
pub(super) fn page() -> egui::Frame {
    egui::Frame::new().inner_margin(egui::Margin { left: 0, top: 12, right: 8, bottom: 8 })
}

/// A folded pane at the right: a thin strip with the fold mark at its top and the pane's `title`
/// written down its length. True when the mark was pressed to open the pane again.
pub(super) fn folded_pane(ui: &mut egui::Ui, id: &str, title: &str, words: &str) -> bool {
    let c = ito_theme::colors();
    let mut open = false;
    egui::Panel::right(id.to_owned())
        .resizable(false)
        .exact_size(34.0)
        .frame(egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.border)).inner_margin(egui::Margin::symmetric(6, 10)))
        .show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.vertical_centered(|ui| {
                open = ito_prefs::mark_button(ui, ito_prefs::Mark::ChevronLeft, words, 22.0).clicked();
                ui.add_space(10.0);
                // The title reads top to bottom, turned a quarter to the right.
                let font = egui::FontId::proportional(ONE_SIZE);
                let galley = ui.painter().layout_no_wrap(title.to_owned(), font, c.dim);
                let size = galley.size();
                let (rect, r) = ui.allocate_exact_size(egui::vec2(size.y, size.x), egui::Sense::click());
                ito_a11y::say(&r, Role::Label, title);
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    open = true;
                }
                let at = egui::pos2(rect.right(), rect.top());
                ui.painter().add(egui::epaint::TextShape::new(at, galley, c.dim).with_angle(std::f32::consts::FRAC_PI_2));
            });
        });
    open
}

/// Scrolls a table's row to the middle of the view, up or down only. `scroll_to_me` would also
/// centre a row wider than the view sideways, and cut off the table's first columns.
fn reveal_row(ui: &egui::Ui, r: &egui::Response) {
    let rect = egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), r.rect.y_range());
    ui.scroll_to_rect(rect, Some(egui::Align::Center));
}

/// The tree and the today page read in one size: every text style at [`ONE_SIZE`] (times stay
/// fixed-width).
fn one_size(style: &mut egui::Style) {
    for (s, f) in style.text_styles.iter_mut() {
        let family = if *s == egui::TextStyle::Monospace { egui::FontFamily::Monospace } else { egui::FontFamily::Proportional };
        *f = egui::FontId::new(ONE_SIZE, family);
    }
}

/// The one text size of the tree and the today page: that of `ito_prefs::tag`, so the chips and
/// badges read the same as the words around them.
pub(crate) const ONE_SIZE: f32 = 11.5;

/// A group's heading in the tree: its fold mark and name, and how many nets in it wait for a person
/// as a small badge at the right.
fn tree_group(ui: &mut egui::Ui, c: &ito_theme::Colors, text: &str, todo: usize) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    let mut elided = false;
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if r.hovered() {
            p.rect_filled(rect, 4.0, c.hover());
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut right = rect.right() - 4.0;
        if todo > 0 {
            let words = format!("✕{todo}");
            let tag = ito_prefs::Tag::new(&words).color(c.err);
            let w = ito_prefs::tag_width(ui, tag);
            right -= w;
            ito_prefs::paint_tag(p, ui.ctx(), c, egui::pos2(right, rect.center().y - 8.5), tag, w);
        }
        let g = one_line(ui, text, font, c.strong(), right - 4.0 - (rect.left() + 4.0));
        elided = g.elided;
        p.galley(egui::pos2(rect.left() + 4.0, rect.center().y - g.size().y / 2.0), g, c.strong());
    }
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if elided {
        r.on_hover_text(text)
    } else {
        r
    }
}

/// One line of text no wider than `room`: what does not fit ends in `…` (the galley says it was cut).
fn one_line(ui: &egui::Ui, text: &str, font: egui::FontId, ink: egui::Color32, room: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(0.0));
    ui.painter().layout_job(job)
}

/// A net's row in the tree, a step in from its group: the mark of its last run today in that run's
/// colour, then the net, and at the right what to know about it (`未確認`, `保留`) kept whole while
/// a long name ends in `…`. The open one is filled with a stripe at the left; a failed one has a
/// dot at the right.
fn tree_row(ui: &mut egui::Ui, c: &ito_theme::Colors, on: bool, last: Option<State>, text: &str, tail: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    let mut elided = false;
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if on {
            p.rect_filled(rect, 4.0, c.chosen());
            p.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, c.run);
        } else if r.hovered() {
            p.rect_filled(rect, 4.0, c.hover());
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let y = rect.center().y;
        if let Some(l) = last {
            p.text(egui::pos2(rect.left() + 24.0, y), egui::Align2::CENTER_CENTER, l.mark(), font.clone(), graph::colour(c, l));
        }
        let failed = last.is_some_and(State::is_error);
        let mut right = rect.right() - if failed { 16.0 } else { 4.0 };
        if failed {
            p.circle_filled(egui::pos2(rect.right() - 8.0, y), 3.0, c.err);
        }
        let left = rect.left() + 36.0;
        if !tail.is_empty() {
            // The word keeps its place even in a narrow tree; a few letters of the name stay before it.
            let g = p.layout_no_wrap(tail.to_owned(), font.clone(), c.wait_text());
            let x = (right - g.size().x).max(left + 48.0);
            let clip = egui::Rect::from_x_y_ranges(x..=right, rect.y_range());
            p.with_clip_rect(clip).galley(egui::pos2(x, y - g.size().y / 2.0), g, c.wait_text());
            right = x - 8.0;
        }
        let ink = if on { c.strong() } else { c.fg };
        let g = one_line(ui, text, font, ink, right - left);
        elided = g.elided;
        p.galley(egui::pos2(left, y - g.size().y / 2.0), g, ink);
    }
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if elided {
        r.on_hover_text(text)
    } else {
        r
    }
}

/// A page tab as a pill: the open one raised with a border and a dot in the running colour, the
/// others plain dim words that light up under the pointer.
fn pill_tab(ui: &mut egui::Ui, c: &ito_theme::Colors, on: bool, label: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let ink = if on { c.fg } else { c.dim };
    let g = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let dot = if on { 12.0 } else { 0.0 };
    let size = egui::vec2(g.size().x + 24.0 + dot, g.size().y + 10.0);
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let radius = rect.height() / 2.0;
        if on {
            p.rect_filled(rect, radius, c.raised());
            p.rect_stroke(rect, radius, egui::Stroke::new(1.0, c.border_strong()), egui::StrokeKind::Inside);
            p.circle_filled(egui::pos2(rect.left() + 14.0, rect.center().y), 3.0, c.run);
        } else if r.hovered() {
            p.rect_filled(rect, radius, c.hover());
        }
        let ink = if r.hovered() && !on { c.fg } else { ink };
        p.galley_with_override_text_color(egui::pos2(rect.left() + 12.0 + dot, rect.center().y - g.size().y / 2.0), g, ink);
    }
    if r.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), rect.height() / 2.0 + 2.0, ui.visuals().selection.stroke, egui::StrokeKind::Outside);
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Everything in the settings folder, as one file under `exports/` in the
/// state folder: its path and how many things it carries.
/// A caution without the part that moves each day, so the band opens for a new caution and not
/// for the same one a day older. The cautions cross the wire as the engine's own text, in its language, so the key
/// is made from the ID: the "table ends soon" caution is its ID and the date it ends (whatever the days left and
/// the language); the "out of date" caution has another ID, and any other line is its own key.
fn caution_key(line: &str) -> String {
    let soon = koyomi_core::msg::TABLE_ENDING_SOON;
    if soon.is_in(line) {
        let until = line
            .as_bytes()
            .windows(10)
            .position(|w| w.iter().enumerate().all(|(i, b)| if i == 4 || i == 7 { *b == b'-' } else { b.is_ascii_digit() }));
        if let Some(at) = until {
            return format!("{} {}", soon.code(), &line[at..at + 10]);
        }
    }
    // An engine from before the IDs: the Japanese text, with 「（あと N 日）」 cut out.
    match (line.find("（あと"), line.find('）')) {
        (Some(a), Some(b)) if a < b => format!("{}{}", &line[..a], &line[b + '）'.len_utf8()..]),
        _ => line.to_owned(),
    }
}

/// Whether an engine's refusal says the file was changed elsewhere.
fn is_changed_elsewhere(m: &str) -> bool {
    koyomi_core::msg::SAVE_CHANGED_ELSEWHERE.is_in(m) || m.starts_with("ほかで変わっています")
    // an engine from before the IDs
}

/// Everything the engine of `link` has, written to `exports/<YYYYMMDD-HHMMSS>.toml` in this PC's state folder.
fn export_all(link: &Link) -> Result<(PathBuf, usize), Said> {
    let text = link.start().and_then(|c| c.call(&Request::ExportAll)).map_err(plain)?;
    let b = koyomi_core::bundle::parse(&text).map_err(plain)?;
    if b.is_empty() {
        return Err(("port.export_empty", Vec::new()));
    }
    let dir = engine::Paths::find().map_err(plain)?.state.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| no_dir(&dir, e))?;
    let path = dir.join(format!("{}.toml", ito_common::now::now_local().format("%Y%m%d-%H%M%S")));
    ito_common::write_atomic(&path, text).map_err(plain)?;
    Ok((path, b.len()))
}

/// `table` as `exports/<what>-<YYYYMMDD-HHMMSS>.csv` in the state folder: its path.
fn export_csv(what: &str, table: &ito_table::Table) -> Result<PathBuf, Said> {
    let dir = engine::Paths::find().map_err(plain)?.state.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| no_dir(&dir, e))?;
    let path = dir.join(format!("{what}-{}.csv", ito_common::now::now_local().format("%Y%m%d-%H%M%S")));
    crate::sheet::write(&path.display().to_string(), table).map_err(plain)?;
    Ok(path)
}

/// The month's sheet written as a PDF under `exports` and handed to the OS to print; where it went.
fn print_plan(days: &[crate::plan::Day], year: i32, m: u32) -> Result<(Said, PathBuf), Said> {
    let dir = engine::Paths::find().map_err(plain)?.state.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| no_dir(&dir, e))?;
    let path = dir.join(format!("plan-{year}-{m:02}.pdf"));
    crate::plan::pdf(days, year, m, &path).map_err(plain)?;
    let shown = path.display().to_string();
    match ito_export::print(&path).map_err(plain)? {
        ito_export::Printed::Sent => Ok((("calendar.printed", vec![shown]), path)),
        _ => Ok((("calendar.print_opened", vec![shown]), path)),
    }
}

/// A failure from below, whose words are already made: told as they are.
fn plain(e: String) -> Said {
    ("port.plain", vec![e])
}

/// A folder that could not be made, as a coded line.
fn no_dir(dir: &std::path::Path, e: std::io::Error) -> Said {
    plain(koyomi_core::msg::line(koyomi_core::msg::DIR_CREATE_FAILED, &[&dir.display().to_string(), &e.to_string()]))
}

/// A section's heading (15px bold, D12), told to a reader as a heading. It wraps rather than run past
/// a narrow pane.
fn section_head(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let r = ui.add(egui::Label::new(egui::RichText::new(text).strong().size(15.0)).wrap());
    ito_a11y::say(&r, Role::Heading, text);
    r
}

/// A card in the inspector: the panel's colour with a line round it (a job, the next times).
fn card(c: &ito_theme::Colors) -> egui::Frame {
    egui::Frame::new().fill(c.panel).stroke(egui::Stroke::new(1.0, c.border)).corner_radius(6).inner_margin(egui::Margin::same(8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::approved::Store;
    use koyomi_core::scan::read_def;

    const NET: &str = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";

    /// On another PC's engine the conflict box has no 上書きする button, so its body must not mention overwriting.
    #[test]
    fn the_far_conflict_body_does_not_offer_an_overwrite() {
        for code in ["en", "ja"] {
            let far = conflict_body_in(code, true);
            assert!(koyomi_core::msg::CONFLICT_FAR.is_in(&far), "{code}: {far}");
            assert!(!far.contains("上書き") && !far.to_lowercase().contains("overwrit"), "{code}: {far}");
            // The near body still does (the button exists there).
            let near = conflict_body_in(code, false);
            assert!(koyomi_core::msg::CONFLICT_NEAR.is_in(&near), "{code}: {near}");
            assert!(near.contains("上書き") || near.to_lowercase().contains("overwrit"), "{code}: {near}");
        }
    }

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

    /// The worker's queue as the tests read it: the requests, one at a time, as the window sent them.
    struct Asked(Receiver<Work>);

    impl Asked {
        fn try_recv(&self) -> Result<Request, mpsc::TryRecvError> {
            self.0.try_recv().map(|w| {
                let Job::Ask(r) = w.job else { panic!("a calendar job where a request was expected") };
                r
            })
        }

        /// The next envelope as it is, for a test that looks at a calendar job.
        fn try_work(&self) -> Result<Work, mpsc::TryRecvError> {
            self.0.try_recv()
        }
    }

    fn asks() -> (Sender<Work>, Asked) {
        let (tx, rx) = mpsc::channel();
        (tx, Asked(rx))
    }

    fn quiet(defs: Vec<Def>) -> Browser {
        let (tx_msg, rx) = mpsc::channel();
        let (tx, _) = mpsc::channel::<Work>();
        let snap = Some(Snapshot { defs, ..Snapshot::default() });
        Browser {
            far: far::Far::idle(&egui::Context::default()),
            config_edit: None,
            hosts: hosts::Hosts::new(),
            rx,
            io: (tx_msg, egui::Context::default()),
            out: Outbox { tx, pending: Arc::default(), switching: Cell::new(false), refused: Cell::new(false) },
            want_link: None,
            link: Link::Local,
            role: None,
            who: None,
            users: users::Users::default(),
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
            result_path: None,
            over: None,
            ended: Vec::new(),
            to_ack: Vec::new(),
            acked: BTreeSet::new(),
            done: None,
            done_path: None,
            seen_notes: BTreeSet::new(),
            notes: Vec::new(),
            runs_known: false,
            moved_urls: Vec::new(),
            autostart_off: false,
            band_seen: None,
            cal_sel: None,
            folded: BTreeSet::new(),
            tree_seen: None,
            cal_month: None,
            cal_day: None,
            cal_result: None,
            cal_inline: None,
            cal_picking: None,
            hist: None,
            gantt: false,
            today_kind: None,
            today_net: None,
            inspector: true,
            hist_pane: true,
            hist_waiting: false,
            cal_pane: true,
            cal_view: calendar::CalView::Month,
            net_pane: true,
            today_sort: None,
            net_sort: None,
            run_sort: None,
            nets_sort: None,
            nets_pick: None,
            back: Vec::new(),
            reveal: false,
            under_card: false,
            under_palette: false,
            vers: None,
            editor: None,
            editors: Vec::new(),
            port: None,
            make: None,
            new_cal: None,
            cal_deleting: None,
            give: None,
            skip_pick: false,
            staged_box: None,
            sending: None,
            wire: Wire::default(),
            easy: None,
            ha: None,
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
            self.press(words, egui::PointerButton::Primary, b);
        }

        /// A right click on the widget that says `words`.
        fn right_click(&mut self, words: &str, b: &mut Browser) {
            self.press(words, egui::PointerButton::Secondary, b);
        }

        /// The middle of the one widget whose words are `words`, or else contain them.
        fn at(&self, words: &str) -> egui::Pos2 {
            self.rect(words).center()
        }

        /// Where the one widget whose words are `words`, or else contain them, is.
        fn rect(&self, words: &str) -> egui::Rect {
            let exact: Vec<_> = self.nodes.values().filter(|(label, _)| label == words).collect();
            let found: Vec<_> =
                if exact.is_empty() { self.nodes.values().filter(|(label, _)| label.contains(words)).collect() } else { exact };
            assert_eq!(found.len(), 1, "one widget says {words:?}: {:?}", self.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
            found[0].1
        }

        fn press(&mut self, words: &str, button: egui::PointerButton, b: &mut Browser) {
            self.presses(words, button, 1, b);
        }

        /// Two quick clicks on the widget that says `words`.
        fn double_click(&mut self, words: &str, b: &mut Browser) {
            self.presses(words, egui::PointerButton::Primary, 2, b);
        }

        fn presses(&mut self, words: &str, button: egui::PointerButton, times: usize, b: &mut Browser) {
            let pos = self.at(words);
            self.presses_at(pos, button, times, b);
        }

        /// A click at `pos`, for a widget whose parts say nothing of their own.
        fn click_at(&mut self, pos: egui::Pos2, b: &mut Browser) {
            self.presses_at(pos, egui::PointerButton::Primary, 1, b);
        }

        fn presses_at(&mut self, pos: egui::Pos2, button: egui::PointerButton, times: usize, b: &mut Browser) {
            let button = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE };
            self.show(vec![egui::Event::PointerMoved(pos)], b);
            for _ in 0..times {
                self.show(vec![button(true)], b);
                self.show(vec![button(false)], b);
            }
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
        s.click("名前の変更…", &mut b);
        assert_eq!(b.new_cal.as_ref().map(|m| m.name.as_str()), Some("会社"));
        let shown = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(shown(&s, "jobnets/試験/bom.toml, jobnets/試験/_group.toml"));
        assert!(shown(&s, "会社 のファイル名を変更します"), "a rename says the content stays");
        s.show(key(egui::Key::Enter), &mut b);
        assert_eq!(
            b.new_cal.as_ref().and_then(|m| m.error.as_deref()),
            Some(koyomi_core::msg::line(koyomi_core::msg::CAL_SAME_NAME, &["会社"]).as_str()),
            "the name it has is no rename"
        );
        s.show(key(egui::Key::Escape), &mut b);
        assert!(b.new_cal.is_none());
        s.click("削除…", &mut b);
        assert_eq!(b.cal_deleting.as_deref(), Some("会社"));
        assert!(shown(&s, "versions/_calendars/会社/"));
        s.show(key(egui::Key::Enter), &mut b);
        assert!(b.cal_deleting.is_none() && b.cal_result.is_none(), "Enter on the fresh box deletes nothing");
    }

    /// 24.19: The calendar's view is 日 | 週 | 月 in that order: 日 shows the picked day, whose ◀ ▶
    /// move a day; a file from before 日 keeps its week or month.
    #[test]
    fn the_calendar_shows_a_day_a_week_or_a_month() {
        let mut b = quiet(defs());
        let cal = koyomi_core::calendar::Calendar::parse(
            "会社",
            "format = 1
",
        )
        .unwrap();
        b.snap.as_mut().unwrap().calendars.insert("会社".into(), cal);
        let mut s = Screen::new();
        s.settle(&mut b);
        b.cal_sel = Some("会社".into());
        b.cal_month = Some((2026, 11));
        b.cal_day = chrono::NaiveDate::from_ymd_opt(2026, 11, 4);
        s.settle(&mut b);
        let seg = s.rect("表示: 月");
        s.click_at(egui::pos2(seg.min.x + 6.0, seg.center().y), &mut b);
        assert_eq!(b.cal_view, calendar::CalView::Day, "日 is the first of the three");
        assert!(s.nodes.values().any(|(l, _)| l == "表示: 日"));
        assert!(s.nodes.values().any(|(l, _)| l.starts_with("11/4(")), "the head says the day");
        s.click("前の日", &mut b);
        assert_eq!(b.cal_day, chrono::NaiveDate::from_ymd_opt(2026, 11, 3));
        let seg = s.rect("表示: 日");
        s.click_at(egui::pos2(seg.center().x, seg.center().y), &mut b);
        assert_eq!(b.cal_view, calendar::CalView::Week, "週 is in the middle");
        let seg = s.rect("表示: 週");
        s.click_at(egui::pos2(seg.max.x - 6.0, seg.center().y), &mut b);
        assert_eq!(b.cal_view, calendar::CalView::Month, "月 is the last");
        use calendar::CalView;
        assert_eq!(CalView::kept(None, Some(true)), CalView::Week, "a file from before 日 kept its week");
        assert_eq!(CalView::kept(None, None), CalView::Month);
        for v in [CalView::Day, CalView::Week, CalView::Month] {
            assert_eq!(CalView::kept(Some(v.word()), None), v);
        }
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
        s.click("決定する", &mut b);
        assert_eq!(b.cal_result, Some(Err(koyomi_core::msg::line(koyomi_core::msg::TABLE_PATH_EMPTY, &[]))));
        assert!(b.cal_inline.is_none());
        b.cal_inline = Some(calendar::Inline::Name { cal: "工場".into(), day, text: String::new(), fresh: true });
        s.settle(&mut b);
        assert!(b.cal_inline.is_none(), "another calendar's field goes");
    }

    /// The holiday table's file chooser: while it is open the button is off, a chosen file fills the field
    /// (which stays editable), and a cancel leaves the field as it was.
    #[test]
    fn the_holiday_table_s_file_chooser_fills_the_field_or_leaves_it() {
        let mut b = quiet(defs());
        let cal = koyomi_core::calendar::Calendar::parse("会社", "format = 1\n");
        b.snap.as_mut().unwrap().calendars.insert("会社".into(), cal.unwrap());
        let mut s = Screen::new();
        b.cal_sel = Some("会社".into());
        b.cal_month = Some((2026, 11));
        let (tx, rx) = std::sync::mpsc::channel();
        b.cal_inline = Some(calendar::Inline::Table { cal: "会社".into(), path: "C:/typed.csv".into(), fresh: false });
        b.cal_picking = Some(("会社".into(), rx));
        s.settle(&mut b);
        let path = |b: &Browser| match &b.cal_inline {
            Some(calendar::Inline::Table { path, .. }) => path.clone(),
            _ => panic!("the field is open"),
        };
        let open = |b: &Browser| b.cal_picking.is_some();
        assert!(open(&b) && path(&b) == "C:/typed.csv", "waiting: nothing changed, no second dialog");
        tx.send(None).unwrap();
        s.settle(&mut b);
        assert!(!open(&b) && path(&b) == "C:/typed.csv", "a cancel leaves the field");
        // Pick again: the file fills the field.
        let (tx, rx) = std::sync::mpsc::channel();
        b.cal_picking = Some(("会社".into(), rx));
        tx.send(Some(std::path::PathBuf::from("C:/picked/syukujitsu.csv"))).unwrap();
        s.settle(&mut b);
        assert!(!open(&b) && path(&b) == "C:/picked/syukujitsu.csv", "{}", path(&b));
    }

    /// The OS chooser outlives the field: closing the field keeps the chooser's record (so the button stays off
    /// for any field), and the answer fills only the same calendar's table field if it is open again.
    #[test]
    fn an_open_file_chooser_outlives_its_field() {
        let mut b = quiet(defs());
        let cal = koyomi_core::calendar::Calendar::parse(
            "会社",
            "format = 1
",
        );
        b.snap.as_mut().unwrap().calendars.insert("会社".into(), cal.unwrap());
        let mut s = Screen::new();
        b.cal_sel = Some("会社".into());
        b.cal_month = Some((2026, 11));
        let table = |path: &str| calendar::Inline::Table { cal: "会社".into(), path: path.into(), fresh: false };
        let field = |b: &Browser| match &b.cal_inline {
            Some(calendar::Inline::Table { path, .. }) => Some(path.clone()),
            _ => None,
        };
        // (c) While the chooser is open and the field is open, the button is off.
        let (tx, rx) = std::sync::mpsc::channel();
        b.cal_inline = Some(table("C:/typed.csv"));
        b.cal_picking = Some(("会社".into(), rx));
        s.settle(&mut b);
        assert!(b.cal_picking.is_some() && field(&b).is_some());
        // (a) The field is closed (Esc, やめる, another calendar) before the answer: the chooser is still known,
        // and the answer reopens nothing.
        b.cal_inline = None;
        s.settle(&mut b);
        assert!(b.cal_picking.is_some(), "the chooser is still open");
        tx.send(Some(std::path::PathBuf::from("C:/picked/a.csv"))).unwrap();
        s.settle(&mut b);
        assert!(b.cal_picking.is_none() && b.cal_inline.is_none(), "answered, and no field is open");
        // (b) The same calendar's field is reopened before the answer: the path fills it.
        let (tx, rx) = std::sync::mpsc::channel();
        b.cal_picking = Some(("会社".into(), rx));
        b.cal_inline = None;
        s.settle(&mut b);
        b.cal_inline = Some(table(""));
        tx.send(Some(std::path::PathBuf::from("C:/picked/b.csv"))).unwrap();
        s.settle(&mut b);
        assert!(b.cal_picking.is_none());
        assert_eq!(field(&b).as_deref(), Some("C:/picked/b.csv"));
    }

    /// A file the window wrote comes with its path, as `Here`; the path belongs to its line only.
    #[test]
    fn a_result_carries_the_path_of_the_file_written_and_loses_it_with_the_line() {
        use paths::{Made, Place};
        let mut b = quiet(defs());
        let mut s = Screen::new();
        let copies = |s: &Screen| s.nodes.values().filter(|(l, _)| l.contains("パスをコピー")).count();
        s.settle(&mut b);
        let none = copies(&s);
        // The three things the window writes: the export, a CSV, the printed PDF.
        for (key, args, file) in [
            ("port.exported", vec!["C:/s/exports/a.toml".to_owned(), "3".to_owned()], "C:/s/exports/a.toml"),
            ("run.history_csv_done", vec!["C:/s/exports/history-1.csv".to_owned()], "C:/s/exports/history-1.csv"),
            ("calendar.print_opened", vec!["C:/s/exports/plan-2026-11.pdf".to_owned()], "C:/s/exports/plan-2026-11.pdf"),
        ] {
            b.io.0.send(Msg::DoneAt(Ok(((key, args), PathBuf::from(file))))).unwrap();
            s.settle(&mut b);
            let text = match &b.result {
                Some(Ok(m)) => m.clone(),
                other => panic!("{other:?}"),
            };
            assert_eq!(copies(&s), none + 1, "the line has its copy button");
            let made = Made::for_result(&b.result_path, &b.result).expect("the line carries its path");
            assert_eq!((made.path.as_path(), made.place, &made.text), (std::path::Path::new(file), Place::Here, &text));
        }
        // A file written on this PC (`Here`) has its buttons whichever engine the window talks to.
        assert!(paths::buttons_shown(Place::Here, false));
        // The next result without a path shows no old path.
        b.io.0.send(Msg::Done(Ok(("run.history_csv_done", vec!["C:/other.csv".to_owned()])))).unwrap();
        s.settle(&mut b);
        assert!(b.result_path.is_none() && Made::for_result(&b.result_path, &b.result).is_none());
        // A failure carries none either.
        b.io.0.send(Msg::DoneAt(Err(plain("x".to_owned())))).unwrap();
        s.settle(&mut b);
        assert!(b.result_path.is_none());
    }

    /// The band has a header for each kind of line: the writable-by warning keeps its own, a file that did not read
    /// has another.
    #[test]
    fn the_band_has_a_header_for_each_kind_of_line() {
        use koyomi_core::msg;
        let writable = msg::line(msg::FILE_WRITABLE_BY, &["koyomi.toml"]);
        let unread = msg::line(msg::CFG_TOKEN_NOT_SECRET, &["x", "y"]);
        let (warning, problem) = ("他のユーザーが書き換えられる場所があります", "koyomi が読むファイルに問題があります");
        for (lines, want_warning, want_problem) in [
            (vec![unread.clone()], false, true),
            (vec![writable.clone()], true, false),
            (vec![unread.clone(), writable.clone()], true, true),
        ] {
            let mut b = quiet(Vec::new());
            b.snap.as_mut().unwrap().warnings = lines.clone();
            let mut s = Screen::new();
            s.settle(&mut b);
            let shown = |w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
            assert_eq!(shown(warning), want_warning, "{lines:?}");
            assert_eq!(shown(problem), want_problem, "{lines:?}");
            for l in &lines {
                assert!(shown(l), "{l}");
            }
        }
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
        assert_eq!(
            b.new_cal.as_ref().and_then(|m| m.error.as_deref()),
            Some(koyomi_core::msg::line(koyomi_core::msg::CAL_NAME_EMPTY, &[]).as_str())
        );
        b.snap
            .as_mut()
            .unwrap()
            .calendars
            .insert("会社".into(), koyomi_core::calendar::Calendar { name: "会社".into(), ..Default::default() });
        b.new_cal.as_mut().unwrap().name = "会社".into();
        s.show(key(egui::Key::Enter), &mut b);
        assert_eq!(
            b.new_cal.as_ref().and_then(|m| m.error.as_deref()),
            Some(koyomi_core::msg::line(koyomi_core::msg::CALENDAR_EXISTS, &["会社"]).as_str())
        );
        s.show(key(egui::Key::Escape), &mut b);
        assert!(b.new_cal.is_none(), "Esc closes it");
    }

    /// The band's key ignores the days left, in either language and in an older engine's Japanese, and changes with the
    /// date the table ends, with the "out of date" caution and with the calendar.
    #[test]
    fn the_caution_key_ignores_the_days_left_in_both_languages() {
        use koyomi_core::msg::{say_in, CAL_HOLIDAYS_VALUE, TABLE_ENDING_SOON, TABLE_STALE};
        for code in ["ja", "en"] {
            let soon = |until: &str, left: &str| caution_key(&say_in(code, TABLE_ENDING_SOON, &[until, left]).line());
            assert_eq!(soon("2027-12-31", "60"), soon("2027-12-31", "59"), "{code}: a day older is the same caution");
            assert_ne!(soon("2027-12-31", "60"), soon("2028-12-31", "60"), "{code}: another end is another caution");
            let stale = caution_key(&say_in(code, TABLE_STALE, &["2027-12-31"]).line());
            assert_ne!(soon("2027-12-31", "1"), stale, "{code}: out of date is another caution");
            let bad = |name: &str| caution_key(&say_in(code, CAL_HOLIDAYS_VALUE, &[name]).line());
            assert_ne!(bad("会社"), bad("工場"), "{code}: another calendar is another caution");
        }
        // An older engine's Japanese text, with no ID.
        let old = |left: u32| caution_key(&format!("祝日の表は 2027-12-31 までの分です（あと {left} 日）。置けば延びる"));
        assert_eq!(old(60), old(59));
        assert_ne!(old(60), caution_key("祝日の表は 2028-12-31 までの分です（あと 60 日）。置けば延びる"));
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

    /// The pane at the right of 履歴 is there with no run picked, says how to pick one, and folds to
    /// its strip and opens again.
    #[test]
    fn the_history_pane_is_always_there_and_folds() {
        let mut b = quiet(defs());
        b.open_history();
        let mut s = Screen::new();
        s.settle(&mut b);
        let has = |s: &Screen, w: &str| s.nodes.values().any(|n| n.0 == w);
        assert!(has(&s, "選択した実行"));
        assert!(has(&s, "表で行を選ぶと、その実行のジョブ・ログ・操作がここに表示されます。"));
        s.click("右の欄をたたむ", &mut b);
        assert!(!b.hist_pane);
        s.click("右の欄を開く", &mut b);
        assert!(b.hist_pane);
    }

    /// A net's page is split into 図, 実行のタイミング, 設定 and ジョブ, in that order.
    #[test]
    fn the_net_page_has_its_sections_and_its_times_at_the_right() {
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
        let at = |w: &str| {
            let rs: Vec<egui::Rect> = s.nodes.values().filter(|(l, _)| l == w).map(|(_, r)| *r).collect();
            assert!(!rs.is_empty(), "{w} is a heading: {:?}", s.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
            rs.into_iter().max_by(|a, b| a.center().y.total_cmp(&b.center().y)).unwrap()
        };
        let ys = ["図", "設定", "ジョブ"].map(|w| at(w).center().y);
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "in order: {ys:?}");
        // The next starts are in the pane at the right of the page, not under the graph.
        let (graph, times) = (at("図"), at("実行のタイミング"));
        assert!(times.left() > graph.right(), "at the right: {graph:?} {times:?}");
        assert!(s.nodes.values().any(|(l, _)| l == "今日の実行"), "the pane lists today's runs");
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
        let now = ito_common::now::now_local().naive_local();
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
        let now = ito_common::now::now_local().naive_local();
        let mut run = Run::new("20261030-090000-01", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.start("a", now);
        run.finish("a", Outcome::Exited(0), now);
        run.start("b", now);
        run.finish("b", Outcome::Exited(1), now);
        assert!(run.is_over());
        let (tx, rx_req) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.runs = vec![run];
        b.run_sel = Some("20261030-090000-01".into());
        let mut s = Screen::new();
        s.settle(&mut b);
        let menu_says = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(!menu_says(&s, "停止したところから"), "one button until opened");
        assert!(menu_says(&s, "ジョブを選ぶと、そのジョブから再実行します。"), "the hint with no job picked");
        s.click("再実行 ▾", &mut b);
        s.click("停止したところから", &mut b);
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
        s.click("ジョブ a を保留", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Hold { id, job: Some(j) }) if id == "試験/bom" && j == "a"));
        b.snap.as_mut().unwrap().held_jobs.push(koyomi_core::held::HeldJob { net: "試験/bom".into(), job: "a".into() });
        s.settle(&mut b);
        s.click("ジョブ a の保留を解除する", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Release { id, job: Some(j) }) if id == "試験/bom" && j == "a"));
    }

    /// The kill box starts on the safe button: a plain Enter does not kill.
    /// 今すぐ実行 starts a net without `[params]` at once, and asks first for one with them,
    /// sending only the values that were changed.
    #[test]
    fn run_now_asks_for_params_and_sends_only_the_changed_ones() {
        let (tx, rx_req) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
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
        let now = ito_common::now::now_local().naive_local();
        let mut run = Run::new("r1", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.start("a", now);
        let (tx, rx_req) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
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
        let (tx, rx_req) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
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
        let (tx, rx_req) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
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
        let now = ito_common::now::now_local().naive_local();
        let mut run = Run::new("r1", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.start("a", now);
        let (tx, rx_req) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.runs = vec![run];
        let mut s = Screen::new();

        b.deleting = Some("試験/bom".into());
        s.settle(&mut b);
        s.click("キャンセル", &mut b);
        assert!(b.deleting.is_none() && rx_req.try_recv().is_err(), "the safe button deletes nothing");
        b.deleting = Some("試験/bom".into());
        s.settle(&mut b);
        s.click("削除する", &mut b);
        assert!(b.deleting.is_none());
        assert!(matches!(rx_req.try_recv(), Ok(Request::Delete { id }) if id == "試験/bom"));

        b.run_sel = Some("r1".into());
        b.killing = Some("r1".into());
        s.settle(&mut b);
        s.click("キャンセル", &mut b);
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
        s.click("確認した", &mut b);
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
        assert!(matches!(&b.result, Some(Ok(m)) if m == "出力しました: C:/x.csv"), "{:?}", b.result);
        // A failure is worded the same way.
        b.io.0.send(Msg::Done(Err(no_dir(std::path::Path::new("C:/e"), std::io::Error::other("denied"))))).unwrap();
        s.settle(&mut b);
        let want = koyomi_core::msg::line(koyomi_core::msg::DIR_CREATE_FAILED, &["C:/e", "denied"]);
        assert!(matches!(&b.result, Some(Err(m)) if *m == want), "{:?}", b.result);
    }

    /// A right click on a row of 今日 holds or releases its net without leaving the page.
    #[test]
    fn today_s_right_click_holds_and_releases_the_net() {
        use koyomi_core::held::HeldJob;
        use koyomi_core::jobnet::{Job, Jobnet};
        use koyomi_core::run::{How, State};
        let net = Jobnet { jobs: vec![Job { id: "a".into(), run: vec!["x".into()], ..Default::default() }], ..Default::default() };
        let now = ito_common::now::now_local().naive_local();
        let mut run = Run::new("r1", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.state = State::Held;
        run.ended = Some(now);
        let (tx, rx_req) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.runs = vec![run];
        b.gantt = true;
        let snap = b.snap.as_mut().unwrap();
        snap.held.push("試験/bom".into());
        snap.held_jobs.push(HeldJob { net: "試験/bom".into(), job: "a".into() });
        let mut s = Screen::new();
        s.settle(&mut b);
        s.right_click("⏸ 保留 bom", &mut b);
        s.click("ジョブネットの保留を解除する", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Release { id, job: None }) if id == "試験/bom"));
        s.right_click("⏸ 保留 bom", &mut b);
        s.click("ジョブ a の保留を解除する", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Release { id, job: Some(j) }) if id == "試験/bom" && j == "a"));
        // Not held: the same place holds it.
        b.snap.as_mut().unwrap().held.clear();
        s.right_click("⏸ 保留 bom", &mut b);
        s.click("ジョブネットを保留", &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Hold { id, job: None }) if id == "試験/bom"));
        assert!(b.gantt, "the right click does not leave 今日");
    }

    /// A click on a run's notice opens that run: today's in the detail with its net marked, an older
    /// one in 履歴's pane; not over a net open in the editor.
    #[test]
    fn a_notice_opens_its_run() {
        use koyomi_core::jobnet::{Job, Jobnet};
        use koyomi_core::run::How;
        let net = Jobnet { jobs: vec![Job { id: "a".into(), run: vec!["x".into()], ..Default::default() }], ..Default::default() };
        let now = ito_common::now::now_local().naive_local();
        let mut b = quiet(defs());
        b.runs = vec![Run::new("r1", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now)];
        b.gantt = true;
        b.show_run("r1");
        assert_eq!((b.run_sel.as_deref(), b.selected.as_deref(), b.gantt, b.hist.is_some()), (Some("r1"), Some("試験/bom"), false, false));
        b.hist_pane = false;
        b.show_run("r0");
        assert_eq!((b.run_sel.as_deref(), b.hist.is_some(), b.hist_pane), (Some("r0"), true, true));
        b.hist = None;
        b.run_sel = None;
        b.open_editor("試験/bom");
        b.show_run("r1");
        assert_eq!(b.run_sel, None, "the editor's work stays");
    }

    /// A click on a row of 今日 shows its net in the pane at the right; the pane folds and opens again.
    #[test]
    fn a_click_on_today_shows_the_net_at_the_right() {
        use koyomi_core::jobnet::{Job, Jobnet};
        use koyomi_core::run::{How, State};
        let net = Jobnet { jobs: vec![Job { id: "a".into(), run: vec!["x".into()], ..Default::default() }], ..Default::default() };
        let now = ito_common::now::now_local().naive_local();
        let mut run = Run::new("r1", "試験/bom", "bom", "2026-10-30", How::Manual, &net, now);
        run.state = State::Error;
        run.jobs[0].state = State::Error;
        run.ended = Some(now);
        let mut b = quiet(defs());
        b.runs = vec![run];
        b.gantt = true;
        let mut s = Screen::new();
        s.settle(&mut b);
        assert!(s.nodes.values().any(|n| n.0 == "選んだジョブネット"));
        s.click("✕ 異常終了 bom", &mut b);
        assert_eq!(b.today_net.as_deref(), Some("試験/bom"));
        assert_eq!(b.run_sel.as_deref(), Some("r1"));
        assert!(s.nodes.values().any(|n| n.0 == "構成ジョブ定義"), "{:?}", s.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
        assert_eq!(b.watch.lock().unwrap().clone(), Some(("r1".to_owned(), "a".to_owned())));
        s.click("右の欄をたたむ", &mut b);
        assert!(!b.inspector);
        s.click("右の欄を開く", &mut b);
        assert!(b.inspector);
    }

    /// 56.5: A double click on a start still to come opens its net; 戻る, at the right of the tabs,
    /// comes back to 今日 with the same row picked, and then goes away.
    #[test]
    fn a_waiting_row_opens_its_net_and_back_returns_to_it() {
        let now = ito_common::now::now_local().naive_local();
        if now.time() >= chrono::NaiveTime::from_hms_opt(23, 58, 0).unwrap() {
            // The day's last start is past: no row waits.
            return;
        }
        let mut d = defs();
        let when = "format = 1
[[when]]
every = \"day\"
at = [\"23:59\"]
[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        d[0].net = Some(koyomi_core::jobnet::parse(when).unwrap());
        d[0].standing = Standing::Confirmed;
        let mut b = quiet(d);
        b.gantt = true;
        let mut s = Screen::new();
        s.settle(&mut b);
        assert!(!s.nodes.values().any(|n| n.0 == "今日に戻る"), "no link was followed yet");
        let left = s.rect("待ち bom 23:59:00").min.x;
        s.double_click("待ち bom 23:59:00", &mut b);
        assert!(!b.gantt && b.run_sel.is_none(), "the net page is open");
        assert_eq!(b.selected.as_deref(), Some("試験/bom"));
        s.click("今日に戻る", &mut b);
        assert!(b.gantt, "back on 今日");
        assert_eq!(b.today_net.as_deref(), Some("試験/bom"), "with the row picked");
        assert!(b.back.is_empty() && !b.reveal, "nothing more to go back to, and the row was brought into sight");
        // Brought up or down only: the inspector leaves the table wider than its room, and centring
        // the row sideways too would cut off its first columns.
        assert_eq!(s.rect("待ち bom 23:59:00").min.x, left, "the table did not slide sideways");
    }

    /// Each link keeps where it came from, newest last; one that stays where it is keeps nothing, and a
    /// tab forgets them all.
    #[test]
    fn back_goes_where_each_link_came_from() {
        let mut b = quiet(defs());
        b.gantt = true;
        b.run_sel = Some("r1".into());
        b.today_net = Some("試験/bom".into());
        b.follow(|b| b.open_net("試験/bom"));
        b.follow(|b| b.open_net("試験/bom"));
        assert_eq!(b.back.len(), 1, "a link to the page already open keeps nothing");
        b.job_sel = Some("a".into());
        b.follow(|b| b.open_calendar("会社"));
        assert_eq!(b.back.len(), 2);
        b.go_back();
        assert_eq!((b.selected.as_deref(), b.job_sel.as_deref(), b.cal_sel.as_deref()), (Some("試験/bom"), Some("a"), None));
        assert!(b.reveal, "the picked row is brought into sight");
        b.go_back();
        assert!(b.gantt);
        assert_eq!((b.run_sel.as_deref(), b.today_net.as_deref()), (Some("r1"), Some("試験/bom")));
        assert!(b.back.is_empty());
        for _ in 0..BACK_MAX + 5 {
            b.follow(|b| b.open_net("試験/bom"));
            b.follow(|b| b.open_gantt());
        }
        assert_eq!(b.back.len(), BACK_MAX, "the oldest go first");
        let mut s = Screen::new();
        s.settle(&mut b);
        s.click("履歴", &mut b);
        assert!(b.hist.is_some() && b.back.is_empty(), "a tab is not a link");
    }

    #[test]
    fn a_press_on_ack_marks_only_a_failed_run() {
        use koyomi_core::jobnet::{Job, Jobnet};
        use koyomi_core::run::{How, State};
        let net = Jobnet { jobs: vec![Job { id: "a".into(), run: vec!["x".into()], ..Default::default() }], ..Default::default() };
        let now = ito_common::now::now_local().naive_local();
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
        assert!(matches!(&b.result, Some(Ok(m)) if m == "対応済みにしました。"));
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
        let (tx, rx_req) = asks();
        b.out.tx = tx;
        b.edit(canvas::Cmd::Save);
        match rx_req.try_recv() {
            Ok(Request::Save { id: saved, force, .. }) => {
                assert_eq!(saved, id);
                assert!(!force);
            }
            other => panic!("{other:?}"),
        }
    }

    /// TODO 110: the editor stages its net for a day; the tree and the detail tell it, and its box
    /// loads it back into the editor or drops it.
    #[test]
    fn the_editor_stages_a_definition_for_a_day() {
        use canvas::Action;
        let defs = defs();
        let id = defs[0].id.clone();
        let (tx, rx_req) = asks();
        let mut b = quiet(defs);
        b.out.tx = tx;
        b.open_editor(&id);
        let e = b.editor.as_mut().unwrap();
        e.add(koyomi_core::jobnet::Part::Notify, Some("a"), None);
        let text = e.text();
        e.stage = Some("2099-12-01".into());
        assert!(!e.can_save(), "with the day row open it is staged, not saved");
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        b.act(Action::Stage("2099-12-01".into()));
        frame(&ctx, size, Vec::new(), &mut b);
        assert!(b.box_open(), "asked first");
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        match rx_req.try_recv() {
            Ok(Request::Stage { id: i, text: x, from, .. }) => {
                assert_eq!((i.as_str(), x.as_str(), from.as_str()), (id.as_str(), text.as_str(), "2099-12-01"))
            }
            other => panic!("not a stage: {other:?}"),
        }
        let staged = Request::Stage { id: id.clone(), text: text.clone(), from: "2099-12-01".into(), expect: None };
        b.io.0.send(Msg::Answer(staged, Ok("置いた".into()))).unwrap();
        frame(&ctx, size, Vec::new(), &mut b);
        let e = b.editor.as_ref().unwrap();
        assert!(!e.dirty && e.stage.is_none() && e.stage_ask.is_none(), "back to the net as it is");
        assert_eq!(e.result, Some(Ok("置いた".to_owned())));
        // The engine tells it back: the tree and the detail show the day, and the box loads it.
        let from = chrono::NaiveDate::from_ymd_opt(2099, 12, 1).unwrap();
        let sha256 = koyomi_core::approved::hash(text.as_bytes());
        b.snap.as_mut().unwrap().staged.push(koyomi_core::next::Entry { id: id.clone(), from, sha256, base: None, text: text.clone() });
        b.editor = None;
        b.selected = Some(id.clone());
        let mut s = Screen::new();
        s.settle(&mut b);
        let says = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(says(&s, "2099-12-01 から次の定義に切り替わります"), "{:?}", s.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
        s.click("次の定義を見る", &mut b);
        assert_eq!(b.staged_box.as_deref(), Some(id.as_str()));
        s.show(key(egui::Key::Enter), &mut b);
        assert!(b.staged_box.is_none());
        let e = b.editor.as_ref().expect("opened on the net");
        assert!(e.dirty && e.stage.as_deref() == Some("2099-12-01") && e.text() == text, "{}", e.report());
        // From the editor: the button by the title opens the box, and it drops what was staged.
        s.settle(&mut b);
        s.click("2099-12-01 から次の定義", &mut b);
        assert_eq!(b.staged_box.as_deref(), Some(id.as_str()));
        b.focus = 2;
        s.show(key(egui::Key::Enter), &mut b);
        assert!(matches!(rx_req.try_recv(), Ok(Request::Unstage { id: i }) if i == id));
        assert!(b.staged_box.is_none());
    }

    /// 別の PC に送る: the `[[remote]]` PCs in a menu; what the picked one has of the net against
    /// this PC's in a box, and 送る sends it with the hash read. The same text, or a PC that cannot
    /// be read, is a line with no box; Esc sends nothing; with no `[[remote]]` there is no button.
    #[test]
    fn a_net_is_sent_to_another_pc() {
        use crate::remote::There;
        use std::sync::Mutex;
        type Sent = (String, String, String, Option<String>);
        static SENT: Mutex<Vec<Sent>> = Mutex::new(Vec::new());
        fn there(pc: &str, id: &str) -> Result<Option<There>, String> {
            let confirmed = Some(Standing::Confirmed);
            match pc {
                "本番" => Ok(Some(There { text: NET.replace("x.exe", "old.exe"), sha256: "abc".into(), standing: confirmed })),
                "同じ" => Ok(Some(There { text: NET.into(), sha256: "s".into(), standing: confirmed })),
                "新しい" => Ok(None),
                _ => Err(format!("{pc}: つながらない（{id}）")),
            }
        }
        fn propose(pc: &str, id: &str, text: &str, base: Option<&str>) -> Result<bool, String> {
            SENT.lock().unwrap().push((pc.into(), id.into(), text.into(), base.map(str::to_owned)));
            Ok(false)
        }
        fn wait(s: &mut Screen, b: &mut Browser, done: impl Fn(&Browser) -> bool) {
            for _ in 0..500 {
                s.show(Vec::new(), b);
                if done(b) {
                    s.settle(b);
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("no answer in time");
        }
        let sent = || SENT.lock().unwrap().clone();
        let defs = defs();
        let id = defs[0].id.clone();
        let mut b = quiet(defs);
        b.selected = Some(id.clone());
        let mut s = Screen::new();
        s.settle(&mut b);
        let says = |s: &Screen, w: &str| s.nodes.values().any(|(l, _)| l.contains(w));
        assert!(says(&s, "今すぐ実行") && !says(&s, "別の PC に送る"), "no [[remote]], no button");
        b.far.set_names(vec!["本番".into(), "同じ".into(), "新しい".into(), "遠い".into()]);
        b.wire = Wire { there, propose };
        s.settle(&mut b);
        s.click("別の PC に送る ▾", &mut b);
        s.click("本番 に送る…", &mut b);
        wait(&mut s, &mut b, |b| b.sending.as_ref().is_some_and(|x| x.there.is_some()));
        assert!(says(&s, &format!("{id} を 本番 に送る")), "{:?}", s.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
        assert!(says(&s, "接続先の現在の定義") && says(&s, "確認済み"));
        s.show(key(egui::Key::Escape), &mut b);
        assert!(b.sending.is_none() && sent().is_empty(), "Esc sends nothing");
        s.click("別の PC に送る ▾", &mut b);
        s.click("本番 に送る…", &mut b);
        wait(&mut s, &mut b, |b| b.sending.as_ref().is_some_and(|x| x.there.is_some()));
        s.show(key(egui::Key::Enter), &mut b);
        assert!(b.sending.is_none());
        wait(&mut s, &mut b, |b| b.result.is_some());
        assert_eq!(
            b.result,
            Some(Ok(format!("本番 に送信しました: {id}（接続先では未確認です。本番 の画面で確認するまで実行されません）")))
        );
        assert_eq!(sent(), [("本番".to_owned(), id.clone(), NET.to_owned(), Some("abc".to_owned()))]);
        // A PC without the net: the box says so, and the send has no hash.
        s.click("別の PC に送る ▾", &mut b);
        s.click("新しい に送る…", &mut b);
        wait(&mut s, &mut b, |b| b.sending.as_ref().is_some_and(|x| x.there.is_some()));
        assert!(says(&s, "なし（新規に配置する）"));
        s.show(key(egui::Key::Enter), &mut b);
        wait(&mut s, &mut b, |b| b.result.is_some());
        assert_eq!(sent().last().map(|x| (x.0.as_str(), x.3.clone())), Some(("新しい", None)));
        s.click("別の PC に送る ▾", &mut b);
        s.click("同じ に送る…", &mut b);
        wait(&mut s, &mut b, |b| b.sending.is_none() && b.result.is_some());
        assert_eq!(b.result, Some(Ok(format!("同じ の {id} はこの PC と同じ内容です（送信していません）"))));
        s.click("別の PC に送る ▾", &mut b);
        s.click("遠い に送る…", &mut b);
        wait(&mut s, &mut b, |b| b.sending.is_none() && b.result.is_some());
        assert_eq!(b.result, Some(Err(format!("遠い: つながらない（{id}）"))));
        assert_eq!(sent().len(), 2, "nothing sent without the box");
    }

    #[test]
    fn a_wide_table_narrows_its_words_and_keeps_its_times() {
        // State, net, group, last, time, next, note: 130 over the room.
        let mut w = vec![100.0, 300.0, 100.0, 120.0, 80.0, 120.0, 130.0];
        TodayTable::squeeze(&mut w, 820.0, &[1, 2, 6], 72.0);
        assert!((w.iter().sum::<f32>() - 820.0).abs() < 0.01, "{w:?}");
        assert_eq!([w[0], w[3], w[4], w[5]], [100.0, 120.0, 80.0, 120.0]);
        // Each gives by what it has over the floor: the long net the most.
        assert!(w[1] < 300.0 && w[2] < 100.0 && w[6] < 130.0 && 300.0 - w[1] > 130.0 - w[6], "{w:?}");

        // Never below the floor; what is still over scrolls sideways.
        let mut w = vec![100.0, 80.0, 72.0, 120.0];
        TodayTable::squeeze(&mut w, 100.0, &[1, 2], 72.0);
        assert_eq!(w, [100.0, 72.0, 72.0, 120.0]);

        // A table that fits is left as it is.
        let mut w = vec![100.0, 200.0];
        TodayTable::squeeze(&mut w, 400.0, &[1], 72.0);
        assert_eq!(w, [100.0, 200.0]);
    }

    fn save_of(id: &str) -> Request {
        Request::Save { id: id.into(), text: "x".into(), base_sha256: None, force: false }
    }

    #[test]
    fn a_send_counts_until_the_work_is_dropped_and_a_read_never_counts() {
        let (tx, rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.send(Request::List);
        assert_eq!(b.pending(), 0, "a read is not counted");
        b.send(save_of("g/a"));
        assert_eq!(b.pending(), 1);
        // The thread takes the envelope, answers, and drops it: exactly once.
        let w = rx.0.try_recv().unwrap();
        let w2 = rx.0.try_recv().unwrap();
        assert!(w.pending.is_none() && w2.pending.is_some());
        drop(w);
        assert_eq!(b.pending(), 1);
        drop(w2);
        assert_eq!(b.pending(), 0);
    }

    #[test]
    fn a_failed_send_and_a_gone_thread_give_the_count_back() {
        let (tx, rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        // Sent, never taken, and the thread is gone: the envelopes drop with the queue.
        b.send(save_of("g/a"));
        b.send(save_of("g/b"));
        assert_eq!(b.pending(), 2);
        drop(rx);
        assert_eq!(b.pending(), 0);
        // Sending into a closed queue takes the count off again.
        assert!(!b.send(save_of("g/c")), "a closed queue says it did not send");
        assert_eq!(b.pending(), 0);
    }

    #[test]
    fn nothing_new_is_sent_while_switching_and_cancel_sends_again() {
        let (tx, rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.send(save_of("g/a"));
        assert_eq!(b.switch_step(), 1, "one is still in the thread's hands");
        b.send(save_of("g/b"));
        assert!(b.out.refused.get(), "a write is refused and says so");
        assert_eq!(b.pending(), 1, "the refused write is not counted");
        b.send(Request::List);
        assert_eq!(b.pending(), 1, "a read is not counted");
        assert!(matches!(rx.try_recv(), Ok(Request::Save { id, .. }) if id == "g/a"));
        assert!(matches!(rx.try_recv(), Ok(Request::List)), "a read still reaches the thread");
        assert!(rx.try_recv().is_err(), "the refused write never left");
        b.cancel_switch();
        b.send(save_of("g/c"));
        assert!(matches!(rx.try_recv(), Ok(Request::Save { id, .. }) if id == "g/c"));
    }

    #[test]
    fn a_start_failure_answers_what_was_sent_and_the_count_returns_to_zero() {
        let mut b = Browser::start(&egui::Context::default(), None, Link::Far("no-such-pc-for-this-test".into()), None);
        b.send(save_of("g/a"));
        let until = std::time::Instant::now() + Duration::from_secs(10);
        let mut answered = false;
        while std::time::Instant::now() < until && !(answered && b.down.is_some() && b.pending() == 0) {
            while let Ok(m) = b.rx.try_recv() {
                match m {
                    Msg::Answer(Request::Save { .. }, Err(_)) => answered = true,
                    Msg::Down(e) => b.down = Some(e),
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(answered, "the save got an Err");
        assert!(b.down.is_some());
        assert_eq!(b.pending(), 0);
    }

    fn body(text: &str, sha: &str) -> Result<String, String> {
        Ok(proto::CalendarBody { text: text.into(), sha256: sha.into() }.to_toml())
    }

    /// Runs `calendar_job` against a scripted engine: `text` answers `CalendarText`, `write` answers anything
    /// else. Gives back every request sent and the job's answer.
    fn run_calendar(
        text: Result<String, String>,
        change: CalendarChange,
        write: Result<String, String>,
    ) -> (Vec<Request>, Result<String, String>) {
        let sent = std::cell::RefCell::new(Vec::new());
        let r = calendar_job("会社", change, &|req: &Request| {
            sent.borrow_mut().push(req.clone());
            match req {
                Request::CalendarText { .. } => text.clone(),
                _ => write.clone(),
            }
        });
        (sent.into_inner(), r)
    }

    #[test]
    fn a_calendar_job_that_cannot_read_the_calendar_writes_nothing_and_says_so() {
        let edit = || CalendarChange::Edit(Box::new(|t| Ok(format!("{t}x"))));
        for text in [Err("会社: つながらない".to_owned()), Ok("not = [toml".to_owned())] {
            let (sent, r) = run_calendar(text, edit(), Ok("書いた".into()));
            assert!(r.as_ref().unwrap_err().ends_with("カレンダーは変更していません。"), "{r:?}");
            assert_eq!(sent.len(), 1, "only the read: {sent:?}");
        }
        let (sent, r) = run_calendar(Err("暦がない: 会社".into()), CalendarChange::Delete, Ok("消した".into()));
        assert_eq!(r, Err("暦がない: 会社。カレンダーは変更していません。".to_owned()));
        assert_eq!(sent.len(), 1);
        // A change that fails is nothing written either.
        let (sent, r) = run_calendar(body("a", "h1"), CalendarChange::Edit(Box::new(|_| Err("直せない".into()))), Ok("書いた".into()));
        assert_eq!(r, Err("直せない。カレンダーは変更していません。".to_owned()));
        assert_eq!(sent.len(), 1);
    }

    #[test]
    fn a_calendar_put_carries_the_hash_of_what_was_fetched_and_the_change_sees_only_that_text() {
        let seen = Arc::new(Mutex::new(String::new()));
        let seen_in = seen.clone();
        let change = CalendarChange::Edit(Box::new(move |t| {
            *seen_in.lock().unwrap() = t.to_owned();
            Ok(format!("{t}+"))
        }));
        let (sent, r) = run_calendar(body("fetched", "h-now"), change, Ok("暦を書いた: calendars/会社.toml".into()));
        assert_eq!(r, Ok("暦を書いた: calendars/会社.toml".to_owned()), "the engine's text as it is");
        assert_eq!(*seen.lock().unwrap(), "fetched");
        assert_eq!(sent.len(), 2);
        assert!(matches!(&sent[0], Request::CalendarText { name } if name == "会社"));
        assert!(matches!(
            &sent[1],
            Request::CalendarPut { name, text, base_sha256: Some(h) } if name == "会社" && text == "fetched+" && h == "h-now"
        ));
    }

    #[test]
    fn a_refusal_comes_back_verbatim_and_is_not_retried() {
        let no = "ほかで変わっています: 会社";
        let (sent, r) = run_calendar(body("a", "h1"), CalendarChange::Edit(Box::new(|t| Ok(t.to_owned()))), Err(no.into()));
        assert_eq!(r, Err(no.to_owned()));
        assert_eq!(sent.len(), 2, "one read and one write: {sent:?}");
        let (sent, r) = run_calendar(body("a", "h2"), CalendarChange::Delete, Err(no.into()));
        assert_eq!(r, Err(no.to_owned()));
        assert_eq!(sent.len(), 2);
    }

    #[test]
    fn a_calendar_delete_carries_the_fetched_hash() {
        let (sent, r) = run_calendar(body("a", "h9"), CalendarChange::Delete, Ok("暦を消した: 会社".into()));
        assert_eq!(r, Ok("暦を消した: 会社".to_owned()));
        assert!(matches!(&sent[1], Request::CalendarDelete { name, base_sha256 } if name == "会社" && base_sha256 == "h9"));
    }

    #[test]
    fn a_calendar_job_counts_until_dropped_is_refused_while_switching_and_gives_the_count_back() {
        let (tx, rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        assert!(b.out.send_calendar("会社".into(), CalendarChange::Delete));
        assert_eq!(b.pending(), 1);
        let w = rx.try_work().unwrap();
        assert!(matches!(&w.job, Job::Calendar { name, .. } if name == "会社"));
        assert!(w.pending.is_some());
        drop(w);
        assert_eq!(b.pending(), 0, "back to 0 once the envelope is dropped");
        // Switching: refused, nothing sent, nothing counted.
        assert!(b.out.send_calendar("会社".into(), CalendarChange::Delete));
        assert_eq!(b.switch_step(), 1);
        assert!(!b.out.send_calendar("会社".into(), CalendarChange::Delete));
        assert!(b.out.refused.get());
        assert_eq!(b.pending(), 1, "the refused job is not counted");
        assert!(rx.try_work().is_ok(), "the first one is there");
        assert!(rx.try_work().is_err(), "the refused one never left");
        // A send into a closed queue takes the count off again.
        b.cancel_switch();
        let before = b.pending();
        let (tx, rx) = asks();
        b.out.tx = tx;
        drop(rx);
        assert!(!b.out.send_calendar("会社".into(), CalendarChange::Delete));
        assert_eq!(b.pending(), before);
    }

    /// Runs `answer` on a calendar delete with a real count; returns what was sent, the count afterwards and the return value.
    fn answered(reply: &dyn Fn(&Request) -> Result<String, String>, listening: bool) -> (Vec<Msg>, usize, bool) {
        let (tx, rx) = asks();
        let out = Outbox { tx, pending: Arc::new(AtomicUsize::new(0)), switching: Cell::new(false), refused: Cell::new(false) };
        assert!(out.send_calendar("会社".into(), CalendarChange::Delete));
        assert_eq!(out.pending.load(Ordering::SeqCst), 1);
        let work = rx.try_work().unwrap();
        let sent = std::cell::RefCell::new(Vec::new());
        let seen_count = std::cell::Cell::new(usize::MAX);
        let send = |m: Msg| {
            // The envelope is still alive while the message is sent.
            seen_count.set(out.pending.load(Ordering::SeqCst));
            sent.borrow_mut().push(m);
            listening
        };
        let alive = answer(work, reply, &send);
        assert_eq!(seen_count.get(), 1, "the count goes after the message");
        (sent.into_inner(), out.pending.load(Ordering::SeqCst), alive)
    }

    #[test]
    fn a_calendar_job_pressed_while_the_engine_cannot_start_comes_back_unchanged_and_counts_zero() {
        let (sent, count, alive) = answered(&|_| Err("起動できない".into()), true);
        assert!(alive);
        assert_eq!(count, 0);
        let [Msg::CalendarDone(Err(e))] = &sent[..] else { panic!("one failed answer expected") };
        assert!(e.ends_with("カレンダーは変更していません。"), "{e}");
    }

    #[test]
    fn a_calendar_job_that_goes_through_answers_with_the_engines_words_and_counts_zero() {
        let (sent, count, alive) = answered(
            &|r| match r {
                Request::CalendarText { .. } => body("a", "h1"),
                _ => Ok("暦を消した: 会社".into()),
            },
            true,
        );
        assert!(alive);
        assert_eq!(count, 0);
        assert!(matches!(&sent[..], [Msg::CalendarDone(Ok(s))] if s == "暦を消した: 会社"), "{} messages", sent.len());
    }

    #[test]
    fn answer_says_false_and_still_counts_zero_when_the_window_is_gone() {
        let (sent, count, alive) = answered(&|_| Err("起動できない".into()), false);
        assert!(!alive);
        assert_eq!(count, 0);
        assert_eq!(sent.len(), 1);
    }

    fn put_of(name: &str) -> Request {
        Request::CalendarPut { name: name.into(), text: String::new(), base_sha256: None }
    }

    #[test]
    fn the_new_calendar_box_waits_for_the_answer_shows_an_error_and_a_yes_closes_it() {
        let (tx, rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.new_cal_box();
        b.new_cal.as_mut().unwrap().name = "工場".into();
        b.new_cal_go();
        assert!(b.new_cal.as_ref().unwrap().waiting, "waits for the answer");
        assert!(matches!(rx.try_recv(), Ok(Request::CalendarPut { name, base_sha256: None, .. }) if name == "工場"));
        // A second press while waiting sends nothing.
        b.new_cal_go();
        assert!(rx.try_recv().is_err());
        // The engine says no: the box stays open with the words, ready for another go.
        b.io.0.send(Msg::Answer(put_of("工場"), Err("もうある".into()))).unwrap();
        b.drain();
        let m = b.new_cal.as_ref().expect("still open");
        assert!(!m.waiting);
        assert_eq!(m.error.as_deref(), Some("もうある"));
        // A yes closes it and shows the calendar with the engine's own words.
        b.new_cal_go();
        assert!(b.new_cal.as_ref().unwrap().waiting);
        b.io.0.send(Msg::Answer(put_of("工場"), Ok("暦を書いた: calendars/工場.toml".into()))).unwrap();
        b.drain();
        assert!(b.new_cal.is_none());
        assert_eq!(b.cal_sel.as_deref(), Some("工場"));
        assert_eq!(b.cal_result, Some(Ok("暦を書いた: calendars/工場.toml".to_owned())));
    }

    #[test]
    fn a_yes_for_a_box_opened_from_the_editor_sets_the_editors_calendar() {
        let (tx, _rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        let net = koyomi_core::jobnet::parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n").unwrap();
        b.editor = Some(canvas::Editor::new_net("試験/a", net));
        b.new_cal_box_for_net();
        b.new_cal.as_mut().unwrap().name = "工場".into();
        b.new_cal_go();
        b.io.0.send(Msg::Answer(put_of("工場"), Ok("暦を書いた: calendars/工場.toml".into()))).unwrap();
        b.drain();
        assert!(b.new_cal.is_none());
        let e = b.editor.as_ref().unwrap();
        assert_eq!(e.result, Some(Ok("暦を書いた: calendars/工場.toml".to_owned())));
        assert!(b.cal_sel.is_none(), "the editor stays, the calendar page is not opened");
    }

    #[test]
    fn a_refused_send_leaves_the_new_calendar_box_not_waiting() {
        let (tx, _rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.new_cal_box();
        b.new_cal.as_mut().unwrap().name = "工場".into();
        b.out.switching.set(true);
        b.new_cal_go();
        let m = b.new_cal.as_ref().expect("stays open");
        assert!(!m.waiting);
        assert_eq!(m.error.as_deref(), Some(koyomi_core::msg::line(koyomi_core::msg::LINK_SWITCHING, &[]).as_str()));
    }

    #[test]
    fn a_waiting_new_calendar_box_ignores_cancel_and_esc() {
        let (tx, _rx) = asks();
        let mut b = quiet(Vec::new());
        b.out.tx = tx;
        let mut s = Screen::new();
        s.settle(&mut b);
        s.click("＋ 新しいカレンダー", &mut b);
        b.new_cal.as_mut().unwrap().name = "工場".into();
        b.new_cal_go();
        assert!(b.new_cal.as_ref().unwrap().waiting);
        s.settle(&mut b);
        s.click("キャンセル", &mut b);
        assert!(b.new_cal.as_ref().is_some_and(|m| m.waiting), "cancel does nothing while waiting");
        s.show(key(egui::Key::Escape), &mut b);
        s.show(key(egui::Key::Enter), &mut b);
        assert!(b.new_cal.as_ref().is_some_and(|m| m.waiting), "Esc and Enter do nothing while waiting");
    }

    #[test]
    fn the_calendar_selected_is_the_one_sent_not_the_name_typed_after() {
        let (tx, _rx) = asks();
        let mut b = quiet(defs());
        b.out.tx = tx;
        b.new_cal_box();
        b.new_cal.as_mut().unwrap().name = "工場".into();
        b.new_cal_go();
        b.new_cal.as_mut().unwrap().name = "別".into();
        b.io.0.send(Msg::Answer(put_of("工場"), Ok("ok".into()))).unwrap();
        b.drain();
        assert!(b.new_cal.is_none());
        assert_eq!(b.cal_sel.as_deref(), Some("工場"));
    }

    #[test]
    fn a_calendar_answer_with_no_waiting_box_goes_to_the_calendar_result() {
        let mut b = quiet(defs());
        b.io.0.send(Msg::Answer(put_of("工場"), Ok("ok".into()))).unwrap();
        b.drain();
        assert_eq!(b.cal_result, Some(Ok("ok".to_owned())));
        assert!(b.cal_sel.is_none());
        // A box that is open but not waiting is not closed by an older answer.
        b.new_cal_box();
        b.io.0.send(Msg::Answer(Request::CalendarRename { from: "a".into(), to: "b".into() }, Err("no".into()))).unwrap();
        b.drain();
        assert!(b.new_cal.as_ref().is_some_and(|m| m.error.is_none()));
        assert_eq!(b.cal_result, Some(Err("no".to_owned())));
    }

    #[test]
    fn edit_proposes_and_admin_and_this_pc_save() {
        use koyomi_core::role::Role;
        let mk = |r| save_request(r, "g/a".into(), "x".into(), None, false);
        assert!(matches!(mk(Some(Role::Edit)), Request::Propose { .. }));
        assert!(matches!(mk(Some(Role::Admin)), Request::Save { force: false, .. }));
        assert!(matches!(mk(None), Request::Save { .. }));
    }

    #[test]
    fn a_proposal_that_was_taken_updates_the_editor_and_a_far_conflict_has_no_force() {
        let defs = defs();
        let id = defs[0].id.clone();
        let (tx, rx) = asks();
        let mut b = quiet(defs);
        b.out.tx = tx;
        b.link = Link::Far("kei".into());
        b.role = Some(koyomi_core::role::Role::Edit);
        b.open_editor(&id);
        let e = b.editor.as_mut().unwrap();
        e.add(koyomi_core::jobnet::Part::Notify, Some("a"), None);
        let text = e.text();
        assert!(e.dirty);
        let req = Request::Propose { id: id.clone(), text: text.clone(), base_sha256: None };
        b.io.0.send(Msg::Answer(req, Ok("置いた".into()))).unwrap();
        b.drain();
        let e = b.editor.as_ref().unwrap();
        assert!(!e.dirty);
        assert_eq!(e.base, Some(koyomi_core::approved::hash(text.as_bytes())));
        assert_eq!(e.result, Some(Ok("置いた".to_owned())), "the engine's own text, verbatim");
        // A conflict over there: the box offers two buttons, never a forced overwrite.
        let req = Request::Propose { id: id.clone(), text: text.clone(), base_sha256: None };
        b.io.0.send(Msg::Answer(req, Err("ほかで変わっています: x".into()))).unwrap();
        b.drain();
        assert!(b.editor.as_ref().unwrap().conflict.is_some());
        let ctx = egui::Context::default();
        frame(&ctx, egui::vec2(1280.0, 800.0), Vec::new(), &mut b);
        assert!(rx.try_recv().is_err(), "no force save went out");
    }

    #[test]
    fn the_third_button_forces_a_save_on_this_pc_and_sends_nothing_on_another() {
        assert!(force_save(true, 2, "g/a", "x".into(), None).is_none());
        assert!(matches!(force_save(false, 2, "g/a", "x".into(), Some("h".into())), Some(Request::Save { force: true, .. })));
        for far in [true, false] {
            assert!(force_save(far, 0, "g/a", "x".into(), None).is_none());
            assert!(force_save(far, 1, "g/a", "x".into(), None).is_none());
        }
    }

    #[test]
    fn the_stage_request_expects_what_the_window_saw() {
        let r = stage_request("g/a", Some("live".into()), Some("seal".into()), "t".into(), "2099-12-01".into()).unwrap();
        let Request::Stage { expect: Some(x), .. } = r else { panic!("{r:?}") };
        assert_eq!((x.live.as_str(), x.staged.as_deref()), ("live", Some("seal")));
        let r = stage_request("g/a", Some("live".into()), None, "t".into(), "2099-12-01".into()).unwrap();
        assert!(matches!(r, Request::Stage { expect: Some(x), .. } if x.staged.is_none()));
        assert!(stage_request("g/a", None, None, "t".into(), "2099-12-01".into()).is_err());
    }

    #[test]
    fn the_names_of_the_remotes_arrive_through_drain() {
        let mut b = quiet(defs());
        assert!(!b.far.any());
        b.far.send_names(vec!["kei".into(), "eigyo".into()]);
        b.drain();
        assert_eq!(b.far.names(), ["kei", "eigyo"]);
    }

    #[test]
    fn a_far_engine_that_went_away_is_said_even_with_a_snapshot() {
        let t = ito_i18n::Table::default();
        let mut b = quiet(defs());
        b.down = Some("gone".into());
        let (word, running, next) = b.engine_facts(&t);
        assert_eq!((word.as_deref(), running, next), (Some(t.tr("status.engine_down")), 0, None));
        b.io.0.send(Msg::Snapshot(Box::<Snapshot>::default())).unwrap();
        b.drain();
        assert!(b.down.is_none() && b.engine_facts(&t).0.is_none());
    }

    #[test]
    fn a_far_list_failure_skips_the_rest_and_this_pc_does_not() {
        assert_eq!(after_list_failure(&Link::Far("kei".into())), Some(Duration::from_secs(10)));
        assert_eq!(after_list_failure(&Link::Local), None);
    }

    #[test]
    fn the_stage_request_carries_what_it_was_made_from() {
        let defs = defs();
        let id = defs[0].id.clone();
        let (tx, rx) = asks();
        let mut b = quiet(defs);
        b.out.tx = tx;
        b.open_editor(&id);
        let e = b.editor.as_mut().unwrap();
        e.add(koyomi_core::jobnet::Part::Notify, Some("a"), None);
        let live = e.base.clone().expect("a net that exists has a base");
        e.stage = Some("2099-12-01".into());
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        b.act(canvas::Action::Stage("2099-12-01".into()));
        frame(&ctx, size, Vec::new(), &mut b);
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        match rx.try_recv() {
            Ok(Request::Stage { expect: Some(x), .. }) => assert_eq!((x.live, x.staged), (live, None)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "GitHub's Windows runners (x64 and ARM64) lose the last save now and then: a write that fails is dropped, not retried (ito's kept::Writer)"
    )]
    fn a_state_without_a_link_is_this_pc_and_one_writer_keeps_the_last_link() {
        let dir = std::env::temp_dir().join(format!("koyomi-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("state.toml"), "inspector = true\n").unwrap();
        assert_eq!(kept_link(Some(&dir)), Link::Local);
        assert_eq!(kept_link(None), Link::Local);
        // Two browsers without a worker (nothing here may start an engine or reach a real [[remote]]).
        let writer = kept::Writer::new(dir.join("state.toml"), true);
        let mut old = quiet(defs());
        old.flow = Some(writer.clone());
        old.link = Link::Local;
        let mut new = quiet(defs());
        new.flow = Some(writer);
        new.link = Link::Far("no-such-pc-for-this-test".into());
        let far = Link::Far("no-such-pc-for-this-test".into());
        let settle = |want: &Link| {
            let until = std::time::Instant::now() + Duration::from_secs(5);
            while std::time::Instant::now() < until && kept_link(Some(&dir)) != *want {
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(kept_link(Some(&dir)), *want);
        };
        for _ in 0..20 {
            old.save_state();
            new.save_state();
        }
        settle(&far);
        for _ in 0..20 {
            new.save_state();
            old.save_state();
        }
        settle(&Link::Local);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One pass as `main.rs` runs it: the command palette first, then the browser, which is told whether the palette
    /// is still open. Returns what the palette answered.
    fn palette_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        b: &mut Browser,
        view: &mut Option<ito_palette::View>,
    ) -> Option<ito_palette::Answer<usize>> {
        let t = crate::i18n::table("ja").0;
        let size = egui::vec2(1280.0, 800.0);
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        let mut answer = None;
        let mut out = ctx.run_ui(input, |ui| {
            if let Some(mut v) = view.take() {
                let c = egui::Color32::GRAY;
                let style =
                    ito_palette::Style { panel: c, border: c, text: c, dim: c, selected_bg: c, selected_text: c, hit: c, accent: c };
                let words = ito_palette::Words { hint: "", none: "", header: "", footer: "" };
                let entries = [
                    ito_palette::Entry::new("alpha", "", 0usize),
                    ito_palette::Entry::new("beta", "", 1usize),
                    ito_palette::Entry::new("gamma", "", 2usize),
                ];
                answer = ito_palette::show(ui.ctx(), &mut v, &entries, &style, &words);
                if answer.is_none() {
                    *view = Some(v);
                }
            }
            b.under_palette = view.is_some();
            b.show(ui, &t);
        });
        out.textures_delta.clear();
        answer
    }

    /// The palette opened over a box takes the typing: a paste and typed letters land in its field, Enter picks the
    /// line the text matches (not the first), and Esc closes the palette only.
    fn palette_over(b: &mut Browser, still_open: impl Fn(&Browser) -> bool) {
        let ctx = egui::Context::default();
        let mut view = Some(ito_palette::View::new());
        for _ in 0..4 {
            assert!(palette_frame(&ctx, Vec::new(), b, &mut view).is_none());
        }
        palette_frame(&ctx, vec![egui::Event::Paste("ga".into())], b, &mut view);
        palette_frame(&ctx, vec![egui::Event::Text("mm".into())], b, &mut view);
        assert_eq!(view.as_ref().map(|v| v.query.as_str()), Some("gamm"));
        assert!(still_open(b), "typing did not close the box");
        let answer = palette_frame(&ctx, key(egui::Key::Enter), b, &mut view);
        assert_eq!(answer, Some(ito_palette::Answer::Pick(2)), "the line the text matches, not the first");
        assert!(still_open(b), "the box is still open after Enter");
        // Esc closes the palette only.
        let mut view = Some(ito_palette::View::new());
        for _ in 0..3 {
            palette_frame(&ctx, Vec::new(), b, &mut view);
        }
        let answer = palette_frame(&ctx, key(egui::Key::Escape), b, &mut view);
        assert_eq!(answer, Some(ito_palette::Answer::Close));
        assert!(still_open(b), "Esc did not close the box");
    }

    #[test]
    fn the_palette_takes_the_typing_over_the_add_box() {
        let mut b = quiet(defs());
        b.add_remote_box();
        palette_over(&mut b, |b| {
            let a = b.far.add.as_ref();
            a.is_some_and(|a| a.name.is_empty() && a.url.is_empty())
        });
    }

    /// The same at the token step (the field that asked for the focus there).
    #[test]
    fn the_palette_takes_the_typing_over_the_add_boxs_token_step() {
        let mut b = quiet(defs());
        b.add_remote_box();
        let a = b.far.add.as_mut().unwrap();
        (a.name, a.url, a.step) = ("kei".into(), "https://pc:8443".into(), far::AddStep::Token);
        palette_over(&mut b, |b| b.far.add.as_ref().is_some_and(|a| a.token.is_empty() && a.step == far::AddStep::Token));
    }

    /// The same over one of the browser's own questions, which takes egui's focus away while it is up.
    #[test]
    fn the_palette_takes_the_typing_over_a_browser_question() {
        let mut b = asking();
        palette_over(&mut b, |b| b.asking.is_some());
    }

    #[test]
    fn the_add_box_leaves_the_focus_to_the_palette() {
        let ctx = egui::Context::default();
        let mut b = quiet(defs());
        b.add_remote_box();
        b.under_palette = true;
        for _ in 0..3 {
            frame(&ctx, egui::vec2(1280.0, 800.0), Vec::new(), &mut b);
        }
        assert!(ctx.memory(|m| m.focused()).is_none(), "the box does not ask for the focus under the palette");
        b.under_palette = false;
        for _ in 0..3 {
            frame(&ctx, egui::vec2(1280.0, 800.0), Vec::new(), &mut b);
        }
        assert!(ctx.memory(|m| m.focused()).is_some(), "the box asks for it again");
    }

    #[test]
    fn the_connection_boxes_count_as_boxes_for_the_browsers_own() {
        for manage in [false, true] {
            let mut b = quiet(defs());
            assert!(!b.box_open());
            if manage {
                b.manage_remotes_box();
            } else {
                b.add_remote_box();
            }
            assert!(b.box_open());
            b.new_box(None);
            assert!(b.make.is_none(), "no new net box over a connection box (manage: {manage})");
            b.import_box();
            assert!(b.port.is_none(), "no import box over a connection box (manage: {manage})");
        }
    }

    #[test]
    fn no_connection_box_opens_over_the_browsers_own() {
        let mut b = quiet(defs());
        b.new_box(None);
        assert!(b.make.is_some());
        b.add_remote_box();
        assert!(!b.far.boxed());
        b.manage_remotes_box();
        assert!(!b.far.boxed());
    }

    #[test]
    fn the_two_connection_boxes_still_replace_each_other() {
        let mut b = quiet(defs());
        b.add_remote_box();
        b.manage_remotes_box();
        assert!(b.far.manage.is_some() && b.far.add.is_none());
        b.add_remote_box();
        assert!(b.far.add.is_some() && b.far.manage.is_none());
    }

    /// A browser on `kei` whose manage box is open with an outcome line.
    fn kei_with_box(busy: bool) -> Browser {
        let mut b = quiet(defs());
        b.link = Link::Far("kei".into());
        b.manage_remotes_box();
        let m = b.far.manage.as_mut().unwrap();
        m.line = Some(Ok("moved".to_owned()));
        m.busy = busy;
        b
    }

    #[test]
    fn the_manage_box_is_carried_into_the_reconnect_on_the_same_link() {
        let mut old = kei_with_box(false);
        let mut fresh = quiet(defs());
        fresh.link = Link::Far("kei".into());
        fresh.keep_from(&mut old);
        let line = fresh.far.manage.as_ref().and_then(|m| m.line.clone());
        assert_eq!(line, Some(Ok("moved".to_owned())));
        assert!(old.far.manage.is_none());
    }

    #[test]
    fn the_manage_box_is_not_carried_to_another_link() {
        let mut old = kei_with_box(false);
        let mut fresh = quiet(defs());
        fresh.link = Link::Local;
        fresh.keep_from(&mut old);
        assert!(fresh.far.manage.is_none());
    }

    #[test]
    fn the_moved_urls_not_taken_yet_go_with_any_switch() {
        let mut old = kei_with_box(false);
        old.moved_urls.push("kei".into());
        let mut fresh = quiet(defs());
        fresh.link = Link::Local;
        fresh.keep_from(&mut old);
        assert_eq!(fresh.take_moved_urls(), ["kei"]);
        assert!(old.moved_urls.is_empty());
    }

    #[test]
    fn a_busy_manage_box_is_not_carried() {
        let mut old = kei_with_box(true);
        let mut fresh = quiet(defs());
        fresh.link = Link::Far("kei".into());
        fresh.keep_from(&mut old);
        assert!(fresh.far.manage.is_none());
    }

    const FAKE_TOKEN: &str = "fake-token-0123456789";

    fn add_request() -> Request {
        Request::UserAdd { name: "bob".into(), role: "viewer".into(), nets: None, roles: None, base: "b".into() }
    }

    #[test]
    fn a_second_role_message_replaces_the_name() {
        let mut b = quiet(defs());
        for who in ["alice", "carol"] {
            b.io.0.send(Msg::Role(Some(koyomi_core::role::Role::Admin), Some(who.to_owned()))).unwrap();
            b.drain();
            assert_eq!(b.who.as_deref(), Some(who));
        }
    }

    #[test]
    fn a_new_name_drops_the_listing_and_asks_again_at_an_open_page() {
        let mut b = quiet(defs());
        let (tx, asked) = asks();
        b.out.tx = tx;
        let admin = Some(koyomi_core::role::Role::Admin);
        b.io.0.send(Msg::Role(admin, Some("alice".into()))).unwrap();
        b.drain();
        b.users.touch(b.io.1.cumulative_frame_nr());
        b.users.ask();
        b.users.listing_answer(Ok("base = \"b\"
"
        .to_owned()));
        assert!(b.users.listing.is_some());
        b.io.0.send(Msg::Role(admin, Some("carol".into()))).unwrap();
        b.drain();
        assert!(b.users.listing.is_none());
        assert!(matches!(asked.try_recv(), Ok(Request::Users)));
    }

    #[test]
    fn a_far_role_not_yet_known_sends_nothing() {
        let mut b = quiet(defs());
        let (tx, asked) = asks();
        b.out.tx = tx;
        b.link = Link::Far("pc".into());
        b.users.touch(b.io.1.cumulative_frame_nr());
        b.users.need = true;
        b.users_reask();
        assert!(asked.try_recv().is_err());
    }

    #[test]
    fn a_new_token_stays_off_the_result_line_and_calls_a_switch_off() {
        let mut b = quiet(defs());
        let started = b.tokens_opened();
        assert!(!b.switch_called_off(started));
        let reply = format!(
            "name = \"bob\"
token = \"{FAKE_TOKEN}\"
note = \"n\"
"
        );
        b.io.0.send(Msg::Answer(add_request(), Ok(reply))).unwrap();
        b.drain();
        assert!(b.holds_token() && b.has_unsaved());
        assert!(b.switch_called_off(started));
        let shown = format!("{:?}", b.result);
        assert!(!shown.contains(FAKE_TOKEN));
        // A box already open when the person agrees does not call the switch off.
        assert!(!b.switch_called_off(b.tokens_opened()));
    }

    #[test]
    fn an_unreadable_reply_holds_no_token() {
        let mut b = quiet(defs());
        b.io.0.send(Msg::Answer(add_request(), Ok(format!("not toml {FAKE_TOKEN} =")))).unwrap();
        b.drain();
        assert!(!b.holds_token());
        assert!(!format!("{:?}", b.result).contains(FAKE_TOKEN));
    }

    #[test]
    fn now_at_moves_the_utc_clock_by_the_offset_both_ways_over_midnight() {
        use chrono::TimeZone;
        let utc = |d: u32, h: u32, m: u32| chrono::Utc.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap();
        let at = |d: u32, h: u32, m: u32| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap().and_hms_opt(h, m, 0).unwrap();
        // +9: 15:30 UTC is already tomorrow; -5: it is still the same day.
        assert_eq!(now_at(utc(30, 15, 30), 540), at(31, 0, 30));
        assert_eq!(now_at(utc(30, 15, 30), -300), at(30, 10, 30));
        // The other way: just after midnight UTC is still yesterday at -5, and 08:59 at +9.
        assert_eq!(now_at(utc(31, 0, 0), -300), at(30, 19, 0));
        assert_eq!(now_at(utc(30, 23, 59), 540), at(31, 8, 59));
        assert_eq!(now_at(utc(30, 12, 0), 0), at(30, 12, 0));
        // A half-hour zone.
        assert_eq!(now_at(utc(30, 18, 40), 330), at(31, 0, 10));
    }

    #[test]
    fn the_engine_clock_is_this_pcs_unless_the_link_is_far_and_a_snapshot_came() {
        let near = |a: chrono::NaiveDateTime, b: chrono::NaiveDateTime| (a - b).num_seconds().abs() < 30;
        let mut b = quiet(defs());
        // This PC: the snapshot's offset is not used.
        b.snap = Some(Snapshot { utc_offset_min: 600, ..Snapshot::default() });
        assert!(near(b.engine_now(), ito_common::now::now_local().naive_local()));
        // Far, with a snapshot: this PC's UTC clock by the engine's offset.
        b.link = Link::Far("pc".into());
        assert!(near(b.engine_now(), now_at(ito_common::now::now(), 600)));
        b.snap = Some(Snapshot { utc_offset_min: -300, ..Snapshot::default() });
        assert!(near(b.engine_now(), now_at(ito_common::now::now(), -300)));
        assert_eq!(b.engine_today(), b.engine_now().date());
        // Far, no snapshot yet: this PC's clock, and the date is not known.
        b.snap = None;
        assert!(near(b.engine_now(), ito_common::now::now_local().naive_local()));
        assert!(!b.engine_date_known());
    }

    #[test]
    fn history_opened_on_a_far_link_before_the_snapshot_asks_the_engines_week_when_it_comes() {
        let mut b = quiet(defs());
        let (tx, asked) = asks();
        b.out.tx = tx;
        b.link = Link::Far("pc".into());
        b.snap = None;
        b.open_history();
        assert!(asked.try_recv().is_err(), "nothing is sent before the engine's date is known");
        assert!(b.hist_waiting && b.hist.is_some());
        // An engine a whole day ahead of this PC.
        let offset = local_utc_offset_min() + 24 * 60;
        let engine_day = now_at(ito_common::now::now(), offset).date();
        b.io.0.send(Msg::Snapshot(Box::new(Snapshot { utc_offset_min: offset, ..Snapshot::default() }))).unwrap();
        b.drain();
        assert!(!b.hist_waiting);
        let to = engine_day.to_string();
        let from = (engine_day - chrono::Duration::days(7)).to_string();
        let mut got = Vec::new();
        while let Ok(r) = asked.try_recv() {
            got.push(r);
        }
        assert!(
            got.iter().any(|r| matches!(r, Request::History { from: f, to: t } if *f == from && *t == to)),
            "the week asked is the engine's: {got:?}"
        );
        let h = b.hist.as_ref().unwrap();
        assert_eq!((h.from.as_str(), h.to.as_str()), (from.as_str(), to.as_str()));
        // A later snapshot changes nothing already asked.
        b.io.0.send(Msg::Snapshot(Box::new(Snapshot { utc_offset_min: offset - 24 * 60, ..Snapshot::default() }))).unwrap();
        b.drain();
        let h = b.hist.as_ref().unwrap();
        assert_eq!((h.from.as_str(), h.to.as_str()), (from.as_str(), to.as_str()));
    }

    #[test]
    fn history_on_this_pc_asks_at_once_and_ask_past_waits_for_a_far_snapshot() {
        let mut b = quiet(defs());
        let (tx, asked) = asks();
        b.out.tx = tx;
        b.open_history();
        assert!(matches!(asked.try_recv(), Ok(Request::History { .. })) && !b.hist_waiting);
        b.link = Link::Far("pc".into());
        b.snap = None;
        b.past_asked = None;
        b.ask_past();
        assert!(asked.try_recv().is_err(), "the 30 days are the engine's: not asked before its date is known");
    }
}
