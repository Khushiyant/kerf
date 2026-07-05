//! The printability envelope: deposit-equal is not the same as printable. A search or optimizer will
//! happily produce a move plan that deposits the right material but runs off the bed, whips through
//! deposited walls, or races a layer too fast to cool. [`is_printable`] is the single gate that keeps
//! those out of results, composing the checks that already exist:
//!  - every point inside the bed bounds and under max Z,
//!  - every layer's kinematic time at or above the min-layer-time floor,
//!  - no travel dragging across deposited material ([`crate::analyze::travel_collisions`]).
//!
//! It is machine-relative (takes a [`MachineProfile`]) and report-only: it returns *why* a plan fails,
//! it never mutates.

use crate::analyze::travel_collisions;
use crate::ir::lo;
use crate::kinematics::{print_time, MachineProfile};

#[cfg(feature = "serde")]
use serde::Serialize;

/// The result of a printability check: an overall verdict plus the per-dimension reasons.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Printability {
    /// True iff every check passed.
    pub printable: bool,
    /// Points that fall outside the bed bounds or above max Z.
    pub out_of_bounds_points: u64,
    /// Layers whose estimated time is below the machine's min-layer-time floor.
    pub layers_below_min_time: u64,
    /// The min-layer-time floor (s) checked against (0 disables the check).
    pub min_layer_time_s: f64,
    /// Fastest layer's estimated time (s).
    pub fastest_layer_s: f64,
    /// Travel-over-material crossing events (nozzle-drag proxy) at `resolution_um`.
    pub travel_collisions: u64,
    pub resolution_um: i64,
}

/// Check a move plan against a machine profile. `resolution_um` sets the grid for the travel-collision
/// check. A plan is printable iff it stays in bounds, no layer is faster than the cooling floor, and
/// no travel drags across deposited material.
pub fn is_printable(
    program: &lo::Program,
    profile: &MachineProfile,
    resolution_um: i64,
) -> Printability {
    // Bounds: every XY inside the bed, every layer Z within [0, max_z].
    let (minx, miny) = profile.bed_min_um;
    let (maxx, maxy) = profile.bed_max_um;
    let mut oob = 0u64;
    for layer in &program.layers {
        let z_ok = layer.z_um >= 0 && layer.z_um <= profile.max_z_um;
        for tp in &layer.toolpaths {
            for p in &tp.path.points {
                if !z_ok || p.x < minx || p.x > maxx || p.y < miny || p.y > maxy {
                    oob += 1;
                }
            }
        }
    }

    // Min layer time: count layers faster than the cooling floor.
    let times = print_time(program, profile);
    let floor = profile.min_layer_time_s.max(0.0);
    let below = if floor > 0.0 {
        times
            .per_layer_s
            .iter()
            .filter(|&&(_, s)| s < floor)
            .count() as u64
    } else {
        0
    };

    // Nozzle-drag: travels crossing deposited cells.
    let collisions = travel_collisions(program, resolution_um).total;

    Printability {
        printable: oob == 0 && below == 0 && collisions == 0,
        out_of_bounds_points: oob,
        layers_below_min_time: below,
        min_layer_time_s: floor,
        fastest_layer_s: times.min_layer_s,
        travel_collisions: collisions,
        resolution_um: resolution_um.max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn ext(pts: Vec<Point>) -> Toolpath {
        Toolpath::extrude(
            SegmentKind::Extrude(RegionKind::Perimeter),
            Polyline::new(pts),
            400,
        )
    }

    fn on_bed() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![ext(vec![
                    Point::new(10_000, 10_000),
                    Point::new(100_000, 10_000),
                    Point::new(100_000, 100_000),
                ])],
            }],
        }
    }

    #[test]
    fn a_plan_on_the_bed_with_no_drag_is_printable() {
        let p = is_printable(&on_bed(), &MachineProfile::default(), 200);
        assert!(p.printable, "{p:?}");
        assert_eq!(p.out_of_bounds_points, 0);
        assert_eq!(p.travel_collisions, 0);
    }

    #[test]
    fn off_bed_geometry_is_rejected() {
        let off = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![ext(vec![Point::new(0, 0), Point::new(500_000, 0)])], // past 200mm
            }],
        };
        let p = is_printable(&off, &MachineProfile::default(), 200);
        assert!(!p.printable);
        assert!(p.out_of_bounds_points > 0);
    }

    #[test]
    fn a_travel_dragging_over_a_wall_is_rejected() {
        let dragging = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![
                    ext(vec![Point::new(10_000, 50_000), Point::new(90_000, 50_000)]),
                    Toolpath::travel(Polyline::new(vec![
                        Point::new(50_000, 10_000),
                        Point::new(50_000, 90_000), // straight across the wall
                    ])),
                ],
            }],
        };
        let p = is_printable(&dragging, &MachineProfile::default(), 200);
        assert!(!p.printable);
        assert!(p.travel_collisions > 0);
    }

    #[test]
    fn a_too_fast_layer_fails_the_cooling_floor() {
        let profile = MachineProfile {
            min_layer_time_s: 1_000.0, // absurd floor no small layer can meet
            ..MachineProfile::default()
        };
        let p = is_printable(&on_bed(), &profile, 200);
        assert!(!p.printable);
        assert_eq!(p.layers_below_min_time, 1);
    }
}
