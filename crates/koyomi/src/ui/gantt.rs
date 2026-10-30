//! Today's runs as bars on one time axis: a lane per run, the bar from when it
//! started to when it ended (to now, while it runs), painted with its state.
//!
//! Like the graph, the picture never stands alone: the lanes are also labelled
//! in words, and a click on a lane picks the run.

use chrono::{NaiveDateTime, Timelike};
use egui::{Pos2, Rect, Sense, Stroke, Vec2};
use koyomi_core::run::Run;

use super::graph;

const LANE: f32 = 26.0;
const LABEL: f32 = 200.0;
const BAR_H: f32 = 14.0;
const MIN_BAR: f32 = 3.0;

/// One run's bar: its index in the runs given, and where it sits on the axis (0..=1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub run: usize,
    pub from: f32,
    pub to: f32,
}

/// The axis (whole hours, at least one) and a bar per run, earliest start first.
pub fn bars(runs: &[&Run], now: NaiveDateTime) -> (NaiveDateTime, NaiveDateTime, Vec<Bar>) {
    let end_of = |r: &Run| r.ended.unwrap_or(now).max(r.started);
    let Some(first) = runs.iter().map(|r| r.started).min() else { return (now, now, Vec::new()) };
    let last = runs.iter().map(|r| end_of(r)).max().unwrap_or(first);
    let lo = first.date().and_hms_opt(first.hour(), 0, 0).unwrap_or(first);
    let mut hi = last.date().and_hms_opt(last.hour(), 0, 0).unwrap_or(last) + chrono::Duration::hours(1);
    if hi <= lo {
        hi = lo + chrono::Duration::hours(1);
    }
    let span = (hi - lo).num_seconds() as f32;
    let at = |t: NaiveDateTime| ((t - lo).num_seconds() as f32 / span).clamp(0.0, 1.0);
    let mut out: Vec<Bar> = runs.iter().enumerate().map(|(run, r)| Bar { run, from: at(r.started), to: at(end_of(r)) }).collect();
    out.sort_by(|a, b| a.from.total_cmp(&b.from).then(a.run.cmp(&b.run)));
    (lo, hi, out)
}

/// Draws the lanes; the id of the run clicked, when one was.
pub fn draw(ui: &mut egui::Ui, runs: &[&Run], now: NaiveDateTime, picked: Option<&str>) -> Option<String> {
    let c = ito_theme::colors();
    let (lo, hi, list) = bars(runs, now);
    let width = (ui.available_width() - LABEL - 16.0).max(200.0);
    let hours = ((hi - lo).num_hours()).max(1) as usize;
    let (resp, painter) = ui.allocate_painter(Vec2::new(LABEL + width + 16.0, 22.0 + LANE * list.len() as f32), Sense::hover());
    let origin = resp.rect.min;
    let x_of = |f: f32| origin.x + LABEL + f * width;
    let font = egui::FontId::proportional(12.0);
    for h in 0..=hours {
        let x = x_of(h as f32 / hours as f32);
        painter.line_segment([Pos2::new(x, origin.y + 16.0), Pos2::new(x, resp.rect.max.y)], Stroke::new(1.0, c.border));
        let label = format!("{:02}", (lo + chrono::Duration::hours(h as i64)).hour());
        painter.text(Pos2::new(x, origin.y), egui::Align2::CENTER_TOP, label, font.clone(), c.dim);
    }
    let mut clicked = None;
    for (row, bar) in list.iter().enumerate() {
        let run = runs[bar.run];
        let y = origin.y + 22.0 + row as f32 * LANE;
        let lane = Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(LABEL + width, LANE));
        let id = ui.id().with(("gantt", &run.id));
        let r = ui.interact(lane, id, Sense::click());
        if picked == Some(run.id.as_str()) || r.hovered() {
            painter.rect_filled(lane, 2.0, c.border.gamma_multiply(0.35));
        }
        let text = format!("{}  {}", run.title, super::show::hms(run.started));
        painter.text(Pos2::new(origin.x + 4.0, y + LANE / 2.0), egui::Align2::LEFT_CENTER, &text, font.clone(), c.fg);
        let (x0, x1) = (x_of(bar.from), x_of(bar.to).max(x_of(bar.from) + MIN_BAR));
        let rect = Rect::from_min_max(Pos2::new(x0, y + (LANE - BAR_H) / 2.0), Pos2::new(x1, y + (LANE + BAR_H) / 2.0));
        painter.rect_filled(rect, 3.0, graph::colour(&c, run.state));
        ito_a11y::say(&r, ito_a11y::Role::Row, &text);
        if r.clicked() {
            clicked = Some(run.id.clone());
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, 5).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn run(start: NaiveDateTime, end: Option<NaiveDateTime>) -> Run {
        let toml = format!(
            "id = \"r\"\njobnet = \"g/n\"\ntitle = \"n\"\ndate = \"2026-01-05\"\nhow = \"schedule\"\nstarted = \"{}\"\nstate = \"running\"\n{}jobs = []\n",
            start.format("%Y-%m-%dT%H:%M:%S"),
            end.map(|e| format!("ended = \"{}\"\n", e.format("%Y-%m-%dT%H:%M:%S"))).unwrap_or_default()
        );
        toml::from_str(&toml).unwrap()
    }

    #[test]
    fn axis_is_whole_hours_and_bars_sit_inside_it() {
        let a = run(at(9, 30), Some(at(10, 30)));
        let b = run(at(9, 45), None);
        let (lo, hi, list) = bars(&[&a, &b], at(11, 15));
        assert_eq!((lo, hi), (at(9, 0), at(12, 0)));
        assert_eq!(list.len(), 2);
        assert!((list[0].from - 0.5 / 3.0).abs() < 1e-5 && (list[0].to - 1.5 / 3.0).abs() < 1e-5);
        // Still running: the bar reaches now.
        assert!((list[1].to - 2.25 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn no_runs_no_bars() {
        assert!(bars(&[], at(9, 0)).2.is_empty());
    }
}
