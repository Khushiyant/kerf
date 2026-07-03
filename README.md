# Kerf

**An open, engine-independent intermediate representation for the mesh → toolpath (G-code) half of the fabrication pipeline — with a defined meaning and a lowering whose correctness is mechanically checked.**

Think "LLVM for slicing," but the point is the *verifier*, not the container.

---

## The one-paragraph pitch

Every 3D-printing slicer (Cura, PrusaSlicer, OrcaSlicer) is secretly a compiler: it takes a
geometric description and lowers it, through several stages, into machine code (G-code). But each one
buries that machinery inside a private, engine-specific codebase, and — as of 2025 — **no slicer can
show its output actually corresponds to the input geometry** (two mainstream slicers demonstrably
disagree on the same model most of the time). Kerf is the missing middle layer: an IR that has a
written-down *meaning* (`denote`), a lowering to G-code that Kerf owns, and a **correctness oracle**
that mechanically checks the lowering preserves that meaning. Optimization passes are enabling
infrastructure on top — each one must discharge the same denotation-preservation check.

## What already exists (and what we are NOT building)

Scoped from a deep-research pass and an adversarial design review (see [`docs/`](docs/)):

| Stage | Status | Are we building it? |
|---|---|---|
| CAD → mesh (geometry kernels, DSLs, verified compilers) | **Solved.** libfive, Fidget, LambdaCAD. | No — crowded, done well. |
| The *idea* "fabrication pipeline = compiler" | **Published** (SNAPL 2017, Carpentry Compiler 2019, ICFP 2018). | No — the concept is prior art. |
| A tagged-toolpath *container* | **Exists** (OpenVectorFormat, 3MF-Toolpath draft). | No — a container with no analysis semantics is redundant. |
| E-graph / equality-saturation optimization of fab plans | **Done** (Carpentry Compiler, Szalinski). | We *reuse* the technique; it is not our contribution. |
| Differential testing of slicers as black boxes | **Done** (GlitchFinder, OOPSLA 2025). | No — it owns no IR/lowering; we do something it structurally can't. |
| An IR with a **defined denotation** + a **verified lowering** to G-code | **Does not exist.** | **Yes — this is the wedge.** |

## The distinction that matters (read this if you're confused about novelty)

"CAD→mesh is done" and "slicing is done" are both true — and neither is Kerf.

The analogy: **compilers existed long before LLVM.** GCC compiled C fine. But each compiler kept its
intermediate representation locked inside itself as a private, throwaway structure. LLVM's
contribution was *not* inventing compilation and *not* inventing a language — it was extracting the
**middle** into a shared, inspectable, optimizable, verifiable IR. That one move made Clang, Rust,
Swift, and the sanitizers possible.

Slicers today are compilers *before LLVM existed*: they work, but their middles are black boxes, they
don't agree with each other, and nobody can check them. Kerf does to slicers what LLVM did to
compilers — it builds the open, **checkable** middle.

## The sharpened novelty claim (survives a hostile reviewer)

> Kerf's non-redundant contribution is an open, engine-independent IR for the mesh→G-code half that
> carries a **defined denotational semantics** (`denote(Program)` = the deposited material region),
> together with a **mechanically-checked lowering-soundness property** on a lowering Kerf *owns* —
> i.e. `denote(prog)` is preserved by the `hi→lo` lowering, and each optimization pass discharges a
> denote-preservation obligation on the move plan. (The concrete `lo`→G-code emitter is a lossy
> convenience backend *outside* the verified boundary; a verified end-to-end emitter is future work.)

This threads a real gap: LambdaCAD proves CAD→mesh and *stops at the mesh*; GlitchFinder tests slicers
as opaque black boxes and owns no IR or lowering; Carpentry Compiler/Szalinski do equality-saturation
over carpentry/CSG (their rewrite framework is the *reused*, not novel, part); OVF/3MF-Toolpath are
storage containers with no analysis semantics; OpenVCAD stops at geometry/voxels. Honestly narrowed:
rotation-invariance alone is **not** the contribution (GlitchFinder owns it) — here it becomes one
*derived* property checked over Kerf's denotation. The differentiator is "an oracle-checked lowering
with a written semantics," not "GlitchFinder again."

## Why this is worth doing (the honest version)

- **The idea being in a paper ≠ the tool existing.** Git and LLVM weren't new ideas either.
- **Slicers are provably unreliable.** GlitchFinder (OOPSLA 2025) showed Cura and PrusaSlicer produce *different* G-code for the same model in most tested cases, and mesh-repair tools *introduce* errors.
- **Verification is a near-empty field** and is the pillar that isn't redundant with prior art.

## Why this is hard (also the honest version)

Adoption is the real problem: Cura isn't extensible at the slicing-step level and OrcaSlicer has no
plugin architecture, so the "many front/back ends target the IR" LLVM story has no near-term adopter.
Kerf will mostly **consume** slicer output to verify it, not sit inside engines. Lean the pitch on the
consumer/verifier story until the verifier proves value. This is a slow, unglamorous systems project;
that's a reason to be clear-eyed, not a reason to skip it.

## Scope, by pillar (verification-first)

Start at the **mesh**, stop at **G-code**, never touch CAD.

1. **Verifier (the contribution).** `denote` — a reference semantics mapping a program to the material
   it deposits — plus a **self-lowering-soundness oracle**: `denote(hi) == denote(lower(hi))`.
2. **The IR (enabling infra).** A two-level, engine-independent, serializable representation:
   `hi` (geometric regions) and `lo` (move plan), joined by a lowering Kerf owns.
3. **Passes (enabling infra).** Composable transforms over `lo`, each of which must preserve
   denotation — "the first fabrication rewrites whose correctness the oracle checks."

## Repository layout

```
kerf/
├── Cargo.toml                       # Rust workspace
├── crates/
│   ├── kerf-core/                   # PURE RUST: IR + lowering + denote + passes + frontend + verify
│   │   └── src/
│   │       ├── ir/{mod,hi,lo}.rs    # shared geom; hi (regions) and lo (move plan) levels
│   │       ├── lower.rs             # hi -> lo lowering (the stage Kerf owns)
│   │       ├── denote.rs            # denotational semantics + Occupancy + self_lowering_sound
│   │       ├── pass/                # Pass trait (pure, denote-preserving) + TravelOrder
│   │       ├── metamorphic.rs       # translation-invariance relation (a la GlitchFinder rotation)
│   │       ├── frontend/gcode.rs    # robust, never-panic G-code -> IR parser (Cura/Prusa/Orca)
│   │       ├── verify.rs            # verify_gcode: parse + pass-soundness + metamorphic checks
│   │       ├── json.rs              # serde JSON boundary
│   │       └── backend.rs           # lo -> G-code (lossy; NOT the semantic reference)
│   ├── kerf-py/                     # THIN PyO3 bindings (abi3-py312); build.rs scopes macOS linker flag
│   └── kerf-cli/                    # the `kerf` binary (verify / inspect / diff)
├── python/kerf/                     # Python package (imports kerf._kerf)
├── examples/                        # sample.gcode + verify.py
├── docs/                            # research + design record (verify everything here)
│   ├── 00-thesis.md .. 07-design-review.md
│   └── sources.md
├── rust-toolchain.toml
└── .github/workflows/ci.yml         # Linux job: fmt + clippy + workspace tests + CLI smoke + wheel
```

## Install

Not yet published to crates.io / PyPI — build from source (Rust 1.90+, and [`uv`](https://docs.astral.sh/uv/) for the Python package):

```console
# CLI
$ cargo install --path crates/kerf-cli      # installs the `kerf` binary
# or run in place: cargo run -p kerf-cli -- <args>

# Python
$ uv sync                                   # builds + installs the `kerf` module (abi3, CPython ≥ 3.12)
```

## Quickstart

```console
# CLI — verify real slicer output
$ cargo run -p kerf-cli -- verify examples/sample.gcode
  ... SOUND — Kerf's operations preserve this print

# CLI — do two slicers / settings make the same part?
$ cargo run -p kerf-cli -- diff old.gcode new.gcode
  ... DIFFER — deposited material is not the same   (or IDENTICAL, exit 0)

# Python — same, on any G-code
$ uv sync && uv run python examples/verify.py
```

```python
import json, kerf
r = json.loads(kerf.verify_gcode(open("my.gcode").read()))
assert r["has_geometry"] and r["pass_preserves_denotation"] and r["translation_invariant"]
```

## Status

Research + scoping + five adversarial reviews complete. **Green and implemented** (55 core + 5 CLI
tests + property/fuzz pass + 4 machine-checked Kani proofs, clippy clean, CI covers the workspace + CLI
+ wheel):

- Two-level IR (`hi` regions / `lo` move plan), `hi→lo` lowering, `denote` reference semantics.
- The self-lowering-soundness **oracle** — reversal-invariant, conservative coverage (hardened after
  the soundness review found real bugs; see `docs/07`).
- The first real optimization **pass** (`TravelOrder`), checked by the oracle (cuts demo travel ~62%).
- A **G-code parser frontend** that reads real Cura / PrusaSlicer / OrcaSlicer / BambuStudio /
  Simplify3D / ideaMaker / KISSlicer / Slic3r output into the IR, including **arc (G2/G3) flattening**
  (I/J and R forms) — never panics on untrusted input (property-fuzzed), with a trust boundary and
  diagnostics. **Validated on real files** from all of the above (incl. a 136k-line PrusaSlicer Benchy
  and ArcWelder arcs); several real-world layer/role vocabulary gaps found and fixed this way (see
  `CHANGELOG.md`).
- **`verify_gcode`** — the delta beyond GlitchFinder: on *real parsed slicer geometry*, check that a
  Kerf pass preserves the deposited material **and** a second metamorphic relation (translation).
- **`kerf diff`** — compare two files by the material they deposit ("do these two slicers make the
  same part?"), with a `has_geometry`-style guard so two unparseable files never read as a match.
- A **`kerf` CLI** (verify / inspect / diff) and a **JSON boundary** for Python; abi3 bindings.
- Usable performance: a 24k-move / 200-layer file verifies in ~0.5 s (release).

## Known limitations (documented, not hidden)

- **Resolution-bounded.** `denote` compares material up to the raster resolution; sub-resolution
  differences are not distinguished (pinned by a test). Choose `resolution ≤ your smallest feature`.
- **Planar only.** 2D-per-layer IR; non-planar / vase-mode is out of scope by design.
- **Parser recovers deposited geometry**, not exact process state: widths without a `;WIDTH:` comment
  are estimated, feature roles are an untrusted re-inference (unknown → `Perimeter`, recorded), and
  pre-extrusion travel is elided. See `crates/kerf-core/src/frontend/gcode.rs`.
- **Oracle, plus first bounded proofs.** The end-to-end guarantee is property/metamorphic checking, not
  a full machine-checked proof. But the load-bearing kernels are now **verified by Kani** (bounded model
  checking): reversal invariance's mechanism for all `i64` endpoints, and the parser's coordinate math
  is panic-/overflow-free for all `f64`. A semantics-level proof over exact geometry remains future work
  (see `docs/08-semantics.md`).

The design record and remaining future work are in [`docs/06-architecture.md`](docs/06-architecture.md)
and [`docs/07-design-review.md`](docs/07-design-review.md). Genuinely-remaining work toward a
production tool: scale/perf for 100k+-move prints; a 3-OS + wheel publish matrix; and — the big
research target — lifting the now-discharged bounded (Kani) proofs of the kernels toward a
semantics-level, mechanized end-to-end proof over exact geometry (`docs/08-semantics.md` §6).
