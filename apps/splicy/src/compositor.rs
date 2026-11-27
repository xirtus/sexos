// SPDX-License-Identifier: GPL-3.0-only
//! CPU Compositor with quarter-res playback and full-res paused quality modes.
//!
//! Applies layer transforms (scale, offset, opacity) and blend modes in pure
//! integer 0x00RRGGBB arithmetic without floating point.

use alloc::vec::Vec;

/// Compositing quality mode: quarter resolution during playback, full resolution when paused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityMode {
    PlaybackQuarter,
    PausedFull,
}

impl QualityMode {
    /// Scale divisor: 4 for PlaybackQuarter, 1 for PausedFull.
    pub const fn scale_divisor(&self) -> usize {
        match self {
            QualityMode::PlaybackQuarter => 4,
            QualityMode::PausedFull => 1,
        }
    }
}

/// Blending modes supported by the compositor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Add,
}

/// Transformation and blending properties for a composited layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerTransform {
    pub offset_x: i32,
    pub offset_y: i32,
    pub scale_per_mille: u32,
    pub opacity_per_mille: u32,
    pub blend: BlendMode,
}

impl Default for LayerTransform {
    fn default() -> Self {
        Self {
            offset_x: 0,
            offset_y: 0,
            scale_per_mille: 1000,
            opacity_per_mille: 1000,
            blend: BlendMode::Normal,
        }
    }
}

/// One input layer to composite.
pub struct LayerInput<'a> {
    pub pixels: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub transform: LayerTransform,
}

/// Output buffer resulting from CPU compositing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeOutput {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

fn blend_channel(d: u32, s: u32, mode: BlendMode) -> u32 {
    let res = match mode {
        BlendMode::Normal => s,
        BlendMode::Multiply => (s * d) / 255,
        BlendMode::Screen => s + d - (s * d) / 255,
        BlendMode::Overlay => {
            if d < 128 {
                (2 * s * d) / 255
            } else {
                255 - (2 * (255 - s) * (255 - d)) / 255
            }
        }
        BlendMode::Add => (s + d).min(255),
    };
    res.min(255)
}

fn blend_pixel(dst: u32, src: u32, mode: BlendMode, opacity_per_mille: u32) -> u32 {
    let alpha = (opacity_per_mille.min(1000) * 255) / 1000;
    if alpha == 0 {
        return dst;
    }
    let r = blend_channel((dst >> 16) & 0xff, (src >> 16) & 0xff, mode);
    let g = blend_channel((dst >> 8) & 0xff, (src >> 8) & 0xff, mode);
    let b = blend_channel(dst & 0xff, src & 0xff, mode);
    let blended = (r << 16) | (g << 8) | b;
    sex_office::mix(dst, blended, alpha)
}

/// Composite layers onto a cleared canvas at full or quarter resolution.
pub fn composite_cpu(
    out_w: usize,
    out_h: usize,
    mode: QualityMode,
    clear_rgb: u32,
    layers: &[LayerInput<'_>],
) -> CompositeOutput {
    let div = mode.scale_divisor();
    let target_w = (out_w / div).max(1);
    let target_h = (out_h / div).max(1);
    let mut pixels = alloc::vec![clear_rgb & 0x00ffffff; target_w * target_h];

    for layer in layers {
        if layer.width == 0 || layer.height == 0 || layer.pixels.is_empty() {
            continue;
        }
        let opacity = layer.transform.opacity_per_mille.min(1000);
        if opacity == 0 {
            continue;
        }
        let scale = layer.transform.scale_per_mille;
        if scale == 0 {
            continue;
        }

        let layer_fw = (layer.width as i64 * scale as i64) / 1000;
        let layer_fh = (layer.height as i64 * scale as i64) / 1000;
        if layer_fw <= 0 || layer_fh <= 0 {
            continue;
        }

        let fx_start = layer.transform.offset_x;
        let fy_start = layer.transform.offset_y;
        let fx_end = fx_start + layer_fw as i32;
        let fy_end = fy_start + layer_fh as i32;

        let div_i32 = div as i32;
        let tx_min = if fx_start < 0 { 0 } else { (fx_start as usize / div).min(target_w) };
        let tx_max = if fx_end < 0 { 0 } else { ((fx_end as usize + div - 1) / div).min(target_w) };
        let ty_min = if fy_start < 0 { 0 } else { (fy_start as usize / div).min(target_h) };
        let ty_max = if fy_end < 0 { 0 } else { ((fy_end as usize + div - 1) / div).min(target_h) };

        for ty in ty_min..ty_max {
            let fy = ty as i32 * div_i32;
            if fy < fy_start || fy >= fy_end {
                continue;
            }
            let sy = ((fy - fy_start) as i64 * 1000 / scale as i64) as usize;
            if sy >= layer.height {
                continue;
            }
            let row_offset = ty * target_w;
            for tx in tx_min..tx_max {
                let fx = tx as i32 * div_i32;
                if fx < fx_start || fx >= fx_end {
                    continue;
                }
                let sx = ((fx - fx_start) as i64 * 1000 / scale as i64) as usize;
                if sx >= layer.width {
                    continue;
                }
                let src_pixel = layer.pixels[sy * layer.width + sx];
                let dst_idx = row_offset + tx;
                pixels[dst_idx] = blend_pixel(pixels[dst_idx], src_pixel, layer.transform.blend, opacity);
            }
        }
    }

    CompositeOutput {
        width: target_w,
        height: target_h,
        pixels,
    }
}

/// Compute exact letterboxed/pillarboxed `(x, y, w, h)` honoring non-square pixel
/// sample aspect ratio (`display_w = src_w * sar_num / sar_den`), using pure integer math.
pub fn fit_anamorphic_rect(
    src_w: usize,
    src_h: usize,
    sar_num: u32,
    sar_den: u32,
    dst_w: usize,
    dst_h: usize,
) -> (usize, usize, usize, usize) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (0, 0, 0, 0);
    }
    let (sn, sd) = if sar_num == 0 || sar_den == 0 {
        (1u128, 1u128)
    } else {
        (sar_num as u128, sar_den as u128)
    };
    let sw = src_w as u128;
    let sh = src_h as u128;
    let dw = dst_w as u128;
    let dh = dst_h as u128;

    let lhs = sw * sn * dh;
    let rhs = dw * sh * sd;

    if lhs > rhs {
        // Display aspect is wider than destination: letterbox (fit width).
        let fit_w = dst_w;
        let fit_h = ((dw * sh * sd) / (sw * sn)).clamp(1, dh) as usize;
        let y = (dst_h - fit_h) / 2;
        (0, y, fit_w, fit_h)
    } else {
        // Display aspect is taller or equal: pillarbox (fit height).
        let fit_h = dst_h;
        let fit_w = ((dh * sw * sn) / (sh * sd)).clamp(1, dw) as usize;
        let x = (dst_w - fit_w) / 2;
        (x, 0, fit_w, fit_h)
    }
}

/// Smooth 16.16 fixed-point bilinear subpixel pan-and-zoom sampler for Ken Burns motion.
///
/// `center_x_q16` and `center_y_q16` are normalized `Q16` coordinates (`32768` = center `0.5`).
/// `zoom_per_mille` is `1000` for `1.0x`, `2000` for `2.0x`, etc.
pub fn sample_ken_burns_q16(
    src: &[u32],
    src_w: usize,
    src_h: usize,
    out_w: usize,
    out_h: usize,
    center_x_q16: i32,
    center_y_q16: i32,
    zoom_per_mille: u32,
) -> Vec<u32> {
    if out_w == 0 || out_h == 0 {
        return Vec::new();
    }
    if src_w == 0 || src_h == 0 || src.len() < src_w * src_h {
        return alloc::vec![0u32; out_w * out_h];
    }

    let zoom = zoom_per_mille.max(1) as i64;
    let span_w_q16 = ((src_w as i64) << 16) * 1000 / zoom;
    let span_h_q16 = ((src_h as i64) << 16) * 1000 / zoom;
    let center_px_x_q16 = center_x_q16 as i64 * src_w as i64;
    let center_px_y_q16 = center_y_q16 as i64 * src_h as i64;
    let max_x_q16 = ((src_w - 1) as i64) << 16;
    let max_y_q16 = ((src_h - 1) as i64) << 16;

    let mut out = Vec::with_capacity(out_w * out_h);
    let ow = out_w as i64;
    let oh = out_h as i64;

    for oy in 0..out_h {
        let v_q16 = ((2 * oy as i64 + 1 - oh) << 15) / oh;
        let sy_q16 = (center_px_y_q16 + ((v_q16 * span_h_q16) >> 16) - 32768).clamp(0, max_y_q16);
        let y0 = (sy_q16 >> 16) as usize;
        let y1 = (y0 + 1).min(src_h - 1);
        let fy = (sy_q16 & 0xffff) as u64;
        let inv_fy = 65536 - fy;
        let row0 = y0 * src_w;
        let row1 = y1 * src_w;

        for ox in 0..out_w {
            let u_q16 = ((2 * ox as i64 + 1 - ow) << 15) / ow;
            let sx_q16 = (center_px_x_q16 + ((u_q16 * span_w_q16) >> 16) - 32768).clamp(0, max_x_q16);
            let x0 = (sx_q16 >> 16) as usize;
            let x1 = (x0 + 1).min(src_w - 1);
            let fx = (sx_q16 & 0xffff) as u64;
            let inv_fx = 65536 - fx;

            let p00 = src[row0 + x0];
            let p10 = src[row0 + x1];
            let p01 = src[row1 + x0];
            let p11 = src[row1 + x1];

            let w00 = inv_fx * inv_fy;
            let w10 = fx * inv_fy;
            let w01 = inv_fx * fy;
            let w11 = fx * fy;

            let r = (((((p00 >> 16) & 0xff) as u64 * w00
                + ((p10 >> 16) & 0xff) as u64 * w10
                + ((p01 >> 16) & 0xff) as u64 * w01
                + ((p11 >> 16) & 0xff) as u64 * w11
                + (1u64 << 31))
                >> 32)
                .min(255)) as u32;
            let g = (((((p00 >> 8) & 0xff) as u64 * w00
                + ((p10 >> 8) & 0xff) as u64 * w10
                + ((p01 >> 8) & 0xff) as u64 * w01
                + ((p11 >> 8) & 0xff) as u64 * w11
                + (1u64 << 31))
                >> 32)
                .min(255)) as u32;
            let b = ((((p00 & 0xff) as u64 * w00
                + (p10 & 0xff) as u64 * w10
                + (p01 & 0xff) as u64 * w01
                + (p11 & 0xff) as u64 * w11
                + (1u64 << 31))
                >> 32)
                .min(255)) as u32;

            out.push((r << 16) | (g << 8) | b);
        }
    }

    out
}
