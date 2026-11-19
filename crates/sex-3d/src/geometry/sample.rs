// SPDX-License-Identifier: GPL-3.0-only
//! Spatial queries for the sampling nodes: nearest point (a k-d tree),
//! nearest surface point and ray casts on a mesh (the modifier BVH), with
//! barycentric weights for interpolating corner, point and face values.
use super::attributes::{interp, AttrType, Domain, Val};
use super::topo::{adapt_mesh, MeshTopo};
use crate::math::Vec3;
use crate::mesh::{Mesh, Tri};
use crate::modifiers::bvh::TriBvh;

/// Nearest-neighbour search over points.
pub struct KdTree {
    pts: Vec<(Vec3, usize)>,
}

impl KdTree {
    pub fn new(points: &[Vec3]) -> KdTree {
        let mut pts: Vec<(Vec3, usize)> = points.iter().copied().zip(0..).collect();
        build(&mut pts, 0);
        KdTree { pts }
    }

    /// Index and squared distance of the closest point.
    pub fn nearest(&self, p: Vec3) -> Option<(usize, f32)> {
        let mut best = (usize::MAX, f32::INFINITY);
        search(&self.pts, 0, p, &mut best, None);
        (best.0 != usize::MAX).then_some(best)
    }

    /// Every point within `r` of `p`.
    pub fn within(&self, p: Vec3, r: f32, out: &mut Vec<usize>) {
        range(&self.pts, 0, p, r * r, out);
    }
}

fn build(pts: &mut [(Vec3, usize)], depth: usize) {
    if pts.len() <= 1 {
        return;
    }
    let axis = depth % 3;
    let mid = pts.len() / 2;
    pts.select_nth_unstable_by(mid, |a, b| a.0[axis].total_cmp(&b.0[axis]));
    let (l, r) = pts.split_at_mut(mid);
    build(l, depth + 1);
    build(&mut r[1..], depth + 1);
}

fn search(pts: &[(Vec3, usize)], depth: usize, p: Vec3, best: &mut (usize, f32), skip: Option<usize>) {
    if pts.is_empty() {
        return;
    }
    let mid = pts.len() / 2;
    let (q, i) = pts[mid];
    let d = (q - p).length_squared();
    if Some(i) != skip && (d < best.1 || (d == best.1 && i < best.0)) {
        *best = (i, d);
    }
    let axis = depth % 3;
    let diff = p[axis] - q[axis];
    let (near, far) = if diff < 0.0 { (&pts[..mid], &pts[mid + 1..]) } else { (&pts[mid + 1..], &pts[..mid]) };
    search(near, depth + 1, p, best, skip);
    if diff * diff <= best.1 {
        search(far, depth + 1, p, best, skip);
    }
}

fn range(pts: &[(Vec3, usize)], depth: usize, p: Vec3, r2: f32, out: &mut Vec<usize>) {
    if pts.is_empty() {
        return;
    }
    let mid = pts.len() / 2;
    let (q, i) = pts[mid];
    if (q - p).length_squared() <= r2 {
        out.push(i);
    }
    let axis = depth % 3;
    let diff = p[axis] - q[axis];
    if diff <= 0.0 || diff * diff <= r2 {
        range(&pts[..mid], depth + 1, p, r2, out);
    }
    if diff >= 0.0 || diff * diff <= r2 {
        range(&pts[mid + 1..], depth + 1, p, r2, out);
    }
}

/// A mesh prepared for surface queries.
pub struct Surface<'m> {
    pub mesh: &'m Mesh,
    pub topo: MeshTopo,
    pub tris: Vec<Tri>,
    bvh: TriBvh,
}

/// A point on a surface: triangle, barycentric weights and the hit data.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceHit {
    pub tri: usize,
    pub bary: [f32; 3],
    pub point: Vec3,
    pub normal: Vec3,
    pub dist: f32,
}

pub fn barycentric(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> [f32; 3] {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-20 {
        return [1.0, 0.0, 0.0];
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    [1.0 - v - w, v, w]
}

impl<'m> Surface<'m> {
    pub fn new(mesh: &'m Mesh) -> Surface<'m> {
        Surface { mesh, topo: MeshTopo::new(mesh), tris: mesh.triangulate(), bvh: TriBvh::new(mesh) }
    }

    fn hit(&self, h: crate::modifiers::bvh::Hit) -> SurfaceHit {
        let t = &self.tris[h.tri];
        let p = t.verts.map(|v| self.mesh.positions[v as usize]);
        SurfaceHit { tri: h.tri, bary: barycentric(h.point, p[0], p[1], p[2]), point: h.point, normal: h.normal, dist: h.dist }
    }

    pub fn nearest(&self, p: Vec3) -> Option<SurfaceHit> {
        self.bvh.nearest(p).map(|h| self.hit(h))
    }

    pub fn ray(&self, o: Vec3, d: Vec3, max: f32) -> Option<SurfaceHit> {
        let len = d.length();
        if len == 0.0 {
            return None;
        }
        self.bvh.ray(o, d / len, max).map(|h| self.hit(h))
    }

    /// The face corner of triangle corner `k`.
    pub fn corner(&self, tri: usize, k: usize) -> usize {
        let t = &self.tris[tri];
        self.topo.face_offsets[t.face as usize] + t.corners[k] as usize
    }

    /// Interpolate values on `domain` at a hit (Blender's "interpolated"
    /// mapping: barycentric on corners, the face's value on faces).
    pub fn interpolate(&self, vals: &[Val], domain: Domain, h: &SurfaceHit) -> Val {
        let def = vals.first().map(|v| AttrType::of_val(v).default_val()).unwrap_or(Val::Float(0.0));
        let t = &self.tris[h.tri];
        match domain {
            Domain::Face => vals.get(t.face as usize).copied().unwrap_or(def),
            Domain::Point => {
                let w: Vec<(Val, f32)> = (0..3).map(|k| (vals.get(t.verts[k] as usize).copied().unwrap_or(def), h.bary[k])).collect();
                AttrType::of_val(&def).convert(interp(&w))
            }
            Domain::Corner => {
                let w: Vec<(Val, f32)> = (0..3).map(|k| (vals.get(self.corner(h.tri, k)).copied().unwrap_or(def), h.bary[k])).collect();
                AttrType::of_val(&def).convert(interp(&w))
            }
            _ => {
                let c = adapt_mesh(self.mesh, &self.topo, vals, domain, Domain::Corner);
                self.interpolate(&c, Domain::Corner, h)
            }
        }
    }

    /// The value at the hit's nearest element ("nearest" mapping).
    pub fn nearest_value(&self, vals: &[Val], domain: Domain, h: &SurfaceHit) -> Val {
        let def = vals.first().map(|v| AttrType::of_val(v).default_val()).unwrap_or(Val::Float(0.0));
        let t = &self.tris[h.tri];
        let k = (0..3).max_by(|&a, &b| h.bary[a].total_cmp(&h.bary[b])).unwrap_or(0);
        let i = match domain {
            Domain::Face => t.face as usize,
            Domain::Point => t.verts[k] as usize,
            Domain::Corner => self.corner(h.tri, k),
            Domain::Edge => {
                let (a, b) = (t.verts[k], t.verts[(k + 1) % 3]);
                self.topo.edge_index(a, b).unwrap_or(0) as usize
            }
            _ => 0,
        };
        vals.get(i).copied().unwrap_or(def)
    }
}

/// Closest point on segment `ab` to `p`.
pub fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let d = b - a;
    let l = d.length_squared();
    if l == 0.0 {
        return a;
    }
    a + d * ((p - a).dot(d) / l).clamp(0.0, 1.0)
}
