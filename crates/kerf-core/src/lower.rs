//! Lowering: high-level (geometric) IR -> low-level (move-plan) IR.
//!
//! Emits each region's fills as extruding toolpaths tagged with the region's role, inserting a
//! `Travel` move between consecutive extruding paths.

use crate::ir::lo::{self, SegmentKind};
use crate::ir::{hi, Point, Polyline};

/// Lower a high-level program to a low-level move plan. Each hi layer maps to exactly one lo layer.
pub fn lower(program: &hi::Program) -> lo::Program {
    let mut out = lo::Program::new();
    for layer in &program.layers {
        let mut toolpaths: Vec<lo::Toolpath> = Vec::new();
        let mut last_end: Option<Point> = None;

        for region in &layer.regions {
            for fill in &region.fills {
                let Some(&start) = fill.path.points.first() else {
                    continue;
                };

                if let Some(prev) = last_end {
                    if prev != start {
                        toolpaths.push(lo::Toolpath {
                            kind: SegmentKind::Travel,
                            path: Polyline::new(vec![prev, start]),
                            width_um: 0,
                        });
                    }
                }

                toolpaths.push(lo::Toolpath {
                    kind: SegmentKind::Extrude(region.kind),
                    path: fill.path.clone(),
                    width_um: fill.width_um,
                });
                last_end = fill.path.points.last().copied();
            }
        }

        out.layers.push(lo::Layer {
            z_um: layer.z_um,
            toolpaths,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Area, ExtrudePath, RegionKind};

    fn square_region() -> hi::Region {
        let outer = Polyline::new(vec![
            Point::new(0, 0),
            Point::new(20_000, 0),
            Point::new(20_000, 20_000),
            Point::new(0, 20_000),
            Point::new(0, 0),
        ]);
        hi::Region {
            kind: RegionKind::Perimeter,
            boundary: Area {
                outer: outer.clone(),
                holes: vec![],
            },
            fills: vec![ExtrudePath {
                path: outer,
                width_um: 400,
            }],
        }
    }

    #[test]
    fn lowers_a_single_region_to_one_extruding_move_no_travel() {
        let prog = hi::Program {
            layers: vec![hi::Layer {
                z_um: 200,
                regions: vec![square_region()],
            }],
        };
        let lowered = lower(&prog);
        assert_eq!(lowered.layers.len(), 1);
        assert_eq!(lowered.extrusion_move_count(), 1);
        assert!(lowered.layers[0]
            .toolpaths
            .iter()
            .all(|t| t.kind != SegmentKind::Travel));
    }

    #[test]
    fn inserts_travel_between_disjoint_fills() {
        let a = Polyline::new(vec![Point::new(0, 0), Point::new(1000, 0)]);
        let b = Polyline::new(vec![Point::new(5000, 5000), Point::new(6000, 5000)]);
        let region = hi::Region {
            kind: RegionKind::Infill,
            boundary: Area::default(),
            fills: vec![
                ExtrudePath {
                    path: a,
                    width_um: 400,
                },
                ExtrudePath {
                    path: b,
                    width_um: 400,
                },
            ],
        };
        let prog = hi::Program {
            layers: vec![hi::Layer {
                z_um: 200,
                regions: vec![region],
            }],
        };
        let lowered = lower(&prog);
        let travels = lowered.layers[0]
            .toolpaths
            .iter()
            .filter(|t| t.kind == SegmentKind::Travel)
            .count();
        assert_eq!(travels, 1);
        assert_eq!(lowered.extrusion_move_count(), 2);
    }
}
