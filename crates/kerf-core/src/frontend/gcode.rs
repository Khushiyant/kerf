//! A robust, never-panic G-code -> [`crate::ir::lo`] parser for real FDM slicer output.
//!
//! Implements the verified spec (see `docs/06-architecture.md`). Key rules:
//!
//!  - **Extrude vs. travel by E-delta, never the G0/G1 opcode** (RepRap/Marlin alias them). Absolute
//!    (M82, the default) vs. relative (M83) E is tracked; `G92 E` resets the baseline. The
//!    extrude/travel/zero-length decision is made on the *rounded micron* displacement, so a
//!    sub-micron move never produces a degenerate zero-length segment or an absurd width.
//!  - **Layers are opened only by markers** (`;LAYER:` / `;LAYER_CHANGE` / `; CHANGE_LAYER`) or the
//!    first extrusion. The layer Z is the `;Z:` / `; Z_HEIGHT:` value when present, else the Z of the
//!    triggering extrusion — *never* a Z reached only by a travel hop, so Z-hops never create a
//!    spurious layer or misfile geometry at the hop height.
//!  - **Trust boundary.** Geometry (XY polyline, Z, extrude/travel) is TRUSTED. Feature ROLE
//!    (`;TYPE:` / `; FEATURE:`) and WIDTH (`;WIDTH:` / `; LINE_WIDTH:`) are UNTRUSTED re-inference:
//!    role is reset at every layer boundary, and any extruding move that falls back to `Perimeter`
//!    (unknown/unmapped/absent role) is recorded in [`Diagnostics`] — never silently trusted.
//!  - A skipped move (overflow, G91) changes no state — including the E baseline — so it can never
//!    corrupt the geometry of a later move.
//!  - **Arcs (G2/G3)** are flattened to chord polylines (I/J centre and R radius forms) within a
//!    ~20 µm deviation, so arc-fitted / ArcWelder output is captured, not skipped.
//!
//! # Known limitations (by design, not bugs)
//!
//!  - **Planar only.** The IR is 2D-per-layer; vase-mode / continuously-ramped Z prints are recovered
//!    as many thin layers or conflated — fundamentally lossy. Out of scope (see `docs/05-direction.md`).
//!  - **Deposited-only.** Pre-extrusion approach travel is elided and travel-only layers are dropped,
//!    so `Diagnostics` travel counts and any travel distance are a deposited-path lower bound.
//!  - **Degenerate input.** An extrusion before any Z is established is filed at z=0 (spec: modal Z
//!    defaults to 0); a `*checksum` embedded inside a `;TYPE:` value leaks into the (untrusted,
//!    diagnostic-only) role string. Both require malformed input no real slicer emits.

use std::collections::BTreeSet;

use crate::ir::lo::{self, SegmentKind, Toolpath};
use crate::ir::{Point, Polyline, RegionKind};

#[cfg(feature = "serde")]
use serde::Serialize;

const EPS_MM: f64 = 1e-6;
const NOMINAL_WIDTH_UM: i64 = 400;
/// Cross-section of 1.75 mm filament, mm² — the default for linear-E width back-computation.
const DEFAULT_FILAMENT_AREA_MM2: f64 = std::f64::consts::PI * (1.75 / 2.0) * (1.75 / 2.0);

/// Side-channel diagnostics from a parse: what was recovered, guessed, or dropped. This is where the
/// untrusted-inference gaps are surfaced so a verifier can distinguish trusted geometry from guesses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Diagnostics {
    pub lines: usize,
    pub layers: usize,
    pub extruding_toolpaths: usize,
    pub travel_toolpaths: usize,
    /// Distinct `;TYPE:` / `; FEATURE:` values (and `<no ;TYPE:>`) that fell back to `Perimeter`.
    pub unknown_roles: BTreeSet<String>,
    /// Count of extruding moves that used a guessed (fallback) role — the volume behind `unknown_roles`.
    pub fallback_role_moves: usize,
    /// Extruding moves whose width was estimated (formula) or nominal, not read from a width comment.
    pub estimated_width_moves: usize,
    /// Extruding moves with no XY displacement (prime / unretract): state advanced, no toolpath.
    pub zero_length_extrudes: usize,
    /// G2/G3 arcs flattened to chord polylines.
    pub arcs_flattened: usize,
    pub skipped_g91_moves: usize,
    pub overflow_skips: usize,
    /// Unknown G/M/T codes encountered (e.g. `M104`, `G28`, `T0`), deduplicated.
    pub skipped_codes: BTreeSet<String>,
}

/// The result of parsing G-code: the recovered program plus diagnostics.
#[derive(Clone, Debug)]
pub struct ParseReport {
    pub program: lo::Program,
    pub diagnostics: Diagnostics,
}

/// Parse untrusted G-code into a low-level program. Never panics.
pub fn parse(gcode: &str) -> ParseReport {
    let mut p = Parser::new();
    for raw in gcode.split('\n') {
        p.diag.lines += 1;
        p.line(raw);
    }
    p.finish()
}

// --------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Fields {
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
    e: Option<f64>,
    d: Option<f64>,
    i: Option<f64>, // arc centre X offset (G2/G3)
    j: Option<f64>, // arc centre Y offset (G2/G3)
    r: Option<f64>, // arc radius (G2/G3)
}

struct CurTp {
    kind: SegmentKind,
    width_um: i64,
    epoch: u64,
    points: Vec<Point>,
}

struct OpenLayer {
    z_um: i64,
    toolpaths: Vec<Toolpath>,
    cur: Option<CurTp>,
}

struct Parser {
    scale: f64,         // input unit -> mm (1.0 for mm/G21, 25.4 for inch/G20)
    abs_e: bool,        // M82 absolute (default) / M83 relative
    relative_xyz: bool, // G91 (unsupported for print moves)
    volumetric: bool,   // M200 D>0
    filament_area: f64,
    x: f64, // modal position, mm
    y: f64,
    z: f64,
    e_prev: f64, // modal E baseline, mm (absolute frame)
    layer_height_hint: Option<f64>,
    pending_layer_z_um: Option<i64>, // trusted layer Z from ;Z: / ; Z_HEIGHT:, applied on next open
    cur_role: Option<RegionKind>,
    cur_role_known: bool, // true only when the current role came from a recognized token
    cur_role_raw: Option<String>, // the raw ;TYPE: value, for diagnostics
    width_um: Option<i64>, // sticky width from comments
    width_epoch: u64,     // bumped when a width comment changes the value (feature boundary)
    force_new_layer: bool,
    open: Option<OpenLayer>,
    program: lo::Program,
    diag: Diagnostics,
}

impl Parser {
    fn new() -> Self {
        Self {
            scale: 1.0,
            abs_e: true,
            relative_xyz: false,
            volumetric: false,
            filament_area: DEFAULT_FILAMENT_AREA_MM2,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            e_prev: 0.0,
            layer_height_hint: None,
            pending_layer_z_um: None,
            cur_role: None,
            cur_role_known: false,
            cur_role_raw: None,
            width_um: None,
            width_epoch: 0,
            force_new_layer: false,
            open: None,
            program: lo::Program::new(),
            diag: Diagnostics::default(),
        }
    }

    fn line(&mut self, raw: &str) {
        let line = raw.trim_end_matches('\r');
        let (code, comment) = split_code_comment(line);
        if let Some(c) = comment {
            self.comment(&c);
        }
        let code = code.trim();
        if code.is_empty() {
            return;
        }
        // Drop a trailing "*checksum".
        let code = match code.find('*') {
            Some(i) => &code[..i],
            None => code,
        };
        let words = lex_words(code);
        if words.is_empty() {
            return;
        }
        self.command(&words);
    }

    fn comment(&mut self, com: &str) {
        if let Some(v) = com
            .strip_prefix(";TYPE:")
            .or_else(|| com.strip_prefix("; FEATURE:"))
        {
            self.set_role(v.trim());
        } else if com == ";LAYER_CHANGE" || com == "; CHANGE_LAYER" || com.starts_with(";LAYER:") {
            // A layer boundary flushes the layer AND clears the role, so an untyped extrude on the new
            // layer is a recorded fallback rather than a silent inherit of the previous layer's role.
            self.force_new_layer = true;
            self.cur_role = None;
            self.cur_role_known = false;
            self.cur_role_raw = None;
        } else if let Some(v) = com
            .strip_prefix(";Z:")
            .or_else(|| com.strip_prefix("; Z_HEIGHT:"))
        {
            if let Some(mm) = parse_mm(v) {
                self.z = mm;
                self.pending_layer_z_um = mm_to_um(mm);
            }
        } else if let Some(v) = com
            .strip_prefix(";HEIGHT:")
            .or_else(|| com.strip_prefix("; LAYER_HEIGHT:"))
            .or_else(|| com.strip_prefix(";Layer height:"))
        {
            if let Some(mm) = parse_mm(v) {
                if mm > 0.0 {
                    self.layer_height_hint = Some(mm);
                }
            }
        } else if let Some(v) = com
            .strip_prefix(";WIDTH:")
            .or_else(|| com.strip_prefix("; LINE_WIDTH:"))
        {
            if let Some(mm) = parse_mm(v) {
                if let Some(w) = mm_to_um(mm) {
                    // A non-positive width comment is malformed; ignore it so extruding toolpaths
                    // always carry a positive width (falls through to the formula/nominal).
                    if w > 0 && Some(w) != self.width_um {
                        self.width_um = Some(w);
                        self.width_epoch += 1;
                    }
                }
            }
        }
        // Any other comment is ignored.
    }

    fn set_role(&mut self, value: &str) {
        self.cur_role_raw = Some(value.to_string());
        match classify_role(value) {
            Some(rk) => {
                self.cur_role = Some(rk);
                self.cur_role_known = true;
            }
            None => {
                self.cur_role = Some(RegionKind::Perimeter); // designated fallback
                self.cur_role_known = false;
            }
        }
    }

    /// Resolve the role for an extruding move, recording a diagnostic when it is a guessed fallback.
    fn consume_role(&mut self) -> RegionKind {
        match (self.cur_role, self.cur_role_known) {
            (Some(rk), true) => rk,
            _ => {
                self.diag.fallback_role_moves += 1;
                let label = self
                    .cur_role_raw
                    .clone()
                    .unwrap_or_else(|| "<no ;TYPE:>".to_string());
                self.diag.unknown_roles.insert(label);
                RegionKind::Perimeter
            }
        }
    }

    fn command(&mut self, words: &[(char, f64)]) {
        let Some((cl, cv)) = words
            .iter()
            .copied()
            .find(|(l, _)| matches!(l, 'G' | 'M' | 'T'))
        else {
            return;
        };
        if !cv.is_finite() {
            return;
        }
        let n = cv.round() as i64;
        let f = self.fields(words);
        match (cl, n) {
            ('G', 0) | ('G', 1) => self.do_move(&f),
            ('G', 2) => self.do_arc(&f, false), // clockwise
            ('G', 3) => self.do_arc(&f, true),  // counter-clockwise
            ('G', 92) => {
                if let Some(e) = f.e {
                    self.e_prev = e;
                }
                if let Some(x) = f.x {
                    self.x = x;
                }
                if let Some(y) = f.y {
                    self.y = y;
                }
                if let Some(z) = f.z {
                    self.z = z;
                }
            }
            ('G', 20) => self.scale = 25.4,
            ('G', 21) => self.scale = 1.0,
            ('G', 90) => self.relative_xyz = false,
            ('G', 91) => self.relative_xyz = true,
            ('G', 17) | ('G', 18) | ('G', 19) => {} // plane select (only affects arcs, punted)
            ('G', 10) | ('G', 11) => {}             // firmware retract, no geometry
            ('M', 82) => self.abs_e = true,
            ('M', 83) => self.abs_e = false,
            ('M', 200) => {
                if let Some(d) = f.d {
                    if d > 0.0 {
                        self.volumetric = true;
                        self.filament_area = std::f64::consts::PI * (d / 2.0) * (d / 2.0);
                    } else {
                        self.volumetric = false;
                    }
                }
            }
            _ => {
                self.diag.skipped_codes.insert(format!("{cl}{n}"));
            }
        }
    }

    fn fields(&self, words: &[(char, f64)]) -> Fields {
        let mut f = Fields::default();
        for &(l, v) in words {
            if !v.is_finite() {
                continue;
            }
            match l {
                'X' => f.x = Some(v * self.scale),
                'Y' => f.y = Some(v * self.scale),
                'Z' => f.z = Some(v * self.scale),
                'E' => f.e = Some(v * self.scale),
                'I' => f.i = Some(v * self.scale),
                'J' => f.j = Some(v * self.scale),
                'R' => f.r = Some(v * self.scale),
                'D' => f.d = Some(v),
                _ => {}
            }
        }
        f
    }

    /// Consume a move's E field into the modal baseline and return the extrusion delta (mm).
    fn take_de(&mut self, f: &Fields) -> f64 {
        match f.e {
            Some(e) => {
                if self.abs_e {
                    let d = e - self.e_prev;
                    self.e_prev = e;
                    d
                } else {
                    self.e_prev += e;
                    e
                }
            }
            None => 0.0,
        }
    }

    /// Handle a G2 (clockwise) / G3 (counter-clockwise) arc by flattening it to chord segments.
    fn do_arc(&mut self, f: &Fields, ccw: bool) {
        if self.relative_xyz {
            self.diag.skipped_g91_moves += 1;
            return;
        }
        let nx = f.x.unwrap_or(self.x);
        let ny = f.y.unwrap_or(self.y);
        let nz = f.z.unwrap_or(self.z);
        if !(nx.is_finite() && ny.is_finite() && nz.is_finite()) {
            return;
        }
        let (Some(px), Some(py)) = (mm_to_um(self.x), mm_to_um(self.y)) else {
            self.diag.overflow_skips += 1;
            return;
        };
        let (Some(tx), Some(ty)) = (mm_to_um(nx), mm_to_um(ny)) else {
            self.diag.overflow_skips += 1;
            return;
        };
        let prev = Point::new(px, py);
        let target = Point::new(tx, ty);
        let de = self.take_de(f);

        // Arc centre, in microns. I/J are offsets from the current point; R computes the centre.
        let center = match (f.i, f.j, f.r) {
            // I/J centre form: either offset present, the other defaults to 0 (a missing offset is
            // legal — e.g. `G2 X20 I10` is a valid semicircle with J=0).
            (i, j, _) if i.is_some() || j.is_some() => Some((
                px as f64 + i.unwrap_or(0.0) * 1000.0,
                py as f64 + j.unwrap_or(0.0) * 1000.0,
            )),
            (_, _, Some(r)) => arc_center_from_r(prev, target, r * 1000.0, ccw),
            _ => None,
        };
        let chords = match center {
            Some((cx, cy)) => flatten_arc(prev, target, cx, cy, ccw, ARC_TOL_UM),
            None => vec![target], // no centre info: fall back to a straight chord to the target
        };
        self.diag.arcs_flattened += 1;

        // Drop points that do not advance (degenerate/duplicate), so no zero-length segment is emitted.
        let mut run: Vec<Point> = Vec::new();
        let mut last = prev;
        for p in chords {
            if p != last {
                run.push(p);
                last = p;
            }
        }

        if run.is_empty() {
            if de > EPS_MM {
                self.diag.zero_length_extrudes += 1;
            }
        } else if de > EPS_MM {
            self.open_layer(nz);
            let width = self.width_for(de, polyline_len_mm(prev, &run));
            let role = self.consume_role();
            let kind = SegmentKind::Extrude(role);
            let mut rp = prev;
            for p in run {
                self.push_segment(kind, width, rp, p);
                rp = p;
            }
        } else if self.open.is_some() {
            let mut rp = prev;
            for p in run {
                self.push_segment(SegmentKind::Travel, 0, rp, p);
                rp = p;
            }
        }

        self.x = nx;
        self.y = ny;
        self.z = nz;
    }

    fn do_move(&mut self, f: &Fields) {
        if self.relative_xyz {
            self.diag.skipped_g91_moves += 1;
            return; // skipped move changes no state (incl. E baseline)
        }
        let nx = f.x.unwrap_or(self.x);
        let ny = f.y.unwrap_or(self.y);
        let nz = f.z.unwrap_or(self.z);
        if !(nx.is_finite() && ny.is_finite() && nz.is_finite()) {
            return;
        }
        // Overflow guards BEFORE consuming E: a skipped move must not perturb the E baseline (SF-1),
        // and the previous position must be representable, not a phantom origin (RE-4).
        let (Some(px), Some(py)) = (mm_to_um(self.x), mm_to_um(self.y)) else {
            self.diag.overflow_skips += 1;
            return;
        };
        let (Some(tx), Some(ty)) = (mm_to_um(nx), mm_to_um(ny)) else {
            self.diag.overflow_skips += 1;
            return;
        };
        let prev = Point::new(px, py);
        let target = Point::new(tx, ty);

        let de = self.take_de(f);
        let dx = nx - self.x;
        let dy = ny - self.y;
        let len_mm = (dx * dx + dy * dy).sqrt();
        let same_point = target == prev; // decided on rounded microns, not float mm

        if de > EPS_MM && !same_point {
            self.open_layer(nz);
            let width = self.width_for(de, len_mm);
            let role = self.consume_role();
            self.push_segment(SegmentKind::Extrude(role), width, prev, target);
        } else if de > EPS_MM {
            // Extruding but no XY move (prime / unretract): advance state, emit nothing.
            self.diag.zero_length_extrudes += 1;
        } else if !same_point && self.open.is_some() {
            self.push_segment(SegmentKind::Travel, 0, prev, target);
        }

        self.x = nx;
        self.y = ny;
        self.z = nz;
    }

    fn width_for(&mut self, de_mm: f64, len_mm: f64) -> i64 {
        if let Some(w) = self.width_um {
            return w; // from a width comment
        }
        self.diag.estimated_width_moves += 1;
        let lh = self.layer_height_hint.unwrap_or(0.0);
        if lh > 0.0 && len_mm > 0.0 && de_mm > 0.0 {
            let w_mm = if self.volumetric {
                de_mm / (lh * len_mm)
            } else {
                (de_mm * self.filament_area) / (lh * len_mm)
            };
            if let Some(w) = mm_to_um(w_mm) {
                if w > 0 {
                    return w;
                }
            }
        }
        NOMINAL_WIDTH_UM
    }

    /// Open a new layer only on a marker or the first extrusion; otherwise continue the current layer
    /// (so a travel Z-hop never opens a spurious layer). The layer Z is the trusted `;Z:` value or the
    /// extruding move's own Z — never a Z reached only by travel.
    fn open_layer(&mut self, extrude_z_mm: f64) {
        if self.open.is_none() || self.force_new_layer {
            self.flush_layer();
            let z_um = self
                .pending_layer_z_um
                .take()
                .or_else(|| mm_to_um(extrude_z_mm))
                .unwrap_or(0);
            self.open = Some(OpenLayer {
                z_um,
                toolpaths: Vec::new(),
                cur: None,
            });
            self.force_new_layer = false;
        }
    }

    fn push_segment(&mut self, kind: SegmentKind, width_um: i64, prev: Point, target: Point) {
        let epoch = if matches!(kind, SegmentKind::Travel) {
            0
        } else {
            self.width_epoch
        };
        let Some(layer) = self.open.as_mut() else {
            return;
        };
        // Continue the current toolpath only if kind, width-epoch, and (for formula widths) the
        // effective width all match — otherwise a same-role run would collapse to its first width.
        let continues = match &layer.cur {
            Some(c) => c.kind == kind && c.epoch == epoch && width_close(c.width_um, width_um),
            None => false,
        };
        if continues {
            layer.cur.as_mut().unwrap().points.push(target);
        } else {
            if let Some(c) = layer.cur.take() {
                layer.toolpaths.push(finish_tp(c));
            }
            layer.cur = Some(CurTp {
                kind,
                width_um,
                epoch,
                points: vec![prev, target],
            });
        }
    }

    fn flush_layer(&mut self) {
        if let Some(mut layer) = self.open.take() {
            if let Some(c) = layer.cur.take() {
                layer.toolpaths.push(finish_tp(c));
            }
            if !layer.toolpaths.is_empty() {
                self.program.layers.push(lo::Layer {
                    z_um: layer.z_um,
                    toolpaths: layer.toolpaths,
                });
            }
        }
    }

    fn finish(mut self) -> ParseReport {
        self.flush_layer();
        self.diag.layers = self.program.layers.len();
        for tp in self.program.layers.iter().flat_map(|l| &l.toolpaths) {
            if tp.kind.extrudes() {
                self.diag.extruding_toolpaths += 1;
            } else {
                self.diag.travel_toolpaths += 1;
            }
        }
        ParseReport {
            program: self.program,
            diagnostics: self.diag,
        }
    }
}

fn finish_tp(c: CurTp) -> Toolpath {
    Toolpath {
        kind: c.kind,
        path: Polyline::new(c.points),
        width_um: c.width_um,
    }
}

/// Whether two widths are close enough to belong to the same toolpath (5% + 20 µm). Splits a genuine
/// width change (a 4× bead) without shattering a run of micro-varying formula widths.
fn width_close(a: i64, b: i64) -> bool {
    if a == b {
        return true;
    }
    let (a, b) = (a as f64, b as f64);
    (a - b).abs() <= 0.05 * a.max(b) + 20.0
}

/// Map a slicer feature-type value to a `RegionKind`. `None` = unknown/unmapped (caller falls back and
/// records a diagnostic). Covers Cura (hyphen-uppercase), PrusaSlicer (Title Case) and OrcaSlicer/Bambu
/// (`; FEATURE:` Title Case) vocabularies.
fn classify_role(v: &str) -> Option<RegionKind> {
    use RegionKind::*;
    match v {
        // Cura
        "WALL-OUTER" | "WALL-INNER" => Some(Perimeter),
        "SKIN" => Some(Skin),
        "FILL" => Some(Infill),
        "SUPPORT" | "SUPPORT-INTERFACE" => Some(Support),
        // PrusaSlicer
        "Perimeter" | "External perimeter" | "Overhang perimeter" => Some(Perimeter),
        "Internal infill" | "Gap fill" => Some(Infill),
        "Solid infill" | "Top solid infill" | "Bridge infill" | "Ironing" => Some(Skin),
        "Support material" | "Support material interface" => Some(Support),
        // OrcaSlicer / Bambu
        "Inner wall" | "Outer wall" | "Overhang wall" => Some(Perimeter),
        "Sparse infill" | "Gap infill" => Some(Infill),
        "Internal solid infill"
        | "Top surface"
        | "Bottom surface"
        | "Bridge"
        | "Internal Bridge" => Some(Skin),
        "Support" | "Support interface" | "Support transition" => Some(Support),
        // Explicitly-unknown / non-structural (Skirt, Brim, Prime tower, Wipe tower, Custom, ...)
        _ => None,
    }
}

/// Split a physical line into (code, comment). Removes balanced `(...)` spans; the comment is
/// everything from the first `;` outside parentheses (including the `;`). Unbalanced `(` => the rest
/// is treated as a comment (dropped from code).
fn split_code_comment(raw: &str) -> (String, Option<String>) {
    let mut code = String::new();
    let mut depth: i32 = 0;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if depth == 0 && c == ';' {
            return (code, Some(chars[i..].iter().collect()));
        }
        match c {
            '(' => depth += 1,
            ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => code.push(c),
            _ => {}
        }
        i += 1;
    }
    (code, None)
}

/// Tokenize G-code words: a letter followed by a numeric literal. Non-word characters are skipped;
/// an unparseable number yields NaN (the caller drops non-finite fields).
fn lex_words(code: &str) -> Vec<(char, f64)> {
    let mut words = Vec::new();
    let b: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() {
            let letter = c.to_ascii_uppercase();
            i += 1;
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], '.' | '-' | '+')) {
                i += 1;
            }
            let s: String = b[start..i].iter().collect();
            words.push((letter, s.parse::<f64>().unwrap_or(f64::NAN)));
        } else {
            i += 1;
        }
    }
    words
}

fn parse_mm(s: &str) -> Option<f64> {
    let v = s.trim().parse::<f64>().ok()?;
    v.is_finite().then_some(v)
}

/// mm -> integer microns, guarding non-finite and i64 overflow.
fn mm_to_um(mm: f64) -> Option<i64> {
    let v = (mm * 1000.0).round();
    (v.is_finite() && v.abs() < 9.0e18).then_some(v as i64)
}

/// Max chord deviation (microns) used when flattening arcs. ~20 µm is well below any raster resolution.
const ARC_TOL_UM: f64 = 20.0;

/// Round a micron-valued f64 to a `Point` coordinate, guarding non-finite / overflow.
fn um_round(x: f64, y: f64) -> Option<Point> {
    if x.is_finite() && y.is_finite() && x.abs() < 9.0e18 && y.abs() < 9.0e18 {
        Some(Point::new(x.round() as i64, y.round() as i64))
    } else {
        None
    }
}

/// Flatten a circular arc from `start` to `end` about `centre` (all microns) into chord points, with
/// `start` EXCLUDED and `end` INCLUDED, keeping the max chord deviation below `tol_um`. `ccw` selects
/// counter-clockwise (G3). A degenerate arc yields just `[end]` (a straight chord).
fn flatten_arc(start: Point, end: Point, cx: f64, cy: f64, ccw: bool, tol_um: f64) -> Vec<Point> {
    let (sx, sy) = (start.x as f64, start.y as f64);
    let radius = ((sx - cx).powi(2) + (sy - cy).powi(2)).sqrt();
    if !radius.is_finite() || radius < 1.0 {
        return vec![end];
    }
    let a0 = (sy - cy).atan2(sx - cx);
    let a1 = ((end.y as f64) - cy).atan2((end.x as f64) - cx);
    // Sweep in the commanded direction, normalized to (0, 2π]; equal endpoints => full circle.
    let mut sweep = if ccw { a1 - a0 } else { a0 - a1 }.rem_euclid(std::f64::consts::TAU);
    if sweep <= 1e-9 {
        sweep = std::f64::consts::TAU;
    }
    let max_dtheta = if tol_um < radius {
        2.0 * (1.0 - tol_um / radius).clamp(-1.0, 1.0).acos()
    } else {
        sweep
    };
    let n = ((sweep / max_dtheta).ceil() as i64).clamp(1, 4096) as usize;
    let dir = if ccw { 1.0 } else { -1.0 };
    let mut pts = Vec::with_capacity(n);
    for k in 1..=n {
        if k == n {
            pts.push(end); // snap the last point exactly to the commanded endpoint
        } else {
            let a = a0 + dir * sweep * (k as f64) / (n as f64);
            if let Some(p) = um_round(cx + radius * a.cos(), cy + radius * a.sin()) {
                pts.push(p);
            }
        }
    }
    pts
}

/// Arc centre (microns) from the R (radius) form, following grbl's construction. Returns `None` if the
/// radius is too small to span the chord. Positive `r_um` selects the minor arc, negative the major.
fn arc_center_from_r(start: Point, end: Point, r_um: f64, ccw: bool) -> Option<(f64, f64)> {
    let (sx, sy) = (start.x as f64, start.y as f64);
    let x = end.x as f64 - sx; // chord vector
    let y = end.y as f64 - sy;
    let d2 = x * x + y * y;
    if d2 < 1.0 || !r_um.is_finite() {
        return None;
    }
    let disc = 4.0 * r_um * r_um - d2;
    if disc < 0.0 {
        return None; // radius too small to reach the endpoint
    }
    let mut h_x2_div_d = -(disc.sqrt()) / d2.sqrt();
    if ccw {
        h_x2_div_d = -h_x2_div_d; // grbl: negate for a counter-clockwise (G3) arc
    }
    if r_um < 0.0 {
        h_x2_div_d = -h_x2_div_d; // negative R selects the major (>180°) arc
    }
    Some((
        sx + 0.5 * (x - y * h_x2_div_d),
        sy + 0.5 * (y + x * h_x2_div_d),
    ))
}

/// Total length (mm) of the polyline `prev -> run[0] -> run[1] -> ...`, from micron points.
fn polyline_len_mm(prev: Point, run: &[Point]) -> f64 {
    let mut total = 0.0;
    let mut a = prev;
    for &b in run {
        let dx = b.x as f64 - a.x as f64;
        let dy = b.y as f64 - a.y as f64;
        total += (dx * dx + dy * dy).sqrt();
        a = b;
    }
    total / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURA: &str = ";FLAVOR:Marlin\n;Layer height: 0.2\nG21\nM82 ;absolute extrusion mode\nG92 E0\n;LAYER:0\nG0 F3600 X50 Y50 Z0.2\n;TYPE:WALL-OUTER\nG1 F1200 X70 Y50 E1.2\nG1 X70 Y70 E2.4\n;TYPE:FILL\nG1 X50 Y70 E3.6\nG0 X55 Y55\n;LAYER:1\nG0 Z0.4\n;TYPE:SKIN\nG1 X60 Y60 E4.8";

    const PRUSA: &str = "; generated by PrusaSlicer 2.4.0\nM83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;HEIGHT:0.2\n;TYPE:Skirt/Brim\n;WIDTH:0.28\nG1 F1200 X100 Y100 E.5\nG1 X120 Y100 E.5\n;TYPE:External perimeter\n;WIDTH:0.45\nG1 X120 Y120 E.7\nG1 E-0.8 F2100\n;LAYER_CHANGE\n;Z:0.4\n;TYPE:Solid infill\nG1 X100 Y120 E.6";

    const BBL: &str = "; generated by OrcaSlicer 2.3.1 on Bambu Lab A1\nM83\nG21\n; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n; FEATURE: Outer wall\n; LINE_WIDTH: 0.42\nG1 F1500 X100 Y100 E.4\nG1 X130 Y100 E.6\n; FEATURE: Sparse infill\n; LINE_WIDTH: 0.45\nG1 X130 Y130 E.5";

    fn ext_widths(r: &ParseReport) -> Vec<i64> {
        r.program
            .layers
            .iter()
            .flat_map(|l| &l.toolpaths)
            .filter(|t| t.kind.extrudes())
            .map(|t| t.width_um)
            .collect()
    }

    #[test]
    fn cura_two_layers_roles_and_estimated_width() {
        let r = parse(CURA);
        assert_eq!(
            r.program.layers.iter().map(|l| l.z_um).collect::<Vec<_>>(),
            vec![200, 400]
        );
        assert_eq!(r.program.extrusion_move_count(), 3); // wall, fill, skin
        assert!(r.diagnostics.unknown_roles.is_empty());
        assert_eq!(r.diagnostics.fallback_role_moves, 0);
        assert!(r.diagnostics.estimated_width_moves > 0); // Cura has no ;WIDTH:
    }

    #[test]
    fn prusa_relative_e_retraction_is_travel_and_unknown_role_flagged() {
        let r = parse(PRUSA);
        assert_eq!(
            r.program.layers.iter().map(|l| l.z_um).collect::<Vec<_>>(),
            vec![200, 400]
        );
        assert_eq!(r.program.extrusion_move_count(), 3); // skirt, ext-perim, solid-infill
        assert!(r.diagnostics.unknown_roles.contains("Skirt/Brim"));
        assert_eq!(r.diagnostics.fallback_role_moves, 2); // two skirt moves used the fallback role
        assert_eq!(ext_widths(&r), vec![280, 450, 450]); // sticky ;WIDTH: in microns
        assert_eq!(r.diagnostics.travel_toolpaths, 0); // retraction (no XY) is not a travel toolpath
    }

    #[test]
    fn bbl_feature_and_line_width_tags() {
        let r = parse(BBL);
        assert_eq!(r.program.layers.len(), 1);
        assert_eq!(r.program.layers[0].z_um, 200);
        assert_eq!(ext_widths(&r), vec![420, 450]);
        assert!(r.diagnostics.unknown_roles.is_empty());
    }

    #[test]
    fn parsed_program_denotes() {
        let r = parse(PRUSA);
        let occ = crate::denote::denote_lo(&r.program, 200);
        assert!(occ.layers.iter().any(|l| !l.cells.is_empty()));
    }

    // --- regression tests for the bugs the adversarial review found ---

    #[test]
    fn sf1_overflow_move_does_not_delete_the_next_extrude() {
        // A single oversized coordinate must not consume the E baseline and flip the next real
        // extrude into a travel.
        let g = ";TYPE:WALL-OUTER\nG1 X5 Y0 E5\nG1 X99999999999999999 Y0 E10\nG1 X8 Y8 E6";
        let r = parse(g);
        assert_eq!(r.diagnostics.overflow_skips, 1);
        assert_eq!(r.diagnostics.travel_toolpaths, 0); // the third move stays an extrude
        assert!(r.program.extrusion_move_count() >= 1);
        // The real geometry survives: the last deposited point is the third move's endpoint (the
        // extrude was not flipped to a travel and deleted). It continues the first path since the
        // skipped move left the head at (5,0).
        let last = r.program.layers.last().unwrap().toolpaths.last().unwrap();
        assert_eq!(*last.path.points.last().unwrap(), Point::new(8000, 8000));
    }

    #[test]
    fn re1_subresolution_move_is_zero_length_not_a_duplicate_vertex() {
        // Second move is a real extrusion (E2 > E1) but rounds to the same micron point.
        let r = parse(";TYPE:WALL-OUTER\nG1 X10 Y10 E1\nG1 X10.0000001 Y10 E2");
        assert_eq!(r.program.extrusion_move_count(), 1);
        assert_eq!(r.diagnostics.zero_length_extrudes, 1);
        let pts = &r.program.layers[0].toolpaths[0].path.points;
        assert_eq!(pts, &vec![Point::new(0, 0), Point::new(10_000, 10_000)]);
    }

    #[test]
    fn re3_subresolution_travel_emits_no_degenerate_segment() {
        let r = parse(";TYPE:WALL-OUTER\nG1 X10 Y10 E1\nG1 X10.0000001 Y10");
        assert_eq!(r.diagnostics.travel_toolpaths, 0);
    }

    #[test]
    fn sm1_travel_zhop_does_not_create_a_spurious_layer() {
        let r = parse(";LAYER:0\nG1 X10 Y10 Z0.2 E1\nG0 Z0.6\nG1 X20 Y10 E2");
        assert_eq!(r.program.layers.len(), 1);
        assert_eq!(r.program.layers[0].z_um, 200); // both extrudes filed at 200, not 600
                                                   // The pure-Z hop moves no XY, so the two extrudes form one continuous path in layer 0 that
                                                   // reaches the second endpoint — the key point is nothing lands on a spurious 600 layer.
        let last = r.program.layers[0].toolpaths.last().unwrap();
        assert_eq!(
            *last.path.points.last().unwrap(),
            Point::new(20_000, 10_000)
        );
    }

    #[test]
    fn trw1_formula_width_splits_on_a_real_change() {
        // Same role, no ;WIDTH: comments, two beads with a >2x width difference => two toolpaths.
        let r = parse(";Layer height:0.2\n;TYPE:WALL-OUTER\nG1 X10 Y0 E1\nG1 X10 Y10 E5");
        let w = ext_widths(&r);
        assert_eq!(
            w.len(),
            2,
            "a 5x flow change must not collapse to one width: {w:?}"
        );
        assert!(w[1] as f64 > 2.0 * w[0] as f64);
    }

    #[test]
    fn trw1_near_identical_formula_widths_do_not_over_fragment() {
        let r =
            parse(";Layer height:0.2\n;TYPE:WALL-OUTER\nG1 X10 Y0 E1\nG1 X20 Y0 E1\nG1 X30 Y0 E1");
        assert_eq!(ext_widths(&r).len(), 1); // constant flow => one toolpath
    }

    #[test]
    fn trw3_role_does_not_leak_across_a_layer_boundary() {
        let r = parse(";TYPE:SUPPORT\nG1 X10 Y10 Z0.2 E1\n;LAYER:1\nG1 X20 Y20 Z0.4 E2");
        // The second (untyped) extrude must NOT silently inherit Support; it falls back + is recorded.
        let last = r.program.layers.last().unwrap().toolpaths.last().unwrap();
        assert_eq!(last.kind, SegmentKind::Extrude(RegionKind::Perimeter));
        assert!(r.diagnostics.fallback_role_moves >= 1);
        assert!(r.diagnostics.unknown_roles.contains("<no ;TYPE:>"));
    }

    #[test]
    fn re4_overflowing_g92_position_does_not_create_a_phantom_origin_edge() {
        let r = parse("G92 X10000000000000000 Y0\nG1 X10 Y10 E1");
        assert!(r.diagnostics.overflow_skips >= 1);
        assert_eq!(r.program.extrusion_move_count(), 0); // skipped, not a phantom edge from (0,0)
    }

    /// The flattened points of a quarter circle (start (10,0) -> end (0,10) about the origin) must lie
    /// on the radius-10 circle, hit the 45° point, and land exactly on both commanded endpoints.
    fn assert_quarter_circle(pts: &[Point]) {
        assert_eq!(*pts.first().unwrap(), Point::new(10_000, 0));
        assert_eq!(*pts.last().unwrap(), Point::new(0, 10_000));
        // Every point sits on radius 10 mm (within chord tolerance + rounding).
        for p in pts {
            let r = ((p.x as f64).powi(2) + (p.y as f64).powi(2)).sqrt();
            assert!(
                (r - 10_000.0).abs() < 30.0,
                "point {p:?} not on the arc (r={r})"
            );
        }
        // Flattened to many chords (not the 2-point straight fallback), and the arc bulges outward:
        // its midpoint sits on radius 10 mm, whereas a straight chord's midpoint would be at ~7.07 mm.
        assert!(
            pts.len() > 5,
            "arc should flatten to many chords, got {}",
            pts.len()
        );
        let mid = pts[pts.len() / 2];
        let mid_r = ((mid.x as f64).powi(2) + (mid.y as f64).powi(2)).sqrt();
        assert!(
            (mid_r - 10_000.0).abs() < 30.0,
            "arc midpoint not on the circle: {mid:?}"
        );
    }

    #[test]
    fn arc_ij_form_flattens_to_the_circle() {
        // G3 (CCW) from (10,0) to (0,10) about the origin: I = -10, J = 0 (offset to centre).
        let r = parse(";LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG0 X10 Y0\nG3 X0 Y10 I-10 J0 E1");
        assert_eq!(r.diagnostics.arcs_flattened, 1);
        assert_eq!(r.program.extrusion_move_count(), 1);
        assert_quarter_circle(&r.program.layers[0].toolpaths[0].path.points);
    }

    #[test]
    fn arc_r_form_matches_the_ij_form() {
        // Same quarter circle via the radius form: positive R selects the minor (90°) arc.
        let r = parse(";LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG0 X10 Y0\nG3 X0 Y10 R10 E1");
        assert_eq!(r.diagnostics.arcs_flattened, 1);
        assert_quarter_circle(&r.program.layers[0].toolpaths[0].path.points);
    }

    #[test]
    fn arc_without_centre_info_falls_back_to_a_straight_chord() {
        let r = parse(";LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG0 X10 Y0\nG3 X0 Y10 E1");
        assert_eq!(r.diagnostics.arcs_flattened, 1);
        let pts = &r.program.layers[0].toolpaths[0].path.points;
        assert_eq!(pts, &vec![Point::new(10_000, 0), Point::new(0, 10_000)]);
    }

    #[test]
    fn arc_with_only_one_offset_still_flattens() {
        // `G2 X-10 Y0 I-10` (J omitted, defaults to 0): a CW semicircle about the origin, NOT a chord.
        let r = parse(";LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG0 X10 Y0\nG2 X-10 Y0 I-10 E1");
        assert_eq!(r.diagnostics.arcs_flattened, 1);
        let pts = &r.program.layers[0].toolpaths[0].path.points;
        assert!(
            pts.len() > 5,
            "single-offset arc collapsed to a chord: {pts:?}"
        );
        assert_eq!(*pts.first().unwrap(), Point::new(10_000, 0));
        assert_eq!(*pts.last().unwrap(), Point::new(-10_000, 0));
        for p in pts {
            let radius = ((p.x as f64).powi(2) + (p.y as f64).powi(2)).sqrt();
            assert!((radius - 10_000.0).abs() < 30.0);
        }
        assert!(
            pts.iter().any(|p| p.y < -9000),
            "CW semicircle should bulge to -y"
        );
    }

    #[test]
    fn malformed_input_never_panics() {
        for junk in [
            "",
            "\n\n\r\n",
            ";just a comment",
            "G1 X Y E",
            "G1 X1e999 Y1",
            "G1 XNaN Y1",
            "N5 G1 X1 Y2*36",
            "G2 X1 Y1 I0.5 J0",
            "M104 S200",
            "G1(inline paren)X10 Y10 E1",
            "garbage \0 bytes \t 123 ;;;",
        ] {
            let _ = parse(junk);
        }
    }
}

#[cfg(test)]
mod proptests {
    //! Property-based fuzzing: the parser must never panic on any input, and every program it returns
    //! must satisfy the structural invariants downstream code relies on.
    use super::*;
    use proptest::prelude::*;

    fn check_invariants(r: &ParseReport) {
        assert_eq!(r.diagnostics.layers, r.program.layers.len());
        let (mut ext, mut trav) = (0usize, 0usize);
        for tp in r.program.layers.iter().flat_map(|l| &l.toolpaths) {
            assert!(tp.path.points.len() >= 2, "toolpath must have >= 2 points");
            for w in tp.path.points.windows(2) {
                assert_ne!(w[0], w[1], "no two consecutive points may be equal");
            }
            if tp.kind.extrudes() {
                assert!(
                    tp.width_um > 0,
                    "extruding toolpath must have positive width"
                );
                ext += 1;
            } else {
                assert_eq!(tp.width_um, 0, "travel width must be 0");
                trav += 1;
            }
        }
        assert_eq!(ext, r.diagnostics.extruding_toolpaths);
        assert_eq!(trav, r.diagnostics.travel_toolpaths);
    }

    fn arb_line() -> impl Strategy<Value = String> {
        prop_oneof![
            (any::<i8>(), any::<i8>(), 0i8..40)
                .prop_map(|(x, y, e)| format!("G1 X{x} Y{y} E{}", (e as f64) / 10.0)),
            (any::<i8>(), any::<i8>()).prop_map(|(x, y)| format!("G0 X{x} Y{y}")),
            (0i8..30).prop_map(|z| format!("G0 Z{}", (z as f64) / 10.0)),
            Just(";LAYER_CHANGE".to_string()),
            Just("; CHANGE_LAYER".to_string()),
            (0i8..30).prop_map(|z| format!(";Z:{}", (z as f64) / 10.0)),
            Just("M82".to_string()),
            Just("M83".to_string()),
            Just("G92 E0".to_string()),
            prop_oneof![
                Just(";TYPE:WALL-OUTER"),
                Just(";TYPE:FILL"),
                Just(";TYPE:Solid infill"),
                Just("; FEATURE: Outer wall"),
                Just(";TYPE:Skirt/Brim"),
                Just(";TYPE:Bogus"),
            ]
            .prop_map(|s| s.to_string()),
            (10i8..80).prop_map(|w| format!(";WIDTH:0.{w:02}")),
            Just("G2 X1 Y1 I0.5".to_string()),
            Just("M104 S200".to_string()),
            "[\\s\\S]{0,20}".prop_map(|s| s),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn arbitrary_text_never_panics_and_holds_invariants(s in "[\\s\\S]{0,300}") {
            check_invariants(&parse(&s));
        }

        #[test]
        fn gcode_like_never_panics_and_holds_invariants(lines in prop::collection::vec(arb_line(), 0..60)) {
            check_invariants(&parse(&lines.join("\n")));
        }
    }
}
