//! Today as bars on one time axis: a lane per net, the runs it has had painted with their
//! state (a run from its start to its end, to now while it runs), and the starts still planned
//! for the rest of the day as outlines.
//!
//! Like the graph, the picture never stands alone: the lanes are also labelled
//! in words, and a click on a lane picks the run last started in it.

use chrono::NaiveDateTime;
use egui::{Pos2, Rect, Sense, Stroke, Vec2};
use koyomi_core::run::{Run, State};

use super::graph;

const LANE: f32 = 20.0;
const LABEL: f32 = 200.0;
const BAR_H: f32 = 12.0;
const MIN_BAR: f32 = 4.0;
/// The width of a planned start's outline.
const PLAN_W: f32 = 8.0;

/// One run's bar: its index in the runs given, and where it sits on the axis (0..=1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub run: usize,
    pub from: f32,
    pub to: f32,
    /// It started before the axis begins (an earlier day): the bar's left end is cut.
    pub before: bool,
}

/// One net's lane: the runs it has had today and the starts still planned for the rest of the day.
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// The net's id (`<group>/<name>`): two nets of one name in different groups are two lanes.
    pub id: String,
    pub title: String,
    /// Earliest start first.
    pub bars: Vec<Bar>,
    /// Planned starts and where they sit on the axis (0..=1), earliest first.
    pub planned: Vec<(NaiveDateTime, f32)>,
}

impl Lane {
    /// The bar of the run started last.
    fn latest(&self) -> Option<&Bar> {
        self.bars.iter().max_by(|a, b| a.from.total_cmp(&b.from).then(a.run.cmp(&b.run)))
    }
}

/// Where `t` sits on the axis from `lo` to `hi` (0..=1).
pub fn position(lo: NaiveDateTime, hi: NaiveDateTime, t: NaiveDateTime) -> f32 {
    ((t - lo).num_seconds() as f32 / (hi - lo).num_seconds() as f32).clamp(0.0, 1.0)
}

/// The axis (the whole day of `now`, 00:00 to 24:00) and a lane per net, the one that starts
/// earliest first. `planned` is every (net id, net title, start) still to come today.
pub fn lanes(runs: &[&Run], planned: &[(String, String, NaiveDateTime)], now: NaiveDateTime) -> (NaiveDateTime, NaiveDateTime, Vec<Lane>) {
    let end_of = |r: &Run| r.ended.unwrap_or(now).max(r.started);
    let lo = now.date().and_hms_opt(0, 0, 0).unwrap_or(now);
    let hi = lo + chrono::Duration::hours(24);
    let at = |t: NaiveDateTime| position(lo, hi, t);
    let mut out: Vec<Lane> = Vec::new();
    let mut lane_of = |id: &str, title: &str| -> usize {
        out.iter().position(|l| l.id == id).unwrap_or_else(|| {
            out.push(Lane { id: id.to_owned(), title: title.to_owned(), bars: Vec::new(), planned: Vec::new() });
            out.len() - 1
        })
    };
    let bars: Vec<(usize, Bar)> = runs
        .iter()
        .enumerate()
        .map(|(run, r)| (lane_of(&r.jobnet, &r.title), Bar { run, from: at(r.started), to: at(end_of(r)), before: r.started < lo }))
        .collect();
    let mut plan: Vec<(usize, NaiveDateTime)> = planned.iter().map(|(id, title, t)| (lane_of(id, title), *t)).collect();
    plan.sort();
    for (i, bar) in bars {
        out[i].bars.push(bar);
    }
    for (i, t) in plan {
        out[i].planned.push((t, at(t)));
    }
    for l in &mut out {
        l.bars.sort_by(|a, b| a.from.total_cmp(&b.from).then(a.run.cmp(&b.run)));
    }
    let first = |l: &Lane| l.bars.first().map(|b| b.from).into_iter().chain(l.planned.first().map(|p| p.1)).fold(f32::MAX, f32::min);
    out.sort_by(|a, b| first(a).total_cmp(&first(b)).then_with(|| a.title.cmp(&b.title)));
    (lo, hi, out)
}

/// The words for a lane: the net, then when its last run started, or else its next planned start.
fn words(lane: &Lane, runs: &[&Run], to_come: &str) -> String {
    let when = match (lane.latest(), lane.planned.first()) {
        (Some(bar), _) if bar.before => format!("◀ {}", runs[bar.run].started.format("%m-%d %H:%M:%S")),
        (Some(bar), _) => super::show::hms(runs[bar.run].started),
        (None, Some((t, _))) => format!("{to_come} {}", super::show::hms(*t)),
        (None, None) => String::new(),
    };
    format!("{}  {when}", lane.title)
}

/// Draws the lanes; the id of the run clicked, when one was (the latest run of the lane).
pub fn draw(
    ui: &mut egui::Ui,
    runs: &[&Run],
    planned: &[(String, String, NaiveDateTime)],
    now: NaiveDateTime,
    picked: Option<&str>,
    earlier: &str,
    to_come: &str,
) -> Option<String> {
    let c = ito_theme::colors();
    let (lo, hi, list) = lanes(runs, planned, now);
    let width = (ui.available_width() - LABEL - 16.0).max(200.0);
    let hours = ((hi - lo).num_hours()).max(1) as usize;
    let (resp, painter) = ui.allocate_painter(Vec2::new(LABEL + width + 16.0, 22.0 + LANE * list.len() as f32), Sense::hover());
    let origin = resp.rect.min;
    let x_of = |f: f32| origin.x + LABEL + f * width;
    let font = egui::FontId::proportional(12.0);
    for h in 0..=hours {
        let x = x_of(h as f32 / hours as f32);
        let major = h % 3 == 0;
        let line = if major { c.border } else { c.border.gamma_multiply(0.4) };
        painter.line_segment([Pos2::new(x, origin.y + 16.0), Pos2::new(x, resp.rect.max.y)], Stroke::new(1.0, line));
        if major {
            let label = format!("{:02}", h);
            painter.text(Pos2::new(x, origin.y), egui::Align2::CENTER_TOP, label, font.clone(), c.dim);
        }
    }
    let mut clicked = None;
    for (row, lane) in list.iter().enumerate() {
        let y = origin.y + 22.0 + row as f32 * LANE;
        let area = Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(LABEL + width, LANE));
        let id = ui.id().with(("gantt", &lane.id));
        let r = ui.interact(area, id, Sense::click());
        let latest = lane.latest().map(|b| runs[b.run]);
        if latest.is_some_and(|run| picked == Some(run.id.as_str())) || r.hovered() {
            painter.rect_filled(area, 2.0, c.border.gamma_multiply(0.35));
        }
        let text = words(lane, runs, to_come);
        painter.text(Pos2::new(origin.x + 4.0, y + LANE / 2.0), egui::Align2::LEFT_CENTER, &text, font.clone(), c.fg);
        let top = y + (LANE - BAR_H) / 2.0;
        for bar in &lane.bars {
            let (x0, x1) = (x_of(bar.from), x_of(bar.to).max(x_of(bar.from) + MIN_BAR));
            let rect = Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, top + BAR_H));
            painter.rect_filled(rect, 3.0, graph::colour(&c, runs[bar.run].state));
        }
        // A start still to come is an outline, not a fill.
        for (_, f) in &lane.planned {
            let rect = Rect::from_min_max(Pos2::new(x_of(*f), top), Pos2::new(x_of(*f) + PLAN_W, top + BAR_H));
            painter.rect_stroke(rect, 3.0, Stroke::new(1.5, graph::colour(&c, State::Waiting)), egui::StrokeKind::Inside);
        }
        // The marks are a picture; the words say them.
        let mut said = match lane.latest() {
            Some(bar) if bar.before => format!("{}  {earlier}, {}", lane.title, runs[bar.run].started.format("%m-%d %H:%M:%S")),
            _ => text.clone(),
        };
        if !lane.planned.is_empty() {
            let next: Vec<String> = lane.planned.iter().map(|(t, _)| super::show::hms(*t)).collect();
            said = match lane.latest() {
                Some(_) => format!("{said}, {to_come} {}", next.join(" ")),
                None => format!("{}  {to_come} {}", lane.title, next.join(" ")),
            };
        }
        ito_a11y::say(&r, ito_a11y::Role::Row, &said);
        if r.clicked() {
            clicked = latest.map(|run| run.id.clone());
        }
    }
    // Now, over the bars.
    let x = x_of(position(lo, hi, now));
    painter.line_segment([Pos2::new(x, origin.y + 16.0), Pos2::new(x, resp.rect.max.y)], Stroke::new(1.5, c.run));
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
    fn axis_is_the_whole_day_and_bars_sit_inside_it() {
        let a = run(at(9, 30), Some(at(10, 30)));
        let b = run(at(9, 45), None);
        let (lo, hi, list) = lanes(&[&a, &b], &[], at(11, 15));
        assert_eq!((lo, hi), (at(0, 0), at(0, 0) + chrono::Duration::hours(24)));
        // Both runs are of the net "n": one lane, two bars.
        assert_eq!(list.len(), 1);
        let bars = &list[0].bars;
        assert!((bars[0].from - 9.5 / 24.0).abs() < 1e-5 && (bars[0].to - 10.5 / 24.0).abs() < 1e-5);
        // Still running: the bar reaches now.
        assert!((bars[1].to - 11.25 / 24.0).abs() < 1e-5);
    }

    #[test]
    fn a_run_from_an_earlier_day_is_cut_at_the_left_and_marked() {
        let early = run(at(0, 0) - chrono::Duration::hours(1), None);
        let today = run(at(9, 0), Some(at(10, 0)));
        let (lo, hi, list) = lanes(&[&early, &today], &[], at(11, 0));
        let bars = &list[0].bars;
        assert!(bars[0].before && bars[0].from == 0.0, "{bars:?}");
        assert!(!bars[1].before);
        assert!((position(lo, hi, at(11, 0)) - 11.0 / 24.0).abs() < 1e-5, "the now line");
    }

    #[test]
    fn a_net_with_only_planned_starts_gets_a_lane_and_lanes_go_by_the_first_start() {
        let a = run(at(9, 0), Some(at(9, 5)));
        let planned = vec![
            ("g/later".to_owned(), "later".to_owned(), at(20, 0)),
            ("g/later".to_owned(), "later".to_owned(), at(18, 0)),
            ("g/n".to_owned(), "n".to_owned(), at(12, 0)),
            ("g/early".to_owned(), "early".to_owned(), at(7, 0)),
        ];
        let (_, _, list) = lanes(&[&a], &planned, at(8, 0));
        let titles: Vec<&str> = list.iter().map(|l| l.title.as_str()).collect();
        assert_eq!(titles, ["early", "n", "later"]);
        assert!(list[0].bars.is_empty() && list[0].planned.len() == 1);
        assert_eq!(list[1].bars.len(), 1);
        assert_eq!(list[1].planned.len(), 1, "a run and a plan share the lane");
        assert_eq!(list[2].planned[0].0, at(18, 0), "earliest first");
    }

    #[test]
    fn the_words_name_the_last_run_or_the_next_plan() {
        let a = run(at(9, 0), Some(at(9, 5)));
        let planned = vec![("g/n".to_owned(), "n".to_owned(), at(12, 0)), ("g/only".to_owned(), "only".to_owned(), at(15, 30))];
        let runs = [&a];
        let (_, _, list) = lanes(&runs, &planned, at(10, 0));
        let by = |t: &str| list.iter().find(|l| l.title == t).unwrap();
        assert_eq!(words(by("n"), &runs, "next"), "n  09:00:00");
        assert_eq!(words(by("only"), &runs, "next"), "only  next 15:30:00");
    }

    #[test]
    fn two_nets_of_one_name_in_different_groups_get_two_lanes() {
        let a = run(at(9, 0), Some(at(9, 5)));
        let planned = vec![("other/n".to_owned(), "n".to_owned(), at(12, 0))];
        let (_, _, list) = lanes(&[&a], &planned, at(10, 0));
        assert_eq!(list.len(), 2, "{list:?}");
        let ids: Vec<&str> = list.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["g/n", "other/n"]);
        assert!(list[0].planned.is_empty() && list[1].bars.is_empty());
    }

    #[test]
    fn no_runs_no_bars() {
        assert!(lanes(&[], &[], at(9, 0)).2.is_empty());
    }
}
