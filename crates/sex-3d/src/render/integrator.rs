// SPDX-License-Identifier: GPL-3.0-only
//! The path integrator: unidirectional path tracing with next-event
//! estimation, MIS (power heuristic), Russian roulette and indirect clamping.
use super::bsdf::Bsdf;
use super::prepare::{LightShape, PreparedScene, SceneHit};
use super::rng::Rng;
use super::sampling::power_heuristic;
use super::{luminance, PixelAccum, RenderSettings};
use crate::math::Vec3;
use core::ops::Range;

/// What one camera path returns.
pub(crate) struct PathResult {
    pub radiance: Vec3,
    pub albedo: Vec3,
    pub normal: Vec3,
}

/// Add samples `samples` of pixel (`x`, `y`) to `acc`. Each sample uses its
/// own RNG, so the result depends only on the pixel and the sample indices.
pub(crate) fn accumulate_pixel(scene: &PreparedScene, s: &RenderSettings, x: u32, y: u32, samples: Range<u32>, acc: &mut PixelAccum) {
    for si in samples {
        let mut rng = Rng::for_pixel(x, y, si, s.seed);
        let (fx, fy) = (x as f32 + rng.f32(), y as f32 + rng.f32());
        let (o, d) = scene.camera.ray(fx, fy, s.width, s.height);
        let r = trace(scene, o, d, &mut rng, s);
        let c = if finite(r.radiance) { r.radiance } else { Vec3::ZERO };
        acc.color += c;
        acc.lum_sq += luminance(c) * luminance(c);
        acc.albedo += r.albedo;
        acc.normal += r.normal;
    }
}

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn clamp_contribution(c: Vec3, limit: f32) -> Vec3 {
    let m = c.max_elem();
    if limit > 0.0 && m > limit { c * (limit / m) } else { c }
}

/// Offset a ray origin off the surface, to the side of `dir`.
fn offset(p: Vec3, ng: Vec3, dir: Vec3) -> Vec3 {
    let eps = 1e-4 * (1.0 + p.abs().max_elem());
    if dir.dot(ng) >= 0.0 { p + ng * eps } else { p - ng * eps }
}

pub(crate) fn trace(scene: &PreparedScene, mut o: Vec3, mut d: Vec3, rng: &mut Rng, s: &RenderSettings) -> PathResult {
    let mut l = Vec3::ZERO;
    let mut beta = Vec3::ONE;
    let mut albedo = None;
    let mut normal = Vec3::ZERO;
    // The previous bounce: BSDF pdf (for MIS) and whether it was singular.
    let mut prev_pdf = 0.0f32;
    let mut prev_delta = true;
    let mut depth = 0u32;
    loop {
        // Emission reached through `depth` bounces counts as indirect from 2.
        let add = |l: &mut Vec3, c: Vec3, indirect: bool| *l += if indirect { clamp_contribution(c, s.clamp) } else { c };
        let h = match scene.intersect(o, d) {
            None => {
                let le = scene.env.eval(d);
                if le != Vec3::ZERO {
                    let w = if prev_delta { 1.0 } else { power_heuristic(prev_pdf, scene.world_pdf(d)) };
                    add(&mut l, beta.mul_elem(le) * w, depth >= 2);
                }
                albedo.get_or_insert(Vec3::ONE);
                break;
            }
            Some(SceneHit::Area(li, t)) => {
                if let LightShape::Area { radiance, normal: n, .. } = &scene.lights[li as usize] {
                    let w = if prev_delta { 1.0 } else { power_heuristic(prev_pdf, scene.surface_light_pdf(li, d, t, *n)) };
                    add(&mut l, beta.mul_elem(*radiance) * w, depth >= 2);
                }
                albedo.get_or_insert(Vec3::ONE);
                break;
            }
            Some(SceneHit::Tri(h)) => h,
        };

        let g = &scene.bvh.tris[h.prim as usize];
        let sh = &scene.shade[h.prim as usize];
        let mat = &scene.materials[sh.material as usize];
        let p = o + d * h.t;
        let wo = -d;
        let ng = g.e1.cross(g.e2).normalize();
        let b0 = 1.0 - h.u - h.v;

        if mat.emission != Vec3::ZERO {
            let w = if prev_delta || sh.light == u32::MAX { 1.0 } else { power_heuristic(prev_pdf, scene.surface_light_pdf(sh.light, d, h.t, ng)) };
            add(&mut l, beta.mul_elem(mat.emission) * w, depth >= 2);
        }

        let outside = ng.dot(wo) > 0.0;
        let ng = if outside { ng } else { -ng };
        let mut ns = (sh.n[0] * b0 + sh.n[1] * h.u + sh.n[2] * h.v).normalize();
        if !outside {
            ns = -ns;
        }
        if ns.dot(wo) <= 1e-4 {
            ns = ng;
        }
        let mut base = mat.base;
        if let Some(ti) = mat.texture {
            let uv = sh.uv[0] * b0 + sh.uv[1] * h.u + sh.uv[2] * h.v;
            let c = scene.images[ti].sample(uv.x, uv.y);
            base = base.mul_elem(Vec3::new(c[0], c[1], c[2]));
        }
        if albedo.is_none() {
            albedo = Some(base);
            normal = ns;
        }
        // The last vertex lights by NEE alone (no BSDF ray follows to share with).
        let last = depth >= s.max_bounces;
        let bsdf = Bsdf::new(base, mat.metallic, mat.roughness, mat.ior, mat.transmission, ns, ng, wo, outside);

        // Next-event estimation.
        if bsdf.has_smooth_lobes() && scene.has_lights() {
            if let Some(ls) = scene.sample_light(p, rng) {
                if ls.pdf > 0.0 && ls.li != Vec3::ZERO {
                    let (f, bpdf) = bsdf.eval(wo, ls.wi);
                    if f != Vec3::ZERO {
                        let origin = offset(p, ng, ls.wi);
                        let tmax = if ls.dist.is_finite() { ls.dist * (1.0 - 1e-3) } else { f32::INFINITY };
                        if !scene.occluded(origin, ls.wi, tmax) {
                            let w = if ls.delta || last { 1.0 } else { power_heuristic(ls.pdf, bpdf) };
                            let c = beta.mul_elem(f).mul_elem(ls.li) * (ns.dot(ls.wi).abs() * w / ls.pdf);
                            add(&mut l, c, depth >= 1);
                        }
                    }
                }
            }
        }

        if last {
            break;
        }
        // Continue the path.
        let Some(bs) = bsdf.sample(wo, rng) else { break };
        beta = beta.mul_elem(bs.weight);
        if !finite(beta) || beta.max_elem() <= 0.0 {
            break;
        }
        prev_pdf = bs.pdf;
        prev_delta = bs.delta;
        depth += 1;
        if depth >= 3 {
            let q = (1.0 - beta.max_elem()).max(0.05);
            if rng.f32() < q {
                break;
            }
            beta *= 1.0 / (1.0 - q);
        }
        o = offset(p, ng, bs.wi);
        d = bs.wi;
    }
    PathResult { radiance: l, albedo: albedo.unwrap_or(Vec3::ONE), normal }
}
