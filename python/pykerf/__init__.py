"""pykerf — a Python surface over the Kerf Rust core (``pykerf._kerf``).

The IR crosses the boundary as JSON (see ``demo_square_json`` for the shape).
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
