"""pykerf — a Python surface over the Kerf Rust core (``pykerf._kerf``).

The IR crosses the boundary as JSON (see ``demo_square_json`` for the shape) for the stateless
functions. For hot loops (search / RL over move plans), use the native :class:`Program` handle, which
keeps the plan in Rust and answers denote / stats / objective / verify queries with no per-step JSON.

Featurization helpers (:func:`occupancy_array`, :func:`feature_array`) return numpy arrays; numpy is
imported lazily, so it is only required if you call them.
"""

from ._kerf import (
    Program,
    apply_action,
    canonical_hash,
    check_self_lowering_sound,
    compare_rotated,
    default_machine_profile,
    demo_self_lowering_sound,
    demo_square_gcode,
    demo_square_json,
    demo_travel_order,
    denote_flow,
    deposit_stats,
    diff_gcode,
    diff_programs,
    e_conserved,
    export_versioned,
    feature_columns,
    feature_matrix,
    flow_stats,
    graded_diff,
    graded_diff_gcode,
    import_versioned,
    is_printable,
    legal_actions,
    lower_to_json,
    occupancy,
    occupancy_grid,
    parse_gcode,
    preserves_within,
    print_time,
    program_stats,
    program_to_gcode,
    rot_x90,
    rot_y90,
    rot_z90,
    rotate_bounds,
    rotate_z,
    travel_collisions,
    travel_graph,
    verify_batch,
    verify_gcode,
    verify_roundtrip,
    version,
    volume_stats,
    voxelize,
)

__all__ = [
    "Program",
    "apply_action",
    "canonical_hash",
    "check_self_lowering_sound",
    "compare_rotated",
    "default_machine_profile",
    "demo_self_lowering_sound",
    "demo_square_gcode",
    "demo_square_json",
    "demo_travel_order",
    "denote_flow",
    "deposit_stats",
    "diff_gcode",
    "diff_programs",
    "e_conserved",
    "export_versioned",
    "feature_array",
    "feature_columns",
    "feature_matrix",
    "flow_stats",
    "graded_diff",
    "graded_diff_gcode",
    "import_versioned",
    "is_printable",
    "legal_actions",
    "lower_to_json",
    "occupancy",
    "occupancy_array",
    "occupancy_grid",
    "parse_gcode",
    "preserves_within",
    "print_time",
    "program_stats",
    "program_to_gcode",
    "rot_x90",
    "rot_y90",
    "rot_z90",
    "rotate_bounds",
    "rotate_z",
    "travel_collisions",
    "travel_graph",
    "verify_batch",
    "verify_gcode",
    "verify_roundtrip",
    "version",
    "volume_stats",
    "voxelize",
]

__version__ = version()


def feature_array(program_json):
    """The per-toolpath feature matrix as a numpy ``(n_toolpaths, len(feature_columns()))`` array.

    Columns are named by :func:`feature_columns`. Requires numpy.
    """
    import numpy as np

    rows, cols, flat = feature_matrix(program_json)
    return np.asarray(flat, dtype=np.float64).reshape(rows, cols)


def occupancy_array(program_json, layer, resolution_um=200):
    """A dense 0/1 occupancy raster for one layer as a numpy ``(rows, cols)`` uint8 array, plus its
    grid origin ``(min_i, min_j)``. Returns ``(array, (min_i, min_j))``. Requires numpy.
    """
    import numpy as np

    rows, cols, min_i, min_j, data = occupancy_grid(program_json, layer, resolution_um)
    arr = np.frombuffer(bytes(data), dtype=np.uint8).reshape(rows, cols)
    return arr, (min_i, min_j)
