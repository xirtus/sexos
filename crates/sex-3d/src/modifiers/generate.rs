// SPDX-License-Identifier: GPL-3.0-only
//! Generate modifiers that build new topology: Screw, Wireframe, Edge Split
//! and Build.
use super::ekey;
use crate::math::{Quat, Vec3};
use crate::mesh::{Face, Mesh};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Screw

pub struct ScrewParams {
    pub axis: u8,
    pub angle: f32,
    pub steps: u32,
    pub screw_offset: f32,
    pub iterations: u32,
    /// Weld vertices closer than this to the axis (no screw offset only).
    pub merge: Option<f32>,
    pub flip: bool,
    pub smooth_shade: bool,
}

/// Every edge, oriented: face edges follow their face's winding, chains of
/// loose edges run head to tail.
fn oriented_edges(m: &Mesh) -> Vec<[u32; 2]> {
    let mut seen: HashMap<(u32, u32), ()> = HashMap::new();
    let mut out = Vec::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            if a != b && seen.insert(ekey(a, b), ()).is_none() {
                out.push([a, b]);
            }
        }
    }
    let loose: Vec<[u32; 2]> = m.loose_edges.iter().copied().filter(|e| e[0] != e[1] && !seen.contains_key(&ekey(e[0], e[1]))).collect();
    let mut by_vert: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, e) in loose.iter().enumerate() {
        by_vert.entry(e[0]).or_default().push(i);
        by_vert.entry(e[1]).or_default().push(i);
    }
    let mut dir: Vec<Option<[u32; 2]>> = vec![None; loose.len()];
    for s in 0..loose.len() {
        if dir[s].is_some() {
            continue;
        }
        dir[s] = Some(loose[s]);
        let mut stack = vec![s];
        while let Some(i) = stack.pop() {
            let [a, b] = dir[i].unwrap();
            for (v, at_end) in [(b, true), (a, false)] {
                for &j in &by_vert[&v] {
                    if dir[j].is_some() {
                        continue;
                    }
                    let e = loose[j];
                    let other = if e[0] == v { e[1] } else { e[0] };
                    // Continue the chain: leave `v` after arriving, arrive before leaving.
                    dir[j] = Some(if at_end { [v, other] } else { [other, v] });
                    stack.push(j);
                }
            }
        }
    }
    out.extend(dir.into_iter().flatten());
    out
}

/// Sweep the edges around the axis. Original faces are not kept, like
/// Blender; the result faces outwards for a profile drawn upwards on +X
/// around Z, and `flip` reverses that.
pub fn screw(m: &Mesh, p: &ScrewParams) -> Mesh {
    let edges = oriented_edges(m);
    if edges.is_empty() {
        return m.clone();
    }
    let axis = [Vec3::X, Vec3::Y, Vec3::Z][(p.axis as usize).min(2)];
    let steps = p.steps.clamp(1, 10_000);
    let iterations = p.iterations.clamp(1, 10_000);
    let tau = std::f32::consts::TAU;
    let close = p.screw_offset == 0.0 && (p.angle.abs() - tau).abs() < 1e-4;
    let segs = if close { steps } else { (steps * iterations).min(1_000_000) };
    let rings = if close { steps } else { segs + 1 };
    let mut used: Vec<u32> = edges.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    let slot: HashMap<u32, usize> = used.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let merge = if p.screw_offset == 0.0 { p.merge } else { None };
    let on_axis: Vec<bool> = used
        .iter()
        .map(|&v| {
            let q = m.positions[v as usize];
            merge.is_some_and(|d| (q - axis * q.dot(axis)).length() <= d)
        })
        .collect();
    let mut out = m.clone();
    out.positions = Vec::with_capacity(rings as usize * used.len());
    out.faces = Vec::new();
    out.loose_edges = Vec::new();
    let mut idx = vec![0u32; rings as usize * used.len()];
    for r in 0..rings as usize {
        let t = r as f32 / steps as f32;
        let rot = Quat::from_axis_angle(axis, p.angle * t);
        let rise = axis * (p.screw_offset * t);
        for (s, &v) in used.iter().enumerate() {
            idx[r * used.len() + s] = if r > 0 && on_axis[s] { idx[s] } else { out.add_vertex(rot.rotate(m.positions[v as usize]) + rise) };
        }
    }
    let flip = p.flip ^ (p.angle < 0.0);
    for r in 0..segs as usize {
        let r2 = (r + 1) % rings as usize;
        for e in &edges {
            let (a, b) = (slot[&e[0]], slot[&e[1]]);
            let mut q = vec![idx[r * used.len() + a], idx[r2 * used.len() + a], idx[r2 * used.len() + b], idx[r * used.len() + b]];
            q.dedup();
            if q.len() > 1 && q.first() == q.last() {
                q.pop();
            }
            if q.len() < 3 {
                continue;
            }
            if flip {
                q.reverse();
            }
            out.faces.push(Face { verts: q, uvs: Vec::new(), material: 0, smooth: p.smooth_shade });
        }
    }
    out.remove_unused_vertices();
    out
}

// ---------------------------------------------------------------------------
// Wireframe

pub struct WireParams {
    pub thickness: f32,
    pub offset: f32,
    pub even: bool,
    pub relative: bool,
    pub boundary: bool,
    pub replace: bool,
}

/// A square tube along every edge: per face corner an inset point on the
/// face, two layers along the vertex normals, and per face edge a top, a
/// bottom and an inner wall quad (plus an outer wall on boundary edges when
/// `boundary` is set; without it boundary edges get no tube). A closed input
/// gives a closed manifold result.
pub fn wireframe(m: &Mesh, p: &WireParams) -> Mesh {
    let o = p.offset.clamp(-1.0, 1.0);
    let t = p.thickness;
    let (outer, inner) = (t * (o + 1.0) * 0.5, t * (o - 1.0) * 0.5);
    let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            *edge_count.entry(ekey(a, b)).or_default() += 1;
        }
    }
    let vn = m.vertex_normals();
    let nv = m.positions.len();
    let mut rel = vec![1.0f32; nv];
    if p.relative {
        let mut sum = vec![0.0f32; nv];
        let mut cnt = vec![0u32; nv];
        for e in m.edges() {
            let l = m.positions[e[0] as usize].distance(m.positions[e[1] as usize]);
            for v in e {
                sum[v as usize] += l;
                cnt[v as usize] += 1;
            }
        }
        for v in 0..nv {
            if cnt[v] > 0 {
                rel[v] = sum[v] / cnt[v] as f32;
            }
        }
    }
    let mut out = if p.replace {
        let mut o = m.clone();
        o.positions.clear();
        o.faces.clear();
        o.loose_edges.clear();
        o
    } else {
        m.clone()
    };
    let mut top = vec![u32::MAX; nv];
    let mut bot = vec![u32::MAX; nv];
    let layer = |v: u32, out: &mut Mesh, top: &mut Vec<u32>, bot: &mut Vec<u32>| -> (u32, u32) {
        let vi = v as usize;
        if top[vi] == u32::MAX {
            let q = m.positions[vi];
            let n = vn[vi] * rel[vi];
            top[vi] = out.add_vertex(q + n * outer);
            bot[vi] = out.add_vertex(q + n * inner);
        }
        (top[vi], bot[vi])
    };
    for (fi, f) in m.faces.iter().enumerate() {
        let k = f.verts.len();
        if k < 3 {
            continue;
        }
        let fnrm = m.face_normal(fi);
        let mut in_t = Vec::with_capacity(k);
        let mut in_b = Vec::with_capacity(k);
        for i in 0..k {
            let v = f.verts[i] as usize;
            let q = m.positions[v];
            let prev = m.positions[f.verts[(i + k - 1) % k] as usize];
            let next = m.positions[f.verts[(i + 1) % k] as usize];
            let (ep, en) = ((prev - q).normalize(), (next - q).normalize());
            let left = fnrm.cross(en).normalize();
            let mut dir = (ep + en).normalize();
            if dir == Vec3::ZERO {
                dir = left;
            } else if dir.dot(left) < 0.0 {
                dir = -dir;
            }
            let mut d = t * rel[v];
            if p.even {
                d /= dir.cross(en).length().max(0.1);
            }
            let c = q + dir * d;
            let n = vn[v] * rel[v];
            in_t.push(out.add_vertex(c + n * outer));
            in_b.push(out.add_vertex(c + n * inner));
        }
        for i in 0..k {
            let j = (i + 1) % k;
            let (a, b) = (f.verts[i], f.verts[j]);
            let boundary_edge = edge_count[&ekey(a, b)] == 1;
            if boundary_edge && !p.boundary {
                continue;
            }
            let (at, ab) = layer(a, &mut out, &mut top, &mut bot);
            let (bt, bb) = layer(b, &mut out, &mut top, &mut bot);
            let mut quad = |v: [u32; 4]| out.faces.push(Face { verts: v.to_vec(), uvs: Vec::new(), material: f.material, smooth: f.smooth });
            quad([at, bt, in_t[j], in_t[i]]);
            quad([ab, in_b[i], in_b[j], bb]);
            quad([in_t[i], in_t[j], in_b[j], in_b[i]]);
            if boundary_edge {
                quad([at, ab, bb, bt]);
            }
        }
    }
    if t < 0.0 {
        // Negative thickness turns the tubes inside out; keep them outward.
        let first_new = if p.replace { 0 } else { m.faces.len() };
        for f in &mut out.faces[first_new..] {
            f.verts.reverse();
            f.uvs.reverse();
        }
    }
    out.remove_unused_vertices();
    out
}

// ---------------------------------------------------------------------------
// Edge split

/// Give every face fan around a vertex its own copy of the vertex wherever
/// the faces meet across a split edge: edges whose face normals differ by
/// more than `angle` (when given) and edges with more than two faces.
pub fn edge_split(m: &Mesh, angle: Option<f32>) -> Mesh {
    let mut offset = Vec::with_capacity(m.faces.len() + 1);
    let mut total = 0usize;
    for f in &m.faces {
        offset.push(total);
        total += f.verts.len();
    }
    let mut parent: Vec<usize> = (0..total).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut by_edge: HashMap<(u32, u32), Vec<(usize, usize)>> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for (i, (a, b)) in f.edges().enumerate() {
            if a != b {
                by_edge.entry(ekey(a, b)).or_default().push((fi, i));
            }
        }
    }
    let normals: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_normal(f)).collect();
    let cos_lim = angle.map(|a| a.clamp(0.0, std::f32::consts::PI).cos());
    let mut keys: Vec<(u32, u32)> = by_edge.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let fs = &by_edge[&key];
        if fs.len() != 2 {
            continue;
        }
        let ((f1, i1), (f2, i2)) = (fs[0], fs[1]);
        if cos_lim.is_some_and(|c| normals[f1].dot(normals[f2]) < c) {
            continue;
        }
        let (n1, n2) = (m.faces[f1].verts.len(), m.faces[f2].verts.len());
        for v in [key.0, key.1] {
            let c1 = if m.faces[f1].verts[i1] == v { i1 } else { (i1 + 1) % n1 };
            let c2 = if m.faces[f2].verts[i2] == v { i2 } else { (i2 + 1) % n2 };
            let (r1, r2) = (find(&mut parent, offset[f1] + c1), find(&mut parent, offset[f2] + c2));
            parent[r1] = r2;
        }
    }
    let mut out = m.clone();
    let mut claimed = vec![false; m.positions.len()];
    let mut index_of: HashMap<usize, u32> = HashMap::new();
    for (fi, f) in out.faces.iter_mut().enumerate() {
        for (c, v) in f.verts.iter_mut().enumerate() {
            let root = find(&mut parent, offset[fi] + c);
            let vi = *v as usize;
            *v = *index_of.entry(root).or_insert_with(|| {
                if !claimed[vi] {
                    claimed[vi] = true;
                    vi as u32
                } else {
                    out.positions.push(m.positions[vi]);
                    (out.positions.len() - 1) as u32
                }
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Build

/// Faces (or, without faces, loose edges) appear one by one from frame
/// `start` over `length` frames, in index order or shuffled by `seed`.
/// `reverse` takes them away instead.
pub fn build(m: &Mesh, frame: f32, start: f32, length: f32, reverse: bool, seed: Option<u32>) -> Mesh {
    let mut frac = if length > 0.0 { ((frame - start) / length).clamp(0.0, 1.0) } else if frame >= start { 1.0 } else { 0.0 };
    if !frac.is_finite() {
        frac = 1.0;
    }
    if reverse {
        frac = 1.0 - frac;
    }
    let pick = |n: usize| -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        if let Some(s) = seed {
            // xorshift32, never zero.
            let mut x = s.wrapping_mul(2_654_435_761).wrapping_add(0x9E37_79B9) | 1;
            for i in (1..n).rev() {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                order.swap(i, x as usize % (i + 1));
            }
        }
        order.truncate((n as f32 * frac) as usize);
        order
    };
    let mut out = m.clone();
    if !m.faces.is_empty() {
        out.faces = pick(m.faces.len()).into_iter().map(|i| m.faces[i].clone()).collect();
    } else {
        out.loose_edges = pick(m.loose_edges.len()).into_iter().map(|i| m.loose_edges[i]).collect();
    }
    out.remove_unused_vertices();
    out
}
