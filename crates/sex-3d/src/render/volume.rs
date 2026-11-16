// SPDX-License-Identifier: GPL-3.0-only
//! Homogeneous participating media: absorption, Henyey-Greenstein scattering
//! and emission, tracked with delta tracking against a constant majorant (the
//! largest extinction channel) and shadowed with analytic transmittance (what
//! ratio tracking converges to in a homogeneous medium, without its noise).
//!
//! The integrator only sees [`VolumeCoeffs`]; today they come from a
//! material's or the world's [`VolumeSettings`] (Principled Volume inputs),
//! and a shader-node evaluator can fill them directly.
//!
//! Heterogeneous media (smoke domains, [`DensityGrid`]) use delta tracking
//! against the grid's maximum density ([`delta_track_fn`]) and ray marching
//! for shadow rays.
use super::rng::Rng;
use super::sampling::onb;
use crate::math::Vec3;
use crate::scene::VolumeSettings;
use core::f32::consts::{FRAC_1_PI, TAU};

/// Per-metre coefficients of a homogeneous medium.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeCoeffs {
    /// Absorption coefficient.
    pub sigma_a: Vec3,
    /// Scattering coefficient.
    pub sigma_s: Vec3,
    /// Emitted radiance per metre.
    pub emission: Vec3,
    /// Henyey-Greenstein mean cosine.
    pub anisotropy: f32,
}

fn clean(v: Vec3) -> Vec3 {
    let c = |x: f32| if x.is_finite() { x.max(0.0) } else { 0.0 };
    Vec3::new(c(v.x), c(v.y), c(v.z))
}

impl VolumeCoeffs {
    /// Cycles' Principled Volume: scattering `color * density`, absorption
    /// `(1 - color) * (1 - sqrt(absorption_color)) * density`.
    pub fn from_settings(v: &VolumeSettings) -> VolumeCoeffs {
        let density = if v.density.is_finite() { v.density.max(0.0) } else { 0.0 };
        let color = clean(v.color);
        let abs_col = clean(v.absorption_color);
        let sq = Vec3::new(abs_col.x.sqrt(), abs_col.y.sqrt(), abs_col.z.sqrt());
        let absorption = (Vec3::ONE - color).max(Vec3::ZERO).mul_elem((Vec3::ONE - sq).max(Vec3::ZERO));
        VolumeCoeffs {
            sigma_a: absorption * density,
            sigma_s: color * density,
            emission: clean(v.emission_color) * if v.emission_strength.is_finite() { v.emission_strength.max(0.0) } else { 0.0 },
            anisotropy: if v.anisotropy.is_finite() { v.anisotropy.clamp(-0.99, 0.99) } else { 0.0 },
        }
    }

    pub fn sigma_t(&self) -> Vec3 {
        self.sigma_a + self.sigma_s
    }

    pub fn is_empty(&self) -> bool {
        self.sigma_t() == Vec3::ZERO && self.emission == Vec3::ZERO
    }

    /// Sum of overlapping media (Cycles adds the closures of every volume
    /// the ray is inside).
    pub fn add(&self, o: &VolumeCoeffs) -> VolumeCoeffs {
        let ws = self.sigma_s.max_elem();
        let wo = o.sigma_s.max_elem();
        let g = if ws + wo > 0.0 { (self.anisotropy * ws + o.anisotropy * wo) / (ws + wo) } else { 0.0 };
        VolumeCoeffs { sigma_a: self.sigma_a + o.sigma_a, sigma_s: self.sigma_s + o.sigma_s, emission: self.emission + o.emission, anisotropy: g }
    }

    /// Transmittance over distance `t`.
    pub fn transmittance(&self, t: f32) -> Vec3 {
        let st = self.sigma_t();
        if !t.is_finite() {
            let f = |s: f32| if s > 0.0 { 0.0 } else { 1.0 };
            return Vec3::new(f(st.x), f(st.y), f(st.z));
        }
        Vec3::new((-st.x * t).exp(), (-st.y * t).exp(), (-st.z * t).exp())
    }

    /// Emission gathered over distance `t`: `Le * integral of T(s) ds`.
    pub fn emission_integral(&self, t: f32) -> Vec3 {
        if self.emission == Vec3::ZERO {
            return Vec3::ZERO;
        }
        let st = self.sigma_t();
        let f = |e: f32, s: f32| {
            if s > 1e-6 {
                let tt = if t.is_finite() { (-s * t).exp() } else { 0.0 };
                e * (1.0 - tt) / s
            } else if t.is_finite() {
                e * t
            } else {
                0.0
            }
        };
        Vec3::new(f(self.emission.x, st.x), f(self.emission.y, st.y), f(self.emission.z, st.z))
    }
}

/// Outcome of tracking a ray through a medium up to a surface at `tmax`.
pub(crate) enum Track {
    /// Reached the end; the throughput was multiplied by the given weight.
    Pass(Vec3),
    /// Real scattering at distance `t`, with this throughput weight.
    Scatter(f32, Vec3),
    /// Absorbed (the path ends).
    Absorb,
}

/// Delta tracking with the majorant `max(sigma_t)` and chromatic null
/// collisions. The returned weights keep the estimator unbiased per channel.
pub(crate) fn delta_track(m: &VolumeCoeffs, tmax: f32, rng: &mut Rng) -> Track {
    delta_track_fn(m.sigma_t().max_elem(), tmax, rng, &mut |_| *m).0
}

/// Delta tracking through a heterogeneous medium whose coefficients at
/// distance `t` are `at(t)`, bounded by `maj` (an upper bound of every
/// channel of `sigma_t`). Also returns the emission gathered on the way
/// (a collision estimator of the integral of `T(s) Le(s)`).
pub(crate) fn delta_track_fn(maj: f32, tmax: f32, rng: &mut Rng, at: &mut dyn FnMut(f32) -> VolumeCoeffs) -> (Track, Vec3) {
    if maj <= 0.0 {
        return (Track::Pass(Vec3::ONE), Vec3::ZERO);
    }
    let mean = |v: Vec3| (v.x + v.y + v.z) / 3.0;
    let mut w = Vec3::ONE;
    let mut le = Vec3::ZERO;
    let mut t = 0.0f32;
    for _ in 0..4096 {
        t -= (1.0 - rng.f32()).ln() / maj;
        if t >= tmax {
            return (Track::Pass(w), le);
        }
        let m = at(t);
        if m.emission != Vec3::ZERO {
            le += w.mul_elem(m.emission) * (1.0 / maj);
        }
        let st = m.sigma_t();
        let (pa, ps) = (mean(m.sigma_a) / maj, mean(m.sigma_s) / maj);
        let null = (Vec3::splat(maj) - st).max(Vec3::ZERO);
        let pn = mean(null) / maj;
        let u = rng.f32() * (pa + ps + pn);
        if u < pa {
            return (Track::Absorb, le);
        }
        if u < pa + ps {
            return (Track::Scatter(t, w.mul_elem(m.sigma_s) * (1.0 / (maj * ps))), le);
        }
        w = w.mul_elem(null) * (1.0 / (maj * pn));
        if w.max_elem() <= 0.0 {
            return (Track::Absorb, le);
        }
    }
    (Track::Absorb, le)
}

/// Transmittance and emission along `[0, tmax]` of a heterogeneous medium
/// by ray marching with `steps` midpoint samples (noise-free, for shadow
/// rays and paths that no longer scatter).
pub(crate) fn march(tmax: f32, steps: usize, at: &mut dyn FnMut(f32) -> VolumeCoeffs) -> (Vec3, Vec3) {
    let steps = steps.max(1);
    let dt = tmax / steps as f32;
    let mut tr = Vec3::ONE;
    let mut le = Vec3::ZERO;
    for i in 0..steps {
        let m = at((i as f32 + 0.5) * dt);
        le += tr.mul_elem(m.emission_integral(dt));
        tr = tr.mul_elem(m.transmittance(dt));
        if tr.max_elem() < 1e-5 {
            return (Vec3::ZERO, le);
        }
    }
    (tr, le)
}

/// A voxel grid of density (and optionally flame) in world space: a
/// smoke domain's cached frame as the renderer sees it. Values sit at cell
/// centres and are interpolated trilinearly; outside the grid they are 0.
#[derive(Clone, Debug)]
pub struct DensityGrid {
    pub res: [usize; 3],
    /// World position of the minimum corner and the cell size.
    pub min: Vec3,
    pub dx: f32,
    pub density: std::sync::Arc<Vec<f32>>,
    pub flame: Option<std::sync::Arc<Vec<f32>>>,
    /// Smoke colour (scattering albedo).
    pub color: Vec3,
    /// Largest density value (for the tracking majorant).
    pub max_density: f32,
    pub max_flame: f32,
}

impl DensityGrid {
    pub fn new(res: [usize; 3], min: Vec3, dx: f32, density: std::sync::Arc<Vec<f32>>, flame: Option<std::sync::Arc<Vec<f32>>>, color: Vec3) -> DensityGrid {
        let mx = |v: &[f32]| v.iter().copied().filter(|x| x.is_finite()).fold(0.0f32, f32::max);
        let max_density = mx(&density);
        let max_flame = flame.as_deref().map_or(0.0, |f| mx(f));
        DensityGrid { res, min, dx, density, flame, color, max_density, max_flame }
    }

    /// World-space bounds.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        (self.min, self.min + Vec3::new(self.res[0] as f32, self.res[1] as f32, self.res[2] as f32) * self.dx)
    }

    fn trilinear(&self, v: &[f32], p: Vec3) -> f32 {
        let [nx, ny, nz] = self.res;
        if nx == 0 || ny == 0 || nz == 0 || v.len() != nx * ny * nz || self.dx <= 0.0 {
            return 0.0;
        }
        let q = (p - self.min) * (1.0 / self.dx) - Vec3::splat(0.5);
        let n = [nx, ny, nz];
        let c = [q.x, q.y, q.z];
        let mut i0 = [0usize; 3];
        let mut f = [0f32; 3];
        for a in 0..3 {
            if !(c[a] > -1.0 && c[a] < n[a] as f32) {
                return 0.0;
            }
            let fl = c[a].floor();
            i0[a] = fl.max(0.0) as usize;
            f[a] = c[a] - fl;
            if fl < 0.0 {
                i0[a] = 0;
                f[a] = 0.0;
            }
        }
        let at = |i: usize, j: usize, k: usize| v[i.min(nx - 1) + nx * (j.min(ny - 1) + ny * k.min(nz - 1))];
        let (i, j, k) = (i0[0], i0[1], i0[2]);
        let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x00 = l(at(i, j, k), at(i + 1, j, k), f[0]);
        let x10 = l(at(i, j + 1, k), at(i + 1, j + 1, k), f[0]);
        let x01 = l(at(i, j, k + 1), at(i + 1, j, k + 1), f[0]);
        let x11 = l(at(i, j + 1, k + 1), at(i + 1, j + 1, k + 1), f[0]);
        l(l(x00, x10, f[1]), l(x01, x11, f[1]), f[2]).max(0.0)
    }

    /// Density at world point `p`.
    pub fn density_at(&self, p: Vec3) -> f32 {
        self.trilinear(&self.density, p)
    }

    /// Flame at world point `p`.
    pub fn flame_at(&self, p: Vec3) -> f32 {
        self.flame.as_ref().map_or(0.0, |f| self.trilinear(f, p))
    }
}

/// Henyey-Greenstein phase function for the angle between `wo` (pointing
/// away, towards the previous vertex) and `wi` (towards the next).
pub(crate) fn hg_eval(g: f32, wo: Vec3, wi: Vec3) -> f32 {
    // Cos of the scattering angle between the incoming travel direction (-wo) and wi.
    let cos = -wo.dot(wi);
    let denom = 1.0 + g * g - 2.0 * g * cos;
    0.25 * FRAC_1_PI * (1.0 - g * g) / (denom * denom.max(1e-12).sqrt())
}

/// Sample `wi` from HG around the travel direction `-wo`; returns (wi, pdf).
pub(crate) fn hg_sample(g: f32, wo: Vec3, u1: f32, u2: f32) -> (Vec3, f32) {
    let cos = if g.abs() < 1e-3 {
        1.0 - 2.0 * u1
    } else {
        let s = (1.0 - g * g) / (1.0 + g - 2.0 * g * u1);
        (1.0 + g * g - s * s) / (2.0 * g)
    };
    let cos = cos.clamp(-1.0, 1.0);
    let sin = (1.0 - cos * cos).max(0.0).sqrt();
    let phi = TAU * u2;
    let axis = -wo;
    let (t, b) = onb(axis);
    let wi = (t * (sin * phi.cos()) + b * (sin * phi.sin()) + axis * cos).normalize();
    (wi, hg_eval(g, wo, wi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hg_normalises_and_samples_its_pdf() {
        for g in [0.0, 0.5, -0.7] {
            let wo = Vec3::new(0.0, 0.0, 1.0);
            let mut rng = Rng::new(3, 9);
            // Integral of the pdf over the sphere via its own samples: E[1] = 1;
            // and E[cos] = g for the travel direction.
            let n = 20000;
            let mut mean_cos = 0.0;
            for _ in 0..n {
                let (wi, pdf) = hg_sample(g, wo, rng.f32(), rng.f32());
                assert!(pdf > 0.0 && (pdf - hg_eval(g, wo, wi)).abs() < 1e-3 * pdf.max(1.0));
                mean_cos += (-wo).dot(wi);
            }
            mean_cos /= n as f32;
            assert!((mean_cos - g).abs() < 0.03, "g {g}: {mean_cos}");
        }
    }

    #[test]
    fn delta_tracking_matches_beer_lambert() {
        let m = VolumeCoeffs { sigma_a: Vec3::new(0.2, 0.5, 1.0), sigma_s: Vec3::ZERO, ..Default::default() };
        let mut rng = Rng::new(1, 2);
        let n = 40000;
        let mut acc = Vec3::ZERO;
        for _ in 0..n {
            if let Track::Pass(w) = delta_track(&m, 1.5, &mut rng) {
                acc += w;
            }
        }
        let est = acc * (1.0 / n as f32);
        let exact = m.transmittance(1.5);
        assert!(est.approx_eq(exact, 0.02), "{est:?} vs {exact:?}");
    }
}
