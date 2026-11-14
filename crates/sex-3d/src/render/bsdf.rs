// SPDX-License-Identifier: GPL-3.0-only
//! A compact Principled-style BSDF.
//!
//! Three components, weighted like Blender's Principled BSDF:
//! - metal (`metallic`): GGX reflection with Schlick Fresnel tinted by the base colour;
//! - opaque dielectric (`(1 - metallic) (1 - transmission)`): GGX reflection
//!   with exact dielectric Fresnel for `ior`, over Lambert diffuse scaled by
//!   the Fresnel transmittance;
//! - glass (`(1 - metallic) transmission`): dielectric reflection/refraction
//!   through GGX-sampled microfacets, treated as a singular (delta) event.
//!
//! The GGX lobes are sampled from the distribution of visible normals.
//! Everything works in a local frame where the shading normal is +Z and the
//! outgoing direction lies above it.
use super::luminance;
use super::rng::Rng;
use super::sampling::{cosine_hemisphere, onb};
use crate::math::Vec3;
use core::f32::consts::{FRAC_1_PI, PI, TAU};

pub(crate) struct Bsdf {
    t: Vec3,
    b: Vec3,
    n: Vec3,
    /// Geometric normal on the side of the outgoing direction.
    ng: Vec3,
    base: Vec3,
    metallic: f32,
    /// Opaque dielectric weight.
    dielectric: f32,
    glass: f32,
    alpha: f32,
    ior: f32,
    /// The outgoing direction is outside the surface (for glass).
    outside: bool,
    p_diffuse: f32,
    p_spec: f32,
    /// Fresnel reflectance of the outgoing direction (dielectric).
    f_out: f32,
}

pub(crate) struct BsdfSample {
    pub wi: Vec3,
    /// `f * cos / pdf`.
    pub weight: Vec3,
    pub pdf: f32,
    pub delta: bool,
}

/// Unpolarised Fresnel reflectance for a dielectric; `eta` = n_t / n_i.
pub(crate) fn fresnel_dielectric(cos_i: f32, eta: f32) -> f32 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let sin2_t = (1.0 - cos_i * cos_i) / (eta * eta);
    if sin2_t >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    let rs = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    let rp = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    0.5 * (rs * rs + rp * rp)
}

fn schlick(f0: Vec3, cos: f32) -> Vec3 {
    let m = (1.0 - cos).clamp(0.0, 1.0);
    let m5 = m * m * m * m * m;
    f0 + (Vec3::ONE - f0) * m5
}

fn ggx_d(h: Vec3, a2: f32) -> f32 {
    let c2 = h.z * h.z;
    let d = c2 * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d)
}

fn ggx_lambda(w: Vec3, a2: f32) -> f32 {
    let c2 = w.z * w.z;
    if c2 <= 0.0 {
        return f32::INFINITY;
    }
    let t2 = (1.0 - c2).max(0.0) / c2;
    0.5 * ((1.0 + a2 * t2).sqrt() - 1.0)
}

fn ggx_g1(w: Vec3, a2: f32) -> f32 {
    1.0 / (1.0 + ggx_lambda(w, a2))
}

/// Visible-normal sample (Heitz 2018) for `wo` above the surface.
fn sample_vndf(wo: Vec3, alpha: f32, u1: f32, u2: f32) -> Vec3 {
    let vh = Vec3::new(alpha * wo.x, alpha * wo.y, wo.z).normalize();
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = if lensq > 0.0 { Vec3::new(-vh.y, vh.x, 0.0) / lensq.sqrt() } else { Vec3::X };
    let t2 = vh.cross(t1);
    let r = u1.sqrt();
    let phi = TAU * u2;
    let p1 = r * phi.cos();
    let s = 0.5 * (1.0 + vh.z);
    let p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * r * phi.sin();
    let nh = t1 * p1 + t2 * p2 + vh * (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
    Vec3::new(alpha * nh.x, alpha * nh.y, nh.z.max(1e-6)).normalize()
}

fn reflect(w: Vec3, h: Vec3) -> Vec3 {
    h * (2.0 * w.dot(h)) - w
}

/// Refract `w` (on the +`h` side) with relative index `eta` = n_t / n_i.
fn refract(w: Vec3, h: Vec3, eta: f32) -> Option<Vec3> {
    let c = w.dot(h);
    let sin2_t = (1.0 - c * c).max(0.0) / (eta * eta);
    if sin2_t >= 1.0 {
        return None;
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    Some((-w / eta + h * (c / eta - cos_t)).normalize())
}

impl Bsdf {
    /// `n` and `ng` must face `wo`; `outside` says whether `wo` is on the
    /// front of the surface (needed to refract the right way).
    #[allow(clippy::too_many_arguments)]
    pub fn new(base: Vec3, metallic: f32, roughness: f32, ior: f32, transmission: f32, n: Vec3, ng: Vec3, wo: Vec3, outside: bool) -> Bsdf {
        let (t, b) = onb(n);
        let dielectric = (1.0 - metallic) * (1.0 - transmission);
        let glass = (1.0 - metallic) * transmission;
        let cos_o = n.dot(wo).clamp(0.0, 1.0);
        let f_out = fresnel_dielectric(cos_o, ior);
        let e_diff = dielectric * (1.0 - f_out) * luminance(base);
        let e_spec = metallic * luminance(schlick(base, cos_o)).max(0.02) + dielectric * f_out;
        let total = e_diff + e_spec + glass;
        let (p_diffuse, p_spec) = if total > 0.0 { (e_diff / total, e_spec / total) } else { (0.0, 0.0) };
        Bsdf { t, b, n, ng, base, metallic, dielectric, glass, alpha: (roughness * roughness).max(1e-3), ior, outside, p_diffuse, p_spec, f_out }
    }

    fn to_local(&self, v: Vec3) -> Vec3 {
        Vec3::new(v.dot(self.t), v.dot(self.b), v.dot(self.n))
    }

    fn to_world(&self, v: Vec3) -> Vec3 {
        self.t * v.x + self.b * v.y + self.n * v.z
    }

    /// Whether [`Bsdf::eval`] can be non-zero (worth light sampling).
    pub fn has_smooth_lobes(&self) -> bool {
        self.p_diffuse + self.p_spec > 0.0
    }

    /// The non-delta part: (f, pdf) for light arriving from `wi`.
    pub fn eval(&self, wo: Vec3, wi: Vec3) -> (Vec3, f32) {
        if wi.dot(self.ng) <= 0.0 {
            return (Vec3::ZERO, 0.0);
        }
        let (o, i) = (self.to_local(wo), self.to_local(wi));
        if o.z <= 0.0 || i.z <= 0.0 {
            return (Vec3::ZERO, 0.0);
        }
        let mut f = Vec3::ZERO;
        let mut pdf = 0.0;
        if self.dielectric > 0.0 {
            f += self.base * (self.dielectric * (1.0 - self.f_out) * FRAC_1_PI);
            pdf += self.p_diffuse * i.z * FRAC_1_PI;
        }
        if self.p_spec > 0.0 {
            let h = (o + i).normalize();
            let a2 = self.alpha * self.alpha;
            let d = ggx_d(h, a2);
            let g = 1.0 / (1.0 + ggx_lambda(o, a2) + ggx_lambda(i, a2));
            let ih = i.dot(h).max(0.0);
            let fr = schlick(self.base, ih) * self.metallic + Vec3::splat(self.dielectric * fresnel_dielectric(ih, self.ior));
            f += fr * (d * g / (4.0 * o.z * i.z));
            pdf += self.p_spec * ggx_g1(o, a2) * d / (4.0 * o.z);
        }
        (f, pdf)
    }

    pub fn sample(&self, wo: Vec3, rng: &mut Rng) -> Option<BsdfSample> {
        let (lobe, u1, u2) = (rng.f32(), rng.f32(), rng.f32());
        let o = self.to_local(wo);
        if o.z <= 0.0 {
            return None;
        }
        if lobe >= self.p_diffuse + self.p_spec {
            return self.sample_glass(o, rng.f32(), u1, u2);
        }
        let i = if lobe < self.p_diffuse { cosine_hemisphere(u1, u2) } else { reflect(o, sample_vndf(o, self.alpha, u1, u2)) };
        let wi = self.to_world(i);
        let (f, pdf) = self.eval(wo, wi);
        if pdf <= 0.0 || f == Vec3::ZERO {
            return None;
        }
        Some(BsdfSample { wi, weight: f * (i.z / pdf), pdf, delta: false })
    }

    fn sample_glass(&self, o: Vec3, uf: f32, u1: f32, u2: f32) -> Option<BsdfSample> {
        let p_glass = 1.0 - self.p_diffuse - self.p_spec;
        if p_glass <= 0.0 || self.glass <= 0.0 {
            return None;
        }
        let smooth = self.alpha <= 2e-3;
        let h = if smooth { Vec3::Z } else { sample_vndf(o, self.alpha, u1, u2) };
        let eta = if self.outside { self.ior } else { 1.0 / self.ior };
        let f = fresnel_dielectric(o.dot(h), eta);
        let a2 = self.alpha * self.alpha;
        let scale = self.glass / p_glass;
        let refracted = if uf < f { None } else { refract(o, h, eta) };
        let reflected = refracted.is_none();
        let (i, tint) = match refracted {
            Some(t) => (t, self.base),
            None => (reflect(o, h), Vec3::ONE),
        };
        if (i.z > 0.0) != reflected {
            return None; // the sampled microfacet sent the ray to the wrong side
        }
        // With visible-normal sampling the remaining weight is G1(wi).
        let g = if smooth { 1.0 } else { ggx_g1(Vec3::new(i.x, i.y, i.z.abs()), a2) };
        let wi = self.to_world(i);
        // Reject leaks through the geometric surface.
        if (wi.dot(self.ng) > 0.0) != reflected {
            return None;
        }
        Some(BsdfSample { wi, weight: tint * (scale * g), pdf: 0.0, delta: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white_furnace(metallic: f32, roughness: f32, ior: f32) -> f32 {
        let n = Vec3::Z;
        let wo = Vec3::new(0.5, 0.0, 0.8).normalize();
        let bsdf = Bsdf::new(Vec3::ONE, metallic, roughness, ior, 0.0, n, n, wo, true);
        let mut rng = Rng::new(7, 3);
        let count = 20000;
        let mut sum = 0.0;
        for _ in 0..count {
            if let Some(s) = bsdf.sample(wo, &mut rng) {
                sum += s.weight.y;
            }
        }
        sum / count as f32
    }

    #[test]
    fn lambert_and_metal_conserve_energy() {
        // ior 1 has no specular: a perfect white Lambertian reflects everything.
        assert!((white_furnace(0.0, 0.5, 1.0) - 1.0).abs() < 0.01);
        // Single-scattering GGX loses a little energy at high roughness, never gains.
        let m = white_furnace(1.0, 0.3, 1.5);
        assert!(m > 0.9 && m < 1.01, "{m}");
        let d = white_furnace(0.0, 0.5, 1.5);
        assert!(d > 0.9 && d < 1.02, "{d}");
    }

    #[test]
    fn eval_matches_sample_pdf() {
        let n = Vec3::Z;
        let wo = Vec3::new(0.3, 0.2, 0.9).normalize();
        let bsdf = Bsdf::new(Vec3::new(0.8, 0.5, 0.2), 0.3, 0.4, 1.45, 0.0, n, n, wo, true);
        let mut rng = Rng::new(1, 1);
        for _ in 0..100 {
            if let Some(s) = bsdf.sample(wo, &mut rng) {
                let (f, pdf) = bsdf.eval(wo, s.wi);
                assert!((pdf - s.pdf).abs() < 1e-4 * pdf.max(1.0));
                assert!(f.approx_eq(s.weight * (s.pdf / s.wi.z), 1e-3));
            }
        }
    }

    #[test]
    fn glass_transmits_most_light() {
        let n = Vec3::Z;
        let wo = Vec3::Z;
        let bsdf = Bsdf::new(Vec3::ONE, 0.0, 0.0, 1.5, 1.0, n, n, wo, true);
        let mut rng = Rng::new(3, 9);
        let mut through = 0;
        for _ in 0..1000 {
            let s = bsdf.sample(wo, &mut rng).unwrap();
            assert!(s.delta && (s.weight.x - 1.0).abs() < 1e-5);
            if s.wi.z < 0.0 {
                through += 1;
            }
        }
        assert!((930..=990).contains(&through), "{through}");
    }
}
