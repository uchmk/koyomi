//! The facts the run page shows about why a run is what it is: a run that never began, the definition
//! of the picked job as the run began with it (read-only), the approval that waits, the calendar the
//! run went by and the 対応済み chip. The words are built by plain functions so a test reads them without a window.

use std::collections::BTreeSet;
use std::path::Path;

use ito_a11y::Role;
use ito_i18n::Table;
use koyomi_core::jobnet::{self, Job, Jobnet, Missed, Part, Wait};
use koyomi_core::run::{Run, State, WentBy};

use super::{paths, Browser};
use crate::proto::Request;
use crate::{review, show};

/// What the window knows of a run's `definition.toml`.
#[derive(Clone, Debug)]
pub enum DefCopy {
    /// Asked for; the reply is not here yet.
    Asked,
    /// The reply: the parsed net, or why there is none (the engine's refusal, or a parse failure).
    Got(Result<Box<Jobnet>, String>),
}

impl DefCopy {
    /// The engine's reply to `RunDefinition`, parsed with the same parser the snapshot uses.
    pub fn of(reply: Result<String, String>) -> DefCopy {
        DefCopy::Got(match reply {
            Ok(text) => jobnet::parse(&text)
                .map(Box::new)
                .map_err(|problems| problems.iter().map(|p| p.message.clone()).collect::<Vec<_>>().join(" ")),
            Err(e) => Err(e),
        })
    }
}

/// The run's net as it is today.
#[derive(Clone, Debug, PartialEq)]
pub enum NetNow {
    /// Not in the engine's list any more.
    Gone,
    /// In the list: its `missed` (none when its file does not parse) and whether it is on hold.
    There { missed: Option<Missed>, held: bool },
}

impl NetNow {
    /// Whether 最初から再実行する can go: the net is there and not on hold.
    pub fn can_rerun(&self) -> bool {
        matches!(self, NetNow::There { held: false, .. })
    }
}

/// The explanation under the header of a run that never began, one line each.
pub fn never_began_lines(t: &Table, run: &Run, now: &NetNow) -> Vec<String> {
    let what = match run.state {
        State::Missed => "run.never_missed",
        State::Skipped => "run.never_skipped",
        State::Held => "run.never_held",
        _ => "run.never_locked",
    };
    let mut out = vec![t.tr(what).to_owned()];
    // What the engine wrote down, as it stands (a Locked run says what held it).
    let reason = koyomi_core::run::one_line(&run.reason);
    if !reason.is_empty() {
        out.push(t.trf("run.never_reason", &[&reason]));
    }
    if let (State::Missed, NetNow::There { missed: Some(m), .. }) = (run.state, now) {
        out.push(t.tr(if *m == Missed::Once { "run.never_missed_once" } else { "run.never_missed_skip" }).to_owned());
    }
    out.push(match now {
        NetNow::Gone => t.tr("run.never_can_gone").to_owned(),
        NetNow::There { held: true, .. } => t.tr("run.never_can_held").to_owned(),
        NetNow::There { .. } => t.trf("run.never_can", &[&run.date]),
    });
    out
}

/// What the definition section shows for the picked job.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    NeverBegan,
    NeedsEdit,
    Loading,
    /// The engine's words, as they came.
    Refused(String),
    /// A job inside a child net: only the parent's text is kept; this is the parent job's `use`.
    Child(String),
    /// The job is not in the copy (renamed since).
    Missing,
    Shown(Vec<Row>),
}

/// Picks what to show for `job_id` of `run`.
pub fn view(t: &Table, run: &Run, job_id: &str, needs_edit: bool, copy: Option<&DefCopy>) -> View {
    if run.never_began() {
        return View::NeverBegan;
    }
    if needs_edit {
        return View::NeedsEdit;
    }
    let net = match copy {
        None | Some(DefCopy::Asked) => return View::Loading,
        Some(DefCopy::Got(Err(e))) => return View::Refused(e.clone()),
        Some(DefCopy::Got(Ok(net))) => net,
    };
    if let Some(j) = net.jobs.iter().find(|j| j.id == job_id) {
        return View::Shown(rows(t, j));
    }
    // A job of a child net is `<parent job>/<its id>`; the parent's station job keeps the plain id.
    let parent = job_id.split_once('/').map(|(p, _)| p);
    match parent.and_then(|p| net.jobs.iter().find(|j| j.id == p)).map(|j| &j.part) {
        Some(Part::Jobnet { use_ }) => View::Child(use_.clone()),
        _ => View::Missing,
    }
}

/// One line of the definition: its words and its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub mono: bool,
    /// A working folder worth the open buttons.
    pub folder: Option<String>,
}

/// The job as written, in the review's words.
pub fn rows(t: &Table, j: &Job) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    put(&mut out, t.tr("run.def_kind"), t.tr(&format!("part.{}", j.part.key())).to_owned(), false);
    match &j.part {
        Part::Job => {}
        Part::Loop { file, count, max } => {
            if let Some(f) = file {
                put(&mut out, t.tr("review.loop_files"), f.clone(), true);
            }
            if let Some(n) = count {
                put(&mut out, t.tr("review.loop_count"), t.trf("review.loop_times", &[&n.to_string()]), false);
            }
            if let Some(n) = max {
                put(&mut out, t.tr("review.loop_max"), n.to_string(), false);
            }
        }
        Part::Branch | Part::Notify => {}
        Part::Approval { message } => {
            if let Some(m) = message.as_deref().filter(|m| !m.is_empty()) {
                put(&mut out, t.tr("review.message"), m.to_owned(), false);
            }
        }
        Part::Join { wait } => {
            put(
                &mut out,
                t.tr("review.wait"),
                t.tr(if *wait == Wait::All { "review.wait_all" } else { "review.wait_any" }).to_owned(),
                false,
            );
        }
        Part::Time { at, days } => {
            let later = if *days > 0 { t.trf("review.days", &[&days.to_string()]) } else { String::new() };
            put(&mut out, t.tr("review.at"), format!("{}{later}", at.format("%H:%M")), true);
        }
        Part::File { file, settle } => {
            put(&mut out, t.tr("review.file_wait"), file.clone(), true);
            if let Some(s) = settle {
                put(&mut out, t.tr("review.settle"), koyomi_core::duration::text(*s), false);
            }
        }
        Part::Jobnet { use_ } => put(&mut out, t.tr("review.use"), use_.clone(), true),
        Part::Chore(c) => {
            for (label, value) in review::chore_pairs(t, c) {
                put(&mut out, &label, value, false);
            }
        }
    }
    if matches!(j.part, Part::Job | Part::Loop { .. }) {
        if let (Some(shell), Some(script)) = (&j.shell, &j.script) {
            put(&mut out, t.tr("review.shell"), shell.clone(), true);
            put(&mut out, t.tr("review.script"), script.clone(), true);
        } else {
            put(&mut out, t.tr("review.exe"), j.run.first().cloned().unwrap_or_default(), true);
            for (i, a) in j.run.iter().skip(1).enumerate() {
                put(&mut out, &format!("{} {}", t.tr("review.arg"), i + 1), a.clone(), true);
            }
        }
        if let Some(dir) = j.dir.as_deref().filter(|d| !d.is_empty()) {
            let folder = (Path::new(dir).is_absolute() && !dir.contains('{')).then(|| dir.to_owned());
            put(&mut out, t.tr("review.dir"), dir.to_owned(), true);
            if let Some(last) = out.last_mut() {
                last.folder = folder;
            }
        }
        // As written: a `secret:<name>` stays a reference, never its value.
    }
    for (k, v) in &j.env {
        out.push(Row { label: format!("{} {k}", t.tr("review.env")), value: v.clone(), mono: true, folder: None });
    }
    if let Some(secs) = j.timeout {
        out.push(Row { label: t.tr("review.timeout").to_owned(), value: koyomi_core::duration::text(secs), mono: false, folder: None });
    }
    if let Some(r) = j.retry {
        let text = t.trf("review.retry_text", &[&r.count.to_string(), &koyomi_core::duration::text(r.wait)]);
        out.push(Row { label: t.tr("review.retry").to_owned(), value: text, mono: false, folder: None });
    }
    if let Some(n) = j.note.as_deref().filter(|n| !n.is_empty()) {
        out.push(Row { label: t.tr("field.note").to_owned(), value: n.to_owned(), mono: false, folder: None });
    }
    out
}

fn put(out: &mut Vec<Row>, label: &str, value: String, mono: bool) {
    out.push(Row { label: label.to_owned(), value, mono, folder: None });
}

/// The jobs right before an approval and its message.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub message: Option<String>,
    pub before: Vec<AskRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AskRow {
    pub id: String,
    pub name: String,
    pub state: State,
    pub exit: Option<i32>,
}

/// What the person decides on at the approval `target` of `run`.
pub fn ask(run: &Run, target: &str) -> Option<Ask> {
    let job = run.job(target)?;
    let Part::Approval { message } = &job.part else { return None };
    let before = job
        .after
        .iter()
        .filter_map(|l| run.job(&l.job))
        .map(|b| AskRow { id: b.id.clone(), name: b.named(), state: b.state, exit: b.exit })
        .collect();
    Some(Ask { message: message.clone().filter(|m| !m.trim().is_empty()), before })
}

/// One job before the approval: name, state and exit code.
pub fn ask_row(t: &Table, r: &AskRow) -> String {
    let mut s = format!("{}  {}", r.name, show::tag(t, r.state));
    if let Some(x) = r.exit {
        s.push_str(&format!("  {}", t.trf("run.ask_exit", &[&x.to_string()])));
    }
    s
}

/// `カレンダー: 会社` / `カレンダー: なし（毎日）`; nothing for a run from before this was recorded.
pub fn calendar_words(t: &Table, went_by: Option<&WentBy>) -> Option<String> {
    match went_by? {
        WentBy::Calendar(name) => Some(t.trf("run.calendar_head", &[name])),
        WentBy::Every => Some(t.tr("run.calendar_none").to_owned()),
    }
}

/// The chip in the header of a run that was marked 対応済み.
pub fn acked_chip(t: &Table, acked: &BTreeSet<String>, run: &Run) -> Option<String> {
    acked.contains(&run.id).then(|| t.tr("run.acked_chip").to_owned())
}

/// The result line after 対応済みにする.
pub fn acked_said(t: &Table, run: &Run) -> String {
    t.trf("run.acked_said", &[&run.title, &run.id, &show::tag(t, run.state)])
}

impl Browser {
    /// The run's net in the engine's list now.
    pub(super) fn net_now(&self, run: &Run) -> NetNow {
        let Some(snap) = &self.snap else { return NetNow::Gone };
        match snap.defs.iter().find(|d| d.id == run.jobnet) {
            None => NetNow::Gone,
            Some(d) => NetNow::There { missed: d.net.as_ref().map(|n| n.missed), held: snap.held.contains(&d.id) },
        }
    }

    /// The caller is known to be below Edit on another PC: the engine would refuse the copy.
    fn below_edit(&self) -> bool {
        self.link.is_far() && self.role.is_some_and(|r| !r.allows(koyomi_core::role::Role::Edit))
    }

    /// Asks for the run's definition copy, once per run.
    fn ask_definition(&mut self, run: &Run) {
        if run.never_began() || self.below_edit() || self.run_defs.contains_key(&run.id) {
            return;
        }
        self.run_defs.insert(run.id.clone(), DefCopy::Asked);
        if !self.send(Request::RunDefinition { run: run.id.clone() }) {
            let why = self.unsent_why();
            self.run_defs.insert(run.id.clone(), DefCopy::Got(Err(why)));
        }
    }

    /// The explanation of a run that never began.
    pub(super) fn never_began_block(&self, ui: &mut egui::Ui, t: &Table, run: &Run) {
        let c = ito_theme::colors();
        ui.add_space(4.0);
        for l in never_began_lines(t, run, &self.net_now(run)) {
            let r = ui.add(egui::Label::new(egui::RichText::new(&l).color(c.dim)).wrap());
            ito_a11y::say(&r, Role::Label, &l);
        }
    }

    /// The calendar the run went by, and the chip of a run dealt with.
    pub(super) fn run_header_extra(&mut self, ui: &mut egui::Ui, t: &Table, run: &Run) {
        let c = ito_theme::colors();
        let calendar = calendar_words(t, run.went_by.as_ref());
        let chip = acked_chip(t, &self.acked, run);
        if calendar.is_none() && chip.is_none() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if let Some(words) = &calendar {
                match &run.went_by {
                    Some(WentBy::Calendar(name)) => {
                        let r = ui.link(words);
                        ito_a11y::say(&r, Role::Button, words);
                        if r.clicked() {
                            self.open_calendar(name);
                        }
                    }
                    _ => {
                        let r = ui.label(egui::RichText::new(words).small().color(c.dim));
                        ito_a11y::say(&r, Role::Label, words);
                    }
                }
            }
            if let Some(chip) = &chip {
                let r =
                    ito_prefs::tag(ui, ito_prefs::Look { c, q: "" }, ito_prefs::Tag::new(chip).color(super::graph::colour(&c, State::Ok)));
                ito_a11y::say(&r, Role::Label, chip);
            }
        });
    }

    /// What the person decides on at the approval that waits: its message and the jobs before it.
    pub(super) fn ask_block(&mut self, ui: &mut egui::Ui, t: &Table, run: &Run, target: &str) {
        let Some(info) = ask(run, target) else { return };
        let c = ito_theme::colors();
        ui.add_space(6.0);
        let head = t.tr("review.message");
        let r = ui.label(egui::RichText::new(head).strong());
        ito_a11y::say(&r, Role::Heading, head);
        match &info.message {
            Some(m) => {
                let r = ui.add(egui::Label::new(m.as_str()).wrap());
                ito_a11y::say(&r, Role::Label, m);
            }
            None => {
                let words = t.tr("run.ask_none");
                let r = ui.add(egui::Label::new(egui::RichText::new(words).color(c.dim)).wrap());
                ito_a11y::say(&r, Role::Label, words);
            }
        }
        if info.before.is_empty() {
            return;
        }
        let before = t.tr("run.ask_before");
        let r = ui.label(egui::RichText::new(before).strong());
        ito_a11y::say(&r, Role::Heading, before);
        for row in &info.before {
            ui.horizontal_wrapped(|ui| {
                let line = ask_row(t, row);
                let r = ui.label(&line);
                ito_a11y::say(&r, Role::Label, &line);
                let words = t.trf("run.ask_log_of", &[&row.name]);
                let r = ui.link(t.tr("run.ask_log"));
                ito_a11y::say(&r, Role::Button, &words);
                if r.clicked() {
                    self.job_sel = Some(row.id.clone());
                }
            });
        }
    }

    /// The picked job as the run began with it, read-only. Asks the engine for the copy once per run.
    pub(super) fn definition_section(&mut self, ui: &mut egui::Ui, t: &Table, run: &Run, job_id: &str) {
        let c = ito_theme::colors();
        self.ask_definition(run);
        let shown = view(t, run, job_id, self.below_edit(), self.run_defs.get(&run.id));
        ui.add_space(8.0);
        let head = t.tr("run.def_head");
        let r = ui.label(egui::RichText::new(head).strong());
        ito_a11y::say(&r, Role::Heading, head);
        let dim = |ui: &mut egui::Ui, words: &str| {
            let r = ui.add(egui::Label::new(egui::RichText::new(words).color(c.dim)).wrap());
            ito_a11y::say(&r, Role::Label, words);
        };
        match shown {
            View::NeverBegan => {
                dim(ui, t.tr("run.def_never"));
                let there = self.snap.as_ref().is_some_and(|s| s.defs.iter().any(|d| d.id == run.jobnet && d.net.is_some()));
                if there {
                    let open = ito_prefs::IconButton::new(ito_prefs::Mark::Pencil, t.tr("run.def_open_editor"));
                    if open.show(ui).clicked() {
                        self.open_editor(&run.jobnet);
                        self.focus = 0;
                    }
                }
            }
            View::NeedsEdit => dim(ui, t.tr("run.def_needs_edit")),
            View::Loading => dim(ui, t.tr("users.loading")),
            View::Refused(why) => dim(ui, &why),
            View::Child(use_) => {
                dim(ui, t.tr("run.def_child"));
                dim(ui, &format!("{}: {use_}", t.tr("review.use")));
            }
            View::Missing => dim(ui, t.tr("run.def_missing")),
            View::Shown(rows) => {
                let local = !self.link.is_far();
                egui::Grid::new("job_def").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                    for row in &rows {
                        ui.label(egui::RichText::new(&row.label).color(c.dim));
                        ui.horizontal_wrapped(|ui| {
                            let text = if row.mono { egui::RichText::new(&row.value).monospace() } else { egui::RichText::new(&row.value) };
                            let r = ui.add(egui::Label::new(text).wrap());
                            ito_a11y::say(&r, Role::Label, &format!("{}: {}", row.label, row.value));
                            if let Some(folder) = &row.folder {
                                paths::path_buttons(ui, t, Path::new(folder), paths::Place::Engine, local);
                            }
                        });
                        ui.end_row();
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::jobnet::{Job, Line};
    use koyomi_core::run::{How, Outcome};

    fn table() -> Table {
        crate::i18n::table("ja").0
    }

    fn at() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(9, 0, 0).unwrap()
    }

    fn job(id: &str, after: &[&str]) -> Job {
        Job { id: id.into(), run: vec!["x.exe".into()], after: after.iter().map(|s| Line::ok(s)).collect(), ..Default::default() }
    }

    fn never(state: State, reason: &str) -> Run {
        let net = Jobnet { jobs: vec![job("a", &[])], ..Default::default() };
        let mut run = Run::new("20261030-090000-01", "試験/bom", "bom", "2026-10-30", How::Manual, &net, at());
        run.state = state;
        run.ended = Some(at());
        run.reason = reason.into();
        run
    }

    #[test]
    fn a_missed_run_says_what_the_net_does_now() {
        let t = table();
        let run = never(State::Missed, "");
        let skip = never_began_lines(&t, &run, &NetNow::There { missed: Some(Missed::Skip), held: false });
        assert_eq!(skip[1], "いまの設定（missed = \"skip\"）では、時刻を過ぎた回を後から実行しません。");
        assert_eq!(skip[2], "再実行すると、いまの定義でこの日（2026-10-30）の回を最初から始めます。");
        let once = never_began_lines(&t, &run, &NetNow::There { missed: Some(Missed::Once), held: false });
        assert!(once[1].contains("once") && once[1].contains("1 回だけ遅れて実行します"), "{once:?}");
        // The net is gone: no setting, and it cannot be rerun.
        let gone = never_began_lines(&t, &run, &NetNow::Gone);
        assert_eq!(gone.len(), 2, "{gone:?}");
        assert!(gone[1].contains("もう無い"), "{gone:?}");
    }

    #[test]
    fn a_locked_run_uses_its_stored_reason_and_a_held_net_must_be_released() {
        let t = table();
        let run = never(State::Locked, "経理/締め が lock=db を持っていた");
        let lines = never_began_lines(&t, &run, &NetNow::There { missed: Some(Missed::Skip), held: false });
        assert!(lines.iter().any(|l| l == "理由: 経理/締め が lock=db を持っていた"), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("missed")), "only a Missed run names the setting: {lines:?}");
        let held = never_began_lines(&t, &never(State::Held, ""), &NetNow::There { missed: None, held: true });
        assert_eq!(held.last().unwrap(), "保留を解いてから再実行できます。");
        assert!(!NetNow::There { missed: None, held: true }.can_rerun() && !NetNow::Gone.can_rerun());
    }

    fn net_of(text: &str) -> DefCopy {
        DefCopy::of(Ok(text.to_owned()))
    }

    fn ran() -> Run {
        let net = Jobnet { jobs: vec![job("a", &[])], ..Default::default() };
        let mut run = Run::new("20261030-090000-01", "試験/bom", "bom", "2026-10-30", How::Manual, &net, at());
        run.start("a", at());
        run.finish("a", Outcome::Exited(0), at());
        run
    }

    const COPY: &str = "format = 1
[[job]]
id = \"a\"
run = [\"x.exe\", \"--all\"]
dir = 'C:\\work'
timeout = \"30m\"
env = { DB_PASS = \"secret:db\" }
note = \"毎日\"
[[job]]
id = \"s\"
shell = \"powershell\"
script = \"Get-Date\"
[[job]]
id = \"ok\"
kind = \"approval\"
message = \"本番に反映してよいか\"
[[job]]
id = \"end\"
run = [\"x.exe\"]
after = [\"ok\"]
[[job]]
id = \"kid\"
kind = \"jobnet\"
use = \"試験/子\"
";

    #[test]
    fn the_definition_shows_a_command_a_shell_and_an_approval() {
        let t = table();
        let run = ran();
        let copy = net_of(COPY);
        let View::Shown(rows) = view(&t, &run, "a", false, Some(&copy)) else { panic!("a is in the copy") };
        let get = |l: &str| rows.iter().find(|r| r.label == l).unwrap_or_else(|| panic!("{l}: {rows:?}"));
        assert_eq!(get("実行").value, "x.exe");
        assert_eq!(get("作業フォルダー").folder.as_deref(), Some("C:\\work"));
        assert!(rows.iter().any(|r| r.label.starts_with("環境変数 DB_PASS") && r.value == "secret:db"), "{rows:?}");
        assert!(rows.iter().any(|r| r.value == "毎日"), "the note: {rows:?}");
        let View::Shown(shell) = view(&t, &run, "s", false, Some(&copy)) else { panic!() };
        assert!(shell.iter().any(|r| r.value == "powershell") && shell.iter().any(|r| r.value == "Get-Date"), "{shell:?}");
        let View::Shown(ok) = view(&t, &run, "ok", false, Some(&copy)) else { panic!() };
        assert!(ok.iter().any(|r| r.label == "判断の材料" && r.value == "本番に反映してよいか"), "{ok:?}");
    }

    #[test]
    fn the_definition_has_words_when_there_is_nothing_to_show() {
        let t = table();
        let run = ran();
        let copy = net_of(COPY);
        assert_eq!(view(&t, &run, "kid/x", false, Some(&copy)), View::Child("試験/子".into()));
        assert_eq!(view(&t, &run, "gone", false, Some(&copy)), View::Missing);
        assert_eq!(view(&t, &run, "a", false, None), View::Loading);
        assert_eq!(view(&t, &run, "a", true, Some(&copy)), View::NeedsEdit);
        let refused = DefCopy::of(Err("E3290 gone".into()));
        assert_eq!(view(&t, &run, "a", false, Some(&refused)), View::Refused("E3290 gone".into()));
        assert_eq!(view(&t, &never(State::Missed, ""), "a", false, Some(&copy)), View::NeverBegan);
    }

    #[test]
    fn an_approval_shows_the_jobs_before_it_and_says_when_there_is_no_message() {
        let t = table();
        let appr = Job { part: Part::Approval { message: None }, ..job("ok", &["a", "b"]) };
        let net = Jobnet { jobs: vec![job("a", &[]), job("b", &[]), appr], ..Default::default() };
        let mut run = Run::new("20261030-090000-01", "試験/bom", "bom", "2026-10-30", How::Manual, &net, at());
        run.start("a", at());
        run.finish("a", Outcome::Exited(0), at());
        run.start("b", at());
        run.finish("b", Outcome::Exited(1), at());
        run.start("ok", at());
        let info = ask(&run, "ok").expect("an approval");
        assert_eq!(info.message, None);
        let rows: Vec<String> = info.before.iter().map(|r| ask_row(&t, r)).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].starts_with("a  ") && rows[1].starts_with("b  ") && rows[1].contains("終了コード 1"), "{rows:?}");
        assert_eq!(t.tr("run.ask_none"), "判断の材料は書かれていません。ジョブネットの「確認待ち」に message を書けます。");
        assert!(ask(&run, "a").is_none());
        // With a message it is the message.
        let mut with = run.clone();
        with.jobs[2].part = Part::Approval { message: Some("反映してよいか".into()) };
        assert_eq!(ask(&with, "ok").unwrap().message.as_deref(), Some("反映してよいか"));
    }

    #[test]
    fn the_calendar_and_the_chip_have_their_words() {
        let t = table();
        assert_eq!(calendar_words(&t, Some(&WentBy::Calendar("会社".into()))).as_deref(), Some("カレンダー: 会社"));
        assert_eq!(calendar_words(&t, Some(&WentBy::Every)).as_deref(), Some("カレンダー: なし（毎日）"));
        assert_eq!(calendar_words(&t, None), None);
        let run = ran();
        let mut acked = BTreeSet::new();
        assert_eq!(acked_chip(&t, &acked, &run), None);
        acked.insert(run.id.clone());
        assert_eq!(acked_chip(&t, &acked, &run).as_deref(), Some("✓ 対応済み"));
        let said = acked_said(&t, &run);
        assert!(
            said.starts_with("bom の回 20261030-090000-01（")
                && said.ends_with("）を対応済みにしました。ステータスバーの異常の数から外れます。"),
            "{said}"
        );
    }
}
