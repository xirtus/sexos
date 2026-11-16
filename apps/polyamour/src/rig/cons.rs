// SPDX-License-Identifier: GPL-3.0-only
//! Object and bone constraint operators: Add Object Constraint / Add Bone
//! Constraint, delete, move up and down, mute, Child Of's Set and Clear
//! Inverse. Constraints are addressed as "object\tbone\tindex" (empty bone:
//! an object constraint).
use crate::doc::Doc;
use crate::ui::{MenuItem, Op};
use sex_3d::constraints::{Constraint, ConstraintKind};
use sex_3d::Mat4;

/// The Add Object Constraint menu, Blender's four columns as headings.
pub const MENU: [(&str, &[(&str, &str)]); 4] = [
    (
        "Motion Tracking",
        &[],
    ),
    (
        "Transform",
        &[
            ("Copy Location", "COPY_LOCATION"),
            ("Copy Rotation", "COPY_ROTATION"),
            ("Copy Scale", "COPY_SCALE"),
            ("Copy Transforms", "COPY_TRANSFORMS"),
            ("Limit Distance", "LIMIT_DISTANCE"),
            ("Limit Location", "LIMIT_LOCATION"),
            ("Limit Rotation", "LIMIT_ROTATION"),
            ("Limit Scale", "LIMIT_SCALE"),
            ("Maintain Volume", "MAINTAIN_VOLUME"),
            ("Transformation", "TRANSFORM"),
        ],
    ),
    (
        "Tracking",
        &[
            ("Damped Track", "DAMPED_TRACK"),
            ("Inverse Kinematics", "IK"),
            ("Locked Track", "LOCKED_TRACK"),
            ("Stretch To", "STRETCH_TO"),
            ("Track To", "TRACK_TO"),
        ],
    ),
    ("Relationship", &[("Armature", "ARMATURE"), ("Child Of", "CHILD_OF"), ("Floor", "FLOOR")]),
];

/// The add menu for objects (`bone` false) or bones.
pub fn add_menu(bone: bool) -> Vec<MenuItem> {
    let op = if bone { "pose.constraint_add" } else { "object.constraint_add" };
    let mut v = Vec::new();
    for (head, items) in MENU {
        if items.is_empty() {
            continue;
        }
        if !v.is_empty() {
            v.push(MenuItem::Separator);
        }
        v.push(MenuItem::Heading(head.into()));
        for (label, id) in items {
            // Inverse Kinematics is a bone constraint.
            if *id == "IK" && !bone {
                continue;
            }
            v.push(MenuItem::op(label, "", Op::with(op, id)));
        }
    }
    v
}

/// Address of a constraint stack.
pub fn addr(object: &str, bone: &str, index: usize) -> String {
    format!("{object}\t{bone}\t{index}")
}

/// The constraint stack of `object` (or of its pose bone `bone`).
pub fn stack_mut<'a>(doc: &'a mut Doc, object: &str, bone: &str) -> Option<&'a mut Vec<Constraint>> {
    let oi = doc.scene.find_object(object)?;
    if bone.is_empty() {
        Some(&mut doc.scene.objects[oi].constraints)
    } else {
        doc.scene.armature_of(oi)?.find(bone)?;
        Some(&mut super::pose_bone_mut(doc, oi, bone).constraints)
    }
}

pub fn stack<'a>(doc: &'a Doc, object: &str, bone: &str) -> Option<&'a Vec<Constraint>> {
    let oi = doc.scene.find_object(object)?;
    if bone.is_empty() {
        Some(&doc.scene.objects[oi].constraints)
    } else {
        doc.scene.objects[oi].pose.iter().find(|p| p.name == bone).map(|p| &p.constraints)
    }
}

fn parse(arg: &str) -> Option<(String, String, usize)> {
    let mut it = arg.split('\t');
    let o = it.next()?.to_string();
    let b = it.next()?.to_string();
    let i = it.next()?.parse().ok()?;
    Some((o, b, i))
}

pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    match op.id.as_str() {
        "object.constraint_add" | "pose.constraint_add" => {
            let bone = op.id == "pose.constraint_add";
            let Some(a) = doc.active() else { return true };
            let Some(mut c) = Constraint::with_defaults(arg, bone) else {
                doc.report = format!("Unknown constraint type '{arg}'");
                return true;
            };
            let object = doc.scene.objects[a].name.clone();
            let bname = if bone {
                match super::active_bone(doc, a) {
                    Some(b) => b,
                    None => {
                        doc.report = "No active bone to add a constraint to".into();
                        return true;
                    }
                }
            } else {
                String::new()
            };
            // Blender fills in another selected object as the target.
            let other = doc.scene.selected.iter().copied().find(|&s| s != a).map(|s| doc.scene.objects[s].name.clone());
            if let (Some(t), Some(o)) = (c.target_mut(), other) {
                t.object = o;
            }
            doc.push_undo("Add Constraint");
            let Some(st) = stack_mut(doc, &object, &bname) else { return true };
            c.name = sex_3d::scene::unique_name(&c.name, |n| st.iter().any(|x| x.name == n));
            st.push(c);
            doc.touch();
        }
        "constraint.delete" | "constraint.move_up" | "constraint.move_down" | "constraint.mute_toggle" | "constraint.move_to_index" | "constraint.duplicate" => {
            let Some((o, b, i)) = parse(arg) else { return true };
            let label = match op.id.as_str() {
                "constraint.delete" => "Delete Constraint",
                "constraint.mute_toggle" => "Mute Constraint",
                "constraint.duplicate" => "Duplicate Constraint",
                _ => "Move Constraint",
            };
            if stack(doc, &o, &b).is_none_or(|s| i >= s.len()) {
                return true;
            }
            doc.push_undo(label);
            let Some(st) = stack_mut(doc, &o, &b) else { return true };
            match op.id.as_str() {
                "constraint.delete" => {
                    st.remove(i);
                }
                "constraint.move_up" if i > 0 => st.swap(i, i - 1),
                "constraint.move_down" if i + 1 < st.len() => st.swap(i, i + 1),
                "constraint.mute_toggle" => st[i].mute = !st[i].mute,
                "constraint.duplicate" => {
                    let mut c = st[i].clone();
                    c.name = sex_3d::scene::unique_name(&c.name, |n| st.iter().any(|x| x.name == n));
                    st.insert(i + 1, c);
                }
                _ => {}
            }
            doc.touch();
        }
        "constraint.childof_set_inverse" | "constraint.childof_clear_inverse" => {
            let Some((o, b, i)) = parse(arg) else { return true };
            let Some(oi) = doc.scene.find_object(&o) else { return true };
            doc.push_undo(if op.id.ends_with("set_inverse") { "Set Inverse" } else { "Clear Inverse" });
            if op.id.ends_with("set_inverse") {
                let bone = (!b.is_empty()).then_some(b.as_str());
                doc.scene.child_of_set_inverse(oi, bone, i);
            } else if let Some(st) = stack_mut(doc, &o, &b) {
                if let Some(ConstraintKind::ChildOf { inverse, .. }) = st.get_mut(i).map(|c| &mut c.kind) {
                    *inverse = Mat4::IDENTITY;
                }
            }
            doc.touch();
        }
        _ => return false,
    }
    true
}
