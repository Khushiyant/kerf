//! The transformation layer: the actions a search or optimizer applies to a move plan, and the
//! enumeration of which are legal — produced *by the library*, so every consumer shares one correct
//! action space instead of re-deriving (differently, wrongly) its own.
//!
//! An [`Action`] names an edit ([`Action::apply`] performs it in place and returns which layers it
//! touched, for an incremental re-denote). Every shipped action is [`Preservation::PreservingByConstruction`]:
//! when it succeeds it deposits exactly the same material (occupancy *and* per-cell deposit count), so
//! any sequence of enumerated actions preserves the denotation exactly — no drift.
//!  - Reversal and reordering are proved unbounded in Lean (`reversal_invariant` / `pass_sound`).
//!  - Split inserts only an *exact* integer lattice point of the segment (refusing otherwise — it
//!    never rounds off the line); merge drops only a vertex that is exactly collinear with, and
//!    strictly between, its neighbours (so a doubled-back spike is never deleted); seam relocation
//!    keeps a closed loop's edge set. Each is geometrically identical, hence denotation-identical.
//!
//! The [`Preservation::NeedsVerification`] tag is reserved for future transforms that trade exactness
//! for reach; apply-then-check them with [`preserves_occupancy`].
//!
//! Appliers never panic and never produce an invalid program: an out-of-range index or an
//! inapplicable action returns [`TransformError`] and leaves the program untouched.

use crate::denote::{denote_lo, denote_lo_deposit};
use crate::ir::lo;
use crate::ir::Point;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Whether an action's denotation-preservation is guaranteed by construction (a proven algebraic
/// property) or must be verified per application with the oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Preservation {
    /// Denotation-preserving by a proved property (reversal / reordering). No check needed.
    PreservingByConstruction,
    /// Preserving in the exact case, but apply-then-check with the oracle (may round or depend on
    /// geometric preconditions).
    NeedsVerification,
}

/// A named, reproducible edit to a move plan. The action space of a layer is enumerated by
/// [`legal_actions`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Action {
    /// Reverse a toolpath's point order. Denotation is reversal-invariant (Lean `reversal_invariant`).
    ReversePath { layer: usize, toolpath: usize },
    /// Swap two toolpaths' positions. A transposition; reordering preserves denotation (Lean
    /// `pass_sound`).
    SwapToolpaths { layer: usize, a: usize, b: usize },
    /// Reorder a layer's toolpaths by a permutation (`perm[k]` = old index now at position `k`).
    PermuteToolpaths { layer: usize, perm: Vec<usize> },
    /// Insert a vertex `t_permille`/1000 of the way along a segment, splitting it in two (same
    /// toolpath). The split point must land on an exact integer lattice point of the segment; if it
    /// would not (`t·Δ` not divisible by 1000 on both axes) the action refuses rather than rounding
    /// off the line, so it can never perturb the denotation. [`legal_actions`] only enumerates splits
    /// that land exactly.
    SplitSegment {
        layer: usize,
        toolpath: usize,
        segment: usize,
        t_permille: u16,
    },
    /// Drop every interior vertex that is exactly collinear with, AND strictly between, its
    /// neighbours (a genuinely redundant midpoint) — a doubled-back spike is kept, not deleted.
    MergeCollinear { layer: usize, toolpath: usize },
    /// Rotate a closed loop's start vertex to index `vertex` (same set of edges, new seam).
    RelocateSeam {
        layer: usize,
        toolpath: usize,
        vertex: usize,
    },
}

/// Why an [`Action::apply`] could not be performed. The program is left unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TransformError {
    InvalidLayer,
    InvalidToolpath,
    InvalidSegment,
    InvalidVertex,
    /// A precondition failed (e.g. seam relocation on a non-closed path, a bad permutation).
    NotApplicable,
}

impl Action {
    /// The layer this action edits.
    pub fn layer(&self) -> usize {
        match *self {
            Action::ReversePath { layer, .. }
            | Action::SwapToolpaths { layer, .. }
            | Action::PermuteToolpaths { layer, .. }
            | Action::SplitSegment { layer, .. }
            | Action::MergeCollinear { layer, .. }
            | Action::RelocateSeam { layer, .. } => layer,
        }
    }

    /// Whether this action preserves denotation by construction or needs verification.
    ///
    /// Every action shipped today is preserving-by-construction: when it succeeds it deposits exactly
    /// the same material (occupancy *and* per-cell deposit count). Reversal and reordering are
    /// Lean-backed; split inserts only exact on-lattice points (refuses otherwise, never rounds);
    /// merge drops only vertices exactly collinear with and between their neighbours; seam relocation
    /// keeps a closed loop's edge set. The `NeedsVerification` tag is for future non-exact transforms.
    pub fn preservation(&self) -> Preservation {
        match self {
            Action::ReversePath { .. }
            | Action::SwapToolpaths { .. }
            | Action::PermuteToolpaths { .. }
            | Action::SplitSegment { .. }
            | Action::MergeCollinear { .. }
            | Action::RelocateSeam { .. } => Preservation::PreservingByConstruction,
        }
    }

    /// Apply the action in place, returning the layer indices it changed (for an incremental
    /// re-denote). On any error the program is left untouched.
    pub fn apply(&self, program: &mut lo::Program) -> Result<Vec<usize>, TransformError> {
        match self {
            Action::ReversePath { layer, toolpath } => {
                tp_mut(program, *layer, *toolpath)?.path.points.reverse();
                Ok(vec![*layer])
            }
            Action::SwapToolpaths { layer, a, b } => {
                let tps = &mut layer_mut(program, *layer)?.toolpaths;
                if *a >= tps.len() || *b >= tps.len() {
                    return Err(TransformError::InvalidToolpath);
                }
                tps.swap(*a, *b);
                Ok(vec![*layer])
            }
            Action::PermuteToolpaths { layer, perm } => {
                let tps = &mut layer_mut(program, *layer)?.toolpaths;
                if !is_permutation(perm, tps.len()) {
                    return Err(TransformError::NotApplicable);
                }
                let taken = std::mem::take(tps);
                let mut slots: Vec<Option<lo::Toolpath>> = taken.into_iter().map(Some).collect();
                *tps = perm
                    .iter()
                    .map(|&i| slots[i].take().expect("permutation visits each once"))
                    .collect();
                Ok(vec![*layer])
            }
            Action::SplitSegment {
                layer,
                toolpath,
                segment,
                t_permille,
            } => {
                let tp = tp_mut(program, *layer, *toolpath)?;
                let pts = &mut tp.path.points;
                if *segment + 1 >= pts.len() {
                    return Err(TransformError::InvalidSegment);
                }
                let a = pts[*segment];
                let b = pts[*segment + 1];
                let t = (*t_permille).min(1000) as i128;
                // Exact lattice split: a + t·(b-a) is an integer lattice point on the segment only
                // when t·Δ is divisible by 1000 on both axes. Otherwise refuse — rounding would bend
                // the path off the line and could shift a boundary cell. i128 keeps the product exact
                // for all i64 coordinates (i64 subtraction alone would overflow at extreme coords).
                let nx = t * (b.x as i128 - a.x as i128);
                let ny = t * (b.y as i128 - a.y as i128);
                if nx % 1000 != 0 || ny % 1000 != 0 {
                    return Err(TransformError::NotApplicable);
                }
                let mid = Point::new(
                    (a.x as i128 + nx / 1000) as i64,
                    (a.y as i128 + ny / 1000) as i64,
                );
                if mid == a || mid == b {
                    return Err(TransformError::NotApplicable); // degenerate split (t at an endpoint)
                }
                pts.insert(*segment + 1, mid);
                Ok(vec![*layer])
            }
            Action::MergeCollinear { layer, toolpath } => {
                let pts = &mut tp_mut(program, *layer, *toolpath)?.path.points;
                if pts.len() < 3 {
                    return Err(TransformError::NotApplicable);
                }
                let mut out: Vec<Point> = Vec::with_capacity(pts.len());
                out.push(pts[0]);
                for k in 1..pts.len() - 1 {
                    // Drop the vertex only if it is exactly collinear with, AND strictly between, the
                    // kept-previous and next vertices — i.e. a genuinely redundant midpoint. A
                    // doubled-back spike (a -> b -> a) is collinear but NOT between, so it is kept:
                    // deleting it would erase the material the spike deposits.
                    let prev = *out.last().unwrap();
                    if !collinear_between(prev, pts[k], pts[k + 1]) {
                        out.push(pts[k]);
                    }
                }
                out.push(pts[pts.len() - 1]);
                if out.len() == pts.len() {
                    return Err(TransformError::NotApplicable); // nothing to merge
                }
                *pts = out;
                Ok(vec![*layer])
            }
            Action::RelocateSeam {
                layer,
                toolpath,
                vertex,
            } => {
                let pts = &mut tp_mut(program, *layer, *toolpath)?.path.points;
                if pts.len() < 3 || pts.first() != pts.last() {
                    return Err(TransformError::NotApplicable); // seam is meaningful only for a loop
                }
                let ring = &pts[..pts.len() - 1]; // distinct vertices
                if *vertex >= ring.len() {
                    return Err(TransformError::InvalidVertex);
                }
                let mut rotated: Vec<Point> = ring[*vertex..].to_vec();
                rotated.extend_from_slice(&ring[..*vertex]);
                rotated.push(rotated[0]); // re-close
                *pts = rotated;
                Ok(vec![*layer])
            }
        }
    }
}

fn layer_mut(program: &mut lo::Program, layer: usize) -> Result<&mut lo::Layer, TransformError> {
    program
        .layers
        .get_mut(layer)
        .ok_or(TransformError::InvalidLayer)
}

fn tp_mut(
    program: &mut lo::Program,
    layer: usize,
    toolpath: usize,
) -> Result<&mut lo::Toolpath, TransformError> {
    layer_mut(program, layer)?
        .toolpaths
        .get_mut(toolpath)
        .ok_or(TransformError::InvalidToolpath)
}

fn is_permutation(perm: &[usize], n: usize) -> bool {
    if perm.len() != n {
        return false;
    }
    let mut seen = vec![false; n];
    for &i in perm {
        if i >= n || seen[i] {
            return false;
        }
        seen[i] = true;
    }
    true
}

/// Exact (integer) collinearity of three points: the cross product of `b-a` and `c-a` is zero. i128
/// keeps the product exact for all i64 coordinates.
fn collinear(a: Point, b: Point, c: Point) -> bool {
    let cross = (b.x as i128 - a.x as i128) * (c.y as i128 - a.y as i128)
        - (b.y as i128 - a.y as i128) * (c.x as i128 - a.x as i128);
    cross == 0
}

/// Whether `b` is a redundant midpoint of `a`—`c`: exactly collinear AND lying within the segment
/// (between the endpoints). Removing such a `b` leaves the polyline geometrically identical. A
/// backtrack/spike (`c == a`, or `b` past an endpoint) is collinear but not between, so it is NOT
/// redundant — dropping it would change the deposited material.
fn collinear_between(a: Point, b: Point, c: Point) -> bool {
    collinear(a, b, c)
        && b.x >= a.x.min(c.x)
        && b.x <= a.x.max(c.x)
        && b.y >= a.y.min(c.y)
        && b.y <= a.y.max(c.y)
}

/// Enumerate the legal actions over a program — the shared action space. Every action returned is
/// applicable and preserving-by-construction.
///
/// Reordering is generated by adjacent transpositions ([`Action::SwapToolpaths`] of `i`,`i+1`), which
/// generate the whole symmetric group, so the space stays linear in toolpath count rather than
/// quadratic — a deliberate generating set, not a truncation. Also emitted: [`Action::ReversePath`]
/// for every toolpath, a midpoint [`Action::SplitSegment`] for each segment *whose midpoint lands
/// exactly on the lattice* (so the split never rounds off the line), [`Action::MergeCollinear`] for
/// any toolpath with a genuinely redundant (collinear-and-between) vertex, and [`Action::RelocateSeam`]
/// for each vertex of a closed loop.
pub fn legal_actions(program: &lo::Program) -> Vec<Action> {
    let mut actions = Vec::new();
    for (li, layer) in program.layers.iter().enumerate() {
        let n = layer.toolpaths.len();
        for i in 0..n {
            actions.push(Action::ReversePath {
                layer: li,
                toolpath: i,
            });
            if i + 1 < n {
                actions.push(Action::SwapToolpaths {
                    layer: li,
                    a: i,
                    b: i + 1,
                });
            }
        }
        for (ti, tp) in layer.toolpaths.iter().enumerate() {
            let pts = &tp.path.points;
            for seg in 0..pts.len().saturating_sub(1) {
                // Only the exact-lattice midpoint (Δx and Δy both even) — no rounding, no drift. i128
                // deltas so extreme coordinates can't overflow the subtraction.
                let dx = pts[seg + 1].x as i128 - pts[seg].x as i128;
                let dy = pts[seg + 1].y as i128 - pts[seg].y as i128;
                if dx % 2 == 0 && dy % 2 == 0 && (dx != 0 || dy != 0) {
                    actions.push(Action::SplitSegment {
                        layer: li,
                        toolpath: ti,
                        segment: seg,
                        t_permille: 500,
                    });
                }
            }
            if pts.len() >= 3
                && (1..pts.len() - 1).any(|k| collinear_between(pts[k - 1], pts[k], pts[k + 1]))
            {
                actions.push(Action::MergeCollinear {
                    layer: li,
                    toolpath: ti,
                });
            }
            if pts.len() >= 3 && pts.first() == pts.last() {
                for v in 1..pts.len() - 1 {
                    actions.push(Action::RelocateSeam {
                        layer: li,
                        toolpath: ti,
                        vertex: v,
                    });
                }
            }
        }
    }
    actions
}

/// Whether `before` and `after` deposit the same material (occupancy) at `resolution_um` — the check
/// to run after a [`Preservation::NeedsVerification`] action.
pub fn preserves_occupancy(before: &lo::Program, after: &lo::Program, resolution_um: i64) -> bool {
    denote_lo(before, resolution_um) == denote_lo(after, resolution_um)
}

/// Whether `before` and `after` agree on the per-cell deposition count (stricter: also rejects
/// path duplication / splitting that changes multiplicity).
pub fn preserves_deposit(before: &lo::Program, after: &lo::Program, resolution_um: i64) -> bool {
    denote_lo_deposit(before, resolution_um) == denote_lo_deposit(after, resolution_um)
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

    fn two_path_layer() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![
                    ext(vec![Point::new(0, 0), Point::new(10_000, 0)]),
                    ext(vec![Point::new(0, 5_000), Point::new(10_000, 5_000)]),
                ],
            }],
        }
    }

    fn closed_square() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![ext(vec![
                    Point::new(0, 0),
                    Point::new(10_000, 0),
                    Point::new(10_000, 10_000),
                    Point::new(0, 10_000),
                    Point::new(0, 0),
                ])],
            }],
        }
    }

    #[test]
    fn preserving_actions_never_change_denotation() {
        // Reverse and swap are preserving by construction; verify the oracle agrees.
        let base = two_path_layer();
        for act in [
            Action::ReversePath {
                layer: 0,
                toolpath: 0,
            },
            Action::SwapToolpaths {
                layer: 0,
                a: 0,
                b: 1,
            },
            Action::PermuteToolpaths {
                layer: 0,
                perm: vec![1, 0],
            },
        ] {
            assert_eq!(act.preservation(), Preservation::PreservingByConstruction);
            let mut p = base.clone();
            let touched = act.apply(&mut p).unwrap();
            assert_eq!(touched, vec![0]);
            assert!(preserves_occupancy(&base, &p, 200));
            assert!(preserves_deposit(&base, &p, 200));
        }
    }

    #[test]
    fn split_at_exact_midpoint_preserves_occupancy_and_deposit() {
        let base = two_path_layer(); // segment (0,0)-(10000,0): midpoint (5000,0) is exact
        let act = Action::SplitSegment {
            layer: 0,
            toolpath: 0,
            segment: 0,
            t_permille: 500,
        };
        assert_eq!(act.preservation(), Preservation::PreservingByConstruction);
        let mut p = base.clone();
        act.apply(&mut p).unwrap();
        assert_eq!(
            p.layers[0].toolpaths[0].path.points[1],
            Point::new(5000, 0),
            "the inserted point is the exact integer midpoint, on the segment"
        );
        for r in [37, 50, 100, 200] {
            assert!(preserves_occupancy(&base, &p, r));
            assert!(preserves_deposit(&base, &p, r));
        }
    }

    #[test]
    fn split_refuses_off_lattice_rather_than_rounding() {
        // A segment whose t=0.5 point is NOT an integer lattice point (dx odd).
        let prog = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![ext(vec![Point::new(0, 0), Point::new(4001, 2003)])],
            }],
        };
        let mut p = prog.clone();
        assert_eq!(
            Action::SplitSegment {
                layer: 0,
                toolpath: 0,
                segment: 0,
                t_permille: 500,
            }
            .apply(&mut p),
            Err(TransformError::NotApplicable)
        );
        assert_eq!(
            p, prog,
            "a non-exact split must not mutate (never rounds off-line)"
        );
        assert!(
            !legal_actions(&prog)
                .iter()
                .any(|a| matches!(a, Action::SplitSegment { .. })),
            "legal_actions must not offer a non-exact split"
        );
    }

    #[test]
    fn merge_keeps_a_doubled_back_spike_rather_than_deleting_material() {
        // A spike a -> b -> a: b is collinear with (a, a) but NOT between them. Merging must NOT drop
        // b, else the material out to b vanishes (occupancy AND deposit change).
        let base = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![ext(vec![
                    Point::new(0, 0),
                    Point::new(10_000, 0),
                    Point::new(0, 0),
                ])],
            }],
        };
        let mut p = base.clone();
        // Nothing is genuinely redundant, so merge reports NotApplicable and does not mutate.
        assert_eq!(
            Action::MergeCollinear {
                layer: 0,
                toolpath: 0,
            }
            .apply(&mut p),
            Err(TransformError::NotApplicable)
        );
        assert_eq!(p, base);
        assert!(!legal_actions(&base)
            .iter()
            .any(|a| matches!(a, Action::MergeCollinear { .. })));
    }

    #[test]
    fn legal_actions_does_not_panic_on_extreme_coordinates() {
        // i64-subtraction of the segment delta would overflow (panic in debug); i128 must be used.
        let prog = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![ext(vec![
                    Point::new(i64::MIN / 2, 0),
                    Point::new(i64::MAX / 2, 0),
                ])],
            }],
        };
        let _ = legal_actions(&prog); // must not panic
    }

    #[test]
    fn merge_collinear_undoes_a_split() {
        let base = two_path_layer();
        let mut p = base.clone();
        Action::SplitSegment {
            layer: 0,
            toolpath: 0,
            segment: 0,
            t_permille: 300,
        }
        .apply(&mut p)
        .unwrap();
        // The inserted point is exactly collinear, so merging removes it and restores the original.
        Action::MergeCollinear {
            layer: 0,
            toolpath: 0,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            p.layers[0].toolpaths[0].path.points,
            base.layers[0].toolpaths[0].path.points
        );
    }

    #[test]
    fn relocate_seam_preserves_a_loop() {
        let base = closed_square();
        let act = Action::RelocateSeam {
            layer: 0,
            toolpath: 0,
            vertex: 2,
        };
        let mut p = base.clone();
        act.apply(&mut p).unwrap();
        let pts = &p.layers[0].toolpaths[0].path.points;
        assert_eq!(pts.first(), pts.last(), "still a closed loop");
        assert!(preserves_occupancy(&base, &p, 100));
        assert!(preserves_deposit(&base, &p, 100));
    }

    #[test]
    fn invalid_actions_error_without_mutating() {
        let base = two_path_layer();
        let mut p = base.clone();
        assert_eq!(
            Action::ReversePath {
                layer: 9,
                toolpath: 0
            }
            .apply(&mut p),
            Err(TransformError::InvalidLayer)
        );
        assert_eq!(
            Action::RelocateSeam {
                layer: 0,
                toolpath: 0,
                vertex: 0
            }
            .apply(&mut p),
            Err(TransformError::NotApplicable) // toolpath 0 is not a closed loop
        );
        assert_eq!(p, base, "a failed action must not mutate the program");
    }

    #[test]
    fn legal_actions_all_apply_without_panic_and_stay_valid() {
        let prog = closed_square();
        let acts = legal_actions(&prog);
        assert!(!acts.is_empty());
        for act in acts {
            let mut p = prog.clone();
            // Never panics; either applies or reports a clean error.
            if let Ok(touched) = act.apply(&mut p) {
                assert!(touched.iter().all(|&l| l < p.layers.len()));
                // A preserving-tagged action must actually preserve denotation.
                if act.preservation() == Preservation::PreservingByConstruction {
                    assert!(preserves_occupancy(&prog, &p, 200));
                }
            }
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::denote::{denote_lo, denote_lo_deposit};
    use crate::ir::lo::{Layer, SegmentKind};
    use crate::ir::{Point, Polyline, RegionKind};
    use proptest::prelude::*;

    fn arb_prog() -> impl Strategy<Value = lo::Program> {
        // Small coordinate grid with repeats so backtracks/spikes and collinear runs actually occur.
        let pt = (-4i64..4, -4i64..4).prop_map(|(x, y)| Point::new(x * 1000, y * 1000));
        let tp = (
            prop_oneof![
                Just(RegionKind::Perimeter),
                Just(RegionKind::Infill),
                Just(RegionKind::Skin),
            ],
            prop::collection::vec(pt, 2..7),
            100i64..500,
        )
            .prop_map(|(role, pts, w)| {
                lo::Toolpath::extrude(SegmentKind::Extrude(role), Polyline::new(pts), w)
            });
        let layer = (0i64..2000, prop::collection::vec(tp, 1..4)).prop_map(|(z, tps)| Layer {
            z_um: z,
            toolpaths: tps,
        });
        prop::collection::vec(layer, 1..3).prop_map(|ls| lo::Program { layers: ls })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        // The anti-drift property: apply an arbitrary sequence of enumerated legal actions and the
        // deposited material is UNCHANGED — occupancy and per-cell deposit alike, at a fine
        // resolution. Every legal action is exact, so no sequence of them can drift. The small
        // repeating grid deliberately produces spikes and collinear runs (the merge/split edge cases).
        #[test]
        fn any_legal_action_sequence_preserves_denotation_exactly(
            prog in arb_prog(),
            picks in prop::collection::vec(0usize..10_000, 0..50),
        ) {
            let occ0 = denote_lo(&prog, 50);
            let dep0 = denote_lo_deposit(&prog, 50);
            let mut p = prog.clone();
            for pick in picks {
                let acts = legal_actions(&p);
                if acts.is_empty() {
                    break;
                }
                // Every enumerated action is applicable and preserving by construction.
                acts[pick % acts.len()].apply(&mut p).unwrap();
            }
            prop_assert_eq!(denote_lo(&p, 50), occ0, "occupancy drifted under legal actions");
            prop_assert_eq!(denote_lo_deposit(&p, 50), dep0, "deposit drifted under legal actions");
        }
    }
}
