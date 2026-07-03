# 01 — Prior art: "compilers for fabrication" is an established research line

**Verdict: the core framing is NOT novel.** Treating a CAD/fabrication toolchain as a compiler — source
language, intermediate representation, lowering passes, optimization, semantic verification — is
well-established, peer-reviewed prior art spanning ~2013–2023, out of UW PLSE, MIT CSAIL, and CMU's
textiles lab, plus mature open-source implementations. If you pitch the *idea* as novel, you will be
pointed to these papers. The novelty must be in the built artifact and the specific downstream gap
(see [05-direction.md](05-direction.md)).

Confidence on everything below: **high** — verified against primary PDFs / project pages via an
adversarial 3-vote verification pass (24/25 sampled claims confirmed).

## The framing itself is published

- **SNAPL 2017** has a section literally titled *"Compilers for 3D Printing"*: it frames the pipeline
  (CAD → STL → G-code → hardware signals) as multi-stage compilation and identifies the **slicer as the
  compiler middle-end**. It also names formal semantics for STL/G-code and proven-correct slicing as
  *unrealized future work*.
- **Carpentry Compiler** (SIGGRAPH Asia 2019, UW GRAIL): states outright that "both designs and
  fabrication plans are programs," and mirrors the hardware/software ISA split — a process-agnostic
  high-level DSL (HL-HELM) lowered to a process-specific target language (LL-HELM).
- **LambdaCAD / ReIncarnate** (ICFP 2018): "solid geometry is a programming language" — a functional CAD
  language plus a polygon-mesh IR.
- **OpenFab** (MIT, SIGGRAPH 2013): a programmable design→printer pipeline with shader-like "fablets"
  (analogized to RenderMan/GPU pipelines rather than compiler IRs).

## Compiler optimization machinery already transferred to fabrication

E-graph / equality-saturation **superoptimization** (a canonical modern compiler technique) has already
been applied to fabrication:

- **Carpentry Compiler (2019):** e-graph superoptimization of fabrication plans, extended to
  **multi-objective Pareto optimization** over precision, fabrication time, and material cost.
- **Szalinski (PLDI 2020):** equality saturation with semantics-preserving CAD rewrites to shrink CAD
  programs (over a CSG/"Caddy" IR).
- **Machine-knitting compilers (SIGGRAPH 2023):** proven-correct rewrite rules, machine-specific
  compilation, time/reliability optimization — "provably generating the same knit object."

**Caveat:** every demonstration is domain-specific (carpentry cuts, CSG, knitting). None is FDM slicing
or general CNC toolpaths.

## Semantic verification already demonstrated (at the front end)

- **LambdaCAD (ICFP 2018):** denotational semantics for both the CAD language and the mesh IR, plus a
  CAD→mesh compiler **with a semantics-preservation proof** (Theorem: `⟦compile(e)⟧ = ⟦e⟧`).
- **Carpentry Compiler:** real-time manufacturability verification (the fabrication analogue of front-end
  type checking).
- **Machine knitting (2023):** formal semantics for the low-level `knitout` DSL + rewrite-rule correctness.

**Important caveat:** most of these "verified" results are **pen-and-paper proofs** under idealized
semantics (e.g. real arithmetic while the implementation uses floats; curves relative to polyhedral
approximations), **not machine-checked / mechanized** proofs. A genuinely mechanized, end-to-end verified
fabrication compiler does not appear to exist.

## DSLs and decompilation are also covered

- **DSLs for fabrication:** OpenFab "fablets" (2013), HL-HELM / LL-HELM (2019), `knitout` (2023). So
  "a DSL for manufacturing" is not an open sub-idea.
- **Decompilation** (mesh → editable CAD/CSG via program synthesis): SNAPL 2017 prototype, LambdaCAD /
  ReIncarnate (2018), InverseCSG (ACM TOG 2018), CSGNet, and Szalinski's structured-CSG lift (2020).

## Mature open-source "programs-as-CAD with a real compiler front end"

- **libfive** — solid-modeling library using functional representations (f-rep); lowers expression trees
  to straight-line "tapes" (an IR) with interval-based tape simplification.
- **Fidget** (Matt Keeter, libfive's author) — a Rust library compiling large math expressions through
  textbook compiler machinery: math tree → DAG → **SSA form** → single-pass register allocation →
  bytecode, with a **JIT backend** lowering to native aarch64/x86_64 (~31× speedup over the interpreter).

Both stop at **geometry evaluation / meshing** — they do **not** cover slicing or G-code. (Source quality
note: libfive/Fidget claims rest on author-published pages, but are corroborated by a peer-reviewed
SIGGRAPH 2020 paper and public repos.)

## The gaps this prior art leaves open (→ Kerf)

Every formalized pipeline above terminates **at or before the mesh/CSG IR**:

- LambdaCAD's verified compiler stops at the mesh IR; its slicing and G-code steps are explicitly
  **prototypes, unproven**.
- Carpentry Compiler's own 2019 limitations flag: **no additive processes (3D printing)**, no free-form
  geometry, incomplete rewrite coverage, heuristic pruning with no optimality guarantee.
- SNAPL 2017 named formal STL/G-code semantics and proven-correct slicing as future work.

**Time-sensitivity caveat:** the 2017 gap on formal G-code semantics has been *partially* closed since —
"Mechanized Semantics for the RS274 Additive Manufacturing Command Language" (NFM/Springer 2025) and
"Formalizing Linear Motion G-code for Invariant Checking" (arXiv 2509.00699, 2025, the GlitchFinder
paper). These formalize G-code **for verification** but still do **not** provide an end-to-end,
semantics-preserving mesh → slice → G-code lowering. That specific gap remains open — and it is Kerf's
territory.

## Scope-of-evidence caveat

This body of confirmed evidence is almost entirely **academic and open-source**. It does **not**
substantiate what commercial slicers do internally — that was investigated separately (see
[02-slicer-architecture.md](02-slicer-architecture.md)).
