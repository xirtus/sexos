// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::math::Vec3;
use crate::primitives;
use crate::scene::{Image, Scene};
use crate::undo::UndoStack;
use std::collections::HashMap;

const UP: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };

/// 40x40 quads over a 2x2 square in XY (vertex spacing 0.05), facing +Z.
fn grid() -> Mesh {
    primitives::grid(40, 40, 2.0)
}

fn vert_at(m: &Mesh, x: f32, y: f32) -> usize {
    m.positions.iter().enumerate().min_by(|a, b| {
        let da = (a.1.x - x).powi(2) + (a.1.y - y).powi(2);
        let db = (b.1.x - x).powi(2) + (b.1.y - y).powi(2);
        da.total_cmp(&db)
    }).map(|(i, _)| i).unwrap_or(0)
}

/// One stroke of `n` identical dabs; returns the stroke's undo record.
fn stroke(m: &mut Mesh, kind: BrushKind, sym: Symmetry, d: Dab, n: usize) -> Vec<Change> {
    stroke_with(m, &Brush::new(kind), sym, d, n)
}

fn stroke_with(m: &mut Mesh, b: &Brush, sym: Symmetry, d: Dab, n: usize) -> Vec<Change> {
    let mut ss = SculptSession::new(m);
    let mut s = Stroke::new(m, &ss, b, sym, d.detail > 0.0);
    for _ in 0..n {
        s.dab(m, &mut ss, &d);
    }
    s.finish(m, 0)
}

fn dab0() -> Dab {
    Dab::at(Vec3::ZERO, 0.5, UP)
}

/// A smooth bump of height `h` at the origin.
fn bump(m: &mut Mesh, h: f32) {
    for p in &mut m.positions {
        let r = (p.x * p.x + p.y * p.y).sqrt();
        p.z = h * (1.0 - (r / 0.6).min(1.0)).powi(2);
    }
}

fn all_finite(m: &Mesh) -> bool {
    m.positions.iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
}

#[test]
fn additive_brushes_raise_the_surface_and_ctrl_lowers_it() {
    let c = |m: &Mesh| m.positions[vert_at(m, 0.0, 0.0)].z;
    for kind in [BrushKind::Draw, BrushKind::DrawSharp, BrushKind::Inflate, BrushKind::Blob, BrushKind::Crease, BrushKind::Layer, BrushKind::Clay, BrushKind::ClayStrips, BrushKind::ClayThumb] {
        let mut m = grid();
        let mut d = dab0();
        d.step = Vec3::new(0.01, 0.0, 0.0);
        stroke(&mut m, kind, Symmetry::default(), d, 3);
        assert!(all_finite(&m), "{kind:?}");
        assert!(c(&m) > 1e-4, "{kind:?} should raise the centre, got {}", c(&m));
        // Outside the radius nothing moves.
        assert_eq!(m.positions[vert_at(&m, 0.9, 0.9)].z, 0.0, "{kind:?}");
        let mut m = grid();
        d.invert = true;
        stroke(&mut m, kind, Symmetry::default(), d, 3);
        assert!(c(&m) < -1e-4, "{kind:?} inverted should lower the centre, got {}", c(&m));
    }
}

#[test]
fn draw_moves_by_strength_times_radius() {
    let mut m = grid();
    let mut b = Brush::new(BrushKind::Draw);
    b.strength = 1.0;
    stroke_with(&mut m, &b, Symmetry::default(), dab0(), 1);
    let z = m.positions[vert_at(&m, 0.0, 0.0)].z;
    assert!((z - brushes::DRAW_SCALE * 0.5).abs() < 0.01, "z = {z}");
    // Falloff: half way out moves less than the centre, and still up.
    let z2 = m.positions[vert_at(&m, 0.25, 0.0)].z;
    assert!(z2 > 0.0 && z2 < z);
}

#[test]
fn layer_stops_at_its_height() {
    let mut m = grid();
    let mut b = Brush::new(BrushKind::Layer);
    b.strength = 1.0;
    stroke_with(&mut m, &b, Symmetry::default(), dab0(), 40);
    let z = m.positions[vert_at(&m, 0.0, 0.0)].z;
    assert!(z <= b.height + 1e-4 && z > b.height * 0.5, "layer z {z}");
}

#[test]
fn flattening_brushes_lower_a_bump_and_fill_raises_a_dent() {
    for kind in [BrushKind::Smooth, BrushKind::Flatten, BrushKind::Scrape, BrushKind::MultiplaneScrape] {
        let mut m = grid();
        bump(&mut m, 0.2);
        let before = m.positions[vert_at(&m, 0.0, 0.0)].z;
        let mut d = dab0();
        d.step = Vec3::new(0.01, 0.0, 0.0);
        stroke(&mut m, kind, Symmetry::default(), d, 4);
        let after = m.positions[vert_at(&m, 0.0, 0.0)].z;
        assert!(all_finite(&m));
        assert!(after < before - 1e-3, "{kind:?}: {before} -> {after}");
    }
    let mut m = grid();
    bump(&mut m, -0.2);
    let before = m.positions[vert_at(&m, 0.0, 0.0)].z;
    stroke(&mut m, BrushKind::Fill, Symmetry::default(), dab0(), 4);
    let after = m.positions[vert_at(&m, 0.0, 0.0)].z;
    assert!(after > before + 1e-3, "fill: {before} -> {after}");
}

#[test]
fn pinch_pulls_towards_the_centre() {
    let mut m = grid();
    let v = vert_at(&m, 0.2, 0.0);
    let x0 = m.positions[v].x;
    stroke(&mut m, BrushKind::Pinch, Symmetry::default(), dab0(), 2);
    assert!(m.positions[v].x < x0 - 1e-4 && m.positions[v].x > 0.0);
    let mut m = grid();
    let mut d = dab0();
    d.invert = true;
    stroke(&mut m, BrushKind::Pinch, Symmetry::default(), d, 2);
    assert!(m.positions[v].x > x0 + 1e-4);
}

#[test]
fn grab_like_brushes_follow_the_pointer() {
    let g = Vec3::new(0.1, 0.0, 0.0);
    for kind in [BrushKind::Grab, BrushKind::ElasticDeform, BrushKind::Thumb, BrushKind::Pose] {
        let mut m = grid();
        let v = vert_at(&m, 0.0, 0.0);
        let mut d = dab0();
        d.grab = g;
        stroke(&mut m, kind, Symmetry::default(), d, 2);
        let dx = m.positions[v].x;
        assert!(all_finite(&m));
        assert!(dx > 0.02 && dx < 0.15, "{kind:?} moved centre by {dx}");
    }
    // Grab at full strength moves the centre exactly with the pointer, and
    // dabs do not accumulate (they restart from the original positions).
    let mut m = grid();
    let v = vert_at(&m, 0.0, 0.0);
    let mut d = dab0();
    d.grab = g;
    stroke(&mut m, BrushKind::Grab, Symmetry::default(), d, 5);
    assert!((m.positions[v].x - 0.1).abs() < 1e-3, "{}", m.positions[v].x);
}

#[test]
fn step_brushes_move_along_the_stroke() {
    for kind in [BrushKind::SnakeHook, BrushKind::Nudge, BrushKind::SlideRelax] {
        let mut m = grid();
        bump(&mut m, 0.1);
        let v = vert_at(&m, 0.05, 0.05);
        let p0 = m.positions[v];
        let mut d = dab0();
        d.step = Vec3::new(0.05, 0.0, 0.0);
        stroke(&mut m, kind, Symmetry::default(), d, 1);
        assert!(all_finite(&m));
        assert!(m.positions[v].x > p0.x + 1e-4, "{kind:?}: {:?} -> {:?}", p0, m.positions[v]);
    }
}

#[test]
fn rotate_turns_around_the_normal() {
    let mut m = grid();
    let v = vert_at(&m, 0.1, 0.0);
    let mut d = dab0();
    d.angle = 0.5;
    stroke(&mut m, BrushKind::Rotate, Symmetry::default(), d, 1);
    let p = m.positions[v];
    assert!(p.y.abs() > 1e-3, "{p:?}");
    assert!(((p.x * p.x + p.y * p.y).sqrt() - 0.1).abs() < 1e-3, "rotation keeps the distance: {p:?}");
    assert!(p.z.abs() < 1e-5);
}

#[test]
fn every_brush_runs_and_stays_finite() {
    let mut sphere = primitives::uv_sphere(32, 16, 1.0);
    for kind in BrushKind::ALL {
        let mut m = sphere.clone();
        let mut d = Dab::at(Vec3::new(0.0, 0.0, 1.0), 0.4, UP);
        d.step = Vec3::new(0.02, 0.0, 0.0);
        d.grab = Vec3::new(0.05, 0.0, 0.0);
        d.angle = 0.2;
        stroke(&mut m, kind, Symmetry { mirror: [true, false, false], lock: [false; 3] }, d, 3);
        assert!(all_finite(&m), "{kind:?}");
        assert!(m.mask.iter().all(|x| (0.0..=1.0).contains(x)), "{kind:?}");
    }
    sphere.positions.clear();
}

#[test]
fn mask_and_face_set_brushes_paint_attributes() {
    let mut m = grid();
    let ch = stroke(&mut m, BrushKind::Mask, Symmetry::default(), dab0(), 3);
    let c = vert_at(&m, 0.0, 0.0);
    assert!(m.mask_at(c) > 0.5);
    assert_eq!(m.mask_at(vert_at(&m, 0.9, 0.9)), 0.0);
    assert!(m.positions.iter().all(|p| p.z == 0.0));
    assert!(matches!(ch.as_slice(), [Change::Mask { .. }] | [Change::MaskAll { .. }]));
    let mut d = dab0();
    d.invert = true;
    stroke(&mut m, BrushKind::Mask, Symmetry::default(), d, 3);
    assert!(m.mask_at(c) < 0.5);

    let mut m = grid();
    stroke(&mut m, BrushKind::DrawFaceSets, Symmetry::default(), dab0(), 1);
    let sets: std::collections::HashSet<i32> = (0..m.faces.len()).map(|f| m.face_set(f)).collect();
    assert_eq!(sets.len(), 2, "{sets:?}");
}

#[test]
fn symmetry_mirrors_strokes() {
    let mut m = grid();
    let d = Dab::at(Vec3::new(0.4, 0.0, 0.0), 0.2, UP);
    stroke(&mut m, BrushKind::Draw, Symmetry { mirror: [true, false, false], lock: [false; 3] }, d, 2);
    let a = m.positions[vert_at(&m, 0.4, 0.0)].z;
    let b = m.positions[vert_at(&m, -0.4, 0.0)].z;
    assert!(a > 1e-4);
    assert!((a - b).abs() < 1e-5, "{a} vs {b}");
    assert_eq!(m.positions[vert_at(&m, 0.0, 0.4)].z, 0.0);
    // Without symmetry the other side stays put.
    let mut m = grid();
    stroke(&mut m, BrushKind::Draw, Symmetry::default(), d, 2);
    assert_eq!(m.positions[vert_at(&m, -0.4, 0.0)].z, 0.0);
    // Mirroring a grab mirrors its direction too.
    let mut m = grid();
    let mut d = Dab::at(Vec3::new(0.4, 0.0, 0.0), 0.2, UP);
    d.grab = Vec3::new(0.05, 0.0, 0.0);
    stroke(&mut m, BrushKind::Grab, Symmetry { mirror: [true, false, false], lock: [false; 3] }, d, 1);
    let ra = m.positions[vert_at(&m, 0.45, 0.0)];
    assert!(m.positions.iter().any(|p| (p.x + ra.x).abs() < 1e-5 && p.y.abs() < 1e-5), "mirror image of {ra:?}");
}

#[test]
fn lock_keeps_an_axis_fixed() {
    let mut m = grid();
    stroke(&mut m, BrushKind::Draw, Symmetry { mirror: [false; 3], lock: [false, false, true] }, dab0(), 2);
    assert!(m.positions.iter().all(|p| p.z == 0.0));
}

#[test]
fn mask_blocks_deformation() {
    let mut m = grid();
    m.mask = m.positions.iter().map(|p| if p.x < 0.0 { 1.0 } else { 0.0 }).collect();
    stroke(&mut m, BrushKind::Draw, Symmetry::default(), dab0(), 3);
    assert_eq!(m.positions[vert_at(&m, -0.1, 0.0)].z, 0.0);
    assert!(m.positions[vert_at(&m, 0.1, 0.0)].z > 1e-4);
    // Half a mask, half the movement.
    let mut a = grid();
    let mut b = grid();
    b.mask = vec![0.5; b.positions.len()];
    stroke(&mut a, BrushKind::Draw, Symmetry::default(), dab0(), 1);
    stroke(&mut b, BrushKind::Draw, Symmetry::default(), dab0(), 1);
    let c = vert_at(&a, 0.0, 0.0);
    assert!((b.positions[c].z - a.positions[c].z * 0.5).abs() < 1e-5);
}

#[test]
fn hidden_faces_are_not_sculpted() {
    let mut m = grid();
    m.hidden_faces = m.faces.iter().map(|f| f.verts.iter().all(|&v| m.positions[v as usize].x < -0.01)).collect();
    stroke(&mut m, BrushKind::Draw, Symmetry::default(), dab0(), 2);
    assert_eq!(m.positions[vert_at(&m, -0.2, 0.0)].z, 0.0);
    assert!(m.positions[vert_at(&m, 0.2, 0.0)].z > 0.0);
}

#[test]
fn topology_automask_keeps_other_parts() {
    let mut m = grid();
    let mut other = grid();
    for p in &mut other.positions {
        p.z = 0.1;
    }
    m.append(&other);
    let mut b = Brush::new(BrushKind::Draw);
    b.automask_topology = true;
    stroke_with(&mut m, &b, Symmetry::default(), dab0(), 1);
    let n = m.positions.len() / 2;
    assert!(m.positions[..n].iter().any(|p| p.z > 0.0));
    assert!(m.positions[n..].iter().all(|p| p.z == 0.1));
}

/// Every edge has one or two faces, interior edges exactly two, and faces
/// are triangles with distinct vertices that all exist.
fn check_manifold(m: &Mesh) {
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        assert_eq!(f.verts.len(), 3);
        let v = &f.verts;
        assert!(v[0] != v[1] && v[1] != v[2] && v[0] != v[2], "degenerate {v:?}");
        for &x in v {
            assert!((x as usize) < m.positions.len());
        }
        for (a, b) in f.edges() {
            *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    assert!(edges.values().all(|&n| n <= 2), "non-manifold edge");
    // The open border of a grid is a single loop: every boundary vertex
    // has exactly two boundary edges.
    let mut deg: HashMap<u32, u32> = HashMap::new();
    for (e, n) in &edges {
        if *n == 1 {
            *deg.entry(e.0).or_insert(0) += 1;
            *deg.entry(e.1).or_insert(0) += 1;
        }
    }
    assert!(deg.values().all(|&d| d == 2), "boundary is not a simple loop");
    // Every vertex is used.
    let mut used = vec![false; m.positions.len()];
    for f in &m.faces {
        for &v in &f.verts {
            used[v as usize] = true;
        }
    }
    assert!(used.iter().all(|u| *u));
}

#[test]
fn dyntopo_subdivides_to_the_detail_size_and_stays_manifold() {
    let mut m = primitives::grid(8, 8, 2.0);
    m.mask = vec![0.0; m.positions.len()];
    m.face_sets = vec![3; m.faces.len()];
    let nf = m.faces.len();
    let mut d = Dab::at(Vec3::ZERO, 0.5, UP);
    d.detail = 0.1;
    d.pressure = 0.2;
    stroke(&mut m, BrushKind::Draw, Symmetry::default(), d, 6);
    check_manifold(&m);
    assert!(m.faces.len() > nf * 2 + 20, "{} faces", m.faces.len());
    assert_eq!(m.mask.len(), m.positions.len());
    assert_eq!(m.face_sets.len(), m.faces.len());
    assert!(m.face_sets.iter().all(|&s| s == 3));
    // Edges well inside the brush are no longer than the detail size.
    let mut longest: f32 = 0.0;
    for f in &m.faces {
        for (a, b) in f.edges() {
            let (pa, pb) = (m.positions[a as usize], m.positions[b as usize]);
            if ((pa + pb) * 0.5).length() < 0.3 {
                longest = longest.max(pa.distance(pb));
            }
        }
    }
    assert!(longest <= 0.1 + 1e-4, "longest edge {longest}");
    // Far away the original resolution is kept.
    let corner = vert_at(&m, 1.0, 1.0);
    assert!(m.positions[corner].distance(Vec3::new(1.0, 1.0, 0.0)) < 1e-6);
}

#[test]
fn dyntopo_collapses_short_edges() {
    let mut m = primitives::grid(60, 60, 2.0);
    let n0 = m.positions.len();
    let mut d = Dab::at(Vec3::ZERO, 0.5, UP);
    d.detail = 0.2;
    d.pressure = 0.0;
    stroke(&mut m, BrushKind::Simplify, Symmetry::default(), d, 6);
    check_manifold(&m);
    assert!(m.positions.len() < n0 - 50, "{} -> {}", n0, m.positions.len());
    let mut shortest = f32::MAX;
    for f in &m.faces {
        for (a, b) in f.edges() {
            let (pa, pb) = (m.positions[a as usize], m.positions[b as usize]);
            if ((pa + pb) * 0.5).length() < 0.25 {
                shortest = shortest.min(pa.distance(pb));
            }
        }
    }
    assert!(shortest >= 0.2 * dyntopo::MIN_FACTOR * 0.5, "shortest edge {shortest}");
}

#[test]
fn dyntopo_on_a_closed_sphere_stays_closed() {
    let mut m = primitives::ico_sphere(2, 1.0);
    let mut d = Dab::at(Vec3::new(0.0, 0.0, 1.0), 0.6, UP);
    d.detail = 0.08;
    stroke(&mut m, BrushKind::Draw, Symmetry { mirror: [true, false, false], lock: [false; 3] }, d, 4);
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    assert!(edges.values().all(|&n| n == 2), "closed mesh got a hole or fin");
    // Euler characteristic of a sphere.
    let chi = m.positions.len() as i64 - edges.len() as i64 + m.faces.len() as i64;
    assert_eq!(chi, 2);
}

fn scene_with(m: Mesh) -> Scene {
    let mut s = Scene::new();
    s.meshes.push(m);
    s
}

#[test]
fn undo_restores_strokes_exactly() {
    let base = grid();
    let mut scene = scene_with(base.clone());
    let mut undo = UndoStack::new();
    let mut d = dab0();
    d.step = Vec3::new(0.01, 0.0, 0.0);
    for kind in [BrushKind::Draw, BrushKind::Mask, BrushKind::DrawFaceSets, BrushKind::Smooth] {
        let ch = stroke(&mut scene.meshes[0], kind, Symmetry::default(), d, 2);
        assert!(!ch.is_empty(), "{kind:?}");
        undo.push_changes(kind.label(), ch);
    }
    let sculpted = scene.meshes[0].clone();
    while undo.undo(&mut scene).is_some() {}
    assert_eq!(scene.meshes[0].positions, base.positions);
    assert!(scene.meshes[0].mask.iter().all(|&x| x == 0.0));
    assert!((0..base.faces.len()).all(|f| scene.meshes[0].face_set(f) == 1));
    while undo.redo(&mut scene).is_some() {}
    assert_eq!(scene.meshes[0].positions, sculpted.positions);
    assert_eq!(scene.meshes[0].mask, sculpted.mask);
    assert_eq!(scene.meshes[0].face_sets, sculpted.face_sets);
    // Patches are far smaller than snapshots.
    assert!(undo.patch_bytes() < base.positions.len() * 12 * 4);

    // A dyntopo stroke restores the whole mesh.
    let mut scene = scene_with(primitives::grid(8, 8, 2.0));
    let before = scene.meshes[0].clone();
    let mut d = dab0();
    d.detail = 0.1;
    let ch = stroke(&mut scene.meshes[0], BrushKind::Draw, Symmetry::default(), d, 2);
    let after = scene.meshes[0].clone();
    undo.push_changes("Dyntopo", ch);
    undo.undo(&mut scene);
    assert_eq!(scene.meshes[0].positions, before.positions);
    assert_eq!(scene.meshes[0].faces.len(), before.faces.len());
    undo.redo(&mut scene);
    assert_eq!(scene.meshes[0].positions, after.positions);
}

#[test]
fn mask_ops_and_face_set_visibility() {
    let mut m = grid();
    ops::mask_flood(&mut m, ops::MaskOp::Fill(1.0));
    assert!(m.mask.iter().all(|&x| x == 1.0));
    ops::mask_flood(&mut m, ops::MaskOp::Invert);
    assert!(m.mask.iter().all(|&x| x == 0.0));
    let c0 = vert_at(&m, 0.0, 0.0);
    m.mask[c0] = 1.0;
    let ss = SculptSession::new(&m);
    ops::mask_grow(&mut m, &ss, true);
    assert!(m.mask.iter().filter(|&&x| x > 0.0).count() >= 5);

    let mut a = grid();
    let b = {
        let mut b = grid();
        for p in &mut b.positions {
            p.z = 1.0;
        }
        b
    };
    a.append(&b);
    ops::face_sets_init(&mut a, ops::FaceSetInit::LooseParts);
    let n = a.faces.len();
    assert_ne!(a.face_set(0), a.face_set(n - 1));
    let s0 = a.face_set(0);
    ops::face_set_visibility(&mut a, ops::Visibility::Isolate(s0));
    assert!(!a.is_face_hidden(0) && a.is_face_hidden(n - 1));
    ops::face_set_visibility(&mut a, ops::Visibility::ShowAll);
    assert!((0..n).all(|f| !a.is_face_hidden(f)));
}

#[test]
fn mesh_filters_move_vertices() {
    for (kind, check) in [
        (ops::FilterKind::Inflate, 0usize),
        (ops::FilterKind::Scale, 1),
        (ops::FilterKind::Smooth, 2),
        (ops::FilterKind::Random, 3),
        (ops::FilterKind::Sphere, 3),
        (ops::FilterKind::Sharpen, 3),
        (ops::FilterKind::EnhanceDetails, 3),
    ] {
        let mut m = primitives::uv_sphere(24, 12, 1.0);
        let mut ss = SculptSession::new(&m);
        let before = m.positions.clone();
        let changed = ops::mesh_filter(&mut m, &mut ss, kind, 0.5, [true; 3], 7);
        assert!(all_finite(&m), "{kind:?}");
        let r0: f32 = before.iter().map(|p| p.length()).sum::<f32>() / before.len() as f32;
        let r1: f32 = m.positions.iter().map(|p| p.length()).sum::<f32>() / before.len() as f32;
        match check {
            0 | 1 => assert!(r1 > r0 + 1e-3, "{kind:?}: {r0} -> {r1}"),
            2 => assert!(r1 < r0 - 1e-4, "{kind:?}: {r0} -> {r1}"),
            _ => {}
        }
        if kind != ops::FilterKind::Sphere {
            assert!(!changed.is_empty() && m.positions != before, "{kind:?}");
        }
    }
}

#[test]
fn voxel_remesh_keeps_the_shape() {
    let mut m = primitives::uv_sphere(24, 12, 1.0);
    m.mask = vec![1.0; m.positions.len()];
    let r = ops::voxel_remesh(&m, 0.1, false);
    assert!(r.faces.len() > 100);
    assert_eq!(r.mask.len(), r.positions.len());
    for p in &r.positions {
        assert!((p.length() - 1.0).abs() < 0.12, "{p:?}");
    }
}

#[test]
fn raycast_hits_the_surface() {
    let m = primitives::uv_sphere(32, 16, 1.0);
    let ss = SculptSession::new(&m);
    let h = ss.raycast(&m, Vec3::new(0.1, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0)).expect("hit");
    assert!((h.pos.z - (1.0f32 - 0.01).sqrt()).abs() < 0.02, "{:?}", h.pos);
    assert!(h.normal.z > 0.9);
    assert!(ss.raycast(&m, Vec3::new(3.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0)).is_none());
}

/// A stroke on a mesh of 100k+ vertices must stay interactive. The budget
/// is generous (it is a smoke test); the real figure is a few ms per dab.
#[test]
fn stroke_on_a_large_mesh_is_fast() {
    let mut m = primitives::grid(330, 330, 2.0);
    assert!(m.positions.len() > 100_000);
    let t = std::time::Instant::now();
    let mut ss = SculptSession::new(&m);
    let build = t.elapsed();
    let b = Brush::new(BrushKind::Draw);
    let mut s = Stroke::new(&m, &ss, &b, Symmetry { mirror: [true, false, false], lock: [false; 3] }, false);
    let t = std::time::Instant::now();
    for i in 0..60 {
        let x = -0.5 + i as f32 / 60.0;
        let mut d = Dab::at(Vec3::new(x, 0.1, 0.0), 0.15, UP);
        d.step = Vec3::new(1.0 / 60.0, 0.0, 0.0);
        s.dab(&mut m, &mut ss, &d);
    }
    let strokes = t.elapsed();
    let ch = s.finish(&m, 0);
    assert!(!ch.is_empty());
    let budget = if cfg!(debug_assertions) { 60.0 } else { 3.0 };
    assert!(build.as_secs_f32() < budget, "session build {build:?}");
    assert!(strokes.as_secs_f32() < budget, "60 dabs took {strokes:?}");
}

#[test]
fn image_change_round_trips() {
    let mut s = Scene::new();
    s.images.push(Image::new("x", 4, 4));
    let mut u = UndoStack::new();
    let before = s.images[0].pixels.clone();
    u.push_changes("Paint", vec![Change::Image { image: 0, x: 0, y: 0, w: 4, h: 4, pixels: before.clone() }]);
    s.images[0].pixels[5] = [1.0, 0.0, 0.0, 1.0];
    let after = s.images[0].pixels.clone();
    u.undo(&mut s);
    assert_eq!(s.images[0].pixels, before);
    u.redo(&mut s);
    assert_eq!(s.images[0].pixels, after);
}
