//! The low, move-plan IR level: what the machine actually does.
//!
//! An ordered sequence of toolpaths per layer, including travel moves. A backend lowers this to G-code.

use super::{Polyline, RegionKind};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// How a toolpath is traversed. An extruding move carries a feature role ([`RegionKind`]); `Travel`
/// has no role and deposits no material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum SegmentKind {
    Extrude(RegionKind),
    Travel,
}

impl SegmentKind {
    /// Whether material is deposited while traversing this kind of path.
    pub fn extrudes(self) -> bool {
        matches!(self, SegmentKind::Extrude(_))
    }
}

/// A low-level program: an ordered stack of move-plan layers.
///
/// `PartialEq` but not `Eq`: a toolpath carries an optional f64 `flow_e`, so structural equality is
/// f64-partial. Denotational comparison (the meaningful one) goes through [`crate::denote`], never
/// `==` on the program.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Program {
    pub layers: Vec<Layer>,
}

/// One planar layer at Z (microns): an ordered list of toolpaths.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Layer {
    pub z_um: i64,
    pub toolpaths: Vec<Toolpath>,
}

/// A toolpath: a polyline plus how it is traversed. `width_um` is ignored for `Travel`.
///
/// `flow_e` is the commanded filament advance (mm of E) for the whole toolpath when known — parsed
/// from real G-code, it lets the checker see over-/under-extrusion expressed as flow at fixed
/// geometry, which the geometry-only denotation misses. It is `None` for geometry-only sources
/// (lowering) and for `Travel`. Serialized only when present and defaulted on read, so adding it
/// keeps old JSON valid.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Toolpath {
    pub kind: SegmentKind,
    pub path: Polyline,
    pub width_um: i64,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub flow_e: Option<f64>,
}

impl Toolpath {
    /// An extruding toolpath with unspecified commanded flow.
    pub fn extrude(kind: SegmentKind, path: Polyline, width_um: i64) -> Self {
        Self {
            kind,
            path,
            width_um,
            flow_e: None,
        }
    }

    /// A travel (non-depositing) toolpath.
    pub fn travel(path: Polyline) -> Self {
        Self {
            kind: SegmentKind::Travel,
            path,
            width_um: 0,
            flow_e: None,
        }
    }
}

impl Program {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of extruding (non-travel) toolpaths across all layers.
    pub fn extrusion_move_count(&self) -> usize {
        self.layers
            .iter()
            .flat_map(|l| &l.toolpaths)
            .filter(|t| t.kind.extrudes())
            .count()
    }

    /// Total non-printing travel distance across all layers, in microns.
    pub fn travel_distance_um(&self) -> f64 {
        self.layers
            .iter()
            .flat_map(|l| &l.toolpaths)
            .filter(|t| t.kind == SegmentKind::Travel)
            .map(|t| t.path.length_um())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Point;

    #[test]
    fn counts_only_extruding_moves() {
        let p = Polyline::new(vec![Point::new(0, 0), Point::new(1000, 0)]);
        let prog = Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![
                    Toolpath {
                        kind: SegmentKind::Travel,
                        path: p.clone(),
                        width_um: 0,
                        flow_e: None,
                    },
                    Toolpath {
                        kind: SegmentKind::Extrude(RegionKind::Perimeter),
                        path: p,
                        width_um: 400,
                        flow_e: None,
                    },
                ],
            }],
        };
        assert_eq!(prog.extrusion_move_count(), 1);
    }
}
