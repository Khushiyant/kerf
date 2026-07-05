//! Pass policy: a pass is a pure transform of the low-level move plan (value-in, value-out).
//!
//! Every pass MUST preserve denotation (`denote_lo(p) == denote_lo(run(p))`);
//! [`preserves_denotation`] checks that obligation with the oracle.

pub mod travel_order;

pub use travel_order::TravelOrder;

use crate::denote::{denote_lo, denote_lo_deposit};
use crate::ir::lo;

/// A denotation-preserving transform over the low-level move plan.
pub trait Pass {
    fn name(&self) -> &str;

    /// Transform a program into an equivalent one. MUST preserve [`crate::denote::denote_lo`].
    fn run(&self, program: lo::Program) -> lo::Program;
}

/// Does `pass` preserve the program's denotation at this resolution?
pub fn preserves_denotation<P: Pass>(pass: &P, program: &lo::Program, resolution_um: i64) -> bool {
    denote_lo(program, resolution_um) == denote_lo(&pass.run(program.clone()), resolution_um)
}

/// Does `pass` preserve the per-cell count of distinct extruding paths? Stricter than
/// [`preserves_denotation`]: it rejects whole-path duplication or repetition (e.g. laying a path down
/// twice), which set-equality cannot see. A pure reorder/reverse pass like [`TravelOrder`] preserves
/// it. It counts paths, not filament, so it does not detect over-extrusion via bead width or flow,
/// and it is the intended obligation only for passes that neither split nor merge paths.
pub fn preserves_deposit<P: Pass>(pass: &P, program: &lo::Program, resolution_um: i64) -> bool {
    denote_lo_deposit(program, resolution_um)
        == denote_lo_deposit(&pass.run(program.clone()), resolution_um)
}

/// The identity pass; trivially denotation-preserving.
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
                    flow_e: None,
                }],
            }],
        }
    }

    #[test]
    fn identity_preserves_denotation() {
        assert!(preserves_denotation(&Identity, &one_segment_program(), 200));
    }

    /// A broken pass that drops the first extruding move; the oracle MUST reject it.
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
        // Two disjoint segments so removing one changes the deposited material.
        let prog = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![
                    Toolpath {
                        kind: SegmentKind::Extrude(RegionKind::Infill),
                        path: Polyline::new(vec![Point::new(0, 0), Point::new(1000, 0)]),
                        width_um: 400,
                        flow_e: None,
                    },
                    Toolpath {
                        kind: SegmentKind::Extrude(RegionKind::Infill),
                        path: Polyline::new(vec![Point::new(9000, 9000), Point::new(10_000, 9000)]),
                        width_um: 400,
                        flow_e: None,
                    },
                ],
            }],
        };
        assert!(!preserves_denotation(&DropFirstExtrude, &prog, 200));
    }

    /// A pass that lays the first extruding move down a second time — the classic over-extrusion bug.
    struct DuplicateFirstExtrude;
    impl Pass for DuplicateFirstExtrude {
        fn name(&self) -> &str {
            "buggy-duplicate-first"
        }
        fn run(&self, mut program: lo::Program) -> lo::Program {
            for layer in &mut program.layers {
                if let Some(pos) = layer.toolpaths.iter().position(|t| t.kind.extrudes()) {
                    let dup = layer.toolpaths[pos].clone();
                    layer.toolpaths.insert(pos + 1, dup);
                }
            }
            program
        }
    }

    #[test]
    fn deposit_oracle_catches_double_deposition_the_set_oracle_misses() {
        let prog = one_segment_program();
        // The set-based oracle is fooled: unioning a duplicate path changes no occupied cell.
        assert!(preserves_denotation(&DuplicateFirstExtrude, &prog, 200));
        // The deposit oracle is not: the doubled material is a real, detected difference.
        assert!(!preserves_deposit(&DuplicateFirstExtrude, &prog, 200));
    }
}
