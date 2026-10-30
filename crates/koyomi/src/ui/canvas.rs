//! The editor: the parts of a net as stations on a canvas, the lines between
//! them, a shelf of parts to drag in, a pane of fields for the part that was
//! double-clicked (or for the net itself), and undo over all of it.
//!
//! The canvas owns a copy of the net and every change goes through a method
//! here, so the keys, the mouse and the palette all do the same thing and the
//! tests drive the methods. What is saved is `write::text` of that copy; the
//! engine checks it again before it writes the file.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use ito_a11y::{Role, State as Mark};
use ito_i18n::Table;
use koyomi_core::calendar::Calendar;
use koyomi_core::chore::Chore;
use koyomi_core::jobnet::{self, Encoding, Job, Jobnet, Judge, Line, Missed, On, Overlap, Part, Retry, Wait};
use koyomi_core::scan::Def;
#[cfg(test)]
use koyomi_core::scan::Standing;
use koyomi_core::{approved, duration, layout, when, write};

use super::whens::WhenEdit;
use crate::show;

const RADIUS: f32 = 9.0;
const MARGIN: f32 = 24.0;
/// How far from the centre the out port sits.
const PORT: f32 = RADIUS + 7.0;
/// How close to a line a click has to be.
const NEAR: f32 = 6.0;

/// What the palette (and the toolbar) can ask of the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Save,
    Align,
    Fit,
    Flow,
    Undo,
    Redo,
    Close,
}

/// What the editor asks of the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Send the text to the engine.
    Save { force: bool },
    /// Flip the flow between horizontal and vertical.
    Flow,
    /// Close the editor (nothing unsaved, or the person let it go).
    Close,
    /// Open the child net `use` in its own editor, this one kept underneath.
    Enter(String),
    /// Open the 新しいカレンダー box; the calendar made becomes this net's.
    NewCalendar,
}

/// What the mouse is doing between a press and a release.
#[derive(Clone, Debug, PartialEq)]
enum Drag {
    None,
    /// The selected parts move with the pointer.
    Parts {
        moved: bool,
    },
    /// A line from `from`'s out port follows the pointer.
    Connect {
        from: String,
    },
    /// A part from the shelf follows the pointer.
    Shelf(Part),
    /// A range from `from` selects what it covers.
    Range {
        from: Pos2,
    },
    Pan,
}

/// The menu after a right click.
#[derive(Clone, Debug, PartialEq)]
enum Menu {
    Line { job: String, line: usize, at: Pos2 },
    Part { job: String, at: Pos2 },
}

/// The fields of one part as text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub id: String,
    pub name: String,
    /// The sticky note, as typed.
    pub note: String,
    /// One argument per line.
    pub run: String,
    pub dir: String,
    pub timeout: String,
    pub retry_count: String,
    pub retry_wait: String,
    pub shell: String,
    pub script: String,
    pub warn: String,
    pub error: String,
    pub error_output: String,
    pub ok_file: String,
    pub encoding: Encoding,
    pub wait: Wait,
    pub at: String,
    pub days: String,
    pub file: String,
    pub settle: String,
    /// A loop's `count`; its `file` is [`Fields::file`].
    pub count: String,
    pub use_: String,
    /// A chore's `path`, `match`, `keep_days`, `from` and `into`.
    pub path: String,
    pub pattern: String,
    pub keep_days: String,
    pub from: String,
    pub into: String,
    pub key: String,
    /// Why the fields do not read, when they do not.
    pub msg: Option<String>,
}

impl Fields {
    pub fn of(job: &Job) -> Fields {
        let mut f = Fields {
            id: job.id.clone(),
            name: job.name.clone().unwrap_or_default(),
            note: job.note.clone().unwrap_or_default(),
            run: job.run.join("\n"),
            dir: job.dir.clone().unwrap_or_default(),
            ..Default::default()
        };
        f.timeout = job.timeout.map(duration::text).unwrap_or_default();
        if let Some(r) = job.retry {
            f.retry_count = r.count.to_string();
            f.retry_wait = duration::text(r.wait);
        }
        f.shell = job.shell.clone().unwrap_or_default();
        f.script = job.script.clone().unwrap_or_default();
        f.encoding = job.encoding;
        if let Some(j) = &job.judge {
            f.warn = Judge::list(&j.warn);
            f.error = Judge::list(&j.error);
            f.error_output = j.error_output.clone().unwrap_or_default();
            f.ok_file = j.ok_file.clone().unwrap_or_default();
        }
        match &job.part {
            Part::Join { wait } => f.wait = *wait,
            Part::Time { at, days } => {
                f.at = at.format("%H:%M").to_string();
                f.days = days.to_string();
            }
            Part::File { file, settle } => {
                f.file = file.clone();
                f.settle = settle.map(duration::text).unwrap_or_default();
            }
            Part::Loop { file, count } => {
                f.file = file.clone().unwrap_or_default();
                f.count = count.map(|n| n.to_string()).unwrap_or_default();
            }
            Part::Jobnet { use_ } => f.use_ = use_.clone(),
            Part::Chore(c) => match c {
                Chore::Delete { path, pattern, keep_days } => {
                    f.path = path.clone();
                    f.pattern = pattern.clone().unwrap_or_default();
                    f.keep_days = keep_days.to_string();
                }
                Chore::Empty { path } => f.path = path.clone(),
                Chore::Zip { from, into } => {
                    f.from = from.clone();
                    f.into = into.clone();
                }
                Chore::Copy { from, into, pattern } => {
                    f.from = from.clone();
                    f.into = into.clone();
                    f.pattern = pattern.clone().unwrap_or_default();
                }
                Chore::Send { from, into, key } | Chore::Fetch { from, into, key } => {
                    f.from = from.clone();
                    f.into = into.clone();
                    f.key = key.clone().unwrap_or_default();
                }
            },
            _ => {}
        }
        f
    }

    /// `job` with these fields, or why they do not read. Lines and the
    /// position are the job's; the kind of part stays.
    pub fn apply(&self, job: &Job) -> Result<Job, String> {
        let mut out = job.clone();
        let id = self.id.trim();
        if id.is_empty() || id.contains(['/', '\\', '"', ' ']) {
            return Err("id は空でなく、/ や空白を含まない".into());
        }
        out.id = id.into();
        out.name = opt(&self.name);
        out.note = opt(&self.note);
        let opt_secs = |s: &str| -> Result<Option<u64>, String> {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                duration::parse(s).map(Some)
            }
        };
        let retry = || -> Result<Option<Retry>, String> {
            if self.retry_count.trim().is_empty() {
                return Ok(None);
            }
            let count: u32 =
                self.retry_count.trim().parse().map_err(|_| format!("retry の回数は数で書く: \"{}\"", self.retry_count.trim()))?;
            Ok(Some(Retry { count, wait: opt_secs(&self.retry_wait)?.unwrap_or(0) }))
        };
        match &job.part {
            Part::Job | Part::Loop { .. } => {
                if matches!(job.part, Part::Loop { .. }) {
                    let file = opt(&self.file);
                    let count = match self.count.trim() {
                        "" => None,
                        n => Some(
                            n.parse::<u32>()
                                .ok()
                                .filter(|c| (1..=10_000).contains(c))
                                .ok_or_else(|| format!("count は 1 から 10000 までの数で書く: \"{n}\""))?,
                        ),
                    };
                    if file.is_some() == count.is_some() {
                        return Err("繰り返しには file か count のどちらか 1 つを書く".into());
                    }
                    out.part = Part::Loop { file, count };
                }
                out.run = self.run.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
                out.dir = opt(&self.dir);
                out.timeout = opt_secs(&self.timeout)?;
                out.retry = retry()?;
                out.shell = opt(&self.shell);
                out.script = if self.script.trim().is_empty() { None } else { Some(self.script.clone()) };
                out.encoding = self.encoding;
                let ranges = |s: &str| -> Result<Vec<_>, String> {
                    if s.trim().is_empty() {
                        Ok(Vec::new())
                    } else {
                        jobnet::ranges(s)
                    }
                };
                let (warn, error) = (ranges(&self.warn)?, ranges(&self.error)?);
                let error_output = opt(&self.error_output);
                if let Some(p) = &error_output {
                    ito_match::Matcher::new(p).map_err(|e| format!("error_output が正規表現として読めない: {e}"))?;
                }
                let ok_file = opt(&self.ok_file);
                if let Some(Err(e)) = ok_file.as_deref().map(koyomi_core::template::check) {
                    return Err(format!("ok_file: {e}"));
                }
                out.judge = if warn.is_empty() && error.is_empty() && error_output.is_none() && ok_file.is_none() {
                    None
                } else {
                    Some(Judge { warn, error, error_output, ok_file })
                };
            }
            Part::Join { .. } => out.part = Part::Join { wait: self.wait },
            Part::Time { .. } => {
                let days: u32 = if self.days.trim().is_empty() {
                    0
                } else {
                    self.days.trim().parse().map_err(|_| format!("days は数で書く: \"{}\"", self.days.trim()))?
                };
                out.part = Part::Time { at: when::time(&self.at)?, days };
            }
            Part::File { .. } => {
                if self.file.trim().is_empty() {
                    return Err("file は待つファイルのパス".into());
                }
                out.part = Part::File { file: self.file.trim().into(), settle: opt_secs(&self.settle)? };
            }
            Part::Jobnet { .. } => {
                if !jobnet::is_id(self.use_.trim()) {
                    return Err(format!("use は \"グループ/名前\": \"{}\"", self.use_.trim()));
                }
                out.part = Part::Jobnet { use_: self.use_.trim().into() };
            }
            Part::Chore(c) => {
                let chore = match c {
                    Chore::Delete { .. } => {
                        let n = self.keep_days.trim();
                        let keep_days = n.parse::<u32>().map_err(|_| format!("keep_days は日数で書く: \"{n}\""))?;
                        Chore::Delete { path: self.path.trim().into(), pattern: opt(&self.pattern), keep_days }
                    }
                    Chore::Empty { .. } => Chore::Empty { path: self.path.trim().into() },
                    Chore::Zip { .. } => Chore::Zip { from: self.from.trim().into(), into: self.into.trim().into() },
                    Chore::Copy { .. } => {
                        Chore::Copy { from: self.from.trim().into(), into: self.into.trim().into(), pattern: opt(&self.pattern) }
                    }
                    Chore::Send { .. } => Chore::Send { from: self.from.trim().into(), into: self.into.trim().into(), key: opt(&self.key) },
                    Chore::Fetch { .. } => {
                        Chore::Fetch { from: self.from.trim().into(), into: self.into.trim().into(), key: opt(&self.key) }
                    }
                };
                chore.check()?;
                for p in chore.paths() {
                    koyomi_core::template::check(p)?;
                }
                out.part = Part::Chore(chore);
                out.timeout = opt_secs(&self.timeout)?;
                out.retry = retry()?;
            }
            Part::Branch | Part::Notify | Part::Approval => {}
        }
        Ok(out)
    }
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.into())
    }
}

/// The fields of the net itself.
#[derive(Clone, Debug, PartialEq)]
pub struct NetFields {
    pub name: String,
    pub calendar: String,
    pub missed: Missed,
    pub overlap: Overlap,
    pub wake: bool,
    pub whens: Vec<WhenEdit>,
}

impl NetFields {
    pub fn of(net: &Jobnet) -> NetFields {
        NetFields {
            name: net.name.clone().unwrap_or_default(),
            calendar: net.calendar.clone().unwrap_or_default(),
            missed: net.missed,
            overlap: net.overlap,
            wake: net.wake,
            whens: net.when.iter().map(WhenEdit::from_when).collect(),
        }
    }
}

/// What the right pane shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Pane {
    Part(String, Box<Fields>),
    Net(Box<NetFields>),
}

pub struct Editor {
    pub id: String,
    net: Jobnet,
    /// What the file hashed to when it was opened: the base of the save.
    pub base: Option<String>,
    /// The file as it was opened (the diff of a conflict is against it).
    pub base_text: String,
    undo: Vec<Jobnet>,
    redo: Vec<Jobnet>,
    /// What the last change was, when the next of the same kind joins it in one undo step.
    last_edit: Option<String>,
    selected: BTreeSet<String>,
    /// The part the keys work from.
    cursor: Option<String>,
    pan: Vec2,
    zoom: f32,
    /// The canvas as last drawn: its size and the flow, for `fit` and `jump`.
    view: Vec2,
    vertical: bool,
    drag: Drag,
    /// `c` mode: the target chosen so far.
    connect: Option<Option<String>>,
    menu: Option<Menu>,
    pane: Option<Pane>,
    clip: Vec<Job>,
    pub dirty: bool,
    problems: Vec<write::Found>,
    /// The engine said the file changed under us: the text we tried to save.
    pub conflict: Option<String>,
    /// Asked to close with changes unsaved.
    pub closing: bool,
    pub result: Option<Result<String, String>>,
}

/// The parts of the shelf, in order.
pub fn shelf() -> [Part; 15] {
    [
        Part::Job,
        Part::Loop { file: None, count: Some(2) },
        Part::Jobnet { use_: String::new() },
        Part::Branch,
        Part::Join { wait: Wait::All },
        Part::Time { at: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(), days: 0 },
        Part::File { file: String::new(), settle: None },
        Part::Notify,
        Part::Approval,
        Part::Chore(Chore::blank("delete").unwrap()),
        Part::Chore(Chore::blank("zip").unwrap()),
        Part::Chore(Chore::blank("copy").unwrap()),
        Part::Chore(Chore::blank("empty").unwrap()),
        Part::Chore(Chore::blank("send").unwrap()),
        Part::Chore(Chore::blank("fetch").unwrap()),
    ]
}

fn prefix(p: &Part) -> &'static str {
    match p {
        Part::Job => "job",
        Part::Loop { .. } => "loop",
        Part::Branch => "branch",
        Part::Join { .. } => "join",
        Part::Time { .. } => "time",
        Part::File { .. } => "file",
        Part::Notify => "notify",
        Part::Approval => "approval",
        Part::Jobnet { .. } => "net",
        Part::Chore(c) => c.key(),
    }
}

fn dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 == 0.0 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

fn dashed(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    let v = to - from;
    let len = v.length();
    if len < 1.0 {
        return;
    }
    let dir = v / len;
    let mut at = 0.0;
    while at < len {
        let end = (at + 8.0).min(len);
        painter.line_segment([from + dir * at, from + dir * end], stroke);
        at += 14.0;
    }
}

impl Editor {
    /// An editor over `def`, when the def reads (a file with problems is fixed in the text first).
    pub fn open(def: &Def) -> Result<Editor, String> {
        let net = def.net.clone().ok_or_else(|| def.problems.iter().map(|p| p.message.clone()).collect::<Vec<_>>().join("\n"))?;
        let mut e = Editor::over(&def.id, net);
        e.base = Some(def.sha256.clone());
        e.base_text = def.text.clone();
        Ok(e)
    }

    /// An editor over a net that is not a file yet.
    /// A net made here, not read: unsaved until the first save writes its file.
    pub fn new_net(id: &str, net: Jobnet) -> Editor {
        let mut e = Editor::over(id, net);
        e.dirty = true;
        e
    }

    pub fn over(id: &str, net: Jobnet) -> Editor {
        let problems = write::check(&net);
        Editor {
            id: id.into(),
            net,
            base: None,
            base_text: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            selected: BTreeSet::new(),
            cursor: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            view: Vec2::ZERO,
            vertical: false,
            drag: Drag::None,
            connect: None,
            menu: None,
            pane: None,
            clip: Vec::new(),
            dirty: false,
            problems,
            conflict: None,
            closing: false,
            result: None,
        }
    }

    /// The file this would save.
    pub fn text(&self) -> String {
        write::text(&self.net)
    }

    pub fn can_save(&self) -> bool {
        self.dirty && self.problems.is_empty()
    }

    /// The engine wrote `text`: that is the base now.
    pub fn saved(&mut self, text: &str, said: String) {
        self.base = Some(approved::hash(text.as_bytes()));
        self.base_text = text.to_owned();
        self.dirty = false;
        self.conflict = None;
        self.result = Some(Ok(said));
    }

    /// Reads the file again (after a conflict): the changes here are dropped.
    pub fn reload(&mut self, def: &Def) {
        if let Ok(mut e) = Editor::open(def) {
            e.pan = self.pan;
            e.zoom = self.zoom;
            *self = e;
        }
    }

    /// One line for the tests and the bug report.
    pub fn report(&self) -> String {
        let pane = match &self.pane {
            Some(Pane::Part(id, _)) => id.as_str(),
            Some(Pane::Net(_)) => "net",
            None => "-",
        };
        format!(
            "editor: {} dirty={} parts={} selected=[{}] cursor={} pane={} problems={} connect={} new={}",
            self.id,
            self.dirty,
            self.net.jobs.len(),
            self.selected.iter().cloned().collect::<Vec<_>>().join(","),
            self.cursor.as_deref().unwrap_or("-"),
            pane,
            self.problems.len(),
            self.connect.is_some(),
            self.base.is_none(),
        )
    }

    // ---- changes ------------------------------------------------------

    /// Keeps the net for undo; `key` joins changes of one kind into one step.
    fn record(&mut self, key: Option<&str>) {
        if key.is_some() && key == self.last_edit.as_deref() {
            return;
        }
        self.undo.push(self.net.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last_edit = key.map(String::from);
    }

    fn touched(&mut self) {
        self.dirty = true;
        self.problems = write::check(&self.net);
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.net.jobs.iter().position(|j| j.id == id)
    }

    fn has(&self, id: &str) -> bool {
        self.index(id).is_some()
    }

    fn fresh_id(&self, prefix: &str) -> String {
        (1..).map(|n| format!("{prefix}{n}")).find(|id| !self.has(id)).unwrap()
    }

    /// Where every part is: `pos`, or the place `layout` gives one without.
    pub fn positions(&self) -> Vec<(f64, f64)> {
        let placed = layout::places(&self.net.jobs);
        self.net
            .jobs
            .iter()
            .zip(placed)
            .map(|(j, (l, r))| j.pos.unwrap_or((l as f64 * layout::STEP_ALONG, r as f64 * layout::STEP_ACROSS)))
            .collect()
    }

    /// Adds `part` after `after` (connected by a 正常 line) or at `at`; the new part is the cursor.
    pub fn add(&mut self, part: Part, after: Option<&str>, at: Option<(f64, f64)>) -> String {
        self.record(None);
        let id = self.fresh_id(prefix(&part));
        let positions = self.positions();
        let pos = at.or_else(|| {
            let i = self.index(after?)?;
            let (x, y) = positions[i];
            let taken = positions.iter().filter(|(px, _)| (px - x - layout::STEP_ALONG).abs() < 1.0).count();
            Some((x + layout::STEP_ALONG, y + taken as f64 * layout::STEP_ACROSS))
        });
        let run = if part == Part::Job { vec!["cmd.exe".into(), "/c".into(), "echo hello".into()] } else { Vec::new() };
        let after = after.filter(|a| self.has(a)).map(|a| vec![Line::ok(a)]).unwrap_or_default();
        self.net.jobs.push(Job { id: id.clone(), run, after, pos, part, ..Default::default() });
        self.selected = BTreeSet::from([id.clone()]);
        self.cursor = Some(id.clone());
        self.touched();
        id
    }

    /// A 正常 line from `from` to `to`; `false` when there is one, or it would be to itself.
    pub fn connect(&mut self, from: &str, to: &str) -> bool {
        if from == to || !self.has(from) {
            return false;
        }
        let Some(i) = self.index(to) else { return false };
        if self.net.jobs[i].after.iter().any(|l| l.job == from) {
            return false;
        }
        self.record(None);
        self.net.jobs[i].after.push(Line::ok(from));
        self.touched();
        true
    }

    /// Changes the kind of the `line`th line into `job`, or removes it (`None`).
    pub fn set_line(&mut self, job: &str, line: usize, on: Option<On>) {
        let Some(i) = self.index(job) else { return };
        if line >= self.net.jobs[i].after.len() {
            return;
        }
        self.record(None);
        match on {
            Some(on) => self.net.jobs[i].after[line].on = on,
            None => {
                self.net.jobs[i].after.remove(line);
            }
        }
        self.touched();
    }

    /// Removes the selected parts and the lines into them.
    pub fn delete(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        self.record(None);
        let gone = std::mem::take(&mut self.selected);
        self.net.jobs.retain(|j| !gone.contains(&j.id));
        for j in &mut self.net.jobs {
            j.after.retain(|l| !gone.contains(&l.job));
        }
        if self.cursor.as_ref().is_some_and(|c| gone.contains(c)) {
            self.cursor = None;
        }
        if let Some(Pane::Part(id, _)) = &self.pane {
            if gone.contains(id) {
                self.pane = None;
            }
        }
        self.touched();
    }

    /// Moves the selected parts by `(along, across)`.
    pub fn move_by(&mut self, along: f64, across: f64) {
        if self.selected.is_empty() {
            return;
        }
        self.record(Some("move"));
        let positions = self.positions();
        for (i, j) in self.net.jobs.iter_mut().enumerate() {
            if self.selected.contains(&j.id) {
                let (x, y) = positions[i];
                j.pos = Some(((x + along).max(0.0), (y + across).max(0.0)));
            }
        }
        self.touched();
    }

    pub fn copy(&mut self) {
        self.clip = self.net.jobs.iter().filter(|j| self.selected.contains(&j.id)).cloned().collect();
    }

    /// Pastes the copied parts with fresh ids a little off their places; the lines among them come along.
    pub fn paste(&mut self) {
        if self.clip.is_empty() {
            return;
        }
        self.record(None);
        let positions = self.positions();
        let mut names: BTreeMap<String, String> = BTreeMap::new();
        let mut taken: BTreeSet<String> = self.net.jobs.iter().map(|j| j.id.clone()).collect();
        for j in &self.clip {
            let id = if taken.contains(&j.id) {
                (2..).map(|n| format!("{}_{n}", j.id)).find(|c| !taken.contains(c)).unwrap()
            } else {
                j.id.clone()
            };
            taken.insert(id.clone());
            names.insert(j.id.clone(), id);
        }
        let mut added = BTreeSet::new();
        for j in &self.clip {
            let mut n = j.clone();
            n.id = names[&j.id].clone();
            n.after = j.after.iter().filter_map(|l| names.get(&l.job).map(|to| Line { job: to.clone(), on: l.on.clone() })).collect();
            let (x, y) = n.pos.or_else(|| self.index(&j.id).map(|i| positions[i])).unwrap_or((0.0, 0.0));
            n.pos = Some((x + 20.0, y + 20.0));
            added.insert(n.id.clone());
            self.net.jobs.push(n);
        }
        self.cursor = added.iter().next().cloned();
        self.selected = added;
        self.touched();
    }

    pub fn undo(&mut self) {
        if let Some(n) = self.undo.pop() {
            let at = self.pane_index();
            self.redo.push(std::mem::replace(&mut self.net, n));
            self.after_jump(at);
        }
    }

    pub fn redo(&mut self) {
        if let Some(n) = self.redo.pop() {
            let at = self.pane_index();
            self.undo.push(std::mem::replace(&mut self.net, n));
            self.after_jump(at);
        }
    }

    /// Where the part in the pane is in the list, so a jump over a rename keeps the pane on it.
    fn pane_index(&self) -> Option<usize> {
        match &self.pane {
            Some(Pane::Part(id, _)) => self.index(id),
            _ => None,
        }
    }

    /// After the net was swapped whole: nothing dangles, the pane shows it.
    fn after_jump(&mut self, at: Option<usize>) {
        self.last_edit = None;
        let ids: BTreeSet<String> = self.net.jobs.iter().map(|j| j.id.clone()).collect();
        if let (Some(Pane::Part(id, _)), Some(i)) = (&mut self.pane, at) {
            if !ids.contains(id.as_str()) {
                if let Some(j) = self.net.jobs.get(i) {
                    *id = j.id.clone();
                    self.cursor = Some(j.id.clone());
                }
            }
        }
        self.selected.retain(|s| ids.contains(s));
        if self.cursor.as_ref().is_some_and(|c| !ids.contains(c)) {
            self.cursor = None;
        }
        self.sync_pane();
        self.touched();
    }

    /// The pane's fields read from the net again.
    fn sync_pane(&mut self) {
        self.pane = match self.pane.take() {
            Some(Pane::Part(id, _)) => self.index(&id).map(|i| Pane::Part(id, Box::new(Fields::of(&self.net.jobs[i])))),
            Some(Pane::Net(_)) => Some(Pane::Net(Box::new(NetFields::of(&self.net)))),
            None => None,
        };
    }

    pub fn align(&mut self) {
        self.record(None);
        layout::align(&mut self.net.jobs);
        self.touched();
    }

    pub fn select_all(&mut self) {
        self.selected = self.net.jobs.iter().map(|j| j.id.clone()).collect();
    }

    /// Opens the fields of `id`.
    pub fn open_part(&mut self, id: &str) {
        if let Some(i) = self.index(id) {
            self.pane = Some(Pane::Part(id.into(), Box::new(Fields::of(&self.net.jobs[i]))));
            self.cursor = Some(id.into());
            self.selected = BTreeSet::from([id.to_owned()]);
        }
    }

    pub fn open_net(&mut self) {
        self.pane = Some(Pane::Net(Box::new(NetFields::of(&self.net))));
    }

    /// Puts `fields` into the part `id` (renaming it when the id changed); the reason when they do not read.
    pub fn apply_fields(&mut self, id: &str, fields: &Fields) -> Result<(), String> {
        let Some(i) = self.index(id) else { return Err("消えた".into()) };
        let job = fields.apply(&self.net.jobs[i])?;
        if job.id != id && self.has(&job.id) {
            return Err(format!("同じ id がある: {}", job.id));
        }
        if job == self.net.jobs[i] {
            return Ok(());
        }
        self.record(Some(&format!("field:{id}")));
        let new = job.id.clone();
        self.net.jobs[i] = job;
        if new != id {
            for j in &mut self.net.jobs {
                for l in &mut j.after {
                    if l.job == id {
                        l.job = new.clone();
                    }
                }
            }
            self.selected.remove(id);
            self.selected.insert(new.clone());
            self.cursor = Some(new.clone());
            if let Some(Pane::Part(p, _)) = &mut self.pane {
                *p = new;
            }
        }
        self.touched();
        Ok(())
    }

    /// Makes `name` the net's calendar, as picking it in the net's pane does (one undo step).
    pub fn set_calendar(&mut self, name: &str) {
        let mut f = NetFields::of(&self.net);
        f.calendar = name.to_owned();
        self.apply_net(&mut f);
        match &mut self.pane {
            // The pane keeps what is being typed in its other fields.
            Some(Pane::Net(p)) => p.calendar = name.to_owned(),
            other => *other = Some(Pane::Net(Box::new(f))),
        }
    }

    /// Puts the net's fields in; a `[[when]]` that does not read keeps its last good value and says why.
    pub fn apply_net(&mut self, f: &mut NetFields) {
        let mut net = self.net.clone();
        net.name = opt(&f.name);
        net.calendar = opt(&f.calendar);
        net.missed = f.missed;
        net.overlap = f.overlap;
        net.wake = f.wake;
        net.when.resize_with(f.whens.len(), || when::When {
            kind: when::Kind::OnStart,
            except: Vec::new(),
            start: None,
            end: None,
            on_closed: None,
            wait_for: None,
            wait_until: None,
        });
        for (i, w) in f.whens.iter_mut().enumerate() {
            match w.to_when() {
                Ok(when) => {
                    w.error = None;
                    net.when[i] = when;
                }
                Err(e) => w.error = Some(e),
            }
        }
        if net != self.net {
            self.record(Some("net"));
            self.net = net;
            self.touched();
        }
    }

    /// A toolbar or palette command.
    pub fn edit(&mut self, cmd: Cmd) -> Option<Action> {
        match cmd {
            Cmd::Save => {
                if self.can_save() {
                    return Some(Action::Save { force: false });
                }
                None
            }
            Cmd::Align => {
                self.align();
                None
            }
            Cmd::Fit => {
                self.fit();
                None
            }
            Cmd::Flow => Some(Action::Flow),
            Cmd::Undo => {
                self.undo();
                None
            }
            Cmd::Redo => {
                self.redo();
                None
            }
            Cmd::Close => {
                if self.dirty {
                    self.closing = true;
                    None
                } else {
                    Some(Action::Close)
                }
            }
        }
    }

    /// Where the part `p` (model place) lies at zoom 1 with no pan, measured from the canvas corner.
    fn flat(&self, p: (f64, f64)) -> Vec2 {
        let (x, y) = if self.vertical { (p.1 + 60.0, p.0) } else { (p.0, p.1) };
        Vec2::splat(MARGIN) + Vec2::new(x as f32, y as f32)
    }

    /// Zooms and pans so every part is in view (nothing to do before the first draw or with no parts).
    pub fn fit(&mut self) {
        let flat: Vec<Vec2> = self.positions().into_iter().map(|p| self.flat(p)).collect();
        if flat.is_empty() || self.view.x < 1.0 || self.view.y < 1.0 {
            return;
        }
        let pad = 48.0;
        let min = flat.iter().fold(Vec2::splat(f32::MAX), |a, p| Vec2::new(a.x.min(p.x), a.y.min(p.y)));
        let max = flat.iter().fold(Vec2::splat(f32::MIN), |a, p| Vec2::new(a.x.max(p.x), a.y.max(p.y)));
        let size = (max - min).max(Vec2::splat(1.0));
        let zoom = ((self.view.x - 2.0 * pad) / size.x).min((self.view.y - 2.0 * pad) / size.y).clamp(0.3, 1.5);
        self.zoom = zoom;
        // `flat` has MARGIN in it already; the pan moves the middle of the parts to the middle of the canvas.
        self.pan = (self.view - (min + max) * zoom) / 2.0 - Vec2::splat(MARGIN) * (1.0 - zoom) * 0.0;
    }

    /// Selects the part `id`, opens its fields and brings it to the middle of the canvas.
    pub fn jump(&mut self, id: &str) {
        let Some(i) = self.index(id) else { return };
        self.open_part(id);
        if self.view.x < 1.0 {
            return;
        }
        let flat = self.flat(self.positions()[i]);
        self.pan = self.view / 2.0 - flat * self.zoom;
    }

    /// The nearest part from the cursor in the direction `(dx, dy)` on the canvas.
    fn step(&self, at: &[Pos2], dx: f32, dy: f32) -> Option<String> {
        let from = self.connect.as_ref().and_then(|t| t.clone()).or_else(|| self.cursor.clone());
        let Some(from) = from else { return self.net.jobs.first().map(|j| j.id.clone()) };
        let i = self.index(&from)?;
        let p = at[i];
        let mut best: Option<(f32, usize)> = None;
        for (k, q) in at.iter().enumerate() {
            if k == i {
                continue;
            }
            let v = *q - p;
            let along = v.x * dx + v.y * dy;
            if along < 1.0 {
                continue;
            }
            let across = (v.x * dy - v.y * dx).abs();
            let score = along + across * 2.0;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, k));
            }
        }
        best.map(|(_, k)| self.net.jobs[k].id.clone())
    }

    // ---- the screen ---------------------------------------------------

    fn to_screen(&self, origin: Pos2, p: (f64, f64), vertical: bool) -> Pos2 {
        let (x, y) = if vertical { (p.1 + 60.0, p.0) } else { (p.0, p.1) };
        origin + Vec2::splat(MARGIN) + Vec2::new(x as f32, y as f32) * self.zoom + self.pan
    }

    fn to_model(&self, origin: Pos2, s: Pos2, vertical: bool) -> (f64, f64) {
        let v = (s - origin - Vec2::splat(MARGIN) - self.pan) / self.zoom;
        if vertical {
            (v.y as f64, (v.x - 60.0) as f64)
        } else {
            (v.x as f64, v.y as f64)
        }
    }

    fn port(at: Pos2, vertical: bool) -> Pos2 {
        if vertical {
            at + Vec2::new(0.0, PORT)
        } else {
            at + Vec2::new(PORT, 0.0)
        }
    }

    fn hit_part(&self, at: &[Pos2], p: Pos2) -> Option<usize> {
        (0..at.len()).rev().find(|&i| (at[i] - p).length() <= RADIUS + 4.0)
    }

    fn hit_port(&self, at: &[Pos2], p: Pos2, vertical: bool) -> Option<usize> {
        (0..at.len()).rev().find(|&i| (Self::port(at[i], vertical) - p).length() <= 6.0)
    }

    /// `(job index, line index)` of the line under `p`.
    fn hit_line(&self, at: &[Pos2], p: Pos2) -> Option<(usize, usize)> {
        for (i, j) in self.net.jobs.iter().enumerate() {
            for (k, l) in j.after.iter().enumerate() {
                if let Some(f) = self.index(&l.job) {
                    if dist(p, at[f], at[i]) <= NEAR {
                        return Some((i, k));
                    }
                }
            }
        }
        None
    }

    /// The editor: toolbar, shelf, fields and the canvas.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        t: &Table,
        vertical: bool,
        calendars: &BTreeMap<String, Calendar>,
        power: &crate::proto::Power,
        depth: usize,
    ) -> Option<Action> {
        let mut action = None;
        egui::Panel::top("editor-bar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let title = format!(
                    "{} {}{}{}",
                    t.tr("canvas.title"),
                    self.id,
                    if self.base.is_none() { t.tr("canvas.new") } else { "" },
                    if self.dirty { " *" } else { "" }
                );
                let r = ui.heading(&title);
                ito_a11y::say(&r, Role::Heading, &title);
                if depth > 0 {
                    let r = ui.button(t.tr("canvas.back"));
                    ito_a11y::say(&r, Role::Button, t.tr("canvas.back"));
                    if r.clicked() {
                        if let Some(a) = self.edit(Cmd::Close) {
                            action = Some(a);
                        }
                    }
                }
                let r = ui.add_enabled(self.can_save(), egui::Button::new(t.tr("canvas.save")));
                ito_a11y::say(&r, Role::Button, t.tr("canvas.save"));
                if r.clicked() {
                    action = self.edit(Cmd::Save);
                }
                for (cmd, key) in
                    [(Cmd::Align, "canvas.align"), (Cmd::Fit, "canvas.fit"), (Cmd::Undo, "canvas.undo"), (Cmd::Redo, "canvas.redo")]
                {
                    let can = match cmd {
                        Cmd::Undo => !self.undo.is_empty(),
                        Cmd::Redo => !self.redo.is_empty(),
                        _ => true,
                    };
                    let r = ui.add_enabled(can, egui::Button::new(t.tr(key)));
                    ito_a11y::say(&r, Role::Button, t.tr(key));
                    if r.clicked() {
                        self.edit(cmd);
                    }
                }
                let flow = t.tr(if vertical { "run.horizontal" } else { "run.vertical" });
                let r = ui.button(flow);
                ito_a11y::say(&r, Role::Button, flow);
                if r.clicked() {
                    action = Some(Action::Flow);
                }
                let r = ui.button(t.tr("canvas.net"));
                ito_a11y::say(&r, Role::Button, t.tr("canvas.net"));
                if r.clicked() {
                    self.open_net();
                }
                let r = ui.button(t.tr("canvas.close"));
                ito_a11y::say(&r, Role::Button, t.tr("canvas.close"));
                if r.clicked() {
                    if let Some(a) = self.edit(Cmd::Close) {
                        action = Some(a);
                    }
                }
            });
            let c = ito_theme::colors();
            if !self.problems.is_empty() {
                let head = t.tr("canvas.problems");
                let r = ui.colored_label(c.err, Mark::Error.marked(head));
                ito_a11y::say(&r, Role::Label, head);
                let mut go = None;
                for p in &self.problems {
                    match &p.job {
                        Some(j) => {
                            let text = format!("{j}: {}", p.message);
                            let r =
                                ui.add(egui::Label::new(egui::RichText::new(text.clone()).color(c.err).underline()).sense(Sense::click()));
                            ito_a11y::say(&r, Role::Button, &text);
                            if r.on_hover_text(t.tr("canvas.jump")).clicked() {
                                go = Some(j.clone());
                            }
                        }
                        None => {
                            ui.colored_label(c.err, p.message.clone());
                        }
                    }
                }
                if let Some(j) = go {
                    self.jump(&j);
                }
            }
            match &self.result {
                Some(Ok(m)) => {
                    ui.colored_label(c.done, Mark::Done.marked(m));
                }
                Some(Err(e)) => {
                    ui.colored_label(c.err, Mark::Error.marked(e));
                }
                None => {}
            }
        });
        egui::Panel::left("shelf").default_size(130.0).size_range(90.0..=320.0).resizable(true).show(ui, |ui| {
            let r = ui.label(t.tr("canvas.shelf"));
            ito_a11y::say(&r, Role::Heading, t.tr("canvas.shelf"));
            for part in shelf() {
                let name = show::part_name(t, &part);
                let mark = match part.key() {
                    "job" => "▭ ",
                    "jobnet" => "▣ ",
                    "branch" => "◇ ",
                    _ => "",
                };
                let label = format!("{mark}{name}");
                let r = ui.add(egui::Button::new(&label).sense(Sense::click_and_drag()));
                ito_a11y::say(&r, Role::Button, &label);
                if r.drag_started() {
                    self.drag = Drag::Shelf(part.clone());
                } else if r.clicked() {
                    let after = self.cursor.clone();
                    self.add(part.clone(), after.as_deref(), None);
                }
            }
            ui.add_space(8.0);
            ui.small(t.tr("canvas.hint"));
        });
        if self.pane.is_some() {
            egui::Panel::right("fields").default_size(320.0).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.fields(ui, t, calendars, power, &mut action));
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(a) = self.canvas(ui, t, vertical) {
                action = Some(a);
            }
        });
        action
    }

    fn fields(
        &mut self,
        ui: &mut egui::Ui,
        t: &Table,
        calendars: &BTreeMap<String, Calendar>,
        power: &crate::proto::Power,
        action: &mut Option<Action>,
    ) {
        let c = ito_theme::colors();
        let field = |ui: &mut egui::Ui, key: &str, s: &mut String| -> bool {
            let label = t.tr(key);
            let mut hit = false;
            ui.horizontal(|ui| {
                ui.label(label);
                let r = ui.add(egui::TextEdit::singleline(s).desired_width(f32::INFINITY));
                ito_a11y::say(&r, Role::Label, label);
                hit = r.changed();
            });
            hit
        };
        let Some(pane) = self.pane.take() else { return };
        match pane {
            Pane::Part(mut id, f) => {
                let mut f = *f;
                let Some(i) = self.index(&id) else { return };
                let part = self.net.jobs[i].part.clone();
                let mut closed = false;
                ui.horizontal(|ui| {
                    let head = show::part_name(t, &part);
                    let r = ui.heading(&head);
                    ito_a11y::say(&r, Role::Heading, &head);
                    let r = ui.button(t.tr("canvas.close_pane"));
                    ito_a11y::say(&r, Role::Button, t.tr("canvas.close_pane"));
                    closed = r.clicked();
                });
                let mut changed = field(ui, "field.id", &mut f.id);
                changed |= field(ui, "field.name", &mut f.name);
                ui.label(t.tr("field.note"));
                let r = ui.add(egui::TextEdit::multiline(&mut f.note).desired_rows(2).desired_width(f32::INFINITY));
                ito_a11y::say(&r, Role::Label, t.tr("field.note"));
                changed |= r.changed();
                match part {
                    Part::Job | Part::Loop { .. } => {
                        if matches!(part, Part::Loop { .. }) {
                            changed |= field(ui, "field.loop_file", &mut f.file);
                            changed |= field(ui, "field.count", &mut f.count);
                        }
                        ui.label(t.tr("field.run"));
                        let r = ui.add(egui::TextEdit::multiline(&mut f.run).desired_rows(3).desired_width(f32::INFINITY));
                        ito_a11y::say(&r, Role::Label, t.tr("field.run"));
                        changed |= r.changed();
                        changed |= field(ui, "field.dir", &mut f.dir);
                        changed |= field(ui, "field.timeout", &mut f.timeout);
                        changed |= field(ui, "field.retry_count", &mut f.retry_count);
                        changed |= field(ui, "field.retry_wait", &mut f.retry_wait);
                        changed |= field(ui, "field.shell", &mut f.shell);
                        ui.label(t.tr("field.script"));
                        let r = ui.add(egui::TextEdit::multiline(&mut f.script).desired_rows(4).desired_width(f32::INFINITY).code_editor());
                        ito_a11y::say(&r, Role::Label, t.tr("field.script"));
                        changed |= r.changed();
                        changed |= field(ui, "field.warn", &mut f.warn);
                        changed |= field(ui, "field.error", &mut f.error);
                        changed |= field(ui, "field.error_output", &mut f.error_output);
                        changed |= field(ui, "field.ok_file", &mut f.ok_file);
                        ui.horizontal(|ui| {
                            ui.label(t.tr("field.encoding"));
                            for e in [Encoding::Auto, Encoding::Sjis, Encoding::Utf8] {
                                let key = format!("field.encoding_{}", e.text());
                                if ui.selectable_label(f.encoding == e, t.tr(&key)).clicked() && f.encoding != e {
                                    f.encoding = e;
                                    changed = true;
                                }
                            }
                        });
                    }
                    Part::Join { .. } => {
                        ui.horizontal(|ui| {
                            ui.label(t.tr("review.wait"));
                            for (w, key) in [(Wait::All, "review.wait_all"), (Wait::Any, "review.wait_any")] {
                                if ui.selectable_label(f.wait == w, t.tr(key)).clicked() && f.wait != w {
                                    f.wait = w;
                                    changed = true;
                                }
                            }
                        });
                    }
                    Part::Time { .. } => {
                        changed |= field(ui, "review.at", &mut f.at);
                        changed |= field(ui, "field.days", &mut f.days);
                    }
                    Part::File { .. } => {
                        changed |= field(ui, "review.file_wait", &mut f.file);
                        changed |= field(ui, "review.settle", &mut f.settle);
                    }
                    Part::Jobnet { .. } => {
                        changed |= field(ui, "review.use", &mut f.use_);
                        let r = ui.button(t.tr("canvas.enter"));
                        ito_a11y::say(&r, Role::Button, t.tr("canvas.enter"));
                        if r.clicked() && jobnet::is_id(f.use_.trim()) {
                            *action = Some(Action::Enter(f.use_.trim().to_owned()));
                        }
                    }
                    Part::Chore(c) => {
                        match c {
                            Chore::Delete { .. } => {
                                changed |= field(ui, "field.path", &mut f.path);
                                changed |= field(ui, "field.match", &mut f.pattern);
                                changed |= field(ui, "field.keep_days", &mut f.keep_days);
                            }
                            Chore::Empty { .. } => changed |= field(ui, "field.path", &mut f.path),
                            Chore::Zip { .. } => {
                                changed |= field(ui, "field.zip_from", &mut f.from);
                                changed |= field(ui, "field.zip_into", &mut f.into);
                            }
                            Chore::Copy { .. } => {
                                changed |= field(ui, "field.from", &mut f.from);
                                changed |= field(ui, "field.into", &mut f.into);
                                changed |= field(ui, "field.match", &mut f.pattern);
                            }
                            Chore::Send { .. } | Chore::Fetch { .. } => {
                                let send = matches!(c, Chore::Send { .. });
                                changed |= field(ui, if send { "field.send_from" } else { "field.fetch_from" }, &mut f.from);
                                changed |= field(ui, if send { "field.send_into" } else { "field.fetch_into" }, &mut f.into);
                                changed |= field(ui, "field.key", &mut f.key);
                            }
                        }
                        changed |= field(ui, "field.timeout", &mut f.timeout);
                        changed |= field(ui, "field.retry_count", &mut f.retry_count);
                        changed |= field(ui, "field.retry_wait", &mut f.retry_wait);
                    }
                    Part::Branch | Part::Notify | Part::Approval => {}
                }
                if changed {
                    f.msg = self.apply_fields(&id, &f).err();
                    // A rename that went through: the pane follows the part.
                    let new = f.id.trim().to_owned();
                    if f.msg.is_none() && new != id && self.has(&new) {
                        id = new;
                    }
                }
                if let Some(m) = &f.msg {
                    ui.colored_label(c.err, Mark::Error.marked(m));
                }
                if !closed {
                    self.pane = Some(Pane::Part(id, Box::new(f)));
                }
            }
            Pane::Net(f) => {
                let mut f = *f;
                let mut closed = false;
                ui.horizontal(|ui| {
                    let r = ui.heading(t.tr("canvas.net"));
                    ito_a11y::say(&r, Role::Heading, t.tr("canvas.net"));
                    let r = ui.button(t.tr("canvas.close_pane"));
                    ito_a11y::say(&r, Role::Button, t.tr("canvas.close_pane"));
                    closed = r.clicked();
                });
                let mut changed = field(ui, "field.name", &mut f.name);
                ui.horizontal(|ui| {
                    ui.label(t.tr("field.calendar"));
                    let shown = if f.calendar.is_empty() { t.tr("field.no_calendar").to_owned() } else { f.calendar.clone() };
                    egui::ComboBox::from_id_salt("net-calendar").selected_text(shown).show_ui(ui, |ui| {
                        if ui.selectable_label(f.calendar.is_empty(), t.tr("field.no_calendar")).clicked() {
                            f.calendar.clear();
                            changed = true;
                        }
                        for name in calendars.keys() {
                            if ui.selectable_label(&f.calendar == name, name).clicked() {
                                f.calendar = name.clone();
                                changed = true;
                            }
                        }
                        ui.separator();
                        if ui.selectable_label(false, t.tr("palette.new_calendar")).clicked() {
                            *action = Some(Action::NewCalendar);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(t.tr("field.missed"));
                    for (m, key) in [(Missed::Skip, "field.skip"), (Missed::Once, "review.missed_once")] {
                        if ui.selectable_label(f.missed == m, t.tr(key)).clicked() && f.missed != m {
                            f.missed = m;
                            changed = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(t.tr("field.overlap"));
                    for (o, key) in [(Overlap::Skip, "field.skip"), (Overlap::Queue, "review.overlap_queue")] {
                        if ui.selectable_label(f.overlap == o, t.tr(key)).clicked() && f.overlap != o {
                            f.overlap = o;
                            changed = true;
                        }
                    }
                });
                changed |= ui.checkbox(&mut f.wake, t.tr("field.wake")).changed();
                ui.separator();
                let r = ui.heading(t.tr("when.title"));
                ito_a11y::say(&r, Role::Heading, t.tr("when.title"));
                let mut remove = None;
                for (i, w) in f.whens.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            changed |= w.show(ui, t, i);
                            let r = ui.button(t.tr("when.remove"));
                            ito_a11y::say(&r, Role::Button, t.tr("when.remove"));
                            if r.clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    f.whens.remove(i);
                    changed = true;
                }
                let r = ui.button(t.tr("when.add"));
                ito_a11y::say(&r, Role::Button, t.tr("when.add"));
                if r.clicked() {
                    f.whens.push(WhenEdit::default());
                    changed = true;
                }
                if changed {
                    self.apply_net(&mut f);
                }
                ui.separator();
                let cal = self.net.calendar.as_ref().and_then(|n| calendars.get(n));
                let head = t.trf("run.next", &["10"]);
                let r = ui.label(&head);
                ito_a11y::say(&r, Role::Heading, &head);
                let sleep = show::Sleep::of(power, self.net.wake);
                for row in show::next_rows(
                    t,
                    &self.net.when,
                    (&self.net.quiet, self.net.on_quiet),
                    cal,
                    chrono::Local::now().naive_local(),
                    10,
                    Some(&sleep),
                ) {
                    ui.label(row);
                }
                if !closed {
                    self.pane = Some(Pane::Net(Box::new(f)));
                }
            }
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, t: &Table, vertical: bool) -> Option<Action> {
        let c = ito_theme::colors();
        let accent = c.blue;
        let mut action = None;
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        let origin = rect.min;
        self.view = rect.size();
        self.vertical = vertical;
        painter.rect_filled(rect, 0.0, c.bg);
        let at: Vec<Pos2> = self.positions().into_iter().map(|p| self.to_screen(origin, p, vertical)).collect();
        let pointer = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
        let (command, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));

        // The wheel zooms about the pointer.
        if resp.hovered() {
            let (scroll, zd) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            let factor = if zd != 1.0 { zd } else { (scroll.y * 0.002).exp() };
            if factor != 1.0 {
                let new = (self.zoom * factor).clamp(0.3, 3.0);
                let factor = new / self.zoom;
                if let Some(p) = pointer {
                    let rel = p - origin - Vec2::splat(MARGIN);
                    self.pan = rel - (rel - self.pan) * factor;
                }
                self.zoom = new;
            }
        }

        // The mouse.
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                self.menu = None;
                if let Some(i) = self.hit_port(&at, p, vertical) {
                    self.drag = Drag::Connect { from: self.net.jobs[i].id.clone() };
                } else if let Some(i) = self.hit_part(&at, p) {
                    let id = self.net.jobs[i].id.clone();
                    if !self.selected.contains(&id) {
                        if !command {
                            self.selected.clear();
                        }
                        self.selected.insert(id.clone());
                    }
                    self.cursor = Some(id);
                    self.drag = Drag::Parts { moved: false };
                } else if shift {
                    self.drag = Drag::Range { from: p };
                } else {
                    self.drag = Drag::Pan;
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            let d = resp.drag_delta();
            match &mut self.drag {
                Drag::Parts { moved } => {
                    *moved = true;
                    let m = d / self.zoom;
                    let (along, across) = if vertical { (m.y, m.x) } else { (m.x, m.y) };
                    self.move_by(along as f64, across as f64);
                }
                Drag::Pan => self.pan += d,
                _ => {}
            }
        }
        if resp.drag_stopped() {
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::Connect { from } => {
                    if let Some(i) = pointer.and_then(|p| self.hit_part(&at, p)) {
                        let to = self.net.jobs[i].id.clone();
                        self.connect(&from, &to);
                    }
                }
                Drag::Range { from } => {
                    if let Some(p) = pointer {
                        let r = Rect::from_two_pos(from, p);
                        if !command {
                            self.selected.clear();
                        }
                        for (i, j) in self.net.jobs.iter().enumerate() {
                            if r.contains(at[i]) {
                                self.selected.insert(j.id.clone());
                            }
                        }
                    }
                }
                Drag::Parts { .. } => self.last_edit = None,
                Drag::Shelf(part) => {
                    if let Some(p) = pointer.filter(|p| rect.contains(*p)) {
                        let pos = self.to_model(origin, p, vertical);
                        self.add(part, None, Some((pos.0.max(0.0), pos.1.max(0.0))));
                    }
                }
                Drag::None | Drag::Pan => {}
            }
        }
        // A part from the shelf: the shelf's button ended its drag, not this canvas.
        if let Drag::Shelf(part) = &self.drag {
            if ui.input(|i| i.pointer.any_released()) {
                let part = part.clone();
                self.drag = Drag::None;
                if let Some(p) = ui.input(|i| i.pointer.latest_pos()).filter(|p| rect.contains(*p)) {
                    let pos = self.to_model(origin, p, vertical);
                    self.add(part, None, Some((pos.0.max(0.0), pos.1.max(0.0))));
                }
            } else if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                painter.circle(p, RADIUS, c.panel, Stroke::new(1.5, accent));
            }
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.menu = None;
                if let Some(i) = self.hit_part(&at, p) {
                    let id = self.net.jobs[i].id.clone();
                    if let Some(Some(_) | None) = &self.connect {
                        if let Some(from) = self.cursor.clone() {
                            self.connect(&from, &id);
                        }
                        self.connect = None;
                    } else if command {
                        if !self.selected.remove(&id) {
                            self.selected.insert(id.clone());
                        }
                        self.cursor = Some(id);
                    } else if !matches!(self.drag, Drag::Parts { moved: true }) {
                        self.selected = BTreeSet::from([id.clone()]);
                        self.cursor = Some(id);
                    }
                } else if !shift && !command {
                    self.selected.clear();
                    self.connect = None;
                }
            }
        }
        if resp.double_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                match self.hit_part(&at, p) {
                    Some(i) => {
                        let id = self.net.jobs[i].id.clone();
                        if let Part::Jobnet { use_ } = &self.net.jobs[i].part {
                            if jobnet::is_id(use_) {
                                action = Some(Action::Enter(use_.clone()));
                            }
                        }
                        self.open_part(&id);
                    }
                    None => self.open_net(),
                }
            }
        }
        if resp.secondary_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.menu = if let Some(i) = self.hit_part(&at, p) {
                    let id = self.net.jobs[i].id.clone();
                    if !self.selected.contains(&id) {
                        self.selected = BTreeSet::from([id.clone()]);
                    }
                    self.cursor = Some(id.clone());
                    Some(Menu::Part { job: id, at: p })
                } else if let Some((i, k)) = self.hit_line(&at, p) {
                    Some(Menu::Line { job: self.net.jobs[i].id.clone(), line: k, at: p })
                } else {
                    None
                };
            }
        }

        // The keys, when no field has them.
        let typing = ui.memory(|m| m.focused().is_some_and(|f| f != resp.id));
        if !typing && self.menu.is_none() {
            self.keys(ui, &at, &mut action);
        }

        // The lines.
        let small = egui::TextStyle::Small.resolve(ui.style());
        for (i, j) in self.net.jobs.iter().enumerate() {
            for l in &j.after {
                let Some(f) = self.index(&l.job) else { continue };
                let (from, to) = (at[f], at[i]);
                let v = to - from;
                if v.length() < RADIUS * 2.0 + 2.0 {
                    continue;
                }
                let dir = v / v.length();
                let (a, b) = (from + dir * RADIUS, to - dir * (RADIUS + 2.0));
                let (col, width, dash, mark) = match &l.on {
                    On::Ok => (c.done, 2.0, false, Mark::Done.mark().to_owned()),
                    On::Error => (c.err, 2.0, true, Mark::Error.mark().to_owned()),
                    On::Any => (c.dim, 1.5, false, "*".to_owned()),
                    On::Code(r) => (c.wait, 2.0, false, Judge::list(r)),
                    On::Else => (c.dim, 1.5, true, "else".to_owned()),
                };
                let stroke = Stroke::new(width * self.zoom.max(0.6), col);
                if dash {
                    dashed(&painter, a, b, stroke);
                } else {
                    painter.line_segment([a, b], stroke);
                }
                // The head.
                let side = Vec2::new(-dir.y, dir.x);
                painter.line_segment([b, b - dir * 7.0 + side * 4.0], stroke);
                painter.line_segment([b, b - dir * 7.0 - side * 4.0], stroke);
                let mid = a + v * 0.5;
                painter.text(mid + side * 8.0, egui::Align2::CENTER_CENTER, mark, small.clone(), col);
            }
        }
        // A line being drawn, or chosen with `c`.
        if let Drag::Connect { from } = &self.drag {
            if let (Some(f), Some(p)) = (self.index(from), pointer) {
                dashed(&painter, Self::port(at[f], vertical), p, Stroke::new(1.5, accent));
            }
        }
        if let Some(target) = &self.connect {
            if let (Some(f), Some(tgt)) = (self.cursor.as_ref().and_then(|c| self.index(c)), target.as_ref().and_then(|t| self.index(t))) {
                dashed(&painter, at[f], at[tgt], Stroke::new(2.0, accent));
            }
        }
        // The parts.
        let bad: BTreeSet<&str> = self.problems.iter().filter_map(|p| p.job.as_deref()).collect();
        let name_font = egui::TextStyle::Body.resolve(ui.style());
        for (i, j) in self.net.jobs.iter().enumerate() {
            let p = at[i];
            let r = RADIUS * self.zoom.max(0.6);
            let selected = self.selected.contains(&j.id);
            let is_cursor = self.cursor.as_deref() == Some(j.id.as_str());
            let fill = if bad.contains(j.id.as_str()) {
                c.err
            } else if selected {
                accent
            } else {
                c.panel
            };
            let ring = if is_cursor { Stroke::new(3.0, accent) } else { Stroke::new(1.5, c.fg) };
            painter.circle(p, r, fill, ring);
            let glyph = match &j.part {
                Part::Job => None,
                part => show::kind(t, part).chars().next(),
            };
            if let Some(g) = glyph {
                painter.text(p, egui::Align2::CENTER_CENTER, g, small.clone(), if selected { c.bg } else { c.fg });
            }
            if bad.contains(j.id.as_str()) {
                painter.text(p + Vec2::new(r + 2.0, -r - 2.0), egui::Align2::LEFT_BOTTOM, Mark::Error.mark(), small.clone(), c.err);
            }
            // The out port.
            let port = Self::port(p, vertical);
            let hot = pointer.is_some_and(|q| (port - q).length() <= 6.0);
            painter.circle(port, 3.5, if hot { accent } else { c.border }, Stroke::new(1.0, c.fg));
            let name = j.name.clone().unwrap_or_else(|| j.id.clone());
            let label = if selected { Mark::Selected.marked(&name) } else { name };
            let label_rect = painter.text(
                p + Vec2::new(0.0, r + 4.0),
                egui::Align2::CENTER_TOP,
                label,
                name_font.clone(),
                if selected { accent } else { c.fg },
            );
            if let Some(note) = j.note.as_deref().filter(|n| !n.is_empty()) {
                let galley = painter.layout(note.to_owned(), small.clone(), c.wait_text(), 140.0 * self.zoom.max(0.6));
                let top = label_rect.center_bottom() + Vec2::new(0.0, 4.0);
                let box_rect = Rect::from_min_size(top - Vec2::new(galley.size().x / 2.0, 0.0), galley.size()).expand(3.0);
                painter.rect(box_rect, 2.0, c.wait_bg(), Stroke::new(1.0, c.border), StrokeKind::Inside);
                painter.galley(box_rect.min + Vec2::splat(3.0), galley, c.wait_text());
            }
        }
        if let Drag::Range { from } = self.drag {
            if let Some(p) = pointer {
                let r = Rect::from_two_pos(from, p);
                painter.rect_filled(r, 0.0, accent.gamma_multiply(0.15));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), StrokeKind::Inside);
            }
        }
        if self.net.jobs.is_empty() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, t.tr("canvas.empty"), name_font, c.dim);
        }
        if self.connect.is_some() {
            painter.text(rect.left_bottom() + Vec2::new(8.0, -8.0), egui::Align2::LEFT_BOTTOM, t.tr("canvas.connecting"), small, accent);
        }
        self.show_menu(ui, t);
        action
    }

    fn keys(&mut self, ui: &mut egui::Ui, at: &[Pos2], action: &mut Option<Action>) {
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let pressed = |ui: &mut egui::Ui, m: Modifiers, k: Key| ui.input_mut(|i| i.consume_key(m, k));
        if pressed(ui, cmd, Key::Z) {
            self.undo();
        } else if pressed(ui, cmd, Key::Y) || pressed(ui, cmd | Modifiers::SHIFT, Key::Z) {
            self.redo();
        } else if pressed(ui, cmd, Key::C) {
            self.copy();
        } else if pressed(ui, cmd, Key::V) {
            self.paste();
        } else if pressed(ui, cmd, Key::A) {
            self.select_all();
        } else if pressed(ui, cmd, Key::S) {
            if let Some(a) = self.edit(Cmd::Save) {
                *action = Some(a);
            }
        } else if pressed(ui, Modifiers::NONE, Key::Delete) || pressed(ui, Modifiers::NONE, Key::Backspace) {
            self.delete();
        } else if pressed(ui, Modifiers::NONE, Key::Escape) {
            if self.connect.is_some() {
                self.connect = None;
            } else {
                self.selected.clear();
            }
        } else if pressed(ui, Modifiers::NONE, Key::A) {
            let after = self.cursor.clone();
            self.add(Part::Job, after.as_deref(), None);
        } else if pressed(ui, Modifiers::NONE, Key::C) {
            if self.cursor.is_some() {
                self.connect = Some(None);
            }
        } else if pressed(ui, Modifiers::NONE, Key::Enter) {
            match self.connect.take() {
                Some(Some(to)) => {
                    if let Some(from) = self.cursor.clone() {
                        self.connect(&from, &to);
                    }
                }
                Some(None) => {}
                None => {
                    if let Some(id) = self.cursor.clone() {
                        self.open_part(&id);
                    }
                }
            }
        } else {
            let dirs = [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)];
            for (k, dx, dy) in dirs {
                if pressed(ui, Modifiers::NONE, k) {
                    if let Some(id) = self.step(at, dx, dy) {
                        match &mut self.connect {
                            Some(target) => *target = Some(id),
                            None => {
                                self.cursor = Some(id.clone());
                                self.selected = BTreeSet::from([id]);
                            }
                        }
                    }
                }
            }
        }
    }

    fn show_menu(&mut self, ui: &mut egui::Ui, t: &Table) {
        let Some(menu) = self.menu.clone() else { return };
        let at = match &menu {
            Menu::Line { at, .. } | Menu::Part { at, .. } => *at,
        };
        let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Area::new(ui.id().with("canvas-menu")).fixed_pos(at).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| match &menu {
                Menu::Line { job, line, .. } => {
                    let choices = [
                        ("canvas.line_ok", Some(On::Ok)),
                        ("canvas.line_error", Some(On::Error)),
                        ("canvas.line_any", Some(On::Any)),
                        ("canvas.line_delete", None),
                    ];
                    for (key, on) in choices {
                        let r = ui.button(t.tr(key));
                        ito_a11y::say(&r, Role::Button, t.tr(key));
                        if r.clicked() {
                            self.set_line(job, *line, on);
                            close = true;
                        }
                    }
                }
                Menu::Part { job, .. } => {
                    let r = ui.button(t.tr("canvas.open"));
                    ito_a11y::say(&r, Role::Button, t.tr("canvas.open"));
                    if r.clicked() {
                        self.open_part(job);
                        close = true;
                    }
                    let r = ui.button(t.tr("canvas.connect_from"));
                    ito_a11y::say(&r, Role::Button, t.tr("canvas.connect_from"));
                    if r.clicked() {
                        self.cursor = Some(job.clone());
                        self.connect = Some(None);
                        close = true;
                    }
                    let r = ui.button(t.tr("canvas.delete"));
                    ito_a11y::say(&r, Role::Button, t.tr("canvas.delete"));
                    if r.clicked() {
                        self.delete();
                        close = true;
                    }
                }
            });
        });
        if close {
            self.menu = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(text: &str) -> Jobnet {
        jobnet::parse(text).unwrap()
    }

    fn two() -> Editor {
        Editor::over(
            "経理/集計",
            net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"b\"\nrun = [\"y\"]\nafter = [\"a\"]\n"),
        )
    }

    #[test]
    fn a_part_is_added_after_the_cursor_and_connected() {
        let mut e = two();
        assert!(!e.dirty);
        e.cursor = Some("b".into());
        let id = e.add(Part::Job, Some("b"), None);
        assert_eq!(id, "job1");
        assert_eq!(e.net.jobs[2].after, vec![Line::ok("b")]);
        assert_eq!(e.cursor.as_deref(), Some("job1"));
        assert!(e.dirty && e.problems.is_empty(), "{:?}", e.problems);
        // Placed one step along b, not on top of it.
        let p = e.positions();
        assert_eq!(p[2].0, p[1].0 + layout::STEP_ALONG);
        // A second one after b goes a row down.
        let id = e.add(Part::Branch, Some("b"), None);
        assert_eq!(id, "branch1");
        let p = e.positions();
        assert_eq!(p[3], (p[2].0, p[2].1 + layout::STEP_ACROSS));
        assert!(e.text().contains("kind = \"branch\""));
    }

    #[test]
    fn lines_connect_once_and_change_or_go() {
        let mut e = two();
        assert!(!e.connect("a", "a"));
        assert!(!e.connect("a", "b"), "already there");
        assert!(!e.connect("nope", "b"));
        assert!(e.connect("b", "a"));
        assert!(!e.problems.is_empty(), "a cycle is a problem");
        e.undo();
        assert!(e.problems.is_empty());
        e.set_line("b", 0, Some(On::Error));
        assert_eq!(e.net.jobs[1].after[0].on, On::Error);
        assert!(e.text().contains("on = \"error\"") || e.text().contains("a:error") || e.text().contains("error"), "{}", e.text());
        e.set_line("b", 0, None);
        assert!(e.net.jobs[1].after.is_empty());
        e.set_line("b", 5, None);
    }

    #[test]
    fn delete_copy_paste_and_undo_keep_the_lines_straight() {
        let mut e = two();
        e.select_all();
        e.copy();
        e.paste();
        assert_eq!(e.net.jobs.len(), 4);
        assert_eq!(e.net.jobs[2].id, "a_2");
        assert_eq!(e.net.jobs[3].id, "b_2");
        assert_eq!(e.net.jobs[3].after, vec![Line::ok("a_2")], "the line among the copied ones is remapped");
        assert_eq!(e.selected, BTreeSet::from(["a_2".to_owned(), "b_2".to_owned()]));
        e.selected = BTreeSet::from(["a".to_owned()]);
        e.cursor = Some("a".into());
        e.delete();
        assert_eq!(e.net.jobs.len(), 3);
        assert!(e.net.jobs[0].after.is_empty(), "the line into the gone part goes too");
        assert_eq!(e.cursor, None);
        e.undo();
        assert_eq!(e.net.jobs.len(), 4);
        assert_eq!(e.net.jobs[1].after, vec![Line::ok("a")]);
        e.undo();
        assert_eq!(e.net.jobs.len(), 2);
        e.redo();
        assert_eq!(e.net.jobs.len(), 4);
        e.redo();
        assert_eq!(e.net.jobs.len(), 3);
        e.redo();
        assert_eq!(e.net.jobs.len(), 3);
    }

    #[test]
    fn moves_join_into_one_undo_step_and_align_writes_pos() {
        let mut e = two();
        e.selected = BTreeSet::from(["b".to_owned()]);
        let before = e.positions()[1];
        e.move_by(10.0, 5.0);
        e.move_by(10.0, 5.0);
        assert_eq!(e.net.jobs[1].pos, Some((before.0 + 20.0, before.1 + 10.0)));
        assert_eq!(e.net.jobs[0].pos, None, "an unmoved part keeps no pos");
        assert_eq!(e.undo.len(), 1);
        e.undo();
        assert_eq!(e.net.jobs[1].pos, None);
        e.align();
        assert_eq!(e.net.jobs[0].pos, Some((0.0, 0.0)));
        assert_eq!(e.net.jobs[1].pos, Some((layout::STEP_ALONG, 0.0)));
        assert!(e.text().contains("pos = ["));
    }

    #[test]
    fn the_fields_read_back_and_a_rename_follows_the_lines() {
        let mut e = two();
        e.open_part("a");
        let Some(Pane::Part(_, f)) = e.pane.clone() else { panic!() };
        let mut f = *f;
        assert_eq!(f.run, "x");
        f.run = "cmd.exe\n/c\necho hi".into();
        f.timeout = "10m".into();
        f.retry_count = "2".into();
        f.retry_wait = "30s".into();
        f.warn = "1..3".into();
        f.name = "一".into();
        assert_eq!(e.apply_fields("a", &f), Ok(()));
        let j = &e.net.jobs[0];
        assert_eq!(j.run, vec!["cmd.exe", "/c", "echo hi"]);
        assert_eq!(j.timeout, Some(600));
        assert_eq!(j.retry, Some(Retry { count: 2, wait: 30 }));
        assert_eq!(j.judge.as_ref().unwrap().warn.len(), 1);
        f.timeout = "soon".into();
        assert!(e.apply_fields("a", &f).is_err());
        assert_eq!(e.net.jobs[0].timeout, Some(600), "a field that does not read changes nothing");
        f.timeout = String::new();
        f.id = "b".into();
        assert!(e.apply_fields("a", &f).unwrap_err().contains("同じ id"));
        f.id = "a/b".into();
        assert!(e.apply_fields("a", &f).is_err());
        f.id = "first".into();
        assert_eq!(e.apply_fields("a", &f), Ok(()));
        assert_eq!(e.net.jobs[0].id, "first");
        assert_eq!(e.net.jobs[1].after, vec![Line::ok("first")]);
        assert_eq!(e.cursor.as_deref(), Some("first"));
        assert!(matches!(&e.pane, Some(Pane::Part(p, _)) if p == "first"));
        // Field edits of one part are one undo step.
        assert_eq!(e.undo.len(), 1);
        e.undo();
        assert_eq!(e.net.jobs[0].id, "a");
        assert!(matches!(&e.pane, Some(Pane::Part(p, f)) if p == "a" && f.run == "x"), "the pane follows the undo");
    }

    #[test]
    fn the_net_fields_keep_the_last_good_when() {
        let mut e = two();
        e.open_net();
        let Some(Pane::Net(f)) = e.pane.clone() else { panic!() };
        let mut f = *f;
        f.name = "集計".into();
        f.whens.push(WhenEdit::default());
        e.apply_net(&mut f);
        assert_eq!(e.net.name.as_deref(), Some("集計"));
        assert_eq!(e.net.when.len(), 1);
        assert!(e.text().contains("every = \"day\""));
        f.whens[0].at = "25:00".into();
        e.apply_net(&mut f);
        assert!(f.whens[0].error.is_some());
        assert!(e.text().contains("at = [\"02:00\"]"), "{}", e.text());
        f.whens.clear();
        e.apply_net(&mut f);
        assert!(e.net.when.is_empty());
        f.calendar = "東京".into();
        e.apply_net(&mut f);
        assert!(e.text().contains("calendar = \"東京\""), "{}", e.text());
        e.set_calendar("工場");
        assert!(e.text().contains("calendar = \"工場\""), "{}", e.text());
        assert!(matches!(&e.pane, Some(Pane::Net(f)) if f.calendar == "工場"), "the pane shows it");
        assert_eq!(e.net.name.as_deref(), Some("集計"), "the rest stays");
        e.undo();
        assert_ne!(e.net.calendar.as_deref(), Some("工場"), "undone with the net's other edits");
    }

    #[test]
    fn other_parts_have_their_own_fields() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let j = e.add(Part::Join { wait: Wait::All }, Some("a"), None);
        let tm = e.add(Part::Time { at: chrono::NaiveTime::from_hms_opt(1, 0, 0).unwrap(), days: 0 }, Some("a"), None);
        let f = e.add(Part::File { file: "x".into(), settle: None }, Some("a"), None);
        let n = e.add(Part::Jobnet { use_: "g/n".into() }, Some("a"), None);
        let mut fields = Fields::of(&e.net.jobs[1]);
        fields.wait = Wait::Any;
        assert!(e.apply_fields(&j, &fields).is_ok());
        assert_eq!(e.net.jobs[1].part, Part::Join { wait: Wait::Any });
        let mut fields = Fields::of(&e.net.jobs[2]);
        assert_eq!(fields.at, "01:00");
        fields.at = "02:30".into();
        fields.days = "1".into();
        assert!(e.apply_fields(&tm, &fields).is_ok());
        assert_eq!(e.net.jobs[2].part, Part::Time { at: chrono::NaiveTime::from_hms_opt(2, 30, 0).unwrap(), days: 1 });
        let mut fields = Fields::of(&e.net.jobs[3]);
        fields.settle = "1m".into();
        assert!(e.apply_fields(&f, &fields).is_ok());
        assert_eq!(e.net.jobs[3].part, Part::File { file: "x".into(), settle: Some(60) });
        fields.file = String::new();
        assert!(e.apply_fields(&f, &fields).is_err());
        let mut fields = Fields::of(&e.net.jobs[4]);
        fields.use_ = "nope".into();
        assert!(e.apply_fields(&n, &fields).is_err());
        fields.use_ = "g/m".into();
        assert!(e.apply_fields(&n, &fields).is_ok());
        assert_eq!(e.net.jobs[4].part, Part::Jobnet { use_: "g/m".into() });
        assert_eq!(e.net.jobs.len(), 5);
        assert!(e.report().starts_with("editor: a/b dirty=true parts=5"));
    }

    #[test]
    fn a_chore_from_the_shelf_takes_its_paths_and_refuses_a_root() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let blank = shelf().into_iter().find(|p| p.key() == "delete").unwrap();
        let d = e.add(blank, Some("a"), None);
        assert!(d.starts_with("delete"), "{d}");
        let mut fields = Fields::of(&e.net.jobs[1]);
        assert_eq!(fields.keep_days, "30");
        fields.path = "D:\\logs\\{date}".into();
        fields.pattern = "*.log".into();
        fields.retry_count = "2".into();
        fields.retry_wait = "1m".into();
        assert_eq!(e.apply_fields(&d, &fields), Ok(()));
        let job = &e.net.jobs[1];
        assert_eq!(job.part, Part::Chore(Chore::Delete { path: "D:\\logs\\{date}".into(), pattern: Some("*.log".into()), keep_days: 30 }));
        assert_eq!(job.retry, Some(Retry { count: 2, wait: 60 }));
        assert!(e.text().contains("kind = \"delete\""), "{}", e.text());
        fields.path = "D:\\".into();
        assert!(e.apply_fields(&d, &fields).is_err());
        fields.path = "D:\\logs".into();
        fields.keep_days = "0".into();
        assert!(e.apply_fields(&d, &fields).is_err());
    }

    #[test]
    fn an_editor_opens_from_a_def_and_saves_against_its_hash() {
        let text = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        let def = Def {
            id: "g/n".into(),
            group: "g".into(),
            name: "n".into(),
            net: Some(net(text)),
            sha256: approved::hash(text.as_bytes()),
            text: text.into(),
            title: None,
            file: "g/n.toml".into(),
            standing: Standing::Confirmed,
            problems: Vec::new(),
            previous: None,
            calendar: None,
        };
        let mut e = Editor::open(&def).unwrap();
        assert_eq!(e.base.as_deref(), Some(def.sha256.as_str()));
        assert_eq!(e.edit(Cmd::Save), None, "nothing to save yet");
        assert_eq!(e.edit(Cmd::Close), Some(Action::Close));
        e.add(Part::Notify, Some("a"), None);
        assert_eq!(e.edit(Cmd::Close), None);
        assert!(e.closing);
        assert_eq!(e.edit(Cmd::Save), Some(Action::Save { force: false }));
        let saved = e.text();
        e.saved(&saved, "ok".into());
        assert!(!e.dirty);
        assert_eq!(e.base, Some(approved::hash(saved.as_bytes())));
        assert_eq!(e.edit(Cmd::Save), None);
        let broken = Def { net: None, ..def.clone() };
        assert!(Editor::open(&broken).is_err());
    }
}
