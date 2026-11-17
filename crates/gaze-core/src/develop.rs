// SPDX-License-Identifier: GPL-3.0-only
//! OmaRAW-class non-destructive RAW develop pipeline powered by `sex-color`:
//! - 4-way Lift/Gamma/Gain/Offset color wheels, monotonic splines, EV, WB, vibrance (`ColorGrade`)
//! - Physical film stock emulation (`FilmStock`: Kodak 2383, Portra, Ektar, Fuji, Cineon)
//! - Tetrahedral 3D `.cube` LUT evaluation (`Lut3D`)
//! - 5 local adjustment mask types (`BrushMask`, `LinearGradientMask`, `RadialMask`,
//!   `LuminanceRangeMask`, `ColorRangeMask`) with per-mask `MaskAdjustments`
//! - Edge-preserving Oklab chroma/luma bilateral RAW denoise (`denoise_bilateral`)
//! - Patch-match spot/object healing (`heal_region`)

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use libm::{atan2f, cosf, exp2f, expf, fabsf, roundf, sinf, sqrtf};
use sex_image::Image;

pub use sex_color::{
    linear_rec709_to_oklab, linear_to_srgb, oklab_to_linear_rec709, srgb_to_linear, ColorGrade,
    ColorPrimaries, Curve1D, FilmStock, InterpMode, LinearFrame, Lut3D, LutError, RgbaF32,
};

use crate::edits::{apply_edits, EditRecipe};

const PI: f32 = core::f32::consts::PI;

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let span = edge1 - edge0;
    if fabsf(span) <= 1e-6 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / span).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn rec709_luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Returns a stable serialization slug for any `FilmStock`.
pub const fn film_stock_slug(stock: FilmStock) -> &'static str {
    match stock {
        FilmStock::Kodak2383Print => "kodak_2383",
        FilmStock::Kodak2393Print => "kodak_2393",
        FilmStock::KodakVision3_5219 => "kodak_5219",
        FilmStock::Portra160 => "portra_160",
        FilmStock::Portra400 => "portra_400",
        FilmStock::Portra800 => "portra_800",
        FilmStock::Ektar100 => "ektar_100",
        FilmStock::Gold200 => "gold_200",
        FilmStock::Fuji3510Print => "fuji_3510",
        FilmStock::Fuji3513Di => "fuji_3513di",
        FilmStock::AgfaCp30 => "agfa_cp30",
        FilmStock::CineonLog => "cineon_log",
    }
}

/// Parses a `FilmStock` from its slug or human-readable name (case-insensitive).
pub fn film_stock_from_slug(s: &str) -> Option<FilmStock> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") || trimmed == "-" {
        return None;
    }
    for stock in FilmStock::ALL {
        if trimmed.eq_ignore_ascii_case(film_stock_slug(stock))
            || trimmed.eq_ignore_ascii_case(stock.name())
        {
            return Some(stock);
        }
    }
    None
}

/// Cycles forward through `None -> Kodak2383Print -> ... -> CineonLog -> None`.
pub fn next_film_stock(current: Option<FilmStock>) -> Option<FilmStock> {
    match current {
        None => Some(FilmStock::ALL[0]),
        Some(stock) => {
            let idx = FilmStock::ALL.iter().position(|&s| s == stock).unwrap_or(0);
            if idx + 1 >= FilmStock::ALL.len() {
                None
            } else {
                Some(FilmStock::ALL[idx + 1])
            }
        }
    }
}

/// Cycles backward through `None <- Kodak2383Print <- ... <- CineonLog <- None`.
pub fn prev_film_stock(current: Option<FilmStock>) -> Option<FilmStock> {
    match current {
        None => Some(FilmStock::ALL[FilmStock::ALL.len() - 1]),
        Some(stock) => {
            let idx = FilmStock::ALL.iter().position(|&s| s == stock).unwrap_or(0);
            if idx == 0 {
                None
            } else {
                Some(FilmStock::ALL[idx - 1])
            }
        }
    }
}

/// Curated built-in `.cube` 3D LUT presets for instant tetrahedral grading without external files.
pub fn builtin_lut_preset(slug: &str) -> Option<(String, Lut3D)> {
    match slug {
        "teal_amber" => {
            let mut grade = ColorGrade::default();
            grade.lift = [-0.015, 0.005, 0.025];
            grade.gain = [1.06, 1.01, 0.92];
            grade.saturation = 1.12;
            let mut lut = grade.bake_lut3d(17, ColorPrimaries::Rec709);
            lut.title = Some("Cine Teal & Amber".to_string());
            Some(("Cine Teal & Amber".to_string(), lut))
        }
        "bleach_bypass" => {
            let mut grade = ColorGrade::default();
            grade.contrast = 1.42;
            grade.pivot = 0.22;
            grade.saturation = 0.45;
            grade.lift = [-0.01, -0.01, -0.005];
            let mut lut = grade.bake_lut3d(17, ColorPrimaries::Rec709);
            lut.title = Some("Bleach Bypass".to_string());
            Some(("Bleach Bypass".to_string(), lut))
        }
        "silver_mono" => {
            let mut grade = ColorGrade::default();
            grade.saturation = 0.0;
            grade.contrast = 1.28;
            grade.pivot = 0.20;
            let mut lut = grade.bake_lut3d(17, ColorPrimaries::Rec709);
            lut.title = Some("Silver Halide Mono".to_string());
            Some(("Silver Halide Mono".to_string(), lut))
        }
        _ => None,
    }
}

/// Edge-preserving Oklab chroma/luma RAW bilateral denoise settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiseParams {
    /// Luma noise reduction strength (`0.0..=1.0`, `0.0` = off).
    pub luma_strength: f32,
    /// Chroma noise reduction strength (`0.0..=1.0`, `0.0` = off).
    pub chroma_strength: f32,
    /// Spatial filter radius in pixels (`1..=5`).
    pub radius: u8,
    /// Edge-preservation range threshold (`0.01..=1.0`, smaller = sharper edge preservation).
    pub edge_preserve: f32,
}

impl Default for DenoiseParams {
    fn default() -> Self {
        Self {
            luma_strength: 0.0,
            chroma_strength: 0.0,
            radius: 2,
            edge_preserve: 0.15,
        }
    }
}

impl DenoiseParams {
    pub const fn off() -> Self {
        Self {
            luma_strength: 0.0,
            chroma_strength: 0.0,
            radius: 2,
            edge_preserve: 0.15,
        }
    }

    pub const fn light() -> Self {
        Self {
            luma_strength: 0.25,
            chroma_strength: 0.60,
            radius: 2,
            edge_preserve: 0.15,
        }
    }

    pub const fn high_iso() -> Self {
        Self {
            luma_strength: 0.55,
            chroma_strength: 0.90,
            radius: 3,
            edge_preserve: 0.18,
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        (self.luma_strength.is_finite() && self.luma_strength > 1e-4)
            || (self.chroma_strength.is_finite() && self.chroma_strength > 1e-4)
    }

    pub fn sanitized(&self) -> Self {
        Self {
            luma_strength: if self.luma_strength.is_finite() {
                self.luma_strength.clamp(0.0, 1.0)
            } else {
                0.0
            },
            chroma_strength: if self.chroma_strength.is_finite() {
                self.chroma_strength.clamp(0.0, 1.0)
            } else {
                0.0
            },
            radius: self.radius.clamp(1, 5),
            edge_preserve: if self.edge_preserve.is_finite() {
                self.edge_preserve.clamp(0.01, 1.0)
            } else {
                0.15
            },
        }
    }
}

/// Applies offline edge-preserving Oklab luma/chroma bilateral denoising in place on `frame`.
///
/// Luma (`L`) and perceptual chroma (`a, b`) are filtered in Oklab space with a joint edge-stopping
/// range kernel so flat sensor noise is suppressed without blurring sharp edges.
pub fn denoise_bilateral(frame: &mut LinearFrame, params: &DenoiseParams) {
    let p = params.sanitized();
    if !p.is_active() || frame.width == 0 || frame.height == 0 {
        return;
    }
    let w = frame.width;
    let h = frame.height;
    let count = w * h;
    if frame.pixels.len() < count {
        return;
    }

    // Convert frame pixels to Oklab [L, a, b] (using Rec.709 conversion for perceptual Oklab).
    let mut labs = Vec::new();
    if labs.try_reserve_exact(count).is_err() {
        return;
    }
    for &px in &frame.pixels[..count] {
        let s = px.sanitize();
        let rgb709 = sex_color::convert_primaries(s.rgb(), frame.primaries, ColorPrimaries::Rec709);
        labs.push(linear_rec709_to_oklab(rgb709));
    }

    let r = p.radius as isize;
    let sigma_s = (r as f32 * 0.75).max(0.5);
    let inv_two_s2 = 1.0 / (2.0 * sigma_s * sigma_s);

    let sigma_l = (p.edge_preserve * (0.45 + 0.65 * p.luma_strength)).max(0.01);
    let inv_two_l2 = 1.0 / (2.0 * sigma_l * sigma_l);
    let sigma_c = (p.edge_preserve * (1.20 + 2.00 * p.chroma_strength)).max(0.05);
    let inv_two_c2 = 1.0 / (2.0 * sigma_c * sigma_c);

    for y in 0..h {
        let y_min = (y as isize - r).max(0) as usize;
        let y_max = (y as isize + r).min(h as isize - 1) as usize;
        for x in 0..w {
            let idx = y * w + x;
            let center = labs[idx];

            let x_min = (x as isize - r).max(0) as usize;
            let x_max = (x as isize + r).min(w as isize - 1) as usize;

            let mut sum_l = 0.0f32;
            let mut sum_wl = 0.0f32;
            let mut sum_a = 0.0f32;
            let mut sum_b = 0.0f32;
            let mut sum_wc = 0.0f32;

            for ny in y_min..=y_max {
                let dy = (ny as isize - y as isize) as f32;
                let dy2 = dy * dy;
                let row = ny * w;
                for nx in x_min..=x_max {
                    let dx = (nx as isize - x as isize) as f32;
                    let dist2 = dx * dx + dy2;
                    let w_spatial = expf(-dist2 * inv_two_s2);

                    let nb = labs[row + nx];
                    let dl = nb[0] - center[0];
                    let da = nb[1] - center[1];
                    let db = nb[2] - center[2];
                    let dc2 = da * da + db * db;

                    let w_luma = w_spatial * expf(-(dl * dl) * inv_two_l2);
                    // Joint chroma weight stops chroma bleeding across luminance edges as well.
                    let w_chroma = w_spatial * expf(-(dl * dl) * inv_two_l2 - dc2 * inv_two_c2);

                    sum_l += nb[0] * w_luma;
                    sum_wl += w_luma;
                    sum_a += nb[1] * w_chroma;
                    sum_b += nb[2] * w_chroma;
                    sum_wc += w_chroma;
                }
            }

            let fil_l = if sum_wl > 1e-6 { sum_l / sum_wl } else { center[0] };
            let fil_a = if sum_wc > 1e-6 { sum_a / sum_wc } else { center[1] };
            let fil_b = if sum_wc > 1e-6 { sum_b / sum_wc } else { center[2] };

            let out_lab = [
                lerp(center[0], fil_l, p.luma_strength),
                lerp(center[1], fil_a, p.chroma_strength),
                lerp(center[2], fil_b, p.chroma_strength),
            ];

            let out_rgb709 = oklab_to_linear_rec709(out_lab);
            let out_rgb = sex_color::convert_primaries(out_rgb709, ColorPrimaries::Rec709, frame.primaries);
            let alpha = frame.pixels[idx].a();
            frame.pixels[idx] = RgbaF32::from_rgb(
                [out_rgb[0].max(0.0), out_rgb[1].max(0.0), out_rgb[2].max(0.0)],
                alpha,
            );
        }
    }
}

/// Convenience wrapper applying [`denoise_bilateral`] onto a `sex_image::Image`.
pub fn denoise_bilateral_image(
    src: &Image,
    params: &DenoiseParams,
) -> Result<Image, gaze_codecs::Error> {
    let mut frame = LinearFrame::from_argb_u32_srgb(src.width, src.height, &src.pixels)
        .map_err(|_| gaze_codecs::Error::TooLarge)?;
    denoise_bilateral(&mut frame, params);
    Ok(Image {
        width: src.width,
        height: src.height,
        pixels: frame.to_argb_u32_srgb(),
    })
}

/// Non-destructive spot or object heal patch stored in normalized `[0, 1]` coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HealPatch {
    pub target_x: f32,
    pub target_y: f32,
    /// Radius normalized against `min(width, height)`.
    pub radius: f32,
    pub source_x: Option<f32>,
    pub source_y: Option<f32>,
    pub feather: f32,
    pub opacity: f32,
    pub enabled: bool,
}

impl HealPatch {
    pub fn auto(target_x: f32, target_y: f32, radius: f32) -> Self {
        Self {
            target_x: target_x.clamp(0.0, 1.0),
            target_y: target_y.clamp(0.0, 1.0),
            radius: radius.clamp(0.002, 0.45),
            source_x: None,
            source_y: None,
            feather: 0.35,
            opacity: 1.0,
            enabled: true,
        }
    }

    pub fn with_source(mut self, sx: f32, sy: f32) -> Self {
        self.source_x = Some(sx.clamp(0.0, 1.0));
        self.source_y = Some(sy.clamp(0.0, 1.0));
        self
    }
}

/// Finds the best-matching surrounding patch center `(sx, sy)` outside `radius` by comparing
/// collar-ring RGB SSD around `(tx, ty)` against candidate centers around `(tx, ty)`.
pub fn find_best_heal_source(
    frame: &LinearFrame,
    tx: usize,
    ty: usize,
    radius: usize,
) -> (usize, usize) {
    let w = frame.width;
    let h = frame.height;
    let r = radius.max(1) as isize;
    if w <= 2 || h <= 2 {
        return (tx.min(w.saturating_sub(1)), ty.min(h.saturating_sub(1)));
    }

    // Build collar sample offsets in [r + 1 .. r + 3].
    let outer = r + 3;
    let mut collar: Vec<(isize, isize)> = Vec::new();
    for dy in -outer..=outer {
        for dx in -outer..=outer {
            let d2 = dx * dx + dy * dy;
            if d2 >= r * r && d2 <= outer * outer && (dx + dy).rem_euclid(2) == 0 {
                collar.push((dx, dy));
            }
        }
    }
    if collar.is_empty() {
        collar.push((r + 1, 0));
        collar.push((-r - 1, 0));
        collar.push((0, r + 1));
        collar.push((0, -r - 1));
    }

    let step_dist = (r * 2 + 2).max(3);
    let mut best_coord = (tx.min(w - 1), ty.min(h - 1));
    let mut best_cost = f32::INFINITY;

    // Search deterministic concentric rings around (tx, ty).
    for ring in [step_dist, step_dist + r + 2, step_dist * 2 + 2] {
        for dir in 0..16 {
            let angle = (dir as f32) * (2.0 * PI / 16.0);
            let cx = tx as isize + roundf(cosf(angle) * ring as f32) as isize;
            let cy = ty as isize + roundf(sinf(angle) * ring as f32) as isize;
            if cx < r || cy < r || cx + r >= w as isize || cy + r >= h as isize {
                continue;
            }
            // Ensure candidate disk doesn't heavily overlap the spot core.
            let cdist2 = (cx - tx as isize).pow(2) + (cy - ty as isize).pow(2);
            if cdist2 < (r * 2).pow(2) {
                continue;
            }

            let mut ssd = 0.0f32;
            let mut n = 0usize;
            for &(dx, dy) in &collar {
                let txx = tx as isize + dx;
                let tyy = ty as isize + dy;
                let sxx = cx + dx;
                let syy = cy + dy;
                if txx >= 0
                    && tyy >= 0
                    && txx < w as isize
                    && tyy < h as isize
                    && sxx >= 0
                    && syy >= 0
                    && sxx < w as isize
                    && syy < h as isize
                {
                    let tp = frame.pixels[tyy as usize * w + txx as usize].rgb();
                    let sp = frame.pixels[syy as usize * w + sxx as usize].rgb();
                    let dr = tp[0] - sp[0];
                    let dg = tp[1] - sp[1];
                    let db = tp[2] - sp[2];
                    ssd += dr * dr + dg * dg + db * db;
                    n += 1;
                }
            }
            if n > 0 {
                let mean_ssd = ssd / n as f32;
                if mean_ssd < best_cost {
                    best_cost = mean_ssd;
                    best_coord = (cx as usize, cy as usize);
                }
            }
        }
    }

    if !best_cost.is_finite() {
        // Fallback: shift horizontally or vertically away from target.
        let fx = if tx + (r as usize * 2 + 2) < w {
            tx + r as usize * 2 + 2
        } else {
            tx.saturating_sub(r as usize * 2 + 2)
        };
        return (fx.min(w - 1), ty.min(h - 1));
    }
    best_coord
}

/// Patch-match spot/object heal with collar color-gain matching and smooth radial feathering.
pub fn heal_region(
    frame: &mut LinearFrame,
    target_x: usize,
    target_y: usize,
    radius: usize,
    source_hint: Option<(usize, usize)>,
    feather: f32,
    opacity: f32,
) {
    let w = frame.width;
    let h = frame.height;
    let op = if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if w == 0 || h == 0 || frame.pixels.len() < w * h || radius == 0 || op <= 1e-4 {
        return;
    }
    let tx = target_x.min(w - 1);
    let ty = target_y.min(h - 1);
    let (sx, sy) = match source_hint {
        Some((hx, hy)) => (hx.min(w - 1), hy.min(h - 1)),
        None => find_best_heal_source(frame, tx, ty, radius),
    };
    if sx == tx && sy == ty {
        return;
    }

    let r = radius as isize;
    let r_f = radius as f32;
    let feather_clamped = if feather.is_finite() {
        feather.clamp(0.0, 1.0)
    } else {
        0.35
    };
    let core = (1.0 - feather_clamped).clamp(0.0, 0.999);

    // Snapshot original pixels so overlapping target/source reads never alias mid-heal,
    // and compute mean collar delta for seamless exposure/tint matching.
    let orig = frame.pixels.clone();
    let mut sum_t = [0.0f32; 3];
    let mut sum_s = [0.0f32; 3];
    let mut collar_n = 0usize;
    let outer = r + 3;
    for dy in -outer..=outer {
        for dx in -outer..=outer {
            let d2 = dx * dx + dy * dy;
            if d2 >= r * r && d2 <= outer * outer {
                let txx = tx as isize + dx;
                let tyy = ty as isize + dy;
                let sxx = sx as isize + dx;
                let syy = sy as isize + dy;
                if txx >= 0
                    && tyy >= 0
                    && txx < w as isize
                    && tyy < h as isize
                    && sxx >= 0
                    && syy >= 0
                    && sxx < w as isize
                    && syy < h as isize
                {
                    let tp = orig[tyy as usize * w + txx as usize].rgb();
                    let sp = orig[syy as usize * w + sxx as usize].rgb();
                    for c in 0..3 {
                        sum_t[c] += tp[c];
                        sum_s[c] += sp[c];
                    }
                    collar_n += 1;
                }
            }
        }
    }
    let color_bias = if collar_n > 0 {
        let inv = 1.0 / collar_n as f32;
        [
            (sum_t[0] - sum_s[0]) * inv,
            (sum_t[1] - sum_s[1]) * inv,
            (sum_t[2] - sum_s[2]) * inv,
        ]
    } else {
        [0.0, 0.0, 0.0]
    };

    for dy in -r..=r {
        let py = ty as isize + dy;
        let qy = sy as isize + dy;
        if py < 0 || py >= h as isize || qy < 0 || qy >= h as isize {
            continue;
        }
        for dx in -r..=r {
            let px = tx as isize + dx;
            let qx = sx as isize + dx;
            if px < 0 || px >= w as isize || qx < 0 || qx >= w as isize {
                continue;
            }
            let dist = sqrtf((dx * dx + dy * dy) as f32) / r_f;
            if dist > 1.0 {
                continue;
            }
            let falloff = if dist <= core {
                1.0
            } else {
                1.0 - smoothstep(core, 1.0, dist)
            };
            let blend = (falloff * op).clamp(0.0, 1.0);

            let t_idx = py as usize * w + px as usize;
            let s_idx = qy as usize * w + qx as usize;
            let dst = orig[t_idx];
            let src_px = orig[s_idx];

            // Seamless gain/bias match weighted slightly stronger near boundary.
            let bias_w = 0.35 + 0.65 * dist;
            let healed = [
                (src_px.r() + color_bias[0] * bias_w).max(0.0),
                (src_px.g() + color_bias[1] * bias_w).max(0.0),
                (src_px.b() + color_bias[2] * bias_w).max(0.0),
            ];
            frame.pixels[t_idx] = RgbaF32::from_rgb(
                [
                    lerp(dst.r(), healed[0], blend),
                    lerp(dst.g(), healed[1], blend),
                    lerp(dst.b(), healed[2], blend),
                ],
                dst.a(),
            );
        }
    }
}

/// Convenience wrapper applying [`heal_region`] directly to a `sex_image::Image`.
pub fn heal_region_image(
    src: &Image,
    target_x: usize,
    target_y: usize,
    radius: usize,
    source_hint: Option<(usize, usize)>,
    feather: f32,
) -> Result<Image, gaze_codecs::Error> {
    let mut frame = LinearFrame::from_argb_u32_srgb(src.width, src.height, &src.pixels)
        .map_err(|_| gaze_codecs::Error::TooLarge)?;
    heal_region(&mut frame, target_x, target_y, radius, source_hint, feather, 1.0);
    Ok(Image {
        width: src.width,
        height: src.height,
        pixels: frame.to_argb_u32_srgb(),
    })
}

/// Single feathered brush dab in normalized `[0, 1]` coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushDab {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub feather: f32,
    pub flow: f32,
}

impl BrushDab {
    pub fn new(x: f32, y: f32, radius: f32, feather: f32, flow: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
            radius: radius.clamp(0.002, 1.0),
            feather: feather.clamp(0.0, 1.0),
            flow: flow.clamp(0.0, 1.0),
        }
    }
}

/// Mask Type 1: Feathered stroke brush mask composed of accumulated dabs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrushMask {
    pub dabs: Vec<BrushDab>,
}

impl BrushMask {
    pub fn new(dabs: Vec<BrushDab>) -> Self {
        Self { dabs }
    }

    pub fn weight_at(&self, nx: f32, ny: f32) -> f32 {
        let mut acc = 0.0f32;
        for dab in &self.dabs {
            let r = dab.radius.max(1e-4);
            let dx = nx - dab.x;
            let dy = ny - dab.y;
            let d = sqrtf(dx * dx + dy * dy) / r;
            if d >= 1.0 {
                continue;
            }
            let core = (1.0 - dab.feather.clamp(0.0, 1.0)).clamp(0.0, 0.999);
            let w = if d <= core {
                dab.flow
            } else {
                dab.flow * (1.0 - smoothstep(core, 1.0, d))
            };
            acc = (acc + w * (1.0 - acc)).clamp(0.0, 1.0);
        }
        acc
    }
}

/// Mask Type 2: Graduated linear filter from `(x0, y0)` (full 1.0) to `(x1, y1)` (0.0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearGradientMask {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Default for LinearGradientMask {
    fn default() -> Self {
        Self {
            x0: 0.5,
            y0: 0.0,
            x1: 0.5,
            y1: 0.55,
        }
    }
}

impl LinearGradientMask {
    pub fn weight_at(&self, nx: f32, ny: f32) -> f32 {
        let vx = self.x1 - self.x0;
        let vy = self.y1 - self.y0;
        let len2 = vx * vx + vy * vy;
        if len2 <= 1e-8 {
            return 0.5;
        }
        let t = ((nx - self.x0) * vx + (ny - self.y0) * vy) / len2;
        if t <= 0.0 {
            1.0
        } else if t >= 1.0 {
            0.0
        } else {
            1.0 - smoothstep(0.0, 1.0, t)
        }
    }
}

/// Mask Type 3: Rotatable elliptical radial gradient mask in normalized `[0, 1]` coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadialMask {
    pub cx: f32,
    pub cy: f32,
    pub rx: f32,
    pub ry: f32,
    pub feather: f32,
    pub rotation_deg: f32,
}

impl Default for RadialMask {
    fn default() -> Self {
        Self {
            cx: 0.5,
            cy: 0.5,
            rx: 0.35,
            ry: 0.35,
            feather: 0.5,
            rotation_deg: 0.0,
        }
    }
}

impl RadialMask {
    pub fn weight_at(&self, nx: f32, ny: f32) -> f32 {
        let rad = -self.rotation_deg * PI / 180.0;
        let c = cosf(rad);
        let s = sinf(rad);
        let vx = nx - self.cx;
        let vy = ny - self.cy;
        let rx_local = vx * c - vy * s;
        let ry_local = vx * s + vy * c;
        let rx_n = rx_local / self.rx.max(1e-4);
        let ry_n = ry_local / self.ry.max(1e-4);
        let d = sqrtf(rx_n * rx_n + ry_n * ry_n);
        if d >= 1.0 {
            return 0.0;
        }
        let core = (1.0 - self.feather.clamp(0.0, 1.0)).clamp(0.0, 0.999);
        if d <= core {
            1.0
        } else {
            1.0 - smoothstep(core, 1.0, d)
        }
    }
}

/// Mask Type 4: Perceptual luminance range mask (`min_luma..=max_luma` in `[0, 1]` with `feather`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LuminanceRangeMask {
    pub min_luma: f32,
    pub max_luma: f32,
    pub feather: f32,
}

impl Default for LuminanceRangeMask {
    fn default() -> Self {
        Self {
            min_luma: 0.55,
            max_luma: 1.0,
            feather: 0.15,
        }
    }
}

impl LuminanceRangeMask {
    pub fn weight_at(&self, linear_rgb: [f32; 3]) -> f32 {
        let lin_y = rec709_luma(linear_rgb).clamp(0.0, 1.0);
        let luma = linear_to_srgb(lin_y).clamp(0.0, 1.0);
        let lo = self.min_luma.min(self.max_luma).clamp(0.0, 1.0);
        let hi = self.min_luma.max(self.max_luma).clamp(0.0, 1.0);
        let f = self.feather.clamp(0.0, 0.5);

        let w_lo = if lo <= 0.0 {
            1.0
        } else if f <= 1e-5 {
            if luma >= lo { 1.0 } else { 0.0 }
        } else {
            smoothstep(lo - f, lo, luma)
        };

        let w_hi = if hi >= 1.0 {
            1.0
        } else if f <= 1e-5 {
            if luma <= hi { 1.0 } else { 0.0 }
        } else {
            1.0 - smoothstep(hi, hi + f, luma)
        };

        (w_lo * w_hi).clamp(0.0, 1.0)
    }
}

/// Mask Type 5: Perceptual Oklab/HSV color range selector mask.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorRangeMask {
    pub target_oklab: [f32; 3],
    pub hue_tolerance: f32,
    pub chroma_tolerance: f32,
    pub luma_tolerance: f32,
    pub feather: f32,
}

impl Default for ColorRangeMask {
    fn default() -> Self {
        Self::from_srgb([0.2, 0.5, 0.9], 0.18, 0.18, 0.45, 0.25)
    }
}

impl ColorRangeMask {
    pub fn from_linear_rgb(
        rgb: [f32; 3],
        hue_tolerance: f32,
        chroma_tolerance: f32,
        luma_tolerance: f32,
        feather: f32,
    ) -> Self {
        Self {
            target_oklab: linear_rec709_to_oklab(rgb),
            hue_tolerance: hue_tolerance.clamp(0.02, 1.0),
            chroma_tolerance: chroma_tolerance.clamp(0.02, 1.0),
            luma_tolerance: luma_tolerance.clamp(0.05, 2.0),
            feather: feather.clamp(0.0, 1.0),
        }
    }

    pub fn from_srgb(
        srgb: [f32; 3],
        hue_tolerance: f32,
        chroma_tolerance: f32,
        luma_tolerance: f32,
        feather: f32,
    ) -> Self {
        let lin = [
            srgb_to_linear(srgb[0]).max(0.0),
            srgb_to_linear(srgb[1]).max(0.0),
            srgb_to_linear(srgb[2]).max(0.0),
        ];
        Self::from_linear_rgb(lin, hue_tolerance, chroma_tolerance, luma_tolerance, feather)
    }

    /// Constructs a `ColorRangeMask` from HSV (`hue_deg` in `0..360`, `sat` in `0..=1`, `val` in `0..=1`).
    pub fn from_hsv(
        hue_deg: f32,
        sat: f32,
        val: f32,
        hue_tolerance: f32,
        chroma_tolerance: f32,
        luma_tolerance: f32,
        feather: f32,
    ) -> Self {
        let h = ((hue_deg % 360.0) + 360.0) % 360.0;
        let s = sat.clamp(0.0, 1.0);
        let v = val.clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - fabsf((h / 60.0) % 2.0 - 1.0));
        let m = v - c;
        let (rp, gp, bp) = if h < 60.0 {
            (c, x, 0.0)
        } else if h < 120.0 {
            (x, c, 0.0)
        } else if h < 180.0 {
            (0.0, c, x)
        } else if h < 240.0 {
            (0.0, x, c)
        } else if h < 300.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };
        Self::from_srgb(
            [rp + m, gp + m, bp + m],
            hue_tolerance,
            chroma_tolerance,
            luma_tolerance,
            feather,
        )
    }

    pub fn weight_at(&self, linear_rgb: [f32; 3]) -> f32 {
        let lab = linear_rec709_to_oklab(linear_rgb);
        let t = self.target_oklab;

        let dl = fabsf(lab[0] - t[0]) / self.luma_tolerance.max(1e-3);
        let da = lab[1] - t[1];
        let db = lab[2] - t[2];
        let dc = sqrtf(da * da + db * db) / self.chroma_tolerance.max(1e-3);

        let t_chroma = sqrtf(t[1] * t[1] + t[2] * t[2]);
        let p_chroma = sqrtf(lab[1] * lab[1] + lab[2] * lab[2]);
        let dh = if t_chroma > 0.015 && self.hue_tolerance < 0.99 {
            if p_chroma < 0.006 {
                2.0
            } else {
                let ht = atan2f(t[2], t[1]);
                let hp = atan2f(lab[2], lab[1]);
                let mut diff = fabsf(hp - ht);
                if diff > PI {
                    diff = 2.0 * PI - diff;
                }
                (diff / PI) / self.hue_tolerance.max(1e-3)
            }
        } else {
            0.0
        };

        let dist = dc.max(dl).max(dh);
        if dist <= 1.0 {
            1.0
        } else if self.feather <= 1e-5 {
            0.0
        } else {
            1.0 - smoothstep(1.0, 1.0 + self.feather, dist)
        }
    }
}

/// Union of the 5 local mask geometry/range selectors.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalMaskKind {
    Brush(BrushMask),
    LinearGradient(LinearGradientMask),
    Radial(RadialMask),
    LuminanceRange(LuminanceRangeMask),
    ColorRange(ColorRangeMask),
}

impl LocalMaskKind {
    pub const fn slug(&self) -> &'static str {
        match self {
            Self::Brush(_) => "brush",
            Self::LinearGradient(_) => "linear_gradient",
            Self::Radial(_) => "radial",
            Self::LuminanceRange(_) => "luminance_range",
            Self::ColorRange(_) => "color_range",
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Brush(_) => "Brush",
            Self::LinearGradient(_) => "Linear Grad",
            Self::Radial(_) => "Radial",
            Self::LuminanceRange(_) => "Luma Range",
            Self::ColorRange(_) => "Color Range",
        }
    }
}

/// Per-mask local parametric adjustments (`0.0` = neutral across every control).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskAdjustments {
    pub exposure_ev: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub temperature: f32,
    pub tint: f32,
    pub saturation: f32,
    pub sharpness: f32,
}

impl Default for MaskAdjustments {
    fn default() -> Self {
        Self {
            exposure_ev: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            temperature: 0.0,
            tint: 0.0,
            saturation: 0.0,
            sharpness: 0.0,
        }
    }
}

impl MaskAdjustments {
    pub fn is_identity(&self) -> bool {
        fabsf(self.exposure_ev) < 1e-5
            && fabsf(self.contrast) < 1e-5
            && fabsf(self.highlights) < 1e-5
            && fabsf(self.shadows) < 1e-5
            && fabsf(self.temperature) < 1e-5
            && fabsf(self.tint) < 1e-5
            && fabsf(self.saturation) < 1e-5
            && fabsf(self.sharpness) < 1e-5
    }
}

/// Complete local adjustment mask with geometry/range selector, inversion, opacity, and edits.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalMask {
    pub name: String,
    pub enabled: bool,
    pub inverted: bool,
    pub opacity: f32,
    pub kind: LocalMaskKind,
    pub adjustments: MaskAdjustments,
}

impl LocalMask {
    pub fn brush(mask: BrushMask, adjustments: MaskAdjustments) -> Self {
        Self {
            name: "Brush Mask".to_string(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            kind: LocalMaskKind::Brush(mask),
            adjustments,
        }
    }

    pub fn linear_gradient(mask: LinearGradientMask, adjustments: MaskAdjustments) -> Self {
        Self {
            name: "Linear Gradient".to_string(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            kind: LocalMaskKind::LinearGradient(mask),
            adjustments,
        }
    }

    pub fn radial(mask: RadialMask, adjustments: MaskAdjustments) -> Self {
        Self {
            name: "Radial Mask".to_string(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            kind: LocalMaskKind::Radial(mask),
            adjustments,
        }
    }

    pub fn luminance_range(mask: LuminanceRangeMask, adjustments: MaskAdjustments) -> Self {
        Self {
            name: "Luminance Range".to_string(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            kind: LocalMaskKind::LuminanceRange(mask),
            adjustments,
        }
    }

    pub fn color_range(mask: ColorRangeMask, adjustments: MaskAdjustments) -> Self {
        Self {
            name: "Color Range".to_string(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            kind: LocalMaskKind::ColorRange(mask),
            adjustments,
        }
    }

    /// Evaluates the effective mask weight `[0.0, 1.0]` at normalized coords `(nx, ny)` and linear RGB.
    pub fn weight_at(&self, nx: f32, ny: f32, linear_rgb: [f32; 3]) -> f32 {
        if !self.enabled || self.opacity <= 0.0 {
            return 0.0;
        }
        let raw = match &self.kind {
            LocalMaskKind::Brush(m) => m.weight_at(nx, ny),
            LocalMaskKind::LinearGradient(m) => m.weight_at(nx, ny),
            LocalMaskKind::Radial(m) => m.weight_at(nx, ny),
            LocalMaskKind::LuminanceRange(m) => m.weight_at(linear_rgb),
            LocalMaskKind::ColorRange(m) => m.weight_at(linear_rgb),
        };
        let shaped = if self.inverted { 1.0 - raw } else { raw };
        (shaped * self.opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0)
    }
}

/// Complete non-destructive OmaRAW develop recipe powered by `sex-color`.
#[derive(Clone, Debug, PartialEq)]
pub struct DevelopRecipe {
    /// 4-way color wheels (Lift/Gamma/Gain/Offset), EV, WB temp/tint, contrast, saturation, vibrance, curves.
    pub grade: ColorGrade,
    /// Highlight recovery/boost (`-1.0..=1.0`, `0.0` = neutral).
    pub highlights: f32,
    /// Shadow lift/crush (`-1.0..=1.0`, `0.0` = neutral).
    pub shadows: f32,
    /// White point trim (`-1.0..=1.0`, `0.0` = neutral).
    pub whites: f32,
    /// Black point trim (`-1.0..=1.0`, `0.0` = neutral).
    pub blacks: f32,
    /// Global sharpening (`0.0..=1.0`).
    pub sharpness: f32,
    /// Optical edge vignette (`-1.0..=1.0`, negative darkens corners).
    pub vignette: f32,
    /// Physical film stock emulation (`Kodak2383Print`, `Portra400`, `Ektar100`, `Fuji3510Print`, ...).
    pub film_stock: Option<FilmStock>,
    /// Film blend strength (`0.0..=1.0`).
    pub film_strength: f32,
    /// Optional 3D `.cube` LUT.
    pub lut: Option<Lut3D>,
    /// Optional LUT label / preset name for persistence.
    pub lut_name: Option<String>,
    /// LUT blend strength (`0.0..=1.0`).
    pub lut_strength: f32,
    /// LUT interpolation mode (`Tetrahedral` default).
    pub lut_interp: InterpMode,
    /// Whether `lut` operates in sRGB-encoded `[0, 1]` instead of linear-light `[0, 1]`.
    pub lut_in_srgb: bool,
    /// Offline edge-preserving bilateral RAW denoise parameters.
    pub denoise: DenoiseParams,
    /// Non-destructive patch-match spot/object heal operations.
    pub heal_spots: Vec<HealPatch>,
    /// Local adjustment masks (brush, linear gradient, radial, luminance range, color range).
    pub masks: Vec<LocalMask>,
}

impl Default for DevelopRecipe {
    fn default() -> Self {
        Self {
            grade: ColorGrade::default(),
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            sharpness: 0.0,
            vignette: 0.0,
            film_stock: None,
            film_strength: 1.0,
            lut: None,
            lut_name: None,
            lut_strength: 1.0,
            lut_interp: InterpMode::Tetrahedral,
            lut_in_srgb: false,
            denoise: DenoiseParams::default(),
            heal_spots: Vec::new(),
            masks: Vec::new(),
        }
    }
}

impl DevelopRecipe {
    pub fn grade_is_identity(g: &ColorGrade) -> bool {
        let eq3 = |a: [f32; 3], b: [f32; 3]| {
            fabsf(a[0] - b[0]) < 1e-5 && fabsf(a[1] - b[1]) < 1e-5 && fabsf(a[2] - b[2]) < 1e-5
        };
        eq3(g.lift, [0.0, 0.0, 0.0])
            && eq3(g.gamma, [1.0, 1.0, 1.0])
            && eq3(g.gain, [1.0, 1.0, 1.0])
            && eq3(g.offset, [0.0, 0.0, 0.0])
            && fabsf(g.contrast - 1.0) < 1e-5
            && fabsf(g.saturation - 1.0) < 1e-5
            && fabsf(g.vibrance) < 1e-5
            && fabsf(g.temperature) < 1e-5
            && fabsf(g.tint) < 1e-5
            && fabsf(g.exposure_ev) < 1e-5
            && g.master.is_identity()
            && g.r.is_identity()
            && g.g.is_identity()
            && g.b.is_identity()
            && g.hue_vs_sat.is_flat(1.0)
    }

    pub fn is_identity(&self) -> bool {
        let has_active_film = self.film_stock.is_some() && self.film_strength > 1e-4;
        let has_active_lut = self.lut.is_some() && self.lut_strength > 1e-4;
        let has_active_heal = self.heal_spots.iter().any(|h| h.enabled);
        let has_active_mask = self
            .masks
            .iter()
            .any(|m| m.enabled && m.opacity > 1e-4 && !m.adjustments.is_identity());

        Self::grade_is_identity(&self.grade)
            && fabsf(self.highlights) < 1e-5
            && fabsf(self.shadows) < 1e-5
            && fabsf(self.whites) < 1e-5
            && fabsf(self.blacks) < 1e-5
            && fabsf(self.sharpness) < 1e-5
            && fabsf(self.vignette) < 1e-5
            && !has_active_film
            && !has_active_lut
            && !self.denoise.is_active()
            && !has_active_heal
            && !has_active_mask
    }

    /// Parses and attaches an Adobe `.cube` 3D LUT using tetrahedral interpolation by default.
    pub fn set_lut_cube(
        &mut self,
        name: Option<&str>,
        cube_src: &str,
        strength: f32,
    ) -> Result<(), LutError> {
        let lut = Lut3D::parse_cube(cube_src)?;
        let resolved_name = name
            .map(|s| s.to_string())
            .or_else(|| lut.title.clone());
        self.lut = Some(lut);
        self.lut_name = resolved_name;
        self.lut_strength = if strength.is_finite() {
            strength.clamp(0.0, 1.0)
        } else {
            1.0
        };
        Ok(())
    }

    pub fn clear_lut(&mut self) {
        self.lut = None;
        self.lut_name = None;
    }

    pub fn summary(&self) -> String {
        if self.is_identity() {
            return "RAW Neutral".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if fabsf(self.grade.exposure_ev) >= 1e-2 {
            parts.push(format!("EV {:+.2}", self.grade.exposure_ev));
        }
        if fabsf(self.grade.temperature) >= 1e-2 || fabsf(self.grade.tint) >= 1e-2 {
            parts.push("WB".to_string());
        }
        if fabsf(self.highlights) >= 1e-2 || fabsf(self.shadows) >= 1e-2 {
            parts.push(format!("Hi/Sh {:+.1}/{:+.1}", self.highlights, self.shadows));
        }
        if fabsf(self.grade.vibrance) >= 1e-2 {
            parts.push(format!("Vib {:+.2}", self.grade.vibrance));
        }
        if let Some(stock) = self.film_stock {
            if self.film_strength > 1e-4 {
                parts.push(stock.name().to_string());
            }
        }
        if let Some(ref name) = self.lut_name {
            if self.lut.is_some() && self.lut_strength > 1e-4 {
                parts.push(format!("LUT:{}", name));
            }
        } else if self.lut.is_some() && self.lut_strength > 1e-4 {
            parts.push("3D LUT".to_string());
        }
        if self.denoise.is_active() {
            parts.push(format!(
                "NR {:.0}%/{:.0}%",
                self.denoise.luma_strength * 100.0,
                self.denoise.chroma_strength * 100.0
            ));
        }
        let active_masks = self
            .masks
            .iter()
            .filter(|m| m.enabled && !m.adjustments.is_identity())
            .count();
        if active_masks > 0 {
            parts.push(format!("{} Mask(s)", active_masks));
        }
        let active_heals = self.heal_spots.iter().filter(|h| h.enabled).count();
        if active_heals > 0 {
            parts.push(format!("{} Heal(s)", active_heals));
        }
        if parts.is_empty() {
            "Color Graded".to_string()
        } else {
            parts.join(" | ")
        }
    }

    /// Compact single-line encoding for `library.gaze` tab-separated persistence (never contains tabs/newlines).
    pub fn encode_compact(&self) -> String {
        if self.is_identity() {
            return "-".to_string();
        }
        let stock_s = self
            .film_stock
            .map(film_stock_slug)
            .unwrap_or("-");
        let lut_s = self
            .lut_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| s.replace(['|', '\t', '\n', '\r'], "_"))
            .unwrap_or_else(|| "-".to_string());
        format!(
            "V1|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{}|{:.3}|{}|{:.3}|{:.3}|{:.3}|{}",
            self.grade.exposure_ev,
            self.grade.temperature,
            self.grade.tint,
            self.grade.contrast,
            self.grade.saturation,
            self.grade.vibrance,
            self.highlights,
            self.shadows,
            self.whites,
            self.blacks,
            self.sharpness,
            self.vignette,
            stock_s,
            self.film_strength,
            lut_s,
            self.lut_strength,
            self.denoise.luma_strength,
            self.denoise.chroma_strength,
            self.denoise.radius,
        )
    }

    pub fn decode_compact(text: &str) -> Self {
        let mut dev = Self::default();
        if text == "-" || text.is_empty() {
            return dev;
        }
        let cols: Vec<&str> = text.split('|').collect();
        if cols.first().copied() != Some("V1") || cols.len() < 19 {
            return dev;
        }
        let pf = |i: usize, def: f32| -> f32 {
            cols.get(i)
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .unwrap_or(def)
        };
        dev.grade.exposure_ev = pf(1, 0.0).clamp(-6.0, 6.0);
        dev.grade.temperature = pf(2, 0.0).clamp(-2.0, 2.0);
        dev.grade.tint = pf(3, 0.0).clamp(-2.0, 2.0);
        dev.grade.contrast = pf(4, 1.0).clamp(0.1, 4.0);
        dev.grade.saturation = pf(5, 1.0).clamp(0.0, 3.0);
        dev.grade.vibrance = pf(6, 0.0).clamp(-1.0, 2.0);
        dev.highlights = pf(7, 0.0).clamp(-1.0, 1.0);
        dev.shadows = pf(8, 0.0).clamp(-1.0, 1.0);
        dev.whites = pf(9, 0.0).clamp(-1.0, 1.0);
        dev.blacks = pf(10, 0.0).clamp(-1.0, 1.0);
        dev.sharpness = pf(11, 0.0).clamp(0.0, 1.0);
        dev.vignette = pf(12, 0.0).clamp(-1.0, 1.0);
        dev.film_stock = cols.get(13).and_then(|s| film_stock_from_slug(s));
        dev.film_strength = pf(14, 1.0).clamp(0.0, 1.0);
        if let Some(l_name) = cols.get(15).filter(|s| **s != "-") {
            let slug = match *l_name {
                "Cine Teal & Amber" => "teal_amber",
                "Bleach Bypass" => "bleach_bypass",
                "Silver Halide Mono" => "silver_mono",
                other => other,
            };
            if let Some((title, lut)) = builtin_lut_preset(slug) {
                dev.lut_name = Some(title);
                dev.lut = Some(lut);
            } else {
                dev.lut_name = Some((*l_name).to_string());
            }
        }
        dev.lut_strength = pf(16, 1.0).clamp(0.0, 1.0);
        dev.denoise.luma_strength = pf(17, 0.0).clamp(0.0, 1.0);
        dev.denoise.chroma_strength = pf(18, 0.0).clamp(0.0, 1.0);
        if let Some(r) = cols.get(19).and_then(|s| s.parse::<u8>().ok()) {
            dev.denoise.radius = r.clamp(1, 5);
        }
        dev
    }
}

/// Applies tone zone recovery (`highlights`, `shadows`, `whites`, `blacks`) in linear light,
/// preserving chromatic ratios.
#[inline]
fn apply_tonal_zones(
    rgb: [f32; 3],
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
) -> [f32; 3] {
    let y = rec709_luma(rgb).max(0.0);
    let mut gain = 1.0f32;

    if fabsf(shadows) > 1e-5 {
        let shadow_w = 1.0 - smoothstep(0.02, 0.45, y);
        gain *= exp2f(shadows * 1.35 * shadow_w);
    }
    if fabsf(highlights) > 1e-5 {
        let hi_w = smoothstep(0.32, 0.95, y);
        gain *= exp2f(highlights * 1.15 * hi_w);
    }
    if fabsf(whites) > 1e-5 {
        let white_w = smoothstep(0.65, 1.0, y);
        gain *= exp2f(whites * 0.85 * white_w);
    }

    let mut out = [
        (rgb[0] * gain).max(0.0),
        (rgb[1] * gain).max(0.0),
        (rgb[2] * gain).max(0.0),
    ];

    if fabsf(blacks) > 1e-5 {
        let black_w = 1.0 - smoothstep(0.0, 0.18, y);
        let shift = blacks * 0.04 * black_w;
        out[0] = (out[0] + shift).max(0.0);
        out[1] = (out[1] + shift).max(0.0);
        out[2] = (out[2] + shift).max(0.0);
    }
    out
}

/// Applies local `MaskAdjustments` weighted by `w in [0, 1]` onto a linear RGB pixel.
fn apply_local_mask_pixel(
    rgb: [f32; 3],
    adj: &MaskAdjustments,
    w: f32,
    primaries: ColorPrimaries,
) -> [f32; 3] {
    if w <= 1e-5 {
        return rgb;
    }
    let mut grade = ColorGrade::default();
    grade.exposure_ev = adj.exposure_ev * w;
    grade.temperature = adj.temperature * w;
    grade.tint = adj.tint * w;
    grade.contrast = (1.0 + adj.contrast * w).clamp(0.1, 3.0);
    grade.saturation = (1.0 + adj.saturation * w).clamp(0.0, 3.0);

    let graded = grade.eval_rgb(rgb, primaries);
    if fabsf(adj.highlights) > 1e-4 || fabsf(adj.shadows) > 1e-4 {
        apply_tonal_zones(graded, adj.highlights * w, adj.shadows * w, 0.0, 0.0)
    } else {
        graded
    }
}

/// Applies a local or global unsharp-mask residual modulated per-pixel by `amounts`.
fn apply_unsharp_modulated(frame: &mut LinearFrame, amounts: &[f32]) {
    let w = frame.width;
    let h = frame.height;
    let count = w * h;
    if w < 3 || h < 3 || frame.pixels.len() < count || amounts.len() < count {
        return;
    }
    let orig = frame.pixels.clone();
    for y in 0..h {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(h - 1);
        for x in 0..w {
            let idx = y * w + x;
            let amt = amounts[idx];
            if fabsf(amt) <= 1e-4 {
                continue;
            }
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);

            let mut blur = [0.0f32; 3];
            for sy in y0..=y1 {
                for sx in x0..=x1 {
                    let p = orig[sy * w + sx];
                    blur[0] += p.r();
                    blur[1] += p.g();
                    blur[2] += p.b();
                }
            }
            let n = ((y1 - y0 + 1) * (x1 - x0 + 1)) as f32;
            blur[0] /= n;
            blur[1] /= n;
            blur[2] /= n;

            let c = orig[idx];
            let k = amt * 1.4;
            frame.pixels[idx] = RgbaF32::from_rgb(
                [
                    (c.r() + (c.r() - blur[0]) * k).max(0.0),
                    (c.g() + (c.g() - blur[1]) * k).max(0.0),
                    (c.b() + (c.b() - blur[2]) * k).max(0.0),
                ],
                c.a(),
            );
        }
    }
}

/// Executes the full non-destructive OmaRAW develop pipeline on a linear-light [`LinearFrame`].
pub fn develop_frame(frame: &mut LinearFrame, recipe: &DevelopRecipe) {
    if recipe.is_identity() || frame.width == 0 || frame.height == 0 {
        return;
    }
    let w = frame.width;
    let h = frame.height;
    let count = w * h;
    if frame.pixels.len() < count {
        return;
    }

    // 1. Offline Edge-Preserving RAW Denoise
    if recipe.denoise.is_active() {
        denoise_bilateral(frame, &recipe.denoise);
    }

    // 2. Patch-Match Spot / Object Healing
    let min_dim = w.min(h) as f32;
    for patch in &recipe.heal_spots {
        if !patch.enabled {
            continue;
        }
        let tx = roundf(patch.target_x.clamp(0.0, 1.0) * (w.saturating_sub(1)) as f32) as usize;
        let ty = roundf(patch.target_y.clamp(0.0, 1.0) * (h.saturating_sub(1)) as f32) as usize;
        let rad = (roundf(patch.radius * min_dim) as usize).max(1);
        let hint = match (patch.source_x, patch.source_y) {
            (Some(sx), Some(sy)) => Some((
                roundf(sx.clamp(0.0, 1.0) * (w.saturating_sub(1)) as f32) as usize,
                roundf(sy.clamp(0.0, 1.0) * (h.saturating_sub(1)) as f32) as usize,
            )),
            _ => None,
        };
        heal_region(frame, tx, ty, rad, hint, patch.feather, patch.opacity);
    }

    // 3. Global ColorGrade (4-way wheels, EV, WB, contrast, saturation, vibrance, curves)
    if !DevelopRecipe::grade_is_identity(&recipe.grade) {
        recipe.grade.apply_frame(frame);
    }

    // 4. Highlights / Shadows / Whites / Blacks + Optical Vignette
    let has_zones = fabsf(recipe.highlights) > 1e-5
        || fabsf(recipe.shadows) > 1e-5
        || fabsf(recipe.whites) > 1e-5
        || fabsf(recipe.blacks) > 1e-5;
    let has_vig = fabsf(recipe.vignette) > 1e-5;
    if has_zones || has_vig {
        let inv_w = 1.0 / (w.saturating_sub(1).max(1) as f32);
        let inv_h = 1.0 / (h.saturating_sub(1).max(1) as f32);
        for y in 0..h {
            let ny = y as f32 * inv_h - 0.5;
            for x in 0..w {
                let idx = y * w + x;
                let px = frame.pixels[idx];
                let mut rgb = px.rgb();
                if has_zones {
                    rgb = apply_tonal_zones(
                        rgb,
                        recipe.highlights,
                        recipe.shadows,
                        recipe.whites,
                        recipe.blacks,
                    );
                }
                if has_vig {
                    let nx = x as f32 * inv_w - 0.5;
                    let r2 = (nx * nx + ny * ny) * 4.0; // 0 at center, ~2 at corners
                    let vig = (1.0 + recipe.vignette * 0.45 * r2).clamp(0.1, 2.5);
                    rgb = [rgb[0] * vig, rgb[1] * vig, rgb[2] * vig];
                }
                frame.pixels[idx] = RgbaF32::from_rgb(rgb, px.a());
            }
        }
    }

    // 5. Local Adjustment Masks (Brush / LinearGradient / Radial / LuminanceRange / ColorRange)
    if !recipe.masks.is_empty() {
        let primaries = frame.primaries;
        let inv_w = 1.0 / (w.saturating_sub(1).max(1) as f32);
        let inv_h = 1.0 / (h.saturating_sub(1).max(1) as f32);
        let mut sharp_map: Option<Vec<f32>> = None;

        for mask in &recipe.masks {
            if !mask.enabled || mask.opacity <= 1e-4 || mask.adjustments.is_identity() {
                continue;
            }
            let has_local_sharp = fabsf(mask.adjustments.sharpness) > 1e-4;
            if has_local_sharp && sharp_map.is_none() {
                sharp_map = Some(vec![0.0f32; count]);
            }

            for y in 0..h {
                let ny = if h == 1 { 0.5 } else { y as f32 * inv_h };
                for x in 0..w {
                    let nx = if w == 1 { 0.5 } else { x as f32 * inv_w };
                    let idx = y * w + x;
                    let px = frame.pixels[idx];
                    let weight = mask.weight_at(nx, ny, px.rgb());
                    if weight <= 1e-5 {
                        continue;
                    }
                    let adjusted = apply_local_mask_pixel(px.rgb(), &mask.adjustments, weight, primaries);
                    frame.pixels[idx] = RgbaF32::from_rgb(adjusted, px.a());
                    if let Some(ref mut map) = sharp_map {
                        map[idx] += mask.adjustments.sharpness * weight;
                    }
                }
            }
        }

        if let Some(mut map) = sharp_map {
            if recipe.sharpness > 1e-4 {
                for v in &mut map {
                    *v += recipe.sharpness;
                }
            }
            apply_unsharp_modulated(frame, &map);
        } else if recipe.sharpness > 1e-4 {
            let map = vec![recipe.sharpness; count];
            apply_unsharp_modulated(frame, &map);
        }
    } else if recipe.sharpness > 1e-4 {
        let map = vec![recipe.sharpness; count];
        apply_unsharp_modulated(frame, &map);
    }

    // 6. Physical Film Stock Emulation (`sex_color::FilmStock`)
    if let Some(stock) = recipe.film_stock {
        let str_f = recipe.film_strength.clamp(0.0, 1.0);
        if str_f >= 0.999 {
            stock.apply_frame(frame);
        } else if str_f > 1e-4 {
            for px in &mut frame.pixels[..count] {
                let orig = px.rgb();
                let filmed = stock.apply_linear(orig);
                *px = RgbaF32::from_rgb(
                    [
                        lerp(orig[0], filmed[0], str_f),
                        lerp(orig[1], filmed[1], str_f),
                        lerp(orig[2], filmed[2], str_f),
                    ],
                    px.a(),
                );
            }
        }
    }

    // 7. Tetrahedral / Trilinear 3D LUT (`sex_color::Lut3D`)
    if let Some(ref lut) = recipe.lut {
        let str_l = recipe.lut_strength.clamp(0.0, 1.0);
        if str_l > 1e-4 {
            for px in &mut frame.pixels[..count] {
                let orig = px.rgb();
                let input = if recipe.lut_in_srgb {
                    [
                        linear_to_srgb(orig[0]).clamp(0.0, 1.0),
                        linear_to_srgb(orig[1]).clamp(0.0, 1.0),
                        linear_to_srgb(orig[2]).clamp(0.0, 1.0),
                    ]
                } else {
                    orig
                };
                let mapped_raw = lut.eval(input, recipe.lut_interp);
                let mapped = if recipe.lut_in_srgb {
                    [
                        srgb_to_linear(mapped_raw[0]).max(0.0),
                        srgb_to_linear(mapped_raw[1]).max(0.0),
                        srgb_to_linear(mapped_raw[2]).max(0.0),
                    ]
                } else {
                    mapped_raw
                };
                *px = RgbaF32::from_rgb(
                    [
                        lerp(orig[0], mapped[0], str_l),
                        lerp(orig[1], mapped[1], str_l),
                        lerp(orig[2], mapped[2], str_l),
                    ],
                    px.a(),
                );
            }
        }
    }
}

/// Applies `DevelopRecipe` non-destructively onto `src` (`0xAARRGGBB`), leaving `src` untouched.
pub fn develop_image(
    src: &Image,
    recipe: &DevelopRecipe,
) -> Result<Image, gaze_codecs::Error> {
    if src.width == 0 || src.height == 0 || src.pixels.len() < src.width * src.height {
        return Err(gaze_codecs::Error::TooLarge);
    }
    if recipe.is_identity() {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(src.width * src.height)
            .map_err(|_| gaze_codecs::Error::OutOfMemory)?;
        pixels.extend_from_slice(&src.pixels[..src.width * src.height]);
        return Ok(Image {
            width: src.width,
            height: src.height,
            pixels,
        });
    }
    let mut frame = LinearFrame::from_argb_u32_srgb(src.width, src.height, &src.pixels)
        .map_err(|_| gaze_codecs::Error::OutOfMemory)?;
    develop_frame(&mut frame, recipe);
    Ok(Image {
        width: src.width,
        height: src.height,
        pixels: frame.to_argb_u32_srgb(),
    })
}

/// Combines geometric / legacy `EditRecipe` (`crop`, `quarter_turns`, `flip_h`, shared `batik-core`
/// filters) and full `DevelopRecipe` non-destructively, leaving `src` untouched.
///
/// When `dev.is_identity()`, output matches [`apply_edits`] bit-for-bit.
pub fn apply_full_develop(
    src: &Image,
    edit: &EditRecipe,
    dev: &DevelopRecipe,
) -> Result<Image, gaze_codecs::Error> {
    if dev.is_identity() {
        return apply_edits(src, edit);
    }
    if edit.is_identity() {
        return develop_image(src, dev);
    }
    let edited = apply_edits(src, edit)?;
    develop_image(&edited, dev)
}
