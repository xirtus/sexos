// SPDX-License-Identifier: GPL-3.0-only
//! "EEVEE": a deferred CPU rasterizer that shades the material node trees
//! per pixel, Polyamour's real-time render engine (Material Preview,
//! Rendered shading with the EEVEE engine, and F12).
//!
//! One *sample* of a frame:
//!
//! 1. **Visibility**: the opaque triangles (objects outside the view
//!    frustum skipped) go through the shared tiled rasterizer (`raster`)
//!    into a visibility buffer: depth and triangle id per pixel, with a
//!    sub-pixel jitter.
//! 2. **G-buffer normals** and **ambient occlusion** (GTAO-lite: horizon
//!    search over the depth buffer in a few rotated directions).
//!    Materials with the Dithered render method drop pixels where a hash
//!    exceeds their opacity (order-independent stochastic transparency);
//!    materials with backface culling skip faces turned away.
//! 3. **Deferred shading**: per pixel the triangle's barycentrics rebuild
//!    the shading point and the material's compiled node program runs
//!    once (no random sampling); its closure lobes are lit analytically:
//!    every light through the lobes' BSDFs with shadow maps (cascaded for
//!    sun, a cube for point and area lights, a frustum for spots;
//!    percentage-closer filtered, softened by the light size) and optional
//!    contact shadows (a screen-space ray to the light); area lights are
//!    integrated over their shape (`area`); the world comes through a
//!    prefiltered probe (split sum: GGX lobes read the roughness-filtered
//!    environment times their directional albedo, diffuse lobes the
//!    spherical-harmonics irradiance), with occlusion. Emission is added;
//!    holdout cuts alpha.
//! 4. **Screen-space passes** (`post`): subsurface scattering blurs the
//!    subsurface share of the diffuse light over the surface; screen-space
//!    reflections replace the probe in glossy lobes by what a ray through
//!    the depth buffer hits.
//! 5. **Volumetrics**: objects with volume materials are integrated along
//!    each pixel's ray between their front and back faces (ray marched,
//!    lit by the lights, optionally shadowed); a homogeneous world volume
//!    fogs every pixel (absorption, in-scattering of the probe and of sun
//!    lights).
//! 6. **Transparency** ("blended"): triangles of materials that let light
//!    through are sorted back to front, depth tested against the opaque
//!    surfaces and composited (`dst * transparency + radiance`); materials
//!    with raytraced transmission refract the opaque image behind them.
//! 7. **Depth of field** (a gather filter by circle of confusion, or
//!    jittering the camera over the aperture per sample) and **bloom**.
//! 8. **Film**: samples accumulate with jittered sub-pixel offsets, each
//!    weighted by the pixel filter, while the view stays still (EEVEE's
//!    progressive TAA samples); noise in the soft shadows and AO rotates
//!    per sample and averages out. With motion blur every sample places
//!    moving objects at another time in the shutter.
//!
//! Material Preview ("look dev") replaces the scene's world and lights by
//! our procedural studio environment, fixed to the view.
mod area;
mod post;
mod probe;

use super::bsdf::{fresnel_dielectric, ClosureBsdf, LobeKind};
use super::bvh::TriGeom;
use super::camera::MotionChain;
use super::prepare::{LightShape, PreparedScene, TriShade};
use super::raster::{raster_tile, split_ranges, Bins, Layout, RasterCamera, NO_ID, TILE, TILE_PX};
use super::shader::surface::{run, surface_ctx};
use super::shader::RayState;
use super::shadow::ShadowMap;
use super::volume::VolumeCoeffs;
use super::workbench::jitter;
use super::{luminance, parallel_for_each, resolve_threads};
use crate::math::{Aabb, Mat4, Vec3};
use crate::scene::{Image, ObjectData, RenderMethod, Scene};
use core::f32::consts::{FRAC_1_PI, PI, TAU};
use post::{SpecPx, SssPx};
pub(crate) use probe::Probe;

/// A thin lens for depth of field (from the active camera's settings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Aperture radius in metres (focal length / (2 f-stop)).
    pub aperture: f32,
    /// Focus distance along the view axis (metres).
    pub focus: f32,
}

impl Lens {
    /// The depth of field of camera object `ci` in `scene` for a `w` x `h`
    /// frame (None when off or not a perspective/orthographic camera).
    pub fn from_camera(scene: &Scene, ci: usize, w: u32, h: u32) -> Option<Lens> {
        let ObjectData::Camera(c) = &scene.objects.get(ci)?.data else { return None };
        let dof = &c.dof;
        if !dof.enabled || c.panorama.is_some() || !(dof.fstop.is_finite() && dof.fstop > 0.0) {
            return None;
        }
        let m = scene.world_matrix(ci);
        let eye = m.transform_point(Vec3::ZERO);
        let fwd = -m.transform_vector(Vec3::Z).normalize();
        let focus = match dof.focus_object.filter(|&f| f < scene.objects.len()) {
            Some(f) => (scene.world_matrix(f).transform_point(Vec3::ZERO) - eye).dot(fwd),
            None => dof.focus_distance,
        };
        let aspect = w.max(1) as f32 / h.max(1) as f32;
        let aperture = c.focal_length(aspect) * 1e-3 / (2.0 * dof.fstop);
        (aperture.is_finite() && aperture > 0.0).then_some(Lens { aperture, focus: if focus.is_finite() { focus.max(1e-3) } else { 10.0 } })
    }

    /// Move `view` to point `i` of a sequence over the aperture, keeping the
    /// focus plane in place (depth of field by averaging such views).
    pub fn jitter(&self, view: &mut RasterCamera, i: u32) {
        if view.ortho {
            return;
        }
        let (u, v) = (radical_inverse2(i.wrapping_add(1)), unit(hash2(i, 0x6a09_e667, 7)));
        let (r, a) = (u.sqrt() * self.aperture, v * TAU);
        let (ax, ay) = (r * a.cos(), r * a.sin());
        view.eye = view.eye + view.right * ax + view.up * ay;
        view.cx += ax * view.scale / self.focus;
        view.cy -= ay * view.scale / self.focus;
    }
}

/// EEVEE's Render Properties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EeveeSettings {
    pub width: u32,
    pub height: u32,
    /// Samples to accumulate (anti-aliasing, soft shadows, AO).
    pub samples: u32,
    pub shadows: bool,
    /// Shadow map size (cascades and cube faces).
    pub shadow_resolution: u32,
    /// Soften shadows by the lights' radius / angle.
    pub soft_shadows: bool,
    /// Sun cascade count (1..4).
    pub shadow_cascades: u32,
    pub ao: bool,
    /// Ambient occlusion distance (world units).
    pub ao_distance: f32,
    pub volumetrics: bool,
    /// Distance the volume integrates to past the last surface (background).
    pub volumetric_end: f32,
    /// Ray-marching steps through object volumes.
    pub volumetric_steps: u32,
    /// Lights in object volumes are shadowed by the shadow maps.
    pub volumetric_shadows: bool,
    /// Raytracing: screen-space reflections, and refraction for materials
    /// with raytraced transmission.
    pub ssr: bool,
    /// Rougher surfaces keep the probe's reflection.
    pub ssr_max_roughness: f32,
    /// Assumed thickness of surfaces in the depth buffer (metres).
    pub ssr_thickness: f32,
    /// Fade near the screen edges (fraction of the frame).
    pub ssr_fade: f32,
    pub bloom: bool,
    pub bloom_threshold: f32,
    pub bloom_knee: f32,
    pub bloom_radius: f32,
    pub bloom_color: [f32; 3],
    pub bloom_intensity: f32,
    /// Clamp the bloom's source (0 = off).
    pub bloom_clamp: f32,
    /// Largest depth-of-field blur, in pixels (diameter).
    pub dof_max_size: f32,
    /// Depth of field by moving the camera over the aperture per sample
    /// (exact with enough samples) instead of the gather filter.
    pub dof_jitter: bool,
    /// Subsurface scattering taps per pixel and sample.
    pub sss_samples: u32,
    /// Motion blur: the shutter interval in frames around the current one.
    pub motion_blur: Option<(f32, f32)>,
    /// The camera's depth of field (None: pinhole).
    pub lens: Option<Lens>,
    /// Pixel filter width in pixels.
    pub filter_size: f32,
    pub film_transparent: bool,
    /// Material Preview: our studio environment, no scene lights or world.
    pub lookdev: bool,
    /// Background blur in look dev (0..1 roughness).
    pub lookdev_blur: f32,
    /// Studio world for look dev (index into [`probe::STUDIO_WORLDS`]).
    pub lookdev_world: usize,
    /// Rotation of the look-dev world about the view's up axis (radians).
    pub lookdev_rotation: f32,
    /// Look-dev world strength.
    pub lookdev_strength: f32,
    /// How much of the look-dev world shows behind (0 = the plain grey of
    /// the scene world colour, 1 = the environment).
    pub lookdev_opacity: f32,
    /// 0 = all cores.
    pub threads: usize,
}

impl Default for EeveeSettings {
    fn default() -> Self {
        EeveeSettings {
            width: 640,
            height: 360,
            samples: 16,
            shadows: true,
            shadow_resolution: 1024,
            soft_shadows: true,
            shadow_cascades: 3,
            ao: true,
            ao_distance: 0.2,
            volumetrics: true,
            volumetric_end: 100.0,
            volumetric_steps: 32,
            volumetric_shadows: false,
            ssr: false,
            ssr_max_roughness: 0.5,
            ssr_thickness: 0.2,
            ssr_fade: 0.075,
            bloom: false,
            bloom_threshold: 0.8,
            bloom_knee: 0.5,
            bloom_radius: 6.5,
            bloom_color: [1.0; 3],
            bloom_intensity: 0.05,
            bloom_clamp: 0.0,
            dof_max_size: 100.0,
            dof_jitter: false,
            sss_samples: 7,
            motion_blur: None,
            lens: None,
            filter_size: 1.5,
            film_transparent: false,
            lookdev: false,
            lookdev_blur: 0.5,
            lookdev_world: 0,
            lookdev_rotation: 0.0,
            lookdev_strength: 1.0,
            lookdev_opacity: 1.0,
            threads: 0,
        }
    }
}

pub use probe::STUDIO_WORLDS;

/// A light as the rasterizer lights with it.
#[derive(Clone, Debug)]
struct RLight {
    shape: LightShape,
    shadow: bool,
    /// Light size for soft shadows (radius, or the sun's angle in radians).
    size: f32,
    /// Contact shadow distance and thickness.
    contact: Option<(f32, f32)>,
}

/// How a material's surfaces go through the passes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct MatFlags {
    /// Sorted, blended pass (transparent "Blended", or refracting).
    blended: bool,
    /// Opaque pass with hashed transparency.
    dithered: bool,
    /// Volume only: no surface, integrated by the volume pass.
    volume: bool,
    cull: bool,
    cull_shadow: bool,
    /// Shadows: 0 none, 1 opaque, 2 hashed by the transparency.
    shadow: u8,
    /// Screen-space refraction, with this thickness.
    refract: Option<f32>,
    sss: bool,
}

/// An object whose triangles move during the shutter.
#[derive(Clone, Debug)]
struct Moving {
    /// Index into [`EeveeScene::objects`].
    object: usize,
    chain: MotionChain,
    /// World matrix at the frame (the triangles' rest pose) inverted.
    inv0: Mat4,
    rest: Vec<TriGeom>,
    rest_n: Vec<[Vec3; 3]>,
}

/// An object filled with a homogeneous volume.
#[derive(Clone, Debug)]
struct VolumeObject {
    range: (usize, usize),
    bounds: Aabb,
    coeffs: VolumeCoeffs,
}

/// A scene prepared for EEVEE (rebuild when the scene changes).
pub struct EeveeScene {
    prep: PreparedScene,
    tris: Vec<TriGeom>,
    shade: Vec<TriShade>,
    /// Per object: triangle range and world bounds.
    objects: Vec<(usize, usize, Aabb)>,
    /// Per material: how it renders.
    flags: Vec<MatFlags>,
    any_hashed_shadow: bool,
    any_sss: bool,
    lights: Vec<RLight>,
    moving: Vec<Moving>,
    volumes: Vec<VolumeObject>,
    probe: Probe,
    /// The background as seen by the camera (None: use `prep.env`).
    background: Option<probe::EqMap>,
    lookdev: bool,
    /// The look-dev world the probe holds (index, rotation, strength).
    lookdev_world: (usize, u32, u32),
    /// The scene world colour (look dev's background at zero opacity).
    world_color: Vec3,
    bounds: Aabb,
}

impl EeveeScene {
    /// Prepare `scene`; with `lookdev` the world and lights are replaced by
    /// the studio environment.
    pub fn new(scene: &Scene, lookdev: bool) -> EeveeScene {
        let threads = resolve_threads(0);
        let (prep, geo) = PreparedScene::new_raster(scene);
        let mut objects = Vec::with_capacity(geo.ranges.len());
        let mut bounds = Aabb::EMPTY;
        for &(_, a, b) in &geo.ranges {
            let bb = tri_bounds(&geo.tris[a..b]);
            bounds = bounds.union(bb);
            objects.push((a, b, bb));
        }
        // Material flags.
        let flags: Vec<MatFlags> = prep
            .materials
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let info = m.program.as_ref().map(|p| (p.info, p.surface.is_none() && p.volume.is_some())).unwrap_or_default();
                let ee = scene.materials.get(i).map(|m| m.eevee).unwrap_or_default();
                let (info, volume_only) = info;
                let mut f = MatFlags { cull: ee.backface_culling, cull_shadow: ee.backface_culling_shadow, sss: info.subsurface, ..MatFlags::default() };
                if volume_only {
                    f.volume = true;
                    return f;
                }
                f.shadow = 1;
                if info.transparent {
                    match ee.render_method {
                        RenderMethod::Dithered => f.dithered = true,
                        RenderMethod::Blended => f.blended = true,
                    }
                    if ee.transparent_shadows {
                        f.shadow = 2;
                    }
                }
                if ee.screen_refraction {
                    f.blended = true;
                    f.dithered = false;
                    f.refract = Some(ee.refraction_depth.max(0.0));
                }
                f
            })
            .collect();
        let any_hashed_shadow = flags.iter().any(|f| f.shadow == 2);
        let any_sss = flags.iter().any(|f| f.sss);
        let mut lights = Vec::new();
        if !lookdev {
            let light_objs: Vec<usize> = (0..scene.objects.len()).filter(|&i| matches!(scene.objects[i].data, ObjectData::Light(_))).collect();
            for (k, l) in prep.lights.iter().enumerate() {
                let info = prep.light_info.get(k);
                let li = info.and_then(|i| light_objs.iter().find(|&&o| o as u32 == i.object)).and_then(|&o| match &scene.objects[o].data {
                    ObjectData::Light(li) => Some(*li),
                    _ => None,
                });
                let size = match (&li, l) {
                    (Some(_), LightShape::Area { u, v, .. }) => u.length().max(v.length()),
                    (Some(li), _) => li.radius.max(0.0),
                    _ => 0.0,
                };
                let contact = li.filter(|li| li.contact_shadow && li.contact_shadow_distance > 0.0).map(|li| (li.contact_shadow_distance, li.contact_shadow_thickness.max(1e-4)));
                if matches!(l, LightShape::Point { .. } | LightShape::Sun { .. } | LightShape::Area { .. }) {
                    lights.push(RLight { shape: l.clone(), shadow: info.is_none_or(|i| i.cast_shadow), size, contact });
                }
            }
        }
        // Moving objects (motion blur) and volume objects.
        let mut moving = Vec::new();
        let mut volumes = Vec::new();
        for (k, &(oi, a, b)) in geo.ranges.iter().enumerate() {
            if b <= a {
                continue;
            }
            let mat = geo.shade[a].material as usize;
            if flags.get(mat).is_some_and(|f| f.volume) {
                if let Some(Some(c)) = prep.material_volume.get(mat) {
                    volumes.push(VolumeObject { range: (a, b), bounds: objects[k].2, coeffs: *c });
                }
            }
            let oi = oi as usize;
            if oi < scene.objects.len() && scene.has_motion(oi) {
                let chain = MotionChain::from_scene(scene, oi);
                moving.push(Moving { object: k, inv0: chain.matrix(0.0).inverse(), chain, rest: geo.tris[a..b].to_vec(), rest_n: geo.shade[a..b].iter().map(|s| s.n).collect() });
            }
        }
        let (probe, background) = if lookdev {
            (studio_probe(0, 0.0, 1.0, threads), None)
        } else {
            let env = &prep.env;
            let probe = Probe::build(threads, &|d| env.eval_ray(d, RayState::camera()));
            let constant = env.is_constant();
            let bg = (!constant).then(|| probe::EqMap::capture(1024, 512, threads, &|d| env.eval_ray(d, RayState::camera())));
            (probe, bg)
        };
        let world_color = scene.world.color * if scene.world.strength.is_finite() { scene.world.strength.max(0.0) } else { 0.0 };
        EeveeScene { tris: geo.tris, shade: geo.shade, objects, flags, any_hashed_shadow, any_sss, lights, moving, volumes, probe, background, lookdev, lookdev_world: (0, 0, 1f32.to_bits()), world_color, bounds, prep }
    }

    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    #[inline]
    fn flags(&self, tri: usize) -> MatFlags {
        self.flags.get(self.shade[tri].material as usize).copied().unwrap_or_default()
    }

    /// Opacity (1 - transparency) of triangle `src` at barycentrics `b`
    /// seen along `dir`.
    fn opacity(&self, src: usize, b: [f32; 3], dir: Vec3, seed: u32) -> f32 {
        let (g, sh) = (&self.tris[src], &self.shade[src]);
        let mats = &self.prep.materials;
        let Some(prog) = mats.get(sh.material as usize).and_then(|m| m.program.as_ref()) else { return 1.0 };
        let mut ctx = surface_ctx(&self.prep, prog, g, sh, b[1], b[2], dir, 1.0, RayState::camera(), seed);
        let c = run(prog, &mut ctx);
        let t = c.transparent.max(Vec3::ZERO).min(Vec3::ONE);
        1.0 - (t.x + t.y + t.z) / 3.0
    }

    /// Place the moving objects at `time` (frames from the current one).
    fn set_time(&mut self, time: f32) {
        for m in &self.moving {
            let mt = m.chain.matrix(time) * m.inv0;
            let nm = mt.inverse().transpose();
            let (a, b, _) = self.objects[m.object];
            for (i, (g, n)) in m.rest.iter().zip(&m.rest_n).enumerate() {
                let (p0, p1, p2) = (mt.transform_point(g.v0), mt.transform_point(g.v0 + g.e1), mt.transform_point(g.v0 + g.e2));
                self.tris[a + i] = TriGeom::new(p0, p1, p2);
                self.shade[a + i].n = n.map(|v| nm.transform_vector(v).normalize());
            }
            let bb = tri_bounds(&self.tris[a..b]);
            self.objects[m.object].2 = bb;
            self.bounds = self.bounds.union(bb);
        }
        for v in &mut self.volumes {
            v.bounds = tri_bounds(&self.tris[v.range.0..v.range.1]);
        }
    }
}

fn tri_bounds(tris: &[TriGeom]) -> Aabb {
    let mut bb = Aabb::EMPTY;
    for t in tris {
        bb.grow(t.v0);
        bb.grow(t.v0 + t.e1);
        bb.grow(t.v0 + t.e2);
    }
    bb
}

/// The look-dev probe for studio world `world`, turned by `rotation` and
/// scaled by `strength` (the default one is cached).
fn studio_probe(world: usize, rotation: f32, strength: f32, threads: usize) -> Probe {
    static DEFAULT: std::sync::OnceLock<Probe> = std::sync::OnceLock::new();
    if world == 0 && rotation == 0.0 && strength == 1.0 {
        return DEFAULT.get_or_init(|| Probe::build(threads, &|d| probe::studio_world(0, d))).clone();
    }
    let (s, c) = rotation.sin_cos();
    Probe::build(threads, &|d| {
        // Turn about the view's up axis (y in view space).
        let r = Vec3::new(c * d.x + s * d.z, d.y, -s * d.x + c * d.z);
        probe::studio_world(world, r) * strength
    })
}

/// Blackman-Harris pixel filter weight at offset `r` (pixels) for width `w`.
fn filter_weight(jx: f32, jy: f32, width: f32) -> f32 {
    let r = (jx * jx + jy * jy).sqrt() / (width.max(1e-3) * 0.5);
    if r >= 1.0 {
        return 0.0;
    }
    let x = PI * (1.0 + r);
    (0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()).max(0.0)
}

#[inline]
fn hash2(x: u32, y: u32, s: u32) -> u32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ s.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h
}

#[inline]
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Van der Corput radical inverse in base 2 (shutter times, lens samples).
fn radical_inverse2(i: u32) -> f32 {
    i.reverse_bits() as f32 * 2.328_306_4e-10
}

/// Shadow maps of one light.
#[derive(Clone, Debug)]
enum LightShadow {
    None,
    /// Cascades: (far view depth, map).
    Sun(Vec<(f32, ShadowMap)>),
    Spot(ShadowMap),
    Cube(Box<[ShadowMap; 6]>),
}

/// Per-frame working memory.
#[derive(Default)]
struct Scratch {
    bins: Bins<u32>,
    tbins: Bins<u32>,
    vbins: Bins<u32>,
    shadow_bins: Bins<u32>,
    depth: Vec<f32>,
    ids: Vec<u32>,
    normals: Vec<Vec3>,
    ao: Vec<f32>,
    color: Vec<[f32; 4]>,
    /// The opaque surfaces' colour (reflections and refraction read it).
    lit: Vec<[f32; 4]>,
    spec: Vec<SpecPx>,
    sss: Vec<SssPx>,
    post: Vec<[f32; 4]>,
    bloom: Vec<post::Img>,
    transparent_order: Vec<(f32, u32)>,
}

/// An EEVEE renderer: one scene and one view, accumulating samples.
pub struct Eevee {
    scene: EeveeScene,
    settings: EeveeSettings,
    cam: Option<RasterCamera>,
    accum: Vec<[f32; 4]>,
    weight: f32,
    samples: u32,
    shadows: Vec<LightShadow>,
    /// The view the sun cascades were fitted for.
    cascade_view: Option<RasterCamera>,
    /// Point/spot/area maps depend only on the scene.
    static_shadows: bool,
    /// Where the moving objects are (frames from the current one).
    time: f32,
    scratch: Scratch,
}

impl Eevee {
    pub fn new(scene: EeveeScene, settings: &EeveeSettings) -> Eevee {
        let mut e = Eevee { scene, settings: *settings, cam: None, accum: Vec::new(), weight: 0.0, samples: 0, shadows: Vec::new(), cascade_view: None, static_shadows: false, time: 0.0, scratch: Scratch::default() };
        e.update_lookdev_world();
        e
    }

    pub fn scene(&self) -> &EeveeScene {
        &self.scene
    }

    /// Change settings; returns true (and restarts accumulation) when they differ.
    pub fn set_settings(&mut self, s: &EeveeSettings) -> bool {
        if *s == self.settings {
            return false;
        }
        if s.shadow_resolution != self.settings.shadow_resolution || s.shadow_cascades != self.settings.shadow_cascades || s.shadows != self.settings.shadows {
            self.static_shadows = false;
            self.cascade_view = None;
        }
        self.settings = *s;
        self.update_lookdev_world();
        self.reset();
        true
    }

    /// Rebuild the look-dev probe when its world, rotation or strength changed.
    fn update_lookdev_world(&mut self) {
        let st = &self.settings;
        let key = (st.lookdev_world, st.lookdev_rotation.to_bits(), st.lookdev_strength.max(0.0).to_bits());
        if self.scene.lookdev && self.scene.lookdev_world != key {
            self.scene.probe = studio_probe(st.lookdev_world, st.lookdev_rotation, st.lookdev_strength.max(0.0), resolve_threads(st.threads));
            self.scene.lookdev_world = key;
        }
    }

    /// Look through `cam` (restarts accumulation when the view changed).
    pub fn set_view(&mut self, cam: RasterCamera) {
        if self.cam != Some(cam) {
            self.cam = Some(cam);
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.samples = 0;
        self.weight = 0.0;
        self.accum.clear();
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Whether more samples are wanted.
    pub fn wants_samples(&self) -> bool {
        self.samples < self.settings.samples.max(1)
    }

    /// Render one more sample (no-op without a view). Returns whether more are wanted.
    pub fn add_sample(&mut self) -> bool {
        let Some(cam) = self.cam else { return false };
        let (w, h) = (cam.width as usize, cam.height as usize);
        if w == 0 || h == 0 || !self.wants_samples() {
            return false;
        }
        let layout = Layout::new(w, h);
        let s = self.samples;
        let (jx, jy) = jitter(s);
        let fs = self.settings.filter_size.max(0.0);
        let (jx, jy) = (jx * fs, jy * fs);
        let wgt = if s == 0 { 1.0 } else { filter_weight(jx, jy, fs.max(1.0)) };
        // Motion blur: this sample's time in the shutter.
        let time = match self.settings.motion_blur {
            Some((t0, t1)) if !self.scene.moving.is_empty() && t1 > t0 => t0 + (t1 - t0) * radical_inverse2(s).clamp(0.0, 1.0),
            _ => 0.0,
        };
        if time != self.time {
            self.scene.set_time(time);
            self.time = time;
            self.static_shadows = false;
            self.cascade_view = None;
        }
        self.update_shadows(&cam);
        let mut view = cam.jittered(jx, jy);
        // Jittered camera depth of field: a point on the aperture per
        // sample, the focus plane kept in place.
        if let (Some(l), true) = (self.settings.lens, self.settings.dof_jitter) {
            l.jitter(&mut view, s);
        }
        self.render_sample(&view, s);
        if self.accum.len() != layout.len() {
            self.accum = vec![[0.0; 4]; layout.len()];
            self.weight = 0.0;
        }
        for (a, c) in self.accum.iter_mut().zip(&self.scratch.color) {
            for k in 0..4 {
                a[k] += c[k] * wgt;
            }
        }
        self.weight += wgt;
        self.samples += 1;
        self.wants_samples()
    }

    /// The current image: linear RGBA, alpha premultiplied.
    pub fn image(&self) -> Image {
        let Some(cam) = self.cam else { return Image::default() };
        let (w, h) = (cam.width as usize, cam.height as usize);
        let layout = Layout::new(w, h);
        let inv = if self.weight > 0.0 { 1.0 / self.weight } else { 0.0 };
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let a = self.accum.get(layout.index(x, y)).copied().unwrap_or([0.0; 4]);
                pixels.push([a[0] * inv, a[1] * inv, a[2] * inv, a[3] * inv]);
            }
        }
        Image { name: "Render".into(), width: w as u32, height: h as u32, pixels }
    }

    // ------------------------------------------------------------ shadows

    fn update_shadows(&mut self, cam: &RasterCamera) {
        let st = self.settings;
        let threads = resolve_threads(st.threads);
        if !st.shadows || self.scene.tris.is_empty() {
            self.shadows = vec![LightShadow::None; self.scene.lights.len()];
            return;
        }
        let res = st.shadow_resolution.clamp(64, 8192);
        let sc = &self.scene;
        // Hashed (transparent) shadows change per sample.
        let sample = self.samples;
        if sc.any_hashed_shadow {
            self.static_shadows = false;
            self.cascade_view = None;
        }
        // Triangles that cast for a light camera: (corners, hashed).
        let tri = |c: &RasterCamera, i: usize| {
            let g = &sc.tris[i];
            let f = sc.flags(i);
            if f.shadow == 0 {
                return None;
            }
            if f.cull_shadow {
                let ng = g.e1.cross(g.e2);
                let to_light = if c.ortho { c.back } else { c.eye - g.v0 };
                if ng.dot(to_light) < 0.0 {
                    return None;
                }
            }
            Some(([g.v0, g.v0 + g.e1, g.v0 + g.e2], f.shadow == 2))
        };
        let keep = |c: &RasterCamera, src: u32, b: [f32; 3], x: usize, y: usize| {
            let dir = if c.ortho { -c.back } else { c.view_dir(x, y) };
            let dir = c.right * dir.x + c.up * dir.y + c.back * dir.z;
            let seed = hash2(x as u32, y as u32, sample.wrapping_mul(0x9e37_79b9) ^ sc.shade[src as usize].object);
            sc.opacity(src as usize, b, dir, seed) > unit(seed)
        };
        let render = |c: RasterCamera, bins: &mut Bins<u32>| ShadowMap::render_masked(c, threads, sc.tris.len(), bins, |i| tri(&c, i), |src, b, x, y| keep(&c, src, b, x, y));
        if !self.static_shadows || self.shadows.len() != sc.lights.len() {
            let bins = &mut self.scratch.shadow_bins;
            self.shadows = sc
                .lights
                .iter()
                .map(|l| {
                    if !l.shadow {
                        return LightShadow::None;
                    }
                    match &l.shape {
                        LightShape::Point { pos, spot: Some((axis, cos_half, _)), .. } if *cos_half > 0.5 => {
                            let fov = 2.0 * cos_half.clamp(-1.0, 1.0).acos() + 0.1;
                            LightShadow::Spot(render(persp_camera(*pos, *axis, fov.min(3.0), res, sc.bounds), bins))
                        }
                        LightShape::Point { pos, .. } | LightShape::Area { center: pos, .. } => {
                            let faces = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
                            let maps: [ShadowMap; 6] = core::array::from_fn(|k| render(persp_camera(*pos, faces[k], PI * 0.5, res / 2, sc.bounds), bins));
                            LightShadow::Cube(Box::new(maps))
                        }
                        _ => LightShadow::None,
                    }
                })
                .collect();
            self.static_shadows = true;
            self.cascade_view = None;
        }
        // Sun cascades follow the view (ignoring the jitter).
        if self.cascade_view != Some(*cam) {
            for (k, l) in sc.lights.iter().enumerate() {
                if let (LightShape::Sun { to_light, .. }, true) = (&l.shape, l.shadow) {
                    let cascades = sun_cascades(cam, *to_light, sc.bounds, st.shadow_cascades.clamp(1, 4) as usize, res);
                    let maps = cascades.into_iter().map(|(far, c)| (far, render(c, &mut self.scratch.shadow_bins))).collect();
                    self.shadows[k] = LightShadow::Sun(maps);
                }
            }
            self.cascade_view = Some(*cam);
        }
        if sc.any_hashed_shadow {
            self.static_shadows = false;
        }
    }

    // ------------------------------------------------------------ a sample

    fn render_sample(&mut self, cam: &RasterCamera, sample: u32) {
        let st = self.settings;
        let threads = resolve_threads(st.threads);
        let (w, h) = (cam.width as usize, cam.height as usize);
        let layout = Layout::new(w, h);
        let sc = &self.scene;
        let scr = &mut self.scratch;

        // 1. Visibility of the opaque (and dithered) triangles.
        let visible: Vec<(usize, usize)> = sc.objects.iter().filter(|o| o.1 > o.0 && cam.sees_box(o.2.min, o.2.max)).map(|o| (o.0, o.1)).collect();
        let total: usize = visible.iter().map(|r| r.1 - r.0).sum();
        let parts = split_ranges(&visible, (threads * 2).min(total.div_ceil(2048)).clamp(1, 250));
        scr.bins.begin(layout, parts.len());
        scr.bins.setup(threads, |k, piece| {
            for &(a, b) in &parts[k] {
                for i in a..b {
                    let f = sc.flags(i);
                    if f.blended || f.volume {
                        continue;
                    }
                    let g = &sc.tris[i];
                    let p = [cam.to_view(g.v0), cam.to_view(g.v0 + g.e1), cam.to_view(g.v0 + g.e2)];
                    if f.cull && back_facing(cam, &p) {
                        continue;
                    }
                    cam.clip_emit(p, |v, b| {
                        piece.push(v, b, super::raster::Facing::Both, i as u32);
                    });
                }
            }
        });
        {
            let (bins, depth, ids) = (&scr.bins, &mut scr.depth, &mut scr.ids);
            depth.resize(layout.len(), 0.0);
            ids.resize(layout.len(), 0);
            parallel_for_each(depth.chunks_mut(TILE_PX).zip(ids.chunks_mut(TILE_PX)).enumerate(), threads, |(t, (d, id))| {
                d.fill(f32::NEG_INFINITY);
                id.fill(NO_ID);
                let rect = layout.tile_rect(t);
                bins.for_tile(t, |tid, tri, &src| {
                    let dithered = sc.flags(src as usize).dithered;
                    raster_tile(tri, rect, |k, x, y| {
                        let z = tri.z.at(x, y);
                        if z > d[k] {
                            if dithered {
                                let seed = hash2(x as u32, y as u32, sample.wrapping_mul(0x9e37_79b9) ^ sc.shade[src as usize].object);
                                let dir = cam.view_dir(x, y);
                                if sc.opacity(src as usize, tri.bary(x, y), cam.right * dir.x + cam.up * dir.y + cam.back * dir.z, seed) <= unit(seed) {
                                    return;
                                }
                            }
                            d[k] = z;
                            id[k] = tid;
                        }
                    });
                });
            });
        }

        // 2. Normals (view space, facing the viewer) and ambient occlusion.
        scr.normals.resize(layout.len(), Vec3::ZERO);
        {
            let (bins, ids) = (&scr.bins, &scr.ids);
            parallel_for_each(scr.normals.chunks_mut(TILE_PX).enumerate(), threads, |(t, nrm)| {
                let (x0, y0, x1, y1) = layout.tile_rect(t);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let k = (y % TILE) * TILE + x % TILE;
                        let id = ids[t * TILE_PX + k];
                        nrm[k] = if id == NO_ID {
                            Vec3::ZERO
                        } else {
                            let (tri, &src) = bins.get(id);
                            let b = tri.bary(x, y);
                            let s = &sc.shade[src as usize];
                            let g = &sc.tris[src as usize];
                            let n = (s.n[0] * b[0] + s.n[1] * b[1] + s.n[2] * b[2]).normalize();
                            let ng = g.e1.cross(g.e2);
                            let d = cam.to_world(cam.unproject(x, y, tri.z.at(x, y))) - cam.eye;
                            let n = if ng.dot(d) > 0.0 { -n } else { n };
                            cam.rotate(n)
                        };
                    }
                }
            });
        }
        scr.ao.resize(layout.len(), 1.0);
        if st.ao && st.ao_distance > 0.0 {
            let (depth, normals) = (&scr.depth, &scr.normals);
            parallel_for_each(scr.ao.chunks_mut(TILE_PX).enumerate(), threads, |(t, ao)| {
                let (x0, y0, x1, y1) = layout.tile_rect(t);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let k = (y % TILE) * TILE + x % TILE;
                        ao[k] = gtao(depth, normals, &layout, cam, x, y, st.ao_distance, hash2(x as u32, y as u32, sample));
                    }
                }
            });
        } else {
            scr.ao.fill(1.0);
        }

        // 3. Deferred shading (and the background).
        let want_spec = st.ssr;
        let want_sss = sc.any_sss;
        scr.color.resize(layout.len(), [0.0; 4]);
        scr.spec.clear();
        scr.sss.clear();
        if want_spec {
            scr.spec.resize(layout.len(), SpecPx::default());
        }
        if want_sss {
            scr.sss.resize(layout.len(), SssPx::default());
        }
        {
            let fr = Frame { sc, cam, st: &st, shadows: &self.shadows, sample, depth: &scr.depth, layout, spec: want_spec, sss: want_sss, opaque: None };
            let (bins, ids, depth, ao) = (&scr.bins, &scr.ids, &scr.depth, &scr.ao);
            let mut spec_it = scr.spec.chunks_mut(TILE_PX);
            let mut sss_it = scr.sss.chunks_mut(TILE_PX);
            let items = scr.color.chunks_mut(TILE_PX).enumerate().map(move |(t, c)| (t, c, spec_it.next(), sss_it.next()));
            parallel_for_each(items, threads, |(t, col, mut spec, mut sss)| {
                let (x0, y0, x1, y1) = layout.tile_rect(t);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let k = (y % TILE) * TILE + x % TILE;
                        let id = ids[t * TILE_PX + k];
                        col[k] = if id == NO_ID {
                            if let Some(s) = spec.as_deref_mut() {
                                s[k] = SpecPx::default();
                            }
                            if let Some(s) = sss.as_deref_mut() {
                                s[k] = SssPx::default();
                            }
                            fr.background(cam.view_dir(x, y))
                        } else {
                            let (tri, &src) = bins.get(id);
                            let key = depth[t * TILE_PX + k];
                            let s = fr.shade(src as usize, tri.bary(x, y), x, y, key, ao[t * TILE_PX + k], false);
                            if let Some(sp) = spec.as_deref_mut() {
                                sp[k] = s.spec;
                            }
                            if let Some(ss) = sss.as_deref_mut() {
                                ss[k] = s.sss;
                            }
                            [s.color.x, s.color.y, s.color.z, s.alpha]
                        };
                    }
                }
            });
        }

        // 4. Subsurface scattering and screen-space reflections.
        if want_sss {
            post::subsurface(&mut scr.color, &scr.sss, &scr.depth, &layout, cam, st.sss_samples, sample, threads);
        }
        if want_spec {
            scr.lit.clone_from(&scr.color);
            let p = post::Ssr { max_roughness: st.ssr_max_roughness.clamp(0.0, 1.0), thickness: st.ssr_thickness.max(1e-3), fade: st.ssr_fade.clamp(0.0, 0.5), max_distance: (sc.bounds.size().length() * 2.0).max(1.0) };
            post::reflections(&mut scr.color, &scr.lit, &scr.spec, &scr.depth, &scr.normals, &layout, cam, &p, sample, threads);
        }

        // 5. Volumes: objects, then the world.
        if st.volumetrics && !sc.lookdev && !sc.volumes.is_empty() {
            let fr = Frame { sc, cam, st: &st, shadows: &self.shadows, sample, depth: &scr.depth, layout, spec: false, sss: false, opaque: None };
            fr.object_volumes(&mut scr.color, &mut scr.vbins, threads);
        }
        if st.volumetrics && !sc.lookdev && sc.prep.world_volume.is_some() {
            let fr = Frame { sc, cam, st: &st, shadows: &self.shadows, sample, depth: &scr.depth, layout, spec: false, sss: false, opaque: None };
            let (ids, depth) = (&scr.ids, &scr.depth);
            parallel_for_each(scr.color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
                let (x0, y0, x1, y1) = layout.tile_rect(t);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let k = (y % TILE) * TILE + x % TILE;
                        let i = t * TILE_PX + k;
                        fr.fog(&mut col[k], x, y, if ids[i] == NO_ID { None } else { Some(depth[i]) });
                    }
                }
            });
        }

        // 6. Blended transparency, back to front.
        scr.transparent_order.clear();
        for &(a, b) in &visible {
            for i in a..b {
                if sc.flags(i).blended {
                    let g = &sc.tris[i];
                    let c = g.v0 + (g.e1 + g.e2) * (1.0 / 3.0);
                    scr.transparent_order.push((cam.to_view(c).z, i as u32));
                }
            }
        }
        if !scr.transparent_order.is_empty() {
            // Most negative z (farthest) first; stable for equal depths.
            scr.transparent_order.sort_by(|a, b| a.0.total_cmp(&b.0));
            let order = &scr.transparent_order;
            let tparts = split_ranges(&[(0, order.len())], (threads * 2).min(order.len().div_ceil(1024)).clamp(1, 250));
            scr.tbins.begin(layout, tparts.len());
            scr.tbins.setup(threads, |k, piece| {
                for &(a, b) in &tparts[k] {
                    for &(_, i) in &order[a..b] {
                        let g = &sc.tris[i as usize];
                        let p = [cam.to_view(g.v0), cam.to_view(g.v0 + g.e1), cam.to_view(g.v0 + g.e2)];
                        if sc.flags(i as usize).cull && back_facing(cam, &p) {
                            continue;
                        }
                        cam.clip_emit(p, |v, bb| {
                            piece.push(v, bb, super::raster::Facing::Both, i);
                        });
                    }
                }
            });
            // Refraction reads the opaque image behind.
            let refracting = sc.flags.iter().any(|f| f.refract.is_some());
            if refracting {
                scr.lit.clone_from(&scr.color);
            }
            let fr = Frame { sc, cam, st: &st, shadows: &self.shadows, sample, depth: &scr.depth, layout, spec: false, sss: false, opaque: refracting.then_some(&scr.lit[..]) };
            let (tbins, depth) = (&scr.tbins, &scr.depth);
            parallel_for_each(scr.color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
                let rect = layout.tile_rect(t);
                let d = &depth[t * TILE_PX..(t + 1) * TILE_PX];
                tbins.for_tile(t, |_, tri, &src| {
                    raster_tile(tri, rect, |k, x, y| {
                        let z = tri.z.at(x, y);
                        if z <= d[k] {
                            return;
                        }
                        let s = fr.shade(src as usize, tri.bary(x, y), x, y, z, 1.0, true);
                        let c = &mut col[k];
                        for (ch, (v, tr)) in [(s.color.x, s.transmit.x), (s.color.y, s.transmit.y), (s.color.z, s.transmit.z)].into_iter().enumerate() {
                            c[ch] = c[ch] * tr + v;
                        }
                        let ta = (s.transmit.x + s.transmit.y + s.transmit.z) / 3.0;
                        c[3] = c[3] * ta + s.alpha;
                    });
                });
            });
        }

        // 7. Depth of field and bloom.
        if let (Some(lens), false) = (st.lens, st.dof_jitter) {
            let p = post::DofBlur { r_scale: lens.aperture * cam.scale, focus: lens.focus, max_radius: (st.dof_max_size * 0.5).max(0.0) };
            if p.max_radius >= 0.5 {
                post::depth_of_field(&mut scr.color, &scr.depth, &layout, cam, &p, sample, threads, &mut scr.post);
            }
        }
        if st.bloom {
            let p = post::BloomParams { threshold: st.bloom_threshold, knee: st.bloom_knee, radius: st.bloom_radius, color: Vec3::new(st.bloom_color[0], st.bloom_color[1], st.bloom_color[2]).max(Vec3::ZERO), intensity: st.bloom_intensity.max(0.0), clamp: st.bloom_clamp.max(0.0) };
            post::bloom(&mut scr.color, &layout, &p, threads, &mut scr.bloom);
        }
    }
}

/// Whether a view-space triangle faces away from the camera.
fn back_facing(cam: &RasterCamera, p: &[Vec3; 3]) -> bool {
    let n = (p[1] - p[0]).cross(p[2] - p[0]);
    if cam.ortho { n.z < 0.0 } else { n.dot(p[0]) > 0.0 }
}

/// What a shaded surface point returns.
struct Shaded {
    /// Outgoing radiance (premultiplied by coverage).
    color: Vec3,
    /// Light let through (transparency).
    transmit: Vec3,
    alpha: f32,
    /// Glossy reflection for the screen-space pass.
    spec: SpecPx,
    /// Subsurface share of the diffuse light.
    sss: SssPx,
}

/// Everything shading reads during one sample.
struct Frame<'a> {
    sc: &'a EeveeScene,
    cam: &'a RasterCamera,
    st: &'a EeveeSettings,
    shadows: &'a [LightShadow],
    sample: u32,
    /// The opaque depth buffer (contact shadows, refraction).
    depth: &'a [f32],
    layout: Layout,
    /// Record glossy reflections / subsurface light.
    spec: bool,
    sss: bool,
    /// The opaque image (screen-space refraction).
    opaque: Option<&'a [[f32; 4]]>,
}

impl Frame<'_> {
    /// World-space direction for the probe (look dev turns with the view).
    #[inline]
    fn probe_dir(&self, world: Vec3) -> Vec3 {
        if self.sc.lookdev { self.cam.rotate(world) } else { world }
    }

    fn background(&self, dir_view: Vec3) -> [f32; 4] {
        if self.st.film_transparent {
            return [0.0; 4];
        }
        let d = self.cam.right * dir_view.x + self.cam.up * dir_view.y + self.cam.back * dir_view.z;
        let c = if self.sc.lookdev {
            let env = self.sc.probe.specular(dir_view, self.st.lookdev_blur);
            let o = self.st.lookdev_opacity.clamp(0.0, 1.0);
            if o < 1.0 { self.sc.world_color.lerp(env, o) } else { env }
        } else if let Some(bg) = &self.sc.background {
            bg.sample(d)
        } else {
            self.sc.prep.env.eval_ray(d, RayState::camera())
        };
        [c.x, c.y, c.z, 1.0]
    }

    /// Ambient light a volume scatters (the probe's mean).
    fn ambient(&self) -> Vec3 {
        let li = self.sc.probe.irradiance(Vec3::Z) * (0.5 * FRAC_1_PI) + self.sc.probe.irradiance(-Vec3::Z) * (0.5 * FRAC_1_PI);
        li * 0.5
    }

    /// Homogeneous world volume between the eye and the surface (or the
    /// volumetric end for the background).
    fn fog(&self, c: &mut [f32; 4], x: usize, y: usize, key: Option<f32>) {
        let Some(vol) = &self.sc.prep.world_volume else { return };
        if self.sc.lookdev {
            return;
        }
        let dist = match key {
            Some(k) => self.cam.unproject(x, y, k).length(),
            None => self.st.volumetric_end.max(0.0),
        };
        let sigma_t = vol.sigma_a + vol.sigma_s;
        let tr = Vec3::new((-sigma_t.x * dist).exp(), (-sigma_t.y * dist).exp(), (-sigma_t.z * dist).exp());
        let dir = self.cam.to_world(self.cam.unproject(x, y, key.unwrap_or(1.0))) - self.cam.eye;
        let dir = if self.cam.ortho { -self.cam.back } else { dir.normalize() };
        // In-scattered light: the probe's ambient plus sun lights (phase HG).
        let mut li = self.ambient();
        for l in &self.sc.lights {
            if let LightShape::Sun { to_light, irradiance, .. } = &l.shape {
                li += *irradiance * hg(vol.anisotropy, dir.dot(*to_light));
            }
        }
        let safe = |s: f32| if s > 1e-8 { 1.0 / s } else { 0.0 };
        let inv = Vec3::new(safe(sigma_t.x), safe(sigma_t.y), safe(sigma_t.z));
        let scatter = (vol.sigma_s.mul_elem(li) + vol.emission).mul_elem(inv).mul_elem(Vec3::ONE - tr);
        for k in 0..3 {
            c[k] = c[k] * tr[k] + scatter[k] * if self.st.film_transparent { 1.0 } else { c[3].max(1.0) };
        }
    }

    /// Integrate the volume objects (farthest first) over the image: each
    /// pixel's ray between the object's front and back faces (or the
    /// opaque surface in between) is ray marched.
    fn object_volumes(&self, color: &mut [[f32; 4]], bins: &mut Bins<u32>, threads: usize) {
        let (cam, layout) = (self.cam, self.layout);
        let mut order: Vec<&VolumeObject> = self.sc.volumes.iter().filter(|v| cam.sees_box(v.bounds.min, v.bounds.max)).collect();
        order.sort_by(|a, b| cam.to_view(a.bounds.center()).z.total_cmp(&cam.to_view(b.bounds.center()).z));
        let steps = self.st.volumetric_steps.clamp(1, 512) as usize;
        for v in order {
            let (a, b) = v.range;
            let parts = split_ranges(&[(a, b)], (threads * 2).min((b - a).div_ceil(2048)).clamp(1, 250));
            bins.begin(layout, parts.len());
            bins.setup(threads, |k, piece| {
                for &(a, b) in &parts[k] {
                    for i in a..b {
                        let g = &self.sc.tris[i];
                        let p = [cam.to_view(g.v0), cam.to_view(g.v0 + g.e1), cam.to_view(g.v0 + g.e2)];
                        let back = back_facing(cam, &p) as u32;
                        cam.clip_emit(p, |vv, bb| {
                            piece.push(vv, bb, super::raster::Facing::Both, i as u32 | back << 31);
                        });
                    }
                }
            });
            let bins = &*bins;
            let vol = v.coeffs;
            parallel_for_each(color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
                let rect = layout.tile_rect(t);
                let mut enter = [f32::NEG_INFINITY; TILE_PX];
                let mut exit = [f32::INFINITY; TILE_PX];
                let mut any = false;
                bins.for_tile(t, |_, tri, &src| {
                    raster_tile(tri, rect, |k, x, y| {
                        let z = tri.z.at(x, y);
                        if src >> 31 == 0 {
                            enter[k] = enter[k].max(z);
                        } else {
                            exit[k] = exit[k].min(z);
                            any = true;
                        }
                        let _ = (x, y);
                    });
                });
                if !any {
                    return;
                }
                let (x0, y0, x1, y1) = rect;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let k = (y % TILE) * TILE + x % TILE;
                        if exit[k] == f32::INFINITY {
                            continue;
                        }
                        let opaque = self.depth[t * TILE_PX + k];
                        let dist = |key: f32| if cam.ortho { -key } else { cam.unproject(x, y, key).length() };
                        let t_in = if enter[k] == f32::NEG_INFINITY || enter[k] < exit[k] { 0.0 } else { dist(enter[k]) };
                        let mut t_out = dist(exit[k]);
                        if opaque != f32::NEG_INFINITY {
                            t_out = t_out.min(dist(opaque));
                        }
                        let t_in = if cam.ortho { t_in } else { t_in.max(0.0) };
                        if !(t_out > t_in) {
                            continue;
                        }
                        self.march_volume(&mut col[k], &vol, x, y, t_in, t_out, steps);
                    }
                }
            });
        }
    }

    /// Ray march a homogeneous medium along pixel (`x`, `y`) from `t0` to
    /// `t1` (distances from the eye; view depths in ortho) and composite it.
    #[allow(clippy::too_many_arguments)]
    fn march_volume(&self, c: &mut [f32; 4], vol: &VolumeCoeffs, x: usize, y: usize, t0: f32, t1: f32, steps: usize) {
        let cam = self.cam;
        let dir_v = cam.view_dir(x, y);
        let dir = cam.right * dir_v.x + cam.up * dir_v.y + cam.back * dir_v.z;
        let origin = if cam.ortho { cam.to_world(Vec3::new(cam.unproject(x, y, 0.0).x, cam.unproject(x, y, 0.0).y, 0.0)) } else { cam.eye };
        let sigma_t = vol.sigma_a + vol.sigma_s;
        let ds = (t1 - t0) / steps as f32;
        let step_tr = Vec3::new((-sigma_t.x * ds).exp(), (-sigma_t.y * ds).exp(), (-sigma_t.z * ds).exp());
        let safe = |s: f32| if s > 1e-8 { 1.0 / s } else { ds };
        let inv = Vec3::new(safe(sigma_t.x), safe(sigma_t.y), safe(sigma_t.z));
        let ambient = self.ambient();
        let h = hash2(x as u32, y as u32, self.sample.wrapping_mul(0x1b87_3593));
        let jit = unit(h);
        let rot = unit(h.wrapping_mul(0x85eb_ca6b)) * TAU;
        let mut tr = Vec3::ONE;
        let mut scatter = Vec3::ZERO;
        for i in 0..steps {
            let t = t0 + (i as f32 + jit) * ds;
            let p = origin + dir * t;
            let mut li = ambient;
            if vol.sigma_s != Vec3::ZERO {
                for (k, l) in self.sc.lights.iter().enumerate() {
                    let Some((wi, l_in, size)) = light_incidence(&l.shape, p) else { continue };
                    if l_in == Vec3::ZERO {
                        continue;
                    }
                    let vis = if self.st.volumetric_shadows { self.shadow(k, l, p, wi, rot, size) } else { 1.0 };
                    li += l_in * (hg(vol.anisotropy, dir.dot(wi)) * vis);
                }
            }
            let s = (vol.sigma_s.mul_elem(li) + vol.emission).mul_elem(inv).mul_elem(Vec3::ONE - step_tr);
            scatter += tr.mul_elem(s);
            tr = tr.mul_elem(step_tr);
        }
        for k in 0..3 {
            c[k] = c[k] * tr[k] + finite(scatter[k]);
        }
        let ta = (tr.x + tr.y + tr.z) / 3.0;
        c[3] = c[3] * ta + (1.0 - ta);
    }

    /// Shade triangle `src` at source barycentrics `b` seen through pixel
    /// (`x`, `y`) with depth key `key` (`blended`: in the transparent pass).
    #[allow(clippy::too_many_arguments)]
    fn shade(&self, src: usize, b: [f32; 3], x: usize, y: usize, key: f32, ao: f32, blended: bool) -> Shaded {
        let sc = self.sc;
        let g = &sc.tris[src];
        let sh = &sc.shade[src];
        let cam = self.cam;
        let pv = cam.unproject(x, y, key);
        let eye_dir = if cam.ortho { -cam.back } else { (cam.to_world(pv) - cam.eye).normalize() };
        let dist = if cam.ortho { (cam.to_world(pv) - cam.eye).dot(-cam.back).abs() } else { pv.length() };
        let mats = &sc.prep.materials;
        let flags = sc.flags(src);
        let prog = mats.get(sh.material as usize).and_then(|m| m.program.as_ref()).or_else(|| mats.last().and_then(|m| m.program.as_ref()));
        let Some(prog) = prog else {
            return Shaded { color: Vec3::splat(0.5), transmit: Vec3::ZERO, alpha: 1.0, spec: SpecPx::default(), sss: SssPx::default() };
        };
        let seed = if flags.dithered { hash2(x as u32, y as u32, self.sample.wrapping_mul(0x9e37_79b9) ^ sh.object) } else { hash2(x as u32, y as u32, self.sample) };
        let mut ctx = surface_ctx(&sc.prep, prog, g, sh, b[1], b[2], eye_dir, dist, RayState::camera(), seed);
        let mut c = run(prog, &mut ctx);
        if prog.surface.is_none() && prog.volume.is_some() {
            c.transparent = Vec3::ONE;
        }
        let p = ctx.p;
        let wo = -eye_dir;
        let ng = ctx.ng;
        let ns = ctx.n;
        let transmit = c.transparent.max(Vec3::ZERO).min(Vec3::ONE);
        let want_sss = self.sss && !blended && c.sss_weight != Vec3::ZERO;
        let sss_weight = c.sss_weight;
        let sss_radius = c.sss_radius;
        let bsdf = ClosureBsdf::new(c.lobes, Vec3::ZERO, ng, wo);
        let mut col = c.emission;
        // Diffuse irradiance at the shading normal (for subsurface).
        let mut irr = Vec3::ZERO;
        // Lights.
        let rot = (hash2(x as u32, y as u32, self.sample.wrapping_mul(7919)) & 0xffff) as f32 / 65536.0 * TAU;
        for (k, l) in sc.lights.iter().enumerate() {
            if let LightShape::Area { .. } = &l.shape {
                col += self.area_light(k, l, &bsdf, wo, p, ng, ns, rot, x, y, want_sss.then_some(&mut irr));
                continue;
            }
            let Some((wi, li, size_texels)) = light_incidence(&l.shape, p) else { continue };
            if li == Vec3::ZERO {
                continue;
            }
            let (f, _, _) = bsdf.eval_split(wo, wi);
            if f == Vec3::ZERO {
                continue;
            }
            let vis = self.shadow(k, l, p, ng, rot, size_texels) * self.contact(l, p, ng, wi, x, y);
            if vis <= 0.0 {
                continue;
            }
            col += f.mul_elem(li) * vis;
            if want_sss {
                irr += li * (vis * ns.dot(wi).max(0.0));
            }
        }
        // World: split sum per lobe, occluded by AO.
        let ndv = ns.dot(wo).clamp(0.0, 1.0);
        let mut spec = SpecPx::default();
        let mut best = -1.0f32;
        let mut note_spec = |w: Vec3, pr: Vec3, n: Vec3, rough: f32| {
            spec.weight += w;
            spec.probe += pr;
            let l = luminance(w);
            if l > best {
                best = l;
                spec.n = n;
                spec.rough = rough;
            }
        };
        for lobe in &bsdf.lobes {
            let n = lobe.n;
            match lobe.kind {
                LobeKind::Diffuse { .. } | LobeKind::Toon { glossy: false, .. } => {
                    col += lobe.weight.mul_elem(sc.probe.irradiance(self.probe_dir(n))) * (FRAC_1_PI * ao);
                }
                LobeKind::Translucent => {
                    col += lobe.weight.mul_elem(sc.probe.irradiance(self.probe_dir(-n))) * FRAC_1_PI;
                }
                LobeKind::Sheen { .. } => {
                    col += lobe.weight.mul_elem(lobe.albedo(wo)).mul_elem(sc.probe.irradiance(self.probe_dir(n))) * (FRAC_1_PI * ao);
                }
                LobeKind::Reflection { ax, ay, .. } => {
                    let rough = (ax * ay).sqrt().sqrt();
                    let r = n * (2.0 * n.dot(wo)) - wo;
                    let so = spec_occlusion(ndv, ao, rough);
                    let w = lobe.weight.mul_elem(lobe.albedo(wo));
                    let pr = w.mul_elem(sc.probe.specular(self.probe_dir(r), rough)) * so;
                    col += pr;
                    note_spec(w, pr, n, rough);
                }
                LobeKind::Toon { glossy: true, .. } => {
                    let r = n * (2.0 * n.dot(wo)) - wo;
                    col += lobe.weight.mul_elem(sc.probe.specular(self.probe_dir(r), 0.3)) * spec_occlusion(ndv, ao, 0.3);
                }
                LobeKind::Glass { alpha, eta, tint_r, tint_t, refract_only } => {
                    let rough = alpha.sqrt();
                    let cos = n.dot(wo).clamp(0.0, 1.0);
                    let f = fresnel_dielectric(cos, eta);
                    let r = n * (2.0 * n.dot(wo)) - wo;
                    if !refract_only {
                        let w = lobe.weight.mul_elem(tint_r) * f;
                        let pr = w.mul_elem(sc.probe.specular(self.probe_dir(r), rough)) * spec_occlusion(ndv, ao, rough);
                        col += pr;
                        note_spec(w, pr, n, rough);
                    }
                    let t = refract(-wo, n, 1.0 / eta).unwrap_or(r);
                    let k = if refract_only { 1.0 } else { 1.0 - f };
                    // Screen-space refraction of the opaque image behind,
                    // else through the probe.
                    let behind = match (flags.refract, self.opaque) {
                        (Some(depth), Some(img)) if blended => self.refraction(img, p, eye_dir, t, depth, rough, x, y),
                        _ => None,
                    };
                    let li = behind.unwrap_or_else(|| sc.probe.specular(self.probe_dir(t), rough.max(0.05)));
                    col += lobe.weight.mul_elem(tint_t).mul_elem(li) * k;
                }
            }
        }
        let opacity = 1.0 - (transmit.x + transmit.y + transmit.z) / 3.0;
        let mut alpha = (opacity * (1.0 - c.holdout.clamp(0.0, 1.0))).clamp(0.0, 1.0);
        let mut transmit = transmit;
        // Subsurface: take its share of the diffuse light out; the blur
        // pass puts it back spread over the surface.
        let mut sss = SssPx::default();
        if want_sss {
            irr += sc.probe.irradiance(self.probe_dir(ns)) * ao;
            let e = irr * FRAC_1_PI;
            col = (col - sss_weight.mul_elem(e)).max(Vec3::ZERO);
            sss = SssPx { irr: e, albedo: sss_weight, radius: sss_radius.max(Vec3::ZERO) };
        }
        let mut scale = 1.0;
        if c.holdout > 0.0 {
            scale = 1.0 - c.holdout.clamp(0.0, 1.0);
        }
        // Dithered surfaces were kept with probability `opacity`: show them opaque.
        if flags.dithered && !blended {
            let op = opacity.max(1e-3);
            scale /= op;
            alpha = (1.0 - c.holdout.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            transmit = Vec3::ZERO;
        }
        if scale != 1.0 {
            col = col * scale;
            spec.weight = spec.weight * scale;
            spec.probe = spec.probe * scale;
            sss.albedo = sss.albedo * scale;
        }
        let col = Vec3::new(finite(col.x), finite(col.y), finite(col.z));
        if !self.spec {
            spec = SpecPx::default();
        }
        Shaded { color: col, transmit, alpha, spec, sss }
    }

    /// What a refracted ray from world point `p` sees in the opaque image:
    /// with a `thickness` the ray leaves a slab of that thickness parallel
    /// to the view (two refraction events), else it travels on along `t`.
    #[allow(clippy::too_many_arguments)]
    fn refraction(&self, img: &[[f32; 4]], p: Vec3, eye_dir: Vec3, t: Vec3, thickness: f32, rough: f32, x: usize, y: usize) -> Option<Vec3> {
        let cam = self.cam;
        let (o, d) = if thickness > 0.0 { (p + t * (thickness / t.dot(eye_dir).max(0.05)), eye_dir) } else { (p, t) };
        let h = hash2(x as u32, y as u32, self.sample ^ 0x3c6e_f372);
        // Rough refraction: jitter the direction (averages over samples).
        let d = if rough > 0.02 {
            let (tx, ty) = super::sampling::onb(d);
            let a = unit(h) * TAU;
            let r = rough * rough * unit(h.wrapping_mul(0x27d4_eb2d)).sqrt();
            (d + (tx * a.cos() + ty * a.sin()) * r).normalize()
        } else {
            d
        };
        let ov = cam.to_view(o);
        let dv = cam.rotate(d);
        let max = (self.sc.bounds.size().length() * 2.0).max(1.0);
        let thick = (self.st.ssr_thickness.max(1e-3)) * 4.0;
        let (hx, hy, _) = post::march(self.depth, &self.layout, cam, ov, dv, max, thick, 0.0, unit(h.wrapping_mul(0x1656_67b1)))?;
        let c = img[self.layout.index(hx, hy)];
        Some(Vec3::new(c[0], c[1], c[2]))
    }

    /// An area light integrated over its shape: Lambertian lobes exactly
    /// (polygon form factor), the others by stratified samples. Adds the
    /// irradiance at the shading normal to `irr` when given.
    #[allow(clippy::too_many_arguments)]
    fn area_light(&self, k: usize, l: &RLight, bsdf: &ClosureBsdf, wo: Vec3, p: Vec3, ng: Vec3, ns: Vec3, rot: f32, x: usize, y: usize, irr: Option<&mut Vec3>) -> Vec3 {
        let LightShape::Area { center, u, v, normal, area: light_area, radiance, ellipse, spread } = &l.shape else { return Vec3::ZERO };
        let to_c = *center - p;
        if normal.dot(to_c) >= 0.0 {
            return Vec3::ZERO; // behind the emitting side
        }
        let d = to_c.length().max(1e-6);
        let wc = to_c / d;
        let size_texels = u.length().max(v.length()) / d * 40.0;
        let vis = self.shadow(k, l, p, ng, rot, size_texels) * self.contact(l, p, ng, wc, x, y);
        if vis <= 0.0 {
            return Vec3::ZERO;
        }
        let mut out = Vec3::ZERO;
        let exact = spread.is_none();
        let mut e = Vec3::ZERO;
        if exact {
            let mut poly = [Vec3::ZERO; 16];
            let n = area::outline(*center, *u, *v, *ellipse, &mut poly);
            for lobe in &bsdf.lobes {
                let ff = match lobe.kind {
                    LobeKind::Diffuse { .. } => area::form_factor(p, lobe.n, &poly[..n]),
                    LobeKind::Translucent => area::form_factor(p, -lobe.n, &poly[..n]),
                    _ => continue,
                };
                out += lobe.weight.mul_elem(*radiance) * ff;
            }
            if irr.is_some() {
                e = *radiance * (PI * area::form_factor(p, ns, &poly[..n]));
            }
        }
        // Stratified samples over the area for everything else.
        let needs_mc = !exact || bsdf.lobes.iter().any(|l| !matches!(l.kind, LobeKind::Diffuse { .. } | LobeKind::Translucent));
        if needs_mc {
            const N: usize = 4;
            let h = hash2(x as u32, y as u32, self.sample.wrapping_mul(0x632b_e5ab) ^ k as u32);
            let (ru, rv) = (unit(h), unit(h.wrapping_mul(0x9e37_79b9) ^ 0x7f4a_7c15));
            for j in 0..N {
                let q = area::sample_point(*center, *u, *v, *ellipse, j, N, ru, rv);
                let to = q - p;
                let d2 = to.length_squared().max(1e-10);
                let wi = to / d2.sqrt();
                let cos_l = -normal.dot(wi);
                if cos_l <= 0.0 {
                    continue;
                }
                let att = super::prepare::spread_attenuation(*spread, cos_l);
                let g = *light_area * cos_l * att / (d2 * N as f32);
                let f = if exact { bsdf.eval_non_lambert(wo, wi) } else { bsdf.eval_split(wo, wi).0 };
                out += f.mul_elem(*radiance) * g;
                if !exact {
                    e += *radiance * (g * ns.dot(wi).max(0.0));
                }
            }
        }
        if let Some(irr) = irr {
            *irr += e * vis;
        }
        out * vis
    }

    /// Contact shadow towards a light: 0 when a screen-space ray from `p`
    /// along `wi` hits the depth buffer within the light's distance.
    fn contact(&self, l: &RLight, p: Vec3, ng: Vec3, wi: Vec3, x: usize, y: usize) -> f32 {
        let Some((dist, thick)) = l.contact else { return 1.0 };
        if self.depth.is_empty() {
            return 1.0;
        }
        let cam = self.cam;
        let pv = cam.to_view(p);
        let scale = if cam.ortho { 1.0 } else { pv.length() };
        let nv = cam.rotate(if ng.dot(wi) < 0.0 { -ng } else { ng });
        let o = pv + nv * (scale * 2e-3 + 1e-4);
        let jit = unit(hash2(x as u32, y as u32, self.sample ^ 0x94d0_49bb));
        match post::march(self.depth, &self.layout, cam, o, cam.rotate(wi), dist, thick, 0.0, jit) {
            Some(_) => 0.0,
            None => 1.0,
        }
    }

    /// Light visibility from the shadow maps (1 lit).
    fn shadow(&self, k: usize, l: &RLight, p: Vec3, ng: Vec3, rot: f32, size: f32) -> f32 {
        let soft = self.st.soft_shadows;
        match self.shadows.get(k) {
            Some(LightShadow::Sun(cascades)) => {
                let depth = (p - self.cam.eye).dot(-self.cam.back);
                let Some((_, m)) = cascades.iter().find(|(far, _)| depth <= *far).or(cascades.last()) else { return 1.0 };
                let radius = if soft { 1.5 + l.size * 0.5 * m.cam.scale * 0.5 } else { 1.0 };
                m.lit(p, ng, radius.min(12.0), rot)
            }
            Some(LightShadow::Spot(m)) => {
                let r = if soft { 1.5 + size } else { 1.0 };
                m.lit(p, ng, r.min(12.0), rot)
            }
            Some(LightShadow::Cube(maps)) => {
                let (LightShape::Point { pos, .. } | LightShape::Area { center: pos, .. }) = &l.shape else { return 1.0 };
                let d = p - *pos;
                let a = d.abs();
                let face = if a.x >= a.y && a.x >= a.z {
                    if d.x > 0.0 { 0 } else { 1 }
                } else if a.y >= a.z {
                    if d.y > 0.0 { 2 } else { 3 }
                } else if d.z > 0.0 {
                    4
                } else {
                    5
                };
                let r = if soft { 1.5 + size } else { 1.0 };
                maps[face].lit(p, ng, r.min(12.0), rot)
            }
            _ => 1.0,
        }
    }
}

/// Henyey-Greenstein phase function.
fn hg(g: f32, cos: f32) -> f32 {
    (1.0 - g * g) / (4.0 * PI * (1.0 + g * g - 2.0 * g * cos).max(1e-6).powf(1.5))
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// Lagarde's specular occlusion from AO.
fn spec_occlusion(ndv: f32, ao: f32, rough: f32) -> f32 {
    ((ndv + ao).powf((-16.0 * rough - 1.0).exp2()) - 1.0 + ao).clamp(0.0, 1.0)
}

fn refract(i: Vec3, n: Vec3, eta: f32) -> Option<Vec3> {
    let cos_i = -n.dot(i);
    let k = 1.0 - eta * eta * (1.0 - cos_i * cos_i);
    (k >= 0.0).then(|| (i * eta + n * (eta * cos_i - k.sqrt())).normalize())
}

/// Direction to a light, its incident radiance factor (irradiance for a
/// surface facing it) and the PCF growth in texels for its size. Area
/// lights come as their representative point (volumes use this; surfaces
/// integrate them in [`Frame::area_light`]).
fn light_incidence(l: &LightShape, p: Vec3) -> Option<(Vec3, Vec3, f32)> {
    match l {
        LightShape::Point { pos, radius, intensity, spot } => {
            let to = *pos - p;
            let d2 = to.length_squared().max(1e-8);
            let dist = d2.sqrt();
            let wi = to / dist;
            let mut li = *intensity / d2;
            if let Some((axis, cos_half, blend)) = spot {
                let c = axis.dot(-wi);
                let x = if *blend > 0.0 { ((c - cos_half) / blend).clamp(0.0, 1.0) } else if c >= *cos_half { 1.0 } else { 0.0 };
                li *= x * x * (3.0 - 2.0 * x);
            }
            Some((wi, li, radius / dist * 40.0))
        }
        LightShape::Sun { to_light, irradiance, .. } => Some((*to_light, *irradiance, 0.0)),
        LightShape::Area { center, u, v, normal, area, radiance, spread, .. } => {
            // Representative point: the closest point of the rectangle.
            let to_c = p - *center;
            let (ul, vl) = (u.length().max(1e-8), v.length().max(1e-8));
            let su = (to_c.dot(*u) / ul).clamp(-ul, ul) / ul;
            let sv = (to_c.dot(*v) / vl).clamp(-vl, vl) / vl;
            let q = *center + *u * su + *v * sv;
            let to = q - p;
            let d2 = (to.length_squared()).max(1e-8);
            let dist = d2.sqrt();
            let wi = to / dist;
            // Cosine at the light, from its centre (keeps the total power).
            let tc = (*center - p).normalize();
            let cos_l = -normal.dot(tc);
            if cos_l <= 0.0 {
                return Some((wi, Vec3::ZERO, 0.0));
            }
            let d2c = (*center - p).length_squared().max(d2);
            let att = super::prepare::spread_attenuation(*spread, cos_l);
            Some((wi, *radiance * (*area * cos_l * att / d2c), (ul.max(vl)) / dist * 40.0))
        }
        _ => None,
    }
}

/// GTAO-lite: horizon search in two directions over the depth buffer,
/// rotated per pixel and per sample (the accumulation averages the
/// rotations, as EEVEE's does); returns visibility (1 = open).
#[allow(clippy::too_many_arguments)]
fn gtao(depth: &[f32], normals: &[Vec3], l: &Layout, cam: &RasterCamera, x: usize, y: usize, radius: f32, h: u32) -> f32 {
    let i = l.index(x, y);
    let key = depth[i];
    let n = normals[i];
    if key == f32::NEG_INFINITY || n == Vec3::ZERO {
        return 1.0;
    }
    let p = cam.unproject(x, y, key);
    let r_px = if cam.ortho { radius * cam.scale } else { radius * cam.scale / (-p.z).max(1e-4) };
    if r_px < 1.0 {
        return 1.0;
    }
    let r_px = r_px.min(64.0);
    let rot = (h & 0xffff) as f32 / 65536.0 * PI;
    let jit = ((h >> 16) & 0xff) as f32 / 256.0;
    const DIRS: usize = 2;
    const STEPS: usize = 6;
    let mut occ = 0.0;
    for d in 0..DIRS {
        let a = rot + d as f32 * (PI / DIRS as f32);
        let (dx, dy) = (a.cos(), a.sin());
        for sgn in [1.0f32, -1.0] {
            let mut best = 0.0f32;
            for s in 0..STEPS {
                let t = (s as f32 + jit) / STEPS as f32 * r_px + 1.0;
                let (sx, sy) = (x as f32 + 0.5 + dx * t * sgn, y as f32 + 0.5 + dy * t * sgn);
                if !(sx >= 0.0 && sy >= 0.0 && sx < l.w as f32 && sy < l.h as f32) {
                    break;
                }
                let (qx, qy) = (sx as usize, sy as usize);
                let qk = depth[l.index(qx, qy)];
                if qk == f32::NEG_INFINITY {
                    continue;
                }
                let v = cam.unproject(qx, qy, qk) - p;
                let len = v.length();
                if !(len > 1e-6) {
                    continue;
                }
                let cos = n.dot(v) / len;
                let fall = (1.0 - (len / radius) * (len / radius)).clamp(0.0, 1.0);
                best = best.max(cos * fall);
            }
            occ += best.max(0.0);
        }
    }
    (1.0 - occ / (2 * DIRS) as f32).clamp(0.0, 1.0)
}

/// A perspective light camera at `pos` looking along `dir`.
fn persp_camera(pos: Vec3, dir: Vec3, fov: f32, res: u32, bounds: Aabb) -> RasterCamera {
    let back = -dir.normalize();
    let up_hint = if back.z.abs() > 0.99 { Vec3::Y } else { Vec3::Z };
    let right = up_hint.cross(back).normalize();
    let up = back.cross(right).normalize();
    let half = res as f32 * 0.5;
    let scale = half / (fov * 0.5).tan();
    let r = (bounds.size().length() * 0.5).max(1e-3);
    RasterCamera { eye: pos, right, up, back, ortho: false, scale, cx: half, cy: half, near: (r * 1e-4).max(1e-3), width: res, height: res }
}

/// Fit `count` orthographic cascades for a sun along `to_light` to the
/// view frustum of `cam` (clamped to the scene bounds).
fn sun_cascades(cam: &RasterCamera, to_light: Vec3, bounds: Aabb, count: usize, res: u32) -> Vec<(f32, RasterCamera)> {
    if bounds.is_empty() {
        return Vec::new();
    }
    let center = bounds.center();
    let radius = (bounds.size().length() * 0.5).max(1e-3);
    // View depth range covering the scene.
    let fwd = -cam.back;
    let dc = (center - cam.eye).dot(fwd);
    let near = if cam.ortho { (dc - radius).max(-radius * 2.0) } else { (dc - radius).max(cam.near) };
    let far = (dc + radius).max(near + 1e-3);
    let back = to_light.normalize();
    let right = back.any_orthonormal().normalize();
    let up = back.cross(right).normalize();
    let mut out = Vec::new();
    let mut d0 = near;
    for k in 0..count {
        let f = (k + 1) as f32 / count as f32;
        let log = if near > 0.0 { near * (far / near).powf(f) } else { near + (far - near) * f };
        let lin = near + (far - near) * f;
        let d1 = if k + 1 == count { far } else { 0.75 * log + 0.25 * lin };
        // Corners of the frustum slice.
        let mut pts = Vec::with_capacity(8);
        for &d in &[d0, d1] {
            for &(px, py) in &[(0.0, 0.0), (cam.width as f32, 0.0), (0.0, cam.height as f32), (cam.width as f32, cam.height as f32)] {
                let (vx, vy) = ((px - cam.cx) / cam.scale, (cam.cy - py) / cam.scale);
                let v = if cam.ortho { Vec3::new(vx, vy, -d) } else { Vec3::new(vx * d, vy * d, -d) };
                pts.push(cam.to_world(v));
            }
        }
        let mut c = pts.iter().fold(Vec3::ZERO, |a, &p| a + p) * (1.0 / pts.len() as f32);
        let mut r = pts.iter().map(|p| (*p - c).length()).fold(0.0f32, f32::max);
        // Never larger than the scene itself.
        if r > radius {
            c = center;
            r = radius;
        }
        r = r.max(1e-3) * 1.05;
        let half = res as f32 * 0.5;
        let scale = half / r;
        // Snap the centre to texels so the cascade does not shimmer.
        let texel = 1.0 / scale;
        let lc = Vec3::new(c.dot(right), c.dot(up), c.dot(back));
        let snapped = Vec3::new((lc.x / texel).round() * texel, (lc.y / texel).round() * texel, lc.z);
        let c = right * snapped.x + up * snapped.y + back * snapped.z;
        let eye = c + back * (radius * 2.0 + (center - c).dot(back).max(0.0));
        out.push((d1, RasterCamera { eye, right, up, back, ortho: true, scale, cx: half, cy: half, near: 1e-4, width: res, height: res }));
        d0 = d1;
    }
    out
}

/// Render the scene's active camera with EEVEE (all samples): a linear
/// RGBA image, alpha premultiplied.
pub fn render_eevee(scene: &Scene, s: &EeveeSettings) -> Image {
    let (w, h) = (s.width.max(1), s.height.max(1));
    let active = scene.active_camera.filter(|&c| c < scene.objects.len());
    // The camera's depth of field unless the settings bring their own lens.
    let s = &EeveeSettings { lens: s.lens.or_else(|| active.and_then(|ci| Lens::from_camera(scene, ci, w, h))), ..*s };
    let cam = match active {
        Some(ci) => {
            let c = match &scene.objects[ci].data {
                ObjectData::Camera(c) => *c,
                _ => crate::scene::Camera::default(),
            };
            RasterCamera::from_camera(&scene.world_matrix(ci), &c, w, h)
        }
        None => RasterCamera::from_view(&super::viewport::ViewCamera::default(), w, h),
    };
    let mut e = Eevee::new(EeveeScene::new(scene, s.lookdev), s);
    e.set_view(cam);
    while e.add_sample() {}
    e.image()
}

/// Mean luminance of an image (tests and the benchmark).
pub fn mean_luminance(img: &Image) -> f32 {
    let n = img.pixels.len().max(1) as f32;
    img.pixels.iter().map(|p| luminance(Vec3::new(p[0], p[1], p[2]))).sum::<f32>() / n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_weights() {
        assert!(filter_weight(0.0, 0.0, 1.5) > 0.9);
        assert!(filter_weight(0.7, 0.0, 1.5) < filter_weight(0.2, 0.0, 1.5));
        assert_eq!(filter_weight(1.0, 0.0, 1.5), 0.0);
    }

    #[test]
    fn cascades_cover_the_view() {
        let cam = RasterCamera::from_view(&super::super::viewport::ViewCamera::default(), 320, 180);
        let b = Aabb { min: Vec3::splat(-5.0), max: Vec3::splat(5.0) };
        let c = sun_cascades(&cam, Vec3::new(0.3, 0.2, 0.9).normalize(), b, 3, 512);
        assert_eq!(c.len(), 3);
        assert!(c[0].0 < c[1].0 && c[1].0 < c[2].0);
        // Later cascades cover more (fewer pixels per unit).
        assert!(c[0].1.scale >= c[2].1.scale);
    }
}
