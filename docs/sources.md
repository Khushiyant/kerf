# Sources

Every URL cited across the research, grouped by topic. Claims in the `docs/` files trace back here.
Verification note: the first research pass ran an adversarial 3-vote check (24/25 sampled claims confirmed,
1 refuted and excluded). Slicer-architecture and frontier claims were verified against primary source /
GitHub API where noted. Items flagged **DO NOT CITE** failed verification.

## Academic "compilers for fabrication" prior art

- Carpentry Compiler (SIGGRAPH Asia 2019) — https://dl.acm.org/doi/10.1145/3355089.3356518 · project: https://grail.cs.washington.edu/projects/carpentrycompiler/
- Carpentry co-optimization follow-up (ACM TOG 2022) — https://dl.acm.org/doi/10.1145/3459666 · arXiv:2107.12265
- SNAPL 2017, "Compilers for 3D Printing" — https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.SNAPL.2017.10 · PDF: http://cnandi.com/docs/snapl17-cr.pdf · https://jamesrwilcox.com/3dp.pdf
- LambdaCAD / ReIncarnate (ICFP 2018) — https://dl.acm.org/doi/10.1145/3236794
- OpenFab (SIGGRAPH 2013) — http://openfab.mit.edu/
- Szalinski (PLDI 2020) — https://dl.acm.org/doi/10.1145/3385412.3386012 · repo: https://github.com/uwplse/szalinski
- Machine-knitting compilers (SIGGRAPH 2023) — https://dl.acm.org/doi/10.1145/3592449
- InverseCSG (ACM TOG 2018) — inverse CSG as program synthesis

## Geometry kernels / programs-as-CAD (the solved front-end half)

- Fidget (Matt Keeter) — https://www.mattkeeter.com/projects/fidget/ · repo: https://github.com/mkeeter/fidget
- libfive — https://libfive.com/ · repo: https://github.com/libfive/libfive
- Curv — https://github.com/curv3d/curv/blob/master/docs/Slides.rst
- curated-code-cad (survey list) — https://github.com/Irev-Dev/curated-code-cad
- OpenVCAD (volumetric geometry compiler, CU Boulder) — https://www.sciencedirect.com/science/article/pii/S2214860423005250 · repo: https://github.com/MacCurdyLab/OpenVCAD-Public

## Slicer architecture (primary source / codebase)

- CuraEngine pipeline (dev portal) — https://ultimaker.github.io/CuraEngine/docs/pipeline.html
- CuraEngine internals wiki — https://github.com/Ultimaker/CuraEngine/wiki/Internals
- CuraEngine source — https://github.com/Ultimaker/CuraEngine/ (FffPolygonGenerator.cpp, FffGcodeWriter.cpp, LayerPlan.h, sliceDataStorage.h, PathOrderOptimizer.h, Comb.cpp, gcodeExport.cpp)
- PrusaSlicer Print.hpp (step enums) — https://raw.githubusercontent.com/prusa3d/PrusaSlicer/master/src/libslic3r/Print.hpp
- PrusaSlicer Surface.hpp / ExtrusionEntity.hpp / LayerRegion.cpp / GCode.cpp — https://github.com/prusa3d/PrusaSlicer/blob/master/src/libslic3r/
- PrusaSlicer FFF pipeline (DeepWiki) — https://deepwiki.com/prusa3d/PrusaSlicer/4.1-fff-slicing-pipeline
- OrcaSlicer FFF process (DeepWiki) — https://deepwiki.com/SoftFever/OrcaSlicer/4.1-fff-printing-process
- Cura architecture wiki — https://github.com/Ultimaker/CuraEngine/wiki/Architecture

### Slicer technical debt / architecture pain
- Cura extensibility discussion — https://github.com/Ultimaker/Cura/discussions/15629
- Cura Clipper2 migration issue — https://github.com/Ultimaker/CuraEngine/issues/1744
- PrusaSlicer 3.x "technical debt" — https://forum.prusa3d.com/forum/english-forum-general-discussion-announcements-and-releases/prusaslicer-3-x-update/
- Non-planar barrier analysis — https://xyzdims.com/2021/04/10/3d-printing-non-planar-slicing-with-planar-slicer/ · https://github.com/prusa3d/PrusaSlicer/issues/2704

## Correctness / verification (Kerf's second pillar)

- GlitchFinder (OOPSLA/SPLASH 2025) — https://arxiv.org/abs/2509.00699 · https://dl.acm.org/doi/10.1145/3763106 · code: https://github.com/ymh1003/GlitchFinder · artifact: https://zenodo.org/records/16595028
- Mechanized Semantics for RS274 AM Command Language (NFM/Springer 2025) — doi:10.1007/978-3-031-93706-4_20
- G-Code Re-compilation and Optimization (IR over G-code) — doi:10.1007/978-3-030-95953-1_8 · https://link.springer.com/chapter/10.1007/978-3-030-95953-1_8
- CGAL slicer numerical fragility — doi:10.1007/s00170-020-06396-2
- Malicious G-code (USENIX Security 2025) — Rossel et al.

## Non-planar / multi-axis slicing

- S3-Slicer (SIGGRAPH Asia 2022) — https://dl.acm.org/doi/10.1145/3550454.3555516 · code: https://github.com/zhangty019/S3_DeformFDM
- Neural Slicer (SIGGRAPH 2024) — https://arxiv.org/abs/2404.15061 · code: https://github.com/RyanTaoLiu/NeuralSlicer
- INF-3DP (SIGGRAPH Asia 2025) — arXiv:2509.05345
- Curve-Based Slicer for Multi-Axis DLP (SIGGRAPH Asia 2025, DLP not FDM) — https://chengkai-dai.github.io/curved_dlp_slicer/ · code: https://github.com/chengkai-dai/CurveSlicer
- CurviSlicer (INRIA, SIGGRAPH 2019) — https://github.com/mfx-inria/curvislicer
- Open5x (CHI 2022) — https://arxiv.org/abs/2202.11426
- S4-Slicer (Joshua Bird, 2025) — https://github.com/jyjblrd/S4_Slicer
- ETH robotic non-planar (architecture) — arXiv:2501.06088
- Curved-layer support (open problem) — arXiv:2302.05510

## Adaptive layers / graded infill / programmatic G-code

- PrusaSlicer variable layer height — https://help.prusa3d.com/article/variable-layer-height-function_1750
- GradientInfill (hobby post-processor) — https://github.com/CNCKitchen/GradientInfill
- Implicit gradient-informed slicing (Wade/Beck/MacCurdy 2025) — arXiv:2505.08093
- Stress-guided layout benchmark SGLDBench — arXiv:2501.03068
- FullControl — https://github.com/FullControlXYZ/fullcontrol · Additive Manufacturing 2021, doi:10.1016/j.addma.2021.102109
- ORNL toolpath path-optimization — arXiv:1908.07452
- ML for slicer parameter selection — arXiv:2506.12252

## Interchange formats / standards (containers, not optimizable IRs)

- 3MF spec + Laser-Toolpath (draft) extension — https://3mf.io/spec/ · https://github.com/3MFConsortium/spec_lasertoolpath
- OpenVectorFormat (Fraunhofer, PBF) — Protobuf-based vector format

## Performance / GPU / commercial

- PrusaSlicer perf bottleneck issue — https://github.com/prusa3d/PrusaSlicer/issues/6295
- Dyndrite GPU geometry kernel — https://developer.nvidia.com/blog/dyndrite-unveils-first-gpu-accelerated-geometry-kernel-to-tackle-data-explosion-in-additive-manufacturing/ · https://www.dyndrite.com/
- Materialise Build Processor / e-Stage — https://www.materialise.com/en/industrial/software/build-processor
- ModuleWorks toolpath components — https://www.moduleworks.com/software-components/toolpath/
- LPBF slicing/hatching primer — https://lukeparry.uk/slicing-and-hatching-for-selective-laser-melting/

## Ecosystem / pain points

- Fork genealogy — https://simplyprint.io/articles/orcaslicer-forks-compared · https://simplyprint.io/articles/bambu-studio-vs-orcaslicer
- Bambu Authorization Control — https://3dprintingindustry.com/news/bambu-lab-controversy-deepens-firmware-update-sparks-backlash-240588/ · https://hackaday.com/2025/01/17/new-bambu-lab-firmware-update-adds-mandatory-authorization-control-system/ · https://all3dp.com/4/bambu-labs-controversial-authorization-control-hits-budget-3d-printers/
- Support/seam/retraction bug threads — Orca #5330, #6266, #7326, #14331, #5621; Prusa #12497, #12588, #12856, #14542, #13173; Cura #15264, #12190, #21438
- Feature-request wishlists (all open mid-2026) — Prusa #1823 (brick layers), #2794 (non-planar), #10321, #4898; Cura #18353, #5980, #14036; Orca #7282, #8181, #2955, #7106
- Best-slicer roundups — https://3dprinting.com/software-guides/best-3d-printer-slicers/ · https://www.xda-developers.com/orcaslicer-best-3d-printing-software-everyone-should-use/

## DO NOT CITE (failed verification)

- "68% of failed prints from non-manifold meshes" — no primary source found.
- "OrcaSlicer GPU-accelerated G-code / AI auto-calibration" repo — SEO/typosquat; real OrcaSlicer is CPU-based.
- All vendor speedup/accuracy % figures (Dyndrite/Materialise/Amphyon) — single-vendor marketing, directional only.
- Anomalous future-dated "CelloCut" arXiv ID — could not verify.
- "Roland Aigner + ETH non-planar" attribution — unverified; S3/Neural-Slicer lineage is Manchester (Wang/Fang).
- One refuted claim (pre-2023 knitting representations implying G-code/CNC verification is open "by analogy") — voted down 0-3, excluded.
