// SPDX-License-Identifier: GPL-3.0-only
//! Non-destructive adjustments, tone curves, and color correction filters.

use alloc::vec;
use alloc::vec::Vec;
use crate::blend::{pack_argb, split_argb};
use crate::selection::SelectionMask;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub in_black: u8,
    pub in_white: u8,
    pub gamma_fp: f32,
    pub out_black: u8,
    pub out_white: u8,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            in_black: 0,
            in_white: 255,
            gamma_fp: 1.0,
            out_black: 0,
            out_white: 255,
        }
    }
}

impl Levels {
    pub fn build_lut(&self) -> [u8; 256] {
        let mut lut = [0u8; 256];
        let in_min = self.in_black.min(self.in_white);
        let in_max = self.in_black.max(self.in_white);
        let in_range = (in_max - in_min).max(1) as f32;
        let out_min = self.out_black as f32;
        let out_max = self.out_white as f32;
        let gamma = if self.gamma_fp.is_finite() && self.gamma_fp > 0.01 {
            self.gamma_fp
        } else {
            1.0
        };
        let gamma_inv = 1.0 / gamma;

        for i in 0..=255 {
            let val = (i as u8).clamp(in_min, in_max);
            let normalized = (val - in_min) as f32 / in_range;
            let corrected = libm::powf(normalized, gamma_inv);
            let result = out_min + corrected * (out_max - out_min);
            lut[i] = libm::roundf(result).clamp(0.0, 255.0) as u8;
        }
        lut
    }
}

/// Named presets for tone curves adjustments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurvesPreset {
    Linear,
    SCurveContrast,
    LiftShadows,
    Solarize,
    WarmVintage,
    CoolCinema,
}

impl CurvesPreset {
    pub fn to_curves(self) -> CurvesAdjustment {
        match self {
            CurvesPreset::Linear => CurvesAdjustment::linear(),
            CurvesPreset::SCurveContrast => CurvesAdjustment::s_curve_contrast(),
            CurvesPreset::LiftShadows => CurvesAdjustment::lift_shadows(),
            CurvesPreset::Solarize => CurvesAdjustment::solarize(),
            CurvesPreset::WarmVintage => CurvesAdjustment::warm_vintage(),
            CurvesPreset::CoolCinema => CurvesAdjustment::cool_cinema(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CurvesPreset::Linear => "Linear",
            CurvesPreset::SCurveContrast => "S-Curve Contrast",
            CurvesPreset::LiftShadows => "Lift Shadows",
            CurvesPreset::Solarize => "Solarize",
            CurvesPreset::WarmVintage => "Warm Vintage",
            CurvesPreset::CoolCinema => "Cool Cinema",
        }
    }
}

/// Channel and master curves adjustment with smooth monotone cubic interpolation.
#[derive(Clone, Debug, PartialEq)]
pub struct CurvesAdjustment {
    pub master: Vec<(u8, u8)>,
    pub red: Vec<(u8, u8)>,
    pub green: Vec<(u8, u8)>,
    pub blue: Vec<(u8, u8)>,
}

pub type Curves = CurvesAdjustment;

impl Default for CurvesAdjustment {
    fn default() -> Self {
        Self::linear()
    }
}

impl CurvesAdjustment {
    pub fn linear() -> Self {
        Self {
            master: vec![(0, 0), (255, 255)],
            red: vec![(0, 0), (255, 255)],
            green: vec![(0, 0), (255, 255)],
            blue: vec![(0, 0), (255, 255)],
        }
    }

    pub fn s_curve_contrast() -> Self {
        Self {
            master: vec![(0, 0), (64, 48), (128, 128), (192, 208), (255, 255)],
            red: vec![(0, 0), (255, 255)],
            green: vec![(0, 0), (255, 255)],
            blue: vec![(0, 0), (255, 255)],
        }
    }

    pub fn lift_shadows() -> Self {
        Self {
            master: vec![(0, 48), (128, 140), (255, 255)],
            red: vec![(0, 0), (255, 255)],
            green: vec![(0, 0), (255, 255)],
            blue: vec![(0, 0), (255, 255)],
        }
    }

    pub fn solarize() -> Self {
        Self {
            master: vec![(0, 0), (128, 255), (255, 0)],
            red: vec![(0, 0), (255, 255)],
            green: vec![(0, 0), (255, 255)],
            blue: vec![(0, 0), (255, 255)],
        }
    }

    pub fn warm_vintage() -> Self {
        Self {
            master: vec![(0, 10), (128, 128), (255, 250)],
            red: vec![(0, 20), (128, 145), (255, 255)],
            green: vec![(0, 5), (128, 130), (255, 250)],
            blue: vec![(0, 0), (128, 110), (255, 235)],
        }
    }

    pub fn cool_cinema() -> Self {
        Self {
            master: vec![(0, 0), (64, 52), (192, 204), (255, 255)],
            red: vec![(0, 0), (128, 120), (255, 245)],
            green: vec![(0, 5), (128, 130), (255, 250)],
            blue: vec![(0, 30), (128, 140), (255, 255)],
        }
    }

    pub fn from_preset(preset: CurvesPreset) -> Self {
        preset.to_curves()
    }

    pub fn build_lut(points: &[(u8, u8)]) -> [u8; 256] {
        Self::build_curves_lut(points)
    }

    /// Evaluates control points to a 256-entry `u8` LUT using smooth monotone cubic Hermite interpolation.
    pub fn build_curves_lut(points: &[(u8, u8)]) -> [u8; 256] {
        let mut lut = [0u8; 256];
        if points.is_empty() {
            for i in 0..=255 {
                lut[i] = i as u8;
            }
            return lut;
        }

        let mut pts = points.to_vec();
        pts.sort_by_key(|p| p.0);

        let mut dedup_pts: Vec<(u8, u8)> = Vec::with_capacity(pts.len());
        for p in pts {
            if let Some(last) = dedup_pts.last_mut() {
                if last.0 == p.0 {
                    *last = p;
                    continue;
                }
            }
            dedup_pts.push(p);
        }

        let n = dedup_pts.len();
        if n == 1 {
            lut.fill(dedup_pts[0].1);
            return lut;
        }

        if n == 2 {
            let (x0, y0) = (dedup_pts[0].0 as f32, dedup_pts[0].1 as f32);
            let (x1, y1) = (dedup_pts[1].0 as f32, dedup_pts[1].1 as f32);
            let span = (x1 - x0).max(1.0);
            for i in 0..=255 {
                let x = i as f32;
                if x <= x0 {
                    lut[i] = dedup_pts[0].1;
                } else if x >= x1 {
                    lut[i] = dedup_pts[1].1;
                } else {
                    let t = (x - x0) / span;
                    let y = y0 + t * (y1 - y0);
                    lut[i] = libm::roundf(y).clamp(0.0, 255.0) as u8;
                }
            }
            return lut;
        }

        // Monotone cubic Hermite spline interpolation (Fritsch & Carlson)
        let mut deltas = alloc::vec![0.0f32; n - 1];
        for k in 0..n - 1 {
            let dx = (dedup_pts[k + 1].0 as f32 - dedup_pts[k].0 as f32).max(1.0);
            let dy = dedup_pts[k + 1].1 as f32 - dedup_pts[k].1 as f32;
            deltas[k] = dy / dx;
        }

        let mut d = alloc::vec![0.0f32; n];
        d[0] = deltas[0];
        d[n - 1] = deltas[n - 2];
        for k in 1..n - 1 {
            d[k] = (deltas[k - 1] + deltas[k]) * 0.5;
        }

        for k in 0..n - 1 {
            if libm::fabsf(deltas[k]) < 1e-6 {
                d[k] = 0.0;
                d[k + 1] = 0.0;
            } else {
                let alpha = d[k] / deltas[k];
                let beta = d[k + 1] / deltas[k];
                if alpha < 0.0 {
                    d[k] = 0.0;
                }
                if beta < 0.0 {
                    d[k + 1] = 0.0;
                }
                let hyp_sq = alpha * alpha + beta * beta;
                if hyp_sq > 9.0 {
                    let tau = 3.0 / libm::sqrtf(hyp_sq);
                    d[k] = tau * alpha * deltas[k];
                    d[k + 1] = tau * beta * deltas[k];
                }
            }
        }

        for i in 0..=255 {
            let x = i as u8;
            if x <= dedup_pts[0].0 {
                lut[i] = dedup_pts[0].1;
            } else if x >= dedup_pts[n - 1].0 {
                lut[i] = dedup_pts[n - 1].1;
            } else {
                for k in 0..n - 1 {
                    let x0 = dedup_pts[k].0;
                    let x1 = dedup_pts[k + 1].0;
                    if x >= x0 && x <= x1 {
                        let h = (x1 as f32 - x0 as f32).max(1.0);
                        let t = (x as f32 - x0 as f32) / h;
                        let t2 = t * t;
                        let t3 = t2 * t;
                        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
                        let h10 = t3 - 2.0 * t2 + t;
                        let h01 = -2.0 * t3 + 3.0 * t2;
                        let h11 = t3 - t2;
                        let y = h00 * (dedup_pts[k].1 as f32)
                            + h10 * h * d[k]
                            + h01 * (dedup_pts[k + 1].1 as f32)
                            + h11 * h * d[k + 1];
                        lut[i] = libm::roundf(y).clamp(0.0, 255.0) as u8;
                        break;
                    }
                }
            }
        }

        lut
    }

    pub fn build_all_luts(&self) -> ([u8; 256], [u8; 256], [u8; 256], [u8; 256]) {
        (
            Self::build_curves_lut(&self.master),
            Self::build_curves_lut(&self.red),
            Self::build_curves_lut(&self.green),
            Self::build_curves_lut(&self.blue),
        )
    }

    #[inline]
    pub fn apply_with_luts(
        pixel: u32,
        lut_m: &[u8; 256],
        lut_r: &[u8; 256],
        lut_g: &[u8; 256],
        lut_b: &[u8; 256],
    ) -> u32 {
        let (a, r, g, b) = split_argb(pixel);
        if a == 0 {
            return pixel;
        }

        let out_r = lut_m[lut_r[r as usize] as usize];
        let out_g = lut_m[lut_g[g as usize] as usize];
        let out_b = lut_m[lut_b[b as usize] as usize];

        pack_argb(a, out_r, out_g, out_b)
    }

    /// Evaluates pixel color through red, green, blue channel curves, then through master curve.
    pub fn apply_pixel(&self, pixel: u32) -> u32 {
        let (lut_m, lut_r, lut_g, lut_b) = self.build_all_luts();
        Self::apply_with_luts(pixel, &lut_m, &lut_r, &lut_g, &lut_b)
    }
}

/// Hue, Saturation, and Lightness non-destructive color adjustment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HueSatAdjustment {
    pub hue_deg: i16,
    pub saturation_pct: i16,
    pub lightness_pct: i16,
    pub colorize: bool,
}

impl Default for HueSatAdjustment {
    fn default() -> Self {
        Self {
            hue_deg: 0,
            saturation_pct: 0,
            lightness_pct: 0,
            colorize: false,
        }
    }
}

impl HueSatAdjustment {
    pub fn new(hue_deg: i16, saturation_pct: i16, lightness_pct: i16) -> Self {
        Self {
            hue_deg: hue_deg.clamp(-180, 180),
            saturation_pct: saturation_pct.clamp(-100, 100),
            lightness_pct: lightness_pct.clamp(-100, 100),
            colorize: false,
        }
    }

    pub fn colorize(hue_deg: i16, saturation_pct: i16, lightness_pct: i16) -> Self {
        Self {
            hue_deg: hue_deg.clamp(-180, 180),
            saturation_pct: saturation_pct.clamp(-100, 100),
            lightness_pct: lightness_pct.clamp(-100, 100),
            colorize: true,
        }
    }

    pub fn apply_pixel(&self, pixel: u32) -> u32 {
        let (a, r, g, b) = split_argb(pixel);
        if a == 0 {
            return pixel;
        }

        let (h, s, l) = rgb_to_hsl(r, g, b);

        let (new_h, new_s, new_l) = if self.colorize {
            let h_out = if self.hue_deg < 0 {
                (self.hue_deg as f32 + 360.0) % 360.0
            } else {
                (self.hue_deg as f32) % 360.0
            };
            let s_out = (self.saturation_pct.max(0) as f32 / 100.0).clamp(0.0, 1.0);
            let l_adj = (l + (self.lightness_pct as f32 / 100.0)).clamp(0.0, 1.0);
            (h_out, s_out, l_adj)
        } else {
            let mut h_out = (h + self.hue_deg as f32) % 360.0;
            if h_out < 0.0 {
                h_out += 360.0;
            }
            let s_out = if self.saturation_pct >= 0 {
                (s + (1.0 - s) * (self.saturation_pct as f32 / 100.0)).clamp(0.0, 1.0)
            } else {
                (s * (1.0 + self.saturation_pct as f32 / 100.0)).clamp(0.0, 1.0)
            };
            let l_out = if self.lightness_pct >= 0 {
                (l + (1.0 - l) * (self.lightness_pct as f32 / 100.0)).clamp(0.0, 1.0)
            } else {
                (l * (1.0 + self.lightness_pct as f32 / 100.0)).clamp(0.0, 1.0)
            };
            (h_out, s_out, l_out)
        };

        let (out_r, out_g, out_b) = hsl_to_rgb(new_h, new_s, new_l);
        pack_argb(a, out_r, out_g, out_b)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AdjustmentKind {
    Curves(CurvesAdjustment),
    HueSaturation(HueSatAdjustment),
    HueSaturationLightness { hue: f32, saturation: f32, lightness: f32 },
    Levels(Levels),
    BrightnessContrast { brightness: f32, contrast: f32 },
    Invert,
    Posterize { levels: u8 },
    Threshold { cutoff: u8 },
    Desaturate,
    Sepia,
}

pub type Adjustment = AdjustmentKind;

impl AdjustmentKind {
    pub fn apply_pixel(&self, pixel: u32) -> u32 {
        let (a, r, g, b) = split_argb(pixel);
        if a == 0 {
            return pixel;
        }

        match self {
            AdjustmentKind::Curves(curves) => curves.apply_pixel(pixel),
            AdjustmentKind::HueSaturation(hs) => hs.apply_pixel(pixel),
            AdjustmentKind::Levels(levels) => {
                let lut = levels.build_lut();
                pack_argb(a, lut[r as usize], lut[g as usize], lut[b as usize])
            }
            AdjustmentKind::BrightnessContrast { brightness, contrast } => {
                let factor = if contrast.is_finite() {
                    (1.0 + contrast).max(0.0)
                } else {
                    1.0
                };
                let b_val = if brightness.is_finite() {
                    *brightness
                } else {
                    0.0
                };
                let map = |c: u8| -> u8 {
                    let norm = c as f32 / 255.0;
                    let adj = (norm - 0.5) * factor + 0.5 + b_val;
                    libm::roundf(adj * 255.0).clamp(0.0, 255.0) as u8
                };
                pack_argb(a, map(r), map(g), map(b))
            }
            AdjustmentKind::HueSaturationLightness { hue, saturation, lightness } => {
                let h_offset = if hue.is_finite() { *hue } else { 0.0 };
                let s_factor = if saturation.is_finite() { *saturation } else { 0.0 };
                let l_offset = if lightness.is_finite() { *lightness } else { 0.0 };
                let (h, s, l) = rgb_to_hsl(r, g, b);
                let mut new_h = (h + h_offset) % 360.0;
                if new_h < 0.0 {
                    new_h += 360.0;
                }
                let new_s = (s * (1.0 + s_factor)).clamp(0.0, 1.0);
                let new_l = (l + l_offset).clamp(0.0, 1.0);
                let (out_r, out_g, out_b) = hsl_to_rgb(new_h, new_s, new_l);
                pack_argb(a, out_r, out_g, out_b)
            }
            AdjustmentKind::Invert => pack_argb(a, 255 - r, 255 - g, 255 - b),
            AdjustmentKind::Posterize { levels } => {
                let n = (*levels).max(2) as f32;
                let step = 255.0 / (n - 1.0);
                let map = |c: u8| -> u8 {
                    let bucket = libm::roundf(c as f32 / step);
                    libm::roundf(bucket * step).clamp(0.0, 255.0) as u8
                };
                pack_argb(a, map(r), map(g), map(b))
            }
            AdjustmentKind::Threshold { cutoff } => {
                let lum = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
                let val = if lum >= *cutoff { 255 } else { 0 };
                pack_argb(a, val, val, val)
            }
            AdjustmentKind::Desaturate => {
                let lum = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
                pack_argb(a, lum, lum, lum)
            }
            AdjustmentKind::Sepia => {
                let rf = r as f32;
                let gf = g as f32;
                let bf = b as f32;
                let nr = libm::roundf(0.393 * rf + 0.769 * gf + 0.189 * bf).clamp(0.0, 255.0) as u8;
                let ng = libm::roundf(0.349 * rf + 0.686 * gf + 0.168 * bf).clamp(0.0, 255.0) as u8;
                let nb = libm::roundf(0.272 * rf + 0.534 * gf + 0.131 * bf).clamp(0.0, 255.0) as u8;
                pack_argb(a, nr, ng, nb)
            }
        }
    }

    /// Automatically computes an optimal Levels adjustment from image pixels.
    pub fn auto_levels(pixels: &[u32]) -> Levels {
        let mut min_r = 255u8;
        let mut max_r = 0u8;
        let mut min_g = 255u8;
        let mut max_g = 0u8;
        let mut min_b = 255u8;
        let mut max_b = 0u8;
        for &p in pixels {
            let (a, r, g, b) = split_argb(p);
            if a > 0 {
                min_r = min_r.min(r);
                max_r = max_r.max(r);
                min_g = min_g.min(g);
                max_g = max_g.max(g);
                min_b = min_b.min(b);
                max_b = max_b.max(b);
            }
        }
        let in_black = min_r.min(min_g).min(min_b);
        let in_white = max_r.max(max_g).max(max_b);
        Levels {
            in_black,
            in_white: if in_white > in_black { in_white } else { 255 },
            gamma_fp: 1.0,
            out_black: 0,
            out_white: 255,
        }
    }

    pub fn apply_to_slice(&self, pixels: &mut [u32], width: usize, mask: Option<&SelectionMask>) {
        if width == 0 {
            return;
        }
        for (i, p) in pixels.iter_mut().enumerate() {
            let (x, y) = (i % width, i / width);
            let weight = match mask {
                Some(m) => m.sample(x, y),
                None => 255,
            };
            if weight == 0 {
                continue;
            }
            let adjusted = self.apply_pixel(*p);
            if weight == 255 {
                *p = adjusted;
            } else {
                let (oa, or, og, ob) = split_argb(*p);
                let (na, nr, ng, nb) = split_argb(adjusted);
                let w = weight as f32 / 255.0;
                let r = libm::roundf(or as f32 + w * (nr as f32 - or as f32)) as u8;
                let g = libm::roundf(og as f32 + w * (ng as f32 - og as f32)) as u8;
                let b = libm::roundf(ob as f32 + w * (nb as f32 - ob as f32)) as u8;
                let a = libm::roundf(oa as f32 + w * (na as f32 - oa as f32)) as u8;
                *p = pack_argb(a, r, g, b);
            }
        }
    }
}

/// Automatically computes an optimal Levels adjustment from image pixels.
pub fn auto_levels(pixels: &[u32]) -> Levels {
    AdjustmentKind::auto_levels(pixels)
}

pub fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;

    let l = (max + min) / 2.0;

    if delta == 0.0 {
        return (0.0, 0.0, l);
    }

    let s = if l < 0.5 {
        delta / (max + min)
    } else {
        delta / (2.0 - max - min)
    };

    let mut h = if max == rf {
        (gf - bf) / delta
    } else if max == gf {
        2.0 + (bf - rf) / delta
    } else {
        4.0 + (rf - gf) / delta
    };

    h *= 60.0;
    if h < 0.0 {
        h += 360.0;
    }

    (h, s, l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    if s == 0.0 {
        let val = libm::roundf(l * 255.0).clamp(0.0, 255.0) as u8;
        return (val, val, val);
    }

    let c = (1.0 - libm::fabsf(2.0 * l - 1.0)) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - libm::fabsf((h_prime % 2.0) - 1.0));
    let m = l - c / 2.0;

    let (r1, g1, b1) = if (0.0..1.0).contains(&h_prime) {
        (c, x, 0.0)
    } else if (1.0..2.0).contains(&h_prime) {
        (x, c, 0.0)
    } else if (2.0..3.0).contains(&h_prime) {
        (0.0, c, x)
    } else if (3.0..4.0).contains(&h_prime) {
        (0.0, x, c)
    } else if (4.0..5.0).contains(&h_prime) {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (
        libm::roundf((r1 + m) * 255.0).clamp(0.0, 255.0) as u8,
        libm::roundf((g1 + m) * 255.0).clamp(0.0, 255.0) as u8,
        libm::roundf((b1 + m) * 255.0).clamp(0.0, 255.0) as u8,
    )
}
