# Kerf

**An open, engine-independent IR for the mesh → G-code half of 3D printing — with a written-down denotational semantics and a lowering whose correctness is mechanically checked.**

Think "LLVM for slicing," but the point is the *verifier*, not the container.

Slicers (Cura, PrusaSlicer, OrcaSlicer) are compilers: they lower geometry into machine code (G-code),
but each buries that machinery in a private codebase, and no mainstream slicer can show its output
corresponds to the input. Kerf is the open middle: an IR whose meaning is defined (`denote` = the
material a program deposits), a lowering Kerf owns, and an oracle that checks the lowering preserves
that meaning. Today it mostly **consumes** slicer G-code to verify it.

## Install

```console
# CLI (the `kerf` binary)
cargo install kerf-cli

# Python (CPython ≥ 3.12)
pip install pykerf

# Server + dashboard
docker run -p 8080:8080 ghcr.io/khushiyant/kerf
```

## Quickstart

```console
# Verify: do Kerf's operations preserve this print?
kerf verify part.gcode              # exit 0 sound · 1 unsound · 3 nothing to verify

# Diff: do two slicers / settings make the same part?
kerf diff old.gcode new.gcode       # exit 0 identical · 1 differ

# Inspect: what did the parser recover, guess, or drop?
kerf inspect part.gcode
```

```python
import json, pykerf
r = json.loads(pykerf.verify_gcode(open("part.gcode").read()))
assert r["has_geometry"] and r["pass_preserves_denotation"] and r["translation_invariant"]
```

## What it does

- **Two-level IR** — `hi` (geometric regions) and `lo` (move plan), joined by a lowering Kerf owns.
- **`denote`** — reference semantics: a program's deposited material as conservative raster occupancy,
  reversal-invariant.
- **Soundness oracle** — checks the lowering and each optimization pass preserve `denote`; a negative
  test confirms a material-dropping pass is rejected.
- **G-code frontend** — parses real Cura / PrusaSlicer / OrcaSlicer / Bambu / Simplify3D / KISSlicer /
  ideaMaker / Slic3r output, including arc (G2/G3) flattening; never panics on untrusted input.
- **`kerf verify` / `kerf diff`** — verification and material comparison over real parsed geometry,
  with CI-friendly exit codes.
- **Proofs** — P1–P4 proved in Lean 4 (no `sorry`); load-bearing kernels model-checked with Kani.

## Limitations

- **Resolution-bounded.** `denote` compares material up to the raster resolution; choose
  `--resolution ≤` your smallest feature. Sub-resolution differences are not distinguished.
- **Planar only.** 2D-per-layer IR; non-planar / vase mode is out of scope.
- **Deposited geometry, not process state.** Widths without a `;WIDTH:` comment are estimated; feature
  roles are an untrusted re-inference. The `lo`→G-code emitter is lossy and sits outside the verified
  boundary.
- **Checked oracle, not an end-to-end proof.** A semantics-level mechanized proof over exact geometry
  is future work.

## Repository

```
crates/kerf-core   IR, lowering, denote, passes, G-code frontend, verify/diff
crates/kerf-cli    the `kerf` binary
crates/kerf-py     PyO3 bindings (published to PyPI as pykerf)
crates/kerf-{api,engine,store,queue,worker,ingest,render}   verification service + dashboard
proofs/            Lean 4 proofs of P1–P4
docs/              design record and semantics
```

The full design rationale, prior-art scoping, and semantics live in [`docs/`](docs/) — start with
[`docs/00-thesis.md`](docs/00-thesis.md) and [`docs/08-semantics.md`](docs/08-semantics.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
