// SPDX-License-Identifier: GPL-3.0-only
//! Edge-avoiding à-trous wavelet denoiser (Dammertz et al. 2010, with the
//! variance-guided luminance weight of SVGF, Schied et al. 2017).
//!
//! Colour is first divided by the albedo AOV so the filter smooths lighting,
//! not texture, and multiplied back at the end. Each of the iterations runs a
//! 5x5 B3-spline kernel with holes (step 1, 2, 4, ...) whose taps are weighted
//! by normal similarity, albedo similarity and luminance difference relative
//! to the locally estimated noise level.
use super::{luminance, parallel_for_each, resolve_threads, RenderResult};
use crate::math::Vec3;
use crate::scene::Image;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiseSettings {
    /// Number of à-trous passes (filter radius 2^iterations * 2 pixels).
    pub iterations: u32,
    /// Luminance edge-stopping strength in noise standard deviations.
    pub sigma_luminance: f32,
    /// Normal edge-stopping exponent.
    pub sigma_normal: f32,
    /// Albedo edge-stopping width.
    pub sigma_albedo: f32,
    /// Worker threads (0 = all cores).
    pub threads: usize,
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        DenoiseSettings { iterations: 5, sigma_luminance: 4.0, sigma_normal: 64.0, sigma_albedo: 0.1, threads: 0 }
    }
}

/// Denoise a render result using its AOVs and variance.
pub fn denoise(r: &RenderResult, settings: &DenoiseSettings) -> Image {
    let var = if r.variance.len() == r.color.pixels.len() { Some(r.variance.as_slice()) } else { None };
    denoise_images(&r.color, &r.albedo, &r.normal, var, settings)
}

/// Denoise `color` guided by `albedo` and `normal` (same size). Without a
/// per-pixel `variance` the noise level is estimated from a 3x3 neighbourhood.
pub fn denoise_images(color: &Image, albedo: &Image, normal: &Image, variance: Option<&[f32]>, settings: &DenoiseSettings) -> Image {
    let (w, h) = (color.width as usize, color.height as usize);
    let n = w * h;
    if n == 0 || albedo.pixels.len() != n || normal.pixels.len() != n {
        return color.clone();
    }
    let v3 = |p: &[f32; 4]| Vec3::new(p[0], p[1], p[2]);
    let albedo: Vec<Vec3> = albedo.pixels.iter().map(v3).collect();
    let normal: Vec<Vec3> = normal.pixels.iter().map(|p| v3(p).normalize()).collect();
    // Demodulate.
    let demod: Vec<Vec3> = albedo.iter().map(|a| a.max(Vec3::splat(0.01))).collect();
    let mut irr: Vec<Vec3> = color.pixels.iter().zip(&demod).map(|(c, a)| Vec3::new(c[0] / a.x, c[1] / a.y, c[2] / a.z)).collect();
    let mut var: Vec<f32> = match variance {
        Some(v) if v.len() == n => v.iter().zip(&demod).map(|(v, a)| v / luminance(*a).powi(2).max(1e-4)).collect(),
        _ => spatial_variance(&irr, w, h),
    };
    let threads = resolve_threads(settings.threads);
    const K: [f32; 3] = [3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0];
    let mut next_irr = vec![Vec3::ZERO; n];
    let mut next_var = vec![0.0f32; n];
    for it in 0..settings.iterations {
        let step = 1usize << it;
        let blurred = blur_variance(&var, w, h);
        let (irr_r, var_r) = (&irr, &var);
        let rows = 8;
        parallel_for_each(next_irr.chunks_mut(w * rows).zip(next_var.chunks_mut(w * rows)).enumerate(), threads, |(band, (out_i, out_v))| {
            for (k, (oi, ov)) in out_i.iter_mut().zip(out_v.iter_mut()).enumerate() {
                let (x, y) = (k % w, band * rows + k / w);
                let p = y * w + x;
                let (cp, np, ap) = (irr_r[p], normal[p], albedo[p]);
                let lp = luminance(cp);
                let sigma = settings.sigma_luminance * blurred[p].max(0.0).sqrt() + 1e-6;
                let (mut sum, mut wsum, mut vsum) = (Vec3::ZERO, 0.0f32, 0.0f32);
                for dy in -2i32..=2 {
                    let yy = y as i64 + (dy as i64) * step as i64;
                    if yy < 0 || yy >= h as i64 {
                        continue;
                    }
                    for dx in -2i32..=2 {
                        let xx = x as i64 + (dx as i64) * step as i64;
                        if xx < 0 || xx >= w as i64 {
                            continue;
                        }
                        let q = yy as usize * w + xx as usize;
                        let kw = K[dx.unsigned_abs() as usize] * K[dy.unsigned_abs() as usize];
                        let wn = match (np == Vec3::ZERO, normal[q] == Vec3::ZERO) {
                            (true, true) => 1.0,
                            (false, false) => np.dot(normal[q]).max(0.0).powf(settings.sigma_normal),
                            _ => 0.0,
                        };
                        if wn <= 0.0 {
                            continue;
                        }
                        let da = (ap - albedo[q]).length_squared();
                        let wa = (-da / (settings.sigma_albedo * settings.sigma_albedo)).exp();
                        let wl = (-(lp - luminance(irr_r[q])).abs() / sigma).exp();
                        let wt = kw * wn * wa * wl;
                        sum += irr_r[q] * wt;
                        wsum += wt;
                        vsum += wt * wt * var_r[q];
                    }
                }
                if wsum > 0.0 {
                    *oi = sum / wsum;
                    *ov = vsum / (wsum * wsum);
                } else {
                    *oi = cp;
                    *ov = var_r[p];
                }
            }
        });
        std::mem::swap(&mut irr, &mut next_irr);
        std::mem::swap(&mut var, &mut next_var);
    }
    let pixels = irr.iter().zip(&demod).zip(&color.pixels).map(|((c, a), src)| [c.x * a.x, c.y * a.y, c.z * a.z, src[3]]).collect();
    Image { name: format!("{} (denoised)", color.name), width: color.width, height: color.height, pixels }
}

/// 3x3 Gaussian blur of the variance (stabilises the luminance weight).
fn blur_variance(v: &[f32], w: usize, h: usize) -> Vec<f32> {
    const K: [f32; 2] = [0.25, 0.125];
    let mut out = vec![0.0; v.len()];
    for y in 0..h {
        for x in 0..w {
            let (mut s, mut ws) = (0.0, 0.0);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                    if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                        continue;
                    }
                    let k = K[dx.unsigned_abs() as usize] * K[dy.unsigned_abs() as usize] * 4.0;
                    s += k * v[yy as usize * w + xx as usize];
                    ws += k;
                }
            }
            out[y * w + x] = s / ws;
        }
    }
    out
}

/// Luminance variance over each pixel's 3x3 neighbourhood.
fn spatial_variance(c: &[Vec3], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; c.len()];
    for y in 0..h {
        for x in 0..w {
            let (mut s, mut s2, mut k) = (0.0, 0.0, 0.0);
            for yy in y.saturating_sub(1)..(y + 2).min(h) {
                for xx in x.saturating_sub(1)..(x + 2).min(w) {
                    let l = luminance(c[yy * w + xx]);
                    s += l;
                    s2 += l * l;
                    k += 1.0;
                }
            }
            let m = s / k;
            out[y * w + x] = (s2 / k - m * m).max(0.0);
        }
    }
    out
}
