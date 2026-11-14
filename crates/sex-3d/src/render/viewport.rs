// SPDX-License-Identifier: GPL-3.0-only
//! The 3D view's "Solid" mode: a multithreaded CPU z-buffer rasterizer.
//!
//! [`ViewportScene`] caches the evaluated, triangulated, world-space geometry
//! of a scene (rebuild it when the scene changes; [`ViewportScene::sync_selection`]
//! is enough for selection changes). [`ViewportScene::draw`] renders it from an
//! orbit [`ViewCamera`]:
//!
//! 1. vertex stage: every vertex to view space (parallel chunks);
//! 2. setup stage: per triangle studio lighting at the corners (headlight,
//!    key, fill, rim, a little specular, on the material base colour), near
//!    clipping, projection and binning into horizontal bands (parallel chunks);
//! 3. raster stage: each band of rows is owned by one worker, which fills its
//!    triangles (edge functions with per-row span narrowing, perspective
//!    correct colour, a reversed `1/w` depth buffer and an object-ID buffer),
//!    then draws the floor grid, axes, object gizmos and optional wireframe as
//!    depth-tested lines;
//! 4. outline stage: selected objects get Blender's orange silhouette from
//!    the ID buffer.
//!
//! No allocation happens per pixel or per triangle in the hot loops.
use super::{parallel_for_each, resolve_threads};
use super::tonemap::srgb_encode;
use crate::math::{Aabb, Vec3};
use crate::scene::{ObjectData, Scene};
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
    fn near(&self) -> f32 {
        (self.distance * 1e-3).max(1e-4)
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
}

impl Default for ViewportSettings {
    fn default() -> Self {
        ViewportSettings { width: 1280, height: 720, wireframe: false, grid: true, backface_culling: false, threads: 0 }
    }
}

const BG_TOP: [f32; 3] = [0.28, 0.28, 0.28];
const BG_BOTTOM: [f32; 3] = [0.20, 0.20, 0.20];
const SELECT: [f32; 3] = [0.93, 0.36, 0.0];
const ACTIVE: [f32; 3] = [1.0, 0.63, 0.16];
const WIRE: [f32; 3] = [0.0, 0.0, 0.0];
const GIZMO: [f32; 3] = [0.05, 0.05, 0.05];

#[derive(Clone, Copy, Debug)]
struct VTri {
    v: [u32; 3],
    /// Display-space base colour.
    color: Vec3,
    /// 0 = flat; otherwise smooth (use vertex normals).
    smooth: bool,
    /// Object slot in [`ViewportScene::objects`].
    obj: u32,
}

#[derive(Clone, Copy, Debug)]
struct ObjInfo {
    /// Index into `Scene::objects`.
    index: usize,
    selected: bool,
    active: bool,
}

#[derive(Clone, Copy, Debug)]
struct Line {
    a: Vec3,
    b: Vec3,
    color: [f32; 3],
    alpha: f32,
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
            scratch: Mutex::default(),
        }
    }
}

impl ViewportScene {
    pub fn new(scene: &Scene) -> ViewportScene {
        let mut vs = ViewportScene::default();
        for oi in 0..scene.objects.len() {
            if !scene.is_visible(oi) {
                continue;
            }
            let slot = vs.objects.len() as u32;
            let m = scene.world_matrix(oi);
            match &scene.objects[oi].data {
                ObjectData::Mesh(_) => {
                    let Some((mesh, slots)) = scene.world_mesh(oi) else { continue };
                    vs.objects.push(ObjInfo { index: oi, selected: false, active: false });
                    let base = vs.positions.len() as u32;
                    vs.positions.extend_from_slice(&mesh.positions);
                    if mesh.faces.iter().any(|f| f.smooth) {
                        vs.normals.extend(mesh.vertex_normals());
                    } else {
                        vs.normals.resize(vs.positions.len(), Vec3::ZERO);
                    }
                    let colors: Vec<Vec3> = slots
                        .iter()
                        .map(|&s| scene.materials.get(s).map_or([0.8; 4], |m| m.base_color))
                        .map(|c| Vec3::new(srgb_encode(c[0]), srgb_encode(c[1]), srgb_encode(c[2])))
                        .collect();
                    let default = Vec3::splat(srgb_encode(0.8));
                    for t in mesh.triangulate() {
                        let f = &mesh.faces[t.face as usize];
                        let color = colors.get(f.material as usize).copied().unwrap_or(default);
                        vs.tris.push(VTri { v: t.verts.map(|v| v + base), color, smooth: f.smooth, obj: slot });
                    }
                    vs.edges.extend(mesh.edges().into_iter().map(|e| ([e[0] + base, e[1] + base], slot)));
                    vs.loose.extend(mesh.loose_edges.iter().map(|e| ([e[0] + base, e[1] + base], slot)));
                }
                data => {
                    vs.objects.push(ObjInfo { index: oi, selected: false, active: false });
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
        let proj = Projection::new(view, s.width, s.height);

        // 1. Vertex stage: view space, projection and smooth-normal lighting.
        sc.verts.resize(self.positions.len(), VertOut::default());
        let vchunk = 8192;
        parallel_for_each(sc.verts.chunks_mut(vchunk).enumerate(), threads, |(ci, out)| {
            for (k, o) in out.iter_mut().enumerate() {
                let i = ci * vchunk + k;
                let v = proj.to_view(self.positions[i]);
                let (sx, sy, key) = proj.project(v).unwrap_or((0.0, 0.0, 0.0));
                let (diff, add) = if self.normals[i] == Vec3::ZERO { (0.0, 0.0) } else { studio_terms(proj.rotate(self.normals[i]), to_eye_at(&proj, v)) };
                *o = VertOut { v, sx, sy, key, iw: proj.inv_w(v), diff, add };
            }
        });

        // 2. Setup stage: shaded, clipped, projected triangles binned by band.
        let band_h = (h / (threads * 4)).clamp(8, 64);
        let bands = h.div_ceil(band_h);
        let chunk = self.tris.len().div_ceil(threads.max(1)).max(4096);
        let pieces = self.tris.len().div_ceil(chunk);
        sc.setups.resize_with(pieces, Setup::default);
        for st in &mut sc.setups {
            st.reset(bands);
        }
        let verts = &sc.verts;
        parallel_for_each(sc.setups.iter_mut().zip(self.tris.chunks(chunk)), threads, |(res, tris)| {
            for t in tris {
                self.setup_tri(t, verts, &proj, s.backface_culling, band_h, res);
            }
        });

        // Lines: grid, gizmos, wireframe, loose edges, projected once.
        sc.lines.clear();
        if s.grid {
            grid_lines(view, &mut sc.lines);
        }
        for (slot, _, gl) in &self.gizmos {
            let c = self.highlight(*slot).unwrap_or(GIZMO);
            sc.lines.extend(gl.iter().map(|&(a, b)| Line { a, b, color: c, alpha: 1.0 }));
        }
        let wire = |(e, slot): &([u32; 2], u32)| {
            let c = self.highlight(*slot).unwrap_or(WIRE);
            Line { a: self.positions[e[0] as usize], b: self.positions[e[1] as usize], color: c, alpha: 0.6 }
        };
        if s.wireframe {
            sc.lines.extend(self.edges.iter().map(wire));
        }
        sc.lines.extend(self.loose.iter().map(wire));
        sc.screen_lines.clear();
        sc.screen_lines.extend(sc.lines.iter().filter_map(|l| proj.line(l)));

        // 3. Raster stage, one band of rows per task.
        sc.depth.resize(w * h, 0.0);
        sc.ids.resize(w * h, 0);
        let (setups, lines) = (&sc.setups, &sc.screen_lines);
        parallel_for_each(out.chunks_mut(w * 4 * band_h).zip(sc.depth.chunks_mut(w * band_h)).zip(sc.ids.chunks_mut(w * band_h)).enumerate(), threads, |(b, ((rgba, depth), ids))| {
            let y0 = b * band_h;
            let rows = depth.len() / w;
            let mut band = Band { w, h, y0, y1: y0 + rows, rgba, depth, ids };
            band.clear();
            for st in setups {
                for &ti in &st.bins[b] {
                    band.triangle(&st.tris[ti as usize]);
                }
            }
            for l in lines {
                band.line(l);
            }
        });

        // 4. Selection outlines.
        if self.objects.iter().any(|o| o.selected) {
            let ids = &sc.ids;
            parallel_for_each(out.chunks_mut(w * 4 * band_h).enumerate(), threads, |(b, rgba)| {
                let y0 = b * band_h;
                for (k, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let (x, y) = (k % w, y0 + k / w);
                    if let Some(c) = self.outline(ids, w, h, x, y) {
                        px[..3].copy_from_slice(&c.map(to_byte));
                    }
                }
            });
        }
    }

    /// Outline colour at a pixel: on either side of a selected silhouette.
    fn outline(&self, ids: &[u32], w: usize, h: usize, x: usize, y: usize) -> Option<[f32; 3]> {
        let id = ids[y * w + x];
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

    fn setup_tri(&self, t: &VTri, verts: &[VertOut], proj: &Projection, cull: bool, band_h: usize, out: &mut Setup) {
        let vo = t.v.map(|i| &verts[i as usize]);
        let p = vo.map(|v| v.v);
        let fnorm = (p[1] - p[0]).cross(p[2] - p[0]);
        let to_eye = if proj.ortho { Vec3::Z } else { -p[0] };
        let front = fnorm.dot(to_eye) >= 0.0;
        if cull && !front {
            return;
        }
        let near = proj.near_z();
        let inside = p.map(|q| q.z <= near);
        if !inside.iter().any(|&i| i) {
            return;
        }
        let fnorm = if front { fnorm } else { -fnorm }.normalize();
        if fnorm == Vec3::ZERO {
            return;
        }
        // Corner colours: per-vertex lighting for smooth front faces, else per face.
        let c = if t.smooth && front {
            vo.map(|v| t.color * v.diff + Vec3::splat(v.add))
        } else if t.smooth {
            let mut c = [Vec3::ZERO; 3];
            for k in 0..3 {
                let n = -proj.rotate(self.normals[t.v[k] as usize]);
                let e = to_eye_at(proj, p[k]);
                let (d, a) = studio_terms(if n.dot(e) > 0.0 { n } else { fnorm }, e);
                c[k] = t.color * d + Vec3::splat(a);
            }
            c
        } else {
            // Flat: the face normal, but the view direction per corner, so the
            // triangles of one polygon shade seamlessly.
            [0, 1, 2].map(|k| {
                let (d, a) = studio_terms(fnorm, to_eye_at(proj, p[k]));
                t.color * d + Vec3::splat(a)
            })
        };
        let obj = t.obj + 1;
        if inside.iter().all(|&i| i) {
            let pv = |k: usize| ProjVert { x: vo[k].sx, y: vo[k].sy, z: vo[k].key, iw: vo[k].iw, c: c[k] };
            out.push(proj, [pv(0), pv(1), pv(2)], obj, band_h);
            return;
        }
        // Clip against the near plane (Sutherland-Hodgman, at most 4 vertices).
        let mut poly = [(Vec3::ZERO, Vec3::ZERO); 4];
        let mut n = 0;
        for k in 0..3 {
            let (a, b) = ((p[k], c[k]), (p[(k + 1) % 3], c[(k + 1) % 3]));
            let (ia, ib) = (inside[k], inside[(k + 1) % 3]);
            if ia {
                poly[n] = a;
                n += 1;
            }
            if ia != ib {
                let s = (near - a.0.z) / (b.0.z - a.0.z);
                poly[n] = (a.0.lerp(b.0, s), a.1.lerp(b.1, s));
                n += 1;
            }
        }
        let pv = |(v, c): (Vec3, Vec3)| {
            let (x, y, z) = proj.project(v).unwrap_or((0.0, 0.0, 0.0));
            ProjVert { x, y, z, iw: proj.inv_w(v), c }
        };
        for k in 1..n.saturating_sub(1) {
            out.push(proj, [pv(poly[0]), pv(poly[k]), pv(poly[k + 1])], obj, band_h);
        }
    }

    /// The object (scene index) under pixel (`x`, `y`): meshes by ray cast,
    /// lights, cameras and empties within 8 pixels of their origin.
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
        let proj = Projection::new(view, width, height);
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

fn to_eye_at(proj: &Projection, p: Vec3) -> Vec3 {
    if proj.ortho { Vec3::Z } else { (-p).normalize() }
}

/// Solid-mode studio lighting in view space for unit `n` and `v` facing the
/// viewer: (factor on the base colour, white added on top).
fn studio_terms(n: Vec3, v: Vec3) -> (f32, f32) {
    const KEY: Vec3 = Vec3 { x: -0.408, y: 0.571, z: 0.712 };
    const FILL: Vec3 = Vec3 { x: 0.6, y: -0.48, z: 0.64 };
    let nv = n.dot(v).max(0.0);
    let diffuse = 0.22 + 0.42 * nv + 0.38 * n.dot(KEY).max(0.0) + 0.12 * n.dot(FILL).max(0.0);
    let h = (KEY + v).normalize();
    let spec = 0.18 * n.dot(h).max(0.0).powi(32);
    let rim = 0.12 * (1.0 - nv).powi(4);
    (diffuse, spec + rim)
}

/// Vertex-stage output.
#[derive(Clone, Copy, Debug, Default)]
struct VertOut {
    /// View-space position.
    v: Vec3,
    sx: f32,
    sy: f32,
    key: f32,
    iw: f32,
    /// Studio lighting of the smooth vertex normal.
    diff: f32,
    add: f32,
}

/// A projected triangle corner.
#[derive(Clone, Copy, Debug)]
struct ProjVert {
    x: f32,
    y: f32,
    z: f32,
    iw: f32,
    c: Vec3,
}

/// Camera-space transform and projection to pixels.
struct Projection {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    back: Vec3,
    ortho: bool,
    /// Pixels per unit of x/(-z) (perspective) or per world unit (ortho).
    sx: f32,
    sy: f32,
    cx: f32,
    cy: f32,
    near: f32,
}

impl Projection {
    fn new(v: &ViewCamera, w: u32, h: u32) -> Projection {
        let t = v.tan_half();
        let half_h = h as f32 * 0.5;
        let scale = if v.ortho { half_h / (v.distance * t) } else { half_h / t };
        Projection { eye: v.eye(), right: v.right(), up: v.up(), back: v.back(), ortho: v.ortho, sx: scale, sy: scale, cx: w as f32 * 0.5, cy: half_h, near: v.near() }
    }
    fn to_view(&self, p: Vec3) -> Vec3 {
        let d = p - self.eye;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.back))
    }
    fn rotate(&self, n: Vec3) -> Vec3 {
        Vec3::new(n.dot(self.right), n.dot(self.up), n.dot(self.back))
    }
    /// View-space z of the near plane (negative); everything is in front in ortho.
    fn near_z(&self) -> f32 {
        if self.ortho { f32::INFINITY } else { -self.near }
    }
    /// Pixel x, y and the depth key (larger is closer; affine in screen space).
    fn project(&self, v: Vec3) -> Option<(f32, f32, f32)> {
        if self.ortho {
            return Some((self.cx + v.x * self.sx, self.cy - v.y * self.sy, v.z));
        }
        if v.z > -self.near {
            return None;
        }
        let iw = -1.0 / v.z;
        Some((self.cx + v.x * iw * self.sx, self.cy - v.y * iw * self.sy, iw))
    }
    /// 1/w for perspective-correct interpolation (1 in ortho).
    fn inv_w(&self, v: Vec3) -> f32 {
        if self.ortho { 1.0 } else { -1.0 / v.z }
    }
    fn line(&self, l: &Line) -> Option<ScreenLine> {
        let (mut a, mut b) = (self.to_view(l.a), self.to_view(l.b));
        let nz = self.near_z();
        if a.z > nz && b.z > nz {
            return None;
        }
        if a.z > nz {
            a = b.lerp(a, (nz - b.z) / (a.z - b.z));
        } else if b.z > nz {
            b = a.lerp(b, (nz - a.z) / (b.z - a.z));
        }
        let (pa, pb) = (self.project(a)?, self.project(b)?);
        Some(ScreenLine { a: pa, b: pb, color: l.color, alpha: l.alpha })
    }
}

/// A triangle ready for rasterization.
#[derive(Clone, Copy, Debug)]
struct RTri {
    x: [f32; 3],
    y: [f32; 3],
    z: [f32; 3],
    iw: [f32; 3],
    c: [Vec3; 3],
    obj: u32,
}

#[derive(Debug, Default)]
struct Setup {
    tris: Vec<RTri>,
    bins: Vec<Vec<u32>>,
}

/// Per-frame working memory, reused between draws.
#[derive(Debug, Default)]
struct Scratch {
    verts: Vec<VertOut>,
    setups: Vec<Setup>,
    depth: Vec<f32>,
    ids: Vec<u32>,
    lines: Vec<Line>,
    screen_lines: Vec<ScreenLine>,
}

impl Setup {
    /// Empty it for a frame with `bands` bands, keeping capacity.
    fn reset(&mut self, bands: usize) {
        self.tris.clear();
        self.bins.resize_with(bands, Vec::new);
        for b in &mut self.bins {
            b.clear();
        }
    }
    fn push(&mut self, proj: &Projection, v: [ProjVert; 3], obj: u32, band_h: usize) {
        let (a, mut b, mut c) = (v[0], v[1], v[2]);
        let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if !(area.abs() > 1e-12) {
            return;
        }
        if area < 0.0 {
            core::mem::swap(&mut b, &mut c);
        }
        let (w, h) = (proj.cx * 2.0, proj.cy * 2.0);
        let (xmin, xmax) = (a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x));
        let (ymin, ymax) = (a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y));
        if xmax < 0.0 || ymax < 0.0 || xmin > w || ymin > h {
            return;
        }
        // Rows and columns whose pixel centres can be covered.
        let r0 = (ymin - 0.5).ceil().max(0.0);
        let r1 = (ymax - 0.5).floor().min(h - 1.0);
        if r1 < r0 || (xmax - 0.5).floor() < (xmin - 0.5).ceil() {
            return;
        }
        let idx = self.tris.len() as u32;
        self.tris.push(RTri { x: [a.x, b.x, c.x], y: [a.y, b.y, c.y], z: [a.z, b.z, c.z], iw: [a.iw, b.iw, c.iw], c: [a.c, b.c, c.c], obj });
        for band in r0 as usize / band_h..=r1 as usize / band_h {
            self.bins[band].push(idx);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ScreenLine {
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    color: [f32; 3],
    alpha: f32,
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The rows `y0..y1` of the frame, owned by one worker.
struct Band<'a> {
    w: usize,
    h: usize,
    y0: usize,
    y1: usize,
    rgba: &'a mut [u8],
    depth: &'a mut [f32],
    ids: &'a mut [u32],
}

impl Band<'_> {
    fn clear(&mut self) {
        for y in self.y0..self.y1 {
            let t = y as f32 / (self.h.max(2) - 1) as f32;
            let c = [0, 1, 2].map(|i| to_byte(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t));
            let row = &mut self.rgba[(y - self.y0) * self.w * 4..(y - self.y0 + 1) * self.w * 4];
            for px in row.chunks_exact_mut(4) {
                px.copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        self.depth.fill(f32::NEG_INFINITY);
        self.ids.fill(0);
    }

    fn triangle(&mut self, t: &RTri) {
        let (x, y) = (&t.x, &t.y);
        let area = (x[1] - x[0]) * (y[2] - y[0]) - (x[2] - x[0]) * (y[1] - y[0]);
        let inv_area = 1.0 / area;
        // Edge k is opposite vertex k: e_k(p) = A_k px + B_k py + C_k.
        let edge = |a: usize, b: usize| {
            let (ea, eb) = (-(y[b] - y[a]), x[b] - x[a]);
            (ea, eb, -(ea * x[a] + eb * y[a]))
        };
        let e = [edge(1, 2), edge(2, 0), edge(0, 1)];
        let ymin = y[0].min(y[1]).min(y[2]);
        let ymax = y[0].max(y[1]).max(y[2]);
        let xmin = x[0].min(x[1]).min(x[2]);
        let xmax = x[0].max(x[1]).max(x[2]);
        let r0 = ((ymin - 0.5).ceil().max(self.y0 as f32)) as usize;
        let r1 = (ymax - 0.5).floor().min(self.y1 as f32 - 1.0);
        if r1 < r0 as f32 {
            return;
        }
        let c0 = (xmin - 0.5).ceil().max(0.0);
        let c1 = (xmax - 0.5).floor().min(self.w as f32 - 1.0);
        if c1 < c0 {
            return;
        }
        // Affine depth plane and perspective-correct weights.
        let (z, iw) = (&t.z, &t.iw);
        let c = [t.c[0] * iw[0], t.c[1] * iw[1], t.c[2] * iw[2]];
        for row in r0..=r1 as usize {
            let py = row as f32 + 0.5;
            // Narrow the span to where all edge functions can be >= 0.
            let (mut lo, mut hi) = (c0, c1);
            for &(a, b, cc) in &e {
                let at0 = a * (lo + 0.5) + b * py + cc;
                if a > 0.0 {
                    lo = lo.max(lo + (-at0 / a).ceil() - 1.0);
                } else if a < 0.0 {
                    hi = hi.min(lo + (at0 / -a).floor() + 1.0);
                } else if at0 < 0.0 {
                    hi = lo - 1.0;
                }
            }
            if hi < lo {
                continue;
            }
            let base = (row - self.y0) * self.w;
            for col in lo as usize..=hi as usize {
                let px = col as f32 + 0.5;
                let w0 = e[0].0 * px + e[0].1 * py + e[0].2;
                let w1 = e[1].0 * px + e[1].1 * py + e[1].2;
                let w2 = e[2].0 * px + e[2].1 * py + e[2].2;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let (l0, l1, l2) = (w0 * inv_area, w1 * inv_area, w2 * inv_area);
                let d = l0 * z[0] + l1 * z[1] + l2 * z[2];
                let i = base + col;
                if d <= self.depth[i] {
                    continue;
                }
                self.depth[i] = d;
                self.ids[i] = t.obj;
                let s = 1.0 / (l0 * iw[0] + l1 * iw[1] + l2 * iw[2]);
                let col_rgb = (c[0] * l0 + c[1] * l1 + c[2] * l2) * s;
                let o = i * 4;
                self.rgba[o] = to_byte(col_rgb.x);
                self.rgba[o + 1] = to_byte(col_rgb.y);
                self.rgba[o + 2] = to_byte(col_rgb.z);
            }
        }
    }

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
            let (px, py) = ((xa + dx * t).floor(), (ya + dy * t).floor());
            if px < 0.0 || py < self.y0 as f32 || px >= self.w as f32 || py >= self.y1 as f32 {
                continue;
            }
            let i = (py as usize - self.y0) * self.w + px as usize;
            let d = za + (zb - za) * t;
            let zbuf = self.depth[i];
            if d < zbuf - zbuf.abs() * 2e-3 - 1e-7 {
                continue;
            }
            let o = i * 4;
            for c in 0..3 {
                let cur = self.rgba[o + c] as f32 / 255.0;
                self.rgba[o + c] = to_byte(cur + (l.color[c] - cur) * a);
            }
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
