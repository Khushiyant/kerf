# Changelog

All notable changes to Kerf are documented here. Versions follow [SemVer](https://semver.org).

## [Unreleased]

## [0.1.0] — 2026-07-04

First public release: a verifiable intermediate representation for the mesh → G-code half of the
fabrication pipeline, with a defined denotational semantics and a mechanically-checked lowering.

### Added
- **IR** — two levels (`hi` geometric regions, `lo` move plan) joined by a lowering Kerf owns.
- **`denote`** — reference semantics mapping a program to the material it deposits (reversal-invariant,
  conservative-coverage rasterization on an integer-micron grid); a covered-band column prune plus
  per-layer parallelism keep it ~5× faster (100k-move / 200-layer print ~4 s), with
  `optimized_raster_marks_exactly_the_bruteforce_set` pinning the marked set unchanged.
- **Soundness oracle** — `self_lowering_sound` and per-pass `preserves_denotation`, with a
  material-drop negative test proving the check is not vacuous.
- **`TravelOrder`** optimization pass, checked by the oracle.
- **G-code frontend** — parses real Cura / PrusaSlicer / OrcaSlicer / Bambu / Simplify3D / KISSlicer /
  ideaMaker / Slic3r output, including arc (G2/G3) flattening (I/J and R forms); never panics on
  untrusted input (property-fuzzed); trust boundary (geometry trusted, role/width untrusted and diagnosed).
- **Faithful backend** — emits G-code with flow-based extrusion, retraction, and `;TYPE:`/`;WIDTH:`
  comments; round-trip (parse → lower → emit → parse) preserves `denote`.
- **`verify_gcode`** — on real parsed geometry, checks own-pass soundness and translation-invariance.
- **`diff_gcode`** — compares two files by deposited material (matched by layer height), with a
  `both_empty` guard so unparseable files never read as a match.
- **`kerf` CLI** — `verify` (single or batch), `inspect`, `diff`, `--version`; JSON output; CI-friendly
  exit codes (0 sound / identical, 1 unsound / differ, 2 usage error, 3 nothing to verify/compare).
- **Python bindings** (`pykerf`, abi3, CPython ≥ 3.12) exposing the above via a JSON boundary.

### Proofs
- **P1–P4 proved in Lean 4** with no `sorry` (`proofs/KerfProofs.lean`), axioms audited.
- **Four Kani harnesses** (`cargo kani -p kerf-core`): `canon_seg_is_order_independent` (reversal
  invariance for all `i64` endpoints), `dist2_point_seg_is_nonneg_and_finite`, and `mm_to_um` /
  `um_round` totality-and-range for the parser's float→micron conversions.
- **Exhaustive bounded check** `reversal_invariant_exhaustively_over_small_programs` enumerates every
  2- and 3-point program over a coordinate grid (~40k programs).

### Validated
- Ran on real output from Cura (3.6, 5.3), PrusaSlicer (2.1, 2.7), BambuStudio/OrcaSlicer (BBL),
  Simplify3D (3.1), ideaMaker (5.3), KISSlicer (FREE 1.1 / PREMIUM 1.6), Slic3r (1.3), and
  ArcWelder-processed arcs — no panics, no unsound verdicts; layer/role recovery correct across all.

### Tooling
- Tag-driven `release.yml` publishes `kerf-core` + `kerf-cli` to crates.io (in dependency order) and
  the `pykerf` abi3 wheels + sdist to PyPI, builds the `ghcr.io/khushiyant/kerf` container, and attaches
  artifacts to a GitHub Release. See `RELEASE.md`.

### Known limitations
- **Resolution-bounded.** `denote` compares material up to the raster resolution; choose a resolution
  ≤ your smallest feature. Sub-resolution differences are not distinguished.
- **Planar only** (2D-per-layer IR); non-planar / vase mode is out of scope.
- **Deposited geometry, not process state.** Widths without a `;WIDTH:` comment are estimated; feature
  roles are an untrusted re-inference. The `lo`→G-code emitter is lossy and sits outside the verified
  boundary.
- **Comment-less G-code isn't segmentable.** Slic3r with `gcode_comments=0` emits no layer/role
  comments, so the print recovers as a single layer; enable `gcode_comments` instead.
- **G91 relative-coordinate moves** are skipped (counted in `skipped_g91_moves`); arcs in relative mode
  are not flattened.

### Reviewed
- Hardened across five independent adversarial reviews (design, oracle soundness, parser, delta,
  production gate); every finding applied. See `docs/07-design-review.md`.
