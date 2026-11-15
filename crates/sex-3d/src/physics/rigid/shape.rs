// SPDX-License-Identifier: GPL-3.0-only
//! Collision shapes: convex polytopes (Box, Convex Hull, and Cylinder and
//! Cone as 16-sided prisms and pyramids), spheres, capsules and static
//! triangle meshes, their mass properties and a 3D convex hull.
use crate::math::{Aabb, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

/// A face of a polytope: a convex polygon, counter-clockwise seen from
/// outside, with its outward plane.
#[derive(Clone, Debug)]
pub struct PolyFace {
    pub verts: Vec<usize>,
    pub normal: Vec3,
    pub d: f32,
}

/// An edge with its two adjacent faces (for the Gauss map test).
#[derive(Clone, Copy, Debug)]
pub struct PolyEdge {
    pub a: usize,
    pub b: usize,
    pub f0: usize,
    pub f1: usize,
}

/// A convex polyhedron in body space.
#[derive(Clone, Debug)]
pub struct Polytope {
    pub verts: Vec<Vec3>,
    pub faces: Vec<PolyFace>,
    pub edges: Vec<PolyEdge>,
    pub center: Vec3,
}

impl Polytope {
    /// Build from points: their convex hull with coplanar triangles merged
    /// into polygons.
    pub fn hull(points: &[Vec3]) -> Option<Polytope> {
        let tris = convex_hull(points)?;
        let mut used: Vec<usize> = tris.iter().flat_map(|t| t.iter().copied()).collect();
        used.sort_unstable();
        used.dedup();
        let remap: HashMap<usize, usize> = used.iter().enumerate().map(|(i, &v)| (v, i)).collect();
        let verts: Vec<Vec3> = used.iter().map(|&v| points[v]).collect();
        let tris: Vec<[usize; 3]> = tris.iter().map(|t| t.map(|v| remap[&v])).collect();
        // Group triangles by plane.
        let scale = verts.iter().fold(0.0f32, |m, v| m.max(v.abs().max_elem())).max(1e-6);
        let mut planes: Vec<(Vec3, f32)> = Vec::new();
        let mut tri_plane = Vec::with_capacity(tris.len());
        for t in &tris {
            let n = (verts[t[1]] - verts[t[0]]).cross(verts[t[2]] - verts[t[0]]).normalize();
            let d = n.dot(verts[t[0]]);
            let found = planes.iter().position(|(pn, pd)| pn.dot(n) > 1.0 - 1e-4 && (pd - d).abs() < 1e-4 * scale);
            tri_plane.push(match found {
                Some(i) => i,
                None => {
                    planes.push((n, d));
                    planes.len() - 1
                }
            });
        }
        let mut faces = Vec::with_capacity(planes.len());
        for (pi, &(n, d)) in planes.iter().enumerate() {
            let mut vs: Vec<usize> = tris.iter().zip(&tri_plane).filter(|(_, &p)| p == pi).flat_map(|(t, _)| t.iter().copied()).collect();
            vs.sort_unstable();
            vs.dedup();
            let c = vs.iter().fold(Vec3::ZERO, |a, &v| a + verts[v]) / vs.len() as f32;
            let u = (verts[vs[0]] - c).normalize();
            let w = n.cross(u);
            vs.sort_by(|&a, &b| {
                let pa = verts[a] - c;
                let pb = verts[b] - c;
                let aa = pa.dot(w).atan2(pa.dot(u));
                let ab = pb.dot(w).atan2(pb.dot(u));
                aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
            });
            // Drop collinear points on the boundary.
            let mut k = 0;
            while vs.len() > 3 && k < vs.len() {
                let p = verts[vs[(k + vs.len() - 1) % vs.len()]];
                let q = verts[vs[k]];
                let r = verts[vs[(k + 1) % vs.len()]];
                if (q - p).cross(r - q).length() < 1e-6 * scale * scale {
                    vs.remove(k);
                } else {
                    k += 1;
                }
            }
            faces.push(PolyFace { verts: vs, normal: n, d });
        }
        Some(Polytope::from_faces(verts, faces))
    }

    fn from_faces(verts: Vec<Vec3>, faces: Vec<PolyFace>) -> Polytope {
        let mut emap: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        for (fi, f) in faces.iter().enumerate() {
            for k in 0..f.verts.len() {
                let a = f.verts[k];
                let b = f.verts[(k + 1) % f.verts.len()];
                let key = (a.min(b), a.max(b));
                emap.entry(key).and_modify(|e| e.1 = fi).or_insert((fi, usize::MAX));
            }
        }
        let mut edges: Vec<PolyEdge> = emap.into_iter().filter(|(_, (_, f1))| *f1 != usize::MAX).map(|((a, b), (f0, f1))| PolyEdge { a, b, f0, f1 }).collect();
        edges.sort_by_key(|e| (e.a, e.b));
        let center = verts.iter().fold(Vec3::ZERO, |a, &v| a + v) / verts.len().max(1) as f32;
        Polytope { verts, faces, edges, center }
    }

    /// An axis-aligned box with half extents `h` around the origin.
    pub fn cuboid(h: Vec3) -> Polytope {
        let h = h.max(Vec3::splat(1e-4));
        let mut pts = Vec::new();
        for i in 0..8 {
            pts.push(Vec3::new(if i & 1 == 0 { -h.x } else { h.x }, if i & 2 == 0 { -h.y } else { h.y }, if i & 4 == 0 { -h.z } else { h.z }));
        }
        Polytope::hull(&pts).expect("box hull")
    }

    /// A prism (`top` = radius at +z) or pyramid (`top` = 0) with `n` sides,
    /// radius `r` at -z and half height `h`.
    pub fn round(r: f32, top: f32, h: f32, n: usize) -> Polytope {
        let mut pts = Vec::new();
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            pts.push(Vec3::new(r * a.cos(), r * a.sin(), -h));
            if top > 0.0 {
                pts.push(Vec3::new(top * a.cos(), top * a.sin(), h));
            }
        }
        if top <= 0.0 {
            pts.push(Vec3::new(0.0, 0.0, h));
        }
        Polytope::hull(&pts).unwrap_or_else(|| Polytope::cuboid(Vec3::new(r, r, h)))
    }

    pub fn support(&self, d: Vec3) -> Vec3 {
        let mut best = self.verts[0];
        let mut bd = best.dot(d);
        for &v in &self.verts[1..] {
            let x = v.dot(d);
            if x > bd {
                bd = x;
                best = v;
            }
        }
        best
    }

    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for &v in &self.verts {
            b.grow(v);
        }
        b
    }
}

/// A static triangle mesh with an AABB tree.
#[derive(Clone, Debug)]
pub struct TriMesh {
    pub verts: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    nodes: Vec<TreeNode>,
    order: Vec<u32>,
}

#[derive(Clone, Debug)]
struct TreeNode {
    b: Aabb,
    /// Leaf: (first, count) into `order`; inner: (left child, 0) and the
    /// right child follows the left subtree.
    first: u32,
    count: u32,
    right: u32,
}

impl TriMesh {
    pub fn new(verts: Vec<Vec3>, tris: Vec<[u32; 3]>) -> TriMesh {
        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let boxes: Vec<Aabb> = tris
            .iter()
            .map(|t| {
                let mut b = Aabb::EMPTY;
                for &v in t {
                    b.grow(verts[v as usize]);
                }
                b
            })
            .collect();
        let mut nodes = Vec::new();
        if !tris.is_empty() {
            build(&mut nodes, &boxes, &mut order, 0, tris.len());
        }
        TriMesh { verts, tris, nodes, order }
    }

    pub fn tri(&self, i: usize) -> [Vec3; 3] {
        self.tris[i].map(|v| self.verts[v as usize])
    }

    /// Triangles whose bounds overlap `q`.
    pub fn query(&self, q: &Aabb, out: &mut Vec<usize>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let nd = &self.nodes[n];
            if !overlap(&nd.b, q) {
                continue;
            }
            if nd.count > 0 {
                for &t in &self.order[nd.first as usize..(nd.first + nd.count) as usize] {
                    out.push(t as usize);
                }
            } else {
                stack.push(n + 1);
                stack.push(nd.right as usize);
            }
        }
    }

    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map_or(Aabb::EMPTY, |n| n.b)
    }
}

pub fn overlap(a: &Aabb, b: &Aabb) -> bool {
    a.min.x <= b.max.x && a.max.x >= b.min.x && a.min.y <= b.max.y && a.max.y >= b.min.y && a.min.z <= b.max.z && a.max.z >= b.min.z
}

fn build(nodes: &mut Vec<TreeNode>, boxes: &[Aabb], order: &mut [u32], first: usize, end: usize) -> usize {
    let mut b = Aabb::EMPTY;
    for &i in &order[first..end] {
        b = b.union(boxes[i as usize]);
    }
    let me = nodes.len();
    nodes.push(TreeNode { b, first: first as u32, count: (end - first) as u32, right: 0 });
    if end - first <= 4 {
        return me;
    }
    let size = b.size();
    let axis = if size.x >= size.y && size.x >= size.z {
        0
    } else if size.y >= size.z {
        1
    } else {
        2
    };
    order[first..end].sort_by(|&a, &c| boxes[a as usize].center()[axis].partial_cmp(&boxes[c as usize].center()[axis]).unwrap_or(std::cmp::Ordering::Equal));
    let mid = (first + end) / 2;
    nodes[me].count = 0;
    build(nodes, boxes, order, first, mid);
    let r = build(nodes, boxes, order, mid, end);
    nodes[me].right = r as u32;
    me
}

/// A collision shape in body space.
#[derive(Clone, Debug)]
pub enum Shape {
    Poly(Arc<Polytope>),
    Sphere(f32),
    /// Along body Z: radius and half length of the core segment.
    Capsule(f32, f32),
    Mesh(Arc<TriMesh>),
}

impl Shape {
    pub fn local_bounds(&self) -> Aabb {
        match self {
            Shape::Poly(p) => p.bounds(),
            Shape::Sphere(r) => Aabb { min: Vec3::splat(-r), max: Vec3::splat(*r) },
            Shape::Capsule(r, h) => Aabb { min: Vec3::new(-r, -r, -r - h), max: Vec3::new(*r, *r, r + h) },
            Shape::Mesh(m) => m.bounds(),
        }
    }

    /// Diagonal inertia tensor for mass `m` (approximating hulls and meshes
    /// by their bounds, cones by cylinders as Bullet's shapes do closely).
    pub fn inertia(&self, m: f32) -> Vec3 {
        match self {
            Shape::Sphere(r) => Vec3::splat(0.4 * m * r * r),
            Shape::Capsule(r, h) => {
                let l = 2.0 * h + 2.0 * r;
                let side = m * (3.0 * r * r + l * l) / 12.0;
                Vec3::new(side, side, 0.5 * m * r * r)
            }
            _ => {
                let s = self.local_bounds().size();
                Vec3::new(m / 12.0 * (s.y * s.y + s.z * s.z), m / 12.0 * (s.x * s.x + s.z * s.z), m / 12.0 * (s.x * s.x + s.y * s.y))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Convex hull

/// Triangles of the convex hull of `pts` (indices into `pts`, outward
/// counter-clockwise), or `None` when the points are flat.
pub fn convex_hull(pts: &[Vec3]) -> Option<Vec<[usize; 3]>> {
    let n = pts.len();
    if n < 4 {
        return None;
    }
    let mut b = Aabb::EMPTY;
    for &p in pts {
        b.grow(p);
    }
    let scale = b.size().max_elem().max(1e-9);
    let eps = scale * 1e-6;
    // Initial tetrahedron from extreme points.
    let i0 = (0..n).min_by(|&a, &c| pts[a].x.partial_cmp(&pts[c].x).unwrap()).unwrap();
    let i1 = (0..n).max_by(|&a, &c| pts[a].distance(pts[i0]).partial_cmp(&pts[c].distance(pts[i0])).unwrap())?;
    let line = pts[i1] - pts[i0];
    if line.length() < eps {
        return None;
    }
    let i2 = (0..n).max_by(|&a, &c| line.cross(pts[a] - pts[i0]).length().partial_cmp(&line.cross(pts[c] - pts[i0]).length()).unwrap())?;
    let nrm = line.cross(pts[i2] - pts[i0]);
    if nrm.length() < eps * scale {
        return None;
    }
    let i3 = (0..n).max_by(|&a, &c| nrm.dot(pts[a] - pts[i0]).abs().partial_cmp(&nrm.dot(pts[c] - pts[i0]).abs()).unwrap())?;
    if nrm.normalize().dot(pts[i3] - pts[i0]).abs() < eps {
        return None;
    }
    let mut faces: Vec<[usize; 3]> = Vec::new();
    let centroid = (pts[i0] + pts[i1] + pts[i2] + pts[i3]) * 0.25;
    let add = |faces: &mut Vec<[usize; 3]>, a: usize, b: usize, c: usize| {
        let n = (pts[b] - pts[a]).cross(pts[c] - pts[a]);
        if n.dot(pts[a] - centroid) < 0.0 {
            faces.push([a, c, b]);
        } else {
            faces.push([a, b, c]);
        }
    };
    add(&mut faces, i0, i1, i2);
    add(&mut faces, i0, i1, i3);
    add(&mut faces, i0, i2, i3);
    add(&mut faces, i1, i2, i3);
    let plane = |f: &[usize; 3]| {
        let nn = (pts[f[1]] - pts[f[0]]).cross(pts[f[2]] - pts[f[0]]).normalize();
        (nn, nn.dot(pts[f[0]]))
    };
    let mut planes: Vec<(Vec3, f32)> = faces.iter().map(plane).collect();
    // Process the farthest points first for fewer temporary faces.
    let mut order: Vec<usize> = (0..n).filter(|&i| i != i0 && i != i1 && i != i2 && i != i3).collect();
    order.sort_by(|&a, &c| pts[c].distance(centroid).partial_cmp(&pts[a].distance(centroid)).unwrap());
    for p in order {
        let q = pts[p];
        let visible: Vec<bool> = planes.iter().map(|(nn, d)| nn.dot(q) - d > eps).collect();
        if !visible.iter().any(|&v| v) {
            continue;
        }
        let mut vis_edges: HashMap<(usize, usize), u8> = HashMap::new();
        for (f, &v) in faces.iter().zip(&visible) {
            if v {
                for k in 0..3 {
                    vis_edges.insert((f[k], f[(k + 1) % 3]), 1);
                }
            }
        }
        let mut horizon = Vec::new();
        for (&(a, b), _) in vis_edges.iter() {
            if !vis_edges.contains_key(&(b, a)) {
                horizon.push((a, b));
            }
        }
        horizon.sort_unstable();
        let mut nf = Vec::with_capacity(faces.len());
        let mut np = Vec::with_capacity(faces.len());
        for (i, f) in faces.iter().enumerate() {
            if !visible[i] {
                nf.push(*f);
                np.push(planes[i]);
            }
        }
        for (a, b) in horizon {
            let f = [a, b, p];
            nf.push(f);
            np.push(plane(&f));
        }
        faces = nf;
        planes = np;
        if faces.len() > 20000 {
            break;
        }
    }
    Some(faces)
}
