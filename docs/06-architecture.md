# 06 — Architecture & build plan

## Language decision: Rust core, thin Python bindings

Confirmed. Performance-sensitive geometry + IR work; a pure-Rust core with no Python coupling can also
power a CLI/WASM; PyO3 + maturin is the standard stack and `uv` drives it. We do **not** extract
Cura's/PrusaSlicer's private C++ IRs — we design a fresh, engine-independent one, informed by them.

## Two-level IR (realized in v0)

The adversarial design review ([`07-design-review.md`](07-design-review.md)) found the original single
IR level was shaped like a compiler's *low* (move-plan) level, leaving nothing for the verifier or
geometric passes to reason about. The v0 IR now has two levels, mirroring how production slicers
already separate them (CuraEngine `SliceLayerPart`/`SkinPart` vs. the lowered `LayerPlan`; PrusaSlicer
`LayerRegion`/`Surface` vs. `ExtrusionEntity`):

```
ir::hi (geometric — "what should be solid")        ir::lo (move plan — "what the machine does")
  Program                                            Program
   └─ Layer { z_um, regions }                         └─ Layer { z_um, toolpaths }
       └─ Region { kind, boundary: Area, fills }          └─ Toolpath { kind: SegmentKind, path, width_um }
           └─ ExtrudePath { path, width_um }
                                    │  crate::lower::lower(&hi) -> lo   │
                                    └───────────────────────────────────┘
```

Shared geometry (`ir::mod`): `Point{x,y:i64}` (integer microns, 2D), `Polyline`, `Area`
(outer loop + holes, à la ExPolygon), `RegionKind` (Perimeter/Infill/Skin/Support), `ExtrudePath`.

Design axioms and the honest caveats around them:

- **Integer microns, fixed-point.** Mirrors CuraEngine/ClipperLib; floats would undermine a verifiable
  IR. `i64` range is ample for build volumes.
- **Planar per layer, on purpose.** `Point` is 2D and `z` is a per-layer scalar. Non-planar/variable-Z
  is *explicitly out of scope* (see [`05-direction.md`](05-direction.md)); the earlier "not married to
  planarity" wording was wrong and is corrected. If non-planar returns, add `z` as a per-segment
  *attribute*, not by widening `Point` (the most-depended-on node). **Rotation-invariance lives in
  `denote`'s geometric domain (re-slice after rotating the input), never as a `rotate()` on the types.**
- **Two axes, not conflated.** Feature-role (`RegionKind`) is separate from machine motion
  (`lo::SegmentKind = Extrude(RegionKind) | Travel`). Travel is a low-level-only concept.
- **Serializable.** `serde` derives are gated behind a feature. Wire caveat to settle before any `.kerf`
  file exists: `i64` micron values > 2^53 lose precision in JS/Python JSON — encode as strings or
  document the bound when a JSON boundary is exposed.

## `denote` — the semantics (the heart of the contribution)

`denote_hi` / `denote_lo` map a program to an `Occupancy`: the set of material-occupied cells per
layer, computed by sweeping each *extruding* path (Minkowski sum with a `width_um` disk) and unioning
per layer, rasterized on the micron lattice. Travel denotes nothing.

- This is the **reference semantics**: correctness-first, deliberately slow. The domain choice
  (rasterized occupancy vs. exact swept region vs. point cloud) is **provisional** and affects the
  oracle's false-positive rate — ship the slow reference first, pressure-test, then optimize.
- The lossy G-code backend drops `width_um`, so it **must not** be the semantic reference. Meaning is
  defined in `denote`, not in `to_gcode`.
- Float distance is used **only inside** the checker; the IR stays exact integer microns. Rotated
  comparison, when added, must lift to rationals or a bounded tolerance *inside the checker only*.

`self_lowering_sound(prog, res)` = `denote_hi(prog) == denote_lo(lower(prog))`. The first
non-redundant artifact — a property GlitchFinder cannot state because it owns no lowering. v0 lowering
only reorders + inserts travel, so it holds by construction; the value now is the harness and the
property. It becomes a real check the moment a pass rewrites the plan.

## Pass policy

`trait Pass { fn name(&self) -> &str; fn run(&self, lo::Program) -> lo::Program; }` — **pure
value-in/value-out**, not `&mut`. This admits mutable impls today and keeps the e-graph door open;
`&mut` would bake destructive rewrite into the contract and force a later break. Every pass MUST
preserve `denote_lo`. No e-graph / interning / operator-enum yet — those wait for a rewrite-rule
catalog and a cost function.

## Trust model (interim)

Inputs are assumed well-formed. The reference semantics is `denote`. The checked property is
denotation preservation, established by property-based testing (proptest) for now — **not** a
machine-checked proof. We say "correctness oracle / checked property," not "proof." A full
CompCert-style TCB writeup lands with the first paper, not before.

## Build pipeline

`cargo test -p kerf-core --all-features` (fast Rust iteration) → `uv sync` (maturin builds the abi3
extension into the venv) → `uv run python -c "import kerf; ..."`. CI is one Linux job:
`fmt --check` + `clippy -D warnings` + `test` + `maturin build`.

## Build order (next milestones)

Foundation hygiene and the two-level refactor + `denote` + oracle are **done** in v0. Remaining:

1. ~~**First real pass.**~~ **Done.** `pass::TravelOrder` reorders extruding toolpaths (greedy NN,
   reversal-safe), checked by `preserves_denotation`; cuts demo travel ~62%. A negative test proves the
   oracle rejects a pass that drops material.
2. ~~**Expose the IR to Python via the serde JSON boundary.**~~ **Done.** `kerf_core::json` +
   `program_to_gcode` / `lower_to_json` / `check_self_lowering_sound` / `demo_square_json`; malformed
   JSON is a `ValueError`, not a crash.
3. ~~**G-code parser frontend.**~~ **Done.** `frontend::gcode::parse` reads real Cura / PrusaSlicer /
   OrcaSlicer output into `lo` (E-delta extrude/travel, marker-driven layers, sticky role/width with a
   trust boundary, diagnostics). Never panics on untrusted input — property-fuzzed rather than
   cargo-fuzz (the review found the proptest sufficient; a nightly libFuzzer target adds little over it).
   Hardened by a dedicated adversarial review (six correctness fixes; see `docs/07`).
4. ~~**Named delta beyond GlitchFinder.**~~ **Done.** `verify::verify_gcode` runs, on real parsed
   geometry, both own-pass soundness (`TravelOrder` preserves `denote`) and a second metamorphic
   relation, `metamorphic::translation_invariant`. The `kerf` CLI (`verify` / `inspect`) exposes it.
5. **Deferred until a concrete trigger:** e-graph superoptimization (needs a rewrite catalog + cost fn);
   arc (G2/G3) *emission* in the backend (parser arc input is done); flow/E/retraction; per-segment width provenance;
   criterion benchmarks (first pass on million-segment inputs); 3-OS + wheel CI matrix (first publish);
   `pyo3` 0.29 bump (CPython 3.13/3.14 need); a mechanized end-to-end proof (the big research target).
