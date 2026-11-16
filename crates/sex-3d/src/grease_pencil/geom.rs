// SPDX-License-Identifier: GPL-3.0-only
//! Stroke geometry shared by the tools, Edit Mode and the modifiers.
use super::{GpPoint, GpStroke};
use crate::math::{Vec2, Vec3};

pub fn polyline_length(p: &[Vec3], cyclic: bool) -> f32 {
    let mut l: f32 = p.windows(2).map(|w| w[0].distance(w[1])).sum();
    if cyclic && p.len() > 2 {
        l += p[p.len() - 1].distance(p[0]);
    }
    l
}

/// Cumulative arc length at each point (0 at the first).
pub fn arc_lengths(p: &[GpPoint]) -> Vec<f32> {
    let mut out = Vec::with_capacity(p.len());
    let mut acc = 0.0;
    for (i, q) in p.iter().enumerate() {
        if i > 0 {
            acc += p[i - 1].position.distance(q.position);
        }
        out.push(acc);
    }
    out
}

/// The point at arc length `s` (clamped), interpolated.
pub fn sample_at(p: &[GpPoint], lens: &[f32], s: f32) -> GpPoint {
    if p.len() == 1 || s <= 0.0 {
        return p[0].clone();
    }
    let i = lens.partition_point(|&l| l < s).clamp(1, p.len() - 1);
    let (a, b) = (lens[i - 1], lens[i]);
    let t = if b > a { ((s - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
    p[i - 1].lerp(&p[i], t)
}

/// Resample to `n` points evenly spaced along the stroke.
pub fn resample_count(p: &[GpPoint], n: usize) -> Vec<GpPoint> {
    if p.is_empty() || n == 0 {
        return Vec::new();
    }
    if p.len() == 1 || n == 1 {
        return vec![p[0].clone(); n];
    }
    let lens = arc_lengths(p);
    let total = *lens.last().unwrap();
    (0..n).map(|i| sample_at(p, &lens, total * i as f32 / (n - 1) as f32)).collect()
}

/// Resample at a spacing of about `len` (ends kept).
pub fn resample_length(p: &[GpPoint], len: f32) -> Vec<GpPoint> {
    if p.len() < 2 || len <= 0.0 {
        return p.to_vec();
    }
    let total = polyline_length(&p.iter().map(|q| q.position).collect::<Vec<_>>(), false);
    let n = ((total / len).round() as usize).max(1) + 1;
    resample_count(p, n.min(100_000))
}

fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let l2 = ab.length_squared();
    if l2 <= 1e-20 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Ramer-Douglas-Peucker: indices of the points to keep within `eps`.
pub fn rdp_keep(p: &[Vec3], eps: f32) -> Vec<bool> {
    let n = p.len();
    let mut keep = vec![false; n];
    if n == 0 {
        return keep;
    }
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut best, mut bi) = (-1.0f32, a);
        for i in a + 1..b {
            let d = seg_dist(p[i], p[a], p[b]);
            if d > best {
                best = d;
                bi = i;
            }
        }
        if best > eps {
            keep[bi] = true;
            stack.push((a, bi));
            stack.push((bi, b));
        }
    }
    keep
}

/// Simplify (Adaptive): drop points within `eps` of the simplified line.
pub fn simplify(points: &[GpPoint], eps: f32) -> Vec<GpPoint> {
    let pos: Vec<Vec3> = points.iter().map(|p| p.position).collect();
    let keep = rdp_keep(&pos, eps);
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p.clone()).collect()
}

/// Merge points closer than `dist` (Simplify: Merge).
pub fn merge_by_distance(points: &[GpPoint], dist: f32) -> Vec<GpPoint> {
    let mut out: Vec<GpPoint> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let last = i + 1 == points.len();
        let n = out.len();
        match out.last_mut() {
            Some(q) if q.position.distance(p.position) < dist && !last => {}
            Some(q) if q.position.distance(p.position) < dist && last && n > 1 => *q = p.clone(),
            _ => out.push(p.clone()),
        }
    }
    out
}

/// What smoothing changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothWhat {
    pub position: bool,
    pub radius: bool,
    pub opacity: bool,
}

impl SmoothWhat {
    pub const POSITION: SmoothWhat = SmoothWhat { position: true, radius: false, opacity: false };
    pub const ALL: SmoothWhat = SmoothWhat { position: true, radius: true, opacity: true };
}

/// Laplacian smoothing of the points where `mask(i)` is true, `iterations`
/// times by `factor`; the ends of open strokes stay put.
pub fn smooth(points: &mut [GpPoint], cyclic: bool, factor: f32, iterations: u32, what: SmoothWhat, mask: &dyn Fn(usize) -> f32) {
    let n = points.len();
    if n < 3 || factor <= 0.0 {
        return;
    }
    for _ in 0..iterations.max(1) {
        let old: Vec<(Vec3, f32, f32)> = points.iter().map(|p| (p.position, p.radius, p.opacity)).collect();
        for i in 0..n {
            let (prev, next) = if cyclic {
                ((i + n - 1) % n, (i + 1) % n)
            } else {
                if i == 0 || i == n - 1 {
                    continue;
                }
                (i - 1, i + 1)
            };
            let w = (factor * mask(i)).clamp(0.0, 1.0);
            if w <= 0.0 {
                continue;
            }
            let p = &mut points[i];
            if what.position {
                let avg = (old[prev].0 + old[next].0) * 0.5;
                p.position = old[i].0.lerp(avg, w);
            }
            if what.radius {
                p.radius = old[i].1 + ((old[prev].1 + old[next].1) * 0.5 - old[i].1) * w;
            }
            if what.opacity {
                p.opacity = old[i].2 + ((old[prev].2 + old[next].2) * 0.5 - old[i].2) * w;
            }
        }
    }
}

/// Insert `cuts` evenly spaced points into every segment where `mask(i, i+1)`.
pub fn subdivide(points: &[GpPoint], cyclic: bool, cuts: u32, mask: &dyn Fn(usize, usize) -> bool) -> Vec<GpPoint> {
    let n = points.len();
    if n < 2 || cuts == 0 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(n * (cuts as usize + 1));
    let segs = if cyclic { n } else { n - 1 };
    for i in 0..n {
        out.push(points[i].clone());
        if i < segs {
            let j = (i + 1) % n;
            if mask(i, j) {
                for k in 1..=cuts {
                    let mut q = points[i].lerp(&points[j], k as f32 / (cuts + 1) as f32);
                    q.select = points[i].select && points[j].select;
                    out.push(q);
                }
            }
        }
    }
    out
}

/// Catmull-Rom style smooth subdivision (each level doubles the segments).
pub fn subdivide_smooth(points: &[GpPoint], cyclic: bool, levels: u32) -> Vec<GpPoint> {
    let mut p = points.to_vec();
    for _ in 0..levels {
        let mut q = subdivide(&p, cyclic, 1, &|_, _| true);
        let n = q.len();
        // Relax the old points towards their new neighbours (Chaikin-like corner cutting).
        let orig = q.clone();
        for i in 0..n {
            if i % 2 == 0 && (cyclic || (i > 0 && i + 1 < n)) {
                let a = &orig[(i + n - 1) % n];
                let b = &orig[(i + 1) % n];
                q[i].position = orig[i].position * 0.5 + (a.position + b.position) * 0.25;
            }
        }
        p = q;
    }
    p
}

// ---------------------------------------------------------------- 2D

pub fn signed_area(p: &[Vec2]) -> f32 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let (u, v) = (p[i], p[(i + 1) % n]);
        a += u.x * v.y - v.x * u.y;
    }
    a * 0.5
}

fn cross2(o: Vec2, a: Vec2, b: Vec2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn in_tri(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let (d1, d2, d3) = (cross2(a, b, p), cross2(b, c, p), cross2(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Ear-clipping triangulation of a simple polygon (no holes). Returns index
/// triples into `p`. Self-intersecting input still yields triangles (the
/// remaining polygon is fanned when no ear is found).
pub fn triangulate(p: &[Vec2]) -> Vec<[u32; 3]> {
    let n = p.len();
    if n < 3 {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    if signed_area(p) < 0.0 {
        idx.reverse();
    }
    // Drop exact duplicates that would make zero-area ears.
    idx.dedup_by(|a, b| p[*a].x == p[*b].x && p[*a].y == p[*b].y);
    let mut tris = Vec::with_capacity(n);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n + 10 {
        guard += 1;
        let m = idx.len();
        let mut found = false;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (p[ia], p[ib], p[ic]);
            if cross2(a, b, c) <= 1e-12 {
                continue;
            }
            let blocked = idx.iter().any(|&j| j != ia && j != ib && j != ic && in_tri(p[j], a, b, c));
            if blocked {
                continue;
            }
            tris.push([ia as u32, ib as u32, ic as u32]);
            idx.remove(k);
            found = true;
            break;
        }
        if !found {
            // Degenerate or self-intersecting: fan what is left.
            for k in 1..idx.len() - 1 {
                tris.push([idx[0] as u32, idx[k] as u32, idx[k + 1] as u32]);
            }
            return tris;
        }
    }
    if idx.len() == 3 {
        tris.push([idx[0] as u32, idx[1] as u32, idx[2] as u32]);
    }
    tris
}

/// Newell normal of a 3D polygon.
pub fn polygon_normal(p: &[Vec3]) -> Vec3 {
    let mut nrm = Vec3::ZERO;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        nrm += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    nrm
}

/// Triangulate a (roughly planar) 3D polygon by projecting it onto its plane.
pub fn triangulate_3d(p: &[Vec3]) -> Vec<[u32; 3]> {
    let n = polygon_normal(p);
    let n = if n.length() < 1e-12 { Vec3::Z } else { n.normalize() };
    let u = n.any_orthonormal();
    let v = n.cross(u);
    let flat: Vec<Vec2> = p.iter().map(|q| Vec2::new(q.dot(u), q.dot(v))).collect();
    triangulate(&flat)
}

/// Even-odd point in polygon.
pub fn point_in_polygon(pt: Vec2, poly: &[Vec2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > pt.y) != (b.y > pt.y) && pt.x < (b.x - a.x) * (pt.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Distance from `p` to segment `a`-`b` and the parameter of the closest point.
pub fn seg_dist2(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let (abx, aby) = (b.x - a.x, b.y - a.y);
    let l2 = abx * abx + aby * aby;
    let t = if l2 > 1e-12 { (((p.x - a.x) * abx + (p.y - a.y) * aby) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.x + abx * t, a.y + aby * t);
    (((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt(), t)
}

/// Remove degenerate strokes (fewer than one point) and split strokes where
/// `cut[i]` is set (the point is removed): erasing in Point mode.
pub fn split_where(s: &GpStroke, cut: &[bool]) -> Vec<GpStroke> {
    let n = s.points.len();
    let mut runs: Vec<Vec<GpPoint>> = Vec::new();
    let mut cur: Vec<GpPoint> = Vec::new();
    for i in 0..n {
        if cut[i] {
            if !cur.is_empty() {
                runs.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(s.points[i].clone());
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    if s.cyclic && runs.len() > 1 && !cut[0] && !cut[n - 1] {
        // The run through the seam joins its ends.
        let first = runs.remove(0);
        runs.last_mut().unwrap().extend(first);
    }
    let all_kept = runs.len() == 1 && runs[0].len() == n;
    runs.into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| GpStroke { points: r, cyclic: s.cyclic && all_kept, ..s.clone() })
        .collect()
}
