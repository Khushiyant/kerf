//! Render deposited-material occupancy to PNG.
//!
//! Diff colours: grey = in both, red = only in A, blue = only in B.

use std::collections::BTreeSet;

use kerf_core::{denote_lo, parse};

type Cell = (i64, i64);

const WHITE: [u8; 3] = [255, 255, 255];
const SHARED: [u8; 3] = [160, 160, 160];
const ONLY_A: [u8; 3] = [220, 40, 40];
const ONLY_B: [u8; 3] = [40, 80, 220];

/// Render the diff of two occupancy cell-sets to a PNG (`max_px` bounds each dimension).
pub fn render_diff_png(a: &BTreeSet<Cell>, b: &BTreeSet<Cell>, max_px: u32) -> Vec<u8> {
    let all: BTreeSet<Cell> = a.union(b).copied().collect();
    let Some((minx, maxx, miny, maxy)) = bbox(&all) else {
        return encode(1, 1, vec![255, 255, 255]);
    };
    let cells_w = (maxx - minx + 1) as u64;
    let cells_h = (maxy - miny + 1) as u64;
    let w = (cells_w as u32).clamp(1, max_px);
    let h = (cells_h as u32).clamp(1, max_px);

    let mut pixels = vec![0u8; (w as usize) * (h as usize) * 3];
    for chunk in pixels.chunks_exact_mut(3) {
        chunk.copy_from_slice(&WHITE);
    }
    for &(x, y) in &all {
        let px = scale((x - minx) as u64, cells_w, w);
        let py = h - 1 - scale((y - miny) as u64, cells_h, h); // flip so +y is up
        let color = match (a.contains(&(x, y)), b.contains(&(x, y))) {
            (true, true) => SHARED,
            (true, false) => ONLY_A,
            _ => ONLY_B,
        };
        let idx = ((py as usize) * (w as usize) + px as usize) * 3;
        pixels[idx..idx + 3].copy_from_slice(&color);
    }
    encode(w, h, pixels)
}

/// Render a single occupancy layer (all cells drawn as material).
pub fn render_occupancy_png(cells: &BTreeSet<Cell>, max_px: u32) -> Vec<u8> {
    render_diff_png(cells, cells, max_px)
}

/// Parse two G-code files, denote each, and render one diff PNG per shared layer height (sorted by Z).
pub fn diff_pngs_from_gcode(
    a: &str,
    b: &str,
    resolution_um: i64,
    max_px: u32,
) -> Vec<(i64, Vec<u8>)> {
    let occ_a = layers_by_z(a, resolution_um);
    let occ_b = layers_by_z(b, resolution_um);
    let mut zs: Vec<i64> = occ_a.iter().chain(occ_b.iter()).map(|(z, _)| *z).collect();
    zs.sort_unstable();
    zs.dedup();
    let empty = BTreeSet::new();
    zs.into_iter()
        .map(|z| {
            let ca = occ_a
                .iter()
                .find(|(zz, _)| *zz == z)
                .map(|(_, c)| c)
                .unwrap_or(&empty);
            let cb = occ_b
                .iter()
                .find(|(zz, _)| *zz == z)
                .map(|(_, c)| c)
                .unwrap_or(&empty);
            (z, render_diff_png(ca, cb, max_px))
        })
        .collect()
}

fn layers_by_z(gcode: &str, resolution_um: i64) -> Vec<(i64, BTreeSet<Cell>)> {
    denote_lo(&parse(gcode).program, resolution_um)
        .layers
        .into_iter()
        .map(|l| (l.z_um, l.cells))
        .collect()
}

fn bbox(cells: &BTreeSet<Cell>) -> Option<(i64, i64, i64, i64)> {
    let mut it = cells.iter();
    let &(x0, y0) = it.next()?;
    let (mut minx, mut maxx, mut miny, mut maxy) = (x0, x0, y0, y0);
    for &(x, y) in it {
        minx = minx.min(x);
        maxx = maxx.max(x);
        miny = miny.min(y);
        maxy = maxy.max(y);
    }
    Some((minx, maxx, miny, maxy))
}

/// Map a cell offset in `[0, span)` to a pixel in `[0, px)`.
fn scale(offset: u64, span: u64, px: u32) -> u32 {
    if px <= 1 || span <= 1 {
        0
    } else {
        (offset * (px as u64 - 1) / (span - 1)) as u32
    }
}

fn encode(w: u32, h: u32, rgb: Vec<u8>) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("png header");
        writer.write_image_data(&rgb).expect("png data");
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(png: &[u8]) -> (u32, u32) {
        let dec = png::Decoder::new(png);
        let reader = dec.read_info().unwrap();
        let info = reader.info();
        (info.width, info.height)
    }

    #[test]
    fn diff_renders_a_valid_png() {
        let a: BTreeSet<Cell> = [(0, 0), (1, 0), (2, 0), (0, 1)].into_iter().collect();
        let b: BTreeSet<Cell> = [(0, 0), (1, 0)].into_iter().collect();
        let png = render_diff_png(&a, &b, 64);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let (w, h) = decode(&png);
        assert_eq!((w, h), (3, 2)); // union bbox
    }

    #[test]
    fn empty_diff_is_a_blank_png() {
        let e = BTreeSet::new();
        let png = render_diff_png(&e, &e, 64);
        assert_eq!(decode(&png), (1, 1));
    }

    #[test]
    fn diff_from_gcode_produces_one_png_per_layer() {
        let a = "M83\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:Perimeter\nG1 X10 Y0 E.4\nG1 X10 Y10 E.4\n;LAYER_CHANGE\n;Z:0.4\nG1 X10 Y0 E.4";
        let b = a.replace("X10 Y10", "X10 Y40"); // layer 0 differs
        let pngs = diff_pngs_from_gcode(a, &b, 200, 128);
        assert_eq!(pngs.len(), 2);
        for (_, png) in &pngs {
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        }
    }
}
