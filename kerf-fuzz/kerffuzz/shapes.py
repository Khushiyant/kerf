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


def random_convex(rng: np.random.Generator, sides: int, r_um: int) -> np.ndarray:
    a = np.sort(rng.uniform(0, 2 * np.pi, sides))
    r = rng.uniform(0.4, 1.0, sides) * r_um
    return np.stack([r * np.cos(a), r * np.sin(a)], axis=1).round().astype(np.int64)


def _closed(ring: np.ndarray) -> dict:
    pts = [{"x": int(x), "y": int(y)} for x, y in ring]
    pts.append(pts[0])  # close the loop
    return {"points": pts}


def _fan(ring, z: float, up: bool) -> list:
    v = [(float(x) / 1000.0, float(y) / 1000.0, z) for x, y in ring]
    return [(v[0], v[i], v[i + 1]) if up else (v[0], v[i + 1], v[i]) for i in range(1, len(v) - 1)]


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
        tris = _fan(ring, 0.0, up=False) + _fan(ring, h_mm, up=True)
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
