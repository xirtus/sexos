// SPDX-License-Identifier: GPL-3.0-only
//! Homogeneous participating media: absorption, Henyey-Greenstein scattering
//! and emission, tracked with delta tracking against a constant majorant (the
//! largest extinction channel) and shadowed with analytic transmittance (what
//! ratio tracking converges to in a homogeneous medium, without its noise).
//!
//! The integrator only sees [`VolumeCoeffs`]; today they come from a
//! material's or the world's [`VolumeSettings`] (Principled Volume inputs),
//! and a shader-node evaluator can fill them directly.
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
    let st = m.sigma_t();
    let maj = st.max_elem();
    if maj <= 0.0 {
        return Track::Pass(Vec3::ONE);
    }
    let mean = |v: Vec3| (v.x + v.y + v.z) / 3.0;
    let (pa, ps) = (mean(m.sigma_a) / maj, mean(m.sigma_s) / maj);
    let null = Vec3::splat(maj) - st;
    let pn = mean(null) / maj;
    let mut w = Vec3::ONE;
    let mut t = 0.0f32;
    for _ in 0..1024 {
        t -= (1.0 - rng.f32()).ln() / maj;
        if t >= tmax {
            return Track::Pass(w);
        }
        let u = rng.f32() * (pa + ps + pn);
        if u < pa {
            return Track::Absorb;
        }
        if u < pa + ps {
            return Track::Scatter(t, w.mul_elem(m.sigma_s) * (1.0 / (maj * ps)));
        }
        w = w.mul_elem(null) * (1.0 / (maj * pn));
        if w.max_elem() <= 0.0 {
            return Track::Absorb;
        }
    }
    Track::Absorb
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
