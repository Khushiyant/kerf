//! Deterministic, versioned wrapper over `kerf-core`. Callers use [`verify`] / [`diff`], which return
//! a self-describing [`VerdictEnvelope`] whose [`VerdictEnvelope::result_digest`] is bit-identical
//! across runs of the same build on the same inputs. No I/O; timing and randomness stay out of the digest.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Instant;

/// Which code produced a verdict: the `kerf-core` semver and the git commit of the running binary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineVersion {
    pub core_semver: String,
    pub git_sha: String,
}

impl EngineVersion {
    /// The version of the currently-running build.
    pub fn current() -> Self {
        Self {
            core_semver: env!("CARGO_PKG_VERSION").to_string(),
            git_sha: env!("KERF_GIT_SHA").to_string(),
        }
    }
}

/// What kind of check produced the verdict.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VerdictKind {
    Verify,
    Diff,
}

/// A content-addressed reference to one input the verdict was computed over.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputRef {
    /// `"input"` for verify; `"a"` / `"b"` for diff.
    pub role: String,
    /// Lowercase hex SHA-256 of the exact input bytes.
    pub sha256: String,
    pub bytes: usize,
}

/// A flat, display-friendly digest of the outcome. Derived from `raw`; not part of `result_digest`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct VerdictSummary {
    /// Headline pass/fail: verify → sound; diff → identical (only when not `both_empty`). `None` when
    /// there is nothing to assert (no geometry / both files empty).
    pub ok: Option<bool>,
    pub has_geometry: Option<bool>,
    pub identical: Option<bool>,
    pub both_empty: Option<bool>,
    /// Intersection-over-union of deposited material for a diff (`None` for verify / empty).
    pub iou: Option<f64>,
}

/// A self-describing, reproducible verdict — the unit the service stores immutably and serves.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct VerdictEnvelope {
    pub engine: EngineVersion,
    pub kind: VerdictKind,
    pub inputs: Vec<InputRef>,
    pub resolution_um: i64,
    /// The exact `kerf-core` `GcodeVerification` / `GcodeDiff` as JSON.
    pub raw: serde_json::Value,
    pub summary: VerdictSummary,
    /// Wall time spent in the engine. Excluded from `result_digest`.
    pub engine_wall_ms: u64,
}

/// The subset of an envelope that defines its identity: reproducible content only, no timing.
#[derive(Serialize)]
struct DigestInput<'a> {
    engine: &'a EngineVersion,
    kind: VerdictKind,
    inputs: &'a [InputRef],
    resolution_um: i64,
    raw: &'a serde_json::Value,
}

impl VerdictEnvelope {
    /// Stable SHA-256 over the reproducible content; `engine_wall_ms` and the derived `summary` are
    /// excluded. `serde_json` sorts object keys, so the serialization is canonical.
    pub fn result_digest(&self) -> String {
        let d = DigestInput {
            engine: &self.engine,
            kind: self.kind,
            inputs: &self.inputs,
            resolution_um: self.resolution_um,
            raw: &self.raw,
        };
        let bytes = serde_json::to_vec(&d).expect("digest input serializes");
        hex(&Sha256::digest(&bytes))
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn input_ref(role: &str, data: &str) -> InputRef {
    InputRef {
        role: role.to_string(),
        sha256: hex(&Sha256::digest(data.as_bytes())),
        bytes: data.len(),
    }
}

/// Verify a single G-code file; returns a reproducible envelope wrapping `kerf_core::verify_gcode`.
pub fn verify(gcode: &str, resolution_um: i64) -> VerdictEnvelope {
    let t = Instant::now();
    let v = kerf_core::verify_gcode(gcode, resolution_um);
    let elapsed = t.elapsed().as_millis() as u64;
    let summary = VerdictSummary {
        ok: Some(v.ok()),
        has_geometry: Some(v.has_geometry),
        ..Default::default()
    };
    VerdictEnvelope {
        engine: EngineVersion::current(),
        kind: VerdictKind::Verify,
        inputs: vec![input_ref("input", gcode)],
        resolution_um,
        raw: serde_json::to_value(&v).expect("GcodeVerification serializes"),
        summary,
        engine_wall_ms: elapsed,
    }
}

/// Diff two G-code files by deposited material; returns a reproducible envelope over
/// `kerf_core::diff_gcode`. `summary.ok` is the meaningful identical verdict (`None` when both empty).
pub fn diff(a: &str, b: &str, resolution_um: i64) -> VerdictEnvelope {
    let t = Instant::now();
    let d = kerf_core::diff_gcode(a, b, resolution_um);
    let elapsed = t.elapsed().as_millis() as u64;
    let summary = VerdictSummary {
        ok: (!d.both_empty).then_some(d.identical),
        identical: Some(d.identical),
        both_empty: Some(d.both_empty),
        iou: d.iou(),
        ..Default::default()
    };
    VerdictEnvelope {
        engine: EngineVersion::current(),
        kind: VerdictKind::Diff,
        inputs: vec![input_ref("a", a), input_ref("b", b)],
        resolution_um,
        raw: serde_json::to_value(&d).expect("GcodeDiff serializes"),
        summary,
        engine_wall_ms: elapsed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GCODE: &str = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:External perimeter\n;WIDTH:0.45\nG0 X0 Y0\nG1 X10 Y0 E.4\nG1 X10 Y10 E.4\n;LAYER_CHANGE\n;Z:0.4\n;TYPE:External perimeter\nG0 X0 Y0\nG1 X10 Y0 E.4";

    #[test]
    fn verify_envelope_is_wellformed() {
        let e = verify(GCODE, 200);
        assert_eq!(e.kind, VerdictKind::Verify);
        assert_eq!(e.inputs.len(), 1);
        assert_eq!(e.inputs[0].sha256.len(), 64);
        assert_eq!(e.inputs[0].bytes, GCODE.len());
        assert_eq!(e.summary.has_geometry, Some(true));
        assert_eq!(e.summary.ok, Some(true));
        assert!(e.raw.get("pass_preserves_denotation").is_some());
        assert_eq!(e.engine.core_semver, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn result_digest_is_deterministic_and_input_sensitive() {
        assert_eq!(
            verify(GCODE, 200).result_digest(),
            verify(GCODE, 200).result_digest()
        );
        // Resolution is part of identity.
        assert_ne!(
            verify(GCODE, 200).result_digest(),
            verify(GCODE, 100).result_digest()
        );
        let other = GCODE.replace("X10 Y10", "X10 Y20");
        assert_ne!(
            verify(GCODE, 200).result_digest(),
            verify(&other, 200).result_digest()
        );
    }

    #[test]
    fn diff_self_is_identical_and_meaningful() {
        let e = diff(GCODE, GCODE, 200);
        assert_eq!(e.kind, VerdictKind::Diff);
        assert_eq!(e.inputs.len(), 2);
        assert_eq!(e.summary.identical, Some(true));
        assert_eq!(e.summary.both_empty, Some(false));
        assert_eq!(e.summary.ok, Some(true));
        assert_eq!(e.summary.iou, Some(1.0));
    }

    #[test]
    fn diff_of_empty_files_is_not_a_meaningful_match() {
        let e = diff("", "", 200);
        assert_eq!(e.summary.both_empty, Some(true));
        assert_eq!(e.summary.ok, None); // identical-but-empty is not a "same part" verdict
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let e = verify(GCODE, 200);
        let back: VerdictEnvelope =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
        assert_eq!(e.result_digest(), back.result_digest());
    }
}
