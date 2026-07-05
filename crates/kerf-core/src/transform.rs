//! The transformation layer: the actions a search or optimizer applies to a move plan, and the
//! enumeration of which are legal — produced *by the library*, so every consumer shares one correct
//! action space instead of re-deriving (differently, wrongly) its own.
//!
//! An [`Action`] names an edit ([`Action::apply`] performs it in place and returns which layers it
//! touched, for an incremental re-denote). Each action is tagged [`Preservation`]:
//!  - **PreservingByConstruction** — reversal and reordering, whose denotation-preservation is proved
//!    unbounded in Lean (`reversal_invariant` / `pass_sound`). No per-application check is required.
//!  - **NeedsVerification** — split / merge / seam relocation, which are preserving in the exact case
//!    but can round off-lattice or depend on a loop being closed; apply, then check with the oracle
//!    ([`preserves_occupancy`]).
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
    /// toolpath). Exact on-lattice, but the inserted point rounds to integer microns.
    SplitSegment {
        layer: usize,
        toolpath: usize,
        segment: usize,
        t_permille: u16,
    },
    /// Drop every interior vertex that is exactly collinear with its neighbours from a toolpath.
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
    pub fn preservation(&self) -> Preservation {
        match self {
            Action::ReversePath { .. }
            | Action::SwapToolpaths { .. }
            | Action::PermuteToolpaths { .. } => Preservation::PreservingByConstruction,
            Action::SplitSegment { .. }
            | Action::MergeCollinear { .. }
            | Action::RelocateSeam { .. } => Preservation::NeedsVerification,
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
                let t = (*t_permille as f64 / 1000.0).clamp(0.0, 1.0);
                let mid = Point::new(
                    (a.x as f64 + t * (b.x - a.x) as f64).round() as i64,
                    (a.y as f64 + t * (b.y - a.y) as f64).round() as i64,
                );
                if mid == a || mid == b {
                    return Err(TransformError::NotApplicable); // degenerate split
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
                    // Keep the vertex unless it is exactly collinear with the kept-previous and next.
                    let prev = *out.last().unwrap();
                    if !collinear(prev, pts[k], pts[k + 1]) {
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

/// Enumerate the legal actions over a program — the shared action space.
///
/// Reordering is generated by adjacent transpositions ([`Action::SwapToolpaths`] of `i`,`i+1`), which
/// generate the whole symmetric group, so the space stays linear in toolpath count rather than
/// quadratic — a deliberate generating set, not a truncation. Also emitted: [`Action::ReversePath`]
/// for every toolpath, one midpoint [`Action::SplitSegment`] per segment, [`Action::MergeCollinear`]
/// for any toolpath with a removable vertex, and [`Action::RelocateSeam`] for each vertex of a closed
/// loop.
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
                actions.push(Action::SplitSegment {
                    layer: li,
                    toolpath: ti,
                    segment: seg,
                    t_permille: 500,
                });
            }
            if pts.len() >= 3
                && (1..pts.len() - 1).any(|k| collinear(pts[k - 1], pts[k], pts[k + 1]))
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
    fn split_at_midpoint_preserves_occupancy() {
        let base = two_path_layer();
        let act = Action::SplitSegment {
            layer: 0,
            toolpath: 0,
            segment: 0,
            t_permille: 500,
        };
        assert_eq!(act.preservation(), Preservation::NeedsVerification);
        let mut p = base.clone();
        act.apply(&mut p).unwrap();
        assert_eq!(
            p.layers[0].toolpaths[0].path.points.len(),
            3,
            "a vertex was inserted"
        );
        assert!(preserves_occupancy(&base, &p, 200));
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
