// SPDX-License-Identifier: GPL-3.0-only
//! The 3D view's rigging side: armatures (Octahedral, Stick, B-Bone,
//! Envelope and Wire display, names and axes) and lattices drawn over the
//! shaded image, bone and lattice point picking and box selection, the
//! weight paint display (blue to red) with its brush, strokes and the
//! F / Shift+F radius and strength controls, and the keymaps and header
//! menus of Pose Mode, armature and lattice Edit Mode and Weight Paint.
use super::proj::{seg_dist, Proj};
use super::{Modal, State};
use crate::doc::{BoxMode, Doc, Mode};
use crate::draw::{Align, Painter, Rect};
use crate::editors::timeline::tri;
use crate::input::{Button, Input, Key, Mods};
use crate::rig::{self, armedit, paint, BoneDisplay, WpBrush};
use crate::ui::{MenuItem, Op, Popup, Ui};
use sex_3d::scene::ObjectData;
use sex_3d::Vec3;

/// A bone as drawn: world head, tail and its X / Z axes scaled by length.
pub struct BoneGeo {
    pub bi: usize,
    pub head: Vec3,
    pub tail: Vec3,
    pub x: Vec3,
    pub z: Vec3,
}

/// Whether the viewport is in a rigging mode (its clicks and keys go here).
pub fn active(d: &Doc) -> bool {
    rig::pose_object(d).is_some() || rig::edit_armature(d).is_some() || rig::edit_lattice(d).is_some() || (d.mode == Mode::WeightPaint && paint::active_mesh(d).is_some())
}

/// Mesh Edit Mode (not armature or lattice Edit Mode).
pub fn mesh_edit(d: &Doc) -> bool {
    d.mode == Mode::Edit && rig::edit_armature(d).is_none() && rig::edit_lattice(d).is_none()
}

pub fn bones(d: &Doc, oi: usize) -> Vec<BoneGeo> {
    let Some(arm) = d.scene.armature_of(oi) else { return Vec::new() };
    let w = d.scene.world_matrix(oi);
    let name = &d.scene.objects[oi].name;
    let ui = d.rig.arm(name);
    let edit = rig::edit_armature(d).is_some_and(|e| e.0 == oi);
    let mats = if edit { arm.rest_matrices() } else { d.scene.pose_matrices(oi) };
    let mut out = Vec::new();
    for (i, b) in arm.bones.iter().enumerate() {
        if !rig::bone_visible(&ui, arm, i) {
            continue;
        }
        let Some(m) = mats.get(i) else { continue };
        let len = b.length();
        let (head, tail) = if edit { (w.transform_point(b.head), w.transform_point(b.tail)) } else { (w.transform_point(m.transform_point(Vec3::ZERO)), w.transform_point(m.transform_point(Vec3::new(0.0, len, 0.0)))) };
        let x = w.transform_vector(m.transform_vector(Vec3::X * len));
        let z = w.transform_vector(m.transform_vector(Vec3::Z * len));
        out.push(BoneGeo { bi: i, head, tail, x, z });
    }
    out
}

fn shade(color: u32, k: f32) -> u32 {
    let c = |s: u32| (((color >> s) & 0xff) as f32 * k).clamp(0.0, 255.0) as u32;
    c(16) << 16 | c(8) << 8 | c(0)
}

fn sc(p: &Proj, v: Vec3) -> Option<(f32, f32)> {
    p.to_screen(v)
}

/// Draw every visible armature and lattice.
pub fn draw(_s: &State, d: &Doc, ui: &mut Ui, p: &Proj) {
    for oi in 0..d.scene.objects.len() {
        if !d.scene.is_visible(oi) {
            continue;
        }
        match d.scene.objects[oi].data {
            ObjectData::Armature(_) => draw_armature(d, ui.p, p, oi),
            ObjectData::Lattice(li) if li < d.scene.lattices.len() => draw_lattice(d, ui.p, p, oi, li),
            _ => {}
        }
    }
}

fn draw_armature(d: &Doc, pt: &mut Painter, p: &Proj, oi: usize) {
    let Some(arm) = d.scene.armature_of(oi) else { return };
    let name = &d.scene.objects[oi].name;
    let ui = d.rig.arm(name);
    let edit = rig::edit_armature(d).is_some_and(|e| e.0 == oi);
    let pose = rig::pose_object(d) == Some(oi);
    let obj_sel = d.scene.selected.contains(&oi);
    let obj_active = d.active() == Some(oi);
    let geo = bones(d, oi);
    let back = p.back;
    // Back to front for the solid shapes.
    let mut order: Vec<usize> = (0..geo.len()).collect();
    let depth = |g: &BoneGeo| p.to_view((g.head + g.tail) * 0.5).z;
    order.sort_by(|&a, &b| depth(&geo[a]).partial_cmp(&depth(&geo[b])).unwrap_or(core::cmp::Ordering::Equal));
    for &gi in &order {
        let g = &geo[gi];
        let b = &arm.bones[g.bi];
        let s = ui.get(&b.name);
        let is_active = ui.active.as_deref() == Some(b.name.as_str());
        let wire = if edit || pose {
            match (s.body, is_active) {
                (true, true) => if edit { 0xffcf6b } else { 0xc4ffff },
                (true, false) => if edit { 0xffa000 } else { 0x50c8ff },
                _ => 0x000000,
            }
        } else if obj_active {
            0xffaa40
        } else if obj_sel {
            0xf09a2e
        } else {
            0x000000
        };
        let body = if pose && s.body { 0x6a8fa8 } else { 0x9a9a9a };
        let joint = |sel: bool| if (edit || pose) && sel { if edit { 0xffa000 } else { 0x50c8ff } } else { 0x1a1a1a };
        let len_w = (g.tail - g.head).length();
        let wpp = p.world_per_pixel(g.head).max(1e-6);
        let jr = (len_w * 0.05 / wpp).clamp(2.0, 7.0);
        match ui.display {
            BoneDisplay::Octahedral | BoneDisplay::BBone => {
                let pts: Vec<Vec3>;
                let faces: Vec<[usize; 3]>;
                if ui.display == BoneDisplay::Octahedral {
                    let c = g.head + (g.tail - g.head) * 0.1;
                    pts = vec![g.head, g.tail, c + (g.x + g.z) * 0.1, c + (g.x - g.z) * 0.1, c + (-g.x - g.z) * 0.1, c + (-g.x + g.z) * 0.1];
                    faces = vec![[0, 2, 3], [0, 3, 4], [0, 4, 5], [0, 5, 2], [1, 3, 2], [1, 4, 3], [1, 5, 4], [1, 2, 5]];
                } else {
                    let (hx, hz) = (g.x * 0.08, g.z * 0.08);
                    let corner = |base: Vec3| [base + hx + hz, base + hx - hz, base - hx - hz, base - hx + hz];
                    let mut v = corner(g.head).to_vec();
                    v.extend(corner(g.tail));
                    pts = v;
                    faces = vec![[0, 1, 2], [0, 2, 3], [4, 6, 5], [4, 7, 6], [0, 4, 5], [0, 5, 1], [1, 5, 6], [1, 6, 2], [2, 6, 7], [2, 7, 3], [3, 7, 4], [3, 4, 0]];
                }
                let sp: Vec<Option<(f32, f32)>> = pts.iter().map(|&v| sc(p, v)).collect();
                let mut fs: Vec<(f32, [usize; 3])> = faces.iter().map(|f| (p.to_view((pts[f[0]] + pts[f[1]] + pts[f[2]]) * (1.0 / 3.0)).z, *f)).collect();
                fs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
                for (_, f) in &fs {
                    let n = (pts[f[1]] - pts[f[0]]).cross(pts[f[2]] - pts[f[0]]).normalize();
                    let k = 0.55 + 0.45 * n.dot(back).abs();
                    if let (Some(a), Some(b2), Some(c)) = (sp[f[0]], sp[f[1]], sp[f[2]]) {
                        tri(pt, a, b2, c, shade(body, k));
                    }
                }
                let mut seen = Vec::new();
                for f in &faces {
                    for (a, b2) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                        let e = (a.min(b2), a.max(b2));
                        if seen.contains(&e) {
                            continue;
                        }
                        seen.push(e);
                        if let (Some(x0), Some(x1)) = (sp[a], sp[b2]) {
                            pt.line(x0.0, x0.1, x1.0, x1.1, wire);
                        }
                    }
                }
                if ui.display == BoneDisplay::Octahedral {
                    if let Some(h) = sp[0] {
                        pt.circle(h.0, h.1, jr, joint(s.head));
                    }
                    if let Some(t2) = sp[1] {
                        pt.circle(t2.0, t2.1, jr, joint(s.tail));
                    }
                }
            }
            BoneDisplay::Stick => {
                if let (Some(a), Some(b2)) = (sc(p, g.head), sc(p, g.tail)) {
                    let (dx, dy) = (b2.0 - a.0, b2.1 - a.1);
                    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
                    let (nx, ny) = (-dy / l, dx / l);
                    let col = if wire == 0 { 0x9a9a9a } else { wire };
                    for o in -2..=2 {
                        let o = o as f32 * 0.7;
                        pt.line(a.0 + nx * o, a.1 + ny * o, b2.0 + nx * o, b2.1 + ny * o, col);
                    }
                    pt.circle(a.0, a.1, 3.5, joint(s.head));
                    pt.circle(b2.0, b2.1, 3.5, joint(s.tail));
                }
            }
            BoneDisplay::Envelope => {
                if let (Some(a), Some(b2)) = (sc(p, g.head), sc(p, g.tail)) {
                    let rh = (b.head_radius / wpp).max(2.0);
                    let rt = (b.tail_radius / p.world_per_pixel(g.tail).max(1e-6)).max(2.0);
                    if edit {
                        pt.ring(a.0, a.1, rh + b.envelope_distance / wpp, 0x5a5a5a);
                    }
                    pt.ring(a.0, a.1, rh, wire.max(0x303030));
                    pt.ring(b2.0, b2.1, rt, wire.max(0x303030));
                    let (dx, dy) = (b2.0 - a.0, b2.1 - a.1);
                    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
                    let (nx, ny) = (-dy / l, dx / l);
                    pt.line(a.0 + nx * rh, a.1 + ny * rh, b2.0 + nx * rt, b2.1 + ny * rt, wire.max(0x303030));
                    pt.line(a.0 - nx * rh, a.1 - ny * rh, b2.0 - nx * rt, b2.1 - ny * rt, wire.max(0x303030));
                }
            }
            BoneDisplay::Wire => {
                if let (Some(a), Some(b2)) = (sc(p, g.head), sc(p, g.tail)) {
                    pt.line(a.0, a.1, b2.0, b2.1, if wire == 0 { 0xb0b0b0 } else { wire });
                }
            }
        }
        if ui.show_axes {
            if let Some(o) = sc(p, g.head) {
                let len = (g.tail - g.head).length().max(1e-6);
                let y = (g.tail - g.head) * (1.0 / len);
                for (v, c) in [(g.x * (0.3 / len.max(1e-6)) * len, 0xe04040), (y * 0.3 * len, 0x40c040), (g.z * (0.3 / len.max(1e-6)) * len, 0x4060e0)] {
                    if let Some(e) = sc(p, g.head + v) {
                        pt.line(o.0, o.1, e.0, e.1, c);
                    }
                }
            }
        }
        if ui.show_names {
            if let Some(m) = sc(p, (g.head + g.tail) * 0.5) {
                let tw = pt.text_width(&b.name);
                pt.label(Rect::new(m.0 as i32 + 6, m.1 as i32 - 8, tw + 4, 16), &b.name, 0xffffff, Align::Left);
            }
        }
    }
}

fn draw_lattice(d: &Doc, pt: &mut Painter, p: &Proj, oi: usize, li: usize) {
    let lat = &d.scene.lattices[li];
    let w = d.scene.world_matrix(oi);
    let edit = rig::edit_lattice(d).is_some_and(|e| e.0 == oi);
    let sel = d.rig.lattice_sel.get(&d.scene.objects[oi].name);
    let (nu, nv, nw) = (lat.points_u as usize, lat.points_v as usize, lat.points_w as usize);
    let idx = |u: usize, v: usize, ww: usize| u + nu * (v + nv * ww);
    let sp: Vec<Option<(f32, f32)>> = lat.points.iter().map(|&q| sc(p, w.transform_point(q))).collect();
    let col = if edit {
        0x101010
    } else if d.active() == Some(oi) {
        0xffaa40
    } else if d.scene.selected.contains(&oi) {
        0xf09a2e
    } else {
        0x101010
    };
    let mut line = |a: usize, b: usize| {
        if let (Some(Some(x)), Some(Some(y))) = (sp.get(a), sp.get(b)) {
            pt.line(x.0, x.1, y.0, y.1, col);
        }
    };
    for ww in 0..nw {
        for v in 0..nv {
            for u in 0..nu {
                let i = idx(u, v, ww);
                if u + 1 < nu {
                    line(i, idx(u + 1, v, ww));
                }
                if v + 1 < nv {
                    line(i, idx(u, v + 1, ww));
                }
                if ww + 1 < nw {
                    line(i, idx(u, v, ww + 1));
                }
            }
        }
    }
    if edit {
        for (i, q) in sp.iter().enumerate() {
            if let Some(q) = q {
                let on = sel.is_some_and(|s| s.contains(&i));
                pt.circle(q.0, q.1, 3.0, if on { 0xffa000 } else { 0x000000 });
            }
        }
    }
}

// ---------------------------------------------------------------- picking

/// The bone of `oi` under (x, y): in Edit Mode the joint or body.
pub fn pick_bone(d: &Doc, p: &Proj, oi: usize, x: f32, y: f32, joints: bool) -> Option<(usize, armedit::Part)> {
    let geo = bones(d, oi);
    let mut best: Option<(f32, usize, armedit::Part)> = None;
    for g in &geo {
        let (Some(a), Some(b)) = (sc(p, g.head), sc(p, g.tail)) else { continue };
        if joints {
            for (q, part) in [(a, armedit::Part::Head), (b, armedit::Part::Tail)] {
                let dd = ((q.0 - x).powi(2) + (q.1 - y).powi(2)).sqrt();
                if dd < 8.0 && best.is_none_or(|bb| dd - 2.0 < bb.0) {
                    best = Some((dd - 2.0, g.bi, part));
                }
            }
        }
        let wpp = p.world_per_pixel((g.head + g.tail) * 0.5).max(1e-6);
        let width = ((g.tail - g.head).length() * 0.1 / wpp).clamp(4.0, 30.0);
        let dd = seg_dist(x, y, a, b);
        if dd < width && best.is_none_or(|bb| dd < bb.0) {
            best = Some((dd, g.bi, armedit::Part::Body));
        }
    }
    best.map(|b| (b.1, b.2))
}

fn pick_lattice(d: &Doc, p: &Proj, oi: usize, li: usize, x: f32, y: f32) -> Option<usize> {
    let w = d.scene.world_matrix(oi);
    let mut best: Option<(f32, usize)> = None;
    for (i, q) in d.scene.lattices[li].points.iter().enumerate() {
        if let Some(s) = sc(p, w.transform_point(*q)) {
            let dd = ((s.0 - x).powi(2) + (s.1 - y).powi(2)).sqrt();
            if dd < 10.0 && best.is_none_or(|b| dd < b.0) {
                best = Some((dd, i));
            }
        }
    }
    best.map(|b| b.1)
}

/// A click in a rigging mode. Returns true when handled.
pub fn click(d: &mut Doc, p: &Proj, x: i32, y: i32, extend: bool) -> bool {
    let (fx, fy) = (x as f32, y as f32);
    if let Some((oi, _)) = rig::edit_armature(d) {
        let hit = pick_bone(d, p, oi, fx, fy, true);
        armedit::select_pick(d, oi, hit, extend);
        d.touch();
        return true;
    }
    if let Some(oi) = rig::pose_object(d) {
        let hit = pick_bone(d, p, oi, fx, fy, false).map(|h| h.0);
        rig::pose::select_pick(d, oi, hit, extend);
        d.touch();
        return true;
    }
    if let Some((oi, li)) = rig::edit_lattice(d) {
        let hit = pick_lattice(d, p, oi, li, fx, fy);
        let name = d.scene.objects[oi].name.clone();
        let sel = d.rig.lattice_sel.entry(name).or_default();
        match (hit, extend) {
            (Some(i), true) => {
                if !sel.remove(&i) {
                    sel.insert(i);
                }
            }
            (Some(i), false) => {
                sel.clear();
                sel.insert(i);
            }
            (None, false) => sel.clear(),
            (None, true) => {}
        }
        d.touch();
        return true;
    }
    false
}

/// In Object Mode, an armature or lattice under the pointer (the shaded
/// image does not show them, so its pick misses them).
pub fn pick_object(d: &Doc, p: &Proj, x: i32, y: i32) -> Option<usize> {
    let (fx, fy) = (x as f32, y as f32);
    for oi in (0..d.scene.objects.len()).rev() {
        if !d.scene.is_visible(oi) {
            continue;
        }
        match d.scene.objects[oi].data {
            ObjectData::Armature(_) if pick_bone(d, p, oi, fx, fy, false).is_some() => return Some(oi),
            ObjectData::Lattice(li) if li < d.scene.lattices.len() => {
                if pick_lattice(d, p, oi, li, fx, fy).is_some() {
                    return Some(oi);
                }
                let w = d.scene.world_matrix(oi);
                let pts: Vec<Option<(f32, f32)>> = d.scene.lattices[li].points.iter().map(|&q| sc(p, w.transform_point(q))).collect();
                let lat = &d.scene.lattices[li];
                let nu = lat.points_u as usize;
                for (i, a) in pts.iter().enumerate() {
                    if let (Some(a), Some(Some(b))) = (a, pts.get(i + 1)) {
                        if (i + 1) % nu != 0 && seg_dist(fx, fy, *a, *b) < 5.0 {
                            return Some(oi);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Box select in a rigging mode. Returns true when handled.
pub fn box_select(d: &mut Doc, p: &Proj, b: Rect, mode: BoxMode) -> bool {
    let inside = |q: Vec3| p.to_screen(q).is_some_and(|(x, y)| b.contains(x as i32, y as i32));
    if let Some((oi, _)) = rig::edit_armature(d) {
        let mut joints = Vec::new();
        for g in bones(d, oi) {
            if inside(g.head) {
                joints.push((g.bi, false));
            }
            if inside(g.tail) {
                joints.push((g.bi, true));
            }
        }
        armedit::select_box(d, oi, &joints, mode);
        d.touch();
        return true;
    }
    if let Some(oi) = rig::pose_object(d) {
        let hits: Vec<usize> = bones(d, oi).iter().filter(|g| inside(g.head) || inside(g.tail) || inside((g.head + g.tail) * 0.5)).map(|g| g.bi).collect();
        rig::pose::select_box(d, oi, &hits, mode);
        d.touch();
        return true;
    }
    if let Some((oi, li)) = rig::edit_lattice(d) {
        let w = d.scene.world_matrix(oi);
        let hits: Vec<usize> = d.scene.lattices[li].points.iter().enumerate().filter(|(_, q)| inside(w.transform_point(**q))).map(|(i, _)| i).collect();
        let name = d.scene.objects[oi].name.clone();
        let sel = d.rig.lattice_sel.entry(name).or_default();
        let cur = sel.clone();
        let n = d.scene.lattices[li].points.len();
        sel.clear();
        for i in 0..n {
            let (c, h) = (cur.contains(&i), hits.contains(&i));
            let on = match mode {
                BoxMode::Set => h,
                BoxMode::Extend => c || h,
                BoxMode::Subtract => c && !h,
                BoxMode::Invert => c != h,
                BoxMode::Intersect => c && h,
            };
            if on {
                sel.insert(i);
            }
        }
        d.touch();
        return true;
    }
    false
}

// ---------------------------------------------------------------- weight paint

/// World positions of the painted mesh as shown (deformed when it can be).
fn paint_positions(d: &Doc, oi: usize, mi: usize) -> Vec<Vec3> {
    let m = &d.scene.meshes[mi];
    if let Some((wm, _)) = d.scene.world_mesh(oi) {
        if wm.positions.len() == m.positions.len() {
            return wm.positions;
        }
    }
    let w = d.scene.world_matrix(oi);
    m.positions.iter().map(|&q| w.transform_point(q)).collect()
}

pub fn screen_verts(d: &Doc, p: &Proj) -> Vec<Option<(f32, f32)>> {
    let Some((oi, mi)) = paint::active_mesh(d) else { return Vec::new() };
    paint_positions(d, oi, mi).iter().map(|&q| sc(p, q)).collect()
}

/// The painted mesh in weight colours (blue 0 to red 1), z-buffered,
/// lit from the view.
pub fn draw_weights(d: &Doc, ui: &mut Ui, p: &Proj) {
    if d.mode != Mode::WeightPaint {
        return;
    }
    let Some((oi, mi)) = paint::active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let g = paint::active_group(d, oi, mi);
    let world = paint_positions(d, oi, mi);
    let proj: Vec<Option<(f32, f32, f32)>> = world.iter().map(|&q| p.project(q)).collect();
    let cols: Vec<[f32; 3]> = (0..m.positions.len()).map(|v| paint::weight_color(g.and_then(|g| m.weight_opt(v, g)))).collect();
    let clip = ui.p.clip().intersect(&p.rect);
    if clip.is_empty() {
        return;
    }
    let (cw, ch) = (clip.w as usize, clip.h as usize);
    let mut zb = vec![f32::NEG_INFINITY; cw * ch];
    let width = ui.p.width;
    let pix = &mut ui.p.pixels;
    for t in m.triangulate() {
        let [a, b, c] = t.verts.map(|v| v as usize);
        let (Some(pa), Some(pb), Some(pc)) = (proj[a], proj[b], proj[c]) else { continue };
        let n = (world[b] - world[a]).cross(world[c] - world[a]).normalize();
        let light = 0.35 + 0.65 * n.dot(p.back).abs();
        let area = (pb.0 - pa.0) * (pc.1 - pa.1) - (pb.1 - pa.1) * (pc.0 - pa.0);
        if area.abs() < 1e-6 {
            continue;
        }
        let x0 = (pa.0.min(pb.0).min(pc.0).floor() as i32).max(clip.x);
        let x1 = (pa.0.max(pb.0).max(pc.0).ceil() as i32).min(clip.right() - 1);
        let y0 = (pa.1.min(pb.1).min(pc.1).floor() as i32).max(clip.y);
        let y1 = (pa.1.max(pb.1).max(pc.1).ceil() as i32).min(clip.bottom() - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((pb.0 - fx) * (pc.1 - fy) - (pb.1 - fy) * (pc.0 - fx)) / area;
                let w1 = ((pc.0 - fx) * (pa.1 - fy) - (pc.1 - fy) * (pa.0 - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = pa.2 * w0 + pb.2 * w1 + pc.2 * w2;
                let zi = (y - clip.y) as usize * cw + (x - clip.x) as usize;
                if z <= zb[zi] {
                    continue;
                }
                zb[zi] = z;
                let ch = |k: usize| ((cols[a][k] * w0 + cols[b][k] * w1 + cols[c][k] * w2) * light * 255.0).clamp(0.0, 255.0) as u32;
                pix[y as usize * width + x as usize] = ch(0) << 16 | ch(1) << 8 | ch(2);
            }
        }
    }
    let _ = ch;
}

/// The brush circle.
pub fn draw_brush(d: &Doc, ui: &mut Ui, r: Rect) {
    if d.mode != Mode::WeightPaint || paint::active_mesh(d).is_none() {
        return;
    }
    let (x, y) = ui.mouse();
    if !r.contains(x, y) {
        return;
    }
    let rad = d.rig.wp.radius;
    ui.p.ring(x as f32, y as f32, rad, 0xffffff);
    ui.p.ring(x as f32, y as f32, rad * d.rig.wp.strength.clamp(0.0, 1.0), 0x909090);
}

fn falloff(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

fn dab(stroke: &paint::Stroke, d: &mut Doc, p: &Proj, at: (f32, f32), dir: (f32, f32)) {
    let screen = screen_verts(d, p);
    let rad = d.rig.wp.radius.max(1.0);
    let hits: Vec<(usize, f32)> = screen
        .iter()
        .enumerate()
        .filter_map(|(v, q)| {
            let q = (*q)?;
            let dd = ((q.0 - at.0).powi(2) + (q.1 - at.1).powi(2)).sqrt();
            (dd < rad).then(|| (v, falloff(dd / rad)))
        })
        .collect();
    stroke.dab(d, &hits, &screen, dir);
}

/// Left press in Weight Paint: Ctrl+click picks a bone (and its group),
/// otherwise a stroke (or a gradient drag) starts.
pub fn paint_press(s: &mut State, d: &mut Doc, p: &Proj, x: i32, y: i32, mods: Mods) -> bool {
    if d.mode != Mode::WeightPaint {
        return false;
    }
    let Some((oi, mi)) = paint::active_mesh(d) else { return false };
    let (fx, fy) = (x as f32, y as f32);
    if mods.ctrl {
        let Some(ao) = rig::deform_armature(d, oi) else { return true };
        if let Some((bi, _)) = pick_bone(d, p, ao, fx, fy, false) {
            let aname = d.scene.objects[ao].name.clone();
            let bname = d.scene.armature_of(ao).map(|a| a.bones[bi].name.clone()).unwrap_or_default();
            let ui = d.rig.arm_mut(&aname);
            ui.clear();
            ui.set(&bname, rig::BoneSel { body: true, head: true, tail: true });
            ui.active = Some(bname.clone());
            let oname = d.scene.objects[oi].name.clone();
            if let Some(g) = d.scene.meshes[mi].vertex_groups.iter().position(|n| *n == bname) {
                d.rig.active_group.insert(oname, g);
            }
            d.touch();
        }
        return true;
    }
    let Some(stroke) = paint::begin(d) else { return true };
    if d.rig.wp.brush == WpBrush::Gradient {
        s.modal = Modal::WeightGradient { stroke: Box::new(stroke), a: (fx, fy), radial: mods.alt };
    } else {
        dab(&stroke, d, p, (fx, fy), (0.0, 0.0));
        s.modal = Modal::WeightPaint { stroke: Box::new(stroke), last: (fx, fy) };
    }
    true
}

/// Run the weight paint modals; returns the modal that continues.
pub fn modal(d: &mut Doc, p: &Proj, inp: Input, m: Modal) -> Modal {
    match m {
        Modal::WeightPaint { stroke, last } => match inp {
            Input::Move { x, y } => {
                let at = (x as f32, y as f32);
                dab(&stroke, d, p, at, (at.0 - last.0, at.1 - last.1));
                Modal::WeightPaint { stroke, last: at }
            }
            Input::Up { button: Button::Left, .. } | Input::Key { key: Key::Esc, .. } => Modal::None,
            _ => Modal::WeightPaint { stroke, last },
        },
        Modal::WeightGradient { stroke, a, radial } => match inp {
            Input::Move { x, y } => {
                let screen = screen_verts(d, p);
                stroke.gradient(d, &screen, a, (x as f32, y as f32), radial);
                Modal::WeightGradient { stroke, a, radial }
            }
            Input::Up { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => Modal::None,
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => {
                d.queue.push(Op::new("ed.undo"));
                Modal::None
            }
            _ => Modal::WeightGradient { stroke, a, radial },
        },
        Modal::BrushSize { strength, x0, orig } => {
            let set = |d: &mut Doc, dx: i32| {
                if strength {
                    d.rig.wp.strength = (orig + dx as f32 / 200.0).clamp(0.0, 1.0);
                } else {
                    d.rig.wp.radius = (orig + dx as f32).clamp(1.0, 500.0);
                }
            };
            match inp {
                Input::Move { x, .. } => {
                    set(d, x - x0);
                    Modal::BrushSize { strength, x0, orig }
                }
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => Modal::None,
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    set(d, 0);
                    Modal::None
                }
                _ => Modal::BrushSize { strength, x0, orig },
            }
        }
        other => other,
    }
}

// ---------------------------------------------------------------- menus

fn op(l: &str, k: &str, id: &str) -> MenuItem {
    MenuItem::op(l, k, Op::new(id))
}
fn opa(l: &str, k: &str, id: &str, a: &str) -> MenuItem {
    MenuItem::op(l, k, Op::with(id, a))
}
fn sub(l: &str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem::Sub(l.into(), items)
}

pub fn pose_select_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        opa("All", "A", "pose.select_all", "SELECT"),
        opa("None", "Alt A", "pose.select_all", "DESELECT"),
        opa("Invert", "Ctrl I", "pose.select_all", "INVERT"),
        MenuItem::Separator,
        op("Box Select", "B", "view3d.select_box"),
        MenuItem::Separator,
        op("Mirror", "Shift Ctrl M", "pose.select_mirror"),
        opa("Parent", "[", "pose.select_hierarchy", "PARENT"),
        opa("Child", "]", "pose.select_hierarchy", "CHILD"),
        op("Linked", "L", "pose.select_linked"),
    ]
}

pub fn pose_menu(d: &Doc) -> Vec<MenuItem> {
    let rot = ["QUATERNION", "XYZ", "XZY", "YXZ", "YZX", "ZXY", "ZYX", "AXIS_ANGLE"];
    let rot_label = ["Quaternion (WXYZ)", "XYZ Euler", "XZY Euler", "YXZ Euler", "YZX Euler", "ZXY Euler", "ZYX Euler", "Axis Angle"];
    vec![
        sub(
            "Clear Transform",
            vec![op("All", "", "pose.transforms_clear"), MenuItem::Separator, op("Location", "Alt G", "pose.loc_clear"), op("Rotation", "Alt R", "pose.rot_clear"), op("Scale", "Alt S", "pose.scale_clear"), MenuItem::Separator, op("Reset Unkeyed", "", "pose.user_transforms_clear")],
        ),
        sub("Apply", vec![op("Apply Pose as Rest Pose", "Ctrl A", "pose.armature_apply"), op("Apply Visual Transform to Pose", "", "pose.visual_transform_apply")]),
        MenuItem::Separator,
        op("Copy Pose", "Ctrl C", "pose.copy"),
        op("Paste Pose", "Ctrl V", "pose.paste"),
        opa("Paste Pose Flipped", "Shift Ctrl V", "pose.paste", "FLIPPED"),
        MenuItem::Separator,
        sub("Animation", vec![op("Insert Keyframe", "I", "anim.keyframe_insert"), op("Delete Keyframes", "Alt I", "anim.keyframe_delete"), op("Clear Keyframes", "Shift Alt I", "anim.keyframes_clear")]),
        sub("Constraints", vec![sub("Add Bone Constraint (with Targets)", rig::cons::add_menu(true))]),
        MenuItem::Check("Auto IK".into(), d.rig.auto_ik, Op::new("pose.ik_toggle")),
        sub("Rotation Mode", rot.iter().zip(rot_label).map(|(a, l)| opa(l, "", "pose.rotation_mode_set", a)).collect()),
        MenuItem::Separator,
        sub("Show/Hide", vec![op("Reveal Bones", "Alt H", "pose.reveal"), op("Hide Selected", "H", "pose.hide"), opa("Hide Unselected", "Shift H", "pose.hide", "UNSELECTED")]),
    ]
}

pub fn armature_select_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        opa("All", "A", "armature.select_all", "SELECT"),
        opa("None", "Alt A", "armature.select_all", "DESELECT"),
        opa("Invert", "Ctrl I", "armature.select_all", "INVERT"),
        MenuItem::Separator,
        op("Box Select", "B", "view3d.select_box"),
        MenuItem::Separator,
        op("Mirror", "Shift Ctrl M", "armature.select_mirror"),
        opa("Parent", "[", "armature.select_hierarchy", "PARENT"),
        opa("Child", "]", "armature.select_hierarchy", "CHILD"),
        op("Linked", "L", "armature.select_linked"),
    ]
}

pub fn armature_add_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![op("Single Bone", "", "armature.bone_primitive_add")]
}

fn roll_items() -> Vec<MenuItem> {
    let r = |l: &str, a: &str| opa(l, "", "armature.calculate_roll", a);
    vec![
        MenuItem::Heading("Recalculate Roll".into()),
        r("Local +X Tangent", "POS_X"),
        r("Local +Z Tangent", "POS_Z"),
        r("Global +X Axis", "GLOBAL_POS_X"),
        r("Global +Y Axis", "GLOBAL_POS_Y"),
        r("Global +Z Axis", "GLOBAL_POS_Z"),
        r("Active Bone", "ACTIVE"),
        r("Cursor", "CURSOR"),
    ]
}

fn armature_parent_items() -> Vec<MenuItem> {
    vec![opa("Connected", "", "armature.parent_set", "CONNECTED"), opa("Keep Offset", "", "armature.parent_set", "OFFSET")]
}
fn armature_clear_parent_items() -> Vec<MenuItem> {
    vec![opa("Clear Parent", "", "armature.parent_clear", "CLEAR"), opa("Disconnect Bone", "", "armature.parent_clear", "DISCONNECT")]
}

pub fn armature_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Extrude", "E", "armature.extrude_move"),
        op("Duplicate", "Shift D", "armature.duplicate_move"),
        op("Fill Between Joints", "F", "armature.fill"),
        op("Subdivide", "", "armature.subdivide"),
        op("Symmetrize", "", "armature.symmetrize"),
        op("Switch Direction", "Alt F", "armature.switch_direction"),
        MenuItem::Separator,
        sub("Bone Roll", {
            let mut v = roll_items();
            v.push(MenuItem::Separator);
            v.push(op("Clear Roll", "Alt R", "armature.roll_clear"));
            v
        }),
        sub("Parent", vec![sub("Make", armature_parent_items()), sub("Clear", armature_clear_parent_items())]),
        MenuItem::Separator,
        op("Delete Bones", "X", "armature.delete"),
        op("Dissolve Bones", "Ctrl X", "armature.dissolve"),
        MenuItem::Separator,
        sub("Show/Hide", vec![op("Reveal Bones", "Alt H", "armature.reveal"), op("Hide Selected", "H", "armature.hide"), opa("Hide Unselected", "Shift H", "armature.hide", "UNSELECTED")]),
    ]
}

pub fn lattice_select_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        opa("All", "A", "lattice.select_all", "SELECT"),
        opa("None", "Alt A", "lattice.select_all", "DESELECT"),
        opa("Invert", "Ctrl I", "lattice.select_all", "INVERT"),
        MenuItem::Separator,
        op("Box Select", "B", "view3d.select_box"),
        op("Mirror", "Shift Ctrl M", "lattice.select_mirror"),
    ]
}

pub fn lattice_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Make Regular", "", "lattice.make_regular"),
        sub("Flip (Distortion Free)", vec![opa("U (X) Axis", "", "lattice.flip", "U"), opa("V (Y) Axis", "", "lattice.flip", "V"), opa("W (Z) Axis", "", "lattice.flip", "W")]),
    ]
}

pub fn weights_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Normalize All", "", "object.vertex_group_normalize_all"),
        op("Normalize", "", "object.vertex_group_normalize"),
        op("Invert", "", "object.vertex_group_invert"),
        op("Clean", "", "object.vertex_group_clean"),
        MenuItem::Separator,
        op("Set Weight", "Shift K", "paint.weight_set"),
    ]
}

pub fn weight_brush_items(d: &Doc) -> Vec<MenuItem> {
    [WpBrush::Draw, WpBrush::Blur, WpBrush::Average, WpBrush::Smear, WpBrush::Gradient].iter().map(|b| MenuItem::Check(b.label().into(), d.rig.wp.brush == *b, Op::with("rig.set", &format!("wp.brush={}", b.label())))).collect()
}

/// Ctrl+P in Object Mode with an armature active: the armature choices.
pub fn object_parent_items(d: &Doc) -> Option<Vec<MenuItem>> {
    let a = d.active()?;
    if !rig::is_armature(d, a) {
        return None;
    }
    let others = d.scene.selected.iter().any(|&i| i != a);
    if !others {
        return None;
    }
    Some(vec![
        op("Object", "", "object.parent_set"),
        opa("Object (Keep Transform)", "", "object.parent_set", "KEEP_TRANSFORM"),
        MenuItem::Separator,
        opa("Armature Deform", "", "object.parent_set", "ARMATURE"),
        opa("   With Empty Groups", "", "object.parent_set", "ARMATURE_NAME"),
        opa("   With Envelope Weights", "", "object.parent_set", "ARMATURE_ENVELOPE"),
        opa("   With Automatic Weights", "", "object.parent_set", "ARMATURE_AUTO"),
        opa("Bone", "", "object.parent_set", "BONE"),
        opa("Bone Relative", "", "object.parent_set", "BONE_RELATIVE"),
    ])
}

/// The header menus of a rigging mode, or None for the default ones.
pub fn header_menus(d: &Doc) -> Option<Vec<(&'static str, fn(&Doc) -> Vec<MenuItem>)>> {
    if rig::pose_object(d).is_some() {
        return Some(vec![("Select", pose_select_menu), ("Pose", pose_menu)]);
    }
    if rig::edit_armature(d).is_some() {
        return Some(vec![("Select", armature_select_menu), ("Add", armature_add_menu), ("Armature", armature_menu)]);
    }
    if rig::edit_lattice(d).is_some() {
        return Some(vec![("Select", lattice_select_menu), ("Lattice", lattice_menu)]);
    }
    if d.mode == Mode::WeightPaint {
        return Some(vec![("Weights", weights_menu)]);
    }
    None
}

// ---------------------------------------------------------------- keys

/// What a rigging-mode key asks the viewport to do afterwards.
pub enum After {
    Nothing,
    /// Start a move (after extrude / duplicate), the undo step is recorded.
    Move,
    /// Ctrl+R: roll the selected bones.
    Roll,
}

/// Keys in the rigging modes. Returns None when the key is not ours.
pub fn key(s: &mut State, d: &mut Doc, ui: &Ui, key: Key, mods: Mods) -> Option<After> {
    use Key as K;
    let q = |d: &mut Doc, id: &str, arg: &str| d.queue.push(Op::with(id, arg));
    let (mx, my) = ui.mouse();
    let popup = |d: &mut Doc, title: &str, items: Vec<MenuItem>| d.menu = Some(Popup::new(title, items, mx - 20, my - 10));
    let alt = mods.alt && !mods.ctrl && !mods.shift;
    if rig::pose_object(d).is_some() {
        match (key, mods) {
            (K::Char('a'), Mods::NONE) => q(d, "pose.select_all", "SELECT"),
            (K::Char('a'), _) if alt => q(d, "pose.select_all", "DESELECT"),
            (K::Char('i'), Mods::CTRL) => q(d, "pose.select_all", "INVERT"),
            (K::Char('g'), _) if alt => q(d, "pose.loc_clear", ""),
            (K::Char('r'), _) if alt => q(d, "pose.rot_clear", ""),
            (K::Char('s'), _) if alt => q(d, "pose.scale_clear", ""),
            (K::Char('c'), Mods::CTRL) => q(d, "pose.copy", ""),
            (K::Char('v'), Mods::CTRL) => q(d, "pose.paste", ""),
            (K::Char('v'), Mods::CTRL_SHIFT) => q(d, "pose.paste", "FLIPPED"),
            (K::Char('m'), Mods::CTRL_SHIFT) => q(d, "pose.select_mirror", ""),
            (K::Char('['), _) => q(d, "pose.select_hierarchy", if mods.shift { "PARENT_EXTEND" } else { "PARENT" }),
            (K::Char(']'), _) => q(d, "pose.select_hierarchy", if mods.shift { "CHILD_EXTEND" } else { "CHILD" }),
            (K::Char('l'), Mods::NONE) => q(d, "pose.select_linked", ""),
            (K::Char('h'), Mods::NONE) => q(d, "pose.hide", ""),
            (K::Char('h'), Mods::SHIFT) => q(d, "pose.hide", "UNSELECTED"),
            (K::Char('h'), _) if alt => q(d, "pose.reveal", ""),
            (K::Char('a'), Mods::CTRL) => popup(d, "Apply", vec![op("Apply Pose as Rest Pose", "", "pose.armature_apply"), op("Apply Visual Transform to Pose", "", "pose.visual_transform_apply")]),
            (K::Char('c'), Mods::CTRL_SHIFT) => popup(d, "Add Bone Constraint (with Targets)", rig::cons::add_menu(true)),
            (K::Char('x'), Mods::NONE) | (K::Delete, _) => {}
            (K::Char('d'), Mods::SHIFT) => {}
            _ => return None,
        }
        return Some(After::Nothing);
    }
    if rig::edit_armature(d).is_some() {
        match (key, mods) {
            (K::Char('a'), Mods::NONE) => q(d, "armature.select_all", "SELECT"),
            (K::Char('a'), _) if alt => q(d, "armature.select_all", "DESELECT"),
            (K::Char('i'), Mods::CTRL) => q(d, "armature.select_all", "INVERT"),
            (K::Char('m'), Mods::CTRL_SHIFT) => q(d, "armature.select_mirror", ""),
            (K::Char('['), _) => q(d, "armature.select_hierarchy", "PARENT"),
            (K::Char(']'), _) => q(d, "armature.select_hierarchy", "CHILD"),
            (K::Char('l'), Mods::NONE) => q(d, "armature.select_linked", ""),
            (K::Char('e'), Mods::NONE) => {
                crate::rig::run(d, &Op::new("armature.extrude_move"));
                return Some(After::Move);
            }
            (K::Char('d'), Mods::SHIFT) => {
                crate::rig::run(d, &Op::new("armature.duplicate_move"));
                return Some(After::Move);
            }
            (K::Char('r'), Mods::CTRL) => return Some(After::Roll),
            (K::Char('r'), _) if alt => q(d, "armature.roll_clear", ""),
            (K::Char('n'), Mods::SHIFT) => popup(d, "Recalculate Roll", roll_items()),
            (K::Char('p'), Mods::CTRL) => popup(d, "Make Parent", armature_parent_items()),
            (K::Char('p'), _) if alt => popup(d, "Clear Parent", armature_clear_parent_items()),
            (K::Char('a'), Mods::SHIFT) => q(d, "armature.bone_primitive_add", ""),
            (K::Char('x'), Mods::NONE) | (K::Delete, _) => popup(d, "Delete", vec![op("Bones", "", "armature.delete"), op("Dissolve Bones", "", "armature.dissolve")]),
            (K::Char('x'), Mods::CTRL) => q(d, "armature.dissolve", ""),
            (K::Char('f'), Mods::NONE) => q(d, "armature.fill", ""),
            (K::Char('f'), _) if alt => q(d, "armature.switch_direction", ""),
            (K::Char('h'), Mods::NONE) => q(d, "armature.hide", ""),
            (K::Char('h'), Mods::SHIFT) => q(d, "armature.hide", "UNSELECTED"),
            (K::Char('h'), _) if alt => q(d, "armature.reveal", ""),
            (K::Char('w'), Mods::NONE) => popup(
                d,
                "Armature Context Menu",
                vec![op("Subdivide", "", "armature.subdivide"), op("Switch Direction", "", "armature.switch_direction"), op("Symmetrize", "", "armature.symmetrize"), op("Fill Between Joints", "F", "armature.fill")],
            ),
            _ => return None,
        }
        return Some(After::Nothing);
    }
    if rig::edit_lattice(d).is_some() {
        match (key, mods) {
            (K::Char('a'), Mods::NONE) => q(d, "lattice.select_all", "SELECT"),
            (K::Char('a'), _) if alt => q(d, "lattice.select_all", "DESELECT"),
            (K::Char('i'), Mods::CTRL) => q(d, "lattice.select_all", "INVERT"),
            (K::Char('m'), Mods::CTRL_SHIFT) => q(d, "lattice.select_mirror", ""),
            (K::Char('x'), Mods::NONE) | (K::Delete, _) | (K::Char('e'), Mods::NONE) | (K::Char('d'), Mods::SHIFT) => {}
            _ => return None,
        }
        return Some(After::Nothing);
    }
    if d.mode == Mode::WeightPaint {
        match (key, mods) {
            (K::Char('f'), Mods::NONE) => s.modal = Modal::BrushSize { strength: false, x0: mx, orig: d.rig.wp.radius },
            (K::Char('f'), Mods::SHIFT) => s.modal = Modal::BrushSize { strength: true, x0: mx, orig: d.rig.wp.strength },
            (K::Char('k'), Mods::SHIFT) => q(d, "paint.weight_set", ""),
            (K::Char('x'), Mods::NONE) | (K::Delete, _) | (K::Char('g'), Mods::NONE) | (K::Char('r'), Mods::NONE) | (K::Char('s'), Mods::NONE) => {}
            _ => return None,
        }
        return Some(After::Nothing);
    }
    None
}

/// K: insert keys with a chosen keying set, or make one active.
pub fn keyframe_menu(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Insert Keyframe".into())];
    v.extend(crate::anim::KEYING_SETS.iter().map(|(id, name)| opa(name, "", "anim.keyframe_insert", id)));
    v.push(MenuItem::Separator);
    let mut sets = vec![MenuItem::Check("None".into(), d.anim.keying_set.is_none(), Op::with("anim.keying_set_active_set", "NONE"))];
    sets.extend(crate::anim::KEYING_SETS.iter().map(|(id, name)| MenuItem::Check(name.to_string(), d.anim.keying_set.as_deref() == Some(*id), Op::with("anim.keying_set_active_set", id))));
    v.push(sub("Active Keying Set", sets));
    v
}
