// SPDX-License-Identifier: GPL-3.0-only
//! Spatial image filters: blur, unsharp mask, and pixelate.

use alloc::vec;
use alloc::vec::Vec;
use crate::blend::{pack_argb, split_argb};
use crate::selection::SelectionMask;

/// Applies a 2-pass separable box blur.
pub fn box_blur(pixels: &[u32], width: usize, height: usize, radius: usize) -> Vec<u32> {
    if radius == 0 || width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }

    let mut temp = vec![0u32; width * height];
    let mut out = vec![0u32; width * height];
    let window_size = (2 * radius + 1) as u32;

    // Horizontal pass
    for y in 0..height {
        let row_offset = y * width;
        let mut sa: u32 = 0;
        let mut sr: u32 = 0;
        let mut sg: u32 = 0;
        let mut sb: u32 = 0;

        for x in 0..=radius {
            let p = pixels[row_offset + x.min(width - 1)];
            let (a, r, g, b) = split_argb(p);
            sa += a as u32;
            sr += r as u32;
            sg += g as u32;
            sb += b as u32;
        }
        let first_p = pixels[row_offset];
        let (fa, fr, fg, fb) = split_argb(first_p);
        sa += fa as u32 * radius as u32;
        sr += fr as u32 * radius as u32;
        sg += fg as u32 * radius as u32;
        sb += fb as u32 * radius as u32;

        for x in 0..width {
            temp[row_offset + x] = pack_argb(
                (sa / window_size) as u8,
                (sr / window_size) as u8,
                (sg / window_size) as u8,
                (sb / window_size) as u8,
            );

            let left_x = (x as isize - radius as isize).max(0) as usize;
            let right_x = (x + radius + 1).min(width - 1);
            let (la, lr, lg, lb) = split_argb(pixels[row_offset + left_x]);
            let (ra, rr, rg, rb) = split_argb(pixels[row_offset + right_x]);

            sa = sa.saturating_sub(la as u32).saturating_add(ra as u32);
            sr = sr.saturating_sub(lr as u32).saturating_add(rr as u32);
            sg = sg.saturating_sub(lg as u32).saturating_add(rg as u32);
            sb = sb.saturating_sub(lb as u32).saturating_add(rb as u32);
        }
    }

    // Vertical pass
    for x in 0..width {
        let mut sa: u32 = 0;
        let mut sr: u32 = 0;
        let mut sg: u32 = 0;
        let mut sb: u32 = 0;

        for y in 0..=radius {
            let p = temp[y.min(height - 1) * width + x];
            let (a, r, g, b) = split_argb(p);
            sa += a as u32;
            sr += r as u32;
            sg += g as u32;
            sb += b as u32;
        }
        let first_p = temp[x];
        let (fa, fr, fg, fb) = split_argb(first_p);
        sa += fa as u32 * radius as u32;
        sr += fr as u32 * radius as u32;
        sg += fg as u32 * radius as u32;
        sb += fb as u32 * radius as u32;

        for y in 0..height {
            out[y * width + x] = pack_argb(
                (sa / window_size) as u8,
                (sr / window_size) as u8,
                (sg / window_size) as u8,
                (sb / window_size) as u8,
            );

            let top_y = (y as isize - radius as isize).max(0) as usize;
            let bot_y = (y + radius + 1).min(height - 1);
            let (ta, tr, tg, tb) = split_argb(temp[top_y * width + x]);
            let (ba, br, bg, bb) = split_argb(temp[bot_y * width + x]);

            sa = sa.saturating_sub(ta as u32).saturating_add(ba as u32);
            sr = sr.saturating_sub(tr as u32).saturating_add(br as u32);
            sg = sg.saturating_sub(tg as u32).saturating_add(bg as u32);
            sb = sb.saturating_sub(tb as u32).saturating_add(bb as u32);
        }
    }

    out
}

/// Applies a 2-pass separable Gaussian blur with standard deviation `sigma`.
pub fn gaussian_blur(pixels: &[u32], width: usize, height: usize, sigma: f32) -> Vec<u32> {
    if !sigma.is_finite() || sigma <= 0.1 || width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }

    let max_radius = width.max(height).min(256);
    let radius = (libm::ceilf(sigma * 3.0) as usize).clamp(1, max_radius);
    let kernel_len = 2 * radius + 1;
    let mut kernel = vec![0.0f32; kernel_len];
    let two_sigma_sq = 2.0 * sigma * sigma;
    let mut sum = 0.0f32;

    for i in 0..kernel_len {
        let x = (i as isize - radius as isize) as f32;
        let w = libm::expf(-(x * x) / two_sigma_sq);
        kernel[i] = w;
        sum += w;
    }
    for w in kernel.iter_mut() {
        *w /= sum;
    }

    let mut temp = vec![0u32; width * height];
    let mut out = vec![0u32; width * height];

    // Horizontal pass
    for y in 0..height {
        let row_offset = y * width;
        for x in 0..width {
            let mut acc_a = 0.0f32;
            let mut acc_r = 0.0f32;
            let mut acc_g = 0.0f32;
            let mut acc_b = 0.0f32;

            for (k_idx, &k_weight) in kernel.iter().enumerate() {
                let sample_x = (x as isize + k_idx as isize - radius as isize)
                    .clamp(0, width as isize - 1) as usize;
                let (a, r, g, b) = split_argb(pixels[row_offset + sample_x]);
                acc_a += a as f32 * k_weight;
                acc_r += r as f32 * k_weight;
                acc_g += g as f32 * k_weight;
                acc_b += b as f32 * k_weight;
            }

            temp[row_offset + x] = pack_argb(
                libm::roundf(acc_a).clamp(0.0, 255.0) as u8,
                libm::roundf(acc_r).clamp(0.0, 255.0) as u8,
                libm::roundf(acc_g).clamp(0.0, 255.0) as u8,
                libm::roundf(acc_b).clamp(0.0, 255.0) as u8,
            );
        }
    }

    // Vertical pass
    for y in 0..height {
        for x in 0..width {
            let mut acc_a = 0.0f32;
            let mut acc_r = 0.0f32;
            let mut acc_g = 0.0f32;
            let mut acc_b = 0.0f32;

            for (k_idx, &k_weight) in kernel.iter().enumerate() {
                let sample_y = (y as isize + k_idx as isize - radius as isize)
                    .clamp(0, height as isize - 1) as usize;
                let (a, r, g, b) = split_argb(temp[sample_y * width + x]);
                acc_a += a as f32 * k_weight;
                acc_r += r as f32 * k_weight;
                acc_g += g as f32 * k_weight;
                acc_b += b as f32 * k_weight;
            }

            out[y * width + x] = pack_argb(
                libm::roundf(acc_a).clamp(0.0, 255.0) as u8,
                libm::roundf(acc_r).clamp(0.0, 255.0) as u8,
                libm::roundf(acc_g).clamp(0.0, 255.0) as u8,
                libm::roundf(acc_b).clamp(0.0, 255.0) as u8,
            );
        }
    }

    out
}

/// Applies unsharp mask sharpening: original + amount * (original - blurred).
pub fn unsharp_mask(
    pixels: &[u32],
    width: usize,
    height: usize,
    sigma: f32,
    amount: f32,
    threshold: u8,
) -> Vec<u32> {
    if !sigma.is_finite() || !amount.is_finite() || width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }
    let blurred = gaussian_blur(pixels, width, height, sigma);
    let mut out = vec![0u32; pixels.len()];

    for i in 0..pixels.len() {
        let (oa, or, og, ob) = split_argb(pixels[i]);
        let (_ba, br, bg, bb) = split_argb(blurred[i]);

        let sharpen = |orig: u8, blur: u8| -> u8 {
            let diff = orig as f32 - blur as f32;
            if libm::fabsf(diff) >= threshold as f32 {
                libm::roundf(orig as f32 + amount * diff).clamp(0.0, 255.0) as u8
            } else {
                orig
            }
        };

        out[i] = pack_argb(oa, sharpen(or, br), sharpen(og, bg), sharpen(ob, bb));
    }
    out
}

/// Downsamples the image in blocks of `block_size` x `block_size` filled with average color.
pub fn pixelate(pixels: &[u32], width: usize, height: usize, block_size: usize) -> Vec<u32> {
    if block_size <= 1 || width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }

    let mut out = vec![0u32; width * height];
    let bs = block_size;

    for by in (0..height).step_by(bs) {
        let ey = (by + bs).min(height);
        for bx in (0..width).step_by(bs) {
            let ex = (bx + bs).min(width);

            let mut sa: u32 = 0;
            let mut sr: u32 = 0;
            let mut sg: u32 = 0;
            let mut sb: u32 = 0;
            let mut count: u32 = 0;

            for y in by..ey {
                let row_offset = y * width;
                for x in bx..ex {
                    let (a, r, g, b) = split_argb(pixels[row_offset + x]);
                    sa += a as u32;
                    sr += r as u32;
                    sg += g as u32;
                    sb += b as u32;
                    count += 1;
                }
            }

            if count > 0 {
                let avg = pack_argb(
                    (sa / count) as u8,
                    (sr / count) as u8,
                    (sg / count) as u8,
                    (sb / count) as u8,
                );
                for y in by..ey {
                    let row_offset = y * width;
                    for x in bx..ex {
                        out[row_offset + x] = avg;
                    }
                }
            }
        }
    }

    out
}

/// Blends filtered pixels back into destination based on optional selection mask.
pub fn blend_with_mask(
    original: &[u32],
    filtered: &[u32],
    out: &mut [u32],
    width: usize,
    mask: Option<&SelectionMask>,
) {
    if width == 0 {
        return;
    }
    for (i, (&orig, &filt)) in original.iter().zip(filtered.iter()).enumerate() {
        let (x, y) = (i % width, i / width);
        let weight = match mask {
            Some(m) => m.sample(x, y),
            None => 255,
        };

        if weight == 0 {
            out[i] = orig;
        } else if weight == 255 {
            out[i] = filt;
        } else {
            let (oa, or, og, ob) = split_argb(orig);
            let (fa, fr, fg, fb) = split_argb(filt);
            let w = weight as f32 / 255.0;
            out[i] = pack_argb(
                libm::roundf(oa as f32 + w * (fa as f32 - oa as f32)) as u8,
                libm::roundf(or as f32 + w * (fr as f32 - or as f32)) as u8,
                libm::roundf(og as f32 + w * (fg as f32 - og as f32)) as u8,
                libm::roundf(ob as f32 + w * (fb as f32 - ob as f32)) as u8,
            );
        }
    }
}

/// 3x3 Sharpen filter.
pub fn sharpen(pixels: &[u32], width: usize, height: usize) -> Vec<u32> {
    if width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }
    let mut out = pixels.to_vec();
    for y in 0..height {
        let row_offset = y * width;
        let y_prev = y.saturating_sub(1);
        let y_next = (y + 1).min(height - 1);
        for x in 0..width {
            let x_prev = x.saturating_sub(1);
            let x_next = (x + 1).min(width - 1);

            let (ca, cr, cg, cb) = split_argb(pixels[row_offset + x]);
            let (_, tr, tg, tb) = split_argb(pixels[y_prev * width + x]);
            let (_, br, bg, bb) = split_argb(pixels[y_next * width + x]);
            let (_, lr, lg, lb) = split_argb(pixels[row_offset + x_prev]);
            let (_, rr, rg, rb) = split_argb(pixels[row_offset + x_next]);

            let calc = |c: u8, t: u8, b: u8, l: u8, r: u8| -> u8 {
                let v = 5 * (c as i32) - (t as i32) - (b as i32) - (l as i32) - (r as i32);
                v.clamp(0, 255) as u8
            };

            out[row_offset + x] = pack_argb(
                ca,
                calc(cr, tr, br, lr, rr),
                calc(cg, tg, bg, lg, rg),
                calc(cb, tb, bb, lb, rb),
            );
        }
    }
    out
}

/// 3x3 Emboss filter with 128 neutral bias.
pub fn emboss(pixels: &[u32], width: usize, height: usize) -> Vec<u32> {
    if width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }
    let mut out = pixels.to_vec();
    for y in 0..height {
        let row_offset = y * width;
        let y_prev = y.saturating_sub(1);
        let y_next = (y + 1).min(height - 1);
        for x in 0..width {
            let x_prev = x.saturating_sub(1);
            let x_next = (x + 1).min(width - 1);

            let (ca, _, _, _) = split_argb(pixels[row_offset + x]);
            let (_, tr, tg, tb) = split_argb(pixels[y_prev * width + x_prev]);
            let (_, br, bg, bb) = split_argb(pixels[y_next * width + x_next]);

            let calc = |tl: u8, br: u8| -> u8 {
                let v = (tl as i32) - (br as i32) + 128;
                v.clamp(0, 255) as u8
            };

            out[row_offset + x] = pack_argb(ca, calc(tr, br), calc(tg, bg), calc(tb, bb));
        }
    }
    out
}

/// 3x3 Edge detect (Laplacian filter).
pub fn edge_detect(pixels: &[u32], width: usize, height: usize) -> Vec<u32> {
    if width == 0 || height == 0 || pixels.len() < width * height {
        return pixels.to_vec();
    }
    let mut out = pixels.to_vec();
    for y in 0..height {
        let row_offset = y * width;
        let y_prev = y.saturating_sub(1);
        let y_next = (y + 1).min(height - 1);
        for x in 0..width {
            let x_prev = x.saturating_sub(1);
            let x_next = (x + 1).min(width - 1);

            let (ca, cr, cg, cb) = split_argb(pixels[row_offset + x]);
            let (_, tr, tg, tb) = split_argb(pixels[y_prev * width + x]);
            let (_, br, bg, bb) = split_argb(pixels[y_next * width + x]);
            let (_, lr, lg, lb) = split_argb(pixels[row_offset + x_prev]);
            let (_, rr, rg, rb) = split_argb(pixels[row_offset + x_next]);
            let (_, tlr, tlg, tlb) = split_argb(pixels[y_prev * width + x_prev]);
            let (_, trr, trg, trb) = split_argb(pixels[y_prev * width + x_next]);
            let (_, blr, blg, blb) = split_argb(pixels[y_next * width + x_prev]);
            let (_, brr, brg, brb) = split_argb(pixels[y_next * width + x_next]);

            let calc = |c: u8, t: u8, b: u8, l: u8, r: u8, tl: u8, tr: u8, bl: u8, br_val: u8| -> u8 {
                let sum = 8 * (c as i32)
                    - (t as i32)
                    - (b as i32)
                    - (l as i32)
                    - (r as i32)
                    - (tl as i32)
                    - (tr as i32)
                    - (bl as i32)
                    - (br_val as i32);
                sum.clamp(0, 255) as u8
            };

            out[row_offset + x] = pack_argb(
                ca,
                calc(cr, tr, br, lr, rr, tlr, trr, blr, brr),
                calc(cg, tg, bg, lg, rg, tlg, trg, blg, brg),
                calc(cb, tb, bb, lb, rb, tlb, trb, blb, brb),
            );
        }
    }
    out
}

