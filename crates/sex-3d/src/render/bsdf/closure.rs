// SPDX-License-Identifier: GPL-3.0-only
//! BSDF closures produced by shader node trees, after Cycles' closure set
//! (`kernel/closure/bsdf_*.h`): Lambert/Oren-Nayar diffuse, translucent,
//! anisotropic GGX reflection with dielectric, F82-tint conductor or constant
//! Fresnel (plus energy compensation for multiple scattering), rough and
//! smooth dielectric glass/refraction, Charlie sheen, toon, and transparency.
//!
//! A [`ClosureBsdf`] is a weighted sum of lobes. It is sampled with one-sample
//! MIS over the lobes: a lobe is picked in proportion to its estimated albedo
//! times its weight, a direction is drawn from it, and the result is weighted
//! by the full sum of all smooth lobes' values over the sum of their pdfs.
//! `eval` returns `f * |cos|`, so lobes may each have their own normal.
use super::{fresnel_dielectric, reflect, BsdfSample, L_DIFFUSE, L_GLOSSY, L_REFLECT, L_SINGULAR, L_TRANSMIT, L_TRANSPARENT};
use crate::math::Vec3;
use crate::render::luminance;
use crate::render::rng::Rng;
use crate::render::sampling::{cosine_hemisphere, onb};
use core::f32::consts::{FRAC_1_PI, PI, TAU};
use std::sync::OnceLock;

/// Below this alpha a glass or refraction lobe is a perfect (delta) interface.
const SINGULAR_ALPHA: f32 = 2e-3;
/// Reflection lobes never go below this alpha (they stay non-delta, like the
/// legacy material).
const MIN_ALPHA: f32 = 1e-3;

/// Thin-film interference over a lobe's Fresnel term.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ThinFilm {
    /// Nanometres.
    pub thickness: f32,
    pub ior: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fresnel {
    /// No Fresnel: the lobe weight is the reflectance (Glossy BSDF).
    Constant,
    /// Dielectric reflection for relative IOR `eta`, tinted.
    Dielectric { eta: f32, tint: Vec3 },
    /// Conductor with Cycles' F82-tint model: Schlick from `f0`, less `b`.
    Conductor { f0: Vec3, b: Vec3 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LobeKind {
    /// Oren-Nayar with `a`, `b` (Lambert when `b` is 0).
    Diffuse { a: f32, b: f32 },
    Translucent,
    /// GGX reflection; `t` is the anisotropy tangent.
    Reflection { t: Vec3, ax: f32, ay: f32, fresnel: Fresnel, film: Option<ThinFilm>, energy: Vec3 },
    /// GGX dielectric interface: reflection and refraction (or refraction
    /// only). `eta` = IOR behind / IOR in front of the normal.
    Glass { alpha: f32, eta: f32, tint_r: Vec3, tint_t: Vec3, refract_only: bool },
    /// Charlie sheen with alpha `r`.
    Sheen { r: f32 },
    /// Toon: hard-edged cone around the normal or the mirror direction.
    Toon { max_angle: f32, smooth: f32, glossy: bool },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lobe {
    pub weight: Vec3,
    /// Unit normal facing the outgoing direction's side.
    pub n: Vec3,
    pub kind: LobeKind,
}

// ---------------------------------------------------------------------------
// Microfacet helpers (anisotropic GGX in a local frame, normal = +Z)

fn d_aniso(h: Vec3, ax: f32, ay: f32) -> f32 {
    if h.z <= 0.0 {
        return 0.0;
    }
    let t = (h.x / ax).powi(2) + (h.y / ay).powi(2) + h.z * h.z;
    1.0 / (PI * ax * ay * t * t)
}

fn lambda_aniso(w: Vec3, ax: f32, ay: f32) -> f32 {
    let z2 = w.z * w.z;
    if z2 <= 0.0 {
        return f32::INFINITY;
    }
    let t = ((ax * w.x).powi(2) + (ay * w.y).powi(2)) / z2;
    0.5 * ((1.0 + t).sqrt() - 1.0)
}

fn g1_aniso(w: Vec3, ax: f32, ay: f32) -> f32 {
    1.0 / (1.0 + lambda_aniso(w, ax, ay))
}

/// Visible-normal sample (Heitz 2018) for `wo` with `wo.z > 0`.
fn sample_vndf_aniso(wo: Vec3, ax: f32, ay: f32, u1: f32, u2: f32) -> Vec3 {
    let vh = Vec3::new(ax * wo.x, ay * wo.y, wo.z).normalize();
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = if lensq > 0.0 { Vec3::new(-vh.y, vh.x, 0.0) / lensq.sqrt() } else { Vec3::X };
    let t2 = vh.cross(t1);
    let r = u1.sqrt();
    let phi = TAU * u2;
    let p1 = r * phi.cos();
    let s = 0.5 * (1.0 + vh.z);
    let p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * r * phi.sin();
    let nh = t1 * p1 + t2 * p2 + vh * (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
    Vec3::new(ax * nh.x, ay * nh.y, nh.z.max(1e-6)).normalize()
}

/// Pdf of a visible normal `h` for `wo`.
fn vndf_pdf(wo: Vec3, h: Vec3, ax: f32, ay: f32) -> f32 {
    if wo.z <= 0.0 {
        return 0.0;
    }
    g1_aniso(wo, ax, ay) * wo.dot(h).max(0.0) * d_aniso(h, ax, ay) / wo.z
}

fn schlick_f82(cos: f32, f0: Vec3, b: Vec3) -> Vec3 {
    let m = (1.0 - cos).clamp(0.0, 1.0);
    let m5 = m * m * m * m * m;
    let fs = f0 + (Vec3::ONE - f0) * m5;
    let v = fs - b * (cos * m5 * m);
    Vec3::new(v.x.clamp(0.0, 1.0), v.y.clamp(0.0, 1.0), v.z.clamp(0.0, 1.0))
}

/// Cycles' F82-tint factor `b` for base colour `f0` and tint.
pub(crate) fn f82_tint_b(f0: Vec3, tint: Vec3) -> Vec3 {
    let f = 6.0f32 / 7.0;
    let f5 = f * f * f * f * f;
    let fs = f0 + (Vec3::ONE - f0) * f5;
    fs.mul_elem(Vec3::ONE - tint) * (7.0 / (f5 * f))
}

/// Hemispherical average of the dielectric Fresnel term (Kulla-Conty fit).
pub(crate) fn fresnel_avg(eta: f32) -> f32 {
    if eta >= 1.0 {
        (eta - 1.0) / (4.08567 + 1.00071 * eta)
    } else {
        let e = 1.0 / eta;
        1.0 - (1.0 - (e - 1.0) / (4.08567 + 1.00071 * e)) / (e * e)
    }
}

/// Unpolarised Airy reflectance of a thin film (`n2`, `d` nm) over a
/// dielectric of index `n3`, from air, at wavelength `lambda` nm.
fn airy(cos_i: f32, n2: f32, n3: f32, d: f32, lambda: f32) -> f32 {
    let sin2 = (1.0 - cos_i * cos_i).max(0.0);
    let st2 = sin2 / (n2 * n2);
    let ss2 = sin2 / (n3 * n3);
    if st2 >= 1.0 || ss2 >= 1.0 {
        return 1.0;
    }
    let (ct, cs) = ((1.0 - st2).sqrt(), (1.0 - ss2).sqrt());
    let r12s = (cos_i - n2 * ct) / (cos_i + n2 * ct);
    let r12p = (n2 * cos_i - ct) / (n2 * cos_i + ct);
    let r23s = (n2 * ct - n3 * cs) / (n2 * ct + n3 * cs);
    let r23p = (n3 * ct - n2 * cs) / (n3 * ct + n2 * cs);
    let delta = 4.0 * PI * n2 * d * ct / lambda;
    let c = delta.cos();
    let r = |a: f32, b: f32| (a * a + b * b + 2.0 * a * b * c) / (1.0 + a * a * b * b + 2.0 * a * b * c);
    0.5 * (r(r12s, r23s) + r(r12p, r23p))
}

/// Thin-film reflectance in RGB (sampled at 650, 510 and 475 nm).
pub(crate) fn thin_film_rgb(cos_i: f32, film: &ThinFilm, substrate: Vec3) -> Vec3 {
    let l = [650.0, 510.0, 475.0];
    Vec3::new(airy(cos_i, film.ior, substrate.x, film.thickness, l[0]), airy(cos_i, film.ior, substrate.y, film.thickness, l[1]), airy(cos_i, film.ior, substrate.z, film.thickness, l[2]))
}

/// IOR of a dielectric with normal reflectance `f0`.
fn ior_from_f0(f0: f32) -> f32 {
    let s = f0.clamp(0.0, 0.99).sqrt();
    (1.0 + s) / (1.0 - s)
}

// ---------------------------------------------------------------------------
// Albedo tables (computed once, deterministically)

const TABLE_N: usize = 32;

struct Tables {
    /// Directional albedo of white GGX reflection, [alpha][mu].
    ggx: Vec<f32>,
    /// Directional albedo of the Charlie sheen lobe, [r][mu].
    sheen: Vec<f32>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let n = TABLE_N;
        let mut ggx = vec![0.0; n * n];
        let mut sheen = vec![0.0; n * n];
        let strata = 16;
        for ai in 0..n {
            let a = ((ai as f32 + 0.5) / n as f32).max(MIN_ALPHA);
            for mi in 0..n {
                let mu = (mi as f32 + 0.5) / n as f32;
                let wo = Vec3::new((1.0 - mu * mu).sqrt(), 0.0, mu);
                let (mut e, mut s) = (0.0f64, 0.0f64);
                for sy in 0..strata {
                    for sx in 0..strata {
                        let (u1, u2) = ((sx as f32 + 0.5) / strata as f32, (sy as f32 + 0.5) / strata as f32);
                        let h = sample_vndf_aniso(wo, a, a, u1, u2);
                        let wi = reflect(wo, h);
                        if wi.z > 0.0 {
                            let lo = lambda_aniso(wo, a, a);
                            e += ((1.0 + lo) / (1.0 + lo + lambda_aniso(wi, a, a))) as f64;
                        }
                        let wi = cosine_hemisphere(u1, u2);
                        let (f, _) = sheen_eval(wo, wi, a);
                        s += (f / (wi.z * FRAC_1_PI).max(1e-8)) as f64;
                    }
                }
                let k = (strata * strata) as f64;
                ggx[ai * n + mi] = (e / k) as f32;
                sheen[ai * n + mi] = (s / k).min(1.0) as f32;
            }
        }
        Tables { ggx, sheen }
    })
}

fn lookup(t: &[f32], a: f32, mu: f32) -> f32 {
    let n = TABLE_N as f32;
    let x = (mu * n - 0.5).clamp(0.0, n - 1.0);
    let y = (a * n - 0.5).clamp(0.0, n - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(TABLE_N - 1), (y0 + 1).min(TABLE_N - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let v = |yy: usize, xx: usize| t[yy * TABLE_N + xx];
    (v(y0, x0) * (1.0 - fx) + v(y0, x1) * fx) * (1.0 - fy) + (v(y1, x0) * (1.0 - fx) + v(y1, x1) * fx) * fy
}

/// Directional albedo of white single-scattering GGX.
pub(crate) fn ggx_albedo(alpha: f32, mu: f32) -> f32 {
    lookup(&tables().ggx, alpha, mu).max(1e-3)
}

/// Energy compensation for multiple scattering (Turquin 2019): the factor
/// that brings a GGX lobe with average Fresnel `f_avg` up to its
/// multiple-scattering energy.
pub(crate) fn ms_compensation(alpha: f32, mu: f32, f_avg: Vec3) -> Vec3 {
    let e = ggx_albedo(alpha, mu);
    Vec3::ONE + f_avg * ((1.0 - e) / e)
}

fn sheen_albedo(r: f32, mu: f32) -> f32 {
    lookup(&tables().sheen, r, mu)
}

// ---------------------------------------------------------------------------
// Lobes

fn sheen_eval(o: Vec3, i: Vec3, r: f32) -> (f32, f32) {
    if o.z <= 0.0 || i.z <= 0.0 {
        return (0.0, 0.0);
    }
    let h = (o + i).normalize();
    let inv_r = 1.0 / r.max(1e-3);
    let sin2 = (1.0 - h.z * h.z).max(0.0);
    let d = (2.0 + inv_r) * sin2.powf(0.5 * inv_r) / TAU;
    let v = 1.0 / (4.0 * (i.z + o.z - i.z * o.z));
    (d * v * i.z, i.z * FRAC_1_PI)
}

fn toon_intensity(max_angle: f32, smooth: f32, angle: f32) -> f32 {
    if angle < max_angle {
        1.0
    } else if angle < max_angle + smooth && smooth != 0.0 {
        1.0 - (angle - max_angle) / smooth
    } else {
        0.0
    }
}

impl Lobe {
    /// Tangent frame (t, b, n).
    fn frame(&self) -> (Vec3, Vec3, Vec3) {
        let n = self.n;
        if let LobeKind::Reflection { t, .. } = self.kind {
            let t = t - n * n.dot(t);
            if t.length_squared() > 1e-12 {
                let t = t.normalize();
                return (t, n.cross(t), n);
            }
        }
        let (t, b) = onb(n);
        (t, b, n)
    }

    pub fn is_delta(&self) -> bool {
        matches!(self.kind, LobeKind::Glass { alpha, .. } if alpha < SINGULAR_ALPHA)
    }

    /// Counts as diffuse for the light passes (Cycles' diffuse label).
    pub fn is_diffuse(&self) -> bool {
        matches!(self.kind, LobeKind::Diffuse { .. } | LobeKind::Translucent | LobeKind::Sheen { .. } | LobeKind::Toon { glossy: false, .. })
    }

    /// Rough albedo estimate seen from `wo` (for picking lobes and layering).
    pub fn albedo(&self, wo: Vec3) -> Vec3 {
        let mu = self.n.dot(wo).clamp(1e-4, 1.0);
        match self.kind {
            LobeKind::Diffuse { .. } | LobeKind::Translucent | LobeKind::Toon { .. } => Vec3::ONE,
            LobeKind::Reflection { fresnel, ax, ay, energy, .. } => {
                let a = (ax * ay).sqrt();
                let e = ggx_albedo(a, mu);
                let f = match fresnel {
                    Fresnel::Constant => Vec3::ONE,
                    Fresnel::Dielectric { eta, tint } => tint * dielectric_albedo(mu, a, eta),
                    Fresnel::Conductor { f0, b } => schlick_f82(mu, f0, b),
                };
                (f * e).mul_elem(energy)
            }
            LobeKind::Glass { eta, tint_r, tint_t, refract_only, .. } => {
                let f = fresnel_dielectric(mu, eta);
                if refract_only {
                    tint_t * (1.0 - f)
                } else {
                    tint_r * f + tint_t * (1.0 - f)
                }
            }
            LobeKind::Sheen { r } => Vec3::splat(sheen_albedo(r, mu)),
        }
    }

    /// `f * |cos|` and the pdf of sampling `wi` (zero for delta lobes).
    fn eval(&self, wo: Vec3, wi: Vec3) -> (Vec3, f32) {
        let (t, b, n) = self.frame();
        let o = Vec3::new(wo.dot(t), wo.dot(b), wo.dot(n));
        let i = Vec3::new(wi.dot(t), wi.dot(b), wi.dot(n));
        if o.z <= 0.0 {
            return (Vec3::ZERO, 0.0);
        }
        match self.kind {
            LobeKind::Diffuse { a, b } => {
                if i.z <= 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                let mut s = wi.dot(wo) - i.z * o.z;
                if s > 0.0 {
                    s /= i.z.max(o.z) + f32::MIN_POSITIVE;
                }
                (Vec3::splat(i.z * (a + b * s)), i.z * FRAC_1_PI)
            }
            LobeKind::Translucent => {
                if i.z >= 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                (Vec3::splat(-i.z * FRAC_1_PI), -i.z * FRAC_1_PI)
            }
            LobeKind::Reflection { ax, ay, fresnel, film, energy, .. } => {
                if i.z <= 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                let h = (o + i).normalize();
                let d = d_aniso(h, ax, ay);
                let g = 1.0 / (1.0 + lambda_aniso(o, ax, ay) + lambda_aniso(i, ax, ay));
                let f = reflection_fresnel(fresnel, film, i.dot(h).max(0.0));
                let fc = f.mul_elem(energy) * (d * g / (4.0 * o.z));
                (fc, vndf_pdf(o, h, ax, ay) / (4.0 * o.dot(h).max(1e-8)))
            }
            LobeKind::Glass { alpha, eta, tint_r, tint_t, refract_only } => {
                if alpha < SINGULAR_ALPHA || i.z == 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                let reflect = i.z > 0.0;
                if reflect && refract_only {
                    return (Vec3::ZERO, 0.0);
                }
                let etap = if reflect { 1.0 } else { eta };
                let mut h = i * etap + o;
                if h.length_squared() == 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                h = h.normalize();
                if h.z < 0.0 {
                    h = -h;
                }
                if h.dot(i) * i.z < 0.0 || h.dot(o) * o.z < 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                let fr = fresnel_dielectric(o.dot(h), eta);
                let d = d_aniso(h, alpha, alpha);
                let g = 1.0 / (1.0 + lambda_aniso(o, alpha, alpha) + lambda_aniso(Vec3::new(i.x, i.y, i.z.abs()), alpha, alpha));
                let (pr, pt) = if refract_only { (0.0, 1.0) } else { (fr, 1.0 - fr) };
                let vpdf = vndf_pdf(o, h, alpha, alpha);
                if reflect {
                    let f = tint_r * (d * g * fr / (4.0 * o.z));
                    (f, vpdf / (4.0 * o.dot(h)) * pr)
                } else {
                    let denom = (i.dot(h) + o.dot(h) / etap).powi(2);
                    let ft = d * (1.0 - fr) * g * (i.dot(h) * o.dot(h)).abs() / (denom * o.z);
                    // Importance-transport form (no 1/eta^2), like the legacy glass.
                    let t = if refract_only { ft / (1.0 - fr).max(1e-6) } else { ft };
                    (tint_t * t, vpdf * i.dot(h).abs() / denom * pt)
                }
            }
            LobeKind::Sheen { r } => {
                let (f, pdf) = sheen_eval(o, i, r);
                (Vec3::splat(f), pdf)
            }
            LobeKind::Toon { max_angle, smooth, glossy } => {
                if i.z <= 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                let axis = if glossy { reflect(o, Vec3::Z) } else { Vec3::Z };
                let angle = axis.dot(i).clamp(-1.0, 1.0).acos();
                let is = toon_intensity(max_angle, smooth, angle);
                if is <= 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                let sa = (max_angle + smooth).min(core::f32::consts::FRAC_PI_2);
                let pdf = 0.5 * FRAC_1_PI / (1.0 - sa.cos()).max(1e-6);
                (Vec3::splat(pdf * is), pdf)
            }
        }
    }

    /// Draw `wi`; returns (wi, value `f * |cos|`, pdf, delta, label). For a
    /// delta lobe the value is the sample weight and the pdf is 0.
    fn sample(&self, wo: Vec3, u: [f32; 3]) -> Option<(Vec3, Vec3, f32, bool, u8)> {
        let (t, b, n) = self.frame();
        let o = Vec3::new(wo.dot(t), wo.dot(b), wo.dot(n));
        if o.z <= 0.0 {
            return None;
        }
        let world = |v: Vec3| t * v.x + b * v.y + n * v.z;
        let (i, label) = match self.kind {
            LobeKind::Diffuse { .. } | LobeKind::Sheen { .. } => (cosine_hemisphere(u[0], u[1]), if matches!(self.kind, LobeKind::Sheen { .. }) { L_GLOSSY } else { L_DIFFUSE } | L_REFLECT),
            LobeKind::Translucent => {
                let c = cosine_hemisphere(u[0], u[1]);
                (Vec3::new(c.x, c.y, -c.z), L_DIFFUSE | L_TRANSMIT)
            }
            LobeKind::Reflection { ax, ay, .. } => (reflect(o, sample_vndf_aniso(o, ax, ay, u[0], u[1])), L_GLOSSY | L_REFLECT),
            LobeKind::Toon { max_angle, smooth, glossy } => {
                let axis = if glossy { reflect(o, Vec3::Z) } else { Vec3::Z };
                let sa = (max_angle + smooth).min(core::f32::consts::FRAC_PI_2);
                let cos_max = sa.cos();
                let z = 1.0 - u[0] * (1.0 - cos_max);
                let r = (1.0 - z * z).max(0.0).sqrt();
                let phi = TAU * u[1];
                let (at, ab) = onb(axis);
                (at * (r * phi.cos()) + ab * (r * phi.sin()) + axis * z, if glossy { L_GLOSSY } else { L_DIFFUSE } | L_REFLECT)
            }
            LobeKind::Glass { alpha, eta, tint_r, tint_t, refract_only } => {
                let smooth = alpha < SINGULAR_ALPHA;
                let h = if smooth { Vec3::Z } else { sample_vndf_aniso(o, alpha, alpha, u[0], u[1]) };
                let fr = fresnel_dielectric(o.dot(h), eta);
                let pick_reflect = !refract_only && u[2] < fr;
                let (i, reflected) = if pick_reflect {
                    (reflect(o, h), true)
                } else {
                    match refract_dir(o, h, eta) {
                        Some(t) => (t, false),
                        None if !refract_only => (reflect(o, h), true),
                        None => return None,
                    }
                };
                if (i.z > 0.0) != reflected {
                    return None;
                }
                let lab = if reflected { L_REFLECT } else { L_TRANSMIT };
                if smooth {
                    let w = if reflected { tint_r } else { tint_t };
                    return Some((world(i), w, 0.0, true, lab | L_SINGULAR));
                }
                (i, lab | L_GLOSSY)
            }
        };
        let wi = world(i);
        let (f, pdf) = self.eval(wo, wi);
        if pdf <= 0.0 {
            return None;
        }
        Some((wi, f, pdf, false, label))
    }
}

fn refract_dir(o: Vec3, h: Vec3, eta: f32) -> Option<Vec3> {
    let c = o.dot(h);
    let sin2_t = (1.0 - c * c).max(0.0) / (eta * eta);
    if sin2_t >= 1.0 {
        return None;
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    Some((-o / eta + h * (c / eta - cos_t)).normalize())
}

fn reflection_fresnel(fresnel: Fresnel, film: Option<ThinFilm>, cos: f32) -> Vec3 {
    match (fresnel, film) {
        (Fresnel::Constant, _) => Vec3::ONE,
        (Fresnel::Dielectric { eta, tint }, None) => tint * fresnel_dielectric(cos, eta),
        (Fresnel::Dielectric { eta, tint }, Some(f)) => tint.mul_elem(thin_film_rgb(cos, &f, Vec3::splat(eta))),
        (Fresnel::Conductor { f0, b }, None) => schlick_f82(cos, f0, b),
        (Fresnel::Conductor { f0, b }, Some(f)) => {
            // Treat the metal as a dielectric of matching normal reflectance
            // under the film, blended by how much the film changes things.
            let sub = Vec3::new(ior_from_f0(f0.x), ior_from_f0(f0.y), ior_from_f0(f0.z));
            let base = schlick_f82(cos, f0, b);
            let filmed = thin_film_rgb(cos, &f, sub);
            let plain = Vec3::new(fresnel_dielectric(cos, sub.x), fresnel_dielectric(cos, sub.y), fresnel_dielectric(cos, sub.z));
            let r = |a: f32, p: f32, q: f32| if p > 1e-4 { (a * q / p).clamp(0.0, 1.0) } else { q };
            Vec3::new(r(base.x, plain.x, filmed.x), r(base.y, plain.y, filmed.y), r(base.z, plain.z, filmed.z))
        }
    }
}

/// Approximate directional albedo of a dielectric GGX reflection layer.
pub(crate) fn dielectric_albedo(mu: f32, alpha: f32, eta: f32) -> f32 {
    let t = alpha.sqrt().min(1.0);
    fresnel_dielectric(mu, eta) * (1.0 - t) + fresnel_avg(eta) * t
}

/// A sum of lobes plus transparency: the surface part of a closure list.
#[derive(Clone, Debug, Default)]
pub(crate) struct ClosureBsdf {
    pub lobes: Vec<Lobe>,
    /// Selection weight of each lobe (same order); then transparency.
    sel: Vec<f32>,
    pub transparent: Vec3,
    sel_transparent: f32,
    sel_total: f32,
    smooth_total: f32,
    /// Geometric normal facing the outgoing direction.
    ng: Vec3,
}

impl ClosureBsdf {
    pub fn new(lobes: Vec<Lobe>, transparent: Vec3, ng: Vec3, wo: Vec3) -> ClosureBsdf {
        let sel: Vec<f32> = lobes.iter().map(|l| luminance(l.weight.mul_elem(l.albedo(wo))).max(0.0) + 1e-6 * l.weight.max_elem().max(0.0)).collect();
        let sel_transparent = luminance(transparent).max(0.0);
        let smooth_total = lobes.iter().zip(&sel).filter(|(l, _)| !l.is_delta()).map(|(_, s)| *s).sum();
        let sel_total = sel.iter().sum::<f32>() + sel_transparent;
        ClosureBsdf { lobes, sel, transparent, sel_transparent, sel_total, smooth_total, ng }
    }

    pub fn has_smooth_lobes(&self) -> bool {
        self.smooth_total > 0.0
    }

    /// Light arriving from `wi`: (`f * |cos|`, pdf) over the smooth lobes.
    pub fn eval(&self, wo: Vec3, wi: Vec3) -> (Vec3, f32) {
        let (f, _, pdf) = self.eval_split(wo, wi);
        (f, pdf)
    }

    /// `eval` with the diffuse lobes' share of `f` separately.
    pub fn eval_split(&self, wo: Vec3, wi: Vec3) -> (Vec3, Vec3, f32) {
        if self.sel_total <= 0.0 {
            return (Vec3::ZERO, Vec3::ZERO, 0.0);
        }
        let above = wi.dot(self.ng) > 0.0;
        let mut f = Vec3::ZERO;
        let mut fd = Vec3::ZERO;
        let mut pdf = 0.0;
        for (l, s) in self.lobes.iter().zip(&self.sel) {
            // Reflection must stay above, transmission below the true surface.
            if l.is_delta() || (l.n.dot(wi) > 0.0) != above {
                continue;
            }
            let (lf, lp) = l.eval(wo, wi);
            let c = l.weight.mul_elem(lf);
            f += c;
            if l.is_diffuse() {
                fd += c;
            }
            pdf += s / self.sel_total * lp;
        }
        (f, fd, pdf)
    }

    pub fn sample(&self, wo: Vec3, rng: &mut Rng) -> Option<BsdfSample> {
        let (pick, u1, u2, u3) = (rng.f32(), rng.f32(), rng.f32(), rng.f32());
        if self.sel_total <= 0.0 {
            return None;
        }
        let mut x = pick * self.sel_total;
        let mut chosen = None;
        for (k, s) in self.sel.iter().enumerate() {
            if x < *s {
                chosen = Some(k);
                break;
            }
            x -= s;
        }
        let Some(k) = chosen else {
            // Transparency: straight through, not a bounce.
            if self.sel_transparent <= 0.0 {
                return None;
            }
            let p = self.sel_transparent / self.sel_total;
            return Some(BsdfSample { wi: -wo, weight: self.transparent / p, pdf: 0.0, delta: true, label: L_TRANSPARENT });
        };
        let l = &self.lobes[k];
        let (wi, f, pdf, delta, label) = l.sample(wo, [u1, u2, u3])?;
        let reflected = label & L_REFLECT != 0;
        if (wi.dot(self.ng) > 0.0) != reflected {
            return None; // leaks through the true surface
        }
        let p = self.sel[k] / self.sel_total;
        if delta {
            return Some(BsdfSample { wi, weight: l.weight.mul_elem(f) / p, pdf: 0.0, delta: true, label });
        }
        let _ = pdf;
        let (ft, pt) = self.eval(wo, wi);
        if pt <= 0.0 || ft == Vec3::ZERO {
            return None;
        }
        Some(BsdfSample { wi, weight: ft / pt, pdf: pt, delta: false, label })
    }

    /// Sum of lobe weights times albedo (the denoising albedo).
    pub fn albedo(&self, wo: Vec3) -> Vec3 {
        self.lobes.iter().fold(self.transparent, |a, l| a + l.weight.mul_elem(l.albedo(wo)))
    }
}

// ---------------------------------------------------------------------------
// Lobe constructors used by the shader VM

/// Oren-Nayar parameters for roughness `sigma` (Lambert at 0).
pub(crate) fn diffuse(weight: Vec3, n: Vec3, roughness: f32) -> Lobe {
    let sigma = roughness.clamp(0.0, 1.0);
    let div = 1.0 / (PI + ((3.0 * PI - 4.0) / 6.0) * sigma);
    Lobe { weight, n, kind: LobeKind::Diffuse { a: div, b: sigma * div } }
}

/// Cycles' anisotropic alphas from roughness and anisotropy in [-1, 1].
pub(crate) fn aniso_alphas(roughness: f32, anisotropy: f32) -> (f32, f32) {
    let r = roughness.clamp(0.0, 1.0);
    let a = (r * r).max(MIN_ALPHA);
    let an = anisotropy.clamp(-0.99, 0.99);
    let (ax, ay) = if an < 0.0 { (a / (1.0 + an), a * (1.0 + an)) } else { (a * (1.0 - an), a / (1.0 - an)) };
    (ax.max(MIN_ALPHA), ay.max(MIN_ALPHA))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn furnace(b: &ClosureBsdf, wo: Vec3, n: usize) -> Vec3 {
        let mut rng = Rng::new(11, 5);
        let mut sum = Vec3::ZERO;
        for _ in 0..n {
            if let Some(s) = b.sample(wo, &mut rng) {
                sum += s.weight;
            }
        }
        sum / n as f32
    }

    #[test]
    fn lobes_sample_consistently_with_eval() {
        let n = Vec3::Z;
        let wo = Vec3::new(0.4, 0.1, 0.9).normalize();
        let lobes = vec![
            diffuse(Vec3::splat(0.3), n, 0.5),
            Lobe { weight: Vec3::splat(0.5), n, kind: LobeKind::Reflection { t: Vec3::X, ax: 0.2, ay: 0.4, fresnel: Fresnel::Constant, film: None, energy: Vec3::ONE } },
            Lobe { weight: Vec3::splat(0.2), n, kind: LobeKind::Sheen { r: 0.25 } },
        ];
        let b = ClosureBsdf::new(lobes, Vec3::ZERO, n, wo);
        let mut rng = Rng::new(2, 3);
        for _ in 0..200 {
            if let Some(s) = b.sample(wo, &mut rng) {
                let (f, pdf) = b.eval(wo, s.wi);
                assert!((pdf - s.pdf).abs() < 1e-3 * pdf.max(1.0), "{pdf} {}", s.pdf);
                assert!((f / pdf).approx_eq(s.weight, 1e-3 * s.weight.max_elem().max(1.0)));
            }
        }
    }

    #[test]
    fn white_lobes_conserve_energy() {
        let n = Vec3::Z;
        let wo = Vec3::new(0.3, 0.0, 0.95).normalize();
        let lam = ClosureBsdf::new(vec![diffuse(Vec3::ONE, n, 0.0)], Vec3::ZERO, n, wo);
        assert!((furnace(&lam, wo, 20000).y - 1.0).abs() < 0.01);
        // Compensated rough metal stays close to 1 (white F0).
        let (ax, ay) = aniso_alphas(0.8, 0.0);
        let mu = n.dot(wo);
        let energy = ms_compensation((ax * ay).sqrt(), mu, Vec3::ONE);
        let metal = Lobe { weight: Vec3::ONE, n, kind: LobeKind::Reflection { t: Vec3::X, ax, ay, fresnel: Fresnel::Conductor { f0: Vec3::ONE, b: Vec3::ZERO }, film: None, energy } };
        let m = furnace(&ClosureBsdf::new(vec![metal], Vec3::ZERO, n, wo), wo, 40000).y;
        assert!((m - 1.0).abs() < 0.04, "{m}");
        // Rough glass reflects plus transmits about everything.
        let glass = Lobe { weight: Vec3::ONE, n, kind: LobeKind::Glass { alpha: 0.3, eta: 1.5, tint_r: Vec3::ONE, tint_t: Vec3::ONE, refract_only: false } };
        let g = furnace(&ClosureBsdf::new(vec![glass], Vec3::ZERO, n, wo), wo, 40000).y;
        assert!(g > 0.85 && g < 1.05, "{g}");
        // Transparency passes straight through with full weight.
        let tb = ClosureBsdf::new(Vec::new(), Vec3::splat(0.5), n, wo);
        let s = tb.sample(wo, &mut Rng::new(1, 1)).unwrap();
        assert!(s.wi.approx_eq(-wo, 1e-6) && s.label == L_TRANSPARENT && (s.weight.x - 0.5).abs() < 1e-6);
    }

    #[test]
    fn f82_tint_reduces_to_schlick() {
        let f0 = Vec3::new(0.9, 0.6, 0.3);
        let b = f82_tint_b(f0, Vec3::ONE);
        assert_eq!(b, Vec3::ZERO);
        // With a tint the value at cos = 1/7 is Schlick times the tint.
        let tint = Vec3::new(0.5, 0.8, 1.0);
        let b = f82_tint_b(f0, tint);
        let c = 1.0 / 7.0;
        let s = schlick_f82(c, f0, Vec3::ZERO);
        assert!(schlick_f82(c, f0, b).approx_eq(s.mul_elem(tint), 1e-5));
    }

    #[test]
    fn thin_film_without_film_is_fresnel() {
        let f = ThinFilm { thickness: 0.0, ior: 1.33 };
        for c in [1.0, 0.7, 0.3] {
            let r = thin_film_rgb(c, &f, Vec3::splat(1.5));
            assert!((r.x - fresnel_dielectric(c, 1.5)).abs() < 1e-4, "{c} {r:?}");
        }
        let film = ThinFilm { thickness: 300.0, ior: 1.33 };
        let r = thin_film_rgb(1.0, &film, Vec3::splat(1.5));
        assert!(r.x != r.z, "film colours the reflection");
    }
}
