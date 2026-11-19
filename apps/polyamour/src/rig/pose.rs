// SPDX-License-Identifier: GPL-3.0-only
//! Pose Mode: bone selection, clearing (Alt+G/R/S), copy and (flipped)
//! paste, rotation modes, visual transform and hiding.
use super::{flip_name, pose_bone_mut, pose_object, BoneSel};
use crate::doc::{BoxMode, Doc};
use crate::ui::Op;
use sex_3d::armature::{PoseBone, RotationMode};
use sex_3d::{Quat, Vec3};

/// Click select in Pose Mode.
pub fn select_pick(doc: &mut Doc, oi: usize, hit: Option<usize>, extend: bool) {
    let name = doc.scene.objects[oi].name.clone();
    let Some(bi) = hit else {
        if !extend {
            doc.rig.arm_mut(&name).clear();
        }
        return;
    };
    let Some(bname) = doc.scene.armature_of(oi).and_then(|a| a.bones.get(bi)).map(|b| b.name.clone()) else { return };
    let ui = doc.rig.arm_mut(&name);
    if extend {
        let on = ui.get(&bname).body;
        if on && ui.active.as_deref() == Some(&bname) {
            ui.set(&bname, BoneSel::default());
        } else {
            ui.set(&bname, BoneSel::ALL);
            ui.active = Some(bname);
        }
    } else {
        ui.clear();
        ui.set(&bname, BoneSel::ALL);
        ui.active = Some(bname);
    }
}

pub fn select_box(doc: &mut Doc, oi: usize, hits: &[usize], mode: BoxMode) {
    let name = doc.scene.objects[oi].name.clone();
    let Some(a) = doc.scene.armature_of(oi).cloned() else { return };
    let ui = doc.rig.arm_mut(&name);
    for (i, b) in a.bones.iter().enumerate() {
        let c = ui.get(&b.name).body;
        let h = hits.contains(&i);
        let on = match mode {
            BoxMode::Set => h,
            BoxMode::Extend => c || h,
            BoxMode::Subtract => c && !h,
            BoxMode::Invert => c != h,
            BoxMode::Intersect => c && h,
        };
        ui.set(&b.name, if on { BoneSel::ALL } else { BoneSel::default() });
    }
    if let Some(&h) = hits.last() {
        ui.active = Some(a.bones[h].name.clone());
    }
}

/// Mirror a pose across the armature's X axis (Paste Flipped).
pub fn flip_pose(p: &PoseBone) -> PoseBone {
    let mut f = p.clone();
    f.name = flip_name(&p.name);
    f.location.x = -p.location.x;
    let q = p.rotation_quaternion;
    f.rotation_quaternion = Quat::new(q.w, q.x, -q.y, -q.z);
    f.rotation_euler = Vec3::new(p.rotation_euler.x, -p.rotation_euler.y, -p.rotation_euler.z);
    let [a, x, y, z] = p.rotation_axis_angle;
    f.rotation_axis_angle = [a, x, -y, -z];
    f
}

pub fn clear(pb: &mut PoseBone, loc: bool, rot: bool, scale: bool) {
    if loc {
        pb.location = Vec3::ZERO;
    }
    if rot {
        pb.rotation_quaternion = Quat::IDENTITY;
        pb.rotation_euler = Vec3::ZERO;
        pb.rotation_axis_angle = [0.0, 0.0, 1.0, 0.0];
    }
    if scale {
        pb.scale = Vec3::ONE;
    }
}

pub fn run(doc: &mut Doc, op: &Op) -> bool {
    if op.id == "pose.constraint_add" {
        return super::cons::run(doc, op);
    }
    let Some(oi) = pose_object(doc) else {
        if op.id.starts_with("pose.") {
            doc.report = "Pose operators need an armature in Pose Mode".into();
            return true;
        }
        return false;
    };
    let name = doc.scene.objects[oi].name.clone();
    let Some(arm) = doc.scene.armature_of(oi).cloned() else { return true };
    let arg = op.arg.as_str();
    let sel = super::selected_pose_bones(doc, oi);
    match op.id.as_str() {
        "pose.select_all" => {
            let any = !sel.is_empty();
            let ui = doc.rig.arm_mut(&name);
            for (i, b) in arm.bones.iter().enumerate() {
                let _ = i;
                let cur = ui.get(&b.name).body;
                let on = match arg {
                    "DESELECT" => false,
                    "INVERT" => !cur,
                    "TOGGLE" => !any,
                    _ => true,
                };
                ui.set(&b.name, if on { BoneSel::ALL } else { BoneSel::default() });
            }
        }
        "pose.select_mirror" => {
            let ui = doc.rig.arm(&name);
            let mut new = ui.clone();
            if arg != "EXTEND" {
                new.clear();
            }
            for b in &sel {
                let f = flip_name(b);
                if arm.find(&f).is_some() {
                    new.set(&f, BoneSel::ALL);
                    if ui.active.as_deref() == Some(b) {
                        new.active = Some(f);
                    }
                }
            }
            doc.rig.arms.insert(name, new);
        }
        "pose.select_hierarchy" => {
            let ui = doc.rig.arm(&name);
            let Some(act) = ui.active.as_ref().and_then(|n| arm.find(n)) else { return true };
            let to = if arg.starts_with("CHILD") { (0..arm.bones.len()).find(|&c| arm.parent(c) == Some(act)) } else { arm.parent(act) };
            if let Some(t) = to {
                let tn = arm.bones[t].name.clone();
                let ui = doc.rig.arm_mut(&name);
                if !arg.ends_with("EXTEND") {
                    ui.clear();
                }
                ui.set(&tn, BoneSel::ALL);
                ui.active = Some(tn);
            }
        }
        "pose.select_linked" => {
            let ui = doc.rig.arm_mut(&name);
            for (i, b) in arm.bones.iter().enumerate() {
                let linked = sel.iter().filter_map(|s| arm.find(s)).any(|s| arm.is_ancestor(s, i) || arm.is_ancestor(i, s));
                if linked {
                    ui.set(&b.name, BoneSel::ALL);
                }
            }
        }
        "pose.loc_clear" | "pose.rot_clear" | "pose.scale_clear" | "pose.transforms_clear" | "pose.user_transforms_clear" => {
            if sel.is_empty() {
                return true;
            }
            let all = op.id == "pose.transforms_clear" || op.id == "pose.user_transforms_clear";
            doc.push_undo(match op.id.as_str() {
                "pose.loc_clear" => "Clear Pose Location",
                "pose.rot_clear" => "Clear Pose Rotation",
                "pose.scale_clear" => "Clear Pose Scale",
                _ => "Clear Pose Transforms",
            });
            for b in &sel {
                let pb = pose_bone_mut(doc, oi, b);
                clear(pb, all || op.id == "pose.loc_clear", all || op.id == "pose.rot_clear", all || op.id == "pose.scale_clear");
            }
            crate::anim::autokey_bones(doc);
            doc.touch();
        }
        "pose.copy" => {
            doc.rig.clipboard = sel.iter().map(|b| super::pose_bone(doc, oi, b)).collect();
            doc.report = format!("Copied pose of {} bone(s)", sel.len());
        }
        "pose.paste" => {
            if doc.rig.clipboard.is_empty() {
                doc.report = "Copy a pose first".into();
                return true;
            }
            let flipped = arg.contains("FLIPPED") || arg == "1";
            let selected_only = arg.contains("SELECTED");
            doc.push_undo(if flipped { "Paste Flipped Pose" } else { "Paste Pose" });
            let clip = doc.rig.clipboard.clone();
            let mut n = 0;
            for p in clip {
                let p = if flipped { flip_pose(&p) } else { p };
                if arm.find(&p.name).is_none() || (selected_only && !sel.contains(&p.name)) {
                    continue;
                }
                let dst = pose_bone_mut(doc, oi, &p.name);
                let keep = (dst.constraints.clone(), dst.ik.clone());
                // Paste converts into the target's rotation mode.
                let mode = dst.rotation_mode;
                *dst = p.clone();
                dst.constraints = keep.0;
                dst.ik = keep.1;
                if mode != p.rotation_mode {
                    let m = p.rotation_mat3();
                    dst.rotation_mode = mode;
                    dst.set_rotation_mat3(&m);
                }
                n += 1;
            }
            crate::anim::autokey_bones(doc);
            doc.report = format!("Pasted pose of {n} bone(s)");
            doc.touch();
        }
        "pose.rotation_mode_set" => {
            let Some(mode) = RotationMode::from_name(arg) else { return true };
            doc.push_undo("Set Rotation Mode");
            for b in &sel {
                let pb = pose_bone_mut(doc, oi, b);
                let m = pb.rotation_mat3();
                pb.rotation_mode = mode;
                pb.set_rotation_mat3(&m);
            }
            doc.touch();
        }
        "pose.visual_transform_apply" => {
            doc.push_undo("Apply Visual Transform");
            let poses = doc.scene.pose_matrices(oi);
            for b in &sel {
                if let Some(v) = super::visual_pose_bone(doc, oi, b, &poses) {
                    let pb = pose_bone_mut(doc, oi, b);
                    pb.location = v.location;
                    pb.rotation_quaternion = v.rotation_quaternion;
                    pb.rotation_euler = v.rotation_euler;
                    pb.rotation_axis_angle = v.rotation_axis_angle;
                    pb.scale = v.scale;
                }
            }
            doc.touch();
        }
        "pose.armature_apply" => {
            // Apply Pose as Rest Pose: bones move to their posed places.
            doc.push_undo("Apply Pose as Rest Pose");
            let poses = sex_3d::armature::simple_pose(&arm, &doc.scene.objects[oi].pose);
            let ai = super::armature_data(doc, oi).expect("armature");
            let a = &mut doc.scene.armatures[ai];
            for (i, m) in poses.iter().enumerate() {
                let b = &mut a.bones[i];
                let len = b.length();
                b.head = m.translation_part();
                let y = m.col3(1).normalize();
                b.tail = b.head + y * len * m.col3(1).length();
                b.roll = sex_3d::armature::mat3_vec_to_roll(&m.mat3().normalized(), y);
            }
            for p in &mut doc.scene.objects[oi].pose {
                clear(p, true, true, true);
            }
            doc.touch();
        }
        "pose.hide" => {
            let ui = doc.rig.arm_mut(&name);
            let hide: Vec<String> = arm.bones.iter().filter(|b| sel.contains(&b.name) != (arg == "UNSELECTED")).map(|b| b.name.clone()).collect();
            for h in hide {
                ui.sel.remove(&h);
                ui.hidden.insert(h);
            }
        }
        "pose.reveal" => {
            let ui = doc.rig.arm_mut(&name);
            for h in std::mem::take(&mut ui.hidden) {
                ui.set(&h, BoneSel::ALL);
            }
        }
        "pose.ik_toggle" => doc.rig.auto_ik = !doc.rig.auto_ik,
        _ => return false,
    }
    true
}
