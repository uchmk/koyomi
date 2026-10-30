//! The calendars in the window: a list under 今日 on the left, and the month
//! table in the detail when one is picked. A click on a day writes
//! `calendars/<name>.toml` (a weekend or holiday becomes `open`, a weekday
//! `closed`, and a second click takes it back); the engine's watcher reads the
//! file again and the table follows what the engine counts by.

use chrono::{Datelike, NaiveDate, Weekday};
use ito_a11y::Role;
use koyomi_core::calendar::{Calendar, Day};
use koyomi_core::scan::Standing;

use super::Browser;
use crate::show;

const WEEK: [Weekday; 7] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];

impl Browser {
    /// The calendars by name, under today's runs. Nothing when there are none.
    pub(super) fn calendar_list(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let Some(snap) = &self.snap else { return };
        if snap.calendars.is_empty() {
            return;
        }
        let c = ito_theme::colors();
        ui.add_space(10.0);
        let r = ui.label(egui::RichText::new(t.tr("calendar.heading")).strong().color(c.dim));
        ito_a11y::say(&r, Role::Heading, t.tr("calendar.heading"));
        let mut pick = None;
        for name in snap.calendars.keys() {
            let on = self.cal_sel.as_deref() == Some(name.as_str());
            let r = ui.selectable_label(on, format!("  {name}"));
            ito_a11y::say(&r, Role::Row, name);
            if r.clicked() {
                pick = Some(name.clone());
            }
        }
        if pick.is_some() {
            self.cal_sel = pick;
            self.cal_month = None;
            self.cal_result = None;
            self.run_sel = None;
            self.job_sel = None;
            self.result = None;
            self.hist = None;
        }
    }

    /// The month as a table, the named days, the holiday table's end, and the
    /// definitions that count by this calendar with the picked one's next times.
    pub(super) fn calendar_view(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(name) = self.cal_sel.clone() else { return };
        let Some(cal) = self.snap.as_ref().and_then(|s| s.calendars.get(&name)).cloned() else {
            ui.label(egui::RichText::new(t.trf("calendar.missing", &[&name])).color(c.err));
            return;
        };
        let today = chrono::Local::now().date_naive();
        let (y, m) = self.cal_month.unwrap_or((today.year(), today.month()));
        let head = show::month_head(t, &cal, y, m);
        ui.horizontal(|ui| {
            let r = ui.button("◀");
            ito_a11y::say(&r, Role::Button, t.tr("calendar.prev"));
            if r.clicked() {
                self.cal_month = Some(if m == 1 { (y - 1, 12) } else { (y, m - 1) });
            }
            let r = ui.heading(&head);
            ito_a11y::say(&r, Role::Heading, &head);
            let r = ui.button("▶");
            ito_a11y::say(&r, Role::Button, t.tr("calendar.next"));
            if r.clicked() {
                self.cal_month = Some(if m == 12 { (y + 1, 1) } else { (y, m + 1) });
            }
        });
        ui.add(egui::Label::new(egui::RichText::new(t.tr("calendar.hint")).color(c.dim)).wrap());
        ui.add_space(6.0);
        let days = show::month_days(&cal, y, m);
        let mut toggle: Option<(NaiveDate, Day)> = None;
        egui::Grid::new("calendar").min_col_width(36.0).show(ui, |ui| {
            for w in WEEK {
                let colour = if matches!(w, Weekday::Sat | Weekday::Sun) { c.dim } else { c.fg };
                ui.label(egui::RichText::new(weekday(t, w)).color(colour));
            }
            ui.end_row();
            let Some((first, _)) = days.first() else { return };
            for _ in 0..first.weekday().num_days_from_monday() {
                ui.label("");
            }
            for (d, day) in &days {
                let colour = match day {
                    Day::Business => c.fg,
                    Day::Open(_) => c.done,
                    Day::Closed(_) | Day::Holiday(_) => c.wait_text(),
                    Day::WeeklyOff => c.dim,
                };
                let text = match day {
                    Day::Open(_) => format!("[{:>2}]", d.day()),
                    Day::Closed(_) | Day::Holiday(_) | Day::WeeklyOff => format!("({:>2})", d.day()),
                    Day::Business => format!(" {:>2} ", d.day()),
                };
                let r = ui.selectable_label(*d == today, egui::RichText::new(text).color(colour).monospace());
                let kind = show::day_name(t, day)
                    .unwrap_or_else(|| t.tr(if day.is_business() { "calendar.business" } else { "calendar.off" }).to_owned());
                ito_a11y::say(&r, Role::Button, &format!("{} {}（{kind}）", d, weekday(t, d.weekday())));
                if r.clicked() {
                    toggle = Some((*d, day.clone()));
                }
                if d.weekday() == Weekday::Sun {
                    ui.end_row();
                }
            }
        });
        if let Some((d, day)) = toggle {
            self.cal_result = Some(toggle_day(t, &name, &cal, d, &day));
        }
        ui.add_space(6.0);
        for (d, day) in &days {
            if let Some(n) = show::day_name(t, day) {
                let colour = if matches!(day, Day::Open(_)) { c.done } else { c.wait_text() };
                ui.label(egui::RichText::new(format!("{} {n}", d.day())).color(colour));
            }
        }
        if let Some(line) = show::until_line(t, &cal, today) {
            let colour = if cal.stale(today) { c.wait_text() } else { c.dim };
            ui.label(egui::RichText::new(line).color(colour));
        }
        if let Some(r) = &self.cal_result {
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

fn toggle_day(t: &ito_i18n::Table, name: &str, cal: &Calendar, d: NaiveDate, day: &Day) -> Result<String, String> {
    let file = format!("calendars/{name}.toml");
    let path = ito_common::app_dir("koyomi").ok_or_else(|| t.trf("calendar.write_failed", &[&file, "app_dir"]))?.join(&file);
    let write = || -> Result<(), String> {
        let text = ito_common::read_or_empty(&path)?;
        let text = rewrite(&text, &toggled(cal, d, day))?;
        ito_common::write_atomic(&path, text)
    };
    match write() {
        Ok(()) => Ok(t.trf("calendar.written", &[&file])),
        Err(e) => Err(t.trf("calendar.write_failed", &[&file, &e])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAL: &str =
        "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [{ date = 2026-11-20, name = \"創立記念日\" }]\n";

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
}
