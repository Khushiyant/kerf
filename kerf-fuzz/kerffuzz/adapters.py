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
import re
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


# Determinism + containment-safe knobs. PrusaSlicer 2.9's CLI does NOT accept these as flags
# (`--arc-fitting`, `--resolution`, `--fill-density`, … are misparsed as input files), so they are
# baked into an augmented copy of the profile instead — the portable, version-stable way. Skirt/brim/
# support are disabled so the containment gate stays sound (mm of skirt outside the part would trip it).
# Perimeter-dominant + deterministic: sparse infill AND solid skin are disabled, because both are laid
# at a fixed WORLD angle and so do not rotate with the part — leaving them on confounds the isometry
# relations (a rotated part's skin/infill lines fall differently). Perimeters do transform rigidly.
_PS_DETERMINISM = {
    "seam_position": "aligned", "spiral_vase": "0", "arc_fitting": "disabled",
    "gcode_resolution": "0.0125", "slice_closing_radius": "0", "avoid_crossing_perimeters": "0",
    "skirts": "0", "brim_width": "0", "brim_type": "no_brim", "support_material": "0",
    "top_solid_layers": "0", "bottom_solid_layers": "0",
}


class _PrusaSlicer(SlicerAdapter):
    """PrusaSlicer 2.9 (verified on 2.9.6, macOS). Augments the profile with determinism keys, then
    slices with a minimal, portable CLI and the part auto-centred on the bed."""

    name = "prusaslicer"

    def __init__(self, profile_ini: str, exe: str = "prusa-slicer", fill_density: str | None = "0",
                 overrides: dict | None = None):
        self.profile_ini = profile_ini
        self.exe = exe
        self.fill_density = fill_density
        self.overrides = overrides or {}  # native PrusaSlicer keys (e.g. from config-space mutation)

    def _write_profile(self, d: Path) -> Path:
        cfg, order = {}, []
        for ln in Path(self.profile_ini).read_text().splitlines():
            if " = " in ln and not ln.lstrip().startswith("#"):
                key = ln.split(" = ", 1)[0]
                cfg[key] = ln
                order.append(key)
        want = dict(_PS_DETERMINISM)
        if self.fill_density is not None:
            fd = str(self.fill_density)
            want["fill_density"] = fd if "%" in fd else f"{fd}%"
        want.update(self.overrides)
        for key, v in want.items():
            if key in cfg:  # only override keys the profile already declares (avoids unknown-key load errors)
                cfg[key] = f"{key} = {v}"
        p = d / "profile.ini"
        p.write_text("\n".join(cfg[key] for key in order) + "\n")
        return p

    def _bed_center(self) -> str:
        """Centre on the bed midpoint parsed from the profile's bed_shape (mm), not a hardcoded point."""
        for ln in Path(self.profile_ini).read_text().splitlines():
            if ln.startswith("bed_shape"):
                pts = [p.split("x") for p in ln.split("=", 1)[1].split(",")]
                xs = [float(a) for a, _ in pts]
                ys = [float(b) for _, b in pts]
                return f"{(min(xs) + max(xs)) / 2:g},{(min(ys) + max(ys)) / 2:g}"
        return "100,100"

    def slice_stl_path(self, stl_path: str, launcher: list | None = None) -> str:
        """Slice an EXISTING STL file (bytes fixed by the caller). `launcher` prepends a wrapper argv
        (e.g. the ASLR-disabling posix_spawn launcher) so the determinism protocol can pin address layout."""
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            out = d / "out.gcode"
            ini = self._write_profile(d)
            args = (launcher or []) + [self.exe, "--export-gcode", "--load", str(ini), "--output", str(out),
                    "--dont-arrange", "--center", self._bed_center(), stl_path]
            r = None
            for _ in range(2):  # retry once: no-G-code under heavy parallel load can be transient
                r = subprocess.run(args, capture_output=True, timeout=180, text=True)
                gpath = out if out.exists() else next(iter(d.glob("*.gcode")), None)
                if gpath is not None:
                    return gpath.read_text()
            tail = "\n".join((r.stderr or r.stdout or "").splitlines()[-3:])
            raise RuntimeError(f"{self.exe} produced no G-code: {tail}")

    def slice_to_gcode(self, instance: Instance) -> str:
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "model.stl"
            p.write_bytes(instance.to_stl_bytes())
            return self.slice_stl_path(str(p))


def prusaslicer(profile_ini: str, exe: str = "prusa-slicer", fill_density: str | None = "0",
                overrides: dict | None = None) -> _PrusaSlicer:
    """PrusaSlicer 2.9 (verified on 2.9.6). Windows: exe='prusa-slicer-console.exe'. Pass a self-contained
    profile .ini (e.g. `PrusaSlicer --save profile.ini` for the defaults, or a GUI-exported config).
    `overrides` = native PrusaSlicer keys, e.g. `configs.to_prusa(cfg)` for config-space mutation."""
    return _PrusaSlicer(profile_ini, exe, fill_density, overrides)


# CuraEngine (Cura 5.x) settings baked for determinism + containment soundness. Several newer settings
# (roofing/flooring) have unresolved formula defaults standalone, so they must be given explicitly;
# _CuraEngine also self-heals for any shape-specific missing setting via a bounded retry.
_CURA_BASE = {
    "z_seam_type": "back", "adaptive_layer_height_enabled": "false", "infill_pattern": "lines",
    "infill_offset_x": "0", "infill_offset_y": "0", "fuzzy_skin_enabled": "false",
    "coasting_enable": "false", "center_object": "true", "adhesion_type": "none",
    "roofing_layer_count": "0", "flooring_layer_count": "0",
    "machine_width": "300", "machine_depth": "300", "machine_height": "300",
    # perimeter-only: no world-anchored skin OR infill (both misalign under rotation and break the
    # isometry relations). density=0 is NOT enough — Cura derives infill_line_distance separately, and
    # skin comes from top/bottom layer counts + thickness; all must be zeroed.
    "top_layers": "0", "bottom_layers": "0", "top_bottom_thickness": "0",
    "infill_sparse_density": "0", "infill_line_distance": "0",
}
_CURA_MISSING = re.compile(r"no value given:\s*(\w+)")


class _CuraEngine(SlicerAdapter):
    """CuraEngine (Cura 5.13, verified). Cannot read GUI/.curaprofile presets — pass a def.json + `-s`.
    Self-heals for settings whose standalone default is an unresolved formula (retries with `-s X=0`)."""

    name = "curaengine"

    def __init__(self, definition_json: str, settings: dict | None = None, exe: str = "CuraEngine",
                 overrides: dict | None = None, threads: int = 1):
        self.definition_json = definition_json
        self.exe = exe
        self.threads = threads  # -m1: CuraEngine is multithreaded and its output is NONDETERMINISTIC
        self.settings = dict(_CURA_BASE)   # across threads; pin to 1 so the determinism GATE is meaningful.
        self.settings.update(settings or {})
        self.settings.update(overrides or {})  # native Cura keys (e.g. from config-space mutation)

    def slice_stl_path(self, stl_path: str, launcher: list | None = None) -> str:
        """Slice an EXISTING STL file. `launcher` prepends a wrapper argv (e.g. the ASLR-disabling
        launcher) so the determinism protocol can rule address layout in or out as the cause."""
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "out.gcode"
            extra: dict = {}
            last = ""
            for _ in range(16):  # bounded self-heal for unresolved-default settings
                s = {**self.settings, **extra}
                args = (launcher or []) + [self.exe, "slice", "-v", f"-m{self.threads}", "-j", self.definition_json]
                for k, v in s.items():
                    args += ["-s", f"{k}={v}"]
                args += ["-l", stl_path, "-o", str(out)]
                r = subprocess.run(args, capture_output=True, timeout=180, text=True)
                if out.exists() and out.stat().st_size > 0:
                    return out.read_text()
                last = (r.stdout or "") + (r.stderr or "")
                new = [m for m in set(_CURA_MISSING.findall(last)) if m not in extra]
                if not new:
                    break
                for m in new:
                    extra[m] = "0"
            raise RuntimeError(f"CuraEngine produced no G-code: {chr(10).join(last.splitlines()[-3:])}")

    def slice_to_gcode(self, instance: Instance) -> str:
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "model.stl"
            p.write_bytes(instance.to_stl_bytes())
            return self.slice_stl_path(str(p))


def curaengine(definition_json: str, settings: dict | None = None, exe: str = "CuraEngine",
               overrides: dict | None = None, threads: int = 1) -> _CuraEngine:
    return _CuraEngine(definition_json, settings, exe, overrides, threads)


def orca(machine_json: str, process_json: str, filament_json: str, exe: str = "orca-slicer") -> _CliSlicer:
    """OrcaSlicer / Bambu (verified). Emits <outdir>/plate_1.gcode. Pin the CPU (taskset) externally
    for byte-stability; set process seam_position=aligned, fuzzy_skin=disabled, enable_arc_fitting=false."""
    profile = f"{machine_json};{process_json}"
    args = ["--slice", "0", "--arrange", "0", "--load-settings", "{profile}",
            "--load-filaments", filament_json, "--outputdir", "{outdir}", "--no-check", "{stl}"]
    a = _CliSlicer(exe, profile, args, out_name="plate_1.gcode")
    a.name = "orca"
    return a
