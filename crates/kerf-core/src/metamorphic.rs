//! Metamorphic verification relations over the IR — properties a correct denotation must satisfy
//! under geometric transforms of the input.
//!
//! GlitchFinder (OOPSLA 2025) pioneered rotation-invariance as a *black-box* oracle over finished
//! slicer output. Kerf's relations run over its own IR — built directly or parsed from real slicer
//! G-code ([`crate::frontend`]) — so they compose with the lowering/pass soundness oracle
//! ([`crate::pass::preserves_denotation`], [`crate::denote::self_lowering_sound`]) that a black-box
//! tester cannot express. This module adds the second relation: translation-invariance.

use std::collections::BTreeSet;

use crate::denote::denote_lo;
use crate::ir::{lo, Point, Polyline};

/// Translate every point of a low-level program by `(dx, dy)` microns.
pub fn translate(program: &lo::Program, dx: i64, dy: i64) -> lo::Program {
    lo::Program {
        layers: program
            .layers
            .iter()
            .map(|l| lo::Layer {
                z_um: l.z_um,
                toolpaths: l
                    .toolpaths
                    .iter()
                    .map(|t| lo::Toolpath {
                        kind: t.kind,
                        width_um: t.width_um,
                        path: Polyline::new(
                            t.path
                                .points
                                .iter()
                                .map(|p| Point::new(p.x.saturating_add(dx), p.y.saturating_add(dy)))
                                .collect(),
                        ),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Translation-invariance: translating the program by an exact whole number of raster cells must
/// shift the occupancy by exactly that many cells and change nothing else. This checks that `denote`
/// handles coordinates shift-consistently (it operates on an already-built `lo::Program`, so it
/// exercises `denote`/geometry, not the parser).
///
/// The translation is by whole cells (`cells_* * resolution`) on purpose: a non-cell-multiple shift
/// aliases the grid and is not a clean relation (see `denote`'s resolution caveat). For whole-cell
/// shifts the property is exact — the coverage decision is bit-identical because it depends only on
/// the (unchanged) relative offset between a cell centre and a segment.
pub fn translation_invariant(
    program: &lo::Program,
    cells_x: i64,
    cells_y: i64,
    resolution_um: i64,
) -> bool {
    let r = resolution_um.max(1);
    let base = denote_lo(program, r);
    let moved = denote_lo(&translate(program, cells_x * r, cells_y * r), r);
    if base.layers.len() != moved.layers.len() {
        return false;
    }
    base.layers.iter().zip(&moved.layers).all(|(b, m)| {
        let shifted: BTreeSet<(i64, i64)> = b
            .cells
            .iter()
            .map(|&(i, j)| (i + cells_x, j + cells_y))
            .collect();
        b.z_um == m.z_um && b.resolution_um == m.resolution_um && m.cells == shifted
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::RegionKind;

    fn square() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Extrude(RegionKind::Perimeter),
                    path: Polyline::new(vec![
                        Point::new(0, 0),
                        Point::new(20_000, 0),
                        Point::new(20_000, 20_000),
                        Point::new(0, 20_000),
                        Point::new(0, 0),
                    ]),
                    width_um: 400,
                }],
            }],
        }
    }

    #[test]
    fn square_is_translation_invariant() {
        assert!(translation_invariant(&square(), 2, 3, 200));
        assert!(translation_invariant(&square(), -5, 7, 300));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::RegionKind;
    use proptest::prelude::*;

    fn arb_program() -> impl Strategy<Value = lo::Program> {
        let point = (0i64..2000, 0i64..2000).prop_map(|(x, y)| Point::new(x, y));
        let role = prop_oneof![
            Just(RegionKind::Perimeter),
            Just(RegionKind::Infill),
            Just(RegionKind::Skin),
            Just(RegionKind::Support),
        ];
        let tp =
            (role, prop::collection::vec(point, 1..4), 100i64..400).prop_map(|(rk, pts, w)| {
                Toolpath {
                    kind: SegmentKind::Extrude(rk),
                    path: Polyline::new(pts),
                    width_um: w,
                }
            });
        let layer = (0i64..2000, prop::collection::vec(tp, 1..3)).prop_map(|(z, tps)| Layer {
            z_um: z,
            toolpaths: tps,
        });
        prop::collection::vec(layer, 1..3).prop_map(|ls| lo::Program { layers: ls })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        // Any program is translation-invariant under a whole-cell shift — the metamorphic relation
        // that catches coordinate mishandling in denote (and, via parsed programs, in the frontend).
        #[test]
        fn any_program_is_translation_invariant(
            prog in arb_program(),
            cx in -6i64..6,
            cy in -6i64..6,
        ) {
            prop_assert!(translation_invariant(&prog, cx, cy, 300));
        }
    }
}
