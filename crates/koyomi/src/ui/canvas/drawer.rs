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

use super::{Action, Cmd, Editor, Far, Fields, NetFields, Pane};
use crate::show;
use crate::ui::pick::{self, Ctx, Want};
use crate::ui::whens::WhenEdit;

/// A note longer than this goes under the field, wrapped, instead of after the label.
const SHORT_NOTE: usize = 16;

/// What the form's rows share: the words, the colours and the required/optional words.
struct Form<'a> {
    t: &'a Table,
    look: Look<'static>,
    words: FormWords<'static>,
    /// The nets, the file chooser and where the engine is.
    pick: &'a Ctx<'a>,
}

impl<'a> Form<'a> {
    fn new(t: &'a Table, pick: &'a Ctx<'a>) -> Self {
        Form { t, look: Look { c: ito_theme::colors(), q: "" }, words: FormWords::of(t.tr("app.lang")), pick }
    }

    /// Whether the person is typing in the field `key` just now: its picker waits until they leave it.
    fn typing(&self, ui: &egui::Ui, key: &str) -> bool {
        ui.memory(|m| m.has_focus(egui::Id::new(("ito-input", key))))
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
        self.line_with(ui, key, need, bad, s, how, None)
    }

    /// [`Form::line`] with 参照… at its right (a path of this PC): a chooser of the kind `want`.
    #[allow(clippy::too_many_arguments)]
    fn path(
        &self,
        ui: &mut egui::Ui,
        key: &'a str,
        need: Need,
        bad: bool,
        s: &mut String,
        how: impl FnOnce(InputField<'a>) -> InputField<'a>,
        want: Want,
    ) -> bool {
        self.line_with(ui, key, need, bad, s, how, Some(want))
    }

    #[allow(clippy::too_many_arguments)]
    fn line_with(
        &self,
        ui: &mut egui::Ui,
        key: &'a str,
        need: Need,
        bad: bool,
        s: &mut String,
        how: impl FnOnce(InputField<'a>) -> InputField<'a>,
        browse: Option<Want>,
    ) -> bool {
        let (label, note) = self.label(ui, key, need);
        let wrong = bad || (need == Need::Required && s.trim().is_empty());
        let mut changed = false;
        match browse.filter(|_| self.pick.local) {
            Some(want) => {
                // The button takes its room first, at the right; the field fills what is left.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        changed |= self.pick.browse_button(ui, self.t, key, label, want, s);
                        changed |= how(InputField::new(key).name(label).error(wrong)).show(ui, s).changed();
                    });
                });
            }
            None => changed = how(InputField::new(key).name(label).error(wrong)).show(ui, s).changed(),
        }
        self.note(ui, note);
        ui.add_space(10.0);
        changed
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

    /// [`Form::code`] with a 参照… row under it: the files chosen are added a line each.
    fn files(&self, ui: &mut egui::Ui, key: &'a str, need: Need, s: &mut String, (rows, aside): (usize, &str)) -> bool {
        let (label, note) = self.label(ui, key, need);
        let mut changed = CodeBox::new(key).name(label).aside(aside).rows(rows).show(ui, s).changed();
        if self.pick.local {
            ui.add_space(4.0);
            changed |= self.pick.browse_button(ui, self.t, key, label, Want::Files, s);
        }
        self.note(ui, note);
        ui.add_space(10.0);
        changed
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

    /// The shell field: a select of the shells the engine has, "none" first (the job runs `run`). A shell the file
    /// names that the engine lacks stays among the choices, said so, so that opening and closing changes nothing.
    fn shell(&self, ui: &mut egui::Ui, value: &mut String) -> bool {
        let (label, _) = self.label(ui, "field.shell", Need::Optional);
        let now = value.trim().to_owned();
        let (entries, at, missing) = shell_entries(self.t, &self.pick.shell_names(), self.pick.shell_default(), &now);
        let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
        let picked = pick::select_index(ui, "field.shell", label, at, &labels);
        if missing {
            let key = if koyomi_core::shell::is_powershell(&now) { "field.shell_install_pwsh" } else { "field.shell_install" };
            self.note(ui, &self.t.trf(key, &[&now]));
        } else if !now.is_empty() {
            // The select has no hover per choice: the picked shell's path is the field's note.
            self.note(ui, self.pick.shell_path(&now));
        }
        ui.add_space(10.0);
        match picked.and_then(|i| picked_shell(&entries, &now, i)) {
            Some(to) => {
                *value = to;
                true
            }
            None => false,
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

/// The choices of the shell field: なし first (it empties `shell`), then the default marked and first, then the rest as
/// the engine lists them; the one written that the engine lacks is last, said so. Returns the entries (`shell` value
/// and words), the index of the one `now` names, and whether `now` is a shell the engine lacks.
fn shell_entries(t: &Table, names: &[String], default: &str, now: &str) -> (Vec<(String, String)>, usize, bool) {
    let mut entries: Vec<(String, String)> = vec![(String::new(), t.tr("field.shell_none").to_owned())];
    let word = |n: &str| if n == default { t.trf("field.shell_default", &[n]) } else { n.to_owned() };
    entries.extend(names.iter().filter(|n| n.as_str() == default).map(|n| (n.clone(), word(n))));
    entries.extend(names.iter().filter(|n| n.as_str() != default).map(|n| (n.clone(), word(n))));
    let missing = !now.is_empty() && !entries.iter().any(|(n, _)| n == now);
    if missing {
        entries.push((now.to_owned(), t.trf("field.shell_missing", &[now])));
    }
    let at = entries.iter().position(|(n, _)| n == now).unwrap_or(0);
    (entries, at, missing)
}

/// The `shell` value the choice `i` gives when it is not the one now (`run` and `script` are not touched).
fn picked_shell(entries: &[(String, String)], now: &str, i: usize) -> Option<String> {
    entries.get(i).filter(|(n, _)| n != now).map(|(n, _)| n.clone())
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn drawer(
        &mut self,
        ui: &mut egui::Ui,
        t: &Table,
        (calendars, stops, now): (&BTreeMap<String, Calendar>, &[koyomi_core::blackout::Blackout], chrono::NaiveDateTime),
        (nets, local, (shells, default)): (&[pick::NetChoice], bool, (&[crate::proto::Shell], &str)),
        (accounts, databases, windows, conns): (&[String], &[crate::proto::DatabaseName], Option<bool>, pick::Conns),
        power: &crate::proto::Power,
        action: &mut Option<Action>,
    ) {
        let c = ito_theme::colors();
        self.browse.poll(ui.ctx());
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
            // A chosen file has no field to land in while the pane is folded.
            self.browse.drop_answer();
            return;
        }
        let Some(mut pane) = self.pane.take() else { return };
        let scope = match &pane {
            Pane::Part(id, _) => id.clone(),
            Pane::Net(_) => String::new(),
        };
        let ctx = Ctx::new(nets, &self.id, local, &scope, std::mem::take(&mut self.browse))
            .with_shells(shells, default)
            .with_registry(accounts, databases, windows)
            .with_conns(conns);
        let form = Form::new(t, &ctx);
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
                            Pane::Net(f) => self.net_form(ui, &form, f, (calendars, stops, now), power, action),
                        });
                    });
                });
            });
        if let Some(page) = ctx.take_settings() {
            *action = Some(Action::OpenSettings(page));
        }
        self.browse = ctx.into_browse();
        // An answer the field it was opened for did not take (another part is open now) is dropped.
        self.browse.drop_answer();
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
                    let mut changed = form.path(ui, "review.file_wait", Need::Required, false, &mut f.file, |i| i.mono(), Want::File);
                    changed |= form.line(ui, "review.settle", Need::Optional, false, &mut f.settle, |i| i.hint("30s").width(120.0));
                    changed
                });
            }
            Part::Jobnet { .. } => {
                changed |= fieldset(ui, look, t.tr("canvas.sec_detail"), "", |ui| {
                    let bad = !f.use_.trim().is_empty() && !jobnet::is_id(f.use_.trim());
                    let mut changed = false;
                    // A select of the engine's nets while the text reads (or is empty); a text that does not read stays typed.
                    if !bad && !form.typing(ui, "review.use") {
                        let (label, note) = form.label(ui, "review.use", Need::Required);
                        ui.horizontal(|ui| {
                            changed = form.pick.net_select(ui, t, "field-use", label, &mut f.use_, false);
                            if IconButton::new(M::ArrowRight, t.tr("canvas.enter")).show(ui).clicked() && jobnet::is_id(f.use_.trim()) {
                                *action = Some(Action::Enter(f.use_.trim().to_owned()));
                            }
                        });
                        form.note(ui, note);
                        ui.add_space(10.0);
                    } else {
                        changed = form.line(ui, "review.use", Need::Required, bad, &mut f.use_, |i| i.mono().hint(t.tr("review.use_hint")));
                        if IconButton::new(M::ArrowRight, t.tr("canvas.enter")).show(ui).clicked() && jobnet::is_id(f.use_.trim()) {
                            *action = Some(Action::Enter(f.use_.trim().to_owned()));
                        }
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
            Part::Approval { .. } => {
                changed |= fieldset(ui, look, t.tr("canvas.sec_detail"), "", |ui| {
                    form.code(ui, "field.approval_message", Need::Optional, false, &mut f.message, (3, ""))
                });
            }
            Part::Branch | Part::Notify => {}
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
        (calendars, stops, now): (&BTreeMap<String, Calendar>, &[koyomi_core::blackout::Blackout], chrono::NaiveDateTime),
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
                        changed |= w.show(ui, t, i, form.pick);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // The chooser's answer is kept by the [[when]]'s number, so none is removed while one is open.
                            let gone = ui.add_enabled_ui(!form.pick.busy(), |ui| IconButton::new(M::Trash, t.tr("when.remove")).show(ui));
                            if gone.inner.on_disabled_hover_text(t.tr("pick.browsing")).clicked() {
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
    let mut changed = form.path(ui, "field.loop_file", file_need, both, &mut f.file, |i| i.mono().hint("D:\\in\\*.csv"), Want::File);
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
    changed |= form.shell(ui, &mut f.shell);
    let script_need = if shell { Need::Required } else { Need::Optional };
    let aside = f.shell.trim().to_owned();
    changed |= form.code(ui, "field.script", script_need, !shell && script, &mut f.script, (5, &aside));
    changed |= form.path(ui, "field.dir", Need::Optional, false, &mut f.dir, |i| i.mono(), Want::Folder);
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
    changed |= form.path(ui, "field.ok_file", Need::Plain, false, &mut f.ok_file, |i| i.mono(), Want::File);
    changed
}

/// What the account select can pick.
#[derive(Clone, Debug, PartialEq)]
enum AccountChoice {
    /// No `run_as`: the job runs as the engine's account.
    Engine,
    /// A registered `[[account]]`.
    Named(String),
    /// The account written out in the job (`user` and `password`).
    Inline,
    /// A name the engine does not have registered.
    Unknown(String),
}

/// Which choice the fields are at now.
fn account_now(f: &Fields, names: &[String]) -> AccountChoice {
    let name = f.run_as_name.trim();
    if !name.is_empty() {
        return if names.iter().any(|n| n == name) {
            AccountChoice::Named(name.to_owned())
        } else {
            AccountChoice::Unknown(name.to_owned())
        };
    }
    if f.run_as_user.trim().is_empty() && f.run_as_secret.trim().is_empty() {
        AccountChoice::Engine
    } else {
        AccountChoice::Inline
    }
}

/// The choices (value and words) and the index of the one the fields are at: なし first, the registered names, the
/// account written out (only while the job has one, or one was set aside for this form), an unknown name last.
fn account_entries(t: &Table, names: &[String], f: &Fields) -> (Vec<(AccountChoice, String)>, usize) {
    let now = account_now(f, names);
    let mut entries = vec![(AccountChoice::Engine, t.tr("field.run_as_none").to_owned())];
    entries.extend(names.iter().map(|n| (AccountChoice::Named(n.clone()), n.clone())));
    let drafted = !f.run_as_draft.0.trim().is_empty() || !f.run_as_draft.1.trim().is_empty();
    if now == AccountChoice::Inline || drafted {
        let user = if f.run_as_user.trim().is_empty() { f.run_as_draft.0.trim() } else { f.run_as_user.trim() };
        entries.push((AccountChoice::Inline, t.trf("field.run_as_inline", &[user])));
    }
    if let AccountChoice::Unknown(n) = &now {
        entries.push((now.clone(), t.trf("field.run_as_unknown", &[n])));
    }
    let at = entries.iter().position(|(c, _)| *c == now).unwrap_or(0);
    (entries, at)
}

/// Picks `to`: the written-out account moves to the draft (and comes back when it is picked again), so that a name
/// and a user are never both in the fields. Nothing else of the job is touched.
fn pick_account(f: &mut Fields, to: &AccountChoice) {
    let set_aside = |f: &mut Fields| {
        if !f.run_as_user.trim().is_empty() || !f.run_as_secret.trim().is_empty() {
            f.run_as_draft = (std::mem::take(&mut f.run_as_user), std::mem::take(&mut f.run_as_secret));
        }
    };
    match to {
        AccountChoice::Engine => {
            set_aside(f);
            f.run_as_name.clear();
        }
        AccountChoice::Named(n) => {
            set_aside(f);
            f.run_as_name.clone_from(n);
        }
        AccountChoice::Inline => {
            f.run_as_name.clear();
            if f.run_as_user.trim().is_empty() && f.run_as_secret.trim().is_empty() {
                (f.run_as_user, f.run_as_secret) = std::mem::take(&mut f.run_as_draft);
            }
        }
        AccountChoice::Unknown(_) => {}
    }
}

/// The account to run as: a select of なし, the registered accounts and (when the job has one) the account written
/// out, whose user and secret show under it.
fn account_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let (label, _) = form.label(ui, "field.run_as", Need::Optional);
    let (entries, at) = account_entries(form.t, form.pick.accounts(), f);
    let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
    let mut changed = false;
    if let Some(i) = pick::select_index(ui, "field.run_as", label, at, &labels) {
        if i != at {
            pick_account(f, &entries[i].0);
            changed = true;
        }
    }
    let now = account_now(f, form.pick.accounts());
    if let AccountChoice::Unknown(_) = now {
        form.note(ui, form.t.tr("field.run_as_unknown_note"));
    }
    if form.pick.windows() == Some(false) && now != AccountChoice::Engine {
        form.note(ui, &koyomi_core::msg::line(koyomi_core::msg::OS_RUN_AS, &[&form.pick.own]));
    }
    register_link(ui, form, "field.run_as_register", crate::settings::ACCOUNTS);
    ui.add_space(10.0);
    if now == AccountChoice::Inline {
        let set = !f.run_as_user.trim().is_empty() || !f.run_as_secret.trim().is_empty();
        let need = if set { Need::Required } else { Need::Optional };
        changed |= form.line(ui, "field.run_as_user", need, false, &mut f.run_as_user, |i| i.mono().mark(M::Person));
        changed |= form.line(ui, "field.run_as_secret", need, false, &mut f.run_as_secret, |i| i.mono().mark(M::Lock));
    }
    changed
}

/// A link to the settings page `page` (to register what the select lacks).
fn register_link(ui: &mut egui::Ui, form: &Form, key: &str, page: usize) {
    let words = form.t.tr(key);
    let r = ui.link(words);
    ito_a11y::say(&r, Role::Button, words);
    ito_a11y::test_id(&r, key);
    if r.clicked() {
        form.pick.ask_settings(page);
    }
}

/// What the SQL connection select can pick.
#[derive(Clone, Debug, PartialEq)]
enum DbChoice {
    /// The address (and its secret and TLS) written in the job.
    Url,
    /// A registered `[[database]]`.
    Named(String),
    /// A name the engine does not have registered.
    Unknown(String),
}

fn db_now(f: &Fields, dbs: &[crate::proto::DatabaseName]) -> DbChoice {
    let name = f.sql_database.trim();
    if name.is_empty() {
        DbChoice::Url
    } else if dbs.iter().any(|d| d.name == name) {
        DbChoice::Named(name.to_owned())
    } else {
        DbChoice::Unknown(name.to_owned())
    }
}

/// The choices (value and words) and the index of the one the fields are at: each registered database with its
/// kind, 「URL を直接書く」, then an unknown name last.
fn db_entries(t: &Table, dbs: &[crate::proto::DatabaseName], f: &Fields) -> (Vec<(DbChoice, String)>, usize) {
    let now = db_now(f, dbs);
    let mut entries: Vec<(DbChoice, String)> = dbs
        .iter()
        .map(|d| (DbChoice::Named(d.name.clone()), t.trf("field.sql_database_entry", &[&d.name, crate::ui::registry::kind_word(&d.kind)])))
        .collect();
    entries.push((DbChoice::Url, t.tr("field.sql_database_url").to_owned()));
    if let DbChoice::Unknown(n) = &now {
        entries.push((now.clone(), t.trf("field.sql_database_unknown", &[n])));
    }
    let at = entries.iter().position(|(c, _)| *c == now).unwrap_or(0);
    (entries, at)
}

/// Picks `to`: the address, password and TLS written in the job move to the draft (and come back when the written-out
/// address is picked again), so that a name and an address are never both in the fields. The SQL and the other fields stay.
fn pick_db(f: &mut Fields, to: &DbChoice) {
    match to {
        DbChoice::Named(n) => {
            if !f.db.trim().is_empty() || !f.password.trim().is_empty() || f.tls != SqlTls::Verify {
                f.sql_draft = (std::mem::take(&mut f.db), std::mem::take(&mut f.password), std::mem::take(&mut f.tls));
            }
            f.sql_database.clone_from(n);
        }
        DbChoice::Url => {
            f.sql_database.clear();
            if f.db.trim().is_empty() && f.password.trim().is_empty() {
                (f.db, f.password, f.tls) = std::mem::take(&mut f.sql_draft);
            }
        }
        DbChoice::Unknown(_) => {}
    }
}

/// The SQL connection: a select of the registered databases and 「URL を直接書く」, what the one picked is, and the
/// link to register one.
fn database_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let dbs = form.pick.databases();
    let (label, _) = form.label(ui, "field.sql_database", Need::Required);
    let (entries, at) = db_entries(form.t, dbs, f);
    let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
    let mut changed = false;
    if let Some(i) = pick::select_index(ui, "field.sql_database", label, at, &labels) {
        if i != at {
            pick_db(f, &entries[i].0);
            changed = true;
        }
    }
    match db_now(f, dbs) {
        DbChoice::Named(n) => {
            let kind = dbs.iter().find(|d| d.name == n).map_or("", |d| crate::ui::registry::kind_word(&d.kind));
            form.note(ui, &form.t.trf("field.sql_database_kind", &[kind]));
        }
        DbChoice::Unknown(_) => form.note(ui, form.t.tr("field.sql_database_unknown_note")),
        DbChoice::Url => {}
    }
    register_link(ui, form, "field.sql_database_register", crate::settings::DATABASES);
    ui.add_space(10.0);
    changed
}

/// What the PC select of an ssh or a winrm can pick.
#[derive(Clone, Debug, PartialEq)]
enum OnChoice {
    /// A registered `[[host]]` (`group: false`) or `[groups]` entry by its name.
    Named { group: bool, name: String },
    /// The address written out.
    Typed,
}

impl OnChoice {
    /// The `on` this choice makes (none: the address written out).
    fn on(&self) -> Option<String> {
        match self {
            OnChoice::Named { group: true, name } => Some(format!("group:{name}")),
            OnChoice::Named { name, .. } => Some(name.clone()),
            OnChoice::Typed => None,
        }
    }
}

/// How a name stands against the engine's list of the job's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    /// Listed, and of the job's kind.
    Listed,
    /// Listed, as the other kind (`ssh` for a winrm job and the reverse).
    Other(&'static str),
    /// Not listed.
    Unknown,
}

/// The kind word the engine lists the hosts and groups of an ssh (`false`) or a winrm (`true`) job by.
fn on_kind(winrm: bool) -> &'static str {
    if winrm {
        "winrm"
    } else {
        "ssh"
    }
}

fn other_kind(kind: &str) -> &'static str {
    if kind == "winrm" {
        "winrm"
    } else {
        "ssh"
    }
}

/// Where `choice` stands against `conns` for a job of `kind`.
fn on_standing(conns: pick::Conns, kind: &str, group: bool, name: &str) -> Standing {
    let found: Option<&str> = if group {
        conns.groups.iter().find(|g| g.name == name).map(|g| g.kind.as_str())
    } else {
        conns.hosts.iter().find(|h| h.name == name).map(|h| h.kind.as_str())
    };
    match found {
        None => Standing::Unknown,
        Some(k) if k == kind => Standing::Listed,
        Some(k) => Standing::Other(other_kind(k)),
    }
}

/// The choice the fields are at: by the choice picked or the job's own at open, then by what `on` names.
fn on_now(f: &Fields) -> OnChoice {
    if f.kept.typed {
        return OnChoice::Typed;
    }
    if let Some(g) = koyomi_core::chore::group(&f.on) {
        OnChoice::Named { group: true, name: g.to_owned() }
    } else if let Some(h) = koyomi_core::chore::host_name(&f.on) {
        OnChoice::Named { group: false, name: h.to_owned() }
    } else {
        OnChoice::Typed
    }
}

/// The words of a name the engine lacks, or has as the other kind.
fn conn_words(t: &Table, standing: Standing, shown: &str) -> String {
    match standing {
        Standing::Listed => shown.to_owned(),
        Standing::Other("winrm") => t.trf("field.conn_other_winrm", &[shown]),
        Standing::Other(_) => t.trf("field.conn_other_ssh", &[shown]),
        Standing::Unknown => t.trf("field.conn_unknown", &[shown]),
    }
}

/// The choices (value and words) and the index of the one the fields are at: the engine's hosts of the job's kind,
/// its groups, 「直接書く」, then the name the job had at open or has now when the engine lacks it (or has it as the other
/// kind). Such a name stays listed until the form closes.
fn on_entries(t: &Table, conns: pick::Conns, winrm: bool, f: &Fields) -> (Vec<(OnChoice, String)>, usize) {
    let kind = on_kind(winrm);
    let mut entries: Vec<(OnChoice, String)> = Vec::new();
    for h in conns.hosts.iter().filter(|h| h.kind == kind) {
        entries.push((OnChoice::Named { group: false, name: h.name.clone() }, h.name.clone()));
    }
    for g in conns.groups.iter().filter(|g| g.kind == kind) {
        entries.push((OnChoice::Named { group: true, name: g.name.clone() }, t.trf("field.on_group", &[&g.name])));
    }
    entries.push((OnChoice::Typed, t.tr("field.on_typed").to_owned()));
    let now = on_now(f);
    let opened = match (koyomi_core::chore::group(&f.kept.on), koyomi_core::chore::host_name(&f.kept.on)) {
        (Some(g), _) => Some(OnChoice::Named { group: true, name: g.to_owned() }),
        (_, Some(h)) => Some(OnChoice::Named { group: false, name: h.to_owned() }),
        _ => None,
    };
    for extra in opened.into_iter().chain(std::iter::once(now.clone())) {
        if let OnChoice::Named { group, name } = &extra {
            if !entries.iter().any(|(c, _)| *c == extra) {
                let shown = if *group { t.trf("field.on_group", &[name]) } else { name.clone() };
                entries.push((extra.clone(), conn_words(t, on_standing(conns, kind, *group, name), &shown)));
            }
        }
    }
    let at = entries.iter().position(|(c, _)| *c == now).unwrap_or(0);
    (entries, at)
}

/// The PC an ssh or a winrm runs on: a select of the engine's hosts, its groups and 「直接書く」, what the one picked is,
/// the link to register a host, and (when the address is written out) its line.
fn on_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields, winrm: bool) -> bool {
    let conns = form.pick.conns();
    let (label, _) = form.label(ui, "field.on_pick", Need::Required);
    let (entries, at) = on_entries(form.t, conns, winrm, f);
    let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
    let mut changed = false;
    if let Some(i) = pick::select_index(ui, "field.on_pick", label, at, &labels) {
        if i != at {
            f.pick_on(entries[i].0.on().as_deref());
            changed = true;
        }
    }
    let now = on_now(f);
    if let OnChoice::Named { group, name } = &now {
        match on_standing(conns, on_kind(winrm), *group, name) {
            Standing::Unknown => form.note(ui, form.t.tr("field.conn_unknown_note")),
            Standing::Other(_) => form.note(ui, form.t.tr("field.conn_other_note")),
            Standing::Listed if !*group => form.note(ui, form.t.tr("field.on_host_note")),
            Standing::Listed => {}
        }
    }
    register_link(ui, form, "field.on_register", crate::settings::HOSTS);
    ui.add_space(10.0);
    if now == OnChoice::Typed {
        let (key, hint, mark) =
            if winrm { ("field.winrm_on", "winrm://user@server", M::Desktop) } else { ("field.on", "ssh://user@server", M::Server) };
        changed |= form.line(ui, key, Need::Required, false, &mut f.on, |i| i.mono().hint(hint).mark(mark));
    }
    changed
}

/// What the far side select of a `send` or `fetch` can pick.
#[derive(Clone, Debug, PartialEq)]
enum FarChoice {
    Share,
    Host(String),
    Bucket(String),
    Url,
}

impl FarChoice {
    fn kind(&self) -> Far {
        match self {
            FarChoice::Share => Far::Share,
            FarChoice::Host(_) => Far::Host,
            FarChoice::Bucket(_) => Far::Bucket,
            FarChoice::Url => Far::Url,
        }
    }

    fn name(&self) -> &str {
        match self {
            FarChoice::Host(n) | FarChoice::Bucket(n) => n,
            _ => "",
        }
    }
}

fn far_now(f: &Fields) -> FarChoice {
    match f.far_kind() {
        Far::Host => FarChoice::Host(f.host.trim().to_owned()),
        Far::Bucket => FarChoice::Bucket(f.bucket.trim().to_owned()),
        Far::Url => FarChoice::Url,
        Far::Share => FarChoice::Share,
    }
}

/// Where a registered host stands for a send or fetch: it has to be an `ssh` one (SFTP).
fn host_standing(conns: pick::Conns, name: &str) -> Standing {
    on_standing(conns, "ssh", false, name)
}

/// The choices (value and words) and the index of the one the fields are at: 「フォルダー・共有」, each ssh host
/// (`web1（SFTP）`), each bucket (`logs（S3）`), 「URL を直接書く」, then the host or bucket the job had at open or has now when the
/// engine lacks it (or, for a host, has it as a WinRM one). Such a name stays listed until the form closes.
fn far_entries(t: &Table, conns: pick::Conns, f: &Fields) -> (Vec<(FarChoice, String)>, usize) {
    let mut entries: Vec<(FarChoice, String)> = vec![(FarChoice::Share, t.tr("field.far_share").to_owned())];
    for h in conns.hosts.iter().filter(|h| h.kind == "ssh") {
        entries.push((FarChoice::Host(h.name.clone()), t.trf("field.far_host", &[&h.name])));
    }
    for b in conns.buckets {
        entries.push((FarChoice::Bucket(b.clone()), t.trf("field.far_bucket", &[b])));
    }
    entries.push((FarChoice::Url, t.tr("field.far_url").to_owned()));
    let now = far_now(f);
    let opened = [
        Some(f.kept.host.trim()).filter(|n| !n.is_empty()).map(|n| FarChoice::Host(n.to_owned())),
        Some(f.kept.bucket.trim()).filter(|n| !n.is_empty()).map(|n| FarChoice::Bucket(n.to_owned())),
    ];
    for extra in opened.into_iter().flatten().chain(std::iter::once(now.clone())) {
        if matches!(extra, FarChoice::Host(_) | FarChoice::Bucket(_)) && !entries.iter().any(|(c, _)| *c == extra) {
            let standing = match &extra {
                FarChoice::Host(n) => host_standing(conns, n),
                _ => Standing::Unknown,
            };
            entries.push((extra.clone(), conn_words(t, standing, extra.name())));
        }
    }
    let at = entries.iter().position(|(c, _)| *c == now).unwrap_or(0);
    (entries, at)
}

/// The far side of a `send` (`into`) or a `fetch` (`from`): a select of 「フォルダー・共有」, the registered hosts and
/// buckets and 「URL を直接書く」, the path (or the address) of the one picked, the notes, the links to register, and (for
/// an address) the rows of its login.
fn far_rows(ui: &mut egui::Ui, form: &Form, send: bool, f: &mut Fields) -> bool {
    let conns = form.pick.conns();
    let key = if send { "field.far_into" } else { "field.far_from" };
    let (label, _) = form.label(ui, key, Need::Required);
    let (entries, at) = far_entries(form.t, conns, f);
    let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
    let mut changed = false;
    if let Some(i) = pick::select_index(ui, key, label, at, &labels) {
        if i != at {
            f.pick_far(send, entries[i].0.kind(), entries[i].0.name());
            changed = true;
        }
    }
    let now = far_now(f);
    match &now {
        FarChoice::Host(n) => match host_standing(conns, n) {
            Standing::Unknown => form.note(ui, form.t.tr("field.conn_unknown_note")),
            Standing::Other(_) => form.note(ui, form.t.tr("field.conn_other_note")),
            Standing::Listed => form.note(ui, form.t.tr("field.far_host_note")),
        },
        FarChoice::Bucket(n) => {
            let note = if conns.buckets.contains(n) { "field.far_bucket_note" } else { "field.conn_unknown_note" };
            form.note(ui, form.t.tr(note));
        }
        _ => {}
    }
    register_link(ui, form, "field.far_register_host", crate::settings::HOSTS);
    register_link(ui, form, "field.far_register_bucket", crate::settings::BUCKETS);
    ui.add_space(10.0);
    let (line, hint) = match &now {
        FarChoice::Share => ("field.far_share_path", ""),
        FarChoice::Host(_) => ("field.far_host_path", if send { "/in" } else { "~/in" }),
        FarChoice::Bucket(_) => ("field.far_bucket_path", "in/today.csv"),
        FarChoice::Url => (if send { "field.send_into" } else { "field.fetch_from" }, ""),
    };
    let far = if send { &mut f.into } else { &mut f.from };
    changed |= form.line(ui, line, Need::Required, false, far, |i| i.mono().hint(hint));
    if now == FarChoice::Url {
        let (opt, req) = (Need::Optional, Need::Required);
        if koyomi_core::chore::s3(f.far(send)).is_some() {
            changed |= form.line(ui, "field.key_id", req, false, &mut f.key_id, |i| i.mono());
            changed |= form.line(ui, "field.s3_password", req, false, &mut f.s3_password, |i| i.mono().mark(M::Lock));
            changed |= form.line(ui, "field.region", opt, false, &mut f.region, |i| i.mono().hint("ap-northeast-1"));
            changed |= form.line(ui, "field.endpoint", opt, false, &mut f.endpoint, |i| i.mono().hint("http://nas:9000").mark(M::Server));
        } else {
            changed |= form.path(ui, "field.key", opt, false, &mut f.key, |i| i.mono(), Want::File);
            changed |= form.line(ui, "field.host_key", opt, false, &mut f.host_key, |i| i.mono());
            changed |= form.line(ui, "field.password", opt, false, &mut f.password, |i| i.mono().mark(M::Lock));
        }
    }
    changed
}

/// The mail `via` choices (the value it sets and the words) and the index of the one the fields are at: the first place
/// (an empty `via`; its name shown when there is one), each `[[notify.mails]]` by name, then a name the engine lacks
/// (the one the job had at open, or has now; a `{…}` too) last.
fn via_entries(t: &Table, mails: &[String], f: &Fields) -> (Vec<(String, String)>, usize) {
    let first = match mails.first() {
        Some(n) => t.trf("field.via_first_named", &[n]),
        None => t.tr("field.via_first_pick").to_owned(),
    };
    let mut entries: Vec<(String, String)> = vec![(String::new(), first)];
    entries.extend(mails.iter().map(|n| (n.clone(), n.clone())));
    for name in [f.kept.via.trim(), f.via.trim()] {
        if !name.is_empty() && !entries.iter().any(|(v, _)| v == name) {
            entries.push((name.to_owned(), t.trf("field.conn_unknown", &[name])));
        }
    }
    let now = f.via.trim();
    let at = entries.iter().position(|(v, _)| v == now).unwrap_or(0);
    (entries, at)
}

/// The place a `mail` goes through: a select of the first place and each `[[notify.mails]]`, and the way to add one (the
/// notify page for this PC's engine; for another, only a note, since its places are set on its own PC).
fn via_rows(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let mails = form.pick.conns().mails;
    let (label, _) = form.label(ui, "field.via_pick", Need::Optional);
    let (entries, at) = via_entries(form.t, mails, f);
    let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
    let mut changed = false;
    if let Some(i) = pick::select_index(ui, "field.via_pick", label, at, &labels) {
        if i != at {
            f.via.clone_from(&entries[i].0);
            changed = true;
        }
    }
    let name = f.via.trim();
    if !name.is_empty() && !mails.iter().any(|m| m == name) {
        form.note(ui, form.t.tr("field.conn_unknown_note"));
    }
    if form.pick.local {
        register_link(ui, form, "field.via_register", crate::settings::NOTIFY);
    } else {
        form.note(ui, form.t.tr("field.via_far_note"));
    }
    ui.add_space(10.0);
    changed
}

/// How a `send` or `fetch` checks each file, and what that means for where it goes.
fn verify_row(ui: &mut egui::Ui, form: &Form, send: bool, f: &mut Fields) -> bool {
    let far = if send { f.into.as_str() } else { f.from.as_str() };
    let note = match f.verify {
        None => "field.verify_note_off",
        Some(_) if !f.bucket.trim().is_empty() => "field.verify_note_s3",
        Some(_) if koyomi_core::chore::s3(far).is_some() => "field.verify_note_s3",
        Some(_) if f.host.trim().is_empty() && koyomi_core::chore::sftp(far).is_none() => "field.verify_note_share",
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

/// Whether a method text is refused: core reads the six words as written (upper case), and an empty one is the default.
fn method_bad(now: &str) -> bool {
    !now.is_empty() && !koyomi_core::chore::METHODS.contains(&now)
}

/// An http job's method: the six words to pick (an empty one shows GET, the default, and stays empty until another is
/// picked). Text that is not one of them stays a typed field, and says so.
fn method_row(ui: &mut egui::Ui, form: &Form, f: &mut Fields) -> bool {
    let methods = koyomi_core::chore::METHODS;
    let now = f.method.trim();
    let at = if now.is_empty() { Some(0) } else { methods.iter().position(|m| *m == now) };
    match at.filter(|_| !form.typing(ui, "field.method")) {
        Some(mut at) => {
            let options: Vec<(usize, &str)> = methods.iter().copied().enumerate().collect();
            let picked = form.choice(ui, "field.method", &mut at, &options, "");
            if picked {
                f.method = methods[at].to_owned();
            }
            picked
        }
        None => {
            let bad = method_bad(now);
            form.line(ui, "field.method", Need::Optional, bad, &mut f.method, |i| i.mono().hint("GET").width(140.0))
        }
    }
}

/// The fields of a chore, by what it does.
fn chore_rows(ui: &mut egui::Ui, form: &Form, chore: &Chore, f: &mut Fields) -> bool {
    let (req, opt) = (Need::Required, Need::Optional);
    let mut changed = false;
    match chore {
        Chore::Delete { .. } => {
            changed |= form.path(ui, "field.path", req, false, &mut f.path, |i| i.mono().mark(M::Folder), Want::Folder);
            changed |= form.line(ui, "field.match", opt, false, &mut f.pattern, |i| i.mono().hint("*.log"));
            let bad = !f.keep_days.trim().is_empty() && f.keep_days.trim().parse::<u32>().is_err();
            changed |= form.line(ui, "field.keep_days", req, bad, &mut f.keep_days, |i| i.width(140.0));
        }
        Chore::Empty { .. } => changed |= form.path(ui, "field.path", req, false, &mut f.path, |i| i.mono().mark(M::Folder), Want::Folder),
        Chore::Zip { .. } => {
            changed |= form.path(ui, "field.zip_from", req, false, &mut f.from, |i| i.mono(), Want::File);
            changed |= form.path(ui, "field.zip_into", req, false, &mut f.into, |i| i.mono(), Want::Save);
        }
        Chore::Copy { .. } => {
            changed |= form.path(ui, "field.from", req, false, &mut f.from, |i| i.mono().mark(M::Folder), Want::Folder);
            changed |= form.path(ui, "field.into", req, false, &mut f.into, |i| i.mono().mark(M::Folder), Want::Folder);
            changed |= form.line(ui, "field.match", opt, false, &mut f.pattern, |i| i.mono().hint("*.csv"));
        }
        Chore::Send { .. } | Chore::Fetch { .. } => {
            let send = matches!(chore, Chore::Send { .. });
            let (from, into) = if send { ("field.send_from", "field.send_into") } else { ("field.fetch_from", "field.fetch_into") };
            // Send reads this PC's files (a file; the folder or glob is typed). Fetch saves into this PC's folder. The far
            // side (where a send goes, what a fetch reads) is a choice: a share, a registered host or bucket, or an address.
            if send {
                changed |= form.path(ui, from, req, false, &mut f.from, |i| i.mono(), Want::File);
                changed |= far_rows(ui, form, true, f);
            } else {
                changed |= far_rows(ui, form, false, f);
                changed |= form.path(ui, into, req, false, &mut f.into, |i| i.mono(), Want::Folder);
            }
            changed |= verify_row(ui, form, send, f);
        }
        Chore::Ssh { .. } => {
            // A group's PCs may each have their host key in koyomi.toml.
            changed |= on_rows(ui, form, f, false);
            let group = koyomi_core::chore::group(&f.on).is_some();
            changed |= form.code(ui, "field.command", req, false, &mut f.command, (2, ""));
            // A name of a `[[host]]` in koyomi.toml owns the login.
            if koyomi_core::chore::host_name(&f.on).is_none() {
                changed |= form.line(ui, "field.host_key", if group { opt } else { req }, false, &mut f.host_key, |i| i.mono());
                changed |= form.path(ui, "field.key", opt, false, &mut f.key, |i| i.mono(), Want::File);
                changed |= form.line(ui, "field.password", opt, false, &mut f.password, |i| i.mono().mark(M::Lock));
            }
            if group {
                changed |= group_rows(ui, form, f);
            }
        }
        Chore::Winrm { .. } => {
            changed |= on_rows(ui, form, f, true);
            let group = koyomi_core::chore::group(&f.on).is_some();
            changed |= form.code(ui, "field.command", req, false, &mut f.command, (2, ""));
            // A name of a `[[host]]` in koyomi.toml owns the login.
            if koyomi_core::chore::host_name(&f.on).is_none() {
                let bad = !f.cert.trim().is_empty() && koyomi_core::chore::cert_ok(f.cert.trim()).is_err();
                changed |= form.line(ui, "field.cert", if group { opt } else { req }, bad, &mut f.cert, |i| i.mono());
                changed |= form.line(ui, "field.winrm_password", req, false, &mut f.password, |i| i.mono().mark(M::Lock));
                let options = WinrmAuth::ALL.map(|a| (a, a.name()));
                let note = form.t.tr(match f.auth {
                    WinrmAuth::Basic => "field.winrm_auth_basic",
                    WinrmAuth::Ntlm => "field.winrm_auth_ntlm",
                });
                changed |= form.choice(ui, "field.winrm_auth", &mut f.auth, &options, note);
            }
            if group {
                changed |= group_rows(ui, form, f);
            }
        }
        Chore::Http { .. } => {
            changed |= form.line(ui, "field.url", req, false, &mut f.url, |i| i.mono().mark(M::Globe));
            changed |= method_row(ui, form, f);
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
            changed |= form.line(ui, "field.subject", req, false, &mut f.subject, |i| i.hint(form.t.tr("field.subject_hint")));
            changed |= form.code(ui, "field.mail_body", opt, false, &mut f.body, (4, ""));
            changed |= form.files(ui, "field.files", opt, &mut f.files, (2, ""));
            changed |= via_rows(ui, form, f);
        }
        Chore::Sql { .. } => {
            changed |= database_rows(ui, form, f);
            // A registered database holds the address, the secret and the TLS; the fields below are for a written-out one.
            let named = !f.sql_database.trim().is_empty();
            // Which server it is decides how the SQL is cut into batches and what it signs in with.
            let db = (!named && !f.db.contains('{')).then(|| koyomi_core::chore::sql_db(f.db.trim()).ok()).flatten();
            if !named {
                let db_bad = !f.db.trim().is_empty() && !f.db.contains('{') && db.is_none();
                changed |= form
                    .line(ui, "field.sql_db", req, db_bad, &mut f.db, |i| i.mono().hint(form.t.tr("field.sql_db_hint")).mark(M::Server));
            }
            let (text, file) = (!f.sql.trim().is_empty(), !f.file.trim().is_empty());
            let aside = if named {
                let name = f.sql_database.trim();
                crate::ui::registry::kind_word(form.pick.databases().iter().find(|d| d.name == name).map_or("", |d| d.kind.as_str()))
            } else {
                match &db {
                    Some(SqlDb::SqlServer { .. }) => "SQL Server",
                    Some(SqlDb::Postgres { .. }) => "PostgreSQL",
                    Some(SqlDb::Sqlite { .. }) => "SQLite",
                    None => "",
                }
            };
            let need = if file { opt } else { req };
            changed |= form.code(ui, "field.sql_text", need, text && file, &mut f.sql, (5, aside));
            let need = if text { opt } else { req };
            changed |= form.path(ui, "field.sql_file", need, text && file, &mut f.file, |i| i.mono(), Want::File);
            changed |= form.path(ui, "field.sql_out", opt, false, &mut f.out, |i| i.mono().hint("D:\\out\\{date}.csv"), Want::Save);
            let rows_bad = !f.rows.trim().is_empty() && koyomi_core::chore::sql_rows(f.rows.trim()).is_err();
            changed |= form.line(ui, "field.sql_rows", opt, rows_bad, &mut f.rows, |i| i.mono().hint("1..").width(180.0));
            // SQL Server signs in as the job's account when the address has no user.
            // SQLite takes neither; one left from another db stays shown (marked) so it can be cleared.
            let windows = matches!(&db, Some(SqlDb::SqlServer { user: None, .. }));
            let lite = matches!(&db, Some(SqlDb::Sqlite { .. }));
            let password = !f.password.trim().is_empty();
            if !named && (!(windows || lite) || password) {
                let bad = (windows || lite) && password;
                changed |= form.line(ui, "field.sql_password", req, bad, &mut f.password, |i| i.mono().mark(M::Lock));
            }
            if !named && (!lite || f.tls != SqlTls::Verify) {
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
    use super::{method_bad, split_label};

    #[test]
    fn a_method_that_core_would_refuse_is_bad() {
        assert!(!method_bad(""));
        assert!(!method_bad("POST"));
        assert!(method_bad("post"));
        assert!(method_bad("FETCH"));
    }

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

#[cfg(test)]
mod shell_tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    /// なし first; the default next and marked; a written shell the engine lacks last and marked.
    #[test]
    fn the_shell_choices_put_the_default_first_and_keep_a_missing_shell() {
        let (t, _) = crate::i18n::table("ja");
        let have = names(&["powershell", "bash", "sh"]);
        let (e, at, missing) = shell_entries(&t, &have, "bash", "sh");
        let words: Vec<&str> = e.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(words, ["なし（run で実行する）", "bash（既定）", "powershell", "sh"]);
        assert_eq!((at, missing), (3, false));
        let (e, at, missing) = shell_entries(&t, &have, "bash", "zsh");
        assert_eq!((at, missing, e[4].0.as_str(), e[4].1.as_str()), (4, true, "zsh", "zsh（このエンジンにはありません）"));
        // No shell written: なし is the one now, and nothing is missing.
        assert_eq!(shell_entries(&t, &have, "bash", ""), (shell_entries(&t, &have, "bash", "").0, 0, false));
        // The engine has not said: the five, none marked missing.
        let (e, _, missing) = shell_entries(&t, &names(&koyomi_core::shell::SHELLS), "", "pwsh");
        assert_eq!((e.len(), missing), (6, false));
    }

    /// Picking なし empties `shell`; picking a shell sets it; the same one changes nothing; `run` and `script` stay.
    #[test]
    fn picking_a_shell_changes_only_shell() {
        let (t, _) = crate::i18n::table("en");
        let have = names(&["bash", "sh"]);
        let mut net = koyomi_core::jobnet::parse(
            "format = 1
[[job]]
id = \"a\"
shell = \"sh\"
script = \"echo 1\"
",
        )
        .unwrap();
        // As the form can hold it: both written (the form marks it; a file would not parse).
        net.jobs[0].run = vec!["x.exe".into(), "y".into()];
        let mut f = Fields::of(&net.jobs[0]);
        let (e, at, _) = shell_entries(&t, &have, "bash", &f.shell);
        assert_eq!(picked_shell(&e, &f.shell, at), None);
        let none = picked_shell(&e, &f.shell, 0).unwrap();
        assert_eq!(none, "");
        f.shell = none;
        assert_eq!(
            (f.shell.as_str(), f.script.as_str(), f.run.as_str()),
            (
                "", "echo 1", "x.exe
y"
            )
        );
        let (e, at, _) = shell_entries(&t, &have, "bash", &f.shell);
        assert_eq!(at, 0);
        f.shell = picked_shell(&e, &f.shell, 1).unwrap();
        assert_eq!(
            (f.shell.as_str(), f.script.as_str(), f.run.as_str()),
            (
                "bash", "echo 1", "x.exe
y"
            )
        );
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use crate::proto::DatabaseName;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn dbs() -> Vec<DatabaseName> {
        vec![DatabaseName { name: "main".into(), kind: "sqlserver".into() }, DatabaseName { name: "log".into(), kind: "sqlite".into() }]
    }

    fn words(e: &[(AccountChoice, String)]) -> Vec<&str> {
        e.iter().map(|(_, w)| w.as_str()).collect()
    }

    /// なし first, each name, the written-out account only while there is one, an unknown name last and marked.
    #[test]
    fn the_account_choices_are_none_names_inline_and_unknown() {
        let (t, _) = crate::i18n::table("ja");
        let have = names(&["ops", "batch"]);
        let f = Fields::default();
        let (e, at) = account_entries(&t, &have, &f);
        assert_eq!(words(&e), ["なし（エンジンと同じアカウント）", "ops", "batch"]);
        assert_eq!(at, 0);
        let f = Fields { run_as_name: "ops".into(), ..Fields::default() };
        assert_eq!(account_entries(&t, &have, &f).1, 1);
        let f = Fields { run_as_user: "batch".into(), run_as_secret: "b".into(), ..Fields::default() };
        let (e, at) = account_entries(&t, &have, &f);
        assert_eq!((words(&e)[3], at), ("直接書いたアカウント（batch）", 3));
        let f = Fields { run_as_name: "gone".into(), ..Fields::default() };
        let (e, at) = account_entries(&t, &have, &f);
        assert_eq!((words(&e)[3], at, e[3].0.clone()), ("gone（登録されていません）", 3, AccountChoice::Unknown("gone".into())));
    }

    /// A name and a user are never both set; the written-out account comes back from the draft.
    #[test]
    fn picking_a_name_sets_the_account_aside_and_inline_brings_it_back() {
        let have = names(&["ops"]);
        let mut f = Fields { run_as_user: "batch".into(), run_as_secret: "b".into(), ..Fields::default() };
        pick_account(&mut f, &AccountChoice::Named("ops".into()));
        assert_eq!((f.run_as_name.as_str(), f.run_as_user.as_str(), f.run_as_secret.as_str()), ("ops", "", ""));
        assert_eq!(account_now(&f, &have), AccountChoice::Named("ops".into()));
        // The state E3398 needs is not reachable, and the fields read.
        assert_eq!(f.run_as(), Ok(Some(jobnet::RunAs::Named("ops".into()))));
        pick_account(&mut f, &AccountChoice::Engine);
        assert_eq!(f.run_as(), Ok(None));
        let (t, _) = crate::i18n::table("en");
        assert!(account_entries(&t, &have, &f).0.iter().any(|(c, _)| *c == AccountChoice::Inline));
        pick_account(&mut f, &AccountChoice::Inline);
        assert_eq!((f.run_as_name.as_str(), f.run_as_user.as_str(), f.run_as_secret.as_str()), ("", "batch", "b"));
        // An unknown name is kept as it is (picking it is no change).
        let mut f = Fields { run_as_name: "gone".into(), ..Fields::default() };
        pick_account(&mut f, &AccountChoice::Unknown("gone".into()));
        assert_eq!(f.run_as_name, "gone");
    }

    #[test]
    fn the_database_choices_name_the_kind_and_keep_an_unknown_name() {
        let (t, _) = crate::i18n::table("ja");
        let f = Fields { db: "sqlite://x.db".into(), ..Fields::default() };
        let (e, at) = db_entries(&t, &dbs(), &f);
        let w: Vec<&str> = e.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!((w, at), (vec!["main — SQL Server", "log — SQLite", "URL を直接書く"], 2));
        let f = Fields { sql_database: "log".into(), ..Fields::default() };
        assert_eq!(db_entries(&t, &dbs(), &f).1, 1);
        let f = Fields { sql_database: "gone".into(), ..Fields::default() };
        let (e, at) = db_entries(&t, &dbs(), &f);
        assert_eq!((e[3].1.as_str(), at), ("gone（登録されていません）", 3));
    }

    /// The URL, password and TLS move to the draft, the SQL stays, and the URL comes back when it is picked again.
    #[test]
    fn picking_a_database_sets_the_url_aside_and_the_url_choice_brings_it_back() {
        let mut f = Fields {
            db: "sqlserver://u@h/d".into(),
            password: "secret:p".into(),
            tls: SqlTls::Trust,
            sql: "select 1".into(),
            file: "q.sql".into(),
            ..Fields::default()
        };
        pick_db(&mut f, &DbChoice::Named("main".into()));
        assert_eq!((f.sql_database.as_str(), f.db.as_str(), f.password.as_str(), f.tls), ("main", "", "", SqlTls::Verify));
        assert_eq!((f.sql.as_str(), f.file.as_str()), ("select 1", "q.sql"));
        pick_db(&mut f, &DbChoice::Url);
        assert_eq!(
            (f.sql_database.as_str(), f.db.as_str(), f.password.as_str(), f.tls),
            ("", "sqlserver://u@h/d", "secret:p", SqlTls::Trust)
        );
        // An unknown name stays.
        let mut f = Fields { sql_database: "gone".into(), ..Fields::default() };
        pick_db(&mut f, &DbChoice::Unknown("gone".into()));
        assert_eq!(f.sql_database, "gone");
    }
}

#[cfg(test)]
mod conn_tests {
    use super::*;
    use crate::proto::{GroupName, HostName};
    use koyomi_core::chore::{Chore, Pass};
    use koyomi_core::jobnet::Part;

    const KEY: &str = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn hosts() -> Vec<HostName> {
        vec![
            HostName { name: "web1".into(), kind: "ssh".into() },
            HostName { name: "web2".into(), kind: "ssh".into() },
            HostName { name: "pc9".into(), kind: "winrm".into() },
        ]
    }

    fn groups() -> Vec<GroupName> {
        vec![GroupName { name: "web".into(), kind: "ssh".into() }, GroupName { name: "desk".into(), kind: "winrm".into() }]
    }

    fn job(body: &str) -> koyomi_core::jobnet::Job {
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{body}");
        koyomi_core::jobnet::parse(&text).unwrap().jobs[0].clone()
    }

    fn fields(body: &str) -> Fields {
        Fields::of(&job(body))
    }

    fn ssh_key() -> String {
        format!("host_key = \"{KEY}\"\n")
    }

    fn words(e: &[(OnChoice, String)]) -> Vec<&str> {
        e.iter().map(|(_, w)| w.as_str()).collect()
    }

    #[test]
    fn the_pc_choices_are_the_kinds_own_hosts_and_groups_then_typed_then_a_stranger() {
        let (t, _) = crate::i18n::table("ja");
        let (h, g) = (hosts(), groups());
        let conns = pick::Conns { hosts: &h, groups: &g, ..Default::default() };
        let f = fields(&format!("kind = \"ssh\"\non = \"ssh://ops@x\"\ncommand = \"c\"\n{}", ssh_key()));
        let (e, at) = on_entries(&t, conns, false, &f);
        assert_eq!((words(&e), at), (vec!["web1", "web2", "グループ web", "直接書く"], 3));
        let f = fields("kind = \"ssh\"\non = \"web2\"\ncommand = \"c\"\n");
        assert_eq!(on_entries(&t, conns, false, &f).1, 1);
        let f = fields(&format!("kind = \"ssh\"\non = \"group:web\"\ncommand = \"c\"\n{}", ssh_key()));
        assert_eq!(on_entries(&t, conns, false, &f).1, 2);
        let f = fields("kind = \"ssh\"\non = \"gone\"\ncommand = \"c\"\n");
        let (e, at) = on_entries(&t, conns, false, &f);
        assert_eq!((e[4].1.as_str(), at), ("gone（登録されていません）", 4));
        let f = fields("kind = \"ssh\"\non = \"pc9\"\ncommand = \"c\"\n");
        assert_eq!(on_entries(&t, conns, false, &f).0[4].1, "pc9（WinRM の接続先）");
        let f = fields("kind = \"winrm\"\non = \"web1\"\ncommand = \"c\"\n");
        let (e, at) = on_entries(&t, conns, true, &f);
        assert_eq!((words(&e), at), (vec!["pc9", "グループ desk", "直接書く", "web1（SSH の接続先）"], 3));
        let f = fields(&format!("kind = \"ssh\"\non = \"group:old\"\ncommand = \"c\"\n{}", ssh_key()));
        assert_eq!(on_entries(&t, conns, false, &f).0[4].1, "グループ old（登録されていません）");
        let f = fields("kind = \"ssh\"\non = \"web1\"\ncommand = \"c\"\n");
        let (e, at) = on_entries(&t, pick::Conns::default(), false, &f);
        assert_eq!((words(&e), at), (vec!["直接書く", "web1（登録されていません）"], 1));
    }

    #[test]
    fn a_stranger_the_job_had_stays_listed_after_it_is_left() {
        let (t, _) = crate::i18n::table("ja");
        let (h, g) = (hosts(), groups());
        let conns = pick::Conns { hosts: &h, groups: &g, ..Default::default() };
        let mut f = fields("kind = \"ssh\"\non = \"gone\"\ncommand = \"c\"\n");
        f.pick_on(Some("web1"));
        let (e, at) = on_entries(&t, conns, false, &f);
        assert_eq!((at, e.last().unwrap().1.as_str()), (0, "gone（登録されていません）"));
    }

    #[test]
    fn picking_a_host_or_a_group_keeps_the_typed_address_and_writes_no_hidden_row() {
        let j = job(&format!("kind = \"ssh\"\non = \"group:web\"\ncommand = \"c\"\nmax = 4\npass = \"any\"\n{}", ssh_key()));
        let mut f = Fields::of(&j);
        assert_eq!(f.apply(&j).unwrap().part, j.part);
        f.pick_on(Some("web1"));
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Ssh { on, host_key, max, pass, .. } = &c else { panic!() };
        assert_eq!((on.as_str(), host_key.is_none(), *max, *pass), ("web1", true, None, Pass::All));
        c.check().unwrap();
        f.pick_on(None);
        f.on = "ssh://ops@web1".into();
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Ssh { max, pass, host_key, .. } = &c else { panic!() };
        assert_eq!((*max, *pass, host_key.is_some()), (None, Pass::All, true));
        c.check().unwrap();
        f.pick_on(Some("group:web"));
        assert_eq!(f.kept.on_typed, "ssh://ops@web1");
        assert_eq!(f.apply(&j).unwrap().part, j.part);
        f.pick_on(None);
        assert_eq!(f.on, "ssh://ops@web1");
    }

    #[test]
    fn the_written_out_address_stays_a_line_whatever_is_typed_in_it() {
        let mut f = fields(&format!("kind = \"ssh\"\non = \"ssh://ops@x\"\ncommand = \"c\"\n{}", ssh_key()));
        f.on = "s".into();
        assert_eq!(on_now(&f), OnChoice::Typed);
        let f = fields("kind = \"ssh\"\non = \"web1\"\ncommand = \"c\"\n");
        assert_eq!(on_now(&f), OnChoice::Named { group: false, name: "web1".into() });
    }

    fn far_words(e: &[(FarChoice, String)]) -> Vec<&str> {
        e.iter().map(|(_, w)| w.as_str()).collect()
    }

    #[test]
    fn the_far_choices_are_share_hosts_buckets_url_and_a_stranger() {
        let (t, _) = crate::i18n::table("ja");
        let (h, b) = (hosts(), names(&["logs"]));
        let conns = pick::Conns { hosts: &h, buckets: &b, ..Default::default() };
        let f = fields("kind = \"send\"\nfrom = \"D:/x\"\ninto = \"//srv/share\"\n");
        let (e, at) = far_entries(&t, conns, &f);
        assert_eq!((far_words(&e), at), (vec!["フォルダー・共有", "web1（SFTP）", "web2（SFTP）", "logs（S3）", "URL を直接書く"], 0));
        let f = fields("kind = \"send\"\nfrom = \"D:/x\"\ninto = \"/in\"\nhost = \"web2\"\n");
        assert_eq!(far_entries(&t, conns, &f).1, 2);
        let f = fields("kind = \"fetch\"\nfrom = \"in/a.csv\"\ninto = \"D:/in\"\nbucket = \"logs\"\n");
        assert_eq!(far_entries(&t, conns, &f).1, 3);
        let f = fields(&format!("kind = \"send\"\nfrom = \"D:/x\"\ninto = \"sftp://ops@web1/in\"\n{}", ssh_key()));
        assert_eq!(far_entries(&t, conns, &f).1, 4);
        let f = fields("kind = \"send\"\nfrom = \"D:/x\"\ninto = \"/in\"\nhost = \"gone\"\n");
        let (e, at) = far_entries(&t, conns, &f);
        assert_eq!((e[5].1.as_str(), at), ("gone（登録されていません）", 5));
        let f = fields("kind = \"send\"\nfrom = \"D:/x\"\ninto = \"/in\"\nhost = \"pc9\"\n");
        assert_eq!(far_entries(&t, conns, &f).0[5].1, "pc9（WinRM の接続先）");
        let f = fields("kind = \"fetch\"\nfrom = \"in/a.csv\"\ninto = \"D:/in\"\nbucket = \"old\"\n");
        assert_eq!(far_entries(&t, conns, &f).0[5].1, "old（登録されていません）");
    }

    #[test]
    fn the_far_side_keeps_each_choices_words_and_writes_only_what_the_choice_owns() {
        let j = job(&format!("kind = \"send\"\nfrom = \"D:/out/*.csv\"\ninto = \"sftp://ops@web1/in\"\nkey = \"C:/k\"\n{}", ssh_key()));
        let mut f = Fields::of(&j);
        assert_eq!(f.far_kind(), Far::Url);
        assert_eq!(f.apply(&j).unwrap().part, j.part);
        // A share: no login.
        f.pick_far(true, Far::Share, "");
        assert_eq!(f.into, "");
        f.into = "//srv/share/in".into();
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Send { key, host_key, password, key_id, .. } = &c else { panic!() };
        assert_eq!((key.is_none(), host_key.is_none(), password.is_none(), key_id.is_none()), (true, true, true, true));
        c.check().unwrap();
        // The address, picked back: its text and its key are back.
        f.pick_far(true, Far::Url, "");
        assert_eq!((f.into.as_str(), f.key.as_str()), ("sftp://ops@web1/in", "C:/k"));
        assert_eq!(f.apply(&j).unwrap().part, j.part);
        // An S3 address: its own login only.
        f.into = "s3://logs/daily/".into();
        f.key_id = "AKIAEXAMPLE".into();
        f.s3_password = "secret:s3".into();
        f.region = "ap-northeast-1".into();
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Send { key, host_key, password, key_id, region, .. } = &c else { panic!() };
        assert_eq!((key.is_none(), host_key.is_none()), (true, true));
        assert_eq!(
            (password.as_deref(), key_id.as_deref(), region.as_deref()),
            (Some("secret:s3"), Some("AKIAEXAMPLE"), Some("ap-northeast-1"))
        );
        c.check().unwrap();
        // Back to the SFTP address: the S3 rows are dropped, the key is back.
        f.into = "sftp://ops@web1/in".into();
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Send { key, key_id, region, password, .. } = &c else { panic!() };
        assert_eq!((key.as_deref(), key_id.is_none(), region.is_none(), password.is_none()), (Some("C:/k"), true, true, true));
        c.check().unwrap();
        // A host, a bucket, a share: the names are never both set.
        f.pick_far(true, Far::Host, "web1");
        assert_eq!(f.into, "");
        f.into = "/in".into();
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Send { host, bucket, key, .. } = &c else { panic!() };
        assert_eq!((host.as_deref(), bucket.is_none(), key.is_none()), (Some("web1"), true, true));
        c.check().unwrap();
        f.pick_far(true, Far::Bucket, "logs");
        assert_eq!((f.host.as_str(), f.bucket.as_str(), f.into.as_str()), ("", "logs", ""));
        f.into = "in/today.csv".into();
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Send { host, bucket, .. } = &c else { panic!() };
        assert_eq!((host.is_none(), bucket.as_deref()), (true, Some("logs")));
        c.check().unwrap();
        f.pick_far(true, Far::Share, "");
        assert_eq!((f.host.as_str(), f.bucket.as_str(), f.into.as_str()), ("", "", "//srv/share/in"));
        let Part::Chore(c) = f.apply(&j).unwrap().part else { panic!() };
        let Chore::Send { host, bucket, .. } = &c else { panic!() };
        assert_eq!((host.is_none(), bucket.is_none()), (true, true));
        c.check().unwrap();
        // Back and forth keeps each.
        f.pick_far(true, Far::Host, "web1");
        assert_eq!(f.into, "/in");
        f.pick_far(true, Far::Bucket, "logs");
        assert_eq!(f.into, "in/today.csv");
        f.pick_far(true, Far::Url, "");
        assert_eq!(f.into, "sftp://ops@web1/in");
        assert_eq!(f.from, "D:/out/*.csv");
    }

    /// Opening a job and closing it with nothing changed writes back exactly what was there.
    #[test]
    fn opening_and_closing_a_connection_job_changes_nothing() {
        let k = ssh_key();
        let mail = "kind = \"mail\"\nto = [\"a@example.com\"]\nsubject = \"s\"\n";
        let bodies = [
            "kind = \"ssh\"\non = \"web1\"\ncommand = \"c\"\n".to_owned(),
            format!("kind = \"ssh\"\non = \"group:web\"\ncommand = \"c\"\nmax = 4\npass = \"any\"\n{k}"),
            format!("kind = \"ssh\"\non = \"ssh://ops@web1\"\ncommand = \"c\"\n{k}"),
            "kind = \"ssh\"\non = \"gone\"\ncommand = \"c\"\n".to_owned(),
            "kind = \"winrm\"\non = \"pc9\"\ncommand = \"c\"\n".to_owned(),
            "kind = \"send\"\nfrom = \"D:/x\"\ninto = \"/in\"\nhost = \"web1\"\n".to_owned(),
            "kind = \"fetch\"\nfrom = \"in/a.csv\"\ninto = \"D:/in\"\nbucket = \"logs\"\n".to_owned(),
            "kind = \"send\"\nfrom = \"D:/x\"\ninto = \"//srv/share\"\n".to_owned(),
            format!("kind = \"send\"\nfrom = \"D:/x\"\ninto = \"sftp://ops@web1/in\"\nkey = \"C:/k\"\n{k}"),
            "kind = \"send\"\nfrom = \"D:/x\"\ninto = \"s3://logs/d/\"\nkey_id = \"AKIAEXAMPLE\"\npassword = \"secret:s3\"\nregion = \"ap-northeast-1\"\n".to_owned(),
            mail.to_owned(),
            format!("{mail}via = \"ops\"\n"),
            format!("{mail}via = \"gone\"\n"),
        ];
        for body in bodies {
            let j = job(&body);
            assert_eq!(Fields::of(&j).apply(&j).unwrap().part, j.part, "{body}");
        }
    }

    #[test]
    fn another_host_keeps_the_path_and_a_typed_address_stays_when_the_url_is_picked() {
        let mut f = fields("kind = \"fetch\"\nfrom = \"~/in\"\ninto = \"D:/in\"\nhost = \"web1\"\n");
        f.pick_far(false, Far::Host, "web2");
        assert_eq!((f.host.as_str(), f.from.as_str()), ("web2", "~/in"));
        let mut f = fields("kind = \"fetch\"\nfrom = \"//srv/in\"\ninto = \"D:/in\"\n");
        assert_eq!(f.far_kind(), Far::Share);
        f.from = "sftp://ops@web1/in".into();
        assert_eq!(f.far_kind(), Far::Share);
        f.pick_far(false, Far::Url, "");
        assert_eq!(f.from, "sftp://ops@web1/in");
    }

    #[test]
    fn the_via_choices_are_the_first_place_each_name_and_a_stranger() {
        let (t, _) = crate::i18n::table("ja");
        let mails = names(&["ops", "boss"]);
        let base = "kind = \"mail\"\nto = [\"a@example.com\"]\nsubject = \"s\"\n";
        let w = |e: &[(String, String)]| e.iter().map(|(_, w)| w.clone()).collect::<Vec<_>>();
        let (e, at) = via_entries(&t, &mails, &fields(base));
        assert_eq!((w(&e), at), (vec!["最初の送り先（ops）".to_owned(), "ops".into(), "boss".into()], 0));
        assert_eq!(via_entries(&t, &mails, &fields(&format!("{base}via = \"boss\"\n"))).1, 2);
        let (e, at) = via_entries(&t, &mails, &fields(&format!("{base}via = \"gone\"\n")));
        assert_eq!((e[3].1.as_str(), at), ("gone（登録されていません）", 3));
        let (e, at) = via_entries(&t, &[], &Fields::default());
        assert_eq!((e.len(), e[0].1.as_str(), at), (1, "最初の送り先", 0));
    }
}
