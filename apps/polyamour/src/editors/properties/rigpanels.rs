// SPDX-License-Identifier: GPL-3.0-only
//! Rigging panels of the Properties editor: the mesh's Vertex Groups and
//! Shape Keys lists, the Armature and Lattice data tabs, the Bone tab (edit
//! bone or pose channel), the Object and Bone Constraints stacks, and the
//! Weight Paint / Pose tool settings.
use super::icons::Icon;
use super::layout::{L, ROW};
use crate::doc::{Doc, Mode};
use crate::draw::Rect;
use crate::rig::{self, BoneDisplay, WpBlend, WpBrush};
use crate::ui::{MenuItem, NumOpts, Op};
use sex_3d::armature::RotationMode;
use sex_3d::constraints::{Constraint, ConstraintKind, Space, TrackAxis};
use sex_3d::scene::ObjectData;
use sex_3d::Vec3;

const M: NumOpts<'static> = NumOpts { step: 0.1, unit: "m", min: f32::NEG_INFINITY, max: f32::INFINITY, slider: false, int: false };
const PLAIN: NumOpts<'static> = NumOpts { step: 0.1, unit: "", min: f32::NEG_INFINITY, max: f32::INFINITY, slider: false, int: false };

fn q(d: &mut Doc, id: &str, arg: &str) {
    d.queue.push(Op::with(id, arg));
}

/// A row of equal buttons; returns the index clicked.
fn button_row(l: &mut L, labels: &[&str]) -> Option<usize> {
    if !l.band(ROW) {
        return None;
    }
    let (x, w) = l.inner();
    let n = labels.len() as i32;
    let bw = (w - (n - 1) * 2) / n;
    let mut hit = None;
    for (i, lab) in labels.iter().enumerate() {
        let r = Rect::new(x + i as i32 * (bw + 2), l.y, bw, ROW);
        l.fields.push((lab.to_string(), r));
        if !l.inactive && l.ui.button(r, lab) {
            hit = Some(i);
        }
    }
    l.y += ROW + 2;
    hit
}

// ---------------------------------------------------------------- mesh

pub fn vertex_groups(l: &mut L, d: &mut Doc, oi: usize, mi: usize) {
    let name = d.scene.objects[oi].name.clone();
    let groups = d.scene.meshes[mi].vertex_groups.clone();
    let active = d.rig.active_group(&name).min(groups.len().saturating_sub(1));
    let rows: Vec<(Icon, String)> = groups.iter().map(|g| (Icon::VertexGroup, if rig::paint::is_locked(d, oi, g) { format!("{g}   (locked)") } else { g.clone() })).collect();
    let (b, picked) = l.list_box(&rows, if groups.is_empty() { usize::MAX } else { active }, 3);
    match b {
        1 => q(d, "object.vertex_group_add", ""),
        2 => q(d, "object.vertex_group_remove", ""),
        _ => {}
    }
    if let Some(i) = picked {
        q(d, "object.vertex_group_set_active", &i.to_string());
    }
    if groups.is_empty() {
        return;
    }
    match button_row(l, &["Move Up", "Move Down", "Lock", "Copy"]) {
        Some(0) => q(d, "object.vertex_group_move", "UP"),
        Some(1) => q(d, "object.vertex_group_move", "DOWN"),
        Some(2) => q(d, "object.vertex_group_lock", &active.to_string()),
        Some(3) => q(d, "object.vertex_group_copy", ""),
        _ => {}
    }
    if d.mode == Mode::Edit && d.active() == Some(oi) {
        match button_row(l, &["Assign", "Remove"]) {
            Some(0) => q(d, "object.vertex_group_assign", ""),
            Some(1) => q(d, "object.vertex_group_remove_from", ""),
            _ => {}
        }
        match button_row(l, &["Select", "Deselect"]) {
            Some(0) => q(d, "object.vertex_group_select", ""),
            Some(1) => q(d, "object.vertex_group_deselect", ""),
            _ => {}
        }
        let mut w = d.rig.wp.vg_weight;
        if l.slider("Weight", "vg_weight", &mut w, 0.0, 1.0) {
            d.rig.wp.vg_weight = w;
        }
    }
}

pub fn shape_keys(l: &mut L, d: &mut Doc, oi: usize, mi: usize) {
    let name = d.scene.objects[oi].name.clone();
    let keys = d.scene.meshes[mi].shape_keys.clone();
    let active = d.rig.active_shape(&name).min(keys.len().saturating_sub(1));
    let rows: Vec<(Icon, String)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (Icon::ShapeKey, if i == 0 { k.name.clone() } else { format!("{}{}   {:.3}", k.name, if k.mute { "  (muted)" } else { "" }, k.value) }))
        .collect();
    let (b, picked) = l.list_box(&rows, if keys.is_empty() { usize::MAX } else { active }, 3);
    match b {
        1 => q(d, "object.shape_key_add", ""),
        2 => q(d, "object.shape_key_remove", ""),
        _ => {}
    }
    if let Some(i) = picked {
        q(d, "object.shape_key_set_active", &i.to_string());
    }
    if keys.is_empty() {
        return;
    }
    let locked = d.rig.shape_lock.contains(&name);
    let edit = d.rig.shape_edit_mode.contains(&name);
    match button_row(l, &["New Shape from Mix", "Clear Shape Keys"]) {
        Some(0) => q(d, "object.shape_key_add", "FROM_MIX"),
        Some(1) => q(d, "object.shape_key_clear", ""),
        _ => {}
    }
    match button_row(l, &["Move Up", "Move Down", if locked { "Unlock Shape" } else { "Lock Shape" }, if edit { "Edit Mode: On" } else { "Edit Mode: Off" }]) {
        Some(0) => q(d, "object.shape_key_move", "UP"),
        Some(1) => q(d, "object.shape_key_move", "DOWN"),
        Some(2) => q(d, "object.shape_key_lock", ""),
        Some(3) => q(d, "object.shape_key_edit_mode", ""),
        _ => {}
    }
    let mut rel = d.scene.meshes[mi].shape_key_settings.use_relative;
    if l.check("", "Relative", &mut rel) {
        l.commit(d, "Relative Shape Keys");
        d.scene.meshes[mi].shape_key_settings.use_relative = rel;
    }
    if !rel {
        let mut t = d.scene.meshes[mi].shape_key_settings.eval_time;
        if l.number("Evaluation Time", "sk_eval", &mut t, NumOpts { step: 1.0, ..PLAIN }) {
            l.commit(d, "Evaluation Time");
            d.scene.meshes[mi].shape_key_settings.eval_time = t;
        }
        return;
    }
    if active == 0 {
        return;
    }
    let k = keys[active].clone();
    let mut kname = k.name.clone();
    if l.text("Name", "sk_name", &mut kname) && !kname.trim().is_empty() && kname != k.name {
        l.commit(d, "Rename Shape Key");
        let n = sex_3d::scene::unique_name(kname.trim(), |n| d.scene.meshes[mi].shape_keys.iter().enumerate().any(|(i, x)| i != active && x.name == n));
        let old = core::mem::replace(&mut d.scene.meshes[mi].shape_keys[active].name, n.clone());
        let (op, np) = (format!("data.shape_keys.key_blocks[\"{old}\"]"), format!("data.shape_keys.key_blocks[\"{n}\"]"));
        for act in &mut d.anim.actions {
            for fc in &mut act.curves {
                if fc.path.starts_with(&op) {
                    fc.path = fc.path.replacen(&op, &np, 1);
                }
            }
        }
    }
    let mut v = k.value;
    l.bind(d, "sk_value", &name, &format!("data.shape_keys.key_blocks[\"{}\"].value", k.name), 0);
    if l.slider("Value", "sk_value", &mut v, k.slider_min, k.slider_max) {
        l.commit(d, "Shape Key Value");
        d.scene.meshes[mi].shape_keys[active].value = v;
    }
    let mut lo = k.slider_min;
    if l.number("Range Min", "sk_min", &mut lo, NumOpts { step: 0.05, min: -10.0, max: 10.0, ..PLAIN }) {
        l.commit(d, "Range");
        let sk = &mut d.scene.meshes[mi].shape_keys[active];
        sk.slider_min = lo.min(sk.slider_max - 1e-3);
    }
    let mut hi = k.slider_max;
    if l.number("Max", "sk_max", &mut hi, NumOpts { step: 0.05, min: -10.0, max: 10.0, ..PLAIN }) {
        l.commit(d, "Range");
        let sk = &mut d.scene.meshes[mi].shape_keys[active];
        sk.slider_max = hi.max(sk.slider_min + 1e-3);
    }
    let groups = d.scene.meshes[mi].vertex_groups.clone();
    let gcur = k.vertex_group.clone().unwrap_or_default();
    let set = move |key: &str, v: &str| Op::with("properties.shape_key_set", &format!("{active}\t{key}={v}"));
    let gitems = {
        let mut v = vec![MenuItem::op("(None)", "", set("vertex_group", ""))];
        v.extend(groups.iter().map(|g| MenuItem::op(g, "", set("vertex_group", g))));
        v
    };
    l.dropdown_row("Vertex Group", "sk_vg", if gcur.is_empty() { "(None)" } else { &gcur }, &|| gitems.clone());
    let names: Vec<String> = keys.iter().map(|k| k.name.clone()).collect();
    let ritems: Vec<MenuItem> = names.iter().enumerate().filter(|(i, _)| *i != active).map(|(i, n)| MenuItem::op(n, "", set("relative_key", &i.to_string()))).collect();
    l.dropdown_row("Relative To", "sk_rel", names.get(k.relative_key).map_or("", |s| s.as_str()), &|| ritems.clone());
    let mut mute = k.mute;
    if l.check("", "Mute", &mut mute) {
        l.commit(d, "Mute Shape Key");
        d.scene.meshes[mi].shape_keys[active].mute = mute;
    }
}

/// `properties.shape_key_set "index\tkey=value"` (vertex group, relative key).
pub fn shape_key_set(d: &mut Doc, oi: usize, arg: &str) {
    let ObjectData::Mesh(mi) = d.scene.objects[oi].data else { return };
    let Some((i, kv)) = arg.split_once('\t') else { return };
    let (Ok(i), Some((k, v))) = (i.parse::<usize>(), kv.split_once('=')) else { return };
    if i >= d.scene.meshes[mi].shape_keys.len() {
        return;
    }
    d.push_undo("Shape Key");
    let sk = &mut d.scene.meshes[mi].shape_keys[i];
    match k {
        "vertex_group" => sk.vertex_group = (!v.is_empty()).then(|| v.to_string()),
        "relative_key" => sk.relative_key = v.parse().unwrap_or(sk.relative_key),
        _ => {}
    }
    d.touch();
}

// ---------------------------------------------------------------- armature and lattice

pub fn armature(l: &mut L, d: &mut Doc, oi: usize, ai: usize) {
    let name = d.scene.objects[oi].name.clone();
    let aname = d.scene.armature_names.get(ai).cloned().unwrap_or_default();
    l.id_block(Icon::ArmatureData, &aname, None);
    l.panel(d, "arm.pose", "Pose", true, &mut |l, d| {
        let rest = d.scene.armatures[ai].rest_position;
        if let Some(i) = l.expand("", &["Pose Position", "Rest Position"], rest as usize) {
            q(d, "rig.set", if i == 1 { "pose_position=REST" } else { "pose_position=POSE" });
        }
    });
    l.panel(d, "arm.collections", "Bone Collections", true, &mut |l, d| {
        let ui = d.rig.arm(&name);
        let cols = d.scene.armatures[ai].collections.clone();
        let rows: Vec<(Icon, String)> = cols.iter().map(|c| (Icon::Bone, if ui.hidden_collections.contains(c) { format!("{c}   (hidden)") } else { c.clone() })).collect();
        let editing = rig::bone_object(d) == Some(oi);
        let (b, picked) = l.list_box(&rows, if cols.is_empty() { usize::MAX } else { ui.active_collection }, 3);
        if !editing && (b != 0) {
            d.report = "Bone collections are edited in Edit Mode or Pose Mode".into();
        } else {
            match b {
                1 => q(d, "armature.collection_add", ""),
                2 => q(d, "armature.collection_remove", ""),
                _ => {}
            }
        }
        if let Some(i) = picked {
            d.rig.arm_mut(&name).active_collection = i;
        }
        if editing && !cols.is_empty() {
            match button_row(l, &["Assign", "Remove", "Show / Hide"]) {
                Some(0) => q(d, "armature.collection_assign", ""),
                Some(1) => q(d, "armature.collection_unassign", ""),
                Some(2) => q(d, "armature.collection_hide", &ui.active_collection.to_string()),
                _ => {}
            }
        }
    });
    l.panel(d, "arm.display", "Viewport Display", true, &mut |l, d| {
        let ui = d.rig.arm(&name);
        let items: Vec<MenuItem> = BoneDisplay::ALL.iter().map(|b| MenuItem::op(b.label(), "", Op::with("rig.set", &format!("display_type={}", b.id())))).collect();
        l.dropdown_row("Display As", "display_type", ui.display.label(), &|| items.clone());
        for (label, key, v) in [("Names", "show_names", ui.show_names), ("Axes", "show_axes", ui.show_axes), ("In Front", "show_in_front", ui.show_in_front)] {
            let mut x = v;
            if l.check(if key == "show_names" { "Show" } else { "" }, label, &mut x) {
                q(d, "rig.set", &format!("{key}={}", x as u8));
            }
        }
    });
}

pub fn lattice(l: &mut L, d: &mut Doc, li: usize) {
    let lname = d.scene.lattice_names.get(li).cloned().unwrap_or_default();
    l.id_block(Icon::LatticeData, &lname, None);
    l.panel(d, "lat.lattice", "Lattice", true, &mut |l, d| {
        let lat = d.scene.lattices[li].clone();
        let res = [lat.points_u, lat.points_v, lat.points_w];
        let interp = [lat.interpolation_u, lat.interpolation_v, lat.interpolation_w];
        for (i, axis) in ["U", "V", "W"].iter().enumerate() {
            let mut v = res[i];
            if l.uint(&format!("Resolution {axis}"), &format!("res{axis}"), &mut v, 1, 64) && v != res[i] {
                let mut r = res;
                r[i] = v;
                q(d, "lattice.resize", &format!("{} {} {}", r[0], r[1], r[2]));
            }
            let a = axis.to_string();
            let items: Vec<MenuItem> = [("Linear", "KEY_LINEAR"), ("Cardinal", "KEY_CARDINAL"), ("Catmull-Rom", "KEY_CATMULL_ROM"), ("BSpline", "KEY_BSPLINE")].iter().map(|(lab, id)| MenuItem::op(lab, "", Op::with("lattice.interpolation", &format!("{a}={id}")))).collect();
            l.dropdown_row("", &format!("interp{axis}"), rig::interp_label(interp[i]), &|| items.clone());
            l.space(4);
        }
        let mut out = lat.use_outside;
        if l.check("", "Outside", &mut out) {
            l.commit(d, "Outside");
            d.scene.lattices[li].use_outside = out;
        }
    });
}

// ---------------------------------------------------------------- bone

/// The Bone tab: the active bone in Edit Mode, else its pose channel.
pub fn bone(l: &mut L, d: &mut Doc, oi: usize) {
    let Some(ai) = rig::armature_data(d, oi) else { return };
    let obj = d.scene.objects[oi].name.clone();
    let Some(bname) = rig::active_bone(d, oi) else {
        l.note("No active bone");
        return;
    };
    let Some(bi) = d.scene.armatures[ai].find(&bname) else { return };
    let mut nm = bname.clone();
    if l.text("", "bone_name", &mut nm) && nm != bname {
        q(d, "bone.set", &format!("{obj}\t{bname}\tname={nm}"));
    }
    let edit = rig::edit_armature(d).is_some_and(|e| e.0 == oi);
    let deg = d.settings.degrees;
    let (unit, k) = if deg { ("°", 180.0 / core::f32::consts::PI) } else { ("", 1.0) };
    let ang = NumOpts { step: if deg { 1.0 } else { 0.02 }, unit, ..PLAIN };
    if edit {
        l.panel(d, "bone.transform", "Transform", true, &mut |l, d| {
            let b = d.scene.armatures[ai].bones[bi].clone();
            for (part, label, v) in [(0, "Head X", b.head), (1, "Tail X", b.tail)] {
                for (i, lab) in [label, "Y", "Z"].iter().enumerate() {
                    let mut x = v.to_array()[i];
                    if l.number(lab, &format!("{}{i}", if part == 0 { "head" } else { "tail" }), &mut x, M) {
                        l.commit(d, "Move Bone");
                        let a = &mut d.scene.armatures[ai];
                        let mut arr = v.to_array();
                        arr[i] = x;
                        let p = Vec3::from_array(arr);
                        if part == 0 {
                            a.bones[bi].head = p;
                            if let (true, Some(par)) = (a.bones[bi].use_connect, a.bones[bi].parent) {
                                a.bones[par].tail = p;
                            }
                        } else {
                            a.bones[bi].tail = p;
                            for c in 0..a.bones.len() {
                                if a.bones[c].parent == Some(bi) && a.bones[c].use_connect {
                                    a.bones[c].head = p;
                                }
                            }
                        }
                    }
                }
                l.space(4);
            }
            let mut r = b.roll * k;
            if l.number("Roll", "roll", &mut r, ang) {
                l.commit(d, "Roll");
                d.scene.armatures[ai].bones[bi].roll = r / k;
            }
            let mut len = b.length();
            if l.number("Length", "length", &mut len, NumOpts { step: 0.05, min: 1e-3, ..M }) {
                l.commit(d, "Bone Length");
                let a = &mut d.scene.armatures[ai];
                let dir = (b.tail - b.head).normalize();
                a.bones[bi].tail = b.head + dir * len;
            }
        });
    } else {
        l.panel(d, "bone.transform", "Transform", true, &mut |l, d| {
            let pb = rig::pose_bone(d, oi, &bname);
            let base = format!("pose.bones[\"{bname}\"]");
            for (i, lab) in ["Location X", "Y", "Z"].iter().enumerate() {
                let mut v = pb.location.to_array()[i];
                l.bind(d, &format!("ploc{i}"), &obj, &format!("{base}.location"), i);
                if l.number(lab, &format!("ploc{i}"), &mut v, M) {
                    l.commit(d, "Location");
                    let p = rig::pose_bone_mut(d, oi, &bname);
                    let mut a = p.location.to_array();
                    a[i] = v;
                    p.location = Vec3::from_array(a);
                }
            }
            l.space(4);
            match pb.rotation_mode {
                RotationMode::Quaternion => {
                    let qv = pb.rotation_quaternion;
                    let a = [qv.w, qv.x, qv.y, qv.z];
                    for (i, lab) in ["Rotation W", "X", "Y", "Z"].iter().enumerate() {
                        let mut v = a[i];
                        l.bind(d, &format!("pquat{i}"), &obj, &format!("{base}.rotation_quaternion"), i);
                        if l.number(lab, &format!("pquat{i}"), &mut v, NumOpts { step: 0.01, ..PLAIN }) {
                            l.commit(d, "Rotation");
                            let mut a2 = a;
                            a2[i] = v;
                            rig::pose_bone_mut(d, oi, &bname).rotation_quaternion = sex_3d::Quat { w: a2[0], x: a2[1], y: a2[2], z: a2[3] };
                        }
                    }
                }
                RotationMode::AxisAngle => {
                    let a = pb.rotation_axis_angle;
                    for (i, lab) in ["Rotation W", "X", "Y", "Z"].iter().enumerate() {
                        let mut v = if i == 0 { a[0] * k } else { a[i] };
                        l.bind(d, &format!("paa{i}"), &obj, &format!("{base}.rotation_axis_angle"), i);
                        if l.number(lab, &format!("paa{i}"), &mut v, if i == 0 { ang } else { NumOpts { step: 0.01, ..PLAIN } }) {
                            l.commit(d, "Rotation");
                            rig::pose_bone_mut(d, oi, &bname).rotation_axis_angle[i] = if i == 0 { v / k } else { v };
                        }
                    }
                }
                RotationMode::Euler(_) => {
                    let e = pb.rotation_euler.to_array();
                    for (i, lab) in ["Rotation X", "Y", "Z"].iter().enumerate() {
                        let mut v = e[i] * k;
                        l.bind(d, &format!("prot{i}"), &obj, &format!("{base}.rotation_euler"), i);
                        if l.number(lab, &format!("prot{i}"), &mut v, ang) {
                            l.commit(d, "Rotation");
                            let mut e2 = e;
                            e2[i] = v / k;
                            rig::pose_bone_mut(d, oi, &bname).rotation_euler = Vec3::from_array(e2);
                        }
                    }
                }
            }
            let modes = [("QUATERNION", "Quaternion (WXYZ)"), ("XYZ", "XYZ Euler"), ("XZY", "XZY Euler"), ("YXZ", "YXZ Euler"), ("YZX", "YZX Euler"), ("ZXY", "ZXY Euler"), ("ZYX", "ZYX Euler"), ("AXIS_ANGLE", "Axis Angle")];
            let cur = match pb.rotation_mode {
                RotationMode::Quaternion => "Quaternion (WXYZ)".to_string(),
                RotationMode::AxisAngle => "Axis Angle".to_string(),
                RotationMode::Euler(o) => format!("{o:?} Euler"),
            };
            let items: Vec<MenuItem> = modes.iter().map(|(id, lab)| MenuItem::op(lab, "", Op::with("pose.rotation_mode_set", id))).collect();
            l.dropdown_row("Mode", "prot_mode", &cur, &|| items.clone());
            l.space(4);
            for (i, lab) in ["Scale X", "Y", "Z"].iter().enumerate() {
                let mut v = pb.scale.to_array()[i];
                l.bind(d, &format!("pscale{i}"), &obj, &format!("{base}.scale"), i);
                if l.number(lab, &format!("pscale{i}"), &mut v, NumOpts { step: 0.05, ..PLAIN }) {
                    l.commit(d, "Scale");
                    let p = rig::pose_bone_mut(d, oi, &bname);
                    let mut a = p.scale.to_array();
                    a[i] = v;
                    p.scale = Vec3::from_array(a);
                }
            }
        });
    }
    l.panel(d, "bone.relations", "Relations", true, &mut |l, d| {
        let a = &d.scene.armatures[ai];
        let b = a.bones[bi].clone();
        let parent = b.parent.map(|p| a.bones[p].name.clone()).unwrap_or_default();
        if edit {
            let mut items = vec![MenuItem::op("(None)", "", Op::with("bone.set", &format!("{obj}\t{bname}\tparent=")))];
            items.extend(a.bones.iter().enumerate().filter(|(i, _)| *i != bi && !a.is_ancestor(bi, *i)).map(|(_, x)| MenuItem::op(&x.name, "", Op::with("bone.set", &format!("{obj}\t{bname}\tparent={}", x.name)))));
            l.dropdown_row("Parent", "bone_parent", if parent.is_empty() { "(None)" } else { &parent }, &|| items.clone());
            let mut c = b.use_connect;
            if l.check("", "Connected", &mut c) {
                q(d, "bone.set", &format!("{obj}\t{bname}\tuse_connect={}", c as u8));
            }
        } else {
            l.info("Parent", if parent.is_empty() { "(None)" } else { &parent });
        }
        let mut r = b.use_inherit_rotation;
        if l.check("Inherit", "Rotation", &mut r) {
            q(d, "bone.set", &format!("{obj}\t{bname}\tuse_inherit_rotation={}", r as u8));
        }
        let mut ll = b.use_local_location;
        if l.check("", "Local Location", &mut ll) {
            q(d, "bone.set", &format!("{obj}\t{bname}\tuse_local_location={}", ll as u8));
        }
    });
    l.panel_with(
        d,
        "bone.deform",
        "Deform",
        true,
        &mut |l, d, hr| {
            let mut on = d.scene.armatures[ai].bones[bi].use_deform;
            let r = Rect::new(hr.x, hr.y, 18, hr.h);
            if l.ui.checkbox(r, "", &mut on) {
                q(d, "bone.set", &format!("{obj}\t{bname}\tuse_deform={}", on as u8));
            }
        },
        &mut |l, d| {
            let b = d.scene.armatures[ai].bones[bi].clone();
            let mut dist = b.envelope_distance;
            if l.number("Envelope Distance", "env_dist", &mut dist, NumOpts { step: 0.01, min: 0.0, ..M }) {
                l.commit(d, "Envelope");
                d.scene.armatures[ai].bones[bi].envelope_distance = dist;
            }
            let mut w = b.envelope_weight;
            if l.number("Envelope Weight", "env_w", &mut w, NumOpts { step: 0.05, min: 0.0, ..PLAIN }) {
                l.commit(d, "Envelope");
                d.scene.armatures[ai].bones[bi].envelope_weight = w;
            }
            let mut mul = b.use_envelope_multiply;
            if l.check("", "Envelope Multiply", &mut mul) {
                l.commit(d, "Envelope");
                d.scene.armatures[ai].bones[bi].use_envelope_multiply = mul;
            }
            let mut hr = b.head_radius;
            if l.number("Radius Head", "rad_head", &mut hr, NumOpts { step: 0.01, min: 0.0, ..M }) {
                l.commit(d, "Radius");
                d.scene.armatures[ai].bones[bi].head_radius = hr;
            }
            let mut tr = b.tail_radius;
            if l.number("Tail", "rad_tail", &mut tr, NumOpts { step: 0.01, min: 0.0, ..M }) {
                l.commit(d, "Radius");
                d.scene.armatures[ai].bones[bi].tail_radius = tr;
            }
        },
    );
    if !edit {
        l.panel(d, "bone.ik", "Inverse Kinematics", false, &mut |l, d| {
            let ik = rig::pose_bone(d, oi, &bname).ik;
            for (i, ax) in ["X", "Y", "Z"].iter().enumerate() {
                let mut lock = ik.lock[i];
                if l.check(if i == 0 { "Lock IK" } else { "" }, ax, &mut lock) {
                    l.commit(d, "Lock IK");
                    rig::pose_bone_mut(d, oi, &bname).ik.lock[i] = lock;
                }
            }
            let mut st = ik.stretch;
            if l.slider("IK Stretch", "ik_stretch", &mut st, 0.0, 1.0) {
                l.commit(d, "IK Stretch");
                rig::pose_bone_mut(d, oi, &bname).ik.stretch = st;
            }
        });
    }
}

// ---------------------------------------------------------------- constraints

fn space_label(s: Space) -> &'static str {
    match s {
        Space::World => "World Space",
        Space::Pose => "Pose Space",
        Space::LocalWithParent => "Local With Parent",
        Space::Local => "Local Space",
        Space::Custom => "Custom Space",
    }
}

fn axis_label(a: TrackAxis) -> &'static str {
    ["X", "Y", "Z", "-X", "-Y", "-Z"][a.index()]
}

/// The constraint stack of `obj` (or of its bone `bone`) with the Add menu.
pub fn constraints(l: &mut L, d: &mut Doc, oi: usize, bone: Option<String>) {
    let obj = d.scene.objects[oi].name.clone();
    let b = bone.clone().unwrap_or_default();
    if bone.is_some() && b.is_empty() {
        l.note("No active bone");
        return;
    }
    l.space(4);
    if l.band(ROW) {
        let (x, w) = l.inner();
        let r = Rect::new(x, l.y, w, ROW);
        l.fields.push(("add_constraint".into(), r));
        if l.ui.dropdown(r, if bone.is_some() { "Add Bone Constraint" } else { "Add Object Constraint" }) {
            l.ui.open_menu(r, "", rig::cons::add_menu(bone.is_some()));
        }
        l.y += ROW + 6;
    }
    let stack = rig::cons::stack(d, &obj, &b).cloned().unwrap_or_default();
    let n = stack.len();
    for (i, c) in stack.iter().enumerate() {
        let addr = rig::cons::addr(&obj, &b, i);
        l.panel_with(
            d,
            &format!("con.{obj}.{b}.{i}"),
            &c.name,
            true,
            &mut |l, d, hr| {
                let mut x = hr.right();
                for (label, id, on) in [("X", "constraint.delete", true), ("v", "constraint.move_down", i + 1 < n), ("^", "constraint.move_up", i > 0), (if c.mute { "o" } else { "*" }, "constraint.mute_toggle", true)] {
                    x -= 22;
                    let r = Rect::new(x, hr.y, 20, hr.h);
                    if on && l.ui.button(r, label) {
                        q(d, id, &addr);
                    }
                }
            },
            &mut |l, d| constraint_body(l, d, &obj, &b, i, c),
        );
    }
}

fn set_op(obj: &str, bone: &str, i: usize, kv: &str) -> Op {
    Op::with("constraint.set", &format!("{obj}\t{bone}\t{i}\t{kv}"))
}

fn edit(l: &mut L, d: &mut Doc, obj: &str, bone: &str, i: usize, f: impl FnOnce(&mut Constraint)) {
    l.commit(d, "Change Constraint");
    if let Some(c) = rig::cons::stack_mut(d, obj, bone).and_then(|s| s.get_mut(i)) {
        f(c);
    }
    d.touch();
}

fn object_items(d: &Doc, obj: &str, bone: &str, i: usize, key: &str) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::op(super::object::NO_OBJECT, "", set_op(obj, bone, i, &format!("{key}=")))];
    v.extend(d.scene.objects.iter().filter(|o| !bone.is_empty() || o.name != obj).map(|o| MenuItem::op(&o.name, "", set_op(obj, bone, i, &format!("{key}={}", o.name)))));
    v
}

fn bone_items(d: &Doc, target: &str, obj: &str, bone: &str, i: usize, key: &str) -> Option<Vec<MenuItem>> {
    let ti = d.scene.find_object(target)?;
    let a = d.scene.armature_of(ti)?;
    let mut v = vec![MenuItem::op("(None)", "", set_op(obj, bone, i, &format!("{key}=")))];
    v.extend(a.bones.iter().filter(|b| !(target == obj && b.name == bone)).map(|b| MenuItem::op(&b.name, "", set_op(obj, bone, i, &format!("{key}={}", b.name)))));
    Some(v)
}

fn xyz_checks(l: &mut L, d: &mut Doc, heading: &str, v: [bool; 3], set: &dyn Fn(&mut Constraint, usize, bool), obj: &str, bone: &str, i: usize) {
    for (ax, lab) in ["X", "Y", "Z"].iter().enumerate() {
        let mut x = v[ax];
        if l.check(if ax == 0 { heading } else { "" }, lab, &mut x) {
            edit(l, d, obj, bone, i, |c| set(c, ax, x));
        }
    }
}

fn constraint_body(l: &mut L, d: &mut Doc, obj: &str, bone: &str, i: usize, c: &Constraint) {
    use ConstraintKind as K;
    if let Some(t) = c.target() {
        let items = object_items(d, obj, bone, i, "target");
        l.dropdown_row("Target", "c_target", if t.object.is_empty() { super::object::NO_OBJECT } else { &t.object }, &|| items.clone());
        if let Some(bi) = bone_items(d, &t.object, obj, bone, i, "subtarget") {
            l.dropdown_row("Bone", "c_sub", if t.subtarget.is_empty() { "(None)" } else { &t.subtarget }, &|| bi.clone());
        }
    }
    match &c.kind {
        K::CopyLocation { use_xyz, invert, use_offset, .. } => {
            xyz_checks(l, d, "Axis", *use_xyz, &|c, a, v| if let K::CopyLocation { use_xyz, .. } = &mut c.kind { use_xyz[a] = v }, obj, bone, i);
            xyz_checks(l, d, "Invert", *invert, &|c, a, v| if let K::CopyLocation { invert, .. } = &mut c.kind { invert[a] = v }, obj, bone, i);
            let mut o = *use_offset;
            if l.check("", "Offset", &mut o) {
                edit(l, d, obj, bone, i, |c| if let K::CopyLocation { use_offset, .. } = &mut c.kind { *use_offset = o });
            }
        }
        K::CopyRotation { use_xyz, invert, .. } => {
            xyz_checks(l, d, "Axis", *use_xyz, &|c, a, v| if let K::CopyRotation { use_xyz, .. } = &mut c.kind { use_xyz[a] = v }, obj, bone, i);
            xyz_checks(l, d, "Invert", *invert, &|c, a, v| if let K::CopyRotation { invert, .. } = &mut c.kind { invert[a] = v }, obj, bone, i);
        }
        K::CopyScale { use_xyz, power, use_offset, use_add, .. } => {
            xyz_checks(l, d, "Axis", *use_xyz, &|c, a, v| if let K::CopyScale { use_xyz, .. } = &mut c.kind { use_xyz[a] = v }, obj, bone, i);
            let mut p = *power;
            if l.number("Power", "c_power", &mut p, PLAIN) {
                edit(l, d, obj, bone, i, |c| if let K::CopyScale { power, .. } = &mut c.kind { *power = p });
            }
            let (mut o, mut a) = (*use_offset, *use_add);
            if l.check("", "Offset", &mut o) {
                edit(l, d, obj, bone, i, |c| if let K::CopyScale { use_offset, .. } = &mut c.kind { *use_offset = o });
            }
            if l.check("", "Additive", &mut a) {
                edit(l, d, obj, bone, i, |c| if let K::CopyScale { use_add, .. } = &mut c.kind { *use_add = a });
            }
        }
        K::LimitDistance { distance, .. } => {
            let mut v = *distance;
            if l.number("Distance", "c_dist", &mut v, NumOpts { min: 0.0, ..M }) {
                edit(l, d, obj, bone, i, |c| if let K::LimitDistance { distance, .. } = &mut c.kind { *distance = v });
            }
        }
        K::LimitLocation { use_min, min, use_max, max } | K::LimitScale { use_min, min, use_max, max } => {
            let is_loc = matches!(c.kind, K::LimitLocation { .. });
            for (ax, axn) in ["X", "Y", "Z"].iter().enumerate() {
                let mut on = use_min[ax];
                if l.check("", &format!("Minimum {axn}"), &mut on) {
                    edit(l, d, obj, bone, i, |c| if let K::LimitLocation { use_min, .. } | K::LimitScale { use_min, .. } = &mut c.kind { use_min[ax] = on });
                }
                if use_min[ax] {
                    let mut v = min[ax];
                    if l.number("", &format!("c_min{ax}"), &mut v, if is_loc { M } else { PLAIN }) {
                        edit(l, d, obj, bone, i, |c| if let K::LimitLocation { min, .. } | K::LimitScale { min, .. } = &mut c.kind { min[ax] = v });
                    }
                }
                let mut on = use_max[ax];
                if l.check("", &format!("Maximum {axn}"), &mut on) {
                    edit(l, d, obj, bone, i, |c| if let K::LimitLocation { use_max, .. } | K::LimitScale { use_max, .. } = &mut c.kind { use_max[ax] = on });
                }
                if use_max[ax] {
                    let mut v = max[ax];
                    if l.number("", &format!("c_max{ax}"), &mut v, if is_loc { M } else { PLAIN }) {
                        edit(l, d, obj, bone, i, |c| if let K::LimitLocation { max, .. } | K::LimitScale { max, .. } = &mut c.kind { max[ax] = v });
                    }
                }
            }
        }
        K::LimitRotation { use_limit, min, max, .. } => {
            let k = 180.0 / core::f32::consts::PI;
            for (ax, axn) in ["X", "Y", "Z"].iter().enumerate() {
                let mut on = use_limit[ax];
                if l.check("", &format!("Limit {axn}"), &mut on) {
                    edit(l, d, obj, bone, i, |c| if let K::LimitRotation { use_limit, .. } = &mut c.kind { use_limit[ax] = on });
                }
                if use_limit[ax] {
                    let mut lo = min[ax] * k;
                    if l.number("Min", &format!("c_rmin{ax}"), &mut lo, NumOpts { step: 1.0, unit: "°", ..PLAIN }) {
                        edit(l, d, obj, bone, i, |c| if let K::LimitRotation { min, .. } = &mut c.kind { min[ax] = lo / k });
                    }
                    let mut hi = max[ax] * k;
                    if l.number("Max", &format!("c_rmax{ax}"), &mut hi, NumOpts { step: 1.0, unit: "°", ..PLAIN }) {
                        edit(l, d, obj, bone, i, |c| if let K::LimitRotation { max, .. } = &mut c.kind { max[ax] = hi / k });
                    }
                }
            }
        }
        K::MaintainVolume { volume, .. } => {
            let mut v = *volume;
            if l.number("Volume", "c_vol", &mut v, NumOpts { step: 0.05, min: 0.001, ..PLAIN }) {
                edit(l, d, obj, bone, i, |c| if let K::MaintainVolume { volume, .. } = &mut c.kind { *volume = v });
            }
        }
        K::ChildOf { use_location, use_rotation, use_scale, .. } => {
            xyz_checks(l, d, "Location", *use_location, &|c, a, v| if let K::ChildOf { use_location, .. } = &mut c.kind { use_location[a] = v }, obj, bone, i);
            xyz_checks(l, d, "Rotation", *use_rotation, &|c, a, v| if let K::ChildOf { use_rotation, .. } = &mut c.kind { use_rotation[a] = v }, obj, bone, i);
            xyz_checks(l, d, "Scale", *use_scale, &|c, a, v| if let K::ChildOf { use_scale, .. } = &mut c.kind { use_scale[a] = v }, obj, bone, i);
            match button_row(l, &["Set Inverse", "Clear Inverse"]) {
                Some(0) => q(d, "constraint.childof_set_inverse", &rig::cons::addr(obj, bone, i)),
                Some(1) => q(d, "constraint.childof_clear_inverse", &rig::cons::addr(obj, bone, i)),
                _ => {}
            }
        }
        K::TrackTo { track_axis, up_axis, use_target_z, .. } => {
            let items: Vec<MenuItem> = TrackAxis::NAMES.iter().map(|(id, a)| MenuItem::op(axis_label(*a), "", set_op(obj, bone, i, &format!("track_axis={id}")))).collect();
            l.dropdown_row("Track Axis", "c_track", axis_label(*track_axis), &|| items.clone());
            let ups: Vec<MenuItem> = ["X", "Y", "Z"].iter().enumerate().map(|(n, a)| MenuItem::op(a, "", set_op(obj, bone, i, &format!("up_axis={n}")))).collect();
            l.dropdown_row("Up", "c_up", ["X", "Y", "Z"][(*up_axis as usize).min(2)], &|| ups.clone());
            let mut z = *use_target_z;
            if l.check("", "Target Z", &mut z) {
                edit(l, d, obj, bone, i, |c| if let K::TrackTo { use_target_z, .. } = &mut c.kind { *use_target_z = z });
            }
        }
        K::DampedTrack { track_axis, .. } | K::LockedTrack { track_axis, .. } => {
            let items: Vec<MenuItem> = TrackAxis::NAMES.iter().map(|(id, a)| MenuItem::op(axis_label(*a), "", set_op(obj, bone, i, &format!("track_axis={id}")))).collect();
            l.dropdown_row("Track Axis", "c_track", axis_label(*track_axis), &|| items.clone());
            if let K::LockedTrack { lock_axis, .. } = &c.kind {
                let ups: Vec<MenuItem> = ["X", "Y", "Z"].iter().enumerate().map(|(n, a)| MenuItem::op(a, "", set_op(obj, bone, i, &format!("lock_axis={n}")))).collect();
                l.dropdown_row("Lock", "c_lock", ["X", "Y", "Z"][(*lock_axis as usize).min(2)], &|| ups.clone());
            }
        }
        K::StretchTo { rest_length, bulge, .. } => {
            let mut r = *rest_length;
            if l.number("Original Length", "c_rest", &mut r, NumOpts { min: 0.0, ..M }) {
                edit(l, d, obj, bone, i, |c| if let K::StretchTo { rest_length, .. } = &mut c.kind { *rest_length = r });
            }
            let mut b = *bulge;
            if l.number("Volume Variation", "c_bulge", &mut b, NumOpts { step: 0.05, min: 0.0, ..PLAIN }) {
                edit(l, d, obj, bone, i, |c| if let K::StretchTo { bulge, .. } = &mut c.kind { *bulge = b });
            }
        }
        K::Floor { floor_location, offset, use_rotation, .. } => {
            let items: Vec<MenuItem> = TrackAxis::NAMES.iter().map(|(id, a)| MenuItem::op(axis_label(*a), "", set_op(obj, bone, i, &format!("track_axis={id}")))).collect();
            l.dropdown_row("Floor Location", "c_floor", axis_label(*floor_location), &|| items.clone());
            let mut o = *offset;
            if l.number("Offset", "c_off", &mut o, M) {
                edit(l, d, obj, bone, i, |c| if let K::Floor { offset, .. } = &mut c.kind { *offset = o });
            }
            let mut r = *use_rotation;
            if l.check("", "Use Rotation", &mut r) {
                edit(l, d, obj, bone, i, |c| if let K::Floor { use_rotation, .. } = &mut c.kind { *use_rotation = r });
            }
        }
        K::Armature { targets, .. } => {
            for (t, w) in targets {
                l.info(&t.subtarget, &format!("{} ({w:.2})", t.object));
            }
            if targets.is_empty() {
                l.note("No bone targets");
            }
        }
        K::Ik { pole_target, pole_angle, chain_count, iterations, use_tail, use_stretch, weight, .. } => {
            let items = object_items(d, obj, bone, i, "pole_target");
            let pole = pole_target.as_ref().map(|p| p.object.clone()).unwrap_or_default();
            l.dropdown_row("Pole Target", "c_pole", if pole.is_empty() { super::object::NO_OBJECT } else { &pole }, &|| items.clone());
            if let Some(bi) = (!pole.is_empty()).then(|| bone_items(d, &pole, obj, bone, i, "pole_subtarget")).flatten() {
                let cur = pole_target.as_ref().map(|p| p.subtarget.clone()).unwrap_or_default();
                l.dropdown_row("Bone", "c_polesub", if cur.is_empty() { "(None)" } else { &cur }, &|| bi.clone());
            }
            if pole_target.is_some() {
                let k = 180.0 / core::f32::consts::PI;
                let mut a = *pole_angle * k;
                if l.number("Pole Angle", "c_pangle", &mut a, NumOpts { step: 1.0, unit: "°", ..PLAIN }) {
                    edit(l, d, obj, bone, i, |c| if let K::Ik { pole_angle, .. } = &mut c.kind { *pole_angle = a / k });
                }
            }
            let mut it = *iterations;
            if l.uint("Iterations", "c_iter", &mut it, 1, 10000) {
                edit(l, d, obj, bone, i, |c| if let K::Ik { iterations, .. } = &mut c.kind { *iterations = it });
            }
            let mut ch = *chain_count;
            if l.uint("Chain Length", "c_chain", &mut ch, 0, 255) {
                edit(l, d, obj, bone, i, |c| if let K::Ik { chain_count, .. } = &mut c.kind { *chain_count = ch });
            }
            let (mut t, mut s) = (*use_tail, *use_stretch);
            if l.check("", "Use Tail", &mut t) {
                edit(l, d, obj, bone, i, |c| if let K::Ik { use_tail, .. } = &mut c.kind { *use_tail = t });
            }
            if l.check("", "Stretch", &mut s) {
                edit(l, d, obj, bone, i, |c| if let K::Ik { use_stretch, .. } = &mut c.kind { *use_stretch = s });
            }
            let mut w = *weight;
            if l.slider("Weight Position", "c_weight", &mut w, 0.01, 1.0) {
                edit(l, d, obj, bone, i, |c| if let K::Ik { weight, .. } = &mut c.kind { *weight = w });
            }
        }
        K::CopyTransforms { .. } | K::Transformation { .. } => {}
    }
    if c.target().is_some() && !matches!(c.kind, K::Ik { .. }) {
        let sp = |key: &str| -> Vec<MenuItem> { Space::NAMES.iter().filter(|(_, s)| *s != Space::Custom).map(|(id, s)| MenuItem::op(space_label(*s), "", set_op(obj, bone, i, &format!("{key}={id}")))).collect() };
        let (ti, oi2) = (sp("target_space"), sp("owner_space"));
        l.dropdown_row("Target", "c_tspace", space_label(c.target_space), &|| ti.clone());
        l.dropdown_row("Owner", "c_ospace", space_label(c.owner_space), &|| oi2.clone());
    }
    let mut inf = c.influence;
    let path = if bone.is_empty() { format!("constraints[\"{}\"].influence", c.name) } else { format!("pose.bones[\"{bone}\"].constraints[\"{}\"].influence", c.name) };
    l.bind(d, "c_influence", obj, &path, 0);
    if l.slider("Influence", "c_influence", &mut inf, 0.0, 1.0) {
        edit(l, d, obj, bone, i, |c| c.influence = inf);
    }
}

// ---------------------------------------------------------------- tool settings

/// The Tool tab in Weight Paint and Pose Mode.
pub fn tool(l: &mut L, d: &mut Doc) -> bool {
    if d.mode == Mode::WeightPaint {
        l.panel(d, "tool.wp.brush", "Brush Settings", true, &mut |l, d| {
            let wp = d.rig.wp;
            let brushes: Vec<MenuItem> = [WpBrush::Draw, WpBrush::Blur, WpBrush::Average, WpBrush::Smear, WpBrush::Gradient].iter().map(|b| MenuItem::op(b.label(), "", Op::with("rig.set", &format!("wp.brush={}", b.label())))).collect();
            l.dropdown_row("Brush", "wp_brush", wp.brush.label(), &|| brushes.clone());
            let blends: Vec<MenuItem> = [WpBlend::Mix, WpBlend::Add, WpBlend::Subtract].iter().map(|b| MenuItem::op(b.label(), "", Op::with("rig.set", &format!("wp.blend={}", b.label())))).collect();
            l.dropdown_row("Blend", "wp_blend", wp.blend.label(), &|| blends.clone());
            let mut w = wp.weight;
            if l.slider("Weight", "wp_weight", &mut w, 0.0, 1.0) {
                d.rig.wp.weight = w;
            }
            let mut r = wp.radius;
            if l.number("Radius", "wp_radius", &mut r, NumOpts { step: 1.0, unit: " px", min: 1.0, max: 500.0, ..PLAIN }) {
                d.rig.wp.radius = r;
            }
            let mut s = wp.strength;
            if l.slider("Strength", "wp_strength", &mut s, 0.0, 1.0) {
                d.rig.wp.strength = s;
            }
        });
        l.panel(d, "tool.wp.options", "Options", true, &mut |l, d| {
            let mut an = d.rig.wp.auto_normalize;
            if l.check("", "Auto Normalize", &mut an) {
                d.rig.wp.auto_normalize = an;
            }
            let mut xm = d.rig.wp.x_mirror;
            if l.check("Mirror", "X", &mut xm) {
                d.rig.wp.x_mirror = xm;
            }
        });
        return true;
    }
    if d.mode == Mode::Pose {
        l.panel(d, "tool.pose", "Pose Options", true, &mut |l, d| {
            let mut ik = d.rig.auto_ik;
            if l.check("", "Auto IK", &mut ik) {
                d.rig.auto_ik = ik;
            }
        });
        return true;
    }
    false
}
