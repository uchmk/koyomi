//! The pane at the right of the editor: the fields of the chosen part, or of the net, as a form in
//! sections. Each field says whether it must be filled in (a red `*`) or may stay empty (`(任意)`), and turns red
//! while it does not read.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Frame, Margin, RichText, Stroke};
use ito_a11y::Role;
use ito_i18n::Table;
use ito_prefs::{
    fieldset, form_label, mark_button, seg_named, tag, CodeBox, FormWords, IconButton, InputField, Look, Mark as M, Need, Tag, Tone,
};
use koyomi_core::calendar::Calendar;
use koyomi_core::chore::{Chore, Pass, SqlDb, SqlTls, Verify, WinrmAuth, GROUP_MAX};
use koyomi_core::jobnet::{self, Encoding, Missed, Overlap, Part, Wait};

use super::{Action, Cmd, Editor, Fields, NetFields, Pane};
use crate::show;
use crate::ui::whens::WhenEdit;

/// A note longer than this goes under the field, wrapped, instead of after the label.
const SHORT_NOTE: usize = 16;

/// What the form's rows share: the words, the colours and the required/optional words.
struct Form<'a> {
    t: &'a Table,
    look: Look<'static>,
    words: FormWords<'static>,
}

impl<'a> Form<'a> {
    fn new(t: &'a Table) -> Self {
        Form { t, look: Look { c: ito_theme::colors(), q: "" }, words: FormWords::of(t.tr("app.lang")) }
    }

    /// The label of `key` (its note in brackets split off) and how much it must be filled in.
    fn label(&self, ui: &mut egui::Ui, key: &'a str, need: Need) -> (&'a str, &'a str) {
        let (label, note) = split_label(self.t.tr(key));
        let short = note.chars().count() <= SHORT_NOTE;
        form_label(ui, self.look, label, need, if short { note } else { "" }, &self.words);
        ui.add_space(4.0);
        (label, if short { "" } else { note })
    }

    /// A long note under its field.
    fn note(&self, ui: &mut egui::Ui, note: &str) {
        if !note.is_empty() {
            let c = self.look.c;
            let size = ito_theme::zoomed(ui.ctx(), 11.0);
            let r = ui.add(egui::Label::new(RichText::new(note).size(size).color(ito_theme::mix(c.bg, c.dim, 0.8))).wrap());
            ito_a11y::say(&r, Role::Label, note);
        }
    }

    /// A labelled field. `bad`: it does not read as it is (an empty required field is bad too). True when it changed.
    fn line(
        &self,
        ui: &mut egui::Ui,
        key: &'a str,
        need: Need,
        bad: bool,
        s: &mut String,
        how: impl FnOnce(InputField<'a>) -> InputField<'a>,
    ) -> bool {
        let (label, note) = self.label(ui, key, need);
        let wrong = bad || (need == Need::Required && s.trim().is_empty());
        let r = how(InputField::new(key).name(label).error(wrong)).show(ui, s);
        self.note(ui, note);
        ui.add_space(10.0);
        r.changed()
    }

    /// A labelled box of code `rows` high, numbered by line; `aside` is said at its top right (the shell).
    fn code(&self, ui: &mut egui::Ui, key: &'a str, need: Need, bad: bool, s: &mut String, (rows, aside): (usize, &str)) -> bool {
        let (label, note) = self.label(ui, key, need);
        let wrong = bad || (need == Need::Required && s.trim().is_empty());
        let r = CodeBox::new(key).name(label).aside(aside).rows(rows).error(wrong).show(ui, s);
        self.note(ui, note);
        ui.add_space(10.0);
        r.changed()
    }

    /// A labelled row of choices; `note` under it (what the choice picked does). True when another was picked.
    fn choice<T: PartialEq + Copy>(&self, ui: &mut egui::Ui, key: &'a str, now: &mut T, options: &[(T, &str)], note: &str) -> bool {
        let (label, _) = self.label(ui, key, Need::Plain);
        let (to, _) = seg_named(ui, self.look, key, label, *now, options);
        self.note(ui, note);
        ui.add_space(10.0);
        match to {
            Some(v) if v != *now => {
                *now = v;
                true
            }
            _ => false,
        }
    }

    /// Two fields side by side, half the width each.
    fn pair(&self, ui: &mut egui::Ui, left: impl FnOnce(&mut egui::Ui) -> bool, right: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
        let mut changed = false;
        ui.columns(2, |cols| {
            changed |= left(&mut cols[0]);
            changed |= right(&mut cols[1]);
        });
        changed
    }
}

/// `"run（1 行に 1 つ）"` → `("run", "1 行に 1 つ")`; `"retry count"` stays whole. A bracket at the start is not a note.
fn split_label(s: &str) -> (&str, &str) {
    for (open, close) in [("（", "）"), (" (", ")")] {
        if let Some(inner) = s.strip_suffix(close) {
            if let Some(at) = inner.find(open).filter(|&at| at > 0) {
                return (inner[..at].trim_end(), &inner[at + open.len()..]);
            }
        }
    }
    (s, "")
}

/// What the head and the foot asked for, done after the pane is drawn.
#[derive(Default)]
struct Asked {
    fold: bool,
    close: bool,
    delete: bool,
    save: bool,
}

impl Editor {
    /// The pane, or its strip when folded. With no part opened it shows the net's fields.
    pub(super) fn drawer(
        &mut self,
        ui: &mut egui::Ui,
        t: &Table,
        (calendars, stops): (&BTreeMap<String, Calendar>, &[koyomi_core::blackout::Blackout]),
        power: &crate::proto::Power,
        action: &mut Option<Action>,
    ) {
        let c = ito_theme::colors();
        let gone = match &self.pane {
            Some(Pane::Part(id, _)) => self.index(id).is_none(),
            Some(Pane::Net(_)) => false,
            None => true,
        };
        if gone {
            self.pane = Some(Pane::Net(Box::new(NetFields::of(&self.net))));
        }
        let title = match &self.pane {
            Some(Pane::Part(id, _)) => id.clone(),
            _ => t.tr("canvas.net").to_owned(),
        };
        if self.fold {
            if crate::ui::folded_pane(ui, "fields_strip", &title, t.tr("inspect.open")) {
                self.fold = false;
            }
            return;
        }
        let Some(mut pane) = self.pane.take() else { return };
        let form = Form::new(t);
        let mut asked = Asked::default();
        let wide = ui.available_width();
        let room = Frame::new().inner_margin(Margin::symmetric(14, 10));
        egui::Panel::right("fields")
            .resizable(true)
            .default_size(360.0_f32.min(wide * 0.45))
            .size_range(280.0..=(wide * 0.6).max(280.0))
            .frame(Frame::new().fill(c.side).stroke(Stroke::new(1.0, c.border)))
            .show(ui, |ui| {
                crate::ui::one_size(ui.style_mut());
                ui.set_min_height(ui.available_height());
                egui::Panel::top("fields_head").frame(room).show(ui, |ui| self.drawer_head(ui, &form, &pane, &mut asked));
                egui::Panel::bottom("fields_foot").frame(room).show(ui, |ui| {
                    let msg = match &pane {
                        Pane::Part(_, f) => f.msg.as_deref(),
                        Pane::Net(_) => None,
                    };
                    if let Some(m) = msg {
                        let text = ito_a11y::State::Error.marked(m);
                        let r = ui.add(egui::Label::new(RichText::new(&text).color(c.err)).wrap());
                        ito_a11y::say(&r, Role::Label, &text);
                        ui.add_space(6.0);
                    }
                    ui.horizontal(|ui| {
                        if matches!(pane, Pane::Part(..)) {
                            asked.delete = IconButton::new(M::Trash, t.tr("canvas.delete_part")).tone(Tone::Danger).show(ui).clicked();
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            asked.save = crate::ui::primary(ui, self.can_save(), M::Save, t.tr("canvas.save")).clicked();
                        });
                    });
                });
                egui::CentralPanel::default().frame(Frame::new()).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("fields_scroll").auto_shrink(false).show(ui, |ui| {
                        Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| match &mut pane {
                            Pane::Part(id, f) => self.part_form(ui, &form, id, f, action),
                            Pane::Net(f) => self.net_form(ui, &form, f, (calendars, stops), power, action),
                        });
                    });
                });
            });
        let part = match &pane {
            Pane::Part(id, _) => Some(id.clone()),
            Pane::Net(_) => None,
        };
        self.pane = Some(pane);
        if asked.save {
            if let Some(a) = self.edit(Cmd::Save) {
                *action = Some(a);
            }
        }
        if let (true, Some(id)) = (asked.delete, part) {
            self.selected = BTreeSet::from([id]);
            self.delete();
        } else if asked.close {
            self.open_net();
        }
        if asked.fold {
            self.fold = true;
        }
    }

    /// What the pane is about (the kind of part, or the net) with ✕ and the fold mark at the right, then its id and name.
    fn drawer_head(&self, ui: &mut egui::Ui, form: &Form, pane: &Pane, asked: &mut Asked) {
        let (t, c) = (form.t, form.look.c);
        let (kind, id, name) = match pane {
            Pane::Part(id, f) => {
                let part = self.index(id).map(|i| self.net.jobs[i].part.key());
                let key = format!("part.{}", part.unwrap_or("job"));
                (t.tr(&key).to_owned(), id.as_str(), f.name.trim())
            }
            Pane::Net(f) => (t.tr("canvas.net").to_owned(), self.id.as_str(), f.name.trim()),
        };
        ui.horizontal(|ui| {
            let r = tag(ui, form.look, Tag::new(&kind).color(c.blue));
            ito_a11y::say(&r, Role::Label, &kind);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                asked.fold = mark_button(ui, M::ChevronRight, t.tr("inspect.fold"), 22.0).clicked();
                if matches!(pane, Pane::Part(..)) {
                    asked.close = mark_button(ui, M::Close, t.tr("canvas.close_pane"), 22.0).clicked();
                }
            });
        });
        ui.add_space(6.0);
        let r = ui.add(egui::Label::new(RichText::new(id).monospace().strong().size(16.0).color(c.strong())).wrap());
        ito_a11y::say(&r, Role::Heading, id);
        if !name.is_empty() {
            let r = ui.add(egui::Label::new(RichText::new(name).color(c.dim)).wrap());
            ito_a11y::say(&r, Role::Label, name);
        }
    }

    /// The fields of the part `id`, in sections by what they are for; a change goes into the net at once.
    fn part_form(&mut self, ui: &mut egui::Ui, form: &Form, id: &mut String, f: &mut Fields, action: &mut Option<Action>) {
        let Some(i) = self.index(id) else { return };
        let part = self.net.jobs[i].part.clone();
        let (t, look) = (form.t, form.look);
        let mut changed = false;
        fieldset(ui, look, t.tr("canvas.sec_basic"), "", |ui| {
            let bad = f.id.trim().contains(['/', '\\', '"', ' ']);
            changed |= form.line(ui, "field.id", Need::Required, bad, &mut f.id, |i| i.mono());
            changed |= form.line(ui, "field.name", Need::Optional, false, &mut f.name, |i| i);
            changed |= form.line(ui, "field.note", Need::Optional, false, &mut f.note, |i| i.rows(2));
        });
        match &part {
            Part::Job | Part::Loop { .. } => {
                if matches!(part, Part::Loop { .. }) {
                    changed |= fieldset(ui, look, t.tr("canvas.sec_loop"), "", |ui| loop_rows(ui, form, f));
                }
                changed |= fieldset(ui, look, t.tr("canvas.sec_run"), "", |ui| run_rows(ui, form, f));
                changed |= fieldset(ui, look, t.tr("canvas.sec_retry"), "", |ui| retry_rows(ui, form, f));
                changed |= fieldset(ui, look, t.tr("canvas.sec_judge"), form.words.optional, |ui| judge_rows(ui, form, f));
                changed |= fieldset(ui, look, t.tr("canvas.sec_account"), "", |ui| account_rows(ui, form, f));
            }
            Part::Join { .. } => {
                changed |= fieldset(ui, look, t.tr("canvas.sec_detail"), "", |ui| {
                    let options = [(Wait::All, t.tr("review.wait_all")), (Wait::Any, t.tr("review.wait_any"))];
                    form.choice(ui, "review.wait", &mut f.wait, &options, "")
                });
            }
            Part::Time { .. } => {
                changed |= fieldset(ui, look, t.tr("canvas.sec_detail"), "", |ui| {
                    let mut changed = form.line(ui, "review.at", Need::Required, false, &mut f.at, |i| i.mono().hint("09:00").width(120.0));
                    changed |= form.line(ui, "field.days", Need::Optional, false, &mut f.days, |i| i.width(120.0).hint("0"));
                    changed
                });
            }
            Part::File { .. } => {
                changed |= fieldset(ui, look, t.tr("canvas.sec_detail"), "", |ui| {
                    let mut changed = form.line(ui, "review.file_wait", Need::Required, false, &mut f.file, |i| i.mono());
                    changed |= form.line(ui, "review.settle", Need::Optional, false, &mut f.settle, |i| i.hint("30s").width(120.0));
                    changed
                });
            }
            Part::Jobnet { .. } => {
                changed |= fieldset(ui, look, t.tr("canvas.sec_detail"), "", |ui| {
                    let bad = !f.use_.trim().is_empty() && !jobnet::is_id(f.use_.trim());
                    let changed = form.line(ui, "review.use", Need::Required, bad, &mut f.use_, |i| i.mono().hint("経理/月次"));
                    if IconButton::new(M::ArrowRight, t.tr("canvas.enter")).show(ui).clicked() && jobnet::is_id(f.use_.trim()) {
                        *action = Some(Action::Enter(f.use_.trim().to_owned()));
                    }
                    changed
                });
            }
            Part::Chore(chore) => {
                changed |= fieldset(ui, look, t.tr("canvas.sec_detail"), "", |ui| chore_rows(ui, form, chore, f));
                changed |= fieldset(ui, look, t.tr("canvas.sec_retry"), "", |ui| retry_rows(ui, form, f));
                if chore.far() {
                    changed |= fieldset(ui, look, t.tr("canvas.sec_judge"), form.words.optional, |ui| judge_rows(ui, form, f));
                }
                // A `mail` is sent as koyomi's own account, with its settings.
                if !matches!(chore, Chore::Mail { .. }) {
                    changed |= fieldset(ui, look, t.tr("canvas.sec_account"), "", |ui| account_rows(ui, form, f));
                }
            }
            Part::Branch | Part::Notify | Part::Approval => {}
        }
        if changed {
            let before = f.msg.clone();
            f.msg = self.apply_fields(id, f).err();
            // A rename that went through: the pane follows the part.
            let new = f.id.trim().to_owned();
            if f.msg.is_none() && new != *id && self.has(&new) {
                *id = new;
            }
            if f.msg != before {
                ui.ctx().request_repaint();
            }
        }
    }

    /// The net's fields: its name and calendar, how it behaves, when it starts, and its next starts.
    fn net_form(
        &mut self,
        ui: &mut egui::Ui,
        form: &Form,
        f: &mut NetFields,
        (calendars, stops): (&BTreeMap<String, Calendar>, &[koyomi_core::blackout::Blackout]),
        power: &crate::proto::Power,
        action: &mut Option<Action>,
    ) {
        let (t, look, c) = (form.t, form.look, form.look.c);
        let mut changed = false;
        fieldset(ui, look, t.tr("canvas.sec_basic"), "", |ui| {
            changed |= form.line(ui, "field.name", Need::Optional, false, &mut f.name, |i| i);
            form.label(ui, "field.calendar", Need::Optional);
            let shown = if f.calendar.is_empty() { t.tr("field.no_calendar").to_owned() } else { f.calendar.clone() };
            egui::ComboBox::from_id_salt("net-calendar").selected_text(shown).width(ui.available_width()).show_ui(ui, |ui| {
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
            ui.add_space(10.0);
        });
        fieldset(ui, look, t.tr("canvas.sec_behave"), "", |ui| {
            let note = if f.missed == Missed::Once { t.tr("review.missed_once") } else { "" };
            let options = [(Missed::Skip, t.tr("field.skip")), (Missed::Once, t.tr("canvas.missed_late"))];
            changed |= form.choice(ui, "field.missed", &mut f.missed, &options, note);
            let note = if f.overlap == Overlap::Queue { t.tr("review.overlap_queue") } else { "" };
            let options = [(Overlap::Skip, t.tr("field.skip")), (Overlap::Queue, t.tr("canvas.overlap_after"))];
            changed |= form.choice(ui, "field.overlap", &mut f.overlap, &options, note);
            changed |= ui.checkbox(&mut f.wake, t.tr("field.wake")).changed();
        });
        fieldset(ui, look, t.tr("when.title"), "", |ui| {
            let mut remove = None;
            for (i, w) in f.whens.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    crate::ui::card(&c).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        changed |= w.show(ui, t, i);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if IconButton::new(M::Trash, t.tr("when.remove")).show(ui).clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                });
                ui.add_space(8.0);
            }
            if let Some(i) = remove {
                f.whens.remove(i);
                changed = true;
            }
            if IconButton::new(M::Plus, t.tr("when.add")).show(ui).clicked() {
                f.whens.push(WhenEdit::default());
                changed = true;
            }
        });
        if changed {
            self.apply_net(f);
        }
        let head = t.trf("run.next", &["10"]);
        fieldset(ui, look, &head, "", |ui| {
            let cal = self.net.calendar.as_ref().and_then(|n| calendars.get(n));
            let sleep = show::Sleep::of(power, self.net.wake);
            let now = ito_common::now::now_local().naive_local();
            for row in show::next_rows(t, &self.net.when, (&self.net.quiet, self.net.on_quiet), cal, now, 10, Some(&sleep), stops) {
                let r = ui.label(RichText::new(&row).monospace());
                ito_a11y::say(&r, Role::Label, &row);
            }
        });
    }
}

/// 繰り返し: `file` or `count`, one of the two, and how many passes run side by side.
fn loop_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let (file, count) = (!f.file.trim().is_empty(), !f.count.trim().is_empty());
    // Neither: both are asked for. One: that one is. Both: both are wrong.
    let file_need = if file || !count { Need::Required } else { Need::Plain };
    let count_need = if count || !file { Need::Required } else { Need::Plain };
    let both = file && count;
    let mut changed = form.line(ui, "field.loop_file", file_need, both, &mut f.file, |i| i.mono().hint("D:\\in\\*.csv"));
    changed |= form.line(ui, "field.count", count_need, both, &mut f.count, |i| i.width(140.0).unit(form.t.tr("canvas.unit_times")));
    changed |= form.line(ui, "field.loop_max", Need::Optional, false, &mut f.max, |i| i.width(140.0));
    changed
}

/// An `ssh`'s or a `winrm`'s group of PCs (`on = "group:…"`): how many at once, and when it passes.
fn group_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let n = f.max.trim();
    let bad = !n.is_empty() && !n.parse::<usize>().is_ok_and(|n| (1..=GROUP_MAX).contains(&n));
    let mut changed = form.line(ui, "field.group_max", Need::Optional, bad, &mut f.max, |i| i.width(140.0));
    let options = Pass::ALL.map(|p| (p, form.t.tr(if p == Pass::All { "field.pass_all" } else { "field.pass_any" })));
    changed |= form.choice(ui, "field.pass", &mut f.pass, &options, form.t.tr("field.group_note"));
    changed
}

/// What the job runs: `run`, or a shell and its script; where, and how its output is read.
fn run_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let t = form.t;
    let shell = !f.shell.trim().is_empty();
    let (run, script) = (!f.run.trim().is_empty(), !f.script.trim().is_empty());
    let run_need = if shell { Need::Plain } else { Need::Required };
    let mut changed = form.line(ui, "field.run", run_need, shell && run, &mut f.run, |i| i.mono().rows(3));
    changed |= form.line(ui, "field.shell", Need::Optional, false, &mut f.shell, |i| i.mono().hint("powershell").width(180.0));
    let script_need = if shell { Need::Required } else { Need::Optional };
    let aside = f.shell.trim().to_owned();
    changed |= form.code(ui, "field.script", script_need, !shell && script, &mut f.script, (5, &aside));
    changed |= form.line(ui, "field.dir", Need::Optional, false, &mut f.dir, |i| i.mono());
    let keys: Vec<String> =
        [Encoding::Auto, Encoding::Sjis, Encoding::Utf8].iter().map(|e| format!("field.encoding_{}", e.text())).collect();
    let options: Vec<(Encoding, &str)> =
        [Encoding::Auto, Encoding::Sjis, Encoding::Utf8].into_iter().zip(keys.iter()).map(|(e, k)| (e, t.tr(k))).collect();
    changed |= form.choice(ui, "field.encoding", &mut f.encoding, &options, "");
    changed
}

/// How long it may take and how often it is tried again.
fn retry_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let mut changed = form.line(ui, "field.timeout", Need::Optional, false, &mut f.timeout, |i| i.hint("30m").width(140.0));
    let bad = !f.retry_count.trim().is_empty() && f.retry_count.trim().parse::<u32>().is_err();
    let unit = form.t.tr("canvas.unit_times");
    changed |= form.pair(
        ui,
        |ui| form.line(ui, "field.retry_count", Need::Optional, bad, &mut f.retry_count, |i| i.hint("0").unit(unit)),
        |ui| form.line(ui, "field.retry_wait", Need::Optional, false, &mut f.retry_wait, |i| i.hint("1m")),
    );
    changed
}

/// Which exit codes, output and files mean warn, failure or success.
fn judge_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let mut changed = form.pair(
        ui,
        |ui| form.line(ui, "field.warn", Need::Plain, false, &mut f.warn, |i| i.mono().hint("1")),
        |ui| form.line(ui, "field.error", Need::Plain, false, &mut f.error, |i| i.mono().hint("2-255")),
    );
    changed |= form.line(ui, "field.error_output", Need::Plain, false, &mut f.error_output, |i| i.mono());
    changed |= form.line(ui, "field.ok_file", Need::Plain, false, &mut f.ok_file, |i| i.mono());
    changed
}

/// Another account to run as: both its name and its secret, or neither.
fn account_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let set = !f.run_as_user.trim().is_empty() || !f.run_as_secret.trim().is_empty();
    let need = if set { Need::Required } else { Need::Optional };
    let mut changed = form.line(ui, "field.run_as_user", need, false, &mut f.run_as_user, |i| i.mono().mark(M::Person));
    changed |= form.line(ui, "field.run_as_secret", need, false, &mut f.run_as_secret, |i| i.mono().mark(M::Lock));
    changed
}

/// How a `send` or `fetch` checks each file, and what that means for where it goes.
fn verify_row(ui: &mut egui::Ui, form: &Form, send: bool, f: &mut Fields) -> bool {
    let far = if send { f.into.as_str() } else { f.from.as_str() };
    let note = match f.verify {
        None => "field.verify_note_off",
        Some(_) if koyomi_core::chore::s3(far).is_some() => "field.verify_note_s3",
        Some(_) if koyomi_core::chore::sftp(far).is_none() => "field.verify_note_share",
        Some(Verify::Sha256) => "field.verify_note_sha256",
        Some(Verify::Readback) => "field.verify_note_readback",
    };
    let options = [
        (None, form.t.tr("field.verify_off")),
        (Some(Verify::Sha256), "sha256"),
        (Some(Verify::Readback), form.t.tr("field.verify_readback")),
    ];
    form.choice(ui, "field.verify", &mut f.verify, &options, form.t.tr(note))
}

/// The fields of a chore, by what it does.
fn chore_rows(ui: &mut egui::Ui, form: &Form, chore: &Chore, f: &mut Fields) -> bool {
    let (req, opt) = (Need::Required, Need::Optional);
    let mut changed = false;
    match chore {
        Chore::Delete { .. } => {
            changed |= form.line(ui, "field.path", req, false, &mut f.path, |i| i.mono().mark(M::Folder));
            changed |= form.line(ui, "field.match", opt, false, &mut f.pattern, |i| i.mono().hint("*.log"));
            let bad = !f.keep_days.trim().is_empty() && f.keep_days.trim().parse::<u32>().is_err();
            changed |= form.line(ui, "field.keep_days", req, bad, &mut f.keep_days, |i| i.width(140.0));
        }
        Chore::Empty { .. } => changed |= form.line(ui, "field.path", req, false, &mut f.path, |i| i.mono().mark(M::Folder)),
        Chore::Zip { .. } => {
            changed |= form.line(ui, "field.zip_from", req, false, &mut f.from, |i| i.mono());
            changed |= form.line(ui, "field.zip_into", req, false, &mut f.into, |i| i.mono());
        }
        Chore::Copy { .. } => {
            changed |= form.line(ui, "field.from", req, false, &mut f.from, |i| i.mono().mark(M::Folder));
            changed |= form.line(ui, "field.into", req, false, &mut f.into, |i| i.mono().mark(M::Folder));
            changed |= form.line(ui, "field.match", opt, false, &mut f.pattern, |i| i.mono().hint("*.csv"));
        }
        Chore::Send { .. } | Chore::Fetch { .. } => {
            let send = matches!(chore, Chore::Send { .. });
            let (from, into) = if send { ("field.send_from", "field.send_into") } else { ("field.fetch_from", "field.fetch_into") };
            changed |= form.line(ui, from, req, false, &mut f.from, |i| i.mono());
            changed |= form.line(ui, into, req, false, &mut f.into, |i| i.mono());
            // S3 logs in with an access key, not with SSH's.
            if koyomi_core::chore::s3(if send { &f.into } else { &f.from }).is_some() {
                changed |= form.line(ui, "field.key_id", req, false, &mut f.key_id, |i| i.mono());
                changed |= form.line(ui, "field.s3_password", req, false, &mut f.password, |i| i.mono().mark(M::Lock));
                changed |= form.line(ui, "field.region", opt, false, &mut f.region, |i| i.mono().hint("ap-northeast-1"));
                changed |=
                    form.line(ui, "field.endpoint", opt, false, &mut f.endpoint, |i| i.mono().hint("http://nas:9000").mark(M::Server));
            } else {
                changed |= form.line(ui, "field.key", opt, false, &mut f.key, |i| i.mono());
                changed |= form.line(ui, "field.host_key", opt, false, &mut f.host_key, |i| i.mono());
                changed |= form.line(ui, "field.password", opt, false, &mut f.password, |i| i.mono().mark(M::Lock));
            }
            changed |= verify_row(ui, form, send, f);
        }
        Chore::Ssh { .. } => {
            // A group's PCs may each have their host key in koyomi.toml.
            let group = koyomi_core::chore::group(&f.on).is_some();
            changed |= form.line(ui, "field.on", req, false, &mut f.on, |i| i.mono().hint("ssh://user@server").mark(M::Server));
            changed |= form.code(ui, "field.command", req, false, &mut f.command, (2, ""));
            changed |= form.line(ui, "field.host_key", if group { opt } else { req }, false, &mut f.host_key, |i| i.mono());
            changed |= form.line(ui, "field.key", opt, false, &mut f.key, |i| i.mono());
            changed |= form.line(ui, "field.password", opt, false, &mut f.password, |i| i.mono().mark(M::Lock));
            if group {
                changed |= group_rows(ui, form, f);
            }
        }
        Chore::Winrm { .. } => {
            let group = koyomi_core::chore::group(&f.on).is_some();
            changed |= form.line(ui, "field.winrm_on", req, false, &mut f.on, |i| i.mono().hint("winrm://user@server").mark(M::Desktop));
            changed |= form.code(ui, "field.command", req, false, &mut f.command, (2, ""));
            let bad = !f.cert.trim().is_empty() && koyomi_core::chore::cert_ok(f.cert.trim()).is_err();
            changed |= form.line(ui, "field.cert", if group { opt } else { req }, bad, &mut f.cert, |i| i.mono());
            changed |= form.line(ui, "field.winrm_password", req, false, &mut f.password, |i| i.mono().mark(M::Lock));
            let options = WinrmAuth::ALL.map(|a| (a, a.name()));
            let note = form.t.tr(match f.auth {
                WinrmAuth::Basic => "field.winrm_auth_basic",
                WinrmAuth::Ntlm => "field.winrm_auth_ntlm",
            });
            changed |= form.choice(ui, "field.winrm_auth", &mut f.auth, &options, note);
            if group {
                changed |= group_rows(ui, form, f);
            }
        }
        Chore::Http { .. } => {
            changed |= form.line(ui, "field.url", req, false, &mut f.url, |i| i.mono().mark(M::Globe));
            changed |= form.line(ui, "field.method", opt, false, &mut f.method, |i| i.mono().hint("GET").width(140.0));
            changed |= form.code(ui, "field.body", opt, false, &mut f.body, (3, ""));
            changed |= form.code(ui, "field.headers", opt, false, &mut f.headers, (2, ""));
            changed |= form.line(ui, "field.content_type", opt, false, &mut f.content_type, |i| i.mono());
            changed |= form.line(ui, "field.expect", opt, false, &mut f.expect, |i| i.mono().hint("200"));
        }
        Chore::Mail { .. } => {
            // An address that is not one, unless it is still a `{…}` to fill.
            let bad = |list: &str| list.lines().map(str::trim).any(|a| !a.is_empty() && koyomi_core::chore::mail_address(a).is_err());
            let (to_bad, cc_bad) = (bad(&f.mail_to), bad(&f.mail_cc));
            changed |= form.code(ui, "field.mail_to", req, to_bad, &mut f.mail_to, (2, ""));
            changed |= form.code(ui, "field.mail_cc", opt, cc_bad, &mut f.mail_cc, (1, ""));
            changed |= form.line(ui, "field.subject", req, false, &mut f.subject, |i| i.hint("売上の集計 {date}"));
            changed |= form.code(ui, "field.mail_body", opt, false, &mut f.body, (4, ""));
            changed |= form.code(ui, "field.files", opt, false, &mut f.files, (2, ""));
            changed |= form.line(ui, "field.via", opt, false, &mut f.via, |i| i.mark(M::Mail));
        }
        Chore::Sql { .. } => {
            // Which server it is decides how the SQL is cut into batches and what it signs in with.
            let db = (!f.db.contains('{')).then(|| koyomi_core::chore::sql_db(f.db.trim()).ok()).flatten();
            let db_bad = !f.db.trim().is_empty() && !f.db.contains('{') && db.is_none();
            changed |= form.line(ui, "field.sql_db", req, db_bad, &mut f.db, |i| i.mono().hint("sqlserver://db01/売上").mark(M::Server));
            let (text, file) = (!f.sql.trim().is_empty(), !f.file.trim().is_empty());
            let aside = match &db {
                Some(SqlDb::SqlServer { .. }) => "SQL Server",
                Some(SqlDb::Postgres { .. }) => "PostgreSQL",
                Some(SqlDb::Sqlite { .. }) => "SQLite",
                None => "",
            };
            let need = if file { opt } else { req };
            changed |= form.code(ui, "field.sql_text", need, text && file, &mut f.sql, (5, aside));
            let need = if text { opt } else { req };
            changed |= form.line(ui, "field.sql_file", need, text && file, &mut f.file, |i| i.mono());
            changed |= form.line(ui, "field.sql_out", opt, false, &mut f.out, |i| i.mono().hint("D:\\out\\{date}.csv"));
            let rows_bad = !f.rows.trim().is_empty() && koyomi_core::chore::sql_rows(f.rows.trim()).is_err();
            changed |= form.line(ui, "field.sql_rows", opt, rows_bad, &mut f.rows, |i| i.mono().hint("1..").width(180.0));
            // SQL Server signs in as the job's account when the address has no user.
            // SQLite takes neither; one left from another db stays shown (marked) so it can be cleared.
            let windows = matches!(&db, Some(SqlDb::SqlServer { user: None, .. }));
            let lite = matches!(&db, Some(SqlDb::Sqlite { .. }));
            let password = !f.password.trim().is_empty();
            if !(windows || lite) || password {
                let bad = (windows || lite) && password;
                changed |= form.line(ui, "field.sql_password", req, bad, &mut f.password, |i| i.mono().mark(M::Lock));
            }
            if !lite || f.tls != SqlTls::Verify {
                let options = SqlTls::ALL.map(|s| (s, s.key()));
                let note = form.t.tr(match f.tls {
                    SqlTls::Verify => "field.sql_tls_verify",
                    SqlTls::Trust => "field.sql_tls_trust",
                    SqlTls::Off => "field.sql_tls_off",
                });
                changed |= form.choice(ui, "field.sql_tls", &mut f.tls, &options, note);
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::split_label;

    #[test]
    fn a_label_splits_off_its_note() {
        assert_eq!(split_label("run（1 行に 1 つ）"), ("run", "1 行に 1 つ"));
        assert_eq!(split_label("run (one per line)"), ("run", "one per line"));
        assert_eq!(split_label("retry count"), ("retry count", ""));
        // A bracket at the start is the whole label, not a note.
        assert_eq!(split_label("（なし）"), ("（なし）", ""));
        assert_eq!(split_label("(none)"), ("(none)", ""));
    }
}
