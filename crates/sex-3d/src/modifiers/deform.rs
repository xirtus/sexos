// SPDX-License-Identifier: GPL-3.0-only
//! Deform modifiers: they move vertices and keep the topology.
use super::bvh::TriBvh;
use super::{adjacency, ekey, CastType, ShrinkwrapMethod, SimpleDeformMode};
use crate::math::Vec3;
use crate::mesh::Mesh;
use std::collections::HashMap;

fn set(p: &mut Vec3, axis: usize, v: f32) {
    match axis {
        0 => p.x = v,
        1 => p.y = v,
        _ => p.z = v,
    }
}

// ---------------------------------------------------------------------------
// Simple Deform

/// Blender's Simple Deform in object space. Coordinates are relabelled so the
/// deform axis is "w" (Z for axis Z, then X, Y cyclically). Twist rotates
/// around w by `factor * w`; Taper scales across w by `1 + factor * w`;
/// Stretch scales w by `1 + factor` and the rest by `factor * w^2 - factor +
/// 1`; Bend curls the u-v plane around w with radius `1 / factor`. The
/// driving coordinate is clamped to `limits` (fractions of the bounds) and
/// geometry past the limits continues straight.
pub fn simple_deform(m: &Mesh, mode: SimpleDeformMode, axis: u8, factor: f32, limits: [f32; 2], lock: [bool; 3]) -> Mesh {
    let mut out = m.clone();
    if factor == 0.0 || !factor.is_finite() || m.positions.is_empty() {
        return out;
    }
    let axis = (axis as usize).min(2);
    let (iu, iv, iw) = match axis {
        0 => (1, 2, 0),
        1 => (2, 0, 1),
        _ => (0, 1, 2),
    };
    // Bend's driving coordinate is u (along the curl), the others' is w.
    let drive = if mode == SimpleDeformMode::Bend { iu } else { iw };
    let b = m.bounds();
    let (l0, l1) = (limits[0].clamp(0.0, 1.0), limits[1].clamp(0.0, 1.0));
    let (l0, l1) = (l0.min(l1), l0.max(l1));
    let span = b.max[drive] - b.min[drive];
    let (lo, hi) = (b.min[drive] + span * l0, b.min[drive] + span * l1);
    for (p, &orig) in out.positions.iter_mut().zip(&m.positions) {
        let (mut u, mut v, mut w) = (orig[iu], orig[iv], orig[iw]);
        match mode {
            SimpleDeformMode::Twist => {
                let th = w.clamp(lo, hi) * factor;
                let (s, c) = th.sin_cos();
                (u, v) = (u * c - v * s, u * s + v * c);
            }
            SimpleDeformMode::Taper => {
                let k = 1.0 + w.clamp(lo, hi) * factor;
                u *= k;
                v *= k;
            }
            SimpleDeformMode::Stretch => {
                let wc = w.clamp(lo, hi);
                let k = wc * wc * factor - factor + 1.0;
                u *= k;
                v *= k;
                w = wc * (1.0 + factor) + (w - wc);
            }
            SimpleDeformMode::Bend => {
                let uc = u.clamp(lo, hi);
                let th = uc * factor;
                let r = 1.0 / factor;
                let (s, c) = th.sin_cos();
                let (nu, nv) = (-(v - r) * s, (v - r) * c + r);
                let rest = u - uc;
                (u, v) = (nu + rest * c, nv + rest * s);
            }
        }
        let mut q = orig;
        set(&mut q, iu, u);
        set(&mut q, iv, v);
        set(&mut q, iw, w);
        for a in 0..3 {
            if lock[a] && a != axis {
                set(&mut q, a, orig[a]);
            }
        }
        *p = q;
    }
    out
}

// ---------------------------------------------------------------------------
// Cast

/// Pull vertices towards a sphere, a cylinder (around Z) or a box centred on
/// the object origin, by `factor` (negative pushes away). The target size is
/// `size`, or `radius` when `size_from_radius`, or else the average distance
/// (sphere/cylinder) or largest extent per axis (box) of the affected
/// vertices. `radius` > 0 leaves vertices farther than it alone.
pub fn cast(m: &Mesh, kind: CastType, factor: f32, radius: f32, size: f32, axes: [bool; 3], size_from_radius: bool) -> Mesh {
    let mut out = m.clone();
    if factor == 0.0 || !axes.iter().any(|&a| a) {
        return out;
    }
    let flat = |p: Vec3| if kind == CastType::Cylinder { Vec3::new(p.x, p.y, 0.0) } else { p };
    let affected: Vec<usize> = (0..m.positions.len()).filter(|&i| radius <= 0.0 || flat(m.positions[i]).length() <= radius).collect();
    if affected.is_empty() {
        return out;
    }
    let fixed = if size > 0.0 {
        Some(size)
    } else if size_from_radius && radius > 0.0 {
        Some(radius)
    } else {
        None
    };
    let mask = |d: Vec3| Vec3::new(if axes[0] { d.x } else { 0.0 }, if axes[1] { d.y } else { 0.0 }, if axes[2] { d.z } else { 0.0 });
    match kind {
        CastType::Sphere | CastType::Cylinder => {
            let len = fixed.unwrap_or_else(|| affected.iter().map(|&i| flat(m.positions[i]).length()).sum::<f32>() / affected.len() as f32);
            for &i in &affected {
                let v = flat(m.positions[i]);
                let l = v.length();
                if l <= 0.0 {
                    continue;
                }
                out.positions[i] += mask((v * (len / l) - v) * factor);
            }
        }
        CastType::Cuboid => {
            let half = match fixed {
                Some(s) => Vec3::splat(s),
                None => affected.iter().fold(Vec3::ZERO, |a, &i| a.max(m.positions[i].abs())),
            };
            for &i in &affected {
                let v = m.positions[i];
                let t = (0..3).filter(|&a| half[a] > 0.0).map(|a| v[a].abs() / half[a]).fold(0.0f32, f32::max);
                if t <= 0.0 {
                    continue;
                }
                out.positions[i] += mask((v / t - v) * factor);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Wave

pub struct WaveParams {
    pub x: bool,
    pub y: bool,
    pub cyclic: bool,
    pub normals: bool,
    pub falloff: f32,
    pub height: f32,
    pub width: f32,
    pub narrowness: f32,
    pub speed: f32,
    pub time_offset: f32,
    pub lifetime: f32,
    pub damping: f32,
    pub start: [f32; 2],
}

/// Blender's Wave: a Gaussian bump `width` wide travels out from `start` at
/// `speed` units per frame (radially with both X and Y), lifting vertices
/// along Z (or their normals) by up to `height`; after `lifetime` frames it
/// dies out over `damping` frames.
pub fn wave(m: &Mesh, frame: f32, w: &WaveParams) -> Mesh {
    let mut out = m.clone();
    if !(w.x || w.y) || w.width <= 0.0 {
        return out;
    }
    let damp = if w.damping == 0.0 { 10.0 } else { w.damping };
    let mut life = w.height;
    if w.lifetime != 0.0 {
        let x = frame - w.time_offset;
        if x > w.lifetime {
            let l = x - w.lifetime;
            life = if l > damp { 0.0 } else { w.height * (1.0 - (l / damp).sqrt()) };
        }
    }
    if life == 0.0 {
        return out;
    }
    let ctime = (frame - w.time_offset) * w.speed;
    let wn = w.width * w.narrowness;
    let minfac = 1.0 / (wn * wn).exp();
    let normals = if w.normals { m.vertex_normals() } else { Vec::new() };
    for (i, p) in out.positions.iter_mut().enumerate() {
        let (x, y) = (p.x - w.start[0], p.y - w.start[1]);
        let dist = if w.x && w.y { (x * x + y * y).sqrt() } else if w.x { x } else { y };
        let falloff = if w.falloff > 0.0 { (1.0 - dist.abs() / w.falloff).clamp(0.0, 1.0) } else { 1.0 };
        let mut amp = dist - ctime;
        if w.cyclic {
            // fmod keeps the sign, like C's fmodf.
            amp = (amp - w.width) % (2.0 * w.width) + w.width;
        }
        if amp > -w.width && amp < w.width {
            let a = amp * w.narrowness;
            let h = (1.0 / (a * a).exp() - minfac) * life * falloff;
            if w.normals {
                *p += normals[i] * h;
            } else {
                p.z += h;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Shrinkwrap

/// Nearest-vertex lookups through a uniform hash grid.
struct PointGrid<'a> {
    pts: &'a [Vec3],
    cell: f32,
    map: HashMap<(i32, i32, i32), Vec<u32>>,
}

impl<'a> PointGrid<'a> {
    fn new(pts: &'a [Vec3]) -> PointGrid<'a> {
        let mut b = crate::math::Aabb::EMPTY;
        pts.iter().for_each(|&p| b.grow(p));
        let vol = b.size().max(Vec3::splat(1e-6));
        let cell = ((vol.x * vol.y * vol.z) / pts.len().max(1) as f32).cbrt().max(1e-6);
        let mut g = PointGrid { pts, cell, map: HashMap::new() };
        for (i, &p) in pts.iter().enumerate() {
            g.map.entry(g.key(p)).or_default().push(i as u32);
        }
        g
    }
    fn key(&self, p: Vec3) -> (i32, i32, i32) {
        let f = |v: f32| (v / self.cell).floor().clamp(-1e9, 1e9) as i32;
        (f(p.x), f(p.y), f(p.z))
    }
    fn nearest(&self, p: Vec3) -> Option<Vec3> {
        let k = self.key(p);
        let mut best: Option<(f32, Vec3)> = None;
        for r in 0..=4096i32 {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                            continue;
                        }
                        if let Some(list) = self.map.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                            for &i in list {
                                let d = (self.pts[i as usize] - p).length_squared();
                                if best.is_none_or(|b| d < b.0) {
                                    best = Some((d, self.pts[i as usize]));
                                }
                            }
                        }
                    }
                }
            }
            // Anything in a farther ring is at least r cells away.
            if let Some((d, q)) = best {
                if d.sqrt() <= r as f32 * self.cell {
                    return Some(q);
                }
            }
            if r > 8 && best.is_some() {
                return best.map(|b| b.1);
            }
        }
        best.map(|b| b.1)
    }
}

/// Move every vertex onto `target` (already in this object's space): to the
/// nearest surface point, along its normal or the chosen axes (Project), or
/// to the nearest target vertex. `offset` keeps that distance from the
/// surface on the side the vertex came from.
pub fn shrinkwrap(m: &Mesh, target: &Mesh, method: ShrinkwrapMethod, offset: f32, axes: [bool; 3], positive: bool, negative: bool) -> Mesh {
    let mut out = m.clone();
    match method {
        ShrinkwrapMethod::NearestVertex => {
            if target.positions.is_empty() {
                return out;
            }
            let g = PointGrid::new(&target.positions);
            for p in &mut out.positions {
                if let Some(q) = g.nearest(*p) {
                    let d = *p - q;
                    *p = q + if offset != 0.0 { d.normalize() * offset } else { Vec3::ZERO };
                }
            }
        }
        ShrinkwrapMethod::NearestSurfacePoint => {
            let bvh = TriBvh::new(target);
            for p in &mut out.positions {
                if let Some(h) = bvh.nearest(*p) {
                    let side = if (*p - h.point).dot(h.normal) < 0.0 { -1.0 } else { 1.0 };
                    *p = h.point + h.normal * (offset * side);
                }
            }
        }
        ShrinkwrapMethod::Project => {
            let bvh = TriBvh::new(target);
            let normals = m.vertex_normals();
            let axis_dir = Vec3::new(axes[0] as u8 as f32, axes[1] as u8 as f32, axes[2] as u8 as f32);
            let far = {
                let b = m.bounds().union(target.bounds());
                b.size().length() * 4.0 + 1.0
            };
            for (i, p) in out.positions.iter_mut().enumerate() {
                let dir = if axis_dir == Vec3::ZERO { normals[i] } else { axis_dir.normalize() };
                if dir == Vec3::ZERO {
                    continue;
                }
                let mut best: Option<crate::modifiers::bvh::Hit> = None;
                for (on, d) in [(positive, dir), (negative, -dir)] {
                    if !on {
                        continue;
                    }
                    if let Some(h) = bvh.ray(*p, d, far) {
                        if best.is_none_or(|b| h.dist < b.dist) {
                            best = Some(h);
                        }
                    }
                }
                if let Some(h) = best {
                    let side = if (*p - h.point).dot(h.normal) < 0.0 { -1.0 } else { 1.0 };
                    *p = h.point + h.normal * (offset * side);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Laplacian Smooth

/// Cotangent weight of every edge (`(a, b)` with a < b), clamped positive.
fn cotan_weights(m: &Mesh, pos: &[Vec3]) -> HashMap<(u32, u32), f32> {
    let mut w: HashMap<(u32, u32), f32> = HashMap::new();
    for t in m.triangulate() {
        for c in 0..3 {
            let (i, j, k) = (t.verts[c], t.verts[(c + 1) % 3], t.verts[(c + 2) % 3]);
            let (a, b) = (pos[i as usize] - pos[k as usize], pos[j as usize] - pos[k as usize]);
            let s = a.cross(b).length();
            let cot = if s > 1e-12 { a.dot(b) / s } else { 0.0 };
            *w.entry(ekey(i, j)).or_default() += 0.5 * cot;
        }
    }
    for v in w.values_mut() {
        *v = v.clamp(1e-3, 1e3);
    }
    w
}

fn boundary_edges(m: &Mesh) -> HashMap<(u32, u32), u32> {
    let mut c: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            *c.entry(ekey(a, b)).or_default() += 1;
        }
    }
    c.retain(|_, n| *n == 1);
    c
}

/// Blender's Laplacian Smooth: each repeat solves `(I - λL) x' = x`
/// (approximately, by Jacobi sweeps) with cotangent weights, normalised per
/// vertex when `normalized`. Boundary vertices smooth along the boundary only,
/// with `border` as their λ. `preserve_volume` rescales about the centroid so
/// the enclosed volume stays the same.
pub fn laplacian_smooth(m: &Mesh, iterations: u32, factor: f32, border: f32, axes: [bool; 3], preserve_volume: bool, normalized: bool) -> Mesh {
    let mut out = m.clone();
    if iterations == 0 || m.faces.is_empty() || !axes.iter().any(|&a| a) {
        return out;
    }
    let nv = m.positions.len();
    let bnd = boundary_edges(m);
    let mut on_boundary = vec![false; nv];
    for &(a, b) in bnd.keys() {
        on_boundary[a as usize] = true;
        on_boundary[b as usize] = true;
    }
    let v0 = m.volume();
    for _ in 0..iterations.min(1000) {
        let x = out.positions.clone();
        let w = cotan_weights(m, &x);
        // Per-vertex neighbour lists with weights.
        let mut nb: Vec<Vec<(u32, f32)>> = vec![Vec::new(); nv];
        for (&(a, b), &wt) in &w {
            let edge_on_boundary = bnd.contains_key(&(a, b));
            for (p, q) in [(a, b), (b, a)] {
                if on_boundary[p as usize] && !edge_on_boundary {
                    continue;
                }
                let wt = if on_boundary[p as usize] { 1.0 / x[a as usize].distance(x[b as usize]).max(1e-6) } else { wt };
                nb[p as usize].push((q, wt));
            }
        }
        if normalized {
            for list in &mut nb {
                let s: f32 = list.iter().map(|e| e.1).sum();
                if s > 0.0 {
                    list.iter_mut().for_each(|e| e.1 /= s);
                }
            }
        }
        let lambda: Vec<f32> = (0..nv).map(|v| if on_boundary[v] { border } else { factor }).collect();
        let mut y = x.clone();
        for _ in 0..12 {
            let prev = y.clone();
            for v in 0..nv {
                if nb[v].is_empty() {
                    continue;
                }
                let (s, ws) = nb[v].iter().fold((Vec3::ZERO, 0.0), |(s, ws), &(u, wt)| (s + prev[u as usize] * wt, ws + wt));
                let l = lambda[v];
                let denom = 1.0 + l * ws;
                if denom.abs() > 1e-6 {
                    y[v] = (x[v] + s * l) / denom;
                }
            }
        }
        for v in 0..nv {
            for a in 0..3 {
                if !axes[a] {
                    set(&mut y[v], a, x[v][a]);
                }
            }
            if !(y[v].x.is_finite() && y[v].y.is_finite() && y[v].z.is_finite()) {
                y[v] = x[v];
            }
        }
        out.positions = y;
        if preserve_volume {
            let v1 = out.volume();
            if v0.abs() > 1e-12 && v1.abs() > 1e-12 && v0.signum() == v1.signum() {
                let k = (v0 / v1).cbrt();
                let c = out.positions.iter().fold(Vec3::ZERO, |a, &p| a + p) / nv as f32;
                for p in &mut out.positions {
                    *p = c + (*p - c) * k;
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Corrective Smooth

fn smooth_positions(pos: &[Vec3], start: &[usize], adj: &[u32], factor: f32, iterations: u32, length_weighted: bool) -> Vec<Vec3> {
    let mut x = pos.to_vec();
    for _ in 0..iterations {
        let prev = x.clone();
        for v in 0..x.len() {
            let nb = &adj[start[v]..start[v + 1]];
            if nb.is_empty() {
                continue;
            }
            let (s, ws) = nb.iter().fold((Vec3::ZERO, 0.0f32), |(s, ws), &u| {
                let wt = if length_weighted { 1.0 / prev[u as usize].distance(prev[v]).max(1e-6) } else { 1.0 };
                (s + prev[u as usize] * wt, ws + wt)
            });
            x[v] = prev[v] + (s / ws - prev[v]) * factor;
        }
    }
    x
}

/// Tangent frame (tangent, bitangent, normal) at every vertex of `pos`.
fn frames(m: &Mesh, pos: &[Vec3], start: &[usize], adj: &[u32]) -> Vec<[Vec3; 3]> {
    let mut tmp = m.clone();
    tmp.positions = pos.to_vec();
    let normals = tmp.vertex_normals();
    (0..pos.len())
        .map(|v| {
            let n = if normals[v] == Vec3::ZERO { Vec3::Z } else { normals[v] };
            let t = adj[start[v]..start[v + 1]]
                .first()
                .map(|&u| {
                    let d = pos[u as usize] - pos[v];
                    (d - n * d.dot(n)).normalize()
                })
                .filter(|t| *t != Vec3::ZERO)
                .unwrap_or_else(|| n.any_orthonormal());
            [t, n.cross(t), n]
        })
        .collect()
}

/// Smooth the deformed mesh, then add back the detail the same smoothing
/// removes from the rest shape, expressed in each vertex's tangent frame
/// (Blender's Corrective Smooth with "Original Coordinates" as rest).
pub fn corrective_smooth(m: &Mesh, rest: &[Vec3], factor: f32, iterations: u32, scale: f32, length_weighted: bool, only_smooth: bool) -> Mesh {
    let mut out = m.clone();
    if iterations == 0 || factor == 0.0 {
        return out;
    }
    let (start, adj) = adjacency(m);
    let iterations = iterations.min(10_000);
    let def_s = smooth_positions(&m.positions, &start, &adj, factor, iterations, length_weighted);
    if only_smooth {
        out.positions = def_s;
        return out;
    }
    let rest_s = smooth_positions(rest, &start, &adj, factor, iterations, length_weighted);
    let fr = frames(m, &rest_s, &start, &adj);
    let fd = frames(m, &def_s, &start, &adj);
    for v in 0..m.positions.len() {
        let d = rest[v] - rest_s[v];
        let local = [d.dot(fr[v][0]), d.dot(fr[v][1]), d.dot(fr[v][2])];
        let back = fd[v][0] * local[0] + fd[v][1] * local[1] + fd[v][2] * local[2];
        out.positions[v] = def_s[v] + back * scale;
    }
    out
}
