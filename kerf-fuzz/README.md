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

### Campaign (`hunt.py`): 85 adversarial shapes, parallel

```bash
python hunt.py --exe "$BIN" --profile profile.ini --random 40 --workers 8 --out runs/hunt
```

Sweeps acute-tip / thin-wall / deep-concavity / near-circle / tiny-island / hole-ligament families
(swept across the parameter that matters) plus random shapes, parallelized across processes, deduping
each family to one representative lead. The first campaign flagged 16 raw violations; **adjudication
traced all the gross ones to a bug in the fuzzer itself** — `random_convex` produced non-convex
polygons and the prism STL triangulated caps with a fan (valid only for convex rings), so concave STLs
spilled outside the true footprint and the (correct) containment gate flagged the slicer for faithfully
filling an invalid mesh. Fixed at the root (`random_convex` now returns a convex hull; caps use a real
triangulator for concave rings). Post-fix, over 85 shapes: **0 determinism / rotation / mirror /
translate violations; containment clean on every convex and concave shape** except sub-nozzle sharp
tips (acute wedges, razor stars) where material over-reaches by ≤63µm (<⅓ cell) — monotonic in tip
sharpness, i.e. inherent FDM bead geometry, not a slicer defect. Sub-nozzle islands (0.2–0.3mm) produce
no G-code (the slicer drops features below the 0.4mm nozzle; surfaced by the adapter's no-G-code guard).

### Differential campaign (`campaign.py`): geometry × config × two slicers

```bash
python campaign.py --prusa-exe "$BIN" --prusa-profile profile.ini \
  --cura-exe "$CE" --cura-def fdmprinter.def.json --configs 12 --random 100 --out runs/campaign
```

1,680 units (140 shapes × 12 configs × PrusaSlicer 2.9.6 + Cura 5.13). 269 raw signals; adjudication
(re-testing each lead in isolation) left **8 surviving** — one filable defect, one quantified boundary,
one inherent effect, and the rest leads/harness — see `reports/trust_boundary.html` (published Artifact):

- **CuraEngine is nondeterministic on the sphere path** *(filable defect)*. A frozen STL (md5 re-checked
  identical before every run), sliced 6× single-threaded (`-m1`), gives **4 distinct deposited-material
  results**; PrusaSlicer gives 1. Threading and ASLR are both **experimentally excluded** — it stays
  nondeterministic under `-m1` + `OMP_NUM_THREADS=1` *and* with ASLR disabled (address pinned via
  `posix_spawn` `_POSIX_SPAWN_DISABLE_ASLR`, verified constant). Divergence is coordinate-level (same
  layers/point counts, ~200µm jitter), sphere-specific (cylinder + box deterministic) — most consistent
  with an uninitialized-memory read, mechanism unconfirmed. Minimal repro:
  `reports/cura_nondeterminism_sphere.stl`. (macOS note: `setarch -R` is Linux-only; the ASLR-off control
  used a `posix_spawn` launcher.)
- **Thin-wall rotational non-equivariance, quantified** *(known, not a defect)*. Below ~1.5× nozzle Cura
  deposits a wall ~700µm differently by orientation (resolution-independent to a 25µm grid); PrusaSlicer:
  0µm. **Both engines run Arachne** (PrusaSlicer ported it in 2.5) and this orientation-sensitivity at the
  2→1 transition is known — not a Cura defect. Equalizing the obvious Arachne params (`min_bead_width`,
  `min_feature_size`, `wall_transition_length`) did *not* reconcile them, which points at a **substrate**
  difference: CuraEngine computes on an integer-micron grid (ClipperLib) so a rotated wall quantizes
  before Arachne runs; PrusaSlicer's port doesn't share that grid. The contribution is the measurement,
  not the phenomenon. (`reports/divergence.json`.)
- Marginal/inherent: acute-tip containment over-reach (≤51µm). Leads: cross-slicer disagreement on
  multi-body STLs (the bbox differential is unsound for disjoint solids). Harness: determinism inflation
  under parallel load (pin `-m1`) and transient no-G-code under load (retry once) — both fixed.

Scaling from 85 to 1,680 units exposed harness limits, all fixed or documented: multithreaded slicers
need thread-pinning for the determinism gate; transient no-G-code under load needs a retry before it's
called a crash; rotation is an exact relation only for prisms; the differential needs per-component
comparison for multi-body solids. This is a smoke test toward a real campaign, not a "clean" verdict.

## Threats to validity (design doc §G)
World-anchored infill (controlled via profiles), and a finding is only as sound as its class. Prism STL
caps are now triangulated correctly for concave rings (via `manifold3d`), but hole interiors are still
not emitted in the prism STL — the containment gate is outer-only so this is not a false-positive
source, but hole geometry on real slicers must be exercised through the meshgen CSG path. GATE/GRADED
are trustworthy; anything else is a lead, not a verdict.
