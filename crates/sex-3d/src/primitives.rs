// SPDX-License-Identifier: GPL-3.0-only
//! Primitive meshes, Z up like Blender, centred on the origin, faces outward.
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use core::f32::consts::{PI, TAU};

/// Cube with edge length `size`. Faces carry a cross-shaped UV layout.
pub fn cube(size: f32) -> Mesh {
    let h = size * 0.5;
    let mut m = Mesh::new();
    for i in 0..8 {
        m.add_vertex(Vec3::new(if i & 1 != 0 { h } else { -h }, if i & 2 != 0 { h } else { -h }, if i & 4 != 0 { h } else { -h }));
    }
    let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let uv = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];
    for f in faces {
        m.faces.push(Face { verts: f.to_vec(), uvs: uv.to_vec(), ..Default::default() });
    }
    m
}

/// Flat square in the XY plane.
pub fn plane(size: f32) -> Mesh {
    grid(1, 1, size)
}

/// `nx` by `ny` quads covering a `size` square in XY, with UVs 0..1.
pub fn grid(nx: u32, ny: u32, size: f32) -> Mesh {
    let (nx, ny) = (nx.max(1), ny.max(1));
    let mut m = Mesh::new();
    for j in 0..=ny {
        for i in 0..=nx {
            m.add_vertex(Vec3::new((i as f32 / nx as f32 - 0.5) * size, (j as f32 / ny as f32 - 0.5) * size, 0.0));
        }
    }
    let row = nx + 1;
    for j in 0..ny {
        for i in 0..nx {
            let a = j * row + i;
            let uv = |i: u32, j: u32| Vec2::new(i as f32 / nx as f32, j as f32 / ny as f32);
            m.faces.push(Face { verts: vec![a, a + 1, a + row + 1, a + row], uvs: vec![uv(i, j), uv(i + 1, j), uv(i + 1, j + 1), uv(i, j + 1)], ..Default::default() });
        }
    }
    m
}

/// UV sphere: `segments` around, `rings` from pole to pole; triangle fans at
/// the poles, quads elsewhere. Smooth shaded.
pub fn uv_sphere(segments: u32, rings: u32, radius: f32) -> Mesh {
    let (segments, rings) = (segments.max(3), rings.max(2));
    let mut m = Mesh::new();
    let bottom = m.add_vertex(Vec3::new(0.0, 0.0, -radius));
    for r in 1..rings {
        let th = PI * r as f32 / rings as f32 - PI * 0.5;
        for s in 0..segments {
            let ph = TAU * s as f32 / segments as f32;
            m.add_vertex(Vec3::new(radius * th.cos() * ph.cos(), radius * th.cos() * ph.sin(), radius * th.sin()));
        }
    }
    let top = m.add_vertex(Vec3::new(0.0, 0.0, radius));
    let ring = |r: u32, s: u32| 1 + (r - 1) * segments + s % segments;
    let uv = |s: u32, r: u32| Vec2::new(s as f32 / segments as f32, r as f32 / rings as f32);
    let mut push = |verts: Vec<u32>, uvs: Vec<Vec2>| m.faces.push(Face { verts, uvs, smooth: true, material: 0 });
    for s in 0..segments {
        push(vec![bottom, ring(1, s + 1), ring(1, s)], vec![uv(s, 0), uv(s + 1, 1), uv(s, 1)]);
        for r in 1..rings - 1 {
            push(vec![ring(r, s), ring(r, s + 1), ring(r + 1, s + 1), ring(r + 1, s)], vec![uv(s, r), uv(s + 1, r), uv(s + 1, r + 1), uv(s, r + 1)]);
        }
        push(vec![ring(rings - 1, s), ring(rings - 1, s + 1), top], vec![uv(s, rings - 1), uv(s + 1, rings - 1), uv(s, rings)]);
    }
    m
}

/// Icosphere: an icosahedron subdivided `subdivisions` times onto the sphere.
pub fn ico_sphere(subdivisions: u32, radius: f32) -> Mesh {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut pos: Vec<Vec3> = [
        (-1.0, t, 0.0), (1.0, t, 0.0), (-1.0, -t, 0.0), (1.0, -t, 0.0),
        (0.0, -1.0, t), (0.0, 1.0, t), (0.0, -1.0, -t), (0.0, 1.0, -t),
        (t, 0.0, -1.0), (t, 0.0, 1.0), (-t, 0.0, -1.0), (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut tris: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut mid = std::collections::HashMap::new();
        let mut mid_of = |a: u32, b: u32, pos: &mut Vec<Vec3>| -> u32 {
            *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
                pos.push(((pos[a as usize] + pos[b as usize]) * 0.5).normalize());
                (pos.len() - 1) as u32
            })
        };
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let (ab, bc, ca) = (mid_of(a, b, &mut pos), mid_of(b, c, &mut pos), mid_of(c, a, &mut pos));
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    let mut m = Mesh::from_faces(pos.into_iter().map(|p| p * radius).collect(), tris.into_iter().map(|t| t.to_vec()).collect());
    for f in &mut m.faces {
        f.smooth = true;
    }
    m
}

/// Cylinder along Z with n-gon caps.
pub fn cylinder(segments: u32, radius: f32, depth: f32) -> Mesh {
    cone(segments, radius, radius, depth)
}

/// Cone (or truncated cone) along Z; a zero radius collapses that cap to a point.
pub fn cone(segments: u32, radius_bottom: f32, radius_top: f32, depth: f32) -> Mesh {
    let n = segments.max(3);
    let h = depth * 0.5;
    let mut m = Mesh::new();
    let ring = |m: &mut Mesh, r: f32, z: f32| -> Option<u32> {
        if r <= 0.0 {
            return None;
        }
        let s = m.vertex_count() as u32;
        for i in 0..n {
            let a = TAU * i as f32 / n as f32;
            m.add_vertex(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
        Some(s)
    };
    let b = ring(&mut m, radius_bottom, -h);
    let t = ring(&mut m, radius_top, h);
    match (b, t) {
        (Some(b), Some(t)) => {
            for i in 0..n {
                let j = (i + 1) % n;
                m.add_face(&[b + i, b + j, t + j, t + i]);
            }
            m.add_face(&(0..n).rev().map(|i| b + i).collect::<Vec<_>>());
            m.add_face(&(0..n).map(|i| t + i).collect::<Vec<_>>());
        }
        (Some(b), None) => {
            let apex = m.add_vertex(Vec3::new(0.0, 0.0, h));
            for i in 0..n {
                m.add_face(&[b + i, b + (i + 1) % n, apex]);
            }
            m.add_face(&(0..n).rev().map(|i| b + i).collect::<Vec<_>>());
        }
        (None, Some(t)) => {
            let apex = m.add_vertex(Vec3::new(0.0, 0.0, -h));
            for i in 0..n {
                m.add_face(&[t + (i + 1) % n, t + i, apex]);
            }
            m.add_face(&(0..n).map(|i| t + i).collect::<Vec<_>>());
        }
        (None, None) => {}
    }
    m
}

/// Torus around Z: `major_segments` around the ring, `minor_segments` around the tube.
pub fn torus(major_segments: u32, minor_segments: u32, major_radius: f32, minor_radius: f32) -> Mesh {
    let (nu, nv) = (major_segments.max(3), minor_segments.max(3));
    let mut m = Mesh::new();
    for i in 0..nu {
        let u = TAU * i as f32 / nu as f32;
        for j in 0..nv {
            let v = TAU * j as f32 / nv as f32;
            let r = major_radius + minor_radius * v.cos();
            m.add_vertex(Vec3::new(r * u.cos(), r * u.sin(), minor_radius * v.sin()));
        }
    }
    let id = |i: u32, j: u32| (i % nu) * nv + j % nv;
    for i in 0..nu {
        for j in 0..nv {
            let uv = |i: u32, j: u32| Vec2::new(i as f32 / nu as f32, j as f32 / nv as f32);
            m.faces.push(Face {
                verts: vec![id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1)],
                uvs: vec![uv(i, j), uv(i + 1, j), uv(i + 1, j + 1), uv(i, j + 1)],
                smooth: true,
                material: 0,
            });
        }
    }
    m
}

/// Default-sized primitive by name, as the Add menu and scripts ask for it.
pub fn by_name(name: &str) -> Option<Mesh> {
    Some(match name {
        "cube" => cube(2.0),
        "plane" => plane(2.0),
        "grid" => grid(10, 10, 2.0),
        "uv_sphere" | "sphere" => uv_sphere(32, 16, 1.0),
        "ico_sphere" | "icosphere" => ico_sphere(2, 1.0),
        "cylinder" => cylinder(32, 1.0, 2.0),
        "cone" => cone(32, 1.0, 0.0, 2.0),
        "torus" => torus(48, 12, 1.0, 0.25),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_are_closed_and_outward() {
        for (name, m) in [
            ("cube", cube(2.0)),
            ("uv_sphere", uv_sphere(16, 8, 1.0)),
            ("ico", ico_sphere(2, 1.0)),
            ("cylinder", cylinder(12, 1.0, 2.0)),
            ("cone", cone(12, 1.0, 0.0, 2.0)),
            ("cone_down", cone(12, 0.0, 1.0, 2.0)),
            ("torus", torus(16, 8, 1.0, 0.25)),
        ] {
            m.validate().unwrap();
            let he = m.half_edges();
            assert!(he.is_closed_manifold(), "{name} not closed");
            assert!(m.volume() > 0.0, "{name} faces inward");
            let euler = if name == "torus" { 0 } else { 2 };
            assert_eq!(m.euler_characteristic(), euler, "{name}");
        }
        let s = ico_sphere(3, 1.0);
        assert_eq!(s.face_count(), 20 * 64);
        assert!((s.volume() - 4.0 / 3.0 * PI).abs() < 0.05);
    }
}
