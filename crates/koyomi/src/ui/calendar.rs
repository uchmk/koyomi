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
/// A day's cell: the date over its name, wide enough for four characters of the name.
const CELL: egui::Vec2 = egui::vec2(72.0, 44.0);

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
        // This calendar's own cautions in a band: this is where the person looks to fix them. When
        // the holiday table ends soon, the band carries the button that brings a new one.
        let cautions: Vec<String> = self.snap.iter().flat_map(|s| s.calendar_cautions.get(&name).into_iter().flatten().cloned()).collect();
        let table_ends = cal.until().is_some_and(|u| (u - today).num_days() <= crate::engine::CAUTION_DAYS);
        let mut bring = false;
        if !cautions.is_empty() {
            egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.wait)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
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
            });
            ui.add_space(4.0);
        }
        let (y, m) = self.cal_month.unwrap_or((today.year(), today.month()));
        // 会社  ◀ 2026年11月 ▶  営業日 20 日; the screen reader hears it as one heading.
        let head = show::month_head(t, &cal, y, m);
        let business = show::month_days(&cal, y, m).iter().filter(|(_, d)| d.is_business()).count();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).strong());
            ui.add_space(8.0);
            let r = ui.button("◀");
            ito_a11y::say(&r, Role::Button, t.tr("calendar.prev"));
            if r.clicked() {
                self.cal_month = Some(if m == 1 { (y - 1, 12) } else { (y, m - 1) });
            }
            let r = ui.heading(t.trf("calendar.ym", &[&y.to_string(), &m.to_string()]));
            ito_a11y::say(&r, Role::Heading, &head);
            let r = ui.button("▶");
            ito_a11y::say(&r, Role::Button, t.tr("calendar.next"));
            if r.clicked() {
                self.cal_month = Some(if m == 12 { (y + 1, 1) } else { (y, m + 1) });
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t.trf("calendar.business_n", &[&business.to_string()])).color(c.dim));
        });
        ui.add(egui::Label::new(egui::RichText::new(t.tr("calendar.hint")).color(c.dim)).wrap());
        // The rules of the week, set in place; and the file itself.
        let mut rules = None;
        let mut act = None;
        ui.horizontal_wrapped(|ui| {
            let mut holidays = cal.holidays;
            // egui names a checkbox for the screen reader itself.
            let mut changed = ui.checkbox(&mut holidays, t.tr("newcal.holidays")).changed();
            ui.label(t.tr("newcal.off"));
            let mut off = WEEK.map(|w| cal.weekly_off.contains(&w));
            // Sunday first, as the table below; `off` stays Monday first.
            for w in GRID {
                changed |= ui.checkbox(&mut off[w.num_days_from_monday() as usize], weekday(t, w)).changed();
            }
            if changed {
                rules = Some((holidays, off));
            }
            ui.separator();
            for (i, key) in ["calendar.rename", "calendar.delete", "calendar.table", "calendar.print"].into_iter().enumerate() {
                let r = ui.add_enabled(!self.blocked(), egui::Button::new(t.tr(key)));
                ito_a11y::say(&r, Role::Button, t.tr(key));
                if r.clicked() {
                    act = Some(i);
                }
            }
        });
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
        ui.add_space(6.0);
        let days = show::month_days(&cal, y, m);
        // The day whose starts show beside the month: the one picked, or today in this month.
        let picked =
            self.cal_day.filter(|d| d.year() == y && d.month() == m).or_else(|| days.iter().any(|(d, _)| *d == today).then_some(today));
        let mut pick = None;
        let mut toggle: Option<(NaiveDate, Day)> = None;
        let mut naming = None;
        ui.horizontal_top(|ui| {
            egui::Grid::new("calendar").min_col_width(CELL.x).spacing(egui::vec2(2.0, 2.0)).show(ui, |ui| {
                for w in GRID {
                    let colour = if matches!(w, Weekday::Sat | Weekday::Sun) { c.dim } else { c.fg };
                    ui.add_sized([CELL.x, 20.0], egui::Label::new(egui::RichText::new(weekday(t, w)).color(colour)));
                }
                ui.end_row();
                let Some((first, _)) = days.first() else { return };
                for _ in 0..first.weekday().num_days_from_sunday() {
                    ui.allocate_space(CELL);
                }
                for (d, day) in &days {
                    let colour = day_colour(&c, day);
                    // A named day carries its name under the date, cut to fit; the whole name is on hover.
                    let named = show::day_name(t, day);
                    let short = named.as_deref().map(|n| {
                        if n.chars().count() > 4 {
                            format!("{}…", n.chars().take(3).collect::<String>())
                        } else {
                            n.to_owned()
                        }
                    });
                    let text = format!(
                        "{}
{}",
                        d.day(),
                        short.unwrap_or_default()
                    );
                    let fill = if day.is_business() { c.panel } else { c.side };
                    let here = picked == Some(*d);
                    let stroke = match (*d == today, here) {
                        (true, true) => egui::Stroke::new(3.0, c.blue),
                        (true, false) => egui::Stroke::new(2.0, c.blue),
                        (false, true) => egui::Stroke::new(2.0, c.fg),
                        (false, false) => egui::Stroke::new(1.0, c.border),
                    };
                    let r = ui.add_sized(CELL, egui::Button::new(egui::RichText::new(text).color(colour)).fill(fill).stroke(stroke));
                    let r = match &named {
                        Some(n) => r.on_hover_text(n),
                        None => r,
                    };
                    let kind =
                        named.unwrap_or_else(|| t.tr(if day.is_business() { "calendar.business" } else { "calendar.off" }).to_owned());
                    ito_a11y::say(&r, Role::Button, &format!("{} {}（{kind}）", d, weekday(t, d.weekday())));
                    // A click picks the day; a double click or the menu moves it between open and closed.
                    if r.double_clicked() {
                        toggle = Some((*d, day.clone()));
                    } else if r.clicked() {
                        pick = Some(*d);
                    }
                    r.context_menu(|ui| {
                        if ui.button(t.tr(toggle_key(day))).clicked() {
                            toggle = Some((*d, day.clone()));
                            ui.close();
                        }
                        if ui.button(t.tr("calendar.name_day")).clicked() {
                            let text = match day {
                                Day::Open(n) | Day::Closed(n) => n.clone(),
                                _ => String::new(),
                            };
                            naming = Some(Inline::Name { cal: name.clone(), day: *d, text, fresh: true });
                            ui.close();
                        }
                    });
                    if d.weekday() == Weekday::Sat {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(12.0);
            if let Some(d) = picked {
                let day = days.iter().find(|(x, _)| *x == d).map(|(_, day)| day.clone()).unwrap_or(Day::Business);
                if self.day_box(ui, t, &cal, d, &day) {
                    toggle = Some((d, day));
                }
            }
        });
        if pick.is_some() {
            self.cal_day = pick;
        }
        if let Some((d, day)) = toggle {
            self.cal_result = Some(toggle_day(t, &name, &cal, d, &day));
        }
        if naming.is_some() {
            self.cal_inline = naming;
        }
        // What the colours and the frame mean.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let legend = [
                (c.fg, "calendar.business"),
                (c.dim, "calendar.legend_off"),
                (c.wait, "calendar.legend_holiday"),
                (c.magenta, "calendar.legend_closed"),
                (c.done, "calendar.open"),
            ];
            for (colour, key) in legend {
                ui.label(egui::RichText::new("■").color(colour));
                ui.label(egui::RichText::new(t.tr(key)).color(c.dim));
                ui.add_space(6.0);
            }
            ui.label(egui::RichText::new("□").color(c.blue));
            ui.label(egui::RichText::new(t.tr("calendar.legend_today")).color(c.dim));
        });
        self.inline_view(ui, t, &name, &cal);
        ui.add_space(6.0);
        for (d, day) in &days {
            if let Some(n) = show::day_name(t, day) {
                ui.label(egui::RichText::new(format!("{} {n}", d.day())).color(day_colour(&c, day)));
            }
        }
        if let Some(line) = show::until_line(t, &cal, today) {
            let colour = if cal.stale(today) { c.wait_text() } else { c.dim };
            ui.label(egui::RichText::new(line).color(colour));
        }
        for r in [&self.cal_result, &self.result].into_iter().flatten() {
            match r {
                Ok(s) => ui.label(egui::RichText::new(s).color(c.dim)),
                Err(e) => ui.label(egui::RichText::new(e).color(c.err)),
            };
        }
        // The definitions on this calendar; the one picked in the tree with its next times.
        ui.add_space(8.0);
        let Some(snap) = &self.snap else { return };
        let users: Vec<_> = snap.defs.iter().filter(|d| d.calendar.as_deref() == Some(name.as_str())).collect();
        if users.is_empty() {
            ui.label(egui::RichText::new(t.tr("calendar.used_none")).color(c.dim));
            return;
        }
        let r = ui.label(egui::RichText::new(t.tr("calendar.used_by")).strong().color(c.dim));
        ito_a11y::say(&r, Role::Heading, t.tr("calendar.used_by"));
        for d in &users {
            ui.label(format!("  {}", d.label()));
        }
        let picked = users.iter().find(|d| self.selected.as_deref() == Some(d.id.as_str()));
        if let Some(net) = picked.and_then(|d| d.net.as_ref().map(|n| (d, n))) {
            let (d, net) = net;
            if !net.when.is_empty() {
                ui.add_space(6.0);
                let head = format!("{}: {}", d.label(), t.trf("run.next", &["10"]));
                let r = ui.label(egui::RichText::new(&head).strong().color(c.dim));
                ito_a11y::say(&r, Role::Heading, &head);
                if d.standing != Standing::Confirmed {
                    ui.label(egui::RichText::new(t.tr("run.next_unconfirmed")).color(c.wait_text()));
                }
                let power = self.snap.as_ref().map(|s| show::Sleep::of(&s.power, net.wake));
                let now = chrono::Local::now().naive_local();
                for row in show::next_rows(t, &net.when, (&net.quiet, net.on_quiet), Some(&cal), now, 10, power.as_ref()) {
                    ui.label(format!("  {row}"));
                }
            }
        }
    }
}

impl Browser {
    /// この日に動くもの（11/7 土・営業日 5）beside the month: the approved starts of that day, on
    /// every calendar, and the button that moves the day between open and closed. True when it
    /// is pressed.
    fn day_box(&self, ui: &mut egui::Ui, t: &ito_i18n::Table, cal: &Calendar, d: NaiveDate, day: &Day) -> bool {
        let c = ito_theme::colors();
        let starts = self.snap.as_ref().map(|s| crate::plan::day(&s.defs, &s.calendars, d).1).unwrap_or_default();
        let mut pressed = false;
        // As wide as the room beside the month allows, up to 300 (the frame's margin and line take 22).
        let width = (ui.available_width() - 22.0).clamp(180.0, 300.0);
        egui::Frame::new().fill(c.side).stroke(egui::Stroke::new(1.0, c.border)).inner_margin(10.0).corner_radius(4.0).show(ui, |ui| {
            // The box sits in the month's row; inside it reads top to bottom, or each word would
            // stand beside the last and wrap a letter at a time.
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(width);
                let head = t.trf(
                    "calendar.day_head",
                    &[&format!("{}/{}", d.month(), d.day()), &weekday(t, d.weekday()), &day_kind(t, cal, d, day)],
                );
                let r = ui.add(egui::Label::new(egui::RichText::new(&head).strong()).wrap());
                ito_a11y::say(&r, Role::Heading, &head);
                ui.add_space(6.0);
                if starts.is_empty() {
                    ui.label(egui::RichText::new(t.tr("calendar.day_none")).color(c.dim));
                } else {
                    egui::Grid::new("cal_day_starts").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                        for (at, net) in &starts {
                            ui.label(egui::RichText::new(at.format("%H:%M").to_string()).monospace().color(c.dim));
                            ui.add(egui::Label::new(net.to_string()).wrap());
                            ui.end_row();
                        }
                    });
                }
                ui.add_space(6.0);
                ui.add(egui::Label::new(egui::RichText::new(t.tr("calendar.day_note")).color(c.dim).small()).wrap());
                ui.add_space(6.0);
                let r = ui.add_enabled(!self.blocked(), egui::Button::new(t.tr(toggle_key(day))));
                ito_a11y::say(&r, Role::Button, t.tr(toggle_key(day)));
                pressed = r.clicked();
            });
        });
        pressed
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
        Day::Holiday(_) => c.wait,
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
