# 07 — Adversarial design review of v0 (and what was applied)

Before building further on the v0 skeleton, five independent expert lenses (compiler/PL,
computational-geometry/slicing, formal-verification, Rust/PyO3 systems, and an adversarial novelty
skeptic) critiqued the actual files; each critique was skeptically verified to strip premature /
over-engineered suggestions; a synthesis produced a prioritized fix-now/defer plan. This file records
the outcome. It is the "verify later" artifact.

## Executive take

The v0 foundation was sound as a *plumbing spike* — the pure-Rust core / thin-PyO3 split, integer-micron
coordinates, and the load-bearing `SegmentKind` tag are correct and were kept. But the spike was shaped
like a compiler's **low** (move-plan) level, while the entire novelty lives at a level that did not yet
exist: there was no `denote` (what a program *means* as deposited material), no high geometric/region
level for an oracle to check against, and the two *redundant* pillars (a tagged-toolpath container ≈ a
less-mature OVF; e-graph passes ≈ Carpentry Compiler) were being built foundation-first while the one
*non-redundant* pillar (a verified lowering Kerf owns) was deferred.

Three corrections drove everything: (1) write `denote` — the missing prerequisite for the oracle AND for
sound passes; (2) reorder pillars so verification is #1 and the first artifact is self-lowering soundness,
not a reproduction of GlitchFinder; (3) land the high-level region layer + polygon-with-holes + `denote`
as one coordinated change *before* any pass or oracle is written against the flat move-plan type.

## Sharpened novelty statement (survives the skeptic)

> An open, engine-independent IR for the mesh→G-code half that carries a **defined denotational
> semantics** (`denote(Program)` = the deposited material region), together with a
> **mechanically-checked lowering-soundness property** on a lowering Kerf *owns* — `denote(prog)`
> preserved by the `hi→lo` lowering, and each pass discharging a denote-preservation obligation on the
> move plan (the concrete `lo`→G-code emitter is lossy and sits outside the verified boundary).

Non-redundancy: LambdaCAD proves CAD→mesh and stops at the mesh; GlitchFinder tests slicers as opaque
black boxes and owns no IR/lowering; Carpentry Compiler/Szalinski do equality-saturation over
carpentry/CSG (reused technique, not the novel part); OVF/3MF-Toolpath are storage containers with no
analysis semantics; OpenVCAD stops at geometry/voxels. **Honest narrowing:** rotation-invariance alone is
GlitchFinder's, not ours — here it becomes one *derived* property over `denote`. The differentiator is
"an oracle-checked lowering with a written semantics."

## Fix-now — applied in this pass

| # | Change | Where | Status |
|---|---|---|---|
| 1 | `denote` reference semantics (swept-material occupancy) + the negative constraint that the lossy backend is not the reference | `crates/kerf-core/src/denote.rs`, `backend.rs` | ✅ done |
| 2 | Reframe docs to verification-first; demote IR/passes to enabling infra; re-aim first artifact to self-lowering soundness; soften "proof"→"checked oracle", "plug into all"→"consume to verify" | `README.md`, `docs/00,05,06` | ✅ done |
| 3 | Two-level IR: `ir::hi` (Region + polygon-with-holes `Area`) / `ir::lo` (move plan) + `hi→lo` lowering; `denote` over the geometric level; Travel is low-level-only | `crates/kerf-core/src/ir/*`, `lower.rs` | ✅ done |
| 4 | `PartialEq/Eq` on all IR types; `serde` behind a feature flag; note the `i64`-in-JSON >2^53 precision trap | `crates/kerf-core/src/ir/*`, `Cargo.toml`, `docs/06` | ✅ done (custom i64 encoding deferred until a JSON boundary is exposed) |
| 5 | `abi3-py312` on the existing pyo3 0.23 (single wheel for CPython ≥ 3.12); do **not** bump to 0.29 | `crates/kerf-py/Cargo.toml` | ✅ done (wheel now `_kerf.abi3.so`) |
| 6 | `.gitignore` `*.so`/`*.pyd`/`*.dylib` + `/target` BEFORE any `git add` | `.gitignore` | ✅ done |
| 7 | `Pass` trait as pure `fn run(&self, Program) -> Program` (not `&mut`), keeping the e-graph door open | `crates/kerf-core/src/pass.rs` | ✅ done |
| 8 | Fix misleading doc claims: planarity axiom (v0 is planar-per-layer, non-planar out of scope), `SegmentKind` two-axis note, record that rotation lives in `denote`'s domain | code doc-comments, `docs/06` | ✅ done |
| — | Single Linux CI job (fmt/clippy/test/wheel) + `rust-toolchain.toml` | `.github/workflows/ci.yml`, `rust-toolchain.toml` | ✅ done |

## Deferred (revisit triggers)

- **3D `Point` / non-planar geometry** — only if non-planar returns to scope; then add `z` as a
  per-segment *attribute*, not a widened `Point`. Rotation-invariance does not require it.
- **Stable node IDs (`ToolpathId`)** — when the verifier localizes discrepancies or does incremental recompute.
- **Generic `AttrMap` metadata** — when a *second* per-node property arrives (first one, flow, lands as a plain field).
- **Per-segment attribute vector (width/flow/speed)** — when the backend stops dropping width.
- **G-code parser frontend** — *after* `denote` + oracle; parse comment-annotated output; add `cargo-fuzz` there.
- **E-graph / equality-saturation + interning** — when a rewrite-rule catalog and cost function exist.
- **Flow/E/retraction/arcs backend; criterion benchmarks** — when flow is needed / a real pass runs on large inputs.
- **`num-rational` / tolerance for rotated comparison; i128 accumulators** — when the oracle & geometry passes are written.
- **CI matrix breadth (3-OS + wheels); pyo3 0.29 bump** — at first publish / when CPython 3.13/3.14 is needed.
- **Full CompCert-style TCB writeup** — with the first paper.

## Killed recommendations (rejected as premature / redundant)

- **Make `Point` 3D now** — YAGNI; non-planar is explicitly out of scope; rotation lives in `denote`'s domain.
- **MLIR-dialect framework (op-enum + interning)** — over-spec; two plain modules + one lowering fn suffice.
- **Reproduce GlitchFinder's "Cura vs. PrusaSlicer disagree" as the first artifact** — redundant; replaced with self-lowering soundness.
- **`#[pyclass]` per IR type** — churns `kerf-py` on every field add; use the serde JSON boundary instead.
- **`&mut Program` pass signature** — bakes destructive rewrite into the contract; fights equality saturation.
- **Bump pyo3 to 0.29 to get abi3** — false coupling; abi3-py312 works on 0.23.
- **Generic `AttrMap` now; full 3-OS CI + CLI/criterion/fuzz for v0** — no present consumer.

## Open risks (carry forward)

1. **Adoption**: engines aren't extensible (Cura) / have no plugin API (OrcaSlicer); Kerf will mostly *consume* slicer output. Lean the pitch on the verifier.
2. **Denotation-domain choice** (rasterized occupancy vs. exact swept region vs. point cloud) affects the oracle's false-positive rate — ship the slow reference, pressure-test before optimizing.
3. **Rotating integer-micron points by non-axis angles isn't closed over the lattice** — a naive oracle will report false positives on a *correct* slicer. Handle tolerance/rationals inside the checker only.
4. **The region/high-level refactor is on the critical path** — if done piecemeal, passes/oracle get authored against the flat type and need rewriting. (Mitigated: done up front in v0.)
5. **The surviving novelty is thin next to GlitchFinder unless a concrete delta is committed** — name it (own-lowering/pass validation; a second metamorphic relation).
6. **`i64`-micron-in-JSON precision trap** — handle at the serde wire-format decision, not after `.kerf` files exist.
7. **Verification scope creep** — resist over-investing in mechanized proofs before the cheap property-based artifact demonstrates the wedge.

---

## Second review — oracle soundness (after the travel-order pass)

A follow-up adversarial review attacked the one thing that must be right: *can the oracle say
"preserved" when the print actually changed?* It found real bugs (a subagent even wrote a probe that
reproduced them). All fixed; regression tests added.

**Found and FIXED:**

| Bug | Severity | Fix | Regression test |
|---|---|---|---|
| `denote` was **not reversal-invariant** — `dist2_point_seg`'s float rounding differed for a segment vs. its reverse, so `TravelOrder` (which reverses paths) was **unsound against its own oracle** at zero width / fine resolution | critical | canonicalize segment endpoints before any float math (`mark_capsule`) | `denote_is_reversal_invariant`, `reversal_invariant` (proptest) |
| **Centre-sampling false confidence** — a 0.4 mm wall between grid centres registered as *zero cells*, so deleting a real wall read as "preserved" | critical | conservative **coverage** (mark a cell if the capsule reaches within its circumradius) so a feature is never entirely missed when resolution ≤ its width | `coverage_catches_a_thin_wall_between_grid_centres` |
| `dist2` `i128` **product overflow** on extreme coordinates (panic in debug) | medium | the NN heuristic uses `f64` (it never affects denotation, only ordering) | covered by clean build; extreme-coord path no longer panics |
| Negative width / `resolution=0` handled silently | low | clamp width to ≥0; `resolution.max(1)` (documented) | — |

**Documented as a known limitation (not a bug):** the oracle is only as sharp as the raster
resolution — a sub-resolution vertex nudge is not distinguished. Now stated in `denote.rs` and **pinned
by a test** (`oracle_is_blind_below_resolution`) instead of hidden. Guidance: choose
`resolution_um ≤ the smallest feature you care about` (e.g. nozzle/line width).

**Corrected overclaims:** "denotation-preserving *by construction*" was literally false before the
reversal fix — the doc now says reversal safety is a property of `denote` that the oracle *checks*.

**Acceptable / conservative (left as-is):** the oracle is *over-strict* on harmless transforms
(inserting an empty layer, reordering layers with distinct Z reads as "changed") because it compares
layers positionally. For a verification tool, over-rejection is the safe direction; revisit if it
becomes annoying.

**Process note:** workflow subagents have write access and left one scratch probe file
(`crates/kerf-core/tests/zz_adjudicate.rs`); its cases were harvested into curated tests and the file
was removed. Watch for stray subagent files after review workflows.

---

## Third review — G-code parser (untrusted input)

An adversarial review of `frontend::gcode` (four lenses: state-machine correctness, spec fidelity,
trust boundary, robustness) found verified correctness bugs where untrusted input silently corrupted
*trusted geometry*. All fixed; each has a regression test.

| Bug | Severity | Fix | Test |
|---|---|---|---|
| **SF-1** an oversized coordinate consumed the E baseline before the overflow guard, flipping the next real extrude into a travel — silently deleting material | critical | overflow/`prev` guards moved *before* `take_de`; a skipped move changes no state | `sf1_overflow_move_does_not_delete_the_next_extrude` |
| **RE-1/2/3** extrude/travel/zero-length decided on float-mm length → degenerate `[P,P]` segments and absurd (~km) widths | high | decision made on rounded-micron displacement; `push_segment` never gets `target==prev` | `re1_*`, `re3_*`, + proptest "no consecutive equal points" |
| **SM-1** a travel Z-hop polluted modal Z, misfiling a later extrude at the hop height | high | layers open only on markers / first extrude; layer Z from `;Z:` or the extruding move, never a travel-reached Z | `sm1_travel_zhop_does_not_create_a_spurious_layer` |
| **TRW-1** Cura formula-widths collapsed a whole same-role run to the first segment's width | medium | toolpath continuation also requires `width_close`; a real width change splits, micro-variation does not | `trw1_*` (splits) + `trw1_*` (no over-fragment) |
| **TRW-2/3** a stale role leaked across layer/object boundaries with no diagnostic | medium | role reset at every layer marker; fallback recorded at the consumption site into a deduped set + `fallback_role_moves` counter | `trw3_role_does_not_leak_across_a_layer_boundary` |
| **RE-4** an overflowing `G92` position produced a phantom edge from the origin | low | `prev` built with the same guarded conversion; overflow → skip + diagnostic | `re4_overflowing_g92_position_*` |

**Documented as limitations (not fixed):** planar-only (vase mode lossy); pre-extrusion travel elided
(deposited-only counts); extrude-before-Z filed at z=0; a `*checksum` inside a `;TYPE:` value leaks into
the untrusted role string. **Killed:** clamping formula width to a "plausible" band (contradicts the
spec's width definition). **cargo-fuzz:** rejected for v0 — the existing proptest (800 cases over
arbitrary bytes + a gcode grammar) covers what a libFuzzer target would, and runs in CI.

**Delta beyond GlitchFinder (built on this):** `verify::verify_gcode` runs, on the *parsed real
geometry*, both own-pass soundness and `metamorphic::translation_invariant` — properties a black-box
slicer tester cannot state because it owns no IR, lowering, or passes.

---

## Fourth review — final staff-engineer sign-off

A whole-product review (five lenses: correctness/soundness, architecture, docs honesty, test quality,
ship gate) returned **no sign-off** on three honesty/soundness gaps on the load-bearing claims — all
small, all fixed:

1. **Overclaim** — the marquee sentence said "denote preserved by IR→G-code", but the checked lowering
   is `hi→lo`; the `lo`→G-code emitter is lossy and outside the verified boundary. Reworded in
   `README.md` and this doc to state exactly what is checked.
2. **Vacuous verdict** — `verify_gcode` / the CLI returned SOUND (exit 0) on G-code with zero recovered
   geometry. Added `has_geometry`; `ok()` now requires it; the CLI prints "NOTHING TO VERIFY" and exits
   3; README/example updated; pinned by `no_geometry_is_not_a_green_verdict` + a CLI integration test.
3. **Tautological guard** — the reversal-invariance tests still passed with the canonicalization they
   protect removed. Fixed by adding explicit float-boundary flip cases to `denote_is_reversal_invariant`
   and **validating** them: with the canonicalization deleted, those cases fail (a random proptest, even
   2000 cases over large coords, does *not* reliably hit the knife-edge and was slow — so the guarantee
   is pinned by deterministic cases, and the non-guarding proptest was removed).

Also applied (fix-now): saturating bbox arithmetic + f64 cell centres (no overflow on extreme coords) +
a regression test; a non-vacuous "real slicer survives" fixture that asserts `TravelOrder` actually
cuts travel; CI toolchain pinned to `1.90.0` to match `rust-toolchain.toml`; corrected
translation-invariance prose (it exercises `denote`, not the parser); a `denote` width-discrimination
test; a CLI exit-code integration test; and a clause that `denote` measures deposited filament, not
region-boundary satisfaction.

After these fixes: 39 `kerf-core` tests + 3 CLI integration tests + property/fuzz pass, `clippy
--workspace --all-targets --all-features -D warnings` is clean on the pinned toolchain, and the central
claim as now written holds as implemented. **Signed off** as a v0/v1 research artifact.

---

## Fifth review — production-hardening (arcs, `diff`) + gate

While pushing the artifact toward professional usability, two features were added — **arc (G2/G3)
flattening** in the parser (I/J and R forms) and **`kerf diff`** (compare two files by deposited
material) — plus a performance characterization (~0.5 s to verify a 24k-move file). A three-lens
review (arc correctness, diff correctness, production gate) found:

- **Ship-blocker (fixed):** `diff` reintroduced the vacuous-verdict bug review #4 fixed for `verify` —
  two empty/unparseable files reported `IDENTICAL` + exit 0. Added `GcodeDiff::both_empty`; the CLI now
  prints "NOTHING TO COMPARE" and exits 3, pinned by unit + CLI tests.
- **Arc bug (fixed):** a single-offset arc (`G2 X.. I..`, `J` omitted) silently degraded to a straight
  chord. Fixed to default the missing offset to 0 (grbl/Marlin semantics); regression-tested. The R-form
  sign convention and the CW/CCW sweep were independently verified correct (incl. a 148k-case fuzz).
- **Fixes-now (done):** `--resolution <= 0` is now rejected; the diff "IDENTICAL" line is qualified with
  the resolution; README/docs stale claims corrected (arcs no longer "future work"; test counts updated;
  `diff` documented); the example gained an arc block and CI a `diff` smoke; an arc-survives-`verify`
  end-to-end test was added.

**Honestly deferred (NOT done — genuinely remaining for production):** a faithful arc-*emitting*
backend (flow/E/retraction); scale/perf beyond ~100k moves (the `denote` rasterizer, run ~3× by
verify); a 3-OS + wheel publish matrix and PyPI/crates.io release; and the mechanized end-to-end
proof. These need real time and, for adoption, a committed user — they are not code the artifact can
credibly fake. Verdict: a defensible, usable **v1 research/pro tool**; not a certified production system.
