// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::math::Vec3;
use crate::primitives;
use crate::scene::Scene;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Valid indices, finite positions, and the face/vertex counts.
fn check(m: &Mesh) -> (usize, usize) {
    m.validate().unwrap();
    assert!(m.positions.iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite()), "NaN in output");
    (m.vertex_count(), m.face_count())
}

fn closed(m: &Mesh) -> bool {
    m.half_edges().is_closed_manifold()
}

fn defaults(ty: &str) -> Modifier {
    Modifier::with_defaults(ty).unwrap()
}

// ---------------------------------------------------------------------------
// Decimate

#[test]
fn decimate_collapse_halves_triangles() {
    let s = primitives::ico_sphere(3, 1.0);
    let n = s.face_count();
    let m = Modifier::Decimate { mode: DecimateMode::Collapse, ratio: 0.5, iterations: 0, angle: 0.1, triangulate: false }.apply(&s);
    let (_, f) = check(&m);
    let want = n as f32 * 0.5;
    assert!((f as f32 - want).abs() <= want * 0.1, "{f} faces from {n}");
    assert!(closed(&m), "collapse keeps a closed surface closed");
    let b = m.bounds();
    assert!(b.max.max_elem() <= 1.01 && b.min.x >= -1.01 && b.size().min_elem_test() > 1.8, "{b:?}");
    // Every vertex stays near the sphere.
    assert!(m.positions.iter().all(|p| (p.length() - 1.0).abs() < 0.08));
}

trait MinElem {
    fn min_elem_test(self) -> f32;
}
impl MinElem for Vec3 {
    fn min_elem_test(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }
}

#[test]
fn decimate_collapse_quads_and_triangulate_flag() {
    let s = primitives::uv_sphere(32, 16, 1.0);
    let tris = s.triangulate().len();
    let all_tris = Modifier::Decimate { mode: DecimateMode::Collapse, ratio: 0.5, iterations: 0, angle: 0.1, triangulate: true }.apply(&s);
    check(&all_tris);
    assert!(all_tris.faces.iter().all(|f| f.verts.len() == 3));
    let got = all_tris.face_count() as f32;
    assert!((got - tris as f32 * 0.5).abs() <= tris as f32 * 0.05, "{got} of {tris}");
    // Ratio 1 changes nothing; a low ratio still gives a valid mesh.
    assert_eq!(defaults("DECIMATE").apply(&s), s);
    let tiny = Modifier::Decimate { mode: DecimateMode::Collapse, ratio: 0.05, iterations: 0, angle: 0.1, triangulate: false }.apply(&s);
    let (_, f) = check(&tiny);
    assert!(f < tris / 10 && f >= 4, "{f}");
}

#[test]
fn decimate_collapse_keeps_open_boundaries() {
    let g = Modifier::Triangulate.apply(&primitives::grid(10, 10, 2.0));
    let m = Modifier::Decimate { mode: DecimateMode::Collapse, ratio: 0.3, iterations: 0, angle: 0.1, triangulate: false }.apply(&g);
    check(&m);
    let b = m.bounds();
    assert!((b.size().x - 2.0).abs() < 1e-4 && (b.size().y - 2.0).abs() < 1e-4, "outline kept: {b:?}");
    assert!(m.positions.iter().all(|p| p.z.abs() < 1e-5), "stays flat");
    assert!((m.surface_area() - 4.0).abs() < 0.05, "area {}", m.surface_area());
}

#[test]
fn decimate_unsubdivide_undoes_subdivision() {
    let cube = primitives::cube(2.0);
    for levels in [1, 2] {
        for simple in [true, false] {
            let sub = Modifier::Subdivision { levels, simple }.apply(&cube);
            let m = Modifier::Decimate { mode: DecimateMode::Unsubdivide, ratio: 1.0, iterations: levels, angle: 0.1, triangulate: false }.apply(&sub);
            assert_eq!(check(&m), (8, 6), "levels {levels} simple {simple}");
            assert!(closed(&m));
            if simple {
                assert!(m.positions.iter().all(|p| (p.abs() - Vec3::ONE).length() < 1e-5));
                assert!((m.volume() - 8.0).abs() < 1e-3);
            }
        }
    }
    let grid = primitives::grid(4, 4, 2.0);
    let m = Modifier::Decimate { mode: DecimateMode::Unsubdivide, ratio: 1.0, iterations: 1, angle: 0.1, triangulate: false }.apply(&grid);
    assert_eq!(check(&m), (9, 4));
    assert!(m.has_uvs());
    // Zero iterations, or a mesh that is not a subdivision, pass through.
    assert_eq!(defaults("DECIMATE").apply(&grid), grid);
    let ico = primitives::ico_sphere(1, 1.0);
    assert_eq!(Modifier::Decimate { mode: DecimateMode::Unsubdivide, ratio: 1.0, iterations: 2, angle: 0.1, triangulate: false }.apply(&ico), ico);
}

#[test]
fn decimate_planar_merges_flat_faces() {
    let m = Modifier::Decimate { mode: DecimateMode::Planar, ratio: 1.0, iterations: 0, angle: 5f32.to_radians(), triangulate: false }.apply(&primitives::grid(4, 4, 2.0));
    assert_eq!(check(&m).1, 1);
    let c = Modifier::Decimate { mode: DecimateMode::Planar, ratio: 1.0, iterations: 0, angle: 5f32.to_radians(), triangulate: false }.apply(&subdivide_simple(&primitives::cube(2.0), 2));
    assert_eq!(check(&c), (8, 6));
}

// ---------------------------------------------------------------------------
// Remesh

#[test]
fn remesh_voxel_sphere_is_closed_manifold() {
    let s = primitives::uv_sphere(32, 16, 1.0);
    let m = defaults("REMESH").apply(&s);
    let (_, f) = check(&m);
    assert!(f > 500, "{f} faces");
    assert!(closed(&m), "voxel remesh is a closed manifold");
    assert!(m.faces.iter().all(|f| f.verts.len() == 4));
    let b = m.bounds();
    assert!((b.max.x - 1.0).abs() < 0.1 && (b.min.z + 1.0).abs() < 0.1, "{b:?}");
    let v = m.volume();
    assert!((v - 4.0 / 3.0 * PI).abs() < 0.15 * 4.0, "volume {v}");
    // Every vertex lies near the original surface.
    assert!(m.positions.iter().all(|p| (p.length() - 1.0).abs() < 0.1));
}

#[test]
fn remesh_voxel_cube_union_and_open_mesh() {
    // Two overlapping cubes union into one closed shell.
    let mut two = primitives::cube(2.0);
    let mut other = primitives::cube(2.0);
    other.translate(Vec3::new(1.0, 0.0, 0.0));
    two.append(&other);
    let m = Modifier::Remesh { mode: RemeshMode::Voxel, voxel_size: 0.2, octree_depth: 4, scale: 0.9, smooth_shade: true }.apply(&two);
    check(&m);
    assert!(closed(&m));
    assert!(m.faces.iter().all(|f| f.smooth));
    assert!((m.volume() - 12.0).abs() < 1.2, "union volume {}", m.volume());
    // An open plane becomes a thin closed slab.
    let p = Modifier::Remesh { mode: RemeshMode::Voxel, voxel_size: 0.1, octree_depth: 4, scale: 0.9, smooth_shade: false }.apply(&primitives::plane(2.0));
    check(&p);
    assert!(closed(&p));
    assert!(p.bounds().size().z < 0.4);
}

#[test]
fn remesh_blocks_and_smooth_modes() {
    let s = primitives::uv_sphere(24, 12, 1.0);
    let blocks = Modifier::Remesh { mode: RemeshMode::Blocks, voxel_size: 0.1, octree_depth: 3, scale: 0.9, smooth_shade: false }.apply(&s);
    let (_, f) = check(&blocks);
    assert!(f > 20);
    // Faces are axis aligned squares.
    for i in 0..blocks.face_count() {
        let n = blocks.face_normal(i);
        assert!((n.abs().max_elem() - 1.0).abs() < 1e-5);
    }
    assert!(blocks.volume() > 0.0);
    let smooth = Modifier::Remesh { mode: RemeshMode::Smooth, voxel_size: 0.1, octree_depth: 4, scale: 0.9, smooth_shade: false }.apply(&s);
    check(&smooth);
    assert!(closed(&smooth));
    assert!((smooth.volume() - 4.0 / 3.0 * PI).abs() < 0.8);
}

// ---------------------------------------------------------------------------
// Screw, Wireframe, Edge Split, Build

fn edge_mesh(a: Vec3, b: Vec3) -> Mesh {
    let mut m = Mesh::new();
    m.add_vertex(a);
    m.add_vertex(b);
    m.loose_edges.push([0, 1]);
    m
}

#[test]
fn screw_sweeps_a_profile() {
    let e = edge_mesh(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0));
    let m = defaults("SCREW").apply(&e);
    assert_eq!(check(&m), (32, 16));
    assert!(m.faces.iter().all(|f| f.smooth));
    // A tube: every face points away from the axis.
    for i in 0..m.face_count() {
        let c = m.face_center(i);
        assert!(m.face_normal(i).dot(Vec3::new(c.x, c.y, 0.0)) > 0.0);
    }
    let he = m.half_edges();
    assert_eq!(he.boundary_half_edges().len(), 32, "open only at top and bottom");
    // Two turns rising 1 per turn.
    let spiral = Modifier::Screw { axis: 2, angle: TAU, steps: 8, screw_offset: 1.0, iterations: 2, merge: false, merge_distance: 0.01, flip: true, smooth_shade: false }.apply(&e);
    assert_eq!(check(&spiral), (34, 16));
    assert!((spiral.bounds().max.z - 3.0).abs() < 1e-4);
    // A cone: the vertex on the axis is merged.
    let cone = Modifier::Screw { axis: 2, angle: TAU, steps: 12, screw_offset: 0.0, iterations: 1, merge: true, merge_distance: 0.01, flip: false, smooth_shade: true }.apply(&edge_mesh(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0)));
    assert_eq!(check(&cone), (13, 12));
    assert!(cone.faces.iter().all(|f| f.verts.len() == 3));
    // Half a turn around X does not close.
    let half = Modifier::Screw { axis: 0, angle: PI, steps: 4, screw_offset: 0.0, iterations: 1, merge: false, merge_distance: 0.01, flip: false, smooth_shade: true }.apply(&e);
    assert_eq!(check(&half), (10, 4));
    // A closed profile (a square's edges) makes a closed torus-like solid.
    let mut ring = Mesh::new();
    ring.positions = vec![Vec3::new(0.75, 0.0, -0.25), Vec3::new(1.25, 0.0, -0.25), Vec3::new(0.75, 0.0, 0.25), Vec3::new(1.25, 0.0, 0.25)];
    ring.loose_edges = vec![[0, 1], [1, 3], [3, 2], [2, 0]];
    let t = defaults("SCREW").apply(&ring);
    check(&t);
    assert!(closed(&t));
}

#[test]
fn wireframe_cube_is_closed_tubes() {
    let m = defaults("WIREFRAME").apply(&primitives::cube(2.0));
    assert_eq!(check(&m), (64, 72));
    assert!(closed(&m));
    let b = m.bounds();
    assert!((b.max.x - 1.0).abs() < 1e-5, "offset -1 grows inwards: {b:?}");
    assert!(m.volume() > 0.0);
    // Keep the original faces too.
    let keep = Modifier::Wireframe { thickness: 0.1, offset: 0.0, even: false, relative: true, boundary: false, replace: false }.apply(&primitives::cube(2.0));
    assert_eq!(check(&keep).1, 78);
    // Boundary edges only get tubes with `boundary`.
    let plane = primitives::plane(2.0);
    let none = defaults("WIREFRAME").apply(&plane);
    assert_eq!(check(&none), (0, 0));
    let framed = Modifier::Wireframe { thickness: 0.1, offset: -1.0, even: true, relative: false, boundary: true, replace: true }.apply(&plane);
    assert_eq!(check(&framed).1, 16);
    assert!(closed(&framed));
}

#[test]
fn edge_split_by_angle() {
    let cube = primitives::cube(2.0);
    let m = defaults("EDGE_SPLIT").apply(&cube);
    assert_eq!(check(&m), (24, 6));
    assert_eq!(m.half_edges().boundary_half_edges().len(), 24);
    let none = Modifier::EdgeSplit { angle: 100f32.to_radians(), use_angle: true }.apply(&cube);
    assert_eq!(check(&none), (8, 6));
    let off = Modifier::EdgeSplit { angle: 0.0, use_angle: false }.apply(&cube);
    assert_eq!(check(&off), (8, 6));
    let s = primitives::uv_sphere(32, 16, 1.0);
    assert_eq!(defaults("EDGE_SPLIT").apply(&s).vertex_count(), s.vertex_count());
}

#[test]
fn build_reveals_faces_over_time() {
    let g = primitives::grid(4, 4, 2.0);
    let at = |frame: f32, md: &Modifier| md.apply_with(&g, &EvalContext { frame, ..Default::default() });
    let md = defaults("BUILD");
    assert_eq!(check(&at(1.0, &md)), (0, 0));
    assert_eq!(check(&at(51.0, &md)).1, 8);
    assert_eq!(at(200.0, &md), g);
    let rev = Modifier::Build { start: 1.0, length: 100.0, reverse: true, random: false, seed: 0 };
    assert_eq!(at(1.0, &rev), g);
    assert_eq!(check(&at(76.0, &rev)).1, 4);
    let rnd = Modifier::Build { start: 1.0, length: 100.0, reverse: false, random: true, seed: 7 };
    let a = at(51.0, &rnd);
    assert_eq!(check(&a).1, 8);
    assert_ne!(a.faces, at(51.0, &md).faces, "shuffled");
    assert_eq!(a, at(51.0, &rnd), "deterministic");
    assert!(md.is_animated() && rnd.is_animated());
}

#[test]
fn boolean_uses_the_lookup() {
    let cube = primitives::cube(2.0);
    let md = Modifier::Boolean { object: "Cutter".into(), operation: BooleanOp::Difference };
    assert_eq!(md.apply(&cube), cube, "no scene: pass through");
    let lookup = |name: &str| {
        (name == "Cutter").then(|| {
            let mut c = primitives::cube(2.0);
            c.translate(Vec3::new(1.0, 0.0, 0.0));
            c
        })
    };
    let cx = EvalContext { object: Some(&lookup), ..Default::default() };
    let m = md.apply_with(&cube, &cx);
    check(&m);
    assert!((m.volume() - 4.0).abs() < 1e-3, "{}", m.volume());
    assert!(closed(&m));
    let u = Modifier::Boolean { object: "Cutter".into(), operation: BooleanOp::Union }.apply_with(&cube, &cx);
    assert!((u.volume() - 12.0).abs() < 1e-3);
    let missing = Modifier::Boolean { object: "Nope".into(), operation: BooleanOp::Union }.apply_with(&cube, &cx);
    assert_eq!(missing, cube);
}

// ---------------------------------------------------------------------------
// Deform

fn column() -> Mesh {
    // A 2x2x4 box from z = 0 to 4, subdivided so deforms have vertices to move.
    let mut c = subdivide_simple(&primitives::cube(1.0), 3);
    for p in &mut c.positions {
        *p = Vec3::new(p.x * 2.0, p.y * 2.0, (p.z + 0.5) * 4.0);
    }
    c
}

#[test]
fn simple_deform_modes() {
    let c = column();
    // Twist 90 degrees per unit of Z: the top corner turns by 4 * 90 degrees.
    let tw = Modifier::SimpleDeform { mode: SimpleDeformMode::Twist, axis: 2, factor: FRAC_PI_2 / 4.0, limits: [0.0, 1.0], lock: [false; 3], vertex_group: None, invert_vertex_group: false }.apply(&c);
    check(&tw);
    let top = c.positions.iter().position(|p| (*p - Vec3::new(1.0, 0.0, 4.0)).length() < 1e-4).unwrap();
    assert!((tw.positions[top] - Vec3::new(0.0, 1.0, 4.0)).length() < 1e-4, "{:?}", tw.positions[top]);
    assert!((tw.volume() - c.volume()).abs() < 0.3);
    // Limits: the lower half does not twist.
    let lim = Modifier::SimpleDeform { mode: SimpleDeformMode::Twist, axis: 2, factor: 1.0, limits: [0.5, 1.0], lock: [false; 3], vertex_group: None, invert_vertex_group: false }.apply(&c);
    let low = c.positions.iter().position(|p| (*p - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-4).unwrap();
    let mid = 2.0f32;
    let want = Vec3::new(mid.cos() - mid.sin(), mid.sin() + mid.cos(), 0.0);
    assert!((lim.positions[low] - want).length() < 1e-4, "clamped to the lower limit: {:?}", lim.positions[low]);
    // Taper widens the top.
    let tp = Modifier::SimpleDeform { mode: SimpleDeformMode::Taper, axis: 2, factor: 0.25, limits: [0.0, 1.0], lock: [false; 3], vertex_group: None, invert_vertex_group: false }.apply(&c);
    check(&tp);
    assert!((tp.positions[top].x - 2.0).abs() < 1e-4);
    // Stretch lengthens along the axis.
    let st = Modifier::SimpleDeform { mode: SimpleDeformMode::Stretch, axis: 2, factor: 0.5, limits: [0.0, 1.0], lock: [true, true, false], vertex_group: None, invert_vertex_group: false }.apply(&c);
    check(&st);
    assert!((st.bounds().max.z - 6.0).abs() < 1e-4);
    assert!((st.bounds().size().x - 2.0).abs() < 1e-4, "locked X");
    // Bend a strip along X into a half circle around Z.
    let strip = primitives::grid(32, 1, PI);
    let bend = Modifier::SimpleDeform { mode: SimpleDeformMode::Bend, axis: 2, factor: 1.0, limits: [0.0, 1.0], lock: [false; 3], vertex_group: None, invert_vertex_group: false }.apply(&strip);
    check(&bend);
    for (p, q) in strip.positions.iter().zip(&bend.positions) {
        let r = p.y - 1.0;
        let d = (q.x * q.x + (q.y - 1.0) * (q.y - 1.0)).sqrt();
        assert!((d - r.abs()).abs() < 1e-3, "on the circle: {q:?}");
    }
    assert_eq!(defaults("SIMPLE_DEFORM").apply(&Mesh::new()), Mesh::new());
}

#[test]
fn cast_to_sphere_cylinder_and_box() {
    let c = subdivide_simple(&primitives::cube(2.0), 3);
    let s = Modifier::Cast { kind: CastType::Sphere, factor: 1.0, radius: 0.0, size: 0.0, axes: [true; 3], size_from_radius: false, vertex_group: None, invert_vertex_group: false }.apply(&c);
    check(&s);
    let r0 = s.positions[0].length();
    assert!(s.positions.iter().all(|p| (p.length() - r0).abs() < 1e-4));
    let half = defaults("CAST").apply(&c);
    check(&half);
    assert!(half.positions.iter().zip(&c.positions).zip(&s.positions).all(|((h, a), b)| (*h - (*a + *b) * 0.5).length() < 1e-4));
    let cyl = Modifier::Cast { kind: CastType::Cylinder, factor: 1.0, radius: 0.0, size: 1.5, axes: [true; 3], size_from_radius: false, vertex_group: None, invert_vertex_group: false }.apply(&c);
    check(&cyl);
    assert!(cyl.positions.iter().zip(&c.positions).all(|(p, o)| (o.x == 0.0 && o.y == 0.0) || (Vec3::new(p.x, p.y, 0.0).length() - 1.5).abs() < 1e-4 && p.z == o.z));
    let sphere = primitives::uv_sphere(16, 8, 1.0);
    let bx = Modifier::Cast { kind: CastType::Cuboid, factor: 1.0, radius: 0.0, size: 0.0, axes: [true; 3], size_from_radius: false, vertex_group: None, invert_vertex_group: false }.apply(&sphere);
    check(&bx);
    assert!(bx.positions.iter().all(|p| (p.abs().max_elem() - 1.0).abs() < 1e-4));
    // Radius limits the effect.
    let lim = Modifier::Cast { kind: CastType::Sphere, factor: 1.0, radius: 1.2, size: 1.0, axes: [true; 3], size_from_radius: false, vertex_group: None, invert_vertex_group: false }.apply(&c);
    let corner = c.positions.iter().position(|p| (*p - Vec3::ONE).length() < 1e-5).unwrap();
    assert_eq!(lim.positions[corner], c.positions[corner]);
}

#[test]
fn wave_moves_with_the_frame() {
    let g = primitives::grid(40, 40, 10.0);
    let md = defaults("WAVE");
    let at = |frame: f32| md.apply_with(&g, &EvalContext { frame, ..Default::default() });
    let a = at(1.0);
    let b = at(11.0);
    check(&a);
    check(&b);
    let max = |m: &Mesh| m.positions.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    assert!(max(&a) > 0.3 && max(&a) <= 0.5 + 1e-5, "{}", max(&a));
    assert!(a.positions.iter().all(|p| p.z >= -1e-5), "Gaussian lifts only");
    assert_ne!(a.positions, b.positions, "time moves the wave");
    assert!(md.is_animated());
    // Lifetime + damping: the wave dies out.
    let dead = Modifier::Wave { x: true, y: true, cyclic: true, normals: false, falloff: 0.0, height: 0.5, width: 1.5, narrowness: 1.5, speed: 0.25, time_offset: 0.0, lifetime: 5.0, damping: 2.0, start: [0.0; 2], vertex_group: None, invert_vertex_group: false }
        .apply_with(&g, &EvalContext { frame: 20.0, ..Default::default() });
    assert_eq!(dead, g);
    // Along normals on a sphere.
    let s = primitives::uv_sphere(16, 8, 1.0);
    let n = Modifier::Wave { x: true, y: false, cyclic: false, normals: true, falloff: 0.0, height: 0.2, width: 1.5, narrowness: 1.5, speed: 0.0, time_offset: 0.0, lifetime: 0.0, damping: 10.0, start: [0.0; 2], vertex_group: None, invert_vertex_group: false }.apply(&s);
    check(&n);
    assert!(n.positions.iter().all(|p| p.length() >= 1.0 - 1e-4 && p.length() <= 1.2 + 1e-4));
}

#[test]
fn shrinkwrap_onto_a_sphere() {
    let big = primitives::ico_sphere(2, 2.0);
    let target = primitives::uv_sphere(48, 24, 1.0);
    let lookup = |n: &str| (n == "Ball").then(|| target.clone());
    let cx = EvalContext { object: Some(&lookup), ..Default::default() };
    for method in [ShrinkwrapMethod::NearestSurfacePoint, ShrinkwrapMethod::NearestVertex] {
        let m = Modifier::Shrinkwrap { target: "Ball".into(), method, offset: 0.0, axes: [false; 3], positive: true, negative: false, vertex_group: None, invert_vertex_group: false }.apply_with(&big, &cx);
        check(&m);
        assert!(m.positions.iter().all(|p| (p.length() - 1.0).abs() < 0.01), "{method:?}");
    }
    // Project along the (outward) normals needs the negative direction.
    let none = Modifier::Shrinkwrap { target: "Ball".into(), method: ShrinkwrapMethod::Project, offset: 0.0, axes: [false; 3], positive: true, negative: false, vertex_group: None, invert_vertex_group: false }.apply_with(&big, &cx);
    assert_eq!(none, big);
    let p = Modifier::Shrinkwrap { target: "Ball".into(), method: ShrinkwrapMethod::Project, offset: 0.1, axes: [false; 3], positive: true, negative: true, vertex_group: None, invert_vertex_group: false }.apply_with(&big, &cx);
    check(&p);
    let worst = p.positions.iter().map(|q| (q.length() - 1.1).abs()).fold(0.0, f32::max);
    assert!(worst < 0.015, "{worst}");
    // Project along -Z flattens a plane above the sphere onto its top.
    let mut plane = primitives::grid(4, 4, 0.5);
    plane.translate(Vec3::new(0.0, 0.0, 3.0));
    let down = Modifier::Shrinkwrap { target: "Ball".into(), method: ShrinkwrapMethod::Project, offset: 0.0, axes: [false, false, true], positive: false, negative: true, vertex_group: None, invert_vertex_group: false }.apply_with(&plane, &cx);
    assert!(down.positions.iter().all(|q| (q.length() - 1.0).abs() < 0.01 && q.z > 0.9));
    assert_eq!(defaults("SHRINKWRAP").apply(&big), big, "no target");
}

fn noisy_sphere() -> Mesh {
    let mut s = primitives::ico_sphere(3, 1.0);
    for (i, p) in s.positions.iter_mut().enumerate() {
        let k = 1.0 + 0.05 * (((i * 7919) % 13) as f32 / 13.0 - 0.5);
        *p = *p * k;
    }
    s
}

fn roughness(m: &Mesh) -> f32 {
    let (start, adj) = adjacency(m);
    (0..m.vertex_count())
        .map(|v| {
            let nb = &adj[start[v]..start[v + 1]];
            let avg = nb.iter().fold(Vec3::ZERO, |a, &u| a + m.positions[u as usize]) / nb.len() as f32;
            (avg - m.positions[v]).length()
        })
        .sum::<f32>()
        / m.vertex_count() as f32
}

#[test]
fn laplacian_smooth_smooths_and_keeps_volume() {
    let s = noisy_sphere();
    let m = Modifier::LaplacianSmooth { iterations: 5, factor: 1.0, border: 0.01, axes: [true; 3], preserve_volume: true, normalized: true, vertex_group: None, invert_vertex_group: false }.apply(&s);
    check(&m);
    assert!(roughness(&m) < roughness(&s) * 0.8, "{} vs {}", roughness(&m), roughness(&s));
    assert!((m.volume() - s.volume()).abs() < 1e-3 * s.volume().abs().max(1.0) * 10.0, "{} vs {}", m.volume(), s.volume());
    // Only Z enabled: X and Y stay.
    let z = Modifier::LaplacianSmooth { iterations: 3, factor: 1.0, border: 0.01, axes: [false, false, true], preserve_volume: false, normalized: false, vertex_group: None, invert_vertex_group: false }.apply(&s);
    check(&z);
    assert!(z.positions.iter().zip(&s.positions).all(|(a, b)| a.x == b.x && a.y == b.y));
    // Defaults are gentle and keep an open grid's outline.
    let g = primitives::grid(6, 6, 2.0);
    let d = defaults("LAPLACIANSMOOTH").apply(&g);
    check(&d);
    assert!((d.bounds().size().x - 2.0).abs() < 1e-3);
}

#[test]
fn corrective_smooth_keeps_rest_detail() {
    let rest = subdivide_simple(&primitives::cube(2.0), 2);
    // A rigidly moved mesh comes back as itself.
    let mut moved = rest.clone();
    moved.translate(Vec3::new(3.0, 0.0, 0.0));
    let md = defaults("CORRECTIVE_SMOOTH");
    let out = md.apply_with(&moved, &EvalContext { rest: Some(&rest.positions), ..Default::default() });
    check(&out);
    let err = out.positions.iter().zip(&moved.positions).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
    assert!(err < 1e-3, "{err}");
    // Only Smooth just smooths.
    let only = Modifier::CorrectiveSmooth { factor: 0.5, iterations: 5, scale: 1.0, length_weighted: true, only_smooth: true, vertex_group: None, invert_vertex_group: false }.apply(&rest);
    check(&only);
    assert!(only.bounds().size().x < 2.0);
    // Through a stack, the rest shape is the stack's input.
    let st = apply_stack(&rest, &[Modifier::Displace { strength: 0.2, vertex_group: None, invert_vertex_group: false }, md]);
    check(&st);
}

#[test]
fn weighted_normal_marks_smooth() {
    let cube = primitives::cube(2.0);
    let m = defaults("WEIGHTED_NORMAL").apply(&cube);
    assert_eq!(check(&m), (8, 6));
    assert!(m.faces.iter().all(|f| f.smooth));
    let sharp = Modifier::WeightedNormal { keep_sharp: true }.apply(&cube);
    assert_eq!(check(&sharp), (24, 6));
}

// ---------------------------------------------------------------------------
// Names, scenes and polyops

#[test]
fn every_type_has_defaults_and_round_trips() {
    for ty in Modifier::TYPE_NAMES.split(", ") {
        let m = Modifier::with_defaults(ty).unwrap_or_else(|| panic!("{ty}"));
        assert_eq!(m.type_name(), ty);
        assert_eq!(Modifier::with_defaults(&m.type_name()), Some(m.clone()));
        assert_eq!(Modifier::with_defaults(m.name()), Some(m.clone()));
    }
    for (blender, ours) in [("SUBSURF", "Subdivision"), ("SIMPLE_DEFORM", "SimpleDeform"), ("EDGE_SPLIT", "EdgeSplit"), ("LAPLACIANSMOOTH", "LaplacianSmooth"), ("CORRECTIVE_SMOOTH", "CorrectiveSmooth"), ("WEIGHTED_NORMAL", "WeightedNormal")] {
        assert_eq!(Modifier::with_defaults(blender).unwrap().type_name(), ours);
    }
    assert!(Modifier::with_defaults("SKIN").is_none());

    // Every variant with non-default settings survives polyops save/load.
    let mut s = Scene::new();
    let o = s.add_mesh_object("Thing", primitives::cube(2.0));
    s.add_mesh_object("Other Thing", primitives::cube(1.0));
    s.objects[o].modifiers = vec![
        Modifier::Decimate { mode: DecimateMode::Planar, ratio: 0.25, iterations: 3, angle: 0.2, triangulate: true },
        Modifier::Remesh { mode: RemeshMode::Blocks, voxel_size: 0.05, octree_depth: 6, scale: 0.75, smooth_shade: true },
        Modifier::Screw { axis: 0, angle: 1.5, steps: 7, screw_offset: 0.5, iterations: 3, merge: true, merge_distance: 0.02, flip: true, smooth_shade: false },
        Modifier::Wireframe { thickness: 0.1, offset: 0.5, even: false, relative: true, boundary: true, replace: false },
        Modifier::EdgeSplit { angle: 0.3, use_angle: false },
        Modifier::Build { start: 5.0, length: 20.0, reverse: true, random: true, seed: 42 },
        Modifier::Boolean { object: "Other Thing".into(), operation: BooleanOp::Intersect },
        Modifier::SimpleDeform { mode: SimpleDeformMode::Bend, axis: 1, factor: 0.5, limits: [0.1, 0.9], lock: [true, false, true], vertex_group: None, invert_vertex_group: false },
        Modifier::Cast { kind: CastType::Cuboid, factor: -0.5, radius: 2.0, size: 1.0, axes: [true, false, true], size_from_radius: true, vertex_group: None, invert_vertex_group: false },
        Modifier::Wave { x: false, y: true, cyclic: false, normals: true, falloff: 2.0, height: 0.3, width: 1.0, narrowness: 2.0, speed: 0.5, time_offset: 3.0, lifetime: 10.0, damping: 5.0, start: [1.0, -2.0], vertex_group: None, invert_vertex_group: false },
        Modifier::Shrinkwrap { target: "Other Thing".into(), method: ShrinkwrapMethod::Project, offset: 0.1, axes: [false, true, false], positive: false, negative: true, vertex_group: None, invert_vertex_group: false },
        Modifier::LaplacianSmooth { iterations: 4, factor: 0.5, border: 0.2, axes: [true, true, false], preserve_volume: false, normalized: false, vertex_group: None, invert_vertex_group: false },
        Modifier::CorrectiveSmooth { factor: 0.3, iterations: 7, scale: 0.5, length_weighted: true, only_smooth: true, vertex_group: None, invert_vertex_group: false },
        Modifier::WeightedNormal { keep_sharp: true },
    ];
    let text = crate::script::scene_to_polyops(&s);
    let mut back = Scene::new();
    crate::script::run(&mut back, &text).unwrap();
    let bo = back.find_object("Thing").unwrap();
    assert_eq!(back.objects[bo].modifiers, s.objects[o].modifiers, "{text}");
}

#[test]
fn scene_resolves_operands_and_frame() {
    let mut s = Scene::new();
    let a = s.add_mesh_object("A", primitives::cube(2.0));
    let b = s.add_mesh_object("B", primitives::cube(2.0));
    s.objects[a].transform.location = Vec3::new(5.0, 0.0, 0.0);
    s.objects[b].transform.location = Vec3::new(6.0, 0.0, 0.0);
    s.objects[a].modifiers.push(Modifier::Boolean { object: "B".into(), operation: BooleanOp::Difference });
    let m = s.evaluated_mesh(a).unwrap();
    assert!((m.volume() - 4.0).abs() < 1e-3, "operand placed in A's space: {}", m.volume());
    assert!(m.bounds().max.x < 0.0 + 1e-4);
    // A cycle (B unions A back) still terminates.
    s.objects[b].modifiers.push(Modifier::Boolean { object: "A".into(), operation: BooleanOp::Union });
    check(&s.evaluated_mesh(a).unwrap());
    check(&s.evaluated_mesh(b).unwrap());
    // Wave follows the scene frame.
    let g = s.add_mesh_object("G", primitives::grid(20, 20, 6.0));
    s.objects[g].modifiers.push(defaults("WAVE"));
    let f1 = s.evaluated_mesh(g).unwrap();
    s.frame_current = 30;
    assert_ne!(s.evaluated_mesh(g).unwrap(), f1);
    // Rename keeps references.
    let mut md = s.objects[a].modifiers[0].clone();
    md.rename_object_ref("B", "C");
    assert_eq!(md.object_ref(), Some("C"));
}
