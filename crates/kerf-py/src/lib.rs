//! PyO3 bindings over `kerf-core`. The IR crosses the boundary as JSON (`kerf_core::json`), not as
//! `#[pyclass]` wrappers, so adding an IR field never touches this file.

use kerf_core::ir::{hi, Area, ExtrudePath, Point, Polyline, RegionKind};
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
    m.add_function(wrap_pyfunction!(occupancy, m)?)?;
    m.add_function(wrap_pyfunction!(program_stats, m)?)?;
    m.add_function(wrap_pyfunction!(deposit_stats, m)?)?;
    m.add_function(wrap_pyfunction!(travel_collisions, m)?)?;
    m.add_function(wrap_pyfunction!(volume_stats, m)?)?;
    // demos
    m.add_function(wrap_pyfunction!(demo_square_gcode, m)?)?;
    m.add_function(wrap_pyfunction!(demo_self_lowering_sound, m)?)?;
    m.add_function(wrap_pyfunction!(demo_travel_order, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
