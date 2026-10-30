//! `koyomi engine`: the one process that owns the definitions.
//!
//! It reads `jobnets/<group>/<name>.toml`, keeps `approved.toml`, watches the
//! folder, and answers the window and the CLI through the door (`proto`). It
//! outlives the window. Nothing here draws or asks a person anything.
//!
//! It is also the one place a run is told of (`notify`): the window and the
//! console only start runs, so a run is told of the same way however it started.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, Utc};
use ito_ipc::door::{ask, Ask};
use ito_prefs::NotifyCfg;
use koyomi_core::approved::{self, Store};
use koyomi_core::bundle::{self, Change};
use koyomi_core::calendar::Calendars;
use koyomi_core::config::Config;
use koyomi_core::group;
use koyomi_core::held::Held;
use koyomi_core::jobnet::Jobnet;
use koyomi_core::run::{How, Run, State as RunState};
use koyomi_core::scan::{def_of_text, link, scan_in, Def, Standing};
use koyomi_core::when::{self, Kind, OnClosed};

use crate::notify;
use crate::proto::{self, Request, Runs, Snapshot, Versions};

mod backup;
mod events;
mod files;
mod retain;
mod runner;
mod schedule;
use backup::Backup;
use retain::Retain;
use runner::Runner;
use schedule::Schedule;

/// Where everything of the engine's lives.
#[derive(Clone, Debug)]
pub struct Paths {
    /// `app_dir("koyomi")`: the settings folder. Holds `jobnets/`, `calendars/` and `koyomi.toml`.
    pub app: PathBuf,
    pub jobnets: PathBuf,
    /// `app_dir("koyomi")/calendars`: `<name>.toml`, what `calendar = "<name>"` names.
    pub calendars: PathBuf,
    /// `app_dir("koyomi")/koyomi.toml`.
    pub config: PathBuf,
    /// `state_dir("koyomi")`: `approved.toml`, `history/`, `logs/`.
    pub state: PathBuf,
    /// `state_dir("koyomi")/approved.toml`.
    pub approved: PathBuf,
    /// `state_dir("koyomi")/held.toml`: what a person stopped by hand.
    pub held: PathBuf,
}

impl Paths {
    pub fn find() -> Result<Paths, String> {
        let app = ito_common::app_dir("koyomi").ok_or("設定のフォルダーが分からない")?;
        let state = ito_common::state_dir("koyomi").ok_or("状態のフォルダーが分からない")?;
        Ok(Paths {
            jobnets: app.join("jobnets"),
            calendars: app.join("calendars"),
            config: app.join("koyomi.toml"),
            approved: state.join("approved.toml"),
            held: state.join("held.toml"),
            app,
            state,
        })
    }

    /// `koyomi.toml`, or why it does not read.
    pub fn config(&self) -> Result<Config, String> {
        Config::load(&self.config).map_err(|e| format!("{} が読めない（{e}）。直すまで流さない", shown(&self.config)))
    }

    /// The approvals, or the line that says `approved.toml` does not read.
    pub fn store(&self) -> Result<Store, String> {
        Store::load(&self.approved).map_err(|e| format!("{} が読めない（{e}）。直すか退けるまで承認しない", shown(&self.approved)))
    }

    /// What is on hold, or the line that says `held.toml` does not read.
    pub fn held(&self) -> Result<Held, String> {
        Held::load(&self.held).map_err(|e| format!("{} が読めない（{e}）。直すか退けるまで流さない", shown(&self.held)))
    }
}

/// Who other than this user can write what koyomi runs from, one line each. Empty: nothing in the way.
///
/// The definitions, the folders above them and `approved.toml` all count: whoever can change one
/// of them can make koyomi run a command nobody looked at.
pub fn access_warnings(paths: &Paths, files: &[PathBuf]) -> Vec<String> {
    let mut seen: Vec<(PathBuf, String)> = Vec::new();
    let mut check = |p: &Path| {
        if !p.exists() {
            return;
        }
        for f in ito_common::access::findings(p).unwrap_or_default() {
            let key = (f.path.clone(), f.who.clone());
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
    };
    check(&paths.jobnets);
    check(&paths.app);
    check(&paths.approved);
    files.iter().for_each(|f| check(f));
    seen.into_iter().map(|(p, who)| format!("{} は {} が書き換えられる", shown(&p), who)).collect()
}

/// The path without Windows' `\\?\` prefix.
fn shown(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
}

struct State {
    paths: Paths,
    store: Store,
    /// Why `approved.toml` did not read: until it does, nothing is approved and nothing written over it.
    unreadable: Option<String>,
    /// The nets on hold and whether the schedule is paused (`held.toml`).
    held: Held,
    /// Why `held.toml` did not read: until it does, nothing starts and nothing is written over it.
    held_bad: Option<String>,
    defs: Vec<Def>,
    /// `calendars/*.toml` as last read; the nets count their days by these.
    calendars: Calendars,
    warnings: Vec<String>,
    /// Why `koyomi.toml` does not read: until it does, nothing is started.
    bad_config: Option<String>,
    runner: Runner,
    /// The clock: when `[[when]]` was last looked at.
    schedule: Schedule,
    /// The rest of `[[when]]`: files arriving and the engine's start.
    events: events::Events,
    /// The sweep of logs and history older than `keep_days`.
    retain: Retain,
    /// The daily backup of the settings folder.
    backup: Backup,
    keep_days: u32,
    /// `[notify]` as runs are told with: the `secret:` addresses read back
    /// (on a thread; until it answers they are not sent to).
    notify: NotifyCfg,
    /// `[notify]` as it last read, to see a change; `None` before the first read.
    notify_raw: Option<NotifyCfg>,
    /// What a person should see about `[notify]`: from the reading, then from the credential store.
    notify_notes: Vec<String>,
    reveal_notes: Vec<String>,
    /// Which reading the thread is reading back for; an older answer is dropped.
    reveal_gen: u64,
    reveal_tx: mpsc::Sender<(u64, NotifyCfg, Vec<String>)>,
    reveal_rx: mpsc::Receiver<(u64, NotifyCfg, Vec<String>)>,
    /// The words a run is told in: common.toml's language.
    words: ito_i18n::Table,
}

impl State {
    fn new(paths: Paths) -> State {
        let (config, bad_config) = match paths.config() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e)),
        };
        let keep_days = config.keep_days;
        let runner = Runner::new(&paths.state, config);
        let schedule = Schedule::new(&paths.state);
        let retain = Retain::new(&paths.state);
        let backup = Backup::new(&paths.app, &paths.state);
        let (reveal_tx, reveal_rx) = mpsc::channel();
        let mut s = State {
            paths,
            store: Store::default(),
            unreadable: None,
            held: Held::default(),
            held_bad: None,
            defs: Vec::new(),
            calendars: Calendars::default(),
            warnings: Vec::new(),
            bad_config,
            runner,
            schedule,
            events: events::Events::new(),
            retain,
            backup,
            keep_days,
            notify: NotifyCfg::default(),
            notify_raw: None,
            notify_notes: Vec::new(),
            reveal_notes: Vec::new(),
            reveal_gen: 0,
            reveal_tx,
            reveal_rx,
            words: crate::cli::table(),
        };
        s.reload();
        s.rescan();
        s
    }

    /// Reads `koyomi.toml` again. A changed `[notify]` is used at once, and its
    /// `secret:` addresses are read back on a thread (the store can be slow).
    fn reload_config(&mut self) {
        match self.paths.config() {
            Ok(c) => {
                let (n, notes) = notify::read(&c);
                if self.notify_raw.as_ref() != Some(&n) {
                    self.notify_raw = Some(n.clone());
                    self.notify_notes = notes;
                    self.reveal_notes.clear();
                    self.notify = n.clone();
                    self.reveal_gen += 1;
                    let (gen, tx) = (self.reveal_gen, self.reveal_tx.clone());
                    std::thread::spawn(move || {
                        let mut n = n;
                        let notes = n.reveal("koyomi");
                        let _ = tx.send((gen, n, notes));
                    });
                }
                self.keep_days = c.keep_days;
                self.runner.set_config(c);
                self.bad_config = None;
            }
            Err(e) => self.bad_config = Some(e),
        }
    }

    /// Takes in the addresses read back, and tells of the runs that ended.
    fn pump(&mut self) {
        while let Ok((gen, n, notes)) = self.reveal_rx.try_recv() {
            if gen == self.reveal_gen {
                self.notify = n;
                self.reveal_notes = notes;
            }
        }
        // A 通知 part tells of the run as it stands.
        for (run, job) in self.runner.take_notices() {
            let Some(r) = self.runner.runs().iter().find(|r| r.id == run) else { continue };
            let (event, title, body) = notify::part_words(&self.words, r, &job);
            notify::tell(&self.notify, event, &title, &body);
        }
        let over = self.runner.take_over();
        self.tick_after(&over);
        let (runs, missed) = notify::group_missed(over);
        // A time stopped by hand (⏸) is not told of: the person did it.
        for r in runs.into_iter().filter(|r| r.state != RunState::Held) {
            let (event, title, body) = notify::words(&self.words, &r);
            notify::tell(&self.notify, event, &title, &body);
        }
        for runs in missed {
            let (event, title, body) = notify::missed_words(&self.words, &runs);
            notify::tell(&self.notify, event, &title, &body);
        }
    }

    /// Starts what `[[when]]` says is due since the last look, and records
    /// each time that did not start with why (`schedule::judge`).
    fn tick_schedule(&mut self) {
        let now = Utc::now();
        let Some(checked) = self.schedule.look(now) else { return };
        let mut due: Vec<(usize, DateTime<Utc>, NaiveDateTime)> = Vec::new();
        for (i, d) in self.defs.iter().enumerate() {
            if let Some(net) = &d.net {
                let cal = d.calendar.as_deref().and_then(|c| self.calendars.get(c));
                due.extend(when::due_in(&Local, &net.when, cal, checked, now).into_iter().map(|(instant, planned)| (i, instant, planned)));
            }
        }
        for (k, (i, instant, planned)) in due.iter().enumerate() {
            let def = self.defs[*i].clone();
            if let Some((state, why)) = self.stopped(&def, planned.date()) {
                self.runner.not_run(&def, *planned, state, &why);
                continue;
            }
            let running = self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over());
            let last = !due[k + 1..].iter().any(|(j, _, _)| j == i);
            let verdict = schedule::judge(&def, *planned, now - *instant, &self.warnings, self.bad_config.as_deref(), running, last);
            match verdict {
                schedule::Verdict::Start { late } => {
                    let out = if late {
                        self.runner.start_late(&def, *planned)
                    } else {
                        self.runner.start(&def, planned.date(), How::Schedule, Some(*planned))
                    };
                    if let Err(e) = out {
                        self.runner.not_run(&def, *planned, RunState::Locked, &e);
                    }
                }
                schedule::Verdict::Queue { late } => self.runner.queue(&def, *planned, late),
                schedule::Verdict::NotRun(state, reason) => {
                    self.runner.not_run(&def, *planned, state, &reason);
                }
            }
        }
        self.schedule.looked(now, !due.is_empty());
    }

    /// Why `def` is not to start on `day` whatever its `[[when]]` says: the
    /// holiday table does not reach the day (it may be a holiday, so it is not
    /// run as if it were not), the schedule is paused, or the net is held.
    /// Each is written down (⏸ for the two by hand), so nothing is lost in silence.
    fn stopped(&self, def: &Def, day: NaiveDate) -> Option<(RunState, String)> {
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if let Some(until) = cal.filter(|c| c.stale(day)).and_then(|c| c.until()) {
            return Some((RunState::Locked, format!("祝日の表が古い（{until} まで）")));
        }
        if self.held.paused {
            return Some((RunState::Held, "スケジュールを止めている（koyomi resume）".to_owned()));
        }
        if self.held.is_held(&def.id) {
            return Some((RunState::Held, format!("保留中（koyomi release {}）", def.id)));
        }
        None
    }

    /// Why an event on `day` does not start `def` by its `when`: the day is
    /// outside `start` / `end` or in `except`, or it is a closed day and
    /// `on_closed` is not `"run"`. `what` is the event, for the record.
    fn off_day(&self, def: &Def, w: &when::When, day: NaiveDate, what: &str) -> Option<String> {
        if !w.runs_on(day) {
            return Some(format!("{what}が、except か start / end の外なので流さない"));
        }
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if cal.is_some_and(|c| !c.is_business(day)) && w.on_closed_in(cal) != OnClosed::Run {
            return Some(format!("{what}が、休業日なので流さない（on_closed = \"run\" で流せる）"));
        }
        None
    }

    /// Starts what an event says: the nets with `on_start = true` once, when
    /// the engine starts; a net with `file = …` for each file that has come
    /// (`events::Events`). What does not start is recorded with why, as a time
    /// is. A file that comes while its net runs waits for the run to end.
    fn tick_events(&mut self) {
        let now = Local::now().naive_local();
        let today = now.date();
        if self.events.take_start() {
            for def in self.defs.clone() {
                let Some(w) = def.net.as_ref().and_then(|n| n.when.iter().find(|w| matches!(w.kind, Kind::OnStart))) else { continue };
                if let Some(why) = self.off_day(&def, w, today, "起動した") {
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if let Some((state, why)) = self.stopped(&def, today) {
                    self.runner.not_run(&def, now, state, &why);
                    continue;
                }
                match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                    schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                        if let Err(e) = self.runner.start_event(&def, today, How::Start, "起動したとき", None) {
                            self.runner.not_run(&def, now, RunState::Locked, &e);
                        }
                    }
                    schedule::Verdict::NotRun(state, reason) => {
                        self.runner.not_run(&def, now, state, &reason);
                    }
                }
            }
        }
        for a in self.events.due() {
            let path = a.path.display().to_string();
            let Some(def) = self.defs.iter().find(|d| d.id == a.def_id).cloned() else {
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            };
            let Some(w) =
                def.net.as_ref().and_then(|n| n.when.iter().find(|w| matches!(&w.kind, Kind::File { file, .. } if *file == a.pattern)))
            else {
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            };
            if let Some(why) = self.off_day(&def, w, today, &format!("{path} が来た")) {
                self.runner.not_run(&def, now, RunState::Skipped, &why);
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            }
            if let Some((state, why)) = self.stopped(&def, today) {
                self.runner.not_run(&def, now, state, &why);
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            }
            // Still running: the file keeps; it starts the net when the run ends.
            if self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over()) {
                continue;
            }
            match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                    if let Err(e) = self.runner.start_event(&def, today, How::File, &format!("{path} が来た"), Some(path)) {
                        self.runner.not_run(&def, now, RunState::Locked, &e);
                    }
                }
                schedule::Verdict::NotRun(state, reason) => {
                    self.runner.not_run(&def, now, state, &reason);
                }
            }
            self.events.fired(&a.def_id, &a.pattern, &a.path);
        }
    }

    /// Starts the nets with `[[when]] after_jobnet = …` on each run of `over`
    /// that ended as their `on` says, however that run was started. The run
    /// takes the business day of the run it follows (`KOYOMI_DATE`). A net
    /// starts this way once a business day: a second end on the same day is
    /// recorded as ⊘ スキップ and told of, so a person runs it by hand if it
    /// is wanted. A net still running from before does not start again.
    fn tick_after(&mut self, over: &[Run]) {
        let now = Local::now().naive_local();
        for prev in over {
            for (def, w) in followers(&self.defs, prev) {
                let date = NaiveDate::parse_from_str(&prev.date, "%Y-%m-%d").unwrap_or(now.date());
                let what = format!("{} が終わった（{}）", prev.jobnet, crate::show::word(&self.words, prev.state));
                if let Some(why) = self.off_day(&def, &w, date, &what) {
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if let Some((state, why)) = self.stopped(&def, date) {
                    self.runner.not_run(&def, now, state, &why);
                    continue;
                }
                if let Some(done) = followed(self.runner.runs(), &def.id, &prev.date) {
                    let why = format!(
                        "{what}が、同じ業務日（{}）に既に流した（{}）: 後続は流していない。要るなら手で流す（koyomi run {}）",
                        prev.date, done.id, def.id
                    );
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over()) {
                    self.runner.not_run(&def, now, RunState::Skipped, &format!("{what}が、前の回がまだ動いているので流さない"));
                    continue;
                }
                match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                    schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                        if let Err(e) = self.runner.start_event(&def, date, How::After, &what, None) {
                            self.runner.not_run(&def, now, RunState::Locked, &e);
                        }
                    }
                    schedule::Verdict::NotRun(state, reason) => {
                        self.runner.not_run(&def, now, state, &reason);
                    }
                }
            }
        }
    }

    /// Reads `approved.toml` again; only while it does not read (the engine is its one writer).
    fn reload(&mut self) {
        match self.paths.store() {
            Ok(store) => {
                self.store = store;
                self.unreadable = None;
            }
            Err(e) => {
                self.store = Store::default();
                self.unreadable = Some(e);
            }
        }
        match self.paths.held() {
            Ok(held) => {
                self.held = held;
                self.held_bad = None;
            }
            Err(e) => {
                self.held = Held::default();
                self.held_bad = Some(e);
            }
        }
    }

    fn rescan(&mut self) {
        if self.unreadable.is_some() || self.held_bad.is_some() {
            self.reload();
        }
        self.reload_config();
        self.calendars = Calendars::load(&self.paths.calendars);
        self.defs = scan_in(&self.paths.jobnets, &self.store, &self.calendars);
        self.runner.set_children(Self::children_of(&self.defs));
        self.runner.set_calendars(
            self.calendars.map.clone(),
            self.defs.iter().filter_map(|d| Some((d.id.clone(), d.calendar.clone()?))).collect(),
        );
        // Files there before the watch began are not run: they may be old, and
        // a restart must not run them again. The record says they were there.
        let now = Local::now().naive_local();
        for (id, pattern, n) in self.events.set_rules(&self.defs) {
            if let Some(def) = self.defs.iter().find(|d| d.id == id).cloned() {
                self.runner.not_run(&def, now, RunState::Skipped, &format!("見張り始めたときに既に {n} 個あった（{pattern}）: 流さない"));
            }
        }
        let files: Vec<PathBuf> = self.defs.iter().map(|d| self.paths.jobnets.join(&d.file)).collect();
        self.warnings = access_warnings(&self.paths, &files);
        if !self.events.watching() {
            self.warnings.push("待つファイルのフォルダーを見張れない（5 秒ごとに見る）".to_owned());
        }
        self.warnings.extend(self.unreadable.clone());
        self.warnings.extend(self.held_bad.clone());
    }

    /// What a `kind = "jobnet"` part may use: each confirmed net, or why the others cannot be.
    fn children_of(defs: &[Def]) -> BTreeMap<String, Result<Jobnet, String>> {
        defs.iter()
            .map(|d| {
                let net = match (d.standing, &d.net) {
                    (Standing::Confirmed, Some(n)) => Ok(n.clone()),
                    (Standing::Unconfirmed, _) => Err(format!("子ジョブネット {} が未確認", d.id)),
                    _ => Err(format!("子ジョブネット {} に定義の誤りがある", d.id)),
                };
                (d.id.clone(), net)
            })
            .collect()
    }

    fn snapshot(&self) -> Snapshot {
        let mut warnings = self.warnings.clone();
        warnings.extend(self.bad_config.clone());
        let notes = self.notify_notes.iter().chain(&self.reveal_notes).cloned().collect();
        Snapshot {
            defs: self.defs.clone(),
            warnings,
            notes,
            held: self.held.held.clone(),
            paused: self.held.paused,
            cautions: self.calendars.cautions(Local::now().date_naive(), CAUTION_DAYS),
            calendars: self.calendars.map.clone(),
        }
    }

    fn status(&self) -> String {
        let confirmed = self.defs.iter().filter(|d| d.standing == Standing::Confirmed).count();
        let running = self.runner.runs().iter().filter(|r| !r.is_over()).count();
        let mut out = format!(
            "koyomi engine pid {}: 定義 {} 件（確認済み {}）、動いている実行 {}\n{}\n{}",
            std::process::id(),
            self.defs.len(),
            confirmed,
            running,
            self.retain.line(),
            self.backup.line()
        );
        if self.held.paused {
            out.push_str("\nスケジュールを止めている（koyomi resume で戻す）");
        }
        for id in &self.held.held {
            out.push_str(&format!("\n保留中: {id}（koyomi release {id} で戻す）"));
        }
        for q in self.runner.queued() {
            out.push_str(&format!(
                "\n待ち: {}  {} の回は前の回が終わってから流す（{} から待っている）",
                q.def.label(),
                q.planned.format("%-H:%M"),
                q.since.format("%-H:%M")
            ));
        }
        out
    }

    /// The sweep of old records: started for a new day or a new `keep_days`
    /// (not while koyomi.toml does not read: a typo must not sweep), its
    /// end told on the console.
    fn tick_retain(&mut self) {
        if self.bad_config.is_some() {
            return;
        }
        if let Some(line) = self.retain.tick(self.keep_days) {
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// The daily backup: its end told on the console.
    fn tick_backup(&mut self) {
        if let Some(line) = self.backup.tick() {
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// Starts `id` by hand, at the job `from` when there is one. Refused for
    /// every reason a run must not start quietly.
    fn start(&mut self, id: &str, date: Option<&str>, from: Option<&str>) -> Result<String, String> {
        let def = self.startable(id)?.clone();
        let date = match date {
            Some(d) => NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| format!("業務日は YYYY-MM-DD: {d}"))?,
            None => Local::now().date_naive(),
        };
        match from {
            Some(from) => self.runner.start_at(&def, date, from),
            None => self.runner.start(&def, date, How::Manual, None),
        }
    }

    /// Starts `run` again ([`Runner::rerun`]): with the definition that run
    /// had, but only while the net is still confirmed as it stands now.
    fn rerun(&mut self, run: &str, from: Option<&str>) -> Result<String, String> {
        let prev = self.runner.previous(run)?;
        self.startable(&prev.jobnet)?;
        self.runner.rerun(run, from)
    }

    /// The definition `id` as it may be started: nothing warned, koyomi.toml reads, and it is confirmed.
    fn startable(&self, id: &str) -> Result<&Def, String> {
        if !self.warnings.is_empty() {
            return Err(format!("流せない: {}", self.warnings.join(" / ")));
        }
        if let Some(e) = &self.bad_config {
            return Err(format!("流せない: {e}"));
        }
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        match def.standing {
            Standing::Invalid => Err(format!("定義の誤りがあり、流せない: {}", def.file)),
            Standing::Unconfirmed => Err(format!("未確認のため流さない: {}（koyomi approve {id}）", def.file)),
            Standing::Confirmed if self.held.is_held(id) => Err(format!("保留中のため流さない: {id}（koyomi release {id}）")),
            Standing::Confirmed => Ok(def),
        }
    }

    /// `held.toml` can be written: it read, and nothing else is in the way.
    fn held_writable(&self) -> Result<(), String> {
        match &self.held_bad {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
    fn save_held(&self) -> Result<(), String> {
        if let Some(dir) = self.paths.held.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.held, self.held.to_toml())
    }
    fn hold(&mut self, id: &str) -> Result<String, String> {
        self.held_writable()?;
        let label = self.defs.iter().find(|d| d.id == id).map(Def::label).ok_or_else(|| format!("定義がない: {id}"))?.to_owned();
        if !self.held.hold(id) {
            return Err(format!("もう保留している: {id}"));
        }
        self.save_held()?;
        for q in self.runner.unqueue(Some(id)) {
            self.runner.not_run(&q.def, q.planned, RunState::Held, &format!("待っている間に保留した（koyomi release {id}）"));
        }
        Ok(format!("保留した: {label}（koyomi release {id} で戻す）"))
    }
    fn release(&mut self, id: &str) -> Result<String, String> {
        self.held_writable()?;
        if !self.held.release(id) {
            return Err(format!("保留していない: {id}"));
        }
        self.save_held()?;
        Ok(format!("保留を解いた: {id}"))
    }
    fn pause(&mut self, on: bool) -> Result<String, String> {
        self.held_writable()?;
        if self.held.paused == on {
            return Err(if on { "もう止めている（koyomi resume で戻す）" } else { "止めていない" }.to_owned());
        }
        self.held.paused = on;
        self.save_held()?;
        if on {
            for q in self.runner.unqueue(None) {
                self.runner.not_run(&q.def, q.planned, RunState::Held, "待っている間にスケジュールを止めた（koyomi resume）");
            }
        }
        Ok(if on { "スケジュールを止めた（koyomi resume で戻す）" } else { "スケジュールを戻した" }.to_owned())
    }

    fn approve(&mut self, id: &str, sha256: &str) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("承認できない: {}", self.warnings.join(" / ")));
        }
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        if def.standing == Standing::Invalid {
            return Err(format!("定義の誤りがあり、承認できない: {}", def.file));
        }
        if def.sha256 != sha256 {
            return Err(format!("{} が確かめたあとに変わった。もう一度確かめてください", def.file));
        }
        let mut next = self.store.clone();
        next.approve(id, &def.sha256, &def.text);
        // Written before it counts, and whole: a crash leaves the old approvals.
        if let Some(dir) = self.paths.approved.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.approved, next.to_toml())?;
        self.store = next;
        self.rescan();
        Ok(format!("確認した: {id}"))
    }

    /// The editor's save (see [`Request::Save`]): checked as a file would be, the old
    /// file kept under `versions/`, written whole, and approved at once.
    fn save(&mut self, id: &str, text: &str, base_sha256: Option<&str>, force: bool) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("保存できない: {}", self.warnings.join(" / ")));
        }
        let (group, name) = split_id(id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        // The file as it is now, not as the last scan saw it: an edit by hand a moment ago counts.
        let now = std::fs::read(&path).ok().map(|b| approved::hash(&b));
        if !force && now.as_deref() != base_sha256 {
            return Err(format!("ほかで変わっています: {id}"));
        }
        let shared = group::load(&self.paths.jobnets.join(group).join("_group.toml"));
        let def = def_of_text(text, group, name, &self.store, shared.as_ref(), &self.calendars);
        let mut all: Vec<Def> = self.defs.iter().filter(|d| d.id != id).cloned().collect();
        all.push(def);
        link(&mut all);
        let def = all.pop().expect("pushed");
        if def.standing == Standing::Invalid {
            let why: Vec<String> = def.problems.iter().map(|p| p.in_file(&def.file)).collect();
            return Err(format!("保存できない:\n{}", why.join("\n")));
        }
        self.keep_version(&path, group, name)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        }
        ito_common::write_atomic(&path, text)?;
        let mut next = self.store.clone();
        next.approve(id, &def.sha256, text);
        if let Some(dir) = self.paths.approved.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.approved, next.to_toml())?;
        self.store = next;
        self.rescan();
        Ok(format!("保存した: {id}"))
    }

    /// Copies the file at `path`, when there is one, to
    /// `versions/<group>/<name>/<YYYYMMDD-HHMMSS>.toml` before it is written over.
    fn keep_version(&self, path: &Path, group: &str, name: &str) -> Result<(), String> {
        self.keep_version_in(path, &self.versions_dir(group, name))
    }

    /// Copies the file at `path`, when there is one, to `dir/<YYYYMMDD-HHMMSS>.toml`.
    fn keep_version_in(&self, path: &Path, dir: &Path) -> Result<(), String> {
        let Ok(old) = std::fs::read(path) else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
        let mut version = stamp.clone();
        let mut n = 1;
        while dir.join(format!("{version}.toml")).exists() {
            n += 1;
            version = format!("{stamp}-{n}");
        }
        ito_common::write_atomic(&dir.join(format!("{version}.toml")), old)
    }

    /// Brings a bundle in. The plan is answered whatever happens; it is carried
    /// out only when nothing is undecided or wrong and `dry_run` is off. What is
    /// written over is kept under `versions/` first; a net that comes in is
    /// unconfirmed, like a file written by hand.
    fn import(&mut self, text: &str, dry_run: bool, on_conflict: Option<&str>) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("取り込めない: {}", self.warnings.join(" / ")));
        }
        let on_conflict = on_conflict.map(bundle::Conflict::parse).transpose()?;
        let b = bundle::parse(text).map_err(|e| format!("取り込めない: {e}"))?;
        if b.is_empty() {
            return Err("取り込めない: ファイルに何も入っていない".to_owned());
        }
        let current = bundle::Source::read(&self.paths.app);
        let p = bundle::plan(&b, &current, on_conflict);
        let mut lines = Vec::new();
        for item in &p.items {
            lines.push(item.line());
            if item.change == Change::Replace {
                if let Some(old) = &item.old {
                    lines.extend(diff_lines(old, &item.new));
                }
            }
            lines.extend(item.problems.iter().map(|p| format!("  問題: {p}")));
        }
        lines.extend(p.notes.iter().cloned());
        let mut reply = proto::Plan {
            lines,
            conflicts: p.conflicts(),
            problems: p.problems(),
            undecided: p.undecided,
            writes: p.writes(),
            applied: false,
        };
        if dry_run || p.undecided || p.problems() > 0 || !p.writes() {
            return Ok(reply.to_toml());
        }
        let mut written = 0;
        for item in &p.items {
            let at = match &item.change {
                Change::Add | Change::Replace => item.name.clone(),
                Change::Rename(to) => to.clone(),
                Change::Same | Change::Skip => continue,
            };
            let versions = self.paths.state.join("versions");
            let (path, dir) = match item.kind {
                bundle::Kind::Settings => (self.paths.config.clone(), versions.join("_settings")),
                bundle::Kind::Calendar => (self.paths.calendars.join(format!("{at}.toml")), versions.join("_calendars").join(&at)),
                bundle::Kind::Group => (self.paths.jobnets.join(&at).join("_group.toml"), versions.join("_groups").join(&at)),
                bundle::Kind::Jobnet => {
                    let (group, name) = split_id(&at)?;
                    (self.paths.jobnets.join(group).join(format!("{name}.toml")), self.versions_dir(group, name))
                }
            };
            let done = (|| {
                self.keep_version_in(&path, &dir)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{} を作れない: {e}", shown(parent)))?;
                }
                ito_common::write_atomic(&path, item.new.clone())
            })();
            if let Err(e) = done {
                self.rescan();
                return Err(format!("取り込みの途中で止まった（{written} 件は書いた。前の中身は versions/ にある）: {e}"));
            }
            written += 1;
        }
        self.rescan();
        reply.applied = true;
        Ok(reply.to_toml())
    }

    fn versions_dir(&self, group: &str, name: &str) -> PathBuf {
        self.paths.state.join("versions").join(group).join(name)
    }

    /// The versions kept of `id`, newest first.
    fn versions(&self, id: &str) -> Result<String, String> {
        let (group, name) = split_id(id)?;
        let mut versions: Vec<String> = match std::fs::read_dir(self.versions_dir(group, name)) {
            Ok(entries) => entries.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".toml").map(str::to_owned)).collect(),
            Err(_) => Vec::new(),
        };
        versions.sort();
        versions.reverse();
        Ok(Versions { versions }.to_toml())
    }

    /// Puts a kept version back as the file (the file as it is is kept first). Not approved:
    /// what comes back is looked at like a file written by hand.
    fn restore(&mut self, id: &str, version: &str) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("戻せない: {}", self.warnings.join(" / ")));
        }
        let (group, name) = split_id(id)?;
        if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Err(format!("版の名前は 20261030-101500 のような形: {version}"));
        }
        let from = self.versions_dir(group, name).join(format!("{version}.toml"));
        let text = std::fs::read(&from).map_err(|_| format!("版がない: {id} の {version}"))?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        self.keep_version(&path, group, name)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        }
        ito_common::write_atomic(&path, text)?;
        self.rescan();
        Ok(format!("{version} に戻した: {id}（未確認。確かめてから流す）"))
    }

    fn answer(&mut self, request: &Request) -> Result<String, String> {
        match request {
            Request::Status => Ok(self.status()),
            Request::Stop => Ok("止める".to_owned()),
            Request::List => Ok(self.snapshot().to_toml()),
            Request::Show { id } => {
                let def = self.defs.iter().find(|d| &d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
                toml::to_string(def).map_err(|e| e.to_string())
            }
            Request::Approve { id, sha256 } => self.approve(id, sha256),
            Request::Run { id, date, from } => self.start(id, date.as_deref(), from.as_deref()),
            Request::Rerun { run, from } => self.rerun(run, from.as_deref()),
            Request::Kill { run, job } => self.runner.kill(run, job.as_deref()),
            Request::Runs => Ok(Runs { runs: self.runner.today() }.to_toml()),
            Request::History { from, to } => {
                let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("日付は YYYY-MM-DD で書く: {s}"));
                Ok(Runs { runs: self.runner.history(day(from)?, day(to)?)? }.to_toml())
            }
            Request::Log { run, job } => self.runner.log_tail(run, job),
            Request::Hold { id } => self.hold(id),
            Request::Release { id } => self.release(id),
            Request::Skip { run, job } => self.runner.skip(run, job),
            Request::Pause => self.pause(true),
            Request::Resume => self.pause(false),
            Request::Save { id, text, base_sha256, force } => self.save(id, text, base_sha256.as_deref(), *force),
            Request::Versions { id } => self.versions(id),
            Request::Restore { id, version } => self.restore(id, version),
            Request::Import { text, dry_run, on_conflict } => self.import(text, *dry_run, on_conflict.as_deref()),
        }
    }
}

/// The difference between `old` and `new` as lines under a plan's item,
/// `  +` added, `  -` removed, the rest for context; long ones are cut.
fn diff_lines(old: &str, new: &str) -> Vec<String> {
    const MOST: usize = 40;
    let items = ito_diff::diff(old, new, 2);
    let mut out: Vec<String> = items
        .iter()
        .map(|i| match i {
            ito_diff::Item::Line(l) => match l.kind {
                ito_diff::Kind::Added => format!("  + {}", l.text),
                ito_diff::Kind::Removed => format!("  - {}", l.text),
                ito_diff::Kind::Same => format!("    {}", l.text),
            },
            ito_diff::Item::Folded(n) => format!("  … {n} 行"),
        })
        .collect();
    if out.len() > MOST {
        let rest = out.len() - MOST;
        out.truncate(MOST);
        out.push(format!("  … あと {rest} 行"));
    }
    out
}

/// `グループ/名前` as its two parts, or why it is not one.
fn split_id(id: &str) -> Result<(&str, &str), String> {
    if !koyomi_core::jobnet::is_id(id) {
        return Err(format!("グループ/名前 の形で書く: {id}"));
    }
    id.split_once('/').ok_or_else(|| format!("グループ/名前 の形で書く: {id}"))
}

/// How many days before the holiday table ends the window's band and `koyomi check` start to say so.
pub const CAUTION_DAYS: i64 = 90;

/// How often everything is looked at again even when the watcher is silent
/// (an access list changes without a file event).
const SWEEP: Duration = Duration::from_secs(5);
/// How long after the last file event the engine waits before it reads (an editor writes in bursts).
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Runs until `koyomi engine stop`. The exit code: 0 after a stop, 1 when another engine is already up.
pub fn run() -> i32 {
    let paths = match Paths::find() {
        Ok(p) => p,
        Err(e) => {
            crate::say(&format!("koyomi: {e}"), true);
            return 1;
        }
    };
    let door = proto::door();
    if ask(&door, &Request::Status).is_ok() {
        crate::say("koyomi: もう動いている", true);
        return 1;
    }
    let _ = std::fs::create_dir_all(&paths.jobnets);
    let _ = std::fs::create_dir_all(&paths.calendars);
    let rx = proto::door().listen(|| {});

    // `listen` quietly waits when another engine holds the door; ask until an engine answers, and
    // leave if it is not this one. Two engines started together are told apart here.
    let (mine_tx, mine_rx) = mpsc::channel::<bool>();
    std::thread::spawn(move || {
        for _ in 0..60 {
            if let Ok(text) = ask(&proto::door(), &Request::Status) {
                let _ = mine_tx.send(text.contains(&format!("pid {}:", std::process::id())));
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = mine_tx.send(false);
    });

    let dirty = Arc::new(AtomicBool::new(true));
    let watching = Arc::new(AtomicBool::new(true));
    let mut watcher = {
        let (dirty, watching) = (dirty.clone(), watching.clone());
        ito_watch::Watcher::start(move |ev| match ev {
            ito_watch::Event::Watching(on) => watching.store(on, Ordering::Relaxed),
            _ => dirty.store(true, Ordering::Relaxed),
        })
    };
    watcher.watch(vec![paths.jobnets.clone(), paths.calendars.clone()]);

    let mut state = State::new(paths);
    let mut last_scan = Instant::now();
    let mut changed_at: Option<Instant> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ask { request, reply, expired }) => {
                let stop = matches!(request, Request::Stop);
                let out = state.answer(&request);
                if !expired.load(Ordering::Relaxed) {
                    let _ = reply.send(out);
                }
                if stop {
                    state.runner.drop_queue();
                    state.runner.flush();
                    state.schedule.flush();
                    state.retain.flush();
                    state.backup.flush();
                    // The reply travels on the connection's thread.
                    std::thread::sleep(Duration::from_millis(150));
                    return 0;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return 1,
        }
        state.runner.tick();
        state.tick_schedule();
        state.tick_events();
        state.tick_retain();
        state.tick_backup();
        state.pump();
        if let Ok(false) = mine_rx.try_recv() {
            crate::say("koyomi: もう動いている", true);
            return 1;
        }
        if dirty.swap(false, Ordering::Relaxed) {
            changed_at = Some(Instant::now());
        }
        let quiet = changed_at.is_some_and(|t| t.elapsed() >= DEBOUNCE);
        if quiet || last_scan.elapsed() >= SWEEP {
            changed_at = None;
            last_scan = Instant::now();
            state.rescan();
        }
    }
}

/// The nets with a `[[when]] after_jobnet` naming `prev`'s net whose `on`
/// takes the way `prev` ended, each with that `[[when]]`.
fn followers(defs: &[Def], prev: &Run) -> Vec<(Def, when::When)> {
    let mut out = Vec::new();
    for d in defs {
        let Some(net) = &d.net else { continue };
        for w in &net.when {
            if let Kind::AfterJobnet { jobnet, on } = &w.kind {
                if *jobnet == prev.jobnet && on.lets(prev.state) {
                    out.push((d.clone(), w.clone()));
                }
            }
        }
    }
    out
}

/// The run of `id` that already started after another net on the business day `date`, if any.
fn followed<'a>(runs: &'a [Run], id: &str, date: &str) -> Option<&'a Run> {
    runs.iter().find(|r| r.jobnet == id && r.how == How::After && r.date == date)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_net_after_another_follows_the_ends_its_on_names_once_a_business_day() {
        let text = "format = 1\n[[when]]\nafter_jobnet = \"経理/売上取込\"\non = \"error\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        let net = koyomi_core::jobnet::parse(text).unwrap();
        let def = Def {
            id: "経理/集計".to_owned(),
            group: "経理".to_owned(),
            name: "集計".to_owned(),
            title: None,
            file: "経理/集計.toml".to_owned(),
            standing: Standing::Confirmed,
            problems: Vec::new(),
            net: Some(net.clone()),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        };
        let now = Local::now().naive_local();
        let mut prev = Run::new("p1", "経理/売上取込", "売上取込", "2026-10-29", How::Rerun, &net, now);
        prev.state = RunState::Error;
        let f = followers(std::slice::from_ref(&def), &prev);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].0.id, "経理/集計");
        prev.state = RunState::Ok;
        assert!(followers(std::slice::from_ref(&def), &prev).is_empty());
        let mut other = Run::new("p2", "経理/他", "他", "2026-10-29", How::Manual, &net, now);
        other.state = RunState::Error;
        assert!(followers(std::slice::from_ref(&def), &other).is_empty());
        let mut done = Run::new("f1", "経理/集計", "集計", "2026-10-29", How::After, &net, now);
        assert_eq!(followed(std::slice::from_ref(&done), "経理/集計", "2026-10-29").map(|r| r.id.as_str()), Some("f1"));
        assert!(followed(std::slice::from_ref(&done), "経理/集計", "2026-10-30").is_none());
        done.how = How::Manual;
        assert!(followed(std::slice::from_ref(&done), "経理/集計", "2026-10-29").is_none());
    }

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("koyomi-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = root.join("app");
        let state = root.join("state");
        std::fs::create_dir_all(app.join("jobnets")).unwrap();
        std::fs::create_dir_all(app.join("calendars")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        Paths {
            jobnets: app.join("jobnets"),
            calendars: app.join("calendars"),
            config: app.join("koyomi.toml"),
            approved: state.join("approved.toml"),
            held: state.join("held.toml"),
            app,
            state,
        }
    }

    fn standing_of(s: &State, id: &str) -> Option<Standing> {
        s.defs.iter().find(|d| d.id == id).map(|d| d.standing)
    }

    /// 30.4: a difference without a choice is refused and nothing is written.
    /// 30.5: overwrite keeps the old file as a version; the net comes in unconfirmed.
    /// 30.6: rename adds beside, skip leaves; a broken bundle is refused whatever the choice.
    #[test]
    fn an_import_is_planned_decided_kept_and_comes_in_unconfirmed() {
        let paths = temp_paths("import");
        let mut s = State::new(paths.clone());
        let one = "format = 1\nname = \"一\"\n\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let two = "format = 1\nname = \"二\"\n\n[[job]]\nid = \"a\"\nrun = [\"y.exe\"]\n";
        assert!(s.save("経理/集計", one, None, false).is_ok());
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        let mut b = bundle::Bundle::new();
        b.jobnets.push(bundle::Net { id: "経理/集計".to_owned(), text: two.to_owned() });
        b.jobnets.push(bundle::Net { id: "経理/新規".to_owned(), text: one.to_owned() });
        b.calendars.push(bundle::Named { name: "会社".to_owned(), text: "format = 1\nholidays = \"japan\"\n".to_owned() });
        let text = b.to_toml();
        let plan = |s: &mut State, dry: bool, on: Option<&str>| proto::Plan::parse(&s.import(&text, dry, on).unwrap()).unwrap();
        // A dry run only says. The net that differs makes the plan undecided.
        let p = plan(&mut s, true, None);
        assert!(!p.applied && p.undecided && p.writes && p.conflicts == 1 && p.problems == 0, "{p:?}");
        assert!(p.lines.iter().any(|l| l.contains("経理/集計")) && p.lines.iter().any(|l| l.contains("- name = \"一\"")), "{:?}", p.lines);
        // Without a choice nothing is done, dry run or not.
        let p = plan(&mut s, false, None);
        assert!(!p.applied && p.undecided, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), one);
        assert!(!paths.jobnets.join("経理/新規.toml").exists());
        // Something wrong in the file: refused whatever the choice.
        assert!(s.import("format = 1\n", false, Some("overwrite")).unwrap_err().contains("何も入っていない"));
        assert!(s.import("nope", false, None).unwrap_err().starts_with("取り込めない"));
        let mut broken = bundle::Bundle::new();
        broken.jobnets.push(bundle::Net {
            id: "経理/親".to_owned(),
            text: "format = 1\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"無い/子\"\n".to_owned(),
        });
        let p = proto::Plan::parse(&s.import(&broken.to_toml(), false, Some("overwrite")).unwrap()).unwrap();
        assert!(!p.applied && p.problems == 1, "{p:?}");
        assert!(!paths.jobnets.join("経理/親.toml").exists());
        assert!(s.import(&text, false, Some("sideways")).is_err());
        // Skip: the one that differs stays, the rest comes in.
        let p = plan(&mut s, false, Some("skip"));
        assert!(p.applied, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), one);
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/新規.toml")).unwrap(), one);
        assert!(paths.calendars.join("会社.toml").exists());
        assert_eq!(standing_of(&s, "経理/新規"), Some(Standing::Unconfirmed));
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        // Overwrite: the old file is kept as a version and the net is unconfirmed again.
        let p = plan(&mut s, false, Some("overwrite"));
        assert!(p.applied, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), two);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Unconfirmed));
        let versions = Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions;
        assert_eq!(versions.len(), 1, "{versions:?}");
        assert_eq!(std::fs::read_to_string(paths.state.join("versions/経理/集計").join(format!("{}.toml", versions[0]))).unwrap(), one);
        // The same again: nothing to write.
        let p = plan(&mut s, false, Some("overwrite"));
        assert!(!p.applied && !p.writes && p.conflicts == 0, "{p:?}");
        // Rename: a different one comes in beside what is there.
        b.jobnets[0].text = one.to_owned();
        let p = proto::Plan::parse(&s.import(&b.to_toml(), false, Some("rename")).unwrap()).unwrap();
        assert!(p.applied, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), two);
        let names: Vec<String> = std::fs::read_dir(paths.jobnets.join("経理"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_save_is_checked_kept_and_confirmed_and_a_restore_is_not() {
        let paths = temp_paths("save");
        let mut s = State::new(paths.clone());
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let one = "format = 1\nname = \"一\"\n\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let two = "format = 1\nname = \"二\"\n\n[[job]]\nid = \"a\"\nrun = [\"y.exe\"]\n";
        // A new net: nothing to be based on.
        assert_eq!(s.save("経理/集計", one, None, false), Ok("保存した: 経理/集計".to_owned()));
        let file = paths.jobnets.join("経理").join("集計.toml");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 0);
        // Saved again on a stale base: refused; on the right base: the old file is kept.
        let hash = approved::hash(one.as_bytes());
        assert_eq!(s.save("経理/集計", two, Some("stale"), false), Err("ほかで変わっています: 経理/集計".to_owned()));
        assert_eq!(s.save("経理/集計", two, None, false), Err("ほかで変わっています: 経理/集計".to_owned()));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert!(s.save("経理/集計", two, Some(&hash), false).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        let versions = Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions;
        assert_eq!(versions.len(), 1);
        assert_eq!(std::fs::read_to_string(paths.state.join("versions/経理/集計").join(format!("{}.toml", versions[0]))).unwrap(), one);
        // What does not read is not saved, whatever the base.
        let err = s.save("経理/集計", "format = 1\n[[job]]\nid = \"a\"\n", Some("x"), true).unwrap_err();
        assert!(err.starts_with("保存できない:"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        // A loop through a child net is caught against the other nets too.
        let child = "format = 1\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"経理/集計\"\n";
        assert!(s.save("経理/子", child, None, false).is_ok());
        let looped = "format = 1\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"経理/子\"\n";
        let err = s.save("経理/集計", looped, None, true).unwrap_err();
        assert!(err.contains("輪") || err.contains("子ジョブネット"), "{err}");
        // Not a group/name: refused before anything is touched.
        assert!(s.save("集計", two, None, true).is_err());
        assert!(s.save("../x/y", two, None, true).is_err());
        // A restore puts the version back, keeps today's, and leaves the net unconfirmed.
        assert_eq!(s.restore("経理/集計", "20000101-000000"), Err("版がない: 経理/集計 の 20000101-000000".to_owned()));
        assert!(s.restore("経理/集計", "../../approved").is_err());
        assert!(s.restore("経理/集計", "nope").is_err());
        let said = s.restore("経理/集計", &versions[0]).unwrap();
        assert!(said.contains("未確認"), "{said}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Unconfirmed));
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 2);
        // Forced over a hand edit: saved, and the hand edit kept as a version.
        std::fs::write(&file, "format = 1\n").unwrap();
        assert!(s.save("経理/集計", two, Some(&hash), false).is_err());
        assert!(s.save("経理/集計", two, Some(&hash), true).is_ok());
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 3);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_snapshot_survives_the_door() {
        let net =
            "format = 1\nname = \"月次締め\"\ncalendar = \"会社\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\", \"-v\"]\npos = [1.0, 2.0]\n";
        let dir = std::env::temp_dir().join(format!("koyomi-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("経理")).unwrap();
        std::fs::write(dir.join("経理/月次締め.toml"), net).unwrap();
        std::fs::create_dir_all(dir.join("共通")).unwrap();
        std::fs::write(dir.join("共通/壊れ.toml"), "format = 1\nrnu = 1\n").unwrap();
        let cal_dir = std::env::temp_dir().join(format!("koyomi-snap-cals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cal_dir);
        std::fs::create_dir_all(&cal_dir).unwrap();
        std::fs::write(
            cal_dir.join("会社.toml"),
            "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [2026-11-20, { date = 2026-12-29, name = \"年末\" }]\nopen = [{ date = 2026-11-21, name = \"棚卸\" }]\n",
        )
        .unwrap();
        let cals = Calendars::load(&cal_dir);
        let snap = Snapshot {
            defs: scan_in(&dir, &Store::default(), &cals),
            warnings: vec!["w".into()],
            notes: vec!["n".into()],
            held: vec!["共通/x".into()],
            paused: true,
            cautions: vec!["c".into()],
            calendars: cals.map.clone(),
        };
        let back = Snapshot::parse(&snap.to_toml()).unwrap();
        assert_eq!(back, snap);
        assert_eq!(back.defs.len(), 2);
        assert_eq!(back.defs[1].calendar.as_deref(), Some("会社"));
        let cal = &back.calendars["会社"];
        assert_eq!(cal.closed.len(), 2);
        assert_eq!(cal.open.values().next().map(String::as_str), Some("棚卸"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&cal_dir);
    }
}
