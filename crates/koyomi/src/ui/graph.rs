//! A run as a line of stations: one circle per job, the lines between them
//! painted with the result that went through, flowing sideways (left to
//! right) or downwards, and a running job's lines pulsing towards what comes
//! next.
//!
//! The picture never stands alone: the row of jobs under it says the same
//! in words (symbol + state + name), and that row is what the keyboard and
//! the screen reader get.

use egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};
use koyomi_core::jobnet::{Job, On};
use koyomi_core::layout;
use koyomi_core::run::{JobRun, Run, State, Tone};

const STEP_ALONG: f32 = 150.0;
const STEP_ACROSS: f32 = 64.0;
const RADIUS: f32 = 11.0;
const MARGIN: f32 = 28.0;
/// How far a line's pulses move each second, in points.
const FLOW: f64 = 40.0;

/// The colour a state paints with.
pub fn colour(c: &ito_theme::Colors, s: State) -> Color32 {
    match s.tone() {
        Tone::Ok => c.done,
        Tone::Warn | Tone::Held => c.wait,
        Tone::Error => c.err,
        Tone::Running => c.run,
        Tone::Dim => c.dim,
    }
}

/// The colour of a state's mark and word in lists (the chips, the table, the chart's key): as
/// [`colour`], but the quiet states get a hue of their own so every mark reads in colour.
pub fn mark_colour(c: &ito_theme::Colors, s: State) -> Color32 {
    match s {
        State::Waiting => c.blue,
        State::Skipped | State::NotRun => c.magenta,
        _ => colour(c, s),
    }
}

/// The softer colour of a line that a state came through.
fn line_colour(c: &ito_theme::Colors, s: State) -> Color32 {
    ito_theme::mix(colour(c, s), c.bg, if c.light { 0.55 } else { 0.45 })
}

/// Where each job sits, by its index in `run.jobs`. With the net's `jobs`
/// (when every job of the run is among them), each sits where the editor
/// puts it: its saved `pos`, or the place 整列 gives one without, so the
/// picture is the flow the person laid out. Else by the run's layers.
pub fn places(run: &Run, vertical: bool, net: Option<&[Job]>) -> Vec<Pos2> {
    if let Some(at) = net.and_then(|jobs| laid_out(run, vertical, jobs)) {
        return at;
    }
    let mut at = vec![Pos2::ZERO; run.jobs.len()];
    for (l, layer) in run.layers().iter().enumerate() {
        for (i, &j) in layer.iter().enumerate() {
            let (along, across) = (MARGIN + l as f32 * STEP_ALONG, MARGIN + i as f32 * STEP_ACROSS);
            at[j] = turn(along, across, vertical);
        }
    }
    at
}

/// A place along and across the flow, on the picture.
fn turn(along: f32, across: f32, vertical: bool) -> Pos2 {
    // Room before the first station for its name, centred under it.
    if vertical {
        Pos2::new(across + 60.0, along)
    } else {
        Pos2::new(along + 50.0, across)
    }
}

/// The editor's places of the run's jobs, moved so the first sits at the margin.
fn laid_out(run: &Run, vertical: bool, jobs: &[Job]) -> Option<Vec<Pos2>> {
    let placed = layout::places(jobs);
    let editor: Vec<(f64, f64)> = jobs
        .iter()
        .zip(placed)
        .map(|(j, (l, r))| j.pos.unwrap_or((l as f64 * layout::STEP_ALONG, r as f64 * layout::STEP_ACROSS)))
        .collect();
    let mine: Vec<(f64, f64)> =
        run.jobs.iter().map(|r| jobs.iter().position(|j| j.id == r.id).map(|i| editor[i])).collect::<Option<_>>()?;
    let x0 = mine.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let y0 = mine.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    // The editor's rows are closer than the picture's, which has names under its stations.
    let across = (STEP_ACROSS as f64) / layout::STEP_ACROSS;
    Some(mine.iter().map(|&(x, y)| turn(MARGIN + (x - x0) as f32, MARGIN + ((y - y0) * across) as f32, vertical)).collect())
}

/// The size the picture needs.
pub fn size(at: &[Pos2], vertical: bool) -> Vec2 {
    let (mut w, mut h) = (0f32, 0f32);
    for p in at {
        w = w.max(p.x);
        h = h.max(p.y);
    }
    Vec2::new(w + MARGIN + if vertical { 140.0 } else { 100.0 }, h + MARGIN + if vertical { 12.0 } else { 40.0 })
}

/// The way a line goes from one station to the next: straight when they
/// sit in a row, else an S-curve that leaves and arrives along the flow.
pub fn path(from: Pos2, to: Pos2, vertical: bool) -> Vec<Pos2> {
    let aligned = if vertical { (from.x - to.x).abs() < 0.5 } else { (from.y - to.y).abs() < 0.5 };
    if aligned {
        return vec![from, to];
    }
    let (c1, c2) = if vertical {
        let m = (from.y + to.y) / 2.0;
        (Pos2::new(from.x, m), Pos2::new(to.x, m))
    } else {
        let m = (from.x + to.x) / 2.0;
        (Pos2::new(m, from.y), Pos2::new(m, to.y))
    };
    (0..=24)
        .map(|k| {
            let t = k as f32 / 24.0;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            Pos2::new(w[0] * from.x + w[1] * c1.x + w[2] * c2.x + w[3] * to.x, w[0] * from.y + w[1] * c1.y + w[2] * c2.y + w[3] * to.y)
        })
        .collect()
}

/// The dashes `(start, end)`, as distances along a line `len` long, `dash`
/// long with `gap` between them and moved `phase` towards the line's end.
pub fn dashes(len: f32, dash: f32, gap: f32, phase: f32) -> Vec<(f32, f32)> {
    let period = dash + gap;
    let mut out = Vec::new();
    let mut s = phase.rem_euclid(period) - period;
    while s < len {
        let (a, b) = (s.max(0.0), (s + dash).min(len));
        if b > a {
            out.push((a, b));
        }
        s += period;
    }
    out
}

/// The part of `points` from `a` to `b` along it.
pub(crate) fn piece(points: &[Pos2], a: f32, b: f32) -> Vec<Pos2> {
    let mut out = Vec::new();
    let mut walked = 0.0;
    for w in points.windows(2) {
        let seg = (w[1] - w[0]).length();
        if seg <= 0.0 {
            continue;
        }
        let (s, e) = (walked, walked + seg);
        if e >= a && s <= b {
            let at = |d: f32| w[0] + (w[1] - w[0]) * ((d - s) / seg).clamp(0.0, 1.0);
            if out.is_empty() {
                out.push(at(a.max(s)));
            }
            out.push(at(b.min(e)));
        }
        walked = e;
    }
    out
}

pub(crate) fn length(points: &[Pos2]) -> f32 {
    points.windows(2).map(|w| (w[1] - w[0]).length()).sum()
}

pub(crate) fn dashed(painter: &egui::Painter, points: &[Pos2], stroke: Stroke, dash: f32, gap: f32, phase: f32) {
    for (a, b) in dashes(length(points), dash, gap, phase) {
        painter.add(Shape::line(piece(points, a, b), stroke));
    }
}

/// When the job ran, under its name: `9:00–9:04`, `9:00–` while it runs,
/// and the exit code after an abnormal end.
pub fn subtext(j: &JobRun) -> Option<String> {
    let started = j.started?;
    let mut s = format!("{}–", started.format("%-H:%M"));
    if let Some(e) = j.ended {
        s.push_str(&e.format("%-H:%M").to_string());
    }
    if let Some(code) = j.exit.filter(|&x| x != 0) {
        s.push_str(&format!(" exit {code}"));
    }
    Some(s)
}

/// Draws the run; the job clicked, when one was.
/// `net` is the net's jobs, for where the editor put them.
pub fn draw(ui: &mut egui::Ui, run: &Run, vertical: bool, selected: Option<&str>, net: Option<&[Job]>) -> Option<String> {
    let c = ito_theme::colors();
    let at = places(run, vertical, net);
    let (resp, painter) = ui.allocate_painter(size(&at, vertical), Sense::click());
    let origin = resp.rect.min.to_vec2();
    let at: Vec<Pos2> = at.into_iter().map(|p| p + origin).collect();
    let moving = run.jobs.iter().any(|j| j.state.tone() == Tone::Running);
    let time = ui.input(|i| i.time);
    let phase = if moving { (time * FLOW) as f32 } else { 0.0 };
    if moving {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    // The lines first. One a result came through is thick, in that result's
    // softer colour (an error / any / code line thinner); one out of a running
    // job carries pulses towards the next job; one not reached yet is dotted.
    for (i, j) in run.jobs.iter().enumerate() {
        for l in &j.after {
            let Some(p) = run.jobs.iter().position(|x| x.id == l.job) else { continue };
            let points = path(at[p], at[i], vertical);
            let pred = &run.jobs[p];
            let width = if l.on == On::Ok { 6.0 } else { 3.5 };
            if pred.state.tone() == Tone::Running {
                painter.add(Shape::line(points.clone(), Stroke::new(width, line_colour(&c, pred.state))));
                dashed(&painter, &points, Stroke::new(width * 0.6, colour(&c, pred.state)), 10.0, 14.0, phase);
            } else if pred.state.is_final() && run.line_met(l) {
                painter.add(Shape::line(points, Stroke::new(width, line_colour(&c, pred.state))));
            } else {
                dashed(&painter, &points, Stroke::new(2.0, c.border_strong()), 2.0, 5.0, 0.0);
            }
        }
    }
    let small = egui::TextStyle::Small.resolve(ui.style());
    let body = egui::TextStyle::Body.resolve(ui.style());
    for (i, j) in run.jobs.iter().enumerate() {
        let p = at[i];
        let col = colour(&c, j.state);
        let quiet = matches!(j.state, State::NotRun | State::Waiting);
        if selected == Some(j.id.as_str()) {
            painter.circle(p, RADIUS + 7.0, ito_theme::mix(c.blue, c.bg, 0.75), Stroke::new(2.0, c.blue));
        }
        if j.state.tone() == Tone::Running {
            // A ring that grows and fades, once a second.
            let t = time.fract() as f32;
            painter.circle_stroke(p, RADIUS + 2.0 + 10.0 * t, Stroke::new(3.0 * (1.0 - t) + 0.5, ito_theme::mix(col, c.bg, t)));
        }
        let (fill, stroke) = if quiet {
            (c.panel, Stroke::new(2.5, if j.state == State::NotRun { c.dim } else { c.border_strong() }))
        } else {
            (col, Stroke::new(3.0, c.bg))
        };
        if j.part.runs_process() {
            painter.circle(p, RADIUS, fill, stroke);
        } else {
            // A part is a square: it runs no process.
            let r = Rect::from_center_size(p, Vec2::splat(RADIUS * 2.0));
            painter.rect_filled(r, 3.0, fill);
            painter.rect_stroke(r, 3.0, stroke, StrokeKind::Inside);
        }
        if !quiet {
            let mark: String = j.state.mark().chars().take(1).collect();
            painter.text(p, egui::Align2::CENTER_CENTER, mark, egui::FontId::proportional(RADIUS * 1.2), c.on_accent());
        }
        let (anchor, align) = if vertical {
            (p + Vec2::new(RADIUS + 10.0, -2.0), egui::Align2::LEFT_CENTER)
        } else {
            (p + Vec2::new(0.0, RADIUS + 6.0), egui::Align2::CENTER_TOP)
        };
        let name = painter.text(anchor, align, j.label(), body.clone(), c.fg);
        if let Some(sub) = subtext(j) {
            let (below, a) = if vertical {
                (Pos2::new(name.left(), name.bottom()), egui::Align2::LEFT_TOP)
            } else {
                (Pos2::new(name.center().x, name.bottom()), egui::Align2::CENTER_TOP)
            };
            let tint = if j.state.tone() == Tone::Error { c.err } else { c.dim };
            painter.text(below, a, sub, small.clone(), tint);
        }
    }
    let summary: Vec<String> = run.jobs.iter().map(|j| format!("{} {}", j.state.mark(), j.label())).collect();
    ito_a11y::say(&resp, ito_a11y::Role::Label, &summary.join(", "));
    let hit = resp.interact_pointer_pos().filter(|_| resp.clicked())?;
    let near = (0..run.jobs.len()).find(|&i| Rect::from_center_size(at[i], Vec2::splat(RADIUS * 2.0 + 12.0)).contains(hit))?;
    Some(run.jobs[near].id.clone())
}

/// The states the picture shows, once each and in the order they first appear.
pub fn legend_states(run: &Run) -> Vec<State> {
    let mut out: Vec<State> = Vec::new();
    for j in &run.jobs {
        if !out.contains(&j.state) {
            out.push(j.state);
        }
    }
    out
}

/// A line under the picture saying what each mark in it means (`✓ 正常終了  ▶ 実行中`).
pub fn legend(ui: &mut egui::Ui, t: &ito_i18n::Table, c: &ito_theme::Colors, run: &Run) {
    ui.horizontal_wrapped(|ui| {
        for s in legend_states(run) {
            let text = format!("{} {}", s.mark(), crate::show::word(t, s));
            ui.label(egui::RichText::new(text).small().color(colour(c, s)));
        }
    });
}

/// What the picture says in words, one job per line.
pub fn words(t: &ito_i18n::Table, job: &JobRun) -> String {
    crate::show::job_line(t, job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::jobnet::{Job, Jobnet, Line};
    use koyomi_core::run::How;

    fn run() -> Run {
        let job = |id: &str, after: &[&str]| Job {
            id: id.into(),
            run: vec!["x".into()],
            after: after.iter().map(|s| Line::ok(s)).collect(),
            ..Default::default()
        };
        let net = Jobnet {
            name: None,
            jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["a"]), job("d", &["b", "c"])],
            when: Vec::new(),
            ..Default::default()
        };
        Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &net, ito_common::now::now_local().naive_local())
    }

    #[test]
    fn later_layers_sit_further_along() {
        let r = run();
        let h = places(&r, false, None);
        assert!(h[0].x < h[1].x && h[1].x == h[2].x && h[2].x < h[3].x);
        assert!(h[1].y < h[2].y);
        let v = places(&r, true, None);
        assert!(v[0].y < v[1].y && v[1].y == v[2].y && v[2].y < v[3].y);
        assert!(v[1].x < v[2].x);
    }

    #[test]
    fn the_editors_places_are_kept() {
        let r = run();
        let mut jobs: Vec<Job> = ["a", "b", "c", "d"].iter().map(|id| Job { id: id.to_string(), ..Default::default() }).collect();
        // Laid out in one row by the editor, out of the layers' order.
        for (i, j) in jobs.iter_mut().enumerate() {
            j.pos = Some((100.0 + i as f64 * 150.0, 40.0));
        }
        let at = places(&r, false, Some(&jobs));
        assert!(at.iter().all(|p| p.y == at[0].y));
        assert!(at[0].x < at[1].x && at[1].x < at[2].x && at[2].x < at[3].x);
        assert_eq!(at[0], places(&r, false, None)[0]);
        // A job the net no longer has: the run's own layers.
        assert_eq!(places(&r, false, Some(&jobs[..3])), places(&r, false, None));
    }

    #[test]
    fn the_pulses_move_towards_the_next_job() {
        let whole = |d: Vec<(f32, f32)>| d.into_iter().filter(|(a, b)| b - a > 9.99).collect::<Vec<_>>();
        let before = whole(dashes(100.0, 10.0, 14.0, 0.0));
        let after = whole(dashes(100.0, 10.0, 14.0, 5.0));
        assert!(!before.is_empty());
        // Every whole dash sits 5 further along the line, towards its end.
        for (x, y) in before.iter().zip(&after) {
            assert!((y.0 - x.0 - 5.0).abs() < 1e-3, "{before:?} -> {after:?}");
        }
        for (a, b) in dashes(100.0, 10.0, 14.0, 123.0) {
            assert!(0.0 <= a && a < b && b <= 100.0);
        }
    }

    #[test]
    fn a_line_between_rows_curves_and_one_in_a_row_is_straight() {
        let (a, b) = (Pos2::new(0.0, 0.0), Pos2::new(150.0, 64.0));
        let curve = path(a, b, false);
        assert!(curve.len() > 2);
        assert_eq!((curve[0], *curve.last().unwrap()), (a, b));
        // It leaves along the flow: the first step is mostly sideways.
        let first = curve[1] - curve[0];
        assert!(first.x > first.y.abs() * 2.0);
        assert_eq!(path(a, Pos2::new(150.0, 0.0), false).len(), 2);
        assert_eq!(path(a, Pos2::new(0.0, 150.0), true).len(), 2);
        let all = piece(&curve, 0.0, length(&curve));
        assert!((all.last().unwrap().x - b.x).abs() < 1e-3);
    }

    #[test]
    fn the_legend_lists_each_shown_state_once() {
        let mut r = run();
        for j in &mut r.jobs {
            j.state = State::Ok;
        }
        assert_eq!(legend_states(&r), [State::Ok]);
        r.jobs[1].state = State::Error;
        assert_eq!(legend_states(&r), [State::Ok, State::Error]);
    }

    #[test]
    fn the_subtext_says_when_it_ran_and_a_bad_exit() {
        let mut j = run().jobs[0].clone();
        assert_eq!(subtext(&j), None);
        let at = |h, m| chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(h, m, 0).unwrap();
        j.started = Some(at(9, 0));
        assert_eq!(subtext(&j).as_deref(), Some("9:00–"));
        j.ended = Some(at(9, 4));
        j.exit = Some(0);
        assert_eq!(subtext(&j).as_deref(), Some("9:00–9:04"));
        j.exit = Some(3);
        assert_eq!(subtext(&j).as_deref(), Some("9:00–9:04 exit 3"));
    }

    #[test]
    fn the_picture_fits_its_stations() {
        let r = run();
        let at = places(&r, false, None);
        let s = size(&at, false);
        for p in at {
            assert!(p.x + RADIUS < s.x && p.y + RADIUS < s.y);
        }
    }
}
