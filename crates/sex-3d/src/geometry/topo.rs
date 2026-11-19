// SPDX-License-Identifier: GPL-3.0-only
//! Mesh connectivity for attribute domains (edges in [`Mesh::edges`] order,
//! face corners in face order) and Blender's domain interpolation
//! (`adapt_mesh_domain`): moving values between points, edges, faces and
//! face corners by averaging the neighbours.
use super::attributes::{mix, AttrData, AttrType, Domain, Val};
use crate::math::{Vec2, Vec3};
use crate::mesh::Mesh;
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, Default)]
pub struct MeshTopo {
    pub edges: Vec<[u32; 2]>,
    pub edge_map: HashMap<(u32, u32), u32>,
    /// First corner of each face, plus the total at the end.
    pub face_offsets: Vec<usize>,
    pub corner_vert: Vec<u32>,
    /// The edge from each corner to the next one.
    pub corner_edge: Vec<u32>,
    pub corner_face: Vec<u32>,
    pub vert_corners: Vec<Vec<u32>>,
    pub vert_edges: Vec<Vec<u32>>,
    pub vert_faces: Vec<Vec<u32>>,
    pub edge_faces: Vec<Vec<u32>>,
}

impl MeshTopo {
    pub fn new(m: &Mesh) -> MeshTopo {
        let edges = m.edges();
        let mut edge_map = HashMap::with_capacity(edges.len());
        for (i, e) in edges.iter().enumerate() {
            edge_map.insert((e[0], e[1]), i as u32);
        }
        let nv = m.positions.len();
        let mut t = MeshTopo { edges, edge_map, vert_corners: vec![Vec::new(); nv], vert_edges: vec![Vec::new(); nv], vert_faces: vec![Vec::new(); nv], ..Default::default() };
        t.edge_faces = vec![Vec::new(); t.edges.len()];
        let mut c = 0usize;
        for (fi, f) in m.faces.iter().enumerate() {
            t.face_offsets.push(c);
            let n = f.verts.len();
            for k in 0..n {
                let (a, b) = (f.verts[k], f.verts[(k + 1) % n]);
                let e = t.edge_index(a, b).unwrap_or(0);
                t.corner_vert.push(a);
                t.corner_edge.push(e);
                t.corner_face.push(fi as u32);
                if let Some(v) = t.vert_corners.get_mut(a as usize) {
                    v.push(c as u32);
                }
                if let Some(v) = t.vert_faces.get_mut(a as usize) {
                    if v.last() != Some(&(fi as u32)) {
                        v.push(fi as u32);
                    }
                }
                if let Some(ef) = t.edge_faces.get_mut(e as usize) {
                    if ef.last() != Some(&(fi as u32)) {
                        ef.push(fi as u32);
                    }
                }
                c += 1;
            }
        }
        t.face_offsets.push(c);
        for (i, e) in t.edges.iter().enumerate() {
            for v in e {
                if let Some(x) = t.vert_edges.get_mut(*v as usize) {
                    x.push(i as u32);
                }
            }
        }
        t
    }

    pub fn edge_index(&self, a: u32, b: u32) -> Option<u32> {
        let k = if a < b { (a, b) } else { (b, a) };
        self.edge_map.get(&k).copied()
    }

    pub fn corners(&self, f: usize) -> std::ops::Range<usize> {
        self.face_offsets[f]..self.face_offsets[f + 1]
    }

    pub fn corner_count(&self) -> usize {
        self.corner_vert.len()
    }

    /// The corner before `c` in its face.
    pub fn prev_corner(&self, c: usize) -> usize {
        let r = self.corners(self.corner_face[c] as usize);
        if c == r.start {
            r.end - 1
        } else {
            c - 1
        }
    }

    pub fn next_corner(&self, c: usize) -> usize {
        let r = self.corners(self.corner_face[c] as usize);
        if c + 1 == r.end {
            r.start
        } else {
            c + 1
        }
    }

    pub fn size(&self, m: &Mesh, d: Domain) -> usize {
        match d {
            Domain::Point => m.positions.len(),
            Domain::Edge => self.edges.len(),
            Domain::Face => m.faces.len(),
            Domain::Corner => self.corner_vert.len(),
            _ => 0,
        }
    }
}

/// Move `vals` from domain `from` to domain `to` of a mesh.
pub fn adapt_mesh(m: &Mesh, t: &MeshTopo, vals: &[Val], from: Domain, to: Domain) -> Vec<Val> {
    if from == to {
        return vals.to_vec();
    }
    let def = vals.first().map(|v| AttrType::of_val(v).default_val()).unwrap_or(Val::Float(0.0));
    let get = |i: u32| vals.get(i as usize).copied().unwrap_or(def);
    let mixed = |ids: &mut dyn Iterator<Item = u32>, all: bool| mix(ids.map(get), all).unwrap_or(def);
    match (from, to) {
        (Domain::Point, Domain::Edge) => t.edges.iter().map(|e| mixed(&mut e.iter().copied(), true)).collect(),
        (Domain::Point, Domain::Face) => m.faces.iter().map(|f| mixed(&mut f.verts.iter().copied(), true)).collect(),
        (Domain::Point, Domain::Corner) => t.corner_vert.iter().map(|&v| get(v)).collect(),
        (Domain::Edge, Domain::Point) => t.vert_edges.iter().map(|es| mixed(&mut es.iter().copied(), false)).collect(),
        (Domain::Edge, Domain::Face) => (0..m.faces.len()).map(|f| mixed(&mut t.corners(f).map(|c| t.corner_edge[c]), true)).collect(),
        (Domain::Edge, Domain::Corner) => (0..t.corner_count()).map(|c| mixed(&mut [t.corner_edge[c], t.corner_edge[t.prev_corner(c)]].into_iter(), false)).collect(),
        (Domain::Face, Domain::Point) => t.vert_faces.iter().map(|fs| mixed(&mut fs.iter().copied(), false)).collect(),
        (Domain::Face, Domain::Edge) => t.edge_faces.iter().map(|fs| mixed(&mut fs.iter().copied(), false)).collect(),
        (Domain::Face, Domain::Corner) => t.corner_face.iter().map(|&f| get(f)).collect(),
        (Domain::Corner, Domain::Point) => t.vert_corners.iter().map(|cs| mixed(&mut cs.iter().copied(), false)).collect(),
        (Domain::Corner, Domain::Face) => (0..m.faces.len()).map(|f| mixed(&mut t.corners(f).map(|c| c as u32), true)).collect(),
        (Domain::Corner, Domain::Edge) => {
            let mut acc: Vec<Vec<Val>> = vec![Vec::new(); t.edges.len()];
            for c in 0..t.corner_count() {
                let e = t.corner_edge[c] as usize;
                if let Some(a) = acc.get_mut(e) {
                    a.push(get(c as u32));
                    a.push(get(t.next_corner(c) as u32));
                }
            }
            acc.into_iter().map(|a| mix(a, false).unwrap_or(def)).collect()
        }
        _ => vec![def; t.size(m, to)],
    }
}

/// Built-in mesh attributes, read as Blender names them, then the generic
/// ones: `position`, `material_index`, `sharp_face`, `sharp_edge`, the UV
/// map (`UVMap`), vertex groups by name.
pub fn read_mesh_attr(m: &Mesh, t: &MeshTopo, name: &str) -> Option<(Domain, AttrData)> {
    match name {
        "position" => return Some((Domain::Point, AttrData::Float3(m.positions.clone()))),
        "material_index" => return Some((Domain::Face, AttrData::Int(m.faces.iter().map(|f| f.material as i32).collect()))),
        "sharp_face" => return Some((Domain::Face, AttrData::Bool(m.faces.iter().map(|f| !f.smooth).collect()))),
        "sharp_edge" => return Some((Domain::Edge, AttrData::Bool(t.edges.iter().map(|e| m.is_sharp(e[0], e[1])).collect()))),
        "UVMap" | "uv_map" if m.has_uvs() && !m.attributes.contains(name) => {
            return Some((Domain::Corner, AttrData::Float2(m.faces.iter().flat_map(|f| f.uvs.iter().map(|u| [u.x, u.y])).collect())));
        }
        _ => {}
    }
    if let Some(a) = m.attributes.get(name) {
        if a.data.len() == t.size(m, a.domain) {
            return Some((a.domain, a.data.clone()));
        }
    }
    let g = m.vertex_group_index(name)?;
    Some((Domain::Point, AttrData::Float((0..m.positions.len()).map(|v| m.weight(v, g)).collect())))
}

/// Write an attribute (values already on `domain`); built-ins go to their
/// own storage.
pub fn write_mesh_attr(m: &mut Mesh, t: &MeshTopo, name: &str, domain: Domain, data: AttrData) {
    let vals = data.to_vals();
    let on = |d: Domain| adapt_mesh(m, t, &vals, domain, d);
    match name {
        "position" => {
            let v = on(Domain::Point);
            for (p, x) in m.positions.iter_mut().zip(v) {
                *p = x.v();
            }
        }
        "material_index" => {
            let v = on(Domain::Face);
            for (f, x) in m.faces.iter_mut().zip(v) {
                f.material = x.i().clamp(0, u16::MAX as i32) as u16;
            }
        }
        "sharp_face" => {
            let v = on(Domain::Face);
            for (f, x) in m.faces.iter_mut().zip(v) {
                f.smooth = !x.b();
            }
        }
        "sharp_edge" => {
            let v = on(Domain::Edge);
            let mut s = BTreeSet::new();
            for (e, x) in t.edges.iter().zip(v) {
                if x.b() {
                    s.insert((e[0], e[1]));
                }
            }
            m.sharp_edges = s;
        }
        "UVMap" | "uv_map" if !m.attributes.contains(name) => {
            let v = on(Domain::Corner);
            let mut c = 0;
            for f in &mut m.faces {
                f.uvs = (0..f.verts.len())
                    .map(|_| {
                        let p = v.get(c).map_or(Vec3::ZERO, |x| x.v());
                        c += 1;
                        Vec2::new(p.x, p.y)
                    })
                    .collect();
            }
        }
        _ => {
            if let Some(g) = m.vertex_group_index(name) {
                let v = on(Domain::Point);
                for (i, x) in v.iter().enumerate() {
                    m.set_weight(i, g, x.f());
                }
                return;
            }
            m.attributes.set(name, domain, data);
        }
    }
}

/// Names of every attribute a mesh exposes (built-ins first).
pub fn mesh_attr_names(m: &Mesh) -> Vec<(String, Domain, AttrType)> {
    let mut out = vec![("position".to_string(), Domain::Point, AttrType::Float3)];
    if m.faces.iter().any(|f| f.material != 0) {
        out.push(("material_index".into(), Domain::Face, AttrType::Int));
    }
    if m.faces.iter().any(|f| !f.smooth) {
        out.push(("sharp_face".into(), Domain::Face, AttrType::Bool));
    }
    if !m.sharp_edges.is_empty() {
        out.push(("sharp_edge".into(), Domain::Edge, AttrType::Bool));
    }
    if m.has_uvs() && !m.attributes.contains("UVMap") {
        out.push(("UVMap".into(), Domain::Corner, AttrType::Float2));
    }
    for g in &m.vertex_groups {
        out.push((g.clone(), Domain::Point, AttrType::Float));
    }
    for a in &m.attributes.list {
        out.push((a.name.clone(), a.domain, a.data.ty()));
    }
    out
}
