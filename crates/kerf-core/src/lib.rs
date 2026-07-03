//! kerf-core: the engine-independent intermediate representation for the mesh -> G-code half of
//! fabrication, plus the lowering, backends, optimization passes, and correctness oracle.
//!
//! This crate has no Python coupling. It is the foundation; `kerf-py` is a thin binding over it.
//!
//! The IR has two levels ([`ir::hi`] geometric, [`ir::lo`] move-plan) joined by a [`lower`]ing that
//! Kerf owns. [`denote`] gives both levels a shared meaning as deposited material, so lowering (and,
//! later, each optimization [`pass`]) can be checked to *preserve denotation* — the project's central,
//! non-redundant claim. See `docs/00-thesis.md` and `docs/06-architecture.md`.

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
pub use diff::{diff_gcode, GcodeDiff, LayerDiff};
pub use frontend::{parse, ParseReport};
pub use ir::{hi, lo, Area, ExtrudePath, Point, Polyline, RegionKind};
pub use metamorphic::{translate, translation_invariant};
pub use pass::{preserves_denotation, Identity, Pass, TravelOrder};
pub use verify::{verify_gcode, GcodeVerification};
