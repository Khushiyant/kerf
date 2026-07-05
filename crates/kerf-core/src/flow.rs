//! Commanded-flow (E-axis) semantics — the extrusion-amount axis the geometric denotation is blind
//! to.
//!
//! A toolpath's [`lo::Toolpath::flow_e`] is the commanded filament advance (mm of E). Geometry-only
//! denotation ([`crate::denote`]) sees coverage, path count, and swept volume, but a program that
//! keeps the same geometry while corrupting E reads as identical to all of them. This module closes
//! that hole: it sums commanded flow ([`e_total`] / [`e_per_layer`]), checks it is conserved across a
//! transformation ([`e_conserved`]), and rasterizes it per cell ([`denote_lo_flow`]) so
//! over-/under-extrusion expressed as flow becomes a per-cell, comparable quantity.
//!
//! Only toolpaths that carry commanded flow participate; a toolpath with `flow_e == None` (e.g. from
//! geometry-only lowering) contributes nothing and is reported separately by [`flow_stats`].

use std::collections::BTreeMap;

use crate::denote::polyline_cells;
use crate::ir::lo;

#[cfg(feature = "serde")]
use serde::Serialize;

/// Total commanded filament (mm of E) over all extruding toolpaths that specify it.
pub fn e_total(program: &lo::Program) -> f64 {
    program
        .layers
        .iter()
        .flat_map(|l| &l.toolpaths)
        .filter(|t| t.kind.extrudes())
        .filter_map(|t| t.flow_e)
        .sum()
}

/// Commanded filament (mm of E) per layer, summed by layer height (Z), in ascending Z order.
pub fn e_per_layer(program: &lo::Program) -> Vec<(i64, f64)> {
    let mut by_z: BTreeMap<i64, f64> = BTreeMap::new();
    for layer in &program.layers {
        let e: f64 = layer
            .toolpaths
            .iter()
            .filter(|t| t.kind.extrudes())
            .filter_map(|t| t.flow_e)
            .sum();
        *by_z.entry(layer.z_um).or_insert(0.0) += e;
    }
    by_z.into_iter().collect()
}

/// The result of an E-conservation check between two programs.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct EConservation {
    /// Tolerance (mm of E) the totals were compared against.
    pub tolerance_mm: f64,
    pub total_a: f64,
    pub total_b: f64,
    /// Largest absolute per-layer (matched by Z) E difference.
    pub max_layer_abs_diff: f64,
    /// True iff the total and every matched layer agree within `tolerance_mm`.
    pub conserved: bool,
}

/// Compare the commanded E of two programs, matched by layer height. Conserved iff the total and every
/// layer agree within `tolerance_mm`. A transformation that alters commanded flow — e.g. doubling E on
/// one segment while leaving geometry unchanged — fails this even though the geometric denotation is
/// unmoved.
pub fn e_conserved_report(a: &lo::Program, b: &lo::Program, tolerance_mm: f64) -> EConservation {
    let tol = tolerance_mm.max(0.0);
    let (ea, eb): (BTreeMap<i64, f64>, BTreeMap<i64, f64>) = (
        e_per_layer(a).into_iter().collect(),
        e_per_layer(b).into_iter().collect(),
    );
    let mut max_layer_abs_diff = 0.0_f64;
    for z in ea.keys().chain(eb.keys()) {
        let d = (ea.get(z).copied().unwrap_or(0.0) - eb.get(z).copied().unwrap_or(0.0)).abs();
        max_layer_abs_diff = max_layer_abs_diff.max(d);
    }
    let total_a = e_total(a);
    let total_b = e_total(b);
    EConservation {
        tolerance_mm: tol,
        total_a,
        total_b,
        max_layer_abs_diff,
        conserved: (total_a - total_b).abs() <= tol && max_layer_abs_diff <= tol,
    }
}

/// Whether two programs conserve commanded E within `tolerance_mm` (see [`e_conserved_report`]).
pub fn e_conserved(a: &lo::Program, b: &lo::Program, tolerance_mm: f64) -> bool {
    e_conserved_report(a, b, tolerance_mm).conserved
}

/// Per-cell commanded flow (mm of E) for one layer.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerFlow {
    pub z_um: i64,
    pub resolution_um: i64,
    pub cells: BTreeMap<(i64, i64), f64>,
}

/// Per-cell commanded flow over a whole program — the flow analogue of [`crate::denote::Volume`].
/// Unlike the geometric denotations it moves when commanded E changes at fixed geometry, so it is the
/// denotation that catches flow-only over-/under-extrusion. f64 values; compare with [`Flow::approx_eq`].
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub layers: Vec<LayerFlow>,
}

impl Flow {
    /// Per-cell flow equality within `eps` mm of E.
    pub fn approx_eq(&self, other: &Flow, eps: f64) -> bool {
        self.layers.len() == other.layers.len()
            && self.layers.iter().zip(&other.layers).all(|(a, b)| {
                a.z_um == b.z_um
                    && a.resolution_um == b.resolution_um
                    && a.cells.len() == b.cells.len()
                    && a.cells
                        .iter()
                        .all(|(k, va)| b.cells.get(k).is_some_and(|vb| (va - vb).abs() <= eps))
            })
    }
}

/// Rasterize commanded flow (mm of E) per cell: each extruding toolpath's `flow_e` spread uniformly
/// over the cells it covers. Total is conserved per toolpath. Toolpaths without commanded flow are
/// skipped, so this is the *commanded*-flow denotation (empty when no toolpath specifies E).
pub fn denote_lo_flow(program: &lo::Program, resolution_um: i64) -> Flow {
    let r = resolution_um.max(1);
    let layers = program
        .layers
        .iter()
        .map(|layer| {
            let mut cells: BTreeMap<(i64, i64), f64> = BTreeMap::new();
            for tp in &layer.toolpaths {
                let Some(e) = tp.flow_e else { continue };
                if !tp.kind.extrudes() {
                    continue;
                }
                let covered = polyline_cells(&tp.path, tp.width_um, r);
                let n = covered.len();
                if n == 0 {
                    continue;
                }
                let per = e / n as f64;
                for c in covered {
                    *cells.entry(c).or_insert(0.0) += per;
                }
            }
            LayerFlow {
                z_um: layer.z_um,
                resolution_um: r,
                cells,
            }
        })
        .collect();
    Flow { layers }
}

/// Commanded-flow coverage and totals for a program.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct FlowStats {
    /// Total commanded filament (mm of E).
    pub total_e_mm: f64,
    /// Commanded E per layer, `(z_um, e_mm)` ascending by Z.
    pub per_layer: Vec<(i64, f64)>,
    /// Extruding toolpaths that carry commanded flow.
    pub toolpaths_with_flow: usize,
    /// Extruding toolpaths with no commanded flow (E is unknown for these).
    pub toolpaths_without_flow: usize,
}

/// Summarize a program's commanded flow, including how much of it is actually specified.
pub fn flow_stats(program: &lo::Program) -> FlowStats {
    let (mut with, mut without) = (0usize, 0usize);
    for t in program.layers.iter().flat_map(|l| &l.toolpaths) {
        if !t.kind.extrudes() {
            continue;
        }
        if t.flow_e.is_some() {
            with += 1;
        } else {
            without += 1;
        }
    }
    FlowStats {
        total_e_mm: e_total(program),
        per_layer: e_per_layer(program),
        toolpaths_with_flow: with,
        toolpaths_without_flow: without,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn seg(e: Option<f64>) -> Toolpath {
        Toolpath {
            kind: SegmentKind::Extrude(RegionKind::Perimeter),
            path: Polyline::new(vec![Point::new(0, 0), Point::new(20_000, 0)]),
            width_um: 400,
            flow_e: e,
        }
    }

    fn prog(e: Option<f64>) -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![seg(e)],
            }],
        }
    }

    #[test]
    fn doubling_e_on_a_segment_breaks_conservation_but_not_geometry() {
        let base = prog(Some(1.0));
        let doubled = prog(Some(2.0));
        // Geometry is identical: the geometric denotation cannot tell them apart.
        assert_eq!(
            crate::denote::denote_lo(&base, 200),
            crate::denote::denote_lo(&doubled, 200)
        );
        // But commanded flow is not conserved — this is exactly the hole E-semantics closes.
        assert!(!e_conserved(&base, &doubled, 1e-6));
        assert!(e_conserved(&base, &base, 1e-6));
    }

    #[test]
    fn flow_denotation_moves_when_commanded_e_changes_at_fixed_geometry() {
        let base = denote_lo_flow(&prog(Some(1.0)), 200);
        let doubled = denote_lo_flow(&prog(Some(2.0)), 200);
        assert!(!base.approx_eq(&doubled, 1e-9), "flow map must move with E");
        // Total flow is conserved across resolution.
        for r in [100, 200, 500] {
            let total: f64 = denote_lo_flow(&prog(Some(3.0)), r).layers[0]
                .cells
                .values()
                .sum();
            assert!((total - 3.0).abs() < 1e-9, "res {r}: {total} != 3.0");
        }
    }

    #[test]
    fn unspecified_flow_contributes_nothing_and_is_reported() {
        let s = flow_stats(&prog(None));
        assert_eq!(s.toolpaths_with_flow, 0);
        assert_eq!(s.toolpaths_without_flow, 1);
        assert_eq!(s.total_e_mm, 0.0);
        assert!(denote_lo_flow(&prog(None), 200).layers[0].cells.is_empty());
    }

    #[test]
    fn parsed_gcode_carries_commanded_flow() {
        // Real slicer output has E words; the parser must recover commanded flow.
        let g = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG1 X10 Y0 E0.5\nG1 X20 Y0 E0.5";
        let prog = crate::frontend::parse(g).program;
        assert!((e_total(&prog) - 1.0).abs() < 1e-6, "expected E total 1.0");
        assert_eq!(flow_stats(&prog).toolpaths_without_flow, 0);
    }
}
