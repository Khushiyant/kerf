//! End-to-end verification of real slicer output: parse G-code into the IR, then check pass
//! soundness (`denote_lo(prog) == denote_lo(pass(prog))`) and translation-invariance at a chosen
//! raster resolution. A verdict requires recovered geometry, so an unextractable file never reads
//! as sound.

use crate::backend::to_gcode;
use crate::denote::{denote_lo, denote_lo_deposit};
use crate::frontend::{parse, Diagnostics};
use crate::ir::lo;
use crate::metamorphic::translation_invariant;
use crate::pass::{preserves_denotation, preserves_deposit, TravelOrder};

#[cfg(feature = "serde")]
use serde::Serialize;

/// The outcome of verifying a G-code file end to end.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct GcodeVerification {
    pub diagnostics: Diagnostics,
    /// Resolution (microns) the checks were run at.
    pub resolution_um: i64,
    /// Whether any extruding geometry was recovered. If false the checks are vacuously true and
    /// [`GcodeVerification::ok`] is not satisfied.
    pub has_geometry: bool,
    /// A Kerf pass preserved the deposited material (set of covered cells) of the parsed program.
    pub pass_preserves_denotation: bool,
    /// A Kerf pass preserved the per-cell deposition count — a stricter check that also rejects
    /// duplicated deposition, not just changes to the touched-cell set.
    pub pass_preserves_deposit: bool,
    /// The parsed program is translation-invariant under a whole-cell shift.
    pub translation_invariant: bool,
}

impl GcodeVerification {
    /// True iff geometry was recovered and it survived Kerf's transforms unchanged.
    pub fn ok(&self) -> bool {
        self.has_geometry
            && self.pass_preserves_denotation
            && self.pass_preserves_deposit
            && self.translation_invariant
    }
}

/// Parse real slicer G-code and verify Kerf's operations are sound on the recovered geometry.
pub fn verify_gcode(gcode: &str, resolution_um: i64) -> GcodeVerification {
    let report = parse(gcode);
    let prog = &report.program;
    GcodeVerification {
        has_geometry: report.diagnostics.extruding_toolpaths >= 1,
        pass_preserves_denotation: preserves_denotation(
            &TravelOrder::default(),
            prog,
            resolution_um,
        ),
        pass_preserves_deposit: preserves_deposit(&TravelOrder::default(), prog, resolution_um),
        translation_invariant: translation_invariant(prog, 3, 5, resolution_um),
        resolution_um,
        diagnostics: report.diagnostics,
    }
}

/// The result of a round-trip check: does emitting a move plan to G-code and re-parsing it recover
/// the same deposited material?
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct RoundTrip {
    pub resolution_um: i64,
    /// Whether the input program has any extruding geometry. If false the check is vacuous and
    /// [`RoundTrip::ok`] is not satisfied.
    pub has_geometry: bool,
    /// The emitted-then-reparsed program covers the same set of cells.
    pub occupancy_preserved: bool,
    /// ...and with the same per-cell deposition count, so duplication survives the round-trip too.
    pub deposit_preserved: bool,
}

impl RoundTrip {
    /// True iff the program had geometry and both the covered cells and their deposition counts
    /// survived emit -> re-parse.
    pub fn ok(&self) -> bool {
        self.has_geometry && self.occupancy_preserved && self.deposit_preserved
    }
}

/// Emit `program` to G-code, re-parse it, and check the deposited material is preserved.
///
/// The `lo`->G-code emitter ([`crate::backend`]) sits outside the verified `hi`->`lo` boundary, so
/// this guards that the G-code a printer will actually run still denotes what the move plan does. Run
/// it on any move plan whose emitted output you intend to trust — e.g. an optimizer's or RL agent's
/// result — before sending it to a machine.
///
/// Scope, stated precisely: this certifies the emitter is faithful to *this* move plan for covered
/// cells and per-cell path multiplicity at `resolution_um`. It does NOT certify filament *volume*
/// (the denotation has no extrusion-amount axis), sub-resolution geometry, or that the plan matches
/// the original design intent (that is [`crate::self_lowering_sound`]). It is a per-program runtime
/// check, not a proof.
pub fn verify_roundtrip(program: &lo::Program, resolution_um: i64) -> RoundTrip {
    let has_geometry = program
        .layers
        .iter()
        .any(|l| l.toolpaths.iter().any(|t| t.kind.extrudes()));
    let reparsed = parse(&to_gcode(program)).program;
    RoundTrip {
        resolution_um,
        has_geometry,
        occupancy_preserved: denote_lo(program, resolution_um)
            == denote_lo(&reparsed, resolution_um),
        deposit_preserved: denote_lo_deposit(program, resolution_um)
            == denote_lo_deposit(&reparsed, resolution_um),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::Pass;

    // Disjoint features in travel-wasting order so TravelOrder genuinely reorders rather than no-ops.
    const SCATTERED: &str = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:External perimeter\n;WIDTH:0.45\nG0 X0 Y0\nG1 X1 Y0 E.1\nG0 X80 Y0\nG1 X81 Y0 E.1\nG0 X20 Y0\nG1 X21 Y0 E.1";

    #[test]
    fn real_slicer_output_survives_kerf_operations_and_the_pass_does_real_work() {
        let report = parse(SCATTERED);
        assert_eq!(report.program.extrusion_move_count(), 3);
        let before = report.program.travel_distance_um();
        let after = TravelOrder::default()
            .run(report.program.clone())
            .travel_distance_um();
        assert!(
            after < before,
            "TravelOrder should cut travel: {after} !< {before}"
        );

        let v = verify_gcode(SCATTERED, 200);
        assert!(v.has_geometry);
        assert!(
            v.pass_preserves_denotation,
            "TravelOrder changed the parsed print"
        );
        assert!(
            v.pass_preserves_deposit,
            "TravelOrder changed per-cell deposition while only reordering"
        );
        assert!(v.translation_invariant);
        assert!(v.ok());
    }

    #[test]
    fn cura_output_survives_kerf_operations() {
        let cura = ";Layer height: 0.2\nG21\nM82\nG92 E0\n;LAYER:0\nG0 X50 Y50 Z0.2\n;TYPE:WALL-OUTER\nG1 X70 Y50 E1.2\nG1 X70 Y70 E2.4\n;LAYER:1\nG0 Z0.4\n;TYPE:SKIN\nG1 X60 Y60 E4.8";
        assert!(verify_gcode(cura, 200).ok());
    }

    #[test]
    fn arc_program_verifies_sound_end_to_end() {
        let arc = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG0 X10 Y0\nG2 X0 Y10 I-10 J0 E1\nG2 X-10 Y0 I0 J-10 E1";
        let v = verify_gcode(arc, 200);
        assert!(v.has_geometry);
        assert!(v.ok(), "arc-derived geometry failed verification");
    }

    #[test]
    fn no_geometry_is_not_a_green_verdict() {
        let v = verify_gcode("M104 S200\nG28 ; home\nM140 S60\n;comment only", 200);
        assert!(!v.has_geometry);
        assert!(
            !v.ok(),
            "a file with no recovered geometry must not verify as sound"
        );
    }

    #[test]
    fn emitted_gcode_round_trips_for_a_lowered_program() {
        use crate::ir::{hi, Area, ExtrudePath, Point, Polyline, RegionKind};
        use crate::lower::lower;
        let outer = Polyline::new(vec![
            Point::new(0, 0),
            Point::new(20_000, 0),
            Point::new(20_000, 20_000),
            Point::new(0, 20_000),
            Point::new(0, 0),
        ]);
        let hi = hi::Program {
            layers: vec![hi::Layer {
                z_um: 200,
                regions: vec![hi::Region {
                    kind: RegionKind::Perimeter,
                    boundary: Area {
                        outer: outer.clone(),
                        holes: vec![],
                    },
                    fills: vec![ExtrudePath {
                        path: outer,
                        width_um: 400,
                    }],
                }],
            }],
        };
        let rt = verify_roundtrip(&lower(&hi), 200);
        assert!(
            rt.occupancy_preserved,
            "emit->parse changed the covered cells"
        );
        assert!(
            rt.deposit_preserved,
            "emit->parse changed the deposition count"
        );
        assert!(rt.ok());
    }

    #[test]
    fn round_trip_holds_on_optimized_real_output() {
        // Parse real output, run the pass, and confirm the emitted G-code still denotes the same.
        let optimized = TravelOrder::default().run(parse(SCATTERED).program);
        assert!(
            verify_roundtrip(&optimized, 200).ok(),
            "an optimized plan must emit to G-code that re-parses to the same material"
        );
    }

    #[test]
    fn empty_or_travel_only_program_is_not_a_green_round_trip() {
        use crate::ir::lo::{Layer, SegmentKind, Toolpath};
        use crate::ir::{Point, Polyline};
        // No layers at all.
        assert!(!verify_roundtrip(&lo::Program { layers: vec![] }, 200).ok());
        // A travel-only layer deposits nothing, so the round-trip must not read as green.
        let travel_only = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Travel,
                    path: Polyline::new(vec![Point::new(0, 0), Point::new(1000, 0)]),
                    width_um: 0,
                }],
            }],
        };
        let rt = verify_roundtrip(&travel_only, 200);
        assert!(!rt.has_geometry);
        assert!(!rt.ok());
    }
}
