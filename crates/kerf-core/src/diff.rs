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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn different_layer_heights_are_reported_as_disjoint() {
        let b = A.replace(";Z:0.2", ";Z:0.4");
        let d = diff_gcode(A, &b, 200);
        assert!(!d.identical);
        // Two distinct Z layers, each present in only one file.
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.total_shared, 0);
    }
}
