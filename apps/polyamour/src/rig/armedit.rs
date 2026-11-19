// SPDX-License-Identifier: GPL-3.0-only
//! Armature Edit Mode: selecting heads, tails and bodies, extrude (E),
//! duplicate (Shift+D), add (Shift+A), delete (X), subdivide, symmetrize,
//! roll (Ctrl+R, Shift+N recalculate, Alt+R clear), parenting (Ctrl+P
//! Connected / Keep Offset, Alt+P), switch direction, hide and reveal.
use super::{edit_armature, flip_name, BoneSel};
use crate::doc::{BoxMode, Doc};
use crate::ui::Op;
use sex_3d::armature::{mat3_vec_to_roll, vec_roll_to_mat3, Armature, Bone};
use sex_3d::Vec3;

/// Which part of a bone a click hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Head,
    Tail,
    Body,
}

/// Bones shorter than this are removed when Edit Mode ends (Blender drops
/// zero-length bones).
const MIN_LEN: f32 = 2e-4;

fn arm<'a>(doc: &'a mut Doc, ai: usize) -> &'a mut Armature {
    &mut doc.scene.armatures[ai]
}

/// Make shared joints agree: a connected child's head is its parent's tail.
pub fn flush(doc: &mut Doc, oi: usize) {
    let Some(a) = super::armature_data(doc, oi) else { return };
    let name = doc.scene.objects[oi].name.clone();
    let bones = doc.scene.armatures[a].bones.clone();
    let ui = doc.rig.arm_mut(&name);
    for (i, b) in bones.iter().enumerate() {
        let _ = i;
        let Some(p) = b.parent.filter(|&p| p < bones.len() && b.use_connect) else { continue };
        let pn = &bones[p].name;
        let (mut ps, mut cs) = (ui.get(pn), ui.get(&b.name));
        if ps.tail || cs.head {
            ps.tail = true;
            cs.head = true;
        }
        ui.set(pn, ps);
        ui.set(&b.name, cs);
    }
    ui.sel.retain(|n, _| bones.iter().any(|b| &b.name == n));
    if ui.active.as_ref().is_some_and(|a| !bones.iter().any(|b| &b.name == a)) {
        ui.active = None;
    }
}

/// Click select. `hit` is the bone and part under the pointer.
pub fn select_pick(doc: &mut Doc, oi: usize, hit: Option<(usize, Part)>, extend: bool) {
    let Some(ai) = super::armature_data(doc, oi) else { return };
    let name = doc.scene.objects[oi].name.clone();
    let Some((bi, part)) = hit else {
        if !extend {
            doc.rig.arm_mut(&name).clear();
        }
        return;
    };
    let bname = doc.scene.armatures[ai].bones[bi].name.clone();
    let ui = doc.rig.arm_mut(&name);
    let cur = ui.get(&bname);
    let want = match part {
        Part::Body => BoneSel::ALL,
        Part::Head => BoneSel { head: true, ..Default::default() },
        Part::Tail => BoneSel { tail: true, ..Default::default() },
    };
    if extend {
        let on = match part {
            Part::Body => cur.body,
            Part::Head => cur.head,
            Part::Tail => cur.tail,
        };
        let is_active = ui.active.as_deref() == Some(&bname);
        if on && is_active {
            let mut s = cur;
            match part {
                Part::Body => s = BoneSel::default(),
                Part::Head => s.head = false,
                Part::Tail => s.tail = false,
            }
            ui.set(&bname, s);
        } else {
            let s = BoneSel { body: cur.body || want.body, head: cur.head || want.head, tail: cur.tail || want.tail };
            ui.set(&bname, s);
            ui.active = Some(bname);
        }
    } else {
        ui.clear();
        ui.set(&bname, want);
        ui.active = Some(bname);
    }
    flush(doc, oi);
}

/// Box select: joints inside the box, and bodies with both ends inside.
pub fn select_box(doc: &mut Doc, oi: usize, joints: &[(usize, bool)], mode: BoxMode) {
    let Some(ai) = super::armature_data(doc, oi) else { return };
    let name = doc.scene.objects[oi].name.clone();
    let bones = doc.scene.armatures[ai].bones.clone();
    let mut hit = vec![BoneSel::default(); bones.len()];
    for &(b, tail) in joints {
        if b < bones.len() {
            if tail {
                hit[b].tail = true;
            } else {
                hit[b].head = true;
            }
        }
    }
    for h in &mut hit {
        h.body = h.head && h.tail;
    }
    let ui = doc.rig.arm_mut(&name);
    for (i, b) in bones.iter().enumerate() {
        let cur = ui.get(&b.name);
        let h = hit[i];
        let f = |c: bool, h: bool| match mode {
            BoxMode::Set => h,
            BoxMode::Extend => c || h,
            BoxMode::Subtract => c && !h,
            BoxMode::Invert => c != h,
            BoxMode::Intersect => c && h,
        };
        ui.set(&b.name, BoneSel { body: f(cur.body, h.body), head: f(cur.head, h.head), tail: f(cur.tail, h.tail) });
    }
    if let Some(i) = hit.iter().rposition(|h| h.body) {
        ui.active = Some(bones[i].name.clone());
    }
    flush(doc, oi);
}

/// Selected bone indices (bodies).
pub fn selected(doc: &Doc, oi: usize) -> Vec<usize> {
    let Some(a) = doc.scene.armature_of(oi) else { return Vec::new() };
    let ui = doc.rig.arms.get(&doc.scene.objects[oi].name);
    (0..a.bones.len()).filter(|&i| ui.is_some_and(|u| u.get(&a.bones[i].name).body)).collect()
}

/// Bones with any part selected.
fn touched(doc: &Doc, oi: usize) -> Vec<usize> {
    let Some(a) = doc.scene.armature_of(oi) else { return Vec::new() };
    let ui = doc.rig.arms.get(&doc.scene.objects[oi].name);
    (0..a.bones.len()).filter(|&i| ui.is_some_and(|u| u.get(&a.bones[i].name).any())).collect()
}

/// Remove bones by index, re-parenting children to the removed bone's
/// parent.
pub fn remove_bones(a: &mut Armature, remove: &[usize]) {
    let n = a.bones.len();
    let gone: Vec<bool> = (0..n).map(|i| remove.contains(&i)).collect();
    // New parents for survivors.
    let mut parents: Vec<Option<usize>> = (0..n).map(|i| a.parent(i)).collect();
    for i in 0..n {
        let mut p = parents[i];
        let mut guard = 0;
        let mut moved = false;
        while let Some(pi) = p {
            if !gone[pi] || guard > n {
                break;
            }
            p = a.parent(pi);
            moved = true;
            guard += 1;
        }
        parents[i] = p;
        if moved {
            a.bones[i].use_connect = false;
        }
    }
    let mut map = vec![usize::MAX; n];
    let mut k = 0;
    for i in 0..n {
        if !gone[i] {
            map[i] = k;
            k += 1;
        }
    }
    let mut out = Vec::with_capacity(k);
    for i in 0..n {
        if !gone[i] {
            let mut b = a.bones[i].clone();
            b.parent = parents[i].map(|p| map[p]);
            out.push(b);
        }
    }
    a.bones = out;
}

/// Leaving Edit Mode: drop zero-length bones.
pub fn validate(doc: &mut Doc, oi: usize, ai: usize) {
    let a = &mut doc.scene.armatures[ai];
    let short: Vec<usize> = (0..a.bones.len()).filter(|&i| a.bones[i].length() < MIN_LEN).collect();
    if !short.is_empty() {
        remove_bones(a, &short);
        flush(doc, oi);
    }
}

/// Roll that turns the bone's Z axis toward `align` (Blender's
/// `ED_armature_ebone_roll_to_vector`).
pub fn roll_to_vector(head: Vec3, tail: Vec3, roll: f32, align: Vec3) -> f32 {
    let nor = (tail - head).normalize();
    if nor == Vec3::ZERO {
        return roll;
    }
    let v = align - nor * align.dot(nor);
    if v.length_squared() < 1e-12 {
        return roll;
    }
    let v = v.normalize();
    let m = vec_roll_to_mat3(nor, 0.0);
    let z = m.cols[2];
    let ang = z.dot(v).clamp(-1.0, 1.0).acos();
    if z.cross(v).dot(nor) < 0.0 {
        -ang
    } else {
        ang
    }
}

/// Bone operators. Every one works on the armature in Edit Mode.
pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let Some((oi, ai)) = edit_armature(doc) else {
        return match op.id.as_str() {
            // Bone collection and property operators work in any mode.
            "armature.collection_add" | "armature.collection_remove" | "armature.collection_assign" | "armature.collection_unassign" | "armature.collection_hide" => {
                collection_op(doc, op);
                true
            }
            _ => {
                if op.id.starts_with("armature.") {
                    doc.report = "Armature operators need an armature in Edit Mode".into();
                    true
                } else {
                    false
                }
            }
        };
    };
    let name = doc.scene.objects[oi].name.clone();
    let arg = op.arg.as_str();
    match op.id.as_str() {
        "armature.select_all" => {
            let bones: Vec<String> = doc.scene.armatures[ai].bones.iter().map(|b| b.name.clone()).collect();
            let any = doc.rig.arm(&name).sel.values().any(|s| s.any());
            let ui = doc.rig.arm_mut(&name);
            for b in &bones {
                let cur = ui.get(b);
                let s = match arg {
                    "DESELECT" => BoneSel::default(),
                    "INVERT" => BoneSel { body: !cur.body, head: !cur.head, tail: !cur.tail },
                    "TOGGLE" if any => BoneSel::default(),
                    _ => BoneSel::ALL,
                };
                ui.set(b, s);
            }
            flush(doc, oi);
        }
        "armature.select_mirror" => {
            let extend = arg == "EXTEND";
            let ui = doc.rig.arm(&name);
            let bones = &doc.scene.armatures[ai].bones;
            let mut new = ui.clone();
            if !extend {
                new.clear();
            }
            for b in bones {
                let s = ui.get(&b.name);
                if s.any() {
                    let f = flip_name(&b.name);
                    if bones.iter().any(|x| x.name == f) {
                        new.set(&f, s);
                        if ui.active.as_deref() == Some(&b.name) {
                            new.active = Some(f);
                        }
                    } else if extend {
                        new.set(&b.name, s);
                    }
                }
            }
            doc.rig.arms.insert(name, new);
        }
        "armature.select_hierarchy" => {
            let a = &doc.scene.armatures[ai];
            let ui = doc.rig.arm(&name);
            let Some(act) = ui.active.as_ref().and_then(|n| a.find(n)) else { return true };
            let to = if arg == "CHILD" { (0..a.bones.len()).find(|&c| a.parent(c) == Some(act)) } else { a.parent(act) };
            if let Some(t) = to {
                let tn = a.bones[t].name.clone();
                let ui = doc.rig.arm_mut(&name);
                if !arg.ends_with("EXTEND") {
                    ui.clear();
                }
                ui.set(&tn, BoneSel::ALL);
                ui.active = Some(tn);
                flush(doc, oi);
            }
        }
        "armature.select_linked" => {
            let a = doc.scene.armatures[ai].clone();
            let sel = touched(doc, oi);
            let ui = doc.rig.arm_mut(&name);
            for i in 0..a.bones.len() {
                // Bones sharing a root with a selected bone.
                let root = |mut x: usize| {
                    let mut g = 0;
                    while let Some(p) = a.parent(x).filter(|_| a.bones[x].use_connect) {
                        x = p;
                        g += 1;
                        if g > a.bones.len() {
                            break;
                        }
                    }
                    x
                };
                if sel.iter().any(|&s| root(s) == root(i)) {
                    ui.set(&a.bones[i].name, BoneSel::ALL);
                }
            }
        }
        "armature.extrude_move" | "armature.extrude" => {
            let a = &doc.scene.armatures[ai];
            let ui = doc.rig.arm(&name);
            let tips: Vec<usize> = (0..a.bones.len()).filter(|&i| ui.get(&a.bones[i].name).tail).collect();
            if tips.is_empty() {
                doc.report = "Select bone tips to extrude".into();
                return true;
            }
            doc.push_undo("Extrude");
            let mut new = Vec::new();
            for p in tips {
                let pb = doc.scene.armatures[ai].bones[p].clone();
                let dir = (pb.tail - pb.head).normalize();
                let mut b = Bone::new(&pb.name, pb.tail, pb.tail + dir * (MIN_LEN * 0.5));
                b.parent = Some(p);
                b.use_connect = true;
                b.roll = pb.roll;
                b.collections = pb.collections.clone();
                b.envelope_distance = pb.envelope_distance;
                b.head_radius = pb.tail_radius;
                b.tail_radius = pb.tail_radius;
                let i = arm(doc, ai).add_bone(b);
                new.push(doc.scene.armatures[ai].bones[i].name.clone());
            }
            let ui = doc.rig.arm_mut(&name);
            ui.clear();
            for n in &new {
                ui.set(n, BoneSel { body: true, head: false, tail: true });
            }
            ui.active = new.last().cloned();
            doc.touch();
        }
        "armature.duplicate_move" | "armature.duplicate" => {
            let sel = selected(doc, oi);
            if sel.is_empty() {
                return true;
            }
            doc.push_undo("Duplicate Bones");
            let mut map = std::collections::BTreeMap::new();
            let order = doc.scene.armatures[ai].order();
            for i in order.into_iter().filter(|i| sel.contains(i)) {
                let mut b = doc.scene.armatures[ai].bones[i].clone();
                b.parent = b.parent.map(|p| map.get(&p).copied().unwrap_or(p));
                let j = arm(doc, ai).add_bone(b);
                map.insert(i, j);
            }
            let names: Vec<String> = map.values().map(|&j| doc.scene.armatures[ai].bones[j].name.clone()).collect();
            let ui = doc.rig.arm_mut(&name);
            ui.clear();
            for n in &names {
                ui.set(n, BoneSel::ALL);
            }
            ui.active = names.last().cloned();
            doc.touch();
        }
        "armature.bone_primitive_add" => {
            doc.push_undo("Add Bone");
            let inv = doc.scene.world_matrix(oi).inverse();
            let head = inv.transform_point(doc.cursor);
            let b = Bone::new(if arg.is_empty() { "Bone" } else { arg }, head, head + Vec3::Z);
            let i = arm(doc, ai).add_bone(b);
            let bn = doc.scene.armatures[ai].bones[i].name.clone();
            let ui = doc.rig.arm_mut(&name);
            ui.clear();
            ui.set(&bn, BoneSel::ALL);
            ui.active = Some(bn);
            doc.touch();
        }
        "armature.delete" => {
            let sel = selected(doc, oi);
            if sel.is_empty() {
                return true;
            }
            doc.push_undo("Delete Bones");
            let gone: Vec<String> = sel.iter().map(|&i| doc.scene.armatures[ai].bones[i].name.clone()).collect();
            remove_bones(arm(doc, ai), &sel);
            doc.scene.objects[oi].pose.retain(|p| !gone.contains(&p.name));
            flush(doc, oi);
            doc.report = format!("Deleted {} bone(s)", gone.len());
            doc.touch();
        }
        "armature.dissolve" => {
            let sel = selected(doc, oi);
            if sel.is_empty() {
                return true;
            }
            doc.push_undo("Dissolve Bones");
            // Each selected bone's child takes its head.
            let a = arm(doc, ai);
            for &i in &sel {
                let head = a.bones[i].head;
                for c in 0..a.bones.len() {
                    if a.parent(c) == Some(i) && a.bones[c].use_connect {
                        a.bones[c].head = head;
                    }
                }
            }
            remove_bones(a, &sel);
            flush(doc, oi);
            doc.touch();
        }
        "armature.subdivide" => {
            let sel = selected(doc, oi);
            if sel.is_empty() {
                return true;
            }
            let cuts: usize = arg.parse().unwrap_or(1).clamp(1, 32);
            doc.push_undo("Subdivide");
            for i in sel {
                let a = arm(doc, ai);
                let b = a.bones[i].clone();
                let segs = cuts + 1;
                let step = (b.tail - b.head) / segs as f32;
                a.bones[i].tail = b.head + step;
                let children: Vec<usize> = (0..a.bones.len()).filter(|&c| a.parent(c) == Some(i)).collect();
                let mut prev = i;
                for s in 1..segs {
                    let mut nb = b.clone();
                    nb.head = b.head + step * s as f32;
                    nb.tail = b.head + step * (s + 1) as f32;
                    nb.parent = Some(prev);
                    nb.use_connect = true;
                    prev = a.add_bone(nb);
                }
                for c in children {
                    a.bones[c].parent = Some(prev);
                }
            }
            flush(doc, oi);
            doc.touch();
        }
        "armature.symmetrize" => {
            let sel = selected(doc, oi);
            doc.push_undo("Symmetrize");
            let mut n = 0;
            for i in sel {
                let a = arm(doc, ai);
                let b = a.bones[i].clone();
                let f = flip_name(&b.name);
                if f == b.name {
                    continue;
                }
                let parent = b.parent.map(|p| {
                    let pn = flip_name(&a.bones[p].name);
                    a.find(&pn).unwrap_or(p)
                });
                let mut m = b.clone();
                m.name = f.clone();
                m.head = Vec3::new(-b.head.x, b.head.y, b.head.z);
                m.tail = Vec3::new(-b.tail.x, b.tail.y, b.tail.z);
                m.roll = -b.roll;
                m.parent = parent;
                m.collections = b.collections.iter().map(|c| flip_name(c)).collect();
                match a.find(&f) {
                    Some(j) => a.bones[j] = m,
                    None => {
                        a.add_bone(m);
                    }
                }
                n += 1;
            }
            doc.report = format!("Symmetrized {n} bone(s)");
            doc.touch();
        }
        "armature.calculate_roll" => {
            let sel = selected(doc, oi);
            doc.push_undo("Recalculate Roll");
            let inv = doc.scene.world_matrix(oi).inverse();
            let cursor = inv.transform_point(doc.cursor);
            let active = doc.rig.arm(&name).active.and_then(|n| doc.scene.armatures[ai].find(&n));
            let a = arm(doc, ai);
            let act_z = active.map(|i| vec_roll_to_mat3((a.bones[i].tail - a.bones[i].head).normalize(), a.bones[i].roll).cols[2]);
            for i in sel {
                let b = a.bones[i].clone();
                let nor = (b.tail - b.head).normalize();
                let (neg, ty) = match arg.strip_prefix("NEG_") {
                    Some(t) => (true, t),
                    None => (false, arg),
                };
                let s = if neg { -1.0 } else { 1.0 };
                let parent_dir = a.parent(i).map(|p| (a.bones[p].tail - a.bones[p].head).normalize());
                let align = match ty {
                    "GLOBAL_POS_X" | "GLOBAL_X" => Some(Vec3::X * s),
                    "GLOBAL_POS_Y" | "GLOBAL_Y" => Some(Vec3::Y * s),
                    "GLOBAL_POS_Z" | "GLOBAL_Z" => Some(Vec3::Z * s),
                    "CURSOR" => Some(cursor - b.head),
                    "ACTIVE" => act_z,
                    // Local tangents: perpendicular to the chain's bend.
                    "POS_Z" | "Z" => Some(parent_dir.map(|p| p.cross(nor)).filter(|v| v.length_squared() > 1e-8).unwrap_or(Vec3::Z) * s),
                    _ => {
                        // POS_X: the X axis along the bend's tangent.
                        let x = parent_dir.map(|p| p.cross(nor)).filter(|v| v.length_squared() > 1e-8).unwrap_or(Vec3::X) * s;
                        Some(x.cross(nor))
                    }
                };
                if let Some(v) = align {
                    a.bones[i].roll = roll_to_vector(b.head, b.tail, b.roll, v);
                }
            }
            doc.touch();
        }
        "armature.roll_clear" => {
            let sel = selected(doc, oi);
            doc.push_undo("Clear Roll");
            let r: f32 = arg.parse::<f32>().unwrap_or(0.0).to_radians();
            for i in sel {
                arm(doc, ai).bones[i].roll = r;
            }
            doc.touch();
        }
        "armature.parent_set" => {
            let ui = doc.rig.arm(&name);
            let a = &doc.scene.armatures[ai];
            let Some(act) = ui.active.as_ref().and_then(|n| a.find(n)) else {
                doc.report = "No active bone".into();
                return true;
            };
            let sel: Vec<usize> = selected(doc, oi).into_iter().filter(|&i| i != act).collect();
            if sel.is_empty() {
                doc.report = "Select bones to parent to the active bone".into();
                return true;
            }
            doc.push_undo("Make Parent");
            let connect = arg == "CONNECTED";
            for i in sel {
                let a = arm(doc, ai);
                if a.is_ancestor(i, act) {
                    continue;
                }
                a.bones[i].parent = Some(act);
                a.bones[i].use_connect = connect;
                if connect {
                    let t = a.bones[act].tail;
                    a.bones[i].head = t;
                }
            }
            flush(doc, oi);
            doc.touch();
        }
        "armature.parent_clear" => {
            let sel = selected(doc, oi);
            doc.push_undo("Clear Parent");
            for i in sel {
                let a = arm(doc, ai);
                a.bones[i].use_connect = false;
                if arg != "DISCONNECT" {
                    a.bones[i].parent = None;
                }
            }
            doc.touch();
        }
        "armature.switch_direction" => {
            let sel = selected(doc, oi);
            doc.push_undo("Switch Direction");
            for i in sel {
                let a = arm(doc, ai);
                let b = &mut a.bones[i];
                core::mem::swap(&mut b.head, &mut b.tail);
                b.use_connect = false;
                for c in 0..a.bones.len() {
                    if a.parent(c) == Some(i) {
                        a.bones[c].use_connect = false;
                    }
                }
            }
            doc.touch();
        }
        "armature.fill" => {
            // Two selected joints: a bone between them.
            let a = &doc.scene.armatures[ai];
            let ui = doc.rig.arm(&name);
            let mut joints: Vec<Vec3> = Vec::new();
            for b in &a.bones {
                let s = ui.get(&b.name);
                if s.head && !joints.iter().any(|j| j.approx_eq(b.head, 1e-5)) {
                    joints.push(b.head);
                }
                if s.tail && !joints.iter().any(|j| j.approx_eq(b.tail, 1e-5)) {
                    joints.push(b.tail);
                }
            }
            if joints.len() != 2 {
                doc.report = "Select two joints to fill between".into();
                return true;
            }
            doc.push_undo("Fill Between Joints");
            let i = arm(doc, ai).add_bone(Bone::new("Bone", joints[0], joints[1]));
            let bn = doc.scene.armatures[ai].bones[i].name.clone();
            let ui = doc.rig.arm_mut(&name);
            ui.clear();
            ui.set(&bn, BoneSel::ALL);
            ui.active = Some(bn);
            doc.touch();
        }
        "armature.hide" => {
            let a = &doc.scene.armatures[ai];
            let ui = doc.rig.arm(&name);
            let hide: Vec<String> = a.bones.iter().filter(|b| ui.get(&b.name).body != (arg == "UNSELECTED")).map(|b| b.name.clone()).collect();
            let ui = doc.rig.arm_mut(&name);
            for h in hide {
                ui.sel.remove(&h);
                ui.hidden.insert(h);
            }
        }
        "armature.reveal" => {
            let ui = doc.rig.arm_mut(&name);
            for h in std::mem::take(&mut ui.hidden) {
                ui.set(&h, BoneSel::ALL);
            }
        }
        "armature.collection_add" | "armature.collection_remove" | "armature.collection_assign" | "armature.collection_unassign" | "armature.collection_hide" => collection_op(doc, op),
        _ => return false,
    }
    true
}

/// Bone collections: add, remove, assign the selected bones, hide.
fn collection_op(doc: &mut Doc, op: &Op) {
    let Some(oi) = doc.active() else { return };
    let Some(ai) = super::armature_data(doc, oi) else { return };
    let name = doc.scene.objects[oi].name.clone();
    let active = doc.rig.arm(&name).active_collection;
    match op.id.as_str() {
        "armature.collection_add" => {
            doc.push_undo("Add Bone Collection");
            let a = arm(doc, ai);
            let n = sex_3d::scene::unique_name("Bones", |n| a.collections.iter().any(|c| c == n));
            a.collections.push(n);
            let len = a.collections.len();
            doc.rig.arm_mut(&name).active_collection = len - 1;
        }
        "armature.collection_remove" => {
            let Some(c) = doc.scene.armatures[ai].collections.get(active).cloned() else { return };
            doc.push_undo("Remove Bone Collection");
            let a = arm(doc, ai);
            a.collections.remove(active);
            for b in &mut a.bones {
                b.collections.retain(|x| *x != c);
            }
            let ui = doc.rig.arm_mut(&name);
            ui.active_collection = active.saturating_sub(1);
            ui.hidden_collections.remove(&c);
        }
        "armature.collection_assign" | "armature.collection_unassign" => {
            let Some(c) = doc.scene.armatures[ai].collections.get(active).cloned() else { return };
            let ui = doc.rig.arm(&name);
            doc.push_undo("Assign to Bone Collection");
            let assign = op.id == "armature.collection_assign";
            for b in &mut arm(doc, ai).bones {
                if ui.get(&b.name).body {
                    b.collections.retain(|x| *x != c);
                    if assign {
                        b.collections.push(c.clone());
                    }
                }
            }
        }
        _ => {
            // collection_hide: arg = collection index
            let i: usize = op.arg.parse().unwrap_or(active);
            let Some(c) = doc.scene.armatures[ai].collections.get(i).cloned() else { return };
            let ui = doc.rig.arm_mut(&name);
            if !ui.hidden_collections.remove(&c) {
                ui.hidden_collections.insert(c);
            }
        }
    }
    doc.touch();
}

/// Roll of a bone after rotating its direction by `r` (keeps the bone's
/// X/Z axes turning with it).
pub fn rotated_roll(head: Vec3, tail: Vec3, roll: f32, r: &sex_3d::math::Mat3, new_head: Vec3, new_tail: Vec3) -> f32 {
    let m = vec_roll_to_mat3((tail - head).normalize(), roll);
    let nm = *r * m;
    let v = (new_tail - new_head).normalize();
    if v == Vec3::ZERO {
        return roll;
    }
    mat3_vec_to_roll(&nm, v)
}
