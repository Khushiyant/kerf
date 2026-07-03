//! Denotational semantics: what a program *means* as deposited material.
//!
//! [`denote_hi`] / [`denote_lo`] map a program to an [`Occupancy`] — the set of material-occupied
//! cells per layer. Meaning is defined over EXTRUDING paths only: each is swept (a Minkowski sum of
//! the polyline with a disk of diameter `width_um`) and the swept regions are unioned within a layer.
//! Travel denotes nothing.
//!
//! This is the REFERENCE semantics: correctness-first. It rasterizes onto the integer-micron lattice
//! at a chosen resolution. Two optimizations keep it usable at scale *without changing the marked set*
//! (both pinned by `optimized_raster_marks_exactly_the_bruteforce_set`): each segment's per-row column
//! scan is restricted to the covered band (perpendicular-distance prune in [`mark_capsule`]), and
//! layers — being independent — are rasterized in parallel. A ~100k-move print verifies in a few
//! seconds; the meaning is unchanged, only the wasted work is removed.
//!
//! # What the oracle guarantees (and what it does not)
//!
//! Equality of [`Occupancy`] means the two programs deposit the same material **up to the raster
//! resolution**. Two properties make that guarantee trustworthy where earlier versions did not:
//!
//!  - **Reversal / direction invariance.** Segment endpoints are canonicalized before any float math
//!    (see [`mark_capsule`]), so traversing a path forward or backward denotes *exactly* the same
//!    cells. Every reordering pass (e.g. [`crate::pass::TravelOrder`], which may reverse paths) relies
//!    on this; without it the pass was unsound against its own oracle.
//!  - **Conservative coverage, not centre-sampling.** A cell is marked if the swept capsule reaches
//!    within the cell's circumradius of its centre — i.e. if it touches the cell *at all* — so a real
//!    feature is never entirely missed when the resolution is `<=` its width. (An earlier centre-only
//!    test let a 0.4 mm wall lying between grid centres register as zero cells, so deleting it read as
//!    "preserved". Coverage closes that false-confidence hole.)
//!
//! **Residual limitation, stated honestly:** the check is only as sharp as the resolution. Changes
//! smaller than a cell (a sub-resolution vertex nudge, a width tweak far below the grid) may be
//! conflated. Choose `resolution_um <= the smallest feature you care about` (e.g. the nozzle/line
//! width). This is pinned by `oracle_is_blind_below_resolution` in the tests, not hidden.
//!
//! Load-bearing rules: the lossy G-code backend ([`crate::backend`]) drops `width_um`, so it MUST NOT
//! be the semantic reference — meaning is defined here. Float distance is used only INSIDE this
//! checker; the IR stays exact integer microns. Rotated-geometry comparison, when added, must lift to
//! rationals or a bounded tolerance inside the checker, never in the IR. The denotation DOMAIN
//! (rasterized coverage vs. exact swept region) remains provisional — see `docs/07-design-review.md`.

use std::collections::BTreeSet;

#[cfg(not(kani))]
use rayon::prelude::*;

use crate::ir::{hi, lo, Point, Polyline};

/// Occupancy of one layer: the occupied cell coordinates on a `resolution_um` grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerOccupancy {
    pub z_um: i64,
    pub resolution_um: i64,
    pub cells: BTreeSet<(i64, i64)>,
}

/// Occupancy of a whole program, layer-parallel. Two programs denote the same material (at this
/// resolution) iff their occupancies are equal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occupancy {
    pub layers: Vec<LayerOccupancy>,
}

/// Squared distance from point `p` to segment `a`-`b`, in f64. Reference-checker math only.
fn dist2_point_seg(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        let (ex, ey) = (p.0 - a.0, p.1 - a.1);
        return ex * ex + ey * ey;
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    let (ex, ey) = (p.0 - cx, p.1 - cy);
    ex * ex + ey * ey
}

/// Canonical (direction-independent) endpoint order for a segment.
///
/// This is the *mechanism* of reversal invariance (P1): everything `mark_capsule` computes downstream
/// depends on a segment only through this ordered pair, so `canon_seg(p, q) == canon_seg(q, p)` implies
/// the marked cells are identical forward and backward. That equality is machine-checked for **all**
/// `i64` endpoints by the Kani harness `canon_seg_is_order_independent` (see the `kani_proofs` module),
/// and pinned at the denotation level by `denote_is_reversal_invariant`.
fn canon_seg(p: Point, q: Point) -> (Point, Point) {
    if (p.x, p.y) <= (q.x, q.y) {
        (p, q)
    } else {
        (q, p)
    }
}

/// Mark every grid cell the capsule (segment `p`-`q` thickened by `half`) overlaps.
///
/// Endpoints are canonicalized (ordered) before any float math, so the marked set is identical
/// whether a path is traversed forward or reversed. Inclusion is conservative *coverage*: a cell is
/// marked when the capsule reaches within the cell's circumradius (`r/√2`) of its centre, so no cell
/// the capsule actually touches is ever missed. That over-marks boundary cells slightly — the right
/// trade for an oracle, which must never *under*-report deposited material.
fn mark_capsule(cells: &mut BTreeSet<(i64, i64)>, p: Point, q: Point, half: f64, r: i64) {
    // Direction-independent endpoint order: the whole pass framework depends on this invariance.
    // (Validated: removing this makes `denote_is_reversal_invariant`'s flip cases fail.)
    let (a, b) = canon_seg(p, q);
    let reach = half + (r as f64) * std::f64::consts::FRAC_1_SQRT_2;
    let pad = reach.ceil() as i64;
    // Saturating so an extreme coordinate can never overflow the bbox (the loop range stays small —
    // it spans only the segment plus `pad`).
    let minx = a.x.min(b.x).saturating_sub(pad);
    let maxx = a.x.max(b.x).saturating_add(pad);
    let miny = a.y.min(b.y).saturating_sub(pad);
    let maxy = a.y.max(b.y).saturating_add(pad);
    let reach2 = reach * reach;
    let af = (a.x as f64, a.y as f64);
    let bf = (b.x as f64, b.y as f64);
    let half_r = (r / 2) as f64;
    let rf = r as f64;
    let (ci0, ci1) = (minx.div_euclid(r), maxx.div_euclid(r));
    let (cj0, cj1) = (miny.div_euclid(r), maxy.div_euclid(r));
    let (dx, dy) = (bf.0 - af.0, bf.1 - af.1);
    let len = (dx * dx + dy * dy).sqrt();
    // Iterate rows; per row, restrict the tested columns to a *superset* of the covered interval —
    // those cells whose PERPENDICULAR distance to the infinite line a-b is within `reach`. Because
    // distance-to-segment >= perpendicular-distance-to-line, that band contains every covered cell, and
    // each candidate is still decided by the exact `dist2_point_seg` predicate below, so the marked set
    // is byte-for-byte identical to a full bounding-box scan — this only skips provably-empty columns
    // (the bulk of the box for a diagonal segment). A near-horizontal / degenerate segment falls back
    // to the full row.
    for cj in cj0..=cj1 {
        let cy = cj as f64 * rf + half_r;
        let (mut lo, mut hi) = (ci0, ci1);
        if len > 0.0 {
            // Perp. distance of (x, cy) to the line: |(-dy)(x-ax) + dx(cy-ay)| / len <= reach.
            let a_coef = -dy;
            let bconst = dx * (cy - af.1);
            if a_coef != 0.0 {
                let rlen = reach * len;
                let x1 = af.0 + (-rlen - bconst) / a_coef;
                let x2 = af.0 + (rlen - bconst) / a_coef;
                let (xlo, xhi) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
                // x = ci*r + half_r  =>  ci = (x - half_r) / r. Round the interval outward, clamp into
                // the bbox range in f64 (avoids an i64-cast overflow for extreme coordinates), then
                // widen by one cell as a float-rounding guard.
                let clo = ((xlo - half_r) / rf).floor().max(ci0 as f64) as i64;
                let chi = ((xhi - half_r) / rf).ceil().min(ci1 as f64) as i64;
                lo = clo.saturating_sub(1).max(ci0);
                hi = chi.saturating_add(1).min(ci1);
            } else if (cy - af.1).abs() > reach {
                // Horizontal segment: the whole row is beyond reach.
                continue;
            }
        }
        if lo > hi {
            continue;
        }
        for ci in lo..=hi {
            // Compute the cell centre in f64: `ci * r` would overflow i64 for extreme coordinates.
            let center = (ci as f64 * rf + half_r, cy);
            if dist2_point_seg(center, af, bf) <= reach2 {
                cells.insert((ci, cj));
            }
        }
    }
}

/// Rasterize the swept material of a set of (path, width) pairs into a layer occupancy.
fn occupancy_of(paths: &[(&Polyline, i64)], z_um: i64, resolution_um: i64) -> LayerOccupancy {
    let r = resolution_um.max(1);
    let mut cells = BTreeSet::new();
    for (poly, width_um) in paths {
        let half = (*width_um).max(0) as f64 / 2.0; // negative width is malformed; treat as 0
        let pts = &poly.points;
        match pts.len() {
            0 => {}
            1 => mark_capsule(&mut cells, pts[0], pts[0], half, r),
            _ => {
                for seg in pts.windows(2) {
                    mark_capsule(&mut cells, seg[0], seg[1], half, r);
                }
            }
        }
    }
    LayerOccupancy {
        z_um,
        resolution_um: r,
        cells,
    }
}

/// Denote a high-level program: union the swept material of every region's fills, per layer.
///
/// Note: `denote` measures *deposited filament* — the swept `fills`. A [`hi::Region`]'s `boundary` is
/// denotationally inert: `denote` does not check that the fills actually cover the boundary as
/// promised. "Preserves `denote`" therefore means "deposits the same material," not "fills the region
/// it claims to." Verifying fill-vs-boundary agreement is a separate (future) property.
pub fn denote_hi(program: &hi::Program, resolution_um: i64) -> Occupancy {
    Occupancy {
        layers: map_layers(&program.layers, |layer| {
            let paths: Vec<(&Polyline, i64)> = layer
                .regions
                .iter()
                .flat_map(|reg| reg.fills.iter().map(|f| (&f.path, f.width_um)))
                .collect();
            occupancy_of(&paths, layer.z_um, resolution_um)
        }),
    }
}

/// Map each layer to its occupancy. Layers are independent, so this fans out across cores; the
/// order-preserving collect keeps the result identical to a serial map (a `denote` value is a
/// deterministic function of the program). Serial under Kani to keep the proof build dependency-light.
#[cfg(not(kani))]
fn map_layers<L, F>(layers: &[L], f: F) -> Vec<LayerOccupancy>
where
    L: Sync,
    F: Fn(&L) -> LayerOccupancy + Sync + Send,
{
    layers.par_iter().map(f).collect()
}

#[cfg(kani)]
fn map_layers<L, F>(layers: &[L], f: F) -> Vec<LayerOccupancy>
where
    F: Fn(&L) -> LayerOccupancy,
{
    layers.iter().map(f).collect()
}

/// Denote a low-level program: union the swept material of every extruding toolpath, per layer.
/// Travel contributes nothing.
pub fn denote_lo(program: &lo::Program, resolution_um: i64) -> Occupancy {
    Occupancy {
        layers: map_layers(&program.layers, |layer| {
            let paths: Vec<(&Polyline, i64)> = layer
                .toolpaths
                .iter()
                .filter(|t| t.kind.extrudes())
                .map(|t| (&t.path, t.width_um))
                .collect();
            occupancy_of(&paths, layer.z_um, resolution_um)
        }),
    }
}

/// The first non-redundant verification artifact: *lowering preserves denotation*.
///
/// Checks `denote_hi(prog) == denote_lo(lower(prog))` at the given resolution. GlitchFinder cannot
/// state this property because it owns no IR or lowering; here the lowering is ours, so its soundness
/// is mechanically checkable. The v0 lowering only reorders and inserts travel, so this holds by
/// construction — the value right now is the *harness and the property*. It becomes a genuine check
/// the moment a pass rewrites the move plan.
pub fn self_lowering_sound(program: &hi::Program, resolution_um: i64) -> bool {
    denote_hi(program, resolution_um) == denote_lo(&crate::lower::lower(program), resolution_um)
}

/// Machine-checked (bounded) proofs, discharged by Kani (`cargo kani -p kerf-core`). These are not run
/// by `cargo test`; they are model-checked exhaustively over their input domains, which property tests
/// only sample. Each harness targets a *pure kernel* the reference semantics depends on — deliberately
/// loop- and allocation-free so the model checker terminates. See `docs/08-semantics.md` §5.
#[cfg(kani)]
mod kani_proofs {
    use super::{canon_seg, dist2_point_seg};
    use crate::ir::Point;

    /// **P1 mechanism, for all `i64` endpoints.** `denote` depends on a segment only through
    /// `canon_seg`, so proving the canonical order is identical regardless of input order proves
    /// reversal invariance at its root — exhaustively, not by sampling. Removing the canonicalization
    /// (the historical bug) makes this fail.
    #[kani::proof]
    fn canon_seg_is_order_independent() {
        let p = Point::new(kani::any(), kani::any());
        let q = Point::new(kani::any(), kani::any());
        assert_eq!(canon_seg(p, q), canon_seg(q, p));
    }

    fn any_finite_bounded(bound: f64) -> f64 {
        let v: f64 = kani::any();
        kani::assume(v.is_finite() && v >= -bound && v <= bound);
        v
    }

    /// The checker's squared distance is **non-negative and finite** for finite inputs, so a NaN or a
    /// negative value can never leak into the `dist <= reach²` coverage comparison and produce a
    /// spurious SOUND/UNSOUND verdict.
    #[kani::proof]
    fn dist2_point_seg_is_nonneg_and_finite() {
        let bound = 1.0e6_f64;
        let c = (any_finite_bounded(bound), any_finite_bounded(bound));
        let a = (any_finite_bounded(bound), any_finite_bounded(bound));
        let b = (any_finite_bounded(bound), any_finite_bounded(bound));
        let d = dist2_point_seg(c, a, b);
        assert!(d >= 0.0);
        assert!(d.is_finite());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Area, ExtrudePath, RegionKind};

    fn square_program() -> hi::Program {
        let outer = Polyline::new(vec![
            Point::new(0, 0),
            Point::new(20_000, 0),
            Point::new(20_000, 20_000),
            Point::new(0, 20_000),
            Point::new(0, 0),
        ]);
        hi::Program {
            layers: vec![hi::Layer {
                z_um: 200,
                regions: vec![hi::Region {
                    kind: RegionKind::Perimeter,
                    boundary: Area {
                        outer: outer.clone(),
                        holes: vec![],
                    },
                    fills: vec![ExtrudePath {
                        path: outer,
                        width_um: 400,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn a_square_denotes_nonempty_material() {
        let occ = denote_hi(&square_program(), 200);
        assert_eq!(occ.layers.len(), 1);
        assert!(!occ.layers[0].cells.is_empty());
    }

    #[test]
    fn demo_lowering_is_sound() {
        assert!(self_lowering_sound(&square_program(), 200));
    }

    // --- regression tests for the soundness bugs the adversarial review found ---

    fn single_path(points: Vec<Point>, width_um: i64) -> hi::Program {
        hi::Program {
            layers: vec![hi::Layer {
                z_um: 0,
                regions: vec![hi::Region {
                    kind: RegionKind::Infill,
                    boundary: Area::default(),
                    fills: vec![ExtrudePath {
                        path: Polyline::new(points),
                        width_um,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn coverage_catches_a_thin_wall_between_grid_centres() {
        // A 0.4 mm wall at a coarse (1 mm) resolution. The old centre-only test missed it entirely,
        // so deleting it read as "preserved". Conservative coverage marks it.
        let wall = single_path(vec![Point::new(1000, 0), Point::new(1000, 10_000)], 400);
        assert!(!denote_hi(&wall, 1000).layers[0].cells.is_empty());
        // And it must differ from depositing nothing.
        assert_ne!(
            denote_hi(&wall, 1000),
            denote_hi(&single_path(vec![], 0), 1000)
        );
    }

    #[test]
    fn denote_is_reversal_invariant() {
        // Cases at zero width / resolution 1 — the float-boundary regime where the pre-canonicalization
        // `dist2_point_seg` asymmetry actually flips a cell. Each of these FAILS if the endpoint
        // canonicalization in `mark_capsule` is removed (verified), so this is a real guard, not a
        // tautology.
        let cases = [
            vec![Point::new(0, 0), Point::new(600, 0), Point::new(0, 300)],
            vec![Point::new(1684, 1700), Point::new(506, 1054)],
            vec![Point::new(3, 7), Point::new(29, 11), Point::new(13, 2)],
        ];
        for pts in cases {
            let mut rev = pts.clone();
            rev.reverse();
            assert_eq!(
                denote_hi(&single_path(pts.clone(), 0), 1),
                denote_hi(&single_path(rev, 0), 1),
                "reversal changed denotation for {pts:?}",
            );
        }
    }

    #[test]
    fn denote_discriminates_line_width() {
        // A property `lowering_preserves_denotation` silently depends on: width must affect denotation,
        // else a width-changing pass would pass vacuously.
        let thin = single_path(vec![Point::new(0, 5000), Point::new(10_000, 5000)], 200);
        let thick = single_path(vec![Point::new(0, 5000), Point::new(10_000, 5000)], 400);
        assert_ne!(denote_hi(&thin, 100), denote_hi(&thick, 100));
    }

    #[test]
    fn denote_does_not_panic_on_extreme_coordinates() {
        let big = i64::MAX / 2;
        let prog = single_path(vec![Point::new(big, big), Point::new(big + 1000, big)], 400);
        let occ = denote_hi(&prog, 200); // must not overflow/panic
        assert!(!occ.layers[0].cells.is_empty());
    }

    fn assert_reversal_invariant(pts: &[Point], width_um: i64) {
        let fwd = single_path(pts.to_vec(), width_um);
        let mut r = pts.to_vec();
        r.reverse();
        let rev = single_path(r, width_um);
        // resolution 1: the sharpest grid, where float-boundary asymmetry would surface first.
        assert_eq!(
            denote_hi(&fwd, 1),
            denote_hi(&rev, 1),
            "reversal changed denotation for {pts:?} w={width_um}"
        );
    }

    #[test]
    fn reversal_invariant_exhaustively_over_small_programs() {
        // Bounded verification *by enumeration* of P1 (reversal invariance): every 2- and 3-point
        // polyline over a small coordinate grid, at resolution 1, must denote identically forwards
        // and backwards. Unlike the proptest (which samples), this checks the whole bounded domain
        // exhaustively — no small program escapes. The knife-edge unit test above is the guard that
        // *fails* if canonicalization is removed; this is the "holds for all of them" companion.
        let grid2 = [0i64, 1, 2, 3, 5, 8, 13]; // dense grid for the 2-point sweep
        let grid3 = [0i64, 1, 3, 7, 13]; // sparser grid keeps the 3-point sweep tractable
        let widths = [0i64, 2, 5];
        let mut checked = 0u64;
        for &w in &widths {
            for &ax in &grid2 {
                for &ay in &grid2 {
                    for &bx in &grid2 {
                        for &by in &grid2 {
                            assert_reversal_invariant(&[Point::new(ax, ay), Point::new(bx, by)], w);
                            checked += 1;
                        }
                    }
                }
            }
            for &ax in &grid3 {
                for &ay in &grid3 {
                    for &bx in &grid3 {
                        for &by in &grid3 {
                            for &cx in &grid3 {
                                for &cy in &grid3 {
                                    assert_reversal_invariant(
                                        &[
                                            Point::new(ax, ay),
                                            Point::new(bx, by),
                                            Point::new(cx, cy),
                                        ],
                                        w,
                                    );
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            checked > 40_000,
            "expected a large exhaustive sweep, got {checked}"
        );
    }

    #[test]
    fn optimized_raster_marks_exactly_the_bruteforce_set() {
        // Safety net for the perpendicular-band column pruning in `mark_capsule`: it must mark EXACTLY
        // the cells a full bounding-box scan marks — the optimization changes speed, never meaning.
        // Uses the real private `canon_seg` / `dist2_point_seg` so the brute reference is bit-identical.
        fn brute(a: Point, b: Point, width_um: i64, r: i64) -> BTreeSet<(i64, i64)> {
            let (a, b) = canon_seg(a, b);
            let half = width_um.max(0) as f64 / 2.0;
            let reach = half + (r as f64) * std::f64::consts::FRAC_1_SQRT_2;
            let pad = reach.ceil() as i64;
            let reach2 = reach * reach;
            let half_r = (r / 2) as f64;
            let (af, bf) = ((a.x as f64, a.y as f64), (b.x as f64, b.y as f64));
            let (minx, maxx) = (a.x.min(b.x) - pad, a.x.max(b.x) + pad);
            let (miny, maxy) = (a.y.min(b.y) - pad, a.y.max(b.y) + pad);
            let mut s = BTreeSet::new();
            for ci in minx.div_euclid(r)..=maxx.div_euclid(r) {
                for cj in miny.div_euclid(r)..=maxy.div_euclid(r) {
                    let c = (ci as f64 * r as f64 + half_r, cj as f64 * r as f64 + half_r);
                    if dist2_point_seg(c, af, bf) <= reach2 {
                        s.insert((ci, cj));
                    }
                }
            }
            s
        }
        let mut st = 0x1234_5678u64;
        let mut rng = || {
            st = st
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (st >> 33) as i64
        };
        for _ in 0..4000 {
            let a = Point::new(rng().rem_euclid(4000), rng().rem_euclid(4000));
            let b = Point::new(rng().rem_euclid(4000), rng().rem_euclid(4000));
            let w = rng().rem_euclid(800);
            let r = 1 + rng().rem_euclid(400);
            let opt = denote_hi(&single_path(vec![a, b], w), r).layers[0]
                .cells
                .clone();
            assert_eq!(
                opt,
                brute(a, b, w, r),
                "mismatch a={a:?} b={b:?} w={w} r={r}"
            );
        }
    }

    #[test]
    fn oracle_is_blind_below_resolution() {
        // Honest, pinned limitation: the check is only as sharp as its resolution.
        let base = single_path(vec![Point::new(0, 500), Point::new(3000, 500)], 400);
        let nudged = single_path(vec![Point::new(0, 500), Point::new(3001, 500)], 400); // +1 µm
        let moved = single_path(vec![Point::new(0, 500), Point::new(3000, 4000)], 400); // +3.5 mm

        // A sub-resolution nudge is NOT distinguished at a coarse resolution (expected, documented).
        assert_eq!(denote_hi(&base, 1000), denote_hi(&nudged, 1000));
        // A change larger than the resolution IS caught.
        assert_ne!(denote_hi(&base, 1000), denote_hi(&moved, 1000));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::ir::{Area, ExtrudePath, RegionKind};
    use proptest::prelude::*;

    fn arb_point() -> impl Strategy<Value = Point> {
        (0i64..2000, 0i64..2000).prop_map(|(x, y)| Point::new(x, y))
    }
    fn arb_polyline() -> impl Strategy<Value = Polyline> {
        prop::collection::vec(arb_point(), 1..4).prop_map(Polyline::new)
    }
    fn arb_kind() -> impl Strategy<Value = RegionKind> {
        prop_oneof![
            Just(RegionKind::Perimeter),
            Just(RegionKind::Infill),
            Just(RegionKind::Skin),
            Just(RegionKind::Support),
        ]
    }
    fn arb_fill() -> impl Strategy<Value = ExtrudePath> {
        (arb_polyline(), 100i64..400).prop_map(|(path, width_um)| ExtrudePath { path, width_um })
    }
    fn arb_region() -> impl Strategy<Value = hi::Region> {
        (arb_kind(), prop::collection::vec(arb_fill(), 1..3)).prop_map(|(kind, fills)| hi::Region {
            kind,
            boundary: Area::default(),
            fills,
        })
    }
    fn arb_layer() -> impl Strategy<Value = hi::Layer> {
        (0i64..2000, prop::collection::vec(arb_region(), 1..3))
            .prop_map(|(z_um, regions)| hi::Layer { z_um, regions })
    }
    fn arb_program() -> impl Strategy<Value = hi::Program> {
        prop::collection::vec(arb_layer(), 1..3).prop_map(|layers| hi::Program { layers })
    }

    fn reverse_all_paths(mut prog: hi::Program) -> hi::Program {
        for layer in &mut prog.layers {
            for region in &mut layer.regions {
                for fill in &mut region.fills {
                    fill.path.points.reverse();
                }
            }
        }
        prog
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        // The self-lowering-soundness property over arbitrary programs: lowering never changes what
        // material is deposited. This is the harness the real (rewriting) passes will be checked by.
        #[test]
        fn lowering_preserves_denotation(prog in arb_program()) {
            prop_assert!(self_lowering_sound(&prog, 300));
        }

        // denote is invariant under path reversal (broad check, with the fix in place). The
        // deterministic *guard* for this property — the cases that FAIL without the endpoint
        // canonicalization — is the `denote_is_reversal_invariant` unit test; a random proptest does
        // not reliably hit the float knife-edge (verified: 2000 cases over large coords missed it),
        // so the guarantee is pinned by explicit cases rather than by chance.
        #[test]
        fn reversal_invariant(prog in arb_program()) {
            prop_assert_eq!(denote_hi(&prog, 100), denote_hi(&reverse_all_paths(prog.clone()), 100));
        }
    }
}
