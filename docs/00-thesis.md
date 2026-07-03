# 00 — Thesis: what Kerf is, and why "it's all already built" is wrong

## The question this document answers

> "You told me CAD→mesh is already done. You told me slicing is already done.
> So what are we even making?"

Both of those statements are true. Neither of them is Kerf. Here is the precise distinction.

## The LLVM analogy (the whole idea in one picture)

```
   BEFORE LLVM                            AFTER LLVM
   ───────────                            ──────────
   C source                               C / Rust / Swift / ...  (many front ends)
      │                                       │
      ▼                                       ▼
   [GCC: private internal guts]           LLVM IR   ◄── open, inspectable, optimizable, reusable
      │                                       │
      ▼                                       ▼
   x86 assembly                           x86 / ARM / WASM / ... (many back ends)
```

Compilers existed for decades before LLVM. GCC compiled C perfectly well. LLVM did **not** invent
compilation, and it did **not** invent a programming language. Its entire contribution was to pull the
**middle** — the intermediate representation — out of the private guts of one compiler and make it a
shared, documented, inspectable, optimizable, verifiable thing that any front end could target and any
back end could consume.

That one move is why Clang, Rust, Swift, address-sanitizer, and half of modern static analysis exist.
The value was not novelty of concept. It was building the middle *properly and openly*.

## Now map that onto fabrication

```
   TODAY (slicers)                        WITH KERF
   ──────────────                         ─────────
   mesh (STL)                             mesh / implicit / voxel  (front ends)
      │                                       │
      ▼                                       ▼
   [Cura / PrusaSlicer:                   KERF IR  ◄── open, inspectable, optimizable, VERIFIABLE
    private internal guts]                    │
      │                                       ▼
      ▼                                   G-code / scan vectors / toolpaths (back ends)
   G-code
```

- **CAD → mesh** is the *front end*. Done. Multiple mature, even formally verified, implementations
  (libfive, Fidget, LambdaCAD). We do not touch it.
- **Slicing (mesh → G-code)** is the *back end / code generation*. It "works" in the sense that Cura
  emits G-code. But it is done the way compilation was done **before LLVM**: as a monolithic black box
  with a private, throwaway internal representation, no way to inspect or optimize the middle from
  outside, and — critically — **no correctness guarantee**.

**Kerf is the missing middle.** The open, inspectable, optimizable, verifiable IR between geometry and
machine code, plus passes over it, plus a correctness oracle.

## So "slicing is done" is like saying "compiling was done in 1990"

It was! GCC worked. And yet LLVM was one of the most important systems projects of the last 25 years,
precisely *because* "it works" and "it is open, reusable, and verifiable" are completely different bars.

Slicers clear the first bar and fail the second:

1. **No shared IR.** Cura's internal representation (`SliceDataStorage`, `LayerPlan`, `GCodePath`) and
   PrusaSlicer's (`Layer`, `LayerRegion`, `ExtrusionEntity`) are private, incompatible, and locked to
   their engines. You cannot target them, extend them, or reuse them. (See [02-slicer-architecture.md](02-slicer-architecture.md).)
2. **No verification.** No slicer checks that its G-code corresponds to the input geometry. When someone
   finally did (GlitchFinder, OOPSLA 2025), Cura and PrusaSlicer produced *different* G-code for the
   same model in the majority of cases, and neither was reliably faithful to the source. Mesh-repair
   tools *introduce* errors. (See [03-research-frontier.md](03-research-frontier.md) §4.)
3. **No composable optimization.** Passes like travel ordering and seam placement exist, but as
   engine-internal code — not as reusable, testable transforms over a public IR.

## What "the difference we are making" is, in one sentence

> We are not slicing better than Cura. We are turning slicing from an opaque, unverified black box
> into an open intermediate representation that has a defined *meaning* and a lowering whose
> correctness is mechanically *checked* — the same move LLVM made for compilers. (A mechanically-checked
> oracle, not a machine-checked proof — see the trust model in [06-architecture.md](06-architecture.md).)

## The honest caveats (so you can verify, not just trust)

- The **framing** ("fabrication pipeline is a compiler") is genuinely old and published. If you pitch
  *that* as your novelty, you will be pointed to 2017–2019 papers. The novelty is the built artifact,
  not the metaphor. (See [01-prior-art.md](01-prior-art.md).)
- The academic work that *did* build IRs and verified lowerings stops at or before the **mesh** (CAD→mesh).
  The mesh → G-code half has prototypes but no verified, reusable IR. That downstream gap is real and is
  where Kerf lives. (See [01-prior-art.md](01-prior-art.md) and [05-direction.md](05-direction.md).)
- This is a hard, slow, unglamorous project competing with beloved free tools. That is a reason to be
  clear-eyed, not a reason it isn't worth doing. (See [05-direction.md](05-direction.md).)
