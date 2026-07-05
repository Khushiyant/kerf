//! Versatile geometric analyses over a move plan, built on the denotation: efficiency stats,
//! over-deposition, and a heuristic travel-vs-material check. Useful for quality metrics, comparing
//! or scoring optimizer/agent output, and dashboards alike — not tied to any one consumer.

use crate::denote::{denote_lo, denote_lo_deposit, denote_lo_volume, polyline_cells};
use crate::ir::lo::{self, SegmentKind};
use crate::ir::Point;

#[cfg(feature = "serde")]
use serde::Serialize;

/// Size and efficiency stats for a move plan.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ProgramStats {
    pub layers: usize,
    pub extruding_toolpaths: usize,
    pub travel_toolpaths: usize,
    /// Total non-printing travel distance (microns) — an efficiency / print-time proxy.
    pub travel_distance_um: f64,
}

/// Compute size and efficiency stats for a program.
pub fn program_stats(program: &lo::Program) -> ProgramStats {
    let travel_toolpaths = program
        .layers
        .iter()
        .flat_map(|l| &l.toolpaths)
        .filter(|t| t.kind == SegmentKind::Travel)
        .count();
    ProgramStats {
        layers: program.layers.len(),
        extruding_toolpaths: program.extrusion_move_count(),
        travel_toolpaths,
        travel_distance_um: program.travel_distance_um(),
    }
}

/// Over-deposition stats from the per-cell deposition count (denote⁺).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct DepositStats {
    pub resolution_um: i64,
    /// Distinct occupied cells (== occupancy size).
    pub total_cells: u64,
    /// Cells covered by more than one path — potential over-extrusion.
    pub over_deposited_cells: u64,
    /// Sum over cells of (count - 1): total "extra" depositions beyond a single pass. A graded
    /// over-deposition magnitude, not just a count of offending cells.
    pub redeposited_cells: u64,
    /// The largest number of distinct paths over any single cell.
    pub max_multiplicity: u32,
}

/// Compute over-deposition stats at a resolution. Note this counts *paths per cell*, not filament
/// volume (see the `denote⁺` limitations).
pub fn deposit_stats(program: &lo::Program, resolution_um: i64) -> DepositStats {
    let dep = denote_lo_deposit(program, resolution_um);
    let (mut total, mut over, mut redep, mut maxm) = (0u64, 0u64, 0u64, 0u32);
    for layer in &dep.layers {
        for &c in layer.cells.values() {
            total += 1;
            if c > 1 {
                over += 1;
                redep += (c - 1) as u64;
            }
            maxm = maxm.max(c);
        }
    }
    DepositStats {
        resolution_um: resolution_um.max(1),
        total_cells: total,
        over_deposited_cells: over,
        redeposited_cells: redep,
        max_multiplicity: maxm,
    }
}

/// Travel-vs-material crossing events in a layer (a nozzle-drag / stringing proxy).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct LayerCollisions {
    pub z_um: i64,
    /// Number of (travel, deposited-cell) crossing events: a cell dragged over by N distinct travels
    /// counts N (each drag is a separate stringing event), deduplicated within a single travel.
    pub crossed_cells: u64,
}

/// Heuristic, report-only travel-vs-material check across a program.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct TravelCollisions {
    pub resolution_um: i64,
    pub total: u64,
    pub per_layer: Vec<LayerCollisions>,
}

/// Deposited melt volume (mm³) for one layer.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct LayerVolumeStats {
    pub z_um: i64,
    pub layer_height_um: i64,
    pub volume_mm3: f64,
}

/// Aggregate deposited melt volume for a program — a physically meaningful quantity that moves with
/// bead width, so it surfaces over-/under-extrusion that coverage and path-count miss. It reflects
/// geometry only (no commanded flow / E-axis).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct VolumeStats {
    pub resolution_um: i64,
    pub total_volume_mm3: f64,
    pub per_layer: Vec<LayerVolumeStats>,
}

/// Compute deposited-volume stats. Layer height is derived from consecutive z unless overridden.
pub fn volume_stats(
    program: &lo::Program,
    resolution_um: i64,
    layer_height_um: Option<i64>,
) -> VolumeStats {
    let vol = denote_lo_volume(program, resolution_um, layer_height_um);
    let mut total = 0.0;
    let per_layer = vol
        .layers
        .iter()
        .map(|l| {
            let v: f64 = l.cells.values().sum();
            total += v;
            LayerVolumeStats {
                z_um: l.z_um,
                layer_height_um: l.layer_height_um,
                volume_mm3: v,
            }
        })
        .collect();
    VolumeStats {
        resolution_um: resolution_um.max(1),
        total_volume_mm3: total,
        per_layer,
    }
}

fn cell_of(p: &Point, r: i64) -> (i64, i64) {
    (p.x.div_euclid(r), p.y.div_euclid(r))
}

/// Count, per layer, the deposited cells each travel move passes through — a heuristic proxy for
/// nozzle-drag / stringing risk. Endpoint cells are excluded, since a travel legitimately starts and
/// ends at deposited geometry. This is report-only and resolution-bounded, NOT exact collision
/// detection (no Z-hop, retraction, or kinematics modeling).
pub fn travel_collisions(program: &lo::Program, resolution_um: i64) -> TravelCollisions {
    let r = resolution_um.max(1);
    let occ = denote_lo(program, r);
    let mut total = 0u64;
    let mut per_layer = Vec::with_capacity(program.layers.len());
    for (layer, occ_layer) in program.layers.iter().zip(occ.layers.iter()) {
        let deposited = &occ_layer.cells;
        let mut crossed = 0u64;
        for tp in &layer.toolpaths {
            if tp.kind != SegmentKind::Travel {
                continue;
            }
            let pts = &tp.path.points;
            if pts.len() < 2 {
                continue;
            }
            let (start, end) = (cell_of(&pts[0], r), cell_of(pts.last().unwrap(), r));
            for c in polyline_cells(&tp.path, 0, r) {
                if c != start && c != end && deposited.contains(&c) {
                    crossed += 1;
                }
            }
        }
        total += crossed;
        per_layer.push(LayerCollisions {
            z_um: layer.z_um,
            crossed_cells: crossed,
        });
    }
    TravelCollisions {
        resolution_um: r,
        total,
        per_layer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, Toolpath};
    use crate::ir::{Polyline, RegionKind};

    fn extrude(pts: Vec<Point>) -> Toolpath {
        Toolpath {
            kind: SegmentKind::Extrude(RegionKind::Infill),
            path: Polyline::new(pts),
            width_um: 400,
            flow_e: None,
        }
    }
    fn travel(pts: Vec<Point>) -> Toolpath {
        Toolpath {
            kind: SegmentKind::Travel,
            path: Polyline::new(pts),
            width_um: 0,
            flow_e: None,
        }
    }

    #[test]
    fn program_stats_counts_moves_and_travel() {
        let prog = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![
                    extrude(vec![Point::new(0, 0), Point::new(10_000, 0)]),
                    travel(vec![Point::new(10_000, 0), Point::new(0, 10_000)]),
                    extrude(vec![Point::new(0, 10_000), Point::new(10_000, 10_000)]),
                ],
            }],
        };
        let s = program_stats(&prog);
        assert_eq!(s.layers, 1);
        assert_eq!(s.extruding_toolpaths, 2);
        assert_eq!(s.travel_toolpaths, 1);
        assert!(s.travel_distance_um > 0.0);
    }

    #[test]
    fn deposit_stats_flags_over_deposition() {
        let seg = || extrude(vec![Point::new(0, 0), Point::new(10_000, 0)]);
        // One pass: no over-deposition.
        let once = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![seg()],
            }],
        };
        let s1 = deposit_stats(&once, 200);
        assert_eq!(s1.over_deposited_cells, 0);
        assert_eq!(s1.max_multiplicity, 1);
        // The same path twice: every cell over-deposited exactly once.
        let twice = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![seg(), seg()],
            }],
        };
        let s2 = deposit_stats(&twice, 200);
        assert_eq!(s2.total_cells, s1.total_cells);
        assert_eq!(s2.over_deposited_cells, s1.total_cells);
        assert_eq!(s2.redeposited_cells, s1.total_cells);
        assert_eq!(s2.max_multiplicity, 2);
    }

    #[test]
    fn volume_stats_total_grows_with_width() {
        let prog = |w: i64| lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Extrude(RegionKind::Infill),
                    path: Polyline::new(vec![Point::new(0, 0), Point::new(20_000, 0)]),
                    width_um: w,
                    flow_e: None,
                }],
            }],
        };
        let thin = volume_stats(&prog(400), 200, None);
        let fat = volume_stats(&prog(800), 200, None);
        assert!(thin.total_volume_mm3 > 0.0);
        assert!(fat.total_volume_mm3 > thin.total_volume_mm3);
        assert_eq!(thin.per_layer.len(), 1);
    }

    #[test]
    fn travel_across_deposited_material_is_flagged_but_a_clear_travel_is_not() {
        // A long horizontal wall, then a travel that cuts straight back across it.
        let wall = extrude(vec![Point::new(0, 5000), Point::new(40_000, 5000)]);
        let crossing = travel(vec![Point::new(20_000, 0), Point::new(20_000, 10_000)]);
        let dragging = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![wall.clone(), crossing],
            }],
        };
        assert!(travel_collisions(&dragging, 200).total > 0);

        // A travel that goes around (never over the wall's cells) is clean.
        let around = travel(vec![Point::new(45_000, 0), Point::new(45_000, 10_000)]);
        let clean = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![wall, around],
            }],
        };
        assert_eq!(travel_collisions(&clean, 200).total, 0);
    }
}
