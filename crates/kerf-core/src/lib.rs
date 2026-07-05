//! kerf-core: the engine-independent IR for the mesh -> G-code half of fabrication, plus the
//! lowering, backends, optimization passes, and correctness oracle. No Python coupling.
//!
//! The IR has two levels ([`ir::hi`] geometric, [`ir::lo`] move-plan) joined by a [`lower`]ing.
//! [`denote`] gives both a shared meaning as deposited material, so lowering and each [`pass`] can
//! be checked to preserve denotation.

pub mod backend;
pub mod denote;
pub mod diff;
pub mod frontend;
pub mod ir;
#[cfg(feature = "serde")]
pub mod json;
pub mod lower;
pub mod metamorphic;
pub mod pass;
pub mod verify;

pub use backend::{to_gcode, to_gcode_with, GcodeOptions};
pub use denote::{denote_hi, denote_lo, self_lowering_sound, LayerOccupancy, Occupancy};
pub use diff::{diff_gcode, diff_programs, GcodeDiff, LayerDiff};
pub use frontend::{parse, ParseReport};
pub use ir::{hi, lo, Area, ExtrudePath, Point, Polyline, RegionKind};
pub use metamorphic::{translate, translation_invariant};
pub use pass::{preserves_denotation, Identity, Pass, TravelOrder};
pub use verify::{verify_gcode, verify_roundtrip, GcodeVerification, RoundTrip};
