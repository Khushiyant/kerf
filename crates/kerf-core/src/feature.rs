//! Featurization: array-shaped views of a program for learned models and analytics, computed in Rust
//! so consumers don't hand-roll slow featurizers over JSON.
//!
//! Three views:
//!  - [`occupancy_grid`] — a dense per-layer 0/1 raster (row-major, cropped to the occupied bbox),
//!    ready to become a `numpy` array.
//!  - [`toolpath_feature_matrix`] — one fixed-width numeric row per toolpath ([`FEATURE_COLUMNS`]),
//!    ready to reshape into an `(n, k)` matrix.
//!  - [`travel_graph`] — toolpaths as nodes (centroid + Z) and the tour's hops as weighted edges.
//!
//! Everything is derived from the denotation and the IR; the numbers are exact and deterministic.

use crate::denote::denote_lo_layer;
use crate::ir::lo::{self, SegmentKind};
use crate::ir::RegionKind;

#[cfg(feature = "serde")]
use serde::Serialize;

/// A dense 0/1 occupancy raster for one layer, cropped to the occupied cells' bounding box.
/// `data` is row-major, `rows * cols` bytes; cell `(i, j)` (grid coords) is at
/// `data[(j - min_j) * cols + (i - min_i)]`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct DenseLayer {
    pub z_um: i64,
    pub resolution_um: i64,
    pub min_i: i64,
    pub min_j: i64,
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<u8>,
}

/// A dense occupancy raster per layer (empty layers yield a 0×0 grid).
pub fn occupancy_grid(program: &lo::Program, resolution_um: i64) -> Vec<DenseLayer> {
    let r = resolution_um.max(1);
    program
        .layers
        .iter()
        .map(|layer| {
            let occ = denote_lo_layer(layer, r);
            let Some(&(mut min_i, mut min_j)) = occ.cells.iter().next() else {
                return DenseLayer {
                    z_um: layer.z_um,
                    resolution_um: r,
                    min_i: 0,
                    min_j: 0,
                    rows: 0,
                    cols: 0,
                    data: Vec::new(),
                };
            };
            let (mut max_i, mut max_j) = (min_i, min_j);
            for &(i, j) in &occ.cells {
                min_i = min_i.min(i);
                max_i = max_i.max(i);
                min_j = min_j.min(j);
                max_j = max_j.max(j);
            }
            let cols = (max_i - min_i + 1) as usize;
            let rows = (max_j - min_j + 1) as usize;
            let mut data = vec![0u8; rows * cols];
            for &(i, j) in &occ.cells {
                let idx = (j - min_j) as usize * cols + (i - min_i) as usize;
                data[idx] = 1;
            }
            DenseLayer {
                z_um: layer.z_um,
                resolution_um: r,
                min_i,
                min_j,
                rows,
                cols,
                data,
            }
        })
        .collect()
}

/// Column names of a [`toolpath_feature_matrix`] row, in order.
pub const FEATURE_COLUMNS: &[&str] = &[
    "layer_index",
    "z_um",
    "is_extrude",
    "role_tag", // 0 travel, 1 perimeter, 2 infill, 3 skin, 4 support
    "width_um",
    "length_um",
    "min_x",
    "min_y",
    "max_x",
    "max_y",
    "start_x",
    "start_y",
    "end_x",
    "end_y",
    "n_points",
    "flow_e", // NaN when unspecified
];

fn role_tag(kind: SegmentKind) -> f64 {
    match kind {
        SegmentKind::Travel => 0.0,
        SegmentKind::Extrude(RegionKind::Perimeter) => 1.0,
        SegmentKind::Extrude(RegionKind::Infill) => 2.0,
        SegmentKind::Extrude(RegionKind::Skin) => 3.0,
        SegmentKind::Extrude(RegionKind::Support) => 4.0,
    }
}

/// One fixed-width numeric feature row per toolpath (see [`FEATURE_COLUMNS`]). Returns
/// `(n_toolpaths, row-major flat data)`; reshape to `(n, FEATURE_COLUMNS.len())`.
pub fn toolpath_feature_matrix(program: &lo::Program) -> (usize, Vec<f64>) {
    let k = FEATURE_COLUMNS.len();
    let mut rows = 0usize;
    let mut out = Vec::new();
    for (li, layer) in program.layers.iter().enumerate() {
        for tp in &layer.toolpaths {
            let pts = &tp.path.points;
            let (mut minx, mut miny, mut maxx, mut maxy) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
            for p in pts {
                minx = minx.min(p.x);
                miny = miny.min(p.y);
                maxx = maxx.max(p.x);
                maxy = maxy.max(p.y);
            }
            if pts.is_empty() {
                minx = 0;
                miny = 0;
                maxx = 0;
                maxy = 0;
            }
            let start = pts.first().copied().unwrap_or(crate::ir::Point::new(0, 0));
            let end = pts.last().copied().unwrap_or(crate::ir::Point::new(0, 0));
            out.extend_from_slice(&[
                li as f64,
                layer.z_um as f64,
                if tp.kind.extrudes() { 1.0 } else { 0.0 },
                role_tag(tp.kind),
                tp.width_um as f64,
                tp.path.length_um(),
                minx as f64,
                miny as f64,
                maxx as f64,
                maxy as f64,
                start.x as f64,
                start.y as f64,
                end.x as f64,
                end.y as f64,
                pts.len() as f64,
                tp.flow_e.unwrap_or(f64::NAN),
            ]);
            rows += 1;
        }
    }
    debug_assert_eq!(out.len(), rows * k);
    (rows, out)
}

/// The travel graph: each toolpath is a node (its centroid X,Y and layer Z); each within-layer hop
/// from one toolpath's end to the next toolpath's start is a weighted edge (distance in microns).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct TravelGraph {
    /// `[x, y, z]` (microns) centroid of each toolpath, in program order.
    pub nodes: Vec<[f64; 3]>,
    /// `(from_node, to_node, distance_um)` hops connecting consecutive toolpaths within a layer.
    pub edges: Vec<(u32, u32, f64)>,
}

/// Build the [`TravelGraph`] for a program.
pub fn travel_graph(program: &lo::Program) -> TravelGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut idx = 0u32;
    for layer in &program.layers {
        let mut prev: Option<(u32, crate::ir::Point)> = None;
        for tp in &layer.toolpaths {
            let pts = &tp.path.points;
            if pts.is_empty() {
                continue;
            }
            let (sx, sy) = pts.iter().fold((0i128, 0i128), |(ax, ay), p| {
                (ax + p.x as i128, ay + p.y as i128)
            });
            nodes.push([
                sx as f64 / pts.len() as f64,
                sy as f64 / pts.len() as f64,
                layer.z_um as f64,
            ]);
            let this = idx;
            let start = *pts.first().unwrap();
            if let Some((pi, pend)) = prev {
                let dx = (start.x - pend.x) as f64;
                let dy = (start.y - pend.y) as f64;
                edges.push((pi, this, (dx * dx + dy * dy).sqrt()));
            }
            prev = Some((this, *pts.last().unwrap()));
            idx += 1;
        }
    }
    TravelGraph { nodes, edges }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn prog() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![
                    Toolpath::extrude(
                        SegmentKind::Extrude(RegionKind::Perimeter),
                        Polyline::new(vec![Point::new(0, 0), Point::new(10_000, 0)]),
                        400,
                    ),
                    Toolpath::extrude(
                        SegmentKind::Extrude(RegionKind::Infill),
                        Polyline::new(vec![Point::new(0, 4_000), Point::new(10_000, 4_000)]),
                        400,
                    ),
                ],
            }],
        }
    }

    #[test]
    fn dense_grid_marks_exactly_the_occupied_cells() {
        let g = &occupancy_grid(&prog(), 200)[0];
        assert!(g.rows > 0 && g.cols > 0);
        let ones: usize = g.data.iter().map(|&b| b as usize).sum();
        assert_eq!(
            ones,
            crate::denote::denote_lo(&prog(), 200).layers[0].cells.len()
        );
        assert_eq!(g.data.len(), g.rows * g.cols);
    }

    #[test]
    fn feature_matrix_has_one_row_per_toolpath_and_correct_width() {
        let (rows, data) = toolpath_feature_matrix(&prog());
        assert_eq!(rows, 2);
        assert_eq!(data.len(), rows * FEATURE_COLUMNS.len());
        // Row 0 is an extruding perimeter of width 400, length 10 mm.
        assert_eq!(
            data[FEATURE_COLUMNS
                .iter()
                .position(|&c| c == "is_extrude")
                .unwrap()],
            1.0
        );
        let len_col = FEATURE_COLUMNS
            .iter()
            .position(|&c| c == "length_um")
            .unwrap();
        assert!((data[len_col] - 10_000.0).abs() < 1.0);
    }

    #[test]
    fn travel_graph_links_consecutive_toolpaths() {
        let g = travel_graph(&prog());
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        let (a, b, d) = g.edges[0];
        assert_eq!((a, b), (0, 1));
        assert!(d > 0.0, "the hop between the two paths has positive length");
    }
}
