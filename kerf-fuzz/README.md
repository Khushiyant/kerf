# kerf-fuzz

Differential + metamorphic fuzzing of 3D-printing slicers, using **kerf as the semantic oracle**.

Compilers got Csmith/EMI once someone had an oracle; slicers never had one. Kerf denotes real slicer
G-code to per-layer deposited material and has *proven* metamorphic invariants, so we can transform a
mesh, re-slice, and check the deposited material transformed the same way — **no ground truth needed**.
A separate consumer of `pykerf` (>=0.10.0); not part of the kerf library.

## Status: complete and validated

- **Self-validation (`validate.py`): 72/72 invariants hold, 0 µm drift** on the kerf reference slicer
  across 8 bug-class shapes — the oracle produces **no false positives**.
- **Mutation test:** a deliberately-leaky slicer (stray extrude outside the part) is **caught** by the
  containment gate + isometry relations, shrunk, and reported — **no false negatives** on a real defect.

## Soundness taxonomy (design doc §B)

Every relation carries a class so we only ever *fail* on sound signals:
- **GATE** — a violation is unconditionally a defect: `determinism` (same input → same output),
  `containment` (no material outside the part footprint, skirt/brim disabled), `differential`
  (two slicers must agree on *what* solid to fill), `crash`.
- **GRADED** — equal up to sub-cell rounding under a µm tolerance, compared translation-normalized so a
  re-position is never flagged: `rotate_z` (90/180/270 + arbitrary), `mirror_x`, `translate`.
- Determinism is checked **first** and gates the rest.

False-positive controls live in the adapters: fixed seed/threads, arc-fitting off, adaptive layers off,
and perimeter-dominant profiles so *world-anchored infill* can't confound the isometry checks.

## Layout

```
kerffuzz/
  instance.py   the Instance contract (STL + transforms + optional exact kerf program)
  shapes.py     prisms (2D polygon extruded in Z): kerf-exact AND STL; the self-validation core
  meshgen.py    3D CSG meshes (manifold3d, numpy fallback) + EMI semantics-preserving mutations
  corpus.py     bug-class-targeted instances (thin walls, tiny features, concave, holes, huge coords…)
  adapters.py   SlicerAdapter: KerfReference (no binary) + verified prusaslicer/curaengine/orca CLIs
  oracle.py     the invariants (determinism/rotation/mirror/translation/containment/differential)
  shrink.py     delta-debug a violation to a minimal reproducer
  report.py     JSON + self-contained HTML + per-finding repro bundles (STL + both G-codes)
validate.py     full oracle over the corpus on the reference slicer — must be 0 violations
run.py          the driver: sweep slicers × instances × invariants → shrink → report
DESIGN.html     the full design doc (also published as an Artifact)
```

## Run

```bash
uv venv && . .venv/bin/activate && uv pip install "pykerf>=0.10.0" numpy manifold3d
python validate.py                        # prove the oracle is sound (no external slicer)
python run.py                             # sweep with the reference slicer -> runs/report/report.html
```

On a machine with real slicers (the differential needs two or more):

```bash
python run.py --random 200 \
  --slicer "prusa:/path/profile.ini" \
  --slicer "cura:/path/fdmprinter.def.json" \
  --slicer "orca:machine.json,process.json,filament.json" \
  --out runs/hunt
```

`report.html` lists violations most-severe-first, color-coded by soundness class, each with a minimal
STL + both G-codes to reproduce. A GATE finding is a hard bug; GRADED/DIRECTIONAL warrant a look.

## Verified against real PrusaSlicer 2.9.6 (macOS)

```bash
brew install --cask prusaslicer
BIN=/Applications/PrusaSlicer.app/Contents/MacOS/PrusaSlicer
"$BIN" --save profile.ini                 # dump a self-contained default profile (bed/nozzle/etc.)
python -c "from run import sweep; from kerffuzz import corpus; from kerffuzz.adapters import prusaslicer; \
  sweep([prusaslicer('profile.ini', exe='$BIN')], \
        [(n,i) for n,i in corpus.boundary_corpus() if i.to_kerf_hi()], outdir='runs/prusa')"
```

The `prusaslicer` adapter bakes determinism + skirt/brim-off into a copy of the profile (PrusaSlicer
2.9's CLI does *not* accept those as flags) and auto-centres the part on the bed midpoint. Result over
the 8 bug-class prisms: **62/64 invariants held** — determinism, all four rotations, mirror, translate,
and containment clean on 7/8 shapes. The one GATE lead is `acute_wedge` containment (~58µm past the
bead-inset threshold at a sharp tip: PrusaSlicer under-insets the outer wall there) — a real, minor
over-reach the oracle is designed to surface, adjudicated by a human, not tolerance-hidden.

Two harness fixes this run surfaced (only a *repositioning* real slicer exposes them): containment is
now translation-normalized by bbox centre (slicers auto-place the part on the bed), and the `translate`
relation uses the same sub-cell tolerance as the other isometries (a re-rasterizing slicer has <1-cell
noise that the exact-reference 1µm tolerance wrongly flagged).

## Threats to validity (design doc §G)
World-anchored infill (controlled via profiles), the prism STL caps are convex-only (use the meshgen
CSG path for concave 3D), and a finding is only as sound as its class. GATE/GRADED are trustworthy;
anything else is a lead, not a verdict.
