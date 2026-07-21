"""3D-mesh test-instance generator for kerf-fuzz — the counterpart to shapes.py (prisms).

A `Mesh3D` is a general triangle soup with no exact 2D representation, so `to_kerf_hi` returns None:
it is sliceable only by real slicers, and the metamorphic oracle acts on the *denoted output*, not the
mesh. Vertices live in integer-free float MICRONS (kerf's unit); `to_stl_bytes` divides by 1000 to emit
millimetres. Transforms act on vertices in micron space; `rotate_z` is about the world origin.

CSG primitives and booleans are built with the `manifold3d` library in MILLIMETRES (print scale,
~5-40 mm), then ingested via `Mesh3D.from_manifold`, which multiplies by 1000 to reach microns. If
manifold3d is unavailable, a numpy-only fallback provides watertight primitives and *approximate*
booleans (bounded fallback) so the module still imports and produces valid solids.

Mutations split into two kinds (see `random_mesh` corpus + the metamorphic oracle):

  EMI (Equivalence-Modulo-Inputs) — semantics-preserving, MUST NOT change a correct slicer's output.
  These form metamorphic *pairs* (original vs. mutant should slice identically):
    - add_coplanar_vertex     : subdivide one face by an on-surface point (same solid)
    - weld_duplicate_vertices : merge coincident vertices (same surface)
    - subdivide_edges         : split every edge at its midpoint (same surface)
    - noop_boolean            : union with, then subtract, an identical copy (same solid)

  PERTURBING (non-EMI) — genuinely changes geometry, expected to change output:
    - jitter_vertices         : displace vertices by up to `max_um` microns
"""

from __future__ import annotations

import math
import struct
from dataclasses import dataclass

import numpy as np

try:
    from .instance import Instance
except ImportError:  # run as `python kerffuzz/meshgen.py`
    import os
    import sys

    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    from kerffuzz.instance import Instance

try:
    import manifold3d as _m3d

    _HAVE_MANIFOLD = True
except Exception:  # pragma: no cover - exercised only when the CSG lib is missing
    _m3d = None
    _HAVE_MANIFOLD = False


# ---- the 3D-mesh instance -----------------------------------------------------------------------

@dataclass
class Mesh3D(Instance):
    """A triangle mesh: vertices [N,3] float microns, faces [M,3] int (indices into vertices)."""

    vertices: np.ndarray
    faces: np.ndarray
    name: str = "mesh"

    def __post_init__(self):
        self.vertices = np.asarray(self.vertices, dtype=np.float64).reshape(-1, 3)
        self.faces = np.asarray(self.faces, dtype=np.int64).reshape(-1, 3)

    @property
    def label(self) -> str:
        return self.name

    @property
    def n_tris(self) -> int:
        return int(self.faces.shape[0])

    def _map_verts(self, fn) -> "Mesh3D":
        return Mesh3D(fn(self.vertices), self.faces.copy(), self.name)

    def rotate_z(self, radians: float) -> "Mesh3D":
        c, s = math.cos(radians), math.sin(radians)
        rot = np.array([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]], dtype=np.float64)
        return self._map_verts(lambda v: v @ rot.T)

    def translate(self, dx_um: int, dy_um: int) -> "Mesh3D":
        off = np.array([float(dx_um), float(dy_um), 0.0], dtype=np.float64)
        return self._map_verts(lambda v: v + off)

    def mirror_x(self) -> "Mesh3D":
        # reflect across the Y axis (x -> -x); a reflection flips handedness, so reverse each
        # triangle's winding to keep normals pointing outward.
        v = self.vertices.copy()
        v[:, 0] = -v[:, 0]
        return Mesh3D(v, self.faces[:, ::-1].copy(), self.name)

    def scale(self, factor: float) -> "Mesh3D":
        return self._map_verts(lambda v: v * float(factor))

    def to_kerf_hi(self) -> dict | None:
        return None  # general 3D meshes have no exact 2D representation

    def to_stl_bytes(self) -> bytes:
        """Binary STL in millimetres. Normals recomputed per-face via the right-hand rule."""
        v_mm = self.vertices / 1000.0
        tris = v_mm[self.faces]  # [M,3,3]
        a, b, c = tris[:, 0], tris[:, 1], tris[:, 2]
        normals = np.cross(b - a, c - a)
        lengths = np.linalg.norm(normals, axis=1, keepdims=True)
        normals = np.divide(normals, lengths, out=np.zeros_like(normals), where=lengths > 0)

        m = tris.shape[0]
        buf = bytearray(b"\0" * 80) + struct.pack("<I", m)
        rec = np.zeros((m, 12), dtype="<f4")
        rec[:, 0:3] = normals
        rec[:, 3:6] = a
        rec[:, 6:9] = b
        rec[:, 9:12] = c
        blob = rec.tobytes()
        # 50 bytes per triangle: 48 float bytes + 2-byte attribute count.
        for i in range(m):
            buf += blob[i * 48:(i + 1) * 48]
            buf += struct.pack("<H", 0)
        return bytes(buf)

    @staticmethod
    def from_manifold(m, label: str = "mesh") -> "Mesh3D":
        """Ingest a manifold3d Manifold. Manifolds are built in MM; multiply by 1000 -> microns."""
        mesh = m.get_mesh() if hasattr(m, "get_mesh") else m.to_mesh()
        verts = np.asarray(mesh.vert_properties, dtype=np.float64)[:, :3] * 1000.0
        faces = np.asarray(mesh.tri_verts, dtype=np.int64)
        return Mesh3D(verts, faces, label)

    # Fluent CSG (delegates to the module-level booleans defined below).
    def union(self, other: "Mesh3D") -> "Mesh3D":
        return union(self, other)

    def difference(self, other: "Mesh3D") -> "Mesh3D":
        return difference(self, other)

    def intersection(self, other: "Mesh3D") -> "Mesh3D":
        return intersection(self, other)


# ---- numpy-only fallback primitives (used only when manifold3d is missing) ----------------------

def _fallback_box(w: float, h: float, d: float) -> Mesh3D:
    hx, hy, hz = w / 2.0, h / 2.0, d / 2.0
    v = np.array([
        [-hx, -hy, -hz], [hx, -hy, -hz], [hx, hy, -hz], [-hx, hy, -hz],
        [-hx, -hy, hz], [hx, -hy, hz], [hx, hy, hz], [-hx, hy, hz],
    ], dtype=np.float64) * 1000.0
    f = np.array([
        [0, 3, 2], [0, 2, 1],  # bottom (-z)
        [4, 5, 6], [4, 6, 7],  # top (+z)
        [0, 1, 5], [0, 5, 4],  # -y
        [1, 2, 6], [1, 6, 5],  # +x
        [2, 3, 7], [2, 7, 6],  # +y
        [3, 0, 4], [3, 4, 7],  # -x
    ], dtype=np.int64)
    return Mesh3D(v, f, "box")


def _fallback_cylinder(r: float, h: float, segments: int) -> Mesh3D:
    segments = max(3, int(segments))
    ang = np.linspace(0.0, 2.0 * np.pi, segments, endpoint=False)
    ring = np.stack([r * np.cos(ang), r * np.sin(ang)], axis=1)
    hz = h / 2.0
    bottom = np.column_stack([ring, np.full(segments, -hz)])
    top = np.column_stack([ring, np.full(segments, hz)])
    cb = np.array([[0.0, 0.0, -hz]])
    ct = np.array([[0.0, 0.0, hz]])
    v = np.vstack([bottom, top, cb, ct]) * 1000.0
    ib, it = 2 * segments, 2 * segments + 1
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        faces.append([ib, j, i])                       # bottom cap (outward -z)
        faces.append([it, segments + i, segments + j])  # top cap (outward +z)
        faces.append([i, j, segments + j])              # side
        faces.append([i, segments + j, segments + i])
    return Mesh3D(v, np.array(faces, dtype=np.int64), "cylinder")


def _fallback_sphere(r: float, segments: int) -> Mesh3D:
    segments = max(4, int(segments))
    rings = max(2, segments // 2)
    verts = [[0.0, 0.0, r]]  # north pole
    for i in range(1, rings):
        phi = np.pi * i / rings
        z = r * np.cos(phi)
        rr = r * np.sin(phi)
        for j in range(segments):
            th = 2.0 * np.pi * j / segments
            verts.append([rr * np.cos(th), rr * np.sin(th), z])
    verts.append([0.0, 0.0, -r])  # south pole
    v = np.array(verts, dtype=np.float64) * 1000.0
    south = len(verts) - 1
    faces = []
    for j in range(segments):  # north cap
        faces.append([0, 1 + j, 1 + (j + 1) % segments])
    for i in range(rings - 2):  # middle bands
        a0 = 1 + i * segments
        b0 = 1 + (i + 1) * segments
        for j in range(segments):
            jn = (j + 1) % segments
            faces.append([a0 + j, b0 + j, b0 + jn])
            faces.append([a0 + j, b0 + jn, a0 + jn])
    last = 1 + (rings - 2) * segments  # south cap
    for j in range(segments):
        faces.append([south, last + (j + 1) % segments, last + j])
    return Mesh3D(v, np.array(faces, dtype=np.int64), "sphere")


def _fallback_boolean(a: Mesh3D, b: Mesh3D, op: str) -> Mesh3D:
    """Bounded numpy-only stand-in for CSG. Not a true boolean — it keeps the module usable when
    manifold3d is absent by returning a valid, watertight solid of the appropriate rough extent."""
    if op == "union":
        return Mesh3D(np.vstack([a.vertices, b.vertices]),
                      np.vstack([a.faces, b.faces + len(a.vertices)]), "union")
    if op == "difference":
        return Mesh3D(a.vertices.copy(), a.faces.copy(), "difference")
    # intersection -> the smaller-extent operand as a coarse proxy
    ea = np.ptp(a.vertices, axis=0).prod()
    eb = np.ptp(b.vertices, axis=0).prod()
    src = a if ea <= eb else b
    return Mesh3D(src.vertices.copy(), src.faces.copy(), "intersection")


# ---- parametric solids (sizes in MM, print scale ~5-40 mm) --------------------------------------

def box(w: float, h: float, d: float) -> Mesh3D:
    if _HAVE_MANIFOLD:
        return Mesh3D.from_manifold(_m3d.Manifold.cube([w, h, d], True), "box")
    return _fallback_box(w, h, d)


def cylinder(r: float, h: float, segments: int = 64) -> Mesh3D:
    if _HAVE_MANIFOLD:
        return Mesh3D.from_manifold(
            _m3d.Manifold.cylinder(h, r, r, int(segments), True), "cylinder")
    return _fallback_cylinder(r, h, segments)


def sphere(r: float, segments: int = 32) -> Mesh3D:
    if _HAVE_MANIFOLD:
        return Mesh3D.from_manifold(_m3d.Manifold.sphere(r, int(segments)), "sphere")
    return _fallback_sphere(r, segments)


def _to_manifold(mesh: Mesh3D):
    """Rebuild a manifold3d Manifold from a Mesh3D (microns -> mm)."""
    verts = np.ascontiguousarray(mesh.vertices / 1000.0, dtype=np.float32)
    tris = np.ascontiguousarray(mesh.faces, dtype=np.uint32)
    return _m3d.Manifold(_m3d.Mesh(verts, tris))


def union(a: Mesh3D, b: Mesh3D) -> Mesh3D:
    if _HAVE_MANIFOLD:
        return Mesh3D.from_manifold(_to_manifold(a) + _to_manifold(b), "union")
    return _fallback_boolean(a, b, "union")


def difference(a: Mesh3D, b: Mesh3D) -> Mesh3D:
    if _HAVE_MANIFOLD:
        return Mesh3D.from_manifold(_to_manifold(a) - _to_manifold(b), "difference")
    return _fallback_boolean(a, b, "difference")


def intersection(a: Mesh3D, b: Mesh3D) -> Mesh3D:
    if _HAVE_MANIFOLD:
        return Mesh3D.from_manifold(_to_manifold(a) ^ _to_manifold(b), "intersection")
    return _fallback_boolean(a, b, "intersection")


# ---- EMI (semantics-preserving) mutations -------------------------------------------------------

def add_coplanar_vertex(mesh: Mesh3D, rng: np.random.Generator | None = None) -> Mesh3D:
    """EMI: pick a face, insert its centroid (which lies on the face), and fan it into three
    triangles. The surface is unchanged, so a correct slicer must produce identical output."""
    rng = rng or np.random.default_rng(0)
    if mesh.n_tris == 0:
        return Mesh3D(mesh.vertices.copy(), mesh.faces.copy(), mesh.name)
    fi = int(rng.integers(mesh.n_tris))
    tri = mesh.faces[fi]
    centroid = mesh.vertices[tri].mean(axis=0)
    verts = np.vstack([mesh.vertices, centroid[None, :]])
    c = len(mesh.vertices)
    i0, i1, i2 = int(tri[0]), int(tri[1]), int(tri[2])
    new = np.array([[i0, i1, c], [i1, i2, c], [i2, i0, c]], dtype=np.int64)
    faces = np.vstack([np.delete(mesh.faces, fi, axis=0), new])
    return Mesh3D(verts, faces, mesh.name)


def weld_duplicate_vertices(mesh: Mesh3D, tol_um: float = 1e-6) -> Mesh3D:
    """EMI: merge coincident (or within-`tol_um`) vertices and reindex faces. Same surface, fewer
    vertices. Degenerate faces (a vertex welded onto its neighbour) are dropped."""
    q = np.round(mesh.vertices / max(tol_um, 1e-9)).astype(np.int64)
    _, inverse, first = np.unique(q, axis=0, return_inverse=True, return_index=True)
    inverse = inverse.reshape(-1)
    verts = mesh.vertices[np.sort(first)]
    # map original index -> compact index in the sorted-first order
    order = np.argsort(np.sort(first))  # noqa: F841 (kept for clarity)
    remap = np.empty(len(np.unique(inverse)), dtype=np.int64)
    sorted_first = np.sort(first)
    pos = {int(inverse[fi]): k for k, fi in enumerate(sorted_first)}
    for grp in range(remap.shape[0]):
        remap[grp] = pos[grp]
    faces = remap[inverse[mesh.faces]]
    keep = (faces[:, 0] != faces[:, 1]) & (faces[:, 1] != faces[:, 2]) & (faces[:, 0] != faces[:, 2])
    return Mesh3D(verts, faces[keep], mesh.name)


def subdivide_edges(mesh: Mesh3D) -> Mesh3D:
    """EMI: split every edge at its midpoint, turning each triangle into four coplanar triangles.
    The surface is unchanged (midpoints lie on straight edges)."""
    if mesh.n_tris == 0:
        return Mesh3D(mesh.vertices.copy(), mesh.faces.copy(), mesh.name)
    verts = list(map(tuple, mesh.vertices))
    vindex = {v: i for i, v in enumerate(verts)}
    edge_mid: dict[tuple, int] = {}

    def midpoint(i: int, j: int) -> int:
        key = (min(i, j), max(i, j))
        if key in edge_mid:
            return edge_mid[key]
        m = tuple((mesh.vertices[i] + mesh.vertices[j]) / 2.0)
        if m in vindex:
            idx = vindex[m]
        else:
            idx = len(verts)
            verts.append(m)
            vindex[m] = idx
        edge_mid[key] = idx
        return idx

    new_faces = []
    for tri in mesh.faces:
        a, b, c = int(tri[0]), int(tri[1]), int(tri[2])
        ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
        new_faces += [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]
    return Mesh3D(np.array(verts, dtype=np.float64), np.array(new_faces, dtype=np.int64), mesh.name)


def noop_boolean(mesh: Mesh3D) -> Mesh3D:
    """EMI: union with a coincident copy of the same solid (A ∪ A == A), then subtract a *disjoint*
    copy of that same solid placed far away ((A) − A@far == A). Both operands are identical copies
    of `mesh`; the net effect is the identity, so a correct slicer's output is unchanged. With
    manifold3d this genuinely round-trips through the real boolean kernel.

    (A literal ``(A ∪ A) − A`` with all three coincident collapses to ∅ under exact CSG — that is a
    *deletion*, not a no-op — hence the difference copy is translated clear of A.)"""
    if _HAVE_MANIFOLD:
        m = _to_manifold(mesh)
        span = float(np.ptp(mesh.vertices, axis=0).max()) / 1000.0 + 10.0  # mm, well clear of A
        far = _to_manifold(mesh.translate(int(span * 4 * 1000), int(span * 4 * 1000)))
        result = (m + m) - far
        return Mesh3D.from_manifold(result, mesh.name)
    # fallback: union with self then subtract a disjoint copy is the identity in the coarse model too
    return Mesh3D(mesh.vertices.copy(), mesh.faces.copy(), mesh.name)


# ---- perturbing (non-EMI) mutation --------------------------------------------------------------

def jitter_vertices(mesh: Mesh3D, rng: np.random.Generator, max_um: float) -> Mesh3D:
    """PERTURBING (non-EMI): displace every vertex by up to `max_um` microns per axis. This changes
    the geometry and is expected to change a slicer's output — the metamorphic negative control."""
    noise = rng.uniform(-float(max_um), float(max_um), size=mesh.vertices.shape)
    return Mesh3D(mesh.vertices + noise, mesh.faces.copy(), mesh.name)


# ---- random CSG-tree generator ------------------------------------------------------------------

def _random_primitive(rng: np.random.Generator) -> Mesh3D:
    kind = rng.integers(3)
    if kind == 0:
        return box(*(rng.uniform(5.0, 40.0, 3)))
    if kind == 1:
        return cylinder(rng.uniform(3.0, 20.0), rng.uniform(5.0, 40.0), segments=32)
    return sphere(rng.uniform(3.0, 20.0), segments=24)


def random_mesh(rng: np.random.Generator, max_depth: int = 3) -> Mesh3D:
    """A bounded-depth random CSG tree over box/cylinder/sphere joined by union/difference/
    intersection. Depth is capped at `max_depth`; leaves are primitives."""

    def build(depth: int) -> Mesh3D:
        if depth <= 0 or rng.random() < 0.35:
            return _random_primitive(rng)
        a = build(depth - 1)
        b = build(depth - 1)
        # nudge b so the boolean has some overlap-and-difference to work with
        b = b.translate(int(rng.uniform(-8000, 8000)), int(rng.uniform(-8000, 8000)))
        op = rng.integers(3)
        try:
            m = (union, difference, intersection)[op](a, b)
        except Exception:
            return a
        # a boolean can collapse to nothing (e.g. disjoint intersection); fall back to a operand
        return m if m.n_tris > 0 else a

    return build(max_depth)


# ---- self-check ---------------------------------------------------------------------------------

def _self_check() -> None:
    import os
    import tempfile

    rng = np.random.default_rng(1234)

    b = box(20.0, 12.0, 8.0)
    c = cylinder(6.0, 15.0, segments=48)
    s = sphere(9.0, segments=24)
    u = union(box(15.0, 15.0, 15.0), sphere(10.0))
    d = difference(box(20.0, 20.0, 20.0), cylinder(6.0, 30.0))

    solids = {"box": b, "cylinder": c, "sphere": s, "union": u, "difference": d}
    for name, m in solids.items():
        assert isinstance(m, Mesh3D), name
        assert m.n_tris > 0, f"{name} produced no triangles"
        assert m.to_kerf_hi() is None, f"{name} should have no 2D rep"

    # write one STL, re-read its triangle count, and confirm it round-trips.
    stl = d.to_stl_bytes()
    with tempfile.NamedTemporaryFile(suffix=".stl", delete=False) as fh:
        fh.write(stl)
        path = fh.name
    try:
        with open(path, "rb") as fh:
            fh.seek(80)
            (count,) = struct.unpack("<I", fh.read(4))
        assert count == d.n_tris, f"STL tri count {count} != {d.n_tris}"
        assert len(stl) == 84 + 50 * count, "STL byte length inconsistent"
    finally:
        os.unlink(path)

    # every transform returns a Mesh3D and preserves triangle count.
    for name, m in solids.items():
        for tname, out in {
            "rotate_z": m.rotate_z(0.7),
            "translate": m.translate(3000, -4000),
            "mirror_x": m.mirror_x(),
            "scale": m.scale(1.5),
        }.items():
            assert isinstance(out, Mesh3D), f"{name}.{tname} not a Mesh3D"
            assert out.n_tris == m.n_tris, f"{name}.{tname} changed tri count"

    # mirror_x must flip winding (reversed faces) while negating x.
    mm = b.mirror_x()
    assert np.allclose(mm.vertices[:, 0], -b.vertices[:, 0])
    assert np.array_equal(mm.faces, b.faces[:, ::-1])

    # rotate_z about origin is an isometry: distances from origin (xy) preserved.
    r = b.rotate_z(0.9)
    d0 = np.linalg.norm(b.vertices[:, :2], axis=1)
    d1 = np.linalg.norm(r.vertices[:, :2], axis=1)
    assert np.allclose(np.sort(d0), np.sort(d1))

    # EMI mutations produce valid meshes.
    emi = {
        "add_coplanar_vertex": add_coplanar_vertex(b, rng),
        "weld_duplicate_vertices": weld_duplicate_vertices(subdivide_edges(b)),
        "subdivide_edges": subdivide_edges(b),
        "noop_boolean": noop_boolean(b),
    }
    for name, m in emi.items():
        assert isinstance(m, Mesh3D) and m.n_tris > 0, name
        m.to_stl_bytes()  # must serialise cleanly
    assert emi["subdivide_edges"].n_tris == b.n_tris * 4, "subdivide should quadruple faces"

    # perturbing mutation changes vertices but keeps the topology.
    j = jitter_vertices(b, rng, 50.0)
    assert isinstance(j, Mesh3D) and j.n_tris == b.n_tris
    assert not np.allclose(j.vertices, b.vertices), "jitter should move vertices"

    # random CSG tree.
    for _ in range(5):
        rm = random_mesh(rng)
        assert isinstance(rm, Mesh3D) and rm.n_tris > 0
        rm.to_stl_bytes()

    backend = "manifold3d" if _HAVE_MANIFOLD else "numpy-fallback"
    print(f"meshgen OK ({backend})")


if __name__ == "__main__":
    _self_check()
