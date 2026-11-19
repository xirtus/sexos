// SPDX-License-Identifier: GPL-3.0-only
//! Remesh: rebuild a surface on a regular grid.
//!
//! The input is sampled into a signed distance grid: unsigned distance to the
//! nearest triangle in a narrow band, signed by the winding number along +X
//! scanlines (so overlapping closed parts union). Open meshes, which have no
//! inside, become a thin shell around the surface instead. Voxel and Smooth
//! extract the zero level with surface nets (one vertex per cell the surface
//! crosses, one quad per crossed grid edge), which gives a closed quad mesh;
//! Blocks keeps whole voxels and emits their outer faces.
use super::bvh::closest_on_triangle;
use super::RemeshMode;
use crate::math::Vec3;
use crate::mesh::{Face, Mesh};
use std::collections::HashMap;

/// Grid points above this make the voxels coarser (about 160^3), so a tiny
/// voxel size cannot exhaust memory.
pub const MAX_GRID_POINTS: usize = 4_000_000;

struct Grid {
    n: [usize; 3],
    origin: Vec3,
    h: f32,
}

impl Grid {
    fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.n[0] * (j + self.n[1] * k)
    }
    fn point(&self, i: usize, j: usize, k: usize) -> Vec3 {
        self.origin + Vec3::new(i as f32, j as f32, k as f32) * self.h
    }
    fn len(&self) -> usize {
        self.n[0] * self.n[1] * self.n[2]
    }
}

pub fn remesh(m: &Mesh, mode: RemeshMode, voxel_size: f32, depth: u32, scale: f32, smooth: bool) -> Mesh {
    let b = m.bounds();
    if m.faces.is_empty() || b.is_empty() {
        return m.clone();
    }
    let size = b.size();
    let maxd = size.max_elem().max(1e-6);
    let mut h = match mode {
        RemeshMode::Voxel => {
            if voxel_size.is_finite() && voxel_size > 0.0 {
                voxel_size
            } else {
                maxd / 32.0
            }
        }
        _ => maxd / (scale.clamp(0.01, 1.0) * (1u32 << depth.clamp(1, 12)) as f32),
    };
    h = h.max(maxd * 1e-4);
    let pad = if mode == RemeshMode::Blocks { 1.0 } else { 2.0 };
    let dims = |h: f32| [0, 1, 2].map(|a| (size[a] / h).ceil() as usize + 2 * pad as usize + 1);
    let mut n = dims(h);
    while n.iter().product::<usize>() > MAX_GRID_POINTS {
        h *= 1.25;
        n = dims(h);
    }
    // A small irrational shift keeps grid points off axis-aligned faces.
    let jitter = Vec3::new(0.013_7, 0.007_1, 0.011_3) * h;
    let mut grid = Grid { n, origin: b.min - Vec3::splat(pad * h) - jitter, h };
    if mode == RemeshMode::Blocks {
        // Points are voxel centres.
        grid.origin += Vec3::splat(0.5 * h);
    }
    let sdf = sample(m, &grid);
    let mut out = match mode {
        RemeshMode::Blocks => blocks(&grid, &sdf),
        _ => surface_nets(&grid, &sdf),
    };
    let material = m.faces.first().map_or(0, |f| f.material);
    for f in &mut out.faces {
        f.smooth = smooth;
        f.material = material;
    }
    out
}

/// True when every edge is shared by exactly two faces.
fn is_closed(m: &Mesh) -> bool {
    let mut count: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            *count.entry(super::ekey(a, b)).or_default() += 1;
        }
    }
    !count.is_empty() && count.values().all(|&c| c == 2)
}

fn sample(m: &Mesh, g: &Grid) -> Vec<f32> {
    let tris: Vec<[Vec3; 3]> = m.triangulate().iter().map(|t| t.verts.map(|v| m.positions[v as usize])).collect();
    let band = 3.0 * g.h;
    let mut dist = vec![band; g.len()];
    let to_idx = |v: f32, a: usize| (v - g.origin[a]) / g.h;
    for t in &tris {
        let lo = t[0].min(t[1]).min(t[2]);
        let hi = t[0].max(t[1]).max(t[2]);
        let r = |a: usize| {
            let l = (to_idx(lo[a], a) - 2.0).floor().max(0.0) as usize;
            let u = ((to_idx(hi[a], a) + 2.0).ceil().max(0.0) as usize).min(g.n[a] - 1);
            l..=u
        };
        for k in r(2) {
            for j in r(1) {
                for i in r(0) {
                    let p = g.point(i, j, k);
                    let d = (closest_on_triangle(p, t[0], t[1], t[2]) - p).length();
                    let idx = g.idx(i, j, k);
                    if d < dist[idx] {
                        dist[idx] = d;
                    }
                }
            }
        }
    }
    if !is_closed(m) {
        let shell = 0.75 * g.h;
        return dist.into_iter().map(|d| d - shell).collect();
    }
    // Winding number along +X for every (y, z) scanline.
    let (ny, nz) = (g.n[1], g.n[2]);
    let mut lines: Vec<Vec<(f32, i32)>> = vec![Vec::new(); ny * nz];
    for t in &tris {
        let (a, b, c) = (t[0], t[1], t[2]);
        let d = (b.y - a.y) * (c.z - a.z) - (b.z - a.z) * (c.y - a.y);
        if d.abs() < 1e-20 {
            continue;
        }
        let lo = a.min(b).min(c);
        let hi = a.max(b).max(c);
        let j0 = to_idx(lo.y, 1).ceil().max(0.0) as usize;
        let j1 = (to_idx(hi.y, 1).floor().max(-1.0) + 1.0) as usize;
        let k0 = to_idx(lo.z, 2).ceil().max(0.0) as usize;
        let k1 = (to_idx(hi.z, 2).floor().max(-1.0) + 1.0) as usize;
        for k in k0..k1.min(nz) {
            for j in j0..j1.min(ny) {
                let p = g.point(0, j, k);
                // Barycentric coordinates of (y, z) in the projected triangle.
                let w1 = ((p.y - a.y) * (c.z - a.z) - (p.z - a.z) * (c.y - a.y)) / d;
                let w2 = ((b.y - a.y) * (p.z - a.z) - (b.z - a.z) * (p.y - a.y)) / d;
                let w0 = 1.0 - w1 - w2;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let x = a.x * w0 + b.x * w1 + c.x * w2;
                // Normal facing -X: the ray enters here.
                lines[j + ny * k].push((x, if d < 0.0 { 1 } else { -1 }));
            }
        }
    }
    let mut sdf = dist;
    for k in 0..nz {
        for j in 0..ny {
            let line = &mut lines[j + ny * k];
            line.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut w = 0;
            let mut c = 0;
            for i in 0..g.n[0] {
                let x = g.origin.x + i as f32 * g.h;
                while c < line.len() && line[c].0 < x {
                    w += line[c].1;
                    c += 1;
                }
                if w > 0 {
                    let idx = g.idx(i, j, k);
                    sdf[idx] = -sdf[idx];
                }
            }
        }
    }
    sdf
}

/// Surface nets on the grid's zero level; quads wind counter-clockwise seen
/// from outside (where the field is positive).
fn surface_nets(g: &Grid, sdf: &[f32]) -> Mesh {
    let [nx, ny, nz] = g.n;
    let (cx, cy) = (nx - 1, ny - 1);
    let cell = |i: usize, j: usize, k: usize| i + cx * (j + cy * k);
    let mut cell_vert = vec![u32::MAX; cx * cy * (nz - 1)];
    let mut out = Mesh::new();
    const CORNERS: [[usize; 3]; 8] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]];
    const EDGES: [[usize; 2]; 12] = [[0, 1], [2, 3], [4, 5], [6, 7], [0, 2], [1, 3], [4, 6], [5, 7], [0, 4], [1, 5], [2, 6], [3, 7]];
    for k in 0..nz - 1 {
        for j in 0..ny - 1 {
            for i in 0..nx - 1 {
                let v = CORNERS.map(|c| sdf[g.idx(i + c[0], j + c[1], k + c[2])]);
                let neg = v.iter().filter(|&&x| x < 0.0).count();
                if neg == 0 || neg == 8 {
                    continue;
                }
                let mut sum = Vec3::ZERO;
                let mut cnt = 0;
                for e in EDGES {
                    let (a, b) = (v[e[0]], v[e[1]]);
                    if (a < 0.0) != (b < 0.0) {
                        let t = (a / (a - b)).clamp(0.0, 1.0);
                        let pa = Vec3::new(CORNERS[e[0]][0] as f32, CORNERS[e[0]][1] as f32, CORNERS[e[0]][2] as f32);
                        let pb = Vec3::new(CORNERS[e[1]][0] as f32, CORNERS[e[1]][1] as f32, CORNERS[e[1]][2] as f32);
                        sum += pa.lerp(pb, t);
                        cnt += 1;
                    }
                }
                let local = sum / cnt as f32;
                cell_vert[cell(i, j, k)] = out.add_vertex(g.point(i, j, k) + local * g.h);
            }
        }
    }
    let quad = |cells: [usize; 4], flip: bool, out: &mut Mesh| {
        let mut v = cells.map(|c| cell_vert[c]);
        if v.contains(&u32::MAX) {
            return;
        }
        if flip {
            v.reverse();
        }
        out.faces.push(Face::new(v.to_vec()));
    };
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let inside = sdf[g.idx(i, j, k)] < 0.0;
                // X edge: cells around it in (y, z).
                if i + 1 < nx && j >= 1 && k >= 1 && j < cy && k < nz - 1 && inside != (sdf[g.idx(i + 1, j, k)] < 0.0) {
                    quad([cell(i, j - 1, k - 1), cell(i, j, k - 1), cell(i, j, k), cell(i, j - 1, k)], !inside, &mut out);
                }
                if j + 1 < ny && i >= 1 && k >= 1 && i < cx && k < nz - 1 && inside != (sdf[g.idx(i, j + 1, k)] < 0.0) {
                    quad([cell(i - 1, j, k - 1), cell(i - 1, j, k), cell(i, j, k), cell(i, j, k - 1)], !inside, &mut out);
                }
                if k + 1 < nz && i >= 1 && j >= 1 && i < cx && j < cy && inside != (sdf[g.idx(i, j, k + 1)] < 0.0) {
                    quad([cell(i - 1, j - 1, k), cell(i, j - 1, k), cell(i, j, k), cell(i - 1, j, k)], !inside, &mut out);
                }
            }
        }
    }
    out
}

/// The outer faces of every inside voxel (grid points are voxel centres).
fn blocks(g: &Grid, sdf: &[f32]) -> Mesh {
    let [nx, ny, nz] = g.n;
    let mut out = Mesh::new();
    let mut corners: HashMap<(usize, usize, usize), u32> = HashMap::new();
    let half = g.origin - Vec3::splat(0.5 * g.h);
    let mut corner = |c: (usize, usize, usize), out: &mut Mesh| *corners.entry(c).or_insert_with(|| out.add_vertex(half + Vec3::new(c.0 as f32, c.1 as f32, c.2 as f32) * g.h));
    let inside = |i: usize, j: usize, k: usize| i < nx && j < ny && k < nz && sdf[g.idx(i, j, k)] < 0.0;
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let here = inside(i, j, k);
                // Faces between this voxel and its +X, +Y, +Z neighbours.
                let nbrs: [(bool, [(usize, usize, usize); 4]); 3] = [
                    (inside(i + 1, j, k), [(i + 1, j, k), (i + 1, j + 1, k), (i + 1, j + 1, k + 1), (i + 1, j, k + 1)]),
                    (inside(i, j + 1, k), [(i, j + 1, k), (i, j + 1, k + 1), (i + 1, j + 1, k + 1), (i + 1, j + 1, k)]),
                    (inside(i, j, k + 1), [(i, j, k + 1), (i + 1, j, k + 1), (i + 1, j + 1, k + 1), (i, j + 1, k + 1)]),
                ];
                for (there, cs) in nbrs {
                    if here == there {
                        continue;
                    }
                    let mut v: Vec<u32> = cs.iter().map(|&c| corner(c, &mut out)).collect();
                    if !here {
                        v.reverse();
                    }
                    out.faces.push(Face::new(v));
                }
            }
        }
    }
    out
}
