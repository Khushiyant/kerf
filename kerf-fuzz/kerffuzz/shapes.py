"""Prisms — 2D polygons (optionally with holes) extruded in Z. A prism is the one instance type that
has BOTH an exact kerf representation (`to_kerf_hi`, driving the reference slicer + self-validation)
and an STL (for real slicers); every transform acts on the polygon so both views stay in lock-step.
Coordinates are integer microns; STL is emitted in millimetres.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass, field

import numpy as np

from .instance import Instance


# ---- polygon generators (return [K,2] int64 microns, CCW, open ring) ----------------------------

def rect(w_um: int, h_um: int, cx: int = 0, cy: int = 0) -> np.ndarray:
    hw, hh = w_um // 2, h_um // 2
    return np.array([[cx - hw, cy - hh], [cx + hw, cy - hh], [cx + hw, cy + hh], [cx - hw, cy + hh]], dtype=np.int64)


def regular_polygon(sides: int, r_um: int, cx: int = 0, cy: int = 0) -> np.ndarray:
    a = np.linspace(0, 2 * np.pi, sides, endpoint=False)
    return np.stack([cx + r_um * np.cos(a), cy + r_um * np.sin(a)], axis=1).round().astype(np.int64)


def star(points: int, r_out: int, r_in: int, cx: int = 0, cy: int = 0) -> np.ndarray:
    a = np.linspace(0, 2 * np.pi, 2 * points, endpoint=False)
    r = np.where(np.arange(2 * points) % 2 == 0, r_out, r_in)
    return np.stack([cx + r * np.cos(a), cy + r * np.sin(a)], axis=1).round().astype(np.int64)


def _convex_hull(pts: np.ndarray) -> np.ndarray:
    """Andrew's monotone chain — CCW open ring (int64). Used to make `random_convex` actually convex."""
    p = sorted(set(map(tuple, pts.tolist())))
    if len(p) < 3:
        return np.array(p, dtype=np.int64)

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lower = []
    for q in p:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], q) <= 0:
            lower.pop()
        lower.append(q)
    upper = []
    for q in reversed(p):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], q) <= 0:
            upper.pop()
        upper.append(q)
    return np.array(lower[:-1] + upper[:-1], dtype=np.int64)


def random_convex(rng: np.random.Generator, sides: int, r_um: int) -> np.ndarray:
    """A genuinely convex polygon: the convex hull of `sides` angularly-spread jittered-radius points
    (the raw points are star-shaped, i.e. usually non-convex — hulling honours the name and keeps the
    prism STL's cap valid)."""
    a = np.sort(rng.uniform(0, 2 * np.pi, sides))
    r = rng.uniform(0.4, 1.0, sides) * r_um
    pts = np.stack([r * np.cos(a), r * np.sin(a)], axis=1).round().astype(np.int64)
    return _convex_hull(pts)


def _closed(ring: np.ndarray) -> dict:
    pts = [{"x": int(x), "y": int(y)} for x, y in ring]
    pts.append(pts[0])  # close the loop
    return {"points": pts}


def _is_convex(ring: np.ndarray) -> bool:
    n = len(ring)
    signs = []
    for i in range(n):
        a, b, c = ring[i], ring[(i + 1) % n], ring[(i + 2) % n]
        cr = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
        if cr != 0:
            signs.append(cr > 0)
    return all(signs) or not any(signs)


def _cap_tris(ring: np.ndarray) -> list:
    """Triangulate a simple polygon into index triples. A fan works for convex rings (no dependency);
    a concave ring MUST use a real triangulator or the fan bridges across notches and the STL solid
    spills outside the true footprint (a silent false-positive generator for the containment gate)."""
    if _is_convex(ring):
        return [(0, i, i + 1) for i in range(1, len(ring) - 1)]
    import manifold3d as m3

    return [tuple(int(x) for x in t) for t in np.asarray(m3.triangulate([ring.astype(np.float64)]))]


# ---- the prism instance -------------------------------------------------------------------------

@dataclass
class Prism(Instance):
    outer: np.ndarray
    holes: list = field(default_factory=list)
    n_layers: int = 5
    layer_h_um: int = 200
    width_um: int = 400
    name: str = "prism"

    @property
    def label(self) -> str:
        return self.name

    def _map(self, fn) -> "Prism":
        return Prism(fn(self.outer), [fn(h) for h in self.holes], self.n_layers, self.layer_h_um, self.width_um, self.name)

    def rotate_z(self, radians: float) -> "Prism":
        c, s = np.cos(radians), np.sin(radians)
        rot = np.array([[c, -s], [s, c]])
        return self._map(lambda p: (p @ rot.T).round().astype(np.int64))

    def translate(self, dx_um: int, dy_um: int) -> "Prism":
        return self._map(lambda p: p + np.array([dx_um, dy_um], dtype=np.int64))

    def mirror_x(self) -> "Prism":
        # reflect across Y, reverse winding so the ring stays CCW
        return self._map(lambda p: np.stack([-p[:, 0], p[:, 1]], axis=1)[::-1].copy())

    def scale(self, factor: float) -> "Prism":
        return self._map(lambda p: (p * factor).round().astype(np.int64))

    def to_kerf_hi(self) -> dict:
        """kerf high-level program: each layer fills the outer ring (and hole rings) as an ExtrudePath."""
        layers = []
        for i in range(self.n_layers):
            fills = [{"path": _closed(self.outer), "width_um": self.width_um}]
            fills += [{"path": _closed(h), "width_um": self.width_um} for h in self.holes]
            layers.append({
                "z_um": (i + 1) * self.layer_h_um,
                "regions": [{
                    "kind": "Perimeter",
                    "boundary": {"outer": _closed(self.outer), "holes": [_closed(h) for h in self.holes]},
                    "fills": fills,
                }],
            })
        return {"layers": layers}

    def footprint(self) -> np.ndarray:
        """The outer polygon (microns) — the object's XY footprint, for the containment gate."""
        return self.outer

    def to_stl_bytes(self) -> bytes:
        h_mm = self.n_layers * self.layer_h_um / 1000.0
        ring = self.outer
        v = [(float(x) / 1000.0, float(y) / 1000.0) for x, y in ring]
        cap = _cap_tris(ring)  # correct for convex AND concave outer polygons
        tris = []
        for i, j, k in cap:  # bottom cap (normal down: reversed), top cap (normal up)
            tris.append(((v[i][0], v[i][1], 0.0), (v[k][0], v[k][1], 0.0), (v[j][0], v[j][1], 0.0)))
            tris.append(((v[i][0], v[i][1], h_mm), (v[j][0], v[j][1], h_mm), (v[k][0], v[k][1], h_mm)))
        for i in range(len(v)):
            a, b = v[i], v[(i + 1) % len(v)]
            tris.append(((a[0], a[1], 0.0), (b[0], b[1], 0.0), (b[0], b[1], h_mm)))
            tris.append(((a[0], a[1], 0.0), (b[0], b[1], h_mm), (a[0], a[1], h_mm)))
        buf = bytearray(b"\0" * 80) + struct.pack("<I", len(tris))
        for t in tris:
            buf += struct.pack("<3f", 0.0, 0.0, 0.0)
            for vert in t:
                buf += struct.pack("<3f", *vert)
            buf += struct.pack("<H", 0)
        return bytes(buf)
