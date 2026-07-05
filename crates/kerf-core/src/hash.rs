//! Canonical content hashing of a move plan: a stable 128-bit digest over the IR, for deduplication,
//! cache keys, and "same program" claims across runs and machines.
//!
//! The hash is a dependency-free FNV-1a-128 over a canonical byte encoding of the program in order
//! (toolpath order is semantically meaningful, so it is part of the identity). The digest is
//! deterministic and endianness-independent (all integers are encoded big-endian), so a saved key
//! means the same thing on any platform.
//!
//! What is in the identity: layer count and Z; per toolpath its kind (extrude role vs travel), and —
//! for extrudes only — width, commanded flow, and the point list. A travel's `width_um`/`flow_e` are
//! inert (never emitted) and excluded. Geometry is integer microns and hashes exactly; commanded flow
//! (`flow_e`) is an f64, so it is quantized to 1e-5 mm of E (below any emitted G-code's resolution)
//! before hashing, which absorbs f64 summation/representation noise that raw bits would not.
//!
//! Stability scope: the digest is stable across the JSON round-trip (`to_json` → `from_json`), the
//! form you persist — that is the "same program across processes" guarantee. It is **not** stable
//! across a G-code emit/re-parse, because that path is lossy and outside the verified boundary (the
//! emitter synthesizes E for flow-less paths and re-quantizes it per segment, and the parser
//! re-coalesces toolpaths and drops travel-only layers). To ask "same deposited material" after a
//! G-code round-trip, compare denotations ([`crate::denote`] / [`crate::diff`]), not this hash.

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
    /// Hash commanded flow canonically: quantized to 1e-5 mm E, with a distinct sentinel for the
    /// absent and (pathological) non-finite cases so none of them collide.
    fn flow(&mut self, flow_e: Option<f64>) {
        match flow_e {
            Some(e) if e.is_finite() => {
                self.byte(0xF1);
                self.i64((e * 1e5).round() as i64);
            }
            Some(_) => self.byte(0xF2), // NaN / ±inf (never arises from parse or JSON)
            None => self.byte(0xF0),
        }
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
                // Width and flow are part of an extrude's identity; for travel they are inert.
                SegmentKind::Extrude(role) => {
                    h.byte(0x10);
                    h.byte(role_tag(role));
                    h.i64(tp.width_um);
                    h.flow(tp.flow_e);
                }
                SegmentKind::Travel => h.byte(0x11),
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
                    v.push(Toolpath::extrude(
                        SegmentKind::Extrude(RegionKind::Infill),
                        Polyline::new(vec![Point::new(0, 5000), Point::new(9, 5000)]),
                        300,
                    ));
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

    #[test]
    fn flow_is_hashed_at_emitter_precision_not_raw_bits() {
        // Two flows agreeing to 1e-5 mm E (below the emitter's resolution) are the same content.
        assert_eq!(
            canonical_hash(&prog(0, Some(0.222221))),
            canonical_hash(&prog(0, Some(0.2222206))),
            "sub-1e-5 flow noise must not change the hash"
        );
        // A difference the emitter *can* represent still changes the hash.
        assert_ne!(
            canonical_hash(&prog(0, Some(0.22222))),
            canonical_hash(&prog(0, Some(0.22223))),
        );
    }

    #[test]
    fn a_travels_inert_width_and_flow_do_not_change_the_identity() {
        let mk = |w: i64, e: Option<f64>| lo::Program {
            layers: vec![Layer {
                z_um: 200,
                toolpaths: vec![Toolpath {
                    kind: SegmentKind::Travel,
                    path: Polyline::new(vec![Point::new(0, 0), Point::new(1000, 0)]),
                    width_um: w,
                    flow_e: e,
                }],
            }],
        };
        // width_um / flow_e are never emitted for a travel, so they are not part of its identity.
        assert_eq!(
            canonical_hash(&mk(0, None)),
            canonical_hash(&mk(999, Some(3.0)))
        );
    }

    #[test]
    fn non_finite_flow_does_not_collide_with_zero_or_absent() {
        let nan = canonical_hash(&prog(0, Some(f64::NAN)));
        let inf = canonical_hash(&prog(0, Some(f64::INFINITY)));
        assert_ne!(nan, canonical_hash(&prog(0, Some(0.0))));
        assert_ne!(nan, canonical_hash(&prog(0, None)));
        assert_eq!(
            nan, inf,
            "all non-finite flow shares one sentinel (all invalid alike)"
        );
    }
}

#[cfg(all(test, feature = "serde"))]
mod proptests {
    use super::*;
    use crate::ir::lo::{Layer, SegmentKind};
    use crate::ir::{Point, Polyline, RegionKind};
    use proptest::prelude::*;

    fn arb_prog() -> impl Strategy<Value = lo::Program> {
        let pt = (-9000i64..9000, -9000i64..9000).prop_map(|(x, y)| Point::new(x, y));
        let flow = prop_oneof![Just(None), (0.0f64..1000.0).prop_map(Some)];
        let tp = (
            prop_oneof![
                Just(RegionKind::Perimeter),
                Just(RegionKind::Infill),
                Just(RegionKind::Skin),
                Just(RegionKind::Support),
            ],
            prop::collection::vec(pt, 1..6),
            50i64..600,
            flow,
        )
            .prop_map(|(role, pts, w, flow_e)| lo::Toolpath {
                kind: SegmentKind::Extrude(role),
                path: Polyline::new(pts),
                width_um: w,
                flow_e,
            });
        let layer = (0i64..5000, prop::collection::vec(tp, 0..4)).prop_map(|(z, tps)| Layer {
            z_um: z,
            toolpaths: tps,
        });
        prop::collection::vec(layer, 0..4).prop_map(|ls| lo::Program { layers: ls })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        // Canonicity across the persistence boundary: serialize to JSON and reload, and the hash is
        // unchanged — for arbitrary programs, flow included. This is the "same program across
        // processes" guarantee dedup/cache-keys rely on.
        #[test]
        fn hash_is_stable_across_json_round_trip(prog in arb_prog()) {
            let json = crate::json::to_json(&prog).unwrap();
            let back: lo::Program = crate::json::from_json(&json).unwrap();
            prop_assert_eq!(canonical_hash(&back), canonical_hash(&prog));
        }
    }
}
