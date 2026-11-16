// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::doc::{Doc, Mode};
use crate::input::{Input, Key, Mods};
use crate::ui::Op;
use crate::App;
use sex_3d::Vec3;

fn run(d: &mut Doc, id: &str, arg: &str) {
    d.run(&Op::with(id, arg));
    let q = core::mem::take(&mut d.queue);
    for op in q {
        d.run(&op);
    }
}

fn at(d: &mut Doc, frame: i32) {
    d.frame = frame;
    evaluate(d);
}

fn cube(d: &Doc) -> usize {
    d.scene.find_object("Cube").expect("default cube")
}

fn armature(d: &mut Doc) -> usize {
    run(d, "object.armature_add", "");
    d.active().unwrap()
}

#[test]
fn data_path_keys_on_bones() {
    let mut d = Doc::default();
    let a = armature(&mut d);
    let p = "pose.bones[\"Bone\"].location";
    assert!(insert_key_value(&mut d, a, p, 0, 1.0, 0.0));
    assert!(insert_key_value(&mut d, a, p, 0, 11.0, 2.0));
    assert!(insert_key_value(&mut d, a, "pose.bones[\"Bone\"].rotation_quaternion", 0, 1.0, 1.0));
    let c = &d.anim.curves("Armature")[0];
    assert_eq!(c.group, "Bone", "bone curves group under the bone");
    at(&mut d, 6);
    let x = crate::rig::pose_bone(&d, a, "Bone").location.x;
    assert!((x - 1.0).abs() < 0.05, "x {x}");
    at(&mut d, 11);
    assert!((crate::rig::pose_bone(&d, a, "Bone").location.x - 2.0).abs() < 1e-4);
    // Pose Mode I keys the selected bones' channels.
    d.select_only(Some(a));
    run(&mut d, "object.mode_set", "POSE");
    let obj = d.scene.objects[a].name.clone();
    d.rig.arm_mut(&obj).set("Bone", crate::rig::BoneSel::ALL);
    d.frame = 20;
    crate::rig::pose_bone_mut(&mut d, a, "Bone").scale = Vec3::splat(2.0);
    run(&mut d, "anim.keyframe_insert", "");
    assert!(d.anim.curves("Armature").iter().any(|c| c.path == "pose.bones[\"Bone\"].scale" && c.keys.iter().any(|k| k.frame == 20.0 && (k.value - 2.0).abs() < 1e-5)));
}

#[test]
fn data_path_keys_on_shape_keys_lights_and_constraints() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.select_only(Some(c));
    run(&mut d, "object.shape_key_add", "");
    run(&mut d, "object.shape_key_add", "");
    let p = "data.shape_keys.key_blocks[\"Key 1\"].value";
    assert!(insert_key_value(&mut d, c, p, 0, 1.0, 0.0));
    assert!(insert_key_value(&mut d, c, p, 0, 21.0, 1.0));
    at(&mut d, 11);
    let sex_3d::scene::ObjectData::Mesh(mi) = d.scene.objects[c].data else { panic!() };
    let v = d.scene.meshes[mi].shape_keys[1].value;
    assert!(v > 0.3 && v < 0.7, "shape key value {v}");
    // The Shape Key Editor's row shows those keys.
    assert_eq!(d.anim.curves("Cube")[0].group, "Key");
    // Light energy.
    let light = d.scene.objects.iter().position(|o| matches!(o.data, sex_3d::scene::ObjectData::Light(_))).unwrap();
    assert!(insert_key_value(&mut d, light, "data.energy", 0, 1.0, 10.0));
    assert!(insert_key_value(&mut d, light, "data.energy", 0, 21.0, 30.0));
    at(&mut d, 21);
    assert!((path::get(&d.scene, light, "data.energy", 0).unwrap() - 30.0).abs() < 1e-3);
    // Constraint influence.
    run(&mut d, "object.constraint_add", "COPY_LOCATION");
    assert!(insert_key_value(&mut d, c, "constraints[\"Copy Location\"].influence", 0, 1.0, 0.25));
    at(&mut d, 1);
    assert!((d.scene.objects[c].constraints[0].influence - 0.25).abs() < 1e-5);
    // Field colours.
    assert_eq!(d.anim.field_state("Cube", p, 0, 1), FieldState::Keyed);
    assert_eq!(d.anim.field_state("Cube", p, 0, 5), FieldState::Animated);
    assert_eq!(d.anim.field_state("Cube", "location", 2, 5), FieldState::None);
}

#[test]
fn auto_key_inserts_on_transform_confirm() {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    let c = cube(&app.doc);
    app.doc.select_only(Some(c));
    app.doc.anim.auto_key = true;
    app.doc.frame = 5;
    let r = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::View3D).unwrap().rect;
    app.frame(Input::Move { x: r.x + r.w / 2, y: r.y + r.h / 2 });
    for (k, t) in [(Key::Char('g'), Some('g')), (Key::Char('x'), Some('x')), (Key::Char('2'), Some('2')), (Key::Enter, None)] {
        app.frame(Input::Key { key: k, mods: Mods::NONE, text: t });
    }
    assert!((app.doc.scene.objects[c].transform.location.x - 2.0).abs() < 1e-4);
    let curves = app.doc.anim.curves("Cube");
    assert!(curves.iter().any(|f| f.path == "location" && f.index == 0 && f.keys.iter().any(|k| k.frame == 5.0 && (k.value - 2.0).abs() < 1e-4)), "auto-key wrote location");
    // Off: no new keys.
    app.doc.anim.auto_key = false;
    app.doc.frame = 9;
    for (k, t) in [(Key::Char('g'), Some('g')), (Key::Char('x'), Some('x')), (Key::Char('1'), Some('1')), (Key::Enter, None)] {
        app.frame(Input::Key { key: k, mods: Mods::NONE, text: t });
    }
    assert!(!app.doc.anim.key_frames("Cube").contains(&9.0));
}

#[test]
fn drivers_evaluate_expressions_and_variables() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.scene.objects[c].transform.location = Vec3::new(1.5, 0.0, 0.0);
    // Ctrl+D on Location Z.
    run(&mut d, "anim.driver_button_add", "Cube\tlocation\t2");
    let o = d.anim.objects.iter().position(|x| x.object == "Cube").unwrap();
    assert_eq!(d.anim.objects[o].drivers.len(), 1);
    assert_eq!(d.anim.field_state("Cube", "location", 2, 1), FieldState::Driven);
    run(&mut d, "anim.prop_set", &format!("drv:{o}:0\texpression=var * 2 + sin(0)"));
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:0\tpath=location[0]"));
    at(&mut d, 1);
    let z = d.scene.objects[c].transform.location.z;
    assert!((z - 3.0).abs() < 1e-4, "driven z {z}, error {:?}", d.anim.objects[o].drivers[0].error);
    // Transform Channel variable from the camera, with a second variable.
    let cam = d.scene.objects.iter().position(|x| matches!(x.data, sex_3d::scene::ObjectData::Camera(_))).unwrap();
    let cam_y = d.scene.objects[cam].transform.location.y;
    run(&mut d, "anim.driver_variable_add", &format!("drv:{o}:0"));
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:1\ttype=TRANSFORMS"));
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:1\tobject=Camera"));
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:1\tchannel=LOC_Y"));
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:1\tname=cy"));
    run(&mut d, "anim.prop_set", &format!("drv:{o}:0\texpression=max(var, cy) + frame * 0"));
    at(&mut d, 1);
    let z = d.scene.objects[c].transform.location.z;
    assert!((z - 1.5f32.max(cam_y)).abs() < 1e-3, "z {z} cam_y {cam_y} {:?}", d.anim.objects[o].drivers[0].error);
    // Distance between the cube and the camera.
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:1\ttype=LOC_DIFF"));
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:1\tt0.object=Light"));
    run(&mut d, "anim.prop_set", &format!("var:{o}:0:1\tt1.object=Camera"));
    run(&mut d, "anim.prop_set", &format!("drv:{o}:0\texpression=cy"));
    at(&mut d, 1);
    let light = d.scene.find_object("Light").unwrap();
    let want = d.scene.world_matrix(light).translation_part().distance(d.scene.world_matrix(cam).translation_part());
    assert!((d.scene.objects[c].transform.location.z - want).abs() < 1e-2);
    // Averaged variables.
    run(&mut d, "anim.prop_set", &format!("drv:{o}:0\ttype=SUM"));
    at(&mut d, 1);
    assert!(d.anim.objects[o].drivers[0].error.is_none());
    // A bad expression reports an error.
    run(&mut d, "anim.prop_set", &format!("drv:{o}:0\ttype=SCRIPTED"));
    run(&mut d, "anim.prop_set", &format!("drv:{o}:0\texpression=var +"));
    at(&mut d, 1);
    assert!(d.anim.objects[o].drivers[0].error.is_some());
}

#[test]
fn expression_evaluator() {
    let vars = |n: &str| match n {
        "a" => Some(2.0),
        "b" => Some(3.0),
        _ => None,
    };
    let e = |s: &str| expr::eval(s, &vars).unwrap();
    assert_eq!(e("a + b * 2"), 8.0);
    assert_eq!(e("(a + b) * 2"), 10.0);
    assert_eq!(e("-a ** 2"), -4.0);
    assert!((e("sin(pi / 2)") - 1.0).abs() < 1e-6);
    assert_eq!(e("max(a, b, 1)"), 3.0);
    assert_eq!(e("a if b > 2 else 0"), 2.0);
    assert!(expr::eval("a +", &vars).is_err());
    assert!(expr::eval("nope", &vars).is_err());
}

#[test]
fn nla_blending_push_down_and_tweak() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.select_only(Some(c));
    for f in [1.0, 21.0] {
        insert_key_value(&mut d, c, "location", 0, f, 2.0);
    }
    run(&mut d, "action.push_down", "Cube");
    for f in [1.0, 21.0] {
        insert_key_value(&mut d, c, "location", 0, f, 4.0);
    }
    run(&mut d, "action.push_down", "Cube");
    let oa = d.anim.get("Cube").unwrap();
    assert_eq!(oa.tracks.len(), 2);
    assert!(oa.action.is_none());
    at(&mut d, 10);
    assert!((d.scene.objects[c].transform.location.x - 4.0).abs() < 1e-4, "upper Replace strip wins");
    // Add blending on the top strip.
    run(&mut d, "anim.prop_set", "strip:0:1:0\tblend_type=ADD");
    at(&mut d, 10);
    assert!((d.scene.objects[c].transform.location.x - 6.0).abs() < 1e-4, "x {}", d.scene.objects[c].transform.location.x);
    // Replace at half influence mixes.
    run(&mut d, "anim.prop_set", "strip:0:1:0\tblend_type=REPLACE");
    {
        let st = &mut d.anim.objects[0].tracks[1].strips[0];
        st.use_influence = true;
        st.influence = 0.5;
    }
    at(&mut d, 10);
    assert!((d.scene.objects[c].transform.location.x - 3.0).abs() < 1e-4);
    // A muted track drops out.
    d.anim.objects[0].tracks[1].mute = true;
    at(&mut d, 10);
    assert!((d.scene.objects[c].transform.location.x - 2.0).abs() < 1e-4);
    // Tweak mode edits the strip's action.
    d.anim.objects[0].tracks[1].mute = false;
    run(&mut d, "nla.select_all", "DESELECT");
    d.anim.objects[0].tracks[0].strips[0].select = true;
    run(&mut d, "nla.tweakmode_enter", "");
    assert_eq!(d.anim.objects[0].tweak, Some((0, 0)));
    assert_eq!(d.anim.edit_action("Cube"), Some(d.anim.objects[0].tracks[0].strips[0].action));
    run(&mut d, "nla.tweakmode_exit", "");
    assert_eq!(d.anim.objects[0].tweak, None);
    // Split and delete.
    d.frame = 11;
    run(&mut d, "nla.split", "");
    assert_eq!(d.anim.objects[0].tracks[0].strips.len(), 2);
    run(&mut d, "nla.delete", "");
    assert_eq!(d.anim.objects[0].tracks[0].strips.len(), 1);
}

#[test]
fn keyframe_editing_ops_and_undo() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.select_only(Some(c));
    run(&mut d, "anim.keyframe_insert", "");
    d.frame = 10;
    d.scene.objects[c].transform.location.x = 3.0;
    run(&mut d, "anim.keyframe_insert", "");
    assert_eq!(d.anim.key_frames("Cube"), vec![1.0, 10.0]);
    run(&mut d, "ed.undo", "");
    assert_eq!(d.anim.key_frames("Cube"), vec![1.0]);
    run(&mut d, "ed.redo", "");
    assert_eq!(d.anim.key_frames("Cube"), vec![1.0, 10.0]);
    // Select all, set interpolation and handle types, delete.
    run(&mut d, "graph.select_all", "SELECT");
    run(&mut d, "graph.interpolation_type", "LINEAR");
    assert!(d.anim.curves("Cube").iter().all(|f| f.keys.iter().all(|k| k.interp == Interp::Linear)));
    d.frame = 4;
    evaluate(&mut d);
    let x = d.scene.objects[c].transform.location.x;
    assert!((x - 1.0).abs() < 1e-4, "linear x {x}");
    run(&mut d, "graph.extrapolation_type", "LINEAR");
    assert!(d.anim.curves("Cube").iter().all(|f| f.extrapolation == Extrapolation::Linear));
    run(&mut d, "graph.fmodifier_add", "CYCLES");
    assert!(d.anim.curves("Cube").iter().all(|f| f.modifiers.len() == 1));
    run(&mut d, "action.duplicate", "");
    run(&mut d, "action.delete", "");
    assert_eq!(d.anim.key_frames("Cube"), vec![1.0, 10.0]);
    run(&mut d, "action.select_all", "SELECT");
    run(&mut d, "action.delete", "");
    assert!(d.anim.key_frames("Cube").is_empty());
    // Keying set: only Location.
    run(&mut d, "anim.keying_set_active_set", "Location");
    run(&mut d, "anim.keyframe_insert", "");
    assert!(d.anim.curves("Cube").iter().all(|f| f.path == "location"));
    assert_eq!(d.mode, Mode::Object);
}

#[test]
fn poly_round_trip_keeps_animation_and_rigs() {
    let mut d = Doc::default();
    let c = cube(&d);
    let a = armature(&mut d);
    d.scene.selected = vec![c, a];
    d.scene.active_object = Some(a);
    run(&mut d, "object.parent_set", "ARMATURE_AUTO");
    // Edit Mode: a second bone.
    run(&mut d, "object.mode_set", "EDIT");
    d.rig.arm_mut("Armature").set("Bone", crate::rig::BoneSel::ALL);
    run(&mut d, "armature.extrude_move", "");
    let ai = crate::rig::armature_data(&d, a).unwrap();
    d.scene.armatures[ai].bones[1].tail = Vec3::new(0.0, 0.0, 2.0);
    run(&mut d, "object.mode_set", "POSE");
    crate::rig::pose_bone_mut(&mut d, a, "Bone").location = Vec3::new(0.1, 0.2, 0.3);
    insert_key_value(&mut d, a, "pose.bones[\"Bone\"].rotation_quaternion", 1, 1.0, 0.5);
    insert_key_value(&mut d, a, "pose.bones[\"Bone\"].rotation_quaternion", 1, 9.0, 0.0);
    d.rig.arm_mut("Armature").display = crate::rig::BoneDisplay::Stick;
    d.rig.arm_mut("Armature").clear();
    d.rig.arm_mut("Armature").set("Bone", crate::rig::BoneSel::ALL);
    d.rig.arm_mut("Armature").active = Some("Bone".into());
    run(&mut d, "pose.constraint_add", "COPY_ROTATION");
    run(&mut d, "action.push_down", "Armature");
    run(&mut d, "object.mode_set", "OBJECT");
    d.select_only(Some(c));
    run(&mut d, "object.shape_key_add", "");
    run(&mut d, "object.shape_key_add", "");
    run(&mut d, "anim.driver_button_add", "Cube\tdata.shape_keys.key_blocks[\"Key 1\"].value\t0");
    run(&mut d, "object.add", "LATTICE");
    assert_eq!(d.anim.get("Armature").unwrap().tracks.len(), 1, "{}", d.report);
    let text = crate::persist::save(&d);
    let e = crate::persist::load(&text).expect("load");
    let a2 = e.scene.find_object("Armature").unwrap();
    assert_eq!(e.scene.armature_of(a2).unwrap().bones.len(), 2);
    assert_eq!(e.scene.armature_of(a2).unwrap().bones[1].parent, Some(0));
    assert_eq!(crate::rig::pose_bone(&e, a2, "Bone").location, Vec3::new(0.1, 0.2, 0.3));
    assert_eq!(crate::rig::pose_bone(&e, a2, "Bone").constraints.len(), 1);
    assert_eq!(e.rig.arm("Armature").display, crate::rig::BoneDisplay::Stick);
    let oa = e.anim.get("Armature").unwrap();
    assert_eq!(oa.tracks.len(), 1);
    assert_eq!(e.anim.actions[oa.tracks[0].strips[0].action].curves[0].path, "pose.bones[\"Bone\"].rotation_quaternion");
    let c2 = e.scene.find_object("Cube").unwrap();
    let sex_3d::scene::ObjectData::Mesh(mi) = e.scene.objects[c2].data else { panic!() };
    assert_eq!(e.scene.meshes[mi].shape_keys.len(), 2);
    assert!(e.scene.meshes[mi].vertex_groups.iter().any(|g| g == "Bone"));
    assert_eq!(e.anim.get("Cube").unwrap().drivers.len(), 1);
    assert!(e.scene.objects.iter().any(|o| matches!(o.data, sex_3d::scene::ObjectData::Lattice(_))));
    // Saving again gives the same text.
    assert_eq!(crate::persist::save(&e), text);
}

#[test]
fn legacy_key_lines_still_load() {
    let d = Doc::default();
    let mut text = crate::persist::save(&d);
    text.push_str("#@key Cube\tlocation\t0\t1\t0\tBEZIER\t0\t0\t2\t0\n#@key Cube\tlocation\t0\t11\t5\tBEZIER\t9\t5\t12\t5\n");
    let mut e = crate::persist::load(&text).expect("load");
    assert_eq!(e.anim.key_frames("Cube"), vec![1.0, 11.0]);
    e.frame = 11;
    evaluate(&mut e);
    let c = cube(&e);
    assert!((e.scene.objects[c].transform.location.x - 5.0).abs() < 1e-4);
}
