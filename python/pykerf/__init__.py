"""pykerf — a Python surface over the Kerf Rust core (``pykerf._kerf``).

The IR crosses the boundary as JSON (see ``demo_square_json`` for the shape).
"""

from ._kerf import (
    check_self_lowering_sound,
    demo_self_lowering_sound,
    demo_square_gcode,
    demo_square_json,
    demo_travel_order,
    deposit_stats,
    diff_gcode,
    diff_programs,
    graded_diff,
    graded_diff_gcode,
    lower_to_json,
    occupancy,
    parse_gcode,
    program_stats,
    program_to_gcode,
    rotate_z,
    travel_collisions,
    verify_gcode,
    verify_roundtrip,
    version,
    volume_stats,
)

__all__ = [
    "check_self_lowering_sound",
    "demo_self_lowering_sound",
    "demo_square_gcode",
    "demo_square_json",
    "demo_travel_order",
    "deposit_stats",
    "diff_gcode",
    "diff_programs",
    "graded_diff",
    "graded_diff_gcode",
    "lower_to_json",
    "occupancy",
    "parse_gcode",
    "program_stats",
    "program_to_gcode",
    "rotate_z",
    "travel_collisions",
    "verify_gcode",
    "verify_roundtrip",
    "version",
    "volume_stats",
]
__version__ = version()
