//! The travel-order pass: reduce wasted non-printing movement.
//!
//! Per layer, greedily reorders extruding toolpaths (nearest-neighbour over endpoints), optionally
//! reversing an open path to shorten a hop, then regenerates travel moves. Reversing a path is safe
//! only because [`crate::denote`] canonicalizes segment endpoints and is reversal-invariant.

use super::Pass;
use crate::ir::lo::{self, Toolpath};
use crate::ir::{Point, Polyline};

/// Reorders each layer's extruding toolpaths to reduce travel distance.
#[derive(Clone, Copy, Debug)]
pub struct TravelOrder {
    /// Reference point the per-layer tour starts from.
    pub start: Point,
    /// Whether an open path may be traversed in reverse to shorten a hop.
    pub allow_reverse: bool,
}

impl Default for TravelOrder {
    fn default() -> Self {
        Self {
            start: Point::new(0, 0),
            allow_reverse: true,
        }
    }
}

/// Squared Euclidean distance in f64. Only ranks greedy-tour candidates, never affects denotation;
/// f64 cannot overflow or panic on extreme `i64` coordinates.
fn dist2(a: Point, b: Point) -> f64 {
    let dx = a.x as f64 - b.x as f64;
    let dy = a.y as f64 - b.y as f64;
    dx * dx + dy * dy
}

impl TravelOrder {
    /// Greedy nearest-neighbour ordering of non-empty extruding paths.
    fn order(&self, mut paths: Vec<Toolpath>) -> Vec<Toolpath> {
        let mut ordered: Vec<Toolpath> = Vec::with_capacity(paths.len());
        let mut cur = self.start;
        while !paths.is_empty() {
            let mut best_i = 0;
            let mut best_rev = false;
            let mut best_d = f64::INFINITY;
            for (i, tp) in paths.iter().enumerate() {
                let front = *tp.path.points.first().expect("non-empty by construction");
                let df = dist2(cur, front);
                if df < best_d {
                    best_d = df;
                    best_i = i;
                    best_rev = false;
                }
                if self.allow_reverse {
                    let back = *tp.path.points.last().expect("non-empty by construction");
                    let db = dist2(cur, back);
                    if db < best_d {
                        best_d = db;
                        best_i = i;
                        best_rev = true;
                    }
                }
            }
            let mut chosen = paths.remove(best_i);
            if best_rev {
                chosen.path.points.reverse();
            }
            cur = *chosen.path.points.last().unwrap();
            ordered.push(chosen);
        }
        ordered
    }
}

impl Pass for TravelOrder {
    fn name(&self) -> &str {
        "travel-order"
    }

    fn run(&self, program: lo::Program) -> lo::Program {
        let mut out = lo::Program::new();
        for layer in program.layers {
            // Keep only extruding paths; drop existing travels (regenerated below). Empty-point
            // paths have no endpoints, so set them aside and append unchanged.
            let (nonempty, empty): (Vec<Toolpath>, Vec<Toolpath>) = layer
                .toolpaths
                .into_iter()
                .filter(|t| t.kind.extrudes())
                .partition(|t| !t.path.points.is_empty());

            let mut sequenced = self.order(nonempty);
            sequenced.extend(empty);

            let mut toolpaths: Vec<Toolpath> = Vec::new();
            let mut last_end: Option<Point> = None;
            for tp in sequenced {
                let Some(&start) = tp.path.points.first() else {
                    toolpaths.push(tp);
                    continue;
                };
                if let Some(prev) = last_end {
                    if prev != start {
                        toolpaths.push(Toolpath::travel(Polyline::new(vec![prev, start])));
                    }
                }
                last_end = tp.path.points.last().copied();
                toolpaths.push(tp);
            }
            out.layers.push(lo::Layer {
                z_um: layer.z_um,
                toolpaths,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{hi, Area, ExtrudePath, RegionKind};
    use crate::lower::lower;
    use crate::pass::preserves_denotation;

    /// Three short horizontal segments deliberately listed in a travel-wasting order.
    fn scattered_hi() -> hi::Program {
        let seg = |x: i64| Polyline::new(vec![Point::new(x, 0), Point::new(x + 100, 0)]);
        hi::Program {
            layers: vec![hi::Layer {
                z_um: 200,
                regions: vec![hi::Region {
                    kind: RegionKind::Infill,
                    boundary: Area::default(),
                    fills: vec![
                        ExtrudePath {
                            path: seg(0),
                            width_um: 400,
                        },
                        ExtrudePath {
                            path: seg(5000),
                            width_um: 400,
                        },
                        ExtrudePath {
                            path: seg(2500),
                            width_um: 400,
                        },
                    ],
                }],
            }],
        }
    }

    #[test]
    fn reduces_travel_and_preserves_denotation() {
        let lowered = lower(&scattered_hi());
        let before = lowered.travel_distance_um();

        let optimized = TravelOrder::default().run(lowered.clone());
        let after = optimized.travel_distance_um();

        assert!(
            after < before,
            "expected travel to shrink: {after} !< {before}"
        );
        assert_eq!(
            lowered.extrusion_move_count(),
            optimized.extrusion_move_count()
        );
        assert!(preserves_denotation(&TravelOrder::default(), &lowered, 200));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::RegionKind;
    use crate::pass::{preserves_denotation, preserves_deposit};
    use proptest::prelude::*;

    fn arb_point() -> impl Strategy<Value = Point> {
        (0i64..2000, 0i64..2000).prop_map(|(x, y)| Point::new(x, y))
    }
    fn arb_role() -> impl Strategy<Value = RegionKind> {
        prop_oneof![
            Just(RegionKind::Perimeter),
            Just(RegionKind::Infill),
            Just(RegionKind::Skin),
            Just(RegionKind::Support),
        ]
    }
    fn arb_extrude() -> impl Strategy<Value = Toolpath> {
        (
            arb_role(),
            prop::collection::vec(arb_point(), 1..4),
            100i64..400,
        )
            .prop_map(|(role, pts, width_um)| Toolpath {
                kind: SegmentKind::Extrude(role),
                path: Polyline::new(pts),
                width_um,
                flow_e: None,
            })
    }
    fn arb_layer() -> impl Strategy<Value = Layer> {
        (0i64..2000, prop::collection::vec(arb_extrude(), 1..5))
            .prop_map(|(z_um, toolpaths)| Layer { z_um, toolpaths })
    }
    fn arb_program() -> impl Strategy<Value = lo::Program> {
        prop::collection::vec(arb_layer(), 1..3).prop_map(|layers| lo::Program { layers })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        // For any program, reordering never changes the deposited material...
        #[test]
        fn travel_order_preserves_denotation(prog in arb_program()) {
            prop_assert!(preserves_denotation(&TravelOrder::default(), &prog, 300));
        }

        // ...nor the per-cell deposition count (the stricter oracle): reorder/reverse never
        // duplicates, splits, or merges a path.
        #[test]
        fn travel_order_preserves_deposit(prog in arb_program()) {
            prop_assert!(preserves_deposit(&TravelOrder::default(), &prog, 300));
        }
    }
}
