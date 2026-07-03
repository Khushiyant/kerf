# 05 — Direction: what Kerf is, what it isn't, and where to start

## The one-line scope

An **open, engine-independent IR for the mesh → toolpath (G-code) half** of the fabrication pipeline
that has a defined *meaning* (`denote`) and a lowering whose correctness is mechanically checked. LLVM
for slicing — but the contribution is the *verifier*, not the container. Start at the mesh, stop at
G-code, never touch CAD.

## What Kerf is NOT

- **Not another slicer competing with OrcaSlicer on UX.** That market is closed ([04-pain-points.md](04-pain-points.md)).
- **Not a claim that "compilers for manufacturing" is a new idea.** It isn't ([01-prior-art.md](01-prior-art.md)).
- **Not a CAD/geometry kernel.** libfive/Fidget already do that half well.
- **Not a metal-AM build-prep tool.** Crowded and physics-heavy ([03-research-frontier.md](03-research-frontier.md) §7).
- **Not a faster planar slicer.** Slicing-geometry speed isn't the bottleneck.

## What Kerf IS — verification-first (per the design review)

The adversarial design review ([07-design-review.md](07-design-review.md)) reordered these: the
verifier is the contribution; the IR and passes are enabling infrastructure that would otherwise be
redundant with prior art on their own.

1. **The verifier (the contribution).** `denote` — a reference semantics mapping a program to the
   material it deposits — plus a **self-lowering-soundness oracle**: `denote(hi) == denote(lower(hi))`.
   This is a property GlitchFinder structurally *cannot* state, because it owns no IR or lowering. The
   first artifact is self-lowering soundness over a synthetic program generator, **not** reproducing
   GlitchFinder's "two slicers disagree" result through an IR wrapper (that would be redundant).
   Rotation-invariance becomes one *derived* property later, checked over `denote`, not the headline.
2. **The IR (enabling infra).** A two-level, engine-independent, serializable representation — `hi`
   (geometric regions) and `lo` (move plan) — joined by a lowering Kerf owns. On its own, a
   tagged-toolpath container is redundant with OpenVectorFormat / 3MF-Toolpath; its value here is being
   the thing the verifier and passes operate over.
3. **Passes (enabling infra).** Composable transforms over `lo`, each of which must preserve
   denotation. E-graph superoptimization is *reused* technique (Carpentry Compiler/Szalinski), not a
   standalone contribution.

(Non-planar is higher *user* demand but its moat is collision/kinematics, not compilers — a different
project.)

## Why it's defensible despite the prior art

- The prior-art verified lowerings **stop at the mesh**; the mesh→G-code half has only unproven prototypes
  ([01-prior-art.md](01-prior-art.md)).
- No shared, optimizable toolpath IR exists — even a G-code-optimization paper says so outright
  ([03-research-frontier.md](03-research-frontier.md) §1).
- Slicers are demonstrably unverified and mutually inconsistent ([03-research-frontier.md](03-research-frontier.md) §4).
- An open IR can *consume* existing slicers' G-code output to verify it, rather than competing with them
  on UX — and it rides the ecosystem's openness/lock-in anxiety ([04-pain-points.md](04-pain-points.md)).
  (Note the adoption caveat: Cura isn't extensible at the slicing step and OrcaSlicer has no plugin API,
  so "engines target the IR" has no near-term adopter — lean on the consumer/verifier story.)

## The honest risk register

- **Slow payoff, no applause.** "I made an IR" doesn't demo well. Value accrues over time.
- **Adoption is the real hard problem.** An IR nobody targets is a data structure. Consider designing to
  *consume existing slicer output* and/or *emit standard G-code* first, so it's useful stand-alone before
  anyone else adopts it.
- **Verification scope creep.** Full formal semantics is a multi-year research program. Start with a
  concrete, checkable property (rotation invariance), not a grand semantics.
- **"Richer than STL" tension.** STL discards design intent; a good IR may need more than triangles as
  input. That's a deliberate design axis, not a bug — but don't let it drag you back into CAD.

## Progress and next steps

**Done in v0** (see [06-architecture.md](06-architecture.md) and [07-design-review.md](07-design-review.md)):
the two-level IR (`hi`/`lo`), the `hi→lo` lowering, `denote` (reference semantics), the
self-lowering-soundness oracle with a property test, a naive G-code backend, and Python bindings.
Foundation hygiene (abi3 wheel, serde behind a feature, `.gitignore`, CI, `rust-toolchain.toml`) is in place.

**Next** (from the reviewed build order):

1. **First real pass** (`fn run(&self, lo::Program) -> lo::Program`, e.g. travel-order or seam), each
   discharging a denote-preservation obligation the oracle checks — the first rewrite the oracle validates.
2. **Expose the IR to Python via a serde JSON boundary** (not `#[pyclass]` per type) so the research loop
   works from Python.
3. **G-code parser frontend** (parse comment-annotated real slicer output; add `cargo-fuzz`), enabling
   differential testing of real slicers as a *derived*, region-aware capability.
4. **Name and build the delta beyond GlitchFinder** — own-lowering/own-pass validation and a second
   metamorphic relation reusing the oracle harness.

## Decision checkpoint

If the "open + verifiable middle layer" excites you, the wedge is clear and the v0 slice already stands.
If the grind-with-slow-payoff profile doesn't appeal, this is a clean place to stop — you know exactly
where the line between "already built" and "genuinely missing" sits.
