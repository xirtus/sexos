// SPDX-License-Identifier: GPL-3.0-only
//! Decimate: quadric edge collapse and un-subdivide (Planar is
//! [`crate::ops::dissolve_coplanar`]).
//!
//! Collapse follows Blender's `BM_mesh_decimate_collapse`: the mesh is
//! triangulated, every vertex gets the sum of the plane quadrics of its
//! triangles (Garland & Heckbert), boundary edges add strongly weighted
//! perpendicular planes so outlines survive, and the cheapest edge is
//! collapsed to its quadric-optimal point until `ratio` of the triangles
//! remain. Collapses that would fold a triangle over or pinch the surface
//! (the link condition) are skipped. Faces whose triangles were never
//! touched come back as the original n-gons unless `triangulate` is set.
use super::ekey;
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

type Q = [f64; 10];
type P = [f64; 3];

/// Boundary planes weigh this much more than surface planes.
const BOUNDARY_WEIGHT: f64 = 100.0;

fn plane_quadric(n: P, d: f64, w: f64) -> Q {
    let [a, b, c] = n;
    [a * a, a * b, a * c, a * d, b * b, b * c, b * d, c * c, c * d, d * d].map(|x| x * w)
}

fn add_q(a: &mut Q, b: &Q) {
    for i in 0..10 {
        a[i] += b[i];
    }
}

fn eval_q(q: &Q, p: P) -> f64 {
    let [x, y, z] = p;
    q[0] * x * x + 2.0 * q[1] * x * y + 2.0 * q[2] * x * z + 2.0 * q[3] * x + q[4] * y * y + 2.0 * q[5] * y * z + 2.0 * q[6] * y + q[7] * z * z + 2.0 * q[8] * z + q[9]
}

/// The point minimising `q`, when the 3x3 system is well conditioned.
fn optimal(q: &Q) -> Option<P> {
    let m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
    let r = [-q[3], -q[6], -q[8]];
    let det = |m: &[[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = det(&m);
    let tr = q[0] + q[4] + q[7];
    if !(d.abs() > 1e-7 * tr * tr * tr) {
        return None;
    }
    let mut out = [0.0; 3];
    for (i, o) in out.iter_mut().enumerate() {
        let mut mi = m;
        for row in 0..3 {
            mi[row][i] = r[row];
        }
        *o = det(&mi) / d;
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: P) -> f64 {
    dot(a, a).sqrt()
}
fn scale(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[derive(PartialEq)]
struct Entry {
    cost: f64,
    a: u32,
    b: u32,
    stamp: (u32, u32),
}
impl Eq for Entry {}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        // Min-heap on cost; ties by index for determinism.
        o.cost.total_cmp(&self.cost).then_with(|| (o.a, o.b).cmp(&(self.a, self.b)))
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

struct State {
    pos: Vec<P>,
    tv: Vec<[u32; 3]>,
    alive: Vec<bool>,
    changed: Vec<bool>,
    vt: Vec<Vec<u32>>,
    q: Vec<Q>,
    boundary: Vec<bool>,
    locked: Vec<bool>,
    version: Vec<u32>,
    vdead: Vec<bool>,
    merged: Vec<u32>,
}

impl State {
    fn neighbours(&self, v: u32) -> Vec<u32> {
        let mut out: Vec<u32> = self.vt[v as usize].iter().flat_map(|&t| self.tv[t as usize]).filter(|&u| u != v).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Where collapsing `a`-`b` would put the vertex, and the cost.
    fn plan(&self, a: u32, b: u32) -> (P, f64) {
        let (ai, bi) = (a as usize, b as usize);
        let mut q = self.q[ai];
        add_q(&mut q, &self.q[bi]);
        let (pa, pb) = (self.pos[ai], self.pos[bi]);
        let mid = scale([pa[0] + pb[0], pa[1] + pb[1], pa[2] + pb[2]], 0.5);
        let p = match (self.boundary[ai], self.boundary[bi]) {
            (true, false) => pa,
            (false, true) => pb,
            _ => {
                let mut cands = vec![pa, pb, mid];
                if let Some(o) = optimal(&q) {
                    // Keep the optimum near the edge; far-away optima are
                    // numerically shaky.
                    let l = len(sub(pa, pb));
                    if len(sub(o, mid)) <= 2.0 * l + 1e-12 {
                        cands.insert(0, o);
                    }
                }
                cands.into_iter().min_by(|x, y| eval_q(&q, *x).total_cmp(&eval_q(&q, *y))).unwrap()
            }
        };
        (p, eval_q(&q, p).max(0.0))
    }

    fn push(&self, heap: &mut BinaryHeap<Entry>, a: u32, b: u32) {
        if self.locked[a as usize] || self.locked[b as usize] {
            return;
        }
        let (_, cost) = self.plan(a, b);
        heap.push(Entry { cost, a, b, stamp: (self.version[a as usize], self.version[b as usize]) });
    }

    /// Collapse `b` into `a` at `p`; returns the number of triangles removed
    /// or `None` when the collapse would damage the surface.
    fn collapse(&mut self, a: u32, b: u32, p: P) -> Option<usize> {
        let shared: Vec<u32> = self.vt[a as usize].iter().copied().filter(|&t| self.tv[t as usize].contains(&b)).collect();
        if shared.is_empty() || shared.len() > 2 {
            return None;
        }
        if shared.len() == 2 && self.boundary[a as usize] && self.boundary[b as usize] {
            return None;
        }
        // Link condition: the only common neighbours are the triangles' apexes.
        let (na, nb) = (self.neighbours(a), self.neighbours(b));
        let common = na.iter().filter(|v| nb.binary_search(v).is_ok()).count();
        if common != shared.len() {
            return None;
        }
        // No triangle may flip or collapse to a sliver.
        for &v in &[a, b] {
            for &t in &self.vt[v as usize] {
                if shared.contains(&t) {
                    continue;
                }
                let tri = self.tv[t as usize];
                let pts = tri.map(|x| self.pos[x as usize]);
                let n0 = cross(sub(pts[1], pts[0]), sub(pts[2], pts[0]));
                let moved = tri.map(|x| if x == v { p } else { self.pos[x as usize] });
                let n1 = cross(sub(moved[1], moved[0]), sub(moved[2], moved[0]));
                let (l0, l1) = (len(n0), len(n1));
                if l1 <= 1e-12 * (l0 + 1e-30) || dot(n0, n1) <= 0.2 * l0 * l1 {
                    return None;
                }
            }
        }
        for &t in &shared {
            self.alive[t as usize] = false;
            for v in self.tv[t as usize] {
                self.vt[v as usize].retain(|&x| x != t);
            }
        }
        let moved: Vec<u32> = std::mem::take(&mut self.vt[b as usize]);
        for t in moved {
            for v in &mut self.tv[t as usize] {
                if *v == b {
                    *v = a;
                }
            }
            self.vt[a as usize].push(t);
        }
        for &t in &self.vt[a as usize] {
            self.changed[t as usize] = true;
        }
        self.pos[a as usize] = p;
        let qb = self.q[b as usize];
        add_q(&mut self.q[a as usize], &qb);
        self.boundary[a as usize] |= self.boundary[b as usize];
        self.vdead[b as usize] = true;
        self.merged[b as usize] = a;
        self.version[a as usize] += 1;
        Some(shared.len())
    }
}

/// Quadric edge collapse to `ratio` of the triangle count.
pub fn collapse(m: &Mesh, ratio: f32, triangulate: bool) -> Mesh {
    let ratio = if ratio.is_finite() { ratio.clamp(0.0, 1.0) } else { 1.0 };
    if ratio >= 1.0 {
        return m.clone();
    }
    let tris = m.triangulate();
    let nv = m.positions.len();
    let target = (tris.len() as f64 * ratio as f64).round() as usize;
    let mut st = State {
        pos: m.positions.iter().map(|p| [p.x as f64, p.y as f64, p.z as f64]).collect(),
        tv: tris.iter().map(|t| t.verts).collect(),
        alive: vec![true; tris.len()],
        changed: vec![false; tris.len()],
        vt: vec![Vec::new(); nv],
        q: vec![[0.0; 10]; nv],
        boundary: vec![false; nv],
        locked: vec![false; nv],
        version: vec![0; nv],
        vdead: vec![false; nv],
        merged: (0..nv as u32).collect(),
    };
    // Triangles with repeated corners are dropped up front.
    for (ti, t) in st.tv.iter().enumerate() {
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
            st.alive[ti] = false;
            continue;
        }
        for &v in t {
            st.vt[v as usize].push(ti as u32);
        }
    }
    let mut edge_tris: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (ti, t) in st.tv.iter().enumerate() {
        if !st.alive[ti] {
            continue;
        }
        let pts = t.map(|v| st.pos[v as usize]);
        let n = cross(sub(pts[1], pts[0]), sub(pts[2], pts[0]));
        let l = len(n);
        if l > 0.0 {
            let n = scale(n, 1.0 / l);
            let q = plane_quadric(n, -dot(n, pts[0]), l * 0.5);
            for &v in t {
                add_q(&mut st.q[v as usize], &q);
            }
        }
        for i in 0..3 {
            edge_tris.entry(ekey(t[i], t[(i + 1) % 3])).or_default().push(ti as u32);
        }
    }
    let mut edges: Vec<(u32, u32)> = edge_tris.keys().copied().collect();
    edges.sort_unstable();
    for &(a, b) in &edges {
        let ts = &edge_tris[&(a, b)];
        match ts.len() {
            1 => {
                let t = st.tv[ts[0] as usize];
                let pts = t.map(|v| st.pos[v as usize]);
                let fnrm = cross(sub(pts[1], pts[0]), sub(pts[2], pts[0]));
                let e = sub(st.pos[b as usize], st.pos[a as usize]);
                let n = cross(e, fnrm);
                let l = len(n);
                if l > 0.0 {
                    let n = scale(n, 1.0 / l);
                    let q = plane_quadric(n, -dot(n, st.pos[a as usize]), BOUNDARY_WEIGHT * dot(e, e));
                    add_q(&mut st.q[a as usize], &q);
                    add_q(&mut st.q[b as usize], &q);
                }
                st.boundary[a as usize] = true;
                st.boundary[b as usize] = true;
            }
            2 => {}
            _ => {
                st.locked[a as usize] = true;
                st.locked[b as usize] = true;
            }
        }
    }
    let mut heap = BinaryHeap::with_capacity(edges.len());
    for &(a, b) in &edges {
        st.push(&mut heap, a, b);
    }
    let mut live = st.alive.iter().filter(|&&a| a).count();
    while live > target {
        let Some(e) = heap.pop() else { break };
        let (a, b) = (e.a, e.b);
        if st.vdead[a as usize] || st.vdead[b as usize] || e.stamp != (st.version[a as usize], st.version[b as usize]) {
            continue;
        }
        let (p, _) = st.plan(a, b);
        let Some(removed) = st.collapse(a, b, p) else { continue };
        live -= removed;
        // Only `a` changed (its version bump made its old entries stale);
        // costs depend on the two end vertices alone.
        for u in st.neighbours(a) {
            st.push(&mut heap, a.min(u), a.max(u));
        }
    }

    // Rebuild: untouched source faces come back whole unless triangulating.
    let mut face_tris: Vec<Vec<u32>> = vec![Vec::new(); m.faces.len()];
    for (ti, t) in tris.iter().enumerate() {
        face_tris[t.face as usize].push(ti as u32);
    }
    let mut out = m.clone();
    out.positions = st.pos.iter().map(|p| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)).collect();
    out.faces = Vec::with_capacity(live);
    for (fi, f) in m.faces.iter().enumerate() {
        let ts = &face_tris[fi];
        if ts.is_empty() {
            continue;
        }
        if !triangulate && ts.iter().all(|&t| st.alive[t as usize] && !st.changed[t as usize]) {
            out.faces.push(f.clone());
            continue;
        }
        let has_uv = f.uvs.len() == f.verts.len();
        for &t in ts {
            if !st.alive[t as usize] {
                continue;
            }
            let uvs: Vec<Vec2> = if has_uv { tris[t as usize].corners.iter().map(|&c| f.uvs[c as usize]).collect() } else { Vec::new() };
            out.faces.push(Face { verts: st.tv[t as usize].to_vec(), uvs, material: f.material, smooth: f.smooth });
        }
    }
    let find = |mut v: u32| {
        while st.merged[v as usize] != v {
            v = st.merged[v as usize];
        }
        v
    };
    out.loose_edges = m.loose_edges.iter().map(|e| [find(e[0]), find(e[1])]).filter(|e| e[0] != e[1]).collect();
    out.remove_unused_vertices();
    out
}

/// Undo `iterations` levels of grid subdivision. Every level must be all
/// quads arranged the way subdivision leaves them (each quad: an original
/// corner, two edge points, a face point); otherwise the mesh is returned as
/// the last level that worked.
pub fn unsubdivide(m: &Mesh, iterations: u32) -> Mesh {
    let mut out = m.clone();
    for _ in 0..iterations.min(16) {
        match unsubdivide_once(&out) {
            Some(o) => out = o,
            None => break,
        }
    }
    out
}

const O: u8 = 1;
const F: u8 = 2;
const E: u8 = 3;

fn unsubdivide_once(m: &Mesh) -> Option<Mesh> {
    if m.faces.is_empty() || m.faces.iter().any(|f| f.verts.len() != 4) {
        return None;
    }
    let nv = m.positions.len();
    let mut vf: Vec<Vec<u32>> = vec![Vec::new(); nv];
    for (fi, f) in m.faces.iter().enumerate() {
        for &v in &f.verts {
            vf[v as usize].push(fi as u32);
        }
    }
    let mut edge_faces: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            *edge_faces.entry(ekey(a, b)).or_default() += 1;
        }
    }
    let mut valence = vec![0u32; nv];
    let mut boundary = vec![false; nv];
    for (&(a, b), &c) in &edge_faces {
        valence[a as usize] += 1;
        valence[b as usize] += 1;
        if c == 1 {
            boundary[a as usize] = true;
            boundary[b as usize] = true;
        }
    }
    // Seeds: original corners are where valence is irregular.
    let score = |v: usize| match (boundary[v], valence[v]) {
        (false, n) if n != 4 => 3,
        (true, 2) => 2,
        _ => 1,
    };
    let mut cands: Vec<usize> = (0..nv).filter(|&v| !vf[v].is_empty()).collect();
    cands.sort_by_key(|&v| (std::cmp::Reverse(score(v)), v));
    let mut label = vec![0u8; nv];
    let mut queue = Vec::new();
    for seed in cands {
        if label[seed] != 0 {
            continue;
        }
        label[seed] = O;
        queue.push(seed as u32);
        while let Some(v) = queue.pop() {
            let lv = label[v as usize];
            for &f in &vf[v as usize] {
                let fv = &m.faces[f as usize].verts;
                let k = fv.iter().position(|&x| x == v)?;
                let opp = fv[(k + 2) % 4] as usize;
                let want = if lv == O { F } else { O };
                match label[opp] {
                    0 => {
                        label[opp] = want;
                        queue.push(opp as u32);
                    }
                    l if l != want => return None,
                    _ => {}
                }
                for side in [fv[(k + 1) % 4] as usize, fv[(k + 3) % 4] as usize] {
                    match label[side] {
                        0 => label[side] = E,
                        E => {}
                        _ => return None,
                    }
                }
            }
        }
    }
    // Around every face point, chain its quads into the coarse face.
    let mut keep = vec![u32::MAX; nv];
    let mut positions = Vec::new();
    for v in 0..nv {
        if label[v] == O {
            keep[v] = positions.len() as u32;
            positions.push(m.positions[v]);
        }
    }
    let mut out = m.clone();
    out.positions = positions;
    out.faces = Vec::new();
    for fp in 0..nv {
        if label[fp] != F {
            continue;
        }
        // F-first rotation of each quad: [F, x, O, y]; the next quad starts at y.
        let mut by_x: HashMap<u32, (u32, u32, u32)> = HashMap::new();
        for &f in &vf[fp] {
            let fv = &m.faces[f as usize].verts;
            let k = fv.iter().position(|&x| x as usize == fp)?;
            by_x.insert(fv[(k + 1) % 4], (fv[(k + 2) % 4], fv[(k + 3) % 4], f));
        }
        let first = *vf[fp].first()?;
        let fv = &m.faces[first as usize].verts;
        let k = fv.iter().position(|&x| x as usize == fp)?;
        let start = fv[(k + 1) % 4];
        let mut x = start;
        let mut verts = Vec::new();
        let mut uvs = Vec::new();
        let src = &m.faces[first as usize];
        loop {
            let &(o, y, f) = by_x.get(&x)?;
            verts.push(keep[o as usize]);
            let face = &m.faces[f as usize];
            if face.uvs.len() == 4 {
                let c = face.verts.iter().position(|&q| q == o)?;
                uvs.push(face.uvs[c]);
            }
            x = y;
            if x == start || verts.len() > vf[fp].len() {
                break;
            }
        }
        if x != start || verts.len() != vf[fp].len() || verts.iter().any(|&v| v == u32::MAX) {
            return None;
        }
        if uvs.len() != verts.len() {
            uvs.clear();
        }
        out.faces.push(Face { verts, uvs, material: src.material, smooth: src.smooth });
    }
    out.loose_edges = m.loose_edges.iter().filter(|e| keep[e[0] as usize] != u32::MAX && keep[e[1] as usize] != u32::MAX).map(|e| [keep[e[0] as usize], keep[e[1] as usize]]).collect();
    if out.faces.is_empty() {
        return None;
    }
    Some(out)
}
