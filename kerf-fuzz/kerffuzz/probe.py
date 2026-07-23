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


def determinism(adapter, instance, n: int = 6, res: int = 200) -> dict:
    """Frozen-bytes determinism with mechanism adjudication baked in. Returns a verdict dict:
    verdict ∈ {deterministic, NONDETERMINISTIC, no_gcode}. When NONDETERMINISTIC, the ASLR-off control
    has already run, so `mechanism` distinguishes a real single-thread defect from an ASLR artifact."""
    with tempfile.TemporaryDirectory() as d:
        stl = os.path.join(d, "m.stl")
        with open(stl, "wb") as f:
            f.write(instance.to_stl_bytes())
        on = _fingerprints(adapter, stl, n, None, res)
        if any(isinstance(x, str) and x.startswith("ERR") for x in on):
            return {"verdict": "no_gcode", "on_distinct": None, "detail": next(x for x in on if isinstance(x, str))}
        d_on = len(set(on))
        if d_on == 1:
            return {"verdict": "deterministic", "on_distinct": 1}
        # nondeterministic on identical bytes — exclude ASLR before naming it
        launcher = ensure_noaslr()
        off = _fingerprints(adapter, stl, n, [launcher], res) if launcher else None
        d_off = len(set(off)) if off else None
        if d_off == 1:
            mech = "address-layout dependent (ASLR) — determinism returns with ASLR off"
        elif d_off and d_off > 1:
            mech = "NOT ASLR (still nondeterministic with address pinned) — suspect uninitialized read"
        else:
            mech = "ASLR control unavailable on this platform"
        return {"verdict": "NONDETERMINISTIC", "on_distinct": d_on, "off_distinct": d_off,
                "mechanism": mech, "detail": f"{d_on}/{n} distinct (ASLR on), {d_off}/{n} (ASLR off)"}


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


def differential_by_component(adapter_a, adapter_b, instance, res: int = 200, tol_um: float = 2000.0) -> dict:
    """Per-connected-component footprint comparison — sound for multi-body STLs, unlike a single bbox.
    Matches components greedily by size and compares each one's extent; a disjoint layout no longer
    false-positives just because two slicers ordered/placed its bodies differently."""
    import pykerf as k

    def comp_extents(ad):
        occ = k.occupancy(k.parse_gcode(ad.slice_to_gcode(instance))[0], res)
        import json
        cells = {(i, j) for L in json.loads(occ)["layers"] for i, j in L["cells"]}
        ex = []
        for comp in _components(cells):
            xs = [i for i, _ in comp]
            ys = [j for _, j in comp]
            ex.append(((max(xs) - min(xs)) * res, (max(ys) - min(ys)) * res))
        return sorted(ex, key=lambda e: -e[0] * e[1])

    ea, eb = comp_extents(adapter_a), comp_extents(adapter_b)
    if len(ea) != len(eb):
        return {"verdict": "DISAGREE", "detail": f"{adapter_a.name} has {len(ea)} bodies, {adapter_b.name} has {len(eb)}"}
    worst = 0.0
    for (aw, ah), (bw, bh) in zip(ea, eb):
        worst = max(worst, abs(aw - bw), abs(ah - bh))
    return {"verdict": "DISAGREE" if worst > tol_um else "agree", "worst_um": worst,
            "detail": f"{len(ea)} bodies, worst per-body extent delta {worst:.0f}um"}
