// SPDX-License-Identifier: GPL-3.0-only
//! Shape keys: the Object Data tab's list (add Basis / new key, New Shape
//! from Mix, remove, move, clear), Shape Key Lock, and editing a key in Edit
//! Mode (the mesh holds the key's positions while Edit Mode lasts; moving
//! the Basis carries the keys relative to it along).
use super::paint::active_mesh;
use crate::doc::{Doc, Mode};
use crate::ui::Op;
use sex_3d::scene::{ObjectData, Scene};
use sex_3d::Vec3;

pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let Some((oi, mi)) = active_mesh(doc) else {
        return op.id.starts_with("object.shape_key") || op.id.starts_with("shape.");
    };
    let name = doc.scene.objects[oi].name.clone();
    let arg = op.arg.as_str();
    let n = doc.scene.meshes[mi].shape_keys.len();
    let active = doc.rig.active_shape(&name).min(n.saturating_sub(1));
    let editing = doc.mode == Mode::Edit && doc.active() == Some(oi);
    match op.id.as_str() {
        "object.shape_key_add" => {
            doc.push_undo("Add Shape Key");
            if editing {
                leave_edit(doc, oi);
            }
            let m = &mut doc.scene.meshes[mi];
            let k = if m.shape_keys.is_empty() {
                m.add_shape_key("Basis", false)
            } else {
                let name = format!("Key {}", m.shape_keys.len());
                m.add_shape_key(&name, arg == "FROM_MIX" || arg == "1")
            };
            doc.rig.active_shape.insert(name, k);
            if editing {
                enter_edit(doc, oi);
            }
            doc.touch();
        }
        "object.shape_key_remove" => {
            if n == 0 {
                return true;
            }
            doc.push_undo("Remove Shape Key");
            if editing {
                leave_edit(doc, oi);
            }
            let m = &mut doc.scene.meshes[mi];
            if arg == "ALL" || m.shape_keys.len() <= 2 && active == 0 {
                // Removing the Basis removes them all, as in Blender (when
                // "Apply Mix" is off the mesh keeps the Basis shape).
                m.shape_keys.clear();
                doc.rig.active_shape.insert(name, 0);
            } else {
                if active == 0 {
                    // The next key becomes the Basis.
                    m.positions = m.shape_keys[1].positions.clone();
                    m.positions.resize(m.positions.len().max(m.shape_keys[0].positions.len()), Vec3::ZERO);
                }
                m.shape_keys.remove(active);
                for k in &mut m.shape_keys {
                    if k.relative_key == active {
                        k.relative_key = 0;
                    } else if k.relative_key > active {
                        k.relative_key -= 1;
                    }
                }
                if m.shape_keys.len() == 1 {
                    m.shape_keys.clear();
                }
                doc.rig.active_shape.insert(name, active.saturating_sub(1));
            }
            if editing {
                enter_edit(doc, oi);
            }
            doc.touch();
        }
        "object.shape_key_move" => {
            if active == 0 || n < 3 {
                return true;
            }
            let to = match arg {
                "UP" => active.saturating_sub(1).max(1),
                "TOP" => 1,
                "BOTTOM" => n - 1,
                _ => (active + 1).min(n - 1),
            };
            if to == active {
                return true;
            }
            doc.push_undo("Move Shape Key");
            let m = &mut doc.scene.meshes[mi];
            let k = m.shape_keys.remove(active);
            m.shape_keys.insert(to, k);
            // Relative keys follow by index.
            let remap = |r: usize| -> usize {
                if r == active {
                    to
                } else if active < to && r > active && r <= to {
                    r - 1
                } else if to < active && r >= to && r < active {
                    r + 1
                } else {
                    r
                }
            };
            for k in &mut m.shape_keys {
                k.relative_key = remap(k.relative_key);
            }
            doc.rig.active_shape.insert(name, to);
            doc.touch();
        }
        "object.shape_key_clear" => {
            doc.push_undo("Clear Shape Keys");
            for k in &mut doc.scene.meshes[mi].shape_keys {
                k.value = 0.0;
            }
            doc.touch();
        }
        "object.shape_key_set_active" => {
            let Ok(k) = arg.parse::<usize>() else { return true };
            if editing {
                leave_edit(doc, oi);
            }
            doc.rig.active_shape.insert(name, k.min(n.saturating_sub(1)));
            if editing {
                enter_edit(doc, oi);
            }
            doc.touch();
        }
        "object.shape_key_lock" => {
            if !doc.rig.shape_lock.remove(&name) {
                doc.rig.shape_lock.insert(name);
            }
            doc.touch();
        }
        "object.shape_key_edit_mode" => {
            if !doc.rig.shape_edit_mode.remove(&name) {
                doc.rig.shape_edit_mode.insert(name);
            }
            doc.touch();
        }
        "object.shape_key_mirror" => {
            if active == 0 {
                return true;
            }
            doc.push_undo("Mirror Shape Key");
            let m = &mut doc.scene.meshes[mi];
            let key = |p: Vec3| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
            let by: std::collections::HashMap<_, usize> = m.positions.iter().enumerate().map(|(i, &p)| (key(p), i)).collect();
            let old = m.shape_keys[active].positions.clone();
            for (v, &p) in m.positions.clone().iter().enumerate() {
                if let Some(&mv) = by.get(&key(Vec3::new(-p.x, p.y, p.z))) {
                    if let (Some(k), Some(slot)) = (old.get(mv).copied(), m.shape_keys[active].positions.get_mut(v)) {
                        *slot = Vec3::new(-k.x, k.y, k.z);
                    }
                }
            }
            doc.touch();
        }
        _ => return false,
    }
    true
}

/// Entering Edit Mode on a mesh: show the active shape key's positions.
pub fn enter_edit(doc: &mut Doc, oi: usize) {
    let ObjectData::Mesh(mi) = doc.scene.objects[oi].data else { return };
    let name = doc.scene.objects[oi].name.clone();
    let m = &mut doc.scene.meshes[mi];
    if m.shape_keys.len() < 2 {
        doc.rig.shape_stash = None;
        return;
    }
    let k = doc.rig.active_shape(&name).min(m.shape_keys.len() - 1);
    let basis = m.positions.clone();
    if k > 0 {
        let kp = &m.shape_keys[k].positions;
        for (v, p) in m.positions.iter_mut().enumerate() {
            if let Some(q) = kp.get(v) {
                *p = *q;
            }
        }
    }
    doc.rig.shape_stash = Some((name, k, basis));
}

/// Leaving Edit Mode: the edited positions go back into the key (or into
/// the Basis, moving keys relative to it by the same offsets).
pub fn leave_edit(doc: &mut Doc, oi: usize) {
    let Some((name, k, basis)) = doc.rig.shape_stash.take() else { return };
    if doc.scene.objects.get(oi).is_none_or(|o| o.name != name) {
        return;
    }
    let ObjectData::Mesh(mi) = doc.scene.objects[oi].data else { return };
    restore(&mut doc.scene.meshes[mi], k, &basis);
}

fn restore(m: &mut sex_3d::Mesh, k: usize, basis: &[Vec3]) {
    if m.shape_keys.is_empty() {
        return;
    }
    let edited = m.positions.clone();
    if k > 0 && k < m.shape_keys.len() {
        m.shape_keys[k].positions = edited.clone();
        // New vertices (extrude) start at their edited place in the Basis.
        let mut b = basis.to_vec();
        if b.len() > edited.len() {
            b.truncate(edited.len());
        }
        b.extend_from_slice(&edited[b.len()..]);
        m.positions = b;
    } else if edited.len() == basis.len() {
        let delta: Vec<Vec3> = edited.iter().zip(basis).map(|(a, b)| *a - *b).collect();
        for (j, key) in m.shape_keys.iter_mut().enumerate() {
            if j == 0 || key.relative_key != 0 {
                continue;
            }
            for (v, p) in key.positions.iter_mut().enumerate() {
                if let Some(d) = delta.get(v) {
                    *p += *d;
                }
            }
        }
    }
    let pos = m.positions.clone();
    m.shape_keys[0].positions = pos;
}

/// The scene as it should be saved: a key being edited goes back into
/// place.
pub fn scene_for_save(doc: &Doc) -> Option<Scene> {
    let (name, k, basis) = doc.rig.shape_stash.as_ref()?;
    let oi = doc.scene.find_object(name)?;
    let ObjectData::Mesh(mi) = doc.scene.objects[oi].data else { return None };
    let mut s = doc.scene.clone();
    restore(&mut s.meshes[mi], *k, basis);
    Some(s)
}

/// Shape Key Lock (pin) shows only the active key at full strength: set the
/// key values for drawing; returns what to put back with [`unpin`].
pub fn pin(doc: &mut Doc) -> Vec<(usize, Vec<(f32, bool)>)> {
    let mut saved = Vec::new();
    if doc.mode == Mode::Edit {
        return saved;
    }
    for oi in 0..doc.scene.objects.len() {
        let ObjectData::Mesh(mi) = doc.scene.objects[oi].data else { continue };
        let name = &doc.scene.objects[oi].name;
        if !doc.rig.shape_lock.contains(name) {
            continue;
        }
        let k = doc.rig.active_shape(name);
        let m = &mut doc.scene.meshes[mi];
        if m.shape_keys.len() < 2 || saved.iter().any(|s: &(usize, _)| s.0 == mi) {
            continue;
        }
        saved.push((mi, m.shape_keys.iter().map(|s| (s.value, s.mute)).collect()));
        for (j, s) in m.shape_keys.iter_mut().enumerate() {
            s.value = if j == k { 1.0 } else { 0.0 };
            s.mute = j != k;
        }
    }
    saved
}

pub fn unpin(doc: &mut Doc, saved: Vec<(usize, Vec<(f32, bool)>)>) {
    for (mi, vals) in saved {
        for (s, (v, m)) in doc.scene.meshes[mi].shape_keys.iter_mut().zip(vals) {
            s.value = v;
            s.mute = m;
        }
    }
}
