// SPDX-License-Identifier: GPL-3.0-only
//! A small triangle BVH for Shrinkwrap: nearest surface point and ray casts.
use crate::math::Vec3;
use crate::mesh::Mesh;

const LEAF: usize = 4;

#[derive(Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle and count. Inner: left child at `start`, right at
    /// `start + 1` (`count` 0).
    start: u32,
    count: u32,
}

pub struct TriBvh {
    tris: Vec<[Vec3; 3]>,
    nodes: Vec<Node>,
}

/// A surface hit: point, unit face normal and distance.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
    pub dist: f32,
}

impl TriBvh {
    pub fn new(m: &Mesh) -> TriBvh {
        let tris: Vec<[Vec3; 3]> = m.triangulate().iter().map(|t| t.verts.map(|v| m.positions[v as usize])).collect();
        let mut bvh = TriBvh { tris, nodes: Vec::new() };
        if !bvh.tris.is_empty() {
            let mut order: Vec<usize> = (0..bvh.tris.len()).collect();
            bvh.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, start: 0, count: 0 });
            bvh.build(0, &mut order, 0);
            bvh.tris = order.iter().map(|&i| bvh.tris[i]).collect();
        }
        bvh
    }

    fn build(&mut self, node: usize, items: &mut [usize], first: usize) {
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        let (mut clo, mut chi) = (lo, hi);
        for &i in items.iter() {
            for p in self.tris[i] {
                lo = lo.min(p);
                hi = hi.max(p);
            }
            let c = (self.tris[i][0] + self.tris[i][1] + self.tris[i][2]) / 3.0;
            clo = clo.min(c);
            chi = chi.max(c);
        }
        self.nodes[node].min = lo;
        self.nodes[node].max = hi;
        if items.len() <= LEAF {
            self.nodes[node].start = first as u32;
            self.nodes[node].count = items.len() as u32;
            return;
        }
        let ext = chi - clo;
        let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
        let key = |t: &[Vec3; 3]| t[0][axis] + t[1][axis] + t[2][axis];
        let mid = items.len() / 2;
        let tris = &self.tris;
        items.select_nth_unstable_by(mid, |&a, &b| key(&tris[a]).total_cmp(&key(&tris[b])));
        let left = self.nodes.len();
        self.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, start: 0, count: 0 });
        self.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, start: 0, count: 0 });
        self.nodes[node].start = left as u32;
        self.nodes[node].count = 0;
        let (a, b) = items.split_at_mut(mid);
        self.build(left, a, first);
        self.build(left + 1, b, first + mid);
    }

    fn box_dist2(n: &Node, p: Vec3) -> f32 {
        let d = (n.min - p).max(p - n.max).max(Vec3::ZERO);
        d.length_squared()
    }

    /// The closest point on the surface to `p`.
    pub fn nearest(&self, p: Vec3) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best = f32::INFINITY;
        let mut hit = None;
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            let n = self.nodes[ni];
            if Self::box_dist2(&n, p) >= best {
                continue;
            }
            if n.count > 0 {
                for t in &self.tris[n.start as usize..(n.start + n.count) as usize] {
                    let q = closest_on_triangle(p, t[0], t[1], t[2]);
                    let d = (q - p).length_squared();
                    if d < best {
                        best = d;
                        hit = Some((q, *t));
                    }
                }
            } else {
                let (l, r) = (n.start as usize, n.start as usize + 1);
                let (dl, dr) = (Self::box_dist2(&self.nodes[l], p), Self::box_dist2(&self.nodes[r], p));
                if dl < dr {
                    stack.push(r);
                    stack.push(l);
                } else {
                    stack.push(l);
                    stack.push(r);
                }
            }
        }
        hit.map(|(q, t)| Hit { point: q, normal: (t[1] - t[0]).cross(t[2] - t[0]).normalize(), dist: best.sqrt() })
    }

    /// The nearest hit along `o + t d` for `t` in `(0, max)`.
    pub fn ray(&self, o: Vec3, d: Vec3, max: f32) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut best = max;
        let mut hit = None;
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            let n = self.nodes[ni];
            if !slab(&n, o, inv, best) {
                continue;
            }
            if n.count > 0 {
                for t in &self.tris[n.start as usize..(n.start + n.count) as usize] {
                    if let Some(s) = ray_triangle(o, d, t) {
                        if s > 0.0 && s < best {
                            best = s;
                            hit = Some(*t);
                        }
                    }
                }
            } else {
                stack.push(n.start as usize);
                stack.push(n.start as usize + 1);
            }
        }
        hit.map(|t| Hit { point: o + d * best, normal: (t[1] - t[0]).cross(t[2] - t[0]).normalize(), dist: best })
    }
}

fn slab(n: &Node, o: Vec3, inv: Vec3, max: f32) -> bool {
    let mut t0 = 0.0f32;
    let mut t1 = max;
    for a in 0..3 {
        let (mut ta, mut tb) = ((n.min[a] - o[a]) * inv[a], (n.max[a] - o[a]) * inv[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        // NaN (0 * inf) means the ray is in the slab's plane: keep going.
        if ta.is_nan() || tb.is_nan() {
            continue;
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    t0 <= t1
}

/// Möller–Trumbore, both faces.
fn ray_triangle(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    const EPS: f32 = 1e-5;
    if !(-EPS..=1.0 + EPS).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -EPS || u + v > 1.0 + EPS {
        return None;
    }
    Some(e2.dot(q) * inv)
}

/// Closest point to `p` on triangle `abc` (Ericson, Real-Time Collision
/// Detection 5.1.5).
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1.0 / (va + vb + vc);
    if !denom.is_finite() {
        // Degenerate triangle: fall back to the nearest corner.
        return [a, b, c].into_iter().min_by(|x, y| (*x - p).length_squared().total_cmp(&(*y - p).length_squared())).unwrap();
    }
    let v = vb * denom;
    let w = vc * denom;
    a + ab * v + ac * w
}
