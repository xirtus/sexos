// SPDX-License-Identifier: GPL-3.0-only
//! Narrowphase: contact manifolds between shapes.
//!
//! * Polytope against polytope (boxes, hulls, cylinders and cones, and the
//!   triangles of a passive Mesh shape): the separating axis test over face
//!   normals and edge pairs (pruned on the Gauss map), then face clipping
//!   (Sutherland-Hodgman) for up to four contacts, or the closest points of
//!   the two edges.
//! * Spheres and capsules: GJK closest points between their core (a point
//!   or segment) and the other shape, inflated by the radius, with the
//!   capsule's end points as extra contacts; when the cores overlap the
//!   polytope's faces give the axis (a SAT fallback in place of EPA).
use super::shape::{PolyEdge, PolyFace, Polytope, Shape};
use crate::math::{Aabb, Mat3, Quat, Vec3};
use std::sync::OnceLock;

/// A body's placement.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
    pub m: Mat3,
}

impl Pose {
    pub fn new(pos: Vec3, rot: Quat) -> Pose {
        Pose { pos, rot, m: Mat3::from_quat(rot) }
    }
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.pos + self.m.mul_vec(p)
    }
    pub fn vector(&self, v: Vec3) -> Vec3 {
        self.m.mul_vec(v)
    }
    pub fn inv_point(&self, p: Vec3) -> Vec3 {
        self.m.transpose().mul_vec(p - self.pos)
    }
}

/// One contact: world point, normal from A to B, penetration depth (> 0
/// when overlapping) and a feature id for warm starting.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub p: Vec3,
    pub n: Vec3,
    pub depth: f32,
}

/// A polytope placed in the world.
pub struct WPoly<'a> {
    pub v: Vec<Vec3>,
    pub n: Vec<Vec3>,
    pub d: Vec<f32>,
    pub faces: &'a [PolyFace],
    pub edges: &'a [PolyEdge],
    pub c: Vec3,
    /// A single triangle (its edges have opposite face normals).
    pub flat: bool,
}

impl<'a> WPoly<'a> {
    pub fn new(p: &'a Polytope, pose: &Pose) -> WPoly<'a> {
        let v: Vec<Vec3> = p.verts.iter().map(|&x| pose.point(x)).collect();
        let n: Vec<Vec3> = p.faces.iter().map(|f| pose.vector(f.normal)).collect();
        let d = p.faces.iter().zip(&n).map(|(f, nn)| nn.dot(v[f.verts[0]])).collect();
        WPoly { v, n, d, faces: &p.faces, edges: &p.edges, c: pose.point(p.center), flat: false }
    }

    pub fn triangle(t: [Vec3; 3]) -> WPoly<'static> {
        let topo = tri_topology();
        let nn = (t[1] - t[0]).cross(t[2] - t[0]).normalize();
        let n = vec![nn, -nn];
        let d = vec![nn.dot(t[0]), -nn.dot(t[0])];
        WPoly { v: t.to_vec(), n, d, faces: &topo.0, edges: &topo.1, c: (t[0] + t[1] + t[2]) / 3.0, flat: true }
    }

    fn support(&self, dir: Vec3) -> Vec3 {
        let mut best = self.v[0];
        let mut bd = best.dot(dir);
        for &x in &self.v[1..] {
            let t = x.dot(dir);
            if t > bd {
                bd = t;
                best = x;
            }
        }
        best
    }

    fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for &x in &self.v {
            b.grow(x);
        }
        b
    }
}

fn tri_topology() -> &'static (Vec<PolyFace>, Vec<PolyEdge>) {
    static T: OnceLock<(Vec<PolyFace>, Vec<PolyEdge>)> = OnceLock::new();
    T.get_or_init(|| {
        let faces = vec![PolyFace { verts: vec![0, 1, 2], normal: Vec3::Z, d: 0.0 }, PolyFace { verts: vec![0, 2, 1], normal: -Vec3::Z, d: 0.0 }];
        let edges = vec![PolyEdge { a: 0, b: 1, f0: 0, f1: 1 }, PolyEdge { a: 1, b: 2, f0: 0, f1: 1 }, PolyEdge { a: 0, b: 2, f0: 0, f1: 1 }];
        (faces, edges)
    })
}

fn query_faces(a: &WPoly, b: &WPoly) -> (f32, usize) {
    let mut best = (f32::NEG_INFINITY, 0);
    for i in 0..a.n.len() {
        let s = a.n[i].dot(b.support(-a.n[i])) - a.d[i];
        if s > best.0 {
            best = (s, i);
        }
    }
    best
}

fn is_minkowski_face(a: Vec3, b: Vec3, bxa: Vec3, c: Vec3, d: Vec3, dxc: Vec3) -> bool {
    let cba = c.dot(bxa);
    let dba = d.dot(bxa);
    let adc = a.dot(dxc);
    let bdc = b.dot(dxc);
    cba * dba < 0.0 && adc * bdc < 0.0 && cba * bdc > 0.0
}

/// Best edge-edge axis: (separation, edge of A, edge of B, axis A->B).
fn query_edges(a: &WPoly, b: &WPoly) -> (f32, usize, usize, Vec3) {
    let mut best = (f32::NEG_INFINITY, 0, 0, Vec3::Z);
    let exact = a.flat || b.flat;
    for (ia, ea) in a.edges.iter().enumerate() {
        let pa = a.v[ea.a];
        let da = a.v[ea.b] - pa;
        let (u1, u2) = (a.n[ea.f0], a.n[ea.f1]);
        for (ib, eb) in b.edges.iter().enumerate() {
            let pb = b.v[eb.a];
            let db = b.v[eb.b] - pb;
            if !exact {
                let (v1, v2) = (-b.n[eb.f0], -b.n[eb.f1]);
                if !is_minkowski_face(u1, u2, u2.cross(u1), v1, v2, v2.cross(v1)) {
                    continue;
                }
            }
            let x = da.cross(db);
            let l = x.length();
            if l < 0.005 * (da.length() * db.length()) || l < 1e-12 {
                continue;
            }
            let mut nrm = x / l;
            let sep = if exact {
                if nrm.dot(b.c - a.c) < 0.0 {
                    nrm = -nrm;
                }
                b.support(-nrm).dot(nrm) - a.support(nrm).dot(nrm)
            } else {
                if nrm.dot(pa - a.c) < 0.0 {
                    nrm = -nrm;
                }
                nrm.dot(pb - pa)
            };
            if sep > best.0 {
                best = (sep, ia, ib, nrm);
            }
        }
    }
    best
}

fn closest_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= 1e-12 && e <= 1e-12 {
        return (p1, p2);
    }
    if a <= 1e-12 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let den = a * e - b * b;
            let mut ss = if den > 1e-12 { ((b * f - c * e) / den).clamp(0.0, 1.0) } else { 0.0 };
            let mut tt = (b * ss + f) / e;
            if tt < 0.0 {
                tt = 0.0;
                ss = (-c / a).clamp(0.0, 1.0);
            } else if tt > 1.0 {
                tt = 1.0;
                ss = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = ss;
            t = tt;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// Clip polygon `poly` to the half space `n.x <= d`.
fn clip(poly: &[Vec3], n: Vec3, d: f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    if poly.is_empty() {
        return out;
    }
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let da = n.dot(a) - d;
        let db = n.dot(b) - d;
        if da <= 0.0 {
            out.push(a);
        }
        if (da <= 0.0) != (db <= 0.0) {
            let t = da / (da - db);
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// Keep at most four contacts: the deepest, then the ones spanning the
/// largest area.
fn reduce(mut c: Vec<Contact>) -> Vec<Contact> {
    if c.len() <= 4 {
        return c;
    }
    c.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));
    let a = c[0];
    let i1 = (1..c.len()).max_by(|&i, &j| c[i].p.distance(a.p).partial_cmp(&c[j].p.distance(a.p)).unwrap()).unwrap();
    let b = c[i1];
    let n = a.n;
    let area = |p: Vec3| (b.p - a.p).cross(p - a.p).dot(n);
    let i2 = (1..c.len()).filter(|&i| i != i1).max_by(|&i, &j| area(c[i].p).partial_cmp(&area(c[j].p)).unwrap()).unwrap();
    let i3 = (1..c.len()).filter(|&i| i != i1 && i != i2).min_by(|&i, &j| area(c[i].p).partial_cmp(&area(c[j].p)).unwrap()).unwrap();
    vec![a, b, c[i2], c[i3]]
}

/// Contacts between two placed polytopes (normal from A to B).
pub fn poly_poly(a: &WPoly, b: &WPoly, margin: f32, out: &mut Vec<Contact>) {
    let fa = query_faces(a, b);
    if fa.0 > margin {
        return;
    }
    let fb = query_faces(b, a);
    if fb.0 > margin {
        return;
    }
    let eq = query_edges(a, b);
    if eq.0 > margin {
        return;
    }
    let max_face = fa.0.max(fb.0);
    if eq.0 > 0.9 * max_face + 0.0025 && eq.0 > f32::NEG_INFINITY {
        let ea = a.edges[eq.1];
        let eb = b.edges[eq.2];
        let (c1, c2) = closest_segments(a.v[ea.a], a.v[ea.b], b.v[eb.a], b.v[eb.b]);
        out.push(Contact { p: (c1 + c2) * 0.5, n: eq.3, depth: -eq.0 });
        return;
    }
    let (rf, inc, ri, flip) = if fb.0 > 0.98 * fa.0 + 0.0025 { (b, a, fb.1, true) } else { (a, b, fa.1, false) };
    let n = rf.n[ri];
    let ii = (0..inc.n.len()).min_by(|&i, &j| inc.n[i].dot(n).partial_cmp(&inc.n[j].dot(n)).unwrap()).unwrap();
    let mut poly: Vec<Vec3> = inc.faces[ii].verts.iter().map(|&v| inc.v[v]).collect();
    let rv = &rf.faces[ri].verts;
    for k in 0..rv.len() {
        let p = rf.v[rv[k]];
        let q = rf.v[rv[(k + 1) % rv.len()]];
        let s = (q - p).cross(n);
        let l = s.length();
        if l < 1e-12 {
            continue;
        }
        let s = s / l;
        poly = clip(&poly, s, s.dot(p));
        if poly.is_empty() {
            break;
        }
    }
    let nn = if flip { -n } else { n };
    let mut cs = Vec::new();
    for x in poly {
        let sep = n.dot(x) - rf.d[ri];
        if sep <= margin {
            cs.push(Contact { p: x - n * (sep * 0.5), n: nn, depth: -sep });
        }
    }
    out.extend(reduce(cs));
}

// ---------------------------------------------------------------------------
// GJK

#[derive(Clone, Copy)]
struct SV {
    w: Vec3,
    a: Vec3,
    b: Vec3,
}

/// Closest point to the origin on the simplex; reduces it and returns the
/// barycentric weights of the kept vertices. `None`: the origin is inside.
fn simplex_closest(s: &mut Vec<SV>) -> Option<Vec<f32>> {
    match s.len() {
        1 => Some(vec![1.0]),
        2 => {
            let (a, b) = (s[0].w, s[1].w);
            let ab = b - a;
            let t = (-a).dot(ab) / ab.dot(ab).max(1e-30);
            if t <= 0.0 {
                s.truncate(1);
                Some(vec![1.0])
            } else if t >= 1.0 {
                s.remove(0);
                Some(vec![1.0])
            } else {
                Some(vec![1.0 - t, t])
            }
        }
        3 => {
            let (bc, keep) = tri_closest(s[0].w, s[1].w, s[2].w);
            let kept: Vec<SV> = keep.iter().map(|&i| s[i]).collect();
            let w: Vec<f32> = keep.iter().map(|&i| bc[i]).collect();
            *s = kept;
            Some(w)
        }
        _ => {
            // Tetrahedron: origin inside, or the closest face.
            let o = Vec3::ZERO;
            let pts = [s[0].w, s[1].w, s[2].w, s[3].w];
            let faces = [[0, 1, 2, 3], [0, 3, 1, 2], [0, 2, 3, 1], [1, 3, 2, 0]];
            let mut best: Option<(f32, Vec<usize>, Vec<f32>)> = None;
            let mut inside = true;
            for f in faces {
                let (a, b, c, d) = (pts[f[0]], pts[f[1]], pts[f[2]], pts[f[3]]);
                let n = (b - a).cross(c - a);
                let so = n.dot(o - a);
                let sd = n.dot(d - a);
                if so * sd < 0.0 {
                    inside = false;
                    let (bc, keep) = tri_closest(a, b, c);
                    let p = a * bc[0] + b * bc[1] + c * bc[2];
                    let dist = p.length_squared();
                    if best.as_ref().is_none_or(|x| dist < x.0) {
                        let idx = [f[0], f[1], f[2]];
                        best = Some((dist, keep.iter().map(|&i| idx[i]).collect(), keep.iter().map(|&i| bc[i]).collect()));
                    }
                }
            }
            if inside {
                return None;
            }
            let (_, keep, w) = best?;
            let kept: Vec<SV> = keep.iter().map(|&i| s[i]).collect();
            *s = kept;
            Some(w)
        }
    }
}

/// Barycentric coordinates of the point of triangle abc closest to the
/// origin (Ericson), and the vertices with non-zero weight.
fn tri_closest(a: Vec3, b: Vec3, c: Vec3) -> ([f32; 3], Vec<usize>) {
    let p = Vec3::ZERO;
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ([1.0, 0.0, 0.0], vec![0]);
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return ([0.0, 1.0, 0.0], vec![1]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return ([1.0 - v, v, 0.0], vec![0, 1]);
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return ([0.0, 0.0, 1.0], vec![2]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return ([1.0 - w, 0.0, w], vec![0, 2]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return ([0.0, 1.0 - w, w], vec![1, 2]);
    }
    let den = 1.0 / (va + vb + vc);
    let v = vb * den;
    let w = vc * den;
    ([1.0 - v - w, v, w], vec![0, 1, 2])
}

/// Closest points between two convex sets given by support functions;
/// `None` when they overlap.
pub fn gjk(sa: &dyn Fn(Vec3) -> Vec3, sb: &dyn Fn(Vec3) -> Vec3, guess: Vec3) -> Option<(Vec3, Vec3)> {
    let d0 = if guess.length_squared() > 1e-12 { guess } else { Vec3::X };
    let a0 = sa(-d0);
    let b0 = sb(d0);
    let mut s = vec![SV { w: a0 - b0, a: a0, b: b0 }];
    let mut lambda = vec![1.0f32];
    let mut v = s[0].w;
    for _ in 0..64 {
        let vv = v.length_squared();
        if vv < 1e-14 {
            return None;
        }
        let a = sa(-v);
        let b = sb(v);
        let w = a - b;
        if vv - v.dot(w) <= 1e-6 * vv.max(1e-6) || s.iter().any(|x| (x.w - w).length_squared() < 1e-14) {
            break;
        }
        s.push(SV { w, a, b });
        lambda = simplex_closest(&mut s)?;
        v = s.iter().zip(&lambda).fold(Vec3::ZERO, |acc, (x, &l)| acc + x.w * l);
        if s.len() == 4 {
            return None;
        }
    }
    let pa = s.iter().zip(&lambda).fold(Vec3::ZERO, |acc, (x, &l)| acc + x.a * l);
    let pb = s.iter().zip(&lambda).fold(Vec3::ZERO, |acc, (x, &l)| acc + x.b * l);
    Some((pa, pb))
}

// ---------------------------------------------------------------------------
// Rounded shapes

/// The core of a rounded shape: segment end points (equal for a sphere)
/// and radius.
fn core(shape: &Shape, pose: &Pose) -> Option<(Vec3, Vec3, f32)> {
    match *shape {
        Shape::Sphere(r) => Some((pose.pos, pose.pos, r)),
        Shape::Capsule(r, h) => Some((pose.point(Vec3::new(0.0, 0.0, -h)), pose.point(Vec3::new(0.0, 0.0, h)), r)),
        _ => None,
    }
}

fn seg_support(p: Vec3, q: Vec3) -> impl Fn(Vec3) -> Vec3 {
    move |d: Vec3| if q.dot(d) > p.dot(d) { q } else { p }
}

enum Other<'a> {
    Poly(&'a WPoly<'a>),
    Core(Vec3, Vec3, f32),
}

/// Contacts of a rounded shape (core `p..q`, radius `r`) against `other`,
/// normal from the rounded shape to the other.
fn rounded(p: Vec3, q: Vec3, r: f32, other: &Other, margin: f32, out: &mut Vec<Contact>) {
    let (ro, so): (f32, Box<dyn Fn(Vec3) -> Vec3 + '_>) = match other {
        Other::Poly(w) => (0.0, Box::new(move |d| w.support(d))),
        Other::Core(a, b, rr) => (*rr, Box::new(seg_support(*a, *b))),
    };
    let mut pts = vec![(p, q)];
    if p.distance(q) > 1e-6 {
        pts.push((p, p));
        pts.push((q, q));
    }
    let mut found: Vec<Contact> = Vec::new();
    for (k, &(a, b)) in pts.iter().enumerate() {
        let sa = seg_support(a, b);
        let guess = (a + b) * 0.5 - match other {
            Other::Poly(w) => w.c,
            Other::Core(x, y, _) => (*x + *y) * 0.5,
        };
        match gjk(&sa, &*so, guess) {
            Some((pa, pb)) => {
                let d = pa.distance(pb);
                if d < r + ro + margin && d > 1e-9 {
                    let n = (pb - pa) / d;
                    let depth = r + ro - d;
                    let c = Contact { p: (pa + n * r + pb - n * ro) * 0.5, n, depth };
                    if found.iter().all(|f| f.p.distance(c.p) > 0.01 * (r + 0.01)) {
                        found.push(c);
                    }
                }
            }
            None if k == 0 => {
                // Deep: the cores overlap.
                match other {
                    Other::Poly(w) => {
                        let mut best = (f32::NEG_INFINITY, 0usize);
                        for i in 0..w.n.len() {
                            let s = w.n[i].dot(sa(-w.n[i])) - w.d[i];
                            if s > best.0 {
                                best = (s, i);
                            }
                        }
                        let n = -w.n[best.1];
                        let deep = sa(w.n[best.1] * -1.0);
                        found.push(Contact { p: deep, n, depth: r - best.0 });
                    }
                    Other::Core(..) => found.push(Contact { p: a, n: Vec3::Z, depth: r + ro }),
                }
                break;
            }
            None => {}
        }
    }
    out.extend(found);
}

/// Contacts between shape `a` at `pa` and shape `b` at `pb`, normal from A to
/// B. `wa` / `wb`: the placed polytopes, when the shapes are polytopes.
pub fn collide(a: &Shape, pa: &Pose, wa: Option<&WPoly>, b: &Shape, pb: &Pose, wb: Option<&WPoly>, margin: f32, out: &mut Vec<Contact>) {
    match (a, b) {
        (Shape::Mesh(_), Shape::Mesh(_)) => {}
        (Shape::Mesh(m), _) => {
            let start = out.len();
            collide_mesh(m, pa, b, pb, wb, margin, out);
            for c in &mut out[start..] {
                c.n = -c.n;
            }
        }
        (_, Shape::Mesh(m)) => {
            let mut tmp = Vec::new();
            collide_mesh(m, pb, a, pa, wa, margin, &mut tmp);
            out.extend(tmp);
        }
        _ => {
            if let Some((p, q, r)) = core(a, pa) {
                let other = match core(b, pb) {
                    Some((x, y, rr)) => Other::Core(x, y, rr),
                    None => Other::Poly(wb.expect("placed polytope")),
                };
                rounded(p, q, r, &other, margin, out);
            } else if let Some((p, q, r)) = core(b, pb) {
                let start = out.len();
                rounded(p, q, r, &Other::Poly(wa.expect("placed polytope")), margin, out);
                for c in &mut out[start..] {
                    c.n = -c.n;
                }
            } else if let (Some(x), Some(y)) = (wa, wb) {
                poly_poly(x, y, margin, out);
            }
        }
    }
}

/// Contacts of shape `o` against the triangles of a static mesh, normal
/// from the mesh to `o`.
fn collide_mesh(m: &super::shape::TriMesh, pm: &Pose, o: &Shape, po: &Pose, wo: Option<&WPoly>, margin: f32, out: &mut Vec<Contact>) {
    // The other shape's bounds in the mesh's space.
    let wb = match wo {
        Some(w) => w.bounds(),
        None => {
            let lb = o.local_bounds();
            let mut b = Aabb::EMPTY;
            for i in 0..8 {
                b.grow(po.point(Vec3::new(if i & 1 == 0 { lb.min.x } else { lb.max.x }, if i & 2 == 0 { lb.min.y } else { lb.max.y }, if i & 4 == 0 { lb.min.z } else { lb.max.z })));
            }
            b
        }
    };
    let mut lb = Aabb::EMPTY;
    for i in 0..8 {
        lb.grow(pm.inv_point(Vec3::new(if i & 1 == 0 { wb.min.x } else { wb.max.x }, if i & 2 == 0 { wb.min.y } else { wb.max.y }, if i & 4 == 0 { wb.min.z } else { wb.max.z })));
    }
    lb.min = lb.min - Vec3::splat(margin);
    lb.max = lb.max + Vec3::splat(margin);
    let mut tris = Vec::new();
    m.query(&lb, &mut tris);
    let mut all = Vec::new();
    for t in tris {
        let w = m.tri(t).map(|p| pm.point(p));
        if (w[1] - w[0]).cross(w[2] - w[0]).length_squared() < 1e-16 {
            continue;
        }
        let tp = WPoly::triangle(w);
        let mut cs = Vec::new();
        match core(o, po) {
            Some((p, q, r)) => {
                rounded(p, q, r, &Other::Poly(&tp), margin, &mut cs);
                for c in &mut cs {
                    c.n = -c.n;
                }
            }
            None => {
                if let Some(wo) = wo {
                    poly_poly(&tp, wo, margin, &mut cs);
                }
            }
        }
        all.extend(cs);
    }
    // Several triangles: keep a manifold of at most eight points.
    if all.len() > 8 {
        all.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));
        let mut keep: Vec<Contact> = Vec::new();
        for c in all {
            if keep.iter().all(|k| k.p.distance(c.p) > 1e-3) {
                keep.push(c);
            }
            if keep.len() == 8 {
                break;
            }
        }
        out.extend(keep);
    } else {
        out.extend(all);
    }
}
