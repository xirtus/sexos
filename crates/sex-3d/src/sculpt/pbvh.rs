// SPDX-License-Identifier: GPL-3.0-only
//! A bounding volume hierarchy over a mesh's triangles whose bounds follow
//! the vertices (Blender's PBVH): strokes find the vertices under the brush
//! and the surface under the pointer without visiting the whole mesh, and
//! after a dab only the leaves holding moved vertices are refitted.
use crate::math::{Aabb, Vec3};
use crate::mesh::Mesh;

/// Triangles per leaf.
const LEAF: usize = 96;

#[derive(Clone, Debug)]
struct Node {
    bounds: Aabb,
    /// Children (left, right), or `u32::MAX` for a leaf.
    left: u32,
    right: u32,
    parent: u32,
    /// Leaf id for leaves.
    leaf: u32,
}

/// The hierarchy. Rebuild it when the topology changes.
#[derive(Clone, Debug, Default)]
pub struct Pbvh {
    /// Triangles (vertex triples), grouped by leaf.
    pub tris: Vec<[u32; 3]>,
    /// Source face of each triangle.
    pub tri_face: Vec<u32>,
    nodes: Vec<Node>,
    /// Per leaf: node index, triangle range, unique vertices.
    leaf_node: Vec<u32>,
    leaf_tris: Vec<(u32, u32)>,
    leaf_verts: Vec<Vec<u32>>,
    /// Vertex to leaves (CSR).
    vl_start: Vec<u32>,
    vl: Vec<u32>,
    dirty: Vec<bool>,
}

/// Distance test of a brush: a ball, or an infinite cylinder along a view
/// direction (Blender's "Projected" falloff shape).
#[derive(Clone, Copy, Debug)]
pub enum Region {
    Sphere { center: Vec3, radius: f32 },
    Cylinder { center: Vec3, axis: Vec3, radius: f32 },
}

impl Region {
    /// Distance of `p` from the region's centre (along the test's measure).
    pub fn dist(&self, p: Vec3) -> f32 {
        match *self {
            Region::Sphere { center, .. } => p.distance(center),
            Region::Cylinder { center, axis, .. } => {
                let d = p - center;
                (d - axis * d.dot(axis)).length()
            }
        }
    }
    pub fn radius(&self) -> f32 {
        match *self {
            Region::Sphere { radius, .. } | Region::Cylinder { radius, .. } => radius,
        }
    }
    fn overlaps(&self, b: &Aabb) -> bool {
        if b.is_empty() {
            return false;
        }
        match *self {
            Region::Sphere { center, radius } => {
                let q = center.max(b.min).min(b.max);
                q.distance(center) <= radius
            }
            Region::Cylinder { center, axis, radius } => {
                let c = b.center();
                let half = b.size().length() * 0.5;
                let d = c - center;
                (d - axis * d.dot(axis)).length() <= radius + half
            }
        }
    }
}

impl Pbvh {
    pub fn build(m: &Mesh) -> Pbvh {
        let tris_src = m.triangulate();
        let pos = &m.positions;
        let n = tris_src.len();
        let cent: Vec<Vec3> = tris_src.iter().map(|t| (pos[t.verts[0] as usize] + pos[t.verts[1] as usize] + pos[t.verts[2] as usize]) / 3.0).collect();
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut pb = Pbvh::default();
        // Iterative top-down median split; ranges into `order`.
        let mut stack: Vec<(usize, usize, u32, bool)> = vec![(0, n, u32::MAX, false)];
        while let Some((lo, hi, parent, is_right)) = stack.pop() {
            let idx = pb.nodes.len() as u32;
            pb.nodes.push(Node { bounds: Aabb::EMPTY, left: u32::MAX, right: u32::MAX, parent, leaf: u32::MAX });
            if parent != u32::MAX {
                let p = &mut pb.nodes[parent as usize];
                if is_right {
                    p.right = idx;
                } else {
                    p.left = idx;
                }
            }
            if hi - lo <= LEAF {
                let leaf = pb.leaf_node.len() as u32;
                pb.nodes[idx as usize].leaf = leaf;
                pb.leaf_node.push(idx);
                let start = pb.tris.len() as u32;
                let mut verts = Vec::with_capacity((hi - lo) * 2);
                for &t in &order[lo..hi] {
                    let tr = &tris_src[t as usize];
                    pb.tris.push(tr.verts);
                    pb.tri_face.push(tr.face);
                    verts.extend_from_slice(&tr.verts);
                }
                verts.sort_unstable();
                verts.dedup();
                pb.leaf_tris.push((start, pb.tris.len() as u32));
                pb.leaf_verts.push(verts);
                continue;
            }
            let mut cb = Aabb::EMPTY;
            for &t in &order[lo..hi] {
                cb.grow(cent[t as usize]);
            }
            let s = cb.size();
            let axis = if s.x >= s.y && s.x >= s.z { 0 } else if s.y >= s.z { 1 } else { 2 };
            let mid = (lo + hi) / 2;
            order[lo..hi].select_nth_unstable_by(mid - lo, |&a, &b| cent[a as usize][axis].total_cmp(&cent[b as usize][axis]));
            // Right first so the left child gets the next index (pre-order).
            stack.push((mid, hi, idx, true));
            stack.push((lo, mid, idx, false));
        }
        if pb.nodes.is_empty() {
            return pb;
        }
        // Vertex to leaves.
        let nv = pos.len();
        let mut count = vec![0u32; nv + 1];
        for lv in &pb.leaf_verts {
            for &v in lv {
                count[v as usize + 1] += 1;
            }
        }
        for i in 0..nv {
            count[i + 1] += count[i];
        }
        let mut fill = count.clone();
        pb.vl = vec![0; count[nv] as usize];
        for (l, lv) in pb.leaf_verts.iter().enumerate() {
            for &v in lv {
                pb.vl[fill[v as usize] as usize] = l as u32;
                fill[v as usize] += 1;
            }
        }
        pb.vl_start = count;
        pb.dirty = vec![true; pb.leaf_node.len()];
        pb.refit(pos);
        pb
    }

    pub fn leaf_count(&self) -> usize {
        self.leaf_node.len()
    }

    /// Unique vertices of a leaf.
    pub fn leaf_verts(&self, leaf: u32) -> &[u32] {
        &self.leaf_verts[leaf as usize]
    }

    /// Triangles of a leaf (indices into [`Pbvh::tris`]).
    pub fn leaf_tri_range(&self, leaf: u32) -> std::ops::Range<usize> {
        let (a, b) = self.leaf_tris[leaf as usize];
        a as usize..b as usize
    }

    /// Mark the leaves holding these vertices for refitting.
    pub fn mark_verts(&mut self, verts: &[u32]) {
        for &v in verts {
            let (a, b) = match (self.vl_start.get(v as usize), self.vl_start.get(v as usize + 1)) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            for &l in &self.vl[a as usize..b as usize] {
                self.dirty[l as usize] = true;
            }
        }
    }

    /// Recompute the bounds of marked leaves and their ancestors.
    pub fn refit(&mut self, pos: &[Vec3]) {
        if !self.dirty.iter().any(|&d| d) {
            return;
        }
        let mut node_dirty = vec![false; self.nodes.len()];
        for (l, d) in self.dirty.iter_mut().enumerate() {
            if !*d {
                continue;
            }
            *d = false;
            let ni = self.leaf_node[l] as usize;
            let mut b = Aabb::EMPTY;
            for &v in &self.leaf_verts[l] {
                if let Some(&p) = pos.get(v as usize) {
                    b.grow(p);
                }
            }
            self.nodes[ni].bounds = b;
            node_dirty[ni] = true;
        }
        // Children come after their parent, so a reverse pass sees them first.
        for i in (0..self.nodes.len()).rev() {
            if !node_dirty[i] {
                continue;
            }
            let p = self.nodes[i].parent;
            if p != u32::MAX && !node_dirty[p as usize] {
                node_dirty[p as usize] = true;
            }
            let n = &self.nodes[i];
            if n.left != u32::MAX {
                let b = self.nodes[n.left as usize].bounds.union(self.nodes[n.right as usize].bounds);
                self.nodes[i].bounds = b;
            }
        }
    }

    /// Bounds of everything.
    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map_or(Aabb::EMPTY, |n| n.bounds)
    }

    /// Leaves whose bounds meet `r`.
    pub fn leaves_in(&self, r: &Region) -> Vec<u32> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if !r.overlaps(&n.bounds) {
                continue;
            }
            if n.left == u32::MAX {
                out.push(n.leaf);
            } else {
                stack.push(n.left);
                stack.push(n.right);
            }
        }
        out
    }

    /// Nearest triangle hit by the ray `o + t d` (t > 0): (t, triangle,
    /// barycentrics u, v). `skip` rejects triangles by source face.
    pub fn raycast(&self, pos: &[Vec3], o: Vec3, d: Vec3, skip: &dyn Fn(u32) -> bool) -> Option<(f32, u32, f32, f32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut best: Option<(f32, u32, f32, f32)> = None;
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            let Some(tn) = slab(&n.bounds, o, inv) else { continue };
            if best.is_some_and(|b| tn > b.0) {
                continue;
            }
            if n.left != u32::MAX {
                stack.push(n.left);
                stack.push(n.right);
                continue;
            }
            for ti in self.leaf_tri_range(n.leaf) {
                if skip(self.tri_face[ti]) {
                    continue;
                }
                let [a, b, c] = self.tris[ti].map(|v| pos[v as usize]);
                if let Some((t, u, v)) = ray_tri(o, d, a, b, c) {
                    if best.is_none_or(|bb| t < bb.0) {
                        best = Some((t, ti as u32, u, v));
                    }
                }
            }
        }
        best
    }
}

fn slab(b: &Aabb, o: Vec3, inv: Vec3) -> Option<f32> {
    if b.is_empty() {
        return None;
    }
    let mut t0 = f32::NEG_INFINITY;
    let mut t1 = f32::INFINITY;
    for k in 0..3 {
        if !inv[k].is_finite() {
            // Parallel to this slab: inside it or never.
            if o[k] < b.min[k] - 1e-6 || o[k] > b.max[k] + 1e-6 {
                return None;
            }
            continue;
        }
        let (lo, hi) = ((b.min[k] - o[k]) * inv[k], (b.max[k] - o[k]) * inv[k]);
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        if lo > t0 {
            t0 = lo;
        }
        if hi < t1 {
            t1 = hi;
        }
    }
    let eps = 1e-6 * (1.0 + t1.abs());
    (t1 + eps >= t0.max(0.0)).then_some(t0.max(0.0))
}

/// Möller–Trumbore, both sides: (t, u, v).
pub fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-14 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = o - a;
    let u = tv.dot(p) * inv;
    if !(-1e-6..=1.0 + 1e-6).contains(&u) {
        return None;
    }
    let q = tv.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-6 || u + v > 1.0 + 1e-6 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-7).then_some((t, u, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn queries_and_refit() {
        let m = primitives::uv_sphere(48, 24, 1.0);
        let mut pb = Pbvh::build(&m);
        assert!(pb.leaf_count() > 4);
        assert_eq!(pb.tris.len(), m.triangulate().len());
        // Ray from +X hits the sphere at x = 1.
        let hit = pb.raycast(&m.positions, Vec3::new(5.0, 0.01, 0.02), Vec3::new(-1.0, 0.0, 0.0), &|_| false).unwrap();
        assert!((hit.0 - 4.0).abs() < 0.02, "{hit:?}");
        // Every vertex within the ball is in a returned leaf.
        let r = Region::Sphere { center: Vec3::new(1.0, 0.0, 0.0), radius: 0.3 };
        let leaves = pb.leaves_in(&r);
        let mut found: Vec<u32> = leaves.iter().flat_map(|&l| pb.leaf_verts(l).iter().copied()).collect();
        found.sort_unstable();
        found.dedup();
        for (v, p) in m.positions.iter().enumerate() {
            if r.dist(*p) < 0.3 {
                assert!(found.binary_search(&(v as u32)).is_ok());
            }
        }
        // Move a vertex far out: after refitting a ray finds it.
        let mut pos = m.positions.clone();
        let v = found[0];
        pos[v as usize] = Vec3::new(10.0, 0.0, 0.0);
        pb.mark_verts(&[v]);
        pb.refit(&pos);
        assert!(pb.bounds().max.x >= 10.0);
    }
}
