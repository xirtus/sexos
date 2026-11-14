// SPDX-License-Identifier: GPL-3.0-only
//! Golden-style checks for the modifier stack and the edit-mode operations:
//! counts, Euler characteristic, closed-manifoldness, volumes and areas.
use sex_3d::math::Vec3;
use sex_3d::mesh::Mesh;
use sex_3d::modifiers::{apply_stack, subdivide_simple, Modifier};
use sex_3d::ops::{self, BooleanOp};
use sex_3d::primitives;

/// (vertices, edges, faces)
fn counts(m: &Mesh) -> (usize, usize, usize) {
    (m.vertex_count(), m.edges().len(), m.face_count())
}

fn closed(m: &Mesh) -> bool {
    m.validate().is_ok() && m.half_edges().is_closed_manifold()
}

fn near(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

fn assert_near(a: f32, b: f32, tol: f32, what: &str) {
    assert!(near(a, b, tol), "{what}: got {a}, expected {b} (±{tol})");
}

/// Cube [0,2]x[-1,1]x[-1,1] with its -X face (at x = 0) removed.
fn half_cube() -> Mesh {
    let mut c = primitives::cube(2.0);
    c.translate(Vec3::new(1.0, 0.0, 0.0));
    let f = (0..c.face_count()).find(|&f| c.face_normal(f).x < -0.9).unwrap() as u32;
    ops::delete_faces(&mut c, &[f]);
    c
}

fn face_with_normal(m: &Mesh, n: Vec3) -> u32 {
    (0..m.face_count()).find(|&f| m.face_normal(f).dot(n) > 0.99).unwrap() as u32
}

// ---------------------------------------------------------------------------
// Subdivision

#[test]
fn catmull_clark_cube_level_one() {
    let m = Modifier::Subdivision { levels: 1, simple: false }.apply(&primitives::cube(2.0));
    assert_eq!(counts(&m), (26, 48, 24));
    assert_eq!(m.euler_characteristic(), 2);
    assert!(closed(&m));
    assert!(m.has_uvs());
    assert!(m.faces.iter().all(|f| f.len() == 4));
    // Original corner (1,1,1) moves to the CC vertex point: (F + 2R + (n-3)P)/n with n = 3.
    let corner = m.positions[7];
    assert!(corner.approx_eq(Vec3::splat(5.0 / 9.0), 1e-5), "{corner:?}");
}

#[test]
fn catmull_clark_converges_towards_a_rounded_cube() {
    let cube = primitives::cube(2.0);
    let vols: Vec<f32> = (1..=4).map(|l| subdivide(&cube, l).volume()).collect();
    for w in vols.windows(2) {
        assert!(w[1] < w[0], "volume should shrink: {vols:?}");
    }
    let d: Vec<f32> = vols.windows(2).map(|w| w[0] - w[1]).collect();
    assert!(d[1] < d[0] * 0.5 && d[2] < d[1] * 0.5, "should converge: {vols:?}");
    // Catmull-Clark pulls a cube well inside its inscribed sphere (4.19):
    // level 1 is 10/3 exactly, the limit about 2.62.
    assert_near(vols[0], 10.0 / 3.0, 1e-4, "level 1");
    assert!(vols[3] > 2.5 && vols[3] < 2.7, "{vols:?}");
    let m = subdivide(&cube, 4);
    assert_eq!(m.face_count(), 6 * 4usize.pow(4));
    assert!(closed(&m));
    assert_eq!(m.euler_characteristic(), 2);
    let r: Vec<f32> = m.positions.iter().map(|p| p.length()).collect();
    let (lo, hi) = (r.iter().cloned().fold(f32::MAX, f32::min), r.iter().cloned().fold(0.0, f32::max));
    assert!(lo / hi > 0.8, "sphere-ish: radii {lo}..{hi}");
}

fn subdivide(m: &Mesh, levels: u32) -> Mesh {
    Modifier::Subdivision { levels, simple: false }.apply(m)
}

#[test]
fn catmull_clark_handles_triangles_and_ngons() {
    let ico = primitives::ico_sphere(0, 1.0);
    let s = subdivide(&ico, 1);
    // 12 + 30 + 20 vertices, 60 quads.
    assert_eq!(counts(&s), (62, 120, 60));
    assert!(closed(&s));
    let cyl = primitives::cylinder(8, 1.0, 2.0);
    let s = subdivide(&cyl, 2);
    assert!(closed(&s));
    assert_eq!(s.euler_characteristic(), 2);
    assert!(s.volume() > 0.0 && s.volume() < cyl.volume());
}

#[test]
fn catmull_clark_boundary_keeps_corners_and_smooths_edges() {
    let g = primitives::grid(2, 2, 2.0);
    let s = subdivide(&g, 1);
    assert_eq!(counts(&s), (25, 40, 16));
    // Corners stay put.
    assert!(s.positions.iter().any(|p| p.approx_eq(Vec3::new(-1.0, -1.0, 0.0), 1e-6)));
    // Still flat and covering the same square.
    assert!(s.positions.iter().all(|p| p.z.abs() < 1e-6));
    assert_near(s.surface_area(), 4.0, 1e-4, "area");
    assert_eq!(s.half_edges().boundary_half_edges().len(), 16);
}

#[test]
fn simple_subdivision_keeps_shape() {
    let m = subdivide_simple(&primitives::cube(2.0), 2);
    assert_eq!(counts(&m), (98, 192, 96));
    assert!(closed(&m));
    assert_near(m.volume(), 8.0, 1e-4, "volume");
    let same = Modifier::Subdivision { levels: 2, simple: true }.apply(&primitives::cube(2.0));
    assert_eq!(same, m);
}

// ---------------------------------------------------------------------------
// Mirror, array, solidify

#[test]
fn mirror_merges_half_cube_into_closed_box() {
    let half = half_cube();
    assert!(!closed(&half));
    let m = Modifier::Mirror { x: true, y: false, z: false, merge: true, merge_distance: 1e-3 }.apply(&half);
    assert_eq!(counts(&m), (12, 20, 10));
    assert!(closed(&m));
    assert_near(m.volume(), 16.0, 1e-4, "volume");
    // Without merging the seam stays open.
    let open = Modifier::Mirror { x: true, y: false, z: false, merge: false, merge_distance: 0.0 }.apply(&half);
    assert_eq!(open.vertex_count(), 16);
    assert!(!open.half_edges().is_closed_manifold());
}

#[test]
fn mirror_on_three_axes_builds_from_an_octant() {
    // One octant of a cube: three outer faces of the [0,1]^3 cube.
    let mut c = primitives::cube(1.0);
    c.translate(Vec3::splat(0.5));
    let inner: Vec<u32> = (0..c.face_count() as u32).filter(|&f| c.face_normal(f as usize).dot(Vec3::ONE) < 0.0).collect();
    ops::delete_faces(&mut c, &inner);
    let m = Modifier::Mirror { x: true, y: true, z: true, merge: true, merge_distance: 1e-4 }.apply(&c);
    assert!(closed(&m));
    assert_near(m.volume(), 8.0, 1e-4, "volume");
    assert_eq!(m.euler_characteristic(), 2);
}

#[test]
fn array_with_merge_is_one_box() {
    let m = Modifier::Array { count: 3, relative: [1.0, 0.0, 0.0], constant: [0.0; 3], merge: true }.apply(&primitives::cube(2.0));
    assert_eq!(counts(&m), (16, 28, 14));
    assert!(closed(&m));
    assert_near(m.volume(), 24.0, 1e-3, "volume");
    let loose = Modifier::Array { count: 4, relative: [0.0; 3], constant: [0.0, 3.0, 0.0], merge: false }.apply(&primitives::cube(2.0));
    assert_eq!(counts(&loose), (32, 48, 24));
    assert_near(loose.bounds().size().y, 11.0, 1e-5, "extent");
}

#[test]
fn solidify_plane_gives_closed_box() {
    let m = Modifier::Solidify { thickness: 0.5, offset: -1.0 }.apply(&primitives::plane(2.0));
    assert_eq!(counts(&m), (8, 12, 6));
    assert!(closed(&m));
    assert_near(m.volume(), 2.0, 1e-5, "volume");
    assert_near(m.bounds().min.z, -0.5, 1e-6, "grows behind");
    // Centred offset and negative thickness still give an outward box.
    let c = Modifier::Solidify { thickness: -0.5, offset: 0.0 }.apply(&primitives::grid(3, 2, 2.0));
    assert!(closed(&c));
    assert_near(c.volume(), 2.0, 1e-4, "volume");
    assert!(c.has_uvs());
}

#[test]
fn solidify_closed_sphere_is_a_hollow_shell() {
    let s = primitives::uv_sphere(24, 12, 1.0);
    let m = Modifier::Solidify { thickness: 0.1, offset: -1.0 }.apply(&s);
    assert!(closed(&m));
    assert_eq!(m.euler_characteristic(), 4);
    let inner = s.volume() * 0.9f32.powi(3);
    assert_near(m.volume(), s.volume() - inner, 0.03, "shell volume");
}

// ---------------------------------------------------------------------------
// Bevel

/// Volume of a 2-cube with every edge chamfered by `w` (one segment).
fn chamfered_cube_volume(w: f32) -> f32 {
    let a = 2.0 - 2.0 * w;
    a * a * a + 6.0 * a * a * w + 12.0 * a * w * w * 0.5 + 8.0 * w * w * w / 6.0
}

#[test]
fn bevel_modifier_chamfers_every_cube_edge() {
    let m = Modifier::Bevel { width: 0.2, segments: 1, angle: 0.5 }.apply(&primitives::cube(2.0));
    assert_eq!(counts(&m), (24, 48, 26));
    assert!(closed(&m));
    assert_near(m.volume(), chamfered_cube_volume(0.2), 1e-4, "volume");
    assert!(m.has_uvs());
    // Nothing is sharper than 100 degrees on a cube.
    let none = Modifier::Bevel { width: 0.2, segments: 1, angle: 100f32.to_radians() }.apply(&primitives::cube(2.0));
    assert_eq!(counts(&none), (8, 12, 6));
}

#[test]
fn bevel_multi_segment_rounds_edges() {
    let m = Modifier::Bevel { width: 0.3, segments: 4, angle: 0.5 }.apply(&primitives::cube(2.0));
    assert!(closed(&m));
    assert_eq!(m.euler_characteristic(), 2);
    let v = m.volume();
    assert!(v > chamfered_cube_volume(0.3) && v < 8.0, "volume {v}");
    // Rounded edge: the edge profile is a quarter circle of radius 0.3 around
    // (0.7, 0.7) in XY, so its midpoint sits at 0.7 + 0.3 / sqrt 2.
    let target = 0.7 + 0.3 * std::f32::consts::FRAC_1_SQRT_2;
    assert!(m.positions.iter().any(|p| near(p.x, target, 1e-3) && near(p.y, target, 1e-3)), "no arc midpoint");
}

#[test]
fn bevel_single_edge_tapers_into_end_faces() {
    let mut m = primitives::cube(2.0);
    // Edge between +X and +Y faces: vertices (1,1,-1) = 3 and (1,1,1) = 7.
    let new = ops::bevel_edges(&mut m, &[[3, 7]], 0.2, 1);
    assert_eq!(new.len(), 1);
    assert_eq!(counts(&m), (10, 15, 7));
    assert!(closed(&m));
    assert_near(m.volume(), 8.0 - 0.2 * 0.2 * 0.5 * 2.0, 1e-4, "volume");
}

#[test]
fn bevel_edge_loop_on_cylinder() {
    let mut m = primitives::cylinder(16, 1.0, 2.0);
    // Top rim edges.
    let top: Vec<[u32; 2]> = m.edges().into_iter().filter(|e| e.iter().all(|&v| m.positions[v as usize].z > 0.9)).collect();
    assert_eq!(top.len(), 16);
    let v0 = m.volume();
    ops::bevel_edges(&mut m, &top, 0.1, 3);
    assert!(closed(&m));
    assert_eq!(m.euler_characteristic(), 2);
    assert!(m.volume() < v0 && m.volume() > v0 - 0.1);
}

// ---------------------------------------------------------------------------
// Other modifiers

#[test]
fn triangulate_weld_displace_smooth() {
    let cube = primitives::cube(2.0);
    let t = Modifier::Triangulate.apply(&cube);
    assert_eq!(counts(&t), (8, 18, 12));
    assert!(closed(&t) && t.has_uvs());
    assert_near(t.volume(), 8.0, 1e-5, "volume");

    let mut split = Mesh::new();
    for f in &cube.faces {
        let base = split.positions.len() as u32;
        split.positions.extend(f.verts.iter().map(|&v| cube.positions[v as usize]));
        split.add_face(&[base, base + 1, base + 2, base + 3]);
    }
    let w = Modifier::Weld { distance: 1e-4 }.apply(&split);
    assert_eq!(counts(&w), (8, 12, 6));
    assert!(closed(&w));

    let s = primitives::ico_sphere(2, 1.0);
    let d = Modifier::Displace { strength: 0.5 }.apply(&s);
    assert_near(d.volume(), s.volume() * 1.5f32.powi(3), 0.05, "displaced volume");

    let sm = apply_stack(&subdivide_simple(&cube, 2), &[Modifier::Smooth { factor: 0.5, iterations: 10 }]);
    assert!(closed(&sm));
    assert!(sm.volume() < 7.5 && sm.volume() > 2.0, "smoothed volume {}", sm.volume());
}

#[test]
fn modifier_stack_runs_in_order() {
    let stack = [
        Modifier::Mirror { x: true, y: false, z: false, merge: true, merge_distance: 1e-3 },
        Modifier::Subdivision { levels: 1, simple: false },
        Modifier::Triangulate,
    ];
    let m = apply_stack(&half_cube(), &stack);
    assert!(closed(&m));
    assert_eq!(m.face_count(), 10 * 4 * 2);
    assert!(m.volume() > 6.0 && m.volume() < 16.0);
}

// ---------------------------------------------------------------------------
// Extrude and inset

#[test]
fn extrude_top_face_of_cube() {
    let mut m = primitives::cube(2.0);
    let top = face_with_normal(&m, Vec3::Z);
    let sides = ops::extrude_faces(&mut m, &[top], 1.0);
    assert_eq!(sides.len(), 4);
    assert_eq!(counts(&m), (12, 20, 10));
    assert!(closed(&m));
    assert_near(m.volume(), 12.0, 1e-4, "volume");
    assert_near(m.face_center(top as usize).z, 2.0, 1e-6, "top moved");
    assert!(m.has_uvs());
}

#[test]
fn region_extrude_keeps_shared_edges() {
    let mut m = primitives::grid(2, 1, 2.0);
    let sides = ops::extrude_faces(&mut m, &[0, 1], 0.5);
    // 6 border edges -> 6 sides; the middle edge stays shared, not doubled.
    assert_eq!(sides.len(), 6);
    assert_eq!(counts(&m), (12, 19, 8));
    assert_eq!(m.half_edges().boundary_half_edges().len(), 6);
    // Extruding two faces of a cube together makes an L-shaped step.
    let mut c = primitives::cube(2.0);
    let (top, side) = (face_with_normal(&c, Vec3::Z), face_with_normal(&c, Vec3::X));
    ops::extrude_faces(&mut c, &[top, side], 0.5);
    assert!(closed(&c));
    assert_eq!(c.euler_characteristic(), 2);
    let individually = {
        let mut c = primitives::cube(2.0);
        ops::extrude_faces_individual(&mut c, &[top, side], 0.5);
        c
    };
    assert!(closed(&individually));
    assert_near(individually.volume(), 8.0 + 2.0 * 4.0 * 0.5, 1e-4, "individual volume");
}

#[test]
fn inset_individual_and_region() {
    let mut m = primitives::cube(2.0);
    let top = face_with_normal(&m, Vec3::Z);
    let ring = ops::inset_faces_individual(&mut m, &[top], 0.25, 0.0);
    assert_eq!(ring.len(), 4);
    assert_eq!(counts(&m), (12, 20, 10));
    assert!(closed(&m));
    assert_near(m.volume(), 8.0, 1e-4, "volume");
    assert_near(m.face_area(top as usize), 1.5 * 1.5, 1e-5, "inner area");
    assert!(m.has_uvs());
    // UVs of the inner face are the matching fraction of the 0..1 square.
    let uvs = &m.faces[top as usize].uvs;
    assert!(uvs.iter().all(|uv| near(uv.x, 0.125, 1e-5) || near(uv.x, 0.875, 1e-5)), "{uvs:?}");

    // Region inset of a 2x2 grid: one inner region, one ring of 8 quads.
    let mut g = primitives::grid(2, 2, 2.0);
    let ring = ops::inset_region(&mut g, &[0, 1, 2, 3], 0.2, 0.1);
    assert_eq!(ring.len(), 8);
    assert_eq!(counts(&g), (17, 28, 12));
    assert_eq!(g.euler_characteristic(), 1);
    let inner_area: f32 = (0..4).map(|f| g.face_area(f)).sum();
    assert_near(inner_area, 1.6 * 1.6, 1e-4, "region area");
    assert!(g.faces[..4].iter().flat_map(|f| &f.verts).all(|&v| near(g.positions[v as usize].z, 0.1, 1e-6)));
}

// ---------------------------------------------------------------------------
// Loop cut, bisect

#[test]
fn loop_cut_around_cube() {
    let mut m = primitives::cube(2.0);
    let new = ops::loop_cut(&mut m, [0, 1], 1);
    assert_eq!(new.len(), 4);
    assert_eq!(counts(&m), (12, 20, 10));
    assert!(closed(&m));
    assert_near(m.volume(), 8.0, 1e-4, "volume");
    // The cut is the x = 0 loop.
    assert!(new.iter().all(|&v| m.positions[v as usize].x.abs() < 1e-6));

    let mut m = primitives::cube(2.0);
    ops::loop_cut(&mut m, [0, 1], 3);
    assert_eq!(counts(&m), (20, 36, 18));
    assert!(closed(&m) && m.has_uvs());
}

#[test]
fn loop_cut_stops_at_boundary_and_ngons() {
    let mut g = primitives::grid(3, 2, 3.0);
    // Edge along X in the bottom row: ring runs up the column (2 quads).
    ops::loop_cut(&mut g, [0, 1], 2);
    assert_eq!(counts(&g), (12 + 6, 17 + 10, 6 + 4));
    assert_eq!(g.euler_characteristic(), 1);

    // Cylinder side quads ring around; the caps are n-gons that get the new vertices.
    let mut c = primitives::cylinder(8, 1.0, 2.0);
    let e = c.edges().into_iter().find(|e| (c.positions[e[0] as usize].z - c.positions[e[1] as usize].z).abs() > 1.0).unwrap();
    let new = ops::loop_cut(&mut c, e, 1);
    assert_eq!(new.len(), 8);
    assert!(closed(&c));
    // A vertical edge's ring goes around the side: the cut is the z = 0 loop.
    assert!(new.iter().all(|&v| c.positions[v as usize].z.abs() < 1e-5));
}

#[test]
fn bisect_cube_and_fill() {
    let mut m = primitives::cube(2.0);
    let cut = ops::bisect(&mut m, Vec3::ZERO, Vec3::Z, true, false, true);
    assert_eq!(cut.len(), 4);
    assert!(closed(&m));
    assert_near(m.volume(), 4.0, 1e-4, "half volume");
    assert_eq!(counts(&m), (8, 12, 6));

    // Diagonal cut through a sphere, keeping both halves split.
    let mut s = primitives::uv_sphere(16, 8, 1.0);
    let v = s.volume();
    ops::bisect(&mut s, Vec3::new(0.1, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0), false, false, false);
    assert!(closed(&s));
    assert_near(s.volume(), v, 1e-4, "split keeps volume");
    let mut s2 = s.clone();
    ops::bisect(&mut s2, Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), true, true, false);
    assert!(closed(&s2));
    assert!(s2.volume() > 0.0 && s2.volume() < v);
}

// ---------------------------------------------------------------------------
// Booleans

fn offset_cube(d: Vec3) -> Mesh {
    let mut c = primitives::cube(2.0);
    c.translate(d);
    c
}

#[test]
fn boolean_cube_minus_offset_cube() {
    let a = primitives::cube(2.0);
    let b = offset_cube(Vec3::splat(1.0));
    let d = ops::boolean(&a, &b, BooleanOp::Difference);
    assert!(closed(&d), "difference not closed");
    assert_eq!(d.euler_characteristic(), 2);
    assert_near(d.volume(), 7.0, 1e-3, "difference");
    // Coplanar fragments are merged back: 3 L-shaped faces, 3 untouched, 3 inner.
    assert_eq!(d.face_count(), 9);

    let u = ops::boolean(&a, &b, BooleanOp::Union);
    assert!(closed(&u), "union not closed");
    assert_near(u.volume(), 15.0, 1e-3, "union");
    assert_eq!(u.euler_characteristic(), 2);

    let i = ops::boolean(&a, &b, BooleanOp::Intersect);
    assert!(closed(&i), "intersect not closed");
    assert_near(i.volume(), 1.0, 1e-3, "intersect");
    assert_eq!(counts(&i), (8, 12, 6));
}

#[test]
fn boolean_drills_a_hole() {
    let a = primitives::cube(2.0);
    let b = primitives::cylinder(16, 0.5, 4.0);
    let d = ops::boolean(&a, &b, BooleanOp::Difference);
    assert!(closed(&d));
    assert_eq!(d.euler_characteristic(), 0, "genus one");
    let hole = 0.5 * 16.0 * 0.25 * (std::f32::consts::TAU / 16.0).sin() * 2.0;
    assert_near(d.volume(), 8.0 - hole, 1e-3, "drilled volume");
}

#[test]
fn boolean_with_coplanar_faces() {
    let a = primitives::cube(2.0);
    let b = offset_cube(Vec3::new(1.0, 0.0, 0.0));
    let u = ops::boolean(&a, &b, BooleanOp::Union);
    assert!(closed(&u));
    assert_near(u.volume(), 12.0, 1e-3, "union");
    assert_eq!(counts(&u), (8, 12, 6));
    let d = ops::boolean(&a, &b, BooleanOp::Difference);
    assert!(closed(&d));
    assert_near(d.volume(), 4.0, 1e-3, "difference");
}

#[test]
fn boolean_sphere_and_cube() {
    let a = primitives::ico_sphere(2, 1.2);
    let b = primitives::cube(2.0);
    let i = ops::boolean(&a, &b, BooleanOp::Intersect);
    assert!(closed(&i));
    assert!(i.volume() < 8.0 && i.volume() < a.volume());
    let d = ops::boolean(&a, &b, BooleanOp::Difference);
    let u = ops::boolean(&a, &b, BooleanOp::Union);
    assert!(closed(&d) && closed(&u));
    // Inclusion-exclusion.
    assert_near(u.volume(), a.volume() + b.volume() - i.volume(), 1e-3, "union");
    assert_near(d.volume(), a.volume() - i.volume(), 1e-3, "difference");
}

// ---------------------------------------------------------------------------
// Delete, merge, normals, holes, bridge, spin, dissolve

#[test]
fn delete_and_fill_hole() {
    let mut m = primitives::cube(2.0);
    let top = face_with_normal(&m, Vec3::Z);
    ops::delete_faces(&mut m, &[top]);
    assert_eq!(counts(&m), (8, 12, 5));
    assert_eq!(ops::boundary_loops(&m).len(), 1);
    let f = ops::fill_hole(&mut m, 7).unwrap();
    assert!(closed(&m));
    assert_near(m.volume(), 8.0, 1e-5, "volume");
    assert!(m.face_normal(f as usize).z > 0.99);
    assert!(m.has_uvs());

    let mut m = primitives::cube(2.0);
    ops::delete_verts(&mut m, &[7]);
    assert_eq!(counts(&m), (7, 9, 3));
    assert_eq!(ops::fill_holes(&mut m).len(), 1);
    assert!(closed(&m));
    assert_eq!(m.face_count(), 4);
}

#[test]
fn merge_at_center_collapses_a_face() {
    let mut m = primitives::cube(2.0);
    let top = face_with_normal(&m, Vec3::Z);
    let verts = m.faces[top as usize].verts.clone();
    let v = ops::merge_at_center(&mut m, &verts).unwrap();
    // A pyramid: 5 vertices, 4 triangles + bottom quad.
    assert_eq!(counts(&m), (5, 8, 5));
    assert!(closed(&m));
    assert!(m.positions[v as usize].approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-6));
    assert_near(m.volume(), 4.0 * 2.0 / 3.0, 1e-4, "pyramid");
}

#[test]
fn recalc_normals_fixes_scrambled_winding() {
    let mut s = primitives::uv_sphere(16, 8, 1.0);
    let v = s.volume();
    let scrambled: Vec<u32> = (0..s.face_count() as u32).filter(|f| f % 3 == 0).collect();
    ops::flip_normals(&mut s, &scrambled);
    assert!(!s.half_edges().is_closed_manifold());
    let n = ops::recalc_normals_outside(&mut s);
    assert_eq!(n, scrambled.len());
    assert!(closed(&s));
    assert_near(s.volume(), v, 1e-5, "volume");

    // A fully inverted cube is turned outside in again.
    let mut c = primitives::cube(2.0);
    c.flip_normals();
    assert!(c.volume() < 0.0);
    assert_eq!(ops::recalc_normals_outside(&mut c), 6);
    assert_near(c.volume(), 8.0, 1e-5, "volume");
}

#[test]
fn bridge_two_planes_into_a_box() {
    let mut m = primitives::plane(2.0);
    m.flip_normals(); // bottom faces down
    let mut top = primitives::plane(2.0);
    top.translate(Vec3::new(0.0, 0.0, 1.0));
    let off = m.append(&top);
    let faces = ops::bridge_edge_loops(&mut m, 0, off).unwrap();
    assert_eq!(faces.len(), 4);
    assert!(closed(&m));
    assert_near(m.volume(), 4.0, 1e-5, "volume");
    // Faces join straight up, not twisted.
    assert!(faces.iter().all(|&f| m.face_normal(f as usize).z.abs() < 1e-5));
    assert!(ops::bridge_edge_loops(&mut m, 0, off).is_err());

    // Unequal loops (square to octagon) bridge with triangles.
    let mut a = primitives::grid(1, 1, 2.0);
    a.flip_normals();
    let mut cyl = primitives::cylinder(8, 1.0, 1.0);
    let bottom = face_with_normal(&cyl, -Vec3::Z);
    ops::delete_faces(&mut cyl, &[bottom]);
    cyl.translate(Vec3::new(0.0, 0.0, 1.5));
    let off = a.append(&cyl);
    let faces = ops::bridge_edge_loops(&mut a, 0, off).unwrap();
    assert_eq!(faces.len(), 12);
    assert!(closed(&a));
    assert!(a.volume() > 0.0);
}

#[test]
fn spin_profile_into_a_closed_lathe() {
    // Half-circle-ish profile from the south pole to the north pole.
    let mut m = Mesh::new();
    let pts = [(0.0, -1.0), (0.7, -0.7), (1.0, 0.0), (0.7, 0.7), (0.0, 1.0)];
    let profile: Vec<u32> = pts.iter().map(|&(x, z)| m.add_vertex(Vec3::new(x, 0.0, z))).collect();
    let faces = ops::spin(&mut m, &profile, Vec3::ZERO, Vec3::Z, std::f32::consts::TAU, 12, 0.0);
    assert_eq!(faces.len(), 4 * 12);
    // Poles are repeated per ring; weld them.
    m.weld(1e-5);
    assert!(closed(&m));
    assert_eq!(m.euler_characteristic(), 2);
    assert!(m.volume() > 2.5);

    // Screw: a half turn with advance keeps separate rings.
    let mut s = Mesh::new();
    let p: Vec<u32> = [1.0f32, 1.5].iter().map(|&x| s.add_vertex(Vec3::new(x, 0.0, 0.0))).collect();
    ops::spin(&mut s, &p, Vec3::ZERO, Vec3::Z, std::f32::consts::PI, 6, 2.0);
    assert_eq!(counts(&s).0, 14);
    assert_near(s.bounds().max.z, 2.0, 1e-5, "advance");
}

#[test]
fn dissolve_coplanar_undoes_simple_subdivision() {
    let mut m = subdivide_simple(&primitives::cube(2.0), 2);
    let removed = ops::dissolve_coplanar(&mut m, 1e-3);
    assert_eq!(removed, 96 - 6);
    assert_eq!(counts(&m), (8, 12, 6));
    assert!(closed(&m) && m.has_uvs());
    assert_near(m.volume(), 8.0, 1e-4, "volume");
}
