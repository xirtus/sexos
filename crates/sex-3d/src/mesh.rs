// SPDX-License-Identifier: GPL-3.0-only
//! Polygon meshes.
//!
//! [`Mesh`] is the stored form: vertex positions, n-gon faces with optional
//! per-corner UVs, and loose (wire) edges. It is what files, modifiers and the
//! renderer exchange. [`HalfEdgeMesh`] is the connectivity view built from it on
//! demand for topology work: neighbours, boundaries, edge loops, manifold checks.
use crate::math::{Aabb, Mat4, Vec2, Vec3};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Face {
    /// Vertex indices, counter-clockwise seen from the front (right-handed).
    pub verts: Vec<u32>,
    /// Per-corner UVs: empty, or exactly `verts.len()` entries.
    pub uvs: Vec<Vec2>,
    /// Index into the object's material slots.
    pub material: u16,
    /// Smooth shading (interpolated vertex normals) instead of flat.
    pub smooth: bool,
}

impl Face {
    pub fn new(verts: Vec<u32>) -> Face {
        Face { verts, ..Default::default() }
    }
    pub fn len(&self) -> usize {
        self.verts.len()
    }
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty()
    }
    /// Directed edges (a, b) around the face.
    pub fn edges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let n = self.verts.len();
        (0..n).map(move |i| (self.verts[i], self.verts[(i + 1) % n]))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub faces: Vec<Face>,
    /// Edges not used by any face (wireframe-only geometry).
    pub loose_edges: Vec<[u32; 2]>,
    /// Edges marked as UV seams (Blender's `mesh.mark_seam`), as (low, high)
    /// vertex pairs. Flags on pairs that are no longer edges are ignored.
    pub seams: BTreeSet<(u32, u32)>,
    /// Edges marked sharp (`mesh.mark_sharp`), as (low, high) vertex pairs.
    pub sharp_edges: BTreeSet<(u32, u32)>,
    /// Pinned UVs (the UV editor's P), as (face, corner) pairs.
    pub uv_pins: BTreeSet<(u32, u32)>,
    /// Vertex group names. Blender keeps them on the object (`vertex_groups`)
    /// and, since 3.0, on the mesh (`vertex_group_names`); here they travel
    /// with the weights. Group `i` of [`Mesh::weights`] is name `i`.
    pub vertex_groups: Vec<String>,
    /// Per-vertex (group, weight) pairs, Blender's `MDeformVert`. May be
    /// shorter than `positions` (or empty): a missing entry has no weights.
    pub weights: Vec<Vec<(u32, f32)>>,
    /// Shape keys (Blender's `Key.key_blocks`); index 0 is the Basis.
    pub shape_keys: Vec<ShapeKey>,
    pub shape_key_settings: ShapeKeySettings,
}

/// Interpolation between absolute shape keys and between lattice points
/// (Blender's `KEY_LINEAR`, `KEY_CARDINAL`, `KEY_BSPLINE`, `KEY_CATMULL_ROM`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyInterpolation {
    #[default]
    Linear,
    Cardinal,
    CatmullRom,
    BSpline,
}

impl KeyInterpolation {
    pub const NAMES: [(&'static str, KeyInterpolation); 4] =
        [("linear", KeyInterpolation::Linear), ("cardinal", KeyInterpolation::Cardinal), ("catmull_rom", KeyInterpolation::CatmullRom), ("bspline", KeyInterpolation::BSpline)];
    /// Blender's `KEY_*` numbers (0 linear, 1 cardinal, 2 B-spline, 3 Catmull-Rom).
    pub fn from_blender(v: i64) -> KeyInterpolation {
        match v {
            1 => KeyInterpolation::Cardinal,
            2 => KeyInterpolation::BSpline,
            3 => KeyInterpolation::CatmullRom,
            _ => KeyInterpolation::Linear,
        }
    }
    pub fn name(self) -> &'static str {
        KeyInterpolation::NAMES.iter().find(|n| n.1 == self).map_or("linear", |n| n.0)
    }
    pub fn from_name(s: &str) -> Option<KeyInterpolation> {
        let s = s.to_ascii_lowercase().replace(['-', ' '], "_");
        KeyInterpolation::NAMES.iter().find(|n| n.0 == s || n.0.replace('_', "") == s).map(|n| n.1)
    }
    /// Weights of the four points around a segment at parameter `t` in 0..1
    /// (Blender's `key_curve_position_weights`).
    pub fn weights(self, t: f32) -> [f32; 4] {
        let (t2, t3) = (t * t, t * t * t);
        let cardinal = |fc: f32| [-fc * t3 + 2.0 * fc * t2 - fc * t, (2.0 - fc) * t3 + (fc - 3.0) * t2 + 1.0, (fc - 2.0) * t3 + (3.0 - 2.0 * fc) * t2 + fc * t, fc * t3 - fc * t2];
        match self {
            KeyInterpolation::Linear => [0.0, 1.0 - t, t, 0.0],
            KeyInterpolation::Cardinal => cardinal(0.71),
            KeyInterpolation::CatmullRom => cardinal(0.5),
            KeyInterpolation::BSpline => [
                -0.166_666_66 * t3 + 0.5 * t2 - 0.5 * t + 0.166_666_66,
                0.5 * t3 - t2 + 0.666_666_66,
                -0.5 * t3 + 0.5 * t2 + 0.5 * t + 0.166_666_66,
                0.166_666_66 * t3,
            ],
        }
    }
}

/// One shape key (Blender's `KeyBlock`).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeKey {
    pub name: String,
    /// The key's vertex positions. Shorter than the mesh: missing vertices
    /// keep the mesh position.
    pub positions: Vec<Vec3>,
    /// Influence (relative keys).
    pub value: f32,
    pub slider_min: f32,
    pub slider_max: f32,
    /// The key this one is relative to (index into `Mesh::shape_keys`).
    pub relative_key: usize,
    /// Vertex group scaling the influence per vertex.
    pub vertex_group: Option<String>,
    pub mute: bool,
    /// Interpolation to the next key (absolute keys).
    pub interpolation: KeyInterpolation,
    /// Position on the evaluation-time axis (absolute keys; Blender's
    /// `KeyBlock.frame`, which is `pos * 100`).
    pub frame: f32,
}

impl ShapeKey {
    pub fn new(name: &str, positions: Vec<Vec3>) -> ShapeKey {
        ShapeKey {
            name: name.into(),
            positions,
            value: 0.0,
            slider_min: 0.0,
            slider_max: 1.0,
            relative_key: 0,
            vertex_group: None,
            mute: false,
            interpolation: KeyInterpolation::Linear,
            frame: 0.0,
        }
    }
}

/// Settings of a mesh's shape key block (Blender's `Key`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeKeySettings {
    /// Relative keys (mixed by value) rather than absolute keys
    /// (interpolated along `eval_time`).
    pub use_relative: bool,
    /// Evaluation time for absolute keys, in key frames.
    pub eval_time: f32,
}

impl Default for ShapeKeySettings {
    fn default() -> Self {
        ShapeKeySettings { use_relative: true, eval_time: 0.0 }
    }
}

/// An undirected edge as its (low, high) vertex pair.
pub fn edge_key(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Point a set of edge flags through a vertex `remap`, dropping edges that
/// collapse.
pub fn remap_edge_set(set: &BTreeSet<(u32, u32)>, remap: &[u32]) -> BTreeSet<(u32, u32)> {
    set.iter()
        .filter(|e| (e.0 as usize) < remap.len() && (e.1 as usize) < remap.len())
        .map(|e| edge_key(remap[e.0 as usize], remap[e.1 as usize]))
        .filter(|e| e.0 != e.1 && e.1 != NONE)
        .collect()
}

/// One triangle of a triangulated mesh, with where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tri {
    pub verts: [u32; 3],
    pub face: u32,
    /// Corner indices inside the source face (for UVs).
    pub corners: [u16; 3],
}

impl Mesh {
    pub fn new() -> Mesh {
        Mesh::default()
    }
    pub fn from_faces(positions: Vec<Vec3>, faces: Vec<Vec<u32>>) -> Mesh {
        Mesh { positions, faces: faces.into_iter().map(Face::new).collect(), ..Default::default() }
    }
    pub fn is_seam(&self, a: u32, b: u32) -> bool {
        !self.seams.is_empty() && self.seams.contains(&edge_key(a, b))
    }
    pub fn is_sharp(&self, a: u32, b: u32) -> bool {
        !self.sharp_edges.is_empty() && self.sharp_edges.contains(&edge_key(a, b))
    }
    /// Mark or clear edge (a, b) as a UV seam.
    pub fn set_seam(&mut self, a: u32, b: u32, on: bool) {
        if on {
            self.seams.insert(edge_key(a, b));
        } else {
            self.seams.remove(&edge_key(a, b));
        }
    }
    pub fn set_sharp(&mut self, a: u32, b: u32, on: bool) {
        if on {
            self.sharp_edges.insert(edge_key(a, b));
        } else {
            self.sharp_edges.remove(&edge_key(a, b));
        }
    }
    /// Drop seam/sharp flags on vertex pairs that are no longer edges and pins
    /// on corners that no longer exist (after topology changes that did not
    /// go through [`Mesh::remap_vertices`]).
    pub fn prune_flags(&mut self) {
        if !self.seams.is_empty() || !self.sharp_edges.is_empty() {
            let edges: std::collections::HashSet<(u32, u32)> = self.edges().into_iter().map(|e| (e[0], e[1])).collect();
            self.seams.retain(|e| edges.contains(e));
            self.sharp_edges.retain(|e| edges.contains(e));
        }
        let faces = &self.faces;
        self.uv_pins.retain(|&(f, c)| faces.get(f as usize).is_some_and(|f| (c as usize) < f.verts.len()));
    }
    pub fn add_vertex(&mut self, p: Vec3) -> u32 {
        self.positions.push(p);
        (self.positions.len() - 1) as u32
    }
    pub fn add_face(&mut self, verts: &[u32]) -> u32 {
        self.faces.push(Face::new(verts.to_vec()));
        (self.faces.len() - 1) as u32
    }
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
    pub fn has_uvs(&self) -> bool {
        !self.faces.is_empty() && self.faces.iter().all(|f| f.uvs.len() == f.verts.len())
    }

    /// Unique undirected edges (face edges then loose edges), each as (low, high).
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut seen = HashMap::new();
        let mut out = Vec::new();
        let mut push = |a: u32, b: u32| {
            let k = if a < b { (a, b) } else { (b, a) };
            if a != b && seen.insert(k, ()).is_none() {
                out.push([k.0, k.1]);
            }
        };
        for f in &self.faces {
            for (a, b) in f.edges() {
                push(a, b);
            }
        }
        for e in &self.loose_edges {
            push(e[0], e[1]);
        }
        out
    }

    /// V - E + F.
    pub fn euler_characteristic(&self) -> i64 {
        self.positions.len() as i64 - self.edges().len() as i64 + self.faces.len() as i64
    }

    /// Newell normal of a face (unit; zero for degenerate faces).
    pub fn face_normal(&self, f: usize) -> Vec3 {
        let face = &self.faces[f];
        let mut n = Vec3::ZERO;
        for (a, b) in face.edges() {
            let (p, q) = (self.positions[a as usize], self.positions[b as usize]);
            n.x += (p.y - q.y) * (p.z + q.z);
            n.y += (p.z - q.z) * (p.x + q.x);
            n.z += (p.x - q.x) * (p.y + q.y);
        }
        n.normalize()
    }
    pub fn face_center(&self, f: usize) -> Vec3 {
        let face = &self.faces[f];
        let mut c = Vec3::ZERO;
        for &v in &face.verts {
            c += self.positions[v as usize];
        }
        c / face.verts.len().max(1) as f32
    }
    pub fn face_area(&self, f: usize) -> f32 {
        let face = &self.faces[f];
        let mut n = Vec3::ZERO;
        let p0 = self.positions[face.verts[0] as usize];
        for i in 1..face.verts.len().saturating_sub(1) {
            let a = self.positions[face.verts[i] as usize] - p0;
            let b = self.positions[face.verts[i + 1] as usize] - p0;
            n += a.cross(b);
        }
        n.length() * 0.5
    }
    pub fn surface_area(&self) -> f32 {
        (0..self.faces.len()).map(|f| self.face_area(f)).sum()
    }
    /// Signed volume of a closed mesh (positive when faces point outwards).
    pub fn volume(&self) -> f32 {
        let mut v = 0.0;
        for t in self.triangulate() {
            let [a, b, c] = t.verts.map(|i| self.positions[i as usize]);
            v += a.dot(b.cross(c));
        }
        v / 6.0
    }

    /// Area-weighted vertex normals.
    pub fn vertex_normals(&self) -> Vec<Vec3> {
        let mut n = vec![Vec3::ZERO; self.positions.len()];
        for t in self.triangulate() {
            let [a, b, c] = t.verts.map(|i| self.positions[i as usize]);
            let fn_ = (b - a).cross(c - a);
            for &v in &t.verts {
                n[v as usize] += fn_;
            }
        }
        n.into_iter().map(Vec3::normalize).collect()
    }

    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for &p in &self.positions {
            b.grow(p);
        }
        b
    }

    pub fn transform(&mut self, m: &Mat4) {
        for p in &mut self.positions {
            *p = m.transform_point(*p);
        }
        for k in &mut self.shape_keys {
            for p in &mut k.positions {
                *p = m.transform_point(*p);
            }
        }
        if m.determinant() < 0.0 {
            self.flip_normals();
        }
    }
    pub fn translate(&mut self, d: Vec3) {
        for p in &mut self.positions {
            *p += d;
        }
        for k in &mut self.shape_keys {
            for p in &mut k.positions {
                *p += d;
            }
        }
    }
    pub fn flip_normals(&mut self) {
        for f in &mut self.faces {
            f.verts.reverse();
            f.uvs.reverse();
        }
    }

    /// Append another mesh; returns the index offset its vertices got.
    pub fn append(&mut self, o: &Mesh) -> u32 {
        let off = self.positions.len() as u32;
        self.append_vertex_data(o);
        self.positions.extend_from_slice(&o.positions);
        for f in &o.faces {
            let mut nf = f.clone();
            for v in &mut nf.verts {
                *v += off;
            }
            self.faces.push(nf);
        }
        let foff = self.faces.len() as u32 - o.faces.len() as u32;
        for e in &o.loose_edges {
            self.loose_edges.push([e[0] + off, e[1] + off]);
        }
        self.seams.extend(o.seams.iter().map(|e| (e.0 + off, e.1 + off)));
        self.sharp_edges.extend(o.sharp_edges.iter().map(|e| (e.0 + off, e.1 + off)));
        self.uv_pins.extend(o.uv_pins.iter().map(|p| (p.0 + foff, p.1)));
        off
    }

    /// Triangulate every face. Convex and mildly concave n-gons use ear
    /// clipping in the face's own plane; triangles keep their source face.
    pub fn triangulate(&self) -> Vec<Tri> {
        let mut out = Vec::with_capacity(self.faces.len() * 2);
        for (fi, f) in self.faces.iter().enumerate() {
            let n = f.verts.len();
            if n < 3 {
                continue;
            }
            if n == 3 {
                out.push(Tri { verts: [f.verts[0], f.verts[1], f.verts[2]], face: fi as u32, corners: [0, 1, 2] });
                continue;
            }
            if n == 4 {
                // Split along the shorter diagonal, like Blender's "beauty" default.
                let p: Vec<Vec3> = f.verts.iter().map(|&v| self.positions[v as usize]).collect();
                let (c0, c1, c2, c3) = if p[0].distance(p[2]) <= p[1].distance(p[3]) { (0, 1, 2, 3) } else { (1, 2, 3, 0) };
                out.push(Tri { verts: [f.verts[c0], f.verts[c1], f.verts[c2]], face: fi as u32, corners: [c0 as u16, c1 as u16, c2 as u16] });
                out.push(Tri { verts: [f.verts[c0], f.verts[c2], f.verts[c3]], face: fi as u32, corners: [c0 as u16, c2 as u16, c3 as u16] });
                continue;
            }
            for c in ear_clip(&self.positions, &f.verts, self.face_normal(fi)) {
                out.push(Tri { verts: c.map(|i| f.verts[i as usize]), face: fi as u32, corners: c });
            }
        }
        out
    }

    /// Merge vertices closer than `dist` (Blender's "Merge by Distance").
    /// Faces that collapse below three distinct vertices are removed.
    /// Returns the number of vertices removed.
    pub fn weld(&mut self, dist: f32) -> usize {
        let cell = dist.max(1e-12);
        let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
        let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        let mut remap = vec![0u32; self.positions.len()];
        let mut kept: Vec<Vec3> = Vec::new();
        for (i, &p) in self.positions.iter().enumerate() {
            let k = key(p);
            let mut found = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                            for &j in list {
                                if kept[j as usize].distance(p) <= dist {
                                    found = Some(j);
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
            remap[i] = match found {
                Some(j) => j,
                None => {
                    kept.push(p);
                    let j = (kept.len() - 1) as u32;
                    grid.entry(k).or_default().push(j);
                    j
                }
            };
        }
        let removed = self.positions.len() - kept.len();
        self.positions = kept;
        self.remap_vertices(&remap);
        removed
    }

    /// Point every face/edge index through `remap`, dropping faces that
    /// collapse and consecutive duplicate corners. [`NONE`] marks a removed
    /// vertex: faces and edges still using one are dropped.
    pub fn remap_vertices(&mut self, remap: &[u32]) {
        self.remap_vertex_data(remap);
        let faces = std::mem::take(&mut self.faces);
        let pins = std::mem::take(&mut self.uv_pins);
        for (fi, mut f) in faces.into_iter().enumerate() {
            if f.verts.iter().any(|&v| remap[v as usize] == NONE) {
                continue;
            }
            let n_before = f.verts.len();
            let mut verts = Vec::with_capacity(f.verts.len());
            let mut uvs = Vec::new();
            for (i, &v) in f.verts.iter().enumerate() {
                let nv = remap[v as usize];
                if verts.last() != Some(&nv) {
                    verts.push(nv);
                    if !f.uvs.is_empty() {
                        uvs.push(f.uvs[i]);
                    }
                }
            }
            while verts.len() > 1 && verts.first() == verts.last() {
                verts.pop();
                uvs.pop();
            }
            if verts.len() >= 3 {
                // Pins survive when the face keeps all its corners.
                if verts.len() == n_before {
                    let nf = self.faces.len() as u32;
                    self.uv_pins.extend(pins.range((fi as u32, 0)..(fi as u32 + 1, 0)).map(|p| (nf, p.1)));
                }
                f.verts = verts;
                f.uvs = uvs;
                self.faces.push(f);
            }
        }
        for e in &mut self.loose_edges {
            *e = [remap[e[0] as usize], remap[e[1] as usize]];
        }
        self.loose_edges.retain(|e| e[0] != e[1] && e[0] != NONE && e[1] != NONE);
        self.seams = remap_edge_set(&self.seams, remap);
        self.sharp_edges = remap_edge_set(&self.sharp_edges, remap);
    }

    /// Drop vertices no face or edge uses. Returns how many were removed.
    pub fn remove_unused_vertices(&mut self) -> usize {
        let mut used = vec![false; self.positions.len()];
        for f in &self.faces {
            for &v in &f.verts {
                used[v as usize] = true;
            }
        }
        for e in &self.loose_edges {
            used[e[0] as usize] = true;
            used[e[1] as usize] = true;
        }
        let mut remap = vec![NONE; self.positions.len()];
        let mut kept = Vec::new();
        for (i, &u) in used.iter().enumerate() {
            if u {
                remap[i] = kept.len() as u32;
                kept.push(self.positions[i]);
            }
        }
        let removed = self.positions.len() - kept.len();
        self.positions = kept;
        self.remap_vertices(&remap);
        removed
    }

    /// Check indices and attribute lengths; returns a list of problems.
    pub fn validate(&self) -> Result<(), String> {
        let n = self.positions.len() as u32;
        for (i, f) in self.faces.iter().enumerate() {
            if f.verts.len() < 3 {
                return Err(format!("face {i} has {} corners", f.verts.len()));
            }
            if let Some(&v) = f.verts.iter().find(|&&v| v >= n) {
                return Err(format!("face {i} uses vertex {v} of {n}"));
            }
            if !f.uvs.is_empty() && f.uvs.len() != f.verts.len() {
                return Err(format!("face {i} has {} uvs for {} corners", f.uvs.len(), f.verts.len()));
            }
        }
        for (i, e) in self.loose_edges.iter().enumerate() {
            if e[0] >= n || e[1] >= n {
                return Err(format!("edge {i} out of range"));
            }
        }
        if self.seams.iter().chain(&self.sharp_edges).any(|e| e.0 >= n || e.1 >= n) {
            return Err("seam or sharp edge out of range".into());
        }
        if self.positions.iter().any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite())) {
            return Err("non-finite vertex position".into());
        }
        Ok(())
    }

    pub fn half_edges(&self) -> HalfEdgeMesh {
        HalfEdgeMesh::build(self)
    }
}

// ---------------------------------------------------------------------------
// Vertex groups and shape keys

/// Blender's `.L`/`.R` style name flip used by Mirror's vertex group
/// mirroring (`BLI_string_flip_side_name`, the common separators only).
pub fn flip_side_name(name: &str) -> String {
    let pairs = [("L", "R"), ("l", "r"), ("Left", "Right"), ("left", "right"), ("LEFT", "RIGHT")];
    for sep in ['.', '_', '-', ' '] {
        for (a, b) in pairs {
            for (x, y) in [(a, b), (b, a)] {
                let suffix = format!("{sep}{x}");
                if let Some(stem) = name.strip_suffix(&suffix) {
                    return format!("{stem}{sep}{y}");
                }
                let prefix = format!("{x}{sep}");
                if let Some(rest) = name.strip_prefix(&prefix) {
                    return format!("{y}{sep}{rest}");
                }
            }
        }
    }
    name.to_string()
}

impl Mesh {
    pub fn vertex_group_index(&self, name: &str) -> Option<usize> {
        self.vertex_groups.iter().position(|g| g == name)
    }

    /// Add a vertex group (made unique like Blender: "Group", "Group.001").
    pub fn add_vertex_group(&mut self, name: &str) -> usize {
        let n = crate::scene::unique_name(name, |n| self.vertex_groups.iter().any(|g| g == n));
        self.vertex_groups.push(n);
        self.vertex_groups.len() - 1
    }

    /// The group named `name`, created when missing.
    pub fn ensure_vertex_group(&mut self, name: &str) -> usize {
        match self.vertex_group_index(name) {
            Some(i) => i,
            None => {
                self.vertex_groups.push(name.to_string());
                self.vertex_groups.len() - 1
            }
        }
    }

    /// Remove group `g` and its weights; later groups move down one.
    pub fn remove_vertex_group(&mut self, g: usize) {
        if g >= self.vertex_groups.len() {
            return;
        }
        self.vertex_groups.remove(g);
        let g = g as u32;
        for w in &mut self.weights {
            w.retain(|e| e.0 != g);
            for e in w.iter_mut() {
                if e.0 > g {
                    e.0 -= 1;
                }
            }
        }
        for k in &mut self.shape_keys {
            if k.vertex_group.as_ref().is_some_and(|n| self.vertex_groups.iter().all(|x| x != n)) {
                k.vertex_group = None;
            }
        }
    }

    /// Weights of vertex `v` (empty when it has none).
    pub fn vertex_weights(&self, v: usize) -> &[(u32, f32)] {
        self.weights.get(v).map_or(&[], |w| w.as_slice())
    }

    /// Weight of vertex `v` in group `g`, `None` when not in the group.
    pub fn weight_opt(&self, v: usize, g: usize) -> Option<f32> {
        self.vertex_weights(v).iter().find(|e| e.0 as usize == g).map(|e| e.1)
    }

    /// Weight of vertex `v` in group `g` (0 when not in the group).
    pub fn weight(&self, v: usize, g: usize) -> f32 {
        self.weight_opt(v, g).unwrap_or(0.0)
    }

    /// Per-vertex weights of the named group (all zero when it is missing).
    pub fn group_weights(&self, name: &str) -> Vec<f32> {
        match self.vertex_group_index(name) {
            Some(g) => (0..self.positions.len()).map(|v| self.weight(v, g)).collect(),
            None => vec![0.0; self.positions.len()],
        }
    }

    /// Put vertex `v` in group `g` with weight `w` (replacing an old weight).
    pub fn set_weight(&mut self, v: usize, g: usize, w: f32) {
        if v >= self.positions.len() {
            return;
        }
        if self.weights.len() <= v {
            self.weights.resize(self.positions.len(), Vec::new());
        }
        let list = &mut self.weights[v];
        match list.iter_mut().find(|e| e.0 as usize == g) {
            Some(e) => e.1 = w,
            None => list.push((g as u32, w)),
        }
    }

    /// Take vertex `v` out of group `g`.
    pub fn remove_weight(&mut self, v: usize, g: usize) {
        if let Some(list) = self.weights.get_mut(v) {
            list.retain(|e| e.0 as usize != g);
        }
    }

    /// Make the per-vertex arrays (weights, shape key positions) exactly as
    /// long as `positions`: missing weights become empty, missing key
    /// positions take the mesh position. Called before code that pushes
    /// vertices so new ones line up.
    pub fn pad_vertex_data(&mut self) {
        let n = self.positions.len();
        if !self.weights.is_empty() {
            self.weights.resize(n, Vec::new());
        }
        for k in &mut self.shape_keys {
            if k.positions.len() > n {
                k.positions.truncate(n);
            } else if k.positions.len() < n {
                let from = k.positions.len();
                k.positions.extend_from_slice(&self.positions[from..]);
            }
        }
    }

    fn has_vertex_data(&self) -> bool {
        !self.weights.is_empty() || !self.shape_keys.is_empty()
    }

    /// Add a vertex at `p` whose weights and shape key positions blend the
    /// `sources` (vertex, factor) pairs (factors should sum to 1). Shape keys
    /// get the blended key position moved by the same offset `p` has from the
    /// blended mesh position.
    pub fn add_vertex_interp(&mut self, p: Vec3, sources: &[(u32, f32)]) -> u32 {
        if !self.has_vertex_data() {
            return self.add_vertex(p);
        }
        self.pad_vertex_data();
        let n = self.positions.len();
        let src: Vec<(usize, f32)> = sources.iter().filter(|s| (s.0 as usize) < n).map(|s| (s.0 as usize, s.1)).collect();
        let base = src.iter().fold(Vec3::ZERO, |a, s| a + self.positions[s.0] * s.1);
        for k in &mut self.shape_keys {
            let kp = src.iter().fold(Vec3::ZERO, |a, s| a + k.positions[s.0] * s.1);
            k.positions.push(if src.is_empty() { p } else { kp + (p - base) });
        }
        if !self.weights.is_empty() {
            let mut w: Vec<(u32, f32)> = Vec::new();
            for &(v, f) in &src {
                for &(g, x) in &self.weights[v] {
                    match w.iter_mut().find(|e| e.0 == g) {
                        Some(e) => e.1 += x * f,
                        None => w.push((g, x * f)),
                    }
                }
            }
            w.retain(|e| e.1 > 0.0);
            self.weights.push(w);
        }
        self.positions.push(p);
        (self.positions.len() - 1) as u32
    }

    /// Duplicate vertex `v` with its weights and shape key positions.
    pub fn copy_vertex(&mut self, v: u32) -> u32 {
        let p = self.positions[v as usize];
        self.add_vertex_interp(p, &[(v, 1.0)])
    }

    /// Point weights and shape key positions through a vertex remap whose
    /// `positions` were already rebuilt (or kept). A vertex keeps its own
    /// data when it maps onto itself; otherwise the first source wins.
    fn remap_vertex_data(&mut self, remap: &[u32]) {
        if !self.has_vertex_data() {
            return;
        }
        let n = self.positions.len();
        let pick = |remap: &[u32]| -> Vec<Option<usize>> {
            let mut src: Vec<Option<usize>> = vec![None; n];
            for (i, &r) in remap.iter().enumerate() {
                if r as usize == i && i < n {
                    src[i] = Some(i);
                }
            }
            for (i, &r) in remap.iter().enumerate() {
                if r == NONE {
                    continue;
                }
                if let Some(s) = src.get_mut(r as usize) {
                    if s.is_none() {
                        *s = Some(i);
                    }
                }
            }
            src
        };
        let src = pick(remap);
        if !self.weights.is_empty() {
            let old = std::mem::take(&mut self.weights);
            self.weights = src.iter().map(|s| s.and_then(|i| old.get(i).cloned()).unwrap_or_default()).collect();
        }
        for k in &mut self.shape_keys {
            let old = std::mem::take(&mut k.positions);
            k.positions = src.iter().enumerate().map(|(v, s)| s.and_then(|i| old.get(i).copied()).unwrap_or(self.positions[v])).collect();
        }
    }

    /// Weights and shape keys of `o` for the vertices appended after ours.
    /// Groups match by name (new names are added); keys match by name, and
    /// vertices of the side lacking a key use their mesh positions.
    fn append_vertex_data(&mut self, o: &Mesh) {
        if !self.has_vertex_data() && !o.has_vertex_data() {
            return;
        }
        self.pad_vertex_data();
        let off = self.positions.len();
        if !o.weights.is_empty() || !self.weights.is_empty() || !o.vertex_groups.is_empty() {
            self.weights.resize(off, Vec::new());
            let map: Vec<u32> = o.vertex_groups.iter().map(|g| self.ensure_vertex_group(g) as u32).collect();
            for v in 0..o.positions.len() {
                let w = o.vertex_weights(v).iter().filter_map(|&(g, x)| Some((*map.get(g as usize)?, x))).collect();
                self.weights.push(w);
            }
        }
        if o.shape_keys.is_empty() && self.shape_keys.is_empty() {
            return;
        }
        if self.shape_keys.is_empty() {
            // Take the other mesh's key layout, with our vertices at rest.
            self.shape_keys = o.shape_keys.iter().map(|k| ShapeKey { positions: self.positions.clone(), ..k.clone() }).collect();
            self.shape_key_settings = o.shape_key_settings;
        }
        for k in &mut self.shape_keys {
            let ok = o.shape_keys.iter().find(|x| x.name == k.name);
            for v in 0..o.positions.len() {
                k.positions.push(ok.and_then(|x| x.positions.get(v).copied()).unwrap_or(o.positions[v]));
            }
        }
    }

    /// Add a shape key. The first key added is the Basis (the current
    /// positions); with `from_mix` a new key starts from the evaluated shape.
    pub fn add_shape_key(&mut self, name: &str, from_mix: bool) -> usize {
        if self.shape_keys.is_empty() {
            let mut basis = ShapeKey::new(if name.is_empty() { "Basis" } else { name }, self.positions.clone());
            basis.frame = 0.0;
            self.shape_keys.push(basis);
            return 0;
        }
        let pos = if from_mix { self.evaluated_shape() } else { self.positions.clone() };
        let n = crate::scene::unique_name(if name.is_empty() { "Key" } else { name }, |n| self.shape_keys.iter().any(|k| k.name == n));
        let mut k = ShapeKey::new(&n, pos);
        k.frame = self.shape_keys.last().map_or(0.0, |l| l.frame + 10.0);
        self.shape_keys.push(k);
        self.shape_keys.len() - 1
    }

    pub fn shape_key_index(&self, name: &str) -> Option<usize> {
        self.shape_keys.iter().position(|k| k.name == name)
    }

    /// Position of vertex `v` in key `k`, falling back to the mesh.
    fn key_pos(&self, k: &ShapeKey, v: usize) -> Vec3 {
        k.positions.get(v).copied().unwrap_or(self.positions[v])
    }

    /// The mesh's vertex positions with its shape keys applied (Blender's
    /// `BKE_key_evaluate_object`). Relative keys add `value × weight × (key -
    /// relative key)` to the mesh positions (the Basis); absolute keys are
    /// interpolated along `eval_time` by their `frame`s.
    pub fn evaluated_shape(&self) -> Vec<Vec3> {
        let n = self.positions.len();
        if self.shape_keys.len() < 2 {
            return self.positions.clone();
        }
        if self.shape_key_settings.use_relative {
            let mut out = self.positions.clone();
            for (ki, k) in self.shape_keys.iter().enumerate() {
                let value = k.value.clamp(k.slider_min.min(k.slider_max), k.slider_max.max(k.slider_min));
                if ki == 0 || k.mute || value == 0.0 {
                    continue;
                }
                let Some(rel) = self.shape_keys.get(k.relative_key) else { continue };
                if k.relative_key == ki {
                    continue;
                }
                let weights = k.vertex_group.as_deref().and_then(|g| self.vertex_group_index(g));
                for (v, o) in out.iter_mut().enumerate() {
                    let w = match weights {
                        Some(g) => self.weight(v, g) * value,
                        None => value,
                    };
                    if w != 0.0 {
                        *o += (self.key_pos(k, v) - self.key_pos(rel, v)) * w;
                    }
                }
            }
            return out;
        }
        // Absolute: interpolate between keys sorted by frame.
        let mut order: Vec<usize> = (0..self.shape_keys.len()).collect();
        order.sort_by(|&a, &b| self.shape_keys[a].frame.total_cmp(&self.shape_keys[b].frame));
        let t = self.shape_key_settings.eval_time;
        let at = |i: usize| &self.shape_keys[order[i]];
        let last = order.len() - 1;
        if t <= at(0).frame {
            return (0..n).map(|v| self.key_pos(at(0), v)).collect();
        }
        if t >= at(last).frame {
            return (0..n).map(|v| self.key_pos(at(last), v)).collect();
        }
        let i = (0..last).rfind(|&i| at(i).frame <= t).unwrap_or(0);
        let span = at(i + 1).frame - at(i).frame;
        let u = if span > 0.0 { (t - at(i).frame) / span } else { 0.0 };
        let w = at(i).interpolation.weights(u);
        let ks = [at(i.saturating_sub(1)), at(i), at(i + 1), at((i + 2).min(last))];
        (0..n).map(|v| ks.iter().zip(w).fold(Vec3::ZERO, |a, (k, w)| a + self.key_pos(k, v) * w)).collect()
    }
}

/// Ear clipping of one polygon projected on its plane; returns corner triples.
fn ear_clip(pos: &[Vec3], verts: &[u32], normal: Vec3) -> Vec<[u16; 3]> {
    let n = verts.len();
    let u = normal.any_orthonormal();
    let v = normal.cross(u);
    let pts: Vec<Vec2> = verts.iter().map(|&i| Vec2::new(pos[i as usize].dot(u), pos[i as usize].dot(v))).collect();
    let cross = |a: Vec2, b: Vec2, c: Vec2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let mut idx: Vec<u16> = (0..n as u16).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia as usize], pts[ib as usize], pts[ic as usize]);
            if cross(a, b, c) <= 1e-12 {
                continue;
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = pts[j as usize];
                cross(a, b, p) > 0.0 && cross(b, c, p) > 0.0 && cross(c, a, p) > 0.0
            });
            if !inside {
                out.push([ia, ib, ic]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    // Whatever is left (degenerate or numerically awkward) becomes a fan.
    for i in 1..idx.len().saturating_sub(1) {
        out.push([idx[0], idx[i], idx[i + 1]]);
    }
    out
}

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HalfEdge {
    /// Vertex this half-edge starts at.
    pub origin: u32,
    /// Opposite half-edge, or [`NONE`] on a boundary.
    pub twin: u32,
    pub next: u32,
    pub prev: u32,
    pub face: u32,
}

/// Half-edge connectivity of a [`Mesh`]'s faces. Half-edge `h` of face `f`
/// corner `c` is stored at `face_start[f] + c`, so corners map back directly.
#[derive(Clone, Debug, Default)]
pub struct HalfEdgeMesh {
    pub half_edges: Vec<HalfEdge>,
    pub face_start: Vec<u32>,
    /// One outgoing half-edge per vertex ([`NONE`] for isolated vertices);
    /// boundary vertices get a boundary half-edge so walks start at the rim.
    pub vertex_out: Vec<u32>,
    /// Directed edges used by more than one face (non-manifold or flipped).
    pub non_manifold_edges: usize,
}

impl HalfEdgeMesh {
    pub fn build(m: &Mesh) -> HalfEdgeMesh {
        let mut hes = Vec::new();
        let mut face_start = Vec::with_capacity(m.faces.len());
        let mut vertex_out = vec![NONE; m.positions.len()];
        let mut dir: HashMap<(u32, u32), u32> = HashMap::new();
        let mut non_manifold = 0;
        for (fi, f) in m.faces.iter().enumerate() {
            let start = hes.len() as u32;
            face_start.push(start);
            let n = f.verts.len() as u32;
            for c in 0..n {
                let h = start + c;
                hes.push(HalfEdge { origin: f.verts[c as usize], twin: NONE, next: start + (c + 1) % n, prev: start + (c + n - 1) % n, face: fi as u32 });
                let a = f.verts[c as usize];
                let b = f.verts[((c + 1) % n) as usize];
                if dir.insert((a, b), h).is_some() {
                    non_manifold += 1;
                }
                if vertex_out[a as usize] == NONE {
                    vertex_out[a as usize] = h;
                }
            }
        }
        for h in 0..hes.len() {
            let a = hes[h].origin;
            let b = hes[hes[h].next as usize].origin;
            if let Some(&t) = dir.get(&(b, a)) {
                hes[h].twin = t;
            }
        }
        // Prefer boundary half-edges as the vertex's outgoing edge.
        for h in 0..hes.len() {
            if hes[h].twin == NONE {
                vertex_out[hes[h].origin as usize] = h as u32;
            }
        }
        HalfEdgeMesh { half_edges: hes, face_start, vertex_out, non_manifold_edges: non_manifold }
    }

    pub fn dest(&self, h: u32) -> u32 {
        self.half_edges[self.half_edges[h as usize].next as usize].origin
    }

    pub fn boundary_half_edges(&self) -> Vec<u32> {
        (0..self.half_edges.len() as u32).filter(|&h| self.half_edges[h as usize].twin == NONE).collect()
    }

    /// Closed 2-manifold: every half-edge has a twin and no edge is shared by
    /// more than two faces with consistent winding.
    pub fn is_closed_manifold(&self) -> bool {
        self.non_manifold_edges == 0 && self.half_edges.iter().all(|h| h.twin != NONE)
    }

    /// Vertices joined to `v` by an edge, walking around it.
    pub fn vertex_neighbors(&self, v: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let start = self.vertex_out[v as usize];
        if start == NONE {
            return out;
        }
        // Walk clockwise (h -> twin(prev(h))) until we return or hit a boundary.
        let mut h = start;
        loop {
            out.push(self.dest(h));
            let p = self.half_edges[h as usize].prev;
            let t = self.half_edges[p as usize].twin;
            if t == NONE {
                // Reached a boundary: the prev edge's origin is the last neighbour.
                out.push(self.half_edges[p as usize].origin);
                break;
            }
            h = t;
            if h == start || out.len() > self.half_edges.len() {
                break;
            }
        }
        out.dedup();
        if out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out
    }

    /// Faces sharing an edge with face `f`.
    pub fn face_neighbors(&self, f: u32, m: &Mesh) -> Vec<u32> {
        let s = self.face_start[f as usize];
        (0..m.faces[f as usize].verts.len() as u32)
            .filter_map(|c| {
                let t = self.half_edges[(s + c) as usize].twin;
                (t != NONE).then(|| self.half_edges[t as usize].face)
            })
            .collect()
    }

    /// Edge loop through a quad mesh starting at half-edge `h`: crosses each
    /// valence-4 vertex to the opposite edge. Returns the half-edges in order.
    pub fn edge_loop(&self, h: u32, m: &Mesh) -> Vec<u32> {
        let mut out = vec![h];
        let mut seen = std::collections::HashSet::from([h]);
        for dir in [0, 1] {
            let mut cur = if dir == 0 { h } else { self.half_edges[h as usize].twin };
            if cur == NONE {
                continue;
            }
            loop {
                // At dest(cur): need valence 4 with quads around.
                let v = self.dest(cur);
                if self.vertex_neighbors(v).len() != 4 {
                    break;
                }
                let n = self.half_edges[cur as usize].next; // leaves v in this face
                if m.faces[self.half_edges[n as usize].face as usize].verts.len() != 4 {
                    break;
                }
                let t = self.half_edges[n as usize].twin;
                if t == NONE {
                    break;
                }
                let nxt = self.half_edges[t as usize].next; // opposite edge leaving v
                if nxt == h || self.half_edges[nxt as usize].twin == h {
                    return out; // closed loop
                }
                if !seen.insert(nxt) {
                    break;
                }
                if dir == 0 {
                    out.push(nxt);
                } else {
                    out.insert(0, nxt);
                }
                cur = nxt;
                if out.len() > self.half_edges.len() {
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn cube_topology() {
        let c = primitives::cube(2.0);
        assert_eq!((c.vertex_count(), c.edges().len(), c.face_count()), (8, 12, 6));
        assert_eq!(c.euler_characteristic(), 2);
        let he = c.half_edges();
        assert!(he.is_closed_manifold());
        assert_eq!(he.vertex_neighbors(0).len(), 3);
        assert!((c.volume() - 8.0).abs() < 1e-4);
        assert!((c.surface_area() - 24.0).abs() < 1e-4);
        assert_eq!(c.triangulate().len(), 12);
    }

    #[test]
    fn plane_has_boundary() {
        let p = primitives::grid(3, 3, 2.0);
        let he = p.half_edges();
        assert!(!he.is_closed_manifold());
        assert_eq!(he.boundary_half_edges().len(), 12);
    }

    #[test]
    fn weld_merges_split_cube() {
        let mut m = Mesh::new();
        for f in primitives::cube(1.0).faces.iter() {
            // Each face gets its own copy of the vertices.
            let base = m.positions.len() as u32;
            let c = primitives::cube(1.0);
            for &v in &f.verts {
                m.positions.push(c.positions[v as usize]);
            }
            m.add_face(&[base, base + 1, base + 2, base + 3]);
        }
        assert_eq!(m.vertex_count(), 24);
        assert_eq!(m.weld(1e-5), 16);
        assert!(m.half_edges().is_closed_manifold());
    }

    #[test]
    fn concave_ngon_triangulates_inside() {
        // L shape.
        let pts = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)];
        let m = Mesh::from_faces(pts.iter().map(|&(x, y)| Vec3::new(x, y, 0.0)).collect(), vec![(0..6).collect()]);
        let tris = m.triangulate();
        assert_eq!(tris.len(), 4);
        let area: f32 = tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.verts.map(|i| m.positions[i as usize]);
                (b - a).cross(c - a).z * 0.5
            })
            .sum();
        assert!((area - 3.0).abs() < 1e-5, "area {area}");
    }

    #[test]
    fn edge_loop_around_sphere_equator() {
        let m = primitives::uv_sphere(16, 8, 1.0);
        let he = m.half_edges();
        let mid = (0..he.half_edges.len() as u32)
            .find(|&h| {
                let a = m.positions[he.half_edges[h as usize].origin as usize];
                let b = m.positions[he.dest(h) as usize];
                a.z.abs() < 1e-4 && b.z.abs() < 1e-4
            })
            .unwrap();
        assert_eq!(he.edge_loop(mid, &m).len(), 16);
    }
}
