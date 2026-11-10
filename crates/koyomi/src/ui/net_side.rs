//! The pane at the right of the ジョブネット page: the net's next starts, its runs of today, its
//! calendar and the job picked on its graph or in its table. With no net open (the overview), it
//! shows the row picked there, with the net's buttons, so a net is looked at without leaving the list.

use ito_a11y::Role;
use koyomi_core::scan::Def;

use super::{card, folded_pane, graph, one_size, review, section_head, Browser};
use crate::show;

/// How many of today's runs the pane lists.
const RUNS: usize = 5;

impl Browser {
    /// The net the pane shows: the one open, else the overview's picked row.
    pub(super) fn side_net(&self) -> Option<Def> {
        let id = self.selected.clone().or_else(|| self.nets_pick.clone())?;
        self.snap.as_ref()?.defs.iter().find(|d| d.id == id).cloned()
    }

    /// The pane, or its strip when folded. Drawn before the page so the page takes the room that is left.
    pub(super) fn net_pane(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let def = self.side_net();
        let title = def.as_ref().map_or_else(|| t.tr("inspect.title").to_owned(), |d| d.label().to_owned());
        if !self.net_pane {
            if folded_pane(ui, "net_side_strip", &title, t.tr("inspect.open")) {
                self.net_pane = true;
                self.save_kept();
            }
            return;
        }
        let wide = ui.available_width();
        let mut fold = false;
        egui::Panel::right("net_side")
            .resizable(true)
            .default_size(320.0_f32.min(wide * 0.4))
            .size_range(240.0..=(wide * 0.6).max(240.0))
            .frame(egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.border)).inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| {
                one_size(ui.style_mut());
                ui.set_min_height(ui.available_height());
                fold = self.net_side_head(ui, t, def.as_ref(), &title);
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("net_side_scroll").auto_shrink(false).show(ui, |ui| match def {
                    Some(def) => self.net_side_body(ui, t, &def),
                    None => {
                        ui.label(egui::RichText::new(t.tr("net.side_none")).color(c.dim));
                    }
                });
            });
        if fold {
            self.net_pane = false;
            self.save_kept();
        }
    }

    /// The head: the net's name (in the overview a click opens its page), the fold mark at the right.
    /// True when the mark was pressed.
    fn net_side_head(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: Option<&Def>, title: &str) -> bool {
        let c = ito_theme::colors();
        let opened = self.selected_def().is_some();
        let (mut fold, mut go) = (false, None);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                fold = ito_prefs::mark_button(ui, ito_prefs::Mark::ChevronRight, t.tr("inspect.fold"), 22.0).clicked();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let Some(def) = def else {
                        let r = ui.label(egui::RichText::new(title).strong().color(c.dim));
                        ito_a11y::say(&r, Role::Heading, title);
                        return;
                    };
                    section_head(ui, title);
                    if !opened {
                        let open =
                            ito_prefs::IconButton::new(ito_prefs::Mark::ArrowRight, t.tr("canvas.enter")).said(t.tr("today.open_net"));
                        if open.show(ui).clicked() {
                            go = Some(def.id.clone());
                        }
                    }
                });
            });
        });
        if let Some(id) = go {
            self.follow(|b| b.open_net(&id));
        }
        fold
    }

    fn net_side_body(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def) {
        let c = ito_theme::colors();
        let look = ito_prefs::Look { c, q: "" };
        // Open on its page, the net's buttons and problems are in the page; in the overview they are here.
        let opened = self.selected_def().is_some_and(|d| d.id == def.id);
        let held = self.snap.as_ref().is_some_and(|s| s.held.contains(&def.id));
        let now = ito_common::now::now_local().naive_local();
        let mut runs: Vec<_> = self.runs.iter().filter(|r| r.jobnet == def.id).cloned().collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.started));
        ui.horizontal_wrapped(|ui| {
            if let Some(run) = runs.first() {
                let text = t.trf("inspect.last", &[&show::tag(t, run.state)]);
                let r = ito_prefs::tag(ui, look, ito_prefs::Tag::new(&text).color(graph::colour(&c, run.state)));
                ito_a11y::say(&r, Role::Label, &text);
            }
            let r = ui.add(egui::Label::new(egui::RichText::new(&def.file).small().color(c.dim)).wrap());
            ito_a11y::say(&r, Role::Label, &def.file);
        });
        if !opened {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                self.run_now_button(ui, t, def, held);
                self.approve_button(ui, t, def);
                self.edit_button(ui, t, def);
                self.hold_button(ui, t, def, held);
            });
            if held {
                let text = format!("{} {}", koyomi_core::run::State::Held.mark(), t.tr("run.held_note"));
                let r = ui.label(egui::RichText::new(&text).color(graph::colour(&c, koyomi_core::run::State::Held)));
                ito_a11y::say(&r, Role::Label, &text);
            }
            self.show_result(ui);
            for p in &def.problems {
                let line = format!("{} {}", ito_a11y::State::Error.mark(), p.in_file(&def.file));
                let r = ui.add(egui::Label::new(egui::RichText::new(&line).color(c.err)).wrap());
                ito_a11y::say(&r, Role::Label, &line);
            }
        }
        let section = |ui: &mut egui::Ui, key: &str| {
            ui.add_space(12.0);
            let r = ui.label(egui::RichText::new(t.tr(key)).strong().color(c.fg));
            ito_a11y::say(&r, Role::Heading, t.tr(key));
            ui.add_space(2.0);
        };
        if def.net.is_some() {
            section(ui, "net.sec_timing");
            self.timing_card(ui, t, def);
        }
        // Today's runs, newest first: a click shows the run.
        section(ui, "net.side_runs");
        let mut open = None;
        card(&c).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if runs.is_empty() {
                ui.label(egui::RichText::new(t.tr("net.side_no_runs")).color(c.dim));
            }
            for run in runs.iter().take(RUNS) {
                let state = show::tag(t, run.state);
                let took = show::duration(t, run.duration().unwrap_or(now - run.started));
                let at = run.started.format("%H:%M").to_string();
                let r = ui
                    .horizontal(|ui| {
                        ui.label(egui::RichText::new(&at).monospace());
                        ui.label(egui::RichText::new(&state).color(graph::colour(&c, run.state)));
                        ui.label(egui::RichText::new(&took).color(c.dim));
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text(t.tr("net.side_run_hint"))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                ito_a11y::say(&r, Role::Button, &format!("{at} {state} {took}"));
                if r.clicked() {
                    open = Some(run.id.clone());
                }
            }
            if let Some(avg) = self.past.get(&def.id) {
                let text = t.trf("today.average", &[&show::duration(t, *avg)]);
                let r = ui.label(egui::RichText::new(&text).color(c.dim));
                ito_a11y::say(&r, Role::Label, &text);
            }
        });
        // The calendar its days are counted by: a click shows it.
        section(ui, "net.side_calendar");
        let mut cal = None;
        match &def.calendar {
            Some(name) => {
                let r = ito_prefs::IconButton::new(ito_prefs::Mark::Calendar, name).show(ui);
                if r.on_hover_text(t.tr("net.side_calendar_open")).clicked() {
                    cal = Some(name.clone());
                }
            }
            None => {
                ui.label(egui::RichText::new(t.tr("field.no_calendar")).color(c.dim));
            }
        }
        if opened {
            self.net_side_job(ui, t, def, &section);
        }
        if let Some(id) = open {
            self.follow(|b| b.open_run(&id));
        }
        if let Some(name) = cal {
            self.follow(|b| b.open_calendar(&name));
        }
    }

    /// The job picked on the graph or in the jobs table, in a card: what it is, what it waits for and
    /// what waits for it, what it runs and where, and the rest of its settings.
    fn net_side_job(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, def: &Def, section: &dyn Fn(&mut egui::Ui, &str)) {
        let c = ito_theme::colors();
        let Some(net) = def.net.as_ref().filter(|n| !n.jobs.is_empty()) else { return };
        section(ui, "net.side_job");
        let picked = self.job_sel.as_deref().and_then(|p| net.jobs.iter().position(|j| j.id == p));
        let Some(i) = picked else {
            ui.label(egui::RichText::new(t.tr("net.side_job_none")).color(c.dim));
            return;
        };
        let j = &net.jobs[i];
        let rows = review::job_rows(t, def);
        let row = rows.get(i);
        let after: Vec<String> = j.after.iter().map(|l| l.text()).collect();
        let next: Vec<String> = net.jobs.iter().filter(|k| k.after.iter().any(|l| l.job == j.id)).map(|k| k.id.clone()).collect();
        let held = self.job_held(&def.id, &j.id);
        card(&c).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let r = ui.label(egui::RichText::new(&j.id).monospace().strong());
                ito_a11y::say(&r, Role::Heading, &j.id);
                ui.label(egui::RichText::new(t.tr(&format!("part.{}", j.part.key()))).color(c.dim));
            });
            if held {
                let text = t.tr("run.job_held_note");
                let r = ui.add(egui::Label::new(egui::RichText::new(text).color(graph::colour(&c, koyomi_core::run::State::Held))).wrap());
                ito_a11y::say(&r, Role::Label, text);
            }
            let mut fact = |key: &str, v: String| {
                if !v.is_empty() {
                    let text = format!("{}: {v}", t.tr(key));
                    let r = ui.add(egui::Label::new(egui::RichText::new(&text).color(c.dim)).wrap());
                    ito_a11y::say(&r, Role::Label, &text);
                }
            };
            fact("inspect.name", j.name.clone().unwrap_or_default());
            fact("inspect.before", after.join(", "));
            fact("inspect.after", next.join(", "));
            fact("net.col_dir", row.and_then(|r| r.dir.clone()).unwrap_or_default());
            for l in row.map(|r| r.rest.clone()).unwrap_or_default() {
                let r = ui.add(egui::Label::new(egui::RichText::new(&l).color(c.dim)).wrap());
                ito_a11y::say(&r, Role::Label, &l);
            }
            if let Some(cmd) = row.and_then(|r| r.command.clone()) {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(c.bg)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let r = ui.add(egui::Label::new(egui::RichText::new(&cmd).monospace()).wrap());
                        ito_a11y::say(&r, Role::Label, &cmd);
                    });
            }
        });
    }
}
