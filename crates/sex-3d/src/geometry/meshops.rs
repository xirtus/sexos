// SPDX-License-Identifier: GPL-3.0-only
//! Mesh operations of the geometry nodes, after Blender's
//! `geometry::extrude_mesh`, `split_edges`, `mesh_delete`, `dual_mesh`,
//! `merge_by_distance` and friends. Each builds a new mesh through
//! [`Builder`], which records where every new vertex, face and face corner
//! came from so generic attributes, UVs, vertex groups and edge flags follow.
use super::attributes::Domain;
use super::sample::KdTree;
use super::topo::MeshTopo;
use super::{transfer_attributes, MeshMaps};
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use std::collections::{HashMap, HashSet};

type Src = Vec<(usize, f32)>;

/// Builds a mesh from an old one, recording sources for attributes.
pub struct Builder<'a> {
    pub old: &'a Mesh,
    pub ot: &'a MeshTopo,
    pub out: Mesh,
    pub maps: MeshMaps,
}

impl<'a> Builder<'a> {
    pub fn new(old: &'a Mesh, ot: &'a MeshTopo) -> Builder<'a> {
        Builder { old, ot, out: Mesh::default(), maps: MeshMaps::default() }
    }

    pub fn vert(&mut self, p: Vec3, src: Src) -> u32 {
        self.out.positions.push(p);
        self.maps.verts.push(src);
        self.out.positions.len() as u32 - 1
    }

    pub fn copy_vert(&mut self, v: usize) -> u32 {
        self.vert(self.old.positions[v], vec![(v, 1.0)])
    }

    /// Add a face; `corners` are the old corners each new corner takes its
    /// attributes and UVs from (empty: none). `src` is the old face its
    /// material, smoothing and face attributes come from.
    pub fn face(&mut self, verts: Vec<u32>, src: Option<usize>, corners: Vec<Src>) -> usize {
        let mut f = Face::new(verts);
        if let Some(s) = src.and_then(|s| self.old.faces.get(s)) {
            f.material = s.material;
            f.smooth = s.smooth;
        }
        if self.old.has_uvs() {
            f.uvs = (0..f.verts.len())
                .map(|k| {
                    let mut acc = Vec2::ZERO;
                    for &(c, w) in corners.get(k).map(|v| v.as_slice()).unwrap_or(&[]) {
                        acc = acc + self.corner_uv(c) * w;
                    }
                    acc
                })
                .collect();
        }
        let mut corners = corners;
        corners.resize(f.verts.len(), Vec::new());
        self.maps.corners.extend(corners);
        self.maps.faces.push(src.map(|s| vec![(s, 1.0)]).unwrap_or_default());
        self.out.faces.push(f);
        self.out.faces.len() - 1
    }

    fn corner_uv(&self, c: usize) -> Vec2 {
        let f = self.ot.corner_face[c] as usize;
        let k = c - self.ot.face_offsets[f];
        self.old.faces[f].uvs.get(k).copied().unwrap_or(Vec2::ZERO)
    }

    /// Copy old face `f` with its vertices mapped through `vmap`.
    pub fn copy_face(&mut self, f: usize, vmap: impl Fn(u32) -> u32) -> usize {
        let verts = self.old.faces[f].verts.iter().map(|&v| vmap(v)).collect();
        let corners = self.ot.corners(f).map(|c| vec![(c, 1.0)]).collect();
        self.face(verts, Some(f), corners)
    }

    pub fn edge(&mut self, a: u32, b: u32) {
        self.out.loose_edges.push([a, b]);
    }

    pub fn finish(mut self) -> Mesh {
        // Keep loose edges only where no face has them.
        let face_edges: HashSet<(u32, u32)> = self.out.faces.iter().flat_map(|f| f.edges().map(|(a, b)| (a.min(b), a.max(b)))).collect();
        let mut seen = HashSet::new();
        self.out.loose_edges.retain(|e| {
            let k = (e[0].min(e[1]), e[0].max(e[1]));
            e[0] != e[1] && !face_edges.contains(&k) && seen.insert(k)
        });
        transfer_attributes(self.old, &mut self.out, &self.maps);
        self.out
    }
}

/// Elements an operation created or marked (Extrude Mesh's Top and Side).
#[derive(Clone, Debug)]
pub enum Marked {
    Points(Vec<bool>),
    Edges(HashSet<(u32, u32)>),
    Faces(Vec<bool>),
}

impl Marked {
    pub fn domain(&self) -> Domain {
        match self {
            Marked::Points(_) => Domain::Point,
            Marked::Edges(_) => Domain::Edge,
            Marked::Faces(_) => Domain::Face,
        }
    }

    /// The marks as booleans on the mesh's domain.
    pub fn on(&self, m: &Mesh, t: &MeshTopo) -> Vec<bool> {
        match self {
            Marked::Points(v) | Marked::Faces(v) => {
                let n = t.size(m, self.domain());
                let mut v = v.clone();
                v.resize(n, false);
                v
            }
            Marked::Edges(s) => t.edges.iter().map(|e| s.contains(&(e[0].min(e[1]), e[0].max(e[1])))).collect(),
        }
    }
}

fn ek(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// The old corner of face `f` at vertex `v`.
fn corner_at(m: &Mesh, t: &MeshTopo, f: usize, v: u32) -> Option<usize> {
    m.faces[f].verts.iter().position(|&x| x == v).map(|k| t.face_offsets[f] + k)
}

fn copy_all_verts(b: &mut Builder) {
    for v in 0..b.old.positions.len() {
        b.copy_vert(v);
    }
}

fn copy_loose(b: &mut Builder) {
    for e in b.old.loose_edges.clone() {
        b.edge(e[0], e[1]);
    }
}

// ---------------------------------------------------------------------------
// Extrude

pub struct Extruded {
    pub mesh: Mesh,
    pub top: Marked,
    pub side: Marked,
}

/// Extrude selected vertices into new vertices joined by edges.
pub fn extrude_vertices(m: &Mesh, sel: &[bool], offsets: &[Vec3]) -> Extruded {
    let t = MeshTopo::new(m);
    let mut b = Builder::new(m, &t);
    copy_all_verts(&mut b);
    for f in 0..m.faces.len() {
        b.copy_face(f, |v| v);
    }
    copy_loose(&mut b);
    let mut top = vec![false; m.positions.len()];
    let mut side = HashSet::new();
    for v in 0..m.positions.len() {
        if sel.get(v).copied().unwrap_or(false) {
            let n = b.vert(m.positions[v] + offsets.get(v).copied().unwrap_or(Vec3::ZERO), vec![(v, 1.0)]);
            b.edge(v as u32, n);
            top.push(true);
            side.insert(ek(v as u32, n));
        }
    }
    Extruded { mesh: b.finish(), top: Marked::Points(top), side: Marked::Edges(side) }
}

/// Extrude selected edges into quads (offsets per vertex).
pub fn extrude_edges(m: &Mesh, sel: &[bool], offsets: &[Vec3]) -> Extruded {
    let t = MeshTopo::new(m);
    let mut b = Builder::new(m, &t);
    copy_all_verts(&mut b);
    for f in 0..m.faces.len() {
        b.copy_face(f, |v| v);
    }
    copy_loose(&mut b);
    let mut newv: HashMap<u32, u32> = HashMap::new();
    let selected: Vec<usize> = (0..t.edges.len()).filter(|&e| sel.get(e).copied().unwrap_or(false)).collect();
    for &e in &selected {
        for v in t.edges[e] {
            if !newv.contains_key(&v) {
                let n = b.vert(m.positions[v as usize] + offsets.get(v as usize).copied().unwrap_or(Vec3::ZERO), vec![(v as usize, 1.0)]);
                newv.insert(v, n);
            }
        }
    }
    let mut top = HashSet::new();
    let mut side = vec![false; m.faces.len()];
    for &e in &selected {
        let [mut a, mut c] = t.edges[e];
        let adj = t.edge_faces[e].first().map(|&f| f as usize);
        if let Some(f) = adj {
            // Run against the neighbour face's direction.
            let fv = &m.faces[f].verts;
            let k = fv.iter().position(|&x| x == a).unwrap_or(0);
            if fv[(k + 1) % fv.len()] == c {
                std::mem::swap(&mut a, &mut c);
            }
        }
        let (na, nc) = (newv[&a], newv[&c]);
        let corners = match adj {
            Some(f) => {
                let ca = corner_at(m, &t, f, a).map(|x| vec![(x, 1.0)]).unwrap_or_default();
                let cc = corner_at(m, &t, f, c).map(|x| vec![(x, 1.0)]).unwrap_or_default();
                vec![ca.clone(), cc.clone(), cc, ca]
            }
            None => Vec::new(),
        };
        b.face(vec![a, c, nc, na], adj, corners);
        side.push(true);
        top.insert(ek(na, nc));
    }
    Extruded { mesh: b.finish(), top: Marked::Edges(top), side: Marked::Faces(side) }
}

/// Extrude selected faces as regions (`individual` false) or one by one.
pub fn extrude_faces(m: &Mesh, sel: &[bool], offsets: &[Vec3], individual: bool) -> Extruded {
    let t = MeshTopo::new(m);
    let mut b = Builder::new(m, &t);
    copy_all_verts(&mut b);
    let is_sel = |f: usize| sel.get(f).copied().unwrap_or(false);
    let nf = m.faces.len();
    let mut top = vec![false; nf];
    let mut sides: Vec<(Vec<u32>, usize, Vec<Src>)> = Vec::new();
    if individual {
        let mut faces: Vec<(Vec<u32>, usize)> = Vec::with_capacity(nf);
        for (f, face) in m.faces.iter().enumerate() {
            if !is_sel(f) {
                faces.push((face.verts.clone(), f));
                continue;
            }
            let off = offsets.get(f).copied().unwrap_or(Vec3::ZERO);
            let nv: Vec<u32> = face.verts.iter().map(|&v| b.vert(m.positions[v as usize] + off, vec![(v as usize, 1.0)])).collect();
            let n = face.verts.len();
            for k in 0..n {
                let (a, c) = (face.verts[k], face.verts[(k + 1) % n]);
                let (ca, cc) = (t.face_offsets[f] + k, t.face_offsets[f] + (k + 1) % n);
                sides.push((vec![a, c, nv[(k + 1) % n], nv[k]], f, vec![vec![(ca, 1.0)], vec![(cc, 1.0)], vec![(cc, 1.0)], vec![(ca, 1.0)]]));
            }
            faces.push((nv, f));
            top[f] = true;
        }
        for (verts, f) in faces {
            let corners = t.corners(f).map(|c| vec![(c, 1.0)]).collect();
            b.face(verts, Some(f), corners);
        }
    } else {
        // Region: boundary edges have exactly one selected face.
        let mut boundary: HashMap<(u32, u32), usize> = HashMap::new();
        for (e, fs) in t.edge_faces.iter().enumerate() {
            let s: Vec<u32> = fs.iter().copied().filter(|&f| is_sel(f as usize)).collect();
            if s.len() == 1 {
                let ed = t.edges[e];
                boundary.insert(ek(ed[0], ed[1]), s[0] as usize);
            }
        }
        // Offsets per vertex: the mean of the selected faces' offsets.
        let mut acc = vec![(Vec3::ZERO, 0u32); m.positions.len()];
        for f in (0..nf).filter(|&f| is_sel(f)) {
            for &v in &m.faces[f].verts {
                acc[v as usize].0 += offsets.get(f).copied().unwrap_or(Vec3::ZERO);
                acc[v as usize].1 += 1;
            }
        }
        let loose_verts: HashSet<u32> = m.loose_edges.iter().flatten().copied().collect();
        let mut remap: Vec<u32> = (0..m.positions.len() as u32).collect();
        for v in 0..m.positions.len() {
            if acc[v].1 == 0 {
                continue;
            }
            let off = acc[v].0 / acc[v].1 as f32;
            let shared = t.vert_faces[v].iter().any(|&f| !is_sel(f as usize)) || loose_verts.contains(&(v as u32)) || t.vert_edges[v].iter().any(|&e| boundary.contains_key(&ek(t.edges[e as usize][0], t.edges[e as usize][1])));
            if shared {
                remap[v] = b.vert(m.positions[v] + off, vec![(v, 1.0)]);
            } else {
                b.out.positions[v] += off;
            }
        }
        for f in 0..nf {
            if is_sel(f) {
                top[f] = true;
                b.copy_face(f, |v| remap[v as usize]);
            } else {
                b.copy_face(f, |v| v);
            }
        }
        let mut bl: Vec<((u32, u32), usize)> = boundary.into_iter().collect();
        bl.sort();
        for ((x, y), f) in bl {
            let fv = &m.faces[f].verts;
            let k = fv.iter().position(|&v| v == x).unwrap_or(0);
            let (a, c) = if fv[(k + 1) % fv.len()] == y { (x, y) } else { (y, x) };
            let ca = corner_at(m, &t, f, a).map(|x| vec![(x, 1.0)]).unwrap_or_default();
            let cc = corner_at(m, &t, f, c).map(|x| vec![(x, 1.0)]).unwrap_or_default();
            sides.push((vec![a, c, remap[c as usize], remap[a as usize]], f, vec![ca.clone(), cc.clone(), cc, ca]));
        }
    }
    copy_loose(&mut b);
    let mut side = vec![false; b.out.faces.len()];
    for (verts, f, corners) in sides {
        b.face(verts, Some(f), corners);
        top.push(false);
        side.push(true);
    }
    // Individual mode put the faces in order already; `top` follows faces.
    let mesh = b.finish();
    top.resize(mesh.faces.len(), false);
    Extruded { mesh, top: Marked::Faces(top), side: Marked::Faces(side) }
}

// ---------------------------------------------------------------------------
// Delete / separate / duplicate

/// Delete Geometry on a mesh. `domain` is what `sel` is on; `mode` is
/// "ALL", "EDGE_FACE" or "ONLY_FACE".
pub fn delete(m: &Mesh, domain: Domain, sel: &[bool], mode: &str) -> Mesh {
    let t = MeshTopo::new(m);
    let s = |i: usize| sel.get(i).copied().unwrap_or(false);
    let nv = m.positions.len();
    let ne = t.edges.len();
    let nf = m.faces.len();
    let mut keep_v = vec![true; nv];
    let mut keep_e = vec![true; ne];
    let mut keep_f = vec![true; nf];
    match domain {
        Domain::Point => {
            keep_v = (0..nv).map(|v| !s(v)).collect();
            keep_e = t.edges.iter().map(|e| keep_v[e[0] as usize] && keep_v[e[1] as usize]).collect();
            keep_f = m.faces.iter().map(|f| f.verts.iter().all(|&v| keep_v[v as usize])).collect();
            if mode == "EDGE_FACE" || mode == "ONLY_FACE" {
                // Point selections in these modes delete faces/edges whose
                // vertices are all selected.
                keep_v = vec![true; nv];
                keep_e = t.edges.iter().map(|e| !(s(e[0] as usize) && s(e[1] as usize))).collect();
                keep_f = m.faces.iter().map(|f| !f.verts.iter().all(|&v| s(v as usize))).collect();
                if mode == "ONLY_FACE" {
                    keep_e = vec![true; ne];
                }
            }
        }
        Domain::Edge => {
            let del_e: Vec<bool> = (0..ne).map(s).collect();
            keep_f = (0..nf).map(|f| t.corners(f).all(|c| !del_e[t.corner_edge[c] as usize])).collect();
            if mode != "ONLY_FACE" {
                keep_e = del_e.iter().map(|d| !d).collect();
            }
            if mode == "ALL" {
                keep_v = (0..nv).map(|v| t.vert_edges[v].is_empty() || t.vert_edges[v].iter().any(|&e| keep_e[e as usize])).collect();
            }
        }
        _ => {
            keep_f = (0..nf).map(|f| !s(f)).collect();
            if mode != "ONLY_FACE" {
                keep_e = (0..ne).map(|e| t.edge_faces[e].is_empty() || t.edge_faces[e].iter().any(|&f| keep_f[f as usize])).collect();
            }
            if mode == "ALL" {
                keep_v = (0..nv).map(|v| t.vert_edges[v].is_empty() || t.vert_edges[v].iter().any(|&e| keep_e[e as usize])).collect();
            }
        }
    }
    build_kept(m, &t, &keep_v, &keep_e, &keep_f)
}

fn build_kept(m: &Mesh, t: &MeshTopo, keep_v: &[bool], keep_e: &[bool], keep_f: &[bool]) -> Mesh {
    let mut b = Builder::new(m, t);
    let mut remap = vec![u32::MAX; m.positions.len()];
    for v in 0..m.positions.len() {
        if keep_v[v] {
            remap[v] = b.copy_vert(v);
        }
    }
    let mut face_edges = HashSet::new();
    for f in 0..m.faces.len() {
        if keep_f[f] && m.faces[f].verts.iter().all(|&v| remap[v as usize] != u32::MAX) {
            b.copy_face(f, |v| remap[v as usize]);
            for c in t.corners(f) {
                face_edges.insert(t.corner_edge[c]);
            }
        }
    }
    for (e, ed) in t.edges.iter().enumerate() {
        if keep_e[e] && !face_edges.contains(&(e as u32)) {
            let (a, c) = (remap[ed[0] as usize], remap[ed[1] as usize]);
            if a != u32::MAX && c != u32::MAX {
                b.edge(a, c);
            }
        }
    }
    b.finish()
}

/// Duplicate Elements on a mesh: each selected element `amount` times as
/// separate geometry. Returns the mesh and the Duplicate Index per new
/// element of the output domain (points for POINT, edges for EDGE, faces).
pub fn duplicate(m: &Mesh, domain: Domain, sel: &[bool], amounts: &[i32]) -> (Mesh, Domain, Vec<i32>) {
    let t = MeshTopo::new(m);
    let mut b = Builder::new(m, &t);
    let mut idx = Vec::new();
    let s = |i: usize| sel.get(i).copied().unwrap_or(false);
    let k = |i: usize| amounts.get(i).copied().unwrap_or(0).max(0);
    match domain {
        Domain::Point => {
            for v in 0..m.positions.len() {
                if s(v) {
                    for d in 0..k(v) {
                        b.copy_vert(v);
                        idx.push(d);
                    }
                }
            }
            (b.finish(), Domain::Point, idx)
        }
        Domain::Edge => {
            for (e, ed) in t.edges.iter().enumerate() {
                if s(e) {
                    for d in 0..k(e) {
                        let a = b.copy_vert(ed[0] as usize);
                        let c = b.copy_vert(ed[1] as usize);
                        b.edge(a, c);
                        idx.push(d);
                    }
                }
            }
            (b.finish(), Domain::Edge, idx)
        }
        _ => {
            for f in 0..m.faces.len() {
                if s(f) {
                    for d in 0..k(f) {
                        let nv: Vec<u32> = m.faces[f].verts.iter().map(|&v| b.copy_vert(v as usize)).collect();
                        let corners = t.corners(f).map(|c| vec![(c, 1.0)]).collect();
                        b.face(nv, Some(f), corners);
                        idx.push(d);
                    }
                }
            }
            (b.finish(), Domain::Face, idx)
        }
    }
}

// ---------------------------------------------------------------------------
// Topology edits

/// Split the selected edges: around each vertex, face corners connected
/// through unselected edges keep sharing a vertex; each other group gets
/// its own copy.
pub fn split_edges(m: &Mesh, sel: &[bool]) -> Mesh {
    let t = MeshTopo::new(m);
    let mut b = Builder::new(m, &t);
    copy_all_verts(&mut b);
    let s = |e: usize| sel.get(e).copied().unwrap_or(false);
    // New vertex for every corner.
    let mut corner_vert: Vec<u32> = t.corner_vert.clone();
    for v in 0..m.positions.len() {
        let cs: Vec<usize> = t.vert_corners[v].iter().map(|&c| c as usize).collect();
        if cs.len() < 2 {
            continue;
        }
        let mut parent: Vec<usize> = (0..cs.len()).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            let mut i = i;
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        // Corners at v of faces sharing an unselected edge at v are joined.
        for &e in &t.vert_edges[v] {
            if s(e as usize) {
                continue;
            }
            let fs = &t.edge_faces[e as usize];
            let ks: Vec<usize> = fs.iter().filter_map(|&f| cs.iter().position(|&c| t.corner_face[c] == f)).collect();
            for w in ks.windows(2) {
                let (a, c) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
                parent[a] = c;
            }
        }
        let mut group_vert: HashMap<usize, u32> = HashMap::new();
        let mut first = true;
        for (i, &c) in cs.iter().enumerate() {
            let r = find(&mut parent, i);
            let nv = *group_vert.entry(r).or_insert_with(|| {
                if std::mem::replace(&mut first, false) {
                    v as u32
                } else {
                    b.copy_vert(v)
                }
            });
            corner_vert[c] = nv;
        }
    }
    for f in 0..m.faces.len() {
        let verts = t.corners(f).map(|c| corner_vert[c]).collect();
        let corners = t.corners(f).map(|c| vec![(c, 1.0)]).collect();
        b.face(verts, Some(f), corners);
    }
    copy_loose(&mut b);
    b.finish()
}

/// Reverse the winding of the selected faces (the first corner stays).
pub fn flip_faces(m: &Mesh, sel: &[bool]) -> Mesh {
    let t = MeshTopo::new(m);
    let mut b = Builder::new(m, &t);
    copy_all_verts(&mut b);
    for f in 0..m.faces.len() {
        let mut order: Vec<usize> = t.corners(f).collect();
        if sel.get(f).copied().unwrap_or(false) && order.len() > 2 {
            order[1..].reverse();
        }
        let verts = order.iter().map(|&c| t.corner_vert[c]).collect();
        let corners = order.iter().map(|&c| vec![(c, 1.0)]).collect();
        b.face(verts, Some(f), corners);
    }
    copy_loose(&mut b);
    b.finish()
}

/// Triangulate selected faces with at least `min_verts` corners.
pub fn triangulate(m: &Mesh, sel: &[bool], min_verts: usize, quad_method: &str) -> Mesh {
    let t = MeshTopo::new(m);
    let mut b = Builder::new(m, &t);
    copy_all_verts(&mut b);
    let tris = m.triangulate();
    let mut by_face: Vec<Vec<[u16; 3]>> = vec![Vec::new(); m.faces.len()];
    for tr in &tris {
        by_face[tr.face as usize].push(tr.corners);
    }
    for f in 0..m.faces.len() {
        let n = m.faces[f].verts.len();
        if !sel.get(f).copied().unwrap_or(false) || n < min_verts.max(4) || n < 4 {
            b.copy_face(f, |v| v);
            continue;
        }
        let list: Vec<[u16; 3]> = if n == 4 {
            let p: Vec<Vec3> = m.faces[f].verts.iter().map(|&v| m.positions[v as usize]).collect();
            let short02 = p[0].distance(p[2]) <= p[1].distance(p[3]);
            let use02 = match quad_method {
                "FIXED" => true,
                "FIXED_ALTERNATE" => false,
                "LONGEST_DIAGONAL" => !short02,
                _ => short02,
            };
            if use02 {
                vec![[0, 1, 2], [0, 2, 3]]
            } else {
                vec![[1, 2, 3], [1, 3, 0]]
            }
        } else {
            by_face[f].clone()
        };
        let base = t.face_offsets[f];
        for c in list {
            let verts = c.iter().map(|&k| m.faces[f].verts[k as usize]).collect();
            let corners = c.iter().map(|&k| vec![(base + k as usize, 1.0)]).collect();
            b.face(verts, Some(f), corners);
        }
    }
    copy_loose(&mut b);
    b.finish()
}

/// Merge selected vertices closer than `dist` (all pairs, or only along
/// edges when `connected`). Merged vertices keep the lowest index's position.
pub fn merge_by_distance(m: &Mesh, sel: &[bool], dist: f32, connected: bool) -> Mesh {
    let t = MeshTopo::new(m);
    let n = m.positions.len();
    let s = |v: usize| sel.get(v).copied().unwrap_or(false);
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let union = |p: &mut Vec<usize>, a: usize, c: usize| {
        let (ra, rc) = (find(p, a), find(p, c));
        if ra != rc {
            let (lo, hi) = (ra.min(rc), ra.max(rc));
            p[hi] = lo;
        }
    };
    if connected {
        for e in &t.edges {
            let (a, c) = (e[0] as usize, e[1] as usize);
            if s(a) && s(c) && m.positions[a].distance(m.positions[c]) <= dist {
                union(&mut parent, a, c);
            }
        }
    } else {
        let idx: Vec<usize> = (0..n).filter(|&v| s(v)).collect();
        let pts: Vec<Vec3> = idx.iter().map(|&v| m.positions[v]).collect();
        let kd = KdTree::new(&pts);
        let mut near = Vec::new();
        for (k, &v) in idx.iter().enumerate() {
            near.clear();
            kd.within(pts[k], dist, &mut near);
            for &j in &near {
                union(&mut parent, v, idx[j]);
            }
        }
    }
    let mut b = Builder::new(m, &t);
    let mut remap = vec![u32::MAX; n];
    for v in 0..n {
        let r = find(&mut parent, v);
        if r == v {
            remap[v] = b.copy_vert(v);
        }
    }
    for v in 0..n {
        let r = find(&mut parent, v);
        remap[v] = remap[r];
    }
    for f in 0..m.faces.len() {
        let mut verts = Vec::new();
        let mut corners = Vec::new();
        for c in t.corners(f) {
            let nv = remap[t.corner_vert[c] as usize];
            if verts.last() != Some(&nv) {
                verts.push(nv);
                corners.push(vec![(c, 1.0)]);
            }
        }
        while verts.len() > 1 && verts.first() == verts.last() {
            verts.pop();
            corners.pop();
        }
        let distinct: HashSet<u32> = verts.iter().copied().collect();
        if distinct.len() >= 3 && distinct.len() == verts.len() {
            b.face(verts, Some(f), corners);
        } else if distinct.len() == 2 {
            let v: Vec<u32> = distinct.into_iter().collect();
            b.edge(v[0], v[1]);
        }
    }
    for e in &m.loose_edges {
        b.edge(remap[e[0] as usize], remap[e[1] as usize]);
    }
    b.finish()
}

/// Scale selected faces or edges about their centres, connected groups
/// together (Blender's Scale Elements). `scales` and `centers` are per
/// element; `axis` (per element) is used for single-axis scaling.
pub fn scale_elements(m: &Mesh, domain: Domain, sel: &[bool], scales: &[f32], centers: &[Vec3], axis: Option<&[Vec3]>) -> Mesh {
    let t = MeshTopo::new(m);
    let elems: Vec<Vec<u32>> = match domain {
        Domain::Edge => t.edges.iter().map(|e| e.to_vec()).collect(),
        _ => m.faces.iter().map(|f| f.verts.clone()).collect(),
    };
    let s = |i: usize| sel.get(i).copied().unwrap_or(false);
    // Group selected elements sharing vertices.
    let mut parent: Vec<usize> = (0..elems.len()).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut owner: HashMap<u32, usize> = HashMap::new();
    for (i, el) in elems.iter().enumerate() {
        if !s(i) {
            continue;
        }
        for &v in el {
            if let Some(&o) = owner.get(&v) {
                let (a, c) = (find(&mut parent, i), find(&mut parent, o));
                parent[a] = c;
            } else {
                owner.insert(v, i);
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in (0..elems.len()).filter(|&i| s(i)) {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out = m.clone();
    let mut keys: Vec<usize> = groups.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        let g = &groups[&k];
        let n = g.len() as f32;
        let scale = g.iter().map(|&i| scales.get(i).copied().unwrap_or(1.0)).sum::<f32>() / n;
        let center = g.iter().fold(Vec3::ZERO, |a, &i| a + centers.get(i).copied().unwrap_or(Vec3::ZERO)) / n;
        let ax = axis.map(|a| g.iter().fold(Vec3::ZERO, |acc, &i| acc + a.get(i).copied().unwrap_or(Vec3::X)).normalize());
        let verts: HashSet<u32> = g.iter().flat_map(|&i| elems[i].iter().copied()).collect();
        for v in verts {
            let p = m.positions[v as usize] - center;
            out.positions[v as usize] = match ax {
                Some(a) => center + p + a * (p.dot(a) * (scale - 1.0)),
                None => center + p * scale,
            };
        }
    }
    out
}

/// Dual mesh: a vertex per face and a face per interior vertex (closed
/// manifold fans). Boundary vertices are skipped.
pub fn dual_mesh(m: &Mesh) -> Mesh {
    let t = MeshTopo::new(m);
    let mut out = Mesh::default();
    for f in 0..m.faces.len() {
        out.positions.push(m.face_center(f));
    }
    let vn = super::field::vertex_normals(m);
    for v in 0..m.positions.len() {
        let corners = &t.vert_corners[v];
        if corners.len() < 3 {
            continue;
        }
        let start = corners[0] as usize;
        let mut ring = vec![t.corner_face[start]];
        let mut c = start;
        let mut closed = false;
        for _ in 0..corners.len() + 1 {
            // Across the edge from the previous vertex to v.
            let e = t.corner_edge[t.prev_corner(c)] as usize;
            let fs = &t.edge_faces[e];
            if fs.len() != 2 {
                break;
            }
            let f = t.corner_face[c];
            let g = if fs[0] == f { fs[1] } else { fs[0] };
            let Some(nc) = corner_at(m, &t, g as usize, v as u32) else { break };
            if nc == start {
                closed = true;
                break;
            }
            ring.push(g);
            c = nc;
        }
        if !closed || ring.len() != corners.len() {
            continue;
        }
        let pts: Vec<Vec3> = ring.iter().map(|&f| out.positions[f as usize]).collect();
        let mut nrm = Vec3::ZERO;
        for i in 0..pts.len() {
            nrm += pts[i].cross(pts[(i + 1) % pts.len()]);
        }
        if nrm.dot(vn[v]) < 0.0 {
            ring.reverse();
        }
        out.faces.push(Face::new(ring));
    }
    out
}

/// Chains of selected edges as poly curves: (point source vertices, cyclic).
pub fn edge_chains(m: &Mesh, sel: &[bool]) -> Vec<(Vec<u32>, bool)> {
    let t = MeshTopo::new(m);
    let mut adj: Vec<Vec<(u32, usize)>> = vec![Vec::new(); m.positions.len()];
    for (e, ed) in t.edges.iter().enumerate() {
        if sel.get(e).copied().unwrap_or(false) {
            adj[ed[0] as usize].push((ed[1], e));
            adj[ed[1] as usize].push((ed[0], e));
        }
    }
    let mut used = vec![false; t.edges.len()];
    let mut out = Vec::new();
    let walk = |start: u32, first: (u32, usize), used: &mut Vec<bool>| -> Vec<u32> {
        let mut chain = vec![start];
        let (mut cur, mut e) = first;
        loop {
            used[e] = true;
            chain.push(cur);
            if adj[cur as usize].len() != 2 {
                break;
            }
            match adj[cur as usize].iter().find(|x| !used[x.1]) {
                Some(&(n, ne)) => {
                    cur = n;
                    e = ne;
                }
                None => break,
            }
        }
        chain
    };
    for v in 0..m.positions.len() {
        if adj[v].len() != 2 && !adj[v].is_empty() {
            for k in 0..adj[v].len() {
                let x = adj[v][k];
                if !used[x.1] {
                    out.push((walk(v as u32, x, &mut used), false));
                }
            }
        }
    }
    // Loops.
    for v in 0..m.positions.len() {
        while let Some(&x) = adj[v].iter().find(|x| !used[x.1]) {
            let mut chain = walk(v as u32, x, &mut used);
            if chain.len() > 1 && chain.first() == chain.last() {
                chain.pop();
            }
            out.push((chain, true));
        }
    }
    out
}
