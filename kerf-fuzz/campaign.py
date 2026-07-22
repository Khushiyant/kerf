"""Scaled fuzzing campaign: geometry x config-space x slicers.

For each (shape, config) unit it runs, per slicer, a lean high-signal battery — determinism (GATE),
containment (GATE), one rotation (metamorphic sanity), and EMI equivalence for meshes (GATE) — plus the
cross-slicer differential (GATE) between the two slicers at that config. Config-space is swept because
compiler-fuzzing history says config bugs outnumber input bugs.

Parallel across units; streams findings to findings.jsonl and the containment over-reach magnitude of
EVERY unit to curve.jsonl (for the trust-boundary artifact), with a checkpoint so a long run resumes.

    python campaign.py --prusa-exe ... --prusa-profile ... \
                       --cura-exe ... --cura-def ... \
                       --configs 10 --random 60 --workers 8 --out runs/campaign
"""
from __future__ import annotations

import argparse
import json
import os
from concurrent.futures import ProcessPoolExecutor, as_completed

import numpy as np

from kerffuzz import configs, corpus, oracle
from kerffuzz.adapters import curaengine, prusaslicer


def _build_adapter(recipe: dict, cfg: dict):
    if recipe["kind"] == "prusa":
        return prusaslicer(recipe["profile"], exe=recipe["exe"], overrides=configs.to_prusa(cfg))
    if recipe["kind"] == "cura":
        return curaengine(recipe["def"], exe=recipe["exe"], overrides=configs.to_cura(cfg))
    raise ValueError(recipe["kind"])


def _emi_mutant(inst):
    """A semantics-preserving mesh mutant (subdivided edges). Prisms have no EMI mutation -> None."""
    try:
        from kerffuzz.meshgen import Mesh3D, subdivide_edges
    except Exception:
        return None
    return subdivide_edges(inst) if isinstance(inst, Mesh3D) else None


def _unit(payload):
    name, inst, cfg, recipes, res = payload
    cl = configs.label(cfg)
    F, curve = [], []

    def add(kind, slicer, **kw):
        F.append(dict(kind=kind, slicer=slicer, name=name, cfg=cl, **kw))

    adapters = []
    for rc in recipes:
        try:
            adapters.append(_build_adapter(rc, cfg))
        except Exception as e:
            add("adapter_build", rc["kind"], max_um=0.0, detail=str(e)[:150])

    for ad in adapters:
        try:
            det = oracle.determinism(ad, inst, res)
            if det.violation:
                add("determinism", ad.name, max_um=0.0, detail=det.detail)
                continue  # determinism gates the rest
            cont = oracle.containment(ad, inst, res)
            if cont is not None:
                curve.append(dict(slicer=ad.name, name=name, cfg=cl, over_um=cont.max_um))
                if cont.violation:
                    add("containment", ad.name, max_um=cont.max_um, detail=cont.detail)
            # Rotation is an EXACT relation only for prisms (exact 2D cross-section). Arbitrary faceted/
            # curved CSG meshes + slicer wall-generation make Z-rotation only approximately preserved,
            # so meshes rely on determinism + differential + EMI instead.
            if inst.to_kerf_hi() is not None:
                rot = oracle.rotation(ad, inst, 90, res)
                if rot.violation:
                    add("rotate_90", ad.name, max_um=rot.max_um, detail=rot.detail)
            mut = _emi_mutant(inst)
            if mut is not None:
                e = oracle.emi(ad, inst, mut, res)
                if e.violation:
                    add("emi", ad.name, max_um=e.max_um, detail=e.detail)
        except Exception as ex:
            add("crash", ad.name, max_um=0.0, detail=str(ex)[:150])

    if len(adapters) >= 2:
        try:
            d = oracle.differential(adapters[0], adapters[1], inst, res)
            if d.violation:
                add("differential", f"{adapters[0].name}~{adapters[1].name}", max_um=d.max_um, detail=d.detail)
        except Exception as ex:
            add("differential", "pair", max_um=0.0, detail=f"crash: {str(ex)[:130]}")
    return name, cl, F, curve


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--prusa-exe"); ap.add_argument("--prusa-profile")
    ap.add_argument("--cura-exe"); ap.add_argument("--cura-def")
    ap.add_argument("--configs", type=int, default=10)
    ap.add_argument("--random", type=int, default=60)
    ap.add_argument("--seed", type=int, default=11)
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--res", type=int, default=200)
    ap.add_argument("--out", default="runs/campaign")
    args = ap.parse_args()

    recipes = []
    if args.prusa_exe:
        recipes.append({"kind": "prusa", "exe": args.prusa_exe, "profile": args.prusa_profile})
    if args.cura_exe:
        recipes.append({"kind": "cura", "exe": args.cura_exe, "def": args.cura_def})
    assert recipes, "need at least one slicer"

    rng = np.random.default_rng(args.seed)
    shapes = corpus.campaign_corpus(rng, args.random)
    cfgs = configs.sample(np.random.default_rng(args.seed + 1), args.configs)
    units = [(n, i, c, recipes, args.res) for (n, i) in shapes for c in cfgs]

    os.makedirs(args.out, exist_ok=True)
    fpath = os.path.join(args.out, "findings.jsonl")
    cpath = os.path.join(args.out, "curve.jsonl")
    kpath = os.path.join(args.out, "done.txt")
    done = set(open(kpath).read().split()) if os.path.exists(kpath) else set()
    units = [u for u in units if f"{u[0]}@@{configs.label(u[2])}" not in done]

    print(f"campaign: {len(shapes)} shapes x {len(cfgs)} configs x {len(recipes)} slicer(s) "
          f"= {len(units)} units to run ({len(done)} already done)")
    ff = open(fpath, "a"); cf = open(cpath, "a"); kf = open(kpath, "a")
    n_f = 0
    with ProcessPoolExecutor(max_workers=args.workers) as ex:
        futs = [ex.submit(_unit, u) for u in units]
        for i, fut in enumerate(as_completed(futs), 1):
            name, cl, F, curve = fut.result()
            for x in F:
                ff.write(json.dumps(x) + "\n"); n_f += 1
            for x in curve:
                cf.write(json.dumps(x) + "\n")
            kf.write(f"{name}@@{cl}\n")
            if i % 25 == 0:
                ff.flush(); cf.flush(); kf.flush()
                print(f"  {i}/{len(units)} units, {n_f} findings")
    ff.close(); cf.close(); kf.close()

    viols = [json.loads(l) for l in open(fpath)]
    groups = {}
    for v in viols:
        groups.setdefault((v["kind"], v.get("slicer", "?")), []).append(v)
    print(f"\n{len(viols)} findings in {len(groups)} group(s):")
    for key, vs in sorted(groups.items(), key=lambda kv: -max(x.get("max_um", 0) for x in kv[1])):
        worst = max(vs, key=lambda x: x.get("max_um", 0))
        print(f"  {key[0]:16} [{key[1]:24}] x{len(vs):4}  worst {worst.get('max_um',0):.0f}um  "
              f"e.g. {worst['name']}/{worst['cfg']} — {worst['detail'][:50]}")
    print(f"\nfindings.jsonl / curve.jsonl -> {args.out}")


if __name__ == "__main__":
    main()
