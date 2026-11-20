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
use super::eevee::Lens;
use super::viewport::{ViewCamera, ViewportScene, ViewportSettings};
use crate::math::{Aabb, Vec3};
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
    /// Index into [`STUDIO_LIGHTS`] (the rig of [`Lighting::Studio`]).
    pub studio: usize,
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
    /// Depth of field of the active camera (renders and camera views).
    pub dof: bool,
    /// Encode world-space surface normals as RGB `(n * 0.5 + 0.5)`.
    pub world_normals: bool,
}

impl Default for Workbench {
    /// The look the Solid view always had: studio lighting with specular on
    /// material colours, nothing else.
    fn default() -> Self {
        Workbench {
            lighting: Lighting::Studio,
            studio: 0,
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
            dof: false,
            world_normals: false,
        }
    }
}

impl Workbench {
    /// Blender's defaults for a new 3D view (outline on).
    pub fn viewport_defaults() -> Workbench {
        Workbench { outline: true, ..Workbench::default() }
    }
}

/// High-level viewport inspection presets (Omamesh-style).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadingPreset {
    TexturedStudio,
    ClayMatcap,
    WorldNormals,
    WireframeOverlay,
}

impl ShadingPreset {
    pub const ALL: [ShadingPreset; 4] = [
        ShadingPreset::TexturedStudio,
        ShadingPreset::ClayMatcap,
        ShadingPreset::WorldNormals,
        ShadingPreset::WireframeOverlay,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ShadingPreset::TexturedStudio => "Textured Studio",
            ShadingPreset::ClayMatcap => "Clay Matcap",
            ShadingPreset::WorldNormals => "World Normals",
            ShadingPreset::WireframeOverlay => "Wireframe Overlay",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            ShadingPreset::TexturedStudio => "textured_studio",
            ShadingPreset::ClayMatcap => "clay_matcap",
            ShadingPreset::WorldNormals => "world_normals",
            ShadingPreset::WireframeOverlay => "wireframe_overlay",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let key = name.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match key.as_str() {
            "textured_studio" | "textured" | "studio" => Some(ShadingPreset::TexturedStudio),
            "clay_matcap" | "clay" | "matcap" => Some(ShadingPreset::ClayMatcap),
            "world_normals" | "normals" | "normal" => Some(ShadingPreset::WorldNormals),
            "wireframe_overlay" | "wireframe" | "wire" => Some(ShadingPreset::WireframeOverlay),
            _ => None,
        }
    }

    pub fn shows_wireframe(self) -> bool {
        matches!(self, ShadingPreset::WireframeOverlay)
    }

    pub fn apply_to_workbench(self, wb: &mut Workbench) {
        match self {
            ShadingPreset::TexturedStudio => {
                wb.lighting = Lighting::Studio;
                wb.studio = 0;
                wb.color = ColorType::Texture;
                wb.specular = true;
                wb.shadow = true;
                wb.cavity = false;
                wb.world_normals = false;
            }
            ShadingPreset::ClayMatcap => {
                wb.lighting = Lighting::MatCap;
                wb.matcap = 0;
                wb.color = ColorType::Single;
                wb.single_color = [0.82, 0.78, 0.74];
                wb.specular = false;
                wb.shadow = true;
                wb.cavity = true;
                wb.cavity_type = CavityType::Screen;
                wb.world_normals = false;
            }
            ShadingPreset::WorldNormals => {
                wb.lighting = Lighting::Flat;
                wb.color = ColorType::Material;
                wb.specular = false;
                wb.shadow = false;
                wb.cavity = false;
                wb.world_normals = true;
            }
            ShadingPreset::WireframeOverlay => {
                wb.lighting = Lighting::Studio;
                wb.color = ColorType::Material;
                wb.specular = true;
                wb.world_normals = false;
            }
        }
    }

    pub fn apply_to_viewport(self, settings: &mut ViewportSettings) {
        self.apply_to_workbench(&mut settings.workbench);
        settings.wireframe = self.shows_wireframe();
    }
}

/// Attenuate a ground-plane background pixel (`1.0` = unshadowed, `< 1.0` =
/// shadowed) for a shadow catcher on `z = 0`.
pub fn shadow_catcher_attenuation(world_pos: Vec3, caster_bounds: Aabb, light_dir: Vec3, shadow_lit: f32, intensity: f32) -> f32 {
    if caster_bounds.is_empty() || intensity <= 0.0 || !intensity.is_finite() {
        return 1.0;
    }
    if light_dir.z.is_finite() && light_dir.z <= 0.0 {
        return 1.0;
    }
    let eff_intensity = intensity.clamp(0.0, 1.0);
    let occl = (1.0 - shadow_lit.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let center = caster_bounds.center();
    let extent = caster_bounds.size().length().max(0.5);
    let dist_xy = (world_pos.x - center.x).hypot(world_pos.y - center.y);
    let fade = 1.0 - smoothstep(extent * 5.0, extent * 8.0, dist_xy);
    (1.0 - eff_intensity * occl * fade).clamp(0.0, 1.0)
}

/// State and stepper for turntable inspection around `+Z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurntableState {
    pub active: bool,
    /// Angular speed in radians per second.
    pub angular_speed_rad: f32,
    /// Accumulated turntable angle in radians within `[0, TAU)`.
    pub angle_rad: f32,
}

impl Default for TurntableState {
    fn default() -> Self {
        TurntableState {
            active: false,
            angular_speed_rad: core::f32::consts::TAU / 8.0,
            angle_rad: 0.0,
        }
    }
}

impl TurntableState {
    /// Advance `angle_rad` by `dt_secs` if active and return the signed delta.
    pub fn step(&mut self, dt_secs: f32) -> f32 {
        if !self.active || !dt_secs.is_finite() || dt_secs <= 0.0 || !self.angular_speed_rad.is_finite() {
            return 0.0;
        }
        let delta = self.angular_speed_rad * dt_secs;
        self.angle_rad = (self.angle_rad + delta).rem_euclid(core::f32::consts::TAU);
        delta
    }
}

/// Advance an orbit/turntable yaw angle (radians) by `angular_speed_rad * dt_secs`,
/// wrapping into `(-PI, PI]`.
pub fn step_turntable_angle(angle_rad: f32, angular_speed_rad: f32, dt_secs: f32) -> f32 {
    if !angle_rad.is_finite() || !angular_speed_rad.is_finite() || !dt_secs.is_finite() {
        return angle_rad;
    }
    let next = (angle_rad + angular_speed_rad * dt_secs).rem_euclid(core::f32::consts::TAU);
    if next > core::f32::consts::PI {
        next - core::f32::consts::TAU
    } else {
        next
    }
}

/// Step a [`ViewCamera`]'s turntable orbit around `+Z` by `dt_secs` and return the new yaw.
pub fn step_turntable_camera(cam: &mut ViewCamera, angular_speed_rad: f32, dt_secs: f32) -> f32 {
    cam.yaw = step_turntable_angle(cam.yaw, angular_speed_rad, dt_secs);
    cam.yaw
}

/// Names of the studio light rigs (Blender's Studio lighting presets).
pub const STUDIO_LIGHTS: [&str; 4] = ["Default", "Basic", "Outdoor", "Rim Light"];

/// A studio rig in view space: key and fill directions, then the weights of
/// ambient, facing ratio, key, fill, rim and specular.
struct Rig {
    key: Vec3,
    fill: Vec3,
    w: [f32; 6],
}

const RIGS: [Rig; 4] = [
    Rig { key: Vec3 { x: -0.408, y: 0.571, z: 0.712 }, fill: Vec3 { x: 0.6, y: -0.48, z: 0.64 }, w: [0.22, 0.42, 0.38, 0.12, 0.12, 0.18] },
    Rig { key: Vec3 { x: 0.0, y: 0.6, z: 0.8 }, fill: Vec3 { x: -0.8, y: 0.0, z: 0.6 }, w: [0.3, 0.3, 0.4, 0.15, 0.05, 0.12] },
    Rig { key: Vec3 { x: 0.48, y: 0.8, z: 0.36 }, fill: Vec3 { x: 0.0, y: 1.0, z: 0.0 }, w: [0.25, 0.15, 0.62, 0.2, 0.06, 0.25] },
    Rig { key: Vec3 { x: -0.3, y: 0.3, z: 0.906 }, fill: Vec3 { x: 0.0, y: -0.6, z: 0.8 }, w: [0.15, 0.4, 0.3, 0.1, 0.45, 0.15] },
];

/// Solid-mode studio lighting in view space for unit `n` and `v` facing the
/// viewer: (factor on the base colour, white added on top).
#[inline]
pub(crate) fn studio_terms(n: Vec3, v: Vec3, wb: &Workbench) -> (f32, f32) {
    let r = &RIGS[wb.studio.min(RIGS.len() - 1)];
    let w = r.w;
    let nv = n.dot(v).max(0.0);
    let diffuse = w[0] + w[1] * nv + w[2] * n.dot(r.key).max(0.0) + w[3] * n.dot(r.fill).max(0.0);
    let spec = if wb.specular {
        let h = (r.key + v).normalize();
        w[5] * n.dot(h).max(0.0).powi(32)
    } else {
        0.0
    };
    let rim = w[4] * (1.0 - nv).powi(4);
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
        None => RasterCamera::from_view(&ViewCamera::default(), w, h),
    };
    let world = scene.world.color * if scene.world.strength.is_finite() { scene.world.strength.max(0.0) } else { 0.0 };
    let bg = [srgb_encode(world.x.clamp(0.0, 1.0)), srgb_encode(world.y.clamp(0.0, 1.0)), srgb_encode(world.z.clamp(0.0, 1.0))];
    let vst = ViewportSettings { width: w, height: h, wireframe: false, grid: false, backface_culling: s.backface_culling, threads: s.threads, background: Some(bg), workbench: s.shading, transparent: s.film_transparent, overlays: false, ..ViewportSettings::default() };
    let lens = scene.active_camera.filter(|_| s.shading.dof).and_then(|ci| Lens::from_camera(scene, ci, w, h));
    let mut frames = WorkbenchFrames::default();
    for _ in 0..s.samples.max(1) {
        frames.add_sample(&vs, &cam, None, &vst, lens);
    }
    let inv = 1.0 / frames.samples as f32;
    let pixels = frames.acc.iter().map(|a| a.map(|c| c * inv)).collect();
    Image { name: "Render".into(), width: w, height: h, pixels }
}

/// Workbench anti-aliasing (and depth of field) one frame at a time: the
/// jittered frames [`render_workbench`] averages, for a progressive view.
#[derive(Default)]
pub struct WorkbenchFrames {
    /// Sums of linear premultiplied RGBA.
    acc: Vec<[f32; 4]>,
    buf: Vec<u8>,
    samples: u32,
}

impl WorkbenchFrames {
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Start over (the view or the scene changed).
    pub fn reset(&mut self) {
        self.samples = 0;
    }

    /// Draw and add the next jittered frame of `cam` (`view` places the
    /// floor grid when `st` shows it; `lens` adds depth of field).
    pub fn add_sample(&mut self, vs: &ViewportScene, cam: &RasterCamera, view: Option<&ViewCamera>, st: &ViewportSettings, lens: Option<Lens>) {
        let n = (st.width.max(1) * st.height.max(1)) as usize;
        if self.samples == 0 || self.acc.len() != n {
            self.acc.clear();
            self.acc.resize(n, [0.0; 4]);
            self.samples = 0;
        }
        let i = self.samples;
        let (jx, jy) = jitter(i);
        let mut c = cam.jittered(jx, jy);
        if let Some(l) = lens {
            l.jitter(&mut c, i);
        }
        vs.draw_camera(&c, view, st, &mut self.buf);
        for (a, p) in self.acc.iter_mut().zip(self.buf.chunks_exact(4)) {
            let alpha = p[3] as f32 / 255.0;
            for c in 0..3 {
                a[c] += srgb_decode(p[c] as f32 / 255.0) * alpha;
            }
            a[3] += alpha;
        }
        self.samples += 1;
    }

    /// The average so far as display RGBA8 (straight alpha, sRGB).
    pub fn rgba8(&self, out: &mut Vec<u8>) {
        if self.samples == 1 && self.buf.len() == self.acc.len() * 4 {
            out.clone_from(&self.buf);
            return;
        }
        let inv = 1.0 / self.samples.max(1) as f32;
        out.clear();
        out.reserve(self.acc.len() * 4);
        for a in &self.acc {
            let alpha = a[3] * inv;
            let k = if alpha > 0.0 { inv / alpha } else { 0.0 };
            for c in 0..3 {
                out.push((srgb_encode((a[c] * k).clamp(0.0, 1.0)) * 255.0 + 0.5) as u8);
            }
            out.push((alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shading_preset_roundtrip_and_apply() {
        for preset in ShadingPreset::ALL {
            assert_eq!(ShadingPreset::from_name(preset.id()), Some(preset));
            assert_eq!(ShadingPreset::from_name(preset.label()), Some(preset));
        }

        let mut st = ViewportSettings::default();
        ShadingPreset::ClayMatcap.apply_to_viewport(&mut st);
        assert_eq!(st.workbench.lighting, Lighting::MatCap);
        assert_eq!(st.workbench.matcap, 0);
        assert_eq!(st.workbench.color, ColorType::Single);
        assert!(!st.wireframe);
        assert!(!st.workbench.world_normals);

        ShadingPreset::WorldNormals.apply_to_viewport(&mut st);
        assert!(st.workbench.world_normals);
        assert!(!st.wireframe);

        ShadingPreset::WireframeOverlay.apply_to_viewport(&mut st);
        assert!(st.wireframe);
        assert!(!st.workbench.world_normals);

        ShadingPreset::TexturedStudio.apply_to_viewport(&mut st);
        assert_eq!(st.workbench.lighting, Lighting::Studio);
        assert_eq!(st.workbench.color, ColorType::Texture);
        assert!(!st.wireframe);
    }

    #[test]
    fn shadow_catcher_attenuation_behaves_physically() {
        let bounds = Aabb { min: Vec3::new(-1.0, -1.0, 0.0), max: Vec3::new(1.0, 1.0, 2.0) };
        let ldir = Vec3::new(0.5, -0.5, 0.7).normalize();
        assert!((shadow_catcher_attenuation(Vec3::ZERO, bounds, ldir, 1.0, 0.55) - 1.0).abs() < 1e-6);
        let dark = shadow_catcher_attenuation(Vec3::ZERO, bounds, ldir, 0.0, 0.55);
        assert!((dark - 0.45).abs() < 1e-4, "dark={dark}");
        assert_eq!(shadow_catcher_attenuation(Vec3::new(1000.0, 0.0, 0.0), bounds, ldir, 0.0, 0.55), 1.0);
        assert_eq!(shadow_catcher_attenuation(Vec3::ZERO, Aabb::EMPTY, ldir, 0.0, 0.55), 1.0);
    }

    #[test]
    fn turntable_stepper_advances_and_wraps() {
        let mut cam = ViewCamera::default();
        let start = cam.yaw;
        let next = step_turntable_camera(&mut cam, 1.0, 0.25);
        assert!((next - (start + 0.25)).abs() < 1e-5);
        assert_eq!(cam.yaw, next);

        let wrapped = step_turntable_angle(3.0, 1.0, 1.0);
        assert!(wrapped <= core::f32::consts::PI && wrapped > -core::f32::consts::PI);

        let mut ts = TurntableState { active: true, angular_speed_rad: core::f32::consts::FRAC_PI_2, angle_rad: 0.0 };
        let delta = ts.step(1.0);
        assert!((delta - core::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((ts.angle_rad - core::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }
}

