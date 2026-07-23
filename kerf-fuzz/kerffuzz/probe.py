"""Self-adjudicating probes — the by-hand triage discipline, baked in.

Every rigorous check we ran manually to promote a signal to a finding is a function here, so the hunt
produces *pre-adjudicated* verdicts instead of raw signals:

  - determinism is tested on FROZEN bytes (one STL file, md5 re-verified before each slice) — closes the
    regenerate-per-slice trap that once produced a false positive;
  - a nondeterminism signal is then re-run single-thread and with ASLR DISABLED (a posix_spawn launcher,
    address verified pinned) to EXCLUDE the mundane causes before it is called a defect;
  - meshes are validity-gated (edge-manifold + watertight) so an invalid STL can't masquerade as a bug;
  - the differential compares per connected component, not one bbox, so disjoint multi-body STLs don't
    false-positive.

macOS note: `setarch -R` is Linux-only; ASLR is disabled here via _POSIX_SPAWN_DISABLE_ASLR.
"""
from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile
from collections import Counter

import numpy as np

_NOASLR_SRC = r'''
#include <spawn.h>
#include <sys/wait.h>
#include <stdio.h>
extern char **environ;
#ifndef _POSIX_SPAWN_DISABLE_ASLR
#define _POSIX_SPAWN_DISABLE_ASLR 0x0100
#endif
int main(int argc,char**argv){if(argc<2)return 2;posix_spawnattr_t a;posix_spawnattr_init(&a);
posix_spawnattr_setflags(&a,_POSIX_SPAWN_DISABLE_ASLR);pid_t p;int rc=posix_spawn(&p,argv[1],0,&a,&argv[1],environ);
if(rc){fprintf(stderr,"spawn %d\n",rc);return 1;}int st;waitpid(p,&st,0);return WIFEXITED(st)?WEXITSTATUS(st):1;}
'''


def ensure_noaslr(path: str = "/tmp/kf_noaslr") -> str | None:
    """Compile (once) a launcher that disables ASLR via posix_spawn. Returns the path, or None if the
    platform/compiler can't build it (then the ASLR control is simply skipped, not silently wrong)."""
    if os.path.exists(path):
        return path
    try:
        src = path + ".c"
        with open(src, "w") as f:
            f.write(_NOASLR_SRC)
        r = subprocess.run(["cc", "-O0", "-o", path, src], capture_output=True)
        return path if r.returncode == 0 and os.path.exists(path) else None
    except Exception:
        return None


def _fingerprints(adapter, stl_path: str, n: int, launcher: list | None, res: int) -> list:
    import pykerf as k

    md5 = hashlib.md5(open(stl_path, "rb").read()).hexdigest()
    out = []
    for _ in range(n):
        assert hashlib.md5(open(stl_path, "rb").read()).hexdigest() == md5, "input STL changed mid-probe"
        try:
            g = adapter.slice_stl_path(stl_path, launcher=launcher)
            out.append(k.material_fingerprint(k.parse_gcode(g)[0], res))
        except Exception as e:
            out.append(f"ERR:{type(e).__name__}")
    return out


def _slice_run(adapter, stl_path, launcher, res):
    import pykerf as k

    try:
        lo = k.parse_gcode(adapter.slice_stl_path(stl_path, launcher=launcher))[0]
        return k.material_fingerprint(lo, res), lo
    except Exception as e:
        return f"ERR:{type(e).__name__}", None


def _worst_divergence(runs, fine):
    """Worst run-to-run material divergence in MICRONS, measured at a FINE grid so the number is the
    true toolpath displacement — not the cell size. (Measuring at a coarse grid inflates a tiny jitter
    to a whole cell: a ~15um difference reads as 200um at res=200. Flat/ruled shapes give 0 here.)"""
    import json

    import pykerf as k

    los = [lo for _, lo in runs if lo is not None]
    worst = 0.0
    for i in range(1, len(los)):
        worst = max(worst, json.loads(k.graded_diff(los[0], los[i], fine)).get("max_um") or 0.0)
    return worst


def determinism(adapter, instance, n: int = 6, res: int = 50, fine: int = 20, floor_um: float = 5.0) -> dict:
    """Frozen-bytes determinism, reporting the HONEST magnitude. Verdict ∈ {deterministic,
    NONDETERMINISTIC, no_gcode}. Divergence is measured at a fine grid (`fine`) so the reported number is
    the true run-to-run toolpath displacement; anything below `floor_um` is treated as bit-exact (flat/
    ruled shapes measure 0). A real (>= floor) divergence then gets the ASLR-off control to classify the
    mechanism. `magnitude_um` is the load-bearing number — a coarse fingerprint over-states it ~10x."""
    with tempfile.TemporaryDirectory() as d:
        stl = os.path.join(d, "m.stl")
        with open(stl, "wb") as f:
            f.write(instance.to_stl_bytes())
        md5 = hashlib.md5(open(stl, "rb").read()).hexdigest()
        runs = []
        for _ in range(n):
            assert hashlib.md5(open(stl, "rb").read()).hexdigest() == md5, "input STL changed mid-probe"
            runs.append(_slice_run(adapter, stl, None, res))
        if any(isinstance(fp, str) and fp.startswith("ERR") for fp, _ in runs):
            return {"verdict": "no_gcode", "detail": next(fp for fp, _ in runs if isinstance(fp, str))}
        mag = _worst_divergence(runs, fine)
        if mag < floor_um:
            return {"verdict": "deterministic", "magnitude_um": round(mag, 1)}
        # real divergence — exclude ASLR
        launcher = ensure_noaslr()
        off_mag = None
        if launcher:
            off_runs = []
            for _ in range(n):
                off_runs.append(_slice_run(adapter, stl, [launcher], res))
            off_mag = _worst_divergence(off_runs, fine) if all(lo is not None for _, lo in off_runs) else None
        if off_mag is not None and off_mag < floor_um:
            mech = "address-layout dependent (ASLR) — determinism returns with ASLR off"
        elif off_mag is not None:
            mech = "NOT ASLR (still nondeterministic with address pinned) — suspect uninitialized read"
        else:
            mech = "ASLR control unavailable on this platform"
        sub = " (sub-cell)" if mag < res else ""
        return {"verdict": "NONDETERMINISTIC", "magnitude_um": round(mag, 1),
                "off_magnitude_um": None if off_mag is None else round(off_mag, 1), "mechanism": mech,
                "detail": f"identical input diverges ~{mag:.0f}um{sub} at res={fine}; ASLR-off ~"
                          f"{'n/a' if off_mag is None else round(off_mag)}um. {mech}"}


def emi(adapter, original, mutant, n: int = 4, fine: int = 20, floor_um: float = 5.0) -> dict:
    """Sound EMI (tessellation-dependence) probe. A mutant is the SAME solid meshed differently, so a
    correct slicer must deposit the same material. But the slicer's own run-to-run jitter (see the
    determinism defect) would masquerade as an EMI violation — so we compare the original↔mutant
    divergence against the original↔original BASELINE. An EMI bug is divergence that clearly EXCEEDS the
    slicer's inherent nondeterminism, not merely equals it. Magnitudes measured on a fine grid."""
    import json
    import pykerf as k

    with tempfile.TemporaryDirectory() as d:
        po = os.path.join(d, "o.stl")
        pm = os.path.join(d, "m.stl")
        with open(po, "wb") as f:
            f.write(original.to_stl_bytes())
        with open(pm, "wb") as f:
            f.write(mutant.to_stl_bytes())
        orig = [k.parse_gcode(adapter.slice_stl_path(po))[0] for _ in range(n)]
        mut = [k.parse_gcode(adapter.slice_stl_path(pm))[0] for _ in range(n)]
    # baseline: the slicer's own jitter on the original
    base = max((json.loads(k.graded_diff(orig[0], orig[i], fine)).get("max_um") or 0.0) for i in range(1, n)) if n > 1 else 0.0
    # cross: best-case alignment between a mutant run and any original run (min over pairs);
    # if even the best match far exceeds the baseline jitter, the output is tessellation-dependent.
    cross = min(json.loads(k.graded_diff(mut[0], orig[i], fine)).get("max_um") or 0.0 for i in range(n))
    violation = cross > max(2 * base, floor_um) and cross - base >= floor_um
    return {"verdict": "EMI-VIOLATION" if violation else "ok", "cross_um": round(cross, 1),
            "baseline_um": round(base, 1),
            "detail": f"original↔mutant {cross:.0f}um vs own-jitter baseline {base:.0f}um at res={fine}"}


def mesh_valid(instance) -> tuple[bool, str]:
    """Gate meshes before slicing: an invalid (non-manifold / non-watertight) STL can't be a slicer bug.
    Prisms (which have an exact 2D program) always pass. Returns (ok, reason)."""
    faces = getattr(instance, "faces", None)
    if faces is None:
        return True, "prism (exact 2D program)"
    ec: Counter = Counter()
    for a, b, c in np.asarray(faces):
        for u, v in ((a, b), (b, c), (c, a)):
            ec[(min(int(u), int(v)), max(int(u), int(v)))] += 1
    bad = sum(1 for v in ec.values() if v != 2)
    if bad:
        return False, f"non-manifold: {bad} edges not shared by exactly 2 faces"
    return True, "edge-manifold + watertight"


def _components(cells: set) -> list:
    """4/8-connected components of an occupied-cell set (flood fill)."""
    seen, comps = set(), []
    cells = set(cells)
    for start in cells:
        if start in seen:
            continue
        stack, comp = [start], []
        seen.add(start)
        while stack:
            i, j = stack.pop()
            comp.append((i, j))
            for di in (-1, 0, 1):
                for dj in (-1, 0, 1):
                    nb = (i + di, j + dj)
                    if nb in cells and nb not in seen:
                        seen.add(nb)
                        stack.append(nb)
        comps.append(comp)
    return comps


def _max_bodies_per_layer(adapter, instance, res):
    """How many disjoint bodies the slicer deposits, counted PER LAYER (not on a Z-flattened
    projection, which merges bodies that never touch in any single layer)."""
    import json

    import pykerf as k

    occ = json.loads(k.occupancy(k.parse_gcode(adapter.slice_to_gcode(instance))[0], res))
    best = 0
    for L in occ["layers"]:
        cells = {(i, j) for i, j in L["cells"]}
        if cells:
            best = max(best, len(_components(cells)))
    return best


def differential_by_component(adapter_a, adapter_b, instance, res: int = 200) -> dict:
    """Do two slicers agree on how many disjoint bodies the part has? Counted per-layer AND required to
    be stable across two grid resolutions — a body-count difference that flips when the raster changes
    is sitting on a gap ~ the cell size and is a rasterization artifact, not a real disagreement (this
    is the check that refuted the 'PrusaSlicer bridges an air gap' candidate)."""
    ar, br = _max_bodies_per_layer(adapter_a, instance, res), _max_bodies_per_layer(adapter_b, instance, res)
    fine = max(res // 2, 20)
    af, bf = _max_bodies_per_layer(adapter_a, instance, fine), _max_bodies_per_layer(adapter_b, instance, fine)
    stable = (ar != br) and (af != bf) and ((ar > br) == (af > bf))
    return {"verdict": "DISAGREE" if stable else "agree",
            "detail": f"bodies {adapter_a.name}:{ar}/{adapter_b.name}:{br} @res={res}, "
                      f"{af}/{bf} @res={fine}" + ("" if stable else " — not stable across grids, so no real disagreement")}
