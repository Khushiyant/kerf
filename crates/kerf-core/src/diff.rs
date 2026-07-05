//! Differential comparison of two programs (or G-code files) by what they physically deposit.
//!
//! Each side is denoted to per-layer material occupancy, then compared matched by layer height (Z),
//! not by line order, up to the raster resolution — yielding a scalar IoU similarity and a per-layer
//! breakdown.

use std::collections::{BTreeMap, BTreeSet};

use crate::denote::denote_lo;
use crate::frontend::parse;
use crate::ir::lo;

#[cfg(feature = "serde")]
use serde::Serialize;

/// Per-layer (matched by Z) material difference between two files.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct LayerDiff {
    pub z_um: i64,
    /// Occupied cells present only in A / only in B / in both, at this layer height.
    pub only_in_a: usize,
    pub only_in_b: usize,
    pub shared: usize,
}

/// The result of diffing two programs' (or files') deposited material.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct GcodeDiff {
    pub resolution_um: i64,
    /// True iff the two inputs deposit the same material at every layer (up to resolution). Trivially
    /// true when both are empty; check [`GcodeDiff::both_empty`] first.
    pub identical: bool,
    /// True iff neither input yielded deposited material. `identical` is only meaningful when false.
    pub both_empty: bool,
    /// Intersection-over-union of the deposited material: 1.0 = identical, 0.0 = disjoint; `None` when
    /// both inputs are empty (nothing to compare). A single scalar similarity score.
    pub iou: Option<f64>,
    pub total_only_in_a: usize,
    pub total_only_in_b: usize,
    pub total_shared: usize,
    pub layers: Vec<LayerDiff>,
}

/// Group a program's occupied cells by layer height at a chosen resolution.
fn occupancy_by_z(
    program: &lo::Program,
    resolution_um: i64,
) -> BTreeMap<i64, BTreeSet<(i64, i64)>> {
    let mut by_z: BTreeMap<i64, BTreeSet<(i64, i64)>> = BTreeMap::new();
    for layer in denote_lo(program, resolution_um).layers {
        by_z.entry(layer.z_um).or_default().extend(layer.cells);
    }
    by_z
}

/// Compare two occupancy-by-layer maps into a material diff.
fn diff_occupancy(
    a: BTreeMap<i64, BTreeSet<(i64, i64)>>,
    b: BTreeMap<i64, BTreeSet<(i64, i64)>>,
    resolution_um: i64,
) -> GcodeDiff {
    let heights: BTreeSet<i64> = a.keys().chain(b.keys()).copied().collect();
    let empty = BTreeSet::new();
    let (mut ta, mut tb, mut ts) = (0usize, 0usize, 0usize);
    let mut layers = Vec::new();
    for z in heights {
        let ca = a.get(&z).unwrap_or(&empty);
        let cb = b.get(&z).unwrap_or(&empty);
        let only_in_a = ca.difference(cb).count();
        let only_in_b = cb.difference(ca).count();
        let shared = ca.intersection(cb).count();
        ta += only_in_a;
        tb += only_in_b;
        ts += shared;
        layers.push(LayerDiff {
            z_um: z,
            only_in_a,
            only_in_b,
            shared,
        });
    }
    let union = ts + ta + tb;
    GcodeDiff {
        resolution_um,
        identical: ta == 0 && tb == 0,
        both_empty: union == 0,
        iou: (union > 0).then(|| ts as f64 / union as f64),
        total_only_in_a: ta,
        total_only_in_b: tb,
        total_shared: ts,
        layers,
    }
}

/// Compare two low-level programs by the material they deposit, at a chosen raster resolution.
/// Works on any move plans — parsed, lowered, optimizer output, or generated — and yields a scalar
/// [`GcodeDiff::iou`] similarity plus a per-layer breakdown.
pub fn diff_programs(a: &lo::Program, b: &lo::Program, resolution_um: i64) -> GcodeDiff {
    diff_occupancy(
        occupancy_by_z(a, resolution_um),
        occupancy_by_z(b, resolution_um),
        resolution_um,
    )
}

/// Compare two G-code files by deposited material, at a chosen raster resolution.
pub fn diff_gcode(a: &str, b: &str, resolution_um: i64) -> GcodeDiff {
    diff_programs(&parse(a).program, &parse(b).program, resolution_um)
}

// ---------------------------------------------------------------------------- graded (distance) diff
//
// Where `diff` counts HOW MANY cells disagree, `graded_diff` measures HOW FAR the disagreeing cells
// are from the other program — a smooth signal (a near-miss scores close to zero, where IoU is flat
// at 0). It is a directed-Hausdorff-style statistic over the per-layer occupancy grids, computed with
// an exact squared Euclidean distance transform. Squared distances stay exact integers (grid units²);
// only the final micron conversion uses f64, consistent with denote's "f64 only in the checker" rule.

/// Cap on a layer's bounding-box grid size (cells) for the distance transform; larger layers report
/// their differing cells as `unmatched` rather than allocating an oversized grid.
const GRADED_GRID_CAP: i128 = 9_000_000;

/// Graded-distance stats for one layer (microns).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct LayerGradedDiff {
    pub z_um: i64,
    pub mean_um: f64,
    pub p95_um: f64,
    pub max_um: f64,
    /// Cells in the symmetric difference that had a finite distance to the other program.
    pub differing_cells: u64,
    /// Differing cells with no counterpart to measure to (the other program had no material in this
    /// layer, or the layer's grid exceeded the size cap).
    pub unmatched_cells: u64,
}

/// Graded (distance-based) difference between two programs' deposited material.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct GradedDiff {
    pub resolution_um: i64,
    /// Mean / 95th-percentile / max nearest-cell distance (microns) over all differing cells.
    pub mean_um: f64,
    pub p95_um: f64,
    pub max_um: f64,
    pub differing_cells: u64,
    pub unmatched_cells: u64,
    pub per_layer: Vec<LayerGradedDiff>,
}

/// Exact 1-D squared distance transform (Felzenszwalb & Huttenlocher). `f[q]` is the seed cost at q
/// (0 at a feature, a large value elsewhere); returns `min_p (q - p)^2 + f[p]` per q — integer output.
fn edt_1d(f: &[i64]) -> Vec<i64> {
    let n = f.len();
    let mut d = vec![0i64; n];
    if n == 0 {
        return d;
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut s;
        loop {
            let p = v[k];
            s = ((f[q] + (q * q) as i64) - (f[p] + (p * p) as i64)) as f64
                / (2 * (q as i64 - p as i64)) as f64;
            if s <= z[k] {
                k -= 1; // z[0] = -inf guarantees this stops at k = 0
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, slot) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let dq = q as i64 - v[k] as i64;
        *slot = dq * dq + f[v[k]];
    }
    d
}

/// Exact 2-D squared EDT over a `w x h` grid: distance² to the nearest feature cell, per cell.
fn edt_2d(w: usize, h: usize, feats: &BTreeSet<(i64, i64)>, min_i: i64, min_j: i64) -> Vec<i64> {
    let inf = 4 * ((w as i64) * (w as i64) + (h as i64) * (h as i64)) + 1;
    let mut grid = vec![inf; w * h];
    for &(i, j) in feats {
        grid[(j - min_j) as usize * w + (i - min_i) as usize] = 0;
    }
    let mut buf = vec![0i64; h.max(w)];
    for x in 0..w {
        for y in 0..h {
            buf[y] = grid[y * w + x];
        }
        let d = edt_1d(&buf[..h]);
        for y in 0..h {
            grid[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        for x in 0..w {
            buf[x] = grid[y * w + x];
        }
        let d = edt_1d(&buf[..w]);
        for x in 0..w {
            grid[y * w + x] = d[x];
        }
    }
    grid
}

/// Distances (microns) of every symmetric-difference cell to the nearest cell of the other set, both
/// directions. `None` if the bounding-box grid exceeds [`GRADED_GRID_CAP`].
fn layer_distances(
    ca: &BTreeSet<(i64, i64)>,
    cb: &BTreeSet<(i64, i64)>,
    r: i64,
) -> Option<Vec<f64>> {
    let (mut min_i, mut min_j, mut max_i, mut max_j) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for &(i, j) in ca.iter().chain(cb.iter()) {
        min_i = min_i.min(i);
        max_i = max_i.max(i);
        min_j = min_j.min(j);
        max_j = max_j.max(j);
    }
    let (wi, hi) = (
        max_i as i128 - min_i as i128 + 1,
        max_j as i128 - min_j as i128 + 1,
    );
    if wi * hi > GRADED_GRID_CAP {
        return None;
    }
    let (w, h) = (wi as usize, hi as usize);
    let idx = |i: i64, j: i64| (j - min_j) as usize * w + (i - min_i) as usize;
    let dt_b = edt_2d(w, h, cb, min_i, min_j);
    let dt_a = edt_2d(w, h, ca, min_i, min_j);
    let rf = r as f64;
    let mut out = Vec::new();
    for &c in ca.iter() {
        if !cb.contains(&c) {
            out.push((dt_b[idx(c.0, c.1)] as f64).sqrt() * rf);
        }
    }
    for &c in cb.iter() {
        if !ca.contains(&c) {
            out.push((dt_a[idx(c.0, c.1)] as f64).sqrt() * rf);
        }
    }
    Some(out)
}

fn dist_stats(d: &[f64]) -> (f64, f64, f64) {
    if d.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let n = d.len();
    let mean = d.iter().sum::<f64>() / n as f64;
    let mut s = d.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = s[(((0.95 * n as f64).ceil() as usize).max(1) - 1).min(n - 1)];
    (mean, p95, s[n - 1])
}

/// Graded (distance-based) difference of two low-level programs: for each cell one program deposits
/// and the other does not, the Euclidean distance (microns) to the nearest cell of the other, summed
/// into mean / p95 / max per layer and overall. A smooth similarity signal that, unlike IoU, still
/// discriminates two disjoint-but-close prints — useful as an RL reward and the basis for
/// rotation-aware comparison.
pub fn graded_diff_programs(a: &lo::Program, b: &lo::Program, resolution_um: i64) -> GradedDiff {
    let r = resolution_um.max(1);
    let (am, bm) = (occupancy_by_z(a, r), occupancy_by_z(b, r));
    let zs: BTreeSet<i64> = am.keys().chain(bm.keys()).copied().collect();
    let empty = BTreeSet::new();
    let mut per_layer = Vec::new();
    let mut all: Vec<f64> = Vec::new();
    let (mut total_diff, mut total_unmatched) = (0u64, 0u64);
    for z in zs {
        let (ca, cb) = (am.get(&z).unwrap_or(&empty), bm.get(&z).unwrap_or(&empty));
        let (mut ld, mut unmatched) = (Vec::new(), 0u64);
        if ca.is_empty() || cb.is_empty() {
            unmatched = ca.symmetric_difference(cb).count() as u64;
        } else {
            match layer_distances(ca, cb, r) {
                Some(d) => ld = d,
                None => unmatched = ca.symmetric_difference(cb).count() as u64,
            }
        }
        let (mean, p95, max) = dist_stats(&ld);
        total_diff += ld.len() as u64;
        total_unmatched += unmatched;
        all.extend_from_slice(&ld);
        per_layer.push(LayerGradedDiff {
            z_um: z,
            mean_um: mean,
            p95_um: p95,
            max_um: max,
            differing_cells: ld.len() as u64,
            unmatched_cells: unmatched,
        });
    }
    let (mean, p95, max) = dist_stats(&all);
    GradedDiff {
        resolution_um: r,
        mean_um: mean,
        p95_um: p95,
        max_um: max,
        differing_cells: total_diff,
        unmatched_cells: total_unmatched,
        per_layer,
    }
}

/// Graded-distance difference of two G-code files (see [`graded_diff_programs`]).
pub fn graded_diff_gcode(a: &str, b: &str, resolution_um: i64) -> GradedDiff {
    graded_diff_programs(&parse(a).program, &parse(b).program, resolution_um)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    const A: &str = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\n;WIDTH:0.45\nG1 X0 Y0 E.1\nG1 X20 Y0 E.5\nG1 X20 Y20 E.5";

    #[test]
    fn a_file_is_identical_to_itself() {
        let d = diff_gcode(A, A, 200);
        assert!(d.identical);
        assert!(!d.both_empty);
        assert_eq!(d.iou, Some(1.0));
        assert_eq!(d.total_only_in_a, 0);
        assert_eq!(d.total_only_in_b, 0);
    }

    #[test]
    fn two_empty_files_are_flagged_both_empty_not_a_real_match() {
        let d = diff_gcode("M104 S200\nG28\n", ";just comments\n", 200);
        assert!(d.both_empty);
        assert_eq!(d.iou, None);
    }

    #[test]
    fn a_shifted_copy_differs() {
        let b = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\n;WIDTH:0.45\nG1 X5 Y50 E.1\nG1 X25 Y50 E.5\nG1 X25 Y70 E.5";
        let d = diff_gcode(A, b, 200);
        assert!(!d.identical);
        assert!(d.total_only_in_a > 0 && d.total_only_in_b > 0);
        assert!(d.iou.unwrap() < 0.5);
    }

    #[test]
    fn diff_programs_matches_diff_gcode_and_scores_a_pass() {
        use crate::pass::{Pass, TravelOrder};
        let prog = parse(A).program;
        // A program is identical to itself, and a reorder pass preserves material (IoU 1.0).
        assert_eq!(diff_programs(&prog, &prog, 200).iou, Some(1.0));
        let reordered = TravelOrder::default().run(prog.clone());
        assert_eq!(diff_programs(&prog, &reordered, 200).iou, Some(1.0));
        // diff_programs on the parsed programs agrees with diff_gcode on the strings.
        assert_eq!(diff_programs(&prog, &prog, 200), diff_gcode(A, A, 200));
    }

    #[test]
    fn edt_computes_exact_squared_distances() {
        let f: Vec<i64> = (0..5).map(|x| if x == 0 { 0 } else { 999 }).collect();
        assert_eq!(edt_1d(&f), vec![0, 1, 4, 9, 16]); // squared distance to x=0
        let feats: BTreeSet<(i64, i64)> = [(0i64, 0i64)].into_iter().collect();
        let dt = edt_2d(3, 3, &feats, 0, 0);
        assert_eq!(dt[0], 0); // the feature itself
        assert_eq!(dt[2 * 3 + 2], 8); // (2,2): 2^2 + 2^2
    }

    fn h_line(y: i64, width_um: i64) -> lo::Program {
        lo::Program {
            layers: vec![lo::Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Extrude(RegionKind::Infill),
                    path: Polyline::new(vec![Point::new(0, y), Point::new(20_000, y)]),
                    width_um,
                }],
            }],
        }
    }

    #[test]
    fn edt_matches_bruteforce_over_random_grids() {
        // The distance transform must be EXACT — a wrong distance is a wrong reward. Differential-test
        // edt_2d against the naive O(cells^2) nearest-feature distance over many random grids.
        let mut s: u64 = 0x9e3779b97f4a7c15; // xorshift PRNG, reproducible, no deps
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..3000 {
            let (w, h) = ((rng() % 9 + 1) as i64, (rng() % 9 + 1) as i64);
            let mut feats = BTreeSet::new();
            for i in 0..w {
                for j in 0..h {
                    if rng() % 3 == 0 {
                        feats.insert((i, j));
                    }
                }
            }
            if feats.is_empty() {
                continue;
            }
            let dt = edt_2d(w as usize, h as usize, &feats, 0, 0);
            for i in 0..w {
                for j in 0..h {
                    let brute = feats
                        .iter()
                        .map(|&(fi, fj)| (i - fi) * (i - fi) + (j - fj) * (j - fj))
                        .min()
                        .unwrap();
                    assert_eq!(
                        dt[j as usize * w as usize + i as usize],
                        brute,
                        "{w}x{h} at ({i},{j})"
                    );
                }
            }
        }
    }

    #[test]
    fn graded_distance_is_zero_when_identical_and_grows_with_offset() {
        let a = h_line(0, 400);
        assert_eq!(graded_diff_programs(&a, &a, 200).max_um, 0.0);
        let near = graded_diff_programs(&a, &h_line(600, 400), 200);
        let far = graded_diff_programs(&a, &h_line(6000, 400), 200);
        assert!(near.mean_um > 0.0);
        assert!(
            far.mean_um > near.mean_um,
            "graded distance must grow with offset"
        );
    }

    #[test]
    fn graded_distance_discriminates_where_iou_is_flat_at_zero() {
        // Thin lines pushed fully apart: IoU is 0 for BOTH offsets (no gradient), but graded distance
        // still ranks the near miss below the far one — the property RL needs.
        let a = h_line(0, 100);
        let near = graded_diff_programs(&a, &h_line(2000, 100), 200);
        let far = graded_diff_programs(&a, &h_line(40_000, 100), 200);
        assert_eq!(
            diff_programs(&a, &h_line(2000, 100), 200)
                .iou
                .unwrap_or(0.0),
            0.0
        );
        assert_eq!(
            diff_programs(&a, &h_line(40_000, 100), 200)
                .iou
                .unwrap_or(0.0),
            0.0
        );
        assert!(far.mean_um > near.mean_um);
    }

    #[test]
    fn a_layer_present_in_only_one_program_is_unmatched() {
        let a = h_line(0, 400);
        let mut b = h_line(0, 400);
        b.layers.push(lo::Layer {
            z_um: 400,
            toolpaths: h_line(0, 400).layers[0].toolpaths.clone(),
        });
        let g = graded_diff_programs(&a, &b, 200);
        assert!(
            g.unmatched_cells > 0,
            "the z=400 layer has no counterpart in A"
        );
    }

    #[test]
    fn different_layer_heights_are_reported_as_disjoint() {
        let b = A.replace(";Z:0.2", ";Z:0.4");
        let d = diff_gcode(A, &b, 200);
        assert!(!d.identical);
        // Two distinct Z layers, each present in only one file.
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.total_shared, 0);
    }
}
