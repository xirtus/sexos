// SPDX-License-Identifier: GPL-3.0-only
//! More of Blender's edit-mode mesh operators, ported from the behaviour of
//! its bmesh operators: dissolves, subdivision, connect/split/rip, fills,
//! spin/screw, symmetry, convex hull, extrude variants, smoothing and the
//! topology queries behind the selection operators.
//!
//! Conventions follow [`crate::ops`]: every function edits a [`Mesh`] in
//! place and keeps it valid (counter-clockwise faces, UVs carried along where
//! the source faces had them). Selections go in as index lists and come back
//! as a [`Selection`] in the indices of the edited mesh, which is what Blender
//! leaves selected after the operator; vertex indices may be compacted by
//! operations that remove geometry.
mod connect;
mod dissolve;
mod fill;
mod hull;
mod misc;
mod rip;
mod select;
mod subdiv;

pub use connect::{connect_vert_pair, vert_connect, vert_connect_concave, vert_connect_nonplanar, vert_connect_path};
pub use dissolve::{delete_edgeloop, dissolve_degenerate, dissolve_edges, dissolve_faces, dissolve_verts, edge_collapse, unsubdivide};
pub use fill::{beautify_fill, face_split_by_edges, fill, fill_grid, tris_to_quads};
pub use hull::{convex_hull, convex_hull_triangles};
pub use misc::{
    edge_rotate, edge_split, extrude_geometry, extrude_manifold, extrude_repeat, face_make_planar, intersect_boolean, merge_at_vertex, offset_edge_loops, poke, screw, smooth_laplacian, spin, split, symmetrize, symmetry_snap,
};
pub use rip::{rip, rip_edge, RipResult};
pub use select::{edge_ring, interior_faces, mirror_vertex_map, non_manifold, shortest_path_faces, shortest_path_verts};
pub use subdiv::{subdivide, subdivide_edgering};

use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh, NONE};
use std::collections::{HashMap, HashSet};

/// What an operator leaves selected, in the indices of the edited mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub verts: Vec<u32>,
    /// Edges as (low, high) pairs.
    pub edges: Vec<[u32; 2]>,
    pub faces: Vec<u32>,
}

impl Selection {
    pub fn verts(mut v: Vec<u32>) -> Selection {
        v.sort_unstable();
        v.dedup();
        Selection { verts: v, ..Default::default() }
    }
    pub fn faces(m: &Mesh, mut f: Vec<u32>) -> Selection {
        f.sort_unstable();
        f.dedup();
        f.retain(|&x| (x as usize) < m.faces.len());
        let mut verts: Vec<u32> = f.iter().flat_map(|&x| m.faces[x as usize].verts.iter().copied()).collect();
        verts.sort_unstable();
        verts.dedup();
        let mut edges: Vec<[u32; 2]> = f.iter().flat_map(|&x| m.faces[x as usize].edges().map(|(a, b)| ek(a, b)).collect::<Vec<_>>()).collect();
        edges.sort_unstable();
        edges.dedup();
        Selection { verts, edges, faces: f }
    }
    pub fn edges(mut e: Vec<[u32; 2]>) -> Selection {
        for x in &mut e {
            *x = ek(x[0], x[1]);
        }
        e.sort_unstable();
        e.dedup();
        let mut verts: Vec<u32> = e.iter().flat_map(|x| [x[0], x[1]]).collect();
        verts.sort_unstable();
        verts.dedup();
        Selection { verts, edges: e, faces: Vec::new() }
    }
}

/// Undirected edge key as a tuple.
pub(crate) fn ekey(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}
/// Undirected edge key as an array.
pub(crate) fn ek(a: u32, b: u32) -> [u32; 2] {
    if a < b {
        [a, b]
    } else {
        [b, a]
    }
}

/// A length scale for tolerances: the bounding box diagonal (at least 1).
pub(crate) fn scale_of(m: &Mesh) -> f32 {
    let b = m.bounds();
    if b.is_empty() {
        1.0
    } else {
        b.size().length().max(1.0)
    }
}

/// Directed face edge (a, b) -> face.
pub(crate) fn dir_map(m: &Mesh) -> HashMap<(u32, u32), u32> {
    let mut map = HashMap::with_capacity(m.faces.len() * 4);
    for (fi, f) in m.faces.iter().enumerate() {
        for e in f.edges() {
            map.insert(e, fi as u32);
        }
    }
    map
}

/// Undirected edge -> faces using it.
pub(crate) fn edge_faces(m: &Mesh) -> HashMap<(u32, u32), Vec<u32>> {
    let mut map: HashMap<(u32, u32), Vec<u32>> = HashMap::with_capacity(m.faces.len() * 4);
    for (fi, f) in m.faces.iter().enumerate() {
        for (a, b) in f.edges() {
            map.entry(ekey(a, b)).or_default().push(fi as u32);
        }
    }
    map
}

/// Faces using each vertex.
pub(crate) fn vert_faces(m: &Mesh) -> Vec<Vec<u32>> {
    let mut out = vec![Vec::new(); m.positions.len()];
    for (fi, f) in m.faces.iter().enumerate() {
        for &v in &f.verts {
            if out[v as usize].last() != Some(&(fi as u32)) {
                out[v as usize].push(fi as u32);
            }
        }
    }
    out
}

/// Vertex adjacency through edges (face edges and loose edges).
pub(crate) fn adjacency(m: &Mesh) -> Vec<Vec<u32>> {
    let mut adj = vec![Vec::new(); m.positions.len()];
    for e in m.edges() {
        adj[e[0] as usize].push(e[1]);
        adj[e[1] as usize].push(e[0]);
    }
    adj
}

/// One face corner: vertex and UV (zero when the face has none).
pub(crate) type Corner = (u32, Vec2);

pub(crate) fn corners(f: &Face) -> Vec<Corner> {
    let has_uv = f.uvs.len() == f.verts.len();
    f.verts.iter().enumerate().map(|(i, &v)| (v, if has_uv { f.uvs[i] } else { Vec2::ZERO })).collect()
}

/// A face shaped like `src` (material, smoothing, UVs if it had them).
pub(crate) fn face_from(src: &Face, cs: &[Corner]) -> Face {
    let has_uv = src.uvs.len() == src.verts.len();
    Face { verts: cs.iter().map(|c| c.0).collect(), uvs: if has_uv { cs.iter().map(|c| c.1).collect() } else { Vec::new() }, material: src.material, smooth: src.smooth }
}

/// Union-find.
pub(crate) struct Dsu(Vec<u32>);
impl Dsu {
    pub fn new(n: usize) -> Dsu {
        Dsu((0..n as u32).collect())
    }
    pub fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            self.0[x as usize] = self.0[self.0[x as usize] as usize];
            x = self.0[x as usize];
        }
        x
    }
    pub fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // Keep the lower index as the root so results are deterministic.
            if ra < rb {
                self.0[rb as usize] = ra;
            } else {
                self.0[ra as usize] = rb;
            }
        }
    }
}

/// The outline of a set of faces as one loop of corners (shared edges
/// removed), or None when the region is pinched or has holes.
pub(crate) fn region_outline(m: &Mesh, faces: &[u32]) -> Option<Vec<Corner>> {
    let mut directed: HashSet<(u32, u32)> = HashSet::new();
    let mut uv_of: HashMap<(u32, u32), Vec2> = HashMap::new();
    for &f in faces {
        let face = &m.faces[f as usize];
        let cs = corners(face);
        let n = cs.len();
        for i in 0..n {
            directed.insert((cs[i].0, cs[(i + 1) % n].0));
            // UV of the corner at the start of this directed edge.
            uv_of.insert((cs[i].0, cs[(i + 1) % n].0), cs[i].1);
        }
    }
    let mut next: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &directed {
        if !directed.contains(&(b, a)) && next.insert(a, b).is_some() {
            return None;
        }
    }
    let &start = next.keys().min()?;
    let mut lp = vec![(start, uv_of[&(start, next[&start])])];
    let mut cur = next[&start];
    while cur != start {
        if lp.len() > next.len() {
            return None;
        }
        let nx = *next.get(&cur)?;
        lp.push((cur, uv_of[&(cur, nx)]));
        cur = nx;
    }
    (lp.len() == next.len() && lp.len() >= 3).then_some(lp)
}

/// Replace each group of faces by one face along its outline (groups whose
/// outline is not a single simple loop are left alone). Faces are removed
/// and the new ones appended; returns the new faces' indices, and a map from
/// old face index to new (NONE for removed faces).
pub(crate) fn join_groups(m: &mut Mesh, groups: &[Vec<u32>]) -> (Vec<u32>, Vec<u32>) {
    let mut remove = vec![false; m.faces.len()];
    let mut added = Vec::new();
    for g in groups {
        if g.len() < 2 {
            continue;
        }
        if let Some(lp) = region_outline(m, g) {
            let src = &m.faces[g[0] as usize];
            added.push(face_from(src, &lp));
            for &f in g {
                remove[f as usize] = true;
            }
        }
    }
    let map = retain_faces(m, &remove);
    let base = m.faces.len() as u32;
    let n = added.len() as u32;
    m.faces.extend(added);
    ((base..base + n).collect(), map)
}

/// Remove the flagged faces; returns old -> new face index (NONE if removed).
pub(crate) fn retain_faces(m: &mut Mesh, remove: &[bool]) -> Vec<u32> {
    let mut map = vec![NONE; m.faces.len()];
    let old = std::mem::take(&mut m.faces);
    for (i, f) in old.into_iter().enumerate() {
        if !remove.get(i).copied().unwrap_or(false) {
            map[i] = m.faces.len() as u32;
            m.faces.push(f);
        }
    }
    map
}

/// Remove vertices that no face or edge uses; returns old -> new (NONE for
/// removed vertices).
pub(crate) fn compact(m: &mut Mesh) -> Vec<u32> {
    let mut used = vec![false; m.positions.len()];
    for f in &m.faces {
        for &v in &f.verts {
            used[v as usize] = true;
        }
    }
    for e in &m.loose_edges {
        used[e[0] as usize] = true;
        used[e[1] as usize] = true;
    }
    let mut remap = vec![NONE; m.positions.len()];
    let mut kept = Vec::with_capacity(m.positions.len());
    for (i, &u) in used.iter().enumerate() {
        if u {
            remap[i] = kept.len() as u32;
            kept.push(m.positions[i]);
        }
    }
    m.positions = kept;
    let full: Vec<u32> = remap.iter().map(|&r| if r == NONE { 0 } else { r }).collect();
    m.remap_vertices(&full);
    remap
}

/// Map a list of indices through a remap, dropping removed ones.
pub(crate) fn remap_list(list: &[u32], remap: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = list.iter().filter_map(|&v| remap.get(v as usize).copied().filter(|&r| r != NONE)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Remove vertices from the faces using them where each such vertex has
/// exactly two edge neighbours and the faces keep at least three corners
/// (Blender's dissolve of valence-2 vertices). Returns how many were removed.
pub(crate) fn dissolve_valence2(m: &mut Mesh, candidates: &[u32]) -> usize {
    let adj = adjacency(m);
    let loose: HashSet<u32> = m.loose_edges.iter().flat_map(|e| [e[0], e[1]]).collect();
    let mut kill: HashSet<u32> = candidates.iter().copied().filter(|&v| (v as usize) < adj.len() && adj[v as usize].len() == 2 && !loose.contains(&v)).collect();
    // Never shrink a face below a triangle.
    for f in &m.faces {
        let k = f.verts.iter().filter(|v| kill.contains(v)).count();
        if k > 0 && f.verts.len() - k < 3 {
            for v in &f.verts {
                kill.remove(v);
            }
        }
    }
    if kill.is_empty() {
        return 0;
    }
    for f in &mut m.faces {
        if f.verts.iter().any(|v| kill.contains(v)) {
            let cs: Vec<Corner> = corners(f).into_iter().filter(|c| !kill.contains(&c.0)).collect();
            *f = face_from(f, &cs);
        }
    }
    kill.len()
}

/// Insert vertex `x` into the edge (p, q) of every face using it, with its
/// UV interpolated at `t` along p -> q.
pub(crate) fn insert_edge_vertex(m: &mut Mesh, p: u32, q: u32, x: u32, t: f32) {
    for f in &mut m.faces {
        let n = f.verts.len();
        let Some(i) = (0..n).find(|&i| {
            let (a, b) = (f.verts[i], f.verts[(i + 1) % n]);
            (a == p && b == q) || (a == q && b == p)
        }) else {
            continue;
        };
        let j = (i + 1) % n;
        if !f.uvs.is_empty() {
            let tt = if f.verts[i] == p { t } else { 1.0 - t };
            let uv = f.uvs[i].lerp(f.uvs[j], tt);
            f.uvs.insert(i + 1, uv);
        }
        f.verts.insert(i + 1, x);
    }
    m.loose_edges.retain(|e| ekey(e[0], e[1]) != ekey(p, q));
}

/// Split face `f` between its corners holding `u` and `w` (not neighbours).
/// The piece starting at `u` keeps the index; returns the other piece's index.
pub(crate) fn split_face(m: &mut Mesh, f: u32, u: u32, w: u32) -> Option<u32> {
    let face = &m.faces[f as usize];
    let n = face.verts.len();
    let iu = face.verts.iter().position(|&v| v == u)?;
    let iw = face.verts.iter().position(|&v| v == w)?;
    if iu == iw || (iu + 1) % n == iw || (iw + 1) % n == iu {
        return None;
    }
    let cs = corners(face);
    let arc = |from: usize, to: usize| -> Vec<Corner> {
        let mut out = Vec::new();
        let mut i = from;
        loop {
            out.push(cs[i]);
            if i == to {
                break;
            }
            i = (i + 1) % n;
        }
        out
    };
    let a = face_from(face, &arc(iu, iw));
    let b = face_from(face, &arc(iw, iu));
    m.faces[f as usize] = a;
    m.faces.push(b);
    Some(m.faces.len() as u32 - 1)
}

/// Merge vertices of `verts` lying within `dist` of each other (the lower
/// index survives). Faces and edges are remapped; nothing is compacted.
pub(crate) fn weld_subset(m: &mut Mesh, verts: &[u32], dist: f32) -> usize {
    let cell = dist.max(1e-12);
    let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
    let mut list: Vec<u32> = verts.iter().copied().filter(|&v| (v as usize) < m.positions.len()).collect();
    list.sort_unstable();
    list.dedup();
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    let mut remap: Vec<u32> = (0..m.positions.len() as u32).collect();
    let mut merged = 0;
    for &v in &list {
        let p = m.positions[v as usize];
        let k = key(p);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(l) = grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                        for &u in l {
                            if m.positions[u as usize].distance(p) <= dist {
                                found = Some(u);
                                break 'search;
                            }
                        }
                    }
                }
            }
        }
        match found {
            Some(u) => {
                remap[v as usize] = u;
                merged += 1;
            }
            None => grid.entry(k).or_default().push(v),
        }
    }
    if merged > 0 {
        m.remap_vertices(&remap);
    }
    merged
}

/// Ordered fan of faces around vertex `v`: the faces, the spoke vertex
/// between face i and face i+1 (the shared edge v-spoke), and whether the
/// fan closes. Open fans start and end at a boundary.
pub(crate) struct Fan {
    pub faces: Vec<u32>,
    /// spokes[i] is shared by faces[i] and faces[(i + 1) % len].
    pub spokes: Vec<u32>,
    pub closed: bool,
}

pub(crate) fn vertex_fan(m: &Mesh, dir: &HashMap<(u32, u32), u32>, vf: &[u32], v: u32) -> Option<Fan> {
    let &f0 = vf.first()?;
    // Next face going over the edge (v, next(v)) of face f.
    let step = |f: u32| -> (u32, Option<u32>) {
        let face = &m.faces[f as usize];
        let n = face.verts.len();
        let i = face.verts.iter().position(|&x| x == v).unwrap_or(0);
        let nx = face.verts[(i + 1) % n];
        (nx, dir.get(&(nx, v)).copied())
    };
    let back = |f: u32| -> (u32, Option<u32>) {
        let face = &m.faces[f as usize];
        let n = face.verts.len();
        let i = face.verts.iter().position(|&x| x == v).unwrap_or(0);
        let pv = face.verts[(i + n - 1) % n];
        (pv, dir.get(&(v, pv)).copied())
    };
    let mut faces = vec![f0];
    let mut spokes = Vec::new();
    let mut f = f0;
    let mut closed = false;
    loop {
        let (sp, g) = step(f);
        match g {
            Some(g) if g == f0 => {
                spokes.push(sp);
                closed = true;
                break;
            }
            Some(g) if !faces.contains(&g) && faces.len() <= vf.len() => {
                spokes.push(sp);
                faces.push(g);
                f = g;
            }
            _ => break,
        }
    }
    if !closed {
        // Walk backwards from f0 to the other boundary.
        let mut f = f0;
        loop {
            let (sp, g) = back(f);
            match g {
                Some(g) if !faces.contains(&g) && faces.len() <= vf.len() => {
                    faces.insert(0, g);
                    spokes.insert(0, sp);
                    f = g;
                }
                _ => break,
            }
        }
    }
    (faces.len() == vf.len()).then_some(Fan { faces, spokes, closed })
}

/// Average of the face normals around each vertex (area weighted).
pub(crate) fn vertex_normal(m: &Mesh, vf: &[Vec<u32>], v: u32) -> Vec3 {
    let mut n = Vec3::ZERO;
    for &f in &vf[v as usize] {
        n += m.face_normal(f as usize) * m.face_area(f as usize).max(1e-12);
    }
    n.normalize()
}

#[cfg(test)]
mod tests;
