//! Kinematics-aware objectives: a trapezoidal print-time estimate over the move plan.
//!
//! Travel distance ([`crate::analyze::program_stats`]) is a crude time proxy — it ignores that a
//! machine accelerates, decelerates, and slows for corners. [`print_time`] models that: each toolpath
//! is planned as a chain of trapezoidal velocity profiles, with per-corner speed limits from the
//! junction-deviation model (the same centripetal approximation GRBL/Marlin/Klipper use), then a
//! backward/forward pass so accel/decel are respected across the whole path. Each toolpath starts and
//! ends at rest (the deterministic, slightly-conservative boundary a travel/retraction implies).
//!
//! The result is deterministic and IR-level, so downstream rewards are exact and reproducible. It is a
//! *model*, not a firmware emulation: it omits per-axis limits, jerk vs. junction-deviation firmware
//! differences, min-layer-time slowdown, and lookahead across toolpath boundaries. Against Klipper's
//! estimator on typical parts it runs a few percent optimistic (Klipper also slows for min-layer-time
//! and per-axis caps); use it for *relative* comparison, which is what optimization needs.

use crate::ir::lo::{self, SegmentKind};
use crate::ir::Point;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Machine motion + envelope limits. Also the profile [`crate::printability`] gates against.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MachineProfile {
    /// Max print (extruding) feedrate, mm/s.
    pub max_print_speed_mm_s: f64,
    /// Max travel feedrate, mm/s.
    pub max_travel_speed_mm_s: f64,
    /// Acceleration, mm/s².
    pub acceleration_mm_s2: f64,
    /// Junction deviation, mm (corner-speed model; larger = faster corners).
    pub junction_deviation_mm: f64,
    /// Minimum time per layer, s (cooling floor; used by printability, not the time sum).
    pub min_layer_time_s: f64,
    /// Bed bounds in microns `(min_x, min_y, max_x, max_y)` and max Z; used by printability.
    pub bed_min_um: (i64, i64),
    pub bed_max_um: (i64, i64),
    pub max_z_um: i64,
}

impl Default for MachineProfile {
    /// A typical desktop i3-class printer (200×200×200 mm bed).
    fn default() -> Self {
        Self {
            max_print_speed_mm_s: 60.0,
            max_travel_speed_mm_s: 150.0,
            acceleration_mm_s2: 1500.0,
            junction_deviation_mm: 0.05,
            min_layer_time_s: 0.0,
            bed_min_um: (0, 0),
            bed_max_um: (200_000, 200_000),
            max_z_um: 200_000,
        }
    }
}

/// A kinematics-aware print-time estimate.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct PrintTime {
    pub total_s: f64,
    /// `(z_um, seconds)` per layer, ascending by Z.
    pub per_layer_s: Vec<(i64, f64)>,
    /// Smallest per-layer time — the quantity a min-layer-time floor is checked against.
    pub min_layer_s: f64,
    pub print_move_s: f64,
    pub travel_move_s: f64,
}

fn dist_mm(a: Point, b: Point) -> f64 {
    let dx = (b.x - a.x) as f64 / 1000.0;
    let dy = (b.y - a.y) as f64 / 1000.0;
    (dx * dx + dy * dy).sqrt()
}

/// Corner speed limit (mm/s) at a vertex from the junction-deviation model. `d_in`/`d_out` are the
/// mm displacement vectors into and out of the vertex; `cap` is the segment feedrate.
fn junction_speed(d_in: (f64, f64), d_out: (f64, f64), a: f64, jd: f64, cap: f64) -> f64 {
    let l_in = (d_in.0 * d_in.0 + d_in.1 * d_in.1).sqrt();
    let l_out = (d_out.0 * d_out.0 + d_out.1 * d_out.1).sqrt();
    if l_in <= 0.0 || l_out <= 0.0 {
        return 0.0;
    }
    // Interior turn angle theta: straight = pi, reversal = 0. cos(theta) = -(unit_in · unit_out).
    let cos_theta = -((d_in.0 * d_out.0 + d_in.1 * d_out.1) / (l_in * l_out)).clamp(-1.0, 1.0);
    let sin_half = (0.5 * (1.0 - cos_theta)).max(0.0).sqrt();
    if sin_half >= 1.0 - 1e-9 {
        return cap; // effectively straight: no corner slowdown
    }
    let r = jd * sin_half / (1.0 - sin_half);
    (a * r).sqrt().min(cap)
}

/// Time (s) for one segment: a trapezoid (or triangle) from `ve` to `vx` cruising up to `vc` under
/// acceleration `a` over length `len` (mm).
fn segment_time(ve: f64, vx: f64, mut vc: f64, len: f64, a: f64) -> f64 {
    if len <= 0.0 {
        return 0.0;
    }
    if a <= 0.0 {
        return len / vc.max(1e-9);
    }
    vc = vc.max(ve).max(vx);
    let d_acc = (vc * vc - ve * ve) / (2.0 * a);
    let d_dec = (vc * vc - vx * vx) / (2.0 * a);
    if d_acc + d_dec <= len {
        let t_acc = (vc - ve) / a;
        let t_dec = (vc - vx) / a;
        let t_cruise = (len - d_acc - d_dec) / vc.max(1e-9);
        t_acc + t_cruise + t_dec
    } else {
        // Triangular: never reaches vc; solve for the peak.
        let vp = (0.5 * (2.0 * a * len + ve * ve + vx * vx)).max(0.0).sqrt();
        (vp - ve).max(0.0) / a + (vp - vx).max(0.0) / a
    }
}

/// Time (s) to traverse one polyline at cruise speed `feedrate`, planned with accel `a` and junction
/// deviation `jd`. Starts and ends at rest.
fn path_time(points: &[Point], feedrate: f64, a: f64, jd: f64) -> f64 {
    if points.len() < 2 || feedrate <= 0.0 {
        return 0.0;
    }
    let lens: Vec<f64> = points.windows(2).map(|s| dist_mm(s[0], s[1])).collect();
    let m = lens.len();
    // Junction speed limit at each vertex; endpoints are rest.
    let mut vj = vec![0.0_f64; m + 1];
    for k in 1..m {
        let d_in = (
            (points[k].x - points[k - 1].x) as f64 / 1000.0,
            (points[k].y - points[k - 1].y) as f64 / 1000.0,
        );
        let d_out = (
            (points[k + 1].x - points[k].x) as f64 / 1000.0,
            (points[k + 1].y - points[k].y) as f64 / 1000.0,
        );
        vj[k] = junction_speed(d_in, d_out, a, jd, feedrate);
    }
    // Backward pass: bound entry speeds so each segment can decelerate to the next limit.
    for i in (0..m).rev() {
        let reachable = (vj[i + 1] * vj[i + 1] + 2.0 * a * lens[i]).sqrt();
        vj[i] = vj[i].min(reachable);
    }
    // Forward pass: bound exit speeds so each segment can accelerate from the previous.
    for i in 0..m {
        let reachable = (vj[i] * vj[i] + 2.0 * a * lens[i]).sqrt();
        vj[i + 1] = vj[i + 1].min(reachable);
    }
    (0..m)
        .map(|i| segment_time(vj[i], vj[i + 1], feedrate, lens[i], a))
        .sum()
}

/// Estimate print time for a move plan under a machine profile.
pub fn print_time(program: &lo::Program, profile: &MachineProfile) -> PrintTime {
    let a = profile.acceleration_mm_s2;
    let jd = profile.junction_deviation_mm.max(0.0);
    let (mut print_s, mut travel_s) = (0.0_f64, 0.0_f64);
    let mut per_layer_s = Vec::with_capacity(program.layers.len());
    for layer in &program.layers {
        let mut layer_s = 0.0;
        for tp in &layer.toolpaths {
            let feed = match tp.kind {
                SegmentKind::Extrude(_) => profile.max_print_speed_mm_s,
                SegmentKind::Travel => profile.max_travel_speed_mm_s,
            };
            let t = path_time(&tp.path.points, feed, a, jd);
            layer_s += t;
            match tp.kind {
                SegmentKind::Extrude(_) => print_s += t,
                SegmentKind::Travel => travel_s += t,
            }
        }
        per_layer_s.push((layer.z_um, layer_s));
    }
    let min_layer_s = per_layer_s
        .iter()
        .map(|&(_, s)| s)
        .fold(f64::INFINITY, f64::min);
    PrintTime {
        total_s: print_s + travel_s,
        min_layer_s: if per_layer_s.is_empty() {
            0.0
        } else {
            min_layer_s
        },
        per_layer_s,
        print_move_s: print_s,
        travel_move_s: travel_s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn line(len_um: i64) -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath::extrude(
                    SegmentKind::Extrude(RegionKind::Perimeter),
                    Polyline::new(vec![Point::new(0, 0), Point::new(len_um, 0)]),
                    400,
                )],
            }],
        }
    }

    #[test]
    fn a_long_straight_move_approaches_the_free_flight_time() {
        // A 200 mm move at 60 mm/s: cruise time ~3.33 s plus the accel/decel ramps.
        let p = MachineProfile::default();
        let t = print_time(&line(200_000), &p).total_s;
        let cruise = 0.2 / p.max_print_speed_mm_s * 1000.0; // 200mm/60
        assert!(t > cruise, "must exceed pure-cruise time (ramps cost time)");
        assert!(t < cruise * 1.5, "but not wildly more for a long move: {t}");
    }

    #[test]
    fn acceleration_makes_short_moves_disproportionately_slow() {
        // Two 10 mm moves take more than 2x... no: a short move never reaches cruise, so its
        // per-mm time is higher than a long move's. Check the short move is triangular-limited.
        let p = MachineProfile::default();
        let short = print_time(&line(2_000), &p).total_s; // 2 mm
        let long = print_time(&line(200_000), &p).total_s; // 200 mm
        assert!(short > 0.0 && long > 0.0);
        assert!(
            long / short < 100.0,
            "100x length is <100x time due to ramps"
        );
    }

    #[test]
    fn a_sharp_corner_costs_more_than_a_straight_path_of_equal_length() {
        let p = MachineProfile::default();
        let straight = Polyline::new(vec![Point::new(0, 0), Point::new(40_000, 0)]);
        let zigzag = Polyline::new(vec![
            Point::new(0, 0),
            Point::new(20_000, 0),
            Point::new(20_000, 20_000), // 90-degree turns
            Point::new(0, 20_000),
        ]);
        let mk = |poly: Polyline| lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath::extrude(
                    SegmentKind::Extrude(RegionKind::Perimeter),
                    poly,
                    400,
                )],
            }],
        };
        let t_straight = print_time(&mk(straight), &p).total_s;
        let t_zig = print_time(&mk(zigzag), &p).total_s;
        assert!(
            t_zig > t_straight,
            "corners must slow the path: {t_zig} !> {t_straight}"
        );
    }

    #[test]
    fn per_layer_and_min_layer_are_reported() {
        let prog = lo::Program {
            layers: vec![line(50_000).layers.remove(0), line(10_000).layers.remove(0)],
        };
        let t = print_time(&prog, &MachineProfile::default());
        assert_eq!(t.per_layer_s.len(), 2);
        assert!(t.min_layer_s <= t.per_layer_s[0].1 && t.min_layer_s <= t.per_layer_s[1].1);
        assert!((t.total_s - t.per_layer_s.iter().map(|x| x.1).sum::<f64>()).abs() < 1e-9);
    }
}
