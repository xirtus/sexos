// SPDX-License-Identifier: GPL-3.0-only
//! Non-destructive adjustments and color correction filters.

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

#[derive(Clone, Debug, PartialEq)]
pub struct Curves {
    pub master: Vec<(u8, u8)>,
    pub red: Vec<(u8, u8)>,
    pub green: Vec<(u8, u8)>,
    pub blue: Vec<(u8, u8)>,
}

impl Default for Curves {
    fn default() -> Self {
        Self {
            master: vec![(0, 0), (255, 255)],
            red: vec![(0, 0), (255, 255)],
            green: vec![(0, 0), (255, 255)],
            blue: vec![(0, 0), (255, 255)],
        }
    }
}

impl Curves {
    pub fn build_lut(points: &[(u8, u8)]) -> [u8; 256] {
        let mut lut = [0u8; 256];
        if points.is_empty() {
            for i in 0..=255 {
                lut[i] = i as u8;
            }
            return lut;
        }

        let mut pts = points.to_vec();
        pts.sort_by_key(|p| p.0);

        for i in 0..=255 {
            let x = i as u8;
            if x <= pts[0].0 {
                lut[i] = pts[0].1;
            } else if x >= pts.last().unwrap().0 {
                lut[i] = pts.last().unwrap().1;
            } else {
                for w in pts.windows(2) {
                    let (x0, y0) = (w[0].0, w[0].1);
                    let (x1, y1) = (w[1].0, w[1].1);
                    if x >= x0 && x <= x1 {
                        let span = (x1 - x0) as f32;
                        let t = if span > 0.0 {
                            (x - x0) as f32 / span
                        } else {
                            0.0
                        };
                        let y = y0 as f32 + t * (y1 as f32 - y0 as f32);
                        lut[i] = libm::roundf(y).clamp(0.0, 255.0) as u8;
                        break;
                    }
                }
            }
        }
        lut
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Adjustment {
    Levels(Levels),
    Curves(Curves),
    BrightnessContrast { brightness: f32, contrast: f32 },
    HueSaturationLightness { hue: f32, saturation: f32, lightness: f32 },
    Invert,
    Posterize { levels: u8 },
    Threshold { cutoff: u8 },
    Desaturate,
}

impl Adjustment {
    pub fn apply_pixel(&self, pixel: u32) -> u32 {
        let (a, r, g, b) = split_argb(pixel);
        if a == 0 {
            return pixel;
        }

        match self {
            Adjustment::Levels(levels) => {
                let lut = levels.build_lut();
                pack_argb(a, lut[r as usize], lut[g as usize], lut[b as usize])
            }
            Adjustment::Curves(curves) => {
                let lut_m = Curves::build_lut(&curves.master);
                let lut_r = Curves::build_lut(&curves.red);
                let lut_g = Curves::build_lut(&curves.green);
                let lut_b = Curves::build_lut(&curves.blue);
                let out_r = lut_r[lut_m[r as usize] as usize];
                let out_g = lut_g[lut_m[g as usize] as usize];
                let out_b = lut_b[lut_m[b as usize] as usize];
                pack_argb(a, out_r, out_g, out_b)
            }
            Adjustment::BrightnessContrast { brightness, contrast } => {
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
            Adjustment::HueSaturationLightness { hue, saturation, lightness } => {
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
            Adjustment::Invert => pack_argb(a, 255 - r, 255 - g, 255 - b),
            Adjustment::Posterize { levels } => {
                let n = (*levels).max(2) as f32;
                let step = 255.0 / (n - 1.0);
                let map = |c: u8| -> u8 {
                    let bucket = libm::roundf(c as f32 / step);
                    libm::roundf(bucket * step).clamp(0.0, 255.0) as u8
                };
                pack_argb(a, map(r), map(g), map(b))
            }
            Adjustment::Threshold { cutoff } => {
                let lum = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
                let val = if lum >= *cutoff { 255 } else { 0 };
                pack_argb(a, val, val, val)
            }
            Adjustment::Desaturate => {
                let lum = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
                pack_argb(a, lum, lum, lum)
            }
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

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
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

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
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
