"""Real-slicer bug hunt: an adversarial corpus x the full oracle battery, parallelized across
processes (slicing is I/O/CPU-bound, not model-bound), with dedup so a whole shape family collapses
to one representative lead.

Emits findings.json (every raw violation) + a grouped summary. Adjudication (real defect vs. harness
artifact) and shrinking happen downstream on the deduped representatives.

    python hunt.py --exe /Applications/PrusaSlicer.app/Contents/MacOS/PrusaSlicer \
                   --profile /tmp/ps_bed300.ini --random 40 --out runs/hunt
"""
from __future__ import annotations

import argparse
import json
import os
from concurrent.futures import ProcessPoolExecutor, as_completed

import numpy as np

from kerffuzz import corpus, oracle, shapes
from kerffuzz.adapters import prusaslicer


def adversarial_corpus(rng: np.random.Generator, n_random: int):
    """Shape families each aimed at a slicer weak spot, swept across the parameter that matters."""
    items: list[tuple[str, object]] = []

    def wedge(h_um):  # long thin triangle; h_um controls tip sharpness
        return shapes.Prism(
            np.array([[-20_000, 0], [20_000, 0], [-20_000, int(h_um)]], dtype=np.int64),
            name=f"wedge_h{h_um}")

    # ACUTE TIPS: sweep the sliver height (the lead class from the first real run)
    for h in (400, 700, 1000, 1500, 2200, 3200, 5000):
        items.append((f"wedge_h{h}", wedge(h)))
    # THIN WALLS: straddle the 0.4 mm nozzle so wall-count/gap-fill logic flips
    for w in (250, 300, 350, 400, 450, 550, 700, 900):
        items.append((f"thinwall_{w}", shapes.Prism(shapes.rect(20_000, w), name=f"thinwall_{w}")))
    # DEEP CONCAVITY: 5-point stars, inner radius shrinking toward a sliver notch
    for rin in (1500, 2500, 4000, 6000, 9000):
        items.append((f"star5_rin{rin}", shapes.Prism(shapes.star(5, 20_000, rin), name=f"star5_rin{rin}")))
    for pts in (6, 7, 8, 10):
        items.append((f"star{pts}", shapes.Prism(shapes.star(pts, 18_000, 5000), name=f"star{pts}")))
    # NEAR-CIRCLES: vertex count x radius (path-simplification / seam divergence)
    for k in (24, 48, 96, 200):
        for r in (4000, 12_000, 28_000):
            items.append((f"circ{k}_r{r}", shapes.Prism(shapes.regular_polygon(k, r), name=f"circ{k}_r{r}")))
    # TINY ISLANDS: near the minimum-feature threshold
    for s in (200, 300, 400, 600, 800):
        items.append((f"tiny_{s}", shapes.Prism(shapes.rect(s, s), name=f"tiny_{s}")))
    # HOLES NEAR WALLS: thin ligament between a hole and the outer wall
    for gap in (300, 500, 800, 1500):
        r_hole = 6000
        items.append((f"hole_gap{gap}", shapes.Prism(
            shapes.rect(2 * r_hole + 2 * gap + 2000, 2 * r_hole + 2 * gap + 2000),
            holes=[shapes.regular_polygon(24, r_hole)], name=f"hole_gap{gap}")))

    items += corpus.random_corpus(rng, n_random)
    return items


def _run_one(payload):
    name, inst, profile, exe = payload
    ad = prusaslicer(profile, exe=exe)
    out = []
    try:
        for r in oracle.run_all(ad, inst, 200):
            if r.violation:
                out.append(dict(name=name, instance=inst.label, invariant=r.kind,
                                cls=r.soundness_class, mean_um=r.mean_um, max_um=r.max_um, detail=r.detail))
    except Exception as e:  # a crash/no-gcode is itself a finding class
        out.append(dict(name=name, instance=getattr(inst, "label", name), invariant="crash",
                        cls="GATE", mean_um=0.0, max_um=0.0, detail=str(e)[:200]))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", required=True)
    ap.add_argument("--profile", required=True)
    ap.add_argument("--random", type=int, default=40)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--out", default="runs/hunt")
    args = ap.parse_args()

    rng = np.random.default_rng(args.seed)
    items = adversarial_corpus(rng, args.random)
    # kerf-ref containment needs a footprint; PrusaSlicer slices any STL, but keep prisms for the
    # footprint-based gate. (meshes still get isometry checks.)
    payloads = [(n, i, args.profile, args.exe) for n, i in items]
    print(f"hunting {len(payloads)} instances on {os.path.basename(args.exe)} with {args.workers} workers")

    viols = []
    done = 0
    with ProcessPoolExecutor(max_workers=args.workers) as ex:
        futs = [ex.submit(_run_one, p) for p in payloads]
        for f in as_completed(futs):
            viols.extend(f.result())
            done += 1
            if done % 10 == 0:
                print(f"  {done}/{len(payloads)} done, {len(viols)} raw violations so far")

    os.makedirs(args.out, exist_ok=True)
    json.dump(viols, open(os.path.join(args.out, "findings.json"), "w"), indent=2)

    # dedup: collapse a family to one representative per (invariant, class, magnitude bucket)
    def bucket(v):
        return (v["invariant"].split("_")[0], v["cls"], int(v["max_um"] // 200))
    groups = {}
    for v in viols:
        groups.setdefault(bucket(v), []).append(v)

    print(f"\n{len(viols)} raw violations in {len(groups)} distinct lead group(s):")
    for key, vs in sorted(groups.items(), key=lambda kv: -max(v["max_um"] for v in kv[1])):
        worst = max(vs, key=lambda v: v["max_um"])
        print(f"  [{key[1]}] {key[0]:12} max~{worst['max_um']:.0f}um  x{len(vs):3}  "
              f"e.g. {worst['name']} / {worst['invariant']} — {worst['detail']}")
    print(f"\nfindings.json -> {os.path.join(args.out, 'findings.json')}")


if __name__ == "__main__":
    main()
