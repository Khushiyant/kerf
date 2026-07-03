#!/usr/bin/env python3
"""Verify a G-code file with Kerf, from Python.

    uv run python examples/verify.py [path/to/file.gcode]

Parses the G-code into Kerf's IR, then checks that Kerf's own operations preserve the deposited
material (a Kerf pass keeps `denote` unchanged) and that the geometry is translation-invariant — the
soundness properties GlitchFinder cannot express, run here on real slicer output.
"""

import json
import pathlib
import sys

import kerf

path = sys.argv[1] if len(sys.argv) > 1 else str(pathlib.Path(__file__).parent / "sample.gcode")
report = json.loads(kerf.verify_gcode(pathlib.Path(path).read_text()))
d = report["diagnostics"]

print(f"{path}")
print(f"  layers:            {d['layers']}")
print(f"  extruding paths:   {d['extruding_toolpaths']}")
print(f"  estimated widths:  {d['estimated_width_moves']} moves")
if d["unknown_roles"]:
    print(f"  unknown roles:     {d['unknown_roles']} ({d['fallback_role_moves']} moves, filed as Perimeter)")
print(f"  pass preserves denotation: {report['pass_preserves_denotation']}")
print(f"  translation-invariant:     {report['translation_invariant']}")

if not report["has_geometry"]:
    print("  => NOTHING TO VERIFY (no extruding geometry recovered)")
    sys.exit(3)
sound = report["pass_preserves_denotation"] and report["translation_invariant"]
print("  => SOUND" if sound else "  => UNSOUND")
sys.exit(0 if sound else 1)
