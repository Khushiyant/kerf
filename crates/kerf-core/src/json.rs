//! JSON serialization boundary for the IR (feature `serde`).
//!
//! Coordinates are `i64` microns encoded as JSON numbers. Integers above 2^53 lose precision in
//! some readers (JavaScript, Python via float), but 2^53 µm is ~9 million km, far outside any
//! physical print. Switch to string-encoded integers if that ever changes.

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
