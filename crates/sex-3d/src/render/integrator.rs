// SPDX-License-Identifier: GPL-3.0-only
//! The path integrator: unidirectional path tracing with next-event
//! estimation, MIS (power heuristic), Russian roulette, Cycles' light path
//! controls (per-type bounce limits, direct/indirect clamping, caustics
//! toggles, filter glossy), homogeneous volumes, ray visibility, holdout,
//! transparent film, motion blur time and the View Layer passes.
//!
//! Bounce limits follow Cycles: when a sampled bounce goes past a limit the
//! ray is still traced and collects the light it hits (keeping MIS
//! balanced), then the path ends.
use super::bsdf::{fresnel_dielectric, Bsdf, L_TRANSPARENT};
use super::shader::surface::{self as shading, SurfaceBsdf};
use super::shader::RayState;
use super::camera::CameraRay;
use super::film::{LightSlot, PassLayout, PixelFilter, SamplePasses};
use super::prepare::{spread_attenuation, LightShape, MediumStack, PreparedScene, SceneHit, VIS_CAMERA, VIS_DIFFUSE, VIS_GLOSSY, VIS_SHADOW, VIS_TRANSMISSION, VIS_VOLUME};
use super::rng::Rng;
use super::sampling::{cosine_hemisphere, onb, power_heuristic};
use super::volume::{delta_track, hg_eval, hg_sample, Track};
use super::{luminance, Pass, PixelAccum, RenderSettings};
use crate::math::Vec3;
use core::f32::consts::FRAC_1_PI;
use core::ops::Range;

/// What one camera path returns.
pub(crate) struct PathResult {
    pub radiance: Vec3,
    pub alpha: f32,
    pub albedo: Vec3,
    pub normal: Vec3,
    pub passes: SamplePasses,
}

impl PathResult {
    fn empty() -> PathResult {
        PathResult { radiance: Vec3::ZERO, alpha: 0.0, albedo: Vec3::ZERO, normal: Vec3::ZERO, passes: SamplePasses::default() }
    }
}

/// Per-frame constants shared by every pixel sample.
pub(crate) struct Frame {
    pub layout: PassLayout,
    pub filter: PixelFilter,
    pub seed: u32,
    /// Shutter interval in frames relative to the current frame.
    pub shutter: Option<(f32, f32)>,
    /// Adaptive sampling: (threshold, min samples, check interval).
    pub adaptive: Option<(f32, u32, u32)>,
}

/// Add samples `samples` of pixel (`x`, `y`) to `acc` (and its pass record
/// `rec`). Each sample uses its own RNG, so the result depends only on the
/// pixel and the sample indices. With adaptive sampling the pixel stops once
/// converged (the decision only looks at the pixel's own samples).
#[allow(clippy::too_many_arguments)]
pub(crate) fn accumulate_pixel(scene: &PreparedScene, s: &RenderSettings, f: &Frame, x: u32, y: u32, samples: Range<u32>, acc: &mut PixelAccum, rec: &mut [f32]) {
    for si in samples {
        if acc.done {
            return;
        }
        let mut rng = Rng::for_pixel(x, y, si, f.seed);
        let (ux, uy, ut, lu, lv) = (rng.f32(), rng.f32(), rng.f32(), rng.f32(), rng.f32());
        let (fx, fy) = (x as f32 + 0.5 + f.filter.sample(ux), y as f32 + 0.5 + f.filter.sample(uy));
        let time = f.shutter.map_or(0.0, |(t0, t1)| t0 + (t1 - t0) * ut);
        let r = match scene.camera.ray(fx, fy, s.width, s.height, time, lu, lv) {
            Some(ray) => trace(scene, &ray, time, &mut rng, s, &f.layout),
            None => PathResult::empty(),
        };
        let c = if finite(r.radiance) { r.radiance } else { Vec3::ZERO };
        acc.color += c;
        acc.alpha += if r.alpha.is_finite() { r.alpha } else { 0.0 };
        acc.lum_sq += luminance(c) * luminance(c);
        acc.albedo += r.albedo;
        acc.normal += r.normal;
        if si % 2 == 1 {
            acc.half += c;
        }
        acc.n += 1;
        f.layout.add(rec, &r.passes);
        if let Some((threshold, min, step)) = f.adaptive {
            if acc.n >= min && acc.n % step == 0 && converged(acc, threshold) {
                acc.done = true;
            }
        }
    }
}

/// Cycles' convergence test: the mean of all samples against the mean of
/// every other sample, relative to the square root of the intensity.
fn converged(acc: &PixelAccum, threshold: f32) -> bool {
    let n = acc.n as f32;
    let i = acc.color * (1.0 / n);
    let a = acc.half * (2.0 / n);
    let diff = (i.x - a.x).abs() + (i.y - a.y).abs() + (i.z - a.z).abs();
    let err = diff / (1e-4 + (i.x + i.y + i.z).max(0.0).sqrt());
    err < threshold
}

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn clamp_contribution(c: Vec3, limit: f32) -> Vec3 {
    let m = c.max_elem();
    if limit > 0.0 && m > limit { c * (limit / m) } else { c }
}

fn safe_div(a: Vec3, b: Vec3) -> Vec3 {
    let d = |x: f32, y: f32| if y > 1e-6 { x / y } else { 0.0 };
    Vec3::new(d(a.x, b.x), d(a.y, b.y), d(a.z, b.z))
}

/// Offset a ray origin off the surface, to the side of `dir`.
fn offset(p: Vec3, ng: Vec3, dir: Vec3) -> Vec3 {
    let eps = 1e-4 * (1.0 + p.abs().max_elem());
    if dir.dot(ng) >= 0.0 { p + ng * eps } else { p - ng * eps }
}

/// What kind of scattering event a bounce was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Label {
    Diffuse,
    Glossy,
    Transmission,
    Volume,
}

impl Label {
    fn vis(self) -> u8 {
        match self {
            Label::Diffuse => VIS_DIFFUSE,
            Label::Glossy => VIS_GLOSSY,
            Label::Transmission => VIS_TRANSMISSION,
            Label::Volume => VIS_VOLUME,
        }
    }
    fn slots(self) -> (LightSlot, LightSlot) {
        match self {
            Label::Diffuse => (LightSlot::DiffuseDirect, LightSlot::DiffuseIndirect),
            Label::Glossy => (LightSlot::GlossyDirect, LightSlot::GlossyIndirect),
            Label::Transmission => (LightSlot::TransmissionDirect, LightSlot::TransmissionIndirect),
            Label::Volume => (LightSlot::VolumeDirect, LightSlot::VolumeIndirect),
        }
    }
}

/// Per-type bounce counters.
#[derive(Clone, Copy, Default)]
struct Bounces {
    total: u32,
    diffuse: u32,
    glossy: u32,
    transmission: u32,
    volume: u32,
    transparent: u32,
}

impl Bounces {
    /// Count a bounce; true when it went past a limit.
    fn count(&mut self, label: Label, s: &RenderSettings) -> bool {
        let lp = &s.light_paths;
        self.total += 1;
        let over = match label {
            Label::Diffuse => {
                self.diffuse += 1;
                self.diffuse > lp.diffuse_bounces
            }
            Label::Glossy => {
                self.glossy += 1;
                self.glossy > lp.glossy_bounces
            }
            Label::Transmission => {
                self.transmission += 1;
                self.transmission > lp.transmission_bounces
            }
            Label::Volume => {
                self.volume += 1;
                self.volume > lp.volume_bounces
            }
        };
        over || self.total > s.max_bounces
    }
}

/// Where light contributions go: the combined sum (clamped) and, when light
/// passes are on, the pass of the path's first scattering event.
struct Film<'a> {
    out: &'a mut PathResult,
    s: &'a RenderSettings,
    passes: bool,
    /// The first scattering event's kind and its colour (pass divisor).
    first: Option<(Label, Vec3)>,
}

impl Film<'_> {
    /// Light reaching the camera after `k` scattering events (0: seen
    /// directly). `label` overrides the category for direct lighting at the
    /// first vertex.
    fn add(&mut self, c: Vec3, k: u32, label: Option<(Label, Vec3)>) {
        if c == Vec3::ZERO || !finite(c) {
            return;
        }
        let lp = &self.s.light_paths;
        let c = if k <= 1 { clamp_contribution(c, lp.clamp_direct) } else { clamp_contribution(c, self.s.clamp) };
        self.out.radiance += c;
        if !self.passes || k == 0 {
            return;
        }
        let Some((lab, color)) = label.or(self.first) else { return };
        let (direct, indirect) = lab.slots();
        let slot = if k == 1 { direct } else { indirect };
        let v = if lab == Label::Volume { c } else { safe_div(c, color) };
        self.out.passes.light[slot as usize] += v;
    }

    /// Light seen directly by the camera: emission or the environment.
    fn camera(&mut self, c: Vec3, env: bool) {
        if c == Vec3::ZERO || !finite(c) {
            return;
        }
        let c = clamp_contribution(c, self.s.light_paths.clamp_direct);
        if self.passes {
            self.out.passes.light[if env { LightSlot::Environment } else { LightSlot::Emission } as usize] += c;
        }
        if !(env && self.s.film_transparent) {
            self.out.radiance += c;
        }
    }
}

/// The diffuse part of the Principled BSDF's `eval` (constant over the
/// upper hemisphere): `base (1 - metallic)(1 - transmission)(1 - F(wo)) / pi`.
fn diffuse_f(base: Vec3, metallic: f32, transmission: f32, ior: f32, cos_o: f32) -> Vec3 {
    base * ((1.0 - metallic) * (1.0 - transmission) * (1.0 - fresnel_dielectric(cos_o.clamp(0.0, 1.0), ior)) * FRAC_1_PI)
}

pub(crate) fn trace(scene: &PreparedScene, ray: &CameraRay, time: f32, rng: &mut Rng, s: &RenderSettings, layout: &PassLayout) -> PathResult {
    let lp = &s.light_paths;
    let mut out = PathResult::empty();
    let light_passes = layout.passes.any_light();
    let data_passes = layout.stride > 0;
    let mut film = Film { out: &mut out, s, passes: light_passes, first: None };
    let (mut o, mut d, mut tmax) = (ray.o, ray.d, ray.tmax);
    let mut beta = Vec3::ONE;
    let mut albedo: Option<Vec3> = None;
    let mut normal = Vec3::ZERO;
    // The previous bounce: BSDF pdf (for MIS) and whether it was singular.
    let mut prev_pdf = 0.0f32;
    let mut prev_delta = true;
    let mut min_ray_pdf = f32::INFINITY;
    // Scattering events so far; ray type of the current ray.
    let mut depth = 0u32;
    let mut vis = VIS_CAMERA;
    let mut bounces = Bounces::default();
    let mut terminate = false;
    let mut had_diffuse = false;
    let mut stack = MediumStack::default();
    // Camera-ray transmittance through media before anything opaque (for alpha).
    let mut cam_tr = Vec3::ONE;
    let mut alpha = 0.0f32;
    let eye = ray.eye;
    'path: loop {
        let hit = scene.intersect_ray(o, d, tmax, time, vis);
        let seg = match &hit {
            Some(SceneHit::Tri(h)) => h.t,
            Some(SceneHit::Area(_, t)) => *t,
            None => tmax,
        };

        // The medium between here and the hit.
        if let Some(m) = if scene.any_volume { scene.medium(&stack) } else { None } {
            // Smoke grids are heterogeneous; everything else is tracked
            // as one homogeneous medium.
            let (le, tracked, m) = match scene.grid_track(&stack, &m, o, d, seg, terminate, rng) {
                Some(r) => r,
                None => (m.emission_integral(seg), if terminate { Track::Pass(m.transmittance(seg)) } else { delta_track(&m, seg, rng) }, m),
            };
            if le != Vec3::ZERO {
                if depth == 0 {
                    film.camera(beta.mul_elem(le), false);
                } else {
                    film.add(beta.mul_elem(le), depth, None);
                }
            }
            match tracked {
                Track::Pass(w) => {
                    beta = beta.mul_elem(w);
                    if depth == 0 {
                        cam_tr = cam_tr.mul_elem(w);
                    }
                }
                Track::Absorb => {
                    if depth == 0 {
                        alpha = 1.0;
                    }
                    break 'path;
                }
                Track::Scatter(t, w) => {
                    beta = beta.mul_elem(w);
                    let p = o + d * t;
                    let wo = -d;
                    if depth == 0 {
                        alpha = 1.0;
                        if albedo.is_none() {
                            let st = m.sigma_t();
                            albedo = Some(safe_div(m.sigma_s, st));
                            normal = Vec3::ZERO;
                        }
                    }
                    let lab = (Label::Volume, Vec3::ONE);
                    // Next-event estimation with the phase function.
                    if scene.has_lights() {
                        if let Some((li, ls)) = scene.sample_light(p, rng) {
                            let info = scene.light_info[li as usize];
                            let seen = scene.object_vis.get(info.object as usize).is_none_or(|v| v & VIS_VOLUME != 0);
                            if seen && depth <= info.max_bounces && ls.pdf > 0.0 && ls.li != Vec3::ZERO {
                                let ph = hg_eval(m.anisotropy, wo, ls.wi);
                                let tmax_s = if ls.dist.is_finite() { ls.dist * (1.0 - 1e-3) } else { f32::INFINITY };
                                let tr = if info.cast_shadow { scene.shadow(p, ls.wi, tmax_s, time, stack, lp.transparent_bounces) } else { Vec3::ONE };
                                if tr != Vec3::ZERO {
                                    let wmis = if ls.delta || !info.use_mis { 1.0 } else { power_heuristic(ls.pdf, ph) };
                                    let c = beta.mul_elem(ls.li).mul_elem(tr) * (ph * wmis / ls.pdf);
                                    film.add(c, depth + 1, if depth == 0 { Some(lab) } else { None });
                                }
                            }
                        }
                    }
                    if depth == 0 {
                        film.first = Some(lab);
                    }
                    depth += 1;
                    if bounces.count(Label::Volume, s) {
                        terminate = true;
                    }
                    let (wi, pdf) = hg_sample(m.anisotropy, wo, rng.f32(), rng.f32());
                    prev_pdf = pdf;
                    prev_delta = false;
                    min_ray_pdf = min_ray_pdf.min(pdf);
                    if !roulette(&mut beta, depth, rng) {
                        break 'path;
                    }
                    o = p;
                    d = wi;
                    tmax = f32::INFINITY;
                    vis = VIS_VOLUME;
                    continue 'path;
                }
            }
        }

        let h = match hit {
            None => {
                let le = scene.env.eval_ray(d, ray_state(vis, prev_delta, depth, &bounces));
                if depth == 0 {
                    film.camera(beta.mul_elem(le), true);
                    alpha = if s.film_transparent { 1.0 - (cam_tr.x + cam_tr.y + cam_tr.z) / 3.0 } else { 1.0 };
                } else if le != Vec3::ZERO {
                    let w = if prev_delta { 1.0 } else { power_heuristic(prev_pdf, scene.world_pdf(d)) };
                    film.add(beta.mul_elem(le) * w, depth, None);
                }
                albedo.get_or_insert(Vec3::ONE);
                break 'path;
            }
            Some(SceneHit::Area(li, t)) => {
                let info = scene.light_info[li as usize];
                if let LightShape::Area { radiance, normal: n, spread, .. } = &scene.lights[li as usize] {
                    if depth == 0 {
                        alpha = 1.0;
                        film.camera(beta.mul_elem(*radiance) * spread_attenuation(*spread, -d.dot(*n)), false);
                    } else if info.use_mis && depth <= info.max_bounces.saturating_add(1) {
                        let w = if prev_delta { 1.0 } else { power_heuristic(prev_pdf, scene.surface_light_pdf(li, d, t, *n)) };
                        film.add(beta.mul_elem(*radiance) * (w * spread_attenuation(*spread, -d.dot(*n))), depth, None);
                    }
                }
                albedo.get_or_insert(Vec3::ONE);
                break 'path;
            }
            Some(SceneHit::Tri(h)) => h,
        };

        let (g, sh) = scene.triangle(h.prim, time);
        let g = &g;
        let sh = &sh;
        let mat = &scene.materials[sh.material as usize];
        let p = o + d * h.t;
        let wo = -d;
        let ng = g.e1.cross(g.e2).normalize();
        let b0 = 1.0 - h.u - h.v;

        // Volume boundaries have no surface: cross into or out of the medium.
        if scene.null_surface(sh.material) {
            stack.cross(sh.material, ng.dot(d) < 0.0);
            bounces.transparent += 1;
            if bounces.transparent > lp.transparent_bounces {
                break 'path;
            }
            o = offset(p, ng, d);
            if tmax.is_finite() {
                tmax -= h.t;
            }
            continue 'path;
        }
        // Holdout objects cut a hole for the camera.
        if depth == 0 && scene.object_holdout.get(sh.object as usize).copied().unwrap_or(false) {
            break 'path;
        }

        // Node materials: run the shader program at the hit.
        let node = mat.program.as_ref().map(|prog| shading::shade_nodes(scene, prog, g, sh, &h, d, ray_state(vis, prev_delta, depth, &bounces), rng.next_u32()));
        if depth == 0 && node.as_ref().is_some_and(|n| n.holdout >= 0.5) {
            break 'path;
        }
        let emission = node.as_ref().map_or(mat.emission, |n| n.emission);
        if emission != Vec3::ZERO {
            if depth == 0 {
                film.camera(beta.mul_elem(emission), false);
            } else {
                let w = if prev_delta || sh.light == u32::MAX { 1.0 } else { power_heuristic(prev_pdf, scene.surface_light_pdf(sh.light, d, h.t, ng)) };
                film.add(beta.mul_elem(emission) * w, depth, None);
            }
        }
        if terminate {
            break 'path;
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
        let (bsdf, colors, base) = match node {
            Some(nd) => {
                ns = nd.ns;
                (SurfaceBsdf::Nodes(nd.bsdf), nd.colors, nd.albedo)
            }
            None => {
                // Filter Glossy: blur glossy lobes after blurry bounces (Cycles).
                let blurred;
                let mat = if lp.filter_glossy > 0.0 && min_ray_pdf.is_finite() && lp.filter_glossy * min_ray_pdf < 1.0 {
                    let rough = ((1.0 - lp.filter_glossy * min_ray_pdf).sqrt() * 0.5).sqrt();
                    if rough > mat.roughness {
                        blurred = super::prepare::ShadeMaterial { roughness: rough, ..mat.clone() };
                        &blurred
                    } else {
                        mat
                    }
                } else {
                    mat
                };
                let mut base = mat.base;
                if let Some(ti) = mat.texture {
                    let uv = sh.uv[0] * b0 + sh.uv[1] * h.u + sh.uv[2] * h.v;
                    let c = scene.images[ti].sample(uv.x, uv.y);
                    base = base.mul_elem(Vec3::new(c[0], c[1], c[2]));
                }
                let bsdf = Bsdf::new(base, mat.metallic, mat.roughness, mat.ior, mat.transmission, ns, ng, wo, outside);
                let cos_o = ns.dot(wo);
                let f_diff = diffuse_f(base, mat.metallic, mat.transmission, mat.ior, cos_o);
                // Pass colours of this vertex's lobes.
                let colors = [
                    base * ((1.0 - mat.metallic) * (1.0 - mat.transmission)),
                    base * mat.metallic + Vec3::splat((1.0 - mat.metallic) * fresnel_dielectric(cos_o.clamp(0.0, 1.0), mat.ior)),
                    base * ((1.0 - mat.metallic) * mat.transmission),
                ];
                (SurfaceBsdf::Legacy { bsdf, f_diff, ns }, colors, base)
            }
        };
        if albedo.is_none() {
            albedo = Some(base);
            normal = ns;
        }
        if depth == 0 {
            alpha = 1.0;
            if data_passes {
                let ps = &mut film.out.passes;
                // Z along the view axis (along the ray for panoramas); mist by
                // distance from the camera (view-axis depth for orthographic).
                let dist = (p - eye).length();
                let z = if ray.panoramic { dist } else { (p - eye).dot(ray.forward) };
                ps.depth = z.max(0.0);
                ps.mist = s.mist.eval(if ray.orthographic { z } else { dist });
                ps.normal = ns;
                ps.position = p;
                let uv = sh.uv[0] * b0 + sh.uv[1] * h.u + sh.uv[2] * h.v;
                ps.uv = Vec3::new(uv.x, uv.y, 0.0);
                ps.object_index = scene.object_pass_index.get(sh.object as usize).copied().unwrap_or(0) as f32;
                ps.material_index = scene.material_pass_index.get(sh.material as usize).copied().unwrap_or(0) as f32;
                ps.diffuse_color = colors[0];
                ps.glossy_color = colors[1];
                ps.transmission_color = colors[2];
                ps.albedo = base;
                ps.dnormal = ns;
                if layout.passes.contains(Pass::AmbientOcclusion) {
                    let (t, b) = onb(ns);
                    let l = cosine_hemisphere(rng.f32(), rng.f32());
                    let dir = t * l.x + b * l.y + ns * l.z;
                    let occluded = scene.intersect_ray(offset(p, ng, dir), dir, s.ao_distance.max(0.0), time, VIS_SHADOW).is_some();
                    ps.ao = if occluded { Vec3::ZERO } else { Vec3::ONE };
                }
            }
        }
        // Lobes removed by the caustics options after a diffuse bounce.
        let no_gloss = had_diffuse && !lp.caustics_reflective;
        let no_trans = had_diffuse && !lp.caustics_refractive;

        // Next-event estimation.
        if bsdf.has_smooth_lobes() && scene.has_lights() {
            if let Some((li, ls)) = scene.sample_light(p, rng) {
                let info = scene.light_info[li as usize];
                let lvis = scene.object_vis.get(info.object as usize).copied().unwrap_or(u8::MAX);
                if depth <= info.max_bounces && ls.pdf > 0.0 && ls.li != Vec3::ZERO && lvis & (VIS_DIFFUSE | VIS_GLOSSY) != 0 {
                    let (f, f_diff, bpdf, cosf) = bsdf.eval(wo, ls.wi);
                    let fd = if lvis & VIS_DIFFUSE != 0 && f != Vec3::ZERO { f_diff } else { Vec3::ZERO };
                    let fg = if lvis & VIS_GLOSSY != 0 && !no_gloss { (f - f_diff).max(Vec3::ZERO) } else { Vec3::ZERO };
                    if fd + fg != Vec3::ZERO {
                        let origin = offset(p, ng, ls.wi);
                        let tmax_s = if ls.dist.is_finite() { ls.dist * (1.0 - 1e-3) } else { f32::INFINITY };
                        let tr = if info.cast_shadow { scene.shadow(origin, ls.wi, tmax_s, time, stack, lp.transparent_bounces) } else { Vec3::ONE };
                        if tr != Vec3::ZERO {
                            let w = if ls.delta || !info.use_mis { 1.0 } else { power_heuristic(ls.pdf, bpdf) };
                            let k = beta.mul_elem(ls.li).mul_elem(tr) * (cosf * w / ls.pdf);
                            if depth == 0 && light_passes {
                                film.add(k.mul_elem(fd), 1, Some((Label::Diffuse, colors[0])));
                                film.add(k.mul_elem(fg), 1, Some((Label::Glossy, colors[1])));
                            } else {
                                film.add(k.mul_elem(fd + fg), depth + 1, None);
                            }
                        }
                    }
                }
            }
        }

        // Continue the path.
        let Some(bs) = bsdf.sample(wo, rng) else { break 'path };
        if bs.label & L_TRANSPARENT != 0 {
            // Straight through a transparent closure: not a bounce.
            beta = beta.mul_elem(bs.weight);
            bounces.transparent += 1;
            if bounces.transparent > lp.transparent_bounces || !finite(beta) || beta.max_elem() <= 0.0 {
                break 'path;
            }
            o = offset(p, ng, d);
            if tmax.is_finite() {
                tmax -= h.t;
            }
            continue 'path;
        }
        let mut weight = bs.weight;
        let label = if bs.wi.dot(ng) < 0.0 {
            Label::Transmission
        } else if bs.delta {
            Label::Glossy
        } else {
            // Which lobe made this reflection: pick by their share of f.
            let (f, fd, _, _) = bsdf.eval(wo, bs.wi);
            let pd = if luminance(f) > 0.0 { (luminance(fd) / luminance(f)).clamp(0.0, 1.0) } else { 0.0 };
            if rng.f32() < pd {
                weight = weight.mul_elem(safe_div(fd, f)) * (1.0 / pd);
                Label::Diffuse
            } else {
                weight = weight.mul_elem(safe_div(f - fd, f)) * (1.0 / (1.0 - pd));
                Label::Glossy
            }
        };
        if (label == Label::Glossy && no_gloss) || (label == Label::Transmission && no_trans) {
            break 'path;
        }
        beta = beta.mul_elem(weight);
        if !finite(beta) || beta.max_elem() <= 0.0 {
            break 'path;
        }
        if depth == 0 {
            film.first = Some((label, colors[match label {
                Label::Diffuse => 0,
                Label::Glossy => 1,
                _ => 2,
            }]));
        }
        had_diffuse |= label == Label::Diffuse;
        prev_pdf = bs.pdf;
        prev_delta = bs.delta;
        if !bs.delta {
            min_ray_pdf = min_ray_pdf.min(bs.pdf);
        }
        depth += 1;
        if bounces.count(label, s) {
            terminate = true;
        }
        if !roulette(&mut beta, depth, rng) {
            break 'path;
        }
        o = offset(p, ng, bs.wi);
        d = bs.wi;
        tmax = f32::INFINITY;
        vis = label.vis();
    }
    out.alpha = alpha;
    out.albedo = albedo.unwrap_or(Vec3::ONE);
    out.normal = normal;
    out
}

/// The Light Path node's view of the current ray.
fn ray_state(vis: u8, prev_delta: bool, depth: u32, b: &Bounces) -> RayState {
    use super::shader::{RAY_CAMERA, RAY_DIFFUSE, RAY_GLOSSY, RAY_REFLECT, RAY_SINGULAR, RAY_TRANSMIT, RAY_VOLUME_SCATTER};
    let singular = if prev_delta { RAY_SINGULAR } else { 0 };
    let flags = match vis {
        VIS_CAMERA => RAY_CAMERA,
        VIS_DIFFUSE => RAY_DIFFUSE | RAY_REFLECT,
        VIS_GLOSSY => RAY_GLOSSY | RAY_REFLECT | singular,
        VIS_TRANSMISSION => RAY_GLOSSY | RAY_TRANSMIT | singular,
        VIS_VOLUME => RAY_VOLUME_SCATTER,
        _ => 0,
    };
    let c = |x: u32| x.min(u16::MAX as u32) as u16;
    RayState { flags, depth: c(depth), diffuse: c(b.diffuse), glossy: c(b.glossy), transparent: c(b.transparent), transmission: c(b.transmission) }
}

/// Russian roulette from the third bounce; false ends the path.
fn roulette(beta: &mut Vec3, depth: u32, rng: &mut Rng) -> bool {
    if depth >= 3 {
        let q = (1.0 - beta.max_elem()).max(0.05);
        if rng.f32() < q {
            return false;
        }
        *beta *= 1.0 / (1.0 - q);
    }
    true
}
