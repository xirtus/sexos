// SPDX-License-Identifier: GPL-3.0-only
//! Sampling helpers: piecewise-constant distributions and direction warps.
use crate::math::Vec3;
use core::f32::consts::{PI, TAU};

/// Piecewise-constant 1D distribution over `n` buckets.
#[derive(Clone, Debug)]
pub(crate) struct Distribution1D {
    func: Vec<f32>,
    /// `n + 1` entries, 0 to 1.
    cdf: Vec<f32>,
    /// Sum of `func`.
    total: f32,
}

impl Distribution1D {
    /// `None` when every weight is zero (or there are none).
    pub fn new(func: &[f32]) -> Option<Distribution1D> {
        let mut cdf = Vec::with_capacity(func.len() + 1);
        let mut acc = 0.0f64;
        cdf.push(0.0);
        for &f in func {
            acc += f.max(0.0) as f64;
            cdf.push(acc as f32);
        }
        if acc <= 0.0 || !acc.is_finite() {
            return None;
        }
        let inv = (1.0 / acc) as f32;
        for c in &mut cdf {
            *c *= inv;
        }
        *cdf.last_mut().unwrap() = 1.0;
        Some(Distribution1D { func: func.iter().map(|f| f.max(0.0)).collect(), cdf, total: acc as f32 })
    }

    pub fn total(&self) -> f32 {
        self.total
    }

    /// Bucket containing `u` in [0, 1).
    fn find(&self, u: f32) -> usize {
        // Last cdf entry <= u, skipping zero-width buckets.
        let i = self.cdf.partition_point(|&c| c <= u);
        i.saturating_sub(1).min(self.func.len() - 1)
    }

    /// Pick a bucket; returns it and its probability.
    pub fn sample_discrete(&self, u: f32) -> (usize, f32) {
        let i = self.find(u);
        (i, self.pmf(i))
    }

    pub fn pmf(&self, i: usize) -> f32 {
        self.func[i] / self.total
    }

    /// Continuous sample in [0, 1): (position, density, bucket).
    pub fn sample_continuous(&self, u: f32) -> (f32, f32, usize) {
        let i = self.find(u);
        let (c0, c1) = (self.cdf[i], self.cdf[i + 1]);
        let du = if c1 > c0 { (u - c0) / (c1 - c0) } else { 0.5 };
        let n = self.func.len() as f32;
        (((i as f32 + du.clamp(0.0, 0.999_999)) / n).min(0.999_999), self.func[i] * n / self.total, i)
    }

    /// Density of the continuous distribution in bucket `i`.
    pub fn density(&self, i: usize) -> f32 {
        self.func[i] * self.func.len() as f32 / self.total
    }
}

/// Orthonormal basis (t, b) around unit `n` (Duff et al. 2017).
pub(crate) fn onb(n: Vec3) -> (Vec3, Vec3) {
    let s = if n.z >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    (Vec3::new(1.0 + s * n.x * n.x * a, s * b, -s * n.x), Vec3::new(b, s + n.y * n.y * a, -n.y))
}

/// Cosine-weighted direction about +Z; pdf = z / pi.
pub(crate) fn cosine_hemisphere(u1: f32, u2: f32) -> Vec3 {
    let r = u1.sqrt();
    let phi = TAU * u2;
    Vec3::new(r * phi.cos(), r * phi.sin(), (1.0 - u1).max(0.0).sqrt())
}

/// Uniform direction on the sphere; pdf = 1 / (4 pi).
pub(crate) fn uniform_sphere(u1: f32, u2: f32) -> Vec3 {
    let z = 1.0 - 2.0 * u1;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = TAU * u2;
    Vec3::new(r * phi.cos(), r * phi.sin(), z)
}

/// Uniform direction in the cone of `cos_max` about +Z.
pub(crate) fn uniform_cone(u1: f32, u2: f32, cos_max: f32) -> Vec3 {
    let z = 1.0 - u1 * (1.0 - cos_max);
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = TAU * u2;
    Vec3::new(r * phi.cos(), r * phi.sin(), z)
}

/// Uniform point on the unit disk (concentric mapping).
pub(crate) fn concentric_disk(u1: f32, u2: f32) -> (f32, f32) {
    let (a, b) = (2.0 * u1 - 1.0, 2.0 * u2 - 1.0);
    if a == 0.0 && b == 0.0 {
        return (0.0, 0.0);
    }
    let (r, th) = if a.abs() > b.abs() { (a, PI / 4.0 * (b / a)) } else { (b, PI / 2.0 - PI / 4.0 * (a / b)) };
    (r * th.cos(), r * th.sin())
}

/// Power heuristic (beta = 2) weight for the strategy with density `a`.
pub(crate) fn power_heuristic(a: f32, b: f32) -> f32 {
    let (a2, b2) = (a * a, b * b);
    if a2 + b2 > 0.0 && (a2 + b2).is_finite() {
        a2 / (a2 + b2)
    } else if a.is_infinite() {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_samples_proportionally() {
        let d = Distribution1D::new(&[1.0, 0.0, 3.0]).unwrap();
        assert_eq!(d.sample_discrete(0.1).0, 0);
        assert_eq!(d.sample_discrete(0.3).0, 2);
        assert!((d.pmf(2) - 0.75).abs() < 1e-6);
        let (x, pdf, i) = d.sample_continuous(0.5);
        assert_eq!(i, 2);
        assert!(x >= 2.0 / 3.0 && (pdf - 2.25).abs() < 1e-5);
        assert!(Distribution1D::new(&[0.0, 0.0]).is_none());
    }

    #[test]
    fn onb_is_orthonormal() {
        for n in [Vec3::Z, -Vec3::Z, Vec3::new(0.3, -0.5, 0.8).normalize()] {
            let (t, b) = onb(n);
            assert!(t.dot(b).abs() < 1e-5 && t.dot(n).abs() < 1e-5 && (t.length() - 1.0).abs() < 1e-5);
            assert!(t.cross(b).approx_eq(n, 1e-5));
        }
    }
}
