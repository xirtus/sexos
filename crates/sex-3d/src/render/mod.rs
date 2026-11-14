// SPDX-License-Identifier: GPL-3.0-only
//! Rendering: a CPU path tracer ("Cycles-class"), a denoiser, view transforms
//! and the fast "Solid" viewport rasterizer.
//!
//! - [`render`] / [`render_region`] / [`Progressive`]: unidirectional path
//!   tracing over a binned-SAH BVH with a Principled-style BSDF (Lambert +
//!   GGX with Fresnel, dielectric glass), next-event estimation with multiple
//!   importance sampling for point/spot/sun/area lights, emissive triangles and
//!   the world (uniform or importance-sampled HDRI), Russian roulette and
//!   firefly clamping.
//! - Every pixel sample draws from its own RNG seeded by (pixel, sample, seed),
//!   so any sub-rectangle renders bit-identically to the same pixels of a full
//!   frame. That is what the render farm relies on: split a frame into regions,
//!   render them anywhere, paste them together.
//! - [`denoise`]: an edge-avoiding à-trous wavelet filter guided by the albedo
//!   and normal AOVs and the per-pixel variance.
//! - [`to_srgb8`] / [`to_srgb8_with`]: "Standard" and AgX-style view transforms.
//! - [`viewport`]: the z-buffer rasterizer behind the 3D view's Solid mode, and
//!   object picking.
use crate::math::Vec3;
use crate::scene::{Image, Scene};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

mod bsdf;
mod bvh;
mod denoise;
mod integrator;
mod prepare;
mod rng;
mod sampling;
mod tonemap;
pub mod viewport;

pub use denoise::{denoise, denoise_images, DenoiseSettings};
pub use prepare::PreparedScene;
pub use tonemap::{srgb_decode, srgb_encode, to_srgb8, to_srgb8_with, ViewTransform};

/// Path tracer settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel.
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
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings { width: 640, height: 360, samples: 64, max_bounces: 8, threads: 0, denoise: false, clamp: 10.0, seed: 0 }
    }
}

impl RenderSettings {
    /// The thread count to actually use.
    pub fn thread_count(&self) -> usize {
        resolve_threads(self.threads)
    }
}

/// A rendered frame (or region) with the AOVs the denoiser needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderResult {
    /// Linear scene-referred colour, alpha 1.
    pub color: Image,
    /// First-hit base colour, averaged over samples.
    pub albedo: Image,
    /// First-hit world shading normal (not renormalised), averaged.
    pub normal: Image,
    /// Per-pixel variance of the mean luminance estimate.
    pub variance: Vec<f32>,
    /// Samples taken per pixel.
    pub samples: u32,
    /// False when the render was cancelled before finishing (unrendered
    /// pixels are black).
    pub complete: bool,
}

/// Render the whole frame from the scene's active camera.
pub fn render(scene: &Scene, settings: &RenderSettings) -> RenderResult {
    render_cancellable(scene, settings, &AtomicBool::new(false))
}

/// [`render`], stopping early (with `complete == false`) once `cancel` is set.
pub fn render_cancellable(scene: &Scene, settings: &RenderSettings, cancel: &AtomicBool) -> RenderResult {
    let prepared = PreparedScene::new(scene);
    let mut r = prepared.render_region(settings, 0, 0, settings.width, settings.height, Some(cancel));
    if settings.denoise && r.complete {
        r.color = denoise(&r, &DenoiseSettings::default());
    }
    r
}

/// Render pixels `x0..x1` by `y0..y1` of the frame described by `settings`
/// (never denoised). The pixels are bit-identical to the same pixels of a
/// full [`render`] with `denoise` off, whatever the thread count.
pub fn render_region(scene: &Scene, settings: &RenderSettings, x0: u32, y0: u32, x1: u32, y1: u32) -> RenderResult {
    PreparedScene::new(scene).render_region(settings, x0, y0, x1, y1, None)
}

/// Per-pixel running sums for one region.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PixelAccum {
    pub color: Vec3,
    pub albedo: Vec3,
    pub normal: Vec3,
    pub lum_sq: f32,
}

impl PreparedScene {
    /// Render a region of the frame (see [`render_region`]); `cancel` stops it
    /// between bands of rows.
    pub fn render_region(&self, settings: &RenderSettings, x0: u32, y0: u32, x1: u32, y1: u32, cancel: Option<&AtomicBool>) -> RenderResult {
        let (x1, y1) = (x1.min(settings.width), y1.min(settings.height));
        let (w, h) = (x1.saturating_sub(x0) as usize, y1.saturating_sub(y0) as usize);
        let mut acc = vec![PixelAccum::default(); w * h];
        let cancelled = AtomicBool::new(false);
        if w > 0 {
            let rows_per_band = 4;
            parallel_for_each(acc.chunks_mut(w * rows_per_band).enumerate(), settings.thread_count(), |(band, chunk)| {
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    cancelled.store(true, Ordering::Relaxed);
                    return;
                }
                for (i, px) in chunk.iter_mut().enumerate() {
                    let x = x0 + (i % w) as u32;
                    let y = y0 + (band * rows_per_band + i / w) as u32;
                    integrator::accumulate_pixel(self, settings, x, y, 0..settings.samples, px);
                }
            });
        }
        let mut r = finish(&acc, w as u32, h as u32, settings.samples);
        r.complete = !cancelled.load(Ordering::Relaxed);
        r
    }
}

/// Turn running sums into a [`RenderResult`].
fn finish(acc: &[PixelAccum], w: u32, h: u32, samples: u32) -> RenderResult {
    let n = samples.max(1) as f32;
    let px = |v: Vec3| [v.x / n, v.y / n, v.z / n, 1.0];
    let image = |name: &str, f: &dyn Fn(&PixelAccum) -> Vec3| Image { name: name.into(), width: w, height: h, pixels: acc.iter().map(|a| px(f(a))).collect() };
    let variance = acc
        .iter()
        .map(|a| {
            if samples < 2 {
                return 0.0;
            }
            let mean = luminance(a.color) / n;
            ((a.lum_sq / n - mean * mean).max(0.0) * n / (n - 1.0)) / n
        })
        .collect();
    RenderResult {
        color: image("Render", &|a| a.color),
        albedo: image("Albedo", &|a| a.albedo),
        normal: image("Normal", &|a| a.normal),
        variance,
        samples,
        complete: true,
    }
}

/// Progressive accumulation for the interactive rendered viewport: each
/// [`Progressive::add_sample`] adds one sample to every pixel. After `n` calls
/// the image equals a [`render`] with `samples = n` (same seed, no denoise).
pub struct Progressive {
    scene: PreparedScene,
    settings: RenderSettings,
    acc: Vec<PixelAccum>,
    samples: u32,
}

impl Progressive {
    /// Prepare the scene once; `settings.samples` is ignored.
    pub fn new(scene: &Scene, settings: &RenderSettings) -> Progressive {
        Progressive::from_prepared(PreparedScene::new(scene), settings)
    }
    pub fn from_prepared(scene: PreparedScene, settings: &RenderSettings) -> Progressive {
        let n = settings.width as usize * settings.height as usize;
        Progressive { scene, settings: *settings, acc: vec![PixelAccum::default(); n], samples: 0 }
    }
    /// Samples accumulated so far.
    pub fn samples(&self) -> u32 {
        self.samples
    }
    /// Start over (e.g. after the view changed but the scene did not).
    pub fn reset(&mut self) {
        self.acc.iter_mut().for_each(|a| *a = PixelAccum::default());
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
        let mut pass = vec![PixelAccum::default(); self.acc.len()];
        let cancelled = AtomicBool::new(false);
        let (scene, settings) = (&self.scene, &self.settings);
        let rows_per_band = 4;
        parallel_for_each(pass.chunks_mut(w * rows_per_band).enumerate(), settings.thread_count(), |(band, chunk)| {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                cancelled.store(true, Ordering::Relaxed);
                return;
            }
            for (i, px) in chunk.iter_mut().enumerate() {
                let (x, y) = ((i % w) as u32, (band * rows_per_band + i / w) as u32);
                integrator::accumulate_pixel(scene, settings, x, y, s..s + 1, px);
            }
        });
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        for (a, p) in self.acc.iter_mut().zip(&pass) {
            a.color += p.color;
            a.albedo += p.albedo;
            a.normal += p.normal;
            a.lum_sq += p.lum_sq;
        }
        self.samples += 1;
        true
    }
    /// The current estimate with its AOVs.
    pub fn result(&self) -> RenderResult {
        finish(&self.acc, self.settings.width, self.settings.height, self.samples)
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
