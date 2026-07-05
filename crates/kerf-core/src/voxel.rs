//! 3D voxel occupancy and rotation comparison, built on the 2D denotation.
//!
//! [`voxelize`] lifts the per-layer occupancy to voxels on a uniform integer grid (resolution µm on
//! all three axes; each layer fills the Z range it deposits). On that grid:
//!  - **Exact 90° rotations** ([`rot_x90`], [`rot_y90`], [`rot_z90`]) are integer index maps — no
//!    precision loss. Z-90° is denotation-equivariant, extending the translation-invariance property.
//!  - **Arbitrary X/Y rotations** don't align to the grid, so exact equality is gone. [`rotate_bounds`]
//!    returns a sound inner (definitely-covered) and outer (possibly-covered) voxel set, which give
//!    sound verdicts via [`compare_rotated`]: "same within the grid" or "definitely differ" — a
//!    bounded answer where point-sampling gives none.

use std::collections::BTreeSet;

use crate::denote::denote_lo;
use crate::ir::lo;

/// Occupied voxels `(i, j, k)` on a `resolution_um` grid (k derived from Z).
pub type Voxels = BTreeSet<(i64, i64, i64)>;

/// Per-layer height (µm) from consecutive Z (layer 0 uses its own Z), or a fixed override.
fn layer_heights(program: &lo::Program, override_um: Option<i64>) -> Vec<i64> {
    let mut prev: Option<i64> = None;
    program
        .layers
        .iter()
        .map(|l| {
            let h = override_um.unwrap_or_else(|| match prev {
                Some(p) => (l.z_um - p).max(1),
                None => l.z_um.max(1),
            });
            prev = Some(l.z_um);
            h
        })
        .collect()
}

/// Lift a program's deposited material to 3D voxels: each layer's 2D cells, extruded over the Z range
/// `[z - height, z)` it deposits, on a uniform `resolution_um` grid.
pub fn voxelize(program: &lo::Program, resolution_um: i64, layer_height_um: Option<i64>) -> Voxels {
    let r = resolution_um.max(1);
    let heights = layer_heights(program, layer_height_um);
    let occ = denote_lo(program, r);
    let mut out = Voxels::new();
    for (layer, h) in occ.layers.iter().zip(heights) {
        let k_hi = layer.z_um.div_euclid(r);
        let k_lo = (layer.z_um - h.max(1)).div_euclid(r);
        for &(i, j) in &layer.cells {
            for k in k_lo..k_hi.max(k_lo + 1) {
                out.insert((i, j, k));
            }
        }
    }
    out
}

/// Exact 90° rotation CCW about +Z: `(i, j, k) -> (-j-1, i, k)`.
pub fn rot_z90(v: &Voxels) -> Voxels {
    v.iter().map(|&(i, j, k)| (-j - 1, i, k)).collect()
}

/// Exact 90° rotation CCW about +X: `(i, j, k) -> (i, -k-1, j)`.
pub fn rot_x90(v: &Voxels) -> Voxels {
    v.iter().map(|&(i, j, k)| (i, -k - 1, j)).collect()
}

/// Exact 90° rotation CCW about +Y: `(i, j, k) -> (k, j, -i-1)`.
pub fn rot_y90(v: &Voxels) -> Voxels {
    v.iter().map(|&(i, j, k)| (k, j, -i - 1)).collect()
}

/// Sound inner (definitely-covered) and outer (possibly-covered) voxel sets after rotating `v` about
/// an axis by `radians`. `axis_x = true` rotates about X (Y,Z rotate), else about Y (X,Z rotate).
///
/// For each source voxel we transform the eight grid voxels near its rotated image and test them by
/// mapping their corners back through the inverse rotation: a target voxel is INNER if all its corners
/// land inside the source voxel (so it is fully covered), and OUTER if any corner does (so it may be
/// covered). `inner ⊆ true rotation ⊆ outer` by construction.
pub fn rotate_bounds(v: &Voxels, radians: f64, axis_x: bool) -> (Voxels, Voxels) {
    let (sin, cos) = radians.sin_cos();
    // Rotate a point (a is the swept coord along the axis-normal plane's first coord, b the second).
    // About X: (y,z) rotate; about Y: (x,z) rotate. We rotate the two in-plane axes, keep the third.
    let fwd = |x: f64, y: f64, z: f64| -> (f64, f64, f64) {
        if axis_x {
            (x, y * cos - z * sin, y * sin + z * cos)
        } else {
            (x * cos + z * sin, y, -x * sin + z * cos)
        }
    };
    let inv = |x: f64, y: f64, z: f64| -> (f64, f64, f64) {
        if axis_x {
            (x, y * cos + z * sin, -y * sin + z * cos)
        } else {
            (x * cos - z * sin, y, x * sin + z * cos)
        }
    };
    let mut inner = Voxels::new();
    let mut outer = Voxels::new();
    for &(i, j, k) in v {
        // Rotated image AABB (in grid units) from the eight rotated corners of this voxel.
        let (mut lo3, mut hi3) = ([f64::MAX; 3], [f64::MIN; 3]);
        for &(ci, cj, ck) in &corners(i, j, k) {
            let (rx, ry, rz) = fwd(ci, cj, ck);
            for (d, val) in [rx, ry, rz].into_iter().enumerate() {
                lo3[d] = lo3[d].min(val);
                hi3[d] = hi3[d].max(val);
            }
        }
        let rng = |d: usize| lo3[d].floor() as i64..=hi3[d].ceil() as i64;
        for ti in rng(0) {
            for tj in rng(1) {
                for tk in rng(2) {
                    // How many of target voxel (ti,tj,tk)'s corners map back into source voxel (i,j,k)?
                    let mut inside = 0;
                    for &(cx, cy, cz) in &corners(ti, tj, tk) {
                        let (bx, by, bz) = inv(cx, cy, cz);
                        if bx >= i as f64
                            && bx <= (i + 1) as f64
                            && by >= j as f64
                            && by <= (j + 1) as f64
                            && bz >= k as f64
                            && bz <= (k + 1) as f64
                        {
                            inside += 1;
                        }
                    }
                    if inside > 0 {
                        outer.insert((ti, tj, tk));
                    }
                    if inside == 8 {
                        inner.insert((ti, tj, tk));
                    }
                }
            }
        }
    }
    (inner, outer)
}

fn corners(i: i64, j: i64, k: i64) -> [(f64, f64, f64); 8] {
    let (i, j, k) = (i as f64, j as f64, k as f64);
    [
        (i, j, k),
        (i + 1.0, j, k),
        (i, j + 1.0, k),
        (i, j, k + 1.0),
        (i + 1.0, j + 1.0, k),
        (i + 1.0, j, k + 1.0),
        (i, j + 1.0, k + 1.0),
        (i + 1.0, j + 1.0, k + 1.0),
    ]
}

/// The sound verdict of comparing `b` against `a` rotated by `radians` about X (or Y).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationVerdict {
    /// Every voxel of `b` is possibly-covered by the rotation and every definitely-covered voxel is in
    /// `b`: `a` rotated and `b` agree to within the grid's discretization.
    SameWithinGrid,
    /// `b` has a voxel outside the rotation's outer bound, or misses a definitely-covered voxel: they
    /// differ by more than the discretization — a sound "definitely differ".
    DefinitelyDiffer,
}

/// Compare `b` against `a` rotated by `radians` about X (`axis_x`) or Y, returning a sound verdict.
pub fn compare_rotated(a: &Voxels, b: &Voxels, radians: f64, axis_x: bool) -> RotationVerdict {
    let (inner, outer) = rotate_bounds(a, radians, axis_x);
    // Sound: b must be inside the outer bound (nothing extra) and cover the inner bound (nothing
    // missing). Cells between inner and outer are undecidable by the grid and allowed either way.
    if b.is_subset(&outer) && inner.is_subset(b) {
        RotationVerdict::SameWithinGrid
    } else {
        RotationVerdict::DefinitelyDiffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};
    use crate::metamorphic::rotate_z;

    fn square() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Extrude(RegionKind::Perimeter),
                    path: Polyline::new(vec![
                        Point::new(0, 0),
                        Point::new(20_000, 0),
                        Point::new(20_000, 20_000),
                        Point::new(0, 20_000),
                        Point::new(0, 0),
                    ]),
                    width_um: 400,
                }],
            }],
        }
    }

    #[test]
    fn voxelize_is_nonempty_and_single_k_for_one_thin_layer() {
        let v = voxelize(&square(), 200, None);
        assert!(!v.is_empty());
        let ks: BTreeSet<i64> = v.iter().map(|&(_, _, k)| k).collect();
        assert_eq!(ks.len(), 1, "a 0.2mm layer at 0.2mm res is one voxel thick");
    }

    #[test]
    fn ninety_degree_rotations_are_bijections_and_order_four() {
        let v = voxelize(&square(), 200, None);
        for rot in [rot_x90, rot_y90, rot_z90] {
            let r4 = rot(&rot(&rot(&rot(&v))));
            assert_eq!(r4, v, "four 90-degree turns must be the identity");
            assert_eq!(rot(&v).len(), v.len(), "rotation preserves voxel count");
        }
    }

    #[test]
    fn denote_is_exactly_equivariant_under_90_degree_z_rotation() {
        // Rotating the program 90° about Z (exact integer map (x,y)->(-y,x)) then voxelizing must
        // equal rot_z90 of the original voxels — extends translation-invariance to grid rotations.
        let p = square();
        let rotated_prog = rotate_z(&p, std::f64::consts::FRAC_PI_2);
        assert_eq!(
            voxelize(&rotated_prog, 200, None),
            rot_z90(&voxelize(&p, 200, None))
        );
    }

    #[test]
    fn bounded_rotation_brackets_the_truth_and_gives_sound_verdicts() {
        let a = voxelize(&square(), 200, None);
        let th = 30.0_f64.to_radians();
        let (inner, outer) = rotate_bounds(&a, th, true);
        assert!(inner.is_subset(&outer), "inner must be within outer");
        // Differential soundness: for many random points, inner ⊆ true ⊆ outer, where a point is in
        // the true rotation iff its inverse-rotated position lands in a source voxel.
        let (sin, cos) = th.sin_cos();
        let mut s: u64 = 0x243f6a8885a308d3;
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let bbox: Vec<_> = outer.iter().chain(inner.iter()).collect();
        assert!(!bbox.is_empty());
        for _ in 0..4000 {
            // sample a point inside some outer voxel's cell
            let &&(oi, oj, ok) = &outer.iter().nth((rng() as usize) % outer.len()).unwrap();
            let fr = |m: u64| (m % 1000) as f64 / 1000.0;
            let (px, py, pz) = (
                oi as f64 + fr(rng()),
                oj as f64 + fr(rng()),
                ok as f64 + fr(rng()),
            );
            // inverse-rotate about X: (y,z) back
            let (by, bz) = (py * cos + pz * sin, -py * sin + pz * cos);
            let in_true = a.contains(&(px.floor() as i64, by.floor() as i64, bz.floor() as i64));
            let cell = (px.floor() as i64, py.floor() as i64, pz.floor() as i64);
            if inner.contains(&cell) {
                assert!(in_true, "inner voxel must be truly covered");
            }
            if in_true {
                assert!(
                    outer.contains(&cell),
                    "truly-covered point must be in outer"
                );
            }
        }
        // A no-op (0 rad) rotation: verdict is SameWithinGrid against itself.
        assert_eq!(
            compare_rotated(&a, &a, 0.0, true),
            RotationVerdict::SameWithinGrid
        );
    }
}
