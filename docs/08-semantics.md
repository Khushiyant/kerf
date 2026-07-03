# 08 — Formal semantics, soundness theorems, and the road to a mechanized proof

Kerf's central claim is a *checked* one: lowering and optimization passes preserve what a program
deposits. Today that is established by property-based and metamorphic testing. This document writes the
semantics and the theorems down precisely — the prerequisite for turning "checked" into "proven,"
since you cannot mechanize a statement you have not formalized. It also states the trust model (TCB)
honestly, so the boundary between what is guaranteed and what is assumed is explicit.

## 1. Denotation domain

Fix a raster resolution `r ∈ ℤ⁺` (microns). A **cell** is a lattice square `(i, j) ∈ ℤ²` covering
`[i·r, (i+1)·r) × [j·r, (j+1)·r)`. A layer's meaning is the set of occupied cells; a program's meaning
is one such set per layer, tagged by height:

```
Occupancy_r  =  list of (z_um : ℤ, cells : ℘(ℤ²))
```

## 2. denote

An extruding path is a polyline `π = (p₀, …, pₙ)` of integer-micron points with width `w ∈ ℤ⁺`. Its
**swept region** is the Minkowski sum of the polyline with a disk of radius `w/2`. Kerf's reference
`denote` is a *conservative rasterization* of that region: cell `(i,j)` is occupied by segment `[a,b]`
iff the distance from the cell centre `c(i,j) = (i·r + r/2, j·r + r/2)` to the segment is at most
`w/2 + r/√2` (the cell circumradius). Writing `cover_r(π, w)` for the union of such cells over the
segments of `π`:

```
denote_r(P)  =  for each layer L of P:  (L.z_um,  ⋃ over extruding (π,w) in L  cover_r(π, w))
```

Travel moves contribute nothing. Two programs **denote the same material at resolution r** iff their
`Occupancy_r` are equal. (In code: `denote_hi`, `denote_lo`, `crates/kerf-core/src/denote.rs`.)

The choice of *conservative coverage* (reach `w/2 + r/√2`, not centre-in-`w/2`) guarantees a feature is
never entirely missed when `r ≤ w`: material present in one program and absent in another is detected.

## 3. The soundness properties (theorems — currently property-checked)

Let `T` denote the identity on deposited material. The properties Kerf relies on, each with the
argument a mechanized proof would formalize:

**P1 — Reversal invariance.** `denote_r(P) = denote_r(reverse(P))` for any path reversal.
*Argument:* `cover_r` canonicalizes a segment's endpoints before any computation, so a segment's cover
depends only on its *unordered* endpoint pair and width; a layer's occupancy is a union over segments,
which is order-independent. Pinned by `denote_is_reversal_invariant` (explicit float-boundary cases that
*fail* if canonicalization is removed) + a proptest.

**P2 — Translation invariance (whole-cell).** For `k, m ∈ ℤ`,
`denote_r(translate(P, k·r, m·r)) = shift(denote_r(P), k, m)`.
*Argument:* the coverage predicate depends only on the offset between a cell centre and a segment; a
whole-cell translation shifts both identically, so the predicate is invariant and cell indices shift by
`(k,m)`. Pinned by `any_program_is_translation_invariant` (proptest) — the metamorphic relation.

**P3 — Lowering soundness.** `denote_r(lower(H)) = denote_r(H)` for the `hi → lo` lowering.
*Argument:* `lower` neither adds, drops, nor alters an extruding path — it copies each region's fills
into extruding toolpaths and inserts travel moves between them; travel denotes ∅, so the multiset of
`(π, w)` pairs (hence the union) is preserved. Pinned by `self_lowering_sound` + a proptest.

**P4 — Pass soundness (obligation).** A pass `f : lo → lo` is *sound* iff `∀P. denote_r(f(P)) = denote_r(P)`.
*Argument for `TravelOrder`:* it reorders extruding toolpaths and may reverse a path, then regenerates
travels. Reordering preserves the union; reversal preserves each cover by P1; travels denote ∅. Pinned
by `preserves_denotation(&TravelOrder, …)` + a proptest, and by a *negative* test (`DropFirstExtrude`)
proving the oracle rejects a pass that drops material.

## 4. Trust model (TCB) — what is guaranteed vs. assumed

- **The reference semantics is `denote` itself.** The lossy convenience nature of the earlier backend
  is irrelevant; the *faithful* backend is validated *against* `denote` by round-trip (P-round-trip:
  `denote_r(parse(emit(P))) = denote_r(P)`, pinned by `round_trip_preserves_denotation`), it is not the
  definition.
- **Resolution-bounded.** Equality of `Occupancy_r` means "same deposited material *up to r*." Sub-`r`
  differences are not distinguished — a stated, tested limitation (`oracle_is_blind_below_resolution`).
  This is the single most important caveat: a "SOUND" / "IDENTICAL" verdict is modulo `r`.
- **Float is checker-internal.** Distances use `f64` *inside* `denote` only; the IR is exact integer
  microns. P1's canonicalization removes the one place float asymmetry could leak into a verdict.
- **Inputs assumed well-formed** as machine instructions; the parser never trusts *semantic* comments
  (role/width) — those are re-inferred and flagged (see `frontend/gcode.rs` trust boundary).
- **Not machine-checked.** The arguments above are proof *sketches* discharged by property/metamorphic
  tests over bounded random inputs, not a mechanized proof. This is the honest status.

## 5. What is machine-checked

### 5a. P1–P4, proved unbounded in Lean 4 (`proofs/`)

The soundness properties are no longer sketches or samples — they are **proved for arbitrary programs**
in Lean 4 (`proofs/KerfProofs.lean`). `lake build` runs the kernel; the four theorems
`reversal_invariant` (P1), `translation_invariant` (P2), `lowering_sound` (P3), `pass_sound` (P4) depend
only on `propext` + `Quot.sound` (standard trusted axioms) with **no `sorry`** (audited by `#print
axioms`). The model is Kerf's discrete denotation (occupancy as a characteristic function on the cell
lattice — exactly what the code computes).

The Lean proof reduces everything to two abstract properties of coverage (`Coverage.symm`,
`Coverage.trans`). The concrete Rust rasterizer **discharges `symm`** via `canon_seg`, which Kani proves
order-independent for all `i64` endpoints — so the chain is complete: Lean proves *properties ⟹ P1–P4*
(unbounded), Kani proves *the implementation has the properties*. See `proofs/README.md`.

### 5b. Bounded model-checking of the concrete Rust (Kani)

Running `cargo kani -p kerf-core` model-checks these harnesses (in `denote.rs` / `frontend/gcode.rs`,
`#[cfg(kani)]`); all four verify:

- **`canon_seg_is_order_independent`** — `canon_seg(p, q) == canon_seg(q, p)` for **all** `i64`
  endpoints. This is the *mechanism* of P1: `denote` depends on a segment only through `canon_seg`, so
  order-independence of that kernel establishes reversal invariance at its root (the exact property the
  historical un-canonicalized code violated). Exhaustive over the input domain, not sampled.
- **`dist2_point_seg_is_nonneg_and_finite`** — the checker's squared distance is `>= 0` and finite for
  finite inputs, so a NaN/negative can never leak into the `dist <= reach²` comparison and flip a verdict.
- **`mm_to_um_is_total_and_in_range`** / **`um_round_is_total_and_in_range`** — the parser's
  float→micron conversions never panic and, when they yield a coordinate, it is inside the guarded
  range (the internal `as i64` cast is exact, never a silent saturation) — checked for every `f64`,
  including NaN / ±inf / subnormals.

These are *bounded* model-checking results over pure kernels, complemented by an exhaustive
enumeration test (`reversal_invariant_exhaustively_over_small_programs`) that checks P1 over every small
program in a coordinate grid. What remains below is the full lift to a semantics-level proof.

## 6. The one remaining layer (the research lift)

P1–P4 are now proved over Kerf's discrete denotation (§5a). The single piece still stated as a *caveat*
rather than a *theorem* is the relationship between that rasterization and an exact real-geometry
denotation — i.e. closing the "up to resolution `r`" gap with a proof instead of a documented bound:

1. **An exact geometric denotation** (`denote*`) as the union of true swept capsules over ℚ (or a
   decidable exact predicate), independent of rasterization.
2. **A soundness/completeness bound** relating rasterized `denote_r` to `denote*`: e.g.
   `denote*(P) ⊆ region(denote_r(P))` within `O(r)` Hausdorff distance — so a rasterized verdict implies
   the exact one up to a proven error, closing the "up to r" gap with a *theorem* rather than a caveat.
3. **Mechanized P1–P4 over `denote*`** — reversal/translation invariance and lowering/pass preservation
   as proved lemmas, with `lower`/`TravelOrder` reflected into the proof assistant (or a Kani harness
   bounding path counts/lengths and discharging `preserves_denotation` by symbolic execution).
4. **Exact arithmetic in the checked core** (rationals or interval arithmetic with proven soundness),
   removing the `f64` assumption from the TCB.

This is the multi-quarter research target that would make Kerf a *verified* — not merely *tested* —
lowering. The value of writing it down now: the properties and the TCB are stated precisely, so the
proof effort has a fixed target and the current guarantees are not overstated.
