// SPDX-License-Identifier: GPL-3.0-only
//! Triangle BVH: binned surface-area-heuristic build, ordered stack traversal
//! and Möller-Trumbore intersection on precomputed edges.
use crate::math::{Aabb, Vec3};

/// A triangle ready for intersection: a vertex and the two edges from it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriGeom {
    pub v0: Vec3,
    pub e1: Vec3,
    pub e2: Vec3,
}

impl TriGeom {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> TriGeom {
        TriGeom { v0: a, e1: b - a, e2: c - a }
    }
    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        b.grow(self.v0);
        b.grow(self.v0 + self.e1);
        b.grow(self.v0 + self.e2);
        b
    }
    /// Möller-Trumbore: (t, u, v) with barycentrics (1-u-v, u, v).
    #[inline(always)]
    pub fn intersect(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<(f32, f32, f32)> {
        let p = d.cross(self.e2);
        let det = self.e1.dot(p);
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let s = o - self.v0;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(self.e1);
        let v = d.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = self.e2.dot(q) * inv;
        (t > 0.0 && t < tmax).then_some((t, u, v))
    }
}

/// 32-byte node: inner nodes store their first child in `first` (the second
/// child is `first + 1`) and `count == 0`; leaves store a triangle range.
#[derive(Clone, Copy, Debug)]
struct Node {
    min: [f32; 3],
    first: u32,
    max: [f32; 3],
    count: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub t: f32,
    pub prim: u32,
    pub u: f32,
    pub v: f32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Bvh {
    nodes: Vec<Node>,
    /// Triangles in leaf order.
    pub tris: Vec<TriGeom>,
}

const BINS: usize = 16;
const MAX_LEAF: u32 = 4;

struct Prim {
    bounds: Aabb,
    centroid: Vec3,
}

impl Bvh {
    /// Build over `tris`; returns the BVH and, for each of its triangles in
    /// leaf order, the index it had in `tris`.
    pub fn build(tris: Vec<TriGeom>) -> (Bvh, Vec<u32>) {
        let prims: Vec<Prim> = tris
            .iter()
            .map(|t| {
                let b = t.bounds();
                Prim { bounds: b, centroid: b.center() }
            })
            .collect();
        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let mut nodes = Vec::with_capacity((tris.len() * 2).max(1));
        nodes.push(Node { min: [0.0; 3], first: 0, max: [0.0; 3], count: tris.len() as u32 });
        if !tris.is_empty() {
            let mut stack = vec![0usize];
            while let Some(ni) = stack.pop() {
                if let Some((l, r)) = split(&mut nodes, ni, &prims, &mut order) {
                    stack.push(l);
                    stack.push(r);
                }
            }
        } else {
            nodes[0].min = [1.0; 3]; // empty box: never hit
            nodes[0].max = [-1.0; 3];
        }
        let sorted = order.iter().map(|&i| tris[i as usize]).collect();
        (Bvh { nodes, tris: sorted }, order)
    }

    pub fn bounds(&self) -> Aabb {
        let n = &self.nodes[0];
        if self.tris.is_empty() {
            return Aabb::EMPTY;
        }
        Aabb { min: Vec3::from_array(n.min), max: Vec3::from_array(n.max) }
    }

    /// Nearest hit with `t < tmax`.
    pub fn intersect(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<Hit> {
        self.traverse::<false>(o, d, tmax)
    }

    /// True if anything is hit with `t < tmax`.
    pub fn occluded(&self, o: Vec3, d: Vec3, tmax: f32) -> bool {
        self.traverse::<true>(o, d, tmax).is_some()
    }

    #[inline(always)]
    fn traverse<const ANY: bool>(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<Hit> {
        if self.tris.is_empty() {
            return None;
        }
        let inv = [1.0 / d.x, 1.0 / d.y, 1.0 / d.z];
        let org = [o.x, o.y, o.z];
        let mut best: Option<Hit> = None;
        let mut tmax = tmax;
        let mut stack = [0u32; 64];
        let mut sp = 0usize;
        let mut ni = 0u32;
        if slab(&self.nodes[0], org, inv, tmax).is_none() {
            return None;
        }
        loop {
            let n = &self.nodes[ni as usize];
            if n.count > 0 {
                for i in n.first..n.first + n.count {
                    if let Some((t, u, v)) = self.tris[i as usize].intersect(o, d, tmax) {
                        tmax = t;
                        best = Some(Hit { t, prim: i, u, v });
                        if ANY {
                            return best;
                        }
                    }
                }
            } else {
                let (a, b) = (n.first, n.first + 1);
                let ta = slab(&self.nodes[a as usize], org, inv, tmax);
                let tb = slab(&self.nodes[b as usize], org, inv, tmax);
                match (ta, tb) {
                    (Some(ta), Some(tb)) => {
                        let (near, far) = if ta <= tb { (a, b) } else { (b, a) };
                        if sp < stack.len() {
                            stack[sp] = far;
                            sp += 1;
                        }
                        ni = near;
                        continue;
                    }
                    (Some(_), None) => {
                        ni = a;
                        continue;
                    }
                    (None, Some(_)) => {
                        ni = b;
                        continue;
                    }
                    (None, None) => {}
                }
            }
            if sp == 0 {
                return best;
            }
            sp -= 1;
            ni = stack[sp];
        }
    }
}

/// Entry distance of the ray into the node's box, if it is hit before `tmax`.
#[inline(always)]
fn slab(n: &Node, o: [f32; 3], inv: [f32; 3], tmax: f32) -> Option<f32> {
    // Plain compares (not f32::min/max) compile to branch-free minss/maxss. A
    // NaN from 0 * inf (ray in a slab plane) compares false and is skipped.
    #[inline(always)]
    fn mn(a: f32, b: f32) -> f32 {
        if a < b { a } else { b }
    }
    #[inline(always)]
    fn mx(a: f32, b: f32) -> f32 {
        if a > b { a } else { b }
    }
    let tx0 = (n.min[0] - o[0]) * inv[0];
    let tx1 = (n.max[0] - o[0]) * inv[0];
    let ty0 = (n.min[1] - o[1]) * inv[1];
    let ty1 = (n.max[1] - o[1]) * inv[1];
    let tz0 = (n.min[2] - o[2]) * inv[2];
    let tz1 = (n.max[2] - o[2]) * inv[2];
    // The possibly-NaN operand always goes first so it never reaches the accumulator.
    let t0 = mx(mn(tz0, tz1), mx(mn(ty0, ty1), mx(mn(tx0, tx1), 0.0)));
    let t1 = mn(mx(tz0, tz1), mn(mx(ty0, ty1), mn(mx(tx0, tx1), tmax)));
    (t0 <= t1).then_some(t0)
}

/// Fit node `ni` to its primitives and split it with binned SAH. Returns the
/// new child indices, or `None` if it stays a leaf.
fn split(nodes: &mut Vec<Node>, ni: usize, prims: &[Prim], order: &mut [u32]) -> Option<(usize, usize)> {
    let (first, count) = (nodes[ni].first as usize, nodes[ni].count as usize);
    let range = &mut order[first..first + count];
    let mut bounds = Aabb::EMPTY;
    let mut cbounds = Aabb::EMPTY;
    for &i in range.iter() {
        bounds = bounds.union(prims[i as usize].bounds);
        cbounds.grow(prims[i as usize].centroid);
    }
    nodes[ni].min = bounds.min.to_array();
    nodes[ni].max = bounds.max.to_array();
    if count as u32 <= 1 {
        return None;
    }
    let ext = cbounds.size();
    let leaf_cost = count as f32;
    let parent_area = bounds.surface_area().max(1e-30);
    let mut best: Option<(f32, usize, usize)> = None; // (cost, axis, split bin)
    for axis in 0..3 {
        if ext[axis] <= 1e-12 {
            continue;
        }
        let scale = BINS as f32 / ext[axis];
        let mut bin_b = [Aabb::EMPTY; BINS];
        let mut bin_n = [0u32; BINS];
        for &i in range.iter() {
            let p = &prims[i as usize];
            let b = (((p.centroid[axis] - cbounds.min[axis]) * scale) as usize).min(BINS - 1);
            bin_b[b] = bin_b[b].union(p.bounds);
            bin_n[b] += 1;
        }
        // Sweep from the right, then from the left.
        let mut right_area = [0.0f32; BINS];
        let mut right_n = [0u32; BINS];
        let (mut acc, mut n) = (Aabb::EMPTY, 0);
        for b in (1..BINS).rev() {
            acc = acc.union(bin_b[b]);
            n += bin_n[b];
            right_area[b] = acc.surface_area();
            right_n[b] = n;
        }
        let (mut acc, mut n) = (Aabb::EMPTY, 0);
        for b in 1..BINS {
            acc = acc.union(bin_b[b - 1]);
            n += bin_n[b - 1];
            if n == 0 || right_n[b] == 0 {
                continue;
            }
            let cost = 0.5 + (acc.surface_area() * n as f32 + right_area[b] * right_n[b] as f32) / parent_area;
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, axis, b));
            }
        }
    }
    let mid = match best {
        Some((cost, axis, bin)) if cost < leaf_cost || count as u32 > MAX_LEAF => {
            let scale = BINS as f32 / ext[axis];
            let goes_left = |i: u32| (((prims[i as usize].centroid[axis] - cbounds.min[axis]) * scale) as usize).min(BINS - 1) < bin;
            partition(range, goes_left)
        }
        Some(_) => return None,
        None if count as u32 > MAX_LEAF => count / 2, // all centroids coincide
        None => return None,
    };
    if mid == 0 || mid == count {
        return None;
    }
    let l = nodes.len();
    nodes.push(Node { min: [0.0; 3], first: first as u32, max: [0.0; 3], count: mid as u32 });
    nodes.push(Node { min: [0.0; 3], first: (first + mid) as u32, max: [0.0; 3], count: (count - mid) as u32 });
    nodes[ni].first = l as u32;
    nodes[ni].count = 0;
    Some((l, l + 1))
}

/// In-place partition; returns how many items satisfy `pred` (now in front).
fn partition(v: &mut [u32], pred: impl Fn(u32) -> bool) -> usize {
    let mut i = 0;
    for j in 0..v.len() {
        if pred(v[j]) {
            v.swap(i, j);
            i += 1;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bvh_matches_brute_force() {
        let mut tris = Vec::new();
        let mut s = 12345u32;
        let mut r = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 10.0 - 5.0
        };
        for _ in 0..500 {
            let a = Vec3::new(r(), r(), r());
            tris.push(TriGeom::new(a, a + Vec3::new(r(), r(), r()) * 0.1, a + Vec3::new(r(), r(), r()) * 0.1));
        }
        let (bvh, order) = Bvh::build(tris.clone());
        for _ in 0..500 {
            let o = Vec3::new(r(), r(), r()) * 2.0;
            let d = Vec3::new(r(), r(), r()).normalize();
            let brute = tris.iter().enumerate().filter_map(|(i, t)| t.intersect(o, d, f32::INFINITY).map(|h| (h.0, i))).min_by(|a, b| a.0.total_cmp(&b.0));
            let h = bvh.intersect(o, d, f32::INFINITY);
            assert_eq!(brute.map(|b| b.1 as u32), h.map(|h| order[h.prim as usize]));
            assert_eq!(brute.is_some(), bvh.occluded(o, d, f32::INFINITY));
        }
    }
}
