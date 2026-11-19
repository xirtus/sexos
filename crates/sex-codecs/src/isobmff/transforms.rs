// SPDX-License-Identifier: GPL-3.0-only
//! ISO image transformations: rotation (`irot`), mirroring (`imir`), clean aperture (`clap`).

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;
use crate::image::alloc_pixels;

/// Clean aperture definition for image cropping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanAperture {
    pub width_n: u32,
    pub width_d: u32,
    pub height_n: u32,
    pub height_d: u32,
    pub horiz_off_n: i32,
    pub horiz_off_d: u32,
    pub vert_off_n: i32,
    pub vert_off_d: u32,
}

/// Applies clean aperture (`clap`) cropping.
pub fn apply_clap(
    src: &[u32],
    w: usize,
    h: usize,
    clap: &CleanAperture,
) -> Result<(usize, usize, Vec<u32>), Error> {
    if clap.width_d == 0 || clap.height_d == 0 || clap.horiz_off_d == 0 || clap.vert_off_d == 0 {
        return Err(Error::CorruptData);
    }
    let crop_w = (clap.width_n / clap.width_d) as usize;
    let crop_h = (clap.height_n / clap.height_d) as usize;
    if crop_w == 0 || crop_h == 0 {
        return Err(Error::CorruptData);
    }

    let h_off = (clap.horiz_off_n as i64) / (clap.horiz_off_d as i64);
    let v_off = (clap.vert_off_n as i64) / (clap.vert_off_d as i64);

    let left = ((w as i64) - (crop_w as i64)) / 2 + h_off;
    let top = ((h as i64) - (crop_h as i64)) / 2 + v_off;

    if left < 0 || top < 0 {
        return Err(Error::CorruptData);
    }
    let left = left as usize;
    let top = top as usize;

    let right = left.checked_add(crop_w).ok_or(Error::CorruptData)?;
    let bottom = top.checked_add(crop_h).ok_or(Error::CorruptData)?;
    if right > w || bottom > h {
        return Err(Error::CorruptData);
    }

    let count = crop_w.checked_mul(crop_h).ok_or(Error::TooLarge)?;
    let mut out = alloc_pixels(count)?;

    for y in 0..crop_h {
        let src_row = (top + y) * w + left;
        let dst_row = y * crop_w;
        out[dst_row..dst_row + crop_w].copy_from_slice(&src[src_row..src_row + crop_w]);
    }

    Ok((crop_w, crop_h, out))
}

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
