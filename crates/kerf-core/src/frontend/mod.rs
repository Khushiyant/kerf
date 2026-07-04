//! Frontends turn external artifacts into the IR, the inverse of backends. v0: a G-code parser
//! (`gcode`) that reads slicer output back into [`crate::ir::lo`].

pub mod gcode;

pub use gcode::{parse, Diagnostics, ParseReport};
