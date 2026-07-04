#!/usr/bin/env python3
"""Verify a G-code file with Kerf, from Python.

    uv run python examples/verify.py [path/to/file.gcode]

Checks that a Kerf pass preserves denotation and that geometry is translation-invariant.
"""

import json
import pathlib
import sys

import pykerf

path = sys.argv[1] if len(sys.argv) > 1 else str(pathlib.Path(__file__).parent / "sample.gcode")
report = json.loads(pykerf.verify_gcode(pathlib.Path(path).read_text()))
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
