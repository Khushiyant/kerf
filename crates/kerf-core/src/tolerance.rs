//! Tolerance-based acceptance: a single call that decides whether two programs are "the same within
//! ε", with the thresholding logic owned — and tested — in one place instead of re-derived by every
//! consumer.
//!
//! Built on the graded distance ([`crate::diff::graded_diff_programs`]): the worst nearest-miss
//! (`max_um`) between the two deposited materials must be within `epsilon_um`. Because the graded
//! distance stays informative when the two are disjoint, this is a smooth, well-behaved acceptance
//! gate for search and RL, unlike exact denotation equality.

use crate::diff::graded_diff_programs;
use crate::flow::e_conserved;
use crate::ir::lo;

#[cfg(feature = "serde")]
use serde::Serialize;

/// The outcome of an ε-tolerance comparison.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct EpsilonVerdict {
    pub epsilon_um: f64,
    pub resolution_um: i64,
    /// Worst nearest-miss distance (microns) between the two deposited materials.
    pub max_um: f64,
    /// Mean nearest-miss distance (microns).
    pub mean_um: f64,
    /// True iff `max_um <= epsilon_um`.
    pub within: bool,
}

/// Whether `a` and `b` deposit the same material to within `epsilon_um` (worst nearest-miss), measured
/// on a `resolution_um` grid.
pub fn preserves_within(
    a: &lo::Program,
    b: &lo::Program,
    epsilon_um: f64,
    resolution_um: i64,
) -> bool {
    preserves_within_report(a, b, epsilon_um, resolution_um).within
}

/// The full ε-tolerance report (see [`preserves_within`]).
pub fn preserves_within_report(
    a: &lo::Program,
    b: &lo::Program,
    epsilon_um: f64,
    resolution_um: i64,
) -> EpsilonVerdict {
    let g = graded_diff_programs(a, b, resolution_um);
    EpsilonVerdict {
        epsilon_um,
        resolution_um: resolution_um.max(1),
        max_um: g.max_um,
        mean_um: g.mean_um,
        within: g.max_um <= epsilon_um,
    }
}

/// Geometry within `epsilon_um` *and* commanded flow (E) conserved within `e_tolerance_mm`. Use when a
/// change must preserve both the shape and the extruded amount (e.g. accepting an optimizer's output
/// as a faithful rewrite, not just a look-alike).
pub fn preserves_within_with_flow(
    a: &lo::Program,
    b: &lo::Program,
    epsilon_um: f64,
    resolution_um: i64,
    e_tolerance_mm: f64,
) -> bool {
    preserves_within(a, b, epsilon_um, resolution_um) && e_conserved(a, b, e_tolerance_mm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn line(y: i64, e: Option<f64>) -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Extrude(RegionKind::Perimeter),
                    path: Polyline::new(vec![Point::new(0, y), Point::new(40_000, y)]),
                    width_um: 400,
                    flow_e: e,
                }],
            }],
        }
    }

    #[test]
    fn identical_programs_are_within_any_epsilon() {
        assert!(preserves_within(&line(0, None), &line(0, None), 0.0, 200));
    }

    #[test]
    fn a_small_shift_is_within_a_generous_epsilon_but_not_a_tight_one() {
        let a = line(0, None);
        let b = line(600, None); // shifted 0.6 mm
        assert!(
            preserves_within(&a, &b, 1000.0, 100),
            "0.6mm shift within 1mm"
        );
        assert!(
            !preserves_within(&a, &b, 100.0, 100),
            "0.6mm shift not within 0.1mm"
        );
        let r = preserves_within_report(&a, &b, 100.0, 100);
        assert!(r.max_um >= 500.0);
    }

    #[test]
    fn flow_gate_rejects_a_geometry_match_with_wrong_e() {
        let a = line(0, Some(1.0));
        let b = line(0, Some(2.0)); // same geometry, double the flow
        assert!(preserves_within(&a, &b, 10.0, 100), "geometry matches");
        assert!(
            !preserves_within_with_flow(&a, &b, 10.0, 100, 1e-6),
            "but the flow gate must reject the doubled E"
        );
    }
}
