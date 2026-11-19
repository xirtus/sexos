// SPDX-License-Identifier: GPL-3.0-only
//! Dynamic topology (Blender's dyntopo): under the brush, edges longer than
//! the detail size are split and edges shorter than 40 % of it collapse, so
//! the surface gains and loses triangles where it is sculpted. Works on
//! triangles (the mesh is triangulated first, as Blender does when dyntopo
//! is switched on). Collapses keep the mesh manifold: they need the link
//! condition, leave boundaries alone and never flip a triangle.
use super::{flip, Symmetry};
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use std::collections::{HashMap, HashSet};

/// Splits are this long at most, collapses this short at least, relative
/// to the detail size (Blender's 1.0 / 0.4).
pub const MIN_FACTOR: f32 = 0.4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub splits: usize,
    pub collapses: usize,
    pub triangulated: bool,
}

impl Report {
    pub fn changed(&self) -> bool {
        self.splits + self.collapses > 0 || self.triangulated
    }
}

/// Split every face into triangles, keeping face sets, hidden flags,
/// materials, smoothing and UVs.
pub fn triangulate(m: &mut Mesh) -> bool {
    if m.faces.iter().all(|f| f.verts.len() == 3) {
        return false;
    }
    let tris = m.triangulate();
    let mut faces = Vec::with_capacity(tris.len());
    let mut sets = Vec::new();
    let mut hidden = Vec::new();
    for t in &tris {
        let f = &m.faces[t.face as usize];
        let uvs = if f.uvs.len() == f.verts.len() { t.corners.iter().map(|&c| f.uvs[c as usize]).collect() } else { Vec::new() };
        faces.push(Face { verts: t.verts.to_vec(), uvs, material: f.material, smooth: f.smooth });
        if !m.face_sets.is_empty() {
            sets.push(m.face_set(t.face as usize));
        }
        if !m.hidden_faces.is_empty() {
            hidden.push(m.is_face_hidden(t.face as usize));
        }
    }
    // Corner colours follow the triangles.
    let offsets = m.corner_offsets();
    for a in &mut m.color_attributes {
        if a.domain == crate::mesh::AttrDomain::Corner {
            a.data = tris.iter().flat_map(|t| t.corners.map(|c| a.data.get(offsets[t.face as usize] + c as usize).copied().unwrap_or([1.0; 4]))).collect();
        }
    }
    m.faces = faces;
    m.face_sets = sets;
    m.hidden_faces = hidden;
    m.uv_pins.clear();
    true
}

struct Work<'a> {
    m: &'a mut Mesh,
    /// Faces of the vertices near the brush.
    vf: HashMap<u32, Vec<u32>>,
    dead_faces: HashSet<u32>,
    boundary: HashSet<u32>,
}

impl Work<'_> {
    fn faces_of_edge(&self, a: u32, b: u32) -> Vec<u32> {
        let (Some(fa), Some(fb)) = (self.vf.get(&a), self.vf.get(&b)) else { return Vec::new() };
        fa.iter().copied().filter(|f| fb.contains(f) && !self.dead_faces.contains(f)).collect()
    }

    fn neighbors(&self, v: u32) -> HashSet<u32> {
        let mut s = HashSet::new();
        if let Some(fs) = self.vf.get(&v) {
            for &f in fs {
                if self.dead_faces.contains(&f) {
                    continue;
                }
                for &w in &self.m.faces[f as usize].verts {
                    if w != v {
                        s.insert(w);
                    }
                }
            }
        }
        s
    }

    fn split(&mut self, a: u32, b: u32) -> bool {
        let faces = self.faces_of_edge(a, b);
        if faces.is_empty() || faces.len() > 2 {
            return false;
        }
        let pm = (self.m.positions[a as usize] + self.m.positions[b as usize]) * 0.5;
        let mv = self.m.add_vertex_interp(pm, &[(a, 0.5), (b, 0.5)]);
        self.vf.insert(mv, Vec::new());
        if faces.len() == 1 {
            self.boundary.insert(mv);
        }
        for f in faces {
            let face = self.m.faces[f as usize].clone();
            let n = face.verts.len();
            // Corner of `p` such that the next corner is `q` and {p, q} = {a, b}.
            let Some(i) = (0..n).find(|&i| {
                let (p, q) = (face.verts[i], face.verts[(i + 1) % n]);
                (p == a && q == b) || (p == b && q == a)
            }) else {
                continue;
            };
            let (p, q, c) = (face.verts[i], face.verts[(i + 1) % n], face.verts[(i + 2) % n]);
            let uv = |k: usize| face.uvs.get(k).copied();
            let (uvs_f, uvs_g) = match (uv(i), uv((i + 1) % n), uv((i + 2) % n)) {
                (Some(up), Some(uq), Some(uc)) => {
                    let um: Vec2 = up.lerp(uq, 0.5);
                    (vec![up, um, uc], vec![um, uq, uc])
                }
                _ => (Vec::new(), Vec::new()),
            };
            self.m.faces[f as usize] = Face { verts: vec![p, mv, c], uvs: uvs_f, material: face.material, smooth: face.smooth };
            let g = self.m.faces.len() as u32;
            self.m.faces.push(Face { verts: vec![mv, q, c], uvs: uvs_g, material: face.material, smooth: face.smooth });
            if !self.m.face_sets.is_empty() {
                let s = self.m.face_set(f as usize);
                self.m.face_sets.push(s);
            }
            if !self.m.hidden_faces.is_empty() {
                let h = self.m.is_face_hidden(f as usize);
                self.m.hidden_faces.push(h);
            }
            if let Some(l) = self.vf.get_mut(&q) {
                l.retain(|&x| x != f);
                l.push(g);
            }
            if let Some(l) = self.vf.get_mut(&c) {
                l.push(g);
            }
            let lm = self.vf.entry(mv).or_default();
            lm.push(f);
            lm.push(g);
        }
        true
    }

    fn collapse(&mut self, a: u32, b: u32) -> bool {
        if self.boundary.contains(&a) || self.boundary.contains(&b) {
            return false;
        }
        let faces = self.faces_of_edge(a, b);
        if faces.len() != 2 {
            return false;
        }
        let (na, nb) = (self.neighbors(a), self.neighbors(b));
        let common: Vec<u32> = na.intersection(&nb).copied().collect();
        let opposite: HashSet<u32> = faces.iter().flat_map(|&f| self.m.faces[f as usize].verts.clone()).filter(|&v| v != a && v != b).collect();
        if common.len() != 2 || common.iter().any(|c| !opposite.contains(c)) {
            return false;
        }
        // The merged vertex keeps at least three neighbours.
        if na.union(&nb).count() < 3 + 2 + 2 - 2 {
            return false;
        }
        // Opposite vertices keep valence of at least three.
        for &c in &common {
            if self.neighbors(c).len() <= 3 {
                return false;
            }
        }
        let pm = (self.m.positions[a as usize] + self.m.positions[b as usize]) * 0.5;
        // No surviving triangle may flip or collapse.
        let around: Vec<u32> = self.vf.get(&a).into_iter().chain(self.vf.get(&b)).flatten().copied().filter(|f| !faces.contains(f) && !self.dead_faces.contains(f)).collect();
        for &f in &around {
            let vs = &self.m.faces[f as usize].verts;
            let p: Vec<Vec3> = vs.iter().map(|&v| self.m.positions[v as usize]).collect();
            let q: Vec<Vec3> = vs.iter().map(|&v| if v == a || v == b { pm } else { self.m.positions[v as usize] }).collect();
            let n0 = (p[1] - p[0]).cross(p[2] - p[0]);
            let n1 = (q[1] - q[0]).cross(q[2] - q[0]);
            if n1.length_squared() < n0.length_squared() * 1e-4 || n0.dot(n1) <= 0.0 {
                return false;
            }
        }
        // Merge b into a.
        self.m.positions[a as usize] = pm;
        if !self.m.mask.is_empty() {
            let k = (self.m.mask_at(a as usize) + self.m.mask_at(b as usize)) * 0.5;
            self.m.mask[a as usize] = k;
        }
        for attr in &mut self.m.color_attributes {
            if attr.domain == crate::mesh::AttrDomain::Point {
                if let (Some(&ca), Some(&cb)) = (attr.data.get(a as usize), attr.data.get(b as usize)) {
                    attr.data[a as usize] = std::array::from_fn(|i| (ca[i] + cb[i]) * 0.5);
                }
            }
        }
        for &f in &faces {
            self.dead_faces.insert(f);
            for &c in &common {
                if let Some(l) = self.vf.get_mut(&c) {
                    l.retain(|&x| x != f);
                }
            }
        }
        let fb = self.vf.remove(&b).unwrap_or_default();
        for f in fb {
            if self.dead_faces.contains(&f) {
                continue;
            }
            for v in &mut self.m.faces[f as usize].verts {
                if *v == b {
                    *v = a;
                }
            }
            if let Some(l) = self.vf.get_mut(&a) {
                if !l.contains(&f) {
                    l.push(f);
                }
            }
        }
        if let Some(l) = self.vf.get_mut(&a) {
            l.retain(|f| !faces.contains(f));
        }
        true
    }
}

/// Split and collapse edges within `radius` of `center` (and its mirror
/// images) towards edge length `detail`.
pub fn update(m: &mut Mesh, center: Vec3, radius: f32, detail: f32, subdivide: bool, collapse: bool, sym: &Symmetry) -> Report {
    let mut rep = Report { triangulated: triangulate(m), ..Default::default() };
    if detail <= 0.0 || !(subdivide || collapse) {
        return rep;
    }
    let max = detail;
    let min = detail * MIN_FACTOR;
    let centers: Vec<Vec3> = sym.passes().into_iter().map(|f| flip(center, f)).collect();
    let r2 = radius * radius;
    let near = |p: Vec3| centers.iter().any(|c| (p - *c).length_squared() <= r2);
    let reach = radius + max * 2.0;
    let reach2 = reach * reach;
    let near_wide = |p: Vec3| centers.iter().any(|c| (p - *c).length_squared() <= reach2);

    // Vertex faces of everything near the brush, and boundary vertices.
    let mut vf: HashMap<u32, Vec<u32>> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        if f.verts.iter().any(|&v| near_wide(m.positions[v as usize])) {
            for &v in &f.verts {
                vf.entry(v).or_default().push(fi as u32);
            }
        }
    }
    let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
    for fs in vf.values() {
        for &f in fs {
            for (a, b) in m.faces[f as usize].edges() {
                edge_count.entry((a.min(b), a.max(b))).or_insert(0);
            }
        }
    }
    let mut seen_faces = HashSet::new();
    for fs in vf.values() {
        for &f in fs {
            if seen_faces.insert(f) {
                for (a, b) in m.faces[f as usize].edges() {
                    *edge_count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
        }
    }
    let boundary: HashSet<u32> = edge_count.iter().filter(|(_, &n)| n == 1).flat_map(|(e, _)| [e.0, e.1]).collect();
    // A face set border or a hidden border is a boundary too (keeps them crisp).
    let mut w = Work { m, vf, dead_faces: HashSet::new(), boundary };

    let candidates = |w: &Work, long: bool| -> Vec<(f32, u32, u32)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (&v, fs) in &w.vf {
            for &f in fs {
                if w.dead_faces.contains(&f) {
                    continue;
                }
                for (a, b) in w.m.faces[f as usize].edges() {
                    if a != v && b != v {
                        continue;
                    }
                    let k = (a.min(b), a.max(b));
                    if !seen.insert(k) || !w.vf.contains_key(&k.0) || !w.vf.contains_key(&k.1) {
                        continue;
                    }
                    let (pa, pb) = (w.m.positions[a as usize], w.m.positions[b as usize]);
                    if !near((pa + pb) * 0.5) {
                        continue;
                    }
                    let l = pa.distance(pb);
                    if (long && l > max) || (!long && l < min) {
                        out.push((l, k.0, k.1));
                    }
                }
            }
        }
        out
    };

    if subdivide {
        for _ in 0..8 {
            let mut c = candidates(&w, true);
            if c.is_empty() {
                break;
            }
            c.sort_by(|x, y| y.0.total_cmp(&x.0));
            let mut n = 0;
            for (_, a, b) in c {
                let l = w.m.positions[a as usize].distance(w.m.positions[b as usize]);
                if l > max && w.split(a, b) {
                    n += 1;
                }
            }
            rep.splits += n;
            if n == 0 {
                break;
            }
        }
    }
    if collapse {
        for _ in 0..4 {
            let mut c = candidates(&w, false);
            if c.is_empty() {
                break;
            }
            c.sort_by(|x, y| x.0.total_cmp(&y.0));
            let mut n = 0;
            for (_, a, b) in c {
                if !w.vf.contains_key(&a) || !w.vf.contains_key(&b) {
                    continue;
                }
                let l = w.m.positions[a as usize].distance(w.m.positions[b as usize]);
                if l < min && w.collapse(a, b) {
                    n += 1;
                }
            }
            rep.collapses += n;
            if n == 0 {
                break;
            }
        }
    }
    let dead = std::mem::take(&mut w.dead_faces);
    if !dead.is_empty() {
        let m = &mut *w.m;
        let keep: Vec<bool> = (0..m.faces.len() as u32).map(|f| !dead.contains(&f)).collect();
        let mut k = keep.iter();
        m.faces.retain(|_| *k.next().unwrap_or(&true));
        if !m.face_sets.is_empty() {
            let mut k = keep.iter();
            m.face_sets.retain(|_| *k.next().unwrap_or(&true));
        }
        if !m.hidden_faces.is_empty() {
            let mut k = keep.iter();
            m.hidden_faces.retain(|_| *k.next().unwrap_or(&true));
        }
        m.remove_unused_vertices();
    }
    if rep.splits + rep.collapses > 0 {
        w.m.uv_pins.clear();
        w.m.sanitize_attributes();
    }
    rep
}
