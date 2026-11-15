// SPDX-License-Identifier: GPL-3.0-only
//! Vertex groups (the Object Data tab's list and its Edit Mode buttons) and
//! Weight Paint strokes: Draw (Mix / Add / Subtract), Blur, Average, Smear
//! and Gradient, with Auto Normalize, X Mirror and locked groups.
use super::{flip_name, WpBlend, WpBrush};
use crate::doc::{Doc, Mode};
use crate::ui::Op;
use sex_3d::scene::ObjectData;
use sex_3d::Mesh;
use std::collections::HashMap;

/// The active mesh object and its mesh.
pub fn active_mesh(doc: &Doc) -> Option<(usize, usize)> {
    let a = doc.active()?;
    match doc.scene.objects[a].data {
        ObjectData::Mesh(mi) if mi < doc.scene.meshes.len() => Some((a, mi)),
        _ => None,
    }
}

pub fn active_group(doc: &Doc, oi: usize, mi: usize) -> Option<usize> {
    let g = doc.rig.active_group(&doc.scene.objects[oi].name);
    (g < doc.scene.meshes[mi].vertex_groups.len()).then_some(g)
}

pub fn is_locked(doc: &Doc, oi: usize, group: &str) -> bool {
    doc.rig.group_lock.contains(&(doc.scene.objects[oi].name.clone(), group.to_string()))
}

/// Vertex-group operators.
pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let Some((oi, mi)) = active_mesh(doc) else {
        return op.id.starts_with("object.vertex_group") || op.id.starts_with("paint.");
    };
    let name = doc.scene.objects[oi].name.clone();
    let arg = op.arg.as_str();
    let ag = active_group(doc, oi, mi);
    match op.id.as_str() {
        "object.vertex_group_add" => {
            doc.push_undo("Add Vertex Group");
            let g = doc.scene.meshes[mi].add_vertex_group(if arg.is_empty() { "Group" } else { arg });
            doc.rig.active_group.insert(name, g);
        }
        "object.vertex_group_remove" => {
            if doc.scene.meshes[mi].vertex_groups.is_empty() {
                return true;
            }
            doc.push_undo("Remove Vertex Group");
            if arg == "ALL" {
                let m = &mut doc.scene.meshes[mi];
                while !m.vertex_groups.is_empty() {
                    m.remove_vertex_group(0);
                }
                doc.rig.active_group.insert(name, 0);
            } else if let Some(g) = ag {
                let gname = doc.scene.meshes[mi].vertex_groups[g].clone();
                doc.scene.meshes[mi].remove_vertex_group(g);
                doc.rig.group_lock.remove(&(name.clone(), gname));
                doc.rig.active_group.insert(name, g.saturating_sub(1));
            }
            doc.touch();
        }
        "object.vertex_group_set_active" => {
            if let Ok(g) = arg.parse::<usize>() {
                doc.rig.active_group.insert(name, g);
            } else if let Some(g) = doc.scene.meshes[mi].vertex_groups.iter().position(|n| n == arg) {
                doc.rig.active_group.insert(name, g);
            }
            doc.touch();
        }
        "object.vertex_group_move" => {
            let Some(g) = ag else { return true };
            let to = if arg == "UP" { g.checked_sub(1) } else { Some(g + 1).filter(|&t| t < doc.scene.meshes[mi].vertex_groups.len()) };
            let Some(to) = to else { return true };
            doc.push_undo("Move Vertex Group");
            let m = &mut doc.scene.meshes[mi];
            m.vertex_groups.swap(g, to);
            for w in &mut m.weights {
                for e in w.iter_mut() {
                    if e.0 as usize == g {
                        e.0 = to as u32;
                    } else if e.0 as usize == to {
                        e.0 = g as u32;
                    }
                }
            }
            doc.rig.active_group.insert(name, to);
        }
        "object.vertex_group_copy" => {
            let Some(g) = ag else { return true };
            doc.push_undo("Copy Vertex Group");
            let m = &mut doc.scene.meshes[mi];
            let src = m.vertex_groups[g].clone();
            let ng = m.add_vertex_group(&format!("{src}_copy"));
            for v in 0..m.positions.len() {
                if let Some(w) = m.weight_opt(v, g) {
                    m.set_weight(v, ng, w);
                }
            }
            doc.rig.active_group.insert(name, ng);
        }
        "object.vertex_group_lock" => {
            let g = arg.parse::<usize>().ok().or(ag);
            let Some(g) = g.filter(|&g| g < doc.scene.meshes[mi].vertex_groups.len()) else { return true };
            doc.push_undo("Lock Vertex Group");
            let key = (name, doc.scene.meshes[mi].vertex_groups[g].clone());
            if !doc.rig.group_lock.remove(&key) {
                doc.rig.group_lock.insert(key);
            }
        }
        "object.vertex_group_assign" | "object.vertex_group_remove_from" | "object.vertex_group_select" | "object.vertex_group_deselect" => {
            if doc.mode != Mode::Edit {
                doc.report = "Vertex group assignment works in Edit Mode".into();
                return true;
            }
            let assign = op.id == "object.vertex_group_assign";
            let g = match ag {
                Some(g) => g,
                None if assign => {
                    doc.push_undo("Assign to Vertex Group");
                    let g = doc.scene.meshes[mi].add_vertex_group("Group");
                    doc.rig.active_group.insert(name.clone(), g);
                    g
                }
                None => return true,
            };
            let sel = crate::edit::selected_verts(doc);
            match op.id.as_str() {
                "object.vertex_group_assign" => {
                    if ag.is_some() {
                        doc.push_undo("Assign to Vertex Group");
                    }
                    let w = doc.rig.wp.vg_weight;
                    for v in sel {
                        doc.scene.meshes[mi].set_weight(v as usize, g, w);
                    }
                }
                "object.vertex_group_remove_from" => {
                    doc.push_undo("Remove from Vertex Group");
                    for v in sel {
                        doc.scene.meshes[mi].remove_weight(v as usize, g);
                    }
                }
                _ => {
                    let select = op.id == "object.vertex_group_select";
                    let m = &doc.scene.meshes[mi];
                    let members: Vec<u32> = (0..m.positions.len()).filter(|&v| m.weight_opt(v, g).is_some()).map(|v| v as u32).collect();
                    for v in members {
                        if doc.edit_hidden.contains(&v) {
                            continue;
                        }
                        if select {
                            doc.edit.verts.insert(v);
                        } else {
                            doc.edit.verts.remove(&v);
                        }
                    }
                    if !select {
                        doc.edit.edges.clear();
                        doc.edit.faces.clear();
                    }
                    crate::edit::flush(doc);
                }
            }
            doc.touch();
        }
        "object.vertex_group_normalize_all" => {
            doc.push_undo("Normalize All Vertex Groups");
            let locked = locked_groups(doc, oi, mi);
            let m = &mut doc.scene.meshes[mi];
            for v in 0..m.weights.len() {
                normalize_vertex(m, v, None, &locked);
            }
            doc.touch();
        }
        "object.vertex_group_normalize" => {
            let Some(g) = ag else { return true };
            doc.push_undo("Normalize Vertex Group");
            let m = &mut doc.scene.meshes[mi];
            let max = (0..m.positions.len()).filter_map(|v| m.weight_opt(v, g)).fold(0.0f32, f32::max);
            if max > 0.0 {
                for v in 0..m.positions.len() {
                    if let Some(w) = m.weight_opt(v, g) {
                        m.set_weight(v, g, w / max);
                    }
                }
            }
            doc.touch();
        }
        "object.vertex_group_invert" => {
            let Some(g) = ag else { return true };
            doc.push_undo("Invert Vertex Group");
            let m = &mut doc.scene.meshes[mi];
            for v in 0..m.positions.len() {
                let w = m.weight(v, g);
                m.set_weight(v, g, 1.0 - w);
            }
            doc.touch();
        }
        "object.vertex_group_clean" => {
            let limit: f32 = arg.parse().unwrap_or(0.0);
            doc.push_undo("Clean Vertex Group");
            let m = &mut doc.scene.meshes[mi];
            for w in &mut m.weights {
                w.retain(|e| e.1 > limit);
            }
            doc.touch();
        }
        "paint.weight_set" => {
            // Shift+X in Weight Paint: fill the active group on all vertices.
            let Some(g) = ag else { return true };
            doc.push_undo("Set Weight");
            let w = doc.rig.wp.weight;
            let m = &mut doc.scene.meshes[mi];
            for v in 0..m.positions.len() {
                m.set_weight(v, g, w);
            }
            doc.touch();
        }
        _ => return false,
    }
    true
}

/// Groups locked on object `oi`.
fn locked_groups(doc: &Doc, oi: usize, mi: usize) -> Vec<bool> {
    doc.scene.meshes[mi].vertex_groups.iter().map(|g| is_locked(doc, oi, g)).collect()
}

/// Scale vertex `v`'s unlocked weights so they sum to 1, keeping `keep`'s.
fn normalize_vertex(m: &mut Mesh, v: usize, keep: Option<usize>, locked: &[bool]) {
    let Some(list) = m.weights.get_mut(v) else { return };
    let fixed: f32 = list.iter().filter(|e| Some(e.0 as usize) == keep || locked.get(e.0 as usize).copied().unwrap_or(false)).map(|e| e.1).sum();
    let free: f32 = list.iter().filter(|e| Some(e.0 as usize) != keep && !locked.get(e.0 as usize).copied().unwrap_or(false)).map(|e| e.1).sum();
    if free <= 1e-9 {
        return;
    }
    let want = (1.0 - fixed).max(0.0);
    let k = want / free;
    for e in list.iter_mut() {
        if Some(e.0 as usize) != keep && !locked.get(e.0 as usize).copied().unwrap_or(false) {
            e.1 = (e.1 * k).clamp(0.0, 1.0);
        }
    }
}

/// A running weight paint stroke.
pub struct Stroke {
    pub oi: usize,
    pub mi: usize,
    pub group: usize,
    /// The X-mirror group (same group for unsided names).
    mirror_group: usize,
    adjacency: Vec<Vec<u32>>,
    mirror: Vec<Option<u32>>,
    /// Weights of the active group when the stroke began (Gradient).
    orig: Vec<f32>,
    locked: Vec<bool>,
    /// Bone groups auto-normalize balances against (deforming groups).
    deform: Vec<bool>,
}

/// Start a stroke: records the undo step and makes sure there is a group
/// to paint (Blender adds "Group" when painting with none).
pub fn begin(doc: &mut Doc) -> Option<Stroke> {
    let (oi, mi) = active_mesh(doc)?;
    let name = doc.scene.objects[oi].name.clone();
    let g = match active_group(doc, oi, mi) {
        Some(g) => g,
        None => {
            doc.push_undo("Weight Paint");
            let g = doc.scene.meshes[mi].add_vertex_group("Group");
            doc.rig.active_group.insert(name.clone(), g);
            g
        }
    };
    let gname = doc.scene.meshes[mi].vertex_groups[g].clone();
    if is_locked(doc, oi, &gname) {
        doc.report = format!("Vertex group '{gname}' is locked");
        return None;
    }
    if active_group(doc, oi, mi).is_some() {
        doc.push_undo("Weight Paint");
    }
    let m = &doc.scene.meshes[mi];
    let n = m.positions.len();
    let mut adjacency = vec![Vec::new(); n];
    for e in m.edges() {
        adjacency[e[0] as usize].push(e[1]);
        adjacency[e[1] as usize].push(e[0]);
    }
    // X mirror: vertex at (-x, y, z).
    let key = |p: sex_3d::Vec3| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
    let by_pos: HashMap<(i64, i64, i64), u32> = m.positions.iter().enumerate().map(|(i, &p)| (key(p), i as u32)).collect();
    let mirror = m.positions.iter().map(|&p| by_pos.get(&key(sex_3d::Vec3::new(-p.x, p.y, p.z))).copied()).collect();
    let fl = flip_name(&gname);
    let mirror_group = m.vertex_groups.iter().position(|x| *x == fl).unwrap_or(g);
    let orig = (0..n).map(|v| m.weight(v, g)).collect();
    let locked = locked_groups(doc, oi, mi);
    let arm = super::deform_armature(doc, oi).and_then(|a| doc.scene.armature_of(a));
    let deform = m.vertex_groups.iter().map(|gn| arm.is_none_or(|a| a.bones.iter().any(|b| &b.name == gn && b.use_deform))).collect();
    Some(Stroke { oi, mi, group: g, mirror_group, adjacency, mirror, orig, locked, deform })
}

fn blend(blend: WpBlend, w: f32, target: f32, a: f32) -> f32 {
    let r = match blend {
        WpBlend::Mix => w + (target - w) * a,
        WpBlend::Add => w + target * a,
        WpBlend::Subtract => w - target * a,
    };
    r.clamp(0.0, 1.0)
}

impl Stroke {
    fn write(&self, doc: &mut Doc, v: usize, w: f32) {
        let wp = doc.rig.wp;
        let m = &mut doc.scene.meshes[self.mi];
        m.set_weight(v, self.group, w);
        if wp.auto_normalize {
            let locked: Vec<bool> = self.locked.iter().zip(&self.deform).map(|(l, d)| *l || !*d).collect();
            normalize_vertex(m, v, Some(self.group), &locked);
        }
        if wp.x_mirror {
            if let Some(mv) = self.mirror.get(v).copied().flatten() {
                if mv as usize != v || self.mirror_group != self.group {
                    m.set_weight(mv as usize, self.mirror_group, w);
                }
            }
        }
    }

    /// One dab: `hits` are vertices under the brush with their falloff
    /// (0..1); `screen` the projected position of every vertex and `dir`
    /// the stroke direction on screen (Smear).
    pub fn dab(&self, doc: &mut Doc, hits: &[(usize, f32)], screen: &[Option<(f32, f32)>], dir: (f32, f32)) {
        let wp = doc.rig.wp;
        let g = self.group;
        let cur: Vec<f32> = {
            let m = &doc.scene.meshes[self.mi];
            (0..m.positions.len()).map(|v| m.weight(v, g)).collect()
        };
        let avg = {
            let (s, n) = hits.iter().fold((0.0, 0.0), |(s, n), &(v, f)| (s + cur[v] * f, n + f));
            if n > 0.0 {
                s / n
            } else {
                0.0
            }
        };
        let mut out = Vec::with_capacity(hits.len());
        for &(v, f) in hits {
            let a = (f * wp.strength).clamp(0.0, 1.0);
            let w = cur[v];
            let nw = match wp.brush {
                WpBrush::Draw => blend(wp.blend, w, wp.weight, a),
                WpBrush::Blur => {
                    let nb = &self.adjacency[v];
                    if nb.is_empty() {
                        w
                    } else {
                        let m = nb.iter().map(|&u| cur[u as usize]).sum::<f32>() / nb.len() as f32;
                        w + (m - w) * a
                    }
                }
                WpBrush::Average => w + (avg - w) * a,
                WpBrush::Smear => {
                    // Pull the weight of the neighbour behind the stroke.
                    let (dx, dy) = dir;
                    let len = (dx * dx + dy * dy).sqrt();
                    let Some(p) = screen.get(v).copied().flatten() else { continue };
                    if len < 1e-3 {
                        continue;
                    }
                    let mut best: Option<(f32, usize)> = None;
                    for &u in &self.adjacency[v] {
                        if let Some(q) = screen.get(u as usize).copied().flatten() {
                            let d = ((q.0 - p.0) * dx + (q.1 - p.1) * dy) / len;
                            if d < 0.0 && best.is_none_or(|b| d < b.0) {
                                best = Some((d, u as usize));
                            }
                        }
                    }
                    match best {
                        Some((_, u)) => w + (cur[u] - w) * a,
                        None => w,
                    }
                }
                WpBrush::Gradient => w,
            };
            out.push((v, nw));
        }
        for (v, w) in out {
            self.write(doc, v, w);
        }
        doc.touch();
    }

    /// Gradient tool: weight falls from the brush weight at `a` to zero at
    /// `b` (screen points), blended into the weights the stroke started with.
    pub fn gradient(&self, doc: &mut Doc, screen: &[Option<(f32, f32)>], a: (f32, f32), b: (f32, f32), radial: bool) {
        let wp = doc.rig.wp;
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l2 = dx * dx + dy * dy;
        if l2 < 1.0 {
            return;
        }
        let mut out = Vec::new();
        for (v, p) in screen.iter().enumerate() {
            let Some(p) = p else { continue };
            let t = if radial { (((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)) / l2).sqrt() } else { ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2 };
            let t = t.clamp(0.0, 1.0);
            let target = wp.weight * (1.0 - t);
            let w = self.orig[v];
            let nw = match wp.blend {
                WpBlend::Mix => w + (target - w) * wp.strength,
                WpBlend::Add => w + target * wp.strength,
                WpBlend::Subtract => w - target * wp.strength,
            }
            .clamp(0.0, 1.0);
            out.push((v, nw));
        }
        for (v, w) in out {
            self.write(doc, v, w);
        }
        doc.touch();
    }
}

/// Weight paint colour ramp (Blender's default: blue 0, cyan, green, yellow,
/// red 1). `None` (not in the group) shows as blue too.
pub fn weight_color(w: Option<f32>) -> [f32; 3] {
    let w = w.unwrap_or(0.0).clamp(0.0, 1.0);
    let stops: [(f32, [f32; 3]); 5] = [(0.0, [0.0, 0.0, 1.0]), (0.25, [0.0, 1.0, 1.0]), (0.5, [0.0, 1.0, 0.0]), (0.75, [1.0, 1.0, 0.0]), (1.0, [1.0, 0.0, 0.0])];
    for i in 0..4 {
        let (a, ca) = stops[i];
        let (b, cb) = stops[i + 1];
        if w <= b {
            let t = (w - a) / (b - a);
            return [ca[0] + (cb[0] - ca[0]) * t, ca[1] + (cb[1] - ca[1]) * t, ca[2] + (cb[2] - ca[2]) * t];
        }
    }
    [1.0, 0.0, 0.0]
}
