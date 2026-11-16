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
//! 3. **Deferred shading**: per pixel the triangle's barycentrics rebuild
//!    the shading point and the material's compiled node program runs
//!    once (no random sampling); its closure lobes are lit analytically:
//!    every light through the lobes' BSDFs (point, spot, sun, and area
//!    lights as a representative point) with shadow maps (cascaded for
//!    sun, a cube for point and area lights, a frustum for spots;
//!    percentage-closer filtered, softened by the light size), and the
//!    world through a prefiltered probe (split sum: GGX lobes read the
//!    roughness-filtered environment times their directional albedo,
//!    diffuse lobes the spherical-harmonics irradiance), with occlusion.
//!    Emission is added; holdout cuts alpha.
//! 4. **Transparency** ("blended"): triangles of materials that let light
//!    through are sorted back to front, depth tested against the opaque
//!    surfaces and composited (`dst * transparency + radiance`).
//! 5. **Volumetrics**: a homogeneous world volume fogs every pixel
//!    (absorption, in-scattering of the probe and of sun lights).
//! 6. **Film**: samples accumulate with jittered sub-pixel offsets, each
//!    weighted by the pixel filter, while the view stays still (EEVEE's
//!    progressive TAA samples); noise in the soft shadows and AO rotates
//!    per sample and averages out.
//!
//! Material Preview ("look dev") replaces the scene's world and lights by
//! our procedural studio environment, fixed to the view.
mod probe;

use super::bsdf::{fresnel_dielectric, ClosureBsdf, LobeKind};
use super::bvh::TriGeom;
use super::prepare::{LightShape, PreparedScene, TriShade};
use super::raster::{split_ranges, Bins, Facing, Layout, RasterCamera, NO_ID, TILE, TILE_PX};
use super::shader::surface::{run, surface_ctx};
use super::shader::RayState;
use super::shadow::ShadowMap;
use super::workbench::jitter;
use super::{luminance, parallel_for_each, resolve_threads};
use crate::math::{Aabb, Vec3};
use crate::scene::{Image, ObjectData, Scene};
use core::f32::consts::{FRAC_1_PI, PI};
pub(crate) use probe::Probe;

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
    /// Pixel filter width in pixels.
    pub filter_size: f32,
    pub film_transparent: bool,
    /// Material Preview: our studio environment, no scene lights or world.
    pub lookdev: bool,
    /// Background blur in look dev (0..1 roughness).
    pub lookdev_blur: f32,
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
            filter_size: 1.5,
            film_transparent: false,
            lookdev: false,
            lookdev_blur: 0.5,
            threads: 0,
        }
    }
}

/// A light as the rasterizer lights with it.
#[derive(Clone, Debug)]
struct RLight {
    shape: LightShape,
    shadow: bool,
    /// Light size for soft shadows (radius, or the sun's angle in radians).
    size: f32,
}

/// A scene prepared for EEVEE (rebuild when the scene changes).
pub struct EeveeScene {
    prep: PreparedScene,
    tris: Vec<TriGeom>,
    shade: Vec<TriShade>,
    /// Per object: triangle range and world bounds.
    objects: Vec<(usize, usize, Aabb)>,
    /// Materials whose surfaces go through the blended transparent pass.
    transparent: Vec<bool>,
    lights: Vec<RLight>,
    probe: Probe,
    /// The background as seen by the camera (None: use `prep.env`).
    background: Option<probe::EqMap>,
    lookdev: bool,
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
            let mut bb = Aabb::EMPTY;
            for t in &geo.tris[a..b] {
                bb.grow(t.v0);
                bb.grow(t.v0 + t.e1);
                bb.grow(t.v0 + t.e2);
            }
            bounds = bounds.union(bb);
            objects.push((a, b, bb));
        }
        let transparent = prep.materials.iter().map(|m| m.program.as_ref().is_some_and(|p| p.info.transparent)).collect();
        let mut lights = Vec::new();
        if !lookdev {
            let light_objs: Vec<usize> = (0..scene.objects.len()).filter(|&i| matches!(scene.objects[i].data, ObjectData::Light(_))).collect();
            for (k, l) in prep.lights.iter().enumerate() {
                let info = prep.light_info.get(k);
                let size = match (info.and_then(|i| light_objs.iter().find(|&&o| o as u32 == i.object)), l) {
                    (Some(&o), _) => match &scene.objects[o].data {
                        ObjectData::Light(li) => match l {
                            LightShape::Area { u, v, .. } => u.length().max(v.length()),
                            _ => li.radius.max(0.0),
                        },
                        _ => 0.0,
                    },
                    _ => 0.0,
                };
                if matches!(l, LightShape::Point { .. } | LightShape::Sun { .. } | LightShape::Area { .. }) {
                    lights.push(RLight { shape: l.clone(), shadow: info.is_none_or(|i| i.cast_shadow), size });
                }
            }
        }
        let (probe, background) = if lookdev {
            // The studio never changes: prefilter it once.
            static STUDIO: std::sync::OnceLock<Probe> = std::sync::OnceLock::new();
            (STUDIO.get_or_init(|| Probe::build(threads, &probe::studio)).clone(), None)
        } else {
            let env = &prep.env;
            let probe = Probe::build(threads, &|d| env.eval_ray(d, RayState::camera()));
            let constant = env.is_constant();
            let bg = (!constant).then(|| probe::EqMap::capture(1024, 512, threads, &|d| env.eval_ray(d, RayState::camera())));
            (probe, bg)
        };
        EeveeScene { tris: geo.tris, shade: geo.shade, objects, transparent, lights, probe, background, lookdev, bounds, prep }
    }

    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }
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
    shadow_bins: Bins<()>,
    depth: Vec<f32>,
    ids: Vec<u32>,
    normals: Vec<Vec3>,
    ao: Vec<f32>,
    color: Vec<[f32; 4]>,
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
    scratch: Scratch,
}

impl Eevee {
    pub fn new(scene: EeveeScene, settings: &EeveeSettings) -> Eevee {
        Eevee { scene, settings: *settings, cam: None, accum: Vec::new(), weight: 0.0, samples: 0, shadows: Vec::new(), cascade_view: None, static_shadows: false, scratch: Scratch::default() }
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
        self.reset();
        true
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
        self.update_shadows(&cam);
        self.render_sample(&cam.jittered(jx, jy), s);
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
        let tri = |i: usize| {
            let g = &sc.tris[i];
            // Transparent surfaces do not cast (they would need stochastic shadows).
            if sc.transparent.get(sc.shade[i].material as usize).copied().unwrap_or(false) {
                return None;
            }
            Some([g.v0, g.v0 + g.e1, g.v0 + g.e2])
        };
        if !self.static_shadows || self.shadows.len() != sc.lights.len() {
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
                            let c = persp_camera(*pos, *axis, fov.min(3.0), res, self.scene.bounds);
                            LightShadow::Spot(ShadowMap::render(c, threads, sc.tris.len(), &mut self.scratch.shadow_bins, tri))
                        }
                        LightShape::Point { pos, .. } | LightShape::Area { center: pos, .. } => {
                            let faces = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
                            let maps: [ShadowMap; 6] = core::array::from_fn(|k| {
                                let c = persp_camera(*pos, faces[k], PI * 0.5, res / 2, self.scene.bounds);
                                ShadowMap::render(c, threads, sc.tris.len(), &mut self.scratch.shadow_bins, tri)
                            });
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
                    let maps = cascades.into_iter().map(|(far, c)| (far, ShadowMap::render(c, threads, sc.tris.len(), &mut self.scratch.shadow_bins, tri))).collect();
                    self.shadows[k] = LightShadow::Sun(maps);
                }
            }
            self.cascade_view = Some(*cam);
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

        // 1. Visibility of the opaque triangles.
        let visible: Vec<(usize, usize)> = sc.objects.iter().filter(|o| o.1 > o.0 && cam.sees_box(o.2.min, o.2.max)).map(|o| (o.0, o.1)).collect();
        let total: usize = visible.iter().map(|r| r.1 - r.0).sum();
        let parts = split_ranges(&visible, (threads * 2).min(total.div_ceil(2048)).clamp(1, 250));
        scr.bins.begin(layout, parts.len());
        scr.bins.setup(threads, |k, piece| {
            for &(a, b) in &parts[k] {
                for i in a..b {
                    if sc.transparent.get(sc.shade[i].material as usize).copied().unwrap_or(false) {
                        continue;
                    }
                    let g = &sc.tris[i];
                    let p = [cam.to_view(g.v0), cam.to_view(g.v0 + g.e1), cam.to_view(g.v0 + g.e2)];
                    cam.clip_emit(p, |v, b| {
                        piece.push(v, b, Facing::Both, i as u32);
                    });
                }
            }
        });
        scr.bins.visibility(&mut scr.depth, &mut scr.ids, threads);

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
        scr.color.resize(layout.len(), [0.0; 4]);
        let fr = Frame { sc, cam, st: &st, shadows: &self.shadows, sample };
        {
            let (bins, ids, depth, ao) = (&scr.bins, &scr.ids, &scr.depth, &scr.ao);
            parallel_for_each(scr.color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
                let (x0, y0, x1, y1) = layout.tile_rect(t);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let k = (y % TILE) * TILE + x % TILE;
                        let id = ids[t * TILE_PX + k];
                        let dir_v = cam.view_dir(x, y);
                        let mut c = if id == NO_ID {
                            fr.background(dir_v)
                        } else {
                            let (tri, &src) = bins.get(id);
                            let key = depth[t * TILE_PX + k];
                            let s = fr.shade(src as usize, tri.bary(x, y), x, y, key, ao[t * TILE_PX + k]);
                            [s.color.x, s.color.y, s.color.z, s.alpha]
                        };
                        if st.volumetrics {
                            fr.fog(&mut c, x, y, if id == NO_ID { None } else { Some(depth[t * TILE_PX + k]) });
                        }
                        col[k] = c;
                    }
                }
            });
        }

        // 4. Blended transparency, back to front.
        scr.transparent_order.clear();
        for &(a, b) in &visible {
            for i in a..b {
                if sc.transparent.get(sc.shade[i].material as usize).copied().unwrap_or(false) {
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
                        cam.clip_emit(p, |v, bb| {
                            piece.push(v, bb, Facing::Both, i);
                        });
                    }
                }
            });
            let (tbins, depth) = (&scr.tbins, &scr.depth);
            parallel_for_each(scr.color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
                let rect = layout.tile_rect(t);
                let d = &depth[t * TILE_PX..(t + 1) * TILE_PX];
                tbins.for_tile(t, |_, tri, &src| {
                    super::raster::raster_tile(tri, rect, |k, x, y| {
                        let z = tri.z.at(x, y);
                        if z <= d[k] {
                            return;
                        }
                        let s = fr.shade(src as usize, tri.bary(x, y), x, y, z, 1.0);
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
    }
}

/// What a shaded surface point returns.
struct Shaded {
    /// Outgoing radiance (premultiplied by coverage).
    color: Vec3,
    /// Light let through (transparency).
    transmit: Vec3,
    alpha: f32,
}

/// Everything shading reads during one sample.
struct Frame<'a> {
    sc: &'a EeveeScene,
    cam: &'a RasterCamera,
    st: &'a EeveeSettings,
    shadows: &'a [LightShadow],
    sample: u32,
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
            self.sc.probe.specular(dir_view, self.st.lookdev_blur)
        } else if let Some(bg) = &self.sc.background {
            bg.sample(d)
        } else {
            self.sc.prep.env.eval_ray(d, RayState::camera())
        };
        [c.x, c.y, c.z, 1.0]
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
        let mut li = self.sc.probe.irradiance(Vec3::Z) * (0.5 * FRAC_1_PI) + self.sc.probe.irradiance(-Vec3::Z) * (0.5 * FRAC_1_PI);
        li = li * 0.5;
        for l in &self.sc.lights {
            if let LightShape::Sun { to_light, irradiance, .. } = &l.shape {
                let g = vol.anisotropy;
                let cos = dir.dot(*to_light);
                let phase = (1.0 - g * g) / (4.0 * PI * (1.0 + g * g - 2.0 * g * cos).max(1e-6).powf(1.5));
                li += *irradiance * phase;
            }
        }
        let safe = |s: f32| if s > 1e-8 { 1.0 / s } else { 0.0 };
        let inv = Vec3::new(safe(sigma_t.x), safe(sigma_t.y), safe(sigma_t.z));
        let scatter = (vol.sigma_s.mul_elem(li) + vol.emission).mul_elem(inv).mul_elem(Vec3::ONE - tr);
        for k in 0..3 {
            c[k] = c[k] * tr[k] + scatter[k] * if self.st.film_transparent { 1.0 } else { c[3].max(1.0) };
        }
    }

    /// Shade triangle `src` at source barycentrics `b` seen through pixel
    /// (`x`, `y`) with depth key `key`.
    fn shade(&self, src: usize, b: [f32; 3], x: usize, y: usize, key: f32, ao: f32) -> Shaded {
        let sc = self.sc;
        let g = &sc.tris[src];
        let sh = &sc.shade[src];
        let cam = self.cam;
        let pv = cam.unproject(x, y, key);
        let eye_dir = if cam.ortho { -cam.back } else { (cam.to_world(pv) - cam.eye).normalize() };
        let dist = if cam.ortho { (cam.to_world(pv) - cam.eye).dot(-cam.back).abs() } else { pv.length() };
        let mats = &sc.prep.materials;
        let prog = mats.get(sh.material as usize).and_then(|m| m.program.as_ref()).or_else(|| mats.last().and_then(|m| m.program.as_ref()));
        let Some(prog) = prog else {
            return Shaded { color: Vec3::splat(0.5), transmit: Vec3::ZERO, alpha: 1.0 };
        };
        let mut ctx = surface_ctx(&sc.prep, prog, g, sh, b[1], b[2], eye_dir, dist, RayState::camera(), hash2(x as u32, y as u32, self.sample));
        let mut c = run(prog, &mut ctx);
        if prog.surface.is_none() && prog.volume.is_some() {
            c.transparent = Vec3::ONE;
        }
        let p = ctx.p;
        let wo = -eye_dir;
        let ng = ctx.ng;
        let ns = ctx.n;
        let transmit = c.transparent.max(Vec3::ZERO).min(Vec3::ONE);
        let bsdf = ClosureBsdf::new(c.lobes, Vec3::ZERO, ng, wo);
        let mut col = c.emission;
        // Lights.
        let rot = (hash2(x as u32, y as u32, self.sample.wrapping_mul(7919)) & 0xffff) as f32 / 65536.0 * core::f32::consts::TAU;
        for (k, l) in sc.lights.iter().enumerate() {
            let Some((wi, li, size_texels)) = light_incidence(&l.shape, p) else { continue };
            if li == Vec3::ZERO {
                continue;
            }
            let (f, _, _) = bsdf.eval_split(wo, wi);
            if f == Vec3::ZERO {
                continue;
            }
            let vis = self.shadow(k, l, p, ng, rot, size_texels);
            if vis <= 0.0 {
                continue;
            }
            col += f.mul_elem(li) * vis;
        }
        // World: split sum per lobe, occluded by AO.
        let ndv = ns.dot(wo).clamp(0.0, 1.0);
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
                    col += lobe.weight.mul_elem(lobe.albedo(wo)).mul_elem(sc.probe.specular(self.probe_dir(r), rough)) * so;
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
                        col += lobe.weight.mul_elem(tint_r).mul_elem(sc.probe.specular(self.probe_dir(r), rough)) * (f * spec_occlusion(ndv, ao, rough));
                    }
                    // Refraction through the probe (no screen-space refraction).
                    let t = refract(-wo, n, 1.0 / eta).unwrap_or(r);
                    let k = if refract_only { 1.0 } else { 1.0 - f };
                    col += lobe.weight.mul_elem(tint_t).mul_elem(sc.probe.specular(self.probe_dir(t), rough.max(0.05))) * k;
                }
            }
        }
        let opacity = 1.0 - (transmit.x + transmit.y + transmit.z) / 3.0;
        let alpha = (opacity * (1.0 - c.holdout.clamp(0.0, 1.0))).clamp(0.0, 1.0);
        let col = if c.holdout > 0.0 { col * (1.0 - c.holdout.clamp(0.0, 1.0)) } else { col };
        let col = Vec3::new(finite(col.x), finite(col.y), finite(col.z));
        Shaded { color: col, transmit, alpha }
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
/// surface facing it) and the PCF growth in texels for its size.
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
    let cam = match scene.active_camera.filter(|&c| c < scene.objects.len()) {
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
