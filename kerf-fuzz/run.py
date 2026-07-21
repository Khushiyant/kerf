"""Driver: sweep (slicers × instances × invariants) → shrink each violation → write a report.

Default slicer is the kerf reference (needs no external binary) so `python run.py` exercises the whole
pipeline anywhere. Point --slicer at a real binary + profile on the Linux box to actually hunt bugs;
add two or more to enable the cross-slicer differential.
"""

from __future__ import annotations

import argparse
import base64
import math

from kerffuzz import corpus, oracle, report, shrink
from kerffuzz.adapters import KerfReference, curaengine, orca, prusaslicer


def _safe(fn):
    try:
        return fn()
    except Exception as e:  # a crashing slice is itself a finding; here we just can't capture the text
        return f"<slice failed: {e}>"


def _transform_for(kind: str, inst):
    if kind.startswith("rotate_"):
        return inst.rotate_z(math.radians(float(kind[len("rotate_"):-3])))
    if kind == "mirror_x":
        return inst.mirror_x()
    if kind.startswith("translate_"):
        _, dx, dy = kind.split("_")
        return inst.translate(int(dx), int(dy))
    return None


def _check_by_kind(adapter, inst, kind: str, res: int, adapter_b=None):
    if kind == "determinism":
        return oracle.determinism(adapter, inst, res)
    if kind == "containment":
        return oracle.containment(adapter, inst, res)
    if kind == "mirror_x":
        return oracle.mirror(adapter, inst, res)
    if kind.startswith("rotate_"):
        return oracle.rotation(adapter, inst, float(kind[len("rotate_"):-3]), res)
    if kind.startswith("translate_"):
        _, dx, dy = kind.split("_")
        return oracle.translation(adapter, inst, int(dx), int(dy), res)
    if kind.startswith("differential:") and adapter_b is not None:
        return oracle.differential(adapter, adapter_b, inst, res)
    return None


def _violation_dict(adapter, inst, r: oracle.Result, res: int, do_shrink: bool, adapter_b=None) -> dict:
    before = shrink.describe(inst)
    small = inst
    if do_shrink and r.kind != "determinism":
        def still_fails(x):
            res_ = _check_by_kind(adapter, x, r.kind, res, adapter_b)
            return bool(res_ and res_.violation)
        try:
            small = shrink.shrink(inst, still_fails)
        except Exception:
            small = inst
    t = _transform_for(r.kind, small)
    return {
        "slicer": adapter.name,
        "instance": inst.label,
        "invariant": r.kind,
        "soundness_class": r.soundness_class,
        "mean_um": r.mean_um,
        "max_um": r.max_um,
        "detail": r.detail,
        "stl_b64": base64.b64encode(small.to_stl_bytes()).decode(),
        "gcode_base": _safe(lambda: adapter.slice_to_gcode(small)),
        "gcode_transformed": _safe(lambda: adapter.slice_to_gcode(t)) if t is not None else None,
        "shrink": {"before": before, "after": shrink.describe(small)},
    }


def sweep(adapters, instances, res: int = 200, do_shrink: bool = True, outdir: str = "runs/report"):
    violations = []
    for adapter in adapters:
        ref = isinstance(adapter, KerfReference)
        for _name, inst in instances:
            if ref and inst.to_kerf_hi() is None:
                continue  # kerf-ref can only slice prisms
            try:
                results = oracle.run_all(adapter, inst, res)
            except Exception as e:
                # A crash while slicing/denoting IS a bug class (for real slicers).
                violations.append({
                    "slicer": adapter.name, "instance": inst.label, "invariant": "crash",
                    "soundness_class": "GATE", "mean_um": 0.0, "max_um": 0.0,
                    "detail": f"slicer/oracle raised: {e}", "stl_b64": base64.b64encode(inst.to_stl_bytes()).decode(),
                    "gcode_base": None, "gcode_transformed": None, "shrink": None,
                })
                continue
            for r in results:
                if r.violation:
                    violations.append(_violation_dict(adapter, inst, r, res, do_shrink))

    for i in range(len(adapters)):
        for j in range(i + 1, len(adapters)):
            for _name, inst in instances:
                try:
                    r = oracle.differential(adapters[i], adapters[j], inst, res)
                except Exception:
                    continue
                if r.violation:
                    violations.append(_violation_dict(adapters[i], inst, r, res, do_shrink, adapter_b=adapters[j]))

    meta = {"adapters": [a.name for a in adapters], "instances": len(instances), "resolution_um": res}
    path = report.write_report(violations, outdir, meta)
    print(f"{len(violations)} violation(s) written to {path}")
    return violations, path


def _build_adapters(args) -> list:
    adapters = []
    for spec in args.slicer:
        kind, _, profile = spec.partition(":")
        if kind == "kerf-ref":
            adapters.append(KerfReference())
        elif kind == "prusa":
            adapters.append(prusaslicer(profile))
        elif kind == "cura":
            adapters.append(curaengine(profile))
        elif kind == "orca":
            m, p, f = profile.split(",")
            adapters.append(orca(m, p, f))
        else:
            raise SystemExit(f"unknown slicer spec: {spec}")
    return adapters


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--slicer", action="append", default=[],
                    help="repeatable: kerf-ref | prusa:profile.ini | cura:def.json | orca:machine.json,process.json,filament.json")
    ap.add_argument("--res", type=int, default=200)
    ap.add_argument("--random", type=int, default=0, help="add N random corpus instances")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--no-shrink", action="store_true")
    ap.add_argument("--out", type=str, default="runs/report")
    args = ap.parse_args()

    if not args.slicer:
        args.slicer = ["kerf-ref"]
    import numpy as np

    instances = corpus.boundary_corpus()
    if args.random:
        instances += corpus.random_corpus(np.random.default_rng(args.seed), args.random)
    adapters = _build_adapters(args)
    sweep(adapters, instances, args.res, not args.no_shrink, args.out)


if __name__ == "__main__":
    main()
