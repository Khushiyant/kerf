//! Canonical content hashing of a move plan: a stable 128-bit digest over the IR, for deduplication,
//! cache keys, and "same program" claims across runs and machines.
//!
//! The hash is a dependency-free FNV-1a-128 over a canonical byte encoding of the program in order
//! (toolpath order is semantically meaningful, so it is part of the identity). It is consistent with
//! structural equality: two programs equal by `==` hash identically. The digest is deterministic and
//! endianness-independent (all integers are encoded big-endian), so a saved key means the same thing
//! on any platform.

use crate::ir::lo::{self, SegmentKind};
use crate::ir::RegionKind;

// FNV-1a-128 parameters (the published offset basis and prime).
const FNV_OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
const FNV_PRIME: u128 = 0x0000000001000000000000000000013b;

struct Fnv1a(u128);

impl Fnv1a {
    fn new() -> Self {
        Self(FNV_OFFSET)
    }
    fn byte(&mut self, b: u8) {
        self.0 ^= b as u128;
        self.0 = self.0.wrapping_mul(FNV_PRIME);
    }
    fn bytes(&mut self, bs: &[u8]) {
        for &b in bs {
            self.byte(b);
        }
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_be_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.bytes(&v.to_be_bytes());
    }
}

fn role_tag(r: RegionKind) -> u8 {
    match r {
        RegionKind::Perimeter => 1,
        RegionKind::Infill => 2,
        RegionKind::Skin => 3,
        RegionKind::Support => 4,
    }
}

/// The 128-bit canonical digest of a program.
pub fn canonical_hash_u128(program: &lo::Program) -> u128 {
    let mut h = Fnv1a::new();
    h.u64(program.layers.len() as u64);
    for layer in &program.layers {
        h.byte(0x01); // layer marker
        h.i64(layer.z_um);
        h.u64(layer.toolpaths.len() as u64);
        for tp in &layer.toolpaths {
            h.byte(0x02); // toolpath marker
            match tp.kind {
                SegmentKind::Extrude(role) => {
                    h.byte(0x10);
                    h.byte(role_tag(role));
                }
                SegmentKind::Travel => h.byte(0x11),
            }
            h.i64(tp.width_um);
            match tp.flow_e {
                Some(e) => {
                    h.byte(0xF1);
                    h.u64(e.to_bits());
                }
                None => h.byte(0xF0),
            }
            h.u64(tp.path.points.len() as u64);
            for p in &tp.path.points {
                h.i64(p.x);
                h.i64(p.y);
            }
        }
    }
    h.0
}

/// The canonical digest as a lowercase 32-char hex string — a stable program identity.
pub fn canonical_hash(program: &lo::Program) -> String {
    format!("{:032x}", canonical_hash_u128(program))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind, Toolpath};
    use crate::ir::{Point, Polyline, RegionKind};

    fn prog(x: i64, e: Option<f64>) -> lo::Program {
        lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Extrude(RegionKind::Perimeter),
                    path: Polyline::new(vec![Point::new(x, 0), Point::new(x + 10_000, 0)]),
                    width_um: 400,
                    flow_e: e,
                }],
            }],
        }
    }

    #[test]
    fn equal_programs_hash_equal_and_are_32_hex_chars() {
        let h = canonical_hash(&prog(0, Some(1.0)));
        assert_eq!(h, canonical_hash(&prog(0, Some(1.0))));
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn geometry_flow_and_order_all_affect_the_hash() {
        let base = canonical_hash(&prog(0, Some(1.0)));
        assert_ne!(
            base,
            canonical_hash(&prog(1, Some(1.0))),
            "geometry matters"
        );
        assert_ne!(base, canonical_hash(&prog(0, Some(2.0))), "flow matters");
        assert_ne!(
            base,
            canonical_hash(&prog(0, None)),
            "flow presence matters"
        );

        // Toolpath order is part of the identity.
        let a = prog(0, None).layers.remove(0).toolpaths;
        let reordered = lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: {
                    let mut v = a.clone();
                    v.push(Toolpath::travel(Polyline::new(vec![
                        Point::new(0, 0),
                        Point::new(1, 1),
                    ])));
                    v
                },
            }],
        };
        let mut swapped = reordered.clone();
        swapped.layers[0].toolpaths.swap(0, 1);
        assert_ne!(
            canonical_hash(&reordered),
            canonical_hash(&swapped),
            "order is part of the content identity"
        );
    }
}
