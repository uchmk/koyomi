//! The calendars in the window: a list under 今日 on the left, and the month
//! table in the detail when one is picked. A click on a day writes
//! `calendars/<name>.toml` (a weekend or holiday becomes `open`, a weekday
//! `closed`, and a second click takes it back); the engine's watcher reads the
//! file again and the table follows what the engine counts by. The boxes above
//! the month set `holidays` and `weekly_off`. 新しいカレンダー writes a new
//! `calendars/<name>.toml` from a box; the same box renames one, and 消す keeps
//! the file under `versions/_calendars/` before it goes. Neither touches the
//! files that name the calendar: the box lists them. A day's menu names it
//! (創立記念日), and 祝日の表を取り込む copies a `syukujitsu.csv` into
//! `calendars/` once it reads; both are typed under the month.

use chrono::{Datelike, NaiveDate, Weekday};
use ito_a11y::Role;
use koyomi_core::calendar::{Calendar, Day, Table, TABLE_FILE};
use koyomi_core::scan::{Def, Standing};

use super::{bad_file_name, Browser, Why};
use crate::show;

const WEEK: [Weekday; 7] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];
/// The month table's columns: Sunday first, as a wall calendar.
const GRID: [Weekday; 7] = [Weekday::Sun, Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat];
/// The 新しいカレンダー box: the name (the file's stem), Japan's holidays, and the days off each week.
pub(super) struct NewCal {
    /// The calendar the box renames; `None` makes a new one.
    from: Option<String>,
    /// Opened from the editor: the calendar made becomes the open net's.
    pub(super) for_net: bool,
    pub(super) name: String,
    holidays: bool,
    /// Monday first, as [`WEEK`].
    off: [bool; 7],
    pub(super) error: Option<String>,
    /// The name field takes the keyboard on the first frame.
    fresh: bool,
}

/// What is typed under the month of `cal`: a day's name, or the path of a holiday table.
pub(super) enum Inline {
    Name { cal: String, day: NaiveDate, text: String, fresh: bool },
    Table { cal: String, path: String, fresh: bool },
}

impl Inline {
    fn cal(&self) -> &str {
        match self {
            Inline::Name { cal, .. } | Inline::Table { cal, .. } => cal,
        }
    }
}

impl Browser {
    /// The calendars by name, under today's runs. The heading stays when there are none, with a
    /// line saying so; the button (and the heading's menu) makes one.
    pub(super) fn calendar_list(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let Some(snap) = &self.snap else { return };
        let c = ito_theme::colors();
        ui.add_space(10.0);
        let mut make = false;
        // The list's title with a small ＋ at the right, as the jobnet list has (its words on hover and to a reader).
        let label = t.tr("calendar.new");
        ui.horizontal(|ui| {
            let head =
                egui::Label::new(egui::RichText::new(t.tr("calendar.heading")).small().strong().color(c.dim)).sense(egui::Sense::click());
            let r = ui.add(head);
            ito_a11y::say(&r, Role::Heading, t.tr("calendar.heading"));
            r.context_menu(|ui| {
                if ui.button(label).clicked() {
                    make = true;
                    ui.close();
                }
            });
            let r = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.small_button("＋").on_hover_text(label)).inner;
            ito_a11y::say(&r, Role::Button, label);
            if r.clicked() {
                make = true;
            }
        });
        ui.add_space(4.0);
        if snap.calendars.is_empty() {
            let r = ui.add(egui::Label::new(egui::RichText::new(t.tr("calendar.none")).color(c.dim)).wrap());
            ito_a11y::say(&r, Role::Label, t.tr("calendar.none"));
        }
        let mut pick = None;
        let mut act = None;
        for name in snap.calendars.keys() {
            let on = self.cal_sel.as_deref() == Some(name.as_str());
            let r = ui.selectable_label(on, format!("  {name}"));
            ito_a11y::say(&r, Role::Row, name);
            if r.clicked() {
                pick = Some(name.clone());
            }
            r.context_menu(|ui| {
                for (i, key) in ["calendar.rename", "calendar.delete"].into_iter().enumerate() {
                    if ui.button(t.tr(key)).clicked() {
                        act = Some((i, name.clone()));
                        ui.close();
                    }
                }
            });
        }
        if make {
            self.new_cal_box();
        }
        match act {
            Some((0, name)) => self.rename_cal_box(&name),
            Some((_, name)) => self.delete_cal_box(&name),
            None => {}
        }
        if pick.is_some() {
            self.cal_sel = pick;
            self.cal_month = None;
            self.cal_result = None;
            self.cal_inline = None;
            self.run_sel = None;
            self.job_sel = None;
            self.result = None;
            self.hist = None;
            // Off the today page too, or the chart keeps the detail and the click shows nothing.
            self.gantt = false;
        }
    }

    /// The month as a table, the named days, the holiday table's end, and the
    /// definitions that count by this calendar with the picked one's next times.
    pub(super) fn calendar_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(name) = self.cal_sel.clone() else { return };
        let Some(cal) = self.snap.as_ref().and_then(|s| s.calendars.get(&name)).cloned() else {
            match &self.cal_result {
                // Just made, renamed or deleted: what was done; a file there is one the engine has
                // not read yet.
                Some(Ok(done)) => {
                    let there = ito_common::app_dir("koyomi").is_some_and(|d| d.join(format!("calendars/{name}.toml")).exists());
                    let line = if there { format!("{done} {}", t.tr("calendar.reading")) } else { done.clone() };
                    ui.label(egui::RichText::new(line).color(c.dim))
                }
                _ => ui.label(egui::RichText::new(t.trf("calendar.missing", &[&name])).color(c.err)),
            };
            return;
        };
        let today = chrono::Local::now().date_naive();
        let (y, m) = self.cal_month.unwrap_or((today.year(), today.month()));
        let days = show::month_days(&cal, y, m);
        // The day whose starts show in the pane: the one picked, or today in this month.
        let picked =
            self.cal_day.filter(|d| d.year() == y && d.month() == m).or_else(|| days.iter().any(|(d, _)| *d == today).then_some(today));
        // On its week, the calendar shows the seven days round the picked one (or the month's first), Sunday first.
        let on_week = self.cal_week;
        let anchor = picked.or_else(|| NaiveDate::from_ymd_opt(y, m, 1)).unwrap_or(today);
        let sunday = anchor - chrono::Days::new(u64::from(anchor.weekday().num_days_from_sunday()));
        let week: Vec<(NaiveDate, Day)> = sunday.iter_days().take(7).map(|d| (d, cal.day(d))).collect();
        // The picked day, the named days and the nets on this calendar: a pane at the right, shown
        // before the month so the month takes the room that is left.
        let wide = ui.available_width();
        let mut side = Side::default();
        if !self.cal_pane {
            if super::folded_pane(ui, "calendar_side_strip", &name, t.tr("inspect.open")) {
                self.cal_pane = true;
                self.save_kept();
            }
        } else {
            let mut fold = false;
            egui::Panel::right("calendar_side")
                .resizable(true)
                .default_size(340.0_f32.min(wide * 0.4))
                .size_range(240.0..=(wide * 0.6).max(240.0))
                .frame(egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.border)).inner_margin(egui::Margin::same(12)))
                .show(ui, |ui| {
                    super::one_size(ui.style_mut());
                    ui.set_min_height(ui.available_height());
                    // The head: the calendar's name, the fold mark at the right.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            fold = ito_prefs::mark_button(ui, ito_prefs::Mark::ChevronRight, t.tr("inspect.fold"), 22.0).clicked();
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let r = ui.label(egui::RichText::new(&name).strong());
                                ito_a11y::say(&r, Role::Heading, &name);
                            });
                        });
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().id_salt("calendar_side_scroll").auto_shrink(false).show(ui, |ui| {
                        side = self.side_pane(ui, t, &name, &cal, &days, picked, today);
                    });
                });
            if fold {
                self.cal_pane = false;
                self.save_kept();
            }
        }
        // The month and what goes with it scroll under the pane when the window is short.
        super::page().show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("calendar_page").auto_shrink(false).show(ui, |ui| {
                // This calendar's own cautions in a band: this is where the person looks to fix them. When
                // the holiday table ends soon, the band carries the button that brings a new one.
                let cautions: Vec<String> =
                    self.snap.iter().flat_map(|s| s.calendar_cautions.get(&name).into_iter().flatten().cloned()).collect();
                let table_ends = cal.until().is_some_and(|u| (u - today).num_days() <= crate::engine::CAUTION_DAYS);
                let mut bring = false;
                if !cautions.is_empty() {
                    egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.wait)).inner_margin(6.0).corner_radius(4.0).show(
                        ui,
                        |ui| {
                            for l in &cautions {
                                let line = format!("{} {l}", ito_a11y::State::Warning.mark());
                                let r = ui.add(egui::Label::new(egui::RichText::new(line).color(c.wait_text())).wrap());
                                ito_a11y::say(&r, Role::Label, l);
                            }
                            if table_ends {
                                let r = ui.add_enabled(!self.blocked(), egui::Button::new(t.tr("calendar.table")));
                                ito_a11y::say(&r, Role::Button, t.tr("calendar.table"));
                                bring = r.clicked();
                            }
                        },
                    );
                    ui.add_space(4.0);
                }
                // 会社 [営業日 19 日]  ‹ 2026年11月 ›; the screen reader hears the month as one heading.
                let head = show::month_head(t, &cal, y, m);
                let shown = if on_week { &week } else { &days };
                let business = shown.iter().filter(|(_, d)| d.is_business()).count();
                // ◀ and ▶ move by the week on its week, and the picked day with it.
                let (prev, next) = if on_week { ("calendar.prev_week", "calendar.next_week") } else { ("calendar.prev", "calendar.next") };
                let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
                ui.horizontal(|ui| {
                    let r = ui.label(egui::RichText::new(&name).heading().strong());
                    ito_a11y::say(&r, Role::Heading, &name);
                    let words = t.trf("calendar.business_n", &[&business.to_string()]);
                    let r = ito_prefs::tag(ui, look, ito_prefs::Tag::new(&words).color(c.done));
                    ito_a11y::say(&r, Role::Label, &words);
                    ui.add_space(10.0);
                    egui::Frame::new()
                        .fill(c.panel)
                        .stroke(egui::Stroke::new(1.0, c.border))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(4, 1))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let r = ui.add(egui::Button::new(egui::RichText::new("◀").color(c.dim)).frame(false));
                                ito_a11y::say(&r, Role::Button, t.tr(prev));
                                if r.clicked() && on_week {
                                    self.cal_go(anchor - chrono::Days::new(7));
                                } else if r.clicked() {
                                    self.cal_month = Some(if m == 1 { (y - 1, 12) } else { (y, m - 1) });
                                }
                                // A click on the month opens a year and its twelve months, to jump far.
                                let ym = t.trf("calendar.ym", &[&y.to_string(), &format!("{m:02}")]);
                                let r = ui
                                    .add(egui::Label::new(egui::RichText::new(ym).heading().strong()).sense(egui::Sense::click()))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(t.tr("calendar.pick_month"));
                                ito_a11y::say(&r, Role::Heading, &head);
                                let words = ito_prefs::DateWords::for_lang(t.tr("app.lang"));
                                let mut to = (y, m);
                                if ito_prefs::month_picker(ui, look, "calendar-month", &words, &r, &mut to) {
                                    self.cal_month = Some(to);
                                }
                                let r = ui.add(egui::Button::new(egui::RichText::new("▶").color(c.dim)).frame(false));
                                ito_a11y::say(&r, Role::Button, t.tr(next));
                                if r.clicked() && on_week {
                                    self.cal_go(anchor + chrono::Days::new(7));
                                } else if r.clicked() {
                                    self.cal_month = Some(if m == 12 { (y + 1, 1) } else { (y, m + 1) });
                                }
                            });
                        });
                    if on_week {
                        let span = |d: NaiveDate| format!("{}/{}({})", d.month(), d.day(), weekday(t, d.weekday()));
                        let range = format!("{} – {}", span(sunday), span(sunday + chrono::Days::new(6)));
                        let r = ui.label(egui::RichText::new(&range).monospace().color(c.dim));
                        ito_a11y::say(&r, Role::Label, &range);
                    }
                    // 月 | 週 at the right of the head.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let options = [(false, t.tr("calendar.view_month")), (true, t.tr("calendar.view_week"))];
                        if let Some(to) = ito_prefs::seg(ui, look, "calendar-view", on_week, &options) {
                            self.cal_week = to;
                            self.save_kept();
                        }
                    });
                });
                ui.add_space(4.0);
                // The file itself; then the rules of the week, set in place.
                let mut rules = None;
                let mut act = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, key) in ["calendar.rename", "calendar.delete", "calendar.table", "calendar.print"].into_iter().enumerate() {
                        let r = ui.add_enabled(!self.blocked(), egui::Button::new(t.tr(key)));
                        ito_a11y::say(&r, Role::Button, t.tr(key));
                        if r.clicked() {
                            act = Some(i);
                        }
                    }
                });
                ui.add_space(4.0);
                let mut holidays = cal.holidays;
                // egui names a checkbox for the screen reader itself.
                let mut changed = ui.checkbox(&mut holidays, t.tr("newcal.holidays")).changed();
                let mut off = WEEK.map(|w| cal.weekly_off.contains(&w));
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{}:", t.tr("newcal.off")));
                    // Sunday first, as the table below; `off` stays Monday first.
                    for w in GRID {
                        let words = egui::RichText::new(weekday(t, w)).color(weekday_colour(&c, w));
                        changed |= ui.checkbox(&mut off[w.num_days_from_monday() as usize], words).changed();
                    }
                });
                if changed {
                    rules = Some((holidays, off));
                }
                ui.add(egui::Label::new(egui::RichText::new(t.tr("calendar.hint")).small().color(c.dim)).wrap());
                if let Some((holidays, off)) = rules {
                    let off = days_off(off);
                    self.cal_result = Some(edit_file(t, &name, |text| set_rules(text, holidays, &off)));
                }
                if bring {
                    act = Some(2);
                }
                match act {
                    Some(0) => self.rename_cal_box(&name),
                    Some(1) => self.delete_cal_box(&name),
                    Some(3) => {
                        // The month's sheet of every approved net, as `koyomi plan --print` makes it.
                        let days = self.snap.as_ref().map(|s| crate::plan::month(&s.defs, &s.calendars, y, m)).unwrap_or_default();
                        self.result = Some(Ok(t.tr("run.writing").to_owned()));
                        Self::background(&self.io, move || super::print_plan(&days, y, m));
                    }
                    Some(_) => self.cal_inline = Some(Inline::Table { cal: name.clone(), path: String::new(), fresh: true }),
                    None => {}
                }
                ui.add_space(8.0);
                let out = if on_week {
                    let starts = self.snap.as_ref().map(|s| crate::plan::days(&s.defs, &s.calendars, sunday, 7)).unwrap_or_default();
                    week_grid(ui, t, &name, &week, &starts, picked, today)
                } else {
                    let starts = self.snap.as_ref().map(|s| crate::plan::month(&s.defs, &s.calendars, y, m)).unwrap_or_default();
                    month_grid(ui, t, &name, (y, m), &days, &starts, picked, today)
                };
                self.month_notes(ui, t, &name, &cal);
                if let Some(d) = out.pick {
                    self.cal_go(d);
                }
                if let Some((d, day)) = out.toggle.or(side.toggle.take()) {
                    self.cal_result = Some(toggle_day(t, &name, &cal, d, &day));
                }
                if out.naming.is_some() {
                    self.cal_inline = out.naming;
                }
                if let Some(id) = out.open.or(side.open.take()) {
                    self.open_net(&id);
                }
            });
        });
    }

    /// Picks `d` and shows its month (a week can reach into the next one).
    fn cal_go(&mut self, d: NaiveDate) {
        self.cal_day = Some(d);
        self.cal_month = Some((d.year(), d.month()));
    }

    /// Under the month: what the colours mean, the field typed in, and what was just done.
    fn month_notes(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, name: &str, cal: &Calendar) {
        let c = ito_theme::colors();
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(c.panel)
            .stroke(egui::Stroke::new(1.0, c.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let legend = [
                        (c.fg, "calendar.business"),
                        (c.dim, "calendar.legend_off"),
                        (c.err, "calendar.legend_holiday"),
                        (c.magenta, "calendar.legend_closed"),
                        (c.done, "calendar.open"),
                        (c.blue, "calendar.legend_today"),
                        (c.run, "calendar.picked"),
                    ];
                    for (colour, key) in legend {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, ito_theme::mix(c.panel, colour, 0.18));
                        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, colour), egui::StrokeKind::Inside);
                        ui.label(egui::RichText::new(t.tr(key)).small().color(c.dim));
                        ui.add_space(8.0);
                    }
                });
            });
        self.inline_view(ui, t, name, cal);
        for r in [&self.cal_result, &self.result].into_iter().flatten() {
            match r {
                Ok(s) => ui.label(egui::RichText::new(s).color(c.dim)),
                Err(e) => ui.label(egui::RichText::new(e).color(c.err)),
            };
        }
    }

    /// The pane at the right of the month: the picked day, the month's named days with the holiday
    /// table's end, and the definitions that count by this calendar. The day pressed to move
    /// between open and closed, and the net double-clicked, if any.
    #[allow(clippy::too_many_arguments)]
    fn side_pane(
        &self,
        ui: &mut egui::Ui,
        t: &ito_i18n::Table,
        name: &str,
        cal: &Calendar,
        days: &[(NaiveDate, Day)],
        picked: Option<NaiveDate>,
        today: NaiveDate,
    ) -> Side {
        let c = ito_theme::colors();
        let mut side = Side::default();
        if let Some(d) = picked {
            side = self.day_box(ui, t, cal, d, &cal.day(d));
            ui.add_space(10.0);
        }
        let named: Vec<_> = days.iter().filter_map(|(d, day)| show::day_name(t, day).map(|n| (*d, day, n))).collect();
        let until = show::until_line(t, cal, today);
        if !named.is_empty() || until.is_some() {
            card(ui, &c, |ui| {
                let r = ui.label(egui::RichText::new(t.tr("calendar.named_head")).strong());
                ito_a11y::say(&r, Role::Heading, t.tr("calendar.named_head"));
                ui.add_space(4.0);
                for (d, day, n) in &named {
                    let line = format!("{}/{} ({}) {n}", d.month(), d.day(), weekday(t, d.weekday()));
                    ui.add(egui::Label::new(egui::RichText::new(line).color(day_colour(&c, day))).wrap());
                }
                if let Some(line) = until {
                    ui.add_space(4.0);
                    let colour = if cal.stale(today) { c.wait_text() } else { c.dim };
                    ui.add(egui::Label::new(egui::RichText::new(line).small().color(colour)).wrap());
                }
            });
            ui.add_space(10.0);
        }
        let open = self.used_by(ui, t, name, cal);
        side.open = side.open.or(open);
        side
    }

    /// The definitions on this calendar; the one picked in the tree with its next times. The one
    /// double-clicked, to open on the net page.
    fn used_by(&self, ui: &mut egui::Ui, t: &ito_i18n::Table, name: &str, cal: &Calendar) -> Option<String> {
        let c = ito_theme::colors();
        let snap = self.snap.as_ref()?;
        let users: Vec<_> = snap.defs.iter().filter(|d| d.calendar.as_deref() == Some(name)).collect();
        let mut open = None;
        card(ui, &c, |ui| {
            let r = ui.label(egui::RichText::new(t.tr("calendar.used_by")).strong());
            ito_a11y::say(&r, Role::Heading, t.tr("calendar.used_by"));
            ui.add_space(4.0);
            if users.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(t.tr("calendar.used_none")).color(c.dim)).wrap());
                return;
            }
            for d in &users {
                let on = self.selected.as_deref() == Some(d.id.as_str());
                let text = egui::RichText::new(d.label());
                let label = egui::Label::new(if on { text.strong() } else { text }).wrap().selectable(false).sense(egui::Sense::click());
                let r = ui.add(label);
                if r.hovered() || r.has_focus() {
                    let y = r.rect.bottom();
                    ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.right(), y)], egui::Stroke::new(1.0, c.run));
                }
                if net_link(ui, t, &r, d.label()) {
                    open = Some(d.id.clone());
                }
            }
            let picked = users.iter().find(|d| self.selected.as_deref() == Some(d.id.as_str()));
            let Some((d, net)) = picked.and_then(|d| d.net.as_ref().map(|n| (d, n))) else { return };
            if net.when.is_empty() {
                return;
            }
            ui.add_space(6.0);
            let head = format!("{}: {}", d.label(), t.trf("run.next", &["10"]));
            let r = ui.add(egui::Label::new(egui::RichText::new(&head).strong().color(c.dim)).wrap());
            ito_a11y::say(&r, Role::Heading, &head);
            if d.standing != Standing::Confirmed {
                ui.add(egui::Label::new(egui::RichText::new(t.tr("run.next_unconfirmed")).color(c.wait_text())).wrap());
            }
            let power = Some(show::Sleep::of(&snap.power, net.wake));
            let now = chrono::Local::now().naive_local();
            for row in show::next_rows(t, &net.when, (&net.quiet, net.on_quiet), Some(cal), now, 10, power.as_ref()) {
                ui.label(egui::RichText::new(row).monospace().small());
            }
        });
        open
    }
}

/// A net named in the pane or the week, as a link to its page: the hand on hover, a tip with
/// `said` and the hint, a menu that opens it (the keys' way in), and `said` to a reader. True when
/// it is to open: a double click, Enter on it, or the menu. The caller paints the hover.
fn net_link(ui: &egui::Ui, t: &ito_i18n::Table, r: &egui::Response, said: &str) -> bool {
    let tip = format!("{said}\n{}", t.tr("calendar.open_net_hint"));
    let r = r.clone().on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
    ito_a11y::say(&r, Role::Button, said);
    let mut open = r.double_clicked() || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    r.context_menu(|ui| {
        if ui.button(t.tr("calendar.open_net")).clicked() {
            open = true;
            ui.close();
        }
    });
    open
}

/// What a click in the month or the week asked for.
#[derive(Default)]
struct Grid {
    pick: Option<NaiveDate>,
    toggle: Option<(NaiveDate, Day)>,
    naming: Option<Inline>,
    /// The id of the net double-clicked, to open on the net page.
    open: Option<String>,
}

/// What was pressed in the pane: the day to move between open and closed, the net to open.
#[derive(Default)]
struct Side {
    toggle: Option<(NaiveDate, Day)>,
    open: Option<String>,
}

/// A card in the pane beside the month: the panel's colour, a line round it, as wide as the pane.
fn card(ui: &mut egui::Ui, c: &ito_theme::Colors, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(c.panel).stroke(egui::Stroke::new(1.0, c.border)).corner_radius(8.0).inner_margin(12.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), add);
    });
}

/// Sunday and Saturday stand out in the table's head, its dates and the week's boxes.
fn weekday_colour(c: &ito_theme::Colors, w: Weekday) -> egui::Color32 {
    match w {
        Weekday::Sun => c.err,
        Weekday::Sat => c.blue,
        _ => c.fg,
    }
}

/// The month as one table without gaps, Sunday first, as wide as the room: a band of weekdays,
/// then a cell a day. Each cell has its date at the top left, its name (or 選択中) as a tag at the
/// top right, a tint by what the day is, and the times that start on it. The days of the months
/// either side fill the first and last weeks, faint and not pressed.
#[allow(clippy::too_many_arguments)]
fn month_grid(
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    name: &str,
    (y, m): (i32, u32),
    days: &[(NaiveDate, Day)],
    starts: &[crate::plan::Day],
    picked: Option<NaiveDate>,
    today: NaiveDate,
) -> Grid {
    let c = ito_theme::colors();
    let mut out = Grid::default();
    let Some(first) = NaiveDate::from_ymd_opt(y, m, 1) else { return out };
    let lead = first.weekday().num_days_from_sunday() as usize;
    let rows = (lead + days.len()).div_ceil(7);
    let start = first - chrono::Days::new(lead as u64);
    let cw = (ui.available_width().max(7.0 * 60.0) / 7.0).floor();
    let ch = (cw * 0.9).clamp(88.0, 124.0);
    let band = 30.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(cw * 7.0, band + ch * rows as f32), egui::Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    let base = c.bg;
    painter.rect_filled(rect, 6.0, base);
    // The band of weekdays.
    let head = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), band));
    painter.rect_filled(head, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, c.panel);
    let font = egui::TextStyle::Body.resolve(ui.style());
    for (i, w) in GRID.into_iter().enumerate() {
        let x = rect.min.x + cw * (i as f32 + 0.5);
        let colour = if matches!(w, Weekday::Sun | Weekday::Sat) { weekday_colour(&c, w) } else { c.dim };
        painter.text(egui::pos2(x, head.center().y), egui::Align2::CENTER_CENTER, weekday(t, w), font.clone(), colour);
    }
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let small = egui::FontId::monospace(egui::TextStyle::Small.resolve(ui.style()).size);
    for i in 0..rows * 7 {
        let d = start + chrono::Days::new(i as u64);
        let (row, col) = (i / 7, i % 7);
        let cell = egui::Rect::from_min_size(egui::pos2(rect.min.x + cw * col as f32, head.max.y + ch * row as f32), egui::vec2(cw, ch));
        let Some((_, day)) = days.iter().find(|(x, _)| *x == d) else {
            // A day of the month before or after: its date, faint.
            let colour = ito_theme::mix(base, c.dim, 0.45);
            painter.text(cell.min + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, format!("{}", d.day()), mono.clone(), colour);
            continue;
        };
        let id = ui.id().with(("cal_cell", d));
        let r = ui.interact(cell, id, egui::Sense::click());
        // The tint says what the day is; a hover lifts it a little.
        let tint = day_tint(&c, base, d, day);
        let tint = if r.hovered() { ito_theme::mix(tint, c.fg, 0.05) } else { tint };
        painter.rect_filled(cell, 0.0, tint);
        let named = show::day_name(t, day);
        let red = matches!(day, Day::Holiday(_)) || d.weekday() == Weekday::Sun;
        let number = if red { c.err } else { weekday_colour(&c, d.weekday()) };
        let here = picked == Some(d);
        let galley = painter.layout_no_wrap(d.day().to_string(), mono.clone(), number);
        let at = cell.min + egui::vec2(8.0, 6.0);
        let size = galley.size();
        if d == today {
            // Today: the date on a round mark of the today colour.
            let mark = egui::Rect::from_min_size(at - egui::vec2(4.0, 1.0), size + egui::vec2(8.0, 2.0));
            painter.rect_filled(mark, mark.height() / 2.0, c.blue);
            painter.text(at, egui::Align2::LEFT_TOP, d.day().to_string(), mono.clone(), c.on_accent());
        } else {
            painter.galley(at, galley, number);
        }
        // The name (or 選択中) as a tag at the top right, as wide as is left. A name too long for
        // the corner takes a line of its own under the date; 選択中 is left out (the ring says it).
        let tag_words = named.clone().or_else(|| here.then(|| t.tr("calendar.picked").to_owned()));
        let line = 16.0;
        let mut top = cell.min.y + 28.0;
        if let Some(words) = &tag_words {
            let colour = if named.is_some() { day_colour(&c, day) } else { c.run };
            let tag = ito_prefs::Tag::new(words).color(colour);
            let max = (cell.width() - size.x - 26.0).max(20.0);
            let w = ito_prefs::tag_width(ui, tag);
            if w <= max {
                ito_prefs::paint_tag(&painter, ui.ctx(), &c, egui::pos2(cell.max.x - 6.0 - w, cell.min.y + 5.0), tag, max);
            } else if named.is_some() {
                let r = ito_prefs::paint_tag(&painter, ui.ctx(), &c, egui::pos2(cell.min.x + 5.0, top), tag, cell.width() - 10.0);
                top = r.max.y + 3.0;
            }
        }
        // The times that start on the day, one a line, as many as fit; +N for the rest.
        let times = starts.iter().find(|(x, _)| *x == d).map(|(_, s)| s.as_slice()).unwrap_or_default();
        let fit = (((cell.max.y - 4.0 - top) / (line + 2.0)).floor() as usize).max(1);
        for (k, (at, net)) in times.iter().enumerate() {
            let y = top + k as f32 * (line + 2.0);
            let chip = egui::Rect::from_min_size(egui::pos2(cell.min.x + 5.0, y), egui::vec2(cell.width() - 10.0, line));
            if k + 1 == fit && times.len() > fit {
                let more = format!("+{}", times.len() - k);
                painter.text(chip.left_center() + egui::vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, more, small.clone(), c.dim);
                break;
            }
            painter.rect_filled(chip, 3.0, ito_theme::mix(tint, c.run, 0.10));
            painter.rect_stroke(chip, 3.0, egui::Stroke::new(1.0, ito_theme::mix(tint, c.run, 0.35)), egui::StrokeKind::Inside);
            let mut job = egui::text::LayoutJob::default();
            let fmt = |colour| egui::TextFormat { font_id: small.clone(), color: colour, ..Default::default() };
            let hm = at.format("%H:%M").to_string();
            let hm_w = painter.layout_no_wrap(hm.clone(), small.clone(), c.run).size().x;
            job.append(&hm, 0.0, fmt(c.run));
            // The net only when a few letters of it fit; else the time alone, and the pane says the rest.
            if chip.width() - 8.0 - hm_w > 40.0 {
                job.append(net, 6.0, fmt(c.fg));
            }
            job.wrap.max_width = chip.width() - 8.0;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let g = painter.layout_job(job);
            painter.galley(egui::pos2(chip.min.x + 4.0, chip.center().y - g.size().y / 2.0), g, c.fg);
        }
        // The rings: today in its colour, the picked day in the accent inside it.
        if d == today && !here {
            painter.rect_stroke(cell.shrink(1.0), 0.0, egui::Stroke::new(2.0, c.blue), egui::StrokeKind::Inside);
        }
        if here {
            painter.rect_stroke(cell.shrink(1.0), 0.0, egui::Stroke::new(2.0, c.run), egui::StrokeKind::Inside);
        }
        if r.has_focus() {
            painter.rect_stroke(cell.shrink(4.0), 2.0, egui::Stroke::new(1.0, c.fg), egui::StrokeKind::Inside);
        }
        let r = match &named {
            Some(n) => r.on_hover_text(n),
            None => r,
        };
        let kind = named.unwrap_or_else(|| t.tr(if day.is_business() { "calendar.business" } else { "calendar.off" }).to_owned());
        ito_a11y::say(&r, Role::Button, &format!("{} {}（{kind}）", d, weekday(t, d.weekday())));
        day_press(&r, t, name, d, day, &mut out);
    }
    // The lines between the cells, and round the whole.
    let line = egui::Stroke::new(1.0, c.border);
    for k in 1..7 {
        let x = rect.min.x + cw * k as f32;
        painter.line_segment([egui::pos2(x, head.max.y), egui::pos2(x, rect.max.y)], line);
    }
    for k in 0..rows {
        let y = head.max.y + ch * k as f32;
        painter.line_segment([egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)], line);
    }
    painter.rect_stroke(rect, 6.0, line, egui::StrokeKind::Inside);
    out
}

/// The tint of a day in the month or the week: what the day is, on `base`.
fn day_tint(c: &ito_theme::Colors, base: egui::Color32, d: NaiveDate, day: &Day) -> egui::Color32 {
    match day {
        Day::Holiday(_) => ito_theme::mix(base, c.err, 0.12),
        Day::Closed(_) => ito_theme::mix(base, c.magenta, 0.12),
        Day::Open(_) => ito_theme::mix(base, c.done, 0.10),
        Day::WeeklyOff => ito_theme::mix(base, weekday_colour(c, d.weekday()), 0.06),
        Day::Business => base,
    }
}

/// A day in the month or the week, pressed: a click picks it; a double click or the menu moves it
/// between open and closed; the menu also names it.
fn day_press(r: &egui::Response, t: &ito_i18n::Table, name: &str, d: NaiveDate, day: &Day, out: &mut Grid) {
    if r.double_clicked() {
        out.toggle = Some((d, day.clone()));
    } else if r.clicked() {
        out.pick = Some(d);
    }
    r.context_menu(|ui| {
        if ui.button(t.tr(toggle_key(day))).clicked() {
            out.toggle = Some((d, day.clone()));
            ui.close();
        }
        if ui.button(t.tr("calendar.name_day")).clicked() {
            let text = match day {
                Day::Open(n) | Day::Closed(n) => n.clone(),
                _ => String::new(),
            };
            out.naming = Some(Inline::Name { cal: name.to_owned(), day: d, text, fresh: true });
            ui.close();
        }
    });
}

/// The week as a timetable, Sunday first, as wide as the room: a band with each day's weekday,
/// date and name, then a row for each time something starts in the week, with the nets that start
/// then as chips in their day's column. A column is tinted by what its day is and pressed as a day
/// of the month is; a chip picks its day, and a double click on it opens the net.
#[allow(clippy::too_many_arguments)]
fn week_grid(
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    name: &str,
    week: &[(NaiveDate, Day)],
    starts: &[(NaiveDate, Vec<crate::plan::Start>)],
    picked: Option<NaiveDate>,
    today: NaiveDate,
) -> Grid {
    let c = ito_theme::colors();
    let mut out = Grid::default();
    let on = |d: NaiveDate| starts.iter().find(|(x, _)| *x == d).map(|(_, l)| l.as_slice()).unwrap_or_default();
    // A row a time, as tall as the day with the most nets at it.
    let mut times: Vec<chrono::NaiveTime> = starts.iter().flat_map(|(_, l)| l.iter().map(|(at, ..)| at.time())).collect();
    times.sort();
    times.dedup();
    let (chip, gap, pad) = (22.0, 3.0, 5.0);
    let heights: Vec<f32> = times
        .iter()
        .map(|tm| {
            let most = week.iter().map(|(d, _)| on(*d).iter().filter(|(at, ..)| at.time() == *tm).count()).max().unwrap_or(0).max(1);
            pad * 2.0 + most as f32 * chip + (most - 1) as f32 * gap
        })
        .collect();
    // The week takes the width there is (as the month does); a name too long for its chip ends in "…".
    let tw = 52.0;
    let cw = ((ui.available_width() - tw).max(7.0 * 52.0) / 7.0).floor();
    let band = 64.0;
    let body = if times.is_empty() { 72.0 } else { heights.iter().sum() };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(tw + cw * 7.0, band + body), egui::Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    let base = c.bg;
    painter.rect_filled(rect, 6.0, base);
    let head = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), band));
    painter.rect_filled(head, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, c.panel);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let column =
        |k: usize| egui::Rect::from_min_size(egui::pos2(rect.min.x + tw + cw * k as f32, rect.min.y), egui::vec2(cw, rect.height()));
    let mut rings = Vec::new();
    // The days: what a press on the column does, its tint, and its head.
    for (k, (d, day)) in week.iter().enumerate() {
        let (d, col) = (*d, column(k));
        let r = ui.interact(col, ui.id().with(("cal_week_day", d)), egui::Sense::click());
        let tint = day_tint(&c, base, d, day);
        let tint = if r.hovered() { ito_theme::mix(tint, c.fg, 0.05) } else { tint };
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(col.min.x, head.max.y), col.max), 0.0, tint);
        let w = d.weekday();
        let red = matches!(day, Day::Holiday(_)) || w == Weekday::Sun;
        let ink = if red { c.err } else { weekday_colour(&c, w) };
        let x = col.center().x;
        painter.text(
            egui::pos2(x, head.min.y + 6.0),
            egui::Align2::CENTER_TOP,
            weekday(t, w),
            small.clone(),
            if red { c.err } else { c.dim },
        );
        let date = format!("{}/{}", d.month(), d.day());
        let at = egui::pos2(x, head.min.y + 30.0);
        if d == today {
            // Today: the date on a round mark of the today colour, as in the month.
            let g = painter.layout_no_wrap(date.clone(), mono.clone(), c.on_accent());
            let mark = egui::Rect::from_center_size(at, g.size() + egui::vec2(10.0, 2.0));
            painter.rect_filled(mark, mark.height() / 2.0, c.blue);
            painter.galley(at - g.size() / 2.0, g, c.on_accent());
        } else {
            painter.text(at, egui::Align2::CENTER_CENTER, &date, mono.clone(), ink);
        }
        let named = show::day_name(t, day);
        if let Some(n) = &named {
            let tag = ito_prefs::Tag::new(n).color(day_colour(&c, day));
            let w = ito_prefs::tag_width(ui, tag).min(cw - 8.0);
            ito_prefs::paint_tag(&painter, ui.ctx(), &c, egui::pos2(x - w / 2.0, head.min.y + 42.0), tag, cw - 8.0);
        }
        if picked == Some(d) {
            rings.push((col, c.run));
        } else if d == today {
            rings.push((col, c.blue));
        }
        if r.has_focus() {
            rings.push((col.shrink(3.0), c.fg));
        }
        let r = match &named {
            Some(n) => r.on_hover_text(n),
            None => r,
        };
        let kind = named.unwrap_or_else(|| t.tr(if day.is_business() { "calendar.business" } else { "calendar.off" }).to_owned());
        ito_a11y::say(&r, Role::Button, &format!("{} {}（{kind}）", d, weekday(t, w)));
        day_press(&r, t, name, d, day, &mut out);
    }
    // The rows: the time at the left, the nets that start then in their day's column, on top.
    let mut y = head.max.y;
    for (tm, h) in times.iter().zip(&heights) {
        painter.text(
            egui::pos2(rect.min.x + 10.0, y + pad + chip / 2.0),
            egui::Align2::LEFT_CENTER,
            tm.format("%H:%M").to_string(),
            mono.clone(),
            c.dim,
        );
        for (k, (d, day)) in week.iter().enumerate() {
            let col = column(k);
            let tint = day_tint(&c, base, *d, day);
            for (n, (at, net, id)) in on(*d).iter().filter(|(at, ..)| at.time() == *tm).enumerate() {
                let cr =
                    egui::Rect::from_min_size(egui::pos2(col.min.x + 4.0, y + pad + n as f32 * (chip + gap)), egui::vec2(cw - 8.0, chip));
                let r = ui.interact(cr, ui.id().with(("cal_week_net", *d, *at, id)), egui::Sense::click());
                let hot = r.hovered() || r.has_focus();
                painter.rect_filled(cr, 4.0, ito_theme::mix(tint, c.run, if hot { 0.22 } else { 0.12 }));
                let edge = if hot { c.run } else { ito_theme::mix(tint, c.run, 0.4) };
                painter.rect_stroke(cr, 4.0, egui::Stroke::new(1.0, edge), egui::StrokeKind::Inside);
                let mut job = egui::text::LayoutJob::single_section(
                    net.clone(),
                    egui::TextFormat { font_id: font.clone(), color: c.fg, ..Default::default() },
                );
                job.wrap.max_width = cr.width() - 12.0;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let g = painter.layout_job(job);
                painter.galley(egui::pos2(cr.min.x + 6.0, cr.center().y - g.size().y / 2.0), g, c.fg);
                let said = format!("{} {} {net}", d, at.format("%H:%M"));
                if net_link(ui, t, &r, &said) {
                    out.open = Some(id.clone());
                } else if r.clicked() {
                    out.pick = Some(*d);
                }
            }
        }
        y += h;
    }
    if times.is_empty() {
        let mid = egui::pos2(rect.min.x + tw + cw * 3.5, head.max.y + body / 2.0);
        painter.text(mid, egui::Align2::CENTER_CENTER, t.tr("calendar.week_none"), font.clone(), c.dim);
    }
    // The lines between the columns and the rows, round the whole, then the rings.
    let line = egui::Stroke::new(1.0, c.border);
    for k in 0..7 {
        let x = rect.min.x + tw + cw * k as f32;
        painter.line_segment([egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)], line);
    }
    let mut y = head.max.y;
    painter.line_segment([egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)], line);
    for h in &heights {
        y += h;
        painter.line_segment([egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)], line);
    }
    painter.rect_stroke(rect, 6.0, line, egui::StrokeKind::Inside);
    for (r, colour) in rings {
        painter.rect_stroke(r.shrink(1.0), 0.0, egui::Stroke::new(2.0, colour), egui::StrokeKind::Inside);
    }
    out
}

impl Browser {
    /// この日に動くもの beside the month: the date with what the day is, the button that moves it
    /// between open and closed, and the approved starts of that day on every calendar, a card each
    /// (a double click opens its net). What was pressed.
    fn day_box(&self, ui: &mut egui::Ui, t: &ito_i18n::Table, cal: &Calendar, d: NaiveDate, day: &Day) -> Side {
        let c = ito_theme::colors();
        let starts =
            self.snap.as_ref().and_then(|s| crate::plan::days(&s.defs, &s.calendars, d, 1).pop()).map(|(_, l)| l).unwrap_or_default();
        let mut side = Side::default();
        let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
        card(ui, &c, |ui| {
            let kind = day_kind(t, cal, d, day);
            ui.horizontal(|ui| {
                let date = format!("{} ({})", d.format("%Y/%m/%d"), weekday(t, d.weekday()));
                ui.label(egui::RichText::new(date).monospace().strong().size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ito_prefs::tag(ui, look, ito_prefs::Tag::new(&kind).color(day_colour(&c, day)));
                });
            });
            ui.add_space(8.0);
            let button = ito_prefs::IconButton::new(toggle_mark(day), t.tr(toggle_key(day))).tone(ito_prefs::Tone::Primary);
            if ui.add_enabled_ui(!self.blocked(), |ui| button.show(ui)).inner.clicked() {
                side.toggle = Some((d, day.clone()));
            }
            ui.add_space(10.0);
            let head = t.trf("calendar.day_head", &[&format!("{}/{}", d.month(), d.day()), &weekday(t, d.weekday()), &kind]);
            let r = ui.add(egui::Label::new(egui::RichText::new(&head).strong()).wrap());
            ito_a11y::say(&r, Role::Heading, &head);
            ui.add(egui::Label::new(egui::RichText::new(t.tr("calendar.day_note")).color(c.dim).small()).wrap());
            ui.add_space(6.0);
            if starts.is_empty() {
                ui.label(egui::RichText::new(t.tr("calendar.day_none")).color(c.dim));
            }
            for (at, net, id) in &starts {
                let hm = at.format("%H:%M").to_string();
                let r = egui::Frame::new()
                    .fill(c.raised())
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(egui::RichText::new(&hm).monospace().strong().color(c.run)).selectable(false));
                        ui.add(egui::Label::new(net.as_str()).wrap().selectable(false));
                    })
                    .response
                    .interact(egui::Sense::click());
                if r.hovered() || r.has_focus() {
                    ui.painter().rect_stroke(r.rect, 6.0, egui::Stroke::new(1.0, c.run), egui::StrokeKind::Inside);
                }
                if net_link(ui, t, &r, &format!("{hm} {net}")) {
                    side.open = Some(id.clone());
                }
                ui.add_space(4.0);
            }
        });
        side
    }

    /// 新しいカレンダー: opens the box, with Japan's holidays and Saturday and Sunday off.
    /// The field under the month: a day's name or a holiday table's path, with its button.
    /// Enter does it, Esc or やめる closes it; it goes when another calendar is shown.
    fn inline_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, name: &str, cal: &Calendar) {
        if self.cal_inline.as_ref().is_some_and(|i| i.cal() != name) {
            self.cal_inline = None;
        }
        let Some(inline) = &mut self.cal_inline else { return };
        let mut pressed = None;
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let (label, hint, text, fresh) = match inline {
                Inline::Name { day, text, fresh, .. } => {
                    (t.trf("calendar.name_of", &[&day.to_string()]), t.tr("calendar.name_hint"), text, fresh)
                }
                Inline::Table { path, fresh, .. } => (t.tr("calendar.table_path").to_owned(), t.tr("port.path_hint"), path, fresh),
            };
            ui.label(&label);
            let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(280.0));
            ito_a11y::say(&r, Role::Label, &label);
            if std::mem::take(fresh) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                pressed = Some(0);
            }
            // The field gives up the keyboard on Esc itself, in the same frame.
            if (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                pressed = Some(1);
            }
            for (i, key) in ["calendar.inline_go", "make.cancel"].into_iter().enumerate() {
                let r = ui.button(t.tr(key));
                ito_a11y::say(&r, Role::Button, t.tr(key));
                if r.clicked() {
                    pressed = Some(i);
                }
            }
        });
        match pressed {
            Some(0) => {
                let Some(inline) = self.cal_inline.take() else { return };
                self.cal_result = Some(match inline {
                    Inline::Name { day, text, .. } => {
                        let named = named(cal, day, &cal.day(day), text.trim());
                        edit_file(t, name, |text| rewrite(text, &named))
                    }
                    Inline::Table { path, .. } => import_table(t, &path),
                });
            }
            Some(_) => self.cal_inline = None,
            None => {}
        }
    }

    pub fn new_cal_box(&mut self) {
        self.new_cal_box_to(false);
    }

    /// The same box from the editor's calendar list: the calendar made becomes the open net's,
    /// and the editor stays.
    pub fn new_cal_box_for_net(&mut self) {
        self.new_cal_box_to(true);
    }

    fn new_cal_box_to(&mut self, for_net: bool) {
        if !self.box_open() && self.snap.is_some() {
            self.new_cal = Some(NewCal {
                from: None,
                for_net,
                name: String::new(),
                holidays: true,
                off: [false, false, false, false, false, true, true],
                error: None,
                fresh: true,
            });
            self.focus = 0;
        }
    }

    /// 名前を変える: the 新しいカレンダー box with the name only, filled in.
    fn rename_cal_box(&mut self, name: &str) {
        if !self.box_open() && self.snap.is_some() {
            self.new_cal = Some(NewCal {
                from: Some(name.to_owned()),
                for_net: false,
                name: name.to_owned(),
                holidays: false,
                off: [false; 7],
                error: None,
                fresh: true,
            });
            self.focus = 0;
        }
    }

    fn delete_cal_box(&mut self, name: &str) {
        if !self.box_open() && self.snap.is_some() {
            self.cal_deleting = Some(name.to_owned());
            // The safe button first: Enter on a fresh box deletes nothing.
            self.focus = 1;
        }
    }

    /// The box that asks before a calendar is deleted: the files that still name it, and where
    /// its content is kept.
    pub(super) fn confirm_cal_delete(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(name) = self.cal_deleting.clone() else { return };
        let users = self.snap.as_ref().map(|s| users(&s.defs, &name)).unwrap_or_default();
        let rows = [(t.tr("calendar.heading").to_owned(), format!("calendars/{name}.toml"))];
        let mut body = vec![t.trf("newcal.delete_ask", &[&name])];
        if !users.is_empty() {
            body.push(t.trf("calendar.users_left", &[&name, &users.join(", ")]));
        }
        let buttons = [t.tr("delete.yes").to_owned(), t.tr("make.cancel").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("newcal.delete_title"),
            rows: &rows,
            body: &body,
            buttons: &buttons,
            hint: t.tr("confirm.hint"),
            focus: Some(self.focus),
            primary: Some(0),
            danger: true,
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &Self::style()).or(key) {
            Some(0) => {
                self.cal_deleting = None;
                self.cal_sel = Some(name.clone());
                self.cal_result = Some(delete_file(t, &name));
            }
            Some(_) => self.cal_deleting = None,
            None => {}
        }
    }

    pub(super) fn confirm_new_cal(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let users = match (&self.new_cal, &self.snap) {
            (Some(NewCal { from: Some(from), .. }), Some(s)) => users(&s.defs, from),
            _ => Vec::new(),
        };
        let Some(m) = &mut self.new_cal else { return };
        let c = ito_theme::colors();
        let mut pressed = None;
        let (title, go) = match &m.from {
            None => (t.tr("newcal.title"), t.tr("newcal.go")),
            Some(_) => (t.tr("newcal.rename_title"), t.tr("newcal.rename_go")),
        };
        egui::Window::new(title)
            .id(egui::Id::new("new-cal"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                let lead = match &m.from {
                    None => t.tr("newcal.lead").to_owned(),
                    Some(from) => t.trf("newcal.rename_lead", &[from]),
                };
                ui.add(egui::Label::new(lead).wrap());
                egui::Grid::new("new-cal-fields").num_columns(2).show(ui, |ui| {
                    ui.label(t.tr("newcal.name"));
                    let r = ui.add(egui::TextEdit::singleline(&mut m.name).hint_text(t.tr("newcal.name_hint")).desired_width(240.0));
                    ito_a11y::say(&r, Role::Label, t.tr("newcal.name"));
                    if m.fresh {
                        r.request_focus();
                        m.fresh = false;
                    }
                    ui.end_row();
                    if m.from.is_some() {
                        return;
                    }
                    ui.label("");
                    // egui names a checkbox for the screen reader itself.
                    ui.checkbox(&mut m.holidays, t.tr("newcal.holidays"));
                    ui.end_row();
                    ui.label(t.tr("newcal.off"));
                    ui.horizontal(|ui| {
                        for w in GRID {
                            ui.checkbox(&mut m.off[w.num_days_from_monday() as usize], weekday(t, w));
                        }
                    });
                    ui.end_row();
                });
                if let (Some(from), false) = (&m.from, users.is_empty()) {
                    let line = t.trf("calendar.users_left", &[from, &users.join(", ")]);
                    ui.add(egui::Label::new(egui::RichText::new(line).color(c.wait_text())).wrap());
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
            Some(0) => self.new_cal_go(t),
            Some(_) => self.new_cal = None,
            None => {}
        }
    }

    /// The box's 作る: writes the file (or renames it) and shows its month. Nothing is overwritten.
    fn new_cal_go(&mut self, t: &ito_i18n::Table) {
        let Some(m) = &mut self.new_cal else { return };
        let have = self.snap.iter().flat_map(|s| s.calendars.keys().map(String::as_str));
        // A rename to the name it has, or only another case of it, is not taken.
        let from = m.from.clone();
        let have = have.filter(|h| from.as_deref().is_none_or(|f| !f.eq_ignore_ascii_case(h)));
        let dir = ito_common::app_dir("koyomi").map(|d| d.join("calendars"));
        let made = match new_name(&m.name, have) {
            Err(Why::Empty) => Err(t.tr("newcal.empty").to_owned()),
            Err(Why::Bad) => Err(t.tr("make.bad").to_owned()),
            Err(Why::Exists(n)) => Err(t.trf("make.exists", &[n.as_str()])),
            Ok(name) => {
                let file = format!("calendars/{name}.toml");
                let same = |d: &std::path::Path| from.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&name)) && d.exists();
                match (&dir, &from) {
                    (None, _) => Err(t.trf("calendar.write_failed", &[&file, "app_dir"])),
                    (Some(_), Some(f)) if *f == name => Err(t.trf("make.exists", &[&name])),
                    // Not read by the engine yet, but there: made by hand a moment ago.
                    (Some(d), _) if d.join(format!("{name}.toml")).exists() && !same(d) => Err(t.trf("make.exists", &[&name])),
                    (Some(d), None) => ito_common::write_atomic(&d.join(format!("{name}.toml")), new_text(m.holidays, &days_off(m.off)))
                        .map(|()| (name, t.trf("calendar.written", &[&file])))
                        .map_err(|e| t.trf("calendar.write_failed", &[&file, &e])),
                    (Some(d), Some(f)) => std::fs::rename(d.join(format!("{f}.toml")), d.join(format!("{name}.toml")))
                        .map(|()| (name.clone(), t.trf("calendar.renamed", &[f, &name])))
                        .map_err(|e| t.trf("calendar.rename_failed", &[&format!("calendars/{f}.toml"), &e.to_string()])),
                }
            }
        };
        match made {
            Err(e) => m.error = Some(e),
            Ok((name, said)) if m.for_net && self.editor.is_some() => {
                self.new_cal = None;
                if let Some(e) = &mut self.editor {
                    e.set_calendar(&name);
                    e.result = Some(Ok(said));
                }
            }
            Ok((name, said)) => {
                self.new_cal = None;
                self.cal_sel = Some(name);
                self.cal_month = None;
                self.cal_result = Some(Ok(said));
                self.run_sel = None;
                self.job_sel = None;
                self.result = None;
                self.hist = None;
                self.gantt = false;
            }
        }
    }
}

/// The name a new calendar takes, when it is a file name and no calendar in `have` has it
/// (Windows does not tell `A` from `a`).
fn new_name<'a>(name: &str, mut have: impl Iterator<Item = &'a str>) -> Result<String, Why> {
    let n = name.trim();
    if n.is_empty() {
        return Err(Why::Empty);
    }
    if bad_file_name(n) {
        return Err(Why::Bad);
    }
    let low = n.to_lowercase();
    if have.any(|h| h.to_lowercase() == low) {
        return Err(Why::Exists(n.to_owned()));
    }
    Ok(n.to_owned())
}

/// A new calendar's file.
pub fn new_text(holidays: bool, off: &[Weekday]) -> String {
    format!("format = {}\nholidays = {}\nweekly_off = {}\n", koyomi_core::calendar::FORMAT, holidays_text(holidays), off_text(off))
}

/// `holidays` and `weekly_off` of `text` set: what the boxes above the month write.
pub fn set_rules(text: &str, holidays: bool, off: &[Weekday]) -> Result<String, String> {
    let text = ito_common::set_key(text, None, "holidays", &holidays_text(holidays))?;
    ito_common::set_key(&text, None, "weekly_off", &off_text(off))
}

fn holidays_text(on: bool) -> String {
    format!("\"{}\"", if on { "japan" } else { "none" })
}

fn off_text(off: &[Weekday]) -> String {
    let days: Vec<String> = off.iter().map(|w| format!("\"{}\"", w.to_string().to_lowercase())).collect();
    format!("[{}]", days.join(", "))
}

/// The ticked days of a row of seven, Monday first.
fn days_off(on: [bool; 7]) -> Vec<Weekday> {
    WEEK.iter().zip(on).filter(|(_, on)| *on).map(|(w, _)| *w).collect()
}

/// The files that make a net count by `name`: the net's own `calendar`, or its group's
/// `_group.toml` when the net names none. What a rename or a delete leaves pointing nowhere.
pub fn users(defs: &[Def], name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in defs.iter().filter(|d| d.calendar.as_deref() == Some(name)) {
        let own = d.net.as_ref().is_some_and(|n| n.calendar.as_deref() == Some(name));
        let file = if own { format!("jobnets/{}", d.file) } else { format!("jobnets/{}/_group.toml", d.group) };
        if !out.contains(&file) {
            out.push(file);
        }
    }
    out
}

/// A day's colour in the table, in the list under it and in the legend: the company's own days
/// off stand apart from the holidays.
fn day_colour(c: &ito_theme::Colors, day: &Day) -> egui::Color32 {
    match day {
        Day::Business => c.fg,
        Day::Open(_) => c.done,
        Day::Holiday(_) => c.err,
        Day::Closed(_) => c.magenta,
        Day::WeeklyOff => c.dim,
    }
}

/// What a day is, after its date in the day box: 営業日 5 (the fifth of the month), or its name,
/// or 休み.
fn day_kind(t: &ito_i18n::Table, cal: &Calendar, d: NaiveDate, day: &Day) -> String {
    if day.is_business() {
        let n = show::month_days(cal, d.year(), d.month()).iter().filter(|(x, day)| *x <= d && day.is_business()).count();
        return t.trf("calendar.day_nth", &[&n.to_string()]);
    }
    show::day_name(t, day).unwrap_or_else(|| t.tr("calendar.off").to_owned())
}

/// The words of what [`toggled`] does to a day.
fn toggle_key(day: &Day) -> &'static str {
    match day {
        Day::Open(_) | Day::Closed(_) => "calendar.toggle_back",
        Day::Holiday(_) | Day::WeeklyOff => "calendar.toggle_open",
        Day::Business => "calendar.toggle_closed",
    }
}

/// The mark on the button that moves a day: back, off, or on.
fn toggle_mark(day: &Day) -> ito_prefs::Mark {
    match day {
        Day::Open(_) | Day::Closed(_) => ito_prefs::Mark::Undo,
        Day::Holiday(_) | Day::WeeklyOff => ito_prefs::Mark::CalendarCheck,
        Day::Business => ito_prefs::Mark::CalendarOff,
    }
}

fn weekday(t: &ito_i18n::Table, w: Weekday) -> String {
    t.tr(&format!("weekday.{}", w.to_string().to_lowercase())).to_owned()
}

/// What a click on `d` does to the file: an `open` or `closed` day is taken
/// out; a weekend or holiday goes into `open`, a plain weekday into `closed`.
pub fn toggled(cal: &Calendar, d: NaiveDate, day: &Day) -> Calendar {
    let mut out = cal.clone();
    match day {
        Day::Open(_) => {
            out.open.remove(&d);
        }
        Day::Closed(_) => {
            out.closed.remove(&d);
        }
        Day::Holiday(_) | Day::WeeklyOff => {
            out.open.insert(d, String::new());
        }
        Day::Business => {
            out.closed.insert(d, String::new());
        }
    }
    out
}

/// `closed` and `open` of `text` set to `cal`'s (a key goes when its list is empty).
pub fn rewrite(text: &str, cal: &Calendar) -> Result<String, String> {
    let mut text = text.to_owned();
    for (key, days) in [("closed", &cal.closed), ("open", &cal.open)] {
        text = if days.is_empty() {
            ito_common::remove_key(&text, None, key)?
        } else {
            ito_common::set_key(&text, None, key, &Calendar::list_text(days))?
        };
    }
    Ok(text)
}

/// `cal` with `d` named `name` (an empty one takes the name off): an `open` day stays open, and
/// any other day becomes `closed` (a weekend or holiday stays off, now under this name).
pub fn named(cal: &Calendar, d: NaiveDate, day: &Day, name: &str) -> Calendar {
    let mut out = cal.clone();
    match day {
        Day::Open(_) => out.open.insert(d, name.to_owned()),
        _ => out.closed.insert(d, name.to_owned()),
    };
    out
}

fn toggle_day(t: &ito_i18n::Table, name: &str, cal: &Calendar, d: NaiveDate, day: &Day) -> Result<String, String> {
    edit_file(t, name, |text| rewrite(text, &toggled(cal, d, day)))
}

/// `calendars/<name>.toml` read, changed by `change`, and written back; what to say under the month.
fn edit_file(t: &ito_i18n::Table, name: &str, change: impl FnOnce(&str) -> Result<String, String>) -> Result<String, String> {
    let file = format!("calendars/{name}.toml");
    let path = ito_common::app_dir("koyomi").ok_or_else(|| t.trf("calendar.write_failed", &[&file, "app_dir"]))?.join(&file);
    let write = || -> Result<(), String> {
        let text = ito_common::read_or_empty(&path)?;
        ito_common::write_atomic(&path, change(&text)?)
    };
    match write() {
        Ok(()) => Ok(t.trf("calendar.written", &[&file])),
        Err(e) => Err(t.trf("calendar.write_failed", &[&file, &e])),
    }
}

/// `calendars/<name>.toml` copied to `versions/_calendars/<name>/<YYYYMMDD-HHMMSS>.toml` in the
/// state folder (as an import keeps one it writes over), then deleted.
fn delete_file(t: &ito_i18n::Table, name: &str) -> Result<String, String> {
    let file = format!("calendars/{name}.toml");
    let failed = |e: &str| t.trf("calendar.delete_failed", &[&file, e]);
    let path = ito_common::app_dir("koyomi").ok_or_else(|| failed("app_dir"))?.join(&file);
    let dir = ito_common::state_dir("koyomi").ok_or_else(|| failed("state_dir"))?.join("versions").join("_calendars").join(name);
    let kept = keep_and_delete(&path, &dir, &chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()).map_err(|e| failed(&e))?;
    Ok(t.trf("calendar.deleted", &[&file, &kept.display().to_string()]))
}

/// `path` copied into `dir` as `<stamp>.toml` (or `<stamp>-2.toml` … when taken), then deleted;
/// where the copy went. Nothing is deleted when the copy fails.
fn keep_and_delete(path: &std::path::Path, dir: &std::path::Path, stamp: &str) -> Result<std::path::PathBuf, String> {
    let kept = keep(path, dir, stamp, "toml")?;
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    Ok(kept)
}

/// `path` copied into `dir` as `<stamp>.<ext>` (or `<stamp>-2.<ext>` … when taken); where it went.
fn keep(path: &std::path::Path, dir: &std::path::Path, stamp: &str, ext: &str) -> Result<std::path::PathBuf, String> {
    let text = std::fs::read(path).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut kept = dir.join(format!("{stamp}.{ext}"));
    let mut n = 1;
    while kept.exists() {
        n += 1;
        kept = dir.join(format!("{stamp}-{n}.{ext}"));
    }
    ito_common::write_atomic(&kept, text)?;
    Ok(kept)
}

/// 祝日の表を取り込む: the file at `path` (quotes from コピー as path are taken off) becomes
/// `calendars/syukujitsu.csv` when it reads; the one there is kept under `versions/_calendars/`.
fn import_table(t: &ito_i18n::Table, path: &str) -> Result<String, String> {
    let path = path.trim().trim_matches('"');
    let failed = |e: &str| t.trf("calendar.table_failed", &[path, e]);
    if path.is_empty() {
        return Err(t.tr("calendar.table_empty").to_owned());
    }
    let dest = ito_common::app_dir("koyomi").ok_or_else(|| failed("app_dir"))?.join("calendars").join(TABLE_FILE);
    let dir = ito_common::state_dir("koyomi").ok_or_else(|| failed("state_dir"))?.join("versions").join("_calendars").join("syukujitsu");
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let (from, to) = bring_table(std::path::Path::new(path), &dest, &dir, &stamp).map_err(|e| failed(&e))?;
    Ok(t.trf("calendar.table_done", &[&format!("calendars/{TABLE_FILE}"), &from.to_string(), &to.to_string()]))
}

/// The holiday table at `src` read, and its bytes written to `dest` as they are (the file there
/// kept in `dir` first); the years it covers. Nothing is written when it does not read.
fn bring_table(src: &std::path::Path, dest: &std::path::Path, dir: &std::path::Path, stamp: &str) -> Result<(i32, i32), String> {
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    let table = Table::parse(&Table::decode(&bytes))?;
    if dest.exists() {
        keep(dest, dir, stamp, "csv")?;
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    ito_common::write_atomic(dest, bytes)?;
    Ok(table.years)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAL: &str =
        "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [{ date = 2026-11-20, name = \"創立記念日\" }]\n";

    #[test]
    fn a_new_calendar_reads_back_and_a_bad_or_taken_name_is_refused() {
        let text = new_text(true, &[Weekday::Sat, Weekday::Sun]);
        assert_eq!(text, "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\n");
        let cal = Calendar::parse("会社", &text).unwrap();
        assert!(cal.holidays);
        assert_eq!(cal.weekly_off, vec![Weekday::Sat, Weekday::Sun]);
        let cal = Calendar::parse("工場", &new_text(false, &[])).unwrap();
        assert!(!cal.holidays && cal.weekly_off.is_empty());
        let have = || ["会社", "Factory"].into_iter();
        assert_eq!(new_name("  工場 ", have()), Ok("工場".to_owned()));
        assert_eq!(new_name(" ", have()), Err(Why::Empty));
        assert_eq!(new_name("a/b", have()), Err(Why::Bad));
        assert_eq!(new_name("会社.", have()), Err(Why::Bad));
        assert_eq!(new_name("会社", have()), Err(Why::Exists("会社".to_owned())));
        assert_eq!(new_name("factory", have()), Err(Why::Exists("factory".to_owned())));
    }

    #[test]
    fn the_week_rules_are_set_in_place_and_the_rest_stays() {
        let text = set_rules(CAL, false, &[Weekday::Sun]).unwrap();
        assert_eq!(
            text,
            "format = 1
holidays = \"none\"
weekly_off = [\"sun\"]
closed = [{ date = 2026-11-20, name = \"創立記念日\" }]
"
        );
        let cal = Calendar::parse("会社", &text).unwrap();
        assert!(!cal.holidays);
        assert_eq!(cal.weekly_off, vec![Weekday::Sun]);
        assert_eq!(cal.closed.len(), 1, "the named closed day stays");
        let back = set_rules(&text, true, &[Weekday::Sat, Weekday::Sun]).unwrap();
        assert_eq!(back, CAL);
        assert_eq!(set_rules(&text, true, &[]).unwrap().lines().nth(2), Some("weekly_off = []"));
        assert_eq!(days_off([true, false, false, false, false, false, true]), vec![Weekday::Mon, Weekday::Sun]);
    }

    #[test]
    fn a_deleted_calendar_is_kept_first_under_a_stamp_that_is_free() {
        let dir = std::env::temp_dir().join(format!("koyomi-caldel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let keep = dir.join("versions/_calendars/会社");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("20261031-120000.toml"), "old").unwrap();
        let path = dir.join("会社.toml");
        std::fs::write(&path, CAL).unwrap();
        let kept = keep_and_delete(&path, &keep, "20261031-120000").unwrap();
        assert_eq!(kept, keep.join("20261031-120000-2.toml"));
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), CAL);
        assert!(!path.exists());
        assert!(keep_and_delete(&path, &keep, "20261031-120001").is_err(), "nothing there: nothing kept");
        assert!(!keep.join("20261031-120001.toml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_click_moves_a_day_between_open_and_closed_and_the_file_follows() {
        let cal = Calendar::parse("会社", CAL).unwrap();
        let sat = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        let fri = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let thu = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let holiday = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
        // A weekend becomes open, a holiday too, a weekday closed, and the named closed day is let go.
        let c1 = toggled(&cal, sat, &cal.day(sat));
        assert!(c1.is_business(sat));
        let c2 = toggled(&c1, holiday, &c1.day(holiday));
        assert!(c2.is_business(holiday));
        let c3 = toggled(&c2, thu, &c2.day(thu));
        assert!(!c3.is_business(thu));
        let c4 = toggled(&c3, fri, &c3.day(fri));
        assert!(c4.is_business(fri));
        let text = rewrite(CAL, &c4).unwrap();
        assert_eq!(
            text,
            "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [2026-11-19]\nopen = [2026-11-21, 2026-11-23]\n"
        );
        assert_eq!(Calendar::parse("会社", &text).unwrap(), c4, "what was written reads back as what was shown");
        // Back again: the lists empty, and the keys go with them.
        let c5 = toggled(&c4, sat, &c4.day(sat));
        let c6 = toggled(&c5, holiday, &c5.day(holiday));
        let c7 = toggled(&c6, thu, &c6.day(thu));
        assert_eq!(rewrite(&text, &c7).unwrap(), "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\n");
    }

    #[test]
    fn a_day_takes_a_name_and_stays_on_its_side() {
        let cal = Calendar::parse("会社", CAL).unwrap();
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let (thu, sat, holiday, founding) = (d("2026-11-19"), d("2026-11-21"), d("2026-11-23"), d("2026-11-20"));
        let c1 = named(&cal, thu, &cal.day(thu), "棚卸し");
        assert_eq!(c1.day(thu), Day::Closed("棚卸し".to_owned()));
        let c2 = named(&c1, sat, &c1.day(sat), "社員旅行");
        assert_eq!(c2.day(sat), Day::Closed("社員旅行".to_owned()), "a weekend stays off");
        let c3 = named(&c2, holiday, &c2.day(holiday), "");
        assert!(!c3.day(holiday).is_business(), "a holiday stays off");
        let c4 = named(&c3, founding, &c3.day(founding), "");
        assert_eq!(c4.day(founding), Day::Closed(String::new()), "an empty name takes it off, the day stays closed");
        let open = toggled(&c4, sat, &c4.day(sat));
        let open = toggled(&open, sat, &open.day(sat));
        assert_eq!(open.day(sat), Day::Open(String::new()));
        let c5 = named(&open, sat, &open.day(sat), "決算");
        assert_eq!(c5.day(sat), Day::Open("決算".to_owned()), "an open day stays open");
        let text = rewrite(CAL, &c5).unwrap();
        let back = Calendar::parse("会社", &text).unwrap();
        assert_eq!(back.closed, c5.closed);
        assert_eq!(back.open, c5.open);
    }

    #[test]
    fn a_holiday_table_is_written_only_when_it_reads_and_the_old_one_is_kept() {
        let dir = std::env::temp_dir().join(format!("koyomi-caltable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (dest, keep) = (dir.join("calendars").join(TABLE_FILE), dir.join("versions/_calendars/syukujitsu"));
        let bad = dir.join("bad.csv");
        std::fs::write(&bad, "国民の祝日・休日月日,国民の祝日・休日名称\n2027/1/1,元日\nあした,何か\n").unwrap();
        let e = bring_table(&bad, &dest, &keep, "20261031-120000").unwrap_err();
        assert!(e.contains("3 行目"), "{e}");
        assert!(!dest.exists(), "nothing written when it does not read");
        let sjis = dir.join("syukujitsu.csv");
        std::fs::write(&sjis, b"2027/1/1,\x8c\xb3\x93\xfa\r\n2028/1/1,\x8c\xb3\x93\xfa\r\n").unwrap();
        assert_eq!(bring_table(&sjis, &dest, &keep, "20261031-120000"), Ok((2027, 2028)));
        assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&sjis).unwrap(), "the bytes as they are");
        assert!(!keep.exists(), "nothing there before: nothing kept");
        std::fs::write(&sjis, "2029/1/1,元日\n").unwrap();
        assert_eq!(bring_table(&sjis, &dest, &keep, "20261031-120000"), Ok((2029, 2029)));
        assert_eq!(std::fs::read(keep.join("20261031-120000.csv")).unwrap()[..8], b"2027/1/1"[..]);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "2029/1/1,元日\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
