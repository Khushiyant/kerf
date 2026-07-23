"""Targeted bug hunt using the self-adjudicating probe layer.

Sweeps a shape FAMILY through a relation (determinism / emi / differential) and prints only
PRE-ADJUDICATED candidates — determinism hits are already classified NOT-ASLR (the real-defect class)
vs address-layout; meshes are validity-gated; the differential is per-connected-component. Families
concentrate on the curved / varying-radius / CSG regions where the confirmed CuraEngine sphere
nondeterminism lives, because that is the highest-yield direction for more of the same class.

    python hunt_probe.py --family cones --relation determinism --slicer cura --n 6
"""
from __future__ import annotations

import argparse
import json

import numpy as np

from kerffuzz import oracle, probe, shapes as S
from kerffuzz.adapters import curaengine, prusaslicer
from kerffuzz.meshgen import (Mesh3D, add_coplanar_vertex, box, cylinder, difference, intersection,
                              random_mesh, sphere, subdivide_edges, union, weld_duplicate_vertices)

CE = "/Applications/UltiMaker Cura.app/Contents/Frameworks/CuraEngine"
DEF = "/Applications/UltiMaker Cura.app/Contents/Resources/share/cura/resources/definitions/fdmprinter.def.json"
BIN = "/Applications/PrusaSlicer.app/Contents/MacOS/PrusaSlicer"
PROF = "/tmp/ps_bed300.ini"

try:
    import manifold3d as _m3
except Exception:
    _m3 = None


def _cone(r0, r1, h, segs):
    return Mesh3D.from_manifold(_m3.Manifold.cylinder(h, r0, r1, int(segs), True), f"cone_{r0}_{r1}_s{segs}")


def family(name, rng):
    """Yield (label, instance) for a named family. Sizes in mm for meshes, microns for prisms."""
    if name == "sphere_sizes":
        for r in (4, 6, 9, 12, 16, 22, 30):
            yield f"sphere_r{r}", sphere(r)
    elif name == "sphere_facets":
        for seg in (12, 16, 24, 32, 48, 64, 96):
            yield f"sphere_s{seg}", sphere(12, segments=seg)
    elif name == "cones":                                   # varying radius per layer — the sphere path's cousin
        if _m3 is None:
            return
        for (r0, r1) in ((8, 0.5), (10, 3), (2, 12), (6, 6.2), (12, 1)):
            yield f"cone_{r0}_{r1}", _cone(r0, r1, 16, 48)
    elif name == "csg_sphere":                              # structured CSG involving spheres
        yield "box_minus_sphere", difference(box(20, 20, 16), sphere(9))
        yield "sphere_minus_box", difference(sphere(12), box(10, 10, 30))
        yield "sphere_union_box", union(sphere(10), box(14, 14, 6))
        yield "sphere_int_box", intersection(sphere(12), box(16, 16, 16))
        yield "two_spheres", union(sphere(9).translate(-6000, 0), sphere(9).translate(6000, 0))
    elif name == "csg_random":
        for i in range(14):
            yield f"rand_csg_{i:02d}", random_mesh(rng)
    elif name == "highfacet_prism":                         # many-vertex near-circles (2D analogue)
        for k in (64, 128, 256, 512):
            for r in (6000, 18000):
                yield f"gon{k}_r{r}", S.Prism(S.regular_polygon(k, r), name=f"gon{k}_r{r}")
    elif name == "thin_curved":                             # thin-walled curved shells
        for r in (5, 8, 12):
            yield f"shell_r{r}", difference(sphere(r), sphere(r - 0.5))
    else:
        raise SystemExit(f"unknown family {name}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--family", required=True)
    ap.add_argument("--relation", default="determinism", choices=["determinism", "emi", "differential"])
    ap.add_argument("--slicer", default="cura", choices=["cura", "prusa"])
    ap.add_argument("--n", type=int, default=6)
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()

    cura = curaengine(DEF, exe=CE)
    prusa = prusaslicer(PROF, exe=BIN)
    ad = cura if args.slicer == "cura" else prusa
    rng = np.random.default_rng(args.seed)

    candidates, checked = [], 0
    for label, inst in family(args.family, rng):
        ok, why = probe.mesh_valid(inst)
        if not ok:
            continue  # invalid STL can't be a slicer bug — gate it out
        checked += 1
        try:
            if args.relation == "determinism":
                r = probe.determinism(ad, inst, n=args.n)
                if r["verdict"] == "NONDETERMINISTIC" and "NOT ASLR" in r.get("mechanism", ""):
                    candidates.append({"shape": label, "family": args.family, "relation": "determinism",
                                       "slicer": ad.name, "detail": r["detail"], "mechanism": r["mechanism"]})
            elif args.relation == "emi":
                if isinstance(inst, Mesh3D):
                    for mut_name, mut in (("subdivide", subdivide_edges(inst)),
                                          ("weld", weld_duplicate_vertices(subdivide_edges(inst))),
                                          ("coplanar", add_coplanar_vertex(inst, rng))):
                        e = oracle.emi(ad, inst, mut, 200)
                        if e.violation:
                            candidates.append({"shape": label, "family": args.family, "relation": f"emi:{mut_name}",
                                               "slicer": ad.name, "detail": e.detail or f"mean {e.mean_um:.0f}um"})
            elif args.relation == "differential":
                r = probe.differential_by_component(prusa, cura, inst, 200)
                if r["verdict"] == "DISAGREE":
                    candidates.append({"shape": label, "family": args.family, "relation": "differential",
                                       "slicer": "prusa~cura", "detail": r["detail"]})
        except Exception as e:
            candidates.append({"shape": label, "family": args.family, "relation": f"{args.relation}:crash",
                               "slicer": ad.name, "detail": str(e)[:160]})

    print(json.dumps({"family": args.family, "relation": args.relation, "slicer": args.slicer,
                      "checked": checked, "candidates": candidates}, indent=2))


if __name__ == "__main__":
    main()
