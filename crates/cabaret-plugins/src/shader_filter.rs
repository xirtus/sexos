//! Pure-Rust port of **obs-shaderfilter** with a safe, fuel-bounded procedural shader AST
//! evaluator (`ShaderExpr`) and 12 built-in studio preset shaders operating directly on
//! [`cabaret_core::compositor::FrameBuffer`].

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use core::f32::consts::PI;

use cabaret_core::compositor::{sample_linear, CompositorError, FrameBuffer, LinearRgba, Scaler};

use crate::util::{
    absf, atan2f, clamp01, cosf, deg_to_rad, floorf, fract, hash2d, lerp, powf, rec709_luma, sinf,
    smoothstep, sqrtf,
};

/// Default instruction/node fuel limit per shader frame invocation.
pub const DEFAULT_SHADER_FUEL: u64 = 20_000_000;

/// Maximum AST recursion depth to guarantee stack safety.
pub const MAX_SHADER_AST_DEPTH: usize = 64;

/// Errors returned by [`eval_shader_program`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaderError {
    FuelExhausted { used: u64, limit: u64 },
    AstDepthExceeded,
    UnknownParam(String),
    Compositor(CompositorError),
}

impl From<CompositorError> for ShaderError {
    fn from(e: CompositorError) -> Self {
        Self::Compositor(e)
    }
}

/// Value produced by evaluating a [`ShaderExpr`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderVal {
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
}

impl ShaderVal {
    #[inline]
    pub fn as_f32(self) -> f32 {
        match self {
            Self::Float(v) => v,
            Self::Vec2(v) => v[0],
            Self::Vec3(v) => v[0],
            Self::Vec4(v) => v[0],
        }
    }

    #[inline]
    pub fn as_vec4(self) -> [f32; 4] {
        match self {
            Self::Float(x) => [x, x, x, 1.0],
            Self::Vec2(v) => [v[0], v[1], 0.0, 1.0],
            Self::Vec3(v) => [v[0], v[1], v[2], 1.0],
            Self::Vec4(v) => v,
        }
    }

    #[inline]
    fn lanes(self) -> usize {
        match self {
            Self::Float(_) => 1,
            Self::Vec2(_) => 2,
            Self::Vec3(_) => 3,
            Self::Vec4(_) => 4,
        }
    }

    #[inline]
    fn to_array(self) -> [f32; 4] {
        match self {
            Self::Float(x) => [x, x, x, x],
            Self::Vec2(v) => [v[0], v[1], 0.0, 0.0],
            Self::Vec3(v) => [v[0], v[1], v[2], 0.0],
            Self::Vec4(v) => v,
        }
    }

    #[inline]
    fn from_lanes(lanes: usize, arr: [f32; 4]) -> Self {
        match lanes {
            1 => Self::Float(arr[0]),
            2 => Self::Vec2([arr[0], arr[1]]),
            3 => Self::Vec3([arr[0], arr[1], arr[2]]),
            _ => Self::Vec4(arr),
        }
    }

    #[inline]
    fn map_unary(self, f: impl Fn(f32) -> f32) -> Self {
        match self {
            Self::Float(x) => Self::Float(f(x)),
            Self::Vec2(v) => Self::Vec2([f(v[0]), f(v[1])]),
            Self::Vec3(v) => Self::Vec3([f(v[0]), f(v[1]), f(v[2])]),
            Self::Vec4(v) => Self::Vec4([f(v[0]), f(v[1]), f(v[2]), f(v[3])]),
        }
    }

    #[inline]
    fn map_binary(self, rhs: Self, f: impl Fn(f32, f32) -> f32) -> Self {
        let lanes = self.lanes().max(rhs.lanes());
        let a = self.to_array();
        let b = rhs.to_array();
        Self::from_lanes(lanes, [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])])
    }
}

/// Safe procedural shader AST over `uv`, `time`, `pixel`, `audio_rms`, and source sampling.
#[derive(Clone, Debug, PartialEq)]
pub enum ShaderExpr {
    Lit(f32),
    Vec2(Box<ShaderExpr>, Box<ShaderExpr>),
    Vec3(Box<ShaderExpr>, Box<ShaderExpr>, Box<ShaderExpr>),
    Vec4(
        Box<ShaderExpr>,
        Box<ShaderExpr>,
        Box<ShaderExpr>,
        Box<ShaderExpr>,
    ),
    /// Normalized pixel coordinates `[u, v]` in `[0..1]^2`.
    Uv,
    /// Elapsed shader time in seconds.
    Time,
    /// Current input linear RGBA pixel `[r, g, b, a]`.
    Pixel,
    /// Live audio RMS level in `[0..1+]`.
    AudioRms,
    /// Named uniform parameter (`UnknownParam` if absent).
    Param(String),
    /// Samples the input frame buffer at `(u, v)`.
    Sample(Box<ShaderExpr>, Box<ShaderExpr>),
    /// Extracts component `0..=3` (`r/x, g/y, b/z, a/w`).
    Channel(Box<ShaderExpr>, u8),
    Neg(Box<ShaderExpr>),
    Abs(Box<ShaderExpr>),
    Sin(Box<ShaderExpr>),
    Cos(Box<ShaderExpr>),
    Floor(Box<ShaderExpr>),
    Fract(Box<ShaderExpr>),
    Sqrt(Box<ShaderExpr>),
    Length(Box<ShaderExpr>),
    Add(Box<ShaderExpr>, Box<ShaderExpr>),
    Sub(Box<ShaderExpr>, Box<ShaderExpr>),
    Mul(Box<ShaderExpr>, Box<ShaderExpr>),
    Div(Box<ShaderExpr>, Box<ShaderExpr>),
    Mod(Box<ShaderExpr>, Box<ShaderExpr>),
    Min(Box<ShaderExpr>, Box<ShaderExpr>),
    Max(Box<ShaderExpr>, Box<ShaderExpr>),
    Pow(Box<ShaderExpr>, Box<ShaderExpr>),
    Dot(Box<ShaderExpr>, Box<ShaderExpr>),
    Step(Box<ShaderExpr>, Box<ShaderExpr>),
    Clamp {
        val: Box<ShaderExpr>,
        min: Box<ShaderExpr>,
        max: Box<ShaderExpr>,
    },
    Lerp {
        a: Box<ShaderExpr>,
        b: Box<ShaderExpr>,
        t: Box<ShaderExpr>,
    },
    Smoothstep {
        edge0: Box<ShaderExpr>,
        edge1: Box<ShaderExpr>,
        x: Box<ShaderExpr>,
    },
    Select {
        cond_gt_zero: Box<ShaderExpr>,
        if_true: Box<ShaderExpr>,
        if_false: Box<ShaderExpr>,
    },
    /// Bounded loop running `count` iterations, updating accumulator via `body`.
    Repeat {
        count: u32,
        init: Box<ShaderExpr>,
        body: Box<ShaderExpr>,
    },
    /// Current accumulator value inside [`ShaderExpr::Repeat`].
    LoopAcc,
    /// Current 0-based loop iteration index inside [`ShaderExpr::Repeat`].
    LoopIndex,
}

/// Complete shader AST program + fuel budget.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderProgram {
    pub name: String,
    pub root: ShaderExpr,
    pub max_fuel: u64,
}

impl ShaderProgram {
    pub fn new(name: impl Into<String>, root: ShaderExpr) -> Self {
        Self {
            name: name.into(),
            root,
            max_fuel: DEFAULT_SHADER_FUEL,
        }
    }
}

/// Uniform inputs passed into [`eval_shader_program`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderUniforms {
    pub time_s: f32,
    pub audio_rms: f32,
    pub params: BTreeMap<String, f32>,
}

/// Result of running a fuel-metered [`ShaderProgram`].
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderEvalOutput {
    pub frame: FrameBuffer,
    pub fuel_consumed: u64,
}

struct EvalEnv<'a> {
    input: &'a FrameBuffer,
    uniforms: &'a ShaderUniforms,
    uv: [f32; 2],
    pixel: LinearRgba,
    loop_acc: ShaderVal,
    loop_idx: f32,
    fuel_remaining: u64,
    fuel_limit: u64,
}

impl<'a> EvalEnv<'a> {
    #[inline]
    fn spend(&mut self) -> Result<(), ShaderError> {
        if self.fuel_remaining == 0 {
            return Err(ShaderError::FuelExhausted {
                used: self.fuel_limit,
                limit: self.fuel_limit,
            });
        }
        self.fuel_remaining -= 1;
        Ok(())
    }

    fn eval(&mut self, expr: &ShaderExpr, depth: usize) -> Result<ShaderVal, ShaderError> {
        if depth > MAX_SHADER_AST_DEPTH {
            return Err(ShaderError::AstDepthExceeded);
        }
        self.spend()?;

        match expr {
            ShaderExpr::Lit(v) => Ok(ShaderVal::Float(*v)),
            ShaderExpr::Vec2(x, y) => {
                let vx = self.eval(x, depth + 1)?.as_f32();
                let vy = self.eval(y, depth + 1)?.as_f32();
                Ok(ShaderVal::Vec2([vx, vy]))
            }
            ShaderExpr::Vec3(x, y, z) => {
                let vx = self.eval(x, depth + 1)?.as_f32();
                let vy = self.eval(y, depth + 1)?.as_f32();
                let vz = self.eval(z, depth + 1)?.as_f32();
                Ok(ShaderVal::Vec3([vx, vy, vz]))
            }
            ShaderExpr::Vec4(r, g, b, a) => {
                let vr = self.eval(r, depth + 1)?.as_f32();
                let vg = self.eval(g, depth + 1)?.as_f32();
                let vb = self.eval(b, depth + 1)?.as_f32();
                let va = self.eval(a, depth + 1)?.as_f32();
                Ok(ShaderVal::Vec4([vr, vg, vb, va]))
            }
            ShaderExpr::Uv => Ok(ShaderVal::Vec2(self.uv)),
            ShaderExpr::Time => Ok(ShaderVal::Float(self.uniforms.time_s)),
            ShaderExpr::Pixel => Ok(ShaderVal::Vec4(self.pixel)),
            ShaderExpr::AudioRms => Ok(ShaderVal::Float(self.uniforms.audio_rms)),
            ShaderExpr::Param(name) => self
                .uniforms
                .params
                .get(name)
                .copied()
                .map(ShaderVal::Float)
                .ok_or_else(|| ShaderError::UnknownParam(name.clone())),
            ShaderExpr::Sample(u_e, v_e) => {
                let u = clamp01(self.eval(u_e, depth + 1)?.as_f32());
                let v = clamp01(self.eval(v_e, depth + 1)?.as_f32());
                let sx = u * self.input.width as f32;
                let sy = v * self.input.height as f32;
                Ok(ShaderVal::Vec4(sample_linear(
                    self.input,
                    sx,
                    sy,
                    1.0,
                    1.0,
                    Scaler::Bilinear,
                )))
            }
            ShaderExpr::Channel(e, ch) => {
                let arr = self.eval(e, depth + 1)?.as_vec4();
                let idx = (*ch as usize).min(3);
                Ok(ShaderVal::Float(arr[idx]))
            }
            ShaderExpr::Neg(e) => Ok(self.eval(e, depth + 1)?.map_unary(|x| -x)),
            ShaderExpr::Abs(e) => Ok(self.eval(e, depth + 1)?.map_unary(absf)),
            ShaderExpr::Sin(e) => Ok(self.eval(e, depth + 1)?.map_unary(sinf)),
            ShaderExpr::Cos(e) => Ok(self.eval(e, depth + 1)?.map_unary(cosf)),
            ShaderExpr::Floor(e) => Ok(self.eval(e, depth + 1)?.map_unary(floorf)),
            ShaderExpr::Fract(e) => Ok(self.eval(e, depth + 1)?.map_unary(fract)),
            ShaderExpr::Sqrt(e) => Ok(self.eval(e, depth + 1)?.map_unary(sqrtf)),
            ShaderExpr::Length(e) => {
                let v = self.eval(e, depth + 1)?;
                let sum2 = match v {
                    ShaderVal::Float(x) => x * x,
                    ShaderVal::Vec2(a) => a[0] * a[0] + a[1] * a[1],
                    ShaderVal::Vec3(a) => a[0] * a[0] + a[1] * a[1] + a[2] * a[2],
                    ShaderVal::Vec4(a) => a[0] * a[0] + a[1] * a[1] + a[2] * a[2] + a[3] * a[3],
                };
                Ok(ShaderVal::Float(sqrtf(sum2)))
            }
            ShaderExpr::Add(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, |x, y| x + y))
            }
            ShaderExpr::Sub(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, |x, y| x - y))
            }
            ShaderExpr::Mul(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, |x, y| x * y))
            }
            ShaderExpr::Div(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, |x, y| if absf(y) < 1e-7 { 0.0 } else { x / y }))
            }
            ShaderExpr::Mod(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, |x, y| if absf(y) < 1e-7 { 0.0 } else { x - y * floorf(x / y) }))
            }
            ShaderExpr::Min(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, f32::min))
            }
            ShaderExpr::Max(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, f32::max))
            }
            ShaderExpr::Pow(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                Ok(va.map_binary(vb, |x, y| powf(x.max(0.0), y)))
            }
            ShaderExpr::Dot(a, b) => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                let lanes = va.lanes().max(vb.lanes());
                let aa = va.to_array();
                let bb = vb.to_array();
                let mut sum = 0.0_f32;
                for i in 0..lanes {
                    sum += aa[i] * bb[i];
                }
                Ok(ShaderVal::Float(sum))
            }
            ShaderExpr::Step(edge, x) => {
                let ve = self.eval(edge, depth + 1)?;
                let vx = self.eval(x, depth + 1)?;
                Ok(ve.map_binary(vx, |e, v| if v < e { 0.0 } else { 1.0 }))
            }
            ShaderExpr::Clamp { val, min, max } => {
                let vv = self.eval(val, depth + 1)?;
                let vmin = self.eval(min, depth + 1)?.as_f32();
                let vmax = self.eval(max, depth + 1)?.as_f32().max(vmin);
                Ok(vv.map_unary(|x| x.clamp(vmin, vmax)))
            }
            ShaderExpr::Lerp { a, b, t } => {
                let va = self.eval(a, depth + 1)?;
                let vb = self.eval(b, depth + 1)?;
                let vt = self.eval(t, depth + 1)?.as_f32();
                Ok(va.map_binary(vb, |x, y| lerp(x, y, vt)))
            }
            ShaderExpr::Smoothstep { edge0, edge1, x } => {
                let e0 = self.eval(edge0, depth + 1)?.as_f32();
                let e1 = self.eval(edge1, depth + 1)?.as_f32();
                let vx = self.eval(x, depth + 1)?;
                Ok(vx.map_unary(|v| smoothstep(e0, e1, v)))
            }
            ShaderExpr::Select {
                cond_gt_zero,
                if_true,
                if_false,
            } => {
                let c = self.eval(cond_gt_zero, depth + 1)?.as_f32();
                if c > 0.0 {
                    self.eval(if_true, depth + 1)
                } else {
                    self.eval(if_false, depth + 1)
                }
            }
            ShaderExpr::Repeat { count, init, body } => {
                let saved_acc = self.loop_acc;
                let saved_idx = self.loop_idx;
                let mut acc = self.eval(init, depth + 1)?;
                for idx in 0..*count {
                    self.spend()?;
                    self.loop_acc = acc;
                    self.loop_idx = idx as f32;
                    acc = self.eval(body, depth + 1)?;
                }
                self.loop_acc = saved_acc;
                self.loop_idx = saved_idx;
                Ok(acc)
            }
            ShaderExpr::LoopAcc => Ok(self.loop_acc),
            ShaderExpr::LoopIndex => Ok(ShaderVal::Float(self.loop_idx)),
        }
    }
}

/// Evaluates a procedural [`ShaderProgram`] per pixel over `input`, strictly enforcing `max_fuel`.
pub fn eval_shader_program(
    program: &ShaderProgram,
    input: &FrameBuffer,
    uniforms: &ShaderUniforms,
) -> Result<ShaderEvalOutput, ShaderError> {
    let mut out = FrameBuffer::new(input.width, input.height)?;
    let w = input.width as f32;
    let h = input.height as f32;

    let mut env = EvalEnv {
        input,
        uniforms,
        uv: [0.0, 0.0],
        pixel: [0.0; 4],
        loop_acc: ShaderVal::Float(0.0),
        loop_idx: 0.0,
        fuel_remaining: program.max_fuel,
        fuel_limit: program.max_fuel,
    };

    for y in 0..input.height {
        let v = (y as f32 + 0.5) / h;
        for x in 0..input.width {
            let u = (x as f32 + 0.5) / w;
            let px = input.get_pixel(x, y).unwrap_or([0.0; 4]);
            env.uv = [u, v];
            env.pixel = px;
            let val = env.eval(&program.root, 0)?.as_vec4();
            out.set_pixel(
                x,
                y,
                [
                    val[0].max(0.0),
                    val[1].max(0.0),
                    val[2].max(0.0),
                    clamp01(val[3]),
                ],
            );
        }
    }

    Ok(ShaderEvalOutput {
        frame: out,
        fuel_consumed: program.max_fuel.saturating_sub(env.fuel_remaining),
    })
}

/// All 12 built-in studio preset shaders for `obs-shaderfilter`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderPreset {
    CrtScanline {
        scanline_density: f32,
        scanline_intensity: f32,
        curvature: f32,
        phosphor_mask: f32,
    },
    ChromaticAberration {
        offset_px: f32,
        radial: bool,
    },
    GlitchBlock {
        block_size_px: f32,
        intensity: f32,
        seed: u32,
    },
    Halftone {
        cell_size_px: f32,
        angle_deg: f32,
        contrast: f32,
    },
    FrostedGlass {
        radius_px: f32,
        jitter: f32,
    },
    Pixelate {
        pixel_size: u32,
    },
    Vignette {
        radius: f32,
        softness: f32,
        strength: f32,
    },
    FilmGrain {
        amount: f32,
        grain_size: f32,
        luma_response: f32,
    },
    EdgeGlow {
        threshold: f32,
        intensity: f32,
        glow_color: LinearRgba,
    },
    KaleidoscopeMirror {
        segments: u8,
        rotation_deg: f32,
        zoom: f32,
    },
    ThermalVision {
        contrast: f32,
        palette_shift: f32,
    },
    VhsTracking {
        wobble_px: f32,
        band_y: f32,
        noise_amount: f32,
        chroma_bleed_px: f32,
    },
}

impl ShaderPreset {
    /// Returns canonical default instances of all 12 built-in studio presets.
    pub fn all_defaults() -> [Self; 12] {
        [
            Self::CrtScanline {
                scanline_density: 240.0,
                scanline_intensity: 0.35,
                curvature: 0.12,
                phosphor_mask: 0.2,
            },
            Self::ChromaticAberration {
                offset_px: 4.0,
                radial: true,
            },
            Self::GlitchBlock {
                block_size_px: 16.0,
                intensity: 0.5,
                seed: 1337,
            },
            Self::Halftone {
                cell_size_px: 8.0,
                angle_deg: 30.0,
                contrast: 1.2,
            },
            Self::FrostedGlass {
                radius_px: 4.0,
                jitter: 0.75,
            },
            Self::Pixelate { pixel_size: 8 },
            Self::Vignette {
                radius: 0.72,
                softness: 0.45,
                strength: 0.65,
            },
            Self::FilmGrain {
                amount: 0.12,
                grain_size: 1.5,
                luma_response: 0.5,
            },
            Self::EdgeGlow {
                threshold: 0.15,
                intensity: 1.4,
                glow_color: [0.1, 0.85, 1.0, 1.0],
            },
            Self::KaleidoscopeMirror {
                segments: 6,
                rotation_deg: 0.0,
                zoom: 1.0,
            },
            Self::ThermalVision {
                contrast: 1.2,
                palette_shift: 0.0,
            },
            Self::VhsTracking {
                wobble_px: 6.0,
                band_y: 0.78,
                noise_amount: 0.25,
                chroma_bleed_px: 3.0,
            },
        ]
    }
}

#[inline]
fn sample_uv_bilinear(fb: &FrameBuffer, u: f32, v: f32) -> LinearRgba {
    sample_linear(
        fb,
        clamp01(u) * fb.width as f32,
        clamp01(v) * fb.height as f32,
        1.0,
        1.0,
        Scaler::Bilinear,
    )
}

/// Applies one of the 12 built-in studio preset shaders directly to a [`FrameBuffer`].
pub fn apply_shader_preset(
    input: &FrameBuffer,
    preset: &ShaderPreset,
    time_s: f32,
    audio_rms: f32,
) -> Result<FrameBuffer, ShaderError> {
    let w = input.width;
    let h = input.height;
    let mut out = FrameBuffer::new(w, h)?;
    let wf = w as f32;
    let hf = h as f32;
    let audio_boost = 1.0 + clamp01(audio_rms) * 0.5;

    match *preset {
        ShaderPreset::CrtScanline {
            scanline_density,
            scanline_intensity,
            curvature,
            phosphor_mask,
        } => {
            let curv = curvature.max(0.0);
            let intens = clamp01(scanline_intensity);
            let mask_amt = clamp01(phosphor_mask);
            let dens = scanline_density.max(8.0);
            for y in 0..h {
                let v = (y as f32 + 0.5) / hf;
                for x in 0..w {
                    let u = (x as f32 + 0.5) / wf;
                    let cu = u * 2.0 - 1.0;
                    let cv = v * 2.0 - 1.0;
                    let r2 = cu * cu + cv * cv;
                    let wu = (cu * (1.0 + curv * r2)) * 0.5 + 0.5;
                    let wv = (cv * (1.0 + curv * r2)) * 0.5 + 0.5;
                    if wu < 0.0 || wu > 1.0 || wv < 0.0 || wv > 1.0 {
                        out.set_pixel(x, y, [0.0, 0.0, 0.0, 1.0]);
                        continue;
                    }
                    let mut p = sample_uv_bilinear(input, wu, wv);
                    let scan = 0.5 + 0.5 * sinf(wv * dens * PI * 2.0);
                    let scan_mul = 1.0 - intens * (1.0 - scan);
                    let triad = x % 3;
                    let m = [
                        if triad == 0 { 1.0 + mask_amt } else { 1.0 - mask_amt * 0.5 },
                        if triad == 1 { 1.0 + mask_amt } else { 1.0 - mask_amt * 0.5 },
                        if triad == 2 { 1.0 + mask_amt } else { 1.0 - mask_amt * 0.5 },
                    ];
                    p[0] = (p[0] * scan_mul * m[0]).max(0.0);
                    p[1] = (p[1] * scan_mul * m[1]).max(0.0);
                    p[2] = (p[2] * scan_mul * m[2]).max(0.0);
                    out.set_pixel(x, y, p);
                }
            }
        }
        ShaderPreset::ChromaticAberration { offset_px, radial } => {
            let off = offset_px * audio_boost;
            for y in 0..h {
                let v = (y as f32 + 0.5) / hf;
                for x in 0..w {
                    let u = (x as f32 + 0.5) / wf;
                    let (du, dv) = if radial {
                        ((u - 0.5) * 2.0 * (off / wf), (v - 0.5) * 2.0 * (off / hf))
                    } else {
                        (off / wf, 0.0)
                    };
                    let pr = sample_uv_bilinear(input, u + du, v + dv);
                    let pg = sample_uv_bilinear(input, u, v);
                    let pb = sample_uv_bilinear(input, u - du, v - dv);
                    out.set_pixel(x, y, [pr[0], pg[1], pb[2], pg[3]]);
                }
            }
        }
        ShaderPreset::GlitchBlock {
            block_size_px,
            intensity,
            seed,
        } => {
            let bs = block_size_px.max(2.0);
            let t_tick = floorf(time_s * 12.0) as i32;
            let intens = clamp01(intensity) * audio_boost;
            for y in 0..h {
                let v = (y as f32 + 0.5) / hf;
                let by = floorf(y as f32 / bs) as i32;
                for x in 0..w {
                    let u = (x as f32 + 0.5) / wf;
                    let bx = floorf(x as f32 / bs) as i32;
                    let hval = hash2d(bx ^ seed as i32, by.wrapping_add(t_tick * 31));
                    let active = hval > (1.0 - intens * 0.45);
                    if active {
                        let shift_u = (hash2d(by + t_tick, seed as i32) - 0.5) * 0.18 * intens;
                        let pr = sample_uv_bilinear(input, fract(u + shift_u + 0.015), v);
                        let pg = sample_uv_bilinear(input, fract(u + shift_u), v);
                        let pb = sample_uv_bilinear(input, fract(u + shift_u - 0.015), v);
                        out.set_pixel(x, y, [pr[0], pg[1], pb[2], pg[3]]);
                    } else {
                        out.set_pixel(x, y, input.get_pixel(x, y).unwrap_or([0.0; 4]));
                    }
                }
            }
        }
        ShaderPreset::Halftone {
            cell_size_px,
            angle_deg,
            contrast,
        } => {
            let cell = cell_size_px.max(2.0);
            let rad = deg_to_rad(angle_deg);
            let c = cosf(rad);
            let s = sinf(rad);
            let cont = contrast.max(0.2);
            for y in 0..h {
                for x in 0..w {
                    let px = input.get_pixel(x, y).unwrap_or([0.0; 4]);
                    let rx = (x as f32) * c - (y as f32) * s;
                    let ry = (x as f32) * s + (y as f32) * c;
                    let gx = fract(rx / cell) - 0.5;
                    let gy = fract(ry / cell) - 0.5;
                    let dist = sqrtf(gx * gx + gy * gy) * 1.414_213_5;
                    let luma = clamp01((rec709_luma([px[0], px[1], px[2]]) - 0.5) * cont + 0.5);
                    let dot_r = sqrtf(luma);
                    let mask = smoothstep(dot_r - 0.08, dot_r + 0.08, dist);
                    let k = 1.0 - mask;
                    out.set_pixel(x, y, [px[0] * k, px[1] * k, px[2] * k, px[3]]);
                }
            }
        }
        ShaderPreset::FrostedGlass { radius_px, jitter } => {
            let rad = radius_px.max(0.5);
            let jit = clamp01(jitter);
            let offsets = [
                [-0.7, -0.7],
                [0.7, -0.7],
                [-0.7, 0.7],
                [0.7, 0.7],
                [0.0, 0.0],
            ];
            for y in 0..h {
                for x in 0..w {
                    let n1 = hash2d(x as i32, y as i32) - 0.5;
                    let n2 = hash2d((x as i32).wrapping_add(19), (y as i32).wrapping_add(73)) - 0.5;
                    let mut acc = [0.0_f32; 4];
                    for &[ox, oy] in &offsets {
                        let sx = x as f32 + 0.5 + (ox * rad + n1 * rad * jit * 2.0);
                        let sy = y as f32 + 0.5 + (oy * rad + n2 * rad * jit * 2.0);
                        let p = sample_linear(input, sx, sy, 1.0, 1.0, Scaler::Bilinear);
                        for ch in 0..4 {
                            acc[ch] += p[ch];
                        }
                    }
                    let inv = 1.0 / offsets.len() as f32;
                    out.set_pixel(x, y, [acc[0] * inv, acc[1] * inv, acc[2] * inv, acc[3] * inv]);
                }
            }
        }
        ShaderPreset::Pixelate { pixel_size } => {
            let ps = pixel_size.max(1);
            for y in 0..h {
                let by = (y / ps) * ps + (ps / 2).min(h - 1 - (y / ps) * ps);
                for x in 0..w {
                    let bx = (x / ps) * ps + (ps / 2).min(w - 1 - (x / ps) * ps);
                    let p = input.get_pixel(bx.min(w - 1), by.min(h - 1)).unwrap_or([0.0; 4]);
                    out.set_pixel(x, y, p);
                }
            }
        }
        ShaderPreset::Vignette {
            radius,
            softness,
            strength,
        } => {
            let rad = radius.max(0.05);
            let soft = softness.max(0.01);
            let str_k = clamp01(strength);
            for y in 0..h {
                let ny = (y as f32 + 0.5) / hf - 0.5;
                for x in 0..w {
                    let nx = (x as f32 + 0.5) / wf - 0.5;
                    let d = sqrtf(nx * nx + ny * ny) * 2.0;
                    let vig = 1.0 - str_k * smoothstep(rad - soft * 0.5, rad + soft * 0.5, d);
                    let mut p = input.get_pixel(x, y).unwrap_or([0.0; 4]);
                    p[0] *= vig;
                    p[1] *= vig;
                    p[2] *= vig;
                    out.set_pixel(x, y, p);
                }
            }
        }
        ShaderPreset::FilmGrain {
            amount,
            grain_size,
            luma_response,
        } => {
            let gs = grain_size.max(1.0);
            let amt = amount.max(0.0) * audio_boost;
            let lr = clamp01(luma_response);
            let frame_seed = floorf(time_s * 24.0) as i32;
            for y in 0..h {
                let gy = floorf(y as f32 / gs) as i32;
                for x in 0..w {
                    let gx = floorf(x as f32 / gs) as i32;
                    let mut p = input.get_pixel(x, y).unwrap_or([0.0; 4]);
                    let n = (hash2d(gx.wrapping_add(frame_seed * 131), gy.wrapping_add(frame_seed * 17)) - 0.5) * 2.0;
                    let y_val = clamp01(rec709_luma([p[0], p[1], p[2]]));
                    let weight = lerp(1.0, 1.0 - 0.6 * absf(y_val - 0.5) * 2.0, lr);
                    let delta = n * amt * weight;
                    p[0] = (p[0] + delta).max(0.0);
                    p[1] = (p[1] + delta).max(0.0);
                    p[2] = (p[2] + delta).max(0.0);
                    out.set_pixel(x, y, p);
                }
            }
        }
        ShaderPreset::EdgeGlow {
            threshold,
            intensity,
            glow_color,
        } => {
            let thresh = threshold.max(0.0);
            let intens = intensity.max(0.0) * audio_boost;
            let luma_at = |ix: i32, iy: i32| -> f32 {
                let p = input.sample_clamped(ix, iy);
                rec709_luma([p[0], p[1], p[2]])
            };
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let tl = luma_at(x - 1, y - 1);
                    let t = luma_at(x, y - 1);
                    let tr = luma_at(x + 1, y - 1);
                    let l = luma_at(x - 1, y);
                    let r = luma_at(x + 1, y);
                    let bl = luma_at(x - 1, y + 1);
                    let b = luma_at(x, y + 1);
                    let br = luma_at(x + 1, y + 1);

                    let gx = (tr + 2.0 * r + br) - (tl + 2.0 * l + bl);
                    let gy = (bl + 2.0 * b + br) - (tl + 2.0 * t + tr);
                    let mag = sqrtf(gx * gx + gy * gy);
                    let edge = smoothstep(thresh, thresh + 0.15, mag) * intens;
                    let base = input.get_pixel(x as u32, y as u32).unwrap_or([0.0; 4]);
                    out.set_pixel(
                        x as u32,
                        y as u32,
                        [
                            base[0] + glow_color[0] * edge,
                            base[1] + glow_color[1] * edge,
                            base[2] + glow_color[2] * edge,
                            base[3],
                        ],
                    );
                }
            }
        }
        ShaderPreset::KaleidoscopeMirror {
            segments,
            rotation_deg,
            zoom,
        } => {
            let segs = segments.max(2) as f32;
            let sector = (2.0 * PI) / segs;
            let rot = deg_to_rad(rotation_deg) + time_s * 0.25;
            let zm = zoom.max(0.1);
            for y in 0..h {
                let ny = ((y as f32 + 0.5) / hf - 0.5) / zm;
                for x in 0..w {
                    let nx = ((x as f32 + 0.5) / wf - 0.5) / zm;
                    let r = sqrtf(nx * nx + ny * ny);
                    let mut ang = atan2f(ny, nx) + rot;
                    if ang < 0.0 {
                        ang += 2.0 * PI * (floorf(-ang / (2.0 * PI)) + 1.0);
                    }
                    let idx = floorf(ang / sector) as i32;
                    let mut local = ang - (idx as f32) * sector;
                    if (idx & 1) != 0 {
                        local = sector - local;
                    }
                    let su = 0.5 + r * cosf(local);
                    let sv = 0.5 + r * sinf(local);
                    out.set_pixel(x, y, sample_uv_bilinear(input, su, sv));
                }
            }
        }
        ShaderPreset::ThermalVision {
            contrast,
            palette_shift,
        } => {
            let cont = contrast.max(0.2);
            for y in 0..h {
                for x in 0..w {
                    let p = input.get_pixel(x, y).unwrap_or([0.0; 4]);
                    let luma = rec709_luma([p[0], p[1], p[2]]);
                    let t = clamp01((luma - 0.5) * cont + 0.5 + palette_shift);
                    let rgb = thermal_ramp(t);
                    out.set_pixel(x, y, [rgb[0], rgb[1], rgb[2], p[3]]);
                }
            }
        }
        ShaderPreset::VhsTracking {
            wobble_px,
            band_y,
            noise_amount,
            chroma_bleed_px,
        } => {
            let wobble = wobble_px * audio_boost;
            let band_center = fract(band_y + time_s * 0.15);
            let bleed_u = chroma_bleed_px / wf;
            let tick = floorf(time_s * 30.0) as i32;
            for y in 0..h {
                let v = (y as f32 + 0.5) / hf;
                let dist_band = absf(v - band_center).min(1.0 - absf(v - band_center));
                let band_mask = 1.0 - smoothstep(0.0, 0.08, dist_band);
                let row_jitter =
                    (sinf(v * 60.0 + time_s * 18.0) * 0.4 + (hash2d(y as i32, tick) - 0.5) * 1.2)
                        * (wobble / wf)
                        * (0.25 + 0.75 * band_mask);
                for x in 0..w {
                    let u = (x as f32 + 0.5) / wf;
                    let pr = sample_uv_bilinear(input, u + row_jitter + bleed_u, v);
                    let pg = sample_uv_bilinear(input, u + row_jitter, v);
                    let pb = sample_uv_bilinear(input, u + row_jitter - bleed_u, v);
                    let snow = (hash2d(x as i32, (y as i32).wrapping_add(tick * 97)) - 0.5)
                        * noise_amount
                        * band_mask;
                    out.set_pixel(
                        x,
                        y,
                        [
                            (pr[0] + snow).max(0.0),
                            (pg[1] + snow).max(0.0),
                            (pb[2] + snow).max(0.0),
                            pg[3],
                        ],
                    );
                }
            }
        }
    }

    Ok(out)
}

#[inline]
fn thermal_ramp(t: f32) -> [f32; 3] {
    let t = clamp01(t);
    if t < 0.25 {
        let k = t / 0.25;
        [lerp(0.02, 0.15, k), lerp(0.02, 0.02, k), lerp(0.15, 0.65, k)]
    } else if t < 0.5 {
        let k = (t - 0.25) / 0.25;
        [lerp(0.15, 0.82, k), lerp(0.02, 0.08, k), lerp(0.65, 0.18, k)]
    } else if t < 0.75 {
        let k = (t - 0.5) / 0.25;
        [lerp(0.82, 0.98, k), lerp(0.08, 0.65, k), lerp(0.18, 0.02, k)]
    } else {
        let k = (t - 0.75) / 0.25;
        [lerp(0.98, 1.0, k), lerp(0.65, 1.0, k), lerp(0.02, 0.85, k)]
    }
}
