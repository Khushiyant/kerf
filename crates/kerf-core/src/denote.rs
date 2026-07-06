//! Denotational semantics: a program's meaning as deposited material.
//!
//! [`denote_hi`] / [`denote_lo`] map a program to an [`Occupancy`] — material-occupied cells per
//! layer. Meaning is defined over EXTRUDING paths only: each is swept (Minkowski sum of the polyline
//! with a disk of diameter `width_um`) and unioned within a layer; travel denotes nothing.
//! Equality of [`Occupancy`] means the two programs deposit the same material up to the raster
//! resolution.
//!
//! Rules: distance math is f64 and lives only inside this checker; the IR stays exact integer
//! microns. Cell inclusion is conservative coverage (marked if the capsule touches the cell at all),
//! never centre-sampling, so a feature is never missed when resolution `<=` its width. The check is
//! only as sharp as the resolution; choose `resolution_um <=` the smallest feature that matters.
//!
//! Cost scales with the number of covered cells (roughly total path length / resolution), and layers
//! rasterize in parallel. For a hot loop such as an RL reward, run at a coarse `resolution_um` for a
//! fast approximate check, then re-verify the chosen candidate at a fine resolution.

use std::collections::{BTreeMap, BTreeSet};

#[cfg(not(kani))]
use rayon::prelude::*;

#[cfg(feature = "serde")]
use serde::Serialize;

use crate::ir::{hi, lo, Point, Polyline};

/// Occupancy of one layer: the occupied cell coordinates on a `resolution_um` grid.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct LayerOccupancy {
    pub z_um: i64,
    pub resolution_um: i64,
    pub cells: BTreeSet<(i64, i64)>,
}

impl LayerOccupancy {
    /// A 64-bit order-independent fingerprint of this layer's deposited material (its z, resolution,
    /// and occupied cells). Equal fingerprints mean equal occupancy up to a ~1-in-2^64 collision — a
    /// preservation verdict by hash compare, instead of re-rasterizing and comparing cell sets.
    /// (`cells` is a `BTreeSet`, so iteration is canonical/sorted and the hash is deterministic.)
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a-64
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        mix(self.z_um as u64);
        mix(self.resolution_um as u64);
        for &(x, y) in &self.cells {
            mix(x as u64);
            mix(y as u64);
        }
        h
    }
}

/// Combine per-layer fingerprints into a 128-bit program fingerprint (FNV-1a-128 over the layer hashes,
/// in layer order).
pub(crate) fn combine_fingerprints(layer_fps: &[u64]) -> u128 {
    let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for &fp in layer_fps {
        h ^= fp as u128;
        h = h.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    h
}

/// Occupancy of a whole program. Two programs denote the same material at this resolution iff their
/// occupancies are equal.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Occupancy {
    pub layers: Vec<LayerOccupancy>,
}

impl Occupancy {
    /// Per-layer fingerprints (see [`LayerOccupancy::fingerprint`]).
    pub fn layer_fingerprints(&self) -> Vec<u64> {
        self.layers.iter().map(|l| l.fingerprint()).collect()
    }

    /// A 128-bit fingerprint of the whole deposited material. Equal fingerprints mean equal occupancy
    /// (same layers, same cells) up to collision — a fast "same material" verdict.
    pub fn fingerprint(&self) -> u128 {
        combine_fingerprints(&self.layer_fingerprints())
    }
}

/// The 128-bit material fingerprint of a low-level program at a resolution — a fast identity for
/// "deposits the same material". One-shot (rasterizes once); for repeated checks under edits use an
/// incremental [`crate::incremental::DenoteCache`], which only re-fingerprints changed layers.
pub fn material_fingerprint(program: &lo::Program, resolution_um: i64) -> u128 {
    denote_lo(program, resolution_um).fingerprint()
}

/// Deposit of one layer: how many distinct extruding paths cover each occupied cell (>= 1). Overlap
/// within a single path counts once; two paths crossing the same cell count twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerDeposit {
    pub z_um: i64,
    pub resolution_um: i64,
    pub cells: BTreeMap<(i64, i64), u32>,
}

/// Per-cell count of distinct extruding paths over a whole program — a strictly finer denotation than
/// [`Occupancy`], which is only the set of touched cells. It catches whole-path duplication or
/// repetition that a set hides (laying the same path down twice compares unequal here).
///
/// It measures geometric coverage *multiplicity*, NOT filament volume. Over-extrusion expressed as a
/// wider bead over the same cells, or as a higher flow/E rate (the IR carries no extrusion-amount
/// axis), moves no cell count and is invisible to both `Deposit` and `Occupancy`. Splitting one path
/// into two abutting paths raises the shared cells from 1 to 2 without changing deposited material,
/// so `Deposit` equality is the intended obligation only for passes that neither split nor merge
/// paths (e.g. pure reordering).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deposit {
    pub layers: Vec<LayerDeposit>,
}

/// Volume of one layer: summed deposited melt volume (mm³) per cell. f64 — compare with
/// [`Volume::approx_eq`], not `==`.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerVolume {
    pub z_um: i64,
    pub resolution_um: i64,
    pub layer_height_um: i64,
    pub cells: BTreeMap<(i64, i64), f64>,
}

/// Per-cell deposited melt volume over a whole program. Unlike [`Occupancy`] (coverage) and
/// [`Deposit`] (path count), this moves when a bead gets wider — surfacing over-/under-extrusion
/// expressed as geometry. It still cannot see commanded flow (E / M221) at fixed geometry, since the
/// IR carries no extrusion-amount axis. f64 volumes; compare with [`Volume::approx_eq`].
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    pub layers: Vec<LayerVolume>,
}

impl Volume {
    /// Per-cell volume equality within `eps` mm³ (values are f64, so `==` is too brittle).
    pub fn approx_eq(&self, other: &Volume, eps: f64) -> bool {
        self.layers.len() == other.layers.len()
            && self.layers.iter().zip(&other.layers).all(|(a, b)| {
                a.z_um == b.z_um
                    && a.resolution_um == b.resolution_um
                    && a.cells.len() == b.cells.len()
                    && a.cells
                        .iter()
                        .all(|(k, va)| b.cells.get(k).is_some_and(|vb| (va - vb).abs() <= eps))
            })
    }
}

/// Squared distance from point `p` to segment `a`-`b`, in f64.
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

/// Canonical (direction-independent) endpoint order for a segment. Everything `mark_capsule`
/// computes depends on a segment only through this ordered pair, so the marked cells are identical
/// forward and backward.
fn canon_seg(p: Point, q: Point) -> (Point, Point) {
    if (p.x, p.y) <= (q.x, q.y) {
        (p, q)
    } else {
        (q, p)
    }
}

/// Mark every grid cell the capsule (segment `p`-`q` thickened by `half`) overlaps.
///
/// Endpoints are canonicalized before any float math, so the marked set is identical forward or
/// reversed. Inclusion is conservative coverage: a cell is marked when the capsule reaches within the
/// cell's circumradius (`r/√2`) of its centre, so no touched cell is ever missed.
fn mark_capsule(cells: &mut BTreeSet<(i64, i64)>, p: Point, q: Point, half: f64, r: i64) {
    let (a, b) = canon_seg(p, q);
    let reach = half + (r as f64) * std::f64::consts::FRAC_1_SQRT_2;
    let pad = reach.ceil() as i64;
    // Saturating so an extreme coordinate can never overflow the bbox.
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
    // Per row, restrict tested columns to a superset of the covered interval: cells whose
    // perpendicular distance to the infinite line a-b is within `reach`. Since distance-to-segment >=
    // perpendicular-distance-to-line, that band contains every covered cell, and each candidate is
    // still decided by the exact `dist2_point_seg` predicate below, so the marked set is identical to
    // a full bounding-box scan. A near-horizontal / degenerate segment falls back to the full row.
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
                // ci = (x - half_r) / r. Round outward, clamp into the bbox range in f64 (avoids an
                // i64-cast overflow for extreme coordinates), then widen by one cell as a
                // float-rounding guard.
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
            // Cell centre in f64: `ci * r` would overflow i64 for extreme coordinates.
            let center = (ci as f64 * rf + half_r, cy);
            if dist2_point_seg(center, af, bf) <= reach2 {
                cells.insert((ci, cj));
            }
        }
    }
}

/// Mark every cell one path covers into `cells` (each segment thickened by half its width).
fn mark_path(cells: &mut BTreeSet<(i64, i64)>, poly: &Polyline, width_um: i64, r: i64) {
    let half = width_um.max(0) as f64 / 2.0; // negative width is malformed; treat as 0
    let pts = &poly.points;
    match pts.len() {
        0 => {}
        1 => mark_capsule(cells, pts[0], pts[0], half, r),
        _ => {
            for seg in pts.windows(2) {
                mark_capsule(cells, seg[0], seg[1], half, r);
            }
        }
    }
}

/// The cells one path covers at resolution `r`, deduplicated within the path (so self-overlap counts
/// once and the set is reversal-invariant).
fn path_cells(poly: &Polyline, width_um: i64, r: i64) -> BTreeSet<(i64, i64)> {
    let mut cells = BTreeSet::new();
    mark_path(&mut cells, poly, width_um, r);
    cells
}

/// The grid cells a single polyline covers at `resolution_um` when laid down at `width_um`. Exposed
/// for analyses over deposited geometry (e.g. checking whether a travel crosses printed material).
pub fn polyline_cells(poly: &Polyline, width_um: i64, resolution_um: i64) -> BTreeSet<(i64, i64)> {
    path_cells(poly, width_um, resolution_um.max(1))
}

/// Rasterize the swept material of a set of (path, width) pairs into a layer occupancy (the union of
/// every path's covered cells). Paths mark directly into one set — no per-path allocation.
fn occupancy_of(paths: &[(&Polyline, i64)], z_um: i64, resolution_um: i64) -> LayerOccupancy {
    let r = resolution_um.max(1);
    let mut cells = BTreeSet::new();
    for (poly, width_um) in paths {
        mark_path(&mut cells, poly, *width_um, r);
    }
    LayerOccupancy {
        z_um,
        resolution_um: r,
        cells,
    }
}

/// Rasterize the same paths into a per-cell deposition count: each path contributes at most 1 to a
/// cell, so the count is the number of distinct paths depositing there.
fn deposit_of(paths: &[(&Polyline, i64)], z_um: i64, resolution_um: i64) -> LayerDeposit {
    let r = resolution_um.max(1);
    let mut cells: BTreeMap<(i64, i64), u32> = BTreeMap::new();
    for (poly, width_um) in paths {
        for cell in path_cells(poly, *width_um, r) {
            *cells.entry(cell).or_insert(0) += 1;
        }
    }
    LayerDeposit {
        z_um,
        resolution_um: r,
        cells,
    }
}

/// Denote a high-level program: union the swept material of every region's fills, per layer.
///
/// Measures deposited filament (the swept `fills`) only; a [`hi::Region`]'s `boundary` is
/// denotationally inert, so equality means "deposits the same material," not "fills the claimed
/// region."
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
/// order-preserving collect keeps the result identical to a serial map.
#[cfg(not(kani))]
fn map_layers<L, T, F>(layers: &[L], f: F) -> Vec<T>
where
    L: Sync,
    T: Send,
    F: Fn(&L) -> T + Sync + Send,
{
    layers.par_iter().map(f).collect()
}

#[cfg(kani)]
fn map_layers<L, T, F>(layers: &[L], f: F) -> Vec<T>
where
    F: Fn(&L) -> T,
{
    layers.iter().map(f).collect()
}

/// Denote a single low-level layer: the union of its extruding toolpaths' swept material. This is the
/// unit of work an incremental denote ([`crate::incremental::DenoteCache`]) recomputes when one layer
/// changes — the whole-program [`denote_lo`] is exactly this mapped over the layers.
pub fn denote_lo_layer(layer: &lo::Layer, resolution_um: i64) -> LayerOccupancy {
    let paths: Vec<(&Polyline, i64)> = layer
        .toolpaths
        .iter()
        .filter(|t| t.kind.extrudes())
        .map(|t| (&t.path, t.width_um))
        .collect();
    occupancy_of(&paths, layer.z_um, resolution_um)
}

/// Denote a low-level program: union the swept material of every extruding toolpath, per layer.
/// Travel contributes nothing.
pub fn denote_lo(program: &lo::Program, resolution_um: i64) -> Occupancy {
    Occupancy {
        layers: map_layers(&program.layers, |layer| {
            denote_lo_layer(layer, resolution_um)
        }),
    }
}

/// Deposition count of a high-level program: how many fills cover each cell, per layer.
pub fn denote_hi_deposit(program: &hi::Program, resolution_um: i64) -> Deposit {
    Deposit {
        layers: map_layers(&program.layers, |layer| {
            let paths: Vec<(&Polyline, i64)> = layer
                .regions
                .iter()
                .flat_map(|reg| reg.fills.iter().map(|f| (&f.path, f.width_um)))
                .collect();
            deposit_of(&paths, layer.z_um, resolution_um)
        }),
    }
}

/// Deposition count of a low-level program: how many extruding toolpaths cover each cell, per layer.
/// Unlike [`denote_lo`], a program that deposits the same path twice does not compare equal.
pub fn denote_lo_deposit(program: &lo::Program, resolution_um: i64) -> Deposit {
    Deposit {
        layers: map_layers(&program.layers, |layer| {
            let paths: Vec<(&Polyline, i64)> = layer
                .toolpaths
                .iter()
                .filter(|t| t.kind.extrudes())
                .map(|t| (&t.path, t.width_um))
                .collect();
            deposit_of(&paths, layer.z_um, resolution_um)
        }),
    }
}

/// Whether lowering preserves denotation: `denote_hi(prog) == denote_lo(lower(prog))` at the given
/// resolution.
pub fn self_lowering_sound(program: &hi::Program, resolution_um: i64) -> bool {
    denote_hi(program, resolution_um) == denote_lo(&crate::lower::lower(program), resolution_um)
}

/// Per-layer height (microns) from consecutive z (layer 0 uses its own z), or a fixed override.
/// Matches the backend's `h = z - prev_z` rule so volume agrees with the emitted flow.
fn layer_heights(zs: impl Iterator<Item = i64>, override_um: Option<i64>) -> Vec<i64> {
    let mut prev: Option<i64> = None;
    zs.map(|z| {
        let h = override_um.unwrap_or_else(|| match prev {
            Some(p) => (z - p).max(1),
            None => z.max(1),
        });
        prev = Some(z);
        h
    })
    .collect()
}

/// Rasterize deposited melt volume (mm³) per cell for one layer: each segment's volume
/// (width × layer-height × length) spread uniformly over the cells it covers. Total is conserved per
/// path regardless of resolution.
fn volume_of(
    paths: &[(&Polyline, i64)],
    z_um: i64,
    resolution_um: i64,
    layer_height_um: i64,
) -> LayerVolume {
    let r = resolution_um.max(1);
    let h_mm = layer_height_um.max(1) as f64 / 1000.0;
    let mut cells: BTreeMap<(i64, i64), f64> = BTreeMap::new();
    for (poly, width_um) in paths {
        let w_mm = (*width_um).max(0) as f64 / 1000.0;
        let half = (*width_um).max(0) as f64 / 2.0;
        for seg in poly.points.windows(2) {
            let mut seg_cells = BTreeSet::new();
            mark_capsule(&mut seg_cells, seg[0], seg[1], half, r);
            let n = seg_cells.len();
            if n == 0 {
                continue;
            }
            let dx = (seg[1].x - seg[0].x) as f64;
            let dy = (seg[1].y - seg[0].y) as f64;
            let len_mm = (dx * dx + dy * dy).sqrt() / 1000.0;
            let per = w_mm * h_mm * len_mm / n as f64;
            for c in seg_cells {
                *cells.entry(c).or_insert(0.0) += per;
            }
        }
    }
    LayerVolume {
        z_um,
        resolution_um: r,
        layer_height_um: layer_height_um.max(1),
        cells,
    }
}

/// Deposited melt volume (mm³) per cell for a low-level program. Layer height is derived from
/// consecutive z (matching the backend) unless `layer_height_um` is given.
pub fn denote_lo_volume(
    program: &lo::Program,
    resolution_um: i64,
    layer_height_um: Option<i64>,
) -> Volume {
    let heights = layer_heights(program.layers.iter().map(|l| l.z_um), layer_height_um);
    let layers = program
        .layers
        .iter()
        .zip(heights)
        .map(|(layer, h)| {
            let paths: Vec<(&Polyline, i64)> = layer
                .toolpaths
                .iter()
                .filter(|t| t.kind.extrudes())
                .map(|t| (&t.path, t.width_um))
                .collect();
            volume_of(&paths, layer.z_um, resolution_um, h)
        })
        .collect();
    Volume { layers }
}

/// Deposited melt volume (mm³) per cell for a high-level program.
pub fn denote_hi_volume(
    program: &hi::Program,
    resolution_um: i64,
    layer_height_um: Option<i64>,
) -> Volume {
    let heights = layer_heights(program.layers.iter().map(|l| l.z_um), layer_height_um);
    let layers = program
        .layers
        .iter()
        .zip(heights)
        .map(|(layer, h)| {
            let paths: Vec<(&Polyline, i64)> = layer
                .regions
                .iter()
                .flat_map(|reg| reg.fills.iter().map(|f| (&f.path, f.width_um)))
                .collect();
            volume_of(&paths, layer.z_um, resolution_um, h)
        })
        .collect();
    Volume { layers }
}

/// Bounded proofs discharged by Kani (`cargo kani -p kerf-core`), not run by `cargo test`. Each
/// harness targets a pure, loop- and allocation-free kernel so the model checker terminates.
#[cfg(kani)]
mod kani_proofs {
    use super::{canon_seg, dist2_point_seg};
    use crate::ir::Point;

    /// Canonical order is identical regardless of input order, for all `i64` endpoints — the root of
    /// reversal invariance.
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

    /// Squared distance is non-negative and finite for finite inputs, so no NaN or negative value can
    /// leak into the `dist <= reach²` coverage comparison.
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
    fn deposit_counts_repeated_paths_but_occupancy_does_not() {
        let seg = Polyline::new(vec![Point::new(0, 0), Point::new(10_000, 0)]);
        let once = [(&seg, 400i64)];
        let twice = [(&seg, 400i64), (&seg, 400i64)];
        // A set unions to the same cells whether the path is laid once or twice.
        assert_eq!(
            occupancy_of(&once, 200, 200),
            occupancy_of(&twice, 200, 200)
        );
        // Deposition count sees the difference: every covered cell is hit once vs. twice.
        let d1 = deposit_of(&once, 200, 200);
        let d2 = deposit_of(&twice, 200, 200);
        assert_ne!(d1, d2);
        assert!(!d1.cells.is_empty());
        assert_eq!(
            d1.cells.keys().collect::<Vec<_>>(),
            d2.cells.keys().collect::<Vec<_>>()
        );
        assert!(d1.cells.values().all(|&c| c == 1));
        assert!(d2.cells.values().all(|&c| c == 2));
    }

    #[test]
    fn occupancy_and_deposit_agree_on_covered_cells() {
        // occupancy_of and deposit_of must mark the same cells; deposit just also counts them.
        let a = Polyline::new(vec![Point::new(0, 0), Point::new(9000, 4000)]);
        let b = Polyline::new(vec![Point::new(2000, 0), Point::new(2000, 8000)]);
        let paths = [(&a, 400i64), (&b, 300i64)];
        let occ = occupancy_of(&paths, 200, 200);
        let dep_keys: BTreeSet<(i64, i64)> =
            deposit_of(&paths, 200, 200).cells.keys().copied().collect();
        assert_eq!(occ.cells, dep_keys);
    }

    #[test]
    fn deposit_is_reversal_invariant() {
        let fwd = Polyline::new(vec![
            Point::new(0, 0),
            Point::new(7000, 3000),
            Point::new(9000, 0),
        ]);
        let mut rev_pts = fwd.points.clone();
        rev_pts.reverse();
        let rev = Polyline::new(rev_pts);
        assert_eq!(
            deposit_of(&[(&fwd, 400)], 200, 200),
            deposit_of(&[(&rev, 400)], 200, 200)
        );
    }

    #[test]
    fn volume_moves_with_bead_width_unlike_occupancy_or_deposit() {
        let seg = Polyline::new(vec![Point::new(0, 0), Point::new(20_000, 0)]);
        let total = |w: i64| -> f64 { volume_of(&[(&seg, w)], 200, 200, 200).cells.values().sum() };
        // A wider bead deposits proportionally more volume over the same span — the exact thing
        // Occupancy (a set) and Deposit (a path count) are blind to.
        assert!(total(800) > total(400) * 1.5);
    }

    #[test]
    fn volume_is_conserved_across_resolution_and_reversal_invariant() {
        let seg = Polyline::new(vec![Point::new(0, 0), Point::new(30_000, 7000)]);
        let len_mm = ((30_000f64).powi(2) + (7000f64).powi(2)).sqrt() / 1000.0;
        let expect = 0.4 * 0.2 * len_mm; // width 400um, height 200um
        for r in [100, 200, 500, 1000] {
            let v: f64 = volume_of(&[(&seg, 400)], 200, r, 200).cells.values().sum();
            assert!((v - expect).abs() < 1e-6, "res {r}: {v} != {expect}");
        }
        let mut rp = seg.points.clone();
        rp.reverse();
        let rev = Polyline::new(rp);
        let a = Volume {
            layers: vec![volume_of(&[(&seg, 400)], 200, 200, 200)],
        };
        let b = Volume {
            layers: vec![volume_of(&[(&rev, 400)], 200, 200, 200)],
        };
        assert!(a.approx_eq(&b, 1e-9));
    }

    #[test]
    fn lowering_preserves_volume() {
        let p = square_program();
        assert!(denote_hi_volume(&p, 200, None)
            .approx_eq(&denote_lo_volume(&crate::lower::lower(&p), 200, None), 1e-9));
    }

    #[test]
    fn deposit_counts_paths_not_filament_so_a_single_path_is_always_one_per_cell() {
        // Documented limitation, pinned: deposit measures how many distinct paths cover a cell, not
        // how much material a path lays there. A single path — thin or fat — counts exactly 1
        // everywhere, so over-extrusion expressed only as a wider bead is invisible to the oracle.
        let seg = Polyline::new(vec![Point::new(0, 0), Point::new(6000, 0)]);
        for w in [200i64, 400, 800, 1600] {
            let d = deposit_of(&[(&seg, w)], 200, 200);
            assert!(!d.cells.is_empty());
            assert!(
                d.cells.values().all(|&c| c == 1),
                "width {w}: a single path must count 1 per cell"
            );
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
        // A 0.4 mm wall at coarse (1 mm) resolution: conservative coverage marks it.
        let wall = single_path(vec![Point::new(1000, 0), Point::new(1000, 10_000)], 400);
        assert!(!denote_hi(&wall, 1000).layers[0].cells.is_empty());
        assert_ne!(
            denote_hi(&wall, 1000),
            denote_hi(&single_path(vec![], 0), 1000)
        );
    }

    #[test]
    fn denote_is_reversal_invariant() {
        // Zero width / resolution 1: the float-boundary regime where endpoint order flips a cell if
        // canonicalization is removed.
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
        // Width must affect denotation, else a width-changing pass would pass vacuously.
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
        // resolution 1: the sharpest grid, where float-boundary asymmetry surfaces first.
        assert_eq!(
            denote_hi(&fwd, 1),
            denote_hi(&rev, 1),
            "reversal changed denotation for {pts:?} w={width_um}"
        );
    }

    #[test]
    fn reversal_invariant_exhaustively_over_small_programs() {
        // Reversal invariance by enumeration: every 2- and 3-point polyline over a small coordinate
        // grid, at resolution 1, must denote identically forwards and backwards.
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
        // The perpendicular-band column pruning in `mark_capsule` must mark exactly the cells a full
        // bounding-box scan marks. Reuses `canon_seg` / `dist2_point_seg` so the reference is
        // bit-identical.
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
        // The check is only as sharp as its resolution.
        let base = single_path(vec![Point::new(0, 500), Point::new(3000, 500)], 400);
        let nudged = single_path(vec![Point::new(0, 500), Point::new(3001, 500)], 400); // +1 µm
        let moved = single_path(vec![Point::new(0, 500), Point::new(3000, 4000)], 400); // +3.5 mm

        // A sub-resolution nudge is not distinguished at a coarse resolution.
        assert_eq!(denote_hi(&base, 1000), denote_hi(&nudged, 1000));
        // A change larger than the resolution is caught.
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

        // Lowering never changes what material is deposited, over arbitrary programs.
        #[test]
        fn lowering_preserves_denotation(prog in arb_program()) {
            prop_assert!(self_lowering_sound(&prog, 300));
        }

        // denote is invariant under path reversal (broad check). The deterministic guard is the
        // `denote_is_reversal_invariant` unit test; a random proptest does not reliably hit the float
        // knife-edge.
        #[test]
        fn reversal_invariant(prog in arb_program()) {
            prop_assert_eq!(denote_hi(&prog, 100), denote_hi(&reverse_all_paths(prog.clone()), 100));
        }
    }
}
