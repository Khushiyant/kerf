//! End-to-end verification of real slicer output: parse G-code into the IR, then check properties
//! Kerf can state *because it owns the IR and the transforms* — which a black-box slicer tester
//! (GlitchFinder) structurally cannot.
//!
//! Two independent checks over the parsed program, at a chosen raster resolution:
//!  - **Pass soundness** — a Kerf optimization pass ([`crate::pass::TravelOrder`]) applied to the real
//!    geometry must preserve its denotation (`denote_lo(prog) == denote_lo(pass(prog))`).
//!  - **Translation-invariance** — a metamorphic relation ([`crate::metamorphic`]) that checks
//!    `denote`'s coordinate handling is shift-consistent on the parsed program.
//!
//! A verdict is only meaningful if geometry was actually recovered: [`GcodeVerification::ok`] requires
//! [`GcodeVerification::has_geometry`], so a file we failed to extract never reads as "sound".

use crate::frontend::{parse, Diagnostics};
use crate::metamorphic::translation_invariant;
use crate::pass::{preserves_denotation, TravelOrder};

#[cfg(feature = "serde")]
use serde::Serialize;

/// The outcome of verifying a G-code file end to end.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct GcodeVerification {
    /// What the parser recovered / guessed / dropped.
    pub diagnostics: Diagnostics,
    /// The resolution (microns) the checks were run at.
    pub resolution_um: i64,
    /// Whether any extruding geometry was recovered. If false, the checks below are vacuously true and
    /// [`GcodeVerification::ok`] is NOT satisfied — there was nothing to verify.
    pub has_geometry: bool,
    /// A Kerf pass preserved the deposited material of the parsed program.
    pub pass_preserves_denotation: bool,
    /// The parsed program is translation-invariant under a whole-cell shift.
    pub translation_invariant: bool,
}

impl GcodeVerification {
    /// The recovered program survives Kerf's transforms unchanged — AND there was geometry to check.
    /// The `has_geometry` guard prevents a vacuously-green verdict on G-code we failed to extract.
    pub fn ok(&self) -> bool {
        self.has_geometry && self.pass_preserves_denotation && self.translation_invariant
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
        translation_invariant: translation_invariant(prog, 3, 5, resolution_um),
        resolution_um,
        diagnostics: report.diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::Pass;

    // Three disjoint features in a travel-wasting order (near-origin, far, middle), so TravelOrder
    // genuinely reorders (and may reverse) rather than no-op'ing — makes the soundness check non-vacuous.
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
        // Arcs flatten to chord polylines that must survive lowering, TravelOrder (incl. reversal),
        // and the metamorphic check just like straight moves.
        let arc = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG0 X10 Y0\nG2 X0 Y10 I-10 J0 E1\nG2 X-10 Y0 I0 J-10 E1";
        let v = verify_gcode(arc, 200);
        assert!(v.has_geometry);
        assert!(v.ok(), "arc-derived geometry failed verification");
    }

    #[test]
    fn no_geometry_is_not_a_green_verdict() {
        // A header-only file extracts nothing; the verdict must NOT be "sound".
        let v = verify_gcode("M104 S200\nG28 ; home\nM140 S60\n;comment only", 200);
        assert!(!v.has_geometry);
        assert!(
            !v.ok(),
            "a file with no recovered geometry must not verify as sound"
        );
    }
}
