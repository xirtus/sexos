// SPDX-License-Identifier: GPL-3.0-only
//! The 3D view's "Solid" mode (Workbench): a multithreaded, tiled CPU
//! rasterizer.
//!
//! [`ViewportScene`] caches the evaluated, triangulated, world-space geometry
//! of a scene (rebuild it when the scene changes; [`ViewportScene::sync_selection`]
//! is enough for selection changes, [`ViewportScene::patch_object`] for
//! sculpting). [`ViewportScene::draw`] renders it from an orbit
//! [`ViewCamera`], [`ViewportScene::draw_camera`] from any [`RasterCamera`]:
//!
//! 1. culling: objects whose bounds are outside the view frustum are skipped
//!    entirely;
//! 2. vertex stage: every remaining vertex to view space, with the studio
//!    lighting of smooth normals (parallel chunks);
//! 3. setup stage: per triangle lighting at the corners (headlight, key,
//!    fill, rim and specular on the Workbench colour), back-face culling,
//!    near clipping, projection, fixed-point edge setup and binning into
//!    32 x 32 pixel tiles (parallel pieces, see `raster`);
//! 4. raster stage: each tile is owned by one worker which depth tests its
//!    triangles into a per-tile visibility buffer (nearest triangle id);
//!    X-ray accumulates every fragment with weighted blended transparency;
//! 5. resolve: per pixel, the triangle's perspective-correct barycentrics
//!    interpolate the lit corner colours (or the texture, or a MatCap from
//!    the interpolated normal), with the shadow map; then cavity, object
//!    outlines, the floor grid, axes, gizmos and the wireframe as
//!    depth-tested lines, and Blender's orange silhouette of selected objects.
//!
//! The output does not depend on the thread count. No allocation happens
//! per pixel or per triangle in the hot loops; per-frame buffers are reused.
use super::raster::{fceil, ffloor, split_ranges, Bins, Facing, Layout, RasterCamera, SVert, SetupTri, NO_ID, TILE, TILE_PX};
use super::shadow::ShadowMap;
use super::tonemap::srgb_encode;
use super::workbench::{self, CavityType, ColorType, Lighting, Workbench};
use super::{parallel_for_each, resolve_threads};
use crate::math::{Aabb, Vec2, Vec3};
use crate::mesh::{Mesh, Tri};
use crate::nodes::Prop;
use crate::scene::{Image, ObjectData, Scene};
use std::sync::Mutex;

/// Orbit view: the camera circles `target` at `distance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewCamera {
    pub target: Vec3,
    pub distance: f32,
    /// Azimuth in radians around +Z, measured from +X.
    pub yaw: f32,
    /// Elevation in radians above the XY plane.
    pub pitch: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    pub ortho: bool,
}

impl Default for ViewCamera {
    /// Roughly Blender's startup view, looking at the origin from +X -Y +Z
    /// (a little above and beside the startup camera, which stays in view).
    fn default() -> Self {
        ViewCamera { target: Vec3::ZERO, distance: 15.0, yaw: -0.6, pitch: 0.55, fov_y: 0.7, ortho: false }
    }
}

impl ViewCamera {
    /// Unit vector from the target towards the eye.
    pub fn back(&self) -> Vec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cp * cy, cp * sy, sp)
    }
    pub fn right(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin(), self.yaw.cos(), 0.0)
    }
    pub fn up(&self) -> Vec3 {
        self.back().cross(self.right())
    }
    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }
    /// Frame `bounds` (keeps the angles).
    pub fn frame(&mut self, bounds: Aabb) {
        if bounds.is_empty() {
            return;
        }
        self.target = bounds.center();
        let r = (bounds.size().length() * 0.5).max(1e-3);
        self.distance = r / (self.fov_y * 0.5).sin().max(1e-3) * 1.1;
    }
    fn tan_half(&self) -> f32 {
        (self.fov_y.clamp(1e-3, 3.0) * 0.5).tan()
    }
    /// World-space ray through pixel position (`x`, `y`) of a `w` x `h` view.
    pub fn ray(&self, x: f32, y: f32, w: u32, h: u32) -> (Vec3, Vec3) {
        let aspect = w as f32 / h.max(1) as f32;
        let sx = 2.0 * x / w.max(1) as f32 - 1.0;
        let sy = 1.0 - 2.0 * y / h.max(1) as f32;
        let (right, up, back) = (self.right(), self.up(), self.back());
        let t = self.tan_half();
        if self.ortho {
            let half = self.distance * t;
            let o = self.eye() + right * (sx * half * aspect) + up * (sy * half) + back * (self.distance * 1000.0);
            (o, -back)
        } else {
            (self.eye(), (-back + right * (sx * t * aspect) + up * (sy * t)).normalize())
        }
    }
}

/// Viewport drawing options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportSettings {
    pub width: u32,
    pub height: u32,
    /// Draw every mesh edge on top of the shading.
    pub wireframe: bool,
    /// Floor grid and the X/Y axis lines.
    pub grid: bool,
    /// Skip triangles facing away from the view.
    pub backface_culling: bool,
    /// 0 = all cores.
    pub threads: usize,
    /// A flat background colour (display-space RGB 0..1) instead of the
    /// default vertical gradient, like Blender 4.x's "Single Color" theme.
    pub background: Option<[f32; 3]>,
    /// Lighting, colour and the Workbench options.
    pub workbench: Workbench,
    /// Background pixels get alpha 0.
    pub transparent: bool,
    /// Gizmos of lights, cameras and empties, loose edges and selection outlines.
    pub overlays: bool,
}

impl Default for ViewportSettings {
    fn default() -> Self {
        ViewportSettings { width: 1280, height: 720, wireframe: false, grid: true, backface_culling: false, threads: 0, background: None, workbench: Workbench::default(), transparent: false, overlays: true }
    }
}

const BG_TOP: [f32; 3] = [0.28, 0.28, 0.28];
const BG_BOTTOM: [f32; 3] = [0.20, 0.20, 0.20];
const SELECT: [f32; 3] = [0.93, 0.36, 0.0];
const ACTIVE: [f32; 3] = [1.0, 0.63, 0.16];
const WIRE: [f32; 3] = [0.0, 0.0, 0.0];
const GIZMO: [f32; 3] = [0.05, 0.05, 0.05];
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct VTri {
    v: [u32; 3],
    /// Display-space material base colour.
    color: Vec3,
    smooth: bool,
    /// Object slot in [`ViewportScene::objects`].
    obj: u32,
    /// Index into [`ViewportScene::corner_colors`] (NONE: none).
    corners: u32,
    /// Index into [`ViewportScene::attr_colors`] (NONE: none).
    attr: u32,
    /// Index into [`ViewportScene::tex_uvs`] (NONE: no texture).
    tex: u32,
}

/// Per-object display extras: corner colours (Vertex Paint, the sculpt mask
/// and face sets) and hiding sculpt-hidden faces.
#[derive(Clone, Debug, Default)]
pub struct ObjectOverlay {
    /// Linear RGB base colour per triangle corner, in the order of
    /// `Mesh::triangulate` of the evaluated mesh (replaces the material).
    pub corner_colors: Option<Vec<[[f32; 3]; 3]>>,
    /// Leave out faces flagged in `Mesh::hidden_faces`.
    pub hide_hidden: bool,
}

#[derive(Clone, Copy, Debug)]
struct ObjInfo {
    /// Index into `Scene::objects`.
    index: usize,
    selected: bool,
    active: bool,
    /// First vertex and vertex count in [`ViewportScene::positions`].
    verts: (u32, u32),
    /// First triangle and triangle count in [`ViewportScene::tris`].
    tris: (u32, u32),
    /// World matrix and its normal matrix (meshes).
    world: crate::math::Mat4,
    normal: crate::math::Mat4,
    smooth: bool,
    /// World-space bounds of the cached vertices.
    bounds: Aabb,
    /// Display colour for Random colouring.
    random: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct Line {
    a: Vec3,
    b: Vec3,
    color: [f32; 3],
    alpha: f32,
}

/// An image reduced for the Texture colour mode (display RGB, at most 256
/// pixels a side, point sampled).
#[derive(Clone, Debug)]
struct DispTex {
    w: usize,
    h: usize,
    px: Vec<Vec3>,
}

impl DispTex {
    fn from(img: &Image) -> Option<DispTex> {
        let (iw, ih) = (img.width as usize, img.height as usize);
        if iw == 0 || ih == 0 || img.pixels.len() != iw * ih {
            return None;
        }
        let (w, h) = (iw.min(256), ih.min(256));
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let p = img.pixels[(y * ih / h) * iw + x * iw / w];
                px.push(Vec3::new(srgb_encode(p[0].clamp(0.0, 1.0)), srgb_encode(p[1].clamp(0.0, 1.0)), srgb_encode(p[2].clamp(0.0, 1.0))));
            }
        }
        Some(DispTex { w, h, px })
    }
    /// Bilinear, repeating; v = 0 at the bottom like the renderer.
    fn sample(&self, u: f32, v: f32) -> Vec3 {
        let x = (u - u.floor()) * self.w as f32 - 0.5;
        let y = (1.0 - (v - v.floor())) * self.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |xi: f32, yi: f32| {
            let xi = (xi as i64).rem_euclid(self.w as i64) as usize;
            let yi = (yi as i64).rem_euclid(self.h as i64) as usize;
            self.px[yi * self.w + xi]
        };
        (at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx) * (1.0 - fy) + (at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx) * fy
    }
}

/// The image a material shows in Texture colour mode: its base colour
/// texture, else the first Image Texture node's image.
fn material_image(m: &crate::scene::Material) -> Option<usize> {
    m.base_color_texture.or_else(|| {
        m.node_tree.as_ref()?.nodes.iter().filter(|n| n.idname == "ShaderNodeTexImage" && !n.mute).find_map(|n| match n.prop("image") {
            Some(Prop::Id(Some(i))) => Some(*i),
            _ => None,
        })
    })
}

/// Cached viewport geometry for one scene state.
#[derive(Debug, Default)]
pub struct ViewportScene {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    tris: Vec<VTri>,
    /// Mesh edges (unique), with their object slot, for the wireframe.
    edges: Vec<([u32; 2], u32)>,
    /// Loose edges (always drawn).
    loose: Vec<([u32; 2], u32)>,
    objects: Vec<ObjInfo>,
    /// Gizmos of lights, cameras and empties: (object slot, world lines).
    gizmos: Vec<(u32, Vec3, Vec<(Vec3, Vec3)>)>,
    /// Display-space corner colours of triangles with an overlay.
    corner_colors: Vec<[Vec3; 3]>,
    /// Display-space corner colours of the meshes' colour attributes.
    attr_colors: Vec<[Vec3; 3]>,
    /// Corner UVs and texture of triangles whose material has an image.
    tex_uvs: Vec<([Vec2; 3], u32)>,
    textures: Vec<DispTex>,
    /// World bounds of all mesh geometry.
    bounds: Aabb,
    /// Bumped by every geometry patch (keeps the shadow map current).
    generation: u64,
    scratch: Mutex<Scratch>,
}

impl Clone for ViewportScene {
    fn clone(&self) -> Self {
        ViewportScene {
            positions: self.positions.clone(),
            normals: self.normals.clone(),
            tris: self.tris.clone(),
            edges: self.edges.clone(),
            loose: self.loose.clone(),
            objects: self.objects.clone(),
            gizmos: self.gizmos.clone(),
            corner_colors: self.corner_colors.clone(),
            attr_colors: self.attr_colors.clone(),
            tex_uvs: self.tex_uvs.clone(),
            textures: self.textures.clone(),
            bounds: self.bounds,
            generation: self.generation,
            scratch: Mutex::default(),
        }
    }
}

fn encode3(c: [f32; 3]) -> Vec3 {
    Vec3::new(srgb_encode(c[0]), srgb_encode(c[1]), srgb_encode(c[2]))
}

impl ViewportScene {
    pub fn new(scene: &Scene) -> ViewportScene {
        ViewportScene::new_with(scene, &|_, _, _| None)
    }

    /// [`ViewportScene::new`] with per-object extras: `overlay` gets the scene
    /// object index, its evaluated world-space mesh and its triangles.
    pub fn new_with(scene: &Scene, overlay: &dyn Fn(usize, &Mesh, &[Tri]) -> Option<ObjectOverlay>) -> ViewportScene {
        let mut vs = ViewportScene { bounds: Aabb::EMPTY, ..Default::default() };
        let mut tex_of_image = std::collections::HashMap::new();
        for oi in 0..scene.objects.len() {
            if !scene.is_visible(oi) {
                continue;
            }
            let slot = vs.objects.len() as u32;
            let m = scene.world_matrix(oi);
            let random = workbench::random_color(oi as u32 ^ scene.objects[oi].name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)));
            match &scene.objects[oi].data {
                ObjectData::Mesh(_) => {
                    let Some((mesh, slots)) = scene.world_mesh(oi) else { continue };
                    let base = vs.positions.len() as u32;
                    let smooth = mesh.faces.iter().any(|f| f.smooth);
                    let mut bounds = Aabb::EMPTY;
                    for p in &mesh.positions {
                        bounds.grow(*p);
                    }
                    vs.bounds = vs.bounds.union(bounds);
                    let tri0 = vs.tris.len() as u32;
                    vs.positions.extend_from_slice(&mesh.positions);
                    if smooth {
                        vs.normals.extend(mesh.vertex_normals());
                    } else {
                        vs.normals.resize(vs.positions.len(), Vec3::ZERO);
                    }
                    let colors: Vec<Vec3> = slots.iter().map(|&s| scene.materials.get(s).map_or([0.8; 4], |m| m.base_color)).map(|c| encode3([c[0], c[1], c[2]])).collect();
                    let images: Vec<Option<u32>> = slots
                        .iter()
                        .map(|&s| {
                            let img = scene.materials.get(s).and_then(material_image)?;
                            if let Some(&t) = tex_of_image.get(&img) {
                                return Some(t);
                            }
                            let t = DispTex::from(scene.images.get(img)?)?;
                            vs.textures.push(t);
                            let k = vs.textures.len() as u32 - 1;
                            tex_of_image.insert(img, k);
                            Some(k)
                        })
                        .collect();
                    let default = Vec3::splat(srgb_encode(0.8));
                    let tris = mesh.triangulate();
                    let ov = overlay(oi, &mesh, &tris).unwrap_or_default();
                    let cc = ov.corner_colors.filter(|c| c.len() == tris.len());
                    let offsets = if mesh.color_attributes.is_empty() { Vec::new() } else { mesh.corner_offsets() };
                    for (ti, t) in tris.iter().enumerate() {
                        if ov.hide_hidden && mesh.is_face_hidden(t.face as usize) {
                            continue;
                        }
                        let f = &mesh.faces[t.face as usize];
                        let color = colors.get(f.material as usize).copied().unwrap_or(default);
                        let corners = match &cc {
                            Some(c) => {
                                vs.corner_colors.push(c[ti].map(encode3));
                                (vs.corner_colors.len() - 1) as u32
                            }
                            None => NONE,
                        };
                        let attr = if offsets.is_empty() {
                            NONE
                        } else {
                            vs.attr_colors.push(t.corners.map(|c| {
                                let k = mesh.corner_color(mesh.render_color, &offsets, t.face as usize, c as usize);
                                encode3([k[0].clamp(0.0, 1.0), k[1].clamp(0.0, 1.0), k[2].clamp(0.0, 1.0)])
                            }));
                            (vs.attr_colors.len() - 1) as u32
                        };
                        let tex = match images.get(f.material as usize).copied().flatten() {
                            Some(img) if f.uvs.len() == f.verts.len() => {
                                vs.tex_uvs.push((t.corners.map(|c| f.uvs[c as usize]), img));
                                (vs.tex_uvs.len() - 1) as u32
                            }
                            _ => NONE,
                        };
                        vs.tris.push(VTri { v: t.verts.map(|v| v + base), color, smooth: f.smooth, obj: slot, corners, attr, tex });
                    }
                    let tri_count = vs.tris.len() as u32 - tri0;
                    vs.objects.push(ObjInfo { index: oi, selected: false, active: false, verts: (base, mesh.positions.len() as u32), tris: (tri0, tri_count), world: m, normal: m.inverse().transpose(), smooth, bounds, random });
                    vs.edges.extend(mesh.edges().into_iter().map(|e| ([e[0] + base, e[1] + base], slot)));
                    vs.loose.extend(mesh.loose_edges.iter().map(|e| ([e[0] + base, e[1] + base], slot)));
                }
                data => {
                    vs.objects.push(ObjInfo { index: oi, selected: false, active: false, verts: (0, 0), tris: (0, 0), world: m, normal: m, smooth: false, bounds: Aabb::EMPTY, random });
                    let o = m.transform_point(Vec3::ZERO);
                    let lines = gizmo_lines(data).into_iter().map(|(a, b)| (m.transform_point(a), m.transform_point(b))).collect();
                    vs.gizmos.push((slot, o, lines));
                }
            }
        }
        vs.sync_selection(scene);
        vs
    }

    /// Refresh selection highlighting without rebuilding geometry.
    pub fn sync_selection(&mut self, scene: &Scene) {
        for o in &mut self.objects {
            o.selected = scene.selected.contains(&o.index);
            o.active = scene.active_object == Some(o.index) && o.selected;
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    /// Vertex count of scene object `object` in the cache (None: not cached).
    pub fn object_vertex_count(&self, object: usize) -> Option<usize> {
        self.objects.iter().find(|o| o.index == object && o.verts.1 > 0).map(|o| o.verts.1 as usize)
    }

    /// Update the cached positions (and smooth normals) of `verts` of scene
    /// object `object` from its evaluated mesh in object space, without a
    /// rebuild (sculpting). `normals` are object-space vertex normals,
    /// indexed like `positions`. False when the topology does not match.
    pub fn patch_object(&mut self, object: usize, positions: &[Vec3], normals: &[Vec3], verts: &[u32]) -> bool {
        let Some(slot) = self.objects.iter().position(|o| o.index == object && o.verts.1 > 0) else { return false };
        let o = self.objects[slot];
        if o.verts.1 as usize != positions.len() {
            return false;
        }
        let base = o.verts.0 as usize;
        let mut bounds = o.bounds;
        for &v in verts {
            let v = v as usize;
            let Some(&p) = positions.get(v) else { continue };
            let w = o.world.transform_point(p);
            self.positions[base + v] = w;
            bounds.grow(w);
            if o.smooth {
                if let Some(&n) = normals.get(v) {
                    self.normals[base + v] = o.normal.transform_vector(n).normalize();
                }
            }
        }
        // Bounds only grow while sculpting (a rebuild tightens them).
        self.objects[slot].bounds = bounds;
        self.bounds = self.bounds.union(bounds);
        self.generation += 1;
        true
    }

    /// Render to tightly packed RGBA8 rows (top row first).
    pub fn draw(&self, view: &ViewCamera, settings: &ViewportSettings) -> Vec<u8> {
        let mut out = Vec::new();
        self.draw_into(view, settings, &mut out);
        out
    }

    /// [`ViewportScene::draw`] into a reusable buffer. Per-frame working
    /// memory is kept inside the scene and reused, so steady-state redraws
    /// do not allocate (fresh pages cost more than the drawing itself).
    pub fn draw_into(&self, view: &ViewCamera, s: &ViewportSettings, out: &mut Vec<u8>) {
        let cam = RasterCamera::from_view(view, s.width, s.height);
        self.draw_camera(&cam, Some(view), s, out);
    }

    /// Draw from any camera (`view` places the floor grid; without it there
    /// is none).
    pub fn draw_camera(&self, cam: &RasterCamera, view: Option<&ViewCamera>, s: &ViewportSettings, out: &mut Vec<u8>) {
        self.draw_impl(cam, view, s, None, out);
    }

    /// The overlays (grid, gizmos, wires, selection outlines) of
    /// [`ViewportScene::draw_into`] over `image` (RGBA, `s.width` x
    /// `s.height`) instead of the Solid surfaces: Material Preview and
    /// Rendered shading. The surfaces still hide what is behind them.
    pub fn draw_over(&self, view: &ViewCamera, s: &ViewportSettings, image: &[u8], out: &mut Vec<u8>) {
        let cam = RasterCamera::from_view(view, s.width, s.height);
        let under = (image.len() >= s.width as usize * s.height as usize * 4).then_some(image);
        let st = ViewportSettings { workbench: Workbench { xray: false, shadow: false, cavity: false, outline: false, ..s.workbench }, ..*s };
        self.draw_impl(&cam, Some(view), &st, under, out);
    }

    fn draw_impl(&self, cam: &RasterCamera, view: Option<&ViewCamera>, s: &ViewportSettings, under: Option<&[u8]>, out: &mut Vec<u8>) {
        let (w, h) = (s.width as usize, s.height as usize);
        out.resize(w * h * 4, 0);
        if w == 0 || h == 0 {
            return;
        }
        let mut guard = self.scratch.try_lock();
        let mut local = Scratch::default();
        let sc: &mut Scratch = match guard {
            Ok(ref mut g) => g,
            Err(_) => &mut local, // drawn concurrently from another thread
        };
        let threads = resolve_threads(s.threads);
        let wb = &s.workbench;
        let cam = *cam;
        let layout = Layout::new(w, h);

        // 1. Frustum culling per object.
        let visible: Vec<usize> = (0..self.objects.len()).filter(|&k| self.objects[k].verts.1 > 0 && cam.sees_box(self.objects[k].bounds.min, self.objects[k].bounds.max)).collect();

        // 2. Vertex stage over the visible objects' vertices.
        sc.verts.resize(self.positions.len(), VertOut::default());
        let studio = wb.lighting == Lighting::Studio;
        {
            let vchunk = 8192;
            let mut jobs: Vec<(usize, &mut [VertOut])> = Vec::new();
            let mut rest: &mut [VertOut] = &mut sc.verts;
            let mut offset = 0usize;
            for &k in &visible {
                let (start, count) = (self.objects[k].verts.0 as usize, self.objects[k].verts.1 as usize);
                let (_, tail) = core::mem::take(&mut rest).split_at_mut(start - offset);
                let (mine, tail) = tail.split_at_mut(count);
                rest = tail;
                offset = start + count;
                for (ci, ch) in mine.chunks_mut(vchunk).enumerate() {
                    jobs.push((start + ci * vchunk, ch));
                }
            }
            parallel_for_each(jobs.into_iter(), threads, |(first, out)| {
                for (k, o) in out.iter_mut().enumerate() {
                    let i = first + k;
                    let v = cam.to_view(self.positions[i]);
                    let sv = cam.svert(v);
                    let nw = self.normals[i];
                    let n = if nw == Vec3::ZERO { Vec3::ZERO } else { cam.rotate(nw) };
                    let (mut diff, mut add, mut bdiff, mut badd) = (0.0, 0.0, f32::NAN, 0.0);
                    if n != Vec3::ZERO && studio {
                        let e = to_eye_at(&cam, v);
                        (diff, add) = workbench::studio_terms(n, e, wb.specular);
                        // Back faces see the reversed normal (when it faces the eye).
                        if (-n).dot(e) > 0.0 {
                            (bdiff, badd) = workbench::studio_terms(-n, e, wb.specular);
                        }
                    }
                    *o = VertOut { v, s: sv, n, diff, add, bdiff, badd };
                }
            });
        }

        // 3. Setup stage: shaded, clipped, projected triangles binned by tile.
        let ranges: Vec<(usize, usize)> = visible.iter().map(|&k| (self.objects[k].tris.0 as usize, (self.objects[k].tris.0 + self.objects[k].tris.1) as usize)).filter(|r| r.1 > r.0).collect();
        let total: usize = ranges.iter().map(|r| r.1 - r.0).sum();
        let parts = split_ranges(&ranges, (threads * 2).min(total.div_ceil(2048)).clamp(1, 250));
        sc.bins.begin(layout, parts.len());
        {
            let verts = &sc.verts;
            let cull = s.backface_culling;
            sc.bins.setup(threads, |k, piece| {
                for &(a, b) in &parts[k] {
                    for ti in a..b {
                        self.setup_tri(ti, verts, &cam, cull, wb, piece);
                    }
                }
            });
        }

        // 4. Raster stage: per-tile visibility buffer.
        {
            sc.depth.resize(layout.len(), 0.0);
            sc.ids.resize(layout.len(), 0);
            sc.colors.resize(layout.len(), 0);
            let bins = &sc.bins;
            let items = sc.depth.chunks_mut(TILE_PX).zip(sc.ids.chunks_mut(TILE_PX)).zip(sc.colors.chunks_mut(TILE_PX)).enumerate();
            parallel_for_each(items, threads, |(t, ((d, id), col))| {
                d.fill(f32::NEG_INFINITY);
                id.fill(NO_ID);
                let rect = layout.tile_rect(t);
                bins.for_tile(t, |tid, tri, p| {
                    // Plain colours are interpolated here, while the triangle
                    // is in cache; other kinds are shaded in the resolve.
                    let direct = p.kind == KIND_COLOR && p.obj < 256;
                    super::raster::raster_tile(tri, rect, |k, x, y| {
                        let z = tri.z.at(x, y);
                        if z > d[k] {
                            d[k] = z;
                            id[k] = tid;
                            col[k] = if direct {
                                let b = tri.bary(x, y);
                                pack_obj(p.c[0] * b[0] + p.c[1] * b[1] + p.c[2] * b[2], p.obj)
                            } else {
                                0
                            };
                        }
                    });
                });
            });
        }
        let xray = wb.xray;
        if xray {
            sc.accum.resize(layout.len(), [0.0; 4]);
            sc.reveal.resize(layout.len(), 1.0);
            let (bins, verts, depth) = (&sc.bins, &sc.verts, &sc.depth);
            let alpha = wb.xray_alpha.clamp(0.0, 1.0);
            parallel_for_each(sc.accum.chunks_mut(TILE_PX).zip(sc.reveal.chunks_mut(TILE_PX)).enumerate(), threads, |(t, (acc, rev))| {
                acc.fill([0.0; 4]);
                rev.fill(1.0);
                let rect = layout.tile_rect(t);
                let near = &depth[t * TILE_PX..(t + 1) * TILE_PX];
                bins.for_tile(t, |_, tri, p| {
                    super::raster::raster_tile(tri, rect, |k, x, y| {
                        let c = self.shade(tri, p, x, y, verts, &cam, wb);
                        let (d, dn) = (cam.depth_of(tri.z.at(x, y)), cam.depth_of(near[k]));
                        let wgt = alpha * (-(d - dn).max(0.0) / (0.1 * dn.abs() + 1e-3)).exp().max(1e-4);
                        let a = &mut acc[k];
                        a[0] += c.x * wgt;
                        a[1] += c.y * wgt;
                        a[2] += c.z * wgt;
                        a[3] += wgt;
                        rev[k] *= 1.0 - alpha;
                    });
                });
            });
        }

        // Shadow map from the light direction (kept while the geometry is).
        let shadow = if wb.shadow && !xray && !self.bounds.is_empty() {
            let dir = wb.light_direction.normalize();
            let key = (self.generation, dir.to_array().map(f32::to_bits), self.positions.len());
            if sc.shadow.as_ref().is_none_or(|(k, _)| *k != key) {
                let lc = light_camera(dir, self.bounds, 2048);
                let sm = ShadowMap::render(lc, threads, self.tris.len(), &mut sc.shadow_bins, |i| Some(self.tris[i].v.map(|v| self.positions[v as usize])));
                sc.shadow = Some((key, sm));
            }
            sc.shadow.as_ref().map(|s| &s.1)
        } else {
            None
        };

        // 5. Resolve: background, surfaces, shadow; object ids and normals.
        let band_h = TILE;
        let cavity = wb.cavity && !xray;
        sc.obj.resize(w * h, 0);
        if cavity {
            sc.normals.resize(w * h, Vec3::ZERO);
        }
        {
            let (bins, verts, depth, ids, colors) = (&sc.bins, &sc.verts, &sc.depth, &sc.ids, &sc.colors);
            let normals_out: Option<&mut [Vec3]> = if cavity { Some(&mut sc.normals[..]) } else { None };
            let mut normal_chunks = normals_out.map(|n| n.chunks_mut(w * band_h));
            let jobs: Vec<_> = out.chunks_mut(w * 4 * band_h).zip(sc.obj.chunks_mut(w * band_h)).map(|(a, b)| (a, b, normal_chunks.as_mut().and_then(|n| n.next()))).enumerate().collect();
            parallel_for_each(jobs.into_iter(), threads, |(b, (rgba, objs, mut nrm))| {
                let y0 = b * band_h;
                let rows = objs.len() / w;
                for y in y0..y0 + rows {
                    let t = y as f32 / (h.max(2) - 1) as f32;
                    let bg = match s.background {
                        Some(bg) => bg,
                        None => [0, 1, 2].map(|i| BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t),
                    };
                    let bgb = [to_byte(bg[0]), to_byte(bg[1]), to_byte(bg[2]), if s.transparent { 0 } else { 255 }];
                    for x in 0..w {
                        let o = (y - y0) * w + x;
                        let li = layout.index(x, y);
                        let id = ids[li];
                        if let Some(u) = under {
                            let i = (y * w + x) * 4;
                            rgba[o * 4..o * 4 + 4].copy_from_slice(&u[i..i + 4]);
                            objs[o] = if id == NO_ID { 0 } else { bins.get(id).1.obj };
                            continue;
                        }
                        if id == NO_ID {
                            rgba[o * 4..o * 4 + 4].copy_from_slice(&bgb);
                            objs[o] = 0;
                            if let Some(n) = nrm.as_deref_mut() {
                                n[o] = Vec3::ZERO;
                            }
                            continue;
                        }
                        if xray {
                            objs[o] = bins.get(id).1.obj;
                            rgba[o * 4..o * 4 + 4].copy_from_slice(&bgb);
                            continue;
                        }
                        let packed = colors[li];
                        if packed != 0 && shadow.is_none() && nrm.is_none() {
                            // Interpolated while rasterizing.
                            objs[o] = packed >> 24;
                            let [r, g, b, _] = packed.to_le_bytes();
                            rgba[o * 4..o * 4 + 4].copy_from_slice(&[r, g, b, 255]);
                            continue;
                        }
                        let (tri, p) = bins.get(id);
                        objs[o] = p.obj;
                        let mut c = if packed != 0 {
                            let [r, g, b, _] = packed.to_le_bytes();
                            Vec3::new(r as f32, g as f32, b as f32) / 255.0
                        } else {
                            self.shade(tri, p, x, y, verts, &cam, wb)
                        };
                        if let Some(n) = nrm.as_deref_mut() {
                            n[o] = self.pixel_normal(p.src, tri.bary(x, y), verts, &cam);
                        }
                        if let Some(sm) = shadow {
                            let wp = cam.to_world(cam.unproject(x, y, depth[li]));
                            let [a, b2, c2] = self.tris[p.src as usize].v.map(|v| self.positions[v as usize]);
                            let ng = (b2 - a).cross(c2 - a).normalize();
                            let lit = sm.lit(wp, ng, 1.0, 0.0);
                            c = c * (1.0 - wb.shadow_intensity.clamp(0.0, 1.0) * (1.0 - lit));
                        }
                        rgba[o * 4..o * 4 + 4].copy_from_slice(&[to_byte(c.x), to_byte(c.y), to_byte(c.z), 255]);
                    }
                }
            });
        }

        // 6. Cavity and outlines (read neighbours, write this band).
        if cavity {
            let (normals, depth) = (&sc.normals, &sc.depth);
            parallel_for_each(out.chunks_mut(w * 4 * band_h).enumerate(), threads, |(b, rgba)| {
                let y0 = b * band_h;
                for (k, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let (x, y) = (k % w, y0 + k / w);
                    let f = cavity_factor(wb, normals, depth, &layout, &cam, w, h, x, y);
                    if f != 1.0 {
                        for c in px.iter_mut().take(3) {
                            *c = to_byte(*c as f32 / 255.0 * f);
                        }
                    }
                }
            });
        }
        if wb.outline {
            let (objs, depth) = (&sc.obj, &sc.depth);
            parallel_for_each(out.chunks_mut(w * 4 * band_h).enumerate(), threads, |(b, rgba)| {
                let y0 = b * band_h;
                for (k, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let (x, y) = (k % w, y0 + k / w);
                    if object_edge(objs, depth, &layout, w, h, x, y) {
                        for c in px.iter_mut().take(3) {
                            *c = (*c as u32 * 64 / 255) as u8;
                        }
                        px[3] = 255;
                    }
                }
            });
        }

        // 7. Lines: grid, gizmos, wireframe, loose edges, projected once.
        sc.lines.clear();
        if s.grid {
            if let Some(v) = view {
                grid_lines(v, &mut sc.lines);
            }
        }
        let grid_n = sc.lines.len();
        if s.overlays {
            for (slot, _, gl) in &self.gizmos {
                let c = self.highlight(*slot).unwrap_or(GIZMO);
                sc.lines.extend(gl.iter().map(|&(a, b)| Line { a, b, color: c, alpha: 1.0 }));
            }
        }
        let wire = |(e, slot): &([u32; 2], u32)| {
            let c = self.highlight(*slot).unwrap_or(WIRE);
            Line { a: self.positions[e[0] as usize], b: self.positions[e[1] as usize], color: c, alpha: 0.6 }
        };
        if s.wireframe {
            sc.lines.extend(self.edges.iter().map(wire));
        }
        if s.overlays {
            sc.lines.extend(self.loose.iter().map(wire));
        }
        sc.screen_lines.clear();
        let mut grid_screen = 0;
        for (i, l) in sc.lines.iter().enumerate() {
            if let Some(sl) = line_to_screen(&cam, l) {
                sc.screen_lines.push(sl);
                if i < grid_n {
                    grid_screen += 1;
                }
            }
        }
        let draw_lines = |out: &mut Vec<u8>, lines: &[ScreenLine], depth_test: bool| {
            if lines.is_empty() {
                return;
            }
            let depth = &sc.depth;
            parallel_for_each(out.chunks_mut(w * 4 * band_h).enumerate(), threads, |(b, rgba)| {
                let y0 = b * band_h;
                let rows = rgba.len() / (w * 4);
                let mut band = Band { w, y0, y1: y0 + rows, rgba, depth, layout, depth_test };
                for l in lines {
                    band.line(l);
                }
            });
        };
        if xray {
            // The grid shows through the see-through surfaces, which are
            // composited over it; gizmos and wires stay on top.
            draw_lines(out, &sc.screen_lines[..grid_screen], false);
            let (acc, rev) = (&sc.accum, &sc.reveal);
            parallel_for_each(out.chunks_mut(w * 4 * band_h).enumerate(), threads, |(b, rgba)| {
                let y0 = b * band_h;
                for (k, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let (x, y) = (k % w, y0 + k / w);
                    let li = layout.index(x, y);
                    let (a, r) = (acc[li], rev[li]);
                    if a[3] <= 0.0 {
                        continue;
                    }
                    let inv = 1.0 / a[3];
                    for c in 0..3 {
                        let cur = px[c] as f32 / 255.0;
                        px[c] = to_byte(a[c] * inv * (1.0 - r) + cur * r);
                    }
                    px[3] = px[3].max(to_byte(1.0 - r));
                }
            });
            draw_lines(out, &sc.screen_lines[grid_screen..], false);
        } else {
            draw_lines(out, &sc.screen_lines, true);
        }

        // 8. Selection outlines.
        if s.overlays && self.objects.iter().any(|o| o.selected) {
            let ids = &sc.obj;
            parallel_for_each(out.chunks_mut(w * 4 * band_h).enumerate(), threads, |(b, rgba)| {
                let y0 = b * band_h;
                for (k, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let (x, y) = (k % w, y0 + k / w);
                    if let Some(c) = self.outline(ids, w, h, x, y) {
                        px[..3].copy_from_slice(&c.map(to_byte));
                        px[3] = 255;
                    }
                }
            });
        }
    }

    /// The display colour of a covered pixel (before shadow and cavity).
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn shade(&self, tri: &SetupTri, p: &SolidTri, x: usize, y: usize, verts: &[VertOut], cam: &RasterCamera, wb: &Workbench) -> Vec3 {
        let b = tri.bary(x, y);
        let c = p.c[0] * b[0] + p.c[1] * b[1] + p.c[2] * b[2];
        let tex = |k: &Self| {
            let (uv, img) = k.tex_uvs[k.tris[p.src as usize].tex as usize];
            let u = uv[0] * b[0] + uv[1] * b[1] + uv[2] * b[2];
            k.textures[img as usize].sample(u.x, u.y)
        };
        match p.kind {
            KIND_COLOR => c,
            KIND_TEX_STUDIO => tex(self) * c.x + Vec3::splat(c.y),
            KIND_TEX_FLAT => tex(self),
            KIND_MATCAP => c.mul_elem(workbench::matcap(wb.matcap, self.pixel_normal(p.src, b, verts, cam))),
            _ => tex(self).mul_elem(workbench::matcap(wb.matcap, self.pixel_normal(p.src, b, verts, cam))),
        }
    }

    /// View-space unit normal facing the viewer at barycentrics `b` of
    /// triangle `src`.
    #[inline]
    fn pixel_normal(&self, src: u32, b: [f32; 3], verts: &[VertOut], cam: &RasterCamera) -> Vec3 {
        let t = &self.tris[src as usize];
        let vo = t.v.map(|i| &verts[i as usize]);
        let mut fnorm = (vo[1].v - vo[0].v).cross(vo[2].v - vo[0].v).normalize();
        if fnorm.dot(to_eye_at(cam, vo[0].v)) < 0.0 {
            fnorm = -fnorm;
        }
        if !t.smooth {
            return fnorm;
        }
        let n = vo[0].n * b[0] + vo[1].n * b[1] + vo[2].n * b[2];
        let n = if n.dot(fnorm) < 0.0 { -n } else { n }.normalize();
        if n == Vec3::ZERO { fnorm } else { n }
    }

    /// Base colours (display space) of a triangle's corners.
    fn base_colors(&self, t: &VTri, wb: &Workbench) -> [Vec3; 3] {
        if let Some(c) = self.corner_colors.get(t.corners as usize) {
            return *c;
        }
        let c = match wb.color {
            ColorType::Material => t.color,
            ColorType::Object => Vec3::ONE,
            ColorType::Attribute => return self.attr_colors.get(t.attr as usize).copied().unwrap_or([Vec3::splat(srgb_encode(0.8)); 3]),
            ColorType::Single => encode3(wb.single_color.map(|v| v.clamp(0.0, 1.0))),
            ColorType::Random => self.objects[t.obj as usize].random,
            ColorType::Texture => {
                if t.tex != NONE {
                    Vec3::ONE
                } else {
                    t.color
                }
            }
        };
        [c; 3]
    }

    fn outline(&self, ids: &[u32], w: usize, h: usize, x: usize, y: usize) -> Option<[f32; 3]> {
        let id = ids[y * w + x];
        // Fast path: inside a region (most pixels).
        let same = |xx: usize, yy: usize| ids[yy * w + xx] == id;
        if x > 0 && y > 0 && x + 1 < w && y + 1 < h && same(x - 1, y) && same(x + 1, y) && same(x, y - 1) && same(x, y + 1) {
            return None;
        }
        let mine = id.checked_sub(1).and_then(|s| self.highlight(s));
        let mut neighbour = None;
        for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
            let (xx, yy) = (x as i32 + dx, y as i32 + dy);
            if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                continue;
            }
            let q = ids[yy as usize * w + xx as usize];
            if q == id {
                continue;
            }
            if mine.is_some() {
                return mine;
            }
            neighbour = neighbour.or_else(|| q.checked_sub(1).and_then(|s| self.highlight(s)));
        }
        neighbour
    }

    fn highlight(&self, slot: u32) -> Option<[f32; 3]> {
        let o = &self.objects[slot as usize];
        if o.active {
            Some(ACTIVE)
        } else if o.selected {
            Some(SELECT)
        } else {
            None
        }
    }

    fn setup_tri(&self, ti: usize, verts: &[VertOut], cam: &RasterCamera, cull: bool, wb: &Workbench, out: &mut super::raster::Piece<SolidTri>) {
        let t = &self.tris[ti];
        let vo = t.v.map(|i| &verts[i as usize]);
        let p = vo.map(|v| v.v);
        let fnorm = (p[1] - p[0]).cross(p[2] - p[0]);
        let to_eye = if cam.ortho { Vec3::Z } else { -p[0] };
        let front = fnorm.dot(to_eye) >= 0.0;
        if cull && !front {
            return;
        }
        let near = cam.near_z();
        let inside = p.map(|q| q.z <= near);
        if !inside.iter().any(|&i| i) {
            return;
        }
        let all_inside = inside.iter().all(|&i| i);
        if all_inside && !covers_pixel(&cam_size(cam), [vo[0].s, vo[1].s, vo[2].s]) {
            return;
        }
        let fnorm = if front { fnorm } else { -fnorm }.normalize();
        if fnorm == Vec3::ZERO {
            return;
        }
        let base = self.base_colors(t, wb);
        let textured = wb.color == ColorType::Texture && t.tex != NONE && t.corners == NONE;
        let (kind, c) = match wb.lighting {
            Lighting::Studio => {
                // Corner lighting: per vertex for smooth front faces, else per face.
                let terms: [(f32, f32); 3] = if t.smooth && front {
                    [0, 1, 2].map(|k| (vo[k].diff, vo[k].add))
                } else if t.smooth {
                    [0, 1, 2].map(|k| if vo[k].bdiff.is_nan() { workbench::studio_terms(fnorm, to_eye_at(cam, p[k]), wb.specular) } else { (vo[k].bdiff, vo[k].badd) })
                } else {
                    // Flat: the face normal, but the view direction per corner, so the
                    // triangles of one polygon shade seamlessly.
                    [0, 1, 2].map(|k| workbench::studio_terms(fnorm, to_eye_at(cam, p[k]), wb.specular))
                };
                if textured {
                    (KIND_TEX_STUDIO, terms.map(|(d, a)| Vec3::new(d, a, 0.0)))
                } else {
                    (KIND_COLOR, [0, 1, 2].map(|k| base[k] * terms[k].0 + Vec3::splat(terms[k].1)))
                }
            }
            Lighting::Flat => (if textured { KIND_TEX_FLAT } else { KIND_COLOR }, base),
            Lighting::MatCap => (if textured { KIND_TEX_MATCAP } else { KIND_MATCAP }, base),
        };
        let data = SolidTri { c, obj: t.obj + 1, src: ti as u32, kind };
        if all_inside {
            out.push([vo[0].s, vo[1].s, vo[2].s], [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], Facing::Both, data);
            return;
        }
        cam.clip_emit(p, |v, b| {
            out.push(v, b, Facing::Both, data);
        });
    }

    /// The object (scene index) under pixel (`x`, `y`): meshes by ray cast,
    /// lights, cameras and empties within 8 pixels of their origin.
    /// The depth buffer of the last [`ViewportScene::draw_into`]: per pixel
    /// the depth key (1/distance in perspective, view z in ortho; larger is
    /// closer, -inf where nothing was drawn). Empty if never drawn.
    pub fn last_depth(&self) -> Vec<f32> {
        self.scratch.lock().map(|s| s.depth.clone()).unwrap_or_default()
    }

    pub fn pick(&self, view: &ViewCamera, width: u32, height: u32, x: f32, y: f32) -> Option<usize> {
        let (o, d) = view.ray(x, y, width, height);
        let mut best: Option<(f32, u32)> = None;
        for t in &self.tris {
            let [a, b, c] = t.v.map(|i| self.positions[i as usize]);
            let g = super::bvh::TriGeom::new(a, b, c);
            if let Some((dist, _, _)) = g.intersect(o, d, best.map_or(f32::INFINITY, |b| b.0)) {
                best = Some((dist, t.obj));
            }
        }
        let proj = RasterCamera::from_view(view, width, height);
        for (slot, origin, _) in &self.gizmos {
            let v = proj.to_view(*origin);
            if let Some((sx, sy, _)) = proj.project(v) {
                let dist = -v.z;
                if (sx - x).hypot(sy - y) <= 8.0 && (proj.ortho || dist > 0.0) && best.is_none_or(|b| dist < b.0) {
                    best = Some((dist, *slot));
                }
            }
        }
        best.map(|(_, s)| self.objects[s as usize].index)
    }
}

/// Draw the scene from `view` (builds a [`ViewportScene`] first).
pub fn draw(scene: &Scene, view: &ViewCamera, settings: &ViewportSettings) -> Vec<u8> {
    ViewportScene::new(scene).draw(view, settings)
}

/// Object under pixel (`x`, `y`) of a `width` x `height` view, if any.
pub fn pick(scene: &Scene, view: &ViewCamera, width: u32, height: u32, x: f32, y: f32) -> Option<usize> {
    ViewportScene::new(scene).pick(view, width, height, x, y)
}

fn cam_size(cam: &RasterCamera) -> (f32, f32) {
    (cam.width as f32, cam.height as f32)
}

/// Cheap early test: can the triangle's bounds contain a pixel centre of
/// the frame? (Most triangles of dense meshes are smaller than a pixel.)
#[inline(always)]
fn covers_pixel(size: &(f32, f32), v: [SVert; 3]) -> bool {
    let (xmin, xmax) = (v[0].x.min(v[1].x).min(v[2].x), v[0].x.max(v[1].x).max(v[2].x));
    let (ymin, ymax) = (v[0].y.min(v[1].y).min(v[2].y), v[0].y.max(v[1].y).max(v[2].y));
    let x0 = fceil(xmin - 0.5).max(0.0);
    let x1 = ffloor(xmax - 0.5).min(size.0 - 1.0);
    let y0 = fceil(ymin - 0.5).max(0.0);
    let y1 = ffloor(ymax - 0.5).min(size.1 - 1.0);
    x1 >= x0 && y1 >= y0
}

fn to_eye_at(cam: &RasterCamera, p: Vec3) -> Vec3 {
    if cam.ortho { Vec3::Z } else { (-p).normalize() }
}

/// An orthographic light camera looking along `-dir` over `bounds`.
pub(crate) fn light_camera(dir: Vec3, bounds: Aabb, res: u32) -> RasterCamera {
    let back = dir.normalize();
    let right = back.any_orthonormal().normalize();
    let up = back.cross(right).normalize();
    let r = (bounds.size().length() * 0.5).max(1e-3) * 1.02;
    let eye = bounds.center() + back * (r * 2.0);
    let half = res as f32 * 0.5;
    RasterCamera { eye, right, up, back, ortho: true, scale: half / r, cx: half, cy: half, near: 1e-4, width: res, height: res }
}

const KIND_COLOR: u8 = 0;
const KIND_TEX_STUDIO: u8 = 1;
const KIND_TEX_FLAT: u8 = 2;
const KIND_MATCAP: u8 = 3;
const KIND_TEX_MATCAP: u8 = 4;

/// What the Solid resolve needs of a triangle: corner values (lit colours,
/// base colours for MatCap, or diffuse/add factors over a texture), the
/// object slot + 1 and the source triangle.
#[derive(Clone, Copy, Debug)]
struct SolidTri {
    c: [Vec3; 3],
    obj: u32,
    src: u32,
    kind: u8,
}

/// Screen-space cavity (curvature) and world cavity (ambient occlusion
/// over the depth buffer) at a pixel: a factor on the colour.
#[allow(clippy::too_many_arguments)]
fn cavity_factor(wb: &Workbench, normals: &[Vec3], depth: &[f32], l: &Layout, cam: &RasterCamera, w: usize, h: usize, x: usize, y: usize) -> f32 {
    let n = normals[y * w + x];
    if n == Vec3::ZERO {
        return 1.0;
    }
    let mut f = 1.0;
    if matches!(wb.cavity_type, CavityType::Screen | CavityType::Both) {
        let at = |xx: usize, yy: usize| {
            let q = normals[yy * w + xx];
            if q == Vec3::ZERO { n } else { q }
        };
        let (l_, r) = (at(x.saturating_sub(1), y), at((x + 1).min(w - 1), y));
        let (u, d) = (at(x, y.saturating_sub(1)), at(x, (y + 1).min(h - 1)));
        let curv = 0.5 * ((r.x - l_.x) + (u.y - d.y));
        let c = if curv > 0.0 { curv * wb.curvature_ridge } else { curv * wb.curvature_valley };
        f *= 1.0 + c.clamp(-0.6, 0.6);
    }
    if matches!(wb.cavity_type, CavityType::World | CavityType::Both) {
        let key = depth[l.index(x, y)];
        let p = cam.unproject(x, y, key);
        let dist = 0.2 * (1.0 + 0.02 * cam.depth_of(key).abs());
        let r_px = if cam.ortho { dist * cam.scale } else { dist * cam.scale / (-p.z).max(1e-4) };
        if r_px >= 1.0 {
            // A fixed spiral of 12 taps, rotated per pixel.
            let rot = (((x * 7 + y * 13) % 16) as f32) * 0.3927;
            let (mut valley, mut ridge) = (0.0, 0.0);
            for k in 0..12 {
                let a = rot + k as f32 * 2.399_963;
                let rr = r_px.min(64.0) * ((k as f32 + 0.5) / 12.0).sqrt();
                let (sx, sy) = (x as f32 + a.cos() * rr, y as f32 + a.sin() * rr);
                if !(sx >= 0.0 && sy >= 0.0 && sx < w as f32 && sy < h as f32) {
                    continue;
                }
                let (qx, qy) = (sx as usize, sy as usize);
                let qk = depth[l.index(qx, qy)];
                if qk == f32::NEG_INFINITY {
                    continue;
                }
                let d = cam.unproject(qx, qy, qk) - p;
                let len = d.length();
                if !(len > 1e-6) || len > dist * 2.0 {
                    continue;
                }
                let cos = n.dot(d) / len;
                let fall = 1.0 - (len / (dist * 2.0));
                valley += (cos - 0.1).max(0.0) * fall;
                ridge += (-cos - 0.1).max(0.0) * fall;
            }
            f *= (1.0 - wb.cavity_valley * valley / 12.0 * 1.5).clamp(0.2, 1.0) * (1.0 + wb.cavity_ridge * ridge / 12.0 * 0.8);
        }
    }
    f
}

/// Whether a pixel is on the near side of an object's silhouette.
fn object_edge(objs: &[u32], depth: &[f32], l: &Layout, w: usize, h: usize, x: usize, y: usize) -> bool {
    let id = objs[y * w + x];
    if id == 0 {
        return false;
    }
    let d = depth[l.index(x, y)];
    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
        let (xx, yy) = (x as i32 + dx, y as i32 + dy);
        if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
            continue;
        }
        let q = objs[yy as usize * w + xx as usize];
        if q != id && (q == 0 || depth[l.index(xx as usize, yy as usize)] < d) {
            return true;
        }
    }
    false
}

/// Vertex-stage output.
#[derive(Clone, Copy, Debug, Default)]
struct VertOut {
    /// View-space position.
    v: Vec3,
    /// Projected (valid when in front of the near plane).
    s: SVert,
    /// View-space smooth normal (zero for flat-shaded objects).
    n: Vec3,
    /// Studio lighting of the smooth vertex normal.
    diff: f32,
    add: f32,
    /// The same for the reversed normal (back faces); NaN when it faces away.
    bdiff: f32,
    badd: f32,
}

/// Per-frame working memory, reused between draws.
#[derive(Debug, Default)]
struct Scratch {
    verts: Vec<VertOut>,
    bins: Bins<SolidTri>,
    /// Tile-major visibility buffer.
    depth: Vec<f32>,
    ids: Vec<u32>,
    /// Colours interpolated during rasterization: RGB and the object slot
    /// + 1 in the top byte (0: shade in the resolve).
    colors: Vec<u32>,
    /// Row-major object slot + 1 per pixel (0: background).
    obj: Vec<u32>,
    /// Row-major view normals (cavity).
    normals: Vec<Vec3>,
    /// X-ray accumulation (tile-major).
    accum: Vec<[f32; 4]>,
    reveal: Vec<f32>,
    lines: Vec<Line>,
    screen_lines: Vec<ScreenLine>,
    shadow: Option<((u64, [u32; 3], usize), ShadowMap)>,
    shadow_bins: Bins<()>,
}

fn line_to_screen(cam: &RasterCamera, l: &Line) -> Option<ScreenLine> {
    let (mut a, mut b) = (cam.to_view(l.a), cam.to_view(l.b));
    let nz = cam.near_z();
    if a.z > nz && b.z > nz {
        return None;
    }
    if a.z > nz {
        a = b.lerp(a, (nz - b.z) / (a.z - b.z));
    } else if b.z > nz {
        b = a.lerp(b, (nz - a.z) / (b.z - a.z));
    }
    let (pa, pb) = (cam.project(a)?, cam.project(b)?);
    Some(ScreenLine { a: pa, b: pb, color: l.color, alpha: l.alpha })
}

#[derive(Clone, Copy, Debug)]
struct ScreenLine {
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    color: [f32; 3],
    alpha: f32,
}

/// RGB bytes and the object slot + 1 (when it fits a byte; else it is
/// looked up) in the top byte; 0 means "not packed".
#[inline(always)]
fn pack_obj(c: Vec3, obj: u32) -> u32 {
    u32::from_le_bytes([to_byte(c.x), to_byte(c.y), to_byte(c.z), obj as u8])
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The rows `y0..y1` of the frame, owned by one worker while drawing lines.
struct Band<'a> {
    w: usize,
    y0: usize,
    y1: usize,
    rgba: &'a mut [u8],
    /// The whole frame's depth keys (tile-major).
    depth: &'a [f32],
    layout: Layout,
    depth_test: bool,
}

impl Band<'_> {
    /// Depth-tested, alpha-blended one-pixel line (DDA), clipped to the band.
    fn line(&mut self, l: &ScreenLine) {
        let ((xa, ya, za), (xb, yb, zb)) = (l.a, l.b);
        let (dx, dy) = (xb - xa, yb - ya);
        let steps = dx.abs().max(dy.abs()).ceil();
        if !steps.is_finite() || steps > 1e5 {
            return;
        }
        let steps = steps.max(1.0);
        // Parameter range inside the band's rows (with a pixel of margin).
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        if dy.abs() > 1e-6 {
            let (ta, tb) = ((self.y0 as f32 - 1.0 - ya) / dy, (self.y1 as f32 + 1.0 - ya) / dy);
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
        } else if ya < self.y0 as f32 - 1.0 || ya > self.y1 as f32 + 1.0 {
            return;
        }
        if t1 < t0 {
            return;
        }
        let (k0, k1) = ((t0 * steps).floor() as i64, (t1 * steps).ceil() as i64);
        let a = l.alpha;
        for k in k0..=k1 {
            let t = k as f32 / steps;
            let (px, py) = (ffloor(xa + dx * t), ffloor(ya + dy * t));
            if px < 0.0 || py < self.y0 as f32 || px >= self.w as f32 || py >= self.y1 as f32 {
                continue;
            }
            let (px, py) = (px as usize, py as usize);
            if self.depth_test {
                let d = za + (zb - za) * t;
                let zbuf = self.depth[self.layout.index(px, py)];
                if d < zbuf - zbuf.abs() * 2e-3 - 1e-7 {
                    continue;
                }
            }
            let o = ((py - self.y0) * self.w + px) * 4;
            for c in 0..3 {
                let cur = self.rgba[o + c] as f32 / 255.0;
                self.rgba[o + c] = to_byte(cur + (l.color[c] - cur) * a);
            }
            self.rgba[o + 3] = self.rgba[o + 3].max(to_byte(a));
        }
    }
}

/// Floor grid on z = 0 around the view target with X (red) and Y (green) axes.
fn grid_lines(view: &ViewCamera, out: &mut Vec<Line>) {
    let spacing = 10f32.powf((view.distance / 15.0).max(1e-3).log10().floor().max(0.0));
    let extent = (view.distance * 2.0 / spacing).ceil().clamp(10.0, 100.0) as i32;
    let (cx, cy) = ((view.target.x / spacing).round() as i32, (view.target.y / spacing).round() as i32);
    let fade = |p: Vec3| (1.0 - (p - Vec3::new(view.target.x, view.target.y, 0.0)).length() / (extent as f32 * spacing)).max(0.0);
    let mut seg = |a: Vec3, b: Vec3, color: [f32; 3], alpha: f32| {
        let al = alpha * fade((a + b) * 0.5);
        if al > 0.01 {
            out.push(Line { a, b, color, alpha: al });
        }
    };
    const GRID: [f32; 3] = [0.38, 0.38, 0.38];
    const XAXIS: [f32; 3] = [0.85, 0.24, 0.29];
    const YAXIS: [f32; 3] = [0.45, 0.68, 0.18];
    // Each line is split into a few segments so the fade can follow distance.
    const SEGMENTS: i32 = 12;
    let span = 2.0 * extent as f32 * spacing;
    for i in -extent..=extent {
        let x = (cx + i) as f32 * spacing;
        let y = (cy + i) as f32 * spacing;
        for k in 0..SEGMENTS {
            let (t0, t1) = (k as f32 / SEGMENTS as f32 * span, (k + 1) as f32 / SEGMENTS as f32 * span);
            // Lines parallel to Y at x, and parallel to X at y.
            let (ya, yb) = ((cy - extent) as f32 * spacing + t0, (cy - extent) as f32 * spacing + t1);
            let (c, a) = if cx + i == 0 { (YAXIS, 1.0) } else { (GRID, 0.6) };
            seg(Vec3::new(x, ya, 0.0), Vec3::new(x, yb, 0.0), c, a);
            let (xa, xb) = ((cx - extent) as f32 * spacing + t0, (cx - extent) as f32 * spacing + t1);
            let (c, a) = if cy + i == 0 { (XAXIS, 1.0) } else { (GRID, 0.6) };
            seg(Vec3::new(xa, y, 0.0), Vec3::new(xb, y, 0.0), c, a);
        }
    }
}

/// Wire gizmo for non-mesh objects, in object space.
fn gizmo_lines(data: &ObjectData) -> Vec<(Vec3, Vec3)> {
    let mut l = Vec::new();
    match data {
        ObjectData::Camera(c) => {
            let d = 1.0;
            let hy = d * (c.fov_y * 0.5).tan();
            let hx = hy * 16.0 / 9.0;
            let corners = [Vec3::new(-hx, -hy, -d), Vec3::new(hx, -hy, -d), Vec3::new(hx, hy, -d), Vec3::new(-hx, hy, -d)];
            for k in 0..4 {
                l.push((Vec3::ZERO, corners[k]));
                l.push((corners[k], corners[(k + 1) % 4]));
            }
            // Up triangle.
            l.push((Vec3::new(-hx * 0.5, hy * 1.1, -d), Vec3::new(0.0, hy * 1.6, -d)));
            l.push((Vec3::new(0.0, hy * 1.6, -d), Vec3::new(hx * 0.5, hy * 1.1, -d)));
        }
        ObjectData::Light(_) => {
            let r = 0.2;
            for k in 0..16 {
                let (a0, a1) = (k as f32 / 16.0 * core::f32::consts::TAU, (k + 1) as f32 / 16.0 * core::f32::consts::TAU);
                l.push((Vec3::new(a0.cos() * r, a0.sin() * r, 0.0), Vec3::new(a1.cos() * r, a1.sin() * r, 0.0)));
            }
            l.push((Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0)));
        }
        _ => {
            for a in [Vec3::X, Vec3::Y, Vec3::Z] {
                l.push((-a, a));
            }
        }
    }
    l
}
