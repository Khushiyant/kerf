//! A stateful, native program handle — the hot-loop API for search/RL over move plans.
//!
//! [`Program`] loads a plan once, keeps it native (never re-serializing to JSON), and answers
//! denote / stats / objective / verify queries directly. Mutations go through the enumerated
//! transform actions and mark only the touched layers dirty, so re-denote is incremental. This is the
//! interface a consumer drives for thousands of steps without ever touching JSON internals; JSON stays
//! the import/export boundary only.

use kerf_core::incremental::DenoteCache;
use kerf_core::ir::lo;
use kerf_core::kinematics::MachineProfile;
use kerf_core::transform::Action;
use kerf_core::{
    backend, feature, flow, frontend, hash, json, kinematics, printability, tolerance,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// A native handle over a low-level move plan with an incremental denote cache.
#[pyclass]
pub struct Program {
    inner: lo::Program,
    cache: DenoteCache,
}

fn value_err<E: std::fmt::Display>(e: E) -> PyErr {
    PyValueError::new_err(e.to_string())
}

impl Program {
    /// Apply an already-parsed action, marking the touched layers dirty. Shared by the JSON, indexed,
    /// and sampled entry points. Not a Python method.
    fn apply_action(&mut self, action: &Action) -> PyResult<Vec<usize>> {
        let touched = action
            .apply(&mut self.inner)
            .map_err(|e| PyValueError::new_err(format!("action failed: {e:?}")))?;
        for &l in &touched {
            self.cache.mark_dirty(l);
        }
        Ok(touched)
    }
}

#[pymethods]
impl Program {
    /// Load from a low-level program JSON (as from `lower_to_json` / `parse_gcode`). `resolution_um`
    /// keys the incremental denote cache.
    #[staticmethod]
    #[pyo3(signature = (lo_program_json, resolution_um=200))]
    fn from_json(lo_program_json: &str, resolution_um: i64) -> PyResult<Self> {
        let inner: lo::Program = json::from_json(lo_program_json).map_err(|e| {
            PyValueError::new_err(format!("expected a LOW-LEVEL program JSON; {e}"))
        })?;
        Ok(Self {
            inner,
            cache: DenoteCache::new(resolution_um),
        })
    }

    /// Load by parsing slicer G-code (commanded flow is captured too).
    #[staticmethod]
    #[pyo3(signature = (gcode, resolution_um=200))]
    fn from_gcode(gcode: &str, resolution_um: i64) -> Self {
        Self {
            inner: frontend::parse(gcode).program,
            cache: DenoteCache::new(resolution_um),
        }
    }

    /// Export the current program to low-level JSON (the interchange boundary).
    fn to_json(&self) -> PyResult<String> {
        json::to_json(&self.inner).map_err(value_err)
    }

    /// A deep copy with a fresh (empty) denote cache — for spawning candidates.
    fn copy(&self) -> Program {
        Program {
            inner: self.inner.clone(),
            cache: DenoteCache::new(self.cache.resolution_um()),
        }
    }

    #[getter]
    fn resolution_um(&self) -> i64 {
        self.cache.resolution_um()
    }

    /// Re-key the denote cache to a new resolution (invalidates it).
    fn set_resolution(&mut self, resolution_um: i64) {
        self.cache = DenoteCache::new(resolution_um);
    }

    fn layer_count(&self) -> usize {
        self.inner.layers.len()
    }

    fn toolpath_count(&self, layer: usize) -> PyResult<usize> {
        self.inner
            .layers
            .get(layer)
            .map(|l| l.toolpaths.len())
            .ok_or_else(|| PyValueError::new_err("layer index out of range"))
    }

    /// Enumerate the legal actions over the current program, as JSON. Serializes the whole list;
    /// for a hot loop prefer `action_count` + `apply_index`/`apply_sampled`, which never marshal it.
    fn legal_actions(&self) -> PyResult<String> {
        json::to_json(&kerf_core::transform::legal_actions(&self.inner)).map_err(value_err)
    }

    /// Number of legal actions right now — no serialization. Pair with `action`/`apply_index`.
    fn action_count(&self) -> usize {
        kerf_core::transform::legal_actions(&self.inner).len()
    }

    /// The i-th legal action as JSON (serializes just that one action).
    fn action(&self, i: usize) -> PyResult<String> {
        let acts = kerf_core::transform::legal_actions(&self.inner);
        let a = acts
            .get(i)
            .ok_or_else(|| PyValueError::new_err("action index out of range"))?;
        json::to_json(a).map_err(value_err)
    }

    /// Apply one action (JSON, as from `legal_actions`/`action`) in place, marking the touched layers
    /// dirty. Returns the touched layer indices. Raises on an invalid/inapplicable action, leaving the
    /// program unchanged.
    fn apply(&mut self, action_json: &str) -> PyResult<Vec<usize>> {
        let action: Action = json::from_json(action_json)
            .map_err(|e| PyValueError::new_err(format!("invalid action JSON; {e}")))?;
        self.apply_action(&action)
    }

    /// Apply the i-th legal action in place — no JSON crosses the boundary. Returns touched layers.
    fn apply_index(&mut self, i: usize) -> PyResult<Vec<usize>> {
        let acts = kerf_core::transform::legal_actions(&self.inner);
        let action = acts
            .get(i)
            .ok_or_else(|| PyValueError::new_err("action index out of range"))?
            .clone();
        self.apply_action(&action)
    }

    /// Apply a seeded-deterministic legal action in place (the hot-loop primitive: no JSON, no full
    /// action list marshaled). Returns touched layers, or an empty list if there are no legal actions.
    fn apply_sampled(&mut self, seed: u64) -> PyResult<Vec<usize>> {
        let acts = kerf_core::transform::legal_actions(&self.inner);
        if acts.is_empty() {
            return Ok(Vec::new());
        }
        let action = acts[(seed % acts.len() as u64) as usize].clone();
        self.apply_action(&action)
    }

    /// Mark a layer dirty by hand (after an out-of-band edit).
    fn mark_dirty(&mut self, layer: usize) {
        self.cache.mark_dirty(layer);
    }

    /// The current deposited-material occupancy (JSON), computed incrementally — only layers changed
    /// since the last call are re-rasterized.
    ///
    /// Note: this MARSHALS every layer's cells to Python (O(cells)), which dominates the cost on a
    /// cache hit — the incremental raster win is buried by serialization. Do NOT call it inside a hot
    /// loop. For a per-step reward/gate use `iou_to` (returns a float) or `preserves_within` (returns
    /// a bool), computed handle-to-handle in Rust; `graded_to` returns a compact per-layer report
    /// (O(layers), not O(cells)) — fine per step, but not a bare scalar.
    fn occupancy(&mut self) -> PyResult<String> {
        let occ = self.cache.occupancy(&self.inner);
        json::to_json(&occ).map_err(value_err)
    }

    /// Size/efficiency stats (JSON).
    fn stats(&self) -> PyResult<String> {
        json::to_json(&kerf_core::analyze::program_stats(&self.inner)).map_err(value_err)
    }

    /// Commanded-flow (E) stats (JSON).
    fn flow_stats(&self) -> PyResult<String> {
        json::to_json(&flow::flow_stats(&self.inner)).map_err(value_err)
    }

    /// Content hash (stable 32-hex-char identity) of the current program.
    fn canonical_hash(&self) -> String {
        hash::canonical_hash(&self.inner)
    }

    /// Incremental 128-bit **material fingerprint** (32-hex) — a microsecond preservation / "same
    /// material" verdict. Only layers changed since the last call are re-rasterized and re-hashed, so
    /// after an edit this is orders of magnitude cheaper than `occupancy()` / `iou_to` / `verify_*`.
    /// Compare two handles' fingerprints (or against a saved reference) to gate an RL step. Equal
    /// fingerprints mean equal occupancy up to a ~1-in-2^128 collision.
    fn fingerprint(&mut self) -> String {
        format!("{:032x}", self.cache.fingerprint(&self.inner))
    }

    /// Whether this program deposits the same material as `other` — a fast fingerprint compare (both
    /// caches update incrementally). The constraint check for order-optimization RL.
    fn same_material(&mut self, other: PyRefMut<'_, Program>) -> bool {
        let mine = self.cache.fingerprint(&self.inner);
        let mut other = other;
        let o = &mut *other;
        mine == o.cache.fingerprint(&o.inner)
    }

    /// Total travel distance (microns) — a cheap scalar objective.
    fn travel_distance_um(&self) -> f64 {
        self.inner.travel_distance_um()
    }

    /// Kinematic print time (seconds) under a machine profile (JSON, or `None` for the default).
    #[pyo3(signature = (profile_json=None))]
    fn print_time_s(&self, profile_json: Option<&str>) -> PyResult<f64> {
        let profile = parse_profile(profile_json)?;
        Ok(kinematics::print_time(&self.inner, &profile).total_s)
    }

    /// Full kinematic print-time report (JSON).
    #[pyo3(signature = (profile_json=None))]
    fn print_time(&self, profile_json: Option<&str>) -> PyResult<String> {
        let profile = parse_profile(profile_json)?;
        json::to_json(&kinematics::print_time(&self.inner, &profile)).map_err(value_err)
    }

    /// Printability verdict (JSON) against a machine profile.
    #[pyo3(signature = (profile_json=None, resolution_um=200))]
    fn printability(&self, profile_json: Option<&str>, resolution_um: i64) -> PyResult<String> {
        let profile = parse_profile(profile_json)?;
        json::to_json(&printability::is_printable(
            &self.inner,
            &profile,
            resolution_um,
        ))
        .map_err(value_err)
    }

    /// IoU similarity of deposited material to another handle (`None` if both empty) — a scalar reward.
    #[pyo3(signature = (other, resolution_um=200))]
    fn iou_to(&self, other: &Program, resolution_um: i64) -> Option<f64> {
        kerf_core::diff::diff_programs(&self.inner, &other.inner, resolution_um).iou
    }

    /// Graded nearest-miss distance report (JSON) to another handle — a smooth reward signal.
    #[pyo3(signature = (other, resolution_um=200))]
    fn graded_to(&self, other: &Program, resolution_um: i64) -> PyResult<String> {
        json::to_json(&kerf_core::diff::graded_diff_programs(
            &self.inner,
            &other.inner,
            resolution_um,
        ))
        .map_err(value_err)
    }

    /// Whether this program is within `epsilon_um` of a reference (worst nearest-miss).
    #[pyo3(signature = (reference, epsilon_um, resolution_um=200))]
    fn preserves_within(&self, reference: &Program, epsilon_um: f64, resolution_um: i64) -> bool {
        tolerance::preserves_within(&self.inner, &reference.inner, epsilon_um, resolution_um)
    }

    /// Whether commanded E is conserved (within `tolerance_mm`) relative to a reference.
    #[pyo3(signature = (reference, tolerance_mm=1e-6))]
    fn e_conserved(&self, reference: &Program, tolerance_mm: f64) -> bool {
        flow::e_conserved(&self.inner, &reference.inner, tolerance_mm)
    }

    /// The per-toolpath feature matrix: `(rows, cols, flat_row_major)`. Wrap as
    /// `np.array(flat).reshape(rows, cols)`; column names are `feature_columns()`.
    fn feature_matrix(&self) -> (usize, usize, Vec<f64>) {
        let (rows, data) = feature::toolpath_feature_matrix(&self.inner);
        (rows, feature::FEATURE_COLUMNS.len(), data)
    }

    /// Dense 0/1 occupancy raster for a layer: `(rows, cols, min_i, min_j, bytes)`. Wrap as
    /// `np.frombuffer(bytes, np.uint8).reshape(rows, cols)`.
    fn occupancy_grid(&self, layer: usize) -> PyResult<(usize, usize, i64, i64, Vec<u8>)> {
        let grids = feature::occupancy_grid(&self.inner, self.cache.resolution_um());
        let g = grids
            .get(layer)
            .ok_or_else(|| PyValueError::new_err("layer index out of range"))?;
        Ok((g.rows, g.cols, g.min_i, g.min_j, g.data.clone()))
    }

    /// The travel graph (JSON): toolpath centroids as nodes, within-layer hops as weighted edges.
    fn travel_graph(&self) -> PyResult<String> {
        json::to_json(&feature::travel_graph(&self.inner)).map_err(value_err)
    }

    /// Emit the current program to G-code. Geometry (integer µm) round-trips exactly; commanded flow
    /// is emitted at the machine's 5-decimal E resolution (so tiny sub-1e-5 mm flows are not exactly
    /// preserved, and flow-less paths get a synthesized volumetric E). The emitter is outside the
    /// verified boundary — use `verify_roundtrip` (via the JSON functions) to check a specific plan.
    fn to_gcode(&self) -> String {
        backend::to_gcode(&self.inner)
    }

    fn __len__(&self) -> usize {
        self.inner.layers.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "Program(layers={}, toolpaths={}, resolution_um={})",
            self.inner.layers.len(),
            self.inner.extrusion_move_count(),
            self.cache.resolution_um()
        )
    }
}

/// The feature-matrix column names, in order (module-level helper).
#[pyfunction]
pub fn feature_columns() -> Vec<String> {
    feature::FEATURE_COLUMNS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn parse_profile(profile_json: Option<&str>) -> PyResult<MachineProfile> {
    match profile_json {
        None => Ok(MachineProfile::default()),
        Some(s) => json::from_json(s)
            .map_err(|e| PyValueError::new_err(format!("invalid MachineProfile JSON; {e}"))),
    }
}
