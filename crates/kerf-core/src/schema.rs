//! Versioned serialization for saved artifacts. Plain [`crate::json`] is the wire format for live
//! interchange; this is what you persist. Every exported artifact carries a `schema_version` and a
//! `kind` tag, and [`import_lo`] / [`import_hi`] refuse to load anything they do not recognize rather
//! than silently reinterpreting it. Saved datasets and trained artifacts therefore survive a library
//! upgrade or fail loudly — never quietly mean something different.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ir::{hi, lo};

/// The current IR schema version. Bump on any breaking change to the serialized IR shape.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

const KIND_LO: &str = "kerf.lo.program";
const KIND_HI: &str = "kerf.hi.program";

#[derive(Serialize, Deserialize)]
struct Envelope {
    schema_version: u32,
    kind: String,
    program: serde_json::Value,
}

/// Why a versioned artifact could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaError {
    /// The envelope or inner program was not valid JSON for the expected shape.
    Parse(String),
    /// The artifact's schema version is not the one this build understands.
    UnsupportedVersion { found: u32, supported: u32 },
    /// The artifact is a different kind of program than requested.
    WrongKind { found: String, expected: String },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Parse(e) => write!(f, "malformed versioned artifact: {e}"),
            SchemaError::UnsupportedVersion { found, supported } => write!(
                f,
                "schema version {found} is not supported (this build reads version {supported}); \
                 migrate the artifact rather than loading it blindly"
            ),
            SchemaError::WrongKind { found, expected } => {
                write!(f, "expected a {expected} artifact, found {found}")
            }
        }
    }
}

impl std::error::Error for SchemaError {}

fn export<T: Serialize>(kind: &str, value: &T) -> serde_json::Result<String> {
    let env = Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        kind: kind.to_string(),
        program: serde_json::to_value(value)?,
    };
    serde_json::to_string_pretty(&env)
}

fn import<T: serde::de::DeserializeOwned>(kind: &str, s: &str) -> Result<T, SchemaError> {
    let env: Envelope = serde_json::from_str(s).map_err(|e| SchemaError::Parse(e.to_string()))?;
    if env.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(SchemaError::UnsupportedVersion {
            found: env.schema_version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    if env.kind != kind {
        return Err(SchemaError::WrongKind {
            found: env.kind,
            expected: kind.to_string(),
        });
    }
    serde_json::from_value(env.program).map_err(|e| SchemaError::Parse(e.to_string()))
}

/// Serialize a low-level program with a version + kind tag, for persistence.
pub fn export_lo(program: &lo::Program) -> serde_json::Result<String> {
    export(KIND_LO, program)
}

/// Load a versioned low-level program, failing loudly on a version or kind mismatch.
pub fn import_lo(s: &str) -> Result<lo::Program, SchemaError> {
    import(KIND_LO, s)
}

/// Serialize a high-level program with a version + kind tag, for persistence.
pub fn export_hi(program: &hi::Program) -> serde_json::Result<String> {
    export(KIND_HI, program)
}

/// Load a versioned high-level program, failing loudly on a version or kind mismatch.
pub fn import_hi(s: &str) -> Result<hi::Program, SchemaError> {
    import(KIND_HI, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn sample() -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath::extrude(
                    SegmentKind::Extrude(RegionKind::Perimeter),
                    Polyline::new(vec![Point::new(0, 0), Point::new(10_000, 0)]),
                    400,
                )],
            }],
        }
    }

    #[test]
    fn round_trips_through_the_versioned_envelope() {
        let p = sample();
        let s = export_lo(&p).unwrap();
        assert!(s.contains("schema_version"));
        assert_eq!(import_lo(&s).unwrap(), p);
    }

    #[test]
    fn a_future_version_is_refused_not_reinterpreted() {
        let s = export_lo(&sample()).unwrap();
        let bumped = s.replace(
            "\"schema_version\": 1",
            &format!("\"schema_version\": {}", CURRENT_SCHEMA_VERSION + 9),
        );
        match import_lo(&bumped) {
            Err(SchemaError::UnsupportedVersion { found, .. }) => {
                assert_eq!(found, CURRENT_SCHEMA_VERSION + 9)
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn a_lo_artifact_will_not_load_as_hi() {
        let s = export_lo(&sample()).unwrap();
        assert!(matches!(import_hi(&s), Err(SchemaError::WrongKind { .. })));
    }
}
