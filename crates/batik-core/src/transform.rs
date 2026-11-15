// SPDX-License-Identifier: GPL-3.0-only
//! Document and layer transforms: rotation, flip, bilinear scaling, and affine transforms.

use alloc::vec;
use alloc::vec::Vec;
use crate::blend::{pack_argb, split_argb};
use crate::error::BatikError;

/// Flips an image buffer horizontally in place.
pub fn flip_horizontal(pixels: &mut [u32], width: usize, height: usize) {
    if width <= 1 || height == 0 || pixels.len() < width * height {
        return;
    }
    for y in 0..height {
        let row_start = y * width;
        let mut left = 0;
        let mut right = width - 1;
        while left < right {
            pixels.swap(row_start + left, row_start + right);
            left += 1;
            right -= 1;
        }
    }
}

/// Flips an image buffer vertically in place.
pub fn flip_vertical(pixels: &mut [u32], width: usize, height: usize) {
    if height <= 1 || width == 0 || pixels.len() < width * height {
        return;
    }
    let mut top = 0;
    let mut bottom = height - 1;
    while top < bottom {
        let top_start = top * width;
        let bot_start = bottom * width;
        for x in 0..width {
            pixels.swap(top_start + x, bot_start + x);
        }
        top += 1;
        bottom -= 1;
    }
}

/// Rotates an image 90 degrees clockwise.
pub fn rotate_90_cw(pixels: &[u32], width: usize, height: usize) -> Vec<u32> {
    if width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }
    let mut out = vec![0u32; width * height];
    for y in 0..height {
        let row_offset = y * width;
        for x in 0..width {
            let new_x = height - 1 - y;
            let new_y = x;
            out[new_y * height + new_x] = pixels[row_offset + x];
        }
    }
    out
}

/// Rotates an image 90 degrees counter-clockwise.
pub fn rotate_90_ccw(pixels: &[u32], width: usize, height: usize) -> Vec<u32> {
    if width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }
    let mut out = vec![0u32; width * height];
    for y in 0..height {
        let row_offset = y * width;
        for x in 0..width {
            let new_x = y;
            let new_y = width - 1 - x;
            out[new_y * height + new_x] = pixels[row_offset + x];
        }
    }
    out
}

/// Rotates an image 180 degrees in place.
pub fn rotate_180(pixels: &mut [u32], width: usize, height: usize) {
    let len = width * height;
    if len == 0 || pixels.len() < len {
        return;
    }
    pixels[..len].reverse();
}

/// Samples a pixel buffer at fractional coordinate `(sx, sy)` using bilinear filtering.
pub fn sample_bilinear(src: &[u32], width: usize, height: usize, sx: f32, sy: f32) -> u32 {
    if !sx.is_finite() || !sy.is_finite() || sx < 0.0 || sy < 0.0 || sx >= width as f32 || sy >= height as f32 {
        return 0;
    }

    let x0 = libm::floorf(sx) as usize;
    let y0 = libm::floorf(sy) as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);

    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;

    let p00 = src[y0 * width + x0];
    let p10 = src[y0 * width + x1];
    let p01 = src[y1 * width + x0];
    let p11 = src[y1 * width + x1];

    let (a00, r00, g00, b00) = split_argb(p00);
    let (a10, r10, g10, b10) = split_argb(p10);
    let (a01, r01, g01, b01) = split_argb(p01);
    let (a11, r11, g11, b11) = split_argb(p11);

    let bilerp = |c00: u8, c10: u8, c01: u8, c11: u8| -> u8 {
        let top = c00 as f32 + fx * (c10 as f32 - c00 as f32);
        let bot = c01 as f32 + fx * (c11 as f32 - c01 as f32);
        let val = top + fy * (bot - top);
        libm::roundf(val).clamp(0.0, 255.0) as u8
    };

    pack_argb(
        bilerp(a00, a10, a01, a11),
        bilerp(r00, r10, r01, r11),
        bilerp(g00, g10, g01, g11),
        bilerp(b00, b10, b01, b11),
    )
}

/// Resizes a pixel buffer to `(dst_w, dst_h)` using bilinear interpolation.
pub fn resize_bilinear(
    src: &[u32],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Result<Vec<u32>, BatikError> {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Err(BatikError::InvalidDimensions);
    }
    let total_pixels = dst_w.checked_mul(dst_h).ok_or(BatikError::TooLarge)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total_pixels).map_err(|_| BatikError::OutOfMemory)?;

    let x_scale = src_w as f32 / dst_w as f32;
    let y_scale = src_h as f32 / dst_h as f32;

    for dy in 0..dst_h {
        let sy = (dy as f32 + 0.5) * y_scale - 0.5;
        let clamped_sy = sy.clamp(0.0, (src_h - 1) as f32);

        for dx in 0..dst_w {
            let sx = (dx as f32 + 0.5) * x_scale - 0.5;
            let clamped_sx = sx.clamp(0.0, (src_w - 1) as f32);
            out.push(sample_bilinear(src, src_w, src_h, clamped_sx, clamped_sy));
        }
    }

    Ok(out)
}

/// Performs a 2D affine transformation `[a, b, c, d, tx, ty]` with bilinear sampling.
///
/// Maps source coordinates `[sx, sy]` to destination `[dx, dy]` via:
/// `dx = a * sx + c * sy + tx`
/// `dy = b * sx + d * sy + ty`
pub fn transform_affine(
    src: &[u32],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
    matrix: [f32; 6],
) -> Result<Vec<u32>, BatikError> {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Err(BatikError::InvalidDimensions);
    }
    for val in matrix {
        if !val.is_finite() {
            return Err(BatikError::InvalidParameter("non-finite transform matrix"));
        }
    }
    let [a, b, c, d, tx, ty] = matrix;
    let det = a * d - b * c;
    if !det.is_finite() || libm::fabsf(det) < 1e-6 {
        return Err(BatikError::InvalidParameter("singular transform matrix"));
    }

    let inv_det = 1.0 / det;
    let ia = d * inv_det;
    let ib = -b * inv_det;
    let ic = -c * inv_det;
    let id = a * inv_det;

    let total_pixels = dst_w.checked_mul(dst_h).ok_or(BatikError::TooLarge)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total_pixels).map_err(|_| BatikError::OutOfMemory)?;

    for dy in 0..dst_h {
        let y_diff = dy as f32 + 0.5 - ty;
        for dx in 0..dst_w {
            let x_diff = dx as f32 + 0.5 - tx;
            let sx = ia * x_diff + ic * y_diff - 0.5;
            let sy = ib * x_diff + id * y_diff - 0.5;

            if sx >= -0.5 && sx < src_w as f32 && sy >= -0.5 && sy < src_h as f32 {
                let clamped_sx = sx.max(0.0);
                let clamped_sy = sy.max(0.0);
                out.push(sample_bilinear(src, src_w, src_h, clamped_sx, clamped_sy));
            } else {
                out.push(0);
            }
        }
    }

    Ok(out)
}
