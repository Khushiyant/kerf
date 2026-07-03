//! Thin PyO3 bindings over `kerf-core`. Keep this layer minimal: translate between Python and the
//! Rust IR, nothing more. All logic lives in `kerf-core`.
//!
//! The IR is exposed to Python through the JSON boundary (`kerf_core::json`), NOT as `#[pyclass]`
//! wrappers per type — so adding an IR field never touches this file. Python builds/inspects programs
//! as JSON strings; the demos below show the shape.

use kerf_core::ir::{hi, Area, ExtrudePath, Point, Polyline, RegionKind};
use kerf_core::pass::{Pass, TravelOrder};
use kerf_core::{backend, denote, frontend, json, lower};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

// ---- helpers (not exposed) -------------------------------------------------------------------

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

fn parse_hi(program_json: &str) -> PyResult<hi::Program> {
    json::from_json(program_json)
        .map_err(|e| PyValueError::new_err(format!("invalid high-level program JSON: {e}")))
}

// ---- JSON boundary: operate on caller-provided programs --------------------------------------

/// Lower a high-level program (given as JSON) and emit G-code.
#[pyfunction]
fn program_to_gcode(program_json: &str) -> PyResult<String> {
    Ok(backend::to_gcode(&lower::lower(&parse_hi(program_json)?)))
}

/// Lower a high-level program (JSON) to the low-level move plan, returned as JSON for inspection.
#[pyfunction]
fn lower_to_json(program_json: &str) -> PyResult<String> {
    let lowered = lower::lower(&parse_hi(program_json)?);
    json::to_json(&lowered).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Run Kerf's correctness oracle on a caller-provided high-level program (JSON): does lowering
/// preserve the deposited material at this resolution (microns)?
#[pyfunction]
#[pyo3(signature = (program_json, resolution_um=200))]
fn check_self_lowering_sound(program_json: &str, resolution_um: i64) -> PyResult<bool> {
    Ok(denote::self_lowering_sound(
        &parse_hi(program_json)?,
        resolution_um,
    ))
}

/// The demo square serialized to JSON — an editable template for building your own programs.
#[pyfunction]
fn demo_square_json() -> PyResult<String> {
    json::to_json(&demo_square(20.0, 0.2)).map_err(|e| PyValueError::new_err(e.to_string()))
}

// ---- self-contained demos --------------------------------------------------------------------

/// G-code for a demo square: build hi IR -> lower to lo -> emit G-code.
#[pyfunction]
#[pyo3(signature = (side_mm=20.0, z_mm=0.2))]
fn demo_square_gcode(side_mm: f64, z_mm: f64) -> String {
    backend::to_gcode(&lower::lower(&demo_square(side_mm, z_mm)))
}

/// Oracle on the demo square: is the lowering denotation-preserving at this resolution?
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

/// Parse real slicer G-code into the IR. Returns `(program_json, diagnostics_json)` — the recovered
/// low-level program and the side-channel diagnostics (unknown roles, estimated widths, skipped
/// arcs/codes). Never raises on malformed G-code; it degrades and reports.
#[pyfunction]
fn parse_gcode(gcode: &str) -> PyResult<(String, String)> {
    let report = frontend::parse(gcode);
    let program =
        json::to_json(&report.program).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let diagnostics =
        json::to_json(&report.diagnostics).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok((program, diagnostics))
}

/// Verify real slicer G-code end to end: parse it, then check that a Kerf pass preserves the
/// deposited material and that the geometry is translation-invariant. Returns a JSON report
/// (diagnostics + verdicts). This is the check GlitchFinder cannot express, run on real output.
#[pyfunction]
#[pyo3(signature = (gcode, resolution_um=200))]
fn verify_gcode(gcode: &str, resolution_um: i64) -> PyResult<String> {
    let v = kerf_core::verify::verify_gcode(gcode, resolution_um);
    json::to_json(&v).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Compare two G-code files by the material they deposit (matched by layer height). Returns a JSON
/// report (per-layer differences, totals, identical flag). Answers "do these two slicers make the
/// same part?" in deposited-material terms, not as a text diff.
#[pyfunction]
#[pyo3(signature = (a, b, resolution_um=200))]
fn diff_gcode(a: &str, b: &str, resolution_um: i64) -> PyResult<String> {
    let d = kerf_core::diff_gcode(a, b, resolution_um);
    json::to_json(&d).map_err(|e| PyValueError::new_err(e.to_string()))
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
    m.add_function(wrap_pyfunction!(diff_gcode, m)?)?;
    // demos
    m.add_function(wrap_pyfunction!(demo_square_gcode, m)?)?;
    m.add_function(wrap_pyfunction!(demo_self_lowering_sound, m)?)?;
    m.add_function(wrap_pyfunction!(demo_travel_order, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
