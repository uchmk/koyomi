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
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use ito_a11y::Role;
use koyomi_core::run::{Run, State};
use serde::Deserialize;

use super::graph;
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
        }
    }

    /// Reads the `[[remote]]` names off the UI thread.
    pub fn start(ctx: &egui::Context) -> Far {
        let far = Far::idle(ctx);
        let (tx, ctx) = (far.tx.clone(), far.ctx.clone());
        std::thread::spawn(move || {
            if let Ok(names) = crate::remote::names() {
                let _ = tx.send(Got::Names(names));
                ctx.request_repaint();
            }
        });
        far
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
    fn drain(&mut self) {
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
        if pc == self.on {
            return;
        }
        let names = std::mem::take(&mut self.names);
        let (tx, rx, ctx) = (self.tx.clone(), std::mem::replace(&mut self.rx, mpsc::channel().1), self.ctx.clone());
        *self = Far { names, on: pc, tx, rx, ..Far::idle(&ctx) };
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
}
