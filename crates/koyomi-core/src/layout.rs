//! 整列: where the editor puts the parts of a net when a person asks for it
//! (and where a part with no `pos` goes).
//!
//! Layers by the longest path from a start (`run::layers`), then the rows of
//! each layer ordered by the mean row of the parts before them, and again by
//! the parts after them, a few times over: the lines cross as little as this
//! simple sweep gets them to. Each group of parts joined by lines is its own
//! block of rows, one block below the other in the order of their first part
//! in the file, so two flows never share rows. A part with no line before or after it is not
//! stacked in layer 0: the loose parts stand in one row along the flow, below
//! the connected ones, in the order they stood (then the file's). The
//! positions are horizontal (along = x); the vertical flow swaps the axes when
//! it draws.

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
    // The loose parts leave the layers and get their own row at the end.
    let loose: Vec<usize> = (0..n).filter(|&i| preds[i].is_empty() && succs[i].is_empty()).collect();
    for layer in &mut layers {
        layer.retain(|i| !loose.contains(i));
    }
    let group = groups(&preds, &succs);
    let mut row = vec![0.0f64; n];
    stack(&mut layers, &group, &mut row);
    for _ in 0..4 {
        for layer in layers.iter_mut().skip(1) {
            sweep(layer, &preds, &mut row);
        }
        for layer in layers.iter_mut().rev().skip(1) {
            sweep(layer, &succs, &mut row);
        }
    }
    let below = stack(&mut layers, &group, &mut row);
    let mut out = vec![(0, 0); n];
    for (l, layer) in layers.iter().enumerate() {
        for &i in layer {
            out[i] = (l, row[i] as usize);
        }
    }
    let mut loose = loose;
    // Stable: the file's order among parts at the same place, and after the placed ones.
    loose.sort_by(|&a, &b| {
        let along = |i: usize| jobs[i].pos.map(|p| p.0);
        match (along(a), along(b)) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    });
    for (l, &i) in loose.iter().enumerate() {
        out[i] = (l, below);
    }
    out
}

/// For each part, the first (in file order) of the parts joined to it by lines.
fn groups(preds: &[Vec<usize>], succs: &[Vec<usize>]) -> Vec<usize> {
    let n = preds.len();
    let mut group = vec![usize::MAX; n];
    for start in 0..n {
        if group[start] != usize::MAX {
            continue;
        }
        group[start] = start;
        let mut todo = vec![start];
        while let Some(i) = todo.pop() {
            for &j in preds[i].iter().chain(&succs[i]) {
                if group[j] == usize::MAX {
                    group[j] = start;
                    todo.push(j);
                }
            }
        }
    }
    group
}

/// Gives each group its own block of rows (in the order of `group`), keeping
/// the order of the parts within a layer; returns the rows used.
fn stack(layers: &mut [Vec<usize>], group: &[usize], row: &mut [f64]) -> usize {
    let mut height: BTreeMap<usize, usize> = BTreeMap::new();
    for layer in layers.iter() {
        let mut count: BTreeMap<usize, usize> = BTreeMap::new();
        for &i in layer {
            *count.entry(group[i]).or_default() += 1;
        }
        for (g, c) in count {
            let h = height.entry(g).or_default();
            *h = (*h).max(c);
        }
    }
    let mut top: BTreeMap<usize, usize> = BTreeMap::new();
    let mut used = 0;
    for (g, h) in height {
        top.insert(g, used);
        used += h;
    }
    for layer in layers.iter_mut() {
        layer.sort_by_key(|&i| top[&group[i]]);
        let mut next = top.clone();
        for &i in layer.iter() {
            let r = next.get_mut(&group[i]).expect("every group has a top");
            row[i] = *r as f64;
            *r += 1;
        }
    }
    used
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
    fn loose_parts_stand_in_a_row_along_the_flow() {
        let jobs = vec![job("a", &[]), job("b", &[]), job("c", &[])];
        assert_eq!(places(&jobs), [(0, 0), (1, 0), (2, 0)]);
    }

    #[test]
    fn loose_parts_go_below_the_connected_ones_in_the_order_they_stood() {
        let mut jobs = vec![job("a", &[]), job("p", &[]), job("b", &["a"]), job("c", &["a"]), job("q", &[]), job("d", &["b", "c"])];
        jobs[1].pos = Some((300.0, 0.0));
        jobs[4].pos = Some((10.0, 400.0));
        let p = places(&jobs);
        assert_eq!([p[0], p[2], p[3], p[5]], [(0, 0), (1, 0), (1, 1), (2, 0)], "the diamond is as before");
        assert_eq!((p[4], p[1]), ((0, 2), (1, 2)), "q stood left of p");
        align(&mut jobs);
        assert_eq!(jobs[1].pos, Some((STEP_ALONG, 2.0 * STEP_ACROSS)));
    }

    #[test]
    fn each_group_of_joined_parts_gets_its_own_rows() {
        // a → c, a → e and b → d: b and d go below the whole a group, not beside e.
        let jobs = vec![job("a", &[]), job("b", &[]), job("c", &["a"]), job("d", &["b"]), job("e", &["a"]), job("z", &[])];
        assert_eq!(places(&jobs), [(0, 0), (0, 2), (1, 0), (1, 2), (1, 1), (0, 3)]);
    }

    #[test]
    fn a_cycle_and_a_missing_part_still_get_a_place() {
        let jobs = vec![job("a", &["b"]), job("b", &["a"]), job("c", &["nope"])];
        let p = places(&jobs);
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|(l, _)| *l < 3));
    }
}
