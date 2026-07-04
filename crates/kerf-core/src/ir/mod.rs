//! The Kerf intermediate representation.
//!
//! Two levels: [`hi`] (filled regions with boundaries) and [`lo`] (ordered toolpaths including
//! travel). [`crate::lower`] goes hi -> lo; [`crate::denote`] gives both a shared meaning as
//! deposited material, making the lowering's soundness checkable.

pub mod hi;
pub mod lo;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A planar coordinate in microns. Integer fixed-point, so the IR stays verifiable. 2D by design;
/// non-planar / variable-Z is out of scope for v0.
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

    /// Total length of the chain in microns (f64 reference math, not IR state).
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

/// A filled area: an outer boundary loop plus zero or more hole loops. Boundary loops are closed.
/// The geometric primitive [`crate::denote`] measures at the high level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Area {
    pub outer: Polyline,
    pub holes: Vec<Polyline>,
}

/// The feature role of deposited material. One axis (feature-role); machine motion
/// (travel vs. extrude) is a separate axis captured by [`lo::SegmentKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RegionKind {
    Perimeter,
    Infill,
    Skin,
    Support,
}

/// An extruding path: a polyline laid down at a given width (microns). Shared by both IR levels.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ExtrudePath {
    pub path: Polyline,
    pub width_um: i64,
}
