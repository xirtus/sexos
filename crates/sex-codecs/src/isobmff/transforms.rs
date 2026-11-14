// SPDX-License-Identifier: GPL-3.0-only
//! ISO image transformations: rotation (`irot`), mirroring (`imir`), clean aperture (`clap`).

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;
use crate::image::alloc_pixels;

/// Applies counter-clockwise rotation (`irot`: 0 = 0°, 1 = 90° CCW, 2 = 180°, 3 = 270° CCW).
/// Returns (new_width, new_height, new_pixels).
pub fn apply_irot(src: &[u32], w: usize, h: usize, irot: u8) -> Result<(usize, usize, Vec<u32>), Error> {
    let mode = irot & 3;
    if mode == 0 {
        let mut out = alloc_pixels(src.len())?;
        out.copy_from_slice(src);
        return Ok((w, h, out));
    }

    let (ow, oh) = if mode == 1 || mode == 3 { (h, w) } else { (w, h) };
    let mut out = alloc_pixels(ow * oh)?;

    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match mode {
                1 => (y, ow - 1 - x),             // 90° CCW
                2 => (w - 1 - x, h - 1 - y),       // 180°
                _ => (oh - 1 - y, x),             // 270° CCW (90° CW)
            };
            out[ny * ow + nx] = src[y * w + x];
        }
    }

    Ok((ow, oh, out))
}

/// Applies image mirroring (`imir`: 0 = vertical, 1 = horizontal).
pub fn apply_imir(pixels: &mut [u32], w: usize, h: usize, axis: u8) {
    if axis == 1 {
        // Horizontal flip: reverse each row
        for y in 0..h {
            let row = &mut pixels[y * w..(y + 1) * w];
            row.reverse();
        }
    } else {
        // Vertical flip: swap rows top with bottom
        for y in 0..h / 2 {
            let top_start = y * w;
            let bot_start = (h - 1 - y) * w;
            for x in 0..w {
                pixels.swap(top_start + x, bot_start + x);
            }
        }
    }
}
