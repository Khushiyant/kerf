//! The Kerf intermediate representation.
//!
//! Two levels (see `docs/06-architecture.md`):
//!  - [`hi`]: the high, geometric level — filled regions with boundaries. *What should be solid.*
//!  - [`lo`]: the low, move-plan level — an ordered sequence of toolpaths including travel. *What
//!    the machine actually does.*
//!
//! A single lowering ([`crate::lower`]) goes hi -> lo. [`crate::denote`] gives both levels a shared
//! meaning as deposited material, so the lowering's soundness is a checkable property. Production
//! slicers keep these levels separate too (CuraEngine `SliceLayerPart`/`SkinPart` vs the lowered
//! `LayerPlan`; PrusaSlicer `LayerRegion`/`Surface` vs `ExtrusionEntity`).

pub mod hi;
pub mod lo;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A planar coordinate in microns. Integer, mirroring production slicers' fixed-point choice
/// (CuraEngine uses 64-bit integer micron coordinates via ClipperLib) — floats would undermine a
/// *verifiable* IR from day one.
///
/// This is 2D by design. v0 commits to planar-per-layer geometry; non-planar / variable-Z is
/// explicitly out of scope (see `docs/05-direction.md`). If it returns, add Z as a per-segment
/// *attribute*, not by widening this — the most-depended-on node. The verifier's rotation-invariance
/// lives in [`crate::denote`]'s geometric domain (re-slicing after rotation), NOT as a `rotate()`
/// method on this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

impl Point {
    pub fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }
}

/// An ordered chain of points; open (a path) or closed (a loop, last point equals first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Polyline {
    pub points: Vec<Point>,
}

impl Polyline {
    pub fn new(points: Vec<Point>) -> Self {
        Self { points }
    }

    /// Total length of the chain in microns (f64; reference math, not IR state).
    pub fn length_um(&self) -> f64 {
        self.points
            .windows(2)
            .map(|s| {
                let dx = s[1].x as f64 - s[0].x as f64;
                let dy = s[1].y as f64 - s[0].y as f64;
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }
}

/// A filled area: an outer boundary loop plus zero or more hole loops (à la Clipper's / PrusaSlicer's
/// `ExPolygon`). Boundary loops are closed. This is the geometric primitive [`crate::denote`] measures
/// at the high level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Area {
    pub outer: Polyline,
    pub holes: Vec<Polyline>,
}

/// The feature role of deposited material — the semantic tag a raw triangle mesh discards and Kerf
/// keeps (the point of the whole IR). This is *one* axis (feature-role); machine motion
/// (travel vs. extrude) is a separate axis captured by [`lo::SegmentKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RegionKind {
    Perimeter,
    Infill,
    Skin,
    Support,
}

/// An extruding path: a polyline laid down at a given width (microns). Shared by both IR levels for
/// the material-bearing moves.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ExtrudePath {
    pub path: Polyline,
    pub width_um: i64,
}
