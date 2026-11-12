//! The editor: the parts of a net as stations on a canvas, the lines between
//! them, a shelf of parts to drag in, a pane of fields for the chosen part
//! (or for the net itself), and undo over all of it.
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
use koyomi_core::chore::{Chore, Pass, SqlTls, Verify, WinrmAuth, GROUP_MAX};
use koyomi_core::jobnet::{self, Encoding, Job, Jobnet, Judge, Line, Missed, On, Overlap, Part, Retry, RunAs, Wait};
use koyomi_core::scan::{def_of_text, link, Def, Standing};
use koyomi_core::{approved, duration, layout, msg, when, write};

use super::graph;
use super::whens::WhenEdit;

mod drawer;

const RADIUS: f32 = 9.0;
/// Room before the first part, so its name (centred under it) is not cut at the edge.
const MARGIN: f32 = 64.0;
/// How far from the centre the out port sits.
const PORT: f32 = RADIUS + 7.0;
/// How close to a line a click has to be.
const NEAR: f32 = 6.0;
/// The wrap width of a part's note at zoom 1.
const NOTE_WRAP: f32 = 140.0;
/// How far the note's box reaches past its text.
const NOTE_PAD: f32 = 3.0;
/// The space between the circle and its name, and between the name and its note.
const LABEL_GAP: f32 = 4.0;
/// The radius of the out port's dot.
const PORT_DOT: f32 = 3.5;
/// How far inside the canvas the first view keeps the edge of what the parts take up.
const SEAT: f32 = 8.0;

/// What a part takes up on screen at zoom 1, from its centre: the circle, the out port, the name and the note.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Sides {
    left: f32,
    right: f32,
    up: f32,
    down: f32,
}

impl Sides {
    /// The same for `layout`, along and across the flow (`flat` swaps the axes for the vertical flow).
    fn room(self, vertical: bool) -> layout::Room {
        let (ab, aa, cb, ca) =
            if vertical { (self.up, self.down, self.left, self.right) } else { (self.left, self.right, self.up, self.down) };
        layout::Room { along_before: ab.into(), along_after: aa.into(), across_before: cb.into(), across_after: ca.into() }
    }

    /// What `job` takes up, drawn with these fonts at zoom 1.
    fn of(painter: &egui::Painter, job: &Job, name_font: &egui::FontId, small: &egui::FontId, vertical: bool) -> Sides {
        let color = egui::Color32::PLACEHOLDER;
        let name = job.name.clone().unwrap_or_else(|| job.id.clone());
        let label = painter.layout_no_wrap(name, name_font.clone(), color).size();
        let mut half = RADIUS.max(label.x / 2.0);
        let mut down = RADIUS + LABEL_GAP + label.y;
        if let Some(note) = job.note.as_deref().filter(|n| !n.is_empty()) {
            let galley = painter.layout(note.to_owned(), small.clone(), color, NOTE_WRAP);
            half = half.max(galley.size().x / 2.0 + NOTE_PAD);
            down += LABEL_GAP + galley.size().y + NOTE_PAD;
        }
        let mut s = Sides { left: half, right: half, up: RADIUS, down };
        let out = PORT + PORT_DOT + 1.0;
        if vertical {
            s.down = s.down.max(out);
        } else {
            s.right = s.right.max(out);
        }
        s
    }
}

/// `pan` moved right and down, and only so far, that the edge of what the parts take up (`edge`, at zoom 1 from the
/// first part's margin) is at least `SEAT` inside the canvas.
fn seated(pan: Vec2, edge: Vec2) -> Vec2 {
    let at = Vec2::splat(MARGIN) + edge + pan;
    pan + (Vec2::splat(SEAT) - at).max(Vec2::ZERO)
}

/// The mark of a part on the shelf, on the canvas and in the legend, drawn by ito so it follows the theme.
fn part_icon(key: &str) -> Option<ito_prefs::Mark> {
    use ito_prefs::Mark as M;
    Some(match key {
        "job" => M::Terminal,
        "loop" => M::Refresh,
        "jobnet" => M::Flow,
        "branch" => M::Branch,
        "join" => M::Merge,
        "time" => M::Clock,
        "file" => M::File,
        "notify" => M::Bell,
        "approval" => M::Person,
        "delete" => M::Trash,
        "zip" => M::Archive,
        "copy" => M::Copy,
        "empty" => M::Folder,
        "send" => M::Upload,
        "fetch" => M::Download,
        "http" => M::Globe,
        "ssh" => M::Server,
        "winrm" => M::Desktop,
        "mail" => M::Mail,
        "sql" => M::Database,
        _ => return None,
    })
}

/// The heading of the shelf's group a part is under; the shelf lists the parts group by group.
fn part_group(key: &str) -> &'static str {
    match key {
        "job" | "loop" | "jobnet" | "ssh" | "winrm" | "sql" => "canvas.group_basic",
        "branch" | "join" => "canvas.group_flow",
        "time" | "file" | "approval" => "canvas.group_wait",
        "notify" | "http" | "mail" => "canvas.group_tell",
        _ => "canvas.group_files",
    }
}

/// How far ahead the strip under the canvas shows the net's starts.
const STRIP_DAYS: u64 = 7;

/// What the editor reads from the engine's last snapshot besides its own net.
pub struct Around<'a> {
    pub calendars: &'a BTreeMap<String, Calendar>,
    pub power: &'a crate::proto::Power,
    /// The planned stops: the next starts step over them.
    pub stops: &'a [koyomi_core::blackout::Blackout],
    /// How many editors lie under this one (0: none, so no 親へ戻る).
    pub depth: usize,
    /// Every definition: 検査 links the net with them, and the 未確認 badge reads this one's.
    pub defs: &'a [Def],
    /// The definitions staged for a day: this net's shows as a button by the title.
    pub staged: &'a [koyomi_core::next::Entry],
    /// The engine's date and time now (not this PC's, when the window is on another PC's engine): every "today" of
    /// the editor, and where its next starts are counted from.
    pub now: chrono::NaiveDateTime,
    /// Every net as a choice (sorted), for the fields that name a net.
    pub nets: &'a [super::pick::NetChoice],
    /// The engine is this PC: a path can be browsed for.
    pub local: bool,
    /// The shells the engine has and its default one (empty until it has said): the shell field picks among them,
    /// and a new Job starts as a script job with the default.
    pub shells: &'a [crate::proto::Shell],
    pub shell: &'a str,
}

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
    /// Ask to stage the text from the day (YYYY-MM-DD).
    Stage(String),
    /// Open the box of what is staged for this net.
    Staged,
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
    /// An approval's `message`, as typed (empty: not written).
    pub message: String,
    /// A loop's `count` and `max`; its `file` is [`Fields::file`]. `max` is also how
    /// many PCs of a group an `ssh` or a `winrm` runs at once.
    pub count: String,
    pub max: String,
    pub use_: String,
    /// A chore's `path`, `match`, `keep_days`, `from` and `into`.
    pub path: String,
    pub pattern: String,
    pub keep_days: String,
    pub from: String,
    pub into: String,
    pub key: String,
    pub host_key: String,
    pub password: String,
    /// A `send`'s or a `fetch`'s S3 access key ID, region and S3-compatible server.
    pub key_id: String,
    pub region: String,
    pub endpoint: String,
    /// How a `send` or `fetch` checks each file; none: not at all.
    pub verify: Option<Verify>,
    /// An `http`'s call; `headers` one `Name: value` a line, `expect` the codes with `,`.
    pub url: String,
    pub method: String,
    pub body: String,
    pub content_type: String,
    pub headers: String,
    pub expect: String,
    /// An `ssh`'s or a `winrm`'s PC (`ssh://…`, `winrm://…`) and its command, as typed.
    pub on: String,
    pub command: String,
    /// A `winrm`'s certificate fingerprint and how it logs in.
    pub cert: String,
    pub auth: WinrmAuth,
    /// When an `ssh`'s or a `winrm`'s group of PCs passes.
    pub pass: Pass,
    /// A `mail`'s addresses and files, one a line; its text is [`Fields::body`].
    pub mail_to: String,
    pub mail_cc: String,
    pub subject: String,
    pub files: String,
    /// The `[[notify.mails]]` place it goes through; empty: the first.
    pub via: String,
    /// A `sql`'s database (`sqlserver://…`, `postgres://…`, `sqlite://…`) and its
    /// SQL (or the file with it, [`Fields::file`]); the CSV of its result, the
    /// rows that pass and how it talks to the server.
    pub db: String,
    pub sql: String,
    pub out: String,
    pub rows: String,
    pub tls: SqlTls,
    /// The `run_as` account, and the name of the secret with its password.
    pub run_as_user: String,
    pub run_as_secret: String,
    /// The name of a registered `[[account]]`; empty when the account is written out above.
    pub run_as_name: String,
    /// The name of a registered `[[database]]` a sql uses instead of `db`.
    pub sql_database: String,
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
        match &job.run_as {
            Some(RunAs::Named(n)) => f.run_as_name = n.clone(),
            Some(RunAs::Inline(r)) => {
                f.run_as_user = r.user.clone();
                f.run_as_secret = r.secret.clone();
            }
            None => {}
        }
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
            Part::Loop { file, count, max } => {
                f.file = file.clone().unwrap_or_default();
                f.count = count.map(|n| n.to_string()).unwrap_or_default();
                f.max = max.map(|n| n.to_string()).unwrap_or_default();
            }
            Part::Jobnet { use_ } => f.use_ = use_.clone(),
            Part::Approval { message } => f.message = message.clone().unwrap_or_default(),
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
                Chore::Send { from, into, key, host_key, password, key_id, region, endpoint, verify }
                | Chore::Fetch { from, into, key, host_key, password, key_id, region, endpoint, verify } => {
                    f.from = from.clone();
                    f.into = into.clone();
                    f.key = key.clone().unwrap_or_default();
                    f.host_key = host_key.clone().unwrap_or_default();
                    f.password = password.clone().unwrap_or_default();
                    f.key_id = key_id.clone().unwrap_or_default();
                    f.region = region.clone().unwrap_or_default();
                    f.endpoint = endpoint.clone().unwrap_or_default();
                    f.verify = *verify;
                }
                Chore::Http { url, method, body, content_type, headers, expect } => {
                    f.url = url.clone();
                    f.method = method.clone();
                    f.body = body.clone().unwrap_or_default();
                    f.content_type = content_type.clone().unwrap_or_default();
                    f.headers = headers.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(
                        "
",
                    );
                    f.expect = expect.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
                }
                Chore::Ssh { on, command, key, host_key, password, max, pass } => {
                    f.on = on.clone();
                    f.command = command.clone();
                    f.key = key.clone().unwrap_or_default();
                    f.host_key = host_key.clone().unwrap_or_default();
                    f.password = password.clone().unwrap_or_default();
                    f.max = max.map(|n| n.to_string()).unwrap_or_default();
                    f.pass = *pass;
                }
                Chore::Winrm { on, command, cert, password, auth, max, pass } => {
                    f.on = on.clone();
                    f.command = command.clone();
                    f.cert = cert.clone().unwrap_or_default();
                    f.password = password.clone().unwrap_or_default();
                    f.auth = *auth;
                    f.max = max.map(|n| n.to_string()).unwrap_or_default();
                    f.pass = *pass;
                }
                Chore::Mail { to, cc, subject, body, files, via } => {
                    f.mail_to = to.join("\n");
                    f.mail_cc = cc.join("\n");
                    f.subject = subject.clone();
                    f.body = body.clone().unwrap_or_default();
                    f.files = files.join("\n");
                    f.via = via.clone().unwrap_or_default();
                }
                Chore::Sql { db, database, password, sql, file, out, rows, tls } => {
                    f.db = db.clone();
                    f.sql_database = database.clone().unwrap_or_default();
                    f.password = password.clone().unwrap_or_default();
                    f.sql = sql.clone().unwrap_or_default();
                    f.file = file.clone().unwrap_or_default();
                    f.out = out.clone().unwrap_or_default();
                    f.rows = rows.clone().unwrap_or_default();
                    f.tls = *tls;
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
            return Err(msg::line(msg::WIN_JOB_ID_BAD, &[]));
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
            let count: u32 = self.retry_count.trim().parse().map_err(|_| msg::line(msg::DEF_INT_RANGE, &["retry.count", "1", "100"]))?;
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
                                .ok_or_else(|| msg::line(msg::DEF_INT_RANGE, &["count", "1", "10000"]))?,
                        ),
                    };
                    match (&file, &count) {
                        (Some(_), Some(_)) => return Err(msg::line(msg::DEF_LOOP_BOTH, &[])),
                        (None, None) => {
                            let shape = "file = 'C:\\in\\*.csv' / count = 3";
                            return Err(msg::line(msg::DEF_EITHER_REQUIRED, &["kind = \"loop\"", "file", "count", shape]));
                        }
                        _ => {}
                    }
                    let max = match self.max.trim() {
                        "" => None,
                        n => Some(
                            n.parse::<u32>()
                                .ok()
                                .filter(|c| (1..=jobnet::LOOP_MAX).contains(c))
                                .ok_or_else(|| msg::line(msg::DEF_INT_RANGE, &["max", "1", &jobnet::LOOP_MAX.to_string()]))?,
                        ),
                    };
                    out.part = Part::Loop { file, count, max };
                }
                out.run = self.run.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
                out.dir = opt(&self.dir);
                out.timeout = opt_secs(&self.timeout)?;
                out.retry = retry()?;
                out.shell = opt(&self.shell);
                out.script = if self.script.trim().is_empty() { None } else { Some(self.script.clone()) };
                out.encoding = self.encoding;
                out.run_as = self.run_as()?;
                out.judge = self.judge()?;
            }
            Part::Join { .. } => out.part = Part::Join { wait: self.wait },
            Part::Time { .. } => {
                let days: u32 = if self.days.trim().is_empty() {
                    0
                } else {
                    self.days.trim().parse().map_err(|_| msg::line(msg::DEF_INT_RANGE, &["days", "0", "366"]))?
                };
                out.part = Part::Time { at: when::time(&self.at)?, days };
            }
            Part::File { .. } => {
                if self.file.trim().is_empty() {
                    return Err(msg::line(msg::DEF_JOB_KEY_REQUIRED, &["file", "file", "file = 'C:\\in\\*.csv'"]));
                }
                out.part = Part::File { file: self.file.trim().into(), settle: opt_secs(&self.settle)? };
            }
            Part::Jobnet { .. } => {
                if !jobnet::is_id(self.use_.trim()) {
                    return Err(msg::line(msg::DEF_JOBNET_REF_BAD, &["use"]));
                }
                out.part = Part::Jobnet { use_: self.use_.trim().into() };
            }
            Part::Chore(c) => {
                let chore = match c {
                    Chore::Delete { .. } => {
                        let n = self.keep_days.trim();
                        let keep_days = n.parse::<u32>().map_err(|_| msg::line(msg::DEF_KEEP_DAYS_BAD, &[]))?;
                        Chore::Delete { path: self.path.trim().into(), pattern: opt(&self.pattern), keep_days }
                    }
                    Chore::Empty { .. } => Chore::Empty { path: self.path.trim().into() },
                    Chore::Zip { .. } => Chore::Zip { from: self.from.trim().into(), into: self.into.trim().into() },
                    Chore::Copy { .. } => {
                        Chore::Copy { from: self.from.trim().into(), into: self.into.trim().into(), pattern: opt(&self.pattern) }
                    }
                    Chore::Send { .. } => Chore::Send {
                        from: self.from.trim().into(),
                        into: self.into.trim().into(),
                        key: opt(&self.key),
                        host_key: opt(&self.host_key),
                        password: opt(&self.password),
                        key_id: opt(&self.key_id),
                        region: opt(&self.region),
                        endpoint: opt(&self.endpoint),
                        verify: self.verify,
                    },
                    Chore::Fetch { .. } => Chore::Fetch {
                        from: self.from.trim().into(),
                        into: self.into.trim().into(),
                        key: opt(&self.key),
                        host_key: opt(&self.host_key),
                        password: opt(&self.password),
                        key_id: opt(&self.key_id),
                        region: opt(&self.region),
                        endpoint: opt(&self.endpoint),
                        verify: self.verify,
                    },
                    Chore::Http { .. } => self.http()?,
                    Chore::Ssh { .. } => Chore::Ssh {
                        on: self.on.trim().into(),
                        command: self.command.clone(),
                        key: opt(&self.key),
                        host_key: opt(&self.host_key),
                        password: opt(&self.password),
                        max: self.group_max()?,
                        pass: self.pass,
                    },
                    Chore::Winrm { .. } => Chore::Winrm {
                        on: self.on.trim().into(),
                        command: self.command.clone(),
                        cert: opt(&self.cert),
                        password: opt(&self.password),
                        auth: self.auth,
                        max: self.group_max()?,
                        pass: self.pass,
                    },
                    Chore::Mail { .. } => Chore::Mail {
                        to: lines(&self.mail_to),
                        cc: lines(&self.mail_cc),
                        subject: self.subject.trim().into(),
                        body: if self.body.trim().is_empty() { None } else { Some(self.body.clone()) },
                        files: lines(&self.files),
                        via: opt(&self.via),
                    },
                    Chore::Sql { .. } => Chore::Sql {
                        db: self.db.trim().into(),
                        database: opt(&self.sql_database),
                        password: opt(&self.password),
                        sql: if self.sql.trim().is_empty() { None } else { Some(self.sql.clone()) },
                        file: opt(&self.file),
                        out: opt(&self.out),
                        rows: opt(&self.rows),
                        tls: self.tls,
                    },
                };
                chore.check()?;
                for p in chore.paths() {
                    koyomi_core::template::check(p)?;
                }
                if chore.far() {
                    out.judge = self.judge()?;
                }
                // A `mail` is sent as koyomi's own account, with its settings.
                let own = matches!(chore, Chore::Mail { .. });
                out.part = Part::Chore(chore);
                out.run_as = if own { None } else { self.run_as()? };
                out.timeout = opt_secs(&self.timeout)?;
                out.retry = retry()?;
            }
            Part::Approval { .. } => {
                let message = Some(self.message.clone()).filter(|m| !m.trim().is_empty());
                out.part = Part::Approval { message };
            }
            Part::Branch | Part::Notify => {}
        }
        Ok(out)
    }
}

impl Fields {
    /// The `run_as` the fields say: none when both are empty.
    fn run_as(&self) -> Result<Option<RunAs>, String> {
        const RUN_AS_SHAPE: &str = "{ user = 'CORP\\batch', password = \"secret:batch\" }";
        let (user, secret) = (self.run_as_user.trim(), self.run_as_secret.trim());
        let secret = secret.strip_prefix("secret:").unwrap_or(secret).trim();
        let name = self.run_as_name.trim();
        if !name.is_empty() {
            if !user.is_empty() || !secret.is_empty() {
                return Err(msg::line(msg::DEF_RUN_AS_NAME_AND_ACCOUNT, &[]));
            }
            if !koyomi_core::registry::valid_name(name) {
                return Err(msg::line(msg::DEF_REGISTERED_NAME_BAD, &["run_as", name]));
            }
            return Ok(Some(RunAs::Named(name.into())));
        }
        match (user.is_empty(), secret.is_empty()) {
            (true, true) => Ok(None),
            (false, true) => Err(msg::line(msg::DEF_SUBKEY_MISSING, &["run_as", "password", RUN_AS_SHAPE])),
            (true, false) => Err(msg::line(msg::DEF_SUBKEY_MISSING, &["run_as", "user", RUN_AS_SHAPE])),
            _ if !jobnet::valid_account(user) => Err(msg::line(msg::DEF_RUN_AS_USER_BAD, &[])),
            _ => Ok(Some(RunAs::Inline(jobnet::Account { user: user.into(), secret: secret.into() }))),
        }
    }

    /// The `judge` the fields say: none when they are all empty.
    fn judge(&self) -> Result<Option<Judge>, String> {
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
            ito_match::Matcher::new(p).map_err(|e| msg::line(msg::DEF_REGEX_BAD, &["judge.error_output", &e.to_string()]))?;
        }
        let ok_file = opt(&self.ok_file);
        if let Some(Err(e)) = ok_file.as_deref().map(koyomi_core::template::check) {
            return Err(e);
        }
        Ok(if warn.is_empty() && error.is_empty() && error_output.is_none() && ok_file.is_none() {
            None
        } else {
            Some(Judge { warn, error, error_output, ok_file })
        })
    }

    /// How many PCs of a group an `ssh` or a `winrm` runs at once; empty: one.
    fn group_max(&self) -> Result<Option<u32>, String> {
        match self.max.trim() {
            "" => Ok(None),
            n => n
                .parse::<u32>()
                .ok()
                .filter(|c| (1..=GROUP_MAX as u32).contains(c))
                .map(Some)
                .ok_or_else(|| msg::line(msg::DEF_INT_RANGE, &["max", "1", &GROUP_MAX.to_string()])),
        }
    }

    /// The `http` the fields say.
    fn http(&self) -> Result<Chore, String> {
        let mut headers = BTreeMap::new();
        for l in self.headers.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (k, v) = l.split_once(':').ok_or_else(|| msg::line(msg::WIN_HEADER_BAD, &[l]))?;
            headers.insert(k.trim().to_owned(), v.trim().to_owned());
        }
        let expect = self
            .expect
            .split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| c.parse::<u16>().map_err(|_| msg::line(msg::WIN_STATUS_CODE_BAD, &[c])))
            .collect::<Result<Vec<_>, _>>()?;
        let method = self.method.trim().to_ascii_uppercase();
        Ok(Chore::Http {
            url: self.url.trim().into(),
            method: if method.is_empty() { "GET".into() } else { method },
            // The body as it is: its spaces and lines may matter.
            body: Some(self.body.clone()).filter(|b| !b.trim().is_empty()),
            content_type: opt(&self.content_type),
            headers,
            expect,
        })
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

/// A list written one a line, without the empty lines.
fn lines(s: &str) -> Vec<String> {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
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
    /// The engine's default shell, as the last frame had it: what a new Job starts with (empty: none known).
    new_shell: String,
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
    /// What each part takes up on screen, by id, as last drawn.
    sides: BTreeMap<String, Sides>,
    /// The first view was seated (see [`seated`]); it happens once.
    seated: bool,
    /// The canvas as last drawn: its size and the flow, for `fit` and `jump`.
    view: Vec2,
    vertical: bool,
    drag: Drag,
    /// `c` mode: the target chosen so far.
    connect: Option<Option<String>>,
    menu: Option<Menu>,
    pane: Option<Pane>,
    /// The file chooser of the fields' pane, when one is open.
    browse: super::pick::Browse,
    /// The fields' pane is folded to its strip.
    fold: bool,
    clip: Vec<Job>,
    pub dirty: bool,
    problems: Vec<write::Found>,
    /// The engine said the file changed under us: the text we tried to save.
    pub conflict: Option<String>,
    /// Asked to close with changes unsaved.
    pub closing: bool,
    pub result: Option<Result<String, String>>,
    /// The 使い始める日 row, while open: its text.
    pub stage: Option<String>,
    /// Asked to stage from this day (the box over the editor).
    pub stage_ask: Option<String>,
}

/// The parts of the shelf, in order, group by group (see [`part_group`]).
pub fn shelf() -> [Part; 20] {
    [
        Part::Job,
        Part::Loop { file: None, count: Some(2), max: None },
        Part::Jobnet { use_: String::new() },
        Part::Chore(Chore::blank("ssh").unwrap()),
        Part::Chore(Chore::blank("winrm").unwrap()),
        Part::Chore(Chore::blank("sql").unwrap()),
        Part::Branch,
        Part::Join { wait: Wait::All },
        Part::Time { at: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(), days: 0 },
        Part::File { file: String::new(), settle: None },
        Part::Approval { message: None },
        Part::Chore(Chore::blank("delete").unwrap()),
        Part::Chore(Chore::blank("zip").unwrap()),
        Part::Chore(Chore::blank("copy").unwrap()),
        Part::Chore(Chore::blank("empty").unwrap()),
        Part::Chore(Chore::blank("send").unwrap()),
        Part::Chore(Chore::blank("fetch").unwrap()),
        Part::Notify,
        Part::Chore(Chore::blank("http").unwrap()),
        Part::Chore(Chore::blank("mail").unwrap()),
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
        Part::Approval { .. } => "approval",
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

/// Where the polyline `points` first lies `r` from `c`, as a length along it (`None` when it never does).
fn leaves(points: &[Pos2], c: Pos2, r: f32) -> Option<f32> {
    let mut walked = 0.0;
    if points.first().is_some_and(|p| (*p - c).length() >= r) {
        return Some(0.0);
    }
    for w in points.windows(2) {
        let (d, seg) = (w[1] - w[0], (w[1] - w[0]).length());
        if (w[1] - c).length() >= r && seg > 0.0 {
            // |w0 + t d - c|^2 = r^2 for t in 0..=1, the later root.
            let o = w[0] - c;
            let (a, b, k) = (d.length_sq(), 2.0 * o.dot(d), o.length_sq() - r * r);
            let t = ((-b + (b * b - 4.0 * a * k).max(0.0).sqrt()) / (2.0 * a)).clamp(0.0, 1.0);
            return Some(walked + seg * t);
        }
        walked += seg;
    }
    None
}

/// The line from the part at `from` to the one at `to`, as the run picture draws it, cut where it
/// leaves the first circle and where it enters the second, less the gap for the arrow head.
/// `r` is the parts' radius as drawn (it grows with the zoom).
/// `None` when the parts touch or the curve is too short to show.
fn line_path(from: Pos2, to: Pos2, vertical: bool, r: f32) -> Option<Vec<Pos2>> {
    if (to - from).length() < r * 2.0 + 2.0 {
        return None;
    }
    let curve = graph::path(from, to, vertical);
    let total = graph::length(&curve);
    let start = leaves(&curve, from, r)?;
    let back: Vec<Pos2> = curve.iter().rev().copied().collect();
    let end = total - leaves(&back, to, r)? - 2.0;
    if end - start < 1.0 {
        return None;
    }
    let cut = graph::piece(&curve, start, end);
    (cut.len() >= 2).then_some(cut)
}

/// The point at length `d` along `points`, and the direction there.
fn along(points: &[Pos2], d: f32) -> (Pos2, Vec2) {
    let mut walked = 0.0;
    let mut last = (points.first().copied().unwrap_or(Pos2::ZERO), Vec2::X);
    for w in points.windows(2) {
        let seg = (w[1] - w[0]).length();
        if seg <= 0.0 {
            continue;
        }
        let dir = (w[1] - w[0]) / seg;
        last = (w[1], dir);
        if walked + seg >= d {
            return (w[0] + dir * (d - walked), dir);
        }
        walked += seg;
    }
    last
}

/// Colour, width, dashed and mark of a line kind.
fn line_look(on: &On, c: &ito_theme::Colors) -> (egui::Color32, f32, bool, String) {
    match on {
        On::Ok => (c.done, 2.0, false, Mark::Done.mark().to_owned()),
        On::Error => (c.err, 2.0, true, Mark::Error.mark().to_owned()),
        On::Any => (c.dim, 1.5, false, "*".to_owned()),
        On::Code(r) => (c.wait, 2.0, false, Judge::list(r)),
        On::Else => (c.dim, 1.5, true, "else".to_owned()),
    }
}

/// The distance from `p` to the polyline.
fn dist_poly(p: Pos2, points: &[Pos2]) -> f32 {
    points.windows(2).map(|w| dist(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
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
        // With no part opened, the pane shows the net's fields.
        let pane = Some(Pane::Net(Box::new(NetFields::of(&net))));
        Editor {
            id: id.into(),
            new_shell: String::new(),
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
            sides: BTreeMap::new(),
            seated: false,
            view: Vec2::ZERO,
            vertical: false,
            drag: Drag::None,
            connect: None,
            menu: None,
            pane,
            browse: super::pick::Browse::default(),
            fold: false,
            clip: Vec::new(),
            dirty: false,
            problems,
            conflict: None,
            closing: false,
            result: None,
            stage: None,
            stage_ask: None,
        }
    }

    /// The file this would save.
    pub fn text(&self) -> String {
        write::text(&self.net)
    }

    /// A file chooser is open, for a test.
    #[cfg(test)]
    pub fn hold_chooser(&mut self) {
        self.browse = super::pick::Browse::pending("n\u{1}when.file-0");
    }

    /// Saved now: changed, sound, and not being staged for a day instead.
    pub fn can_save(&self) -> bool {
        self.dirty && self.problems.is_empty() && self.stage.is_none()
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
            e.fold = self.fold;
            *self = e;
        }
    }

    /// A staged definition loaded to change (undo goes back), the day row open on its day.
    pub fn take_next(&mut self, text: &str, from: &str) -> Result<(), String> {
        let net = jobnet::parse(text).map_err(|p| p.iter().map(|p| p.message.clone()).collect::<Vec<_>>().join("\n"))?;
        self.record(None);
        self.net = net;
        self.after_jump(None);
        self.stage = Some(from.to_owned());
        Ok(())
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
        let spaced = layout::spaced(&self.net.jobs, &self.rooms());
        self.net.jobs.iter().zip(spaced).map(|(j, s)| j.pos.unwrap_or(s)).collect()
    }

    /// What each part takes up along and across the current flow (nothing, before it was measured).
    fn rooms(&self) -> Vec<layout::Room> {
        self.net.jobs.iter().map(|j| self.sides.get(&j.id).copied().unwrap_or_default().room(self.vertical)).collect()
    }

    /// Adds `part` after `after` (connected by a 正常 line) or at `at`; the new part is the cursor. With neither it starts
    /// a flow of its own: in the first column, under every part there is.
    pub fn add(&mut self, part: Part, after: Option<&str>, at: Option<(f64, f64)>) -> String {
        self.record(None);
        let id = self.fresh_id(prefix(&part));
        let positions = self.positions();
        let pos = at.or_else(|| match after.and_then(|a| self.index(a)) {
            Some(i) => {
                let (x, y) = positions[i];
                let taken = positions.iter().filter(|(px, _)| (px - x - layout::STEP_ALONG).abs() < 1.0).count();
                Some((x + layout::STEP_ALONG, y + taken as f64 * layout::STEP_ACROSS))
            }
            None => {
                let left = positions.iter().map(|p| p.0).reduce(f64::min)?;
                let low = positions.iter().map(|p| p.1).reduce(f64::max)?;
                Some((left, low + layout::STEP_ACROSS))
            }
        });
        // With the engine's default shell known a new Job is a script job; before that (or from an older engine) it is
        // the old `cmd.exe` one.
        let script = part == Part::Job && !self.new_shell.is_empty();
        let run = if part == Part::Job && !script { vec!["cmd.exe".into(), "/c".into(), "echo hello".into()] } else { Vec::new() };
        let (shell, script) = if script { (Some(self.new_shell.clone()), Some("echo hello".to_owned())) } else { (None, None) };
        let after = after.filter(|a| self.has(a)).map(|a| vec![Line::ok(a)]).unwrap_or_default();
        self.net.jobs.push(Job { id: id.clone(), run, after, pos, part, shell, script, ..Default::default() });
        self.choose(id.clone());
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
                self.pane = Some(Pane::Net(Box::new(NetFields::of(&self.net))));
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
        if let Some(id) = self.cursor.clone() {
            self.follow(&id);
        }
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
            Some(Pane::Part(id, _)) => self
                .index(&id)
                .map(|i| Pane::Part(id, Box::new(Fields::of(&self.net.jobs[i]))))
                .or_else(|| Some(Pane::Net(Box::new(NetFields::of(&self.net))))),
            Some(Pane::Net(_)) => Some(Pane::Net(Box::new(NetFields::of(&self.net)))),
            None => None,
        };
    }

    pub fn align(&mut self) {
        self.record(None);
        let rooms = self.rooms();
        layout::align_in(&mut self.net.jobs, &rooms);
        self.touched();
    }

    pub fn select_all(&mut self) {
        self.selected = self.net.jobs.iter().map(|j| j.id.clone()).collect();
    }

    /// Nothing chosen and no cursor, so the next part from the shelf (or `a`) starts a flow of its own.
    pub fn clear_target(&mut self) {
        self.selected.clear();
        self.cursor = None;
    }

    /// The part a click on the shelf adds after, when it is still there.
    fn target(&self) -> Option<&Job> {
        self.cursor.as_deref().and_then(|id| self.index(id)).map(|i| &self.net.jobs[i])
    }

    /// Opens the fields of `id`: it is chosen, and the pane unfolds.
    pub fn open_part(&mut self, id: &str) {
        if self.has(id) {
            self.choose(id.into());
            self.fold = false;
        }
    }

    /// `id` alone is chosen and is the cursor; the pane shows its fields.
    fn choose(&mut self, id: String) {
        self.follow(&id);
        self.selected = BTreeSet::from([id.clone()]);
        self.cursor = Some(id);
    }

    /// The pane shows the fields of `id`, folded or open as it was. The fields of the part already there stay as
    /// they are, with what is being typed in them.
    fn follow(&mut self, id: &str) {
        if matches!(&self.pane, Some(Pane::Part(p, _)) if p == id) {
            return;
        }
        if let Some(i) = self.index(id) {
            self.pane = Some(Pane::Part(id.into(), Box::new(Fields::of(&self.net.jobs[i]))));
        }
    }

    /// Nothing is chosen any more: the pane shows the net's fields instead of a part's.
    fn follow_net(&mut self) {
        if matches!(self.pane, Some(Pane::Part(..))) {
            self.pane = Some(Pane::Net(Box::new(NetFields::of(&self.net))));
        }
    }

    /// Shows the net's fields in the pane (the part's are closed).
    pub fn open_net(&mut self) {
        self.pane = Some(Pane::Net(Box::new(NetFields::of(&self.net))));
        self.fold = false;
    }

    /// Puts `fields` into the part `id` (renaming it when the id changed); the reason when they do not read.
    pub fn apply_fields(&mut self, id: &str, fields: &Fields) -> Result<(), String> {
        let Some(i) = self.index(id) else { return Err(msg::line(msg::WIN_JOB_GONE, &[id])) };
        let job = fields.apply(&self.net.jobs[i])?;
        if job.id != id && self.has(&job.id) {
            return Err(msg::line(msg::DEF_ID_DUPLICATE, &[&job.id]));
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

    /// Each part's centre at zoom 1 (without the margin) and what it takes up.
    fn extents(&self) -> Vec<(Vec2, Sides)> {
        let margin = Vec2::splat(MARGIN);
        let at = self.positions();
        self.net.jobs.iter().zip(at).map(|(j, p)| (self.flat(p) - margin, self.sides.get(&j.id).copied().unwrap_or_default())).collect()
    }

    /// The edges of what the parts take up at `zoom`: centres scale, the text around them does not.
    fn bounds(items: &[(Vec2, Sides)], zoom: f32) -> (Vec2, Vec2) {
        let mut lo = Vec2::splat(f32::MAX);
        let mut hi = Vec2::splat(f32::MIN);
        for (c, s) in items {
            let c = *c * zoom;
            lo = Vec2::new(lo.x.min(c.x - s.left), lo.y.min(c.y - s.up));
            hi = Vec2::new(hi.x.max(c.x + s.right), hi.y.max(c.y + s.down));
        }
        (lo, hi)
    }

    /// Zooms and pans so every part, with its name and note, is in view (nothing to do before the first draw or with no parts).
    pub fn fit(&mut self) {
        let items = self.extents();
        if items.is_empty() || self.view.x < 1.0 || self.view.y < 1.0 {
            return;
        }
        let pad = 48.0;
        let room = self.view - Vec2::splat(2.0 * pad);
        let fits = |z: f32| {
            let (lo, hi) = Self::bounds(&items, z);
            hi.x - lo.x <= room.x && hi.y - lo.y <= room.y
        };
        // The largest zoom in 0.3..=1.5 that fits (the extent is convex in the zoom, so what fits is one stretch).
        let zoom = if fits(1.5) {
            1.5
        } else {
            let (mut good, mut bad) = (0.3f32, 1.5f32);
            for _ in 0..24 {
                let mid = (good + bad) / 2.0;
                if fits(mid) {
                    good = mid;
                } else {
                    bad = mid;
                }
            }
            good
        };
        self.zoom = zoom;
        // The middle of what the parts take up goes to the middle of the canvas.
        let (lo, hi) = Self::bounds(&items, zoom);
        self.pan = self.view / 2.0 - Vec2::splat(MARGIN) - (lo + hi) / 2.0;
    }

    /// 検査: what the engine's save would refuse in the text, without saving it (empty: nothing).
    pub fn check(&self, defs: &[Def], calendars: &BTreeMap<String, Calendar>) -> Vec<String> {
        let Some((group, name)) = self.id.split_once('/').filter(|_| jobnet::is_id(&self.id)) else {
            return vec![msg::line(msg::DEF_JOBNET_REF_BAD, &[&format!("\"{}\"", self.id)])];
        };
        // The window does not read the engine's `_group.toml` (the engine may be on another PC):
        // a net of the group that names no calendar of its own counts by the group's.
        let calendar = defs
            .iter()
            .filter(|d| d.group == group)
            .find(|d| d.net.as_ref().is_some_and(|n| n.calendar.is_none()))
            .and_then(|d| d.calendar.clone());
        let shared = koyomi_core::group::Group { calendar };
        let cals = koyomi_core::calendar::Calendars { map: calendars.clone(), ..Default::default() };
        let def = def_of_text(&self.text(), group, name, &approved::Store::default(), Ok(&shared), &cals);
        let mut all: Vec<Def> = defs.iter().filter(|d| d.id != self.id).cloned().collect();
        all.push(def);
        link(&mut all);
        let def = all.pop().expect("pushed");
        def.problems.iter().map(|p| p.in_file(&def.file)).collect()
    }

    /// When the net as edited starts in the `days` days from `from` (the strip under the canvas).
    pub fn starts(&self, from: chrono::NaiveDateTime, days: u64, cal: Option<&Calendar>) -> Vec<chrono::NaiveDateTime> {
        let until = from + chrono::Days::new(days);
        let times = when::between_in(&self.net.when, from, until, cal);
        koyomi_core::quiet::starts(&self.net.quiet, self.net.on_quiet, times).into_iter().filter(|s| *s >= from && *s < until).collect()
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
        let p = *at.get(i)?;
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
        best.and_then(|(_, k)| self.net.jobs.get(k)).map(|j| j.id.clone())
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

    /// The part under `p`: only one the net still has, whatever `at` was taken from.
    fn hit_part(&self, at: &[Pos2], p: Pos2) -> Option<usize> {
        (0..at.len().min(self.net.jobs.len())).rev().find(|&i| (at[i] - p).length() <= RADIUS + 4.0)
    }

    fn hit_port(&self, at: &[Pos2], p: Pos2, vertical: bool) -> Option<usize> {
        (0..at.len()).rev().find(|&i| (Self::port(at[i], vertical) - p).length() <= 6.0)
    }

    /// A part's radius on the screen: it grows with the zoom, and does not shrink below 0.6 of it.
    fn part_radius(&self) -> f32 {
        RADIUS * self.zoom.max(0.6)
    }

    /// `(job index, line index)` of the line under `p`.
    fn hit_line(&self, at: &[Pos2], p: Pos2) -> Option<(usize, usize)> {
        for (i, j) in self.net.jobs.iter().enumerate() {
            for (k, l) in j.after.iter().enumerate() {
                if let Some((&from, &to)) = self.index(&l.job).and_then(|f| at.get(f).zip(at.get(i))) {
                    if line_path(from, to, self.vertical, self.part_radius()).is_some_and(|path| dist_poly(p, &path) <= NEAR) {
                        return Some((i, k));
                    }
                }
            }
        }
        None
    }

    /// The editor: toolbar, shelf, fields and the canvas.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &Table, vertical: bool, around: &Around) -> Option<Action> {
        self.new_shell = around.shell.to_owned();
        let (calendars, power, depth) = (around.calendars, around.power, around.depth);
        let mine = around.defs.iter().find(|d| d.id == self.id);
        let mut action = None;
        egui::Panel::top("editor-bar").show(ui, |ui| {
            use ito_prefs::{IconButton, Mark as M, Tone};
            // The first row says where the editor is and how to leave it; the second holds the tools.
            ui.horizontal(|ui| {
                if depth > 0 && IconButton::new(M::ChevronLeft, t.tr("canvas.back")).show(ui).clicked() {
                    if let Some(a) = self.edit(Cmd::Close) {
                        action = Some(a);
                    }
                }
                let title = format!(
                    "{} {}{}{}",
                    t.tr("canvas.title"),
                    self.id,
                    if self.base.is_none() { t.tr("canvas.new") } else { "" },
                    if self.dirty { " *" } else { "" }
                );
                let r = ui.heading(&title);
                ito_a11y::say(&r, Role::Heading, &title);
                if self.base.is_some() && mine.is_some_and(|d| d.standing == Standing::Unconfirmed) {
                    let c = ito_theme::colors();
                    let badge = t.tr("canvas.unconfirmed");
                    let r = ui.label(egui::RichText::new(badge).color(c.wait_text()).background_color(c.wait_bg()));
                    ito_a11y::say(&r, Role::Label, badge);
                    r.on_hover_text(t.tr("canvas.unconfirmed_hint"));
                }
                if let Some(next) = around.staged.iter().find(|n| n.id == self.id) {
                    let words = t.trf("canvas.staged", &[&next.from.to_string()]);
                    if IconButton::new(M::Calendar, &words).show(ui).on_hover_text(t.tr("canvas.staged_hint")).clicked() {
                        action = Some(Action::Staged);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if IconButton::new(M::Close, t.tr("canvas.close")).show(ui).clicked() {
                        if let Some(a) = self.edit(Cmd::Close) {
                            action = Some(a);
                        }
                    }
                    if IconButton::new(M::Settings, t.tr("canvas.net")).show(ui).clicked() {
                        self.open_net();
                    }
                    // Inside a sub-net, back to its parent from the right as well.
                    if depth > 0 && IconButton::new(M::ChevronLeft, t.tr("canvas.back")).show(ui).clicked() {
                        if let Some(a) = self.edit(Cmd::Close) {
                            action = Some(a);
                        }
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                let can = self.can_save();
                let save = IconButton::new(M::Save, t.tr("canvas.save")).tone(Tone::Primary);
                let mut r = ui.add_enabled(can, |ui: &mut egui::Ui| save.show(ui));
                if self.stage.is_some() {
                    r = r.on_disabled_hover_text(t.tr("canvas.stage_save"));
                }
                if r.clicked() {
                    action = self.edit(Cmd::Save);
                }
                if IconButton::new(M::Search, t.tr("canvas.check")).show(ui).on_hover_text(t.tr("canvas.check_hint")).clicked() {
                    let found = self.check(around.defs, calendars);
                    self.result = Some(if found.is_empty() {
                        Ok(t.tr("canvas.check_ok").to_owned())
                    } else {
                        Err(format!(
                            "{}
{}",
                            t.tr("canvas.check_bad"),
                            found.join(
                                "
"
                            )
                        ))
                    });
                }
                // A day from which the net as drawn here takes over (it runs as it is till then).
                let staging = self.stage.is_some();
                let r = ui
                    .add_enabled(self.base.is_some(), |ui: &mut egui::Ui| IconButton::new(M::Calendar, t.tr("canvas.stage")).show(ui))
                    .on_hover_text(t.tr("canvas.stage_hint"));
                if r.clicked() {
                    self.stage = if staging {
                        None
                    } else {
                        // The first of next month: where a switch most often falls.
                        let today = around.now.date();
                        let first = chrono::Datelike::with_day(&today, 1).and_then(|d| d.checked_add_months(chrono::Months::new(1)));
                        Some(first.map(ito_prefs::format_date).unwrap_or_default())
                    };
                }
                ui.separator();
                let groups: [&[(Cmd, M, &str)]; 2] = [
                    &[(Cmd::Undo, M::Undo, "canvas.undo"), (Cmd::Redo, M::Redo, "canvas.redo")],
                    &[(Cmd::Align, M::Arrange, "canvas.align"), (Cmd::Fit, M::Expand, "canvas.fit")],
                ];
                for group in groups {
                    for &(cmd, mark, key) in group {
                        let can = match cmd {
                            Cmd::Undo => !self.undo.is_empty(),
                            Cmd::Redo => !self.redo.is_empty(),
                            _ => true,
                        };
                        if ui.add_enabled(can, |ui: &mut egui::Ui| IconButton::new(mark, t.tr(key)).show(ui)).clicked() {
                            self.edit(cmd);
                        }
                    }
                    ui.separator();
                }
                // The way the flow would turn to; the mark is the arrow, so the words carry none.
                let (flow, mark) =
                    if vertical { ("canvas.flow_horizontal", M::ArrowRight) } else { ("canvas.flow_vertical", M::ArrowDown) };
                if IconButton::new(mark, t.tr(flow)).show(ui).clicked() {
                    action = Some(Action::Flow);
                }
            });
            let c = ito_theme::colors();
            if let Some(mut day) = self.stage.clone() {
                let (mut go, mut cancel) = (false, false);
                ui.horizontal_wrapped(|ui| {
                    let today = around.now.date();
                    let label = t.tr("canvas.stage_from");
                    ui.label(label);
                    let d = ito_prefs::DateField::new(("stage", &self.id), ito_prefs::DateWords::for_lang(t.tr("app.lang")))
                        .name(label)
                        .range(today.succ_opt(), None)
                        .width(130.0)
                        .show(ui, ito_prefs::Look { c, q: "" }, &mut day);
                    let can = d.valid && self.dirty && self.problems.is_empty();
                    let enter = d.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let put = IconButton::new(M::CalendarCheck, t.tr("canvas.stage_go")).tone(Tone::Primary);
                    go = ui.add_enabled(can, |ui: &mut egui::Ui| put.show(ui)).clicked() || (can && enter);
                    cancel = IconButton::new(M::Close, t.tr("canvas.cancel")).show(ui).clicked();
                    if !can {
                        let why = if !d.valid {
                            "canvas.stage_bad_day"
                        } else if !self.dirty {
                            "canvas.stage_same"
                        } else {
                            "canvas.stage_fix"
                        };
                        ui.label(egui::RichText::new(t.tr(why)).color(c.dim));
                    }
                });
                self.stage = if cancel { None } else { Some(day.clone()) };
                if go {
                    action = Some(Action::Stage(ito_prefs::parse_date(&day).map(ito_prefs::format_date).unwrap_or(day)));
                }
            }
            if !self.problems.is_empty() {
                let head = &koyomi_core::msg::say(koyomi_core::msg::EDITOR_PROBLEMS, &[]).short();
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
        egui::Panel::left("shelf").default_size(180.0).size_range(120.0..=320.0).resizable(true).show(ui, |ui| {
            let c = ito_theme::colors();
            let r = ui.label(egui::RichText::new(t.tr("canvas.shelf")).strong());
            ito_a11y::say(&r, Role::Heading, t.tr("canvas.shelf"));
            ui.add_space(4.0);
            // Where a click on a part puts it: after the cursor, or at the start of a flow of its own.
            let target = self.target().map(|j| j.name.clone().unwrap_or_else(|| j.id.clone()));
            let mut clear = false;
            super::card(&c).show(ui, |ui| {
                ui.set_width(ui.available_width());
                // The heading row carries the clear button, so the target below has the whole width.
                ui.horizontal(|ui| {
                    let r = ui.label(egui::RichText::new(t.tr("canvas.target")).small().color(c.dim));
                    ito_a11y::say(&r, Role::Heading, t.tr("canvas.target"));
                    if target.is_some() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            clear = ito_prefs::mark_button(ui, ito_prefs::Mark::Close, t.tr("canvas.target_clear"), 18.0).clicked();
                        });
                    }
                });
                match &target {
                    Some(name) => {
                        let text = t.trf("canvas.target_after", &[name]);
                        let r = ui.label(egui::RichText::new(&text).strong());
                        ito_a11y::say(&r, Role::Label, &text);
                    }
                    None => {
                        let r = ui.label(egui::RichText::new(t.tr("canvas.target_start")).color(c.dim));
                        ito_a11y::say(&r, Role::Label, t.tr("canvas.target_start"));
                    }
                }
            });
            if clear {
                self.clear_target();
            }
            // How the two ways of adding differ, where the eye is before it reaches the parts.
            ui.add_space(2.0);
            for key in ["canvas.hint_drag", "canvas.hint_click"] {
                let r = ui.label(egui::RichText::new(t.tr(key)).small().color(c.dim));
                ito_a11y::say(&r, Role::Label, t.tr(key));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut group = "";
                for part in shelf() {
                    let head = part_group(part.key());
                    if head != group {
                        group = head;
                        ui.add_space(6.0);
                        let r = ui.label(egui::RichText::new(t.tr(head)).small().color(c.dim));
                        ito_a11y::say(&r, Role::Heading, t.tr(head));
                    }
                    let key = format!("part.{}", part.key());
                    let name = t.tr(&key);
                    let icon = part_icon(part.key()).unwrap_or(ito_prefs::Mark::Terminal);
                    let r = ito_prefs::IconButton::new(icon, name).wide().drag().show(ui);
                    if r.drag_started() {
                        self.drag = Drag::Shelf(part.clone());
                    } else if r.clicked() {
                        let after = self.cursor.clone();
                        self.add(part.clone(), after.as_deref(), None);
                    }
                }
            });
        });
        self.drawer(
            ui,
            t,
            (calendars, around.stops, around.now),
            (around.nets, around.local, (around.shells, around.shell)),
            power,
            &mut action,
        );
        egui::Panel::bottom("editor-foot").show(ui, |ui| {
            let cal = self.net.calendar.as_deref().or(mine.and_then(|d| d.calendar.as_deref())).and_then(|n| calendars.get(n));
            self.foot(ui, t, cal, around.now);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(a) = self.canvas(ui, t, vertical) {
                action = Some(a);
            }
        });
        action
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
        let small = egui::TextStyle::Small.resolve(ui.style());
        let name_font = egui::TextStyle::Body.resolve(ui.style());
        self.sides = self.net.jobs.iter().map(|j| (j.id.clone(), Sides::of(&painter, j, &name_font, &small, vertical))).collect();
        // The first view: the edge of the first part's name or note is not cut at the canvas's left or top.
        if !self.seated && self.view.x >= 1.0 && self.view.y >= 1.0 {
            self.seated = true;
            if self.pan == Vec2::ZERO && self.zoom == 1.0 {
                let (lo, _) = Self::bounds(&self.extents(), 1.0);
                if lo.x != f32::MAX {
                    self.pan = seated(self.pan, lo);
                }
            }
        }
        // Where each part is on the screen, by its index in the net. A part added or removed in this frame
        // (the shelf, the keys) makes it stale, so it is taken again after those.
        let place = |e: &Self| -> Vec<Pos2> { e.positions().into_iter().map(|p| e.to_screen(origin, p, vertical)).collect() };
        let mut at = place(self);
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
                    self.follow(&id);
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
                        for (j, q) in self.net.jobs.iter().zip(&at) {
                            if r.contains(*q) {
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
        at = place(self);
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
                        self.follow(&id);
                        self.cursor = Some(id);
                    } else if !matches!(self.drag, Drag::Parts { moved: true }) {
                        // A click chooses the part, and the pane shows its fields.
                        self.choose(id);
                    }
                } else if !shift && !command {
                    self.clear_target();
                    self.follow_net();
                    self.connect = None;
                }
            }
        }
        if resp.double_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                match self.hit_part(&at, p) {
                    // `hit_part` gives only a part the net still has.
                    Some(i) if i < self.net.jobs.len() => {
                        let id = self.net.jobs[i].id.clone();
                        if let Part::Jobnet { use_ } = &self.net.jobs[i].part {
                            if jobnet::is_id(use_) {
                                action = Some(Action::Enter(use_.clone()));
                            }
                        }
                        self.open_part(&id);
                    }
                    _ => self.open_net(),
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
                    self.follow(&id);
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
            // `a`, a paste, Delete, undo and redo add or remove parts.
            at = place(self);
        }

        // The lines.
        for (i, j) in self.net.jobs.iter().enumerate() {
            for l in &j.after {
                let Some(f) = self.index(&l.job) else { continue };
                let Some(path) = line_path(at[f], at[i], vertical, self.part_radius()) else { continue };
                let (col, width, dash, mark) = line_look(&l.on, &c);
                let stroke = Stroke::new(width * self.zoom.max(0.6), col);
                if dash {
                    graph::dashed(&painter, &path, stroke, 8.0, 6.0, 0.0);
                } else {
                    painter.add(egui::Shape::line(path.clone(), stroke));
                }
                // The head, along the curve's last direction.
                let len = graph::length(&path);
                let (b, dir) = along(&path, len);
                let side = Vec2::new(-dir.y, dir.x);
                painter.line_segment([b, b - dir * 7.0 + side * 4.0], stroke);
                painter.line_segment([b, b - dir * 7.0 - side * 4.0], stroke);
                let (mid, dir) = along(&path, len * 0.5);
                painter.text(mid + Vec2::new(-dir.y, dir.x) * 8.0, egui::Align2::CENTER_CENTER, mark, small.clone(), col);
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
        for (i, j) in self.net.jobs.iter().enumerate() {
            let p = at[i];
            let r = self.part_radius();
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
            // A job is the plain circle; every other part shows its mark from the shelf.
            if let Some(m) = part_icon(j.part.key()).filter(|_| !j.part.is_job()) {
                ito_prefs::mark(&painter, Rect::from_center_size(p, Vec2::splat(r * 1.1)), m, if selected { c.bg } else { c.fg });
            }
            if bad.contains(j.id.as_str()) {
                painter.text(p + Vec2::new(r + 2.0, -r - 2.0), egui::Align2::LEFT_BOTTOM, Mark::Error.mark(), small.clone(), c.err);
            }
            // The out port.
            let port = Self::port(p, vertical);
            let hot = pointer.is_some_and(|q| (port - q).length() <= 6.0);
            painter.circle(port, PORT_DOT, if hot { accent } else { c.border }, Stroke::new(1.0, c.fg));
            let name = j.name.clone().unwrap_or_else(|| j.id.clone());
            let label = if selected { Mark::Selected.marked(&name) } else { name };
            let label_rect = painter.text(
                p + Vec2::new(0.0, r + LABEL_GAP),
                egui::Align2::CENTER_TOP,
                label,
                name_font.clone(),
                if selected { accent } else { c.fg },
            );
            if let Some(note) = j.note.as_deref().filter(|n| !n.is_empty()) {
                let galley = painter.layout(note.to_owned(), small.clone(), c.wait_text(), NOTE_WRAP * self.zoom.max(0.6));
                let top = label_rect.center_bottom() + Vec2::new(0.0, LABEL_GAP);
                let box_rect = Rect::from_min_size(top - Vec2::new(galley.size().x / 2.0, 0.0), galley.size()).expand(NOTE_PAD);
                painter.rect(box_rect, 2.0, c.wait_bg(), Stroke::new(1.0, c.border), StrokeKind::Inside);
                painter.galley(box_rect.min + Vec2::splat(NOTE_PAD), galley, c.wait_text());
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
        self.minimap(ui, t, &painter, rect);
        self.show_menu(ui, t);
        action
    }

    /// Pans so the model point `m` (flat, at zoom 1, without the margin) is in the middle of the canvas.
    fn center_on(&mut self, m: Vec2) {
        self.pan = self.view / 2.0 - Vec2::splat(MARGIN) - m * self.zoom;
    }

    /// The minimap's lines, one for each `after` entry: the curve through `to_mini`, its colour and
    /// whether it is dashed. A curve under 2 px long is left out.
    fn minimap_lines(&self, pts: &[Pos2], to_mini: impl Fn(Pos2) -> Pos2, c: &ito_theme::Colors) -> Vec<(Vec<Pos2>, egui::Color32, bool)> {
        let mut out = Vec::new();
        for (i, j) in self.net.jobs.iter().enumerate() {
            for l in &j.after {
                let Some((&from, &to)) = self.index(&l.job).and_then(|f| pts.get(f).zip(pts.get(i))) else { continue };
                let path = graph::path(to_mini(from), to_mini(to), self.vertical);
                if graph::length(&path) < 2.0 {
                    continue;
                }
                let (col, _, dash, _) = line_look(&l.on, c);
                out.push((path, col, dash));
            }
        }
        out
    }

    /// The whole net, small, in the canvas's corner: a dot for each part and a frame for what is in
    /// view. A click or a drag there brings that place to the middle.
    fn minimap(&mut self, ui: &mut egui::Ui, t: &Table, painter: &egui::Painter, rect: Rect) {
        let pts: Vec<Pos2> = self.positions().into_iter().map(|p| (self.flat(p) - Vec2::splat(MARGIN)).to_pos2()).collect();
        let size = Vec2::new(160.0, 100.0);
        if pts.len() < 2 || rect.width() < size.x * 2.5 || rect.height() < size.y * 2.5 {
            return;
        }
        let c = ito_theme::colors();
        let world = Rect::from_points(&pts).expand(RADIUS * 3.0);
        let frame = Rect::from_min_size(rect.right_bottom() - size - Vec2::splat(8.0), size);
        let scale = ((size.x - 8.0) / world.width()).min((size.y - 8.0) / world.height());
        let to_mini = |m: Pos2| frame.center() + (m - world.center()) * scale;
        let resp = ui.interact(frame, ui.id().with("minimap"), Sense::click_and_drag());
        ito_a11y::say(&resp, Role::Button, t.tr("canvas.minimap"));
        if resp.clicked() || resp.dragged() {
            if let Some(q) = resp.interact_pointer_pos() {
                self.center_on((world.center() + (q - frame.center()) / scale).to_vec2());
            }
        }
        painter.rect(frame, 3.0, c.side, Stroke::new(1.0, c.border), StrokeKind::Inside);
        let inside = painter.with_clip_rect(frame);
        for (path, col, dash) in self.minimap_lines(&pts, to_mini, &c) {
            let stroke = Stroke::new(1.0, col);
            if dash {
                graph::dashed(&inside, &path, stroke, 3.0, 2.0, 0.0);
            } else {
                inside.add(egui::Shape::line(path, stroke));
            }
        }
        let bad: BTreeSet<&str> = self.problems.iter().filter_map(|p| p.job.as_deref()).collect();
        for (j, p) in self.net.jobs.iter().zip(&pts) {
            let col = if bad.contains(j.id.as_str()) {
                c.err
            } else if self.selected.contains(&j.id) {
                c.blue
            } else {
                c.fg
            };
            inside.circle_filled(to_mini(*p), 2.5, col);
        }
        let seen = Rect::from_min_max(
            ((-Vec2::splat(MARGIN) - self.pan) / self.zoom).to_pos2(),
            ((self.view - Vec2::splat(MARGIN) - self.pan) / self.zoom).to_pos2(),
        );
        inside.rect_stroke(Rect::from_two_pos(to_mini(seen.min), to_mini(seen.max)), 0.0, Stroke::new(1.5, c.blue), StrokeKind::Inside);
        resp.on_hover_text(t.tr("canvas.minimap_hint"));
    }

    /// Under the canvas: what the lines and the marks mean, and when the net starts in the next days.
    fn foot(&self, ui: &mut egui::Ui, t: &Table, cal: Option<&Calendar>, now: chrono::NaiveDateTime) {
        let c = ito_theme::colors();
        ui.horizontal_wrapped(|ui| {
            let head = t.tr("canvas.legend");
            let r = ui.label(egui::RichText::new(head).small());
            ito_a11y::say(&r, Role::Heading, head);
            for (col, dash, mark, key) in [
                (c.done, false, Mark::Done.mark(), "canvas.line_ok"),
                (c.err, true, Mark::Error.mark(), "canvas.line_error"),
                (c.dim, false, "*", "canvas.line_any"),
                (c.wait, false, "1,2", "canvas.line_code"),
                (c.dim, true, "else", "canvas.line_else"),
            ] {
                let (r, painter) = ui.allocate_painter(Vec2::new(24.0, 10.0), Sense::hover());
                let y = r.rect.center().y;
                let (a, b) = (Pos2::new(r.rect.left(), y), Pos2::new(r.rect.right(), y));
                let stroke = Stroke::new(2.0, col);
                if dash {
                    dashed(&painter, a, b, stroke);
                } else {
                    painter.line_segment([a, b], stroke);
                }
                let text = format!("{mark} {}", t.tr(key));
                let r = ui.label(egui::RichText::new(&text).small().color(col));
                ito_a11y::say(&r, Role::Label, &text);
            }
            ui.separator();
            let mut seen = BTreeSet::new();
            for part in shelf() {
                let Some(m) = part_icon(part.key()).filter(|_| !part.is_job() && seen.insert(part.key())) else {
                    continue;
                };
                // One widget of a known width, so the mark and its name wrap together.
                let key = format!("part.{}", part.key());
                let text = t.tr(&key);
                let galley = egui::WidgetText::from(egui::RichText::new(text).small()).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Small,
                );
                let (side, gap) = (12.0, 3.0);
                let size = Vec2::new(side + gap + galley.size().x, galley.size().y.max(side));
                let (rect, r) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    let p = ui.painter();
                    let at = Rect::from_center_size(Pos2::new(rect.left() + side / 2.0, rect.center().y), Vec2::splat(side));
                    ito_prefs::mark(p, at, m, c.dim);
                    p.galley(Pos2::new(at.right() + gap, rect.center().y - galley.size().y / 2.0), galley, c.fg);
                }
                ito_a11y::say(&r, Role::Label, text);
            }
        });
        let starts = self.starts(now, STRIP_DAYS, cal);
        ui.horizontal(|ui| {
            let head = t.trf("canvas.strip", &[&STRIP_DAYS.to_string()]);
            let r = ui.label(egui::RichText::new(&head).small());
            ito_a11y::say(&r, Role::Heading, &head);
            if self.net.when.is_empty() {
                let r = ui.label(egui::RichText::new(t.tr("canvas.strip_hand")).small().color(c.dim));
                ito_a11y::say(&r, Role::Label, t.tr("canvas.strip_hand"));
                return;
            }
            let count = t.trf("canvas.strip_count", &[&starts.len().to_string()]);
            let r = ui.label(egui::RichText::new(&count).small());
            ito_a11y::say(&r, Role::Label, &count);
            let (resp, painter) = ui.allocate_painter(Vec2::new(ui.available_width().max(120.0), 22.0), Sense::hover());
            let r = resp.rect;
            painter.rect_filled(r, 2.0, c.panel);
            let span = (STRIP_DAYS * 86_400) as f32;
            let x_of = |at: chrono::NaiveDateTime| r.left() + (at - now).num_seconds() as f32 / span * r.width();
            let small = egui::TextStyle::Small.resolve(ui.style());
            for k in 0..STRIP_DAYS {
                let Some(day) = now.date().checked_add_days(chrono::Days::new(k + 1)) else { continue };
                let x = x_of(day.and_time(chrono::NaiveTime::MIN));
                painter.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], Stroke::new(1.0, c.border));
                painter.text(Pos2::new(x + 2.0, r.top()), egui::Align2::LEFT_TOP, day.format("%-m/%-d").to_string(), small.clone(), c.dim);
            }
            for s in &starts {
                let x = x_of(*s);
                painter.line_segment([Pos2::new(x, r.top() + 10.0), Pos2::new(x, r.bottom() - 1.0)], Stroke::new(2.0, c.run));
            }
            let near = resp
                .hover_pos()
                .and_then(|p| starts.iter().min_by_key(|s| (x_of(**s) - p.x).abs() as i64).filter(|s| (x_of(**s) - p.x).abs() <= 6.0));
            if let Some(s) = near {
                resp.on_hover_text(s.format("%Y-%m-%d %H:%M").to_string());
            }
        });
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
                // A part's fields close back to the net's.
                if matches!(self.pane, Some(Pane::Part(..))) {
                    self.open_net();
                }
                self.clear_target();
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
                            None => self.choose(id),
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
    fn every_part_has_its_own_mark_and_its_group_is_in_one_run() {
        let mut seen = Vec::new();
        for part in shelf() {
            let m = part_icon(part.key()).unwrap_or_else(|| panic!("{} has no mark", part.key()));
            assert!(!seen.contains(&m), "{} shares its mark", part.key());
            seen.push(m);
        }
        // The shelf heads each group once, so its parts come one after another.
        let mut runs: Vec<_> = shelf().iter().map(|p| part_group(p.key())).collect();
        runs.dedup();
        let mut groups = runs.clone();
        groups.sort();
        groups.dedup();
        assert_eq!(runs.len(), groups.len(), "{runs:?}");
    }

    #[test]
    fn with_no_target_a_part_starts_a_flow_of_its_own() {
        let mut e = two();
        e.selected.insert("b".into());
        e.cursor = Some("b".into());
        assert_eq!(e.target().map(|j| j.id.as_str()), Some("b"));
        // Esc or a click on the empty canvas.
        e.clear_target();
        assert!(e.selected.is_empty() && e.target().is_none());
        let before = e.positions();
        let after = e.cursor.clone();
        let id = e.add(Part::Job, after.as_deref(), None);
        assert!(e.net.jobs[2].after.is_empty(), "not after b");
        // In the first column, under every part there is, not on top of one.
        let left = before.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let low = before.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(e.positions()[2], (left, low + layout::STEP_ACROSS));
        // The new part is the target now, so the next one follows it.
        assert_eq!(e.target().map(|j| j.id.clone()), Some(id));
    }

    /// A new Job is a script job with the engine's default shell once the engine has said it; before that it is the
    /// old `cmd.exe` one. A shell the engine lacks stays in the fields as it was written.
    #[test]
    fn a_new_job_takes_the_engines_default_shell_and_a_written_shell_survives_the_fields() {
        let mut e = two();
        let id = e.add(Part::Job, None, None);
        let j = e.net.jobs.iter().find(|j| j.id == id).unwrap();
        assert_eq!((j.run.first().map(String::as_str), j.shell.as_deref(), j.script.as_deref()), (Some("cmd.exe"), None, None));
        e.new_shell = "bash".into();
        let id = e.add(Part::Job, None, None);
        let j = e.net.jobs.iter().find(|j| j.id == id).unwrap().clone();
        assert_eq!((j.run.is_empty(), j.shell.as_deref(), j.script.as_deref()), (true, Some("bash"), Some("echo hello")));
        // A part that is not a Job takes none.
        let id = e.add(Part::Branch, None, None);
        assert_eq!(e.net.jobs.iter().find(|j| j.id == id).unwrap().shell, None);
        // Opening the fields and applying them changes nothing, whichever shell is written.
        let mut z = j.clone();
        z.shell = Some("zsh".into());
        let f = Fields::of(&z);
        assert_eq!(f.shell, "zsh");
        assert_eq!(f.apply(&z).unwrap(), z);
    }

    #[test]
    fn the_check_says_what_a_save_would_refuse_and_nothing_when_it_would_not() {
        let e = two();
        assert!(e.check(&[], &BTreeMap::new()).is_empty());
        let mut e = two();
        e.net.calendar = Some("会社".into());
        let found = e.check(&[], &BTreeMap::new());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            koyomi_core::msg::CAL_MISSING.is_in(&found[0]) && found[0].contains("会社") && found[0].ends_with("（経理/集計.toml）"),
            "{found:?}"
        );
        let cals = BTreeMap::from([("会社".to_owned(), Calendar::default())]);
        assert!(e.check(&[], &cals).is_empty());
        let mut e = two();
        e.net.jobs[1].part = Part::Jobnet { use_: "経理/ない".into() };
        assert!(!e.check(&[], &BTreeMap::new()).is_empty(), "a child net that is not there");
    }

    #[test]
    fn the_strip_shows_the_starts_of_the_next_days_and_none_by_hand() {
        let text = "format = 1
[[when]]
every = \"day\"
at = [\"09:00\"]
[[job]]
id = \"a\"
run = [\"x\"]
";
        let e = Editor::over("g/n", net(text));
        let from = chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let starts = e.starts(from, STRIP_DAYS, None);
        assert_eq!(starts.len(), 7);
        assert!(starts.iter().all(|s| *s > from && s.format("%H:%M").to_string() == "09:00"));
        assert!(two().starts(from, STRIP_DAYS, None).is_empty());
    }

    #[test]
    fn the_minimap_brings_the_place_clicked_to_the_middle() {
        let mut e = two();
        e.view = Vec2::new(800.0, 600.0);
        e.zoom = 1.5;
        let p = e.positions()[1];
        e.center_on(e.flat(p) - Vec2::splat(MARGIN));
        let at = e.to_screen(Pos2::ZERO, p, e.vertical);
        assert!((at - Pos2::new(400.0, 300.0)).length() < 0.01, "{at:?}");
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

    /// One frame of the canvas alone, filling an 800×600 screen from its top left.
    fn canvas_frame(ctx: &egui::Context, e: &mut Editor, events: Vec<egui::Event>) {
        let t = crate::i18n::table("ja").0;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| drop(e.canvas(ui, &t, false)));
        out.textures_delta.clear();
    }

    /// A click at `pos`, then long enough for the next click not to make a double click.
    fn canvas_click(ctx: &egui::Context, e: &mut Editor, pos: Pos2) {
        let button =
            |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        canvas_frame(ctx, e, vec![egui::Event::PointerMoved(pos)]);
        canvas_frame(ctx, e, vec![button(true)]);
        canvas_frame(ctx, e, vec![button(false)]);
        for _ in 0..40 {
            canvas_frame(ctx, e, Vec::new());
        }
    }

    #[test]
    fn a_click_on_a_part_shows_its_fields_in_the_pane() {
        let ctx = egui::Context::default();
        let mut e = two();
        e.fold = true;
        canvas_frame(&ctx, &mut e, Vec::new());
        let at = |e: &Editor, i: usize| e.to_screen(Pos2::ZERO, e.positions()[i], false);
        let b = at(&e, 1);
        canvas_click(&ctx, &mut e, b);
        assert!(matches!(&e.pane, Some(Pane::Part(p, _)) if p == "b"), "{:?}", e.pane);
        assert_eq!((e.cursor.as_deref(), e.selected.len()), (Some("b"), 1));
        assert!(e.fold, "a click leaves the pane folded");
        // What is being typed in the pane stays while the same part is clicked again.
        if let Some(Pane::Part(_, f)) = &mut e.pane {
            f.timeout = "soon".into();
        }
        canvas_click(&ctx, &mut e, b);
        assert!(matches!(&e.pane, Some(Pane::Part(_, f)) if f.timeout == "soon"));
        let a = at(&e, 0);
        canvas_click(&ctx, &mut e, a);
        assert!(matches!(&e.pane, Some(Pane::Part(p, f)) if p == "a" && f.run == "x"));
        // A click on nothing: nothing is chosen, and the pane shows the net's fields.
        canvas_click(&ctx, &mut e, Pos2::new(20.0, 580.0));
        assert!(e.cursor.is_none() && e.selected.is_empty());
        assert!(matches!(e.pane, Some(Pane::Net(_))));
        // A new part shows its fields too; opening one unfolds the pane.
        let id = e.add(Part::Job, Some("b"), None);
        assert!(matches!(&e.pane, Some(Pane::Part(p, _)) if *p == id));
        e.open_part("a");
        assert!(matches!(&e.pane, Some(Pane::Part(p, _)) if p == "a"));
        assert!(!e.fold);
    }

    /// A part a key adds is drawn in the same frame, with its line: where each part is is taken again after
    /// the keys (it was taken at the frame's start, one short, and drawing the line failed). One a key removes
    /// is gone, and the frames after draw without it.
    #[test]
    fn a_part_a_key_adds_is_drawn_in_the_same_frame() {
        let ctx = egui::Context::default();
        let mut e = two();
        canvas_frame(&ctx, &mut e, Vec::new());
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        e.cursor = Some("b".to_owned());
        canvas_frame(&ctx, &mut e, vec![key(egui::Key::A)]);
        assert_eq!(e.net.jobs.len(), 3, "`a` adds a part after the cursor's");
        assert!(!e.net.jobs[2].after.is_empty(), "the new part comes with its line, which is drawn too");
        e.selected = BTreeSet::from([e.net.jobs[2].id.clone()]);
        canvas_frame(&ctx, &mut e, vec![key(egui::Key::Delete)]);
        assert_eq!(e.net.jobs.len(), 2);
        canvas_frame(&ctx, &mut e, Vec::new());
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
        assert!(e.apply_fields("a", &f).unwrap_err().contains(&msg::DEF_ID_DUPLICATE.code()));
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
    fn an_approvals_message_is_read_edited_and_written_back() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let ask = e.add(Part::Approval { message: Some("金額を確かめる".into()) }, Some("a"), None);
        let mut fields = Fields::of(&e.net.jobs[1]);
        assert_eq!(fields.message, "金額を確かめる");
        fields.message = "宛先も見る".into();
        assert!(e.apply_fields(&ask, &fields).is_ok());
        assert_eq!(e.net.jobs[1].part, Part::Approval { message: Some("宛先も見る".into()) });
        assert!(e.text().contains("message = \"宛先も見る\""), "{}", e.text());
        fields.message = "  ".into();
        assert!(e.apply_fields(&ask, &fields).is_ok());
        assert_eq!(e.net.jobs[1].part, Part::Approval { message: None }, "blank: not written");
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
    fn run_as_takes_an_account_and_a_secret_name_both_or_neither() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let mut fields = Fields::of(&e.net.jobs[0]);
        fields.run_as_user = "CORP\\batch".into();
        assert!(e.apply_fields("a", &fields).unwrap_err().contains("run_as に password が指定されていません"));
        fields.run_as_secret = "secret:batch".into();
        assert_eq!(e.apply_fields("a", &fields), Ok(()));
        assert_eq!(e.net.jobs[0].run_as, Some(RunAs::Inline(jobnet::Account { user: "CORP\\batch".into(), secret: "batch".into() })));
        assert!(e.text().contains("run_as = { user = \"CORP\\\\batch\", password = \"secret:batch\" }"), "{}", e.text());
        assert_eq!(Fields::of(&e.net.jobs[0]).run_as_secret, "batch");
        fields.run_as_user = "bad user".into();
        assert!(e.apply_fields("a", &fields).is_err());
        fields.run_as_user.clear();
        fields.run_as_secret.clear();
        assert_eq!(e.apply_fields("a", &fields), Ok(()));
        assert_eq!(e.net.jobs[0].run_as, None);
    }

    /// A `run_as` by name and a sql job's `database` go through the fields and back as they were; a name beside a
    /// user or a secret is refused (E3398), and a bad name too.
    #[test]
    fn a_registered_account_and_database_round_trip_through_the_fields() {
        let mut e = Editor::over(
            "a/b",
            net("format = 1
[[job]]
id = \"a\"
run = [\"x\"]
run_as = \"ops\"
[[job]]
id = \"q\"
kind = \"sql\"
sql = \"select 1\"
database = \"main\"
"),
        );
        let before = e.net.jobs.clone();
        for i in 0..2 {
            let fields = Fields::of(&e.net.jobs[i]);
            assert_eq!(e.apply_fields(&e.net.jobs[i].id.clone(), &fields), Ok(()));
        }
        assert_eq!(e.net.jobs, before);
        let fields = Fields::of(&e.net.jobs[0]);
        assert_eq!((fields.run_as_name.as_str(), fields.run_as_user.as_str()), ("ops", ""));
        assert_eq!(Fields::of(&e.net.jobs[1]).sql_database, "main");
        assert!(e.text().contains("run_as = \"ops\"") && e.text().contains("database = \"main\""), "{}", e.text());
        // A name and a user, or a name and a secret.
        let mut both = Fields::of(&e.net.jobs[0]);
        both.run_as_user = "batch".into();
        assert!(e.apply_fields("a", &both).unwrap_err().contains("E3398"));
        let mut both = Fields::of(&e.net.jobs[0]);
        both.run_as_secret = "secret:x".into();
        assert!(e.apply_fields("a", &both).unwrap_err().contains("E3398"));
        let mut bad = Fields::of(&e.net.jobs[0]);
        bad.run_as_name = "a/b".into();
        assert!(e.apply_fields("a", &bad).unwrap_err().contains("E3395"));
        assert_eq!(e.net.jobs, before);
        // Emptying the name takes the account off.
        let mut none = Fields::of(&e.net.jobs[0]);
        none.run_as_name.clear();
        assert_eq!(e.apply_fields("a", &none), Ok(()));
        assert_eq!(e.net.jobs[0].run_as, None);
    }

    #[test]
    fn an_http_from_the_shelf_reads_its_headers_a_line_each() {
        let mut e = Editor::over(
            "a/b",
            net("format = 1
[[job]]
id = \"a\"
run = [\"x\"]
"),
        );
        let blank = shelf().into_iter().find(|p| p.key() == "http").unwrap();
        let h = e.add(blank, Some("a"), None);
        let mut fields = Fields::of(&e.net.jobs[1]);
        assert_eq!(fields.method, "GET");
        fields.url = "https://api.example.com/{date}".into();
        fields.method = "post".into();
        fields.body = "{\"a\": 1}
"
        .into();
        fields.headers = "Authorization: secret:api

X-Day: 1
"
        .into();
        fields.expect = "200, 201".into();
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        let Part::Chore(Chore::Http { method, body, headers, expect, .. }) = &e.net.jobs[1].part else { panic!() };
        assert_eq!(method, "POST");
        assert_eq!(
            body.as_deref(),
            Some(
                "{\"a\": 1}
"
            )
        );
        assert_eq!(headers.get("Authorization").map(String::as_str), Some("secret:api"));
        assert_eq!(expect, &vec![200, 201]);
        let again = Fields::of(&e.net.jobs[1]);
        assert_eq!(
            again.headers,
            "Authorization: secret:api
X-Day: 1"
        );
        assert_eq!(again.expect, "200, 201");
        fields.headers = "Authorization".into();
        assert!(e.apply_fields(&h, &fields).is_err());
        fields.headers.clear();
        fields.expect = "ok".into();
        assert!(e.apply_fields(&h, &fields).is_err());
        fields.expect.clear();
        fields.method = "GET".into();
        assert!(e.apply_fields(&h, &fields).is_err());
    }

    #[test]
    fn a_mail_from_the_shelf_takes_its_lists_a_line_each_and_no_account() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let blank = shelf().into_iter().find(|p| p.key() == "mail").unwrap();
        let h = e.add(blank, Some("a"), None);
        let mut fields = Fields::of(&e.net.jobs[1]);
        fields.mail_to = "ops@example.com\n\n 経理 <keiri@example.com> \n".into();
        fields.mail_cc = "boss@example.com".into();
        fields.subject = " 売上の集計 {date} ".into();
        fields.body = "添付のとおり。\n".into();
        fields.files = "D:\\out\\売上.csv\nD:\\out\\*.log".into();
        fields.via = " ops ".into();
        // Another account would read its own settings: a mail goes as koyomi's.
        (fields.run_as_user, fields.run_as_secret) = ("other".into(), "secret:other".into());
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        let j = &e.net.jobs[1];
        assert_eq!(j.run_as, None);
        let Part::Chore(Chore::Mail { to, cc, subject, body, files, via }) = &j.part else { panic!() };
        assert_eq!(to, &["ops@example.com", "経理 <keiri@example.com>"]);
        assert_eq!(cc, &["boss@example.com"]);
        assert_eq!(subject, "売上の集計 {date}");
        assert_eq!(body.as_deref(), Some("添付のとおり。\n"));
        assert_eq!(files.len(), 2);
        assert_eq!(via.as_deref(), Some("ops"));
        let again = Fields::of(j);
        assert_eq!(again.mail_to, "ops@example.com\n経理 <keiri@example.com>");
        assert_eq!((again.via.as_str(), again.files.lines().count()), ("ops", 2));
        fields.mail_to = "ops".into();
        assert!(e.apply_fields(&h, &fields).unwrap_err().contains("アドレス"));
        fields.mail_to = "ops@example.com".into();
        fields.subject.clear();
        assert!(e.apply_fields(&h, &fields).is_err());
    }

    #[test]
    fn a_sql_from_the_shelf_takes_its_database_sql_and_rows() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let blank = shelf().into_iter().find(|p| p.key() == "sql").unwrap();
        let h = e.add(blank, Some("a"), None);
        let mut fields = Fields::of(&e.net.jobs[1]);
        fields.db = " sqlserver://ops@db01/売上 ".into();
        fields.password = "secret:db".into();
        fields.sql = "update 売上 set 済 = 1 where 日 = '{date}'\nGO\n".into();
        fields.out = " D:\\out\\{date}.csv ".into();
        fields.rows = "1..".into();
        fields.tls = SqlTls::Trust;
        // Windows sign-in takes the job's account, so it keeps one.
        (fields.run_as_user, fields.run_as_secret) = ("CORP\\batch".into(), "secret:batch".into());
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        let j = &e.net.jobs[1];
        assert_eq!(
            j.run_as.as_ref().map(|r| match r {
                RunAs::Inline(a) => a.user.as_str(),
                RunAs::Named(n) => n.as_str(),
            }),
            Some("CORP\\batch")
        );
        let Part::Chore(Chore::Sql { db, password, sql, file, out, rows, tls, .. }) = &j.part else { panic!() };
        assert_eq!((db.as_str(), password.as_deref()), ("sqlserver://ops@db01/売上", Some("secret:db")));
        assert_eq!(sql.as_deref(), Some("update 売上 set 済 = 1 where 日 = '{date}'\nGO\n"));
        assert_eq!((file, out.as_deref(), rows.as_deref(), *tls), (&None, Some("D:\\out\\{date}.csv"), Some("1.."), SqlTls::Trust));
        let again = Fields::of(j);
        assert_eq!((again.db.as_str(), again.rows.as_str(), again.tls), ("sqlserver://ops@db01/売上", "1..", SqlTls::Trust));
        // Both the SQL and a file is one too many; a plain password is not taken.
        fields.file = "D:\\sql\\a.sql".into();
        assert!(e.apply_fields(&h, &fields).unwrap_err().contains("どちらか一方"));
        fields.sql.clear();
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        fields.password = "hunter2".into();
        assert!(e.apply_fields(&h, &fields).is_err());
        fields.password = "secret:db".into();
        fields.rows = "many".into();
        assert!(e.apply_fields(&h, &fields).is_err());
    }

    #[test]
    fn an_ssh_from_the_shelf_takes_its_pc_command_and_judge() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let blank = shelf().into_iter().find(|p| p.key() == "ssh").unwrap();
        let h = e.add(blank, Some("a"), None);
        let mut fields = Fields::of(&e.net.jobs[1]);
        fields.on = " ssh://ops@db1 ".into();
        fields.command = "backup.sh --day {date}".into();
        fields.host_key = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into();
        fields.warn = "1".into();
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        let j = &e.net.jobs[1];
        let Part::Chore(Chore::Ssh { on, command, host_key, .. }) = &j.part else { panic!() };
        assert_eq!((on.as_str(), command.as_str()), ("ssh://ops@db1", "backup.sh --day {date}"));
        assert!(host_key.is_some());
        assert_eq!(j.judge.as_ref().map(|jd| jd.warn.len()), Some(1));
        let again = Fields::of(j);
        assert_eq!((again.on.as_str(), again.warn.as_str()), ("ssh://ops@db1", "1"));
        fields.warn.clear();
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        assert!(e.net.jobs[1].judge.is_none());
        fields.warn = "x".into();
        assert!(e.apply_fields(&h, &fields).is_err());
        // A group of PCs: each may have its host key in koyomi.toml, so the job's is not needed.
        fields.warn.clear();
        fields.on = "group:web".into();
        fields.host_key.clear();
        fields.max = "4".into();
        fields.pass = Pass::Any;
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        let Part::Chore(Chore::Ssh { max, pass, host_key, .. }) = &e.net.jobs[1].part else { panic!() };
        assert_eq!((*max, *pass, host_key.is_none()), (Some(4), Pass::Any, true));
        let again = Fields::of(&e.net.jobs[1]);
        assert_eq!((again.max.as_str(), again.pass), ("4", Pass::Any));
        fields.max = "65".into();
        assert!(e.apply_fields(&h, &fields).unwrap_err().contains("max が 1 から 64 までの整数ではありません"));
        // One PC runs alone: no max.
        fields.on = "ssh://ops@db1".into();
        fields.max = "2".into();
        fields.host_key = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into();
        assert!(e.apply_fields(&h, &fields).is_err());
    }

    #[test]
    fn a_winrm_from_the_shelf_takes_its_pc_command_certificate_password_and_judge() {
        let mut e = Editor::over("a/b", net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let blank = shelf().into_iter().find(|p| p.key() == "winrm").unwrap();
        let h = e.add(blank, Some("a"), None);
        let mut fields = Fields::of(&e.net.jobs[1]);
        fields.on = " winrm://ops@app01 ".into();
        fields.command = "backup.cmd {date}".into();
        fields.password = "secret:app01".into();
        // The certificate is what keeps the password from going to the wrong PC.
        assert!(e.apply_fields(&h, &fields).is_err());
        fields.cert = ["AB"; 32].join(":");
        fields.warn = "1".into();
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        let j = &e.net.jobs[1];
        let Part::Chore(Chore::Winrm { on, command, cert, password, auth, .. }) = &j.part else { panic!() };
        assert_eq!((on.as_str(), command.as_str()), ("winrm://ops@app01", "backup.cmd {date}"));
        assert!(cert.is_some() && password.as_deref() == Some("secret:app01"));
        assert_eq!(*auth, WinrmAuth::Basic);
        assert_eq!(j.judge.as_ref().map(|jd| jd.warn.len()), Some(1));
        let again = Fields::of(j);
        assert_eq!((again.on.as_str(), again.cert.as_str()), ("winrm://ops@app01", fields.cert.as_str()));
        fields.auth = WinrmAuth::Ntlm;
        assert_eq!(e.apply_fields(&h, &fields), Ok(()));
        assert_eq!(Fields::of(&e.net.jobs[1]).auth, WinrmAuth::Ntlm);
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

    fn rows() -> Editor {
        Editor::over(
            "経理/集計",
            net("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"b\"\nrun = [\"x\"]\n[[job]]\nid = \"c\"\nrun = [\"x\"]\nafter = [\"a\"]\n[[job]]\nid = \"d\"\nrun = [\"x\"]\nafter = [\"b\"]\n"),
        )
    }

    fn tall_note() -> Sides {
        Sides { left: 73.0, right: 73.0, up: RADIUS, down: 120.0 }
    }

    #[test]
    fn align_leaves_room_for_a_tall_note_in_both_flows() {
        for vertical in [false, true] {
            let mut e = rows();
            e.vertical = vertical;
            let plain = Sides { left: RADIUS, right: RADIUS, up: RADIUS, down: RADIUS };
            e.sides.insert("a".into(), tall_note());
            e.sides.insert("b".into(), plain);
            e.sides.insert("c".into(), plain);
            e.align();
            let p = e.positions();
            let a = e.sides["a"];
            let (sa, sb, sc) = (e.flat(p[0]), e.flat(p[1]), e.flat(p[2]));
            // b stands beside a across the flow and c after it along the flow: both clear of a's note on the screen.
            let (beside, after) = if vertical {
                ((sb.x - plain.left) - (sa.x + a.right), (sc.y - plain.up) - (sa.y + a.down))
            } else {
                ((sb.y - plain.up) - (sa.y + a.down), (sc.x - plain.left) - (sa.x + a.right))
            };
            assert!(beside >= layout::GAP as f32 - 0.01, "vertical={vertical} beside={beside}");
            assert!(after >= layout::GAP as f32 - 0.01, "vertical={vertical} after={after}");
        }
    }

    #[test]
    fn a_saved_place_stays_where_it_is() {
        let mut e = rows();
        e.sides.insert("a".into(), tall_note());
        e.net.jobs[1].pos = Some((7.0, 9.0));
        assert_eq!(e.positions()[1], (7.0, 9.0));
        assert!(!e.dirty);
    }

    #[test]
    fn fit_keeps_the_edge_of_a_wide_note_in_view() {
        let mut e = rows();
        e.view = Vec2::new(800.0, 600.0);
        e.sides.insert("a".into(), Sides { left: 300.0, right: 300.0, up: RADIUS, down: 120.0 });
        e.align();
        e.fit();
        let origin = Pos2::ZERO;
        for (i, j) in e.net.jobs.iter().enumerate() {
            let s = e.sides.get(&j.id).copied().unwrap_or_default();
            let c = e.to_screen(origin, e.positions()[i], false);
            assert!(c.x - s.left >= -0.01 && c.x + s.right <= 800.01, "{} {c:?}", j.id);
            assert!(c.y - s.up >= -0.01 && c.y + s.down <= 600.01, "{} {c:?}", j.id);
        }
    }

    #[test]
    fn the_first_view_only_moves_right_and_down_and_only_as_far_as_needed() {
        // The edge at -73 from the margin of 64 is at -9: 17 short of SEAT.
        let pan = seated(Vec2::ZERO, Vec2::new(-73.0, -2.0));
        assert_eq!(pan.x, SEAT + 9.0);
        assert_eq!(pan.y, 0.0, "already inside");
        // Already inside: nothing moves, never left or up.
        assert_eq!(seated(Vec2::ZERO, Vec2::new(0.0, 0.0)), Vec2::ZERO);
        let far = seated(Vec2::new(5.0, -5.0), Vec2::new(500.0, 500.0));
        assert_eq!(far, Vec2::new(5.0, -5.0));
    }

    #[test]
    fn a_line_is_cut_on_the_circles_for_an_s_curve_and_an_aligned_pair() {
        // At zoom 1 and zoomed in, where the parts are drawn larger.
        for r in [RADIUS, RADIUS * 2.0] {
            for (to, vertical) in [(Pos2::new(220.0, 90.0), false), (Pos2::new(240.0, 10.0), false), (Pos2::new(90.0, 220.0), true)] {
                let from = Pos2::new(10.0, 10.0);
                let path = line_path(from, to, vertical, r).expect("a line");
                assert!(((path[0] - from).length() - r).abs() <= 0.5, "{path:?}");
                let end = *path.last().unwrap();
                assert!(((end - to).length() - (r + 2.0)).abs() <= 0.5, "{path:?}");
            }
        }
    }

    #[test]
    fn the_editor_cuts_its_lines_at_the_radius_it_draws() {
        let mut e = two();
        e.zoom = 1.5;
        assert_eq!(e.part_radius(), RADIUS * 1.5);
        e.zoom = 0.2;
        assert_eq!(e.part_radius(), RADIUS * 0.6);
    }

    #[test]
    fn parts_that_touch_have_no_line() {
        let a = Pos2::new(50.0, 50.0);
        // Every distance is under the skip rule (RADIUS * 2 + 2), on a slant and straight down.
        for d in [0.0, 1.0, RADIUS * 2.0, RADIUS * 2.0 + 1.9] {
            for vertical in [false, true] {
                assert!(line_path(a, a + Vec2::angled(0.29) * d, vertical, RADIUS).is_none(), "{d}");
                assert!(line_path(a, a + Vec2::new(0.0, d), vertical, RADIUS).is_none(), "{d}");
            }
        }
    }

    #[test]
    fn a_line_is_hit_on_its_curve_and_not_on_the_old_chord() {
        let mut e = two();
        e.vertical = false;
        let at = [Pos2::new(0.0, 0.0), Pos2::new(200.0, 100.0)];
        let path = line_path(at[0], at[1], false, e.part_radius()).unwrap();
        let (mid, _) = along(&path, graph::length(&path) * 0.5);
        assert_eq!(e.hit_line(&at, mid), Some((1, 0)));
        // A quarter of the way along the chord is far from the S-curve there.
        let chord = at[0] + (at[1] - at[0]) * 0.25;
        assert!(dist_poly(chord, &path) > NEAR);
        assert_eq!(e.hit_line(&at, chord), None);
    }

    #[test]
    fn the_minimap_has_a_curve_for_each_line() {
        let mut e = two();
        let c = ito_theme::colors();
        let pts = [Pos2::new(0.0, 0.0), Pos2::new(100.0, 60.0)];
        assert_eq!(e.minimap_lines(&pts, |p| p, &c).len(), 1);
        e.net.jobs[0].after.push(Line { job: "b".into(), on: On::Error });
        let lines = e.minimap_lines(&pts, |p| p, &c);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].2 && !lines[1].2, "only the error line is dashed");
        // Too short to show.
        assert!(e.minimap_lines(&pts, |p| Pos2::new(p.x * 0.001, p.y * 0.001), &c).is_empty());
    }
}
