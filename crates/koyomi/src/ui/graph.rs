//! A run as a line of stations: one circle per job, the lines between them
//! painted with the result that went through, flowing sideways or downwards.
//!
//! The picture never stands alone: the row of jobs under it says the same
//! in words (symbol + state + name), and that row is what the keyboard and
//! the screen reader get.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use koyomi_core::run::{JobRun, Run, State, Tone};

const STEP_ALONG: f32 = 150.0;
const STEP_ACROSS: f32 = 52.0;
const RADIUS: f32 = 9.0;
const MARGIN: f32 = 24.0;

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

/// Where each job sits, by its index in `run.jobs`.
pub fn places(run: &Run, vertical: bool) -> Vec<Pos2> {
    let mut at = vec![Pos2::ZERO; run.jobs.len()];
    for (l, layer) in run.layers().iter().enumerate() {
        for (i, &j) in layer.iter().enumerate() {
            let (along, across) = (MARGIN + l as f32 * STEP_ALONG, MARGIN + i as f32 * STEP_ACROSS);
            at[j] = if vertical { Pos2::new(across + 60.0, along) } else { Pos2::new(along, across) };
        }
    }
    at
}

/// The size the picture needs.
pub fn size(run: &Run, vertical: bool) -> Vec2 {
    let at = places(run, vertical);
    let (mut w, mut h) = (0f32, 0f32);
    for p in &at {
        w = w.max(p.x);
        h = h.max(p.y);
    }
    Vec2::new(w + MARGIN + if vertical { 120.0 } else { 100.0 }, h + MARGIN + if vertical { 12.0 } else { 28.0 })
}

/// Draws the run; the job clicked, when one was.
pub fn draw(ui: &mut egui::Ui, run: &Run, vertical: bool, selected: Option<&str>) -> Option<String> {
    let c = ito_theme::colors();
    let (resp, painter) = ui.allocate_painter(size(run, vertical), Sense::click());
    let origin = resp.rect.min.to_vec2();
    let at: Vec<Pos2> = places(run, vertical).into_iter().map(|p| p + origin).collect();
    let moving = run.jobs.iter().any(|j| j.state == State::Running);
    let phase = if moving { (ui.input(|i| i.time) * 40.0) as f32 } else { 0.0 };
    if moving {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    }
    // The lines first, painted with the predecessor's result (what came through it).
    for (i, j) in run.jobs.iter().enumerate() {
        for a in &j.after {
            let Some(p) = run.jobs.iter().position(|x| &x.id == a) else { continue };
            let (from, to) = (at[p], at[i]);
            let pred = &run.jobs[p];
            let col = if pred.state.is_final() || pred.state == State::Running { colour(&c, pred.state) } else { c.border };
            if pred.state == State::Running {
                dashed(&painter, from, to, col, phase);
            } else {
                painter.line_segment([from, to], Stroke::new(3.0, col));
            }
        }
    }
    for (i, j) in run.jobs.iter().enumerate() {
        let p = at[i];
        let on = selected == Some(j.id.as_str());
        let fill = if j.state == State::NotRun || j.state == State::Waiting { c.panel } else { colour(&c, j.state) };
        painter.circle(p, RADIUS, fill, Stroke::new(if on { 3.0 } else { 2.0 }, if on { c.fg } else { colour(&c, j.state) }));
        let text = format!("{} {}", j.state.mark(), j.label());
        let (anchor, align) = if vertical {
            (p + Vec2::new(RADIUS + 8.0, 0.0), egui::Align2::LEFT_CENTER)
        } else {
            (p + Vec2::new(0.0, RADIUS + 6.0), egui::Align2::CENTER_TOP)
        };
        painter.text(anchor, align, text, egui::TextStyle::Small.resolve(ui.style()), c.fg);
    }
    let summary: Vec<String> = run.jobs.iter().map(|j| format!("{} {}", j.state.mark(), j.label())).collect();
    ito_a11y::say(&resp, ito_a11y::Role::Label, &summary.join(", "));
    let hit = resp.interact_pointer_pos().filter(|_| resp.clicked())?;
    let near = (0..run.jobs.len()).find(|&i| Rect::from_center_size(at[i], Vec2::splat(RADIUS * 2.0 + 12.0)).contains(hit))?;
    Some(run.jobs[near].id.clone())
}

fn dashed(painter: &egui::Painter, from: Pos2, to: Pos2, col: Color32, phase: f32) {
    let d = to - from;
    let len = d.length();
    if len <= 0.0 {
        return;
    }
    let dir = d / len;
    let (dash, gap) = (8.0, 6.0);
    let mut s = -(phase % (dash + gap));
    while s < len {
        let (a, b) = (s.max(0.0), (s + dash).min(len));
        if b > a {
            painter.line_segment([from + dir * a, from + dir * b], Stroke::new(3.0, col));
        }
        s += dash + gap;
    }
}

/// What the picture says in words, one job per line.
pub fn words(t: &ito_i18n::Table, job: &JobRun) -> String {
    crate::show::job_line(t, job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::jobnet::{Job, Jobnet};
    use koyomi_core::run::How;

    fn run() -> Run {
        let job = |id: &str, after: &[&str]| Job {
            id: id.into(),
            name: None,
            run: vec!["x".into()],
            dir: None,
            after: after.iter().map(|s| s.to_string()).collect(),
            pos: None,
        };
        let net = Jobnet { name: None, jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["a"]), job("d", &["b", "c"])] };
        Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &net, chrono::Local::now().naive_local())
    }

    #[test]
    fn later_layers_sit_further_along() {
        let r = run();
        let h = places(&r, false);
        assert!(h[0].x < h[1].x && h[1].x == h[2].x && h[2].x < h[3].x);
        assert!(h[1].y < h[2].y);
        let v = places(&r, true);
        assert!(v[0].y < v[1].y && v[1].y == v[2].y && v[2].y < v[3].y);
        assert!(v[1].x < v[2].x);
    }

    #[test]
    fn the_picture_fits_its_stations() {
        let r = run();
        let s = size(&r, false);
        for p in places(&r, false) {
            assert!(p.x + RADIUS < s.x && p.y + RADIUS < s.y);
        }
    }
}
