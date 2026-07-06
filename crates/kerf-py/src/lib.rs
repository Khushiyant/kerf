//! PyO3 bindings over `kerf-core`. The IR crosses the boundary as JSON (`kerf_core::json`) for the
//! stateless functions; the hot loop uses the native [`handle::Program`] instead (no per-step JSON).

mod handle;

use kerf_core::ir::{hi, lo, Area, ExtrudePath, Point, Polyline, RegionKind};
use kerf_core::kinematics::MachineProfile;
use kerf_core::pass::{Pass, TravelOrder};
use kerf_core::{backend, denote, frontend, json, lower};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

fn demo_square(side_mm: f64, z_mm: f64) -> hi::Program {
    let s = (side_mm * 1000.0) as i64;
    let z = (z_mm * 1000.0) as i64;
    let outer = Polyline::new(vec![
        Point::new(0, 0),
        Point::new(s, 0),
        Point::new(s, s),
        Point::new(0, s),
        Point::new(0, 0),
    ]);
    hi::Program {
        layers: vec![hi::Layer {
            z_um: z,
            regions: vec![hi::Region {
                kind: RegionKind::Perimeter,
                boundary: Area {
                    outer: outer.clone(),
                    holes: vec![],
                },
                fills: vec![ExtrudePath {
                    path: outer,
                    width_um: 400,
                }],
            }],
        }],
    }
}

fn demo_scattered() -> hi::Program {
    let seg = |x: i64| Polyline::new(vec![Point::new(x, 0), Point::new(x + 100, 0)]);
    let xs = [0, 8000, 2000, 6000, 4000];
    hi::Program {
        layers: vec![hi::Layer {
            z_um: 200,
            regions: vec![hi::Region {
                kind: RegionKind::Infill,
                boundary: Area::default(),
                fills: xs
                    .iter()
                    .map(|&x| ExtrudePath {
                        path: seg(x),
                        width_um: 400,
                    })
                    .collect(),
            }],
        }],
    }
}

fn parse_hi(hi_program_json: &str) -> PyResult<hi::Program> {
    json::from_json(hi_program_json).map_err(|e| {
        PyValueError::new_err(format!(
            "expected a HIGH-LEVEL program JSON (layers of regions with fills, e.g. from \
             demo_square_json()); {e}"
        ))
    })
}

fn parse_lo(lo_program_json: &str) -> PyResult<kerf_core::ir::lo::Program> {
    json::from_json(lo_program_json).map_err(|e| {
        PyValueError::new_err(format!(
            "expected a LOW-LEVEL program JSON (layers of toolpaths, e.g. from lower_to_json() or \
             parse_gcode()); {e}"
        ))
    })
}

fn parse_voxels(voxels_json: &str) -> PyResult<kerf_core::voxel::Voxels> {
    json::from_json(voxels_json).map_err(|e| {
        PyValueError::new_err(format!(
            "expected a voxel-set JSON (array of [i, j, k], e.g. from voxelize()); {e}"
        ))
    })
}

/// Lower a HIGH-LEVEL program (JSON with regions/fills, e.g. from `demo_square_json`) and emit G-code.
#[pyfunction]
fn program_to_gcode(hi_program_json: &str) -> PyResult<String> {
    Ok(backend::to_gcode(&lower::lower(&parse_hi(
        hi_program_json,
    )?)))
}

/// Lower a HIGH-LEVEL program (JSON) to the low-level move plan, returned as JSON. This is the bridge:
/// its output is the LOW-LEVEL JSON the analysis/verify functions consume.
#[pyfunction]
fn lower_to_json(hi_program_json: &str) -> PyResult<String> {
    let lowered = lower::lower(&parse_hi(hi_program_json)?);
    json::to_json(&lowered).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Does lowering the given HIGH-LEVEL program (JSON) preserve the deposited material at this
/// resolution (microns)?
#[pyfunction]
#[pyo3(signature = (hi_program_json, resolution_um=200))]
fn check_self_lowering_sound(hi_program_json: &str, resolution_um: i64) -> PyResult<bool> {
    Ok(denote::self_lowering_sound(
        &parse_hi(hi_program_json)?,
        resolution_um,
    ))
}

/// The demo square serialized to JSON — a template for building programs.
#[pyfunction]
fn demo_square_json() -> PyResult<String> {
    json::to_json(&demo_square(20.0, 0.2)).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// G-code for a demo square.
#[pyfunction]
#[pyo3(signature = (side_mm=20.0, z_mm=0.2))]
fn demo_square_gcode(side_mm: f64, z_mm: f64) -> String {
    backend::to_gcode(&lower::lower(&demo_square(side_mm, z_mm)))
}

/// Is the demo square's lowering denotation-preserving at this resolution?
#[pyfunction]
#[pyo3(signature = (side_mm=20.0, z_mm=0.2, resolution_um=200))]
fn demo_self_lowering_sound(side_mm: f64, z_mm: f64, resolution_um: i64) -> bool {
    denote::self_lowering_sound(&demo_square(side_mm, z_mm), resolution_um)
}

/// Demonstrate the travel-order pass: `(sound, travel_before_mm, travel_after_mm)`.
#[pyfunction]
#[pyo3(signature = (resolution_um=200))]
fn demo_travel_order(resolution_um: i64) -> (bool, f64, f64) {
    let lowered = lower::lower(&demo_scattered());
    let before = lowered.travel_distance_um();
    let optimized = TravelOrder::default().run(lowered.clone());
    let after = optimized.travel_distance_um();
    let sound =
        denote::denote_lo(&lowered, resolution_um) == denote::denote_lo(&optimized, resolution_um);
    (sound, before / 1000.0, after / 1000.0)
}

/// Parse slicer G-code into the IR. Returns `(program_json, diagnostics_json)`. Never raises on
/// malformed G-code; it degrades and reports via diagnostics.
#[pyfunction]
fn parse_gcode(gcode: &str) -> PyResult<(String, String)> {
    let report = frontend::parse(gcode);
    let program =
        json::to_json(&report.program).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let diagnostics =
        json::to_json(&report.diagnostics).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok((program, diagnostics))
}

/// Verify slicer G-code end to end: parse it, then check that a Kerf pass preserves the deposited
/// material and that the geometry is translation-invariant. Returns a JSON report.
#[pyfunction]
#[pyo3(signature = (gcode, resolution_um=200))]
fn verify_gcode(gcode: &str, resolution_um: i64) -> PyResult<String> {
    let v = kerf_core::verify::verify_gcode(gcode, resolution_um);
    json::to_json(&v).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Compare two G-code files by the material they deposit (matched by layer height). Returns a JSON
/// report (per-layer differences, totals, identical flag).
#[pyfunction]
#[pyo3(signature = (a, b, resolution_um=200))]
fn diff_gcode(a: &str, b: &str, resolution_um: i64) -> PyResult<String> {
    let d = kerf_core::diff_gcode(a, b, resolution_um);
    json::to_json(&d).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Emit an (optimized) low-level move plan (JSON, as returned by `lower_to_json`) to G-code, re-parse
/// it, and check the deposited material is preserved. The lo->G-code emitter is outside the verified
/// boundary, so run this before trusting emitted G-code (e.g. an RL agent's output). Returns a JSON
/// report with `occupancy_preserved` / `deposit_preserved`.
#[pyfunction]
#[pyo3(signature = (lo_program_json, resolution_um=200))]
fn verify_roundtrip(lo_program_json: &str, resolution_um: i64) -> PyResult<String> {
    let rt = kerf_core::verify::verify_roundtrip(&parse_lo(lo_program_json)?, resolution_um);
    json::to_json(&rt).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Compare two low-level programs (JSON, as from `lower_to_json`) by the material they deposit.
/// Returns a JSON report including a scalar `iou` similarity (1.0 identical, 0.0 disjoint, null if
/// both empty) and a per-layer breakdown. General-purpose: regression tests, optimizer/agent scoring,
/// or comparing any two move plans without going through G-code text.
#[pyfunction]
#[pyo3(signature = (a_json, b_json, resolution_um=200))]
fn diff_programs(a_json: &str, b_json: &str, resolution_um: i64) -> PyResult<String> {
    let a: kerf_core::ir::lo::Program = json::from_json(a_json)
        .map_err(|e| PyValueError::new_err(format!("invalid low-level program JSON (a): {e}")))?;
    let b: kerf_core::ir::lo::Program = json::from_json(b_json)
        .map_err(|e| PyValueError::new_err(format!("invalid low-level program JSON (b): {e}")))?;
    let d = kerf_core::diff_programs(&a, &b, resolution_um);
    json::to_json(&d).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// The deposited-material occupancy of a low-level program (JSON) at a resolution: per-layer occupied
/// cells (`[[x, y], ...]`). Useful as a spatial observation/feature, for custom metrics, or drawing.
#[pyfunction]
#[pyo3(signature = (program_json, resolution_um=200))]
fn occupancy(program_json: &str, resolution_um: i64) -> PyResult<String> {
    let occ = denote::denote_lo(&parse_lo(program_json)?, resolution_um);
    json::to_json(&occ).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Rotate a LOW-LEVEL program (JSON) about the origin by `radians` (CCW) in the XY plane, returned as
/// JSON. Combine with `graded_diff` to compare two prints up to a known Z-rotation (graded distance
/// absorbs the sub-cell rounding) — e.g. rotation-augmented RL.
#[pyfunction]
fn rotate_z(lo_program_json: &str, radians: f64) -> PyResult<String> {
    let rotated = kerf_core::metamorphic::rotate_z(&parse_lo(lo_program_json)?, radians);
    json::to_json(&rotated).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Lift a LOW-LEVEL program (JSON) to 3D voxels on a uniform `resolution_um` grid, returned as JSON
/// (`[[i, j, k], ...]`). Each layer's cells are extruded over the Z range they deposit; layer height
/// is derived from consecutive Z unless `layer_height_um` is given. The input to the rotation API.
#[pyfunction]
#[pyo3(signature = (lo_program_json, resolution_um=200, layer_height_um=None))]
fn voxelize(
    lo_program_json: &str,
    resolution_um: i64,
    layer_height_um: Option<i64>,
) -> PyResult<String> {
    let v = kerf_core::voxel::voxelize(&parse_lo(lo_program_json)?, resolution_um, layer_height_um);
    json::to_json(&v).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Exact 90° rotation (CCW about +X) of a voxel set (JSON, as from `voxelize`). Integer index map,
/// lossless. See also `rot_y90`, `rot_z90`.
#[pyfunction]
fn rot_x90(voxels_json: &str) -> PyResult<String> {
    let r = kerf_core::voxel::rot_x90(&parse_voxels(voxels_json)?);
    json::to_json(&r).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Exact 90° rotation (CCW about +Y) of a voxel set (JSON). Integer index map, lossless.
#[pyfunction]
fn rot_y90(voxels_json: &str) -> PyResult<String> {
    let r = kerf_core::voxel::rot_y90(&parse_voxels(voxels_json)?);
    json::to_json(&r).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Exact 90° rotation (CCW about +Z) of a voxel set (JSON). Integer index map, lossless — and
/// denotation-equivariant (proven in `proofs/KerfProofs.lean`).
#[pyfunction]
fn rot_z90(voxels_json: &str) -> PyResult<String> {
    let r = kerf_core::voxel::rot_z90(&parse_voxels(voxels_json)?);
    json::to_json(&r).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Sound (inner, outer) voxel-set bounds for rotating `voxels` (JSON) by `radians` about X
/// (`axis_x=True`) or Y. Returns `(inner_json, outer_json)`: `inner` is definitely covered, `outer`
/// possibly covered, with `inner ⊆ true rotation ⊆ outer`. Arbitrary angles don't align to the grid,
/// so these bracket the answer point-sampling can't bound.
#[pyfunction]
#[pyo3(signature = (voxels_json, radians, axis_x=true))]
fn rotate_bounds(voxels_json: &str, radians: f64, axis_x: bool) -> PyResult<(String, String)> {
    let (inner, outer) =
        kerf_core::voxel::rotate_bounds(&parse_voxels(voxels_json)?, radians, axis_x);
    let inner = json::to_json(&inner).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let outer = json::to_json(&outer).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok((inner, outer))
}

/// Sound verdict comparing voxel set `b` against `a` rotated by `radians` about X (`axis_x=True`) or
/// Y. Returns `"SameWithinGrid"` (indistinguishable at this grid) or `"DefinitelyDiffer"` (differ by
/// more than the grid can absorb). Both arguments are voxel-set JSON (from `voxelize`).
#[pyfunction]
#[pyo3(signature = (a_voxels_json, b_voxels_json, radians, axis_x=true))]
fn compare_rotated(
    a_voxels_json: &str,
    b_voxels_json: &str,
    radians: f64,
    axis_x: bool,
) -> PyResult<String> {
    let verdict = kerf_core::voxel::compare_rotated(
        &parse_voxels(a_voxels_json)?,
        &parse_voxels(b_voxels_json)?,
        radians,
        axis_x,
    );
    Ok(match verdict {
        kerf_core::voxel::RotationVerdict::SameWithinGrid => "SameWithinGrid",
        kerf_core::voxel::RotationVerdict::DefinitelyDiffer => "DefinitelyDiffer",
    }
    .to_string())
}

/// Size and efficiency stats for a program (JSON): layer and toolpath counts and total travel
/// distance (an efficiency / print-time proxy).
#[pyfunction]
fn program_stats(program_json: &str) -> PyResult<String> {
    let s = kerf_core::analyze::program_stats(&parse_lo(program_json)?);
    json::to_json(&s).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Over-deposition stats for a program (JSON) at a resolution: over-deposited cell count, graded
/// redeposition magnitude, and max multiplicity. Counts paths per cell, not filament volume.
#[pyfunction]
#[pyo3(signature = (program_json, resolution_um=200))]
fn deposit_stats(program_json: &str, resolution_um: i64) -> PyResult<String> {
    let s = kerf_core::analyze::deposit_stats(&parse_lo(program_json)?, resolution_um);
    json::to_json(&s).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Heuristic, report-only travel-vs-material check for a program (JSON): deposited cells each layer's
/// travels pass through, a nozzle-drag / stringing proxy. Not exact collision detection.
#[pyfunction]
#[pyo3(signature = (program_json, resolution_um=200))]
fn travel_collisions(program_json: &str, resolution_um: i64) -> PyResult<String> {
    let s = kerf_core::analyze::travel_collisions(&parse_lo(program_json)?, resolution_um);
    json::to_json(&s).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Graded (distance-based) difference of two LOW-LEVEL programs (JSON): for each cell one deposits
/// and the other does not, the Euclidean distance (microns) to the nearest cell of the other, as
/// mean/p95/max per layer and overall. Unlike IoU it stays informative when the two are disjoint (a
/// near miss scores small) — a smooth reward signal and the basis for rotation-aware comparison.
#[pyfunction]
#[pyo3(signature = (a_json, b_json, resolution_um=200))]
fn graded_diff(a_json: &str, b_json: &str, resolution_um: i64) -> PyResult<String> {
    let d = kerf_core::diff::graded_diff_programs(
        &parse_lo(a_json)?,
        &parse_lo(b_json)?,
        resolution_um,
    );
    json::to_json(&d).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Graded (distance-based) difference of two G-code files. See `graded_diff`.
#[pyfunction]
#[pyo3(signature = (a, b, resolution_um=200))]
fn graded_diff_gcode(a: &str, b: &str, resolution_um: i64) -> PyResult<String> {
    let d = kerf_core::diff::graded_diff_gcode(a, b, resolution_um);
    json::to_json(&d).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Deposited melt volume (mm³) of a program (JSON): total and per-layer. Moves with bead width, so it
/// surfaces over-/under-extrusion that coverage and path-count miss (geometry only, not commanded
/// flow). Layer height is derived from consecutive Z unless `layer_height_um` is given.
#[pyfunction]
#[pyo3(signature = (program_json, resolution_um=200, layer_height_um=None))]
fn volume_stats(
    program_json: &str,
    resolution_um: i64,
    layer_height_um: Option<i64>,
) -> PyResult<String> {
    let s =
        kerf_core::analyze::volume_stats(&parse_lo(program_json)?, resolution_um, layer_height_um);
    json::to_json(&s).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Per-cell commanded flow (mm of E) of a program (JSON), at a resolution — the flow analogue of the
/// occupancy denotation. Empty when no toolpath specifies E. Serialized as `[[x, y, e], ...]` per layer.
#[pyfunction]
#[pyo3(signature = (program_json, resolution_um=200))]
fn denote_flow(program_json: &str, resolution_um: i64) -> PyResult<String> {
    let f = kerf_core::flow::denote_lo_flow(&parse_lo(program_json)?, resolution_um);
    // The flow map's cells are a tuple-keyed map; emit an explicit per-layer array form.
    let layers: Vec<_> = f
        .layers
        .iter()
        .map(|l| {
            let cells: Vec<(i64, i64, f64)> =
                l.cells.iter().map(|(&(x, y), &e)| (x, y, e)).collect();
            (l.z_um, l.resolution_um, cells)
        })
        .collect();
    json::to_json(&layers).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Commanded-flow (E) stats for a program (JSON): total, per-layer, and how many extruding toolpaths
/// actually specify flow.
#[pyfunction]
fn flow_stats(program_json: &str) -> PyResult<String> {
    json::to_json(&kerf_core::flow::flow_stats(&parse_lo(program_json)?))
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Whether two programs (JSON) conserve commanded E within `tolerance_mm` (total and per layer).
#[pyfunction]
#[pyo3(signature = (a_json, b_json, tolerance_mm=1e-6))]
fn e_conserved(a_json: &str, b_json: &str, tolerance_mm: f64) -> PyResult<bool> {
    Ok(kerf_core::flow::e_conserved(
        &parse_lo(a_json)?,
        &parse_lo(b_json)?,
        tolerance_mm,
    ))
}

/// Stable 32-hex-char content hash of a program (JSON): dedup, cache keys, "same program" claims.
#[pyfunction]
fn canonical_hash(program_json: &str) -> PyResult<String> {
    Ok(kerf_core::hash::canonical_hash(&parse_lo(program_json)?))
}

/// 128-bit material fingerprint (32-hex) of a program (JSON) at a resolution — a fast "same deposited
/// material" identity. Equal fingerprints mean equal occupancy up to collision. For repeated checks
/// under edits, a `Program` handle's `fingerprint()` maintains this incrementally (only changed layers
/// re-hashed), turning a per-step preservation verdict from ~100 ms into microseconds.
#[pyfunction]
#[pyo3(signature = (program_json, resolution_um=200))]
fn material_fingerprint(program_json: &str, resolution_um: i64) -> PyResult<String> {
    Ok(format!(
        "{:032x}",
        kerf_core::denote::material_fingerprint(&parse_lo(program_json)?, resolution_um)
    ))
}

/// The enumerated legal actions over a program (JSON), returned as a JSON list. Feed one back to
/// `apply_action` (or a `Program` handle) to edit.
#[pyfunction]
fn legal_actions(program_json: &str) -> PyResult<String> {
    json::to_json(&kerf_core::transform::legal_actions(&parse_lo(
        program_json,
    )?))
    .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Apply an action (JSON) to a program (JSON), returning `(new_program_json, touched_layers)`. Raises
/// on an invalid/inapplicable action.
#[pyfunction]
fn apply_action(program_json: &str, action_json: &str) -> PyResult<(String, Vec<usize>)> {
    let mut prog = parse_lo(program_json)?;
    let action: kerf_core::transform::Action = json::from_json(action_json)
        .map_err(|e| PyValueError::new_err(format!("invalid action JSON; {e}")))?;
    let touched = action
        .apply(&mut prog)
        .map_err(|e| PyValueError::new_err(format!("action failed: {e:?}")))?;
    let out = json::to_json(&prog).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok((out, touched))
}

/// Kinematics-aware print-time report (JSON) for a program (JSON) under a machine profile (JSON, or
/// `None` for a default desktop printer).
#[pyfunction]
#[pyo3(signature = (program_json, profile_json=None))]
fn print_time(program_json: &str, profile_json: Option<&str>) -> PyResult<String> {
    let profile = machine_profile_or_default(profile_json)?;
    json::to_json(&kerf_core::kinematics::print_time(
        &parse_lo(program_json)?,
        &profile,
    ))
    .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Printability verdict (JSON) for a program (JSON) against a machine profile (JSON or `None`).
#[pyfunction]
#[pyo3(signature = (program_json, profile_json=None, resolution_um=200))]
fn is_printable(
    program_json: &str,
    profile_json: Option<&str>,
    resolution_um: i64,
) -> PyResult<String> {
    let profile = machine_profile_or_default(profile_json)?;
    json::to_json(&kerf_core::printability::is_printable(
        &parse_lo(program_json)?,
        &profile,
        resolution_um,
    ))
    .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// The default machine profile as JSON — a template to edit.
#[pyfunction]
fn default_machine_profile() -> PyResult<String> {
    json::to_json(&MachineProfile::default()).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Whether two programs (JSON) deposit the same material within `epsilon_um` (worst nearest-miss).
#[pyfunction]
#[pyo3(signature = (a_json, b_json, epsilon_um, resolution_um=200))]
fn preserves_within(
    a_json: &str,
    b_json: &str,
    epsilon_um: f64,
    resolution_um: i64,
) -> PyResult<bool> {
    Ok(kerf_core::tolerance::preserves_within(
        &parse_lo(a_json)?,
        &parse_lo(b_json)?,
        epsilon_um,
        resolution_um,
    ))
}

/// Verify a batch of candidate programs (list of JSON) against a reference (JSON) by deposited
/// material, in parallel with the GIL released. Returns one bool per candidate.
#[pyfunction]
#[pyo3(signature = (candidates_json, reference_json, resolution_um=200))]
fn verify_batch(
    py: Python<'_>,
    candidates_json: Vec<String>,
    reference_json: &str,
    resolution_um: i64,
) -> PyResult<Vec<bool>> {
    let reference = parse_lo(reference_json)?;
    let progs = candidates_json
        .iter()
        .map(|s| json::from_json::<lo::Program>(s))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| PyValueError::new_err(format!("invalid candidate program JSON; {e}")))?;
    Ok(py.allow_threads(|| kerf_core::verify::verify_batch(&progs, &reference, resolution_um)))
}

/// The per-toolpath feature matrix of a program (JSON): `(rows, cols, flat_row_major)`. Reshape to
/// `(rows, cols)`; column names are `feature_columns()`.
#[pyfunction]
fn feature_matrix(program_json: &str) -> PyResult<(usize, usize, Vec<f64>)> {
    let (rows, data) = kerf_core::feature::toolpath_feature_matrix(&parse_lo(program_json)?);
    Ok((rows, kerf_core::feature::FEATURE_COLUMNS.len(), data))
}

/// Dense 0/1 occupancy raster for one layer of a program (JSON): `(rows, cols, min_i, min_j, bytes)`.
#[pyfunction]
#[pyo3(signature = (program_json, layer, resolution_um=200))]
fn occupancy_grid(
    program_json: &str,
    layer: usize,
    resolution_um: i64,
) -> PyResult<(usize, usize, i64, i64, Vec<u8>)> {
    let grids = kerf_core::feature::occupancy_grid(&parse_lo(program_json)?, resolution_um);
    let g = grids
        .get(layer)
        .ok_or_else(|| PyValueError::new_err("layer index out of range"))?;
    Ok((g.rows, g.cols, g.min_i, g.min_j, g.data.clone()))
}

/// The travel graph (JSON) of a program (JSON): toolpath centroids as nodes, within-layer hops as
/// weighted edges.
#[pyfunction]
fn travel_graph(program_json: &str) -> PyResult<String> {
    json::to_json(&kerf_core::feature::travel_graph(&parse_lo(program_json)?))
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Serialize a program (JSON) into a versioned, persistable artifact (adds a schema-version tag).
#[pyfunction]
fn export_versioned(lo_program_json: &str) -> PyResult<String> {
    kerf_core::schema::export_lo(&parse_lo(lo_program_json)?)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Load a versioned artifact (from `export_versioned`) back to a program (JSON), failing loudly on a
/// schema-version or kind mismatch.
#[pyfunction]
fn import_versioned(versioned_json: &str) -> PyResult<String> {
    let prog = kerf_core::schema::import_lo(versioned_json)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    json::to_json(&prog).map_err(|e| PyValueError::new_err(e.to_string()))
}

fn machine_profile_or_default(profile_json: Option<&str>) -> PyResult<MachineProfile> {
    match profile_json {
        None => Ok(MachineProfile::default()),
        Some(s) => json::from_json(s)
            .map_err(|e| PyValueError::new_err(format!("invalid MachineProfile JSON; {e}"))),
    }
}

/// The kerf-core crate version.
#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pymodule]
fn _kerf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // JSON boundary
    m.add_function(wrap_pyfunction!(program_to_gcode, m)?)?;
    m.add_function(wrap_pyfunction!(lower_to_json, m)?)?;
    m.add_function(wrap_pyfunction!(check_self_lowering_sound, m)?)?;
    m.add_function(wrap_pyfunction!(demo_square_json, m)?)?;
    // G-code frontend + verification
    m.add_function(wrap_pyfunction!(parse_gcode, m)?)?;
    m.add_function(wrap_pyfunction!(verify_gcode, m)?)?;
    m.add_function(wrap_pyfunction!(verify_roundtrip, m)?)?;
    m.add_function(wrap_pyfunction!(diff_gcode, m)?)?;
    m.add_function(wrap_pyfunction!(diff_programs, m)?)?;
    m.add_function(wrap_pyfunction!(graded_diff, m)?)?;
    m.add_function(wrap_pyfunction!(graded_diff_gcode, m)?)?;
    // transforms + analyses
    m.add_function(wrap_pyfunction!(rotate_z, m)?)?;
    m.add_function(wrap_pyfunction!(voxelize, m)?)?;
    m.add_function(wrap_pyfunction!(rot_x90, m)?)?;
    m.add_function(wrap_pyfunction!(rot_y90, m)?)?;
    m.add_function(wrap_pyfunction!(rot_z90, m)?)?;
    m.add_function(wrap_pyfunction!(rotate_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(compare_rotated, m)?)?;
    m.add_function(wrap_pyfunction!(occupancy, m)?)?;
    m.add_function(wrap_pyfunction!(program_stats, m)?)?;
    m.add_function(wrap_pyfunction!(deposit_stats, m)?)?;
    m.add_function(wrap_pyfunction!(travel_collisions, m)?)?;
    m.add_function(wrap_pyfunction!(volume_stats, m)?)?;
    // E-axis (commanded flow)
    m.add_function(wrap_pyfunction!(denote_flow, m)?)?;
    m.add_function(wrap_pyfunction!(flow_stats, m)?)?;
    m.add_function(wrap_pyfunction!(e_conserved, m)?)?;
    // transform actions
    m.add_function(wrap_pyfunction!(legal_actions, m)?)?;
    m.add_function(wrap_pyfunction!(apply_action, m)?)?;
    // objectives + gates
    m.add_function(wrap_pyfunction!(print_time, m)?)?;
    m.add_function(wrap_pyfunction!(is_printable, m)?)?;
    m.add_function(wrap_pyfunction!(default_machine_profile, m)?)?;
    m.add_function(wrap_pyfunction!(preserves_within, m)?)?;
    m.add_function(wrap_pyfunction!(verify_batch, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_hash, m)?)?;
    m.add_function(wrap_pyfunction!(material_fingerprint, m)?)?;
    // featurization
    m.add_function(wrap_pyfunction!(feature_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(occupancy_grid, m)?)?;
    m.add_function(wrap_pyfunction!(travel_graph, m)?)?;
    m.add_function(wrap_pyfunction!(handle::feature_columns, m)?)?;
    // versioned persistence
    m.add_function(wrap_pyfunction!(export_versioned, m)?)?;
    m.add_function(wrap_pyfunction!(import_versioned, m)?)?;
    // stateful native handle (hot-loop API)
    m.add_class::<handle::Program>()?;
    // demos
    m.add_function(wrap_pyfunction!(demo_square_gcode, m)?)?;
    m.add_function(wrap_pyfunction!(demo_self_lowering_sound, m)?)?;
    m.add_function(wrap_pyfunction!(demo_travel_order, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
