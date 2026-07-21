"""The Instance contract — the single interface every test object implements, so the mesh generator,
adapters, and oracle all compose.

An Instance is a sliceable object that: (1) exports a binary STL for real slicers; (2) supports the
isometry/scale transforms the metamorphic oracle applies, each returning a NEW Instance in the frame
the oracle expects — `rotate_z` about the origin, `mirror_x` across the Y axis, `translate` by
microns; and (3) optionally exposes an exact kerf high-level program (`to_kerf_hi`) when it has a true
2D representation (a prism). Only prisms have that, so only they drive the reference slicer and the
zero-external-slicer self-validation; general 3D meshes return None and are sliceable only by real
slicers (the metamorphic relation still holds — the oracle rotates the *denoted output*, not the mesh).
"""

from __future__ import annotations

from abc import ABC, abstractmethod


class Instance(ABC):
    @abstractmethod
    def to_stl_bytes(self) -> bytes:
        """Binary STL of the solid, in millimetres (STL convention)."""

    @abstractmethod
    def rotate_z(self, radians: float) -> "Instance":
        """A copy rotated about the origin (Z axis) by `radians` CCW."""

    @abstractmethod
    def translate(self, dx_um: int, dy_um: int) -> "Instance":
        """A copy translated by (dx, dy) microns."""

    @abstractmethod
    def mirror_x(self) -> "Instance":
        """A copy reflected across the Y axis (x -> -x), winding preserved."""

    def scale(self, factor: float) -> "Instance":
        raise NotImplementedError(f"{type(self).__name__} does not support scale")

    def to_kerf_hi(self) -> dict | None:
        """A kerf high-level program (dict), or None if this instance has no exact 2D representation."""
        return None

    @property
    def label(self) -> str:
        return type(self).__name__
