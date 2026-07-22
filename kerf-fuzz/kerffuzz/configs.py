"""Config-space for mutation-based fuzzing.

Compiler-fuzzing history is unambiguous: configuration-space bugs outnumber input-space bugs. Slicers
have a huge settings surface, so we fuzz over it too — each abstract config maps to per-slicer native
keys. Every value here keeps the oracle's soundness preconditions intact (the adapters separately force
no sparse infill, no skirt/brim, deterministic seam), so metamorphic and differential verdicts stay
valid across the whole config sweep. Seam modes are restricted to deterministic ones (never `random`).
"""
from __future__ import annotations

import itertools

# abstract axes -> the values that flip interesting slicer code paths
AXES = {
    "layer_height": [0.10, 0.15, 0.20, 0.28],   # thin vs thick layers; skin/bridge thresholds shift
    "perimeters":   [1, 2, 3, 5],               # wall-count / gap-fill / thin-wall logic
    "seam":         ["aligned", "rear", "nearest"],  # deterministic seam placement only
}


def to_prusa(cfg: dict) -> dict:
    return {
        "layer_height": f"{cfg['layer_height']}",
        "perimeters": str(cfg["perimeters"]),
        "seam_position": cfg["seam"],
    }


_CURA_SEAM = {"aligned": "sharpest_corner", "rear": "back", "nearest": "shortest"}


def to_cura(cfg: dict) -> dict:
    return {
        "layer_height": f"{cfg['layer_height']}",
        "wall_line_count": str(cfg["perimeters"]),
        "z_seam_type": _CURA_SEAM[cfg["seam"]],
    }


TRANSLATE = {"prusaslicer": to_prusa, "curaengine": to_cura}


def sample(rng, n: int) -> list[dict]:
    ks = list(AXES)
    return [{k: AXES[k][int(rng.integers(len(AXES[k])))] for k in ks} for _ in range(n)]


def grid() -> list[dict]:
    ks = list(AXES)
    return [dict(zip(ks, v)) for v in itertools.product(*[AXES[k] for k in ks])]


def label(cfg: dict) -> str:
    return f"lh{cfg['layer_height']}_p{cfg['perimeters']}_{cfg['seam']}"


def default() -> dict:
    return {"layer_height": 0.20, "perimeters": 3, "seam": "aligned"}
