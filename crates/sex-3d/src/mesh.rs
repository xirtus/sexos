// SPDX-License-Identifier: GPL-3.0-only
//! Polygon meshes.
//!
//! [`Mesh`] is the stored form: vertex positions, n-gon faces with optional
//! per-corner UVs, and loose (wire) edges. It is what files, modifiers and the
//! renderer exchange. [`HalfEdgeMesh`] is the connectivity view built from it on
//! demand for topology work: neighbours, boundaries, edge loops, manifold checks.
use crate::math::{Aabb, Mat4, Vec2, Vec3};
use std::collections::HashMap;

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
        Mesh { positions, faces: faces.into_iter().map(Face::new).collect(), loose_edges: Vec::new() }
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
        if m.determinant() < 0.0 {
            self.flip_normals();
        }
    }
    pub fn translate(&mut self, d: Vec3) {
        for p in &mut self.positions {
            *p += d;
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
        self.positions.extend_from_slice(&o.positions);
        for f in &o.faces {
            let mut nf = f.clone();
            for v in &mut nf.verts {
                *v += off;
            }
            self.faces.push(nf);
        }
        for e in &o.loose_edges {
            self.loose_edges.push([e[0] + off, e[1] + off]);
        }
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
    /// collapse and consecutive duplicate corners.
    pub fn remap_vertices(&mut self, remap: &[u32]) {
        let faces = std::mem::take(&mut self.faces);
        for mut f in faces {
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
                f.verts = verts;
                f.uvs = uvs;
                self.faces.push(f);
            }
        }
        for e in &mut self.loose_edges {
            *e = [remap[e[0] as usize], remap[e[1] as usize]];
        }
        self.loose_edges.retain(|e| e[0] != e[1]);
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
        let mut remap = vec![0u32; self.positions.len()];
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
        if self.positions.iter().any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite())) {
            return Err("non-finite vertex position".into());
        }
        Ok(())
    }

    pub fn half_edges(&self) -> HalfEdgeMesh {
        HalfEdgeMesh::build(self)
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
