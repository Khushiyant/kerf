"""Kerf — an LLVM for slicing.

A thin Python surface over the Rust core (compiled as ``kerf._kerf``). The IR, lowering, backends,
passes, and the correctness oracle all live in Rust (``crates/kerf-core``); this package re-exports a
clean API. The IR crosses the boundary as JSON (see ``demo_square_json`` for the shape), so you can
build, inspect, and verify arbitrary programs from Python.

See ``docs/`` for the research and design record, starting with ``docs/00-thesis.md``.
"""

from ._kerf import (
    check_self_lowering_sound,
    demo_self_lowering_sound,
    demo_square_gcode,
    demo_square_json,
    demo_travel_order,
    diff_gcode,
    lower_to_json,
    parse_gcode,
    program_to_gcode,
    verify_gcode,
    version,
)

__all__ = [
    "check_self_lowering_sound",
    "demo_self_lowering_sound",
    "demo_square_gcode",
    "demo_square_json",
    "demo_travel_order",
    "diff_gcode",
    "lower_to_json",
    "parse_gcode",
    "program_to_gcode",
    "verify_gcode",
    "version",
]
__version__ = version()
