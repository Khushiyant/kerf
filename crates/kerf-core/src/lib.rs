//! kerf-core: the engine-independent IR for the mesh -> G-code half of fabrication, plus the
//! lowering, backends, optimization passes, and correctness oracle. No Python coupling.
//!
//! The IR has two levels ([`ir::hi`] geometric, [`ir::lo`] move-plan) joined by a [`lower`]ing.
//! [`denote`] gives both a shared meaning as deposited material, so lowering and each [`pass`] can
//! be checked to preserve denotation.

pub mod analyze;
pub mod backend;
pub mod denote;
pub mod diff;
pub mod feature;
pub mod flow;
pub mod frontend;
pub mod hash;
pub mod incremental;
pub mod ir;
#[cfg(feature = "serde")]
pub mod json;
pub mod kinematics;
pub mod lower;
pub mod metamorphic;
pub mod pass;
pub mod printability;
#[cfg(feature = "serde")]
pub mod schema;
pub mod tolerance;
pub mod transform;
pub mod verify;
pub mod voxel;

pub use analyze::{
    deposit_stats, program_stats, travel_collisions, volume_stats, DepositStats, LayerCollisions,
    LayerVolumeStats, ProgramStats, TravelCollisions, VolumeStats,
};
pub use backend::{to_gcode, to_gcode_with, GcodeOptions};
pub use denote::{
    denote_hi, denote_hi_deposit, denote_hi_volume, denote_lo, denote_lo_deposit, denote_lo_layer,
    denote_lo_volume, polyline_cells, self_lowering_sound, Deposit, LayerDeposit, LayerOccupancy,
    LayerVolume, Occupancy, Volume,
};
pub use diff::{
    diff_gcode, diff_programs, graded_diff_gcode, graded_diff_programs, GcodeDiff, GradedDiff,
    LayerDiff, LayerGradedDiff,
};
pub use feature::{
    occupancy_grid, toolpath_feature_matrix, travel_graph, DenseLayer, TravelGraph, FEATURE_COLUMNS,
};
pub use flow::{
    denote_lo_flow, e_conserved, e_conserved_report, e_per_layer, e_total, flow_stats,
    EConservation, Flow, FlowStats, LayerFlow,
};
pub use frontend::{parse, ParseReport};
pub use hash::{canonical_hash, canonical_hash_u128};
pub use incremental::DenoteCache;
pub use ir::{hi, lo, Area, ExtrudePath, Point, Polyline, RegionKind};
pub use kinematics::{print_time, MachineProfile, PrintTime};
pub use metamorphic::{rotate_z, translate, translation_invariant};
pub use pass::{preserves_denotation, Identity, Pass, TravelOrder};
pub use printability::{is_printable, Printability};
pub use tolerance::{
    preserves_within, preserves_within_report, preserves_within_with_flow, EpsilonVerdict,
};
pub use transform::{legal_actions, Action, Preservation, TransformError};
pub use verify::{verify_batch, verify_gcode, verify_roundtrip, GcodeVerification, RoundTrip};
pub use voxel::{
    compare_rotated, rot_x90, rot_y90, rot_z90, rotate_bounds, voxelize, RotationVerdict, Voxels,
};
