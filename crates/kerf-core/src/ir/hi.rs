//! The high, geometric IR level: what should be solid.
//!
//! A program is a stack of layers, each a set of filled [`Region`]s. No travel here — travel is a
//! lowering artifact ([`crate::lower`]).

use super::{Area, ExtrudePath, RegionKind};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A high-level program: an ordered stack of geometric layers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Program {
    pub layers: Vec<Layer>,
}

/// A planar layer at Z (microns), described as filled regions.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Layer {
    pub z_um: i64,
    pub regions: Vec<Region>,
}

/// A filled region: a `boundary` to be solid, tagged by role, plus the extruding paths that realize it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Region {
    pub kind: RegionKind,
    pub boundary: Area,
    pub fills: Vec<ExtrudePath>,
}

impl Program {
    pub fn new() -> Self {
        Self::default()
    }
}
