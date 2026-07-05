# 09 — Volume-weighted denotation

`denote` measures deposited material three ways, from coarsest to finest:

1. **`Occupancy`** — the *set* of covered cells. Blind to how many times, or how much, material lands.
2. **`Deposit`** (denote⁺) — the *count of distinct paths* over each cell. Catches whole-path
   duplication, but still counts paths, not material.
3. **`Volume`** — deposited *melt volume* (mm³) per cell. Moves when a bead gets wider, so it surfaces
   over-/under-extrusion expressed as geometry.

This doc records the design of layer 3 and what remains deferred.

## What shipped (Approach B — derive volume from geometry)

`denote_lo_volume` / `denote_hi_volume` reuse the existing raster kernel (`mark_capsule`), but instead
of a set or a count they accumulate an f64 weight per cell: each segment's volume
`width_mm × layer_height_mm × length_mm` is spread uniformly over the cells that segment covers. Total
volume per path is conserved regardless of resolution.

- **Layer height** is derived from consecutive `z_um` (`h = z − prev_z`, layer 0 uses its own z),
  matching the backend's flow model so the oracle and the emitted G-code agree by construction. An
  optional `layer_height_um` override handles single-layer / non-uniform first layers.
- **Comparison** uses `Volume::approx_eq(eps)` — f64 lives inside the checker, never in the IR.
- **Exposed** in Rust (`denote_lo_volume`, `denote_hi_volume`, `Volume`) and, as a serializable
  aggregate, via `analyze::volume_stats` (total + per-layer mm³) and `pykerf.volume_stats`.

Pinned by tests: volume moves with bead width (invisible to `Occupancy`/`Deposit`), is conserved
across resolution, is reversal-invariant, and is preserved by lowering.

### Honest limitations (all documented in code)

- **No commanded-flow axis.** Two toolpaths with identical geometry but different commanded flow
  (`M221`, a per-move E scale) derive the *same* volume — the IR carries no extrusion-amount axis, so
  this is structurally invisible. Volume reflects *geometry-implied* material only.
- **Uniform per-cell split** smears end-caps, corners, and overlaps; it is a comparative oracle, not a
  metrology tool. Absolute per-cell volume in overlapped regions is non-physical (overlaps sum).
- **Resolution-bounded**, like the other denotations.
- **Layer-height heuristic** assumes monotonic Z / one height per layer.

## Deferred (Approach A — a true extrusion axis)

To ever see commanded-flow over-extrusion at fixed geometry, the IR needs an extrusion-amount field:
an additive `deposit_fl: Option<i64>` (femtolitres — integer, preserves `Eq`/`Hash` and the
no-f64-in-IR invariant) on `ExtrudePath` and `lo::Toolpath`, `#[serde(default)]` so legacy JSON still
loads. `denote_volume` would then prefer commanded volume when present and fall back to Approach B's
geometry estimate when `None`, with a diagnostic distinguishing measured vs estimated toolpaths.

This is deferred deliberately: it touches ~36 struct-literal sites across ~12 files and couples the
parser and backend flow models (the round-trip becomes the highest-risk test). It is not worth that
blast radius until a consumer actually needs to distinguish commanded flow — Approach B delivers the
primary win (over-extrusion as a wider bead) with zero IR churn.
