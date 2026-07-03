# Changelog

All notable changes to Kerf are documented here. Versions follow [SemVer](https://semver.org).

## [Unreleased]

### Fixed (found by testing on real slicer output)
- **PrusaSlicer layer segmentation.** Recognize `;BEFORE_LAYER_CHANGE` / `;AFTER_LAYER_CHANGE`
  custom-gcode hooks as layer boundaries. A real PrusaSlicer 2.1.1 3DBenchy (136k lines) previously
  collapsed all 241 layers into one; now segments correctly and verifies SOUND in ~0.7 s.
- **Simplify3D vocabulary.** Recognize `; layer N, Z = <mm>` layer markers (with inline Z) and the
  bare role comments (`; outer perimeter`, `; inner perimeter`, `; infill`, `; solid layer`,
  `; support`, `; bridge`, `; skirt`) that have no `;TYPE:` prefix. A real Simplify3D 3.1.0 file (106
  layers) previously collapsed to 1 layer with every move an unknown-role fallback; now segments to
  106 layers with roles recovered.
- **OrcaSlicer/Bambu roles.** Map `Floating vertical shell` / `Floating interface shell` to Perimeter.

### Validated
- Ran on real output from Cura (3.6, 5.3), PrusaSlicer (2.1, 2.7), BambuStudio/OrcaSlicer (BBL),
  Simplify3D (3.1), and ArcWelder-processed arcs — no panics, no unsound verdicts; geometry and (post-fix)
  layer/role recovery correct across all.

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
