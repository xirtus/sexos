// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::doc::{Doc, Mode};
use crate::input::{Button, Input, Key, Mods};
use crate::ui::Op;
use crate::App;
use sex_3d::Mat4;

fn run(d: &mut Doc, id: &str, arg: &str) {
    d.run(&Op::with(id, arg));
    let q = core::mem::take(&mut d.queue);
    for op in q {
        d.run(&op);
    }
}

fn cube(d: &Doc) -> usize {
    d.scene.find_object("Cube").expect("default cube")
}

/// The startup scene plus an armature with its one bone, active.
fn with_armature() -> (Doc, usize) {
    let mut d = Doc::default();
    run(&mut d, "object.armature_add", "");
    let a = d.active().unwrap();
    assert!(is_armature(&d, a));
    (d, a)
}

fn select_bone(d: &mut Doc, oi: usize, name: &str, extend: bool) {
    let obj = d.scene.objects[oi].name.clone();
    let ui = d.rig.arm_mut(&obj);
    if !extend {
        ui.clear();
    }
    ui.set(name, BoneSel::ALL);
    ui.active = Some(name.to_string());
}

fn bones(d: &Doc, oi: usize) -> Vec<sex_3d::armature::Bone> {
    d.scene.armature_of(oi).unwrap().bones.clone()
}

#[test]
fn armature_add_and_modes() {
    let (mut d, a) = with_armature();
    assert_eq!(bones(&d, a).len(), 1);
    assert_eq!(modes_for(&d, Some(a)), vec![Mode::Object, Mode::Edit, Mode::Pose]);
    run(&mut d, "object.mode_set", "TOGGLE_POSE");
    assert_eq!(d.mode, Mode::Pose);
    run(&mut d, "object.mode_set", "TOGGLE_EDIT");
    assert_eq!(d.mode, Mode::Edit);
    assert!(edit_armature(&d).is_some());
    run(&mut d, "object.mode_set", "OBJECT");
    assert_eq!(d.mode, Mode::Object);
}

#[test]
fn armature_edit_extrude_roll_parent() {
    let (mut d, a) = with_armature();
    run(&mut d, "object.mode_set", "EDIT");
    select_bone(&mut d, a, "Bone", false);
    // E: a connected child at the tip.
    run(&mut d, "armature.extrude_move", "");
    let b = bones(&d, a);
    assert_eq!(b.len(), 2);
    assert_eq!(b[1].parent, Some(0));
    assert!(b[1].use_connect);
    assert!((b[1].head - b[0].tail).length() < 1e-5);
    // Pull the new tip up, as the move after E would.
    let ai = armature_data(&d, a).unwrap();
    d.scene.armatures[ai].bones[1].tail = sex_3d::Vec3::new(0.0, 0.0, 2.0);
    // Ctrl+R: roll the selected bone.
    let (t, _) = xform::start(&d).expect("selected bone");
    t.roll(&mut d, 0.5);
    assert!((bones(&d, a)[1].roll - 0.5).abs() < 1e-5);
    run(&mut d, "armature.roll_clear", "");
    assert!(bones(&d, a)[1].roll.abs() < 1e-5);
    // Alt+P clears the parent, Ctrl+P (Keep Offset) sets it again.
    run(&mut d, "armature.parent_clear", "CLEAR");
    assert_eq!(bones(&d, a)[1].parent, None);
    let second = bones(&d, a)[1].name.clone();
    select_bone(&mut d, a, &second, false);
    select_bone(&mut d, a, "Bone", true);
    run(&mut d, "armature.parent_set", "OFFSET");
    let b = bones(&d, a);
    assert_eq!(b[1].parent, Some(0));
    assert!(!b[1].use_connect);
    // Shift+D, X and undo.
    select_bone(&mut d, a, &second, false);
    run(&mut d, "armature.duplicate_move", "");
    assert_eq!(bones(&d, a).len(), 3);
    run(&mut d, "armature.delete", "");
    assert_eq!(bones(&d, a).len(), 2);
    run(&mut d, "ed.undo", "");
    assert_eq!(bones(&d, a).len(), 3);
    run(&mut d, "ed.redo", "");
    assert_eq!(bones(&d, a).len(), 2);
    // Subdivide and symmetrize.
    select_bone(&mut d, a, "Bone", false);
    run(&mut d, "armature.subdivide", "");
    assert_eq!(bones(&d, a).len(), 3);
    run(&mut d, "object.mode_set", "OBJECT");
    assert_eq!(d.mode, Mode::Object);
}

#[test]
fn pose_transforms_and_clear() {
    let (mut d, a) = with_armature();
    run(&mut d, "object.mode_set", "POSE");
    select_bone(&mut d, a, "Bone", false);
    // Rotate 90 degrees about world X around the bone's head.
    let (t, _) = xform::start(&d).expect("pose bone");
    let rot = Mat4::from_trs(sex_3d::Vec3::ZERO, sex_3d::Quat::from_axis_angle(sex_3d::Vec3::X, core::f32::consts::FRAC_PI_2), sex_3d::Vec3::ONE);
    t.apply(&mut d, &|_| rot, false, false);
    let poses = d.scene.pose_matrices(a);
    let tip = poses[0].transform_point(sex_3d::Vec3::new(0.0, 1.0, 0.0));
    assert!((tip - sex_3d::Vec3::new(0.0, -1.0, 0.0)).length() < 1e-3, "tip {tip:?}");
    let pb = pose_bone(&d, a, "Bone");
    assert!(pb.rotation_quaternion.w < 0.99);
    // Move.
    let (t, _) = xform::start(&d).unwrap();
    t.apply(&mut d, &|_| Mat4::translation(sex_3d::Vec3::new(0.0, 0.0, 1.0)), false, true);
    assert!(pose_bone(&d, a, "Bone").location.length() > 0.5);
    // Alt+R / Alt+G / Alt+S.
    run(&mut d, "pose.rot_clear", "");
    assert!((pose_bone(&d, a, "Bone").rotation_quaternion.w - 1.0).abs() < 1e-5);
    run(&mut d, "pose.loc_clear", "");
    assert!(pose_bone(&d, a, "Bone").location.length() < 1e-6);
    // Copy, flipped paste on a mirrored pair.
    pose_bone_mut(&mut d, a, "Bone").location = sex_3d::Vec3::new(0.3, 0.0, 0.0);
    run(&mut d, "pose.copy", "");
    run(&mut d, "pose.transforms_clear", "");
    assert!(pose_bone(&d, a, "Bone").location.length() < 1e-6);
    run(&mut d, "pose.paste", "");
    assert!((pose_bone(&d, a, "Bone").location.x - 0.3).abs() < 1e-5);
    run(&mut d, "pose.paste", "FLIPPED");
    assert!((pose_bone(&d, a, "Bone").location.x + 0.3).abs() < 1e-5);
    // Rest Position shows the rest pose.
    run(&mut d, "rig.set", "pose_position=REST");
    let rest = d.scene.pose_matrices(a);
    assert!(rest[0].translation_part().length() < 1e-6);
}

#[test]
fn automatic_weights_parenting_deforms_the_mesh() {
    let (mut d, a) = with_armature();
    let c = cube(&d);
    d.scene.selected = vec![c, a];
    d.scene.active_object = Some(a);
    run(&mut d, "object.parent_set", "ARMATURE_AUTO");
    let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[c].data else { panic!() };
    assert_eq!(d.scene.objects[c].parent, Some(a));
    assert!(d.scene.meshes[mi].vertex_groups.iter().any(|g| g == "Bone"), "groups {:?}", d.scene.meshes[mi].vertex_groups);
    let g = d.scene.meshes[mi].vertex_groups.iter().position(|g| g == "Bone").unwrap();
    assert!((0..d.scene.meshes[mi].positions.len()).any(|v| d.scene.meshes[mi].weight(v, g) > 0.5));
    let before = d.scene.world_mesh(c).unwrap().0.positions;
    run(&mut d, "object.mode_set", "POSE");
    select_bone(&mut d, a, "Bone", false);
    let (t, _) = xform::start(&d).unwrap();
    let rot = Mat4::from_trs(sex_3d::Vec3::ZERO, sex_3d::Quat::from_axis_angle(sex_3d::Vec3::X, 1.0), sex_3d::Vec3::ONE);
    t.apply(&mut d, &|_| rot, false, false);
    let after = d.scene.world_mesh(c).unwrap().0.positions;
    let moved = before.iter().zip(&after).filter(|(p, q)| (**p - **q).length() > 1e-3).count();
    assert!(moved > 0, "posing the bone moves the skinned cube");
}

#[test]
fn bone_parent_and_envelope_and_empty_groups() {
    let (mut d, a) = with_armature();
    let c = cube(&d);
    d.scene.selected = vec![c, a];
    d.scene.active_object = Some(a);
    run(&mut d, "object.parent_set", "ARMATURE_NAME");
    let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[c].data else { panic!() };
    let g = d.scene.meshes[mi].vertex_groups.iter().position(|g| g == "Bone").unwrap_or_else(|| panic!("empty group: {} {:?}", d.report, d.scene.objects[c].parent));
    assert!((0..d.scene.meshes[mi].positions.len()).all(|v| d.scene.meshes[mi].weight_opt(v, g).is_none()));
    run(&mut d, "object.parent_clear", "CLEAR");
    assert_eq!(d.scene.objects[c].parent, None);
}

#[test]
fn weight_paint_stroke_changes_weights() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.select_only(Some(c));
    run(&mut d, "object.vertex_group_add", "");
    run(&mut d, "object.mode_set", "WEIGHT_PAINT");
    assert_eq!(d.mode, Mode::WeightPaint);
    d.rig.wp.weight = 1.0;
    d.rig.wp.strength = 1.0;
    let stroke = paint::begin(&mut d).expect("stroke");
    let n = d.scene.meshes[stroke.mi].positions.len();
    let hits: Vec<(usize, f32)> = (0..n).map(|v| (v, 1.0)).collect();
    stroke.dab(&mut d, &hits, &vec![None; n], (0.0, 0.0));
    let m = &d.scene.meshes[stroke.mi];
    assert!((0..n).all(|v| (m.weight(v, stroke.group) - 1.0).abs() < 1e-5));
    // Subtract half.
    d.rig.wp.blend = WpBlend::Subtract;
    d.rig.wp.weight = 0.5;
    let stroke = paint::begin(&mut d).unwrap();
    stroke.dab(&mut d, &hits[..1], &vec![None; n], (0.0, 0.0));
    assert!((d.scene.meshes[stroke.mi].weight(0, stroke.group) - 0.5).abs() < 1e-4);
    // One undo step per stroke.
    run(&mut d, "ed.undo", "");
    assert!((d.scene.meshes[stroke.mi].weight(0, stroke.group) - 1.0).abs() < 1e-5);
}

#[test]
fn weight_paint_stroke_in_the_viewport() {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    let c = cube(&app.doc);
    app.doc.select_only(Some(c));
    app.doc.run(&Op::with("object.mode_set", "WEIGHT_PAINT"));
    app.doc.rig.wp.radius = 400.0;
    let r = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::View3D).unwrap().rect;
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: x + 10, y });
    app.frame(Input::Up { x: x + 10, y, button: Button::Left });
    let sex_3d::scene::ObjectData::Mesh(mi) = app.doc.scene.objects[c].data else { panic!() };
    let m = &app.doc.scene.meshes[mi];
    assert_eq!(m.vertex_groups.len(), 1, "painting adds a group");
    assert!((0..m.positions.len()).any(|v| m.weight(v, 0) > 0.0));
    // F changes the radius from the mouse.
    app.frame(Input::Key { key: Key::Char('f'), mods: Mods::NONE, text: Some('f') });
    app.frame(Input::Move { x: x + 60, y });
    app.frame(Input::Down { x: x + 60, y, button: Button::Left, mods: Mods::NONE });
    assert!((app.doc.rig.wp.radius - 450.0).abs() < 1.0, "radius {}", app.doc.rig.wp.radius);
}

#[test]
fn shape_key_panel_ops_and_edit_mode() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.select_only(Some(c));
    let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[c].data else { panic!() };
    run(&mut d, "object.shape_key_add", "");
    run(&mut d, "object.shape_key_add", "");
    run(&mut d, "object.shape_key_add", "");
    assert_eq!(d.scene.meshes[mi].shape_keys.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Basis", "Key 1", "Key 2"]);
    assert_eq!(d.rig.active_shape("Cube"), 2);
    // Move up, set active.
    run(&mut d, "object.shape_key_move", "UP");
    assert_eq!(d.scene.meshes[mi].shape_keys[1].name, "Key 2");
    run(&mut d, "object.shape_key_set_active", "2");
    // Editing Key 1 in Edit Mode changes the key, not the Basis.
    let basis = d.scene.meshes[mi].positions.clone();
    run(&mut d, "object.mode_set", "EDIT");
    d.scene.meshes[mi].positions[0].z += 1.0;
    run(&mut d, "object.mode_set", "OBJECT");
    assert_eq!(d.scene.meshes[mi].positions, basis, "the Basis stays");
    assert!((d.scene.meshes[mi].shape_keys[2].positions[0].z - (basis[0].z + 1.0)).abs() < 1e-5);
    // The value mixes it in.
    d.scene.meshes[mi].shape_keys[2].value = 1.0;
    let shown = d.scene.world_mesh(c).unwrap().0.positions;
    let w = d.scene.world_matrix(c);
    assert!((shown[0] - w.transform_point(d.scene.meshes[mi].shape_keys[2].positions[0])).length() < 1e-4);
    run(&mut d, "object.shape_key_clear", "");
    assert!(d.scene.meshes[mi].shape_keys.iter().all(|k| k.value == 0.0));
    // New Shape from Mix, remove, remove all.
    run(&mut d, "object.shape_key_add", "FROM_MIX");
    assert_eq!(d.scene.meshes[mi].shape_keys.len(), 4);
    run(&mut d, "object.shape_key_remove", "");
    assert_eq!(d.scene.meshes[mi].shape_keys.len(), 3);
    run(&mut d, "ed.undo", "");
    assert_eq!(d.scene.meshes[mi].shape_keys.len(), 4);
    run(&mut d, "object.shape_key_remove", "ALL");
    assert!(d.scene.meshes[mi].shape_keys.is_empty());
}

#[test]
fn vertex_groups_assign_select_and_lock() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.select_only(Some(c));
    let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[c].data else { panic!() };
    run(&mut d, "object.vertex_group_add", "");
    run(&mut d, "object.vertex_group_add", "");
    assert_eq!(d.scene.meshes[mi].vertex_groups, ["Group", "Group.001"]);
    run(&mut d, "object.mode_set", "EDIT");
    // Everything is selected on entering Edit Mode: assign at 0.25.
    d.rig.wp.vg_weight = 0.25;
    run(&mut d, "object.vertex_group_assign", "");
    let g = d.rig.active_group("Cube");
    assert!((d.scene.meshes[mi].weight(0, g) - 0.25).abs() < 1e-5);
    run(&mut d, "object.mode_set", "OBJECT");
    run(&mut d, "object.vertex_group_lock", "");
    assert!(paint::is_locked(&d, c, "Group.001"));
    run(&mut d, "object.vertex_group_remove", "");
    assert_eq!(d.scene.meshes[mi].vertex_groups.len(), 1);
}

#[test]
fn constraints_add_and_bone_constraints() {
    let (mut d, a) = with_armature();
    let c = cube(&d);
    // Object constraint on the cube targeting the armature.
    d.scene.selected = vec![a, c];
    d.scene.active_object = Some(c);
    run(&mut d, "object.constraint_add", "COPY_LOCATION");
    assert_eq!(d.scene.objects[c].constraints.len(), 1);
    assert_eq!(d.scene.objects[c].constraints[0].target().unwrap().object, "Armature");
    run(&mut d, "constraint.set", "Cube\t\t0\tsubtarget=Bone");
    assert_eq!(d.scene.objects[c].constraints[0].target().unwrap().subtarget, "Bone");
    run(&mut d, "constraint.mute_toggle", "Cube\t\t0");
    assert!(d.scene.objects[c].constraints[0].mute);
    // A bone constraint in Pose Mode.
    d.select_only(Some(a));
    run(&mut d, "object.mode_set", "POSE");
    select_bone(&mut d, a, "Bone", false);
    run(&mut d, "pose.constraint_add", "IK");
    assert_eq!(pose_bone(&d, a, "Bone").constraints.len(), 1);
    run(&mut d, "constraint.delete", "Armature\tBone\t0");
    assert!(pose_bone(&d, a, "Bone").constraints.is_empty());
}

#[test]
fn bone_rename_carries_groups_and_keys() {
    let (mut d, a) = with_armature();
    let c = cube(&d);
    d.scene.selected = vec![c, a];
    d.scene.active_object = Some(a);
    run(&mut d, "object.parent_set", "ARMATURE_NAME");
    crate::anim::insert_key_value(&mut d, a, "pose.bones[\"Bone\"].location", 0, 1.0, 0.0);
    run(&mut d, "bone.set", "Armature\tBone\tname=Spine");
    assert_eq!(bones(&d, a)[0].name, "Spine");
    let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[c].data else { panic!() };
    assert!(d.scene.meshes[mi].vertex_groups.iter().any(|g| g == "Spine"));
    assert!(d.anim.curves("Armature").iter().any(|f| f.path == "pose.bones[\"Spine\"].location"));
}

#[test]
fn lattice_edit_and_data() {
    let mut d = Doc::default();
    run(&mut d, "object.add", "LATTICE");
    let l = d.active().unwrap();
    assert!(is_lattice(&d, l));
    run(&mut d, "lattice.resize", "3 2 2");
    let sex_3d::scene::ObjectData::Lattice(li) = d.scene.objects[l].data else { panic!() };
    assert_eq!(d.scene.lattices[li].points.len(), 12);
    run(&mut d, "lattice.interpolation", "U=KEY_LINEAR");
    assert_eq!(d.scene.lattices[li].interpolation_u, sex_3d::mesh::KeyInterpolation::Linear);
    run(&mut d, "object.mode_set", "EDIT");
    assert!(edit_lattice(&d).is_some());
    run(&mut d, "lattice.select_all", "SELECT");
    let (t, pts) = xform::start(&d).expect("lattice points");
    assert_eq!(pts.len(), 12);
    t.apply(&mut d, &|_| Mat4::translation(sex_3d::Vec3::new(0.0, 0.0, 1.0)), false, true);
    assert!((d.scene.lattices[li].points[0].z - (-0.5 + 1.0)).abs() < 1e-4);
    run(&mut d, "lattice.make_regular", "");
    assert!((d.scene.lattices[li].points[0].z + 0.5).abs() < 1e-4);
}

#[test]
fn ctrl_tab_enters_pose_mode_in_the_viewport() {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    app.doc.run(&Op::new("object.armature_add"));
    let r = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::View3D).unwrap().rect;
    app.frame(Input::Move { x: r.x + r.w / 2, y: r.y + r.h / 2 });
    app.frame(Input::Key { key: Key::Tab, mods: Mods::CTRL, text: None });
    assert_eq!(app.doc.mode, Mode::Pose);
    app.frame(Input::Key { key: Key::Tab, mods: Mods::CTRL, text: None });
    assert_eq!(app.doc.mode, Mode::Object);
}
