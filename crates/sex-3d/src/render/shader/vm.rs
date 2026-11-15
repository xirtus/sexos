// SPDX-License-Identifier: GPL-3.0-only
//! The shader VM: executes a [`Program`]'s instructions over a register file
//! and walks its closure tree into [`Closures`].
use super::color::{self, Blend};
use super::ops::{self, MapInterp, MappingType, MathOp, VecOp};
use super::tex::{self, Fractal, NoiseType, VoronoiFeature, VoronoiMetric, VoronoiParams, WaveType};
use super::{ClosureKind, Instr, Program, Reg, ShadeCtx, ShaderOp, V4};
use super::{RAY_CAMERA, RAY_DIFFUSE, RAY_GLOSSY, RAY_REFLECT, RAY_SHADOW, RAY_SINGULAR, RAY_TRANSMIT, RAY_VOLUME_SCATTER};
use crate::math::Vec3;
use crate::nodes::{ColorRamp, CurveMapping};
use crate::render::bsdf::{self, aniso_alphas, dielectric_albedo, f82_tint_b, fresnel_avg, ms_compensation, Fresnel, Lobe, LobeKind, ThinFilm};
use crate::render::sampling::onb;
use crate::scene::Image;
use core::f32::consts::{PI, TAU};
use std::sync::Arc;

/// Context values an instruction can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Implicit {
    /// The shading normal.
    Normal,
    /// Generated texture coordinates (the direction for the world).
    Generated,
    Uv,
    /// The default radial tangent.
    Tangent,
}

/// One instruction's operation, with the node's properties baked in.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    // Conversions (Blender's implicit socket conversions).
    ColorToFloat,
    VecToFloat,
    FloatToVec,
    ToColor,
    Implicit(Implicit),
    // Converter, color and vector nodes.
    Math { op: MathOp, clamp: bool },
    VecMath(VecOp),
    MixFloat { clamp: bool },
    MixVec { clamp: bool, non_uniform: bool },
    MixColor { blend: Blend, clamp_fac: bool, clamp_result: bool },
    MixRgb { blend: Blend, use_alpha: bool, clamp: bool },
    MapRange { interp: MapInterp, clamp: bool, vector: bool },
    Clamp { range: bool },
    Mapping(MappingType),
    VecRotate { ty: u8, invert: bool },
    CombineXyz,
    SeparateXyz,
    /// 0 RGB, 1 HSV, 2 HSL.
    CombineColor(u8),
    SeparateColor(u8),
    RgbToBw,
    Invert,
    Gamma,
    BrightContrast,
    HueSat,
    Ramp(Arc<ColorRamp>),
    RgbCurve(Arc<CurveMapping>),
    VecCurve(Arc<CurveMapping>),
    FloatCurve(Arc<CurveMapping>),
    Blackbody,
    Wavelength,
    /// Normal node: the node's direction is the constant in `ins[0]`.
    NormalDot,
    // Textures.
    Noise { dims: u8, ty: NoiseType, normalize: bool },
    Musgrave { dims: u8, ty: NoiseType },
    Voronoi { dims: u8, feature: VoronoiFeature, metric: VoronoiMetric, normalize: bool },
    Wave { ty: WaveType, direction: u8, profile: u8 },
    Magic { depth: i32 },
    Brick { offset: f32, offset_frequency: i32, squash: f32, squash_frequency: i32 },
    Checker,
    Gradient(u8),
    WhiteNoise { dims: u8 },
    Gabor { three_d: bool },
    Image { image: Option<usize>, interp: u8, ext: u8, projection: u8, blend: f32 },
    Environment { image: Option<usize>, interp: u8, mirror_ball: bool },
    Sky(Arc<super::sky::Sky>),
    // Vector nodes that read the shading point.
    Bump { invert: bool },
    NormalMap { space: u8 },
    Displacement { object_space: bool },
    VecDisplacement { space: u8 },
    VecTransform { ty: u8, from: u8, to: u8 },
    // Input nodes.
    Fresnel,
    LayerWeight,
    LightPath,
    LightFalloff,
    Geometry,
    TexCoord { object: Option<usize> },
    ObjectInfo,
    Attribute { uv: bool },
    /// The Color Attribute (Vertex Color) node: the hit's interpolated color.
    VertexColor,
    TangentNode { axis: Option<u8> },
    CameraData,
    Wireframe { pixel: bool },
    Ao { samples: u32, inside: bool },
}

impl Op {
    /// Pure: the result depends only on the inputs (may be folded).
    pub fn pure(&self) -> bool {
        !matches!(
            self,
            Op::Implicit(_)
                | Op::Image { .. }
                | Op::Environment { .. }
                | Op::Bump { .. }
                | Op::NormalMap { .. }
                | Op::Displacement { .. }
                | Op::VecDisplacement { .. }
                | Op::VecTransform { .. }
                | Op::Fresnel
                | Op::LayerWeight
                | Op::LightPath
                | Op::LightFalloff
                | Op::Geometry
                | Op::TexCoord { .. }
                | Op::ObjectInfo
                | Op::Attribute { .. }
                | Op::VertexColor
                | Op::TangentNode { .. }
                | Op::CameraData
                | Op::Wireframe { .. }
                | Op::Ao { .. }
        )
    }
}

/// What the closure tree produced at a shading point.
#[derive(Clone, Debug, Default)]
pub(crate) struct Closures {
    pub lobes: Vec<Lobe>,
    pub emission: Vec3,
    pub transparent: Vec3,
    pub holdout: f32,
    /// World: background radiance.
    pub background: Vec3,
    pub vol_absorb: Vec3,
    pub vol_scatter: Vec3,
    pub vol_emit: Vec3,
    /// Henyey-Greenstein anisotropy, scatter-weighted.
    pub vol_g: f32,
}

fn v3(r: V4) -> Vec3 {
    Vec3::new(r[0], r[1], r[2])
}

fn f4(f: f32) -> V4 {
    [f, f, f, 1.0]
}

fn vec4(v: Vec3) -> V4 {
    [v.x, v.y, v.z, 0.0]
}

fn col4(c: [f32; 3], a: f32) -> V4 {
    [c[0], c[1], c[2], a]
}

fn safe_norm(v: Vec3) -> Vec3 {
    let l = v.length();
    if l > 0.0 && l.is_finite() {
        v / l
    } else {
        Vec3::ZERO
    }
}

fn reflect(i: Vec3, n: Vec3) -> Vec3 {
    n * (2.0 * n.dot(i)) - i
}

fn interp_code(s: &str) -> u8 {
    match s {
        "Closest" => 0,
        "Cubic" | "Smart" => 2,
        _ => 1,
    }
}

pub(crate) fn image_interp(s: &str) -> u8 {
    interp_code(s)
}

pub(crate) fn image_ext(s: &str) -> u8 {
    match s {
        "EXTEND" => 1,
        "CLIP" => 2,
        "MIRROR" => 3,
        _ => 0,
    }
}

/// Fetch a texel with an extension mode; `None` outside for Clip.
fn texel(img: &Image, x: i64, y: i64, ext: u8) -> Option<[f32; 4]> {
    let (w, h) = (img.width as i64, img.height as i64);
    let fix = |i: i64, n: i64| -> Option<i64> {
        match ext {
            1 => Some(i.clamp(0, n - 1)),
            2 => (0..n).contains(&i).then_some(i),
            3 => {
                let m = i.rem_euclid(2 * n);
                Some(if m >= n { 2 * n - 1 - m } else { m })
            }
            _ => Some(i.rem_euclid(n)),
        }
    };
    let (x, y) = (fix(x, w)?, fix(y, h)?);
    img.pixels.get((y * w + x) as usize).copied()
}

fn cubic_bspline(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [(1.0 - t) * (1.0 - t) * (1.0 - t) / 6.0, (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0, (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0, t3 / 6.0]
}

/// Image lookup at (u, v) (v = 0 at the bottom) with interpolation 0 closest,
/// 1 linear, 2 cubic (B-spline) and extension 0 repeat, 1 extend, 2 clip,
/// 3 mirror, like Cycles' `kernel_tex_image_interp`.
pub(crate) fn image_lookup(img: &Image, u: f32, v: f32, interp: u8, ext: u8) -> [f32; 4] {
    if img.width == 0 || img.height == 0 || img.pixels.len() < (img.width * img.height) as usize {
        return [1.0, 0.0, 1.0, 1.0];
    }
    if !(u.is_finite() && v.is_finite()) {
        return [0.0; 4];
    }
    if ext == 2 && !((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)) {
        return [0.0; 4];
    }
    let x = u * img.width as f32;
    // Rows are stored from the top.
    let y = (1.0 - v) * img.height as f32;
    let get = |xi: i64, yi: i64| texel(img, xi, yi, ext).unwrap_or([0.0; 4]);
    match interp {
        0 => get(x.floor() as i64, y.floor() as i64),
        1 => {
            let (x, y) = (x - 0.5, y - 0.5);
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            let (xi, yi) = (x0 as i64, y0 as i64);
            let (a, b, c, d) = (get(xi, yi), get(xi + 1, yi), get(xi, yi + 1), get(xi + 1, yi + 1));
            core::array::from_fn(|i| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy)
        }
        _ => {
            let (x, y) = (x - 0.5, y - 0.5);
            let (x0, y0) = (x.floor(), y.floor());
            let (wx, wy) = (cubic_bspline(x - x0), cubic_bspline(y - y0));
            let (xi, yi) = (x0 as i64, y0 as i64);
            let mut o = [0.0f32; 4];
            for (j, wyj) in wy.iter().enumerate() {
                for (i, wxi) in wx.iter().enumerate() {
                    let p = get(xi + i as i64 - 1, yi + j as i64 - 1);
                    for k in 0..4 {
                        o[k] += p[k] * wxi * wyj;
                    }
                }
            }
            o
        }
    }
}

/// Cycles' box projection weights for an object-space normal.
fn box_weights(n: Vec3, blend: f32) -> Vec3 {
    let n = n.abs();
    let limit = 0.5 * (1.0 + blend);
    let mut w = Vec3::ZERO;
    if n.x > limit * (n.x + n.y) && n.x > limit * (n.x + n.z) {
        w.x = 1.0;
    } else if n.y > limit * (n.x + n.y) && n.y > limit * (n.y + n.z) {
        w.y = 1.0;
    } else if n.z > limit * (n.x + n.z) && n.z > limit * (n.y + n.z) {
        w.z = 1.0;
    } else if blend > 0.0 {
        let sat = |x: f32| x.clamp(0.0, 1.0);
        if n.z < (1.0 - limit) * (n.y + n.x) {
            w.x = sat((n.x / (n.x + n.y) - 0.5 * (1.0 - blend)) / blend);
            w.y = 1.0 - w.x;
        } else if n.x < (1.0 - limit) * (n.y + n.z) {
            w.y = sat((n.y / (n.y + n.z) - 0.5 * (1.0 - blend)) / blend);
            w.z = 1.0 - w.y;
        } else if n.y < (1.0 - limit) * (n.x + n.z) {
            w.x = sat((n.x / (n.x + n.z) - 0.5 * (1.0 - blend)) / blend);
            w.z = 1.0 - w.x;
        } else {
            let f = |c: f32| ((2.0 - limit) * c + (limit - 1.0)) / (2.0 * limit - 1.0);
            w = Vec3::new(f(n.x), f(n.y), f(n.z));
        }
    } else {
        w.x = 1.0;
    }
    w
}

fn object<'a>(ctx: &'a ShadeCtx) -> Option<&'a super::surface::ObjectShade> {
    ctx.data.and_then(|s| s.objects.get(ctx.object? as usize))
}

/// Object-space position for variant `v` (world position without an object).
fn object_pos(ctx: &ShadeCtx, v: u8) -> Vec3 {
    let p = ctx.pos(v);
    match object(ctx) {
        Some(o) => o.to_object.transform_point(p),
        None => p,
    }
}

fn generated(ctx: &ShadeCtx, v: u8) -> Vec3 {
    if ctx.world {
        return ctx.p;
    }
    match object(ctx) {
        Some(o) => (o.to_object.transform_point(ctx.pos(v)) - o.gen_min).mul_elem(o.gen_inv_size),
        None => ctx.pos(v),
    }
}

/// Cycles' default radial tangent about object axis `axis` (2 = Z).
fn radial_tangent(ctx: &ShadeCtx, v: u8, axis: u8) -> Vec3 {
    let g = generated(ctx, v) - Vec3::splat(0.5);
    let data = match axis {
        0 => Vec3::new(0.0, -g.z, g.y),
        1 => Vec3::new(-g.z, 0.0, g.x),
        _ => Vec3::new(-g.y, g.x, 0.0),
    };
    let data = match object(ctx) {
        Some(o) => o.to_world.transform_vector(data),
        None => data,
    };
    safe_norm(ctx.n.cross(safe_norm(data.cross(ctx.n))))
}

fn camera_space(ctx: &ShadeCtx, p: Vec3, point: bool) -> Vec3 {
    match ctx.data.and_then(|s| s.camera) {
        Some(c) => {
            let c = &c;
            let d = if point { p - c.origin } else { p };
            Vec3::new(d.dot(c.right), d.dot(c.up), d.dot(c.forward))
        }
        None => p,
    }
}

fn read(regs: &[V4], ins: &[Reg], i: usize) -> V4 {
    ins.get(i).and_then(|&r| regs.get(r as usize)).copied().unwrap_or([0.0; 4])
}

fn fractal(detail: f32, roughness: f32, lacunarity: f32, offset: f32, gain: f32, normalize: bool) -> Fractal {
    Fractal { detail: detail.clamp(0.0, 15.0), roughness: roughness.max(0.0), lacunarity, offset, gain, normalize }
}

fn noise_dims(dims: u8, p: Vec3, w: f32, f: &Fractal, ty: NoiseType, distortion: f32) -> (f32, [f32; 3]) {
    match dims {
        1 => tex::noise_texture([w], f, ty, distortion, true),
        2 => tex::noise_texture([p.x, p.y], f, ty, distortion, true),
        4 => tex::noise_texture([p.x, p.y, p.z, w], f, ty, distortion, true),
        _ => tex::noise_texture([p.x, p.y, p.z], f, ty, distortion, true),
    }
}

/// Execute one instruction.
pub(crate) fn exec(ins: &Instr, ctx: &ShadeCtx, regs: &mut [V4]) {
    let r = |i: usize| read(regs, &ins.ins, i);
    let f = |i: usize| read(regs, &ins.ins, i)[0];
    let v = |i: usize| v3(read(regs, &ins.ins, i));
    let var = ins.variant;
    let mut out: [V4; 14] = [[0.0; 4]; 14];
    let n_out: usize;
    match &ins.op {
        Op::ColorToFloat => {
            out[0] = f4(color::luminance([r(0)[0], r(0)[1], r(0)[2]]));
            n_out = 1;
        }
        Op::VecToFloat => {
            let a = r(0);
            out[0] = f4((a[0] + a[1] + a[2]) / 3.0);
            n_out = 1;
        }
        Op::FloatToVec => {
            out[0] = f4(f(0));
            n_out = 1;
        }
        Op::ToColor => {
            let a = r(0);
            out[0] = [a[0], a[1], a[2], 1.0];
            n_out = 1;
        }
        Op::Implicit(k) => {
            out[0] = match k {
                Implicit::Normal => vec4(ctx.n),
                Implicit::Generated => vec4(generated(ctx, var)),
                Implicit::Uv => vec4(ctx.uv_at(var)),
                Implicit::Tangent => vec4(radial_tangent(ctx, var, 2)),
            };
            n_out = 1;
        }
        Op::Math { op, clamp } => {
            let x = ops::math(*op, f(0), f(1), f(2));
            out[0] = f4(if *clamp { x.clamp(0.0, 1.0) } else { x });
            n_out = 1;
        }
        Op::VecMath(op) => {
            let (a, b) = ops::vector_math(*op, v(0), v(1), v(2), f(3));
            out[0] = vec4(a);
            out[1] = f4(b);
            n_out = 2;
        }
        Op::MixFloat { clamp } => {
            let t = if *clamp { f(0).clamp(0.0, 1.0) } else { f(0) };
            out[0] = f4(f(1) * (1.0 - t) + f(2) * t);
            n_out = 1;
        }
        Op::MixVec { clamp, non_uniform } => {
            let t = if *non_uniform { v(0) } else { Vec3::splat(f(0)) };
            let t = if *clamp { t.max(Vec3::ZERO).min(Vec3::ONE) } else { t };
            let (a, b) = (v(1), v(2));
            out[0] = vec4(a + (b - a).mul_elem(t));
            n_out = 1;
        }
        Op::MixColor { blend, clamp_fac, clamp_result } => {
            let t = if *clamp_fac { f(0).clamp(0.0, 1.0) } else { f(0) };
            let (a, b) = (r(1), r(2));
            let mut c = color::blend(*blend, t, [a[0], a[1], a[2]], [b[0], b[1], b[2]]);
            if *clamp_result {
                c = c.map(|x| x.clamp(0.0, 1.0));
            }
            out[0] = col4(c, a[3] * (1.0 - t) + b[3] * t);
            n_out = 1;
        }
        Op::MixRgb { blend, use_alpha, clamp } => {
            let (a, b) = (r(1), r(2));
            let mut t = f(0).clamp(0.0, 1.0);
            if *use_alpha {
                t *= b[3];
            }
            let mut c = color::blend(*blend, t, [a[0], a[1], a[2]], [b[0], b[1], b[2]]);
            if *clamp {
                c = c.map(|x| x.clamp(0.0, 1.0));
            }
            out[0] = col4(c, 1.0);
            n_out = 1;
        }
        Op::MapRange { interp, clamp, vector } => {
            if *vector {
                let (x, a, b, c, d, s) = (v(0), v(1), v(2), v(3), v(4), v(5));
                let m = |i: usize| ops::map_range(*interp, *clamp, x[i], a[i], b[i], c[i], d[i], s[i]);
                out[1] = vec4(Vec3::new(m(0), m(1), m(2)));
                out[0] = f4(0.0);
            } else {
                out[0] = f4(ops::map_range(*interp, *clamp, f(0), f(1), f(2), f(3), f(4), f(5)));
            }
            n_out = 2;
        }
        Op::Clamp { range } => {
            out[0] = f4(ops::clamp_node(*range, f(0), f(1), f(2)));
            n_out = 1;
        }
        Op::Mapping(ty) => {
            out[0] = vec4(ops::mapping(*ty, v(0), v(1), v(2), v(3)));
            n_out = 1;
        }
        Op::VecRotate { ty, invert } => {
            out[0] = vec4(ops::vector_rotate(*ty, *invert, v(0), v(1), v(2), f(3), v(4)));
            n_out = 1;
        }
        Op::CombineXyz => {
            out[0] = vec4(Vec3::new(f(0), f(1), f(2)));
            n_out = 1;
        }
        Op::SeparateXyz => {
            let a = r(0);
            out[0] = f4(a[0]);
            out[1] = f4(a[1]);
            out[2] = f4(a[2]);
            n_out = 3;
        }
        Op::CombineColor(mode) => {
            let c = [f(0), f(1), f(2)];
            let c = match mode {
                1 => color::hsv_to_rgb(c),
                2 => color::hsl_to_rgb(c),
                _ => c,
            };
            out[0] = col4(c, 1.0);
            n_out = 1;
        }
        Op::SeparateColor(mode) => {
            let a = r(0);
            let c = [a[0], a[1], a[2]];
            let c = match mode {
                1 => color::rgb_to_hsv(c),
                2 => color::rgb_to_hsl(c),
                _ => c,
            };
            out[0] = f4(c[0]);
            out[1] = f4(c[1]);
            out[2] = f4(c[2]);
            n_out = 3;
        }
        Op::RgbToBw => {
            let a = r(0);
            out[0] = f4(color::luminance([a[0], a[1], a[2]]));
            n_out = 1;
        }
        Op::Invert => {
            let (t, c) = (f(0), r(1));
            out[0] = [c[0] + t * (1.0 - 2.0 * c[0]), c[1] + t * (1.0 - 2.0 * c[1]), c[2] + t * (1.0 - 2.0 * c[2]), c[3]];
            n_out = 1;
        }
        Op::Gamma => {
            let (c, g) = (r(0), f(1));
            let p = |x: f32| if x > 0.0 { x.powf(g) } else { x };
            out[0] = [p(c[0]), p(c[1]), p(c[2]), c[3]];
            n_out = 1;
        }
        Op::BrightContrast => {
            let (c, bright, contrast) = (r(0), f(1), f(2));
            let a = 1.0 + contrast;
            let b = bright - contrast * 0.5;
            let g = |x: f32| (a * x + b).max(0.0);
            out[0] = [g(c[0]), g(c[1]), g(c[2]), c[3]];
            n_out = 1;
        }
        Op::HueSat => {
            let (hue, sat, val, fac, c) = (f(0), f(1), f(2), f(3), r(4));
            let mut hsv = color::rgb_to_hsv([c[0], c[1], c[2]]);
            hsv[0] = (hsv[0] + hue + 0.5).rem_euclid(1.0);
            hsv[1] = (hsv[1] * sat).clamp(0.0, 1.0);
            hsv[2] *= val;
            let n = color::hsv_to_rgb(hsv);
            let m = |i: usize| (fac * n[i] + (1.0 - fac) * c[i]).max(0.0);
            out[0] = [m(0), m(1), m(2), c[3]];
            n_out = 1;
        }
        Op::Ramp(ramp) => {
            let c = ramp.eval(f(0));
            out[0] = c;
            out[1] = f4(c[3]);
            n_out = 2;
        }
        Op::RgbCurve(cm) => {
            let (t, c) = (f(0), r(1));
            // Blender's RGB curves: per-channel curve, then the combined curve.
            let ch = |i: usize| cm.eval(0, cm.eval(i + 1, c[i]));
            let m = |i: usize| (1.0 - t) * c[i] + t * ch(i);
            out[0] = [m(0), m(1), m(2), c[3]];
            n_out = 1;
        }
        Op::VecCurve(cm) => {
            let (t, a) = (f(0), v(1));
            let m = |i: usize| (1.0 - t) * a[i] + t * cm.eval(i, a[i]);
            out[0] = vec4(Vec3::new(m(0), m(1), m(2)));
            n_out = 1;
        }
        Op::FloatCurve(cm) => {
            let (t, x) = (f(0), f(1));
            out[0] = f4((1.0 - t) * x + t * cm.eval(0, x));
            n_out = 1;
        }
        Op::Blackbody => {
            out[0] = col4(color::blackbody(f(0)), 1.0);
            n_out = 1;
        }
        Op::Wavelength => {
            out[0] = col4(color::wavelength(f(0)), 1.0);
            n_out = 1;
        }
        Op::NormalDot => {
            let dir = safe_norm(v(0));
            out[0] = vec4(dir);
            out[1] = f4(dir.dot(safe_norm(v(1))));
            n_out = 2;
        }
        Op::Noise { dims, ty, normalize } => {
            let scale = f(2);
            let fr = fractal(f(3), f(4), f(5), f(6), f(7), *normalize);
            let (fac, c) = noise_dims(*dims, v(0) * scale, f(1) * scale, &fr, *ty, f(8));
            out[0] = f4(fac);
            out[1] = col4(c, 1.0);
            n_out = 2;
        }
        Op::Musgrave { dims, ty } => {
            // Blender 4.1's versioning: roughness = lacunarity^-dimension.
            let scale = f(2);
            let lac = f(5);
            let fr = fractal(f(3), lac.max(1e-5).powf(-f(4)), lac, f(6), f(7), false);
            let (fac, _) = noise_dims(*dims, v(0) * scale, f(1) * scale, &fr, *ty, 0.0);
            out[0] = f4(fac);
            n_out = 1;
        }
        Op::Voronoi { dims, feature, metric, normalize } => {
            let scale = f(2);
            let params = VoronoiParams {
                scale,
                detail: f(3).clamp(0.0, 15.0),
                roughness: f(4).clamp(0.0, 1.0),
                lacunarity: f(5),
                smoothness: (f(6) * 0.5).clamp(0.0, 0.5),
                exponent: f(7),
                randomness: f(8).clamp(0.0, 1.0),
                max_distance: 0.0,
                normalize: *normalize,
                feature: *feature,
                metric: *metric,
            };
            let p = v(0) * scale;
            let w = f(1) * scale;
            let (o, radius) = match dims {
                1 => tex::voronoi(params, [w]),
                2 => tex::voronoi(params, [p.x, p.y]),
                4 => tex::voronoi(params, [p.x, p.y, p.z, w]),
                _ => tex::voronoi(params, [p.x, p.y, p.z]),
            };
            let s = if scale != 0.0 { 1.0 / scale } else { 0.0 };
            let pos = o.position;
            out[0] = f4(o.distance);
            out[1] = col4(o.color, 1.0);
            out[2] = match dims {
                1 => vec4(Vec3::ZERO),
                2 => vec4(Vec3::new(pos[0] * s, pos[1] * s, 0.0)),
                _ => vec4(Vec3::new(pos[0] * s, pos[1] * s, pos[2] * s)),
            };
            out[3] = f4(match dims {
                1 => pos[0] * s,
                4 => pos[3] * s,
                _ => 0.0,
            });
            out[4] = f4(radius);
            n_out = 5;
        }
        Op::Wave { ty, direction, profile } => {
            let x = tex::wave(*ty, *direction, *profile, (v(0) * f(1)).to_array(), f(2), f(3), f(4), f(5), f(6));
            out[0] = f4(x);
            out[1] = f4(x);
            n_out = 2;
        }
        Op::Magic { depth } => {
            let c = tex::magic((v(0) * f(1)).to_array(), *depth, f(2));
            out[0] = col4(c, 1.0);
            out[1] = f4((c[0] + c[1] + c[2]) / 3.0);
            n_out = 2;
        }
        Op::Brick { offset, offset_frequency, squash, squash_frequency } => {
            let b = tex::Brick { mortar_size: f(5), mortar_smooth: f(6), bias: f(7), brick_width: f(8), row_height: f(9), offset: *offset, offset_frequency: *offset_frequency, squash: *squash, squash_frequency: *squash_frequency };
            let (tint, fac) = tex::brick((v(0) * f(4)).to_array(), &b);
            let (c1, c2, mortar) = (r(1), r(2), r(3));
            let mut c = c1;
            if fac != 1.0 {
                for i in 0..3 {
                    c[i] = (1.0 - tint) * c1[i] + tint * c2[i];
                }
            }
            out[0] = core::array::from_fn(|i| if i == 3 { 1.0 } else { c[i] * (1.0 - fac) + mortar[i] * fac });
            out[1] = f4(fac);
            n_out = 2;
        }
        Op::Checker => {
            let x = tex::checker((v(0) * f(3)).to_array());
            out[0] = if x == 1.0 { r(1) } else { r(2) };
            out[1] = f4(x);
            n_out = 2;
        }
        Op::Gradient(ty) => {
            let x = tex::gradient(v(0).to_array(), *ty);
            out[0] = col4([x; 3], 1.0);
            out[1] = f4(x);
            n_out = 2;
        }
        Op::WhiteNoise { dims } => {
            let (p, w) = (v(0), f(1));
            let (x, c) = match dims {
                1 => tex::white_noise([w]),
                2 => tex::white_noise([p.x, p.y]),
                4 => tex::white_noise([p.x, p.y, p.z, w]),
                _ => tex::white_noise([p.x, p.y, p.z]),
            };
            out[0] = f4(x);
            out[1] = col4(c, 1.0);
            n_out = 2;
        }
        Op::Gabor { three_d } => {
            let (a, b, c) = tex::gabor((v(0) * f(1)).to_array(), *three_d, f(2), f(3), f(4), v(5).to_array());
            out[0] = f4(a);
            out[1] = f4(b);
            out[2] = f4(c);
            n_out = 3;
        }
        Op::Image { image, interp, ext, projection, blend } => {
            let img = image.and_then(|i| ctx.data.and_then(|s| s.images.get(i)));
            let co = v(0);
            let c = match img {
                None => [1.0, 0.0, 1.0, 1.0],
                Some(img) => match projection {
                    1 => {
                        let n = match object(ctx) {
                            Some(o) => safe_norm(o.to_world.transpose().transform_vector(ctx.n)),
                            None => ctx.n,
                        };
                        let w = box_weights(n, *blend);
                        let mut c = [0.0f32; 4];
                        let mut add = |wt: f32, u: f32, vv: f32| {
                            if wt > 0.0 {
                                let s = image_lookup(img, u, vv, *interp, *ext);
                                for k in 0..4 {
                                    c[k] += wt * s[k];
                                }
                            }
                        };
                        add(w.x, if n.x < 0.0 { 1.0 - co.y } else { co.y }, co.z);
                        add(w.y, if n.y > 0.0 { 1.0 - co.x } else { co.x }, co.z);
                        add(w.z, if n.z > 0.0 { 1.0 - co.y } else { co.y }, co.x);
                        c
                    }
                    2 | 3 => {
                        let d = ((co - Vec3::splat(0.5)) * 2.0).to_array();
                        let (u, vv) = if *projection == 2 { tex::map_to_sphere(d) } else { tex::map_to_tube(d) };
                        image_lookup(img, u, vv, *interp, *ext)
                    }
                    _ => image_lookup(img, co.x, co.y, *interp, *ext),
                },
            };
            out[0] = c;
            out[1] = f4(c[3]);
            n_out = 2;
        }
        Op::Environment { image, interp, mirror_ball } => {
            let img = image.and_then(|i| ctx.data.and_then(|s| s.images.get(i)));
            let d = safe_norm(v(0));
            out[0] = match img {
                None => [1.0, 0.0, 1.0, 1.0],
                Some(img) => {
                    let (u, vv) = if *mirror_ball { tex::direction_to_mirrorball(d.to_array()) } else { crate::render::prepare::dir_to_equirect(d) };
                    image_lookup(img, u, vv, *interp, if *mirror_ball { 1 } else { 0 })
                }
            };
            n_out = 1;
        }
        Op::Sky(sky) => {
            out[0] = col4(sky.eval(safe_norm(v(0))), 1.0);
            n_out = 1;
        }
        Op::Bump { invert } => {
            let (strength, dist, hc, hx, hy) = (f(0), f(1), f(2), f(3), f(4));
            let n_in = safe_norm(v(5));
            let (dpdx, dpdy) = (ctx.dpdx, ctx.dpdy);
            let rx = dpdy.cross(n_in);
            let ry = n_in.cross(dpdx);
            let det = dpdx.dot(rx);
            let surfgrad = rx * (hx - hc) + ry * (hy - hc);
            let dist = if *invert { -dist } else { dist };
            let mut n = safe_norm(n_in * det.abs() - surfgrad * (dist * det.signum()));
            if n == Vec3::ZERO || det == 0.0 {
                n = n_in;
            }
            let s = strength.clamp(0.0, 1.0);
            out[0] = vec4(safe_norm(n * s + n_in * (1.0 - s)));
            n_out = 1;
        }
        Op::NormalMap { space } => {
            let (strength, c) = (f(0), v(1));
            let m = c * 2.0 - Vec3::ONE;
            let n_geom = if ctx.backfacing { -ctx.n_smooth } else { ctx.n_smooth };
            let mut n = match space {
                // Tangent space.
                0 => match ctx.tangent {
                    Some((t, sign)) => {
                        let b = n_geom.cross(t) * sign;
                        let n = safe_norm(t * m.x + b * m.y + n_geom * m.z);
                        if ctx.backfacing {
                            -n
                        } else {
                            n
                        }
                    }
                    None => ctx.n,
                },
                // Object space.
                1 | 3 => {
                    let m = match object(ctx) {
                        Some(o) => o.to_object.transpose().transform_vector(m),
                        None => m,
                    };
                    let n = safe_norm(m);
                    if ctx.backfacing {
                        -n
                    } else {
                        n
                    }
                }
                // World space.
                _ => {
                    let n = safe_norm(m);
                    if ctx.backfacing {
                        -n
                    } else {
                        n
                    }
                }
            };
            if n == Vec3::ZERO {
                n = ctx.n;
            }
            if strength != 1.0 {
                n = safe_norm(ctx.n + (n - ctx.n) * strength.max(0.0));
            }
            out[0] = vec4(if n == Vec3::ZERO { ctx.n } else { n });
            n_out = 1;
        }
        Op::Displacement { object_space } => {
            let (h, mid, scale) = (f(0), f(1), f(2));
            let n = safe_norm(v(3));
            let d = if *object_space {
                match object(ctx) {
                    Some(o) => {
                        let no = safe_norm(o.to_world.transpose().transform_vector(n));
                        o.to_world.transform_vector(no * ((h - mid) * scale))
                    }
                    None => n * ((h - mid) * scale),
                }
            } else {
                n * ((h - mid) * scale)
            };
            out[0] = vec4(d);
            n_out = 1;
        }
        Op::VecDisplacement { space } => {
            let (c, mid, scale) = (v(0), f(1), f(2));
            let d = (c - Vec3::splat(mid)) * scale;
            let d = match space {
                0 => match ctx.tangent {
                    Some((t, sign)) => {
                        let b = ctx.n.cross(t) * sign;
                        t * d.x + ctx.n * d.y + b * d.z
                    }
                    None => ctx.n * d.y,
                },
                1 => match object(ctx) {
                    Some(o) => o.to_world.transform_vector(d),
                    None => d,
                },
                _ => d,
            };
            out[0] = vec4(d);
            n_out = 1;
        }
        Op::VecTransform { ty, from, to } => {
            // ty: 0 point, 1 vector, 2 normal; spaces: 0 world, 1 object, 2 camera.
            let mut x = v(0);
            let o = object(ctx);
            let to_world = |x: Vec3, sp: u8| -> Vec3 {
                match sp {
                    1 => match o {
                        Some(o) => match ty {
                            0 => o.to_world.transform_point(x),
                            2 => o.to_object.transpose().transform_vector(x),
                            _ => o.to_world.transform_vector(x),
                        },
                        None => x,
                    },
                    2 => match ctx.data.and_then(|s| s.camera) {
                        Some(c) => {
                            let c = &c;
                            let w = c.right * x.x + c.up * x.y + c.forward * x.z;
                            if *ty == 0 {
                                w + c.origin
                            } else {
                                w
                            }
                        }
                        None => x,
                    },
                    _ => x,
                }
            };
            x = to_world(x, *from);
            x = match to {
                1 => match o {
                    Some(o) => match ty {
                        0 => o.to_object.transform_point(x),
                        2 => o.to_world.transpose().transform_vector(x),
                        _ => o.to_object.transform_vector(x),
                    },
                    None => x,
                },
                2 => camera_space(ctx, x, *ty == 0),
                _ => x,
            };
            if *ty == 2 {
                x = safe_norm(x);
            }
            out[0] = vec4(x);
            n_out = 1;
        }
        Op::Fresnel => {
            let n = safe_norm(v(1));
            let eta = f(0).max(1e-5);
            let eta = if ctx.backfacing { 1.0 / eta } else { eta };
            out[0] = f4(bsdf::fresnel_dielectric(ctx.i.dot(n).abs(), eta));
            n_out = 1;
        }
        Op::LayerWeight => {
            let blend = f(0);
            let n = safe_norm(v(1));
            let eta = (1.0 - blend).max(1e-5);
            let eta = if ctx.backfacing { eta } else { 1.0 / eta };
            out[0] = f4(bsdf::fresnel_dielectric(ctx.i.dot(n).abs(), eta));
            let mut facing = ctx.i.dot(n).abs();
            if blend != 0.5 {
                let b = blend.clamp(0.0, 1.0 - 1e-5);
                let b = if b < 0.5 { 2.0 * b } else { 0.5 / (1.0 - b) };
                facing = facing.powf(b);
            }
            out[1] = f4(1.0 - facing);
            n_out = 2;
        }
        Op::LightPath => {
            let fl = ctx.ray.flags;
            let b = |m: u16| if fl & m != 0 { 1.0 } else { 0.0 };
            let vals = [
                b(RAY_CAMERA),
                b(RAY_SHADOW),
                b(RAY_DIFFUSE),
                b(RAY_GLOSSY),
                b(RAY_SINGULAR),
                b(RAY_REFLECT),
                b(RAY_TRANSMIT),
                b(RAY_VOLUME_SCATTER),
                ctx.ray_length,
                ctx.ray.depth as f32,
                ctx.ray.diffuse as f32,
                ctx.ray.glossy as f32,
                ctx.ray.transparent as f32,
                ctx.ray.transmission as f32,
            ];
            for (k, x) in vals.iter().enumerate() {
                out[k] = f4(*x);
            }
            n_out = 14;
        }
        Op::LightFalloff => {
            let (mut strength, smooth) = (f(0), f(1));
            let t = ctx.ray_length;
            if smooth > 0.0 {
                let sq = t * t;
                strength *= sq / (smooth + sq);
            }
            out[0] = f4(strength);
            out[1] = f4(strength * t);
            out[2] = f4(strength * t * t);
            n_out = 3;
        }
        Op::Geometry => {
            out[0] = vec4(ctx.pos(var));
            out[1] = vec4(ctx.n);
            out[2] = vec4(radial_tangent(ctx, var, 2));
            out[3] = vec4(ctx.ng);
            out[4] = vec4(ctx.i);
            out[5] = vec4(Vec3::new(ctx.bary.0, ctx.bary.1, 0.0));
            out[6] = f4(if ctx.backfacing { 1.0 } else { 0.0 });
            out[7] = f4(0.5);
            out[8] = f4(0.0);
            n_out = 9;
        }
        Op::TexCoord { object: target } => {
            if ctx.world {
                let d = ctx.p;
                for k in 0..7 {
                    out[k] = vec4(d);
                }
                out[2] = vec4(Vec3::ZERO);
                out[4] = vec4(camera_space(ctx, d, false));
            } else {
                let p = ctx.pos(var);
                out[0] = vec4(generated(ctx, var));
                out[1] = vec4(match object(ctx) {
                    Some(o) => safe_norm(o.to_world.transpose().transform_vector(ctx.n)),
                    None => ctx.n,
                });
                out[2] = vec4(ctx.uv_at(var));
                let tobj = target.and_then(|t| ctx.data.and_then(|s| s.objects.get(t)));
                out[3] = vec4(match tobj {
                    Some(o) => o.to_object.transform_point(p),
                    None => object_pos(ctx, var),
                });
                let cp = camera_space(ctx, p, true);
                out[4] = vec4(cp);
                let tan = ctx.data.map_or(1.0, |s| s.tan_half.max(1e-6));
                out[5] = vec4(if cp.z > 0.0 { Vec3::new(0.5 + 0.5 * cp.x / (cp.z * tan), 0.5 + 0.5 * cp.y / (cp.z * tan), 0.0) } else { Vec3::ZERO });
                out[6] = vec4(reflect(ctx.i, ctx.n));
            }
            n_out = 7;
        }
        Op::ObjectInfo => {
            let o = object(ctx);
            out[0] = vec4(o.map_or(Vec3::ZERO, |o| o.location));
            out[1] = [1.0, 1.0, 1.0, 1.0];
            out[2] = f4(1.0);
            out[3] = f4(0.0);
            out[4] = f4(ctx.material_index);
            out[5] = f4(o.map_or(0.0, |o| o.random));
            n_out = 6;
        }
        Op::Attribute { uv } => {
            if *uv {
                let u = ctx.uv_at(var);
                out[0] = [u.x, u.y, u.z, 1.0];
                out[1] = vec4(u);
                out[2] = f4((u.x + u.y + u.z) / 3.0);
            }
            out[3] = f4(1.0);
            n_out = 4;
        }
        Op::VertexColor => {
            out[0] = [ctx.vcol[0], ctx.vcol[1], ctx.vcol[2], 1.0];
            out[1] = f4(ctx.vcol[3]);
            n_out = 2;
        }
        Op::TangentNode { axis } => {
            out[0] = vec4(match axis {
                Some(a) => radial_tangent(ctx, var, *a),
                None => ctx.tangent.map_or_else(|| radial_tangent(ctx, var, 2), |t| t.0),
            });
            n_out = 1;
        }
        Op::CameraData => {
            let c = camera_space(ctx, ctx.pos(var), true);
            out[0] = vec4(safe_norm(c));
            out[1] = f4(c.z);
            out[2] = f4(c.length());
            n_out = 3;
        }
        Op::Wireframe { pixel } => {
            let mut size = f(0) * 0.5;
            if *pixel {
                // Approximate a pixel's footprint at the hit distance.
                size *= ctx.ray_length.max(1e-4) * 2.0 * ctx.data.map_or(0.4, |s| s.tan_half) / 500.0;
            }
            let p = ctx.pos(var);
            let mut near = false;
            for k in 0..3 {
                let (a, b) = (ctx.tri[k], ctx.tri[(k + 1) % 3]);
                let ab = b - a;
                let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                if (a + ab * t - p).length() < size {
                    near = true;
                }
            }
            out[0] = f4(if near { 1.0 } else { 0.0 });
            n_out = 1;
        }
        Op::Ao { samples, inside } => {
            let (c, dist) = (r(0), f(1));
            let n = safe_norm(v(2));
            let mut ao = 1.0;
            if let (Some(scene), false) = (ctx.scene, ctx.world) {
                let n = if *inside { -n } else { n };
                let (t, b) = onb(n);
                let count = (*samples).clamp(1, 64);
                let mut hits = 0;
                let mut rng = crate::render::rng::Rng::new(ctx.seed as u64, 0xa0);
                let eps = 1e-4 * (1.0 + ctx.p.abs().max_elem());
                for _ in 0..count {
                    let d = crate::render::sampling::cosine_hemisphere(rng.f32(), rng.f32());
                    let w = t * d.x + b * d.y + n * d.z;
                    let tmax = if dist > 0.0 { dist } else { f32::INFINITY };
                    if scene.intersect_ray(ctx.p + ctx.ng * if *inside { -eps } else { eps }, w, tmax, 0.0, crate::render::prepare::VIS_SHADOW).is_some() {
                        hits += 1;
                    }
                }
                ao = 1.0 - hits as f32 / count as f32;
            }
            out[0] = [c[0] * ao, c[1] * ao, c[2] * ao, c[3]];
            out[1] = f4(ao);
            n_out = 2;
        }
    }
    let base = ins.out as usize;
    for (k, o) in out.iter().enumerate().take(n_out) {
        if let Some(slot) = regs.get_mut(base + k) {
            *slot = *o;
        }
    }
}

/// Run a list of instructions.
pub(crate) fn run(code: &[Instr], ctx: &ShadeCtx, regs: &mut [V4]) {
    for i in code {
        exec(i, ctx, regs);
    }
}

/// Bump-only displacement: replace the shading normal by the normal of the
/// surface displaced by the vectors in `d` (centre, dx, dy).
pub(crate) fn apply_displacement(ctx: &mut ShadeCtx, regs: &[V4], d: [Reg; 3]) {
    let get = |r: Reg| regs.get(r as usize).map_or(Vec3::ZERO, |x| v3(*x));
    let (dc, dx, dy) = (get(d[0]), get(d[1]), get(d[2]));
    if dc == Vec3::ZERO && dx == Vec3::ZERO && dy == Vec3::ZERO {
        return;
    }
    let ex = ctx.dpdx + dx - dc;
    let ey = ctx.dpdy + dy - dc;
    let n = safe_norm(ex.cross(ey));
    if n == Vec3::ZERO || !n.x.is_finite() {
        return;
    }
    ctx.n = if n.dot(ctx.n) < 0.0 { -n } else { n };
}

const MIN_WEIGHT: f32 = 1e-5;

fn tint_pow(c: Vec3, e: f32) -> Vec3 {
    Vec3::new(c.x.max(0.0).powf(e), c.y.max(0.0).powf(e), c.z.max(0.0).powf(e))
}

/// Facing unit normal from a register (falls back to the shading normal).
fn normal_in(ctx: &ShadeCtx, x: Vec3) -> Vec3 {
    let n = safe_norm(x);
    if n == Vec3::ZERO {
        ctx.n
    } else {
        n
    }
}

/// Walk the closure tree from `s` with weight `w`.
pub(crate) fn emit(prog: &Program, s: u32, w: Vec3, ctx: &ShadeCtx, regs: &[V4], out: &mut Closures, depth: u32) {
    if depth > 64 || w.max_elem() <= MIN_WEIGHT {
        return;
    }
    let Some(op) = prog.shaders.get(s as usize) else { return };
    match op {
        ShaderOp::Mix { fac, a, b } => {
            let t = regs.get(*fac as usize).map_or(0.0, |r| r[0]).clamp(0.0, 1.0);
            if let Some(a) = a {
                emit(prog, *a, w * (1.0 - t), ctx, regs, out, depth + 1);
            }
            if let Some(b) = b {
                emit(prog, *b, w * t, ctx, regs, out, depth + 1);
            }
        }
        ShaderOp::Add { a, b } => {
            for x in [a, b].into_iter().flatten() {
                emit(prog, *x, w, ctx, regs, out, depth + 1);
            }
        }
        ShaderOp::Closure { kind, ins } => closure(*kind, ins, w, ctx, regs, out),
    }
}

fn closure(kind: ClosureKind, ins: &[Reg], w: Vec3, ctx: &ShadeCtx, regs: &[V4], out: &mut Closures) {
    let r = |i: usize| read(regs, ins, i);
    let f = |i: usize| read(regs, ins, i)[0];
    let c = |i: usize| v3(read(regs, ins, i)).max(Vec3::ZERO);
    let nrm = |i: usize| normal_in(ctx, v3(read(regs, ins, i)));
    let wo = ctx.i;
    match kind {
        ClosureKind::Principled { ms } => principled(ins, w, ctx, regs, out, ms),
        ClosureKind::Diffuse => out.lobes.push(bsdf::diffuse(w.mul_elem(c(0)), nrm(2), f(1))),
        ClosureKind::Emission => {
            let e = w.mul_elem(c(0)) * f(1);
            if ctx.world {
                out.background += e;
            } else {
                out.emission += e;
            }
        }
        ClosureKind::Background => out.background += w.mul_elem(c(0)) * f(1),
        // Multiscatter GGX glass is rendered single-scatter (no energy
        // compensation for the combined reflect/refract lobe).
        ClosureKind::Glass { .. } | ClosureKind::Refraction { .. } => {
            let refract_only = matches!(kind, ClosureKind::Refraction { .. });
            let rough = f(1).clamp(0.0, 1.0);
            let ior = f(2).max(1e-5);
            let eta = if ctx.backfacing { 1.0 / ior } else { ior };
            let col = c(0);
            out.lobes.push(Lobe { weight: w, n: nrm(3), kind: LobeKind::Glass { alpha: rough * rough, eta, tint_r: col, tint_t: col, refract_only } });
        }
        ClosureKind::Glossy { ms } => {
            let n = nrm(4);
            let (ax, ay) = aniso_alphas(f(1), f(2));
            let t = rotate_tangent(n, safe_norm(v3(r(5))), f(3));
            let energy = if ms { ms_compensation((ax * ay).sqrt(), wo.dot(n).clamp(1e-4, 1.0), c(0)) } else { Vec3::ONE };
            out.lobes.push(Lobe { weight: w.mul_elem(c(0)), n, kind: LobeKind::Reflection { t, ax, ay, fresnel: Fresnel::Constant, film: None, energy } });
        }
        ClosureKind::Sheen => out.lobes.push(Lobe { weight: w.mul_elem(c(0)), n: nrm(2), kind: LobeKind::Sheen { r: f(1).clamp(0.0, 1.0) } }),
        ClosureKind::Translucent => out.lobes.push(Lobe { weight: w.mul_elem(c(0)), n: nrm(1), kind: LobeKind::Translucent }),
        ClosureKind::Transparent => out.transparent += w.mul_elem(c(0)),
        ClosureKind::Holdout => out.holdout += (w.x + w.y + w.z) / 3.0,
        ClosureKind::Toon { glossy } => {
            let size = f(1).clamp(0.0, 1.0);
            let smooth = f(2).clamp(0.0, 1.0);
            out.lobes.push(Lobe { weight: w.mul_elem(c(0)), n: nrm(3), kind: LobeKind::Toon { max_angle: size * PI * 0.5, smooth: smooth * PI * 0.5, glossy } });
        }
        ClosureKind::Subsurface => {
            // Approximation: subsurface scattering renders as diffuse.
            out.lobes.push(bsdf::diffuse(w.mul_elem(c(0)), nrm(6), 0.0));
        }
        ClosureKind::Hair => {
            // Approximation: a rough glossy lobe along the tangent.
            let n = ctx.n;
            let (ax, ay) = (f(2).clamp(0.05, 1.0).powi(2), f(3).clamp(0.05, 1.0).powi(2));
            let t = safe_norm(v3(r(4)));
            out.lobes.push(Lobe { weight: w.mul_elem(c(0)), n, kind: LobeKind::Reflection { t, ax, ay, fresnel: Fresnel::Constant, film: None, energy: Vec3::ONE } });
        }
        ClosureKind::HairPrincipled => {
            // Approximation: diffuse in the hair colour.
            out.lobes.push(bsdf::diffuse(w.mul_elem(c(0)), ctx.n, f(1)));
        }
        ClosureKind::VolumePrincipled => {
            let (col, density, g, absorb) = (c(0), f(1).max(0.0), f(2), c(3));
            let scatter = w.mul_elem(col) * density;
            // Cycles: absorption (1 - color)(1 - sqrt(absorption color)).
            let sq = Vec3::new(absorb.x.sqrt(), absorb.y.sqrt(), absorb.z.sqrt());
            let absorption = w.mul_elem((Vec3::ONE - col).max(Vec3::ZERO).mul_elem((Vec3::ONE - sq).max(Vec3::ZERO))) * density;
            add_volume(out, absorption + scatter, scatter, g);
            out.vol_emit += w.mul_elem(c(5)) * f(4);
            let bb = f(6);
            if bb > 0.0 {
                let t = f(8);
                if t > 0.0 {
                    let rgb = color::blackbody(t);
                    out.vol_emit += w.mul_elem(c(7)).mul_elem(Vec3::new(rgb[0], rgb[1], rgb[2])) * (bb * t.powi(4) * 5.670_374e-8 / PI * 1e-6);
                }
            }
        }
        ClosureKind::VolumeAbsorption => {
            let a = w.mul_elem(Vec3::ONE - c(0).min(Vec3::ONE)) * f(1).max(0.0);
            add_volume(out, a, Vec3::ZERO, 0.0);
        }
        ClosureKind::VolumeScatter => {
            let s = w.mul_elem(c(0)) * f(1).max(0.0);
            add_volume(out, s, s, f(2));
        }
    }
}

/// `absorb` here is the extinction added (absorption + scattering).
fn add_volume(out: &mut Closures, extinction: Vec3, scatter: Vec3, g: f32) {
    let ws = scatter.max_elem();
    let total = out.vol_scatter.max_elem() + ws;
    if total > 0.0 {
        out.vol_g = (out.vol_g * out.vol_scatter.max_elem() + g.clamp(-0.99, 0.99) * ws) / total;
    }
    out.vol_absorb += extinction - scatter;
    out.vol_scatter += scatter;
}

/// The anisotropy tangent `t` (or the default) rotated about `n` by
/// `rotation` turns.
fn rotate_tangent(n: Vec3, t: Vec3, rotation: f32) -> Vec3 {
    let t = if t == Vec3::ZERO { onb(n).0 } else { t };
    if rotation == 0.0 {
        return t;
    }
    ops::rotate_around_axis(t, n, rotation * TAU)
}

fn ior_from_f0(f0: f32) -> f32 {
    let s = f0.clamp(0.0, 0.999_99).sqrt();
    (1.0 + s) / (1.0 - s)
}

/// Inputs of the Principled BSDF in the order the compiler loads them.
pub(crate) const PRINCIPLED_INS: [&str; 29] = [
    "Base Color",
    "Metallic",
    "Roughness",
    "IOR",
    "Alpha",
    "Normal",
    "Subsurface Weight",
    "Subsurface Radius",
    "Subsurface Scale",
    "Subsurface IOR",
    "Subsurface Anisotropy",
    "Specular IOR Level",
    "Specular Tint",
    "Anisotropic",
    "Anisotropic Rotation",
    "Tangent",
    "Transmission Weight",
    "Coat Weight",
    "Coat Roughness",
    "Coat IOR",
    "Coat Tint",
    "Coat Normal",
    "Sheen Weight",
    "Sheen Roughness",
    "Sheen Tint",
    "Emission Color",
    "Emission Strength",
    "Thin Film Thickness",
    "Thin Film IOR",
];

/// The Principled BSDF as layered closures, in Cycles 4.x order: alpha,
/// sheen, coat, emission, metal, transmission, specular, subsurface/diffuse.
fn principled(ins: &[Reg], w: Vec3, ctx: &ShadeCtx, regs: &[V4], out: &mut Closures, ms: bool) {
    let f = |i: usize| read(regs, ins, i)[0];
    let c = |i: usize| v3(read(regs, ins, i)).max(Vec3::ZERO);
    let sat = |x: f32| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
    let wo = ctx.i;
    let base = c(0);
    let metallic = sat(f(1));
    let roughness = sat(f(2));
    let ior = if f(3).is_finite() { f(3).max(1e-5) } else { 1.5 };
    let alpha = sat(f(4));
    let n = normal_in(ctx, v3(read(regs, ins, 5)));
    let subsurface = sat(f(6));
    let spec_level = f(11).max(0.0);
    let spec_tint = c(12);
    let aniso = sat(f(13));
    let aniso_rot = f(14);
    let tangent = safe_norm(v3(read(regs, ins, 15)));
    let transmission = sat(f(16));
    let coat = f(17).max(0.0);
    let coat_rough = sat(f(18));
    let coat_ior = f(19).max(1.0);
    let coat_tint = c(20);
    let coat_n = normal_in(ctx, v3(read(regs, ins, 21)));
    let sheen = f(22).max(0.0);
    let sheen_rough = sat(f(23));
    let sheen_tint = c(24);
    let emission = c(25) * f(26).max(0.0);
    let film_thickness = f(27).max(0.0);
    let film_ior = f(28).max(1.0);
    let film = (film_thickness > 1e-5).then_some(ThinFilm { thickness: film_thickness, ior: film_ior });

    let mut w = w;
    if alpha < 1.0 {
        out.transparent += w * (1.0 - alpha);
        w = w * alpha;
    }
    if w.max_elem() <= MIN_WEIGHT {
        return;
    }
    // Sheen layer on top.
    if sheen > MIN_WEIGHT {
        let sw = w.mul_elem(sheen_tint) * sheen;
        let lobe = Lobe { weight: sw, n, kind: LobeKind::Sheen { r: sheen_rough } };
        let albedo = lobe.albedo(wo).mul_elem(sw);
        out.lobes.push(lobe);
        let a = (albedo.max_elem() / w.max_elem().max(1e-8)).clamp(0.0, 1.0);
        w = w * (1.0 - a);
    }
    // Coat.
    if coat > MIN_WEIGHT {
        let eta = if ctx.backfacing { 1.0 / coat_ior } else { coat_ior };
        let ca = (coat_rough * coat_rough).max(1e-3);
        let lobe = Lobe { weight: w * coat, n: coat_n, kind: LobeKind::Reflection { t: Vec3::ZERO, ax: ca, ay: ca, fresnel: Fresnel::Dielectric { eta, tint: Vec3::ONE }, film: None, energy: Vec3::ONE } };
        let albedo = dielectric_albedo(coat_n.dot(wo).clamp(1e-4, 1.0), ca, eta);
        out.lobes.push(lobe);
        w = w * (1.0 - (coat * albedo).clamp(0.0, 1.0));
        if coat_tint != Vec3::ONE {
            let cos_i = coat_n.dot(wo).clamp(0.0, 1.0);
            let s = 1.0 / coat_ior;
            let cos_nt = (1.0 - s * s * (1.0 - cos_i * cos_i)).max(1e-4).sqrt();
            let t = tint_pow(coat_tint, 1.0 / cos_nt);
            w = w.mul_elem(Vec3::ONE.lerp(t, coat.min(1.0)));
        }
    }
    // Emission sits under sheen and coat.
    if emission != Vec3::ZERO {
        out.emission += w.mul_elem(emission);
    }
    let (ax, ay) = aniso_alphas(roughness, aniso);
    let t = if aniso > 0.0 { rotate_tangent(n, tangent, aniso_rot) } else { Vec3::ZERO };
    let mu = n.dot(wo).clamp(1e-4, 1.0);
    let alpha_avg = (ax * ay).sqrt();
    // Metal (F82-tint conductor).
    if metallic > 0.0 {
        let f0 = base.min(Vec3::ONE);
        let b = f82_tint_b(f0, spec_tint.min(Vec3::ONE));
        let f_avg = f0 + (Vec3::ONE - f0) * (1.0 / 21.0);
        let energy = if ms { ms_compensation(alpha_avg, mu, f_avg) } else { Vec3::ONE };
        out.lobes.push(Lobe { weight: w * metallic, n, kind: LobeKind::Reflection { t, ax, ay, fresnel: Fresnel::Conductor { f0, b }, film, energy } });
        w = w * (1.0 - metallic);
    }
    // The dielectric's IOR, with Specular IOR Level applied through F0.
    let mut eta = ior;
    if (spec_level - 0.5).abs() > 1e-6 {
        let f0 = ((ior - 1.0) / (ior + 1.0)).powi(2) * 2.0 * spec_level;
        eta = ior_from_f0(f0);
        if ior < 1.0 {
            eta = 1.0 / eta;
        }
    }
    let eta_rel = if ctx.backfacing { 1.0 / eta } else { eta };
    // Transmission.
    if transmission > 0.0 && w.max_elem() > MIN_WEIGHT {
        let ga = roughness * roughness;
        out.lobes.push(Lobe { weight: w * transmission, n, kind: LobeKind::Glass { alpha: ga, eta: eta_rel, tint_r: spec_tint, tint_t: base, refract_only: false } });
        w = w * (1.0 - transmission);
    }
    if w.max_elem() <= MIN_WEIGHT {
        return;
    }
    // Specular layer over the diffuse base.
    if (eta_rel - 1.0).abs() > 1e-5 || film.is_some() {
        let f_avg = spec_tint * fresnel_avg(eta_rel);
        let energy = if ms { ms_compensation(alpha_avg, mu, f_avg) } else { Vec3::ONE };
        let lobe = Lobe { weight: w, n, kind: LobeKind::Reflection { t, ax, ay, fresnel: Fresnel::Dielectric { eta: eta_rel, tint: spec_tint }, film, energy } };
        let albedo = lobe.albedo(wo);
        out.lobes.push(lobe);
        w = w * (1.0 - albedo.max_elem().clamp(0.0, 1.0));
    }
    // Subsurface (approximated as diffuse) and diffuse.
    let base_w = w.mul_elem(base);
    if base_w.max_elem() > MIN_WEIGHT {
        out.lobes.push(bsdf::diffuse(base_w, n, 0.0));
    }
    let _ = subsurface;
}
