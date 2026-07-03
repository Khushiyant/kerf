# Changelog

All notable changes to Kerf are documented here. Versions follow [SemVer](https://semver.org).

## [Unreleased]

### Performance
- **~5× faster verification, provably identical output.** The reference rasterizer now (1) prunes each
  segment's per-row column scan to the covered band (cells whose perpendicular distance to the segment's
  line is within reach — a superset of the covered interval, still decided by the exact predicate) and
  (2) rasterizes independent layers in parallel (rayon). A 100k-move / 200-layer print went from ~20 s
  to ~4 s. `optimized_raster_marks_exactly_the_bruteforce_set` differential-tests that the marked set is
  unchanged over thousands of random segments.

### Added (verification)
- **Machine-checked (bounded) proofs via Kani.** Four `#[cfg(kani)]` harnesses, all verified by
  `cargo kani -p kerf-core`: `canon_seg_is_order_independent` (the mechanism of reversal invariance,
  for *all* `i64` endpoints), `dist2_point_seg_is_nonneg_and_finite` (no NaN/negative leaks into the
  coverage comparison), and `mm_to_um` / `um_round` totality-and-range (the parser's float→micron
  conversions never panic and never silently saturate). See `docs/08-semantics.md` §5.
- **Exhaustive bounded verification test.** `reversal_invariant_exhaustively_over_small_programs`
  enumerates *every* 2- and 3-point program over a coordinate grid (~40k programs) and checks P1 holds
  — a complete check of the bounded domain, where the proptest only samples.

### Fixed (found by testing on real slicer output)
- **PrusaSlicer layer segmentation.** Recognize `;BEFORE_LAYER_CHANGE` / `;AFTER_LAYER_CHANGE`
  custom-gcode hooks as layer boundaries. A real PrusaSlicer 2.1.1 3DBenchy (136k lines) previously
  collapsed all 241 layers into one; now segments correctly and verifies SOUND in ~0.7 s.
- **Simplify3D vocabulary.** Recognize `; layer N, Z = <mm>` layer markers (with inline Z) and the
  bare role comments (`; outer perimeter`, `; inner perimeter`, `; infill`, `; solid layer`,
  `; support`, `; bridge`, `; skirt`) that have no `;TYPE:` prefix. A real Simplify3D 3.1.0 file (106
  layers) previously collapsed to 1 layer with every move an unknown-role fallback; now segments to
  106 layers with roles recovered.
- **ideaMaker (Raise3D) roles.** Map the `;TYPE:` values `SOLID-FILL` / `BRIDGE` → Skin and `GAP-FILL`
  → Infill. On a real ideaMaker 5.3 print these were the *majority* of deposited material and all fell
  back to Perimeter; the same file now recovers 23 layers with only its 64 skirt moves as fallback.
- **KISSlicer support (new).** Recognize `; BEGIN_LAYER_OBJECT z=<mm>` layer markers (inline lowercase
  `z=`) and the quoted role form `; '<Role> Path', <feed> [feed mm/s], ...` (PREMIUM) / `; '<Role>'`
  (FREE). A real FREE cube previously collapsed to 1 layer with every extrude an unknown-role fallback;
  now recovers 40 layers with only wipe moves (correctly) non-structural.
- **Verbose Slic3r / SuperSlicer roles.** Recognize the lowercase bare comments (`; perimeter`,
  `; external perimeter`, `; solid infill`, `; support material`, …) emitted with `gcode_comments` on.
- **OrcaSlicer/Bambu roles.** Map `Floating vertical shell` / `Floating interface shell` to Perimeter.
- **Cleaner KISSlicer diagnostics.** Record only the quoted role *name* (not the trailing feed/head
  rates), so a wipe feature no longer appears as many distinct "unknown roles".

### Validated
- Ran on real output from Cura (3.6, 5.3), PrusaSlicer (2.1, 2.7), BambuStudio/OrcaSlicer (BBL),
  Simplify3D (3.1), ideaMaker (5.3), KISSlicer (FREE 1.1 / PREMIUM 1.6), Slic3r (1.3), and
  ArcWelder-processed arcs — no panics, no unsound verdicts; geometry and (post-fix) layer/role
  recovery correct across all.

### Known limitations (documented, not hidden)
- **Comment-less G-code isn't segmentable.** Slic3r with `gcode_comments=0` (the default) emits no
  layer/role comments at all — only bare `G1 Z` moves — so the print recovers as a single layer.
  Inferring layers from Z-increasing travel would misfire on Z-hops; enable `gcode_comments` instead.
- **Older marker-less KISSlicer.** A pre-`BEGIN_LAYER_OBJECT` KISSlicer dialect (seen in 2019 output)
  segments only via `; Prepare for …` feature comments with no reliable per-layer *start*; not supported.
- **G91 relative-coordinate moves** are skipped (counted in `skipped_g91_moves`); arcs in relative mode
  are not flattened. Real FFF prints use absolute XY.

## [1.0.0]

First production release: a verifiable intermediate representation for the mesh → G-code half of the
fabrication pipeline, with a defined denotational semantics and a mechanically-checked lowering.

### Added
- **IR** — two levels (`hi` geometric regions, `lo` move plan) joined by a lowering Kerf owns.
- **`denote`** — reference semantics mapping a program to the material it deposits (reversal-invariant,
  conservative-coverage rasterization on an integer-micron grid).
- **Soundness oracle** — `self_lowering_sound` and per-pass `preserves_denotation`, with a
  material-drop negative test proving the check has teeth.
- **`TravelOrder`** optimization pass, checked by the oracle.
- **G-code frontend** — parses real Cura / PrusaSlicer / OrcaSlicer output, including **arc (G2/G3)**
  flattening (I/J and R forms); never panics on untrusted input (property-fuzzed); trust boundary
  (geometry trusted, role/width untrusted and diagnosed).
- **Faithful backend** — emits G-code with flow-based extrusion, retraction, and `;TYPE:`/`;WIDTH:`
  comments; round-trip (parse → lower → emit → parse) preserves `denote`.
- **`verify_gcode`** — on real parsed geometry, checks own-pass soundness and translation-invariance.
- **`diff_gcode`** — compares two files by deposited material (matched by layer height), with a
  `both_empty` guard so unparseable files never read as a match.
- **`kerf` CLI** — `verify` (single or batch), `inspect`, `diff`; JSON output; CI-friendly exit codes
  (0 sound / identical, 1 unsound / differ, 2 usage error, 3 nothing to verify/compare).
- **Python bindings** (abi3, CPython ≥ 3.12) exposing the above via a JSON boundary.

### Known limitations (documented, not hidden)
- Verification is **resolution-bounded** (deposited material up to a raster resolution), not a
  machine-checked proof. Choose a resolution ≤ your smallest feature.
- **Planar only** (2D-per-layer IR); non-planar / vase-mode is out of scope.
- The parser recovers **deposited geometry**, not exact process state (widths without a `;WIDTH:`
  comment are estimated; feature roles are an untrusted re-inference).
- Not yet optimized for very large (100k+ move) prints; the reference `denote` rasterizer is the
  bottleneck.

### Reviewed
- Hardened across five independent adversarial reviews (design, oracle soundness, parser, delta,
  production gate); every finding applied. See `docs/07-design-review.md`.
