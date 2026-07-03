# Kerf — machine-checked soundness proofs (Lean 4)

This is a **real proof**, not a test: `lake build` runs the Lean kernel over `KerfProofs.lean` and only
succeeds if every theorem is derived with no gaps. The theorems depend on **`propext` and `Quot.sound`
only** — two of Lean's three standard trusted axioms — and **no `sorry`** (audited by the `#print axioms`
lines at the bottom of the file). No Mathlib: the trusted base is Lean core plus this file.

## What is proven

Kerf's meaning is `denote`: a program → the set of deposited cells on the integer lattice (a
characteristic function `Cell → Bool`), unioned over a layer's segments — exactly what `kerf-core`
computes. Over that model, all four soundness properties from `docs/08-semantics.md` are proved for
**arbitrary programs** (unbounded, not sampled):

| Theorem | Property |
|---|---|
| `reversal_invariant`   | **P1** — reversing a path deposits the same material. |
| `translation_invariant`| **P2** — translating a program shifts its material by the same vector. |
| `pass_sound`           | **P4** — a pass that reorders toolpaths and reverses any subset of them (what `TravelOrder` does) preserves the material. |
| `lowering_sound`       | **P3** — the `hi → lo` lowering (copy fills to extrudes, insert travels) preserves the material; travels denote nothing. |

## How it connects to the Rust

The proofs reduce everything to two abstract properties of coverage, bundled in `structure Coverage`:

- `symm` — coverage depends on a segment only through its **unordered** endpoints.
- `trans` — coverage is equivariant under a whole-cell translation.

The Rust `kerf-core` **discharges `symm`** via `canon_seg`, which **Kani proves order-independent for
all `i64` endpoints** (`canon_seg_is_order_independent`). So the argument is complete end to end:

- **Lean (unbounded):** `symm ∧ trans ⟹ P1–P4`.
- **Kani (all `i64`):** the concrete rasterizer satisfies `symm`.
- **Rust tests:** the rasterizer's exact marked set matches a brute-force reference
  (`optimized_raster_marks_exactly_the_bruteforce_set`) and P1 holds by exhaustive enumeration.

## Honest boundary

This proves soundness over Kerf's **discrete (rasterized) denotation** — the semantics the tool
actually implements and compares. The one layer still stated as a caveat rather than a theorem is the
relationship between that rasterization and an *exact* real-geometry denotation within `O(r)`
(the "up to resolution" bound); see `docs/08-semantics.md` §6. Everything Kerf *claims to check* is now
proved; closing that last analytic gap is the remaining research lift.

## Check it yourself

```console
$ cd proofs && lake build      # Lean fetches its toolchain on first run; success == proof checked
```
