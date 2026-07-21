"""Delta-debugging shrinker: turn a failing Instance into a MINIMAL reproducer for a human to inspect.

Decoupled from the oracle by design — `shrink` takes a predicate `still_fails(instance) -> bool`, so it
imports no oracle. It greedily proposes simpler candidates and accepts one iff it still reproduces. For a
prism that means dropping holes, deleting outer vertices (keeping a valid CCW ring of >=3), halving the
scale, collapsing layers toward 1, and snapping coordinates to a coarser grid; any other Instance falls
back to `.scale(0.5)` if it has one. We iterate to a fixed point (a full round with no accepted change)
or `max_rounds`, and never return a candidate that fails to reproduce — the input is the safe fallback.
"""

from __future__ import annotations

import numpy as np

try:  # normal package use: `import kerffuzz.shrink` / `python -m kerffuzz.shrink`
    from . import shapes  # noqa: F401  (kept for the self-check's shape generators)
    from .instance import Instance
    from .shapes import Prism
except ImportError:  # run directly as a script: `python kerffuzz/shrink.py`
    import pathlib
    import sys

    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
    from kerffuzz import shapes  # noqa: F401
    from kerffuzz.instance import Instance
    from kerffuzz.shapes import Prism


# ---- ring helpers ---------------------------------------------------------------------------------

def _ring_area2(ring: np.ndarray) -> int:
    """Twice the signed area (shoelace). >0 is CCW; ==0 is degenerate (collinear / zero area)."""
    x, y = ring[:, 0].astype(np.int64), ring[:, 1].astype(np.int64)
    return int(np.sum(x * np.roll(y, -1) - np.roll(x, -1) * y))


def _valid_ring(ring: np.ndarray) -> bool:
    """A usable outer ring: >=3 vertices, no consecutive duplicates, strictly positive (CCW) area."""
    if len(ring) < 3:
        return False
    if np.any(np.all(ring == np.roll(ring, -1, axis=0), axis=1)):
        return False
    return _ring_area2(ring) > 0


def _with(prism: Prism, **kw) -> Prism:
    base = dict(
        outer=prism.outer,
        holes=prism.holes,
        n_layers=prism.n_layers,
        layer_h_um=prism.layer_h_um,
        width_um=prism.width_um,
        name=prism.name,
    )
    base.update(kw)
    return Prism(**base)


# ---- candidate generators (each yields simpler Prisms) --------------------------------------------

def _drop_holes(prism: Prism):
    """Remove one hole at a time (bigger reductions first: try dropping all, then each individually)."""
    if not prism.holes:
        return
    if len(prism.holes) > 1:
        yield _with(prism, holes=[])
    for i in range(len(prism.holes)):
        yield _with(prism, holes=[h for j, h in enumerate(prism.holes) if j != i])


def _drop_vertices(prism: Prism):
    """Delete one outer vertex at a time, keeping a valid CCW ring of >=3 vertices."""
    if len(prism.outer) <= 3:
        return
    for i in range(len(prism.outer)):
        cand = np.delete(prism.outer, i, axis=0)
        if _valid_ring(cand):
            yield _with(prism, outer=cand)


def _halve_scale(prism: Prism):
    """Halve the geometry — Prism.scale keeps outer/holes in lock-step; skip if it collapses the ring."""
    cand = prism.scale(0.5)
    if _valid_ring(cand.outer):
        yield cand


def _reduce_layers(prism: Prism):
    """Collapse layer count toward 1 (halving, then a single layer)."""
    if prism.n_layers <= 1:
        return
    half = max(1, prism.n_layers // 2)
    if half != prism.n_layers:
        yield _with(prism, n_layers=half)
    if prism.n_layers != 1:
        yield _with(prism, n_layers=1)


def _snap_grid(prism: Prism):
    """Snap every coordinate to a coarser grid, shrinking the numeric spread of the coordinates."""
    span = int(np.ptp(prism.outer)) if prism.outer.size else 0
    for grid in (10000, 1000, 100, 10):
        if grid >= span:
            continue

        def snap(ring):
            return (np.round(ring / grid) * grid).astype(np.int64)

        outer = snap(prism.outer)
        if not _valid_ring(outer):
            continue
        holes = [snap(h) for h in prism.holes]
        cand = _with(prism, outer=outer, holes=holes)
        if not np.array_equal(cand.outer, prism.outer):
            yield cand


_PRISM_MOVES = (_drop_holes, _drop_vertices, _halve_scale, _reduce_layers, _snap_grid)


def _candidates(instance):
    """Simpler candidates for `instance`, cheapest-first. Prism-specific; else fall back to .scale(0.5)."""
    if isinstance(instance, Prism):
        for move in _PRISM_MOVES:
            yield from move(instance)
        return
    try:
        yield instance.scale(0.5)
    except (NotImplementedError, AttributeError):
        return


# ---- driver ---------------------------------------------------------------------------------------

def shrink(instance, still_fails, max_rounds: int = 200):
    """Return a smaller Instance that still satisfies `still_fails(instance) -> bool` (the reproducing
    predicate). Greedy delta-debugging: repeatedly try a simpler candidate; accept it iff it still fails."""
    if not still_fails(instance):
        return instance  # not a reproducer to begin with — nothing to shrink, don't invent one

    current = instance
    for _ in range(max_rounds):
        changed = False
        for cand in _candidates(current):
            if still_fails(cand):
                current = cand
                changed = True
                break  # restart the round from the freshly reduced instance
        if not changed:
            break  # fixed point: a full pass produced no accepted reduction

    return current if still_fails(current) else instance  # never return a non-reproducer


# ---- reporting ------------------------------------------------------------------------------------

def describe(instance) -> dict:
    """A small size summary so a report can show 'shrank from X to Y'."""
    if isinstance(instance, Prism):
        pts = np.vstack([instance.outer, *instance.holes])
        lo, hi = pts.min(axis=0), pts.max(axis=0)
        return {
            "label": instance.label,
            "n_vertices": int(len(instance.outer)),
            "n_holes": int(len(instance.holes)),
            "n_layers": int(instance.n_layers),
            "bbox": (int(lo[0]), int(lo[1]), int(hi[0]), int(hi[1])),
        }
    return {"label": getattr(instance, "label", type(instance).__name__)}


# ---- self-check (no oracle) -----------------------------------------------------------------------

if __name__ == "__main__":
    start = Prism(shapes.star(8, 20000, 8000))
    n0 = len(start.outer)  # star(8) -> 16 vertices

    # Predicate with a boundary: keep >4 outer vertices. The shrinker should drive vertex count down
    # toward that boundary while every accepted candidate still satisfies the predicate.
    boundary = shrink(start, lambda inst: len(inst.outer) > 4)
    assert len(boundary.outer) > 4, "must still satisfy the predicate"
    assert len(boundary.outer) < n0, f"should have reduced vertices ({n0} -> {len(boundary.outer)})"
    assert len(boundary.outer) == 5, f"should sit at the boundary, got {len(boundary.outer)}"
    print(f"boundary predicate: {describe(start)} -> {describe(boundary)}")

    # Always-True: everything reproduces, so shrink to the minimal shape it can reach.
    minimal = shrink(start, lambda inst: True)
    assert isinstance(minimal, Prism)
    assert len(minimal.outer) == 3, f"minimal ring is a triangle, got {len(minimal.outer)}"
    assert not minimal.holes and minimal.n_layers == 1
    print(f"always-True: {describe(start)} -> {describe(minimal)}")

    # Always-False: the input never reproduces, so it is returned unchanged.
    unchanged = shrink(start, lambda inst: False)
    assert unchanged is start, "always-False must return the original untouched"
    print(f"always-False: returned original unchanged ({describe(unchanged)})")

    # Non-prism fallback: an Instance whose only simplification is .scale(0.5).
    class _Blob(Instance):
        def __init__(self, r: float):
            self.r = r

        def to_stl_bytes(self):
            return b""

        def rotate_z(self, radians):
            return self

        def translate(self, dx_um, dy_um):
            return self

        def mirror_x(self):
            return self

        def scale(self, factor):
            return _Blob(self.r * factor)

    blob = shrink(_Blob(1000.0), lambda inst: inst.r > 100.0)
    assert isinstance(blob, _Blob) and 100.0 < blob.r <= 125.0, f"blob should shrink to boundary, got {blob.r}"
    print(f"non-prism fallback: _Blob scaled down to r={blob.r}")

    print("shrink OK")
