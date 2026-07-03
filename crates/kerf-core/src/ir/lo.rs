//! The low, move-plan IR level: *what the machine actually does*.
//!
//! An ordered sequence of toolpaths per layer, including travel moves. This is what a backend lowers
//! to G-code. Structurally this is CuraEngine's *lowered* level (`LayerPlan`/`GCodePath`).

use super::{Polyline, RegionKind};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// How a toolpath is traversed. Two axes are separated here: an extruding move carries a feature
/// role ([`RegionKind`]); `Travel` is a low-level-only motion with no role and deposits no material.
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Program {
    pub layers: Vec<Layer>,
}

/// One planar layer at Z (microns): an ordered list of toolpaths.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Layer {
    pub z_um: i64,
    pub toolpaths: Vec<Toolpath>,
}

/// A toolpath: a polyline plus how it is traversed. `width_um` is ignored for `Travel`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Toolpath {
    pub kind: SegmentKind,
    pub path: Polyline,
    pub width_um: i64,
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
                    },
                    Toolpath {
                        kind: SegmentKind::Extrude(RegionKind::Perimeter),
                        path: p,
                        width_um: 400,
                    },
                ],
            }],
        };
        assert_eq!(prog.extrusion_move_count(), 1);
    }
}
