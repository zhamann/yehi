//! Contact sheets: frames in a grid, each labeled with its time in a tiny built-in pixel font so the
//! sheet needs no installed fonts.

use image::{RgbaImage, imageops};

const GAP: u32 = 8;
const BG: [u8; 4] = [24, 24, 27, 255];

/// 3x5 glyphs for the label characters, one row per u8 (3 low bits, left to right).
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        '.' => [0, 0, 0, 0, 2],
        's' => [0, 3, 6, 1, 6],
        _ => [0; 5],
    }
}

fn label(img: &mut RgbaImage, x: u32, y: u32, text: &str, scale: u32) {
    let w = text.len() as u32 * 4 * scale + scale;
    let h = 7 * scale;
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, image::Rgba([0, 0, 0, 200]));
        }
    }
    for (i, c) in text.chars().enumerate() {
        let g = glyph(c);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 == 1 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = x + scale + (i as u32 * 4 + col) * scale + dx;
                            let py = y + scale + row as u32 * scale + dy;
                            if px < img.width() && py < img.height() {
                                img.put_pixel(px, py, image::Rgba([255, 255, 255, 255]));
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn compose(frames: &[(f64, RgbaImage)], width: u32) -> RgbaImage {
    let (fw, fh) = frames[0].1.dimensions();
    let th = (fh as f64 * width as f64 / fw as f64).round() as u32;
    let n = frames.len() as u32;
    // aim for a roughly square sheet
    let mut cols = 1;
    while cols < n && (cols * (width + GAP)) < n.div_ceil(cols) * (th + GAP) {
        cols += 1;
    }
    let rows = n.div_ceil(cols);
    let mut out = RgbaImage::from_pixel(cols * (width + GAP) + GAP, rows * (th + GAP) + GAP, image::Rgba(BG));
    let scale = (width / 90).max(2);
    for (k, (t, img)) in frames.iter().enumerate() {
        let thumb = imageops::resize(img, width, th, imageops::FilterType::Triangle);
        let (x, y) = (GAP + (k as u32 % cols) * (width + GAP), GAP + (k as u32 / cols) * (th + GAP));
        imageops::overlay(&mut out, &thumb, x as i64, y as i64);
        label(&mut out, x, y, &format!("{t:.2}s"), scale);
    }
    out
}
