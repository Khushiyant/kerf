"""Slicer adapters: turn an `Instance` into G-code. One interface, several backends.

- KerfReference: no external binary — uses the instance's exact kerf high-level program (prisms only)
  and emits G-code via pykerf. Exact + metamorphically sound by construction, so it VALIDATES the whole
  harness before any real slicer is trusted.
- prusaslicer / curaengine / orca: write the instance's STL, invoke the real headless CLI with a
  profile + determinism knobs (verified against docs/source), read back the G-code. They run on the
  user's Linux box.

NOTE on infill (design doc §B): infill is *world*-anchored in every slicer, so under a Z-rotation the
infill lines do NOT rotate with the part. For the isometry relations use a perimeter-dominant profile
(fill-density 0, baked into `prusaslicer`/`curaengine` here) or a model-anchored-infill knob.
"""

from __future__ import annotations

import json
import subprocess
import tempfile
from pathlib import Path

from .instance import Instance


class SlicerAdapter:
    name = "base"

    def slice_to_gcode(self, instance: Instance) -> str:  # pragma: no cover - interface
        raise NotImplementedError


class KerfReference(SlicerAdapter):
    """Reference slicer: kerf's own lower -> G-code on the instance's exact 2D program."""

    name = "kerf-ref"

    def slice_to_gcode(self, instance: Instance) -> str:
        import pykerf as k

        hi = instance.to_kerf_hi()
        if hi is None:
            raise ValueError(f"{instance.label}: kerf-ref needs a 2D prism (to_kerf_hi returned None)")
        return k.program_to_gcode(json.dumps(hi))


class _CliSlicer(SlicerAdapter):
    """Write the instance STL, run a CLI, read the produced G-code."""

    name = "cli"

    def __init__(self, exe: str, profile: str, arg_template: list[str], out_name: str = "out.gcode"):
        self.exe = exe
        self.profile = profile
        self.arg_template = arg_template  # tokens; {stl},{profile},{out},{outdir} substituted
        self.out_name = out_name

    def slice_to_gcode(self, instance: Instance) -> str:
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            stl, out = d / "model.stl", d / self.out_name
            stl.write_bytes(instance.to_stl_bytes())
            args = [self.exe] + [
                t.format(stl=str(stl), profile=self.profile, out=str(out), outdir=str(d))
                for t in self.arg_template
            ]
            subprocess.run(args, check=True, capture_output=True, timeout=180)
            gpath = out if out.exists() else next(d.glob("*.gcode"))
            return gpath.read_text()


# Verified determinism knobs (research pass, high confidence).
_PS_DETERMINISM = [
    "--threads", "1", "--arc-fitting", "disabled", "--seam-position", "aligned",
    "--resolution", "0", "--gcode-resolution", "0.0125", "--slice-closing-radius", "0",
    "--staggered-inner-seams", "0", "--spiral-vase", "0",
]


def prusaslicer(profile_ini: str, exe: str = "prusa-slicer", fill_density: str | None = "0") -> _CliSlicer:
    """PrusaSlicer 2.9.6 (verified). Windows: exe='prusa-slicer-console.exe'."""
    extra = ["--fill-density", fill_density] if fill_density is not None else []
    args = ["--export-gcode", "--load", "{profile}", "--output", "{out}", "--dont-arrange",
            "--center", "100,100"] + _PS_DETERMINISM + extra + ["{stl}"]
    a = _CliSlicer(exe, profile_ini, args)
    a.name = "prusaslicer"
    return a


def curaengine(definition_json: str, settings: dict | None = None, exe: str = "CuraEngine") -> _CliSlicer:
    """CuraEngine (Cura 5.13, verified). Cannot read GUI/.curaprofile presets — pass a def.json + `-s`."""
    s = {
        "z_seam_type": "back", "adaptive_layer_height_enabled": "false", "infill_pattern": "lines",
        "infill_offset_x": "0", "infill_offset_y": "0", "fuzzy_skin_enabled": "false",
        "coasting_enable": "false", "center_object": "false",
        "mesh_position_x": "0", "mesh_position_y": "0", "mesh_position_z": "0", "infill_sparse_density": "0",
    }
    s.update(settings or {})
    args = ["slice", "-v", "-m1", "-j", "{profile}"]
    for k, v in s.items():
        args += ["-s", f"{k}={v}"]
    args += ["-l", "{stl}", "-o", "{out}"]
    a = _CliSlicer(exe, definition_json, args)
    a.name = "curaengine"
    return a


def orca(machine_json: str, process_json: str, filament_json: str, exe: str = "orca-slicer") -> _CliSlicer:
    """OrcaSlicer / Bambu (verified). Emits <outdir>/plate_1.gcode. Pin the CPU (taskset) externally
    for byte-stability; set process seam_position=aligned, fuzzy_skin=disabled, enable_arc_fitting=false."""
    profile = f"{machine_json};{process_json}"
    args = ["--slice", "0", "--arrange", "0", "--load-settings", "{profile}",
            "--load-filaments", filament_json, "--outputdir", "{outdir}", "--no-check", "{stl}"]
    a = _CliSlicer(exe, profile, args, out_name="plate_1.gcode")
    a.name = "orca"
    return a
