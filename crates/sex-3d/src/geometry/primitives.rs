// SPDX-License-Identifier: GPL-3.0-only
//! Geometry node primitives with Blender's topology (`mesh_primitive_*.cc`,
//! `node_geo_curve_primitive_*.cc`): vertex, edge and face counts match,
//! as does the element order where it is cheap to follow (grid, circle,
//! cylinder and cone rings from top to bottom, UV sphere rings from the top
//! pole). UVs are written as the mesh's face corners (the `UVMap` output).
use super::curves::{Curves, BEZIER, HANDLE_FREE, POLY};
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use std::f32::consts::{PI, TAU};

/// Circle fill types (`GEO_NODE_MESH_CIRCLE_FILL_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    None,
    Ngon,
    TriangleFan,
}

impl Fill {
    pub fn from_name(s: &str) -> Fill {
        match s {
            "NONE" => Fill::None,
            "TRIANGLE_FAN" => Fill::TriangleFan,
            _ => Fill::Ngon,
        }
    }
}

fn face(verts: Vec<u32>, uvs: Vec<Vec2>) -> Face {
    let mut f = Face::new(verts);
    f.uvs = uvs;
    f
}

/// Grid in the XY plane: `verts_x * verts_y` vertices, x-major.
pub fn grid(size_x: f32, size_y: f32, verts_x: usize, verts_y: usize) -> Mesh {
    let (vx, vy) = (verts_x.max(1), verts_y.max(1));
    let mut m = Mesh::default();
    let (ex, ey) = (vx.saturating_sub(1), vy.saturating_sub(1));
    let dx = if ex > 0 { size_x / ex as f32 } else { 0.0 };
    let dy = if ey > 0 { size_y / ey as f32 } else { 0.0 };
    let (x0, y0) = (-size_x * 0.5 * (ex > 0) as u8 as f32, -size_y * 0.5 * (ey > 0) as u8 as f32);
    for x in 0..vx {
        for y in 0..vy {
            m.positions.push(Vec3::new(x0 + dx * x as f32, y0 + dy * y as f32, 0.0));
        }
    }
    if ex == 0 || ey == 0 {
        for i in 0..(vx * vy).saturating_sub(1) {
            m.loose_edges.push([i as u32, i as u32 + 1]);
        }
        return m;
    }
    let uv = |x: usize, y: usize| Vec2::new(x as f32 / ex as f32, y as f32 / ey as f32);
    for x in 0..ex {
        for y in 0..ey {
            let v = (x * vy + y) as u32;
            let vyu = vy as u32;
            m.faces.push(face(vec![v, v + vyu, v + vyu + 1, v + 1], vec![uv(x, y), uv(x + 1, y), uv(x + 1, y + 1), uv(x, y + 1)]));
        }
    }
    m
}

/// Box with `verts` vertices along each axis (each at least 2).
pub fn cube(size: Vec3, verts: [usize; 3]) -> Mesh {
    let [nx, ny, nz] = verts.map(|v| v.max(2));
    let mut m = Mesh::default();
    let mut index = vec![u32::MAX; nx * ny * nz];
    let id = |x: usize, y: usize, z: usize| (z * ny + y) * nx + x;
    let pos = |x: usize, y: usize, z: usize| {
        Vec3::new(size.x * (x as f32 / (nx - 1) as f32 - 0.5), size.y * (y as f32 / (ny - 1) as f32 - 0.5), size.z * (z as f32 / (nz - 1) as f32 - 0.5))
    };
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                if x == 0 || y == 0 || z == 0 || x == nx - 1 || y == ny - 1 || z == nz - 1 {
                    index[id(x, y, z)] = m.positions.len() as u32;
                    m.positions.push(pos(x, y, z));
                }
            }
        }
    }
    // Six sides: (fixed axis, value, u axis, v axis, outward sign, uv tile).
    let sides: [(usize, bool, usize, usize, f32, (f32, f32)); 6] = [
        (2, false, 0, 1, -1.0, (0.375, 0.0)),
        (2, true, 0, 1, 1.0, (0.375, 0.5)),
        (1, false, 0, 2, -1.0, (0.375, 0.75)),
        (1, true, 0, 2, 1.0, (0.375, 0.25)),
        (0, false, 1, 2, -1.0, (0.125, 0.5)),
        (0, true, 1, 2, 1.0, (0.625, 0.5)),
    ];
    let dims = [nx, ny, nz];
    for (axis, hi, ua, va, sign, tile) in sides {
        let (nu, nv) = (dims[ua], dims[va]);
        for u in 0..nu - 1 {
            for v in 0..nv - 1 {
                let at = |du: usize, dv: usize| {
                    let mut c = [0usize; 3];
                    c[axis] = if hi { dims[axis] - 1 } else { 0 };
                    c[ua] = u + du;
                    c[va] = v + dv;
                    index[id(c[0], c[1], c[2])]
                };
                let uvp = |du: usize, dv: usize| Vec2::new(tile.0 + 0.25 * (u + du) as f32 / (nu - 1) as f32, tile.1 + 0.25 * (v + dv) as f32 / (nv - 1) as f32);
                let mut f = vec![at(0, 0), at(1, 0), at(1, 1), at(0, 1)];
                let mut uv = vec![uvp(0, 0), uvp(1, 0), uvp(1, 1), uvp(0, 1)];
                // Make the face point outward.
                let p: Vec<Vec3> = f.iter().map(|&i| m.positions[i as usize]).collect();
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                if n[axis] * sign < 0.0 {
                    f.reverse();
                    uv.reverse();
                }
                m.faces.push(face(f, uv));
            }
        }
    }
    m
}

/// Circle of `n` vertices in the XY plane.
pub fn circle(n: usize, radius: f32, fill: Fill) -> Mesh {
    let n = n.max(3);
    let mut m = Mesh::default();
    for i in 0..n {
        let a = TAU * i as f32 / n as f32;
        m.positions.push(Vec3::new(radius * a.cos(), radius * a.sin(), 0.0));
    }
    let uv = |i: usize| {
        let a = TAU * i as f32 / n as f32;
        Vec2::new(0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())
    };
    match fill {
        Fill::None => {
            for i in 0..n {
                m.loose_edges.push([i as u32, ((i + 1) % n) as u32]);
            }
        }
        Fill::Ngon => m.faces.push(face((0..n as u32).collect(), (0..n).map(uv).collect())),
        Fill::TriangleFan => {
            let c = n as u32;
            m.positions.push(Vec3::ZERO);
            for i in 0..n {
                m.faces.push(face(vec![i as u32, ((i + 1) % n) as u32, c], vec![uv(i), uv((i + 1) % n), Vec2::new(0.5, 0.5)]));
            }
        }
    }
    m
}

/// Which faces of a cylinder or cone are caps or sides.
#[derive(Clone, Debug, Default)]
pub struct ConeParts {
    pub top: Vec<bool>,
    pub bottom: Vec<bool>,
    pub side: Vec<bool>,
}

/// Cone (`radius_top` 0 makes a tip) and cylinder, as Blender's
/// `create_cylinder_or_cone_mesh`: rings from top to bottom, the top fill
/// rings growing outward and the bottom ones shrinking inward.
pub fn cone(verts: usize, side_segments: usize, fill_segments: usize, radius_top: f32, radius_bottom: f32, depth: f32, fill: Fill) -> (Mesh, ConeParts) {
    let n = verts.max(3);
    let sides = side_segments.max(1);
    let fills = if fill == Fill::None { 1 } else { fill_segments.max(1) };
    let top_point = radius_top == 0.0;
    let bottom_point = radius_bottom == 0.0;
    let top_center = top_point || fill == Fill::TriangleFan;
    let bottom_center = bottom_point || fill == Fill::TriangleFan;
    let (zt, zb) = (depth * 0.5, -depth * 0.5);
    let mut m = Mesh::default();
    let mut parts = ConeParts::default();
    if top_point && bottom_point {
        for i in 0..=sides {
            m.positions.push(Vec3::new(0.0, 0.0, zt + (zb - zt) * i as f32 / sides as f32));
        }
        for i in 0..sides {
            m.loose_edges.push([i as u32, i as u32 + 1]);
        }
        return (m, parts);
    }
    // (ring start, kind) where kind: 0 top fill, 1 side, 2 bottom fill.
    let mut rings: Vec<(u32, u8)> = Vec::new();
    let ring = |m: &mut Mesh, r: f32, z: f32| -> u32 {
        let s = m.positions.len() as u32;
        for i in 0..n {
            let a = TAU * i as f32 / n as f32;
            m.positions.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
        s
    };
    let top_c = if top_center {
        m.positions.push(Vec3::new(0.0, 0.0, zt));
        Some(0u32)
    } else {
        None
    };
    if !top_point {
        for i in 1..=fills {
            let s = ring(&mut m, radius_top * i as f32 / fills as f32, zt);
            rings.push((s, 0));
        }
    }
    for i in 1..sides {
        let f = i as f32 / sides as f32;
        let s = ring(&mut m, radius_top + (radius_bottom - radius_top) * f, zt + (zb - zt) * f);
        rings.push((s, 1));
    }
    if !bottom_point {
        for i in (1..=fills).rev() {
            let s = ring(&mut m, radius_bottom * i as f32 / fills as f32, zb);
            rings.push((s, 2));
        }
    }
    let bottom_c = if bottom_center {
        m.positions.push(Vec3::new(0.0, 0.0, zb));
        Some(m.positions.len() as u32 - 1)
    } else {
        None
    };
    let nn = n as u32;
    let next = |i: u32| (i + 1) % nn;
    // UVs: the side band in the lower half, the caps as circles above.
    let cap_uv = |i: u32, r: f32, cx: f32| {
        let a = TAU * i as f32 / n as f32;
        Vec2::new(cx + 0.25 * r * a.cos(), 0.75 + 0.25 * r * a.sin())
    };
    let ring_frac = |k: usize| -> f32 {
        // Radius fraction of a fill ring for cap UVs.
        match rings[k].1 {
            0 => (k + 1) as f32 / fills as f32,
            2 => {
                let first_bottom = rings.iter().position(|r| r.1 == 2).unwrap_or(0);
                (fills - (k - first_bottom)) as f32 / fills as f32
            }
            _ => 1.0,
        }
    };
    let mut kinds: Vec<u8> = Vec::new();
    // Top cap.
    if let Some(c) = top_c {
        let r0 = rings[0].0;
        let kind = if top_point { 1 } else { 0 };
        for i in 0..nn {
            let uvs = if kind == 0 { vec![Vec2::new(0.25, 0.75), cap_uv(i, ring_frac(0), 0.25), cap_uv(i + 1, ring_frac(0), 0.25)] } else { side_uvs(i, 0, 1, sides, n) };
            m.faces.push(face(vec![c, r0 + i, r0 + next(i)], uvs));
            kinds.push(kind);
        }
    } else if fill == Fill::Ngon && !top_point {
        let r0 = rings[0].0;
        m.faces.push(face((0..nn).map(|i| r0 + i).collect(), (0..nn).map(|i| cap_uv(i, 1.0, 0.25)).collect()));
        kinds.push(0);
    }
    // Rings.
    let side_rows = |k: usize| rings.iter().take(k + 1).filter(|r| r.1 != 0).count();
    for k in 0..rings.len().saturating_sub(1) {
        let (a, ka) = rings[k];
        let (b, kb) = rings[k + 1];
        let kind = if ka == 0 && kb == 0 {
            0
        } else if ka == 2 && kb == 2 {
            2
        } else {
            1
        };
        for i in 0..nn {
            let uvs = match kind {
                0 => vec![cap_uv(i, ring_frac(k), 0.25), cap_uv(i, ring_frac(k + 1), 0.25), cap_uv(i + 1, ring_frac(k + 1), 0.25), cap_uv(i + 1, ring_frac(k), 0.25)],
                2 => vec![cap_uv(i, ring_frac(k), 0.75), cap_uv(i, ring_frac(k + 1), 0.75), cap_uv(i + 1, ring_frac(k + 1), 0.75), cap_uv(i + 1, ring_frac(k), 0.75)],
                _ => {
                    let row = side_rows(k) - usize::from(ka == 1 || ka == 2) + usize::from(top_point);
                    side_uvs(i, row, row + 1, sides, n)
                }
            };
            m.faces.push(face(vec![a + i, b + i, b + next(i), a + next(i)], uvs));
            kinds.push(kind);
        }
    }
    // Bottom cap.
    if let Some(c) = bottom_c {
        let rl = rings.last().unwrap().0;
        let kind = if bottom_point { 1 } else { 2 };
        for i in 0..nn {
            let uvs = if kind == 2 { vec![cap_uv(i, ring_frac(rings.len() - 1), 0.75), Vec2::new(0.75, 0.75), cap_uv(i + 1, ring_frac(rings.len() - 1), 0.75)] } else { side_uvs(i, sides - 1, sides, sides, n) };
            m.faces.push(face(vec![rl + i, c, rl + next(i)], uvs));
            kinds.push(kind);
        }
    } else if fill == Fill::Ngon && !bottom_point {
        let rl = rings.last().unwrap().0;
        m.faces.push(face((0..nn).rev().map(|i| rl + i).collect(), (0..nn).rev().map(|i| cap_uv(i, 1.0, 0.75)).collect()));
        kinds.push(2);
    }
    parts.top = kinds.iter().map(|&k| k == 0).collect();
    parts.side = kinds.iter().map(|&k| k == 1).collect();
    parts.bottom = kinds.iter().map(|&k| k == 2).collect();
    (m, parts)
}

fn side_uvs(i: u32, row_a: usize, row_b: usize, rows: usize, n: usize) -> Vec<Vec2> {
    let u = |i: u32| i as f32 / n as f32;
    let v = |r: usize| 0.5 - 0.5 * r.min(rows) as f32 / rows as f32;
    vec![Vec2::new(u(i), v(row_a)), Vec2::new(u(i), v(row_b)), Vec2::new(u(i + 1), v(row_b)), Vec2::new(u(i + 1), v(row_a))]
}

/// UV sphere: `segments * (rings - 1) + 2` vertices, the top pole first.
pub fn uv_sphere(segments: usize, rings: usize, radius: f32) -> Mesh {
    let (s, r) = (segments.max(3), rings.max(2));
    let mut m = Mesh::default();
    m.positions.push(Vec3::new(0.0, 0.0, radius));
    for ring in 1..r {
        let phi = PI * ring as f32 / r as f32;
        for seg in 0..s {
            let th = TAU * seg as f32 / s as f32;
            m.positions.push(Vec3::new(radius * phi.sin() * th.cos(), radius * phi.sin() * th.sin(), radius * phi.cos()));
        }
    }
    m.positions.push(Vec3::new(0.0, 0.0, -radius));
    let bottom = m.positions.len() as u32 - 1;
    let ring_start = |k: usize| 1 + (k * s) as u32;
    let su = s as u32;
    let nx = |i: u32| (i + 1) % su;
    let uv = |seg: u32, ring: usize| Vec2::new(seg as f32 / s as f32, 1.0 - ring as f32 / r as f32);
    for i in 0..su {
        let mid = Vec2::new((i as f32 + 0.5) / s as f32, 1.0);
        m.faces.push(face(vec![0, ring_start(0) + i, ring_start(0) + nx(i)], vec![mid, uv(i, 1), uv(i + 1, 1)]));
    }
    for k in 0..r - 2 {
        let (a, b) = (ring_start(k), ring_start(k + 1));
        for i in 0..su {
            m.faces.push(face(vec![a + i, b + i, b + nx(i), a + nx(i)], vec![uv(i, k + 1), uv(i, k + 2), uv(i + 1, k + 2), uv(i + 1, k + 1)]));
        }
    }
    let last = ring_start(r - 2);
    for i in 0..su {
        let mid = Vec2::new((i as f32 + 0.5) / s as f32, 0.0);
        m.faces.push(face(vec![last + i, bottom, last + nx(i)], vec![uv(i, r - 1), mid, uv(i + 1, r - 1)]));
    }
    m
}

/// Icosphere: an icosahedron (12 vertices, 20 faces) with
/// `subdivisions - 1` rounds of 4-way splitting projected to the sphere,
/// so `10 * 4^(s-1) + 2` vertices and `20 * 4^(s-1)` faces.
pub fn ico_sphere(radius: f32, subdivisions: usize) -> Mesh {
    let t = (1.0 + 5.0f32.sqrt()) / 2.0;
    // Blender's icosahedron has vertices on the poles.
    let raw = [
        Vec3::new(-1.0, t, 0.0),
        Vec3::new(1.0, t, 0.0),
        Vec3::new(-1.0, -t, 0.0),
        Vec3::new(1.0, -t, 0.0),
        Vec3::new(0.0, -1.0, t),
        Vec3::new(0.0, 1.0, t),
        Vec3::new(0.0, -1.0, -t),
        Vec3::new(0.0, 1.0, -t),
        Vec3::new(t, 0.0, -1.0),
        Vec3::new(t, 0.0, 1.0),
        Vec3::new(-t, 0.0, -1.0),
        Vec3::new(-t, 0.0, 1.0),
    ];
    // Rotate so vertex 5 (0, 1, t) sits on +Z.
    let rot = crate::math::Quat::from_rotation_arc(raw[5], Vec3::Z);
    let mut pos: Vec<Vec3> = raw.iter().map(|&p| rot.rotate(p).normalize()).collect();
    let mut tris: Vec<[u32; 3]> = crate::ops2::convex_hull_triangles(&pos).unwrap_or_default().into_iter().map(|t| t.map(|i| i as u32)).collect();
    for _ in 1..subdivisions.max(1) {
        let mut mid = std::collections::HashMap::new();
        let mut next = Vec::with_capacity(tris.len() * 4);
        let mut midpoint = |a: u32, b: u32, pos: &mut Vec<Vec3>| -> u32 {
            let k = (a.min(b), a.max(b));
            *mid.entry(k).or_insert_with(|| {
                pos.push((pos[a as usize] + pos[b as usize]).normalize());
                pos.len() as u32 - 1
            })
        };
        for [a, b, c] in tris {
            let ab = midpoint(a, b, &mut pos);
            let bc = midpoint(b, c, &mut pos);
            let ca = midpoint(c, a, &mut pos);
            next.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        tris = next;
    }
    let mut m = Mesh::default();
    m.positions = pos.iter().map(|&p| p * radius).collect();
    for t in tris {
        // Spherical UVs per corner, unwrapped across the seam.
        let mut uvs: Vec<Vec2> = t
            .iter()
            .map(|&v| {
                let p = pos[v as usize];
                Vec2::new(0.5 + p.y.atan2(p.x) / TAU, 0.5 + p.z.clamp(-1.0, 1.0).asin() / PI)
            })
            .collect();
        let maxu = uvs.iter().fold(0.0f32, |a, u| a.max(u.x));
        for u in &mut uvs {
            if maxu - u.x > 0.5 {
                u.x += 1.0;
            }
        }
        m.faces.push(face(t.to_vec(), uvs));
    }
    m
}

/// A line of `count` vertices with an edge between neighbours.
pub fn line(start: Vec3, step: Vec3, count: usize) -> Mesh {
    let mut m = Mesh::default();
    for i in 0..count {
        m.positions.push(start + step * i as f32);
    }
    for i in 1..count {
        m.loose_edges.push([i as u32 - 1, i as u32]);
    }
    m
}

// ---------------------------------------------------------------------------
// Curve primitives

pub fn curve_poly(pts: &[Vec3], cyclic: bool) -> Curves {
    let mut c = Curves::default();
    c.add_curve(pts, POLY, cyclic);
    c
}

pub fn curve_circle(resolution: usize, radius: f32) -> Curves {
    let n = resolution.max(3);
    let pts: Vec<Vec3> = (0..n).map(|i| {
        let a = TAU * i as f32 / n as f32;
        Vec3::new(radius * a.cos(), radius * a.sin(), 0.0)
    }).collect();
    curve_poly(&pts, true)
}

/// Circle through three points; returns the curve and its centre.
pub fn curve_circle_points(resolution: usize, p1: Vec3, p2: Vec3, p3: Vec3) -> Option<(Curves, Vec3)> {
    let (center, normal, r) = circumcircle(p1, p2, p3)?;
    let x = (p1 - center).normalize();
    let y = normal.cross(x);
    let n = resolution.max(3);
    let pts: Vec<Vec3> = (0..n).map(|i| {
        let a = TAU * i as f32 / n as f32;
        center + (x * a.cos() + y * a.sin()) * r
    }).collect();
    Some((curve_poly(&pts, true), center))
}

/// Centre, unit normal and radius of the circle through three points.
pub fn circumcircle(a: Vec3, b: Vec3, c: Vec3) -> Option<(Vec3, Vec3, f32)> {
    let (ab, ac) = (b - a, c - a);
    let n = ab.cross(ac);
    let n2 = n.length_squared();
    if n2 < 1e-12 {
        return None;
    }
    let center = a + (n.cross(ab) * ac.length_squared() + ac.cross(n) * ab.length_squared()) / (2.0 * n2);
    Some((center, n.normalize(), center.distance(a)))
}

pub fn quadratic_bezier(resolution: usize, a: Vec3, b: Vec3, c: Vec3) -> Curves {
    let n = resolution.max(1);
    let pts: Vec<Vec3> = (0..=n).map(|i| {
        let t = i as f32 / n as f32;
        a * ((1.0 - t) * (1.0 - t)) + b * (2.0 * (1.0 - t) * t) + c * (t * t)
    }).collect();
    curve_poly(&pts, false)
}

/// A two-point Bezier curve with free handles.
pub fn bezier_segment(resolution: usize, start: Vec3, start_handle: Vec3, end_handle: Vec3, end: Vec3) -> Curves {
    use super::attributes::{Domain, Val};
    let mut c = Curves::default();
    c.add_curve(&[start, end], BEZIER, false);
    c.set_resolution(0, resolution.max(1) as i32);
    let left = [start * 2.0 - start_handle, end_handle];
    let right = [start_handle, end * 2.0 - end_handle];
    for (name, v) in [("handle_left", left), ("handle_right", right)] {
        let d = c.ensure(name, Domain::Point, Val::Vector(Vec3::ZERO));
        d.set(0, Val::Vector(v[0]));
        d.set(1, Val::Vector(v[1]));
    }
    for name in ["handle_type_left", "handle_type_right"] {
        c.ensure(name, Domain::Point, Val::Int(HANDLE_FREE));
    }
    c
}

/// Spiral (`node_geo_curve_spiral.cc`).
pub fn spiral(resolution: usize, rotations: f32, start_radius: f32, end_radius: f32, height: f32, reverse: bool) -> Curves {
    let total = ((resolution as f32 * rotations) as i64).max(1) as usize;
    let dr = (end_radius - start_radius) / total as f32;
    let dh = height / total as f32;
    let dt = TAU * rotations / total as f32 * if reverse { -1.0 } else { 1.0 };
    let pts: Vec<Vec3> = (0..=total).map(|i| {
        let (th, r) = (i as f32 * dt, start_radius + i as f32 * dr);
        Vec3::new(r * th.cos(), r * th.sin(), dh * i as f32)
    }).collect();
    curve_poly(&pts, false)
}

/// Star: `points` outer points alternating with inner ones; the second
/// value marks the outer points.
pub fn star(points: usize, inner: f32, outer: f32, twist: f32) -> (Curves, Vec<bool>) {
    let n = points.max(3);
    let mut pts = Vec::with_capacity(n * 2);
    let mut outer_sel = Vec::with_capacity(n * 2);
    for i in 0..n {
        let a = TAU * i as f32 / n as f32;
        pts.push(Vec3::new(a.cos() * outer, a.sin() * outer, 0.0));
        let b = a + PI / n as f32 + twist;
        pts.push(Vec3::new(b.cos() * inner, b.sin() * inner, 0.0));
        outer_sel.extend([true, false]);
    }
    (curve_poly(&pts, true), outer_sel)
}

/// Arc by radius and angles (`connect_center` closes it through the origin).
pub fn arc_radius(resolution: usize, radius: f32, start_angle: f32, sweep: f32, connect_center: bool, invert: bool) -> Curves {
    let n = resolution.max(2);
    let sweep = if invert { -(TAU - sweep) } else { sweep };
    let step = sweep / (n - 1) as f32;
    let mut pts: Vec<Vec3> = (0..n).map(|i| {
        let t = start_angle + step * i as f32;
        Vec3::new(t.cos() * radius, t.sin() * radius, 0.0)
    }).collect();
    if connect_center {
        pts.push(Vec3::ZERO);
    }
    curve_poly(&pts, connect_center)
}

/// Arc through three points; also returns centre, normal and radius.
pub fn arc_points(resolution: usize, start: Vec3, middle: Vec3, end: Vec3, offset_angle: f32, connect_center: bool, invert: bool) -> (Curves, Vec3, Vec3, f32) {
    let n = resolution.max(2);
    let Some((center, normal, r)) = circumcircle(start, middle, end) else {
        let pts: Vec<Vec3> = (0..n).map(|i| start.lerp(end, i as f32 / (n - 1) as f32)).collect();
        return (curve_poly(&pts, false), (start + end) * 0.5, Vec3::Z, 0.0);
    };
    let x = (start - center).normalize();
    let y = normal.cross(x);
    let angle = |p: Vec3| {
        let d = p - center;
        let a = d.dot(y).atan2(d.dot(x));
        if a < 0.0 { a + TAU } else { a }
    };
    let mut sweep = angle(end);
    if invert {
        sweep -= TAU;
    }
    let step = sweep / (n - 1) as f32;
    let mut pts: Vec<Vec3> = (0..n).map(|i| {
        let t = offset_angle + step * i as f32;
        center + (x * t.cos() + y * t.sin()) * r
    }).collect();
    if connect_center {
        pts.push(center);
    }
    (curve_poly(&pts, connect_center), center, normal, r)
}

/// Quadrilateral modes (`node_geo_curve_primitive_quadrilateral.cc`).
#[allow(clippy::too_many_arguments)]
pub fn quadrilateral(mode: &str, width: f32, height: f32, bottom_width: f32, top_width: f32, offset: f32, bottom_height: f32, top_height: f32, pts: [Vec3; 4]) -> Curves {
    let p = match mode {
        "PARALLELOGRAM" => [
            Vec3::new(width / 2.0 - offset / 2.0, height / 2.0, 0.0),
            Vec3::new(-width / 2.0 - offset / 2.0, height / 2.0, 0.0),
            Vec3::new(-width / 2.0 + offset / 2.0, -height / 2.0, 0.0),
            Vec3::new(width / 2.0 + offset / 2.0, -height / 2.0, 0.0),
        ],
        "TRAPEZOID" => [
            Vec3::new(top_width / 2.0 + offset, height / 2.0, 0.0),
            Vec3::new(-top_width / 2.0 + offset, height / 2.0, 0.0),
            Vec3::new(-bottom_width / 2.0, -height / 2.0, 0.0),
            Vec3::new(bottom_width / 2.0, -height / 2.0, 0.0),
        ],
        "KITE" => [Vec3::new(0.0, top_height, 0.0), Vec3::new(width / 2.0, 0.0, 0.0), Vec3::new(0.0, -bottom_height, 0.0), Vec3::new(-width / 2.0, 0.0, 0.0)],
        "POINTS" => pts,
        _ => [
            Vec3::new(width / 2.0, height / 2.0, 0.0),
            Vec3::new(-width / 2.0, height / 2.0, 0.0),
            Vec3::new(-width / 2.0, -height / 2.0, 0.0),
            Vec3::new(width / 2.0, -height / 2.0, 0.0),
        ],
    };
    curve_poly(&p, true)
}
