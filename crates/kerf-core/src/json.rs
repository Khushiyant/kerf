//! JSON serialization boundary for the IR (feature `serde`).
//!
//! This is how Python — and any non-Rust consumer — builds, inspects, and diffs programs without a
//! `#[pyclass]` wrapper per IR type. Adding an IR field never touches the bindings; it just appears in
//! the JSON. Programs become plain text you can print, store in a `.kerf` file, or diff.
//!
//! Precision note: coordinates are `i64` microns encoded as JSON numbers. JSON integers above 2^53
//! lose precision in some readers (JavaScript, and Python when a value round-trips through a float),
//! but 2^53 µm ≈ 9 million km — any physical print is far inside the safe range. If astronomically
//! large coordinates ever matter, switch to string-encoded integers here.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Serialize any IR value to pretty JSON.
pub fn to_json<T: Serialize>(value: &T) -> serde_json::Result<String> {
    serde_json::to_string_pretty(value)
}

/// Parse any IR value from JSON.
pub fn from_json<T: DeserializeOwned>(s: &str) -> serde_json::Result<T> {
    serde_json::from_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{hi, lo, Area, ExtrudePath, Point, Polyline, RegionKind};

    fn sample() -> hi::Program {
        let path = Polyline::new(vec![
            Point::new(0, 0),
            Point::new(1000, 0),
            Point::new(1000, 1000),
        ]);
        hi::Program {
            layers: vec![hi::Layer {
                z_um: 200,
                regions: vec![hi::Region {
                    kind: RegionKind::Perimeter,
                    boundary: Area {
                        outer: path.clone(),
                        holes: vec![],
                    },
                    fills: vec![ExtrudePath {
                        path,
                        width_um: 400,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn hi_program_round_trips() {
        let prog = sample();
        let back: hi::Program = from_json(&to_json(&prog).unwrap()).unwrap();
        assert_eq!(prog, back);
    }

    #[test]
    fn lowered_program_round_trips() {
        let lowered = crate::lower::lower(&sample());
        let back: lo::Program = from_json(&to_json(&lowered).unwrap()).unwrap();
        assert_eq!(lowered, back);
    }
}
