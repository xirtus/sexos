// SPDX-License-Identifier: GPL-3.0-only
//! Rendering: a CPU path tracer ("Cycles-class"), a denoiser, view transforms
//! and the fast "Solid" viewport rasterizer.
//!
//! - [`render`] / [`render_region`] / [`Progressive`]: unidirectional path
//!   tracing over a binned-SAH BVH with a Principled-style BSDF (Lambert +
//!   GGX with Fresnel, dielectric glass), next-event estimation with multiple
//!   importance sampling for point/spot/sun/area lights (square, rectangle,
//!   disk, ellipse; spread), emissive triangles and the world (uniform or
//!   importance-sampled HDRI), homogeneous volumes, Russian roulette, and
//!   Cycles' light path controls ([`LightPaths`]).
//! - Cameras: perspective (sensor fit, shift), orthographic, panoramic
//!   (equirectangular, fisheye equidistant/equisolid), depth of field with
//!   bladed apertures, clipping, and motion blur ([`MotionBlur`]).
//! - Film: Blackman-Harris pixel filter, transparent film, holdout, the View
//!   Layer passes ([`Pass`]), adaptive sampling, time limit, render border.
//! - Every pixel sample draws from its own RNG seeded by (pixel, sample, seed),
//!   so any sub-rectangle renders bit-identically to the same pixels of a full
//!   frame, whatever the thread count (adaptive sampling only looks at each
//!   pixel's own samples). That is what the render farm relies on: split a
//!   frame into regions, render them anywhere, paste them together.
//! - [`denoise`]: an edge-avoiding à-trous wavelet filter guided by the albedo
//!   and normal AOVs and the per-pixel variance.
//! - [`to_srgb8`] / [`to_srgb8_with`] / [`to_display8`]: view transforms
//!   (Standard, Filmic, AgX, Khronos PBR Neutral, False Color, Raw), looks,
//!   exposure and gamma.
//! - [`viewport`]: the z-buffer rasterizer behind the 3D view's Solid mode, and
//!   object picking.
use crate::math::Vec3;
use crate::scene::{Image, Scene};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

mod bsdf;
mod bvh;
mod camera;
mod denoise;
mod film;
mod integrator;
mod prepare;
mod rng;
mod sampling;
pub(crate) mod shader;

/// Blender's stored enum numbers for node settings, for file importers.
pub(crate) mod shader_names {
    /// `NodeMathOperation` number to the Math node's operation identifier.
    pub(crate) fn math_from_file(n: i64) -> &'static str {
        super::shader::ops::MathOp::file_name(n)
    }
    /// `NodeVectorMathOperation` number to the operation identifier.
    pub(crate) fn vector_math_from_file(n: i64) -> &'static str {
        super::shader::ops::VecOp::file_name(n)
    }
    /// `MA_RAMP_*` number to the Mix node's blend type identifier.
    pub(crate) fn blend_from_file(n: i64) -> &'static str {
        super::shader::color::Blend::from_file(n)
    }
}
mod tonemap;
pub mod viewport;
pub mod volume;

pub use denoise::{denoise, denoise_images, DenoiseSettings};
pub use film::{MistFalloff, MistSettings, Pass, Passes, BACKGROUND_DEPTH};
pub use prepare::PreparedScene;
pub use tonemap::{display, srgb_decode, srgb_encode, to_display8, to_srgb8, to_srgb8_with, ColorManagement, Look, ViewTransform};
pub use volume::VolumeCoeffs;

use film::PassLayout;
use integrator::Frame;

/// Cycles' Light Paths panel (besides the total, [`RenderSettings::max_bounces`],
/// and indirect clamping, [`RenderSettings::clamp`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightPaths {
    pub diffuse_bounces: u32,
    pub glossy_bounces: u32,
    pub transmission_bounces: u32,
    pub volume_bounces: u32,
    /// Crossings of volume boundaries and objects hidden from a ray.
    pub transparent_bounces: u32,
    /// Clamp the brightest component of direct light (0 = off).
    pub clamp_direct: f32,
    /// Glossy reflection after a diffuse bounce.
    pub caustics_reflective: bool,
    /// Refraction after a diffuse bounce.
    pub caustics_refractive: bool,
    /// Blur glossy lobes after blurry bounces (0 = off).
    pub filter_glossy: f32,
}

impl Default for LightPaths {
    /// Blender's defaults.
    fn default() -> Self {
        LightPaths { diffuse_bounces: 4, glossy_bounces: 4, transmission_bounces: 12, volume_bounces: 0, transparent_bounces: 8, clamp_direct: 0.0, caustics_reflective: true, caustics_refractive: true, filter_glossy: 1.0 }
    }
}

impl LightPaths {
    /// Only the total bounce count limits paths; no caustic tricks.
    pub fn unlimited() -> LightPaths {
        LightPaths { diffuse_bounces: 1024, glossy_bounces: 1024, transmission_bounces: 1024, volume_bounces: 1024, transparent_bounces: 1024, clamp_direct: 0.0, caustics_reflective: true, caustics_refractive: true, filter_glossy: 0.0 }
    }
}

/// When the shutter opens relative to the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShutterPosition {
    Start,
    #[default]
    Center,
    End,
}

/// Motion blur: objects with [`crate::scene::Object::motion`] move linearly
/// over the shutter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlur {
    /// Shutter length in frames (Blender's default 0.5).
    pub shutter: f32,
    pub position: ShutterPosition,
}

impl Default for MotionBlur {
    fn default() -> Self {
        MotionBlur { shutter: 0.5, position: ShutterPosition::Center }
    }
}

impl MotionBlur {
    /// The open interval in frames relative to the current frame.
    pub fn interval(&self) -> (f32, f32) {
        let s = if self.shutter.is_finite() { self.shutter.max(0.0) } else { 0.0 };
        match self.position {
            ShutterPosition::Start => (0.0, s),
            ShutterPosition::Center => (-0.5 * s, 0.5 * s),
            ShutterPosition::End => (-s, 0.0),
        }
    }
}

/// Render region in 0..1 frame coordinates, y up from the bottom (Blender's
/// border), optionally cropping the output to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderBorder {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub crop: bool,
}

impl RenderBorder {
    /// Pixel rectangle (x0, y0, x1, y1), rows from the top.
    pub fn pixels(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let (ax, bx) = (c(self.min_x.min(self.max_x)), c(self.min_x.max(self.max_x)));
        let (ay, by) = (c(self.min_y.min(self.max_y)), c(self.min_y.max(self.max_y)));
        let x0 = (ax * w as f32).floor() as u32;
        let x1 = ((bx * w as f32).ceil() as u32).clamp(x0, w);
        let y0 = ((1.0 - by) * h as f32).floor() as u32;
        let y1 = (((1.0 - ay) * h as f32).ceil() as u32).clamp(y0, h);
        (x0, y0, x1, y1)
    }
}

/// Path tracer settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel (the maximum with adaptive sampling).
    pub samples: u32,
    /// Maximum number of bounces after the camera ray (0 = direct light only).
    pub max_bounces: u32,
    /// Worker threads; 0 means `std::thread::available_parallelism()`.
    pub threads: usize,
    /// Run [`denoise`] on the final frame (only [`render`]; regions are raw).
    pub denoise: bool,
    /// Clamp the brightest component of each indirect contribution to this
    /// value (Cycles' "Clamp Indirect"); 0 disables clamping.
    pub clamp: f32,
    /// Changes the noise pattern; same seed, same image.
    pub seed: u32,
    pub light_paths: LightPaths,
    /// Adaptive sampling noise threshold (0 = off): a pixel stops once its
    /// estimated error is below it.
    pub noise_threshold: f32,
    /// Samples before adaptive sampling may stop a pixel (0 = automatic).
    pub min_samples: u32,
    /// Stop sampling after this many seconds (0 = no limit).
    pub time_limit: f32,
    /// Vary the seed with [`RenderSettings::frame`].
    pub animated_seed: bool,
    pub frame: i32,
    /// Background seen by the camera is transparent (alpha 0).
    pub film_transparent: bool,
    /// Blackman-Harris pixel filter width in pixels.
    pub filter_width: f32,
    /// Extra View Layer passes in [`RenderResult::passes`].
    pub passes: Passes,
    pub mist: MistSettings,
    /// Ambient Occlusion pass distance.
    pub ao_distance: f32,
    pub motion_blur: Option<MotionBlur>,
    /// Only render this part of the frame ([`render`] only).
    pub border: Option<RenderBorder>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            width: 640,
            height: 360,
            samples: 64,
            max_bounces: 8,
            threads: 0,
            denoise: false,
            clamp: 10.0,
            seed: 0,
            light_paths: LightPaths::default(),
            noise_threshold: 0.0,
            min_samples: 0,
            time_limit: 0.0,
            animated_seed: false,
            frame: 1,
            film_transparent: false,
            filter_width: 1.5,
            passes: Passes::NONE,
            mist: MistSettings::default(),
            ao_distance: 10.0,
            motion_blur: None,
            border: None,
        }
    }
}

impl RenderSettings {
    /// The thread count to actually use.
    pub fn thread_count(&self) -> usize {
        resolve_threads(self.threads)
    }

    /// The seed after the animated-seed option (Cycles hashes the frame in).
    pub fn effective_seed(&self) -> u32 {
        if self.animated_seed {
            let mut h = (self.frame as u32).wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
            h ^= h >> 16;
            h = h.wrapping_mul(0x7FEB_352D);
            h ^= h >> 15;
            self.seed.wrapping_add(h)
        } else {
            self.seed
        }
    }

    /// Adaptive sampling (threshold, min samples, check interval), if on.
    /// Automatic min samples follow the threshold, like Cycles.
    fn adaptive(&self) -> Option<(f32, u32, u32)> {
        if !(self.noise_threshold > 0.0) || self.samples < 2 {
            return None;
        }
        let min = if self.min_samples > 0 { self.min_samples } else { ((4.0 / self.noise_threshold.sqrt()).ceil() as u32).max(16) };
        let step = 4;
        // Checks happen on multiples of the step (even counts, for the half buffer).
        Some((self.noise_threshold, min.max(step).div_ceil(step) * step, step))
    }

    fn frame(&self, adaptive: bool) -> Frame {
        Frame {
            layout: PassLayout::new(self.passes),
            filter: film::PixelFilter::new(self.filter_width),
            seed: self.effective_seed(),
            shutter: self.motion_blur.map(|m| m.interval()).filter(|(a, b)| b > a),
            adaptive: if adaptive { self.adaptive() } else { None },
        }
    }
}

/// A rendered frame (or region) with the AOVs the denoiser needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderResult {
    /// Linear scene-referred colour; alpha is coverage (0 for transparent
    /// film background and holdout).
    pub color: Image,
    /// First-hit base colour, averaged over samples.
    pub albedo: Image,
    /// First-hit world shading normal (not renormalised), averaged.
    pub normal: Image,
    /// Per-pixel variance of the mean luminance estimate.
    pub variance: Vec<f32>,
    /// Samples per pixel requested (the maximum with adaptive sampling).
    pub samples: u32,
    /// Samples each pixel actually took.
    pub sample_counts: Vec<u32>,
    /// The enabled View Layer passes (not Combined, which is `color`).
    /// Scalar passes repeat their value in R, G and B.
    pub passes: BTreeMap<Pass, Image>,
    /// False when the render was cancelled (or hit the time limit) before
    /// finishing (unrendered pixels are black).
    pub complete: bool,
}

impl RenderResult {
    /// A pass by kind; `Combined` is the colour.
    pub fn pass(&self, p: Pass) -> Option<&Image> {
        if p == Pass::Combined { Some(&self.color) } else { self.passes.get(&p) }
    }
}

/// Render the whole frame from the scene's active camera.
pub fn render(scene: &Scene, settings: &RenderSettings) -> RenderResult {
    render_cancellable(scene, settings, &AtomicBool::new(false))
}

/// [`render`], stopping early (with `complete == false`) once `cancel` is set.
pub fn render_cancellable(scene: &Scene, settings: &RenderSettings, cancel: &AtomicBool) -> RenderResult {
    let prepared = PreparedScene::new(scene);
    prepared.render_frame(settings, Some(cancel))
}

/// Render pixels `x0..x1` by `y0..y1` of the frame described by `settings`
/// (never denoised). The pixels are bit-identical to the same pixels of a
/// full [`render`] with `denoise` off, whatever the thread count.
pub fn render_region(scene: &Scene, settings: &RenderSettings, x0: u32, y0: u32, x1: u32, y1: u32) -> RenderResult {
    PreparedScene::new(scene).render_region(settings, x0, y0, x1, y1, None)
}

/// Render an animation: for each frame of `frames`, build its scene with
/// `scene_at(frame)` and hand the result to `on_frame`, which returns false
/// to stop. `settings.frame` follows the frame (for the animated seed).
/// Returns the number of frames rendered.
pub fn render_frames(frames: impl IntoIterator<Item = i32>, settings: &RenderSettings, mut scene_at: impl FnMut(i32) -> Scene, mut on_frame: impl FnMut(i32, RenderResult) -> bool, cancel: Option<&AtomicBool>) -> usize {
    let mut n = 0;
    for f in frames {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            break;
        }
        let scene = scene_at(f);
        let st = RenderSettings { frame: f, ..*settings };
        let r = PreparedScene::new(&scene).render_frame(&st, cancel);
        let complete = r.complete;
        n += 1;
        if !on_frame(f, r) || !complete {
            break;
        }
    }
    n
}

/// Per-pixel running sums for one region.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PixelAccum {
    pub color: Vec3,
    pub alpha: f32,
    pub albedo: Vec3,
    pub normal: Vec3,
    pub lum_sq: f32,
    /// Sum of the odd-numbered samples (adaptive sampling's second estimate).
    pub half: Vec3,
    /// Samples taken.
    pub n: u32,
    /// Converged (adaptive sampling).
    pub done: bool,
}

impl PreparedScene {
    /// The whole frame with the render border, denoising and cropping applied.
    pub fn render_frame(&self, settings: &RenderSettings, cancel: Option<&AtomicBool>) -> RenderResult {
        let (w, h) = (settings.width, settings.height);
        let (x0, y0, x1, y1) = settings.border.map_or((0, 0, w, h), |b| b.pixels(w, h));
        let mut r = self.render_region(settings, x0, y0, x1, y1, cancel);
        if settings.denoise && r.complete {
            r.color = denoise(&r, &DenoiseSettings::default());
        }
        match settings.border {
            Some(b) if !b.crop && (x0, y0, x1, y1) != (0, 0, w, h) => paste(r, w, h, x0, y0),
            _ => r,
        }
    }

    /// Render a region of the frame (see [`render_region`]); `cancel` stops it
    /// between bands of rows.
    pub fn render_region(&self, settings: &RenderSettings, x0: u32, y0: u32, x1: u32, y1: u32, cancel: Option<&AtomicBool>) -> RenderResult {
        let (x1, y1) = (x1.min(settings.width), y1.min(settings.height));
        let (w, h) = (x1.saturating_sub(x0) as usize, y1.saturating_sub(y0) as usize);
        let frame = settings.frame(true);
        let stride = frame.layout.stride.max(1);
        let mut acc = vec![PixelAccum::default(); w * h];
        let mut rec = vec![0.0f32; w * h * stride];
        for r in rec.chunks_mut(stride) {
            frame.layout.init(r);
        }
        let cancelled = AtomicBool::new(false);
        let timed_out = AtomicBool::new(false);
        if w > 0 && h > 0 {
            let rows_per_band = 4;
            // With a time limit, samples go in rounds over the whole region so
            // stopping leaves an evenly sampled image.
            let round = if settings.time_limit > 0.0 { 4 } else { settings.samples.max(1) };
            let start = (settings.time_limit > 0.0).then(std::time::Instant::now);
            let mut s0 = 0;
            while s0 < settings.samples {
                let s1 = (s0 + round).min(settings.samples);
                let items = acc.chunks_mut(w * rows_per_band).zip(rec.chunks_mut(w * rows_per_band * stride)).enumerate();
                parallel_for_each(items, settings.thread_count(), |(band, (chunk, recs))| {
                    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                        cancelled.store(true, Ordering::Relaxed);
                        return;
                    }
                    for (i, (px, r)) in chunk.iter_mut().zip(recs.chunks_mut(stride)).enumerate() {
                        let x = x0 + (i % w) as u32;
                        let y = y0 + (band * rows_per_band + i / w) as u32;
                        integrator::accumulate_pixel(self, settings, &frame, x, y, s0..s1, px, r);
                    }
                });
                s0 = s1;
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                if start.is_some_and(|t| t.elapsed().as_secs_f32() >= settings.time_limit) && s0 < settings.samples {
                    timed_out.store(true, Ordering::Relaxed);
                    break;
                }
                if acc.iter().all(|a| a.done) {
                    break;
                }
            }
        }
        let mut r = finish(&acc, &rec, &frame.layout, w as u32, h as u32, settings.samples);
        // A time limit is a normal way to finish; cancelling is not.
        r.complete = !cancelled.load(Ordering::Relaxed);
        let _ = timed_out;
        r
    }
}

/// Put a region result into a transparent full frame at (x0, y0).
fn paste(r: RenderResult, w: u32, h: u32, x0: u32, y0: u32) -> RenderResult {
    let rw = r.color.width;
    let put = |src: &Image, fill: [f32; 4]| {
        let mut img = Image { name: src.name.clone(), width: w, height: h, pixels: vec![fill; (w * h) as usize] };
        for y in 0..src.height {
            for x in 0..src.width {
                img.pixels[((y + y0) * w + x + x0) as usize] = src.pixels[(y * rw + x) as usize];
            }
        }
        img
    };
    let mut variance = vec![0.0; (w * h) as usize];
    let mut sample_counts = vec![0; (w * h) as usize];
    for y in 0..r.color.height {
        for x in 0..rw {
            let (s, d) = ((y * rw + x) as usize, ((y + y0) * w + x + x0) as usize);
            variance[d] = r.variance[s];
            sample_counts[d] = r.sample_counts[s];
        }
    }
    let passes = r
        .passes
        .iter()
        .map(|(p, img)| {
            let fill = if *p == Pass::Depth { [BACKGROUND_DEPTH, BACKGROUND_DEPTH, BACKGROUND_DEPTH, 1.0] } else { [0.0, 0.0, 0.0, 0.0] };
            (*p, put(img, fill))
        })
        .collect();
    RenderResult {
        color: put(&r.color, [0.0; 4]),
        albedo: put(&r.albedo, [0.0; 4]),
        normal: put(&r.normal, [0.0; 4]),
        variance,
        samples: r.samples,
        sample_counts,
        passes,
        complete: r.complete,
    }
}

/// Turn running sums into a [`RenderResult`].
fn finish(acc: &[PixelAccum], rec: &[f32], layout: &PassLayout, w: u32, h: u32, samples: u32) -> RenderResult {
    let inv = |a: &PixelAccum| 1.0 / a.n.max(1) as f32;
    let image = |name: &str, f: &dyn Fn(&PixelAccum) -> [f32; 4]| Image { name: name.into(), width: w, height: h, pixels: acc.iter().map(f).collect() };
    let rgb = |v: Vec3, k: f32| [v.x * k, v.y * k, v.z * k, 1.0];
    let variance = acc
        .iter()
        .map(|a| {
            let n = a.n as f32;
            if a.n < 2 {
                return 0.0;
            }
            let mean = luminance(a.color) / n;
            ((a.lum_sq / n - mean * mean).max(0.0) * n / (n - 1.0)) / n
        })
        .collect();
    let stride = layout.stride.max(1);
    let mut passes = BTreeMap::new();
    for p in layout.passes.iter() {
        let Some(o) = layout.offset(p) else { continue };
        let pixels = acc
            .iter()
            .zip(rec.chunks(stride))
            .map(|(a, r)| {
                let k = if p.nearest() { 1.0 } else { inv(a) };
                if p.channels() == 1 {
                    let v = r[o] * k;
                    [v, v, v, 1.0]
                } else {
                    [r[o] * k, r[o + 1] * k, r[o + 2] * k, 1.0]
                }
            })
            .collect();
        passes.insert(p, Image { name: p.name().into(), width: w, height: h, pixels });
    }
    RenderResult {
        color: image("Render", &|a| {
            let k = inv(a);
            [a.color.x * k, a.color.y * k, a.color.z * k, if a.n == 0 { 1.0 } else { a.alpha * k }]
        }),
        albedo: image("Albedo", &|a| rgb(a.albedo, inv(a))),
        normal: image("Normal", &|a| rgb(a.normal, inv(a))),
        variance,
        samples,
        sample_counts: acc.iter().map(|a| a.n).collect(),
        passes,
        complete: true,
    }
}

/// Progressive accumulation for the interactive rendered viewport: each
/// [`Progressive::add_sample`] adds one sample to every pixel. After `n` calls
/// the image equals a [`render`] with `samples = n` (same seed, no denoise,
/// no adaptive sampling). With `noise_threshold` set, pixels that converged
/// stop taking samples and [`Progressive::converged`] reports when all have.
pub struct Progressive {
    scene: PreparedScene,
    settings: RenderSettings,
    frame: Frame,
    acc: Vec<PixelAccum>,
    rec: Vec<f32>,
    samples: u32,
}

impl Progressive {
    /// Prepare the scene once; `settings.samples` is ignored.
    pub fn new(scene: &Scene, settings: &RenderSettings) -> Progressive {
        Progressive::from_prepared(PreparedScene::new(scene), settings)
    }
    pub fn from_prepared(scene: PreparedScene, settings: &RenderSettings) -> Progressive {
        let n = settings.width as usize * settings.height as usize;
        let mut frame = settings.frame(false);
        if settings.noise_threshold > 0.0 {
            frame.adaptive = RenderSettings { samples: u32::MAX, ..*settings }.adaptive();
        }
        let stride = frame.layout.stride.max(1);
        let mut p = Progressive { scene, settings: *settings, frame, acc: vec![PixelAccum::default(); n], rec: vec![0.0; n * stride], samples: 0 };
        p.reset();
        p
    }
    /// Samples accumulated so far.
    pub fn samples(&self) -> u32 {
        self.samples
    }
    /// True once adaptive sampling has stopped every pixel.
    pub fn converged(&self) -> bool {
        self.frame.adaptive.is_some() && self.acc.iter().all(|a| a.done)
    }
    /// Start over (e.g. after the view changed but the scene did not).
    pub fn reset(&mut self) {
        self.acc.iter_mut().for_each(|a| *a = PixelAccum::default());
        let stride = self.frame.layout.stride.max(1);
        for r in self.rec.chunks_mut(stride) {
            self.frame.layout.init(r);
        }
        self.samples = 0;
    }
    /// Add one sample per pixel. Returns false (and adds nothing) if `cancel`
    /// was set before the pass finished.
    pub fn add_sample(&mut self, cancel: Option<&AtomicBool>) -> bool {
        let w = self.settings.width as usize;
        if w == 0 {
            return true;
        }
        let s = self.samples;
        let stride = self.frame.layout.stride.max(1);
        let mut pass = self.acc.clone();
        let mut prec = vec![0.0f32; self.rec.len()];
        for r in prec.chunks_mut(stride) {
            self.frame.layout.init(r);
        }
        let cancelled = AtomicBool::new(false);
        let (scene, settings, frame) = (&self.scene, &self.settings, &self.frame);
        let rows_per_band = 4;
        let items = pass.chunks_mut(w * rows_per_band).zip(prec.chunks_mut(w * rows_per_band * stride)).enumerate();
        parallel_for_each(items, settings.thread_count(), |(band, (chunk, recs))| {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                cancelled.store(true, Ordering::Relaxed);
                return;
            }
            for (i, (px, r)) in chunk.iter_mut().zip(recs.chunks_mut(stride)).enumerate() {
                let (x, y) = ((i % w) as u32, (band * rows_per_band + i / w) as u32);
                integrator::accumulate_pixel(scene, settings, frame, x, y, s..s + 1, px, r);
            }
        });
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        self.acc = pass;
        if self.frame.layout.stride > 0 {
            for (a, b) in self.rec.chunks_mut(stride).zip(prec.chunks(stride)) {
                self.frame.layout.merge(a, b);
            }
        }
        self.samples += 1;
        true
    }
    /// The current estimate with its AOVs and passes.
    pub fn result(&self) -> RenderResult {
        finish(&self.acc, &self.rec, &self.frame.layout, self.settings.width, self.settings.height, self.samples)
    }
    /// The current colour estimate.
    pub fn image(&self) -> Image {
        self.result().color
    }
}

/// Rec. 709 luminance.
pub(crate) fn luminance(c: Vec3) -> f32 {
    0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z
}

pub(crate) fn resolve_threads(threads: usize) -> usize {
    if threads > 0 {
        threads
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    }
}

/// Run `f` on every item of `items` from `threads` scoped workers that pull
/// the next item as they finish (dynamic load balancing). Items are typically
/// disjoint `chunks_mut` of output buffers, so no locking of pixels is needed.
pub(crate) fn parallel_for_each<I>(items: I, threads: usize, f: impl Fn(I::Item) + Sync)
where
    I: Iterator + Send,
    I::Item: Send,
{
    let threads = threads.min(items.size_hint().1.unwrap_or(usize::MAX));
    if threads <= 1 {
        items.for_each(f);
        return;
    }
    let queue = Mutex::new(items);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                match next {
                    Some(item) => f(item),
                    None => break,
                }
            });
        }
    });
}
