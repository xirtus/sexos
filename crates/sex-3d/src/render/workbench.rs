// SPDX-License-Identifier: GPL-3.0-only
//! Workbench: the shading options of the Solid viewport and the Workbench
//! render engine (F12), after Blender's `View3DShading` and its Workbench
//! engine.
//!
//! - Lighting: Studio (the viewport's key/fill/rim studio rig, with or
//!   without specular), Flat (unlit colour) or MatCap (a view-space normal
//!   lookup into one of our procedurally generated material captures).
//! - Colour: Material, Object, Attribute (the mesh's colour attribute),
//!   Single, Random (per object) or Texture (the material's image).
//! - Options: back-face culling, X-ray with alpha (weighted blended
//!   order-independent transparency), a shadow from the light direction
//!   (a shadow map over the scene), cavity (screen-space curvature, world
//!   space ambient occlusion, or both), object outlines.
//!
//! [`render_workbench`] renders a scene camera's view with these options,
//! anti-aliased by accumulating jittered samples, as a linear image.
use super::raster::RasterCamera;
use super::tonemap::{srgb_decode, srgb_encode};
use super::viewport::{ViewportScene, ViewportSettings};
use crate::math::Vec3;
use crate::scene::{Image, Scene};
use std::sync::OnceLock;

/// Viewport lighting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lighting {
    #[default]
    Studio,
    Flat,
    MatCap,
}

impl Lighting {
    pub const ALL: [Lighting; 3] = [Lighting::Studio, Lighting::Flat, Lighting::MatCap];
    pub fn label(self) -> &'static str {
        match self {
            Lighting::Studio => "Studio",
            Lighting::Flat => "Flat",
            Lighting::MatCap => "MatCap",
        }
    }
}

/// Where surface colour comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorType {
    #[default]
    Material,
    Object,
    Attribute,
    Single,
    Random,
    Texture,
}

impl ColorType {
    pub const ALL: [ColorType; 6] = [ColorType::Material, ColorType::Object, ColorType::Attribute, ColorType::Single, ColorType::Random, ColorType::Texture];
    pub fn label(self) -> &'static str {
        match self {
            ColorType::Material => "Material",
            ColorType::Object => "Object",
            ColorType::Attribute => "Attribute",
            ColorType::Single => "Single",
            ColorType::Random => "Random",
            ColorType::Texture => "Texture",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CavityType {
    #[default]
    World,
    Screen,
    Both,
}

impl CavityType {
    pub const ALL: [CavityType; 3] = [CavityType::World, CavityType::Screen, CavityType::Both];
    pub fn label(self) -> &'static str {
        match self {
            CavityType::World => "World",
            CavityType::Screen => "Screen",
            CavityType::Both => "Both",
        }
    }
}

/// Workbench shading options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Workbench {
    pub lighting: Lighting,
    /// Index into [`MATCAPS`].
    pub matcap: usize,
    pub color: ColorType,
    /// Linear RGB for [`ColorType::Single`].
    pub single_color: [f32; 3],
    /// Studio specular highlights.
    pub specular: bool,
    /// See-through surfaces with this opacity.
    pub xray: bool,
    pub xray_alpha: f32,
    pub shadow: bool,
    /// How dark shadowed areas get (0..1).
    pub shadow_intensity: f32,
    /// World-space direction towards the light.
    pub light_direction: Vec3,
    pub cavity: bool,
    pub cavity_type: CavityType,
    /// World cavity: ridge brightening and valley darkening.
    pub cavity_ridge: f32,
    pub cavity_valley: f32,
    /// Screen cavity (curvature): ridge and valley factors.
    pub curvature_ridge: f32,
    pub curvature_valley: f32,
    /// Dark outline around objects.
    pub outline: bool,
}

impl Default for Workbench {
    /// The look the Solid view always had: studio lighting with specular on
    /// material colours, nothing else.
    fn default() -> Self {
        Workbench {
            lighting: Lighting::Studio,
            matcap: 0,
            color: ColorType::Material,
            single_color: [0.8, 0.8, 0.8],
            specular: true,
            xray: false,
            xray_alpha: 0.5,
            shadow: false,
            shadow_intensity: 0.5,
            light_direction: Vec3::new(0.57735, -0.57735, 0.57735),
            cavity: false,
            cavity_type: CavityType::World,
            cavity_ridge: 1.0,
            cavity_valley: 1.0,
            curvature_ridge: 1.0,
            curvature_valley: 1.0,
            outline: false,
        }
    }
}

impl Workbench {
    /// Blender's defaults for a new 3D view (outline on).
    pub fn viewport_defaults() -> Workbench {
        Workbench { outline: true, ..Workbench::default() }
    }
}

/// Solid-mode studio lighting in view space for unit `n` and `v` facing the
/// viewer: (factor on the base colour, white added on top).
#[inline]
pub(crate) fn studio_terms(n: Vec3, v: Vec3, specular: bool) -> (f32, f32) {
    const KEY: Vec3 = Vec3 { x: -0.408, y: 0.571, z: 0.712 };
    const FILL: Vec3 = Vec3 { x: 0.6, y: -0.48, z: 0.64 };
    let nv = n.dot(v).max(0.0);
    let diffuse = 0.22 + 0.42 * nv + 0.38 * n.dot(KEY).max(0.0) + 0.12 * n.dot(FILL).max(0.0);
    let spec = if specular {
        let h = (KEY + v).normalize();
        0.18 * n.dot(h).max(0.0).powi(32)
    } else {
        0.0
    };
    let rim = 0.12 * (1.0 - nv).powi(4);
    (diffuse, spec + rim)
}

// ---------------------------------------------------------------- matcaps

/// Names of the bundled material captures, all generated procedurally.
pub const MATCAPS: [&str; 6] = ["Clay", "Chrome", "Toon", "Jade", "Ceramic", "Normals"];

const MATCAP_RES: usize = 128;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A matcap's colour (display RGB) for a view-space unit normal.
fn matcap_fn(i: usize, n: Vec3) -> Vec3 {
    let v = Vec3::Z;
    let key = Vec3::new(-0.45, 0.6, 0.66).normalize();
    let rim = (1.0 - n.z.max(0.0)).powi(3);
    let spec = |l: Vec3, p: i32| n.dot((l + v).normalize()).max(0.0).powi(p);
    match i {
        0 => {
            // Clay: warm wrapped diffuse, cool fill from below, soft rim.
            let wrap = ((n.dot(key) + 0.4) / 1.4).max(0.0);
            let fill = ((-n.y + 0.3) / 1.3).max(0.0) * 0.18;
            let c = Vec3::new(0.78, 0.66, 0.58) * (0.18 + 0.78 * wrap) + Vec3::new(0.35, 0.42, 0.55) * fill + Vec3::splat(0.25 * rim) + Vec3::splat(0.12 * spec(key, 12));
            c
        }
        1 => {
            // Chrome: the reflected direction picks a sky / horizon / ground.
            let r = n * (2.0 * n.z) - v;
            let sky = Vec3::new(0.85, 0.9, 1.0) * (0.55 + 0.45 * r.y.max(0.0));
            let ground = Vec3::new(0.12, 0.1, 0.09) + Vec3::splat(0.2 * (-r.y).max(0.0).powi(2));
            let t = smoothstep(-0.04, 0.04, r.y);
            let horizon = (1.0 - (r.y.abs() * 12.0).min(1.0)) * 0.5;
            ground + (sky - ground) * t + Vec3::splat(horizon) + Vec3::splat(spec(key, 120) * 1.2)
        }
        2 => {
            // Toon: three bands and a hard highlight.
            let d = n.dot(key);
            let band = if d > 0.55 { 1.0 } else if d > 0.1 { 0.68 } else { 0.38 };
            let hi = if spec(key, 40) > 0.6 { 0.35 } else { 0.0 };
            Vec3::new(0.95, 0.72, 0.42) * band + Vec3::splat(hi) + Vec3::splat(if rim > 0.55 { 0.15 } else { 0.0 })
        }
        3 => {
            // Jade: translucent green, light wrapping deep around the form.
            let wrap = ((n.dot(key) + 0.8) / 1.8).max(0.0);
            Vec3::new(0.18, 0.55, 0.38) * (0.25 + 0.75 * wrap) + Vec3::new(0.3, 0.7, 0.5) * (0.35 * rim) + Vec3::splat(0.55 * spec(key, 60))
        }
        4 => {
            // Ceramic: white glaze, sharp highlights from two lights, dark edges.
            let fill = Vec3::new(0.7, -0.2, 0.7).normalize();
            let d = 0.25 + 0.7 * n.dot(key).max(0.0) + 0.15 * n.dot(fill).max(0.0);
            Vec3::splat(0.92) * d * (1.0 - 0.45 * rim) + Vec3::splat(0.9 * spec(key, 200) + 0.4 * spec(fill, 120))
        }
        _ => (n + Vec3::ONE) * 0.5,
    }
}

fn matcap_tables() -> &'static Vec<Vec<[f32; 3]>> {
    static T: OnceLock<Vec<Vec<[f32; 3]>>> = OnceLock::new();
    T.get_or_init(|| {
        (0..MATCAPS.len())
            .map(|i| {
                let mut img = vec![[0.0; 3]; MATCAP_RES * MATCAP_RES];
                for y in 0..MATCAP_RES {
                    for x in 0..MATCAP_RES {
                        let nx = (x as f32 + 0.5) / MATCAP_RES as f32 * 2.0 - 1.0;
                        let ny = 1.0 - (y as f32 + 0.5) / MATCAP_RES as f32 * 2.0;
                        let r2 = nx * nx + ny * ny;
                        // Outside the disk: the edge normal (lookups clamp there).
                        let n = if r2 < 1.0 { Vec3::new(nx, ny, (1.0 - r2).sqrt()) } else { Vec3::new(nx, ny, 0.0).normalize() };
                        let c = matcap_fn(i, n);
                        img[y * MATCAP_RES + x] = [c.x.clamp(0.0, 1.0), c.y.clamp(0.0, 1.0), c.z.clamp(0.0, 1.0)];
                    }
                }
                img
            })
            .collect()
    })
}

/// Matcap `i`'s colour (display RGB) for a view-space normal (bilinear).
pub(crate) fn matcap(i: usize, n: Vec3) -> Vec3 {
    let t = &matcap_tables()[i.min(MATCAPS.len() - 1)];
    let fx = ((n.x * 0.5 + 0.5) * MATCAP_RES as f32 - 0.5).clamp(0.0, MATCAP_RES as f32 - 1.0);
    let fy = ((0.5 - n.y * 0.5) * MATCAP_RES as f32 - 0.5).clamp(0.0, MATCAP_RES as f32 - 1.0);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(MATCAP_RES - 1), (y0 + 1).min(MATCAP_RES - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let p = |x: usize, y: usize| Vec3::from_array(t[y * MATCAP_RES + x]);
    (p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx) * (1.0 - ty) + (p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx) * ty
}

/// Matcap `i` as an RGBA8 sphere preview of `size` x `size` pixels.
pub fn matcap_preview(i: usize, size: u32) -> Vec<u8> {
    let s = size.max(1) as usize;
    let mut out = vec![0u8; s * s * 4];
    for y in 0..s {
        for x in 0..s {
            let nx = (x as f32 + 0.5) / s as f32 * 2.0 - 1.0;
            let ny = 1.0 - (y as f32 + 0.5) / s as f32 * 2.0;
            let r2 = nx * nx + ny * ny;
            if r2 >= 1.0 {
                continue;
            }
            let c = matcap(i, Vec3::new(nx, ny, (1.0 - r2).sqrt()));
            let o = (y * s + x) * 4;
            out[o..o + 4].copy_from_slice(&[(c.x * 255.0) as u8, (c.y * 255.0) as u8, (c.z * 255.0) as u8, 255]);
        }
    }
    out
}

/// A colour per object for [`ColorType::Random`] (display RGB): a hue from
/// a hash, at Blender's random-colour saturation and value.
pub(crate) fn random_color(seed: u32) -> Vec3 {
    let mut h = seed.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    let hue = (h & 0xffff) as f32 / 65536.0;
    let (s, v) = (0.5, 0.9);
    let k = |n: f32| {
        let k = (n + hue * 6.0) % 6.0;
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    Vec3::new(k(5.0), k(3.0), k(1.0))
}

// ---------------------------------------------------------------- F12

/// The Workbench render engine's settings (Render Properties).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkbenchRender {
    pub width: u32,
    pub height: u32,
    /// Anti-aliasing samples (jittered frames averaged).
    pub samples: u32,
    pub shading: Workbench,
    /// Background alpha 0 instead of the world colour.
    pub film_transparent: bool,
    pub backface_culling: bool,
    /// 0 = all cores.
    pub threads: usize,
}

impl Default for WorkbenchRender {
    fn default() -> Self {
        WorkbenchRender { width: 640, height: 360, samples: 8, shading: Workbench::default(), film_transparent: false, backface_culling: false, threads: 0 }
    }
}

/// Subpixel offsets in (-0.5, 0.5): the R2 low-discrepancy sequence.
pub(crate) fn jitter(i: u32) -> (f32, f32) {
    if i == 0 {
        return (0.0, 0.0);
    }
    const G: f64 = 1.324_717_957_244_746;
    let (a1, a2) = (1.0 / G, 1.0 / (G * G));
    let x = (0.5 + a1 * i as f64).fract() as f32 - 0.5;
    let y = (0.5 + a2 * i as f64).fract() as f32 - 0.5;
    (x, y)
}

/// Render the scene's active camera with the Workbench engine: a linear
/// RGBA image, alpha premultiplied (the world colour behind, or transparent).
pub fn render_workbench(scene: &Scene, s: &WorkbenchRender) -> Image {
    let (w, h) = (s.width.max(1), s.height.max(1));
    let mut sc = scene.clone();
    sc.selected.clear();
    let vs = ViewportScene::new(&sc);
    let cam = match scene.active_camera.filter(|&c| c < scene.objects.len()) {
        Some(ci) => {
            let c = match &scene.objects[ci].data {
                crate::scene::ObjectData::Camera(c) => *c,
                _ => crate::scene::Camera::default(),
            };
            RasterCamera::from_camera(&scene.world_matrix(ci), &c, w, h)
        }
        None => RasterCamera::from_view(&super::viewport::ViewCamera::default(), w, h),
    };
    let world = scene.world.color * if scene.world.strength.is_finite() { scene.world.strength.max(0.0) } else { 0.0 };
    let bg = [srgb_encode(world.x.clamp(0.0, 1.0)), srgb_encode(world.y.clamp(0.0, 1.0)), srgb_encode(world.z.clamp(0.0, 1.0))];
    let vst = ViewportSettings { width: w, height: h, wireframe: false, grid: false, backface_culling: s.backface_culling, threads: s.threads, background: Some(bg), workbench: s.shading, transparent: s.film_transparent, overlays: false };
    let n = (w * h) as usize;
    let mut acc = vec![[0.0f32; 4]; n];
    let mut buf = Vec::new();
    let samples = s.samples.max(1);
    for i in 0..samples {
        let (jx, jy) = jitter(i);
        vs.draw_camera(&cam.jittered(jx, jy), None, &vst, &mut buf);
        for (a, p) in acc.iter_mut().zip(buf.chunks_exact(4)) {
            let alpha = p[3] as f32 / 255.0;
            for c in 0..3 {
                a[c] += srgb_decode(p[c] as f32 / 255.0) * alpha;
            }
            a[3] += alpha;
        }
    }
    let inv = 1.0 / samples as f32;
    let pixels = acc
        .into_iter()
        .map(|a| [a[0] * inv, a[1] * inv, a[2] * inv, a[3] * inv])
        .collect();
    Image { name: "Render".into(), width: w, height: h, pixels }
}
