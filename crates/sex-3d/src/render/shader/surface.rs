// SPDX-License-Identifier: GPL-3.0-only
//! The renderer's side of shader nodes: per-scene shading data, building the
//! [`ShadeCtx`] at a hit, the surface BSDF the integrator works with (legacy
//! Principled fields or node closures), emission of node-material
//! triangles, transparent shadows and the world shader.
use super::{Closures, Program, RayState, Scratch, ShadeCtx};
use crate::math::{Mat4, Vec2, Vec3};
use crate::render::bsdf::{Bsdf, BsdfSample, ClosureBsdf, LobeKind};
use crate::render::bvh::{Hit, TriGeom};
use crate::render::camera::Pose;
use crate::render::luminance;
use crate::render::prepare::{PreparedScene, TriShade};
use crate::render::rng::Rng;
use crate::render::sampling::{onb, Distribution1D};
use crate::render::volume::VolumeCoeffs;
use crate::scene::{Image, ObjectData, Scene};
use core::f32::consts::{PI, TAU};
use std::sync::Arc;

/// Per-object data for input and texture nodes.
#[derive(Clone, Debug)]
pub(crate) struct ObjectShade {
    pub to_world: Mat4,
    pub to_object: Mat4,
    /// Generated coordinates: (object position - min) * inv_size.
    pub gen_min: Vec3,
    pub gen_inv_size: Vec3,
    pub location: Vec3,
    /// Object Info "Random" (a hash of the name).
    pub random: f32,
}

/// What node programs read besides the hit itself.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShadeData {
    pub images: Vec<Image>,
    pub objects: Vec<ObjectShade>,
    pub camera: Option<Pose>,
    pub tan_half: f32,
}

fn hash_string(s: &str) -> u32 {
    // FNV-1a, then Jenkins' finaliser.
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    super::tex::hash_uint(h)
}

impl ShadeData {
    pub fn new(scene: &Scene, with_objects: bool) -> ShadeData {
        let objects = if with_objects {
            (0..scene.objects.len())
                .map(|oi| {
                    let to_world = scene.world_matrix(oi);
                    let mut lo = Vec3::splat(f32::INFINITY);
                    let mut hi = Vec3::splat(f32::NEG_INFINITY);
                    if matches!(scene.objects[oi].data, ObjectData::Mesh(_)) {
                        if let Some(m) = scene.evaluated_mesh(oi) {
                            for p in &m.positions {
                                lo = lo.min(*p);
                                hi = hi.max(*p);
                            }
                        }
                    }
                    if lo.x > hi.x {
                        lo = Vec3::splat(-1.0);
                        hi = Vec3::ONE;
                    }
                    let size = hi - lo;
                    let inv = |x: f32| if x > 1e-8 { 1.0 / x } else { 0.0 };
                    ObjectShade {
                        to_world,
                        to_object: to_world.inverse(),
                        gen_min: lo,
                        gen_inv_size: Vec3::new(inv(size.x), inv(size.y), inv(size.z)),
                        location: to_world.transform_point(Vec3::ZERO),
                        random: hash_string(&scene.objects[oi].name) as f32 / u32::MAX as f32,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        ShadeData { images: scene.images.clone(), objects, camera: None, tan_half: 0.4 }
    }
}

/// A node tree equivalent to a material's legacy Principled fields (for the
/// rasterized engines, which shade everything through node programs).
pub(crate) fn legacy_tree(m: &crate::scene::Material) -> crate::nodes::NodeTree {
    use crate::nodes::{NodeTree, Prop, Value};
    let mut t = NodeTree::default_material(&m.name);
    if let Some(p) = t.find_type("ShaderNodeBsdfPrincipled") {
        let c = m.base_color;
        let n = &mut t.nodes[p];
        n.set_input("Base Color", Value::Color([c[0], c[1], c[2], 1.0]));
        n.set_input("Metallic", Value::Float(m.metallic));
        n.set_input("Roughness", Value::Float(m.roughness));
        n.set_input("IOR", Value::Float(m.ior));
        n.set_input("Transmission Weight", Value::Float(m.transmission));
        let e = m.emission;
        n.set_input("Emission Color", Value::Color([e.x, e.y, e.z, 1.0]));
        n.set_input("Emission Strength", Value::Float(m.emission_strength));
        if let Some(img) = m.base_color_texture {
            if let Some(i) = t.add("ShaderNodeTexImage", [-500.0, 300.0]) {
                t.nodes[i].set_prop("image", Prop::Id(Some(img)));
                let _ = t.link(i, "Color", p, "Base Color");
            }
        }
    }
    t
}

/// The BSDF at a surface hit: the legacy Principled fields or node closures.
pub(crate) enum SurfaceBsdf {
    Legacy { bsdf: Bsdf, f_diff: Vec3, ns: Vec3 },
    Nodes(ClosureBsdf),
}

impl SurfaceBsdf {
    pub fn has_smooth_lobes(&self) -> bool {
        match self {
            SurfaceBsdf::Legacy { bsdf, .. } => bsdf.has_smooth_lobes(),
            SurfaceBsdf::Nodes(b) => b.has_smooth_lobes(),
        }
    }

    /// (f, its diffuse part, pdf, cosine factor still to apply). Node
    /// closures include the cosine already (factor 1).
    pub fn eval(&self, wo: Vec3, wi: Vec3) -> (Vec3, Vec3, f32, f32) {
        match self {
            SurfaceBsdf::Legacy { bsdf, f_diff, ns } => {
                let (f, pdf) = bsdf.eval(wo, wi);
                (f, f_diff.min(f), pdf, ns.dot(wi).abs())
            }
            SurfaceBsdf::Nodes(b) => {
                let (f, fd, pdf) = b.eval_split(wo, wi);
                (f, fd, pdf, 1.0)
            }
        }
    }

    pub fn sample(&self, wo: Vec3, rng: &mut Rng) -> Option<BsdfSample> {
        match self {
            SurfaceBsdf::Legacy { bsdf, .. } => bsdf.sample(wo, rng),
            SurfaceBsdf::Nodes(b) => b.sample(wo, rng),
        }
    }
}

/// A node material evaluated at a hit.
pub(crate) struct NodeSurface {
    pub bsdf: ClosureBsdf,
    /// Shading normal after bump/displacement (facing the ray).
    pub ns: Vec3,
    pub emission: Vec3,
    /// Diffuse, glossy and transmission pass colours.
    pub colors: [Vec3; 3],
    pub albedo: Vec3,
    pub holdout: f32,
}

/// The hit's interpolated UV and the world-to-UV derivative helper.
struct UvFrame {
    uv: Vec2,
    e: (Vec3, Vec3),
    duv: (Vec2, Vec2),
}

impl UvFrame {
    /// UV change for a world offset `w` in the triangle's plane.
    fn delta(&self, w: Vec3) -> Vec3 {
        let (e1, e2) = self.e;
        let (a11, a12, a22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-20 {
            return Vec3::ZERO;
        }
        let (b1, b2) = (w.dot(e1), w.dot(e2));
        let a = (b1 * a22 - b2 * a12) / det;
        let b = (a11 * b2 - a12 * b1) / det;
        let d = self.duv.0 * a + self.duv.1 * b;
        Vec3::new(d.x, d.y, 0.0)
    }

    /// The UV tangent (dP/du, orthogonal to `n`) and bitangent sign.
    fn tangent(&self, n: Vec3) -> Option<(Vec3, f32)> {
        let (e1, e2) = self.e;
        let (d1, d2) = self.duv;
        let det = d1.x * d2.y - d2.x * d1.y;
        if det.abs() < 1e-20 {
            return None;
        }
        let t = (e1 * d2.y - e2 * d1.y) / det;
        let b = (e2 * d1.x - e1 * d2.x) / det;
        let t = t - n * n.dot(t);
        if t.length_squared() < 1e-20 {
            return None;
        }
        let t = t.normalize();
        Some((t, if n.cross(t).dot(b) < 0.0 { -1.0 } else { 1.0 }))
    }
}

/// Build the shading context for a point (barycentrics `u`, `v`) of a
/// triangle seen along `d`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn surface_ctx<'a>(scene: &'a PreparedScene, prog: &Program, g: &TriGeom, sh: &TriShade, u: f32, v: f32, d: Vec3, t: f32, ray: RayState, seed: u32) -> ShadeCtx<'a> {
    let p = g.v0 + g.e1 * u + g.e2 * v;
    let b0 = 1.0 - u - v;
    let wo = -d;
    let ng = g.e1.cross(g.e2).normalize();
    let outside = ng.dot(wo) > 0.0;
    let ng_f = if outside { ng } else { -ng };
    let n_interp = (sh.n[0] * b0 + sh.n[1] * u + sh.n[2] * v).normalize();
    let mut ns = if outside { n_interp } else { -n_interp };
    if ns.dot(wo) <= 1e-4 {
        ns = ng_f;
    }
    let uvf = UvFrame { uv: sh.uv[0] * b0 + sh.uv[1] * u + sh.uv[2] * v, e: (g.e1, g.e2), duv: (sh.uv[1] - sh.uv[0], sh.uv[2] - sh.uv[0]) };
    // Bump offsets: about a pixel footprint at the hit.
    let eps = (t * 1e-3).max(1e-5 * (1.0 + p.abs().max_elem()));
    let (tx, ty) = onb(ns);
    let (dpdx, dpdy) = (tx * eps, ty * eps);
    let data = scene.shade_data.as_deref();
    ShadeCtx {
        scene: Some(scene),
        data,
        p,
        dpdx,
        dpdy,
        n: ns,
        n_smooth: ns,
        ng: ng_f,
        i: wo,
        backfacing: !outside,
        uv: Vec3::new(uvf.uv.x, uvf.uv.y, 0.0),
        duvdx: uvf.delta(dpdx),
        duvdy: uvf.delta(dpdy),
        bary: (u, v),
        tangent: if prog.info.uv_tangent { uvf.tangent(n_interp) } else { None },
        object: Some(sh.object),
        material_index: scene.material_pass_index.get(sh.material as usize).copied().unwrap_or(0) as f32,
        ray,
        ray_length: t,
        tri: [g.v0, g.v0 + g.e1, g.v0 + g.e2],
        world: false,
        seed,
        vcol: std::array::from_fn(|k| sh.col[0][k] * b0 + sh.col[1][k] * u + sh.col[2][k] * v),
    }
}

thread_local! {
    static SCRATCH: core::cell::RefCell<Scratch> = core::cell::RefCell::new(Scratch::default());
}

/// Run `prog` at a context with this thread's scratch registers.
pub(crate) fn run(prog: &Program, ctx: &mut ShadeCtx) -> Closures {
    SCRATCH.with(|s| match s.try_borrow_mut() {
        Ok(mut s) => prog.eval_surface(ctx, &mut s),
        Err(_) => prog.eval_surface(ctx, &mut Scratch::default()),
    })
}

/// Evaluate a node material at a hit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shade_nodes(scene: &PreparedScene, prog: &Program, g: &TriGeom, sh: &TriShade, h: &Hit, d: Vec3, ray: RayState, seed: u32) -> NodeSurface {
    let mut ctx = surface_ctx(scene, prog, g, sh, h.u, h.v, d, h.t, ray, seed);
    let mut c = run(prog, &mut ctx);
    if prog.surface.is_none() && prog.volume.is_some() {
        // A volume-only material: the surface is a transparent boundary.
        c.transparent = Vec3::ONE;
    }
    let wo = -d;
    let mut colors = [Vec3::ZERO; 3];
    for l in &c.lobes {
        let a = l.weight.mul_elem(l.albedo(wo));
        match l.kind {
            LobeKind::Diffuse { .. } | LobeKind::Translucent | LobeKind::Sheen { .. } | LobeKind::Toon { glossy: false, .. } => colors[0] += l.weight,
            LobeKind::Glass { tint_t, .. } => {
                colors[1] += a;
                colors[2] += l.weight.mul_elem(tint_t);
            }
            _ => colors[1] += a,
        }
    }
    let bsdf = ClosureBsdf::new(c.lobes, c.transparent, ctx.ng, wo);
    let albedo = bsdf.albedo(wo);
    NodeSurface { bsdf, ns: ctx.n, emission: c.emission, colors, albedo, holdout: c.holdout }
}

impl PreparedScene {
    /// Emitted radiance of node-material triangle `prim` at point `q`
    /// seen from direction `-d`.
    pub(crate) fn node_emission(&self, prim: u32, q: Vec3, d: Vec3) -> Option<Vec3> {
        let (g, sh) = self.triangle(prim, 0.0);
        let prog = self.materials.get(sh.material as usize)?.program.as_ref()?;
        if !prog.info.emission {
            return Some(Vec3::ZERO);
        }
        let (u, v) = barycentric(&g, q);
        let mut ctx = surface_ctx(self, prog, &g, &sh, u, v, d, 1.0, RayState { flags: super::RAY_SHADOW, ..Default::default() }, 0);
        ctx.p = q;
        Some(run(prog, &mut ctx).emission)
    }

    /// Light let through a surface hit by a shadow ray (zero when opaque).
    pub(crate) fn shadow_transparency(&self, h: &Hit, d: Vec3, time: f32) -> Vec3 {
        let (g, sh) = self.triangle(h.prim, time);
        let Some(prog) = self.materials.get(sh.material as usize).and_then(|m| m.program.as_ref()) else { return Vec3::ZERO };
        if !prog.info.transparent {
            return Vec3::ZERO;
        }
        let mut ctx = surface_ctx(self, prog, &g, &sh, h.u, h.v, d, h.t, RayState::shadow(), 0);
        let c = run(prog, &mut ctx);
        if prog.surface.is_none() {
            return Vec3::ONE;
        }
        c.transparent
    }
}

fn barycentric(g: &TriGeom, q: Vec3) -> (f32, f32) {
    let w = q - g.v0;
    let (e1, e2) = (g.e1, g.e2);
    let (a11, a12, a22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let det = a11 * a22 - a12 * a12;
    if det.abs() < 1e-30 {
        return (0.0, 0.0);
    }
    let (b1, b2) = (w.dot(e1), w.dot(e2));
    ((b1 * a22 - b2 * a12) / det, (a11 * b2 - a12 * b1) / det)
}

/// Estimated radiance of a node-emissive triangle, for light selection:
/// the largest of a few points (centre and towards the corners).
pub(crate) fn emission_estimate(data: &ShadeData, prog: &Program, g: &TriGeom, sh: &TriShade, material_index: f32) -> Vec3 {
    if !prog.info.emission {
        return Vec3::ZERO;
    }
    let mut best = Vec3::ZERO;
    for (u, v) in [(1.0 / 3.0, 1.0 / 3.0), (0.1, 0.1), (0.8, 0.1), (0.1, 0.8)] {
        let p = g.v0 + g.e1 * u + g.e2 * v;
        let ng = g.e1.cross(g.e2).normalize();
        let b0 = 1.0 - u - v;
        let n = (sh.n[0] * b0 + sh.n[1] * u + sh.n[2] * v).normalize();
        let uv = sh.uv[0] * b0 + sh.uv[1] * u + sh.uv[2] * v;
        let mut ctx = ShadeCtx {
            data: Some(data),
            p,
            n,
            n_smooth: n,
            ng,
            i: ng,
            uv: Vec3::new(uv.x, uv.y, 0.0),
            bary: (u, v),
            object: Some(sh.object),
            material_index,
            ray_length: 1.0,
            tri: [g.v0, g.v0 + g.e1, g.v0 + g.e2],
            vcol: std::array::from_fn(|k| sh.col[0][k] * b0 + sh.col[1][k] * u + sh.col[2][k] * v),
            ..Default::default()
        };
        let e = run(prog, &mut ctx).emission;
        if luminance(e) > luminance(best) {
            best = e;
        }
    }
    best
}

/// The homogeneous medium of a node material with a Volume output and no
/// Surface output (the integrator's media are homogeneous: the volume tree is
/// evaluated once, at the object's origin).
pub(crate) fn volume_coeffs(prog: &Program, data: &ShadeData) -> Option<VolumeCoeffs> {
    prog.volume?;
    if prog.surface.is_some() {
        return None;
    }
    let mut ctx = ShadeCtx { data: Some(data), ..Default::default() };
    let c = run(prog, &mut ctx);
    let clean = |v: Vec3| v.max(Vec3::ZERO);
    let v = VolumeCoeffs { sigma_a: clean(c.vol_absorb), sigma_s: clean(c.vol_scatter), emission: clean(c.vol_emit), anisotropy: c.vol_g.clamp(-0.99, 0.99) };
    (!v.is_empty()).then_some(v)
}

/// A compiled world shader with what it needs to run.
#[derive(Clone, Debug)]
pub(crate) struct WorldShader {
    pub program: Arc<Program>,
    pub data: Arc<ShadeData>,
}

fn equirect_to_dir(u: f32, v: f32) -> Vec3 {
    let phi = (0.5 - u) * TAU;
    let el = (v - 0.5) * PI;
    Vec3::new(phi.cos() * el.cos(), phi.sin() * el.cos(), el.sin())
}

impl WorldShader {
    /// Background radiance towards `d` for a ray of type `ray`.
    pub fn eval(&self, d: Vec3, ray: RayState) -> Vec3 {
        let mut ctx = ShadeCtx { data: Some(&self.data), p: d, n: -d, n_smooth: -d, ng: -d, i: -d, world: true, ray, ..Default::default() };
        run(&self.program, &mut ctx).background
    }

    /// Whether the background is the same in every direction.
    pub fn constant(&self) -> bool {
        self.program.surf.is_empty()
    }

    /// Luminance over a lat-long grid (row 0 at the top), for importance
    /// sampling: the image's resolution when the tree reads an environment
    /// image (up to 512 x 256), else 128 x 64.
    pub fn grid(&self) -> (usize, usize, Vec<f32>) {
        let mut res = (128usize, 64usize);
        for i in &self.program.surf {
            if let super::Op::Environment { image: Some(im), .. } = i.op {
                if let Some(img) = self.data.images.get(im) {
                    res = ((img.width as usize).clamp(16, 512), (img.height as usize).clamp(8, 256));
                }
            }
        }
        let (w, h) = res;
        let ray = RayState::default();
        let mut grid = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let d = equirect_to_dir((x as f32 + 0.5) / w as f32, 1.0 - (y as f32 + 0.5) / h as f32);
                grid[y * w + x] = luminance(self.eval(d, ray)).max(0.0);
            }
        }
        (w, h, grid)
    }
}

/// Importance-sampling rows and columns from a lat-long luminance grid.
pub(crate) fn grid_rows(w: usize, h: usize, grid: &[f32]) -> Option<(Distribution1D, Vec<Option<Distribution1D>>)> {
    let mut row_w = Vec::with_capacity(h);
    let mut cols = Vec::with_capacity(h);
    for gy in 0..h {
        let sin_theta = ((gy as f32 + 0.5) / h as f32 * PI).sin();
        let row: Vec<f32> = grid[gy * w..(gy + 1) * w].iter().map(|&l| (l + 1e-3) * sin_theta).collect();
        let d = Distribution1D::new(&row);
        row_w.push(d.as_ref().map_or(0.0, |d| d.total()));
        cols.push(d);
    }
    Some((Distribution1D::new(&row_w)?, cols))
}

/// Solid-angle weighted average of a lat-long grid.
pub(crate) fn grid_average(w: usize, h: usize, grid: &[f32]) -> f32 {
    let mut sum = 0.0f64;
    let mut wsum = 0.0f64;
    for y in 0..h {
        let s = ((y as f32 + 0.5) / h as f32 * PI).sin() as f64;
        sum += grid[y * w..(y + 1) * w].iter().map(|&x| x as f64).sum::<f64>() * s;
        wsum += w as f64 * s;
    }
    (sum / wsum.max(1e-30)) as f32
}
