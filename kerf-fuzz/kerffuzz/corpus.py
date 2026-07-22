"""The test corpus — a curated set of instances that target known slicer failure modes, plus a random
generator. Every corpus item is an `Instance` (so the mesh generator, adapters, and oracle all compose),
paired with a name that says which bug class it exercises.

Prisms are the 2D-exact backbone of the corpus (built straight from `shapes.Prism` + the polygon
generators). 3D meshes are optional: `meshgen` may not exist yet, so it is imported lazily and its
entries are simply skipped when it is absent.
"""

from __future__ import annotations

import numpy as np

try:  # normal package use: `import kerffuzz.corpus` / `python -m kerffuzz.corpus`
    from . import shapes
    from .instance import Instance
except ImportError:  # run directly as a script: `python kerffuzz/corpus.py`
    import pathlib
    import sys

    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
    from kerffuzz import shapes
    from kerffuzz.instance import Instance


def _prism(name: str, outer: np.ndarray, holes: list | None = None, **kw) -> shapes.Prism:
    return shapes.Prism(outer=outer, holes=holes or [], name=name, **kw)


# ---- curated boundary corpus --------------------------------------------------------------------

def boundary_corpus() -> list[tuple[str, Instance]]:
    """Named instances that each stress a known slicer failure mode. A typical FDM nozzle is 0.4 mm
    (400 um) with 0.2 mm layers, so "thin" / "tiny" are measured against those."""
    items: list[tuple[str, Instance]] = []

    def add(name: str, inst: Instance) -> None:
        items.append((name, inst))

    # THIN WALL: a 6 mm x 0.3 mm slab, thinner than a 0.4 mm nozzle — slicers drop/merge the wall or
    # emit a gap-fill hack; extrusion-width vs. wall-count logic diverges here.
    add("thin_wall_0p3mm", _prism("thin_wall_0p3mm", shapes.rect(6000, 300)))

    # TINY FEATURE: a sub-millimetre 0.4 mm square — near the minimum-feature / vertex-snap threshold,
    # where a slicer may discard the whole island as noise.
    add("tiny_feature_0p4mm", _prism("tiny_feature_0p4mm", shapes.rect(400, 400)))

    # LARGE / NEAR-BED-LIMIT: a 210 mm x 210 mm plate at the edge of a common 220x220 bed — exercises
    # bed-bounds clamping, arrangement, and float precision over a large coordinate span.
    add("large_near_bed_210mm", _prism("large_near_bed_210mm", shapes.rect(210_000, 210_000)))

    # HUGE-COORDINATE PLACEMENT: a small part translated a metre off origin — stresses integer/float
    # coordinate handling and any fixed-point rounding in the slicer's geometry pipeline.
    add("huge_coord_offset_1m", _prism("huge_coord_offset_1m", shapes.rect(5000, 5000, cx=1_000_000, cy=1_000_000)))

    # HIGHLY CONCAVE: a 5-point star with a deep inner radius (r_in << r_out) — sharp reflex corners
    # stress inside-corner offsetting, self-intersection avoidance, and seam placement.
    add("deep_star_concave", _prism("deep_star_concave", shapes.star(5, 20_000, 4000)))

    # POLYGON WITH HOLES: a 30 mm square with two circular holes — exercises hole detection, inner
    # perimeter winding, and bridging over the voids.
    add("square_two_holes", _prism(
        "square_two_holes",
        shapes.rect(30_000, 30_000),
        holes=[shapes.regular_polygon(24, 4000, cx=-6000), shapes.regular_polygon(24, 4000, cx=6000)],
    ))

    # MANY-VERTEX NEAR-CIRCLE: a 256-gon approximating a 15 mm circle — path-simplification /
    # arc-fitting and resolution settings decide how many segments survive; a classic divergence point.
    add("near_circle_256gon", _prism("near_circle_256gon", shapes.regular_polygon(256, 15_000)))

    # SHARP ACUTE WEDGE: a long thin triangle with a very acute tip — an in-plane sliver whose apex
    # stresses seam placement and thin-tip extrusion (the 2D analogue of an overhang/tip artifact).
    add("acute_wedge", _prism("acute_wedge", np.array(
        [[-20_000, 0], [20_000, 0], [-20_000, 1500]], dtype=np.int64)))

    # 3D meshes, only if the mesh generator is available (it may not exist yet).
    try:
        from kerffuzz import meshgen
    except Exception:
        return items

    # CSG DIFFERENCE: a box with a bored-out hole — a boolean solid that exercises watertight-mesh
    # handling and internal-perimeter detection on a true 3D input (not a prism's fake inner walls).
    # NOTE: meshgen primitives take sizes in MILLIMETRES (print scale ~5-40 mm); prisms use microns.
    if hasattr(meshgen, "box_with_hole"):
        add("csg_box_minus_cylinder", meshgen.box_with_hole())
    elif hasattr(meshgen, "box") and hasattr(meshgen, "cylinder"):
        box = meshgen.box(20, 20, 20)
        add("csg_box_minus_cylinder", box.difference(meshgen.cylinder(5, 30)))

    # CURVED OVERHANG: a sphere — every layer's growing/shrinking radius is a continuous overhang, the
    # worst case for support generation and staircase artifacts.
    if hasattr(meshgen, "sphere"):
        add("sphere_curved_overhang", meshgen.sphere(15))

    return items


# ---- bug-rich family sweeps (where slicer bugs historically live) -------------------------------

def thin_wall_sweep() -> list[tuple[str, Instance]]:
    """Rect walls swept finely across the 0.4 mm nozzle: 1-/2-/3-bead transitions and gap-fill flip
    exactly here. The wall-count / variable-width-beading logic is a classic divergence point."""
    out = []
    for w in (200, 300, 350, 390, 410, 450, 550, 650, 790, 810, 900):
        out.append((f"thinwall_{w}um", _prism(f"thinwall_{w}um", shapes.rect(24_000, w))))
    return out


def hole_tolerance_sweep() -> list[tuple[str, Instance]]:
    """A 30 mm plate with one central hole whose diameter shrinks toward the nozzle (slicers drop or
    merge sub-nozzle holes), plus the thin ligament between a hole and the outer wall."""
    out = []
    for d_um in (400, 600, 900, 1400, 2200, 4000):
        out.append((f"hole_d{d_um}um", _prism(f"hole_d{d_um}um", shapes.rect(30_000, 30_000),
                    holes=[shapes.regular_polygon(32, d_um // 2)])))
    for gap in (250, 400, 700, 1200):  # ligament between hole edge and outer wall
        r_hole = 8000
        side = 2 * (r_hole + gap) + 1600
        out.append((f"ligament_{gap}um", _prism(f"ligament_{gap}um", shapes.rect(side, side),
                    holes=[shapes.regular_polygon(32, r_hole)])))
    return out


def tall_seam_sweep() -> list[tuple[str, Instance]]:
    """Tall prisms (many layers) — seam placement must stay consistent up the Z stack; a shape whose
    seam wanders layer-to-layer is a real defect this exercises (determinism + isometry catch it)."""
    out = []
    for n in (20, 50, 90):
        out.append((f"tall_hex_{n}layer", shapes.Prism(shapes.regular_polygon(6, 10_000),
                    n_layers=n, name=f"tall_hex_{n}layer")))
        out.append((f"tall_square_{n}layer", shapes.Prism(shapes.rect(16_000, 16_000),
                    n_layers=n, name=f"tall_square_{n}layer")))
    return out


def _mesh_families() -> list[tuple[str, Instance]]:
    """3D families that need a real solid: multi-island layers, bridging, near-degenerate facets."""
    out: list[tuple[str, Instance]] = []
    try:
        from kerffuzz import meshgen
    except Exception:
        return out
    # (meshgen primitive sizes are in MILLIMETRES; translate is in microns.)
    # MULTI-ISLAND: two disjoint 12 mm blocks 22 mm apart — per-layer island handling + travel between.
    if hasattr(meshgen, "box"):
        a = meshgen.box(12, 12, 6).translate(-11_000, 0)
        b = meshgen.box(12, 12, 6).translate(11_000, 0)
        out.append(("multi_island_2box", meshgen.union(a, b)))
    # BRIDGING: a 30x16x8 mm slab with a 10 mm-wide tunnel through Y — the top layers bridge the gap.
    if hasattr(meshgen, "box"):
        try:
            out.append(("bridge_tunnel", meshgen.box(30, 16, 8).difference(meshgen.box(10, 20, 4))))
        except Exception:
            pass
    # NEAR-DEGENERATE FACETS: subdivide then jitter sub-micron -> sliver triangles (robustness/crash).
    if hasattr(meshgen, "box") and hasattr(meshgen, "subdivide_edges") and hasattr(meshgen, "jitter_vertices"):
        import numpy as _np
        base = meshgen.subdivide_edges(meshgen.box(18, 18, 8))
        out.append(("near_degenerate_facets", meshgen.jitter_vertices(base, _np.random.default_rng(0), 0.5)))
    return out


def campaign_corpus(rng: np.random.Generator, n_random: int) -> list[tuple[str, Instance]]:
    """The scaled corpus: curated boundary + the bug-rich family sweeps + 3D families + random."""
    items = list(boundary_corpus())
    items += thin_wall_sweep()
    items += hole_tolerance_sweep()
    items += tall_seam_sweep()
    items += _mesh_families()
    items += random_corpus(rng, n_random)
    return items


# ---- random corpus --------------------------------------------------------------------------------

def random_corpus(rng: np.random.Generator, n: int) -> list[tuple[str, Instance]]:
    """`n` random instances: random-convex prisms with varied vertex counts, sizes and (sometimes)
    holes, plus `meshgen.random_mesh` entries when the mesh generator is available."""
    try:
        from kerffuzz import meshgen
        have_mesh = hasattr(meshgen, "random_mesh")
    except Exception:
        meshgen = None
        have_mesh = False

    items: list[tuple[str, Instance]] = []
    for i in range(n):
        # Mix in a random mesh roughly a third of the time when meshgen is present.
        if have_mesh and rng.random() < 1 / 3:
            items.append((f"rand_mesh_{i:03d}", meshgen.random_mesh(rng)))
            continue

        sides = int(rng.integers(3, 24))
        r_um = int(rng.integers(2000, 40_000))
        outer = shapes.random_convex(rng, sides, r_um)
        holes = []
        if rng.random() < 0.4:
            for _ in range(int(rng.integers(1, 3))):
                cx = int(rng.integers(-r_um // 3, r_um // 3))
                cy = int(rng.integers(-r_um // 3, r_um // 3))
                holes.append(shapes.regular_polygon(int(rng.integers(3, 12)), r_um // 6, cx=cx, cy=cy))
        name = f"rand_prism_{i:03d}_s{sides}_r{r_um}"
        items.append((name, _prism(name, outer, holes=holes)))

    return items


# ---- self-check ------------------------------------------------------------------------------------

if __name__ == "__main__":
    corpus = boundary_corpus()
    for name, inst in corpus:
        assert isinstance(inst, Instance), f"{name} is not an Instance: {type(inst)}"
        stl = inst.to_stl_bytes()
        assert isinstance(stl, (bytes, bytearray)) and len(stl) > 0, f"{name} produced empty STL"

    print(f"{len(corpus)} boundary instances:")
    for name, _ in corpus:
        print(f"  - {name}")
    print("corpus OK")
