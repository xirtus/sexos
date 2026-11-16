// SPDX-License-Identifier: GPL-3.0-only
//! The world light probe: the environment's radiance captured into an
//! equirectangular map, prefiltered for GGX at a range of roughnesses
//! (split-sum, filtered importance sampling from a box-filtered pyramid),
//! and projected onto order-2 spherical harmonics convolved with the cosine
//! lobe for diffuse irradiance. Also our procedural studio environment for
//! Material Preview.
use crate::math::Vec3;
use crate::render::parallel_for_each;
use core::f32::consts::{PI, TAU};

/// Roughness levels of the prefiltered chain (0, 0.2, .. 1).
const LEVELS: usize = 6;
const BASE_W: usize = 256;
const BASE_H: usize = 128;

/// An equirectangular map, row 0 at the top (+Z).
#[derive(Clone, Debug)]
pub(crate) struct EqMap {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Vec3>,
}

/// Blender's equirectangular mapping (as the renderer's): direction to
/// (u, v) with v = 0 at the bottom.
#[inline]
pub(crate) fn dir_to_uv(d: Vec3) -> (f32, f32) {
    let u = -d.y.atan2(d.x) / TAU + 0.5;
    let v = d.z.atan2((d.x * d.x + d.y * d.y).sqrt()) / PI + 0.5;
    (u, v)
}

#[inline]
fn uv_to_dir(u: f32, v: f32) -> Vec3 {
    let phi = (0.5 - u) * TAU;
    let el = (v - 0.5) * PI;
    Vec3::new(phi.cos() * el.cos(), phi.sin() * el.cos(), el.sin())
}

impl EqMap {
    /// Direction of the centre of texel (`x`, `y`).
    fn texel_dir(&self, x: usize, y: usize) -> Vec3 {
        uv_to_dir((x as f32 + 0.5) / self.w as f32, 1.0 - (y as f32 + 0.5) / self.h as f32)
    }

    pub fn capture(w: usize, h: usize, threads: usize, f: &(dyn Fn(Vec3) -> Vec3 + Sync)) -> EqMap {
        let mut m = EqMap { w, h, px: vec![Vec3::ZERO; w * h] };
        let probe = EqMap { w, h, px: Vec::new() };
        parallel_for_each(m.px.chunks_mut(w).enumerate(), threads, |(y, row)| {
            for (x, p) in row.iter_mut().enumerate() {
                let c = f(probe.texel_dir(x, y));
                *p = if c.x.is_finite() && c.y.is_finite() && c.z.is_finite() { c.max(Vec3::ZERO) } else { Vec3::ZERO };
            }
        });
        m
    }

    /// Bilinear lookup, wrapping in u and clamping in v.
    #[inline]
    pub fn sample(&self, d: Vec3) -> Vec3 {
        let (u, v) = dir_to_uv(d);
        let x = u * self.w as f32 - 0.5;
        let y = (1.0 - v) * self.h as f32 - 0.5;
        let x0 = x.floor();
        let y0 = y.floor().clamp(0.0, self.h as f32 - 1.0);
        let (fx, fy) = (x - x0, (y - y0).clamp(0.0, 1.0));
        let xi = |k: f32| (k as i64).rem_euclid(self.w as i64) as usize;
        let (xa, xb) = (xi(x0), xi(x0 + 1.0));
        let ya = y0 as usize;
        let yb = (ya + 1).min(self.h - 1);
        let p = |xx: usize, yy: usize| self.px[yy * self.w + xx];
        (p(xa, ya) * (1.0 - fx) + p(xb, ya) * fx) * (1.0 - fy) + (p(xa, yb) * (1.0 - fx) + p(xb, yb) * fx) * fy
    }

    /// Half resolution: 2 x 2 texels averaged by solid angle, so the mean
    /// radiance is kept.
    fn half(&self) -> EqMap {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let row_w = |y: usize| ((y as f32 + 0.5) / self.h as f32 * PI).sin();
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            let (ya, yb) = ((y * 2).min(self.h - 1), (y * 2 + 1).min(self.h - 1));
            let (wa, wb) = (row_w(ya), row_w(yb));
            let norm = 0.5 / (wa + wb).max(1e-12);
            for x in 0..w {
                let (xa, xb) = ((x * 2).min(self.w - 1), (x * 2 + 1).min(self.w - 1));
                let s = (self.px[ya * self.w + xa] + self.px[ya * self.w + xb]) * wa + (self.px[yb * self.w + xa] + self.px[yb * self.w + xb]) * wb;
                px.push(s * norm);
            }
        }
        EqMap { w, h, px }
    }

    /// Solid-angle weighted mean.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mean(&self) -> Vec3 {
        let mut s = Vec3::ZERO;
        let mut wsum = 0.0;
        for y in 0..self.h {
            let wgt = ((y as f32 + 0.5) / self.h as f32 * PI).sin();
            for x in 0..self.w {
                s += self.px[y * self.w + x] * wgt;
            }
            wsum += wgt * self.w as f32;
        }
        s / wsum.max(1e-12)
    }
}

/// The prefiltered world.
#[derive(Clone, Debug)]
pub(crate) struct Probe {
    /// Prefiltered radiance per roughness level (level 0 is the capture).
    levels: Vec<EqMap>,
    /// Irradiance SH (cosine-convolved), order 2.
    sh: [Vec3; 9],
}

fn sh_basis(d: Vec3) -> [f32; 9] {
    let (x, y, z) = (d.x, d.y, d.z);
    [0.282_095, 0.488_603 * y, 0.488_603 * z, 0.488_603 * x, 1.092_548 * x * y, 1.092_548 * y * z, 0.315_392 * (3.0 * z * z - 1.0), 1.092_548 * x * z, 0.546_274 * (x * x - y * y)]
}

fn radical_inverse(mut b: u32) -> f32 {
    b = b.rotate_right(16);
    b = ((b & 0x5555_5555) << 1) | ((b & 0xAAAA_AAAA) >> 1);
    b = ((b & 0x3333_3333) << 2) | ((b & 0xCCCC_CCCC) >> 2);
    b = ((b & 0x0F0F_0F0F) << 4) | ((b & 0xF0F0_F0F0) >> 4);
    b = ((b & 0x00FF_00FF) << 8) | ((b & 0xFF00_FF00) >> 8);
    b as f32 * 2.328_306_4e-10
}

impl Probe {
    /// Capture `radiance` (world direction to radiance) and prefilter it.
    pub fn build(threads: usize, radiance: &(dyn Fn(Vec3) -> Vec3 + Sync)) -> Probe {
        let base = EqMap::capture(BASE_W, BASE_H, threads, radiance);
        Probe::from_map(base, threads)
    }

    pub fn from_map(base: EqMap, threads: usize) -> Probe {
        // Box-filtered pyramid for filtered importance sampling.
        let mut pyramid = vec![base.clone()];
        while pyramid.last().is_some_and(|m| m.w > 8) {
            let next = pyramid.last().map(EqMap::half).unwrap_or_else(|| base.clone());
            pyramid.push(next);
        }
        let texel_sa0 = 4.0 * PI / (base.w * base.h) as f32;
        let lookup = |d: Vec3, lod: f32| -> Vec3 {
            let lod = lod.clamp(0.0, (pyramid.len() - 1) as f32);
            let l0 = lod.floor() as usize;
            let l1 = (l0 + 1).min(pyramid.len() - 1);
            let t = lod - l0 as f32;
            pyramid[l0].sample(d) * (1.0 - t) + pyramid[l1].sample(d) * t
        };
        let mut levels = vec![base.clone()];
        const SAMPLES: u32 = 256;
        for k in 1..LEVELS {
            let rough = k as f32 / (LEVELS - 1) as f32;
            let a = (rough * rough).max(1e-3);
            let a2 = a * a;
            let (w, h) = ((BASE_W >> k).max(32), (BASE_H >> k).max(16));
            let mut m = EqMap { w, h, px: vec![Vec3::ZERO; w * h] };
            let dirs = EqMap { w, h, px: Vec::new() };
            parallel_for_each(m.px.chunks_mut(w).enumerate(), threads, |(y, row)| {
                for (x, out) in row.iter_mut().enumerate() {
                    let n = dirs.texel_dir(x, y);
                    let (t, b) = crate::render::sampling::onb(n);
                    let mut sum = Vec3::ZERO;
                    let mut wsum = 0.0;
                    // Per-texel rotation of the point set (Cranley-Patterson)
                    // so neighbouring texels do not alias the same way.
                    let rot = radical_inverse((y * w + x) as u32 ^ 0x9E37_79B9);
                    for i in 0..SAMPLES {
                        let (u1, u2) = ((i as f32 + 0.5) / SAMPLES as f32, (radical_inverse(i) + rot).fract());
                        // GGX half vector around n (n = v = r).
                        let cos_t = ((1.0 - u1) / (1.0 + (a2 - 1.0) * u1)).sqrt();
                        let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
                        let phi = TAU * u2;
                        let hv = t * (sin_t * phi.cos()) + b * (sin_t * phi.sin()) + n * cos_t;
                        let l = hv * (2.0 * n.dot(hv)) - n;
                        let ndl = n.dot(l);
                        if ndl <= 0.0 {
                            continue;
                        }
                        // pdf of l = D(h) / 4 (n = v); lod from its solid angle.
                        let d = a2 / (PI * (cos_t * cos_t * (a2 - 1.0) + 1.0).powi(2));
                        let sa = 4.0 / (SAMPLES as f32 * d.max(1e-6));
                        let lod = 0.5 * (sa / texel_sa0).log2() + 1.0;
                        sum += lookup(l, lod) * ndl;
                        wsum += ndl;
                    }
                    *out = sum / wsum.max(1e-6);
                }
            });
            levels.push(m);
        }
        // Irradiance SH from a small map.
        let small = &pyramid[2.min(pyramid.len() - 1)];
        let mut l = [Vec3::ZERO; 9];
        for y in 0..small.h {
            let theta = (y as f32 + 0.5) / small.h as f32 * PI;
            let sa = theta.sin() * (PI / small.h as f32) * (TAU / small.w as f32);
            for x in 0..small.w {
                let d = small.texel_dir(x, y);
                let c = small.px[y * small.w + x] * sa;
                for (k, y) in sh_basis(d).iter().enumerate() {
                    l[k] += c * *y;
                }
            }
        }
        let band = [PI, 2.0 * PI / 3.0, 2.0 * PI / 3.0, 2.0 * PI / 3.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0];
        let sh = core::array::from_fn(|k| l[k] * band[k]);
        Probe { levels, sh }
    }

    /// Prefiltered radiance around `d` for GGX `roughness` (0..1).
    #[inline]
    pub fn specular(&self, d: Vec3, roughness: f32) -> Vec3 {
        let f = roughness.clamp(0.0, 1.0) * (LEVELS - 1) as f32;
        let l0 = (f as usize).min(LEVELS - 1);
        let l1 = (l0 + 1).min(LEVELS - 1);
        let t = f - l0 as f32;
        if t <= 1e-4 || l0 == l1 {
            return self.levels[l0].sample(d);
        }
        self.levels[l0].sample(d) * (1.0 - t) + self.levels[l1].sample(d) * t
    }

    /// Irradiance (cosine-weighted incoming radiance) around normal `n`;
    /// a Lambertian surface reflects `albedo * E / pi`.
    #[inline]
    pub fn irradiance(&self, n: Vec3) -> Vec3 {
        let b = sh_basis(n);
        let mut e = Vec3::ZERO;
        for k in 0..9 {
            e += self.sh[k] * b[k];
        }
        e.max(Vec3::ZERO)
    }

    /// Mean radiance of a level (for tests and volumes).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mean(&self, level: usize) -> Vec3 {
        self.levels[level.min(LEVELS - 1)].mean()
    }
}

/// Our studio environment for Material Preview, in view space (x right,
/// y up, z towards the viewer): a soft sky-to-floor gradient with a large
/// key softbox above left, a fill on the right, a rim strip behind and a
/// warm floor bounce.
pub(crate) fn studio(d: Vec3) -> Vec3 {
    let d = d.normalize();
    let up = d.y;
    let sky = Vec3::new(0.42, 0.45, 0.5);
    let horizon = Vec3::new(0.28, 0.28, 0.28);
    let floor = Vec3::new(0.16, 0.145, 0.13);
    let mut c = if up >= 0.0 { horizon + (sky - horizon) * up.powf(0.6) } else { horizon + (floor - horizon) * (-up).powf(0.35) };
    let soft = |axis: Vec3, radius: f32, edge: f32| {
        let cos = d.dot(axis.normalize());
        let ang = cos.clamp(-1.0, 1.0).acos();
        1.0 - ((ang - radius) / edge).clamp(0.0, 1.0)
    };
    // Key: above left in front.
    c += Vec3::new(1.0, 0.97, 0.92) * (6.0 * soft(Vec3::new(-0.55, 0.62, 0.56), 0.28, 0.12));
    // Fill: right, lower, cooler.
    c += Vec3::new(0.8, 0.86, 1.0) * (1.6 * soft(Vec3::new(0.85, 0.15, 0.5), 0.35, 0.2));
    // Rim strip behind, above.
    c += Vec3::new(1.0, 1.0, 1.0) * (3.5 * soft(Vec3::new(0.25, 0.55, -0.8), 0.16, 0.08));
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_world_irradiance_and_energy() {
        let p = Probe::build(2, &|_| Vec3::splat(0.5));
        for n in [Vec3::Z, Vec3::X, -Vec3::Y, Vec3::new(0.3, -0.5, 0.8).normalize()] {
            let e = p.irradiance(n);
            assert!((e.x - 0.5 * PI).abs() < 0.02, "{e:?}");
            for r in [0.0, 0.3, 0.7, 1.0] {
                let s = p.specular(n, r);
                assert!((s.x - 0.5).abs() < 0.01, "{r}: {s:?}");
            }
        }
    }

    #[test]
    fn prefiltering_keeps_energy() {
        // A small bright sun and a dim sky: blurring spreads the sun out
        // but keeps the mean radiance.
        let sun = Vec3::new(0.3, 0.4, 0.8).normalize();
        let p = Probe::build(4, &|d| if d.dot(sun) > 0.995 { Vec3::splat(200.0) } else { Vec3::splat(0.2) });
        let m0 = p.mean(0).x;
        for k in 1..LEVELS {
            let m = p.mean(k).x;
            assert!((m - m0).abs() < 0.05 * m0, "level {k}: {m} vs {m0}");
        }
        // Sharp at roughness 0, spread at 1.
        assert!(p.specular(sun, 0.0).x > 10.0 * p.specular(sun, 1.0).x);
        assert!(p.specular(-sun, 1.0).x < p.specular(sun, 1.0).x);
        // Irradiance towards the sun beats away from it.
        assert!(p.irradiance(sun).x > p.irradiance(-sun).x * 2.0);
    }
}
