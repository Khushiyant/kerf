//! Differential comparison of two G-code files by *what they physically deposit*.
//!
//! Parse both files into the IR, `denote` each to per-layer material occupancy, and compare —
//! matched by layer height (Z), not by line order. This answers a question a professional actually
//! asks — "do these two slicer versions / settings produce the same part?" — in terms of deposited
//! material, up to the raster resolution, rather than a meaningless text diff of the G-code.

use std::collections::{BTreeMap, BTreeSet};

use crate::denote::denote_lo;
use crate::frontend::parse;

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

/// The result of diffing two G-code files' deposited material.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct GcodeDiff {
    pub resolution_um: i64,
    /// True iff the two files deposit exactly the same material at every layer (up to resolution).
    /// NB: this is trivially true when both files are empty — check [`GcodeDiff::both_empty`] before
    /// treating `identical` as a meaningful "same part" verdict.
    pub identical: bool,
    /// True iff *neither* file yielded any deposited material (nothing to compare). A verdict of
    /// `identical` is only meaningful when this is false.
    pub both_empty: bool,
    pub total_only_in_a: usize,
    pub total_only_in_b: usize,
    pub total_shared: usize,
    pub layers: Vec<LayerDiff>,
}

impl GcodeDiff {
    /// Intersection-over-union of the deposited material: 1.0 = identical, 0.0 = disjoint. `None` if
    /// both files are empty (nothing to compare).
    pub fn iou(&self) -> Option<f64> {
        let union = self.total_shared + self.total_only_in_a + self.total_only_in_b;
        (union > 0).then(|| self.total_shared as f64 / union as f64)
    }
}

fn occupancy_by_z(gcode: &str, resolution_um: i64) -> BTreeMap<i64, BTreeSet<(i64, i64)>> {
    let program = parse(gcode).program;
    let mut by_z: BTreeMap<i64, BTreeSet<(i64, i64)>> = BTreeMap::new();
    for layer in denote_lo(&program, resolution_um).layers {
        by_z.entry(layer.z_um).or_default().extend(layer.cells);
    }
    by_z
}

/// Compare two G-code files by deposited material, at a chosen raster resolution.
pub fn diff_gcode(a: &str, b: &str, resolution_um: i64) -> GcodeDiff {
    let (a, b) = (
        occupancy_by_z(a, resolution_um),
        occupancy_by_z(b, resolution_um),
    );
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

    GcodeDiff {
        resolution_um,
        identical: ta == 0 && tb == 0,
        both_empty: ta == 0 && tb == 0 && ts == 0,
        total_only_in_a: ta,
        total_only_in_b: tb,
        total_shared: ts,
        layers,
    }
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
        assert_eq!(d.iou(), Some(1.0));
        assert_eq!(d.total_only_in_a, 0);
        assert_eq!(d.total_only_in_b, 0);
    }

    #[test]
    fn two_empty_files_are_flagged_both_empty_not_a_real_match() {
        // Neither file yields geometry; `identical` is trivially true, but `both_empty` marks it as
        // "nothing to compare" so a caller never reads a green "same part" verdict.
        let d = diff_gcode("M104 S200\nG28\n", ";just comments\n", 200);
        assert!(d.both_empty);
        assert_eq!(d.iou(), None);
    }

    #[test]
    fn a_shifted_copy_differs() {
        // Same shape moved 5 mm over: mostly disjoint material.
        let b = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\n;WIDTH:0.45\nG1 X5 Y50 E.1\nG1 X25 Y50 E.5\nG1 X25 Y70 E.5";
        let d = diff_gcode(A, b, 200);
        assert!(!d.identical);
        assert!(d.total_only_in_a > 0 && d.total_only_in_b > 0);
        assert!(d.iou().unwrap() < 0.5);
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
