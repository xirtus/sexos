// SPDX-License-Identifier: GPL-3.0-only
//! Line Art: feature edges (contours, creases, borders, loose edges) of the
//! evaluated meshes seen from a camera, cut where hidden behind surfaces
//! and chained into polylines. Occlusion uses a depth buffer rasterized from
//! the camera with the same projection Grease Pencil draws with.
use super::view::GpView;
use crate::math::Vec3;
use crate::scene::{ObjectData, Scene};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum LineArtSource {
    Scene,
    Collection(String),
    Object(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineArtSettings {
    pub source: LineArtSource,
    /// Camera object; empty: the scene camera.
    pub camera: String,
    pub target_layer: String,
    pub material: usize,
    pub radius: f32,
    pub opacity: f32,
    /// Faces meeting at an angle sharper than this (degrees, 180 = flat) make a crease.
    pub crease_threshold: f32,
    pub use_contour: bool,
    pub use_crease: bool,
    pub use_border: bool,
    pub use_loose: bool,
    /// Depth buffer height in pixels.
    pub resolution: u32,
    /// Frame aspect (width / height) of the camera view.
    pub aspect: f32,
}

impl Default for LineArtSettings {
    fn default() -> Self {
        LineArtSettings {
            source: LineArtSource::Scene,
            camera: String::new(),
            target_layer: "Lines".into(),
            material: 0,
            radius: 0.01,
            opacity: 1.0,
            crease_threshold: 140.0,
            use_contour: true,
            use_crease: true,
            use_border: true,
            use_loose: true,
            resolution: 540,
            aspect: 16.0 / 9.0,
        }
    }
}

/// Depth buffer of distances along the view axis (infinite where empty).
pub struct DepthBuffer {
    pub w: u32,
    pub h: u32,
    pub z: Vec<f32>,
}

impl DepthBuffer {
    pub fn new(w: u32, h: u32) -> DepthBuffer {
        DepthBuffer { w, h, z: vec![f32::INFINITY; (w * h) as usize] }
    }
    /// Rasterize a world-space triangle.
    pub fn triangle(&mut self, view: &GpView, a: Vec3, b: Vec3, c: Vec3) {
        let (Some(pa), Some(pb), Some(pc)) = (view.project(a), view.project(b), view.project(c)) else { return };
        let (w, h) = (self.w as i32, self.h as i32);
        let x0 = pa.0.min(pb.0).min(pc.0).floor().max(0.0) as i32;
        let x1 = (pa.0.max(pb.0).max(pc.0).ceil() as i32).min(w - 1);
        let y0 = pa.1.min(pb.1).min(pc.1).floor().max(0.0) as i32;
        let y1 = (pa.1.max(pb.1).max(pc.1).ceil() as i32).min(h - 1);
        let area = (pb.0 - pa.0) * (pc.1 - pa.1) - (pc.0 - pa.0) * (pb.1 - pa.1);
        if area.abs() < 1e-9 || x0 > x1 || y0 > y1 {
            return;
        }
        // Perspective-correct depth: interpolate 1/z.
        let (ia, ib, ic) = (1.0 / pa.2.max(1e-6), 1.0 / pb.2.max(1e-6), 1.0 / pc.2.max(1e-6));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((pb.0 - px) * (pc.1 - py) - (pc.0 - px) * (pb.1 - py)) / area;
                let w1 = ((pc.0 - px) * (pa.1 - py) - (pa.0 - px) * (pc.1 - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = if view.ortho { w0 * pa.2 + w1 * pb.2 + w2 * pc.2 } else { 1.0 / (w0 * ia + w1 * ib + w2 * ic) };
                let i = (y * w + x) as usize;
                if z < self.z[i] {
                    self.z[i] = z;
                }
            }
        }
    }
    /// The farthest depth in the 3x3 block around a pixel (lenient at silhouettes).
    fn around(&self, x: f32, y: f32) -> f32 {
        let (cx, cy) = (x.floor() as i32, y.floor() as i32);
        let mut m = f32::NEG_INFINITY;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = (cx + dx, cy + dy);
                if px < 0 || py < 0 || px >= self.w as i32 || py >= self.h as i32 {
                    return f32::INFINITY;
                }
                m = m.max(self.z[(py * self.w as i32 + px) as usize]);
            }
        }
        m
    }
}

/// Feature edges of the source objects, visible from the camera, as world-space polylines.
pub fn compute(scene: &Scene, owner: usize, s: &LineArtSettings) -> Vec<Vec<Vec3>> {
    let cam_i = if s.camera.is_empty() { scene.active_camera } else { scene.find_object(&s.camera) };
    let Some(ci) = cam_i else { return Vec::new() };
    let ObjectData::Camera(cam) = &scene.objects[ci].data else { return Vec::new() };
    let h = s.resolution.clamp(16, 4096);
    let w = ((h as f32 * s.aspect.max(0.1)).round() as u32).clamp(16, 8192);
    let view = GpView::from_camera(&scene.world_matrix(ci), cam, w, h);
    let in_collection = |c: usize, name: &str| {
        let mut k = Some(c);
        while let Some(i) = k {
            if scene.collections.get(i).is_some_and(|x| x.name == name) {
                return true;
            }
            k = scene.collections.get(i).and_then(|x| x.parent);
        }
        false
    };
    let sources: Vec<usize> = (0..scene.objects.len())
        .filter(|&i| i != owner && matches!(scene.objects[i].data, ObjectData::Mesh(_)) && scene.is_visible(i))
        .filter(|&i| match &s.source {
            LineArtSource::Scene => true,
            LineArtSource::Collection(c) => in_collection(scene.objects[i].collection, c),
            LineArtSource::Object(o) => scene.objects[i].name == *o,
        })
        .collect();
    // Every visible mesh occludes, whatever the source.
    let mut zb = DepthBuffer::new(w, h);
    let mut meshes = Vec::new();
    for i in 0..scene.objects.len() {
        if i == owner || !matches!(scene.objects[i].data, ObjectData::Mesh(_)) || !scene.is_visible(i) {
            continue;
        }
        let Some((m, _)) = scene.world_mesh(i) else { continue };
        for t in m.triangulate() {
            let [a, b, c] = t.verts.map(|v| m.positions[v as usize]);
            zb.triangle(&view, a, b, c);
        }
        if sources.contains(&i) {
            meshes.push(m);
        }
    }
    let cos_crease = (180.0 - s.crease_threshold.clamp(0.0, 180.0)).to_radians().cos();
    let mut segments: Vec<(Vec3, Vec3)> = Vec::new();
    for m in &meshes {
        let normals: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_normal(f)).collect();
        let centers: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_center(f)).collect();
        let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (fi, f) in m.faces.iter().enumerate() {
            for (a, b) in f.edges() {
                edge_faces.entry(crate::mesh::edge_key(a, b)).or_default().push(fi);
            }
        }
        let facing = |f: usize| {
            let to_eye = if view.ortho { view.back } else { view.eye - centers[f] };
            normals[f].dot(to_eye) > 0.0
        };
        let mut feature: Vec<(u32, u32)> = edge_faces
            .iter()
            .filter(|(_, fs)| match fs.as_slice() {
                [_] => s.use_border,
                [a, b] => (s.use_contour && facing(*a) != facing(*b)) || (s.use_crease && normals[*a].dot(normals[*b]) < cos_crease),
                _ => s.use_contour,
            })
            .map(|(k, _)| *k)
            .collect();
        if s.use_loose {
            feature.extend(m.loose_edges.iter().map(|e| crate::mesh::edge_key(e[0], e[1])));
        }
        feature.sort_unstable();
        for (a, b) in feature {
            visible_parts(&view, &zb, m.positions[a as usize], m.positions[b as usize], &mut segments);
        }
    }
    chain(segments)
}

/// Split segment a-b into its visible runs.
fn visible_parts(view: &GpView, zb: &DepthBuffer, a: Vec3, b: Vec3, out: &mut Vec<(Vec3, Vec3)>) {
    let (Some(pa), Some(pb)) = (view.project(a), view.project(b)) else { return };
    let len_px = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
    let n = ((len_px / 1.5).ceil() as usize).clamp(1, 4096);
    let vis = |t: f32| {
        let p = a.lerp(b, t);
        let Some((x, y, d)) = view.project(p) else { return false };
        d <= zb.around(x, y) + (d * 2e-3).max(1e-4)
    };
    let mut run: Option<f32> = None;
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let v = vis(t);
        match (v, run) {
            (true, None) => run = Some(t),
            (false, Some(t0)) => {
                let t1 = (k as f32 - 0.5) / n as f32;
                if t1 > t0 {
                    out.push((a.lerp(b, t0), a.lerp(b, t1)));
                }
                run = None;
            }
            _ => {}
        }
    }
    if let Some(t0) = run {
        if t0 < 1.0 {
            out.push((a.lerp(b, t0), b));
        }
    }
}

/// Chain segments sharing end points into polylines.
pub fn chain(segs: Vec<(Vec3, Vec3)>) -> Vec<Vec<Vec3>> {
    let key = |p: Vec3| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
    let mut at: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (i, (a, b)) in segs.iter().enumerate() {
        at.entry(key(*a)).or_default().push(i);
        at.entry(key(*b)).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut out = Vec::new();
    for i in 0..segs.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut line = vec![segs[i].0, segs[i].1];
        // Grow forwards, then backwards.
        for dir in 0..2 {
            loop {
                let end = if dir == 0 { *line.last().unwrap() } else { line[0] };
                let next = at.get(&key(end)).and_then(|v| v.iter().copied().find(|&j| !used[j]));
                let Some(j) = next else { break };
                used[j] = true;
                let (a, b) = segs[j];
                let other = if key(a) == key(end) { b } else { a };
                if dir == 0 {
                    line.push(other);
                } else {
                    line.insert(0, other);
                }
            }
        }
        out.push(line);
    }
    out
}
