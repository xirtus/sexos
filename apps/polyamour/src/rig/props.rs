// SPDX-License-Identifier: GPL-3.0-only
//! The text-valued properties the Bone and constraint panels change from
//! menus: `bone.set "object\tbone\tkey=value"` (name, parent, use_connect,
//! use_deform) and `constraint.set "object\tbone\tindex\tkey=value"`
//! (target, subtarget, pole_target, pole_subtarget, owner_space,
//! target_space, track_axis, up_axis, lock_axis).
use crate::doc::Doc;
use crate::ui::Op;
use sex_3d::constraints::{ConstraintKind, Space, Target, TrackAxis};

pub fn run(doc: &mut Doc, op: &Op) -> bool {
    match op.id.as_str() {
        "bone.set" => bone_set(doc, &op.arg),
        "constraint.set" => constraint_set(doc, &op.arg),
        _ => return false,
    }
    true
}

/// Rename bone `old` of armature object `oi` (unique names), carrying pose
/// channels, vertex groups of the meshes it deforms, constraint and driver
/// targets, animation paths and the selection along.
pub fn rename_bone(doc: &mut Doc, oi: usize, old: &str, want: &str) -> Option<String> {
    let ai = super::armature_data(doc, oi)?;
    let bi = doc.scene.armatures[ai].find(old)?;
    let want = want.trim();
    if want.is_empty() || want == old {
        return None;
    }
    let new = sex_3d::scene::unique_name(want, |n| doc.scene.armatures[ai].bones.iter().enumerate().any(|(i, b)| i != bi && b.name == n));
    let obj = doc.scene.objects[oi].name.clone();
    doc.scene.armatures[ai].bones[bi].name = new.clone();
    // Every object using this armature data shares the bone.
    let users: Vec<usize> = (0..doc.scene.objects.len()).filter(|&o| super::armature_data(doc, o) == Some(ai)).collect();
    for &u in &users {
        for pb in &mut doc.scene.objects[u].pose {
            if pb.name == old {
                pb.name = new.clone();
            }
        }
        let uname = doc.scene.objects[u].name.clone();
        doc.rig.rename_bone(&uname, old, &new);
        doc.anim.rename_bone(&uname, old, &new);
    }
    let user_names: Vec<String> = users.iter().map(|&u| doc.scene.objects[u].name.clone()).collect();
    for o in 0..doc.scene.objects.len() {
        // Vertex groups of meshes deformed by (or parented to) the armature.
        if let Some(a) = super::deform_armature(doc, o) {
            if users.contains(&a) {
                if let sex_3d::scene::ObjectData::Mesh(mi) = doc.scene.objects[o].data {
                    for g in &mut doc.scene.meshes[mi].vertex_groups {
                        if g == old {
                            *g = new.clone();
                        }
                    }
                }
            }
        }
        let ob = &mut doc.scene.objects[o];
        let fix = |t: &mut Target| {
            if user_names.contains(&t.object) && t.subtarget == old {
                t.subtarget = new.clone();
            }
        };
        for c in ob.constraints.iter_mut().chain(ob.pose.iter_mut().flat_map(|p| p.constraints.iter_mut())) {
            if let Some(t) = c.target_mut() {
                fix(t);
            }
            if let ConstraintKind::Ik { pole_target: Some(t), .. } = &mut c.kind {
                fix(t);
            }
        }
        if ob.parent_bone == old && ob.parent.is_some_and(|p| users.contains(&p)) {
            ob.parent_bone = new.clone();
        }
    }
    for oa in &mut doc.anim.objects {
        for drv in &mut oa.drivers {
            for v in &mut drv.vars {
                for t in &mut v.targets {
                    if user_names.contains(&t.object) && t.bone == old {
                        t.bone = new.clone();
                    }
                }
            }
        }
    }
    let _ = obj;
    doc.touch();
    Some(new)
}

fn bone_set(doc: &mut Doc, arg: &str) {
    let mut it = arg.splitn(3, '\t');
    let (Some(obj), Some(bone), Some(kv)) = (it.next(), it.next(), it.next()) else { return };
    let Some((key, value)) = kv.split_once('=') else { return };
    let Some(oi) = doc.scene.find_object(obj) else { return };
    let Some(ai) = super::armature_data(doc, oi) else { return };
    let Some(bi) = doc.scene.armatures[ai].find(bone) else { return };
    let flag = |cur: bool| match value {
        "TOGGLE" => !cur,
        v => v == "1" || v == "true",
    };
    match key {
        "name" => {
            doc.push_undo("Rename Bone");
            rename_bone(doc, oi, bone, value);
        }
        "parent" => {
            let a = &doc.scene.armatures[ai];
            let p = if value.is_empty() { None } else { a.find(value) };
            if p.is_some_and(|p| p == bi || a.is_ancestor(bi, p)) {
                doc.report = "Cannot parent a bone to itself or its children".into();
                return;
            }
            doc.push_undo("Bone Parent");
            let a = &mut doc.scene.armatures[ai];
            a.bones[bi].parent = p;
            if p.is_none() {
                a.bones[bi].use_connect = false;
            } else if a.bones[bi].use_connect {
                let t = a.bones[p.unwrap_or(0)].tail;
                a.bones[bi].head = t;
            }
        }
        "use_connect" => {
            doc.push_undo("Connected");
            let a = &mut doc.scene.armatures[ai];
            let on = flag(a.bones[bi].use_connect);
            if let (true, Some(p)) = (on, a.bones[bi].parent) {
                let t = a.bones[p].tail;
                a.bones[bi].head = t;
            }
            a.bones[bi].use_connect = on && a.bones[bi].parent.is_some();
        }
        "use_deform" => {
            doc.push_undo("Deform");
            let b = &mut doc.scene.armatures[ai].bones[bi];
            b.use_deform = flag(b.use_deform);
        }
        "use_inherit_rotation" => {
            doc.push_undo("Inherit Rotation");
            let b = &mut doc.scene.armatures[ai].bones[bi];
            b.use_inherit_rotation = flag(b.use_inherit_rotation);
        }
        "use_local_location" => {
            doc.push_undo("Local Location");
            let b = &mut doc.scene.armatures[ai].bones[bi];
            b.use_local_location = flag(b.use_local_location);
        }
        _ => return,
    }
    doc.touch();
}

fn constraint_set(doc: &mut Doc, arg: &str) {
    let mut it = arg.splitn(4, '\t');
    let (Some(obj), Some(bone), Some(idx), Some(kv)) = (it.next(), it.next(), it.next(), it.next()) else { return };
    let Ok(i) = idx.parse::<usize>() else { return };
    let Some((key, value)) = kv.split_once('=') else { return };
    let value = if value == "(None)" { "" } else { value };
    if super::cons::stack(doc, obj, bone).is_none_or(|s| i >= s.len()) {
        return;
    }
    doc.push_undo("Change Constraint");
    let Some(c) = super::cons::stack_mut(doc, obj, bone).and_then(|s| s.get_mut(i)) else { return };
    let space = || Space::NAMES.iter().find(|(n, _)| *n == value).map(|x| x.1);
    let axis = || TrackAxis::NAMES.iter().find(|(n, _)| *n == value).map(|x| x.1);
    let num = || value.parse::<u8>().ok();
    match key {
        "target" => {
            if let Some(t) = c.target_mut() {
                if t.object != value {
                    t.subtarget.clear();
                }
                t.object = value.to_string();
            }
        }
        "subtarget" => {
            if let Some(t) = c.target_mut() {
                t.subtarget = value.to_string();
            }
        }
        "pole_target" | "pole_subtarget" => {
            if let ConstraintKind::Ik { pole_target, .. } = &mut c.kind {
                if key == "pole_target" {
                    *pole_target = (!value.is_empty()).then(|| Target { object: value.to_string(), ..Target::default() });
                } else if let Some(t) = pole_target {
                    t.subtarget = value.to_string();
                }
            }
        }
        "owner_space" => c.owner_space = space().unwrap_or(c.owner_space),
        "target_space" => c.target_space = space().unwrap_or(c.target_space),
        "track_axis" => match &mut c.kind {
            ConstraintKind::TrackTo { track_axis, .. } | ConstraintKind::DampedTrack { track_axis, .. } | ConstraintKind::LockedTrack { track_axis, .. } => *track_axis = axis().unwrap_or(*track_axis),
            ConstraintKind::Floor { floor_location, .. } => *floor_location = axis().unwrap_or(*floor_location),
            _ => {}
        },
        "up_axis" => {
            if let ConstraintKind::TrackTo { up_axis, .. } = &mut c.kind {
                *up_axis = num().unwrap_or(*up_axis).min(2);
            }
        }
        "lock_axis" => {
            if let ConstraintKind::LockedTrack { lock_axis, .. } = &mut c.kind {
                *lock_axis = num().unwrap_or(*lock_axis).min(2);
            }
        }
        _ => {}
    }
    doc.touch();
}
