# 02 — Commercial slicers are already compilers (implicitly)

**Verdict:** Structurally, CuraEngine and PrusaSlicer/OrcaSlicer are *already* compilers — ordered
pipeline stages, an explicit in-memory intermediate representation between mesh and G-code, dedicated
optimization/path-planning passes, and a discrete code-generation backend. **But** the *vocabulary* is
absent (they say "pipeline / stages / backend," never "IR / pass / lowering"), and — crucially — each
IR is **private, engine-specific, un-reusable, and unverified**. The compiler framing lives only in
academic PL literature, not in the maintainers' code.

This is exactly why Kerf is not redundant: the machinery exists but has never been extracted into a
shared, inspectable, verifiable layer.

Source: an agent read the actual `main`/`master` source of both engines. Class/enum names below are
verbatim from the codebases.

## CuraEngine (Ultimaker Cura's C++ backend)

**Five named pipeline stages** (official developer portal): Slicing → Generating Areas → Generating
Paths → Inserts → G-code. Some stages run in a producer–consumer threading pattern.

| Compiler concept | CuraEngine class(es) |
|---|---|
| Front end (mesh → 2D contours) | `Slicer` / `SlicerLayer` / `SlicerSegment` |
| Central IR | `SliceDataStorage` → `SliceMeshStorage` → `SliceLayer` → `SliceLayerPart` → `SkinPart` |
| Geometry primitive | `Polygon` / `Shape` (ClipperLib, 64-bit integer micron coords, `Point2LL`) |
| **Lowered IR** (move plan) | `LayerPlan` → `ExtruderPlan` → `GCodePath` |
| Optimization passes | `PathOrderOptimizer`, `InsetOrderOptimizer`, `Comb` (travel avoidance), `LayerPlanBuffer` (cross-layer look-ahead), `ZSeamConfig` |
| Backend / codegen | `FffGcodeWriter` + `GCodeExport` |

`GCodePath` is described in its own header as "a compact premature representation in which all line
segments have the same config" — i.e. a lowered IR node. The pipeline doc even says G-code is
"translated from CuraEngine's **internal representation**" — the closest any maintainer doc comes to
saying "IR."

Naming caveats: `GCodePlanner` is the *legacy* name for today's `LayerPlan`; `Polygons` (plural) is
legacy for `Shape`.

## PrusaSlicer / OrcaSlicer (libslic3r)

A **step state machine** driven by two enums (verbatim from `Print.hpp`), which is essentially a
demand-driven pass manager with dependency-based invalidation:

- `PrintObjectStep`: `posSlice, posPerimeters, posPrepareInfill, posInfill, posIroning,
  posSupportSpotsSearch, posSupportMaterial, ... posCount`
- `PrintStep`: `psWipeTower, psToolOrdering, psAlertWhenSupportsNeeded, psSkirtBrim, psGCodeExport, psCount`

| Compiler concept | PrusaSlicer class(es) |
|---|---|
| Front end | `posSlice`: `TriangleMesh` → `ExPolygon` |
| Central geometry IR | `Layer` → `LayerRegion` → `Surface` (with `SurfaceType`: `stTop`, `stBottom`, `stInternal`, ...) / `SurfaceCollection` |
| **Toolpath IR** | `ExtrusionEntity` → `ExtrusionPath` / `ExtrusionMultiPath` / `ExtrusionLoop` / `ExtrusionEntityCollection` |
| Optimization passes | `SeamPlacer`, `AvoidCrossingPerimeters` (+ `EdgeGrid` spatial index), `ToolOrdering` |
| Backend / codegen | `psGCodeExport` → `GCode::do_export()` + `GCodeWriter` (+ `PlaceholderParser`, `WipeTower`) |

`Print::process()` walks the steps via `set_started()`/`set_done()`; `invalidate_state_by_config_options()`
re-invalidates affected steps when config changes — an incremental-build / pass-manager pattern.

## Compiler terminology: structure present, vocabulary absent

- Both engines call their slicer the "backend"; Cura's GUI plugin is literally `CuraEngineBackend`.
- The words **"intermediate representation," "pass," "lowering," "AST"** do not appear in the
  maintainer docs (Cura says "internal representation" once).
- The full compiler framing appears only in PL papers, e.g. the GlitchFinder paper (arXiv 2509.00699):
  a mesh "functions like a conventional intermediate representation (IR)," then is "'compiled' to
  instructions"; CAD is "lowered to polygon meshes to be ultimately compiled to machine code by 3D
  slicers." Hackaday casually calls G-code "the assembly language of 3D printers."

## Known architectural pain points / technical debt (documented)

- **Cura:** historically not extensible at the slicing-step level (plugins limited to front end +
  post-processing); ClipperLib 64-bit-integer micron representation is limiting (open issue to migrate to
  Clipper2); settings/`ExtruderTrain` coupling has documented footguns.
- **PrusaSlicer:** Prusa itself publicly cited a "massive amount of technical debt"; a contributor called
  the code "very convoluted"; Arachne (variable-width perimeters) is a recurring source of output bugs;
  multi-generational fork divergence (Slic3r → PrusaSlicer → Bambu Studio → OrcaSlicer).
- **Both:** the **layer-based architecture is a documented barrier to non-planar slicing**.

Flagged **unverified** (no maintainer source found): `FffGcodeWriter` being "monolithic"; specific
memory-usage complaints; the step-invalidation system being "buggy."

## What this means for Kerf

The compiler machinery is real and battle-tested — but it is trapped inside two incompatible, private,
unverified engines. Kerf's contribution is to lift that middle into:

1. a **shared** IR (not `SliceDataStorage`-locked or `ExtrusionEntity`-locked),
2. **reusable, testable** passes (not engine-internal C++),
3. a **verification** layer these engines completely lack.
