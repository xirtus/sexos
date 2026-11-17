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
