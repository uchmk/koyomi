//! 整列: where the editor puts the parts of a net when a person asks for it
//! (and where a part with no `pos` goes).
//!
//! Layers by the longest path from a start (`run::layers`), then the rows of
//! each layer ordered by the mean row of the parts before them, and again by
//! the parts after them, a few times over: the lines cross as little as this
//! simple sweep gets them to. The positions are horizontal (along = x);
//! the vertical flow swaps the axes when it draws.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::jobnet::{Job, Line};
use crate::run;

/// The distance between layers (along the flow).
pub const STEP_ALONG: f64 = 150.0;
/// The distance between rows (across the flow).
pub const STEP_ACROSS: f64 = 52.0;

/// For each job, its layer and its row in that layer.
pub fn places(jobs: &[Job]) -> Vec<(usize, usize)> {
    let refs: Vec<(&str, &[Line])> = jobs.iter().map(|j| (j.id.as_str(), j.after.as_slice())).collect();
    let mut layers = run::layers(&refs);
    let index: BTreeMap<&str, usize> = refs.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
    let n = jobs.len();
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, (_, after)) in refs.iter().enumerate() {
        for l in after.iter() {
            if let Some(&p) = index.get(l.job.as_str()) {
                if p != i && !preds[i].contains(&p) {
                    preds[i].push(p);
                    succs[p].push(i);
                }
            }
        }
    }
    let mut row = vec![0.0f64; n];
    for layer in &layers {
        for (r, &i) in layer.iter().enumerate() {
            row[i] = r as f64;
        }
    }
    for _ in 0..4 {
        for layer in layers.iter_mut().skip(1) {
            sweep(layer, &preds, &mut row);
        }
        for layer in layers.iter_mut().rev().skip(1) {
            sweep(layer, &succs, &mut row);
        }
    }
    let mut out = vec![(0, 0); n];
    for (l, layer) in layers.iter().enumerate() {
        for (r, &i) in layer.iter().enumerate() {
            out[i] = (l, r);
        }
    }
    out
}

/// Orders `layer` by the mean row of each part's `neighbours`, and writes the new rows back.
fn sweep(layer: &mut Vec<usize>, neighbours: &[Vec<usize>], row: &mut [f64]) {
    let mut keyed: Vec<(usize, f64)> = layer
        .iter()
        .map(|&i| {
            let ns = &neighbours[i];
            let mean = if ns.is_empty() { row[i] } else { ns.iter().map(|&p| row[p]).sum::<f64>() / ns.len() as f64 };
            (i, mean)
        })
        .collect();
    keyed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal).then(row[a.0].partial_cmp(&row[b.0]).unwrap_or(Ordering::Equal)));
    for (r, (i, _)) in keyed.iter().enumerate() {
        row[*i] = r as f64;
    }
    *layer = keyed.into_iter().map(|(i, _)| i).collect();
}

/// Puts every job where [`places`] says, `STEP_ALONG` apart along the flow and `STEP_ACROSS` across it.
pub fn align(jobs: &mut [Job]) {
    let placed = places(jobs);
    for (job, (layer, row)) in jobs.iter_mut().zip(placed) {
        job.pos = Some((layer as f64 * STEP_ALONG, row as f64 * STEP_ACROSS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, after: &[&str]) -> Job {
        Job { id: id.into(), run: vec!["x".into()], after: after.iter().map(|s| Line::ok(s)).collect(), ..Default::default() }
    }

    #[test]
    fn a_diamond_lays_out_in_three_layers() {
        let mut jobs = vec![job("a", &[]), job("b", &["a"]), job("c", &["a"]), job("d", &["b", "c"])];
        assert_eq!(places(&jobs), [(0, 0), (1, 0), (1, 1), (2, 0)]);
        align(&mut jobs);
        assert_eq!(jobs[2].pos, Some((STEP_ALONG, STEP_ACROSS)));
        assert_eq!(jobs[3].pos, Some((2.0 * STEP_ALONG, 0.0)));
    }

    #[test]
    fn the_rows_follow_the_parts_before_them() {
        // In file order y (after b) comes before x (after a): the sweep uncrosses them.
        let jobs = vec![job("a", &[]), job("b", &[]), job("y", &["b"]), job("x", &["a"])];
        assert_eq!(places(&jobs), [(0, 0), (0, 1), (1, 1), (1, 0)]);
    }

    #[test]
    fn a_cycle_and_a_missing_part_still_get_a_place() {
        let jobs = vec![job("a", &["b"]), job("b", &["a"]), job("c", &["nope"])];
        let p = places(&jobs);
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|(l, _)| *l < 3));
    }
}
