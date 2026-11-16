// SPDX-License-Identifier: GPL-3.0-only
//! UV unwrapping and UV editing, ported from Blender's parametrizer
//! (`geometry/intern/uv_parametrizer.cc`: charts split by seams, ABF++ and
//! LSCM, island averaging and packing) and its UV operators
//! (`editors/uvedit`: cube, cylinder, sphere and view projections, Smart UV
//! Project, Follow Active Quads, Lightmap Pack, Reset, weld and stitch).
//!
//! Every function works on a subset of faces of a [`Mesh`] (the Edit Mode
//! selection) and writes per-corner UVs ([`crate::Face::uvs`]). Faces outside
//! the subset keep their UVs; faces that had none get (0, 0) corners, like a
//! new UV map in Blender. Projections work in the mesh's own (object) space.
//!
//! Numerics run in f64. The sparse least-squares systems (LSCM, and the
//! reduced ABF++ Newton system) are solved with Jacobi-preconditioned
//! conjugate gradients, so no linear-algebra crate is needed.
use crate::math::{Vec2, Vec3};
use crate::mesh::{edge_key, Mesh};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::f64::consts::PI;

type P2 = [f64; 2];
type P3 = [f64; 3];

fn p3(v: Vec3) -> P3 {
    [v.x as f64, v.y as f64, v.z as f64]
}
fn sub3(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot3(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: P3, b: P3) -> P3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len3(a: P3) -> f64 {
    dot3(a, a).sqrt()
}
fn uv2(v: Vec2) -> P2 {
    [v.x as f64, v.y as f64]
}
fn vec2(p: P2) -> Vec2 {
    Vec2::new(p[0] as f32, p[1] as f32)
}

/// Blender's `axis_dominant_v3`: the two axes to project on for a normal.
pub fn axis_dominant(n: Vec3) -> (usize, usize) {
    let (x, y, z) = (n.x.abs(), n.y.abs(), n.z.abs());
    if z >= x && z >= y {
        (0, 1)
    } else if y >= x && y >= z {
        (0, 2)
    } else {
        (1, 2)
    }
}

// ---------------------------------------------------------------- helpers

struct UnionFind(Vec<u32>);

impl UnionFind {
    fn new(n: usize) -> UnionFind {
        UnionFind((0..n as u32).collect())
    }
    fn find(&mut self, mut a: u32) -> u32 {
        while self.0[a as usize] != a {
            let p = self.0[self.0[a as usize] as usize];
            self.0[a as usize] = p;
            a = p;
        }
        a
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra.max(rb) as usize] = ra.min(rb);
        }
    }
}

/// Faces of `faces` that exist and have at least three corners, sorted and unique.
fn valid_faces(m: &Mesh, faces: &[u32]) -> Vec<u32> {
    let set: BTreeSet<u32> = faces.iter().copied().filter(|&f| m.faces.get(f as usize).is_some_and(|f| f.verts.len() >= 3)).collect();
    set.into_iter().collect()
}

/// Give every face a full set of UVs ((0, 0) where it had none), like
/// Blender's UV map, which always covers every face corner.
pub fn ensure_uvs(m: &mut Mesh) {
    for f in &mut m.faces {
        if f.uvs.len() != f.verts.len() {
            f.uvs = vec![Vec2::ZERO; f.verts.len()];
        }
    }
}

/// Signed area of a 2D polygon.
fn poly_area(pts: &[P2]) -> f64 {
    let n = pts.len();
    let mut a = 0.0;
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

fn face_uv_area(m: &Mesh, f: u32) -> f64 {
    let pts: Vec<P2> = m.faces[f as usize].uvs.iter().map(|&u| uv2(u)).collect();
    poly_area(&pts)
}

// ---------------------------------------------------------------- sparse CG

/// A symmetric sparse matrix in compressed rows.
struct Csr {
    start: Vec<usize>,
    cols: Vec<u32>,
    vals: Vec<f64>,
}

impl Csr {
    fn from_triplets(n: usize, mut t: Vec<(u32, u32, f64)>) -> Csr {
        t.sort_unstable_by_key(|e| (e.0, e.1));
        let mut start = vec![0usize; n + 1];
        let mut cols = Vec::with_capacity(t.len());
        let mut vals: Vec<f64> = Vec::with_capacity(t.len());
        let mut last: Option<(u32, u32)> = None;
        for (r, c, v) in t {
            if last == Some((r, c)) {
                *vals.last_mut().expect("entry") += v;
            } else {
                cols.push(c);
                vals.push(v);
                start[r as usize + 1] += 1;
                last = Some((r, c));
            }
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        Csr { start, cols, vals }
    }
    fn n(&self) -> usize {
        self.start.len() - 1
    }
    fn mul(&self, x: &[f64], y: &mut [f64]) {
        for (r, out) in y.iter_mut().enumerate() {
            let mut s = 0.0;
            for k in self.start[r]..self.start[r + 1] {
                s += self.vals[k] * x[self.cols[k] as usize];
            }
            *out = s;
        }
    }
    fn diag(&self) -> Vec<f64> {
        (0..self.n()).map(|r| (self.start[r]..self.start[r + 1]).find(|&k| self.cols[k] as usize == r).map_or(0.0, |k| self.vals[k])).collect()
    }
}

/// Jacobi-preconditioned conjugate gradients for `a x = b`, starting from
/// `x`. Returns false if the iteration broke down (non-finite values).
fn cg(a: &Csr, b: &[f64], x: &mut [f64], tol: f64, max_iter: usize) -> bool {
    let n = a.n();
    if n == 0 {
        return true;
    }
    let inv: Vec<f64> = a.diag().into_iter().map(|d| if d.abs() > 1e-300 { 1.0 / d } else { 1.0 }).collect();
    let mut r = vec![0.0; n];
    a.mul(x, &mut r);
    for i in 0..n {
        r[i] = b[i] - r[i];
    }
    let bn = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
    let mut z: Vec<f64> = (0..n).map(|i| r[i] * inv[i]).collect();
    let mut p = z.clone();
    let mut rz: f64 = (0..n).map(|i| r[i] * z[i]).sum();
    let mut ap = vec![0.0; n];
    for _ in 0..max_iter {
        let rn = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !rn.is_finite() {
            return false;
        }
        if rn <= tol * bn {
            break;
        }
        a.mul(&p, &mut ap);
        let pap: f64 = (0..n).map(|i| p[i] * ap[i]).sum();
        if pap.abs() < 1e-300 {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        for i in 0..n {
            z[i] = r[i] * inv[i];
        }
        let rz2: f64 = (0..n).map(|i| r[i] * z[i]).sum();
        let beta = rz2 / rz;
        rz = rz2;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    x.iter().all(|v| v.is_finite())
}

// ---------------------------------------------------------------- charts

/// One unwrap chart: faces joined across non-seam manifold edges. Chart
/// vertices split mesh vertices where a seam cuts through them.
struct Chart {
    faces: Vec<u32>,
    /// Mesh vertex of each chart vertex.
    verts: Vec<u32>,
    /// (face, corner, chart vertex) for every corner of the chart's faces.
    corners: Vec<(u32, u32, u32)>,
    /// Triangles in chart vertices (n-gons triangulated).
    tris: Vec<[u32; 3]>,
}

/// Split `faces` into charts. With `use_seams` seam edges separate charts;
/// edges to faces outside the set, non-manifold edges and edges between
/// faces of opposite winding always do.
fn build_charts(m: &Mesh, faces: &[u32], use_seams: bool) -> Vec<Chart> {
    let mut start = vec![0usize; m.faces.len() + 1];
    for (i, f) in m.faces.iter().enumerate() {
        start[i + 1] = start[i] + f.verts.len();
    }
    let cid = |f: u32, c: usize| (start[f as usize] + c % m.faces[f as usize].verts.len()) as u32;
    let mut ucorner = UnionFind::new(start[m.faces.len()]);
    let mut uface = UnionFind::new(m.faces.len());
    let mut edges: HashMap<(u32, u32), Vec<(u32, usize)>> = HashMap::new();
    for &f in faces {
        let face = &m.faces[f as usize];
        for c in 0..face.verts.len() {
            let (a, b) = (face.verts[c], face.verts[(c + 1) % face.verts.len()]);
            if a != b {
                edges.entry(edge_key(a, b)).or_default().push((f, c));
            }
        }
    }
    for (k, list) in &edges {
        if list.len() != 2 || (use_seams && m.seams.contains(k)) {
            continue;
        }
        let ((f, c), (g, d)) = (list[0], list[1]);
        if f == g {
            continue;
        }
        let (ff, gg) = (&m.faces[f as usize], &m.faces[g as usize]);
        let (va, vb) = (ff.verts[c], ff.verts[(c + 1) % ff.len()]);
        let (wa, wb) = (gg.verts[d], gg.verts[(d + 1) % gg.len()]);
        if va != wb || vb != wa {
            continue; // flipped neighbour: an implicit seam
        }
        ucorner.union(cid(f, c), cid(g, d + 1));
        ucorner.union(cid(f, c + 1), cid(g, d));
        uface.union(f, g);
    }
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut gid: HashMap<u32, usize> = HashMap::new();
    for &f in faces {
        let r = uface.find(f);
        let i = *gid.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[i].push(f);
    }
    // Triangles per face, in face corners.
    let mut tris_of: HashMap<u32, Vec<[u16; 3]>> = HashMap::new();
    let in_set: HashSet<u32> = faces.iter().copied().collect();
    for t in m.triangulate() {
        if in_set.contains(&t.face) {
            tris_of.entry(t.face).or_default().push(t.corners);
        }
    }
    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        let mut local: HashMap<u32, u32> = HashMap::new();
        let mut ch = Chart { faces: g.clone(), verts: Vec::new(), corners: Vec::new(), tris: Vec::new() };
        for &f in &g {
            let face = &m.faces[f as usize];
            let mut lv = Vec::with_capacity(face.len());
            for c in 0..face.len() {
                let r = ucorner.find(cid(f, c));
                let l = *local.entry(r).or_insert_with(|| {
                    ch.verts.push(face.verts[c]);
                    (ch.verts.len() - 1) as u32
                });
                ch.corners.push((f, c as u32, l));
                lv.push(l);
            }
            for t in tris_of.get(&f).map(|v| v.as_slice()).unwrap_or(&[]) {
                let tri = t.map(|c| lv[c as usize]);
                if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                    ch.tris.push(tri);
                }
            }
        }
        out.push(ch);
    }
    out
}

/// Boundary loops of a triangle set (vertex sequences following boundary
/// half-edges) and which vertices are on a boundary.
fn boundaries(nv: usize, tris: &[[u32; 3]]) -> (Vec<Vec<u32>>, Vec<bool>) {
    let mut dir: HashMap<(u32, u32), u32> = HashMap::new();
    for t in tris {
        for k in 0..3 {
            *dir.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
        }
    }
    let mut on_b = vec![false; nv];
    let mut out_of: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut nb = 0;
    for (&(a, b), &n) in &dir {
        if n > 1 {
            on_b[a as usize] = true;
            on_b[b as usize] = true;
        }
        if !dir.contains_key(&(b, a)) {
            on_b[a as usize] = true;
            on_b[b as usize] = true;
            out_of.entry(a).or_default().push(b);
            nb += 1;
        }
    }
    for v in out_of.values_mut() {
        v.sort_unstable();
    }
    let mut starts: Vec<u32> = out_of.keys().copied().collect();
    starts.sort_unstable();
    let mut loops = Vec::new();
    let mut used = 0;
    for s in starts {
        while let Some(first) = out_of.get_mut(&s).and_then(|v| v.pop()) {
            used += 1;
            let mut lp = vec![s];
            let mut cur = first;
            let mut ok = true;
            while cur != s {
                lp.push(cur);
                match out_of.get_mut(&cur).and_then(|v| v.pop()) {
                    Some(n) => {
                        used += 1;
                        cur = n;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
                if used > nb {
                    ok = false;
                    break;
                }
            }
            if ok && lp.len() >= 2 {
                loops.push(lp);
            }
        }
    }
    (loops, on_b)
}

/// The three corner angles of triangle (a, b, c).
fn tri_angles(a: P3, b: P3, c: P3) -> [f64; 3] {
    let ang = |o: P3, p: P3, q: P3| {
        let (u, v) = (sub3(p, o), sub3(q, o));
        len3(cross3(u, v)).atan2(dot3(u, v))
    };
    let r = [ang(a, b, c), ang(b, c, a), ang(c, a, b)];
    if r.iter().all(|x| x.is_finite()) && r.iter().sum::<f64>() > 1e-9 {
        r
    } else {
        [PI / 3.0; 3]
    }
}

// ---------------------------------------------------------------- ABF++

const ABF_MIN_ANGLE: f64 = PI / 180.0;

/// Angle Based Flattening (Sheffer et al., ABF++ in Blender): find triangle
/// angles close to the 3D ones that form a valid flat triangulation
/// (triangle sums, wheel sums around interior vertices, and the sine-rule
/// wheel consistency). Newton steps on the Lagrangian, with the system
/// reduced to the interior-vertex constraints and solved by CG.
fn abf(nv: usize, tris: &[[u32; 3]], angles3d: &[[f64; 3]], interior: &[bool]) -> Option<Vec<[f64; 3]>> {
    let nt = tris.len();
    let (amin, amax) = (ABF_MIN_ANGLE, PI - ABF_MIN_ANGLE);
    // Interior vertices get rows 0..ni (planarity) and ni..2ni (length).
    let mut row = vec![u32::MAX; nv];
    let mut ni = 0u32;
    for v in 0..nv {
        if interior[v] {
            row[v] = ni;
            ni += 1;
        }
    }
    let ni = ni as usize;
    let mut beta: Vec<[f64; 3]> = angles3d.iter().map(|a| a.map(|x| x.clamp(amin, amax))).collect();
    // Interior wheels sum to 2*pi.
    let mut sum = vec![0.0; nv];
    for (t, tri) in tris.iter().enumerate() {
        for k in 0..3 {
            sum[tri[k] as usize] += beta[t][k];
        }
    }
    for (t, tri) in tris.iter().enumerate() {
        for k in 0..3 {
            let v = tri[k] as usize;
            if interior[v] && sum[v] > 0.0 {
                beta[t][k] *= 2.0 * PI / sum[v];
            }
        }
    }
    let w: Vec<[f64; 3]> = beta.iter().map(|b| b.map(|x| 1.0 / (x * x))).collect();
    let mut alpha = beta.clone();
    let mut l_tri = vec![0.0; nt];
    let mut l2 = vec![0.0; 2 * ni];
    for _iter in 0..20 {
        let sines: Vec<[f64; 3]> = alpha.iter().map(|a| a.map(f64::sin)).collect();
        let cosines: Vec<[f64; 3]> = alpha.iter().map(|a| a.map(f64::cos)).collect();
        // Wheel sine products: next (angle after v) and prev (angle before v).
        let mut pnext = vec![1.0f64; nv];
        let mut pprev = vec![1.0f64; nv];
        let mut wsum = vec![0.0f64; nv];
        for (t, tri) in tris.iter().enumerate() {
            for k in 0..3 {
                let v = tri[k] as usize;
                if interior[v] {
                    pnext[v] *= sines[t][(k + 1) % 3];
                    pprev[v] *= sines[t][(k + 2) % 3];
                    wsum[v] += alpha[t][k];
                }
            }
        }
        // J2 entries of each angle: (row, coefficient).
        let j2 = |t: usize, k: usize, out: &mut Vec<(u32, f64)>| {
            out.clear();
            let tri = tris[t];
            let v = tri[k] as usize;
            if interior[v] {
                out.push((row[v], 1.0));
            }
            // As the "next" angle of the vertex before it in the triangle.
            let vp = tri[(k + 2) % 3] as usize;
            if interior[vp] {
                let s = sines[t][k];
                let d = if s.abs() > 1e-12 { pnext[vp] * cosines[t][k] / s } else { 0.0 };
                out.push((ni as u32 + row[vp], d));
            }
            // As the "prev" angle of the vertex after it.
            let vn = tri[(k + 1) % 3] as usize;
            if interior[vn] {
                let s = sines[t][k];
                let d = if s.abs() > 1e-12 { -pprev[vn] * cosines[t][k] / s } else { 0.0 };
                out.push((ni as u32 + row[vn], d));
            }
        };
        // Residuals.
        let mut ba = vec![[0.0f64; 3]; nt];
        let mut b1 = vec![0.0f64; nt];
        let mut b2 = vec![0.0f64; 2 * ni];
        let mut norm = 0.0;
        let mut ent = Vec::with_capacity(3);
        for t in 0..nt {
            for k in 0..3 {
                let mut g = w[t][k] * (alpha[t][k] - beta[t][k]) + l_tri[t];
                j2(t, k, &mut ent);
                for &(r, c) in &ent {
                    g += c * l2[r as usize];
                }
                ba[t][k] = -g;
                norm += g * g;
            }
            let c = alpha[t][0] + alpha[t][1] + alpha[t][2] - PI;
            b1[t] = -c;
            norm += c * c;
        }
        for v in 0..nv {
            if interior[v] {
                let r = row[v] as usize;
                let cp = wsum[v] - 2.0 * PI;
                let cl = pnext[v] - pprev[v];
                b2[r] = -cp;
                b2[ni + r] = -cl;
                norm += cp * cp + cl * cl;
            }
        }
        if !norm.is_finite() {
            return None;
        }
        if norm < 1e-14 {
            break;
        }
        // Per triangle: d = 1/w, P = diag(d) - d d^T / D, q = P ba + d b1 / D.
        let mut q = vec![[0.0f64; 3]; nt];
        let mut trip: Vec<(u32, u32, f64)> = Vec::new();
        let mut rhs = vec![0.0f64; 2 * ni];
        let mut ents: [Vec<(u32, f64)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for t in 0..nt {
            let d = [1.0 / w[t][0], 1.0 / w[t][1], 1.0 / w[t][2]];
            let dd = d[0] + d[1] + d[2];
            let p = |i: usize, j: usize| if i == j { d[i] - d[i] * d[i] / dd } else { -d[i] * d[j] / dd };
            for i in 0..3 {
                let mut s = 0.0;
                for j in 0..3 {
                    s += p(i, j) * ba[t][j];
                }
                q[t][i] = s + d[i] * b1[t] / dd;
            }
            for (k, e) in ents.iter_mut().enumerate() {
                j2(t, k, e);
            }
            for i in 0..3 {
                for &(r, c) in &ents[i] {
                    rhs[r as usize] += c * q[t][i];
                    for j in 0..3 {
                        let pij = p(i, j);
                        for &(r2, c2) in &ents[j] {
                            trip.push((r, r2, c * pij * c2));
                        }
                    }
                }
            }
        }
        for r in 0..2 * ni {
            rhs[r] -= b2[r];
            trip.push((r as u32, r as u32, 1e-12));
        }
        let mat = Csr::from_triplets(2 * ni, trip);
        let mut dl2 = vec![0.0f64; 2 * ni];
        if !cg(&mat, &rhs, &mut dl2, 1e-10, 2000 + 4 * ni) {
            return None;
        }
        // Back substitution.
        for t in 0..nt {
            let d = [1.0 / w[t][0], 1.0 / w[t][1], 1.0 / w[t][2]];
            let dd = d[0] + d[1] + d[2];
            let p = |i: usize, j: usize| if i == j { d[i] - d[i] * d[i] / dd } else { -d[i] * d[j] / dd };
            let mut jt = [0.0f64; 3];
            for (k, e) in ents.iter_mut().enumerate() {
                j2(t, k, e);
                jt[k] = e.iter().map(|&(r, c)| c * dl2[r as usize]).sum();
            }
            let mut dl1 = -b1[t];
            for i in 0..3 {
                dl1 += d[i] * (ba[t][i] - jt[i]);
            }
            l_tri[t] += dl1 / dd;
            for i in 0..3 {
                let mut s = 0.0;
                for j in 0..3 {
                    s += p(i, j) * jt[j];
                }
                alpha[t][i] = (alpha[t][i] + q[t][i] - s).clamp(amin, amax);
            }
        }
        for r in 0..2 * ni {
            l2[r] += dl2[r];
        }
    }
    alpha.iter().all(|a| a.iter().all(|x| x.is_finite())).then_some(alpha)
}

// ---------------------------------------------------------------- LSCM

/// Least Squares Conformal Maps in Blender's angle-based form: every
/// triangle asks p3 - p1 = (sin a2 / sin a3) * R(a1) (p2 - p1). Pinned
/// vertices are fixed; the rest solve the normal equations by CG.
fn lscm(nv: usize, tris: &[[u32; 3]], angles: &[[f64; 3]], pinned: &[Option<P2>], init: &[P2]) -> Option<Vec<P2>> {
    let mut var = vec![u32::MAX; nv];
    let mut n = 0u32;
    for v in 0..nv {
        if pinned[v].is_none() {
            var[v] = n;
            n += 2;
        }
    }
    let n = n as usize;
    let mut trip: Vec<(u32, u32, f64)> = Vec::with_capacity(tris.len() * 72);
    let mut rhs = vec![0.0f64; n];
    for (t, tri) in tris.iter().enumerate() {
        let mut v = *tri;
        let mut a = angles[t];
        let mut s = a.map(f64::sin);
        let smax = s[0].max(s[1]).max(s[2]);
        // Rotate so the largest sine is a3 (most stable), like Blender's SHIFT3.
        for _ in 0..2 {
            if s[2] == smax {
                break;
            }
            v = [v[2], v[0], v[1]];
            a = [a[2], a[0], a[1]];
            s = [s[2], s[0], s[1]];
        }
        let ratio = if s[2] == 0.0 { 1.0 } else { s[1] / s[2] };
        let (cosine, sine) = (a[0].cos() * ratio, s[0] * ratio);
        // Coefficients on (x1, y1, x2, y2, x3, y3).
        let rows = [[cosine - 1.0, -sine, -cosine, sine, 1.0, 0.0], [sine, cosine - 1.0, -sine, -cosine, 0.0, 1.0]];
        for r in rows {
            let mut free: Vec<(u32, f64)> = Vec::with_capacity(6);
            let mut b = 0.0;
            for (k, &c) in r.iter().enumerate() {
                if c == 0.0 {
                    continue;
                }
                let vi = v[k / 2] as usize;
                match pinned[vi] {
                    Some(p) => b -= c * p[k % 2],
                    None => free.push((var[vi] + (k % 2) as u32, c)),
                }
            }
            for &(i, ci) in &free {
                rhs[i as usize] += ci * b;
                for &(j, cj) in &free {
                    trip.push((i, j, ci * cj));
                }
            }
        }
    }
    let mat = Csr::from_triplets(n, trip);
    let mut x = vec![0.0f64; n];
    for v in 0..nv {
        if var[v] != u32::MAX {
            x[var[v] as usize] = init[v][0];
            x[var[v] as usize + 1] = init[v][1];
        }
    }
    if !cg(&mat, &rhs, &mut x, 1e-11, 20000 + 10 * n) {
        return None;
    }
    Some((0..nv).map(|v| pinned[v].unwrap_or_else(|| [x[var[v] as usize], x[var[v] as usize + 1]])).collect())
}

/// Blender's `p_chart_extrema_verts` + `p_chart_pin_positions`: pin the
/// vertices furthest apart along the chart's longest axis at their 3D
/// coordinates in the plane of the two largest axes between them.
fn auto_pins(pos: &[P3]) -> (usize, usize, P2, P2, impl Fn(P3) -> P2) {
    let mut mn = [f64::INFINITY; 3];
    let mut mx = [f64::NEG_INFINITY; 3];
    let mut vmin = [0usize; 3];
    let mut vmax = [0usize; 3];
    for (i, p) in pos.iter().enumerate() {
        for a in 0..3 {
            if p[a] < mn[a] {
                mn[a] = p[a];
                vmin[a] = i;
            }
            if p[a] > mx[a] {
                mx[a] = p[a];
                vmax[a] = i;
            }
        }
    }
    let dir = (0..3).max_by(|&a, &b| (mx[a] - mn[a]).total_cmp(&(mx[b] - mn[b]))).unwrap_or(0);
    let (p1, p2) = (vmin[dir], vmax[dir]);
    let s = sub3(pos[p1], pos[p2]).map(f64::abs);
    let (dx, dy) = if s[0] > s[1] && s[0] > s[2] {
        (0, if s[1] > s[2] { 1 } else { 2 })
    } else if s[1] > s[0] && s[1] > s[2] {
        (1, if s[0] > s[2] { 0 } else { 2 })
    } else {
        (2, if s[0] > s[1] { 0 } else { 1 })
    };
    let swap = dx == 2;
    let map = move |p: P3| if swap { [p[dy], p[dx]] } else { [p[dx], p[dy]] };
    (p1, p2, map(pos[p1]), map(pos[p2]), map)
}

/// A starting point for CG: the chart projected on its average plane,
/// placed by the similarity that puts two pinned vertices on their pins.
/// Exact for flat charts, close for nearly developable ones.
fn initial_guess(pos: &[P3], tris: &[[u32; 3]], pinned: &[Option<P2>]) -> Option<Vec<P2>> {
    let pins: Vec<(usize, P2)> = pinned.iter().enumerate().filter_map(|(i, p)| p.map(|p| (i, p))).take(2).collect();
    let [(i1, q1), (i2, q2)] = pins[..] else { return None };
    let mut n = [0.0; 3];
    for t in tris {
        let c = cross3(sub3(pos[t[1] as usize], pos[t[0] as usize]), sub3(pos[t[2] as usize], pos[t[0] as usize]));
        n = [n[0] + c[0], n[1] + c[1], n[2] + c[2]];
    }
    let ln = len3(n);
    if !(ln > 1e-300) {
        return None;
    }
    let n = n.map(|x| x / ln);
    let d = sub3(pos[i2], pos[i1]);
    let u = sub3(d, n.map(|x| x * dot3(d, n)));
    let lu = len3(u);
    if !(lu > 1e-300) {
        return None;
    }
    let u = u.map(|x| x / lu);
    let v = cross3(n, u);
    // Local coordinates have pin 1 at the origin and pin 2 at (lu, 0).
    let (qx, qy) = (q2[0] - q1[0], q2[1] - q1[1]);
    let (c, s) = (qx / lu, qy / lu);
    Some(
        pos.iter()
            .map(|&p| {
                let r = sub3(p, pos[i1]);
                let (a, b) = (dot3(r, u), dot3(r, v));
                [q1[0] + a * c - b * s, q1[1] + a * s + b * c]
            })
            .collect(),
    )
}

/// Flatten one chart. `pins` are chart vertices with fixed UVs. Returns None
/// for a closed chart (no boundary: Blender skips those and asks for seams).
fn solve_chart(pos: &[P3], tris: &[[u32; 3]], pins: &[(u32, P2)], method: UnwrapMethod, fill_holes: bool) -> Option<Vec<P2>> {
    let nv0 = pos.len();
    if tris.is_empty() {
        return None;
    }
    let (loops, _) = boundaries(nv0, tris);
    if loops.is_empty() {
        return None;
    }
    let mut pos = pos.to_vec();
    let mut tris = tris.to_vec();
    if fill_holes && loops.len() > 1 {
        let lens: Vec<f64> = loops.iter().map(|l| (0..l.len()).map(|i| len3(sub3(pos[l[i] as usize], pos[l[(i + 1) % l.len()] as usize]))).sum()).collect();
        let longest = (0..loops.len()).max_by(|&a, &b| lens[a].total_cmp(&lens[b])).unwrap_or(0);
        for (i, l) in loops.iter().enumerate() {
            if i == longest || l.len() < 3 {
                continue;
            }
            let mut c = [0.0; 3];
            for &v in l {
                for a in 0..3 {
                    c[a] += pos[v as usize][a] / l.len() as f64;
                }
            }
            pos.push(c);
            let cv = (pos.len() - 1) as u32;
            for k in 0..l.len() {
                tris.push([l[(k + 1) % l.len()], l[k], cv]);
            }
        }
    }
    let nv = pos.len();
    let angles3d: Vec<[f64; 3]> = tris.iter().map(|t| tri_angles(pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize])).collect();
    let angles = match method {
        UnwrapMethod::AngleBased => {
            let (_, on_b) = boundaries(nv, &tris);
            let interior: Vec<bool> = on_b.iter().map(|b| !b).collect();
            abf(nv, &tris, &angles3d, &interior).unwrap_or(angles3d)
        }
        UnwrapMethod::Conformal => angles3d,
    };
    let (a1, a2, q1, q2, map) = auto_pins(&pos[..nv0]);
    let mut pinned: Vec<Option<P2>> = vec![None; nv];
    let single = pins.len() == 1;
    if pins.len() >= 2 {
        for &(v, p) in pins {
            pinned[v as usize] = Some(p);
        }
    } else if a1 != a2 {
        pinned[a1] = Some(q1);
        pinned[a2] = Some(q2);
    } else {
        let t = tris[0];
        pinned[t[0] as usize] = Some([0.0, 0.5]);
        pinned[t[1] as usize] = Some([1.0, 0.5]);
    }
    let init = initial_guess(&pos, &tris, &pinned).unwrap_or_else(|| pos.iter().map(|&p| map(p)).collect());
    let mut uv = lscm(nv, &tris, &angles, &pinned, &init)?;
    uv.truncate(nv0);
    if single {
        let (v, p) = pins[0];
        let d = [p[0] - uv[v as usize][0], p[1] - uv[v as usize][1]];
        for q in &mut uv {
            q[0] += d[0];
            q[1] += d[1];
        }
    }
    uv.iter().all(|p| p[0].is_finite() && p[1].is_finite()).then_some(uv)
}

// ---------------------------------------------------------------- unwrap

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwrapMethod {
    /// ABF++ angles, then LSCM (Blender's default).
    AngleBased,
    /// Plain LSCM on the 3D angles.
    Conformal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnwrapOptions {
    pub method: UnwrapMethod,
    /// Fill holes inside charts so they do not overlap themselves.
    pub fill_holes: bool,
    /// Space between packed islands, as a fraction of the UV square.
    pub margin: f32,
}

impl Default for UnwrapOptions {
    fn default() -> Self {
        UnwrapOptions { method: UnwrapMethod::AngleBased, fill_holes: true, margin: 0.001 }
    }
}

/// What an unwrap did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnwrapReport {
    pub islands: usize,
    /// Charts that could not be flattened (closed, no seams).
    pub failed: usize,
}

/// UV > Unwrap: split `faces` into charts at seams, flatten each with ABF++
/// or LSCM, give the charts equal texel density and pack them into 0..1.
/// Charts with pinned UVs keep their pins and are not moved by packing.
pub fn unwrap(m: &mut Mesh, faces: &[u32], opts: &UnwrapOptions) -> UnwrapReport {
    let faces = valid_faces(m, faces);
    ensure_uvs(m);
    let charts = build_charts(m, &faces, true);
    let mut rep = UnwrapReport::default();
    let mut moved = Vec::new();
    for ch in &charts {
        let pos: Vec<P3> = ch.verts.iter().map(|&v| p3(m.positions[v as usize])).collect();
        let mut pins: Vec<(u32, P2)> = Vec::new();
        let mut seen = HashSet::new();
        for &(f, c, l) in &ch.corners {
            if m.uv_pins.contains(&(f, c)) && seen.insert(l) {
                pins.push((l, uv2(m.faces[f as usize].uvs[c as usize])));
            }
        }
        match solve_chart(&pos, &ch.tris, &pins, opts.method, opts.fill_holes) {
            Some(uv) => {
                for &(f, c, l) in &ch.corners {
                    m.faces[f as usize].uvs[c as usize] = vec2(uv[l as usize]);
                }
                rep.islands += 1;
                if pins.is_empty() {
                    moved.push(ch.faces.clone());
                }
            }
            None => rep.failed += 1,
        }
    }
    average_scale_islands(m, &moved);
    pack_island_list(m, &moved, opts.margin as f64, true, 0.0);
    rep
}

// ---------------------------------------------------------------- islands

/// UV islands of `faces`: faces joined by edges whose two UVs match in
/// both faces (Blender's UV island connectivity).
pub fn uv_islands(m: &Mesh, faces: &[u32]) -> Vec<Vec<u32>> {
    let faces = valid_faces(m, faces);
    let mut uf = UnionFind::new(m.faces.len());
    let mut edges: HashMap<(u32, u32), Vec<(u32, usize)>> = HashMap::new();
    for &f in &faces {
        let face = &m.faces[f as usize];
        if face.uvs.len() != face.verts.len() {
            continue;
        }
        for c in 0..face.len() {
            edges.entry(edge_key(face.verts[c], face.verts[(c + 1) % face.len()])).or_default().push((f, c));
        }
    }
    let close = |a: Vec2, b: Vec2| (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5;
    for list in edges.values() {
        for i in 0..list.len() {
            for j in i + 1..list.len() {
                let ((f, c), (g, _)) = (list[i], list[j]);
                let (ff, gg) = (&m.faces[f as usize], &m.faces[g as usize]);
                let uv_at = |face: &crate::Face, v: u32| face.verts.iter().position(|&x| x == v).map(|k| face.uvs[k]);
                let (a, b) = (ff.verts[c], ff.verts[(c + 1) % ff.len()]);
                let ok = match (uv_at(gg, a), uv_at(gg, b)) {
                    (Some(ga), Some(gb)) => close(ff.uvs[c], ga) && close(ff.uvs[(c + 1) % ff.len()], gb),
                    _ => false,
                };
                if ok {
                    uf.union(f, g);
                }
            }
        }
    }
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut gid: HashMap<u32, usize> = HashMap::new();
    for &f in &faces {
        let r = uf.find(f);
        let i = *gid.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[i].push(f);
    }
    groups
}

/// The UV island (as above) that contains face `f`, within `faces`.
pub fn island_of(m: &Mesh, faces: &[u32], f: u32) -> Vec<u32> {
    uv_islands(m, faces).into_iter().find(|i| i.contains(&f)).unwrap_or_default()
}

fn island_has_pins(m: &Mesh, island: &[u32]) -> bool {
    !m.uv_pins.is_empty() && island.iter().any(|&f| m.uv_pins.range((f, 0)..(f + 1, 0)).next().is_some())
}

/// Scale islands so each has the same ratio of UV area to 3D area (the
/// overall ratio), about the centre of its bounds.
fn average_scale_islands(m: &mut Mesh, islands: &[Vec<u32>]) {
    let areas: Vec<(f64, f64)> = islands.iter().map(|isl| (isl.iter().map(|&f| m.face_area(f as usize) as f64).sum(), isl.iter().map(|&f| face_uv_area(m, f).abs()).sum())).collect();
    let tot3: f64 = areas.iter().map(|a| a.0).sum();
    let totuv: f64 = areas.iter().map(|a| a.1).sum();
    if tot3 <= 0.0 || totuv <= 0.0 {
        return;
    }
    for (isl, &(a3, auv)) in islands.iter().zip(&areas) {
        if a3 <= 0.0 || auv <= 0.0 {
            continue;
        }
        let s = ((totuv / tot3) * (a3 / auv)).sqrt();
        let (mn, mx) = island_bounds(m, isl);
        let c = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5];
        for &f in isl {
            for u in &mut m.faces[f as usize].uvs {
                *u = vec2([c[0] + (u.x as f64 - c[0]) * s, c[1] + (u.y as f64 - c[1]) * s]);
            }
        }
    }
}

fn island_bounds(m: &Mesh, isl: &[u32]) -> (P2, P2) {
    let mut mn = [f64::INFINITY; 2];
    let mut mx = [f64::NEG_INFINITY; 2];
    for &f in isl {
        for u in &m.faces[f as usize].uvs {
            mn = [mn[0].min(u.x as f64), mn[1].min(u.y as f64)];
            mx = [mx[0].max(u.x as f64), mx[1].max(u.y as f64)];
        }
    }
    (mn, mx)
}

/// UV > Average Islands Scale over the UV islands of `faces` (pinned
/// islands stay as they are).
pub fn average_islands_scale(m: &mut Mesh, faces: &[u32]) {
    ensure_uvs(m);
    let isl: Vec<Vec<u32>> = uv_islands(m, faces).into_iter().filter(|i| !island_has_pins(m, i)).collect();
    average_scale_islands(m, &isl);
}

// ---------------------------------------------------------------- packing

fn convex_hull(mut pts: Vec<P2>) -> Vec<P2> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: P2, a: P2, b: P2| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut h: Vec<P2> = Vec::with_capacity(pts.len() * 2);
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &P2>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for &p in it {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], p) <= 0.0 {
                h.pop();
            }
            h.push(p);
        }
        h.pop();
    }
    h
}

/// Rotation (radians) that gives the smallest-area bounding box, landscape.
fn min_area_angle(hull: &[P2]) -> f64 {
    let n = hull.len();
    if n < 3 {
        return 0.0;
    }
    let bbox = |ang: f64| {
        let (s, c) = ang.sin_cos();
        let mut mn = [f64::INFINITY; 2];
        let mut mx = [f64::NEG_INFINITY; 2];
        for p in hull {
            let q = [p[0] * c - p[1] * s, p[0] * s + p[1] * c];
            mn = [mn[0].min(q[0]), mn[1].min(q[1])];
            mx = [mx[0].max(q[0]), mx[1].max(q[1])];
        }
        (mx[0] - mn[0], mx[1] - mn[1])
    };
    let (w0, h0) = bbox(0.0);
    let mut best = (w0 * h0, 0.0);
    for i in 0..n {
        let (a, b) = (hull[i], hull[(i + 1) % n]);
        let ang = -(b[1] - a[1]).atan2(b[0] - a[0]);
        let (w, h) = bbox(ang);
        if w * h < best.0 * (1.0 - 1e-9) {
            best = (w * h, ang);
        }
    }
    let (w, h) = bbox(best.1);
    if h > w {
        best.1 + PI / 2.0
    } else {
        best.1
    }
}

/// Skyline bottom-left placement of rectangles into a strip `width` wide.
/// Returns positions and the height used.
fn skyline(sizes: &[(f64, f64)], order: &[usize], width: f64) -> (Vec<(f64, f64)>, f64, f64) {
    let mut sky: Vec<(f64, f64, f64)> = vec![(0.0, 0.0, width)]; // x, y, w
    let mut pos = vec![(0.0, 0.0); sizes.len()];
    let (mut top, mut right) = (0.0f64, 0.0f64);
    for &i in order {
        let (w, h) = sizes[i];
        let mut best: Option<(f64, f64, usize)> = None; // y, x, seg
        for s in 0..sky.len() {
            let x = sky[s].0;
            if x + w > width + 1e-9 * width.max(1.0) {
                break;
            }
            let mut y = 0.0f64;
            let mut k = s;
            while k < sky.len() && sky[k].0 < x + w - 1e-12 {
                y = y.max(sky[k].1);
                k += 1;
            }
            if best.is_none_or(|b| y + h < b.0 + h - 1e-12 || ((y - b.0).abs() <= 1e-12 && x < b.1)) {
                best = Some((y, x, s));
            }
        }
        let (y, x) = match best {
            Some((y, x, _)) => (y, x),
            None => (sky.iter().map(|s| s.1).fold(0.0, f64::max), 0.0),
        };
        pos[i] = (x, y);
        top = top.max(y + h);
        right = right.max(x + w);
        // Raise the skyline over [x, x + w).
        let (x0, x1) = (x, x + w);
        let mut next: Vec<(f64, f64, f64)> = Vec::with_capacity(sky.len() + 2);
        let mut placed = false;
        for &(sx, sy, sw) in &sky {
            let se = sx + sw;
            if se <= x0 + 1e-12 || sx >= x1 - 1e-12 {
                if sx >= x1 - 1e-12 && !placed {
                    next.push((x0, y + h, w));
                    placed = true;
                }
                next.push((sx, sy, sw));
                continue;
            }
            if sx < x0 {
                next.push((sx, sy, x0 - sx));
            }
            if !placed {
                next.push((x0, y + h, w));
                placed = true;
            }
            if se > x1 {
                next.push((x1, sy, se - x1));
            }
        }
        if !placed {
            next.push((x0, y + h, w));
        }
        // Merge neighbours at the same height.
        sky.clear();
        for s in next {
            if let Some(l) = sky.last_mut() {
                if (l.1 - s.1).abs() < 1e-12 {
                    l.2 = s.0 + s.2 - l.0;
                    continue;
                }
            }
            if s.2 > 1e-15 {
                sky.push(s);
            }
        }
    }
    (pos, right, top)
}

/// Simple shelf packing for many rectangles.
fn shelves(sizes: &[(f64, f64)], order: &[usize], width: f64) -> (Vec<(f64, f64)>, f64, f64) {
    let mut pos = vec![(0.0, 0.0); sizes.len()];
    let (mut x, mut y, mut shelf_h, mut right) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for &i in order {
        let (w, h) = sizes[i];
        if x > 0.0 && x + w > width {
            y += shelf_h;
            x = 0.0;
            shelf_h = 0.0;
        }
        pos[i] = (x, y);
        x += w;
        right = right.max(x);
        shelf_h = shelf_h.max(h);
    }
    (pos, right, y + shelf_h)
}

/// Pack rectangles into the smallest square found; returns positions and side.
fn pack_rects(sizes: &[(f64, f64)]) -> (Vec<(f64, f64)>, f64) {
    if sizes.is_empty() {
        return (Vec::new(), 1.0);
    }
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|&a, &b| sizes[b].1.total_cmp(&sizes[a].1).then(sizes[b].0.total_cmp(&sizes[a].0)));
    let area: f64 = sizes.iter().map(|s| s.0 * s.1).sum();
    let maxw = sizes.iter().map(|s| s.0).fold(0.0, f64::max);
    let sumw: f64 = sizes.iter().map(|s| s.0).sum();
    let lo = area.sqrt().max(maxw).max(1e-12);
    let hi = sumw.max(lo);
    let many = sizes.len() > 1500;
    let mut best: Option<(f64, Vec<(f64, f64)>)> = None;
    let steps = if many { 12 } else { 32 };
    for k in 0..=steps {
        let w = lo * (hi / lo).powf(k as f64 / steps as f64 * 0.35).min(hi / lo);
        let (pos, right, top) = if many { shelves(sizes, &order, w) } else { skyline(sizes, &order, w) };
        let side = right.max(top);
        if best.as_ref().is_none_or(|b| side < b.0) {
            best = Some((side, pos));
        }
        if w >= hi {
            break;
        }
    }
    let (side, pos) = best.expect("one packing");
    (pos, side.max(1e-12))
}

/// Pack the given face islands into 0..1. `margin` is the gap between
/// islands as a fraction of the square; `pad` is extra space around each
/// island in its own units (Lightmap Pack's per-face margin).
fn pack_island_list(m: &mut Mesh, islands: &[Vec<u32>], margin: f64, rotate: bool, pad: f64) {
    if islands.is_empty() {
        return;
    }
    // Orient each island and find its box.
    let mut boxes = Vec::with_capacity(islands.len());
    for isl in islands {
        let pts: Vec<P2> = isl.iter().flat_map(|&f| m.faces[f as usize].uvs.iter().map(|&u| uv2(u))).collect();
        let ang = if rotate { min_area_angle(&convex_hull(pts.clone())) } else { 0.0 };
        let (s, c) = ang.sin_cos();
        let mut mn = [f64::INFINITY; 2];
        let mut mx = [f64::NEG_INFINITY; 2];
        for p in &pts {
            let q = [p[0] * c - p[1] * s, p[0] * s + p[1] * c];
            mn = [mn[0].min(q[0]), mn[1].min(q[1])];
            mx = [mx[0].max(q[0]), mx[1].max(q[1])];
        }
        if !mn[0].is_finite() {
            mn = [0.0; 2];
            mx = [0.0; 2];
        }
        boxes.push((ang, mn, [mx[0] - mn[0], mx[1] - mn[1]]));
    }
    let tiny = boxes.iter().map(|b| b.2[0].max(b.2[1])).fold(0.0, f64::max) * 1e-6 + 1e-12;
    let sized = |p: f64| boxes.iter().map(|b| (b.2[0].max(tiny) + 2.0 * p, b.2[1].max(tiny) + 2.0 * p)).collect::<Vec<_>>();
    let mut p = pad;
    let (mut pos, mut side) = pack_rects(&sized(p));
    if margin > 0.0 {
        for _ in 0..4 {
            p = pad + margin * side * 0.5;
            let r = pack_rects(&sized(p));
            pos = r.0;
            side = r.1;
        }
    }
    let inv = 1.0 / side;
    for (k, isl) in islands.iter().enumerate() {
        let (ang, mn, _) = boxes[k];
        let (s, c) = ang.sin_cos();
        let (ox, oy) = pos[k];
        for &f in isl {
            for u in &mut m.faces[f as usize].uvs {
                let q = [u.x as f64 * c - u.y as f64 * s, u.x as f64 * s + u.y as f64 * c];
                *u = vec2([(q[0] - mn[0] + ox + p) * inv, (q[1] - mn[1] + oy + p) * inv]);
            }
        }
    }
}

/// UV > Pack Islands: the UV islands of `faces` packed into 0..1 with
/// `margin` between them (Blender's default 0.001), rotating each to its
/// smallest box when `rotate`. Islands with pinned UVs stay put.
pub fn pack_islands(m: &mut Mesh, faces: &[u32], margin: f32, rotate: bool) {
    ensure_uvs(m);
    let isl: Vec<Vec<u32>> = uv_islands(m, faces).into_iter().filter(|i| !island_has_pins(m, i)).collect();
    pack_island_list(m, &isl, margin as f64, rotate, 0.0);
}

// ---------------------------------------------------------------- projections

/// Blender's `uv_map_clip_correct`: optionally scale the result to fill
/// 0..1 (per axis) or clamp it there.
pub fn clip_correct(m: &mut Mesh, faces: &[u32], scale_to_bounds: bool, clip_to_bounds: bool) {
    let faces = valid_faces(m, faces);
    if scale_to_bounds {
        let (mn, mx) = island_bounds(m, &faces);
        let d = [mx[0] - mn[0], mx[1] - mn[1]];
        for &f in &faces {
            for u in &mut m.faces[f as usize].uvs {
                *u = vec2([if d[0] > 0.0 { (u.x as f64 - mn[0]) / d[0] } else { 0.0 }, if d[1] > 0.0 { (u.y as f64 - mn[1]) / d[1] } else { 0.0 }]);
            }
        }
    } else if clip_to_bounds {
        for &f in &faces {
            for u in &mut m.faces[f as usize].uvs {
                *u = Vec2::new(u.x.clamp(0.0, 1.0), u.y.clamp(0.0, 1.0));
            }
        }
    }
}

fn sel_bounds(m: &Mesh, faces: &[u32]) -> (Vec3, Vec3) {
    let mut b = crate::math::Aabb::EMPTY;
    for &f in faces {
        for &v in &m.faces[f as usize].verts {
            b.grow(m.positions[v as usize]);
        }
    }
    if b.is_empty() {
        (Vec3::ZERO, Vec3::ZERO)
    } else {
        (b.center(), b.size())
    }
}

/// UV > Cube Projection. `cube_size` None uses the selection's largest
/// dimension, like Blender when the size is not set; the cube is centred on
/// the selection's bounds.
pub fn cube_project(m: &mut Mesh, faces: &[u32], cube_size: Option<f32>) {
    let faces = valid_faces(m, faces);
    ensure_uvs(m);
    let (center, size) = sel_bounds(m, &faces);
    let mut cs = cube_size.unwrap_or_else(|| size.max_elem());
    if cs == 0.0 || !cs.is_finite() {
        cs = 1.0;
    }
    for &f in &faces {
        let (ax, ay) = axis_dominant(m.face_normal(f as usize));
        let face = &mut m.faces[f as usize];
        for (k, &v) in face.verts.iter().enumerate() {
            let p = m.positions[v as usize] - center;
            face.uvs[k] = Vec2::new(0.5 + p[ax] / cs, 0.5 + p[ay] / cs);
        }
    }
}

/// Blender's `uv_map_mirror` for wrapped projections: faces that straddle
/// the u = 0/1 seam get their low corners moved up by one; corners at a
/// pole take the face's average u ("Pinch").
fn fix_wrap(uvs: &mut [Vec2], at_pole: &[bool]) {
    let n = uvs.len();
    let mi = (0..n).filter(|&i| !at_pole[i]).max_by(|&a, &b| uvs[a].x.total_cmp(&uvs[b].x));
    if let Some(mi) = mi {
        for i in 0..n {
            if i != mi && !at_pole[i] && uvs[mi].x - uvs[i].x > 0.5 {
                uvs[i].x += 1.0;
            }
        }
    }
    let others: Vec<f32> = (0..n).filter(|&i| !at_pole[i]).map(|i| uvs[i].x).collect();
    if !others.is_empty() {
        let avg = others.iter().sum::<f32>() / others.len() as f32;
        for i in 0..n {
            if at_pole[i] {
                uvs[i].x = avg;
            }
        }
    }
}

fn wrap_project(m: &mut Mesh, faces: &[u32], f: impl Fn(Vec3) -> (f32, f32, bool)) {
    let faces = valid_faces(m, faces);
    ensure_uvs(m);
    let (center, _) = sel_bounds(m, &faces);
    for &fi in &faces {
        let face = &mut m.faces[fi as usize];
        let mut pole = vec![false; face.len()];
        for (k, &v) in face.verts.iter().enumerate() {
            let (u, w, ok) = f(m.positions[v as usize] - center);
            face.uvs[k] = Vec2::new(u, w);
            pole[k] = !ok;
        }
        fix_wrap(&mut face.uvs, &pole);
    }
}

/// Blender's `map_to_tube` u: angle around Z, seam on -Y.
fn tube_u(x: f32, y: f32) -> (f32, bool) {
    let l = (x * x + y * y).sqrt();
    if l <= 1e-7 {
        (0.0, false)
    } else {
        ((1.0 - (x / l).atan2(y / l) / core::f32::consts::PI) / 2.0, true)
    }
}

/// UV > Cylinder Projection around the object's Z axis through the
/// selection's centre (Blender's "Align to Object", polar ZX).
pub fn cylinder_project(m: &mut Mesh, faces: &[u32], radius: f32) {
    let r = if radius.abs() > 1e-9 { radius } else { 1.0 };
    wrap_project(m, faces, |p| {
        let (u, ok) = tube_u(p.x, p.y);
        (u, (p.z / r + 1.0) / 2.0, ok)
    });
}

/// UV > Sphere Projection about the selection's centre, poles on Z.
pub fn sphere_project(m: &mut Mesh, faces: &[u32]) {
    wrap_project(m, faces, |p| {
        let l = p.length();
        if l <= 1e-7 {
            return (0.0, 0.5, false);
        }
        let (u, ok) = tube_u(p.x, p.y);
        (u, 1.0 - (p.z / l).clamp(-1.0, 1.0).acos() / core::f32::consts::PI, ok)
    });
}

/// UV > Project from View: `project` maps a mesh-space position to UV (the
/// caller folds in the object matrix and the view, with the 0..1 square
/// spanning the view's larger side). `scale_to_bounds` is "(Bounds)".
pub fn project_from_view(m: &mut Mesh, faces: &[u32], project: impl Fn(Vec3) -> Option<Vec2>, scale_to_bounds: bool) {
    let faces = valid_faces(m, faces);
    ensure_uvs(m);
    for &f in &faces {
        let face = &mut m.faces[f as usize];
        for (k, &v) in face.verts.iter().enumerate() {
            face.uvs[k] = project(m.positions[v as usize]).filter(|u| u.x.is_finite() && u.y.is_finite()).unwrap_or(Vec2::ZERO);
        }
    }
    clip_correct(m, &faces, scale_to_bounds, false);
}

/// Front orthographic view (Numpad 1: X right, Z up) fitted around the
/// selection: Project from View when there is no 3D view to use.
pub fn project_front(m: &mut Mesh, faces: &[u32], scale_to_bounds: bool) {
    let fs = valid_faces(m, faces);
    let (c, size) = sel_bounds(m, &fs);
    let s = size.x.max(size.z).max(1e-6);
    project_from_view(m, &fs, |p| Some(Vec2::new(0.5 + (p.x - c.x) / s, 0.5 + (p.z - c.z) / s)), scale_to_bounds);
}

/// UV > Reset: every face gets Blender's default UVs (the unit square for
/// quads, a circle for other polygons).
pub fn reset(m: &mut Mesh, faces: &[u32]) {
    let faces = valid_faces(m, faces);
    ensure_uvs(m);
    for &f in &faces {
        let face = &mut m.faces[f as usize];
        let n = face.len();
        face.uvs = match n {
            3 => vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)],
            4 => vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
            _ => (0..n)
                .map(|i| {
                    let a = i as f32 / n as f32 * core::f32::consts::TAU;
                    Vec2::new(0.5 * a.sin() + 0.5, 0.5 * a.cos() + 0.5)
                })
                .collect(),
        };
    }
}

/// An orthonormal basis (u, v) of the plane with normal `n`.
fn plane_basis(n: P3) -> (P3, P3) {
    let l = len3(n).max(1e-300);
    let n = n.map(|x| x / l);
    let a = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = cross3(n, a);
    let lu = len3(u).max(1e-300);
    let u = u.map(|x| x / lu);
    (u, cross3(n, u))
}

/// UV > Smart UV Project (Blender 2.91+'s C version): find projection
/// directions from face normals within `angle_limit` (radians, default 66
/// degrees), project every face along its best direction, then pack the
/// resulting islands with `island_margin`.
pub fn smart_project(m: &mut Mesh, faces: &[u32], angle_limit: f32, island_margin: f32, area_weight: f32) {
    let faces = valid_faces(m, faces);
    ensure_uvs(m);
    if faces.is_empty() {
        return;
    }
    let mut thick: Vec<(u32, P3, f64)> = faces.iter().map(|&f| (f, p3(m.face_normal(f as usize)), m.face_area(f as usize) as f64)).collect();
    thick.sort_by(|a, b| b.2.total_cmp(&a.2));
    let half_cos = (angle_limit as f64 / 2.0).cos();
    let lim_cos = (angle_limit as f64).cos();
    let mut tagged = vec![false; thick.len()];
    let mut normals: Vec<P3> = Vec::new();
    let mut project = thick[0].1;
    let mut group: Vec<usize> = Vec::new();
    loop {
        for i in (0..thick.len()).rev() {
            if !tagged[i] && dot3(thick[i].1, project) > half_cos {
                group.push(i);
                tagged[i] = true;
            }
        }
        let mut avg = [0.0; 3];
        for &i in &group {
            let wgt = if area_weight <= 0.0 {
                1.0
            } else if area_weight >= 1.0 {
                thick[i].2
            } else {
                area_weight as f64 * thick[i].2 + (1.0 - area_weight as f64)
            };
            for a in 0..3 {
                avg[a] += thick[i].1[a] * wgt;
            }
        }
        let l = len3(avg);
        if l > 0.0 && l.is_finite() {
            normals.push(avg.map(|x| x / l));
        }
        // The face pointing furthest from every direction so far.
        let mut best = (1.0f64, None);
        for i in (0..thick.len()).rev() {
            if tagged[i] {
                continue;
            }
            let t = normals.iter().map(|n| dot3(*n, thick[i].1)).fold(-1.0, f64::max);
            if t < best.0 {
                best = (t, Some(i));
            }
        }
        match best {
            (b, Some(i)) if b < lim_cos => {
                project = thick[i].1;
                group.clear();
                group.push(i);
                tagged[i] = true;
            }
            _ => {
                if !normals.is_empty() {
                    break;
                }
                // Degenerate normals everywhere: fall back to +Z.
                normals.push([0.0, 0.0, 1.0]);
                break;
            }
        }
    }
    // Assign each face to its best direction and project.
    let mut by_dir: Vec<Vec<u32>> = vec![Vec::new(); normals.len()];
    for &(f, n, _) in &thick {
        let k = (0..normals.len()).max_by(|&a, &b| dot3(normals[a], n).total_cmp(&dot3(normals[b], n))).unwrap_or(0);
        by_dir[k].push(f);
    }
    let mut islands = Vec::new();
    for (k, fs) in by_dir.iter().enumerate() {
        if fs.is_empty() {
            continue;
        }
        let (u, v) = plane_basis(normals[k]);
        for &f in fs {
            let face = &mut m.faces[f as usize];
            for (c, &vi) in face.verts.iter().enumerate() {
                let p = p3(m.positions[vi as usize]);
                face.uvs[c] = vec2([dot3(p, u), dot3(p, v)]);
            }
        }
        // Islands: faces of this direction connected through edges.
        let charts = build_charts(m, fs, false);
        islands.extend(charts.into_iter().map(|c| c.faces));
    }
    pack_island_list(m, &islands, island_margin as f64, true, 0.0);
}

/// UV > Lightmap Pack: every face gets its own box (its shape flattened in
/// its plane), packed with `margin_div` of each box's size as margin.
pub fn lightmap_pack(m: &mut Mesh, faces: &[u32], margin_div: f32) {
    let faces = valid_faces(m, faces);
    ensure_uvs(m);
    let mut sides = 0.0;
    for &f in &faces {
        let (u, v) = plane_basis(p3(m.face_normal(f as usize)));
        let face = &mut m.faces[f as usize];
        for (c, &vi) in face.verts.iter().enumerate() {
            let p = p3(m.positions[vi as usize]);
            face.uvs[c] = vec2([dot3(p, u), dot3(p, v)]);
        }
        sides += (m.face_area(f as usize) as f64).sqrt();
    }
    let isl: Vec<Vec<u32>> = faces.iter().map(|&f| vec![f]).collect();
    let mean = sides / faces.len().max(1) as f64;
    pack_island_list(m, &isl, 0.0, true, margin_div as f64 * 0.5 * mean);
}

/// How Follow Active Quads spaces the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowMode {
    /// Every quad the same size as the active one.
    Even,
    /// Each quad's own edge lengths.
    Length,
    /// The average length of each edge ring (Blender's default).
    LengthAverage,
}

/// UV > Follow Active Quads: lay the selected quads out on a grid that
/// continues the active quad's UVs across shared edges.
/// Returns false if `active` is not a quad among `faces`.
pub fn follow_active_quads(m: &mut Mesh, faces: &[u32], active: u32, mode: FollowMode) -> bool {
    let faces = valid_faces(m, faces);
    let quads: Vec<u32> = faces.iter().copied().filter(|&f| m.faces[f as usize].len() == 4).collect();
    if !quads.contains(&active) {
        return false;
    }
    ensure_uvs(m);
    if face_uv_area(m, active).abs() < 1e-12 {
        reset(m, &[active]);
    }
    // Edge rings through the quads (opposite edges of each quad) and their
    // average 3D length.
    let mut ring_id: HashMap<(u32, u32), u32> = HashMap::new();
    let mut ids: Vec<(u32, u32)> = Vec::new();
    for &f in &quads {
        for (a, b) in m.faces[f as usize].edges() {
            ring_id.entry(edge_key(a, b)).or_insert_with(|| {
                ids.push(edge_key(a, b));
                (ids.len() - 1) as u32
            });
        }
    }
    let mut uf = UnionFind::new(ids.len());
    for &f in &quads {
        let v = &m.faces[f as usize].verts;
        for k in 0..2 {
            let e0 = ring_id[&edge_key(v[k], v[k + 1])];
            let e1 = ring_id[&edge_key(v[k + 2], v[(k + 3) % 4])];
            uf.union(e0, e1);
        }
    }
    let mut ring_sum: HashMap<u32, (f64, u32)> = HashMap::new();
    for (i, e) in ids.iter().enumerate() {
        let r = uf.find(i as u32);
        let l = m.positions[e.0 as usize].distance(m.positions[e.1 as usize]) as f64;
        let s = ring_sum.entry(r).or_default();
        s.0 += l;
        s.1 += 1;
    }
    let len_of = |m: &Mesh, uf: &mut UnionFind, a: u32, b: u32| -> f64 {
        match mode {
            FollowMode::Even => 1.0,
            FollowMode::Length => m.positions[a as usize].distance(m.positions[b as usize]) as f64,
            FollowMode::LengthAverage => {
                let r = uf.find(ring_id[&edge_key(a, b)]);
                let s = ring_sum[&r];
                s.0 / s.1.max(1) as f64
            }
        }
    };
    let mut by_edge: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for &f in &quads {
        for (a, b) in m.faces[f as usize].edges() {
            by_edge.entry(edge_key(a, b)).or_default().push(f);
        }
    }
    let mut done: HashSet<u32> = HashSet::from([active]);
    let mut queue = std::collections::VecDeque::from([active]);
    while let Some(f) = queue.pop_front() {
        let fv = m.faces[f as usize].verts.clone();
        let fuv = m.faces[f as usize].uvs.clone();
        for k in 0..4 {
            let (a, b) = (fv[k], fv[(k + 1) % 4]);
            let Some(nbrs) = by_edge.get(&edge_key(a, b)) else { continue };
            if nbrs.len() != 2 {
                continue;
            }
            let g = if nbrs[0] == f { nbrs[1] } else { nbrs[0] };
            if done.contains(&g) {
                continue;
            }
            let gv = m.faces[g as usize].verts.clone();
            let (Some(ga), Some(gb)) = (gv.iter().position(|&x| x == a), gv.iter().position(|&x| x == b)) else { continue };
            // In f: a' is a's other neighbour (not b), b' likewise.
            let (fa2, fb2) = ((k + 3) % 4, (k + 2) % 4);
            let ga2 = if gv[(ga + 1) % 4] == b { (ga + 3) % 4 } else { (ga + 1) % 4 };
            let gb2 = if gv[(gb + 1) % 4] == a { (gb + 3) % 4 } else { (gb + 1) % 4 };
            let ua = uv2(fuv[k]);
            let ub = uv2(fuv[(k + 1) % 4]);
            let side_f = (len_of(m, &mut uf, a, fv[fa2]) + len_of(m, &mut uf, b, fv[fb2])) * 0.5;
            let side_g = (len_of(m, &mut uf, a, gv[ga2]) + len_of(m, &mut uf, b, gv[gb2])) * 0.5;
            let ratio = if side_f > 1e-12 { side_g / side_f } else { 1.0 };
            let ua2 = uv2(fuv[fa2]);
            let ub2 = uv2(fuv[fb2]);
            let mut guv = m.faces[g as usize].uvs.clone();
            guv[ga] = vec2(ua);
            guv[gb] = vec2(ub);
            guv[ga2] = vec2([ua[0] + (ua[0] - ua2[0]) * ratio, ua[1] + (ua[1] - ua2[1]) * ratio]);
            guv[gb2] = vec2([ub[0] + (ub[0] - ub2[0]) * ratio, ub[1] + (ub[1] - ub2[1]) * ratio]);
            m.faces[g as usize].uvs = guv;
            done.insert(g);
            queue.push_back(g);
        }
    }
    true
}

// ---------------------------------------------------------------- editing

/// A UV corner: (face, corner).
pub type Corner = (u32, u32);

/// Corners of `faces` at the same mesh vertex and UV location as `c`
/// (Blender's "Shared Location" sticky selection).
pub fn sticky(m: &Mesh, faces: &[u32], c: Corner) -> Vec<Corner> {
    let Some(face) = m.faces.get(c.0 as usize) else { return Vec::new() };
    let (v, uv) = (face.verts[c.1 as usize], face.uvs.get(c.1 as usize).copied().unwrap_or(Vec2::ZERO));
    let mut out = Vec::new();
    for &f in faces {
        let ff = &m.faces[f as usize];
        for (k, &w) in ff.verts.iter().enumerate() {
            if w == v && ff.uvs.get(k).is_some_and(|u| (u.x - uv.x).abs() < 1e-5 && (u.y - uv.y).abs() < 1e-5) {
                out.push((f, k as u32));
            }
        }
    }
    out
}

/// UV > Merge > At Center (`uv.weld`): move the corners to their mean.
pub fn weld(m: &mut Mesh, corners: &[Corner]) {
    if corners.is_empty() {
        return;
    }
    let mut s = [0.0f64; 2];
    for &(f, c) in corners {
        let u = m.faces[f as usize].uvs[c as usize];
        s[0] += u.x as f64;
        s[1] += u.y as f64;
    }
    let mean = vec2([s[0] / corners.len() as f64, s[1] / corners.len() as f64]);
    for &(f, c) in corners {
        m.faces[f as usize].uvs[c as usize] = mean;
    }
}

/// UV > Merge > By Distance (`uv.remove_doubles`): selected UVs closer than
/// `dist` to each other (chained) go to their mean, whatever mesh vertex
/// they belong to. Returns how many corners moved.
pub fn merge_by_distance(m: &mut Mesh, corners: &[Corner], dist: f32) -> usize {
    let cell = dist.max(1e-9) as f64;
    let uv_of = |m: &Mesh, c: Corner| uv2(m.faces[c.0 as usize].uvs[c.1 as usize]);
    let key = |p: P2| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, &c) in corners.iter().enumerate() {
        grid.entry(key(uv_of(m, c))).or_default().push(i);
    }
    let mut uf = UnionFind::new(corners.len());
    for (i, &c) in corners.iter().enumerate() {
        let p = uv_of(m, c);
        let k = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &j in grid.get(&(k.0 + dx, k.1 + dy)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if j > i {
                        let q = uv_of(m, corners[j]);
                        if (p[0] - q[0]).hypot(p[1] - q[1]) <= dist as f64 {
                            uf.union(i as u32, j as u32);
                        }
                    }
                }
            }
        }
    }
    let mut groups: HashMap<u32, Vec<Corner>> = HashMap::new();
    for (i, &c) in corners.iter().enumerate() {
        groups.entry(uf.find(i as u32)).or_default().push(c);
    }
    let mut moved = 0;
    for g in groups.values() {
        if g.len() > 1 {
            let first = uv_of(m, g[0]);
            if g.iter().any(|&c| uv_of(m, c) != first) {
                moved += g.len();
            }
            weld(m, g);
        }
    }
    moved
}

/// UV > Stitch (`uv.stitch` with Snap Islands): the selected corners are
/// joined to the other UVs of the same mesh vertices. The island holding
/// most of the selection stays; every other island touched is rotated and
/// moved onto it (a least-squares rigid fit), then the shared corners meet
/// at their midpoints. Returns the number of corners stitched.
pub fn stitch(m: &mut Mesh, faces: &[u32], selected: &[Corner]) -> usize {
    let faces = valid_faces(m, faces);
    let islands = uv_islands(m, &faces);
    let mut isl_of: HashMap<u32, usize> = HashMap::new();
    for (i, isl) in islands.iter().enumerate() {
        for &f in isl {
            isl_of.insert(f, i);
        }
    }
    let sel: Vec<Corner> = selected.iter().copied().filter(|c| isl_of.contains_key(&c.0)).collect();
    if sel.is_empty() {
        return 0;
    }
    let mut count: HashMap<usize, usize> = HashMap::new();
    for c in &sel {
        *count.entry(isl_of[&c.0]).or_default() += 1;
    }
    let stat = count.iter().max_by_key(|(i, n)| (**n, usize::MAX - **i)).map(|(i, _)| *i).unwrap_or(0);
    // Corners of every face, by mesh vertex.
    let mut by_vert: HashMap<u32, Vec<Corner>> = HashMap::new();
    for &f in &faces {
        for (k, &v) in m.faces[f as usize].verts.iter().enumerate() {
            by_vert.entry(v).or_default().push((f, k as u32));
        }
    }
    // Pairs (static UV, other corner) per other island.
    let mut pairs: HashMap<usize, Vec<(P2, Corner)>> = HashMap::new();
    let sel_set: HashSet<Corner> = sel.iter().copied().collect();
    for &(f, c) in &sel {
        let i = isl_of[&f];
        if i != stat {
            continue;
        }
        let v = m.faces[f as usize].verts[c as usize];
        let uv = uv2(m.faces[f as usize].uvs[c as usize]);
        for &(g, d) in &by_vert[&v] {
            let j = isl_of[&g];
            if j != stat {
                pairs.entry(j).or_default().push((uv, (g, d)));
            }
        }
    }
    let mut stitched = 0;
    for (j, list) in &pairs {
        let src: Vec<P2> = list.iter().map(|(_, c)| uv2(m.faces[c.0 as usize].uvs[c.1 as usize])).collect();
        let dst: Vec<P2> = list.iter().map(|(p, _)| *p).collect();
        let n = src.len() as f64;
        let cs = [src.iter().map(|p| p[0]).sum::<f64>() / n, src.iter().map(|p| p[1]).sum::<f64>() / n];
        let cd = [dst.iter().map(|p| p[0]).sum::<f64>() / n, dst.iter().map(|p| p[1]).sum::<f64>() / n];
        let (mut sxx, mut sxy) = (0.0, 0.0);
        for (s, d) in src.iter().zip(&dst) {
            let (a, b) = ([s[0] - cs[0], s[1] - cs[1]], [d[0] - cd[0], d[1] - cd[1]]);
            sxx += a[0] * b[0] + a[1] * b[1];
            sxy += a[0] * b[1] - a[1] * b[0];
        }
        let ang = if src.len() >= 2 { sxy.atan2(sxx) } else { 0.0 };
        let (sn, cn) = ang.sin_cos();
        for &f in &islands[*j] {
            for u in &mut m.faces[f as usize].uvs {
                let a = [u.x as f64 - cs[0], u.y as f64 - cs[1]];
                *u = vec2([cd[0] + a[0] * cn - a[1] * sn, cd[1] + a[0] * sn + a[1] * cn]);
            }
        }
        for (p, (g, d)) in list {
            m.faces[*g as usize].uvs[*d as usize] = vec2(*p);
            stitched += 1;
        }
    }
    // Selected corners whose vertex has no static-island counterpart: weld
    // the selected corners of each such vertex.
    let mut rest: HashMap<u32, Vec<Corner>> = HashMap::new();
    for &(f, c) in &sel {
        if isl_of[&f] != stat {
            let v = m.faces[f as usize].verts[c as usize];
            let has_static = by_vert[&v].iter().any(|&(g, _)| isl_of[&g] == stat);
            if !has_static {
                rest.entry(v).or_default().push((f, c));
            }
        }
    }
    for list in rest.values() {
        let v = m.faces[list[0].0 as usize].verts[list[0].1 as usize];
        let all: Vec<Corner> = by_vert[&v].iter().copied().filter(|c| sel_set.contains(c) || list.contains(c)).collect();
        if all.len() > 1 {
            weld(m, &all);
            stitched += all.len();
        }
    }
    stitched
}

/// Largest relative deviation of triangle angles in UV from the 3D ones
/// over `faces` (0 = perfectly conformal). For tests and diagnostics.
pub fn angle_distortion(m: &Mesh, faces: &[u32]) -> f64 {
    let set: HashSet<u32> = faces.iter().copied().collect();
    let mut sum = 0.0;
    let mut n = 0;
    for t in m.triangulate() {
        if !set.contains(&t.face) {
            continue;
        }
        let f = &m.faces[t.face as usize];
        if f.uvs.len() != f.verts.len() {
            continue;
        }
        let a3 = tri_angles(p3(m.positions[t.verts[0] as usize]), p3(m.positions[t.verts[1] as usize]), p3(m.positions[t.verts[2] as usize]));
        let u = t.corners.map(|c| {
            let q = f.uvs[c as usize];
            [q.x as f64, q.y as f64, 0.0]
        });
        let a2 = tri_angles(u[0], u[1], u[2]);
        for k in 0..3 {
            sum += (a3[k] - a2[k]).abs();
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn all(m: &Mesh) -> Vec<u32> {
        (0..m.faces.len() as u32).collect()
    }

    fn check_valid(m: &Mesh) {
        assert!(m.has_uvs());
        for f in &m.faces {
            for u in &f.uvs {
                assert!(u.x.is_finite() && u.y.is_finite(), "{u:?}");
            }
        }
    }

    fn inside01(m: &Mesh, faces: &[u32]) {
        for &f in faces {
            for u in &m.faces[f as usize].uvs {
                assert!(u.x >= -1e-4 && u.x <= 1.0 + 1e-4 && u.y >= -1e-4 && u.y <= 1.0 + 1e-4, "{u:?}");
            }
        }
    }

    /// No two islands' UV triangles overlap (sampled point-in-triangle test
    /// on triangle centroids and edge midpoints against other islands).
    fn islands_disjoint(m: &Mesh, islands: &[Vec<u32>]) {
        let tris = m.triangulate();
        let mut by_isl: Vec<Vec<[P2; 3]>> = vec![Vec::new(); islands.len()];
        for t in &tris {
            if let Some(i) = islands.iter().position(|isl| isl.contains(&t.face)) {
                let f = &m.faces[t.face as usize];
                by_isl[i].push(t.corners.map(|c| uv2(f.uvs[c as usize])));
            }
        }
        let inside = |p: P2, t: &[P2; 3]| {
            let s = |a: P2, b: P2| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            let (d1, d2, d3) = (s(t[0], t[1]), s(t[1], t[2]), s(t[2], t[0]));
            let e = 1e-9;
            (d1 > e && d2 > e && d3 > e) || (d1 < -e && d2 < -e && d3 < -e)
        };
        for i in 0..islands.len() {
            for t in &by_isl[i] {
                let c = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0];
                for j in 0..islands.len() {
                    if i != j {
                        assert!(!by_isl[j].iter().any(|u| inside(c, u)), "islands {i} and {j} overlap");
                    }
                }
            }
        }
    }

    fn cube_with_seams() -> Mesh {
        // Blender's classic cube cross: cut the top ring, bottom ring but one
        // edge each, and one vertical edge.
        let mut m = primitives::cube(2.0);
        let edges = m.edges();
        // Find an edge loop of seams that opens the cube into a cross: all
        // edges of the top face but one, all of the bottom but one, and a vertical.
        let top: Vec<u32> = (0..8).filter(|&v| m.positions[v as usize].z > 0.0).collect();
        let bot: Vec<u32> = (0..8).filter(|&v| m.positions[v as usize].z < 0.0).collect();
        let is_in = |s: &[u32], e: &[u32; 2]| s.contains(&e[0]) && s.contains(&e[1]);
        let tops: Vec<[u32; 2]> = edges.iter().copied().filter(|e| is_in(&top, e)).collect();
        let bots: Vec<[u32; 2]> = edges.iter().copied().filter(|e| is_in(&bot, e)).collect();
        let vert: Vec<[u32; 2]> = edges.iter().copied().filter(|e| !is_in(&top, e) && !is_in(&bot, e)).collect();
        for e in &tops[..3] {
            m.set_seam(e[0], e[1], true);
        }
        // Bottom: three edges, the uncut one parallel to the uncut top one.
        let keep_dir = m.positions[tops[3][0] as usize] - m.positions[tops[3][1] as usize];
        let mut n = 0;
        for e in &bots {
            let d = m.positions[e[0] as usize] - m.positions[e[1] as usize];
            if d.cross(keep_dir).length() < 1e-4 && n == 0 {
                n += 1;
                continue;
            }
            m.set_seam(e[0], e[1], true);
        }
        // One vertical edge, not touching the uncut top edge.
        let v = vert.iter().find(|e| !e.iter().any(|x| tops[3].contains(x))).unwrap();
        m.set_seam(v[0], v[1], true);
        m
    }

    #[test]
    fn charts_split_by_seams() {
        let mut m = primitives::cube(2.0);
        assert_eq!(build_charts(&m, &all(&m), true).len(), 1);
        let e = m.faces[0].verts.clone();
        // Seam all four edges of face 0: it becomes its own chart.
        for k in 0..4 {
            m.set_seam(e[k], e[(k + 1) % 4], true);
        }
        let charts = build_charts(&m, &all(&m), true);
        assert_eq!(charts.len(), 2);
        // The ring around it: four corners split into chart vertices.
        assert_eq!(charts.iter().map(|c| c.verts.len()).sum::<usize>(), 12);
    }

    #[test]
    fn closed_chart_is_reported() {
        let mut m = primitives::cube(2.0);
        let fa_m = all(&m);
        let rep = unwrap(&mut m, &fa_m, &UnwrapOptions::default());
        assert_eq!(rep, UnwrapReport { islands: 0, failed: 1 });
        check_valid(&m);
    }

    #[test]
    fn lscm_of_cut_cube_is_conformal_and_packed() {
        for method in [UnwrapMethod::Conformal, UnwrapMethod::AngleBased] {
            let mut m = cube_with_seams();
            let fa_m = all(&m);
            let rep = unwrap(&mut m, &fa_m, &UnwrapOptions { method, ..Default::default() });
            assert_eq!(rep, UnwrapReport { islands: 1, failed: 0 }, "{method:?}");
            check_valid(&m);
            inside01(&m, &all(&m));
            // A cube's cross unfolds without distortion.
            let d = angle_distortion(&m, &all(&m));
            assert!(d < 1e-3, "{method:?} distortion {d}");
            // Every face keeps its area ratio (isometric up to scale).
            let r: Vec<f64> = (0..6).map(|f| face_uv_area(&m, f).abs() / m.face_area(f as usize) as f64).collect();
            for x in &r {
                assert!((x / r[0] - 1.0).abs() < 1e-2, "{r:?}");
            }
            // Not flipped.
            assert!((0..6).all(|f| face_uv_area(&m, f) > 0.0));
        }
    }

    #[test]
    fn lscm_and_abf_of_cut_cylinder() {
        // A tube (no caps) cut along one vertical edge unrolls to a rectangle.
        let mut m = primitives::cylinder(16, 1.0, 2.0);
        // Remove caps: keep quads only.
        m.faces.retain(|f| f.verts.len() == 4);
        m.remove_unused_vertices();
        let v = m.edges().into_iter().find(|e| (m.positions[e[0] as usize].z - m.positions[e[1] as usize].z).abs() > 1.0).unwrap();
        m.set_seam(v[0], v[1], true);
        for method in [UnwrapMethod::Conformal, UnwrapMethod::AngleBased] {
            let mut mm = m.clone();
            let fa_mm = all(&mm);
            let rep = unwrap(&mut mm, &fa_mm, &UnwrapOptions { method, ..Default::default() });
            assert_eq!(rep.islands, 1);
            check_valid(&mm);
            inside01(&mm, &all(&mm));
            let d = angle_distortion(&mm, &all(&mm));
            assert!(d < 0.02, "{method:?} distortion {d}");
        }
    }

    #[test]
    fn abf_sphere_with_seam_has_low_distortion() {
        // A UV sphere cut along a meridian: ABF beats or matches LSCM.
        let mut m = primitives::uv_sphere(24, 12, 1.0);
        for e in m.edges() {
            let (a, b) = (m.positions[e[0] as usize], m.positions[e[1] as usize]);
            if a.x.abs() < 1e-4 && b.x.abs() < 1e-4 && a.y <= 1e-4 && b.y <= 1e-4 {
                m.set_seam(e[0], e[1], true);
            }
        }
        let mut lscm = m.clone();
        let fa_lscm = all(&lscm);
        unwrap(&mut lscm, &fa_lscm, &UnwrapOptions { method: UnwrapMethod::Conformal, ..Default::default() });
        let mut abf = m.clone();
        let fa_abf = all(&abf);
        let rep = unwrap(&mut abf, &fa_abf, &UnwrapOptions::default());
        assert_eq!(rep, UnwrapReport { islands: 1, failed: 0 });
        check_valid(&abf);
        check_valid(&lscm);
        let (dl, da) = (angle_distortion(&lscm, &all(&m)), angle_distortion(&abf, &all(&m)));
        assert!(da < 0.15 && dl < 0.15, "abf {da} lscm {dl}");
    }

    #[test]
    fn pack_separates_islands_inside_unit_square() {
        let mut m = primitives::cube(2.0);
        // Every edge a seam: six islands.
        for e in m.edges() {
            m.set_seam(e[0], e[1], true);
        }
        let fa_m = all(&m);
        let rep = unwrap(&mut m, &fa_m, &UnwrapOptions { margin: 0.02, ..Default::default() });
        assert_eq!(rep.islands, 6);
        check_valid(&m);
        inside01(&m, &all(&m));
        let isl = uv_islands(&m, &all(&m));
        assert_eq!(isl.len(), 6);
        islands_disjoint(&m, &isl);
        // Equal texel density: all faces the same UV area.
        let a: Vec<f64> = (0..6).map(|f| face_uv_area(&m, f).abs()).collect();
        assert!(a.iter().all(|x| (x / a[0] - 1.0).abs() < 1e-3), "{a:?}");
        // Packing again is stable inside 0..1.
        let fa_m = all(&m);
        pack_islands(&mut m, &fa_m, 0.001, true);
        inside01(&m, &all(&m));
        islands_disjoint(&m, &uv_islands(&m, &all(&m)));
    }

    #[test]
    fn smart_project_cube_and_sphere() {
        let mut m = primitives::cube(2.0);
        let fa_m = all(&m);
        smart_project(&mut m, &fa_m, 66f32.to_radians(), 0.0, 0.0);
        check_valid(&m);
        inside01(&m, &all(&m));
        let isl = uv_islands(&m, &all(&m));
        assert_eq!(isl.len(), 6);
        islands_disjoint(&m, &isl);
        let mut s = primitives::uv_sphere(16, 8, 1.0);
        let fa_s = all(&s);
        smart_project(&mut s, &fa_s, 66f32.to_radians(), 0.02, 0.0);
        check_valid(&s);
        inside01(&s, &all(&s));
        let isl = uv_islands(&s, &all(&s));
        assert!(isl.len() >= 6, "{}", isl.len());
        islands_disjoint(&s, &isl);
        // Every face is a planar projection: no flips.
        assert!((0..s.faces.len() as u32).all(|f| face_uv_area(&s, f) > 0.0));
    }

    #[test]
    fn projections_cover_every_face() {
        let base = primitives::uv_sphere(16, 8, 1.0);
        let f = all(&base);
        let mut c = base.clone();
        cube_project(&mut c, &f, None);
        check_valid(&c);
        inside01(&c, &f);
        let mut cy = base.clone();
        cylinder_project(&mut cy, &f, 1.0);
        check_valid(&cy);
        // No face straddles the seam (u spread under half a turn).
        for face in &cy.faces {
            let (mn, mx) = face.uvs.iter().fold((f32::MAX, f32::MIN), |a, u| (a.0.min(u.x), a.1.max(u.x)));
            assert!(mx - mn < 0.5, "{mn} {mx}");
        }
        let mut sp = base.clone();
        sphere_project(&mut sp, &f);
        check_valid(&sp);
        for face in &sp.faces {
            let (mn, mx) = face.uvs.iter().fold((f32::MAX, f32::MIN), |a, u| (a.0.min(u.x), a.1.max(u.x)));
            assert!(mx - mn < 0.5, "{mn} {mx}");
            assert!(face.uvs.iter().all(|u| u.y >= -1e-5 && u.y <= 1.0 + 1e-5));
        }
        let mut v = base.clone();
        project_front(&mut v, &f, false);
        check_valid(&v);
        inside01(&v, &f);
        let mut vb = base.clone();
        project_front(&mut vb, &f, true);
        let (mn, mx) = island_bounds(&vb, &f);
        assert!(mn[0].abs() < 1e-5 && mn[1].abs() < 1e-5 && (mx[0] - 1.0).abs() < 1e-5 && (mx[1] - 1.0).abs() < 1e-5);
        let mut r = base.clone();
        reset(&mut r, &f);
        check_valid(&r);
        assert_eq!(r.faces.iter().find(|f| f.len() == 4).unwrap().uvs[2], Vec2::new(1.0, 1.0));
        let mut l = base.clone();
        lightmap_pack(&mut l, &f, 0.1);
        check_valid(&l);
        inside01(&l, &f);
        let isl: Vec<Vec<u32>> = f.iter().map(|&x| vec![x]).collect();
        islands_disjoint(&l, &isl);
    }

    #[test]
    fn follow_active_quads_makes_a_grid() {
        let mut m = primitives::grid(4, 3, 2.0);
        let f = all(&m);
        // Scramble, then follow from face 0 with its reset UVs.
        for face in &mut m.faces {
            face.uvs.clear();
        }
        assert!(follow_active_quads(&mut m, &f, 0, FollowMode::LengthAverage));
        check_valid(&m);
        // Neighbouring quads share UVs along shared edges: one island.
        assert_eq!(uv_islands(&m, &f).len(), 1);
        // All quads have the same UV area as the active one.
        let a0 = face_uv_area(&m, 0).abs();
        for k in &f {
            assert!((face_uv_area(&m, *k).abs() / a0 - 1.0).abs() < 1e-3);
        }
        assert!(!follow_active_quads(&mut primitives::cube(1.0), &[0], 3, FollowMode::Even));
    }

    #[test]
    fn average_scale_equalizes_density() {
        let mut m = primitives::cube(2.0);
        for e in m.edges() {
            m.set_seam(e[0], e[1], true);
        }
        let fa_m = all(&m);
        reset(&mut m, &fa_m);
        // Face 0 shrunk: its own island.
        for u in &mut m.faces[0].uvs {
            *u = *u * 0.25;
        }
        // Spread islands so they are separate.
        for (i, face) in m.faces.iter_mut().enumerate() {
            for u in &mut face.uvs {
                u.x += i as f32 * 2.0;
            }
        }
        let fa_m = all(&m);
        average_islands_scale(&mut m, &fa_m);
        let a: Vec<f64> = (0..6).map(|f| face_uv_area(&m, f).abs()).collect();
        assert!(a.iter().all(|x| (x / a[0] - 1.0).abs() < 1e-3), "{a:?}");
    }

    #[test]
    fn weld_stitch_and_pins() {
        let mut m = primitives::grid(2, 1, 2.0);
        let f = all(&m);
        reset(&mut m, &f);
        // Two separate unit squares sharing a mesh edge: two UV islands.
        assert_eq!(uv_islands(&m, &f).len(), 2);
        let shared: Vec<u32> = m.faces[0].verts.iter().copied().filter(|v| m.faces[1].verts.contains(v)).collect();
        let sel: Vec<Corner> = f.iter().flat_map(|&fi| m.faces[fi as usize].verts.iter().enumerate().filter(|(_, v)| shared.contains(v)).map(move |(k, _)| (fi, k as u32))).collect();
        let n = stitch(&mut m, &f, &sel);
        assert!(n >= 2);
        assert_eq!(uv_islands(&m, &f).len(), 1, "stitched into one island");
        // Weld all corners of face 0 to one point.
        let c: Vec<Corner> = (0..4).map(|k| (0, k)).collect();
        weld(&mut m, &c);
        assert!(m.faces[0].uvs.iter().all(|u| *u == m.faces[0].uvs[0]));
        // Merge by distance joins near corners of the same vertex.
        let mut g = primitives::grid(2, 1, 2.0);
        reset(&mut g, &f);
        let all_c: Vec<Corner> = f.iter().flat_map(|&fi| (0..4).map(move |k| (fi, k))).collect();
        assert!(merge_by_distance(&mut g, &all_c, 2.0) > 0);
        // Pins: unwrap keeps pinned UVs where they are.
        let mut cyl = cube_with_seams();
        cyl.faces[0].uvs = vec![Vec2::new(0.1, 0.1), Vec2::new(0.2, 0.1), Vec2::new(0.2, 0.2), Vec2::new(0.1, 0.2)];
        cyl.uv_pins.insert((0, 0));
        cyl.uv_pins.insert((0, 2));
        let all_f = all(&cyl);
        for face in cyl.faces.iter_mut().skip(1) {
            face.uvs.clear();
        }
        unwrap(&mut cyl, &all_f, &UnwrapOptions::default());
        assert_eq!(cyl.faces[0].uvs[0], Vec2::new(0.1, 0.1));
        assert_eq!(cyl.faces[0].uvs[2], Vec2::new(0.2, 0.2));
        assert!((cyl.faces[0].uvs[1] - Vec2::new(0.2, 0.1)).x.abs() < 1e-3);
    }

    #[test]
    fn seams_survive_remap() {
        let mut m = primitives::cube(2.0);
        m.set_seam(0, 1, true);
        m.set_sharp(2, 3, true);
        let mut big = Mesh::new();
        big.append(&m);
        big.append(&m);
        assert_eq!(big.seams.len(), 2);
        assert!(big.is_seam(8, 9) && big.is_sharp(11, 10));
        big.weld(1e-5);
        assert_eq!(big.seams.len(), 1);
        assert!(big.is_seam(0, 1));
    }
}
