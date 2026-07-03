# 03 — The slicer research frontier (2024–2026): where the room actually is

**Executive verdict:** "Make a better planar FDM slicer than Cura/PrusaSlicer" is largely closed — those
tools are mature, free, and community-defended, and slicing *geometry* performance is not the bottleneck.
The genuinely open territory is at the edges and seams. Ranked by openness × defensibility × unmet demand:

1. **Toolpath IR ("LLVM for slicing")** — wide-open, highest leverage. ← **Kerf**
2. **Production non-planar / multi-axis slicer** — strongest *user* demand; but the moat is collision/kinematics, not compilers.
3. **Slicer correctness / verification** — near-empty field. ← **Kerf's second pillar**
4. **Slicer-native functionally-graded infill for commodity FDM** — real but narrower gap.

Avoid as primary bets: faster planar geometry slicing (not the bottleneck), metal LPBF/DED build-prep
(crowded, consolidating, physics-heavy), and yet-another deformation-field curved-layer paper (defended).

---

## 1. Intermediate representation / "slicing IR" — WIDE OPEN (Kerf's core)

**Does a standard, optimizable toolpath IR exist? NO.** The pieces are fragmented across three
non-overlapping buckets, none of which is a reusable, optimizable, verifiable IR:

- **Interchange/serialization containers (not optimizable):** 3MF (Core now ISO/IEC 25422:2025) with
  Slice/Beam-Lattice/Volumetric extensions and a **DRAFT Laser-Toolpath extension**; OpenVectorFormat
  (OVF, Fraunhofer, Protobuf-based, PBF-only); AMF; CLI. These *store* data; they carry no
  optimization/analysis semantics.
- **Geometry compilers (stop before toolpaths):** OpenVCAD (CU Boulder) — a "volumetric multi-material
  geometry compiler" over an implicit SDF core, but compiles design→geometry/voxels, not toolpaths;
  Carpentry Compiler's two-level HL/LL IR (carpentry, not AM).
- **Isolated G-code IR / formalization papers:** Xiaoming Li's "G-Code Re-compilation and Optimization"
  builds a higher-level IR over G-code and states outright "**there has been a lack of proper
  intermediate representation in the current slicing pipeline**"; plus GlitchFinder's formal semantics.
  Both are isolated prototypes, not standards.

Corroborating the gap: advanced toolpath optimizations (graph/Chinese-postman) exist in research but
"are not available through common slicing programs such as Cura and PrusaSlicer" (ORNL).

**Open problems:** (1) one IR general across extrusion paths, LPBF hatch vectors, and volumetric fields;
(2) preserving high-level intent while staying optimizable (the "missing middle" between SDF geometry and
G-code); (3) process-physics constraints (thermal, support, collision) as first-class IR invariants.

## 2. Non-planar / multi-axis slicing — strong USER demand, hard for non-compiler reasons

Dominant academic lineage is **Charlie C.L. Wang / Guoxin Fang (Manchester)** — NOT ETH (common
misattribution):
- **S3-Slicer** (SIGGRAPH Asia 2022, Best Paper) — quaternion deformation fields; open but Windows/Qt/oneMKL-locked.
- **Neural Slicer** (SIGGRAPH 2024) — differentiable neural deformation field.
- **INF-3DP** (SIGGRAPH Asia 2025) — implicit neural fields unifying toolpath + motion planning.
- **Curve-Based Slicer for Multi-Axis DLP** (SIGGRAPH Asia 2025, Best Paper) — **DLP/resin, not FDM**; code released.
- Accessible/community: **CurviSlicer** (INRIA 2019, 3-axis, open), **Open5x** (CHI 2022, Rhino/Grasshopper-locked), **S4-Slicer** (Joshua Bird, 2025 — open, generic, tetrahedralizes then reuses Cura).

**The hard part is NOT slicing math — it's collision-free motion planning coupled to slicing** (the
nozzle/gantry hits already-printed material), plus hardware/kinematics fragmentation (no hardware-agnostic
5-axis post-processor) and process physics on curved layers. A mainstream, robust, hardware-agnostic
non-planar slicer **does not exist** and demand is surging — but the moat is geometry/kinematics, not the
compiler angle. Good target if you want users; weaker fit for the compiler thesis.

## 3. Slicer correctness & robustness — NEAR-EMPTY FIELD (Kerf's second pillar)

**GlitchFinder** — "Formalizing Linear Motion G-code for Invariant Checking and Differential Testing of
Fabrication Tools," He (Utah), Nandi (Certora), Pai (Rochester), **OOPSLA/SPLASH 2025, PACMPL**
(arXiv 2509.00699; code at github.com/ymh1003/GlitchFinder). Method: lift G-code to cuboids + point
cloud, check a **rotation-invariance property** (rotate-then-slice ≡ slice-then-rotate) as a correctness
oracle.

**Verified findings:**
- Cura and PrusaSlicer produce **different G-code for the same model in the large majority of tested
  cases**, with neither uniformly faithful to the CAD source. (Exact split ~40/52 was in a truncated
  section; verified base is 56 models, 50 problematic benchmarks all correctly localized — treat the
  precise split as directional.)
- **Mesh-repair tools (MeshLab, Meshmixer) can introduce NEW errors during "repair."**
- Mesh slicing is numerically fragile at degenerate configs (coplanar triangles, >2 triangles at a point).

**Open problems:** correctness oracles beyond linear-motion + planar (arcs G2/G3, variable height,
non-planar); a true formal fabrication semantics (rotation-invariance is only a proxy); provably-safe
(monotone) mesh repair. **Wide-open** — GlitchFinder is essentially the first serious PL-style attack on
slicer correctness and has no direct competitor.

## 4. Adaptive layers & functionally-graded infill — partly open

All mainstream slicers use single geometry-error heuristics for variable layer height (nothing
perceptual/stress-aware). Graded infill in the open FDM world is essentially **one hobby script**
(CNC Kitchen's GradientInfill, which modulates *flow*, not toolpaths). Fiber/stress-aligned toolpaths and
TPMS materials studies are **crowded**; a unified adaptive-layer + graded-density framework does **not**
exist. Real but narrower than the IR gap.

## 5. Programmatic G-code (FullControl et al.) — concept populated but fragmented

**FullControl** (Andrew Gleadall, Loughborough; *Additive Manufacturing* 2021 + Python `fullcontrol`)
gives total explicit toolpath control for things slicers can't do. Neighbors: Silkworm/Droid (Grasshopper),
COMPAS_SLICER (ETH), p5.fab (DIS 2022). **No automatic support or collision avoidance** — expert-only.
Fragmented across ecosystems (Excel/VBA, Python, Grasshopper, p5.js) with no interop and no accessible
standard. Open at the "usable design-by-toolpath" layer — and a natural *front end* that could target
Kerf's IR.

## 6. GPU / performance — mostly NOT the bottleneck

For desktop FDM, the geometric slicing step is fast; slowness is single-threaded **infill/perimeter/
G-code generation**, not contour extraction. Real exception: industrial metal AM (million-triangle data
prep). **Dyndrite** has a genuine CUDA-native geometry kernel (productized as LPBF Pro for metal, not
FDM) — its speedup figures are single-vendor marketing, treat as directional. No serious modern
WebGPU/compute-shader FDM slicer exists, but the payoff is limited since geometry slicing isn't the
bottleneck.

**Flagged FALSE — do not cite:** a repo claiming "OrcaSlicer GPU G-code / AI auto-calibration" is an
SEO/typosquat; real OrcaSlicer is CPU-based.

## 7. Metal AM (LPBF/DED) — CROWDED, avoid as primary bet

Fundamentally a *physics* problem (exposure/vector files with per-vector energy, ~67°/layer rotating hatch,
simulation-coupled pre-deformation), not a geometry→motion problem. Crowded/consolidating: Materialise
Magics, Oqton/3DXpert, Autodesk Netfabb, Dyndrite LPBF Pro, OEM tools. The one open frontier is the
**standards/interoperability layer** (OpenVectorFormat) — which is adjacent to, and could inform, Kerf's
IR design for multi-process generality.

---

## Corrections & do-not-cite list (verified during research)

- S3/Neural-Slicer lineage is **Manchester (Wang/Fang), not ETH**. "Roland Aigner + ETH non-planar"
  could not be verified — likely misattribution.
- Curve-Based Slicer is **DLP/resin, not FDM**.
- **Do NOT cite:** "68% of failed prints from non-manifold meshes" (no primary source); the "OrcaSlicer
  GPU G-code" repo (typosquat); an anomalous future-dated arXiv ID; all vendor % figures
  (Dyndrite/Materialise/Amphyon — directional marketing only).
- GlitchFinder's precise Cura-vs-PrusaSlicer model split is directional (truncated source section).
