// SPDX-License-Identifier: GPL-3.0-only
//! Rigging: vertex groups, shape keys, armatures (rest matrices, pose
//! inheritance, skinning, automatic weights), constraints and IK, lattices,
//! hooks, warps and the vertex weight modifiers.
use sex_3d::armature::{mat3_vec_to_roll, vec_roll_to_mat3, Armature, Bone, InheritScale, PoseBone, RotationMode};
use sex_3d::constraints::{Constraint, ConstraintKind, Target, TrackAxis};
use sex_3d::lattice::Lattice;
use sex_3d::math::{EulerOrder, Mat3, Mat4, Quat, Vec3};
use sex_3d::mesh::{KeyInterpolation, Mesh};
use sex_3d::modifiers::{Falloff, Modifier, ProximityMode, WeightMixMode, WeightMixSet};
use sex_3d::scene::{ArmatureWeights, Object, ObjectData, Scene};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn near(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a - b).length() <= eps
}

#[track_caller]
fn assert_near(a: Vec3, b: Vec3, eps: f32) {
    assert!(near(a, b, eps), "{a:?} != {b:?} (eps {eps})");
}

#[track_caller]
fn assert_close(a: f32, b: f32, eps: f32) {
    assert!((a - b).abs() <= eps, "{a} != {b} (eps {eps})");
}

/// A closed tube along +Z from 0 to `len`: `rings` rings of `seg` vertices
/// and n-gon caps.
fn tube(rings: u32, seg: u32, radius: f32, len: f32) -> Mesh {
    let mut m = Mesh::new();
    for r in 0..rings {
        let z = len * r as f32 / (rings - 1) as f32;
        for s in 0..seg {
            let a = 2.0 * PI * s as f32 / seg as f32;
            m.positions.push(v(radius * a.cos(), radius * a.sin(), z));
        }
    }
    for r in 0..rings - 1 {
        for s in 0..seg {
            let (a, b) = (r * seg + s, r * seg + (s + 1) % seg);
            m.add_face(&[a, b, b + seg, a + seg]);
        }
    }
    let bottom: Vec<u32> = (0..seg).rev().collect();
    let top: Vec<u32> = (0..seg).map(|s| (rings - 1) * seg + s).collect();
    m.add_face(&bottom);
    m.add_face(&top);
    m
}

fn volume(m: &Mesh) -> f32 {
    m.triangulate()
        .iter()
        .map(|t| {
            let [a, b, c] = t.verts.map(|i| m.positions[i as usize]);
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

fn two_bone_arm() -> Armature {
    let mut a = Armature::default();
    a.add_bone(Bone::new("Lower", v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0)));
    let mut up = Bone::new("Upper", v(0.0, 0.0, 1.0), v(0.0, 0.0, 2.0));
    up.parent = Some(0);
    up.use_connect = true;
    a.add_bone(up);
    a
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A rigged tube: armature object "Rig", mesh object "Tube" with an
/// Armature modifier and weights blending across the joint.
fn rigged_tube(dqs: bool) -> (Scene, usize, usize) {
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", two_bone_arm());
    let mut m = tube(41, 24, 0.25, 2.0);
    let (lo, up) = (m.add_vertex_group("Lower"), m.add_vertex_group("Upper"));
    for i in 0..m.positions.len() {
        let t = smoothstep(0.7, 1.3, m.positions[i].z);
        m.set_weight(i, lo, 1.0 - t);
        m.set_weight(i, up, t);
    }
    let t = s.add_mesh_object("Tube", m);
    let mut md = Modifier::with_defaults("Armature").unwrap();
    if let Modifier::Armature { object, use_deform_preserve_volume, .. } = &mut md {
        *object = "Rig".into();
        *use_deform_preserve_volume = dqs;
    }
    s.objects[t].modifiers.push(md);
    (s, rig, t)
}

fn set_pose_rot(s: &mut Scene, obj: usize, bone: &str, q: Quat) {
    let mut pb = PoseBone::new(bone);
    pb.rotation_quaternion = q;
    s.objects[obj].pose.retain(|p| p.name != bone);
    s.objects[obj].pose.push(pb);
}

/// World position of a point at `y` along bone `b` (0 head, length tail).
fn bone_point(s: &Scene, obj: usize, b: usize, y: f32) -> Vec3 {
    (s.world_matrix(obj) * s.pose_matrices(obj)[b]).transform_point(v(0.0, y, 0.0))
}

// ---------------------------------------------------------------------------
// Rest matrices

#[test]
fn rest_matrices_match_blender() {
    // A bone added in Blender points up +Z: matrix_local rows are
    // (1,0,0), (0,0,-1), (0,1,0).
    let m = vec_roll_to_mat3(v(0.0, 0.0, 1.0), 0.0);
    let want = Mat3::from_cols(v(1.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(0.0, -1.0, 0.0));
    assert!(m.approx_eq(&want, 1e-6), "{m:?}");
    // Along +Y the rest matrix is the identity; along -Y Blender flips X and Y.
    assert!(vec_roll_to_mat3(v(0.0, 1.0, 0.0), 0.0).approx_eq(&Mat3::IDENTITY, 1e-6));
    assert!(vec_roll_to_mat3(v(0.0, -1.0, 0.0), 0.0).approx_eq(&Mat3::diagonal(v(-1.0, -1.0, 1.0)), 1e-6));
    // Along +X: X axis -Y, Z stays Z.
    let m = vec_roll_to_mat3(v(1.0, 0.0, 0.0), 0.0);
    assert!(m.approx_eq(&Mat3::from_cols(v(0.0, -1.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 0.0, 1.0)), 1e-6), "{m:?}");
    // Diagonal bone, from `vec_roll_to_mat3_normalized`'s closed form.
    let m = vec_roll_to_mat3(v(1.0, 1.0, 1.0), 0.0);
    let (a, c, d) = (0.788_675_1, 0.577_350_3, 0.211_324_87);
    assert!(m.approx_eq(&Mat3::from_cols(v(a, -c, -d), v(c, c, c), v(-d, -c, a)), 1e-5), "{m:?}");
    // Roll turns X about the bone axis: +Z bone rolled 90 degrees has X = +Y.
    let m = vec_roll_to_mat3(v(0.0, 0.0, 1.0), FRAC_PI_2);
    assert_near(m.mul_vec(Vec3::X), v(0.0, 1.0, 0.0), 1e-6);
    // Nearly -Y (inside Blender's critical threshold) stays orthonormal.
    let m = vec_roll_to_mat3(v(1e-4, -1.0, 0.0), 0.3);
    assert!((m.determinant() - 1.0).abs() < 1e-4);
    assert_near(m.mul_vec(Vec3::Y), v(1e-4, -1.0, 0.0).normalize(), 1e-4);
    // Bone rest matrices sit at the head.
    let b = Bone::new("B", v(1.0, 2.0, 3.0), v(1.0, 2.0, 5.0));
    assert_near(b.rest_matrix().translation_part(), v(1.0, 2.0, 3.0), 1e-6);
}

#[test]
fn roll_round_trips() {
    let dirs = [v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0), v(0.3, -0.8, 0.2), v(-1.0, 2.0, 0.5), v(0.0, -1.0, 0.001), v(0.2, 0.1, -3.0)];
    for d in dirs {
        for roll in [-3.0f32, -1.2, 0.0, 0.4, 1.5, 3.1] {
            let m = vec_roll_to_mat3(d, roll);
            let back = mat3_vec_to_roll(&m, d);
            let diff = (back - roll + PI).rem_euclid(2.0 * PI) - PI;
            assert!(diff.abs() < 1e-3, "dir {d:?} roll {roll} -> {back}");
        }
    }
}

#[test]
fn bone_local_data_matches_blender_layout() {
    let a = two_bone_arm();
    // The connected child's head sits on the parent's tail: local head 0.
    let l = a.bone_local(1);
    assert_near(l.head, Vec3::ZERO, 1e-6);
    assert_near(l.tail, v(0.0, 1.0, 0.0), 1e-6);
    assert!(l.bone_mat.approx_eq(&Mat3::IDENTITY, 1e-6));
}

// ---------------------------------------------------------------------------
// Pose

#[test]
fn pose_inheritance() {
    let mut a = two_bone_arm();
    let mut free = Bone::new("Free", v(0.0, 0.0, 2.0), v(0.0, 0.0, 3.0));
    free.parent = Some(1);
    free.use_inherit_rotation = false;
    a.add_bone(free);
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", a);
    // Rest pose: pose matrices equal rest matrices.
    let rest = s.armature_of(rig).unwrap().rest_matrices();
    for (p, r) in s.pose_matrices(rig).iter().zip(&rest) {
        assert!(p.approx_eq(r, 1e-6));
    }
    // Rotating Lower 90 degrees about its X (world X) swings the chain to -Y.
    set_pose_rot(&mut s, rig, "Lower", Quat::from_axis_angle(Vec3::X, FRAC_PI_2));
    // A connected bone ignores its pose location.
    let mut up = PoseBone::new("Upper");
    up.location = v(5.0, 5.0, 5.0);
    s.objects[rig].pose.push(up);
    assert_near(bone_point(&s, rig, 1, 0.0), v(0.0, -1.0, 0.0), 1e-5);
    assert_near(bone_point(&s, rig, 1, 1.0), v(0.0, -2.0, 0.0), 1e-5);
    // "Free" follows its parent's position but keeps its rest orientation.
    assert_near(bone_point(&s, rig, 2, 0.0), v(0.0, -2.0, 0.0), 1e-5);
    assert_near(bone_point(&s, rig, 2, 1.0), v(0.0, -2.0, 1.0), 1e-5);
    // The armature object's transform applies on top.
    s.objects[rig].transform.location = v(10.0, 0.0, 0.0);
    assert_near(bone_point(&s, rig, 1, 1.0), v(10.0, -2.0, 0.0), 1e-5);
}

#[test]
fn inherit_scale_modes() {
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", two_bone_arm());
    let mut lo = PoseBone::new("Lower");
    lo.scale = Vec3::splat(2.0);
    s.objects[rig].pose.push(lo);
    // Full: the child is scaled too.
    assert_near(bone_point(&s, rig, 1, 0.0), v(0.0, 0.0, 2.0), 1e-5);
    assert_near(bone_point(&s, rig, 1, 1.0), v(0.0, 0.0, 4.0), 1e-5);
    // None: the child keeps its size (its head still moves with the parent).
    let ObjectData::Armature(ai) = s.objects[rig].data else { unreachable!() };
    s.armatures[ai].bones[1].inherit_scale = InheritScale::None;
    assert_near(bone_point(&s, rig, 1, 0.0), v(0.0, 0.0, 2.0), 1e-5);
    assert_near(bone_point(&s, rig, 1, 1.0), v(0.0, 0.0, 3.0), 1e-5);
}

#[test]
fn rotation_modes_agree() {
    let mut pb = PoseBone::new("B");
    let q = Quat::from_axis_angle(v(1.0, 2.0, 3.0).normalize(), 0.7);
    pb.rotation_quaternion = q;
    let mq = pb.rotation_mat3();
    for order in EulerOrder::ALL {
        let mut e = pb.clone();
        e.rotation_mode = RotationMode::Euler(order);
        e.set_rotation_mat3(&mq);
        assert!(e.rotation_mat3().approx_eq(&mq, 1e-5), "{order:?}");
    }
    let mut aa = pb.clone();
    aa.rotation_mode = RotationMode::AxisAngle;
    aa.set_rotation_mat3(&mq);
    assert_close(aa.rotation_axis_angle[0], 0.7, 1e-5);
    assert!(aa.rotation_mat3().approx_eq(&mq, 1e-5));
}

#[test]
fn bone_parenting_follows_the_posed_bone() {
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", two_bone_arm());
    let e = s.add_object(Object::new("Prop", ObjectData::Empty));
    assert!(s.parent_to_bone(e, rig, "Upper", false));
    // At rest the child sits on Upper's tail.
    assert_near(s.world_matrix(e).translation_part(), v(0.0, 0.0, 2.0), 1e-5);
    set_pose_rot(&mut s, rig, "Lower", Quat::from_axis_angle(Vec3::X, FRAC_PI_2));
    assert_near(s.world_matrix(e).translation_part(), v(0.0, -2.0, 0.0), 1e-5);
    // Keep transform: parenting does not move the object.
    let f = s.add_object(Object::new("Other", ObjectData::Empty));
    s.objects[f].transform.location = v(3.0, 1.0, 0.0);
    assert!(s.parent_to_bone(f, rig, "Lower", true));
    assert_near(s.world_matrix(f).translation_part(), v(3.0, 1.0, 0.0), 1e-5);
}

// ---------------------------------------------------------------------------
// Skinning

#[test]
fn armature_rest_pose_is_identity() {
    let (s, _, t) = rigged_tube(false);
    let m = s.evaluated_mesh(t).unwrap();
    let ObjectData::Mesh(mi) = s.objects[t].data else { unreachable!() };
    for (a, b) in m.positions.iter().zip(&s.meshes[mi].positions) {
        assert_near(*a, *b, 1e-5);
    }
}

#[test]
fn lbs_loses_more_volume_than_dqs() {
    let mut vols = Vec::new();
    for dqs in [false, true] {
        let (mut s, rig, t) = rigged_tube(dqs);
        set_pose_rot(&mut s, rig, "Upper", Quat::from_axis_angle(Vec3::X, 100f32.to_radians()));
        let m = s.evaluated_mesh(t).unwrap();
        vols.push(volume(&m));
    }
    let v0 = volume(&tube(41, 24, 0.25, 2.0));
    let (lbs, dqs) = (vols[0], vols[1]);
    assert!(lbs < v0 * 0.97, "LBS should collapse the joint: {lbs} vs {v0}");
    assert!((dqs - v0).abs() < (lbs - v0).abs(), "DQS {dqs} should keep more volume than LBS {lbs} (rest {v0})");
}

#[test]
fn fully_weighted_vertices_follow_their_bone() {
    let (mut s, rig, t) = rigged_tube(false);
    set_pose_rot(&mut s, rig, "Upper", Quat::from_axis_angle(Vec3::X, FRAC_PI_2));
    let m = s.evaluated_mesh(t).unwrap();
    let ObjectData::Mesh(mi) = s.objects[t].data else { unreachable!() };
    let rest = &s.meshes[mi].positions;
    for (p, r) in m.positions.iter().zip(rest) {
        if r.z >= 1.3 {
            // Rotate about the joint (0,0,1) around X by 90 degrees.
            let d = *r - v(0.0, 0.0, 1.0);
            let want = v(0.0, 0.0, 1.0) + Quat::from_axis_angle(Vec3::X, FRAC_PI_2).rotate(d);
            assert_near(*p, want, 1e-4);
        } else if r.z <= 0.7 {
            assert_near(*p, *r, 1e-5);
        }
    }
}

#[test]
fn armature_modifier_vertex_group_and_envelopes() {
    let (mut s, rig, t) = rigged_tube(false);
    s.objects[rig].transform.location = Vec3::ZERO;
    set_pose_rot(&mut s, rig, "Lower", Quat::from_axis_angle(Vec3::X, FRAC_PI_2));
    // An overall influence group with zero weight everywhere: no change.
    let ObjectData::Mesh(mi) = s.objects[t].data else { unreachable!() };
    s.meshes[mi].add_vertex_group("Mask");
    if let Modifier::Armature { vertex_group, .. } = &mut s.objects[t].modifiers[0] {
        *vertex_group = Some("Mask".into());
    }
    let m = s.evaluated_mesh(t).unwrap();
    for (a, b) in m.positions.iter().zip(&s.meshes[mi].positions) {
        assert_near(*a, *b, 1e-6);
    }
    // Inverted: full effect.
    if let Modifier::Armature { invert_vertex_group, .. } = &mut s.objects[t].modifiers[0] {
        *invert_vertex_group = true;
    }
    let m = s.evaluated_mesh(t).unwrap();
    assert!(m.positions.iter().zip(&s.meshes[mi].positions).any(|(a, b)| !near(*a, *b, 1e-3)));
    // Envelopes only, groups off: vertices within Lower's envelope follow it.
    if let Modifier::Armature { use_vertex_groups, use_bone_envelopes, vertex_group, .. } = &mut s.objects[t].modifiers[0] {
        *use_vertex_groups = false;
        *use_bone_envelopes = true;
        *vertex_group = None;
    }
    let m = s.evaluated_mesh(t).unwrap();
    let i = s.meshes[mi].positions.iter().position(|p| p.z.abs() < 1e-6).unwrap();
    let r = s.meshes[mi].positions[i];
    assert_near(m.positions[i], Quat::from_axis_angle(Vec3::X, FRAC_PI_2).rotate(r), 1e-4);
}

#[test]
fn multi_modifier_deforms_the_previous_input() {
    let (mut s, rig, t) = rigged_tube(false);
    set_pose_rot(&mut s, rig, "Lower", Quat::from_axis_angle(Vec3::Z, 0.5));
    // A displace before the armature; with Multi Modifier the armature
    // deforms the displace's input instead, and without a group the
    // armature result replaces it.
    let mut d = Modifier::with_defaults("Displace").unwrap();
    if let Modifier::Displace { strength, .. } = &mut d {
        *strength = 0.5;
    }
    s.objects[t].modifiers.insert(0, d);
    let plain = s.evaluated_mesh(t).unwrap();
    if let Modifier::Armature { use_multi_modifier, .. } = &mut s.objects[t].modifiers[1] {
        *use_multi_modifier = true;
    }
    let multi = s.evaluated_mesh(t).unwrap();
    s.objects[t].modifiers.remove(0);
    if let Modifier::Armature { use_multi_modifier, .. } = &mut s.objects[t].modifiers[0] {
        *use_multi_modifier = false;
    }
    let only_arm = s.evaluated_mesh(t).unwrap();
    for (a, b) in multi.positions.iter().zip(&only_arm.positions) {
        assert_near(*a, *b, 1e-5);
    }
    assert!(plain.positions.iter().zip(&only_arm.positions).any(|(a, b)| !near(*a, *b, 1e-3)));
}

#[test]
fn automatic_weights_sum_to_one() {
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", two_bone_arm());
    let t = s.add_mesh_object("Tube", tube(21, 12, 0.25, 2.0));
    assert!(s.parent_with_armature(t, rig, ArmatureWeights::Automatic));
    assert_eq!(s.objects[t].parent, Some(rig));
    assert!(matches!(s.objects[t].modifiers[0], Modifier::Armature { .. }));
    let ObjectData::Mesh(mi) = s.objects[t].data else { unreachable!() };
    let m = &s.meshes[mi];
    let (lo, up) = (m.vertex_group_index("Lower").unwrap(), m.vertex_group_index("Upper").unwrap());
    for (i, p) in m.positions.iter().enumerate() {
        let sum = m.weight(i, lo) + m.weight(i, up);
        assert_close(sum, 1.0, 1e-4);
        if p.z < 0.3 {
            assert!(m.weight(i, lo) > 0.9, "vertex at {p:?}: {}", m.weight(i, lo));
        }
        if p.z > 1.7 {
            assert!(m.weight(i, up) > 0.9, "vertex at {p:?}: {}", m.weight(i, up));
        }
    }
}

#[test]
fn envelope_and_empty_groups() {
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", two_bone_arm());
    let t = s.add_mesh_object("Tube", tube(5, 6, 0.05, 2.0));
    assert!(s.parent_with_armature(t, rig, ArmatureWeights::Empty));
    let ObjectData::Mesh(mi) = s.objects[t].data else { unreachable!() };
    assert_eq!(s.meshes[mi].vertex_groups, vec!["Lower".to_string(), "Upper".to_string()]);
    assert!(s.meshes[mi].weights.iter().all(|w| w.is_empty()));
    let t2 = s.add_mesh_object("Thin", tube(5, 6, 0.05, 2.0));
    assert!(s.parent_with_armature(t2, rig, ArmatureWeights::Envelope));
    let ObjectData::Mesh(mi) = s.objects[t2].data else { unreachable!() };
    let m = &s.meshes[mi];
    let lo = m.vertex_group_index("Lower").unwrap();
    // A vertex on Lower's axis region is fully inside its envelope.
    let i = m.positions.iter().position(|p| (p.z - 0.5).abs() < 1e-5).unwrap();
    assert_close(m.weight(i, lo), 1.0, 1e-5);
}

// ---------------------------------------------------------------------------
// IK

fn ik_scene(goal: Vec3, stretch: bool) -> (Scene, usize) {
    let mut a = Armature::default();
    a.add_bone(Bone::new("B0", v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0)));
    for i in 1..3 {
        let mut b = Bone::new(&format!("B{i}"), v(0.0, 0.0, i as f32), v(0.0, 0.0, i as f32 + 1.0));
        b.parent = Some(i - 1);
        b.use_connect = true;
        a.add_bone(b);
    }
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", a);
    let g = s.add_object(Object::new("Goal", ObjectData::Empty));
    s.objects[g].transform.location = goal;
    let mut c = Constraint::with_defaults("IK", true).unwrap();
    if let ConstraintKind::Ik { target, use_stretch, .. } = &mut c.kind {
        *target = Target::object("Goal");
        *use_stretch = stretch;
    }
    let mut pb = PoseBone::new("B2");
    pb.constraints.push(c);
    s.objects[rig].pose.push(pb);
    (s, rig)
}

#[test]
fn ik_reaches_a_reachable_target() {
    for goal in [v(1.0, 0.0, 1.8), v(-0.5, 1.2, 1.5), v(0.3, 0.2, 2.6)] {
        let (s, rig) = ik_scene(goal, false);
        let tip = bone_point(&s, rig, 2, 1.0);
        assert!(near(tip, goal, 1e-2), "goal {goal:?}: tip at {tip:?}");
        // Bone lengths are kept.
        assert_close((bone_point(&s, rig, 1, 1.0) - bone_point(&s, rig, 1, 0.0)).length(), 1.0, 1e-4);
        // The root stays put.
        assert_near(bone_point(&s, rig, 0, 0.0), Vec3::ZERO, 1e-5);
    }
}

#[test]
fn ik_points_at_an_unreachable_target() {
    let goal = v(0.0, 10.0, 1.0);
    let (s, rig) = ik_scene(goal, false);
    let tip = bone_point(&s, rig, 2, 1.0);
    let want = goal.normalize() * 3.0;
    assert!(near(tip, want, 5e-2), "tip {tip:?}, want {want:?}");
    // With stretch (and IK Stretch on the bones) the chain reaches it.
    let (mut s, rig) = ik_scene(goal, true);
    for b in ["B0", "B1", "B2"] {
        let pose = &mut s.objects[rig].pose;
        let i = match pose.iter().position(|p| p.name == b) {
            Some(i) => i,
            None => {
                pose.push(PoseBone::new(b));
                pose.len() - 1
            }
        };
        pose[i].ik.stretch = 0.5;
    }
    let tip = bone_point(&s, rig, 2, 1.0);
    assert!(near(tip, goal, 0.1), "stretched tip {tip:?}");
}

#[test]
fn ik_influence_and_mute() {
    let goal = v(1.0, 0.0, 1.8);
    let (mut s, rig) = ik_scene(goal, false);
    let pb = s.objects[rig].pose.iter_mut().find(|p| p.name == "B2").unwrap();
    pb.constraints[0].mute = true;
    assert_near(bone_point(&s, rig, 2, 1.0), v(0.0, 0.0, 3.0), 1e-5);
    let pb = s.objects[rig].pose.iter_mut().find(|p| p.name == "B2").unwrap();
    pb.constraints[0].mute = false;
    pb.constraints[0].influence = 0.0;
    assert_near(bone_point(&s, rig, 2, 1.0), v(0.0, 0.0, 3.0), 1e-5);
}

#[test]
fn ik_respects_locks() {
    let goal = v(1.0, 0.0, 1.8);
    let (mut s, rig) = ik_scene(goal, false);
    // Lock every axis of every bone: nothing moves.
    for b in ["B0", "B1"] {
        let mut pb = PoseBone::new(b);
        pb.ik.lock = [true; 3];
        s.objects[rig].pose.push(pb);
    }
    s.objects[rig].pose.iter_mut().find(|p| p.name == "B2").unwrap().ik.lock = [true; 3];
    assert_near(bone_point(&s, rig, 2, 1.0), v(0.0, 0.0, 3.0), 1e-4);
}

// ---------------------------------------------------------------------------
// Constraints

/// Scene with an empty "Owner" (constraint `c`) and an empty "T".
fn con_scene(c: Constraint, t: Mat4) -> (Scene, usize, usize) {
    let mut s = Scene::new();
    let tgt = s.add_object(Object::new("T", ObjectData::Empty));
    s.objects[tgt].transform = sex_3d::scene::Transform::from_matrix(&t);
    let o = s.add_object(Object::new("Owner", ObjectData::Empty));
    let mut c = c;
    if let Some(tt) = c.target_mut() {
        tt.object = "T".into();
    }
    s.objects[o].constraints.push(c);
    (s, o, tgt)
}

fn con(ty: &str) -> Constraint {
    Constraint::with_defaults(ty, false).unwrap_or_else(|| panic!("no constraint {ty}"))
}

#[test]
fn copy_constraints() {
    let t = Mat4::from_trs(v(1.0, 2.0, 3.0), Quat::from_axis_angle(Vec3::Z, FRAC_PI_4), v(2.0, 3.0, 4.0));
    let (s, o, _) = con_scene(con("COPY_LOCATION"), t);
    assert_near(s.world_matrix(o).translation_part(), v(1.0, 2.0, 3.0), 1e-5);

    let (s, o, _) = con_scene(con("COPY_ROTATION"), t);
    let w = s.world_matrix(o);
    assert!(w.mat3().normalized().approx_eq(&Mat3::from_axis_angle(Vec3::Z, FRAC_PI_4), 1e-5));
    assert_near(w.scale_part(), Vec3::ONE, 1e-5);

    let (s, o, _) = con_scene(con("COPY_SCALE"), t);
    assert_near(s.world_matrix(o).scale_part(), v(2.0, 3.0, 4.0), 1e-5);

    let (s, o, _) = con_scene(con("COPY_TRANSFORMS"), t);
    assert!(s.world_matrix(o).approx_eq(&t, 1e-5));

    // Axis toggles and invert.
    let mut c = con("COPY_LOCATION");
    if let ConstraintKind::CopyLocation { use_xyz, invert, .. } = &mut c.kind {
        *use_xyz = [true, false, true];
        invert[2] = true;
    }
    let (s, o, _) = con_scene(c, t);
    assert_near(s.world_matrix(o).translation_part(), v(1.0, 0.0, -3.0), 1e-5);

    // Influence blends; mute switches off.
    let mut c = con("COPY_LOCATION");
    c.influence = 0.5;
    let (mut s, o, _) = con_scene(c, t);
    assert_near(s.world_matrix(o).translation_part(), v(0.5, 1.0, 1.5), 1e-5);
    s.objects[o].constraints[0].mute = true;
    assert_near(s.world_matrix(o).translation_part(), Vec3::ZERO, 1e-6);
}

#[test]
fn limit_constraints() {
    let mut c = con("LIMIT_DISTANCE");
    if let ConstraintKind::LimitDistance { distance, .. } = &mut c.kind {
        *distance = 1.0;
    }
    let (mut s, o, _) = con_scene(c, Mat4::IDENTITY);
    s.objects[o].transform.location = v(3.0, 0.0, 0.0);
    assert_near(s.world_matrix(o).translation_part(), v(1.0, 0.0, 0.0), 1e-5);

    let mut c = con("LIMIT_LOCATION");
    if let ConstraintKind::LimitLocation { use_max, max, use_min, min } = &mut c.kind {
        use_max[0] = true;
        max[0] = 1.0;
        use_min[2] = true;
        min[2] = -0.5;
    }
    let (mut s, o, _) = con_scene(c, Mat4::IDENTITY);
    s.objects[o].transform.location = v(5.0, 7.0, -3.0);
    assert_near(s.world_matrix(o).translation_part(), v(1.0, 7.0, -0.5), 1e-5);

    let mut c = con("LIMIT_ROTATION");
    if let ConstraintKind::LimitRotation { use_limit, min, max, .. } = &mut c.kind {
        use_limit[2] = true;
        min[2] = -0.1;
        max[2] = 0.1;
    }
    let (mut s, o, _) = con_scene(c, Mat4::IDENTITY);
    s.objects[o].transform.rotation = Quat::from_axis_angle(Vec3::Z, 1.0);
    let e = s.world_matrix(o).mat3().to_euler(EulerOrder::XYZ);
    assert_close(e[2], 0.1, 1e-5);

    let mut c = con("LIMIT_SCALE");
    if let ConstraintKind::LimitScale { use_min, min, .. } = &mut c.kind {
        use_min[0] = true;
        min[0] = 0.5;
    }
    let (mut s, o, _) = con_scene(c, Mat4::IDENTITY);
    s.objects[o].transform.scale = v(0.1, 0.1, 0.1);
    assert_near(s.world_matrix(o).scale_part(), v(0.5, 0.1, 0.1), 1e-5);
}

#[test]
fn maintain_volume_and_transformation() {
    let (mut s, o, _) = con_scene(con("MAINTAIN_VOLUME"), Mat4::IDENTITY);
    s.objects[o].transform.scale = v(1.0, 4.0, 1.0);
    assert_near(s.world_matrix(o).scale_part(), v(0.5, 4.0, 0.5), 1e-5);

    // Target X 0..1 drives owner Z rotation 0..90 degrees.
    let mut c = con("TRANSFORM");
    if let ConstraintKind::Transformation { map_to, from_max, to_max, map, .. } = &mut c.kind {
        *map_to = sex_3d::constraints::TransformChannel::Rotation;
        from_max[0] = 1.0;
        to_max[2] = FRAC_PI_2;
        *map = [0, 0, 0];
    }
    let (s, o, _) = con_scene(c, Mat4::translation(v(0.5, 0.0, 0.0)));
    let e = s.world_matrix(o).mat3().to_euler(EulerOrder::XYZ);
    assert_close(e[2], FRAC_PI_4, 1e-4);
}

#[test]
fn child_of_and_set_inverse() {
    let (mut s, o, t) = con_scene(con("CHILD_OF"), Mat4::translation(v(1.0, 0.0, 0.0)));
    assert_near(s.world_matrix(o).translation_part(), v(1.0, 0.0, 0.0), 1e-5);
    assert!(s.child_of_set_inverse(o, None, 0));
    assert_near(s.world_matrix(o).translation_part(), Vec3::ZERO, 1e-5);
    s.objects[t].transform.location = v(1.0, 2.0, 0.0);
    assert_near(s.world_matrix(o).translation_part(), v(0.0, 2.0, 0.0), 1e-5);
    // Rotation follows too (around the target).
    s.objects[t].transform.rotation = Quat::from_axis_angle(Vec3::Z, FRAC_PI_2);
    assert_near(s.world_matrix(o).translation_part(), v(1.0, 1.0, 0.0), 1e-5);
}

#[test]
fn tracking_constraints() {
    // Track To (object defaults: -Z at the target, Y up).
    let (s, o, _) = con_scene(con("TRACK_TO"), Mat4::translation(v(5.0, 0.0, 0.0)));
    let m = s.world_matrix(o).mat3();
    assert_near(m.mul_vec(-Vec3::Z), v(1.0, 0.0, 0.0), 1e-5);
    assert_near(m.mul_vec(Vec3::Y), v(0.0, 0.0, 1.0), 1e-5);

    let (s, o, _) = con_scene(con("DAMPED_TRACK"), Mat4::translation(v(0.0, 0.0, 5.0)));
    assert_near(s.world_matrix(o).mat3().mul_vec(Vec3::Y), Vec3::Z, 1e-5);

    let mut c = con("DAMPED_TRACK");
    if let ConstraintKind::DampedTrack { track_axis, .. } = &mut c.kind {
        *track_axis = TrackAxis::NegX;
    }
    let (s, o, _) = con_scene(c, Mat4::translation(v(0.0, 3.0, 0.0)));
    assert_near(s.world_matrix(o).mat3().mul_vec(-Vec3::X), Vec3::Y, 1e-5);

    // Locked Track: Y towards the target around a locked Z.
    let (s, o, _) = con_scene(con("LOCKED_TRACK"), Mat4::translation(v(1.0, 1.0, 5.0)));
    let m = s.world_matrix(o).mat3();
    assert_near(m.mul_vec(Vec3::Z), Vec3::Z, 1e-5);
    assert_near(m.mul_vec(Vec3::Y), v(1.0, 1.0, 0.0).normalize(), 1e-5);
}

#[test]
fn stretch_to_and_floor() {
    let mut c = con("STRETCH_TO");
    if let ConstraintKind::StretchTo { rest_length, .. } = &mut c.kind {
        *rest_length = 2.0;
    }
    let (s, o, _) = con_scene(c, Mat4::translation(v(0.0, 4.0, 0.0)));
    let w = s.world_matrix(o);
    assert_near(w.mat3().mul_vec(Vec3::Y), v(0.0, 2.0, 0.0), 1e-5);
    let sx = 0.5f32.sqrt();
    assert_near(w.scale_part(), v(sx, 2.0, sx), 1e-5);

    let (mut s, o, _) = con_scene(con("FLOOR"), Mat4::translation(v(0.0, 0.0, 1.0)));
    s.objects[o].transform.location = v(2.0, 0.0, -2.0);
    assert_near(s.world_matrix(o).translation_part(), v(2.0, 0.0, 1.0), 1e-5);
    s.objects[o].transform.location = v(2.0, 0.0, 3.0);
    assert_near(s.world_matrix(o).translation_part(), v(2.0, 0.0, 3.0), 1e-5);
}

#[test]
fn constraint_spaces_and_bone_targets() {
    // Copy Location from a bone (head, then tail with head_tail 1).
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", two_bone_arm());
    s.objects[rig].transform.location = v(0.0, 5.0, 0.0);
    let o = s.add_object(Object::new("Owner", ObjectData::Empty));
    let mut c = con("COPY_LOCATION");
    *c.target_mut().unwrap() = Target { object: "Rig".into(), subtarget: "Upper".into(), head_tail: 0.0 };
    s.objects[o].constraints.push(c);
    assert_near(s.world_matrix(o).translation_part(), v(0.0, 5.0, 1.0), 1e-5);
    s.objects[o].constraints[0].target_mut().unwrap().head_tail = 1.0;
    assert_near(s.world_matrix(o).translation_part(), v(0.0, 5.0, 2.0), 1e-5);
    // Local space: copy the target's local location onto the owner's.
    let mut c = con("COPY_LOCATION");
    c.target_mut().unwrap().object = "Rig".into();
    c.owner_space = sex_3d::constraints::Space::Local;
    c.target_space = sex_3d::constraints::Space::Local;
    let p = s.add_object(Object::new("Parent", ObjectData::Empty));
    s.objects[p].transform.location = v(10.0, 0.0, 0.0);
    let o2 = s.add_object(Object::new("Owner2", ObjectData::Empty));
    s.objects[o2].parent = Some(p);
    s.objects[o2].constraints.push(c);
    assert_near(s.world_matrix(o2).translation_part(), v(10.0, 5.0, 0.0), 1e-5);
}

#[test]
fn bone_constraint_copy_rotation() {
    let mut s = Scene::new();
    let rig = s.add_armature_object("Rig", two_bone_arm());
    let e = s.add_object(Object::new("Ctl", ObjectData::Empty));
    s.objects[e].transform.rotation = Quat::from_axis_angle(Vec3::X, FRAC_PI_2);
    let mut c = Constraint::with_defaults("COPY_ROTATION", true).unwrap();
    c.target_mut().unwrap().object = "Ctl".into();
    let mut pb = PoseBone::new("Lower");
    pb.constraints.push(c);
    s.objects[rig].pose.push(pb);
    // World-space copy: Lower's orientation becomes the empty's, so its
    // Y axis (rest +Z ... now the empty's Y) points along world Z.
    let m = (s.world_matrix(rig) * s.pose_matrices(rig)[0]).mat3();
    assert_near(m.mul_vec(Vec3::Y), Quat::from_axis_angle(Vec3::X, FRAC_PI_2).rotate(Vec3::Y), 1e-5);
    // The child follows.
    assert_near(bone_point(&s, rig, 1, 0.0), bone_point(&s, rig, 0, 1.0), 1e-5);
}

// ---------------------------------------------------------------------------
// Shape keys

fn quad() -> Mesh {
    let mut m = Mesh::new();
    m.positions = vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(1.0, 1.0, 0.0), v(0.0, 1.0, 0.0)];
    m.add_face(&[0, 1, 2, 3]);
    m
}

fn lifted(m: &Mesh, dz: f32) -> Vec<Vec3> {
    m.positions.iter().map(|p| *p + v(0.0, 0.0, dz)).collect()
}

#[test]
fn shape_keys_mix() {
    let mut m = quad();
    assert_eq!(m.add_shape_key("Basis", false), 0);
    let k = m.add_shape_key("Up", false);
    m.shape_keys[k].positions = lifted(&m, 1.0);
    m.shape_keys[k].value = 0.5;
    assert!(m.evaluated_shape().iter().all(|p| (p.z - 0.5).abs() < 1e-6));
    // Two keys add up.
    let k2 = m.add_shape_key("Up2", false);
    m.shape_keys[k2].positions = lifted(&m, 2.0);
    m.shape_keys[k2].value = 0.25;
    assert!(m.evaluated_shape().iter().all(|p| (p.z - 1.0).abs() < 1e-6));
    // Muted keys do nothing; sliders clamp the value.
    m.shape_keys[k2].mute = true;
    m.shape_keys[k].value = 3.0;
    assert!(m.evaluated_shape().iter().all(|p| (p.z - 1.0).abs() < 1e-6));
    // A vertex group limits a key.
    let g = m.add_vertex_group("Corner");
    m.set_weight(0, g, 1.0);
    m.shape_keys[k].vertex_group = Some("Corner".into());
    let e = m.evaluated_shape();
    assert_close(e[0].z, 1.0, 1e-6);
    assert_close(e[2].z, 0.0, 1e-6);
    // Scene evaluation applies keys before modifiers.
    let mut s = Scene::new();
    let o = s.add_mesh_object("Q", m);
    let ev = s.evaluated_mesh(o).unwrap();
    assert_close(ev.positions[0].z, 1.0, 1e-6);
}

#[test]
fn absolute_shape_keys() {
    let mut m = quad();
    m.add_shape_key("Basis", false);
    let a = m.add_shape_key("A", false);
    let b = m.add_shape_key("B", false);
    m.shape_keys[a].positions = lifted(&m, 1.0);
    m.shape_keys[b].positions = lifted(&m, 2.0);
    for k in &mut m.shape_keys {
        k.interpolation = KeyInterpolation::Linear;
    }
    assert_eq!((m.shape_keys[a].frame, m.shape_keys[b].frame), (10.0, 20.0));
    m.shape_key_settings.use_relative = false;
    for (t, z) in [(0.0, 0.0), (5.0, 0.5), (10.0, 1.0), (15.0, 1.5), (20.0, 2.0), (30.0, 2.0)] {
        m.shape_key_settings.eval_time = t;
        let e = m.evaluated_shape();
        assert!(e.iter().all(|p| (p.z - z).abs() < 1e-5), "time {t}: {:?}", e[0]);
    }
}

#[test]
fn vertex_data_survives_topology_ops() {
    let mut m = tube(4, 6, 1.0, 1.0);
    let g = m.add_vertex_group("G");
    for i in 0..m.positions.len() {
        m.set_weight(i, g, i as f32 / 100.0);
    }
    m.add_shape_key("Basis", false);
    let k = m.add_shape_key("K", false);
    m.shape_keys[k].positions = lifted(&m, 1.0);
    let mut other = quad();
    other.add_vertex_group("H");
    other.set_weight(0, 0, 1.0);
    let n0 = m.positions.len();
    m.append(&other);
    assert_eq!(m.vertex_groups, vec!["G".to_string(), "H".to_string()]);
    assert_eq!(m.weight(n0, 1), 1.0);
    assert_eq!(m.shape_keys[k].positions.len(), m.positions.len());
    // Welding merges vertices: weights and keys stay one per vertex.
    m.positions.push(m.positions[0]);
    m.weld(1e-4);
    assert!(m.weights.len() <= m.positions.len());
    assert!(m.shape_keys.iter().all(|k| k.positions.len() == m.positions.len()));
}

// ---------------------------------------------------------------------------
// Lattice, Hook, Warp

fn cube_object(s: &mut Scene) -> usize {
    s.add_mesh_object("Cube", sex_3d::primitives::cube(1.0))
}

#[test]
fn identity_lattice_is_identity() {
    for interp in [KeyInterpolation::BSpline, KeyInterpolation::Linear, KeyInterpolation::Cardinal, KeyInterpolation::CatmullRom] {
        let mut s = Scene::new();
        let mut l = Lattice::new(3, 4, 2);
        (l.interpolation_u, l.interpolation_v, l.interpolation_w) = (interp, interp, interp);
        let lo = s.add_lattice_object("Lat", l);
        s.objects[lo].transform.scale = Vec3::splat(2.0);
        s.objects[lo].transform.location = v(0.1, 0.2, 0.0);
        let c = cube_object(&mut s);
        let mut md = Modifier::with_defaults("Lattice").unwrap();
        if let Modifier::Lattice { object, .. } = &mut md {
            *object = "Lat".into();
        }
        s.objects[c].modifiers.push(md);
        let m = s.evaluated_mesh(c).unwrap();
        let ObjectData::Mesh(mi) = s.objects[c].data else { unreachable!() };
        for (a, b) in m.positions.iter().zip(&s.meshes[mi].positions) {
            assert_near(*a, *b, 1e-5);
        }
    }
}

#[test]
fn moved_lattice_moves_the_mesh() {
    let mut s = Scene::new();
    let mut l = Lattice::new(2, 2, 2);
    for p in &mut l.points {
        if p.z > 0.0 {
            p.z += 0.5;
        }
    }
    l.interpolation_u = KeyInterpolation::Linear;
    l.interpolation_v = KeyInterpolation::Linear;
    l.interpolation_w = KeyInterpolation::Linear;
    let lo = s.add_lattice_object("Lat", l);
    s.objects[lo].transform.scale = Vec3::splat(1.0);
    let c = cube_object(&mut s);
    let mut md = Modifier::with_defaults("Lattice").unwrap();
    if let Modifier::Lattice { object, .. } = &mut md {
        *object = "Lat".into();
    }
    s.objects[c].modifiers.push(md);
    let m = s.evaluated_mesh(c).unwrap();
    // Linear 2x2x2 lattice: top face (z 0.5) lifted by 0.5, bottom stays.
    for p in &m.positions {
        assert!((p.z - 1.0).abs() < 1e-5 || (p.z + 0.5).abs() < 1e-5, "{p:?}");
    }
}

#[test]
fn hook_moves_its_vertices() {
    let mut s = Scene::new();
    let h = s.add_object(Object::new("H", ObjectData::Empty));
    let c = cube_object(&mut s);
    let mut md = Modifier::with_defaults("Hook").unwrap();
    if let Modifier::Hook { object, vertex_indices, .. } = &mut md {
        *object = "H".into();
        *vertex_indices = vec![0];
    }
    s.objects[c].modifiers.push(md);
    s.objects[h].transform.location = v(0.0, 0.0, 1.0);
    let m = s.evaluated_mesh(c).unwrap();
    let ObjectData::Mesh(mi) = s.objects[c].data else { unreachable!() };
    let rest = s.meshes[mi].positions.clone();
    assert_near(m.positions[0], rest[0] + v(0.0, 0.0, 1.0), 1e-5);
    assert_near(m.positions[1], rest[1], 1e-6);
    // Falloff: a radius smaller than the distance to the centre leaves it.
    if let Modifier::Hook { falloff_type, falloff_radius, center, .. } = &mut s.objects[c].modifiers[0] {
        *falloff_type = Falloff::Linear;
        *falloff_radius = 0.1;
        *center = [5.0, 5.0, 5.0];
    }
    let m = s.evaluated_mesh(c).unwrap();
    assert_near(m.positions[0], rest[0], 1e-6);
}

#[test]
fn warp_moves_space() {
    let mut s = Scene::new();
    s.add_object(Object::new("From", ObjectData::Empty));
    let to = s.add_object(Object::new("To", ObjectData::Empty));
    s.objects[to].transform.location = v(1.0, 0.0, 0.0);
    let c = cube_object(&mut s);
    let mut md = Modifier::with_defaults("Warp").unwrap();
    if let Modifier::Warp { object_from, object_to, falloff_type, .. } = &mut md {
        *object_from = "From".into();
        *object_to = "To".into();
        *falloff_type = Falloff::None;
    }
    s.objects[c].modifiers.push(md);
    let m = s.evaluated_mesh(c).unwrap();
    let ObjectData::Mesh(mi) = s.objects[c].data else { unreachable!() };
    for (a, b) in m.positions.iter().zip(&s.meshes[mi].positions) {
        assert_near(*a, *b + v(1.0, 0.0, 0.0), 1e-5);
    }
}

#[test]
fn deform_modifier_vertex_group_masks() {
    let mut s = Scene::new();
    let c = cube_object(&mut s);
    let ObjectData::Mesh(mi) = s.objects[c].data else { unreachable!() };
    let g = s.meshes[mi].add_vertex_group("Top");
    for i in 0..s.meshes[mi].positions.len() {
        if s.meshes[mi].positions[i].z > 0.0 {
            s.meshes[mi].set_weight(i, g, 1.0);
        }
    }
    let mut md = Modifier::with_defaults("Displace").unwrap();
    if let Modifier::Displace { strength, vertex_group, .. } = &mut md {
        *strength = 1.0;
        *vertex_group = Some("Top".into());
    }
    s.objects[c].modifiers.push(md);
    let m = s.evaluated_mesh(c).unwrap();
    for (a, b) in m.positions.iter().zip(&s.meshes[mi].positions) {
        if b.z > 0.0 {
            assert!(!near(*a, *b, 1e-3));
        } else {
            assert_near(*a, *b, 1e-6);
        }
    }
}

// ---------------------------------------------------------------------------
// Vertex weight modifiers

fn weights_of(m: &Mesh, g: &str) -> Vec<Option<f32>> {
    let g = m.vertex_group_index(g).unwrap();
    (0..m.positions.len()).map(|i| m.weight_opt(i, g)).collect()
}

#[test]
fn vertex_weight_edit_mix_proximity() {
    let mut s = Scene::new();
    let mut m = quad();
    let (a, b) = (m.add_vertex_group("A"), m.add_vertex_group("B"));
    m.set_weight(0, a, 0.2);
    m.set_weight(1, b, 0.3);
    m.set_weight(0, b, 0.5);
    let q = s.add_mesh_object("Q", m);
    // Edit: add every vertex at the default weight.
    let mut md = Modifier::with_defaults("VertexWeightEdit").unwrap();
    if let Modifier::VertexWeightEdit { vertex_group, default_weight, use_add, .. } = &mut md {
        *vertex_group = "A".into();
        *default_weight = 0.6;
        *use_add = true;
    }
    s.objects[q].modifiers = vec![md.clone()];
    let w = weights_of(&s.evaluated_mesh(q).unwrap(), "A");
    assert_eq!(w, vec![Some(0.2), Some(0.6), Some(0.6), Some(0.6)]);
    // Remove below a threshold.
    if let Modifier::VertexWeightEdit { use_add, use_remove, remove_threshold, .. } = &mut md {
        *use_add = false;
        *use_remove = true;
        *remove_threshold = 0.5;
    }
    s.objects[q].modifiers = vec![md];
    assert_eq!(weights_of(&s.evaluated_mesh(q).unwrap(), "A"), vec![None; 4]);
    // Mix: A + B over vertices in either group.
    let mut md = Modifier::with_defaults("VertexWeightMix").unwrap();
    if let Modifier::VertexWeightMix { vertex_group_a, vertex_group_b, mix_mode, mix_set, .. } = &mut md {
        *vertex_group_a = "A".into();
        *vertex_group_b = "B".into();
        *mix_mode = WeightMixMode::Add;
        *mix_set = WeightMixSet::Or;
    }
    s.objects[q].modifiers = vec![md];
    let w = weights_of(&s.evaluated_mesh(q).unwrap(), "A");
    assert!((w[0].unwrap() - 0.7).abs() < 1e-6 && (w[1].unwrap() - 0.3).abs() < 1e-6 && w[2].is_none());
    // Proximity to an empty at the origin, 0..2 mapped to 0..1.
    s.add_object(Object::new("Origin", ObjectData::Empty));
    let mut md = Modifier::with_defaults("VertexWeightProximity").unwrap();
    if let Modifier::VertexWeightProximity { vertex_group, target, proximity_mode, min_dist, max_dist, .. } = &mut md {
        *vertex_group = "B".into();
        *target = "Origin".into();
        *proximity_mode = ProximityMode::Object;
        *min_dist = 0.0;
        *max_dist = 2.0;
    }
    s.objects[q].modifiers = vec![md];
    let w = weights_of(&s.evaluated_mesh(q).unwrap(), "B");
    assert_eq!(w[0], Some(0.0));
    assert!((w[1].unwrap() - 0.5).abs() < 1e-6);
    assert!(w[2].is_none());
}

// ---------------------------------------------------------------------------
// polyops

fn run_ok(s: &mut Scene, src: &str) {
    if let Err(e) = sex_3d::script::run(s, src) {
        panic!("{e}\n--- program ---\n{src}");
    }
}

const RIG_PROGRAM: &str = "polyops 1
add_primitive cylinder name=Body segments=8 depth=2 location=0,0,1
vgroup Body Lower indices=0,1,2,3 weights=1,0.5,0.25,0
vgroup Body Upper indices=4,5 weights=0.75
shape_key Body Basis
shape_key Body Smile value=0.4 slider_min=-1 slider_max=2 vertex_group=Upper interpolation=linear
shape_key Body Frown relative_key=Smile mute=1 frame=30
shape_key_settings Body use_relative=0 eval_time=12.5
armature_new Skeleton as=$a
bone $a Root head=0,0,0 tail=0,0,1 roll=0.25 envelope_distance=0.4
bone $a \"Spine.L\" head=0,0,1 tail=0,0,2 parent=Root use_connect=1 inherit_scale=none use_deform=0
bone $a Tip head=0,0,2 tail=0,1,3 parent=\"Spine.L\" use_inherit_rotation=0 bbone_segments=4 head_radius=0.3
bone_collection $a Deform Root \"Spine.L\"
bone_collection $a Empty
add_armature Rig data=$a location=1,0,0
add_armature Spare
lattice_new Cage points=3,2,2 interpolation=linear,bspline,cardinal use_outside=1
lattice_set Cage verts=-0.5,-0.5,-0.5,0,-0.5,-0.5,0.5,-0.5,-0.5,-0.5,0.5,-0.5,0,0.5,-0.5,0.5,0.5,-0.5,-0.5,-0.5,0.5,0,-0.5,0.5,0.5,-0.5,0.5,-0.5,0.5,0.5,0,0.7,0.5,0.5,0.5,0.5
add_lattice Box data=Cage scale=2
add_empty Goal location=0,2,2
add_empty Pole location=1,0,1
add_empty Prop
parent Prop Rig bone=Tip
parent Body Rig type=armature
pose_bone Rig Root location=0.1,0,0 rotation_mode=zyx euler=0.1,0.2,0.3 scale=1,2,1
pose_bone Rig \"Spine.L\" quaternion=0.9,0.1,0,0 ik_lock=1,0,0 ik_stiffness=0.5,0,0 ik_stretch=0.2 ik_limit=0,1,0 ik_min=0,-1,0 ik_max=0,1,0
pose_bone Rig Tip rotation_mode=axis_angle axis_angle=0.5,0,0,1
constraint_add Rig ik bone=Tip target=Goal pole_target=Pole pole_angle=-1.5 chain_count=2 use_stretch=0 iterations=100
constraint_add Rig copy_rotation bone=Root target=Goal influence=0.3 mix_mode=before euler_order=zxy owner_space=local target_space=local_with_parent
constraint_add Rig armature bone=Tip
constraint_target Rig 1 Rig subtarget=Root weight=0.5 bone=Tip
constraint_add Goal child_of target=Rig subtarget=Root use_rotation=0,0,1 inverse_matrix=1,0,0,0,0,1,0,0,0,0,1,0,1,2,3,1
constraint_add Goal track_to target=Pole track_axis=track_negative_x up_axis=z use_target_z=1
constraint_add Goal limit_location use_min=1,0,0 min=-1,0,0 use_max=0,0,1 max=0,0,4 mute=1
constraint_add Goal limit_rotation use_limit=1,1,0 min=-0.1,-0.2,0 max=0.1,0.2,0
constraint_add Goal transform target=Pole map_from=rotation map_to=scale from_max=1,1,1 to_min=1,1,1 to_max=2,2,2 map=z,y,x mix_mode=replace
constraint_add Goal stretch_to target=Pole rest_length=2 volume=no_volume keep_axis=plane_z bulge=1.5 use_bulge_min=1 bulge_min=0.5
constraint_add Goal floor target=Pole floor_location=track_negative_z offset=0.2 use_rotation=1
constraint_add Goal maintain_volume mode=uniform free_axis=x volume=2
constraint_add Goal limit_distance target=Pole distance=3 limit_mode=onsurface head_tail=0.5 space_object=Rig space_subtarget=Root
constraint_add Goal copy_scale target=Pole use_xyz=1,0,1 power=2 use_make_uniform=1 use_add=1
constraint_add Goal copy_transforms target=Pole mix_mode=after_split
constraint_add Goal copy_location target=Pole use_offset=1 invert=0,1,0
constraint_add Goal damped_track target=Pole track_axis=track_z
constraint_add Goal locked_track target=Pole lock_axis=x name=\"My Lock\"
constraint_add Goal limit_scale use_max=1,1,1 max=2,2,2
modifier_add Body Armature object=Rig use_bone_envelopes=1 use_deform_preserve_volume=1 vertex_group=Lower invert_vertex_group=1 use_multi_modifier=1
modifier_add Body Hook object=Rig subtarget=Tip falloff_type=smooth falloff_radius=2 center=0,0,1 strength=0.5 vertex_indices=1,2,3 use_falloff_uniform=1 matrix_inverse=1,0,0,0,0,1,0,0,0,0,1,0,0,0,-1,1
modifier_add Body Lattice object=Box strength=0.8
modifier_add Body Warp object_from=Goal object_to=Pole falloff_type=sphere falloff_radius=3 use_volume_preserve=1 strength=0.7
modifier_add Body VertexWeightEdit vertex_group=Upper use_add=1 add_threshold=0.1 use_remove=1 remove_threshold=0.05 falloff_type=step invert_falloff=1 normalize=1 mask_constant=0.5 default_weight=0.2
modifier_add Body VertexWeightMix vertex_group_a=Upper vertex_group_b=Lower mix_mode=avg mix_set=or default_weight_b=0.3 invert_vertex_group_b=1
modifier_add Body VertexWeightProximity vertex_group=Upper target=Goal proximity_mode=object min_dist=1 max_dist=0 falloff_type=sharp use_proximity_geometry=1,1,0
modifier_add Body Smooth factor=0.5 vertex_group=Upper
modifier_add Body Displace strength=0.1 vertex_group=none
end
";

#[test]
fn polyops_round_trip() {
    let mut s = Scene::startup();
    run_ok(&mut s, RIG_PROGRAM);
    // Spot checks of what the program built.
    let rig = s.find_object("Rig").unwrap();
    let arm = s.armature_of(rig).unwrap();
    assert_eq!(arm.bones.len(), 3);
    assert_eq!(arm.bones[2].parent, Some(1));
    assert_eq!(arm.bones[1].inherit_scale, InheritScale::None);
    assert_eq!(arm.collections, vec!["Deform".to_string(), "Empty".to_string()]);
    assert_eq!(arm.bones[0].collections, vec!["Deform".to_string()]);
    let prop = &s.objects[s.find_object("Prop").unwrap()];
    assert_eq!(prop.parent_bone, "Tip");
    let body = s.find_object("Body").unwrap();
    let ObjectData::Mesh(mi) = s.objects[body].data else { unreachable!() };
    let m = &s.meshes[mi];
    assert_eq!(m.vertex_groups, vec!["Lower".to_string(), "Upper".to_string()]);
    assert_eq!(m.weight_opt(1, 0), Some(0.5));
    assert_eq!(m.weight_opt(5, 1), Some(0.75));
    assert_eq!(m.shape_keys.len(), 3);
    assert_eq!(m.shape_keys[2].relative_key, 1);
    assert!(!m.shape_key_settings.use_relative);
    assert_eq!(s.objects[body].modifiers.len(), 9);
    let goal = &s.objects[s.find_object("Goal").unwrap()];
    assert_eq!(goal.constraints.len(), 15);
    assert_eq!(goal.constraints[13].name, "My Lock");
    // Every object evaluates (no panics on odd settings).
    for i in 0..s.objects.len() {
        let _ = s.world_matrix(i);
        let _ = s.evaluated_mesh(i);
    }
    let text = sex_3d::script::scene_to_polyops(&s);
    let mut back = Scene::new();
    run_ok(&mut back, &text);
    assert_eq!(back, s, "--- text ---\n{text}");
    assert_eq!(sex_3d::script::scene_to_polyops(&back), text);
}

#[test]
fn polyops_rig_errors() {
    for (src, want) in [
        ("bone Nope B", "no armature named"),
        ("armature_new A\nbone A B parent=Missing", "no bone named"),
        ("armature_new A\nbone A B\nbone A C parent=B\nbone A B parent=C", "cycle"),
        ("add_empty E\npose_bone E B", "not an armature"),
        ("add_empty E\nconstraint_add E wobble", "unknown constraint type"),
        ("add_empty E\nconstraint_add E track_to track_axis=sideways", "track_axis"),
        ("add_empty E\nconstraint_set E 3", "does not exist"),
        ("add_empty E\nconstraint_add E copy_location\nconstraint_target E 0 X", "Armature constraints"),
        ("add_primitive cube\nvgroup Cube G indices=99", "does not exist"),
        ("add_primitive cube\nshape_key Cube K verts=1,2", "verts="),
        ("add_primitive cube\nmodifier_add Cube Hook falloff_type=wobbly", "falloff_type"),
        ("lattice_new L points=0,2,2", "points="),
        ("add_empty E\nadd_empty F\nparent E F bone=B", "bone named"),
    ] {
        let mut s = Scene::startup();
        let e = sex_3d::script::run(&mut s, &format!("polyops 1\n{src}\n")).expect_err(src);
        assert!(e.message.contains(want), "{src}: {}", e.message);
    }
}

#[test]
fn polyops_parent_with_armature_and_set_inverse() {
    let mut s = Scene::new();
    run_ok(
        &mut s,
        "polyops 1
armature_new A
bone A Lower head=0,0,0 tail=0,0,1
bone A Upper head=0,0,1 tail=0,0,2 parent=Lower use_connect=1
add_armature Rig data=A
add_primitive cylinder name=Tube depth=2 location=0,0,1
parent_with_armature Tube Rig weights=envelope
add_empty T location=1,0,0
add_empty O
constraint_add O child_of target=T
constraint_set_inverse O 0
",
    );
    let t = s.find_object("Tube").unwrap();
    assert!(matches!(&s.objects[t].modifiers[0], Modifier::Armature { object, .. } if object == "Rig"));
    let o = s.find_object("O").unwrap();
    assert_near(s.world_matrix(o).translation_part(), Vec3::ZERO, 1e-5);
    // Renaming the target keeps the constraint pointing at it.
    run_ok(&mut s, "polyops 1\nset T.name Target\n");
    assert_eq!(s.objects[o].constraints[0].target().unwrap().object, "Target");
}

#[test]
fn mirror_modifier_flips_groups_and_keys() {
    let mut m = quad();
    for p in &mut m.positions {
        p.x += 1.0;
    }
    let l = m.add_vertex_group("Arm.L");
    m.add_vertex_group("Arm.R");
    for i in 0..4 {
        m.set_weight(i, l, 1.0);
    }
    let mut s = Scene::new();
    let o = s.add_mesh_object("M", m);
    s.objects[o].modifiers.push(Modifier::with_defaults("Mirror").unwrap());
    let out = s.evaluated_mesh(o).unwrap();
    assert_eq!(out.positions.len(), 8);
    for i in 4..8 {
        assert!(out.positions[i].x < 0.0);
        assert_eq!(out.weight_opt(i, 1), Some(1.0));
        assert_eq!(out.weight_opt(i, 0), None);
    }
}

#[test]
fn ops_carry_vertex_data() {
    // Subdividing an edge interpolates weights and keys; extruding copies them.
    let mut m = quad();
    let g = m.add_vertex_group("G");
    m.set_weight(0, g, 1.0);
    m.set_weight(1, g, 0.0);
    m.add_shape_key("Basis", false);
    let k = m.add_shape_key("K", false);
    m.shape_keys[k].positions = lifted(&m, 1.0);
    sex_3d::ops2::subdivide(&mut m, &[[0, 1]], 1);
    let nv = m.positions.len() - 1;
    assert_close(m.weight(nv, g), 0.5, 1e-6);
    assert_close(m.shape_keys[k].positions[nv].z, 1.0, 1e-6);
    assert!(m.shape_keys.iter().all(|k| k.positions.len() == m.positions.len()));
    let n = m.positions.len();
    sex_3d::ops::extrude_faces(&mut m, &[0], 0.5);
    assert!(m.positions.len() > n);
    assert!(m.shape_keys.iter().all(|k| k.positions.len() == m.positions.len()));
    assert_eq!(m.weights.len(), m.positions.len());
    // Vertex 0's copy carries its weight.
    let copies = (n..m.positions.len()).filter(|&i| m.weight_opt(i, g) == Some(1.0)).count();
    assert_eq!(copies, 1);
    // Deleting vertices keeps one entry per vertex.
    sex_3d::ops::delete_verts(&mut m, &[0]);
    assert!(m.shape_keys.iter().all(|k| k.positions.len() == m.positions.len()));
    assert!(m.weights.len() <= m.positions.len());
}
