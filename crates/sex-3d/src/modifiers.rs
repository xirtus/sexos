// SPDX-License-Identifier: GPL-3.0-only
//! The non-destructive modifier stack.
//!
//! A [`Modifier`] is a pure function from mesh to mesh; an object's stack is
//! evaluated top to bottom by [`apply_stack`] whenever the viewport or the
//! renderer needs the evaluated mesh. Modifiers that share an algorithm with
//! an edit-mode operation (bevel, doubled-face cleanup) call into [`crate::ops`].
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::ops;
use std::collections::HashMap;

/// Subdivision levels above this are clamped: each level quadruples the face
/// count, so level 6 already turns a cube into 24k quads.
pub const MAX_SUBDIVISION_LEVELS: u32 = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum Modifier {
    /// Mirror across the object's local axes; `merge` welds seam vertices
    /// within `merge_distance`.
    Mirror { x: bool, y: bool, z: bool, merge: bool, merge_distance: f32 },
    /// Catmull-Clark subdivision `levels` times (`simple` keeps the shape).
    Subdivision { levels: u32, simple: bool },
    /// `count` copies offset by `relative` (times bounds size) plus `constant`.
    Array { count: u32, relative: [f32; 3], constant: [f32; 3], merge: bool },
    /// Give surfaces `thickness` (offset along normals, -1..1 picks the side).
    Solidify { thickness: f32, offset: f32 },
    /// Bevel every edge sharper than `angle` radians by `width`, `segments` cuts.
    Bevel { width: f32, segments: u32, angle: f32 },
    Triangulate,
    /// Merge by distance.
    Weld { distance: f32 },
    /// Move vertices along normals by `strength` (uniform).
    Displace { strength: f32 },
    /// Laplacian smoothing.
    Smooth { factor: f32, iterations: u32 },
}

impl Modifier {
    pub fn name(&self) -> &'static str {
        match self {
            Modifier::Mirror { .. } => "Mirror",
            Modifier::Subdivision { .. } => "Subdivision",
            Modifier::Array { .. } => "Array",
            Modifier::Solidify { .. } => "Solidify",
            Modifier::Bevel { .. } => "Bevel",
            Modifier::Triangulate => "Triangulate",
            Modifier::Weld { .. } => "Weld",
            Modifier::Displace { .. } => "Displace",
            Modifier::Smooth { .. } => "Smooth",
        }
    }

    /// Apply this one modifier to `m`.
    pub fn apply(&self, m: &Mesh) -> Mesh {
        match *self {
            Modifier::Mirror { x, y, z, merge, merge_distance } => mirror(m, [x, y, z], merge.then_some(merge_distance.max(0.0))),
            Modifier::Subdivision { levels, simple } => {
                if simple {
                    subdivide_simple(m, levels)
                } else {
                    subdivide_catmull_clark(m, levels)
                }
            }
            Modifier::Array { count, relative, constant, merge } => array(m, count, Vec3::from_array(relative), Vec3::from_array(constant), merge),
            Modifier::Solidify { thickness, offset } => solidify(m, thickness, offset),
            Modifier::Bevel { width, segments, angle } => bevel_by_angle(m, width, segments, angle),
            Modifier::Triangulate => triangulate(m),
            Modifier::Weld { distance } => {
                let mut out = m.clone();
                out.weld(distance);
                out
            }
            Modifier::Displace { strength } => displace(m, strength),
            Modifier::Smooth { factor, iterations } => smooth(m, factor, iterations),
        }
    }
}

/// Run a stack top to bottom.
pub fn apply_stack(m: &Mesh, stack: &[Modifier]) -> Mesh {
    let mut out = m.clone();
    for md in stack {
        out = md.apply(&out);
    }
    out
}

// ---------------------------------------------------------------------------
// Subdivision

/// Midpoint ("simple") subdivision: every n-gon becomes n quads through its
/// edge midpoints and centroid; the shape does not change.
pub fn subdivide_simple(m: &Mesh, levels: u32) -> Mesh {
    let mut out = m.clone();
    for _ in 0..levels.min(MAX_SUBDIVISION_LEVELS) {
        out = subdivide_once(&out, true);
    }
    out
}

/// Catmull-Clark subdivision on n-gons. Boundary edges (and non-manifold or
/// loose edges) are treated as creases: boundary vertices follow the cubic
/// B-spline rule along the boundary, and boundary corners (one face) stay put,
/// like Blender's "keep corners" default. UVs are interpolated linearly.
pub fn subdivide_catmull_clark(m: &Mesh, levels: u32) -> Mesh {
    let mut out = m.clone();
    for _ in 0..levels.min(MAX_SUBDIVISION_LEVELS) {
        out = subdivide_once(&out, false);
    }
    out
}

/// One undirected edge of the input with up to two adjacent faces.
struct SubEdge {
    v: [u32; 2],
    faces: [u32; 2],
    count: u32,
}

/// One level of subdivision. New vertex layout: the original vertices (moved),
/// then one point per edge, then one per face.
fn subdivide_once(m: &Mesh, simple: bool) -> Mesh {
    let nv = m.positions.len();
    let pos = &m.positions;

    // Unique edges, and for every face corner the edge leaving it.
    let mut edge_index: HashMap<(u32, u32), u32> = HashMap::with_capacity(m.faces.len() * 2);
    let mut edges: Vec<SubEdge> = Vec::with_capacity(m.faces.len() * 2);
    let mut corner_edge: Vec<u32> = Vec::new();
    let mut face_offset: Vec<usize> = Vec::with_capacity(m.faces.len());
    let mut edge_of = |a: u32, b: u32, edges: &mut Vec<SubEdge>| -> u32 {
        *edge_index.entry(ekey(a, b)).or_insert_with(|| {
            edges.push(SubEdge { v: [a, b], faces: [u32::MAX; 2], count: 0 });
            (edges.len() - 1) as u32
        })
    };
    for (fi, f) in m.faces.iter().enumerate() {
        face_offset.push(corner_edge.len());
        for (a, b) in f.edges() {
            let e = edge_of(a, b, &mut edges);
            let se = &mut edges[e as usize];
            if se.count < 2 {
                se.faces[se.count as usize] = fi as u32;
            }
            se.count += 1;
            corner_edge.push(e);
        }
    }
    let loose: Vec<u32> = m.loose_edges.iter().map(|e| edge_of(e[0], e[1], &mut edges)).collect();

    // Face points: centroids.
    let face_pts: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_center(f)).collect();

    // Edge points: average of the ends and the two face points, or the midpoint.
    let edge_pts: Vec<Vec3> = edges
        .iter()
        .map(|e| {
            let mid = (pos[e.v[0] as usize] + pos[e.v[1] as usize]) * 0.5;
            if !simple && e.count == 2 {
                (mid * 2.0 + face_pts[e.faces[0] as usize] + face_pts[e.faces[1] as usize]) * 0.25
            } else {
                mid
            }
        })
        .collect();

    // Vertex points.
    let vert_pts: Vec<Vec3> = if simple {
        pos.clone()
    } else {
        let mut face_sum = vec![Vec3::ZERO; nv];
        let mut face_n = vec![0u32; nv];
        let mut mid_sum = vec![Vec3::ZERO; nv];
        let mut edge_n = vec![0u32; nv];
        let mut sharp_sum = vec![Vec3::ZERO; nv];
        let mut sharp_n = vec![0u32; nv];
        for (fi, f) in m.faces.iter().enumerate() {
            for &v in &f.verts {
                face_sum[v as usize] += face_pts[fi];
                face_n[v as usize] += 1;
            }
        }
        for e in &edges {
            let [a, b] = [e.v[0] as usize, e.v[1] as usize];
            let mid = (pos[a] + pos[b]) * 0.5;
            mid_sum[a] += mid;
            mid_sum[b] += mid;
            edge_n[a] += 1;
            edge_n[b] += 1;
            if e.count != 2 {
                sharp_sum[a] += pos[b];
                sharp_sum[b] += pos[a];
                sharp_n[a] += 1;
                sharp_n[b] += 1;
            }
        }
        (0..nv)
            .map(|v| {
                let p = pos[v];
                if face_n[v] == 0 {
                    p
                } else if sharp_n[v] == 0 {
                    let n = face_n[v] as f32;
                    let f = face_sum[v] / n;
                    let r = mid_sum[v] / edge_n[v] as f32;
                    (f + r * 2.0 + p * (n - 3.0)) / n
                } else if sharp_n[v] == 2 && face_n[v] >= 2 {
                    (p * 6.0 + sharp_sum[v]) / 8.0
                } else {
                    p
                }
            })
            .collect()
    };

    let ne = edges.len() as u32;
    let mut out = Mesh::new();
    out.positions = vert_pts;
    out.positions.extend_from_slice(&edge_pts);
    out.positions.extend_from_slice(&face_pts);
    out.faces.reserve(corner_edge.len());
    for (fi, f) in m.faces.iter().enumerate() {
        let n = f.verts.len();
        if n < 3 {
            continue;
        }
        let fp = nv as u32 + ne + fi as u32;
        let ce = &corner_edge[face_offset[fi]..face_offset[fi] + n];
        let has_uv = f.uvs.len() == n;
        let face_uv = if has_uv { f.uvs.iter().fold(Vec2::ZERO, |a, &b| a + b) * (1.0 / n as f32) } else { Vec2::ZERO };
        for i in 0..n {
            let ip = (i + n - 1) % n;
            let verts = vec![f.verts[i], nv as u32 + ce[i], fp, nv as u32 + ce[ip]];
            let uvs = if has_uv {
                let (u, un, up) = (f.uvs[i], f.uvs[(i + 1) % n], f.uvs[ip]);
                vec![u, u.lerp(un, 0.5), face_uv, up.lerp(u, 0.5)]
            } else {
                Vec::new()
            };
            out.faces.push(Face { verts, uvs, material: f.material, smooth: f.smooth });
        }
    }
    for e in loose {
        let [a, b] = edges[e as usize].v;
        let mid = nv as u32 + e;
        out.loose_edges.push([a, mid]);
        out.loose_edges.push([mid, b]);
    }
    out
}

fn ekey(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

// ---------------------------------------------------------------------------
// Mirror, array, solidify

fn axis_value(p: Vec3, axis: usize) -> f32 {
    p[axis]
}

fn set_axis(p: &mut Vec3, axis: usize, v: f32) {
    match axis {
        0 => p.x = v,
        1 => p.y = v,
        _ => p.z = v,
    }
}

/// Mirror across each enabled local axis in turn (X, then Y, then Z).
fn mirror(m: &Mesh, axes: [bool; 3], merge: Option<f32>) -> Mesh {
    let mut out = m.clone();
    for (axis, &on) in axes.iter().enumerate() {
        if on {
            out = mirror_axis(&out, axis, merge);
        }
    }
    out
}

/// Append a copy of `m` reflected through the plane `axis = 0`. The copy's
/// winding is reversed so its normals still face outwards. With `merge`, seam
/// vertices within the distance of the plane are snapped onto it and shared by
/// both halves, and copied faces lying entirely in the plane are dropped.
fn mirror_axis(m: &Mesh, axis: usize, merge: Option<f32>) -> Mesh {
    let n = m.positions.len();
    let mut out = m.clone();
    let mut map = vec![0u32; n];
    let mut on_plane = vec![false; n];
    for i in 0..n {
        let c = axis_value(m.positions[i], axis);
        if let Some(d) = merge {
            if c.abs() <= d {
                set_axis(&mut out.positions[i], axis, 0.0);
                map[i] = i as u32;
                on_plane[i] = true;
                continue;
            }
        }
        let mut p = m.positions[i];
        set_axis(&mut p, axis, -c);
        map[i] = out.add_vertex(p);
    }
    for f in &m.faces {
        if merge.is_some() && f.verts.iter().all(|&v| on_plane[v as usize]) {
            continue;
        }
        let mut nf = f.clone();
        nf.verts = f.verts.iter().rev().map(|&v| map[v as usize]).collect();
        nf.uvs.reverse();
        out.faces.push(nf);
    }
    for e in &m.loose_edges {
        if !(on_plane[e[0] as usize] && on_plane[e[1] as usize]) {
            out.loose_edges.push([map[e[0] as usize], map[e[1] as usize]]);
        }
    }
    out
}

/// `count` copies, copy `i` offset by `i * (relative * bounds size + constant)`.
/// With `merge`, coincident vertices of neighbouring copies are welded and the
/// back-to-back faces between them removed, so an array of cubes is one box.
fn array(m: &Mesh, count: u32, relative: Vec3, constant: Vec3, merge: bool) -> Mesh {
    let count = count.max(1);
    let b = m.bounds();
    let size = if b.is_empty() { Vec3::ZERO } else { b.size() };
    let step = relative.mul_elem(size) + constant;
    let mut out = Mesh::new();
    for i in 0..count {
        let mut c = m.clone();
        c.translate(step * i as f32);
        out.append(&c);
    }
    if merge && count > 1 {
        out.weld(1e-4 * size.max_elem().max(1.0));
        ops::remove_doubled_faces(&mut out);
        out.remove_unused_vertices();
    }
    out
}

/// Thicken a surface into a shell. The original surface moves to
/// `thickness * (offset + 1) / 2` along the vertex normals and a reversed copy
/// to `thickness * (offset - 1) / 2`, so offset -1 (Blender's default) grows
/// the shell behind the surface. Boundary edges get rim quads, so an open
/// surface becomes a closed solid.
pub fn solidify(m: &Mesh, thickness: f32, offset: f32) -> Mesh {
    let o = offset.clamp(-1.0, 1.0);
    let outer = thickness * (o + 1.0) * 0.5;
    let inner = thickness * (o - 1.0) * 0.5;
    let normals = m.vertex_normals();
    let n = m.positions.len() as u32;
    let mut out = Mesh::new();
    out.positions.reserve(2 * n as usize);
    out.positions.extend(m.positions.iter().zip(&normals).map(|(&p, &nr)| p + nr * outer));
    out.positions.extend(m.positions.iter().zip(&normals).map(|(&p, &nr)| p + nr * inner));
    let directed: std::collections::HashSet<(u32, u32)> = m.faces.iter().flat_map(|f| f.edges()).collect();
    out.faces.extend(m.faces.iter().cloned());
    for f in &m.faces {
        let mut nf = f.clone();
        nf.verts = f.verts.iter().rev().map(|&v| v + n).collect();
        nf.uvs.reverse();
        out.faces.push(nf);
    }
    for f in &m.faces {
        let k = f.verts.len();
        for i in 0..k {
            let (a, b) = (f.verts[i], f.verts[(i + 1) % k]);
            if directed.contains(&(b, a)) {
                continue;
            }
            let uvs = if f.uvs.len() == k {
                let (ua, ub) = (f.uvs[i], f.uvs[(i + 1) % k]);
                vec![ub, ua, ua, ub]
            } else {
                Vec::new()
            };
            out.faces.push(Face { verts: vec![b, a, a + n, b + n], uvs, material: f.material, smooth: f.smooth });
        }
    }
    out.loose_edges = m.loose_edges.clone();
    if thickness < 0.0 {
        out.flip_normals();
    }
    out
}

// ---------------------------------------------------------------------------
// Bevel, triangulate, displace, smooth

/// Bevel every manifold edge whose dihedral angle (between face normals)
/// exceeds `angle` radians.
fn bevel_by_angle(m: &Mesh, width: f32, segments: u32, angle: f32) -> Mesh {
    let mut out = m.clone();
    if width <= 0.0 {
        return out;
    }
    let normals: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_normal(f)).collect();
    let mut by_edge: HashMap<(u32, u32), (u32, u32, u32)> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for (a, b) in f.edges() {
            let e = by_edge.entry(ekey(a, b)).or_insert((u32::MAX, u32::MAX, 0));
            if e.2 == 0 {
                e.0 = fi as u32;
            } else {
                e.1 = fi as u32;
            }
            e.2 += 1;
        }
    }
    let cos_limit = angle.cos();
    let mut edges: Vec<[u32; 2]> = by_edge
        .iter()
        .filter(|(_, &(f0, f1, c))| c == 2 && normals[f0 as usize].dot(normals[f1 as usize]) < cos_limit)
        .map(|(&(a, b), _)| [a, b])
        .collect();
    edges.sort_unstable();
    ops::bevel_edges(&mut out, &edges, width, segments.max(1));
    out
}

/// Split every face into triangles (ear clipping for n-gons), keeping UVs,
/// material and shading.
fn triangulate(m: &Mesh) -> Mesh {
    let mut out = Mesh { positions: m.positions.clone(), faces: Vec::new(), loose_edges: m.loose_edges.clone() };
    for t in m.triangulate() {
        let f = &m.faces[t.face as usize];
        let uvs = if f.uvs.len() == f.verts.len() { t.corners.iter().map(|&c| f.uvs[c as usize]).collect() } else { Vec::new() };
        out.faces.push(Face { verts: t.verts.to_vec(), uvs, material: f.material, smooth: f.smooth });
    }
    out
}

/// Push every vertex along its normal by `strength`.
fn displace(m: &Mesh, strength: f32) -> Mesh {
    let mut out = m.clone();
    for (p, n) in out.positions.iter_mut().zip(m.vertex_normals()) {
        *p += n * strength;
    }
    out
}

/// Laplacian smoothing: each iteration moves every vertex `factor` of the way
/// towards the average of its edge neighbours.
fn smooth(m: &Mesh, factor: f32, iterations: u32) -> Mesh {
    let mut out = m.clone();
    let nv = m.positions.len();
    // Compressed adjacency lists.
    let edges = m.edges();
    let mut start = vec![0usize; nv + 1];
    for e in &edges {
        start[e[0] as usize + 1] += 1;
        start[e[1] as usize + 1] += 1;
    }
    for i in 0..nv {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut adj = vec![0u32; start[nv]];
    for e in &edges {
        adj[fill[e[0] as usize]] = e[1];
        fill[e[0] as usize] += 1;
        adj[fill[e[1] as usize]] = e[0];
        fill[e[1] as usize] += 1;
    }
    for _ in 0..iterations {
        let prev = out.positions.clone();
        for v in 0..nv {
            let nb = &adj[start[v]..start[v + 1]];
            if nb.is_empty() {
                continue;
            }
            let avg = nb.iter().fold(Vec3::ZERO, |a, &u| a + prev[u as usize]) / nb.len() as f32;
            out.positions[v] = prev[v] + (avg - prev[v]) * factor;
        }
    }
    out
}
