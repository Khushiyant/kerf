"""The semantic oracle — slice an instance (and transformed copies), denote each with kerf, and check
the relations a correct slicer must satisfy. No ground truth needed.

Every relation carries a SOUNDNESS CLASS (design doc §B):
  - GATE      : a violation is unconditionally a defect (nondeterminism; material outside the part).
  - GRADED    : equal up to sub-cell rounding, judged by graded distance under a micron tolerance
                (isometry relations); comparisons are translation-normalized so a re-position is fine.
  - DIRECTIONAL / EXACT are reserved for future relations.
Determinism is checked FIRST and gates the rest: metamorphic verdicts are meaningless on a
nondeterministic slicer. False-positive controls live in the adapters (fixed seed/threads, no
arc-fitting, perimeter-dominant profile so world-anchored infill can't confound isometry checks).
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass

import numpy as np

from .instance import Instance


def parse(gcode: str) -> str:
    import pykerf as k

    return k.parse_gcode(gcode)[0]


@dataclass
class Result:
    kind: str
    soundness_class: str  # GATE | GRADED | DIRECTIONAL | EXACT
    mean_um: float
    max_um: float
    violation: bool
    detail: str = ""


# ---- point-op helpers on lo-program JSON --------------------------------------------------------

def _map_points(lo_json: str, fn) -> str:
    d = json.loads(lo_json)
    for layer in d["layers"]:
        for tp in layer["toolpaths"]:
            for p in tp["path"]["points"]:
                p["x"], p["y"] = fn(p["x"], p["y"])
    return json.dumps(d)


def _to_origin(lo_json: str) -> str:
    d = json.loads(lo_json)
    pts = [(p["x"], p["y"]) for layer in d["layers"] for tp in layer["toolpaths"] for p in tp["path"]["points"]]
    if not pts:
        return lo_json
    mnx, mny = min(x for x, _ in pts), min(y for _, y in pts)
    return _map_points(lo_json, lambda x, y: (x - mnx, y - mny))


def _graded(expected_lo: str, actual_lo: str, res: int, tol_mean_um: float, kind: str, cls: str = "GRADED") -> Result:
    import pykerf as k

    g = json.loads(k.graded_diff(_to_origin(expected_lo), _to_origin(actual_lo), res))
    mean, mx = g.get("mean_um") or 0.0, g.get("max_um") or 0.0
    return Result(kind, cls, mean, mx, mean > tol_mean_um)


def _occupied_cells(lo_json: str, res: int) -> set:
    occ = json.loads(_pk_occupancy(lo_json, res))
    return {(i, j) for layer in occ["layers"] for i, j in layer["cells"]}


def _pk_occupancy(lo_json: str, res: int) -> str:
    import pykerf as k

    return k.occupancy(lo_json, res)


# ---- relations ----------------------------------------------------------------------------------

def determinism(adapter, instance: Instance, res: int = 200) -> Result:
    """GATE: slicing the same input twice must deposit identical material."""
    import pykerf as k

    a = parse(adapter.slice_to_gcode(instance))
    b = parse(adapter.slice_to_gcode(instance))
    same = k.material_fingerprint(a, res) == k.material_fingerprint(b, res)
    return Result("determinism", "GATE", 0.0, 0.0, not same, "identical" if same else "NONDETERMINISTIC output")


def rotation(adapter, instance: Instance, degrees: float, res: int = 200, tol_mean_um: float = 250.0) -> Result:
    import pykerf as k

    rad = math.radians(degrees)
    base = parse(adapter.slice_to_gcode(instance))
    actual = parse(adapter.slice_to_gcode(instance.rotate_z(rad)))
    expected = k.rotate_z(base, rad)  # kerf's proven denotation rotation (about origin)
    return _graded(expected, actual, res, tol_mean_um, f"rotate_{degrees:g}deg")


def mirror(adapter, instance: Instance, res: int = 200, tol_mean_um: float = 250.0) -> Result:
    base = parse(adapter.slice_to_gcode(instance))
    actual = parse(adapter.slice_to_gcode(instance.mirror_x()))
    expected = _map_points(base, lambda x, y: (-x, y))
    return _graded(expected, actual, res, tol_mean_um, "mirror_x")


def translation(adapter, instance: Instance, dx_um: int, dy_um: int, res: int = 200, tol_mean_um: float = 250.0) -> Result:
    # Translation is a GRADED isometry: moving the part on the bed must not change the deposited SHAPE.
    # A real slicer re-rasterizes (and re-centres) each variant, so sub-cell (<1 cell) noise is sound —
    # use the same sub-cell tolerance as rotation/mirror, not the exact-reference 1µm. A genuine
    # translation-variance bug (e.g. world-anchored features shifting) is many cells, well past this.
    base = parse(adapter.slice_to_gcode(instance))
    actual = parse(adapter.slice_to_gcode(instance.translate(dx_um, dy_um)))
    return _graded(base, actual, res, tol_mean_um, f"translate_{dx_um}_{dy_um}", cls="GRADED")


def _inside(px: float, py: float, poly: np.ndarray) -> bool:
    """Ray-cast point-in-polygon (poly = [K,2] open ring)."""
    n = len(poly)
    inside = False
    j = n - 1
    for i in range(n):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if (yi > py) != (yj > py) and px < (xj - xi) * (py - yi) / (yj - yi + 1e-12) + xi:
            inside = not inside
        j = i
    return inside


def _dist_to_boundary(px: float, py: float, poly: np.ndarray) -> float:
    n = len(poly)
    best = math.inf
    for i in range(n):
        ax, ay = poly[i]
        bx, by = poly[(i + 1) % n]
        dx, dy = bx - ax, by - ay
        L2 = dx * dx + dy * dy
        t = 0.0 if L2 == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / L2))
        cx, cy = ax + t * dx, ay + t * dy
        best = min(best, math.hypot(px - cx, py - cy))
    return best


def containment(adapter, instance: Instance, res: int = 200, margin_um: float = 1.5) -> Result | None:
    """GATE: no material outside the part's XY footprint (dilated by the bead half-width + margin).

    Sound only with skirt/brim disabled (the adapters' fuzz profiles do so). Applies to instances with
    a 2D footprint (prisms); returns None otherwise. A cell beyond `width/2 + margin` outside the
    polygon is material the slicer put where the object is not — a hard defect."""
    if not hasattr(instance, "footprint"):
        return None
    poly = np.asarray(instance.footprint(), dtype=np.float64)
    # A deposited cell's centre can sit up to (bead half-width + one cell of conservative-coverage
    # reach) outside the path, which lies on the boundary — so the sound "outside the part" threshold
    # is width/2 + res + a rounding margin. Skirt/brim (mm outside) still trips it; profiles disable them.
    half = getattr(instance, "width_um", 400) / 2.0 + res + margin_um
    lo = parse(adapter.slice_to_gcode(instance))
    cells = _occupied_cells(lo, res)
    if not cells:
        return Result("containment", "GATE", 0.0, 0.0, False, "no material deposited")
    # Translation-normalize: many slicers auto-place the part on the bed, so the deposited material is
    # shifted from the footprint's frame. Align by bbox CENTRE (not min-corner): kerf's occupancy is
    # conservatively dilated ~1 cell on every side, and centre-alignment cancels that symmetric reach
    # while absorbing a rigid re-position. A true leak still pushes material past the far boundary.
    xs = [i for i, _ in cells]
    ys = [j for _, j in cells]
    dep_cx = (min(xs) + max(xs)) / 2.0 * res + res / 2.0
    dep_cy = (min(ys) + max(ys)) / 2.0 * res + res / 2.0
    poly = poly + np.array([dep_cx - (poly[:, 0].min() + poly[:, 0].max()) / 2.0,
                            dep_cy - (poly[:, 1].min() + poly[:, 1].max()) / 2.0])
    worst = 0.0
    n_out = 0
    for i, j in cells:
        px, py = i * res + res / 2.0, j * res + res / 2.0
        if _inside(px, py, poly):
            continue
        d = _dist_to_boundary(px, py, poly)
        if d > half:
            n_out += 1
            worst = max(worst, d - half)
    return Result("containment", "GATE", worst, worst, n_out > 0, f"{n_out} cells outside footprint")


def emi(adapter, original: Instance, mutant: Instance, res: int = 200, tol_mean_um: float = 250.0) -> Result:
    """GRADED (EMI): an Equivalence-Modulo-Inputs mutant is a *different mesh of the same solid*
    (subdivided edges, welded duplicates, an inserted coplanar vertex, an identity boolean). A correct
    slicer MUST deposit the same material. Re-triangulation can shift float coords sub-cell, so we allow
    the isometry tolerance and flag only a real divergence — a slicer whose output depends on how the
    same solid was tessellated. Compared translation-normalized."""
    base = parse(adapter.slice_to_gcode(original))
    mut = parse(adapter.slice_to_gcode(mutant))
    return _graded(base, mut, res, tol_mean_um, "emi", cls="GRADED")


def differential(adapter_a, adapter_b, instance: Instance, res: int = 200, tol_bbox_um: float = 2000.0) -> Result:
    """GATE (cross-slicer): two slicers must at least agree on WHAT solid to fill — non-empty output
    and the same XY footprint extent. They legitimately differ on infill/perimeter counts, so we only
    compare the deposited bounding box (translation-normalized), not the material itself."""
    def bbox(adapter):
        cells = _occupied_cells(parse(adapter.slice_to_gcode(instance)), res)
        if not cells:
            return None
        xs = [i for i, _ in cells]
        ys = [j for _, j in cells]
        return (max(xs) - min(xs)) * res, (max(ys) - min(ys)) * res

    ba, bb = bbox(adapter_a), bbox(adapter_b)
    if ba is None or bb is None:
        who = adapter_a.name if ba is None else adapter_b.name
        return Result(f"differential:{adapter_a.name}~{adapter_b.name}", "GATE", 0.0, 0.0, True, f"{who} produced NO material")
    dw, dh = abs(ba[0] - bb[0]), abs(ba[1] - bb[1])
    worst = float(max(dw, dh))
    return Result(f"differential:{adapter_a.name}~{adapter_b.name}", "GATE", worst, worst, worst > tol_bbox_um,
                  f"footprint bbox delta {dw}x{dh} um")


def run_all(adapter, instance: Instance, res: int = 200) -> list[Result]:
    """The single-slicer battery. Determinism gates the rest (metamorphic verdicts are meaningless on
    a nondeterministic slicer)."""
    det = determinism(adapter, instance, res)
    if det.violation:
        return [det, Result("metamorphic", "GATE", 0.0, 0.0, False, "skipped: slicer is nondeterministic")]
    out = [det]
    for deg in (90, 180, 270, 37.0):
        out.append(rotation(adapter, instance, deg, res))
    out.append(mirror(adapter, instance, res))
    out.append(translation(adapter, instance, 5 * res, 3 * res, res))
    c = containment(adapter, instance, res)
    if c is not None:
        out.append(c)
    return out
