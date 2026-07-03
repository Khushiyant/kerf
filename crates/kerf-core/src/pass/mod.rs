//! Pass policy (see `docs/06-architecture.md`).
//!
//! A pass is a *pure* transform of the low-level move plan: value-in, value-out. This signature
//! admits mutable implementations today AND leaves the door open to wrapping the IR in an e-graph
//! later; a `&mut` signature would bake destructive rewrite into the contract and fight equality
//! saturation, forcing a later trait break.
//!
//! Every pass carries a proof obligation: it MUST preserve denotation
//! (`denote_lo(p) == denote_lo(run(p))`). [`preserves_denotation`] checks that obligation with the
//! oracle. There is deliberately no e-graph, interning, or operator enum yet — those wait for a
//! rewrite-rule catalog and a cost function worth saturating over.

pub mod travel_order;

pub use travel_order::TravelOrder;

use crate::denote::denote_lo;
use crate::ir::lo;

/// A denotation-preserving transform over the low-level move plan.
pub trait Pass {
    fn name(&self) -> &str;

    /// Transform a program into an equivalent one. MUST preserve [`crate::denote::denote_lo`].
    fn run(&self, program: lo::Program) -> lo::Program;
}

/// The oracle for passes: does `pass` preserve the program's denotation at this resolution?
///
/// This is what makes a pass trustworthy. GlitchFinder cannot state this about its transforms
/// because it has none — it only probes finished slicer output. Here the transform is ours, so its
/// soundness is directly checkable.
pub fn preserves_denotation<P: Pass>(pass: &P, program: &lo::Program, resolution_um: i64) -> bool {
    denote_lo(program, resolution_um) == denote_lo(&pass.run(program.clone()), resolution_um)
}

/// The trivial identity pass — proves the trait shape and is trivially denotation-preserving.
pub struct Identity;

impl Pass for Identity {
    fn name(&self) -> &str {
        "identity"
    }
    fn run(&self, program: lo::Program) -> lo::Program {
        program
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn one_segment_program() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Extrude(RegionKind::Perimeter),
                    path: Polyline::new(vec![Point::new(0, 0), Point::new(5000, 0)]),
                    width_um: 400,
                }],
            }],
        }
    }

    #[test]
    fn identity_preserves_denotation() {
        assert!(preserves_denotation(&Identity, &one_segment_program(), 200));
    }

    /// A deliberately broken pass that drops the first extruding move. The oracle MUST reject it —
    /// this proves the check is not vacuous (it has teeth).
    struct DropFirstExtrude;
    impl Pass for DropFirstExtrude {
        fn name(&self) -> &str {
            "buggy-drop-first"
        }
        fn run(&self, mut program: lo::Program) -> lo::Program {
            for layer in &mut program.layers {
                if let Some(pos) = layer.toolpaths.iter().position(|t| t.kind.extrudes()) {
                    layer.toolpaths.remove(pos);
                }
            }
            program
        }
    }

    #[test]
    fn oracle_catches_a_pass_that_drops_material() {
        // Two disjoint segments so removing one provably changes the deposited material.
        let prog = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![
                    Toolpath {
                        kind: SegmentKind::Extrude(RegionKind::Infill),
                        path: Polyline::new(vec![Point::new(0, 0), Point::new(1000, 0)]),
                        width_um: 400,
                    },
                    Toolpath {
                        kind: SegmentKind::Extrude(RegionKind::Infill),
                        path: Polyline::new(vec![Point::new(9000, 9000), Point::new(10_000, 9000)]),
                        width_um: 400,
                    },
                ],
            }],
        };
        assert!(!preserves_denotation(&DropFirstExtrude, &prog, 200));
    }
}
