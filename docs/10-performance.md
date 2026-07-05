# Performance

Denote/verify latency is a load-bearing property: a search or RL loop calls it thousands of times. It
is guarded two ways in CI (`.github/workflows/ci.yml`):

- **Compile gate** — `cargo bench --workspace --no-run` builds every benchmark, so a bench never
  bit-rots.
- **Regression gate** — the release-mode test `a_single_layer_re_denote_is_far_cheaper_than_a_full_one`
  asserts the incremental cache stays far cheaper than a full re-denote. It fails if a change
  reintroduces full recompute per edit.

Benchmarks live in `crates/kerf-core/benches/denote.rs`; run them with
`cargo bench -p kerf-core --bench denote`.

## Reference numbers

Apple-silicon laptop (8 performance cores), `--release`, resolution 200 µm. Treat as ballpark; the
CI runners (2–4 cores) differ, but the *ratios* hold.

| Benchmark | Scale | Time |
|---|---|---|
| `denote_lo` | 2k extruding paths (RL-step scale) | ~38 ms |
| `denote_lo` | 20k paths (a whole part) | ~420 ms |
| `denote_lo_deposit` | 2k paths | ~50 ms |
| `verify_roundtrip` | 2k paths | ~207 ms |
| `denote_lo` (full) | 50-layer, 20k-path part | ~421 ms |
| `incremental` single-layer edit | same part, one layer dirty | ~55 ms |
| `verify_batch` | 64 candidates × 2k paths | ~1.92 s |

## Incremental denote: work vs. wall-clock

The incremental cache (`DenoteCache`) re-rasterizes only the layers marked dirty and returns the
occupancy by reference — no full clone. Editing one layer of an N-layer part is **~N× less work** (one
layer rasterized, not N).

Wall-clock speedup is smaller than the work ratio, because a full `denote_lo` already parallelizes
across layers with rayon: a full re-denote of an N-layer part costs about `N / cores` layer-times,
while a single-layer edit costs one. So on this 8-core machine the 50-layer part shows ~7.7× wall-clock
(55 ms vs 421 ms), and on a 2–4 core CI runner the same edit is 50×+ faster in wall-clock. Either way
the point stands: an edit costs one layer's work, not the whole part's.

For a hot RL/search loop: keep a `Program` handle (Python) or a `DenoteCache` (Rust), apply enumerated
transform actions (which report the touched layers), and read occupancy/objectives back incrementally —
no JSON, no full re-denote per step.
