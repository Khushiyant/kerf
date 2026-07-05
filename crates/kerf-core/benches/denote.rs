//! Benchmarks for the denotation hot path — the loop an RL reward/verify call runs thousands of times.
//! Run with `cargo bench -p kerf-core`.

use criterion::{criterion_group, criterion_main, Criterion};
use kerf_core::denote::{denote_lo, denote_lo_deposit};
use kerf_core::incremental::DenoteCache;
use kerf_core::ir::lo::{Layer, Program, SegmentKind, Toolpath};
use kerf_core::ir::{Point, Polyline, RegionKind};
use kerf_core::verify::{verify_batch, verify_roundtrip};

/// A grid of parallel infill lines: `layers` layers, `lines` lines each, every line 100 mm long.
fn gen(layers: usize, lines: usize) -> Program {
    Program {
        layers: (0..layers)
            .map(|li| Layer {
                z_um: 200 + li as i64 * 200,
                toolpaths: (0..lines)
                    .map(|i| Toolpath {
                        kind: SegmentKind::Extrude(RegionKind::Infill),
                        path: Polyline::new(vec![
                            Point::new(0, i as i64 * 500),
                            Point::new(100_000, i as i64 * 500),
                        ]),
                        width_um: 400,
                        flow_e: None,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn bench(c: &mut Criterion) {
    let small = gen(20, 100); // ~2k extruding paths — RL-step scale
    let medium = gen(50, 400); // ~20k paths — a whole part

    c.bench_function("denote_lo/2k", |b| b.iter(|| denote_lo(&small, 200)));
    c.bench_function("denote_lo/20k", |b| b.iter(|| denote_lo(&medium, 200)));
    c.bench_function("denote_lo_deposit/2k", |b| {
        b.iter(|| denote_lo_deposit(&small, 200))
    });
    c.bench_function("verify_roundtrip/2k", |b| {
        b.iter(|| verify_roundtrip(&small, 200))
    });

    // Incremental re-denote after a single-layer edit vs a full denote of the same 50-layer part —
    // the RL/search hot path. The gap is the whole point of the layer cache.
    c.bench_function("denote_lo/full-50layer", |b| {
        b.iter(|| denote_lo(&medium, 200))
    });
    c.bench_function("incremental/single-layer-edit", |b| {
        let mut cache = DenoteCache::new(200);
        cache.occupancy(&medium); // prime once
        b.iter(|| {
            cache.mark_dirty(25);
            criterion::black_box(cache.occupancy(&medium).layers.len())
        })
    });

    // Batched verification of 64 candidates against a reference (population-based search shape).
    let candidates: Vec<Program> = (0..64).map(|_| small.clone()).collect();
    c.bench_function("verify_batch/64x2k", |b| {
        b.iter(|| verify_batch(&candidates, &small, 200))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
