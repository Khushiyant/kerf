//! Incremental denotation: a layer-level occupancy cache with dirty tracking.
//!
//! Denotation is embarrassingly per-layer — [`denote_lo`](crate::denote::denote_lo) is
//! [`denote_lo_layer`](crate::denote::denote_lo_layer) mapped over the layers. A single-layer edit
//! therefore only needs that one layer re-rasterized, not all of them. [`DenoteCache`] holds one
//! cached [`LayerOccupancy`] per layer; a mutation reports which layers it touched via [`mark_dirty`],
//! and [`occupancy`] recomputes only the dirty ones. The result is bit-identical to a full
//! [`denote_lo`] at the same resolution — this is a pure performance layer, never a semantic one.
//!
//! [`mark_dirty`]: DenoteCache::mark_dirty
//! [`occupancy`]: DenoteCache::occupancy

use crate::denote::{denote_lo, denote_lo_layer, LayerOccupancy, Occupancy};
use crate::ir::lo;

/// A per-layer occupancy cache keyed to a fixed resolution. It holds a *materialized* occupancy and a
/// per-layer dirty bit; [`DenoteCache::occupancy`] re-rasterizes only the dirty layers in place and
/// returns a borrow, so an edit costs one layer's work — not a full re-denote and not even a full
/// clone of the unchanged layers.
#[derive(Clone, Debug)]
pub struct DenoteCache {
    resolution_um: i64,
    occ: Occupancy,
    dirty: Vec<bool>,
    fps: Vec<u64>, // per-layer fingerprints, kept in sync with occ.layers
}

impl DenoteCache {
    /// A cache for the given raster resolution. Empty until the first [`DenoteCache::occupancy`].
    pub fn new(resolution_um: i64) -> Self {
        Self {
            resolution_um: resolution_um.max(1),
            occ: Occupancy { layers: Vec::new() },
            dirty: Vec::new(),
            fps: Vec::new(),
        }
    }

    /// The resolution (microns) this cache is keyed to.
    pub fn resolution_um(&self) -> i64 {
        self.resolution_um
    }

    /// Number of cached layers currently clean (not awaiting recompute).
    pub fn clean_layers(&self) -> usize {
        self.dirty.iter().filter(|&&d| !d).count()
    }

    /// Mark one layer dirty, so it is recomputed on the next [`DenoteCache::occupancy`].
    pub fn mark_dirty(&mut self, layer_idx: usize) {
        if let Some(d) = self.dirty.get_mut(layer_idx) {
            *d = true;
        }
    }

    /// Mark every layer dirty (a full recompute next call). Use after a change whose layer extent is
    /// unknown, or after resolution-independent structural edits.
    pub fn mark_all_dirty(&mut self) {
        for d in &mut self.dirty {
            *d = true;
        }
    }

    /// The program's occupancy, recomputing only the dirty layers, returned by reference. Bit-identical
    /// to [`denote_lo`](crate::denote::denote_lo) at this resolution.
    ///
    /// If the program's layer count changed since the last call every layer is recomputed (indices no
    /// longer line up); for in-place edits that keep the layer count, mark the touched layers and only
    /// those re-rasterize.
    pub fn occupancy(&mut self, program: &lo::Program) -> &Occupancy {
        let r = self.resolution_um;
        if self.occ.layers.len() != program.layers.len() {
            self.occ = denote_lo(program, r);
            self.dirty = vec![false; program.layers.len()];
            self.fps = self.occ.layer_fingerprints();
            return &self.occ;
        }
        let idxs: Vec<usize> = self
            .dirty
            .iter()
            .enumerate()
            .filter(|(_, &d)| d)
            .map(|(i, _)| i)
            .collect();
        for (i, layer) in idxs.iter().copied().zip(compute_dirty(&idxs, program, r)) {
            self.fps[i] = layer.fingerprint(); // re-fingerprint only the recomputed layer
            self.occ.layers[i] = layer;
            self.dirty[i] = false;
        }
        &self.occ
    }

    /// The program's occupancy as an owned value (a clone of [`DenoteCache::occupancy`]) — when you
    /// need to keep it past the next edit.
    pub fn occupancy_cloned(&mut self, program: &lo::Program) -> Occupancy {
        self.occupancy(program).clone()
    }

    /// The program's 128-bit **material fingerprint**, maintained incrementally: only layers marked
    /// dirty since the last call are re-rasterized and re-hashed; the digest is then a combine of the
    /// cached per-layer hashes. This turns a preservation / "same material" verdict from a full
    /// re-denote (tens–hundreds of ms) into microseconds — the check an RL env runs every step.
    pub fn fingerprint(&mut self, program: &lo::Program) -> u128 {
        let _ = self.occupancy(program); // brings occ + fps up to date (dirty layers only)
        crate::denote::combine_fingerprints(&self.fps)
    }
}

/// Recompute the dirty layers' occupancies, in parallel across cores off the Kani model checker
/// (which needs a single-threaded, allocation-lean build).
#[cfg(not(kani))]
fn compute_dirty(dirty: &[usize], program: &lo::Program, r: i64) -> Vec<LayerOccupancy> {
    use rayon::prelude::*;
    dirty
        .par_iter()
        .map(|&i| denote_lo_layer(&program.layers[i], r))
        .collect()
}

#[cfg(kani)]
fn compute_dirty(dirty: &[usize], program: &lo::Program, r: i64) -> Vec<LayerOccupancy> {
    dirty
        .iter()
        .map(|&i| denote_lo_layer(&program.layers[i], r))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::denote::denote_lo;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn layer(z: i64, x: i64) -> Layer {
        Layer {
            z_um: z,
            toolpaths: vec![Toolpath::extrude(
                SegmentKind::Extrude(RegionKind::Perimeter),
                Polyline::new(vec![Point::new(x, 0), Point::new(x + 10_000, 5_000)]),
                400,
            )],
        }
    }

    fn stack(n: i64) -> lo::Program {
        lo::Program {
            layers: (0..n).map(|k| layer((k + 1) * 200, k * 100)).collect(),
        }
    }

    #[test]
    fn incremental_matches_full_denote_after_a_single_layer_edit() {
        let mut prog = stack(100);
        let mut cache = DenoteCache::new(200);
        // Prime the cache (full compute).
        assert_eq!(cache.occupancy(&prog), &denote_lo(&prog, 200));
        assert_eq!(cache.clean_layers(), 100);

        // Edit one layer's geometry and mark only it dirty.
        prog.layers[42].toolpaths[0].path = Polyline::new(vec![
            Point::new(0, 0),
            Point::new(5_000, 15_000),
            Point::new(9_000, 1_000),
        ]);
        cache.mark_dirty(42);

        // The incremental result is bit-identical to a from-scratch denote.
        assert_eq!(cache.occupancy(&prog), &denote_lo(&prog, 200));
    }

    #[test]
    fn incremental_fingerprint_matches_one_shot_and_tracks_edits() {
        use crate::denote::material_fingerprint;
        let mut prog = stack(60);
        let mut cache = DenoteCache::new(200);
        assert_eq!(cache.fingerprint(&prog), material_fingerprint(&prog, 200));

        // Edit one layer, mark it dirty: the incremental fingerprint re-hashes only that layer and
        // still equals a from-scratch fingerprint of the edited program.
        prog.layers[30].toolpaths[0].path =
            Polyline::new(vec![Point::new(1_000, 2_000), Point::new(8_000, 9_000)]);
        cache.mark_dirty(30);
        assert_eq!(cache.fingerprint(&prog), material_fingerprint(&prog, 200));
    }

    #[test]
    fn reordering_and_reversing_leave_the_fingerprint_unchanged() {
        use crate::denote::material_fingerprint;
        // A layer with several toolpaths; reorder + reverse them. denote is order/reversal-invariant,
        // so the material fingerprint is identical — the fast preservation verdict for the RL env.
        let mk = |x: i64, rev: bool| {
            let mut pts = vec![
                Point::new(x, 0),
                Point::new(x + 6_000, 3_000),
                Point::new(x, 6_000),
            ];
            if rev {
                pts.reverse();
            }
            Toolpath::extrude(
                SegmentKind::Extrude(RegionKind::Infill),
                Polyline::new(pts),
                400,
            )
        };
        let base = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![mk(0, false), mk(20_000, false), mk(40_000, false)],
            }],
        };
        let shuffled = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![mk(40_000, true), mk(0, true), mk(20_000, false)],
            }],
        };
        assert_eq!(
            material_fingerprint(&base, 200),
            material_fingerprint(&shuffled, 200),
            "reorder + reverse preserves material, so the fingerprint is unchanged"
        );
        // A genuinely different program must differ.
        let moved = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![mk(0, false), mk(20_000, false), mk(41_000, false)],
            }],
        };
        assert_ne!(
            material_fingerprint(&base, 200),
            material_fingerprint(&moved, 200)
        );
    }

    #[test]
    fn only_dirty_layers_are_recomputed() {
        let prog = stack(10);
        let mut cache = DenoteCache::new(200);
        cache.occupancy(&prog);
        cache.mark_dirty(3);
        assert_eq!(cache.clean_layers(), 9, "only the dirty layer is uncached");
        cache.occupancy(&prog);
        assert_eq!(cache.clean_layers(), 10);
    }

    #[test]
    fn a_layer_count_change_forces_a_full_recompute_and_stays_correct() {
        let prog = stack(5);
        let mut cache = DenoteCache::new(300);
        cache.occupancy(&prog);
        let bigger = stack(8);
        assert_eq!(cache.occupancy(&bigger), &denote_lo(&bigger, 300));
    }

    #[test]
    fn a_single_layer_re_denote_is_far_cheaper_than_a_full_one() {
        // Wall-clock ratio on the same machine: re-denoting one dirty layer vs a full re-denote of a
        // deep stack. Full denote parallelizes across layers, so the ratio is bounded by (layers /
        // cores); at 300 layers this clears 8x on anything up to ~37 cores (and is >50x on the 2-4
        // core CI runners). The underlying *work* reduction is ~300x (one layer rasterized, not 300).
        use std::time::Instant;
        let prog = stack(300);
        let mut cache = DenoteCache::new(100);
        cache.occupancy(&prog); // prime

        let reps = 20;
        let t_full = Instant::now();
        for _ in 0..reps {
            let _ = denote_lo(&prog, 100);
        }
        let full = t_full.elapsed().as_secs_f64() / reps as f64;

        let t_inc = Instant::now();
        for _ in 0..reps {
            cache.mark_dirty(150);
            let _ = cache.occupancy(&prog);
        }
        let inc = t_inc.elapsed().as_secs_f64() / reps as f64;

        // >=5x is robust across core counts (full denote parallelizes, so wall-clock speedup is
        // bounded by layers/cores; on the 2-4 core CI runners this is 50x+). The work reduction is
        // ~300x regardless. This gate catches a regression that reintroduces full recompute per edit.
        assert!(
            full > inc * 5.0,
            "single-layer re-denote should be >=5x faster: full={full:.6}s inc={inc:.6}s ({:.1}x)",
            full / inc.max(1e-9)
        );
    }
}
