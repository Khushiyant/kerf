//! Frontends turn external artifacts INTO the IR — the inverse of backends. v0: a G-code parser
//! (`gcode`) that reads real Cura / PrusaSlicer / OrcaSlicer output back into [`crate::ir::lo`], so
//! Kerf can verify what real slicers actually produce.

pub mod gcode;

pub use gcode::{parse, Diagnostics, ParseReport};
