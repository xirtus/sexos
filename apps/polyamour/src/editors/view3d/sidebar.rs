// SPDX-License-Identifier: GPL-3.0-only
//! The 3D Viewport's sidebar (N): Item (Transform: Location, Rotation, Scale,
//! Dimensions; in Edit Mode the selection's Median), Tool (the active tool and
//! its options) and View (focal length, clipping, the 3D cursor). Fields are
//! Blender number fields: drag to change, click to type; each edit is one
//! undo step.
use super::{SideTab, State};
use crate::doc::{Doc, Mode, Tool};
use crate::draw::{Align, Painter, Rect};
use crate::theme as t;
use crate::ui::{id, MenuItem, Op, Ui};
use sex_3d::{Quat, Vec3};

pub const TAB_W: i32 = 22;

/// XYZ Euler angles (radians) of a rotation, the inverse of `Quat::from_euler_xyz`.
pub fn quat_to_euler(q: Quat) -> Vec3 {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    let r00 = 1.0 - 2.0 * (y * y + z * z);
    let r10 = 2.0 * (x * y + w * z);
    let r20 = 2.0 * (x * z - w * y);
    let r21 = 2.0 * (y * z + w * x);
    let r22 = 1.0 - 2.0 * (x * x + y * y);
    let b = (-r20).clamp(-1.0, 1.0).asin();
    if r20.abs() > 0.9999 {
        // Gimbal lock: put everything in Z.
        let r01 = 2.0 * (x * y - w * z);
        let r11 = 1.0 - 2.0 * (x * x + z * z);
        return Vec3::new(0.0, b, (-r01).atan2(r11));
    }
    Vec3::new(r21.atan2(r22), b, r10.atan2(r00))
}

/// Text rotated a quarter turn counter-clockwise (reads bottom to top), as
/// Blender draws region tab labels.
pub fn vertical_text(p: &mut Painter, x: i32, y_bottom: i32, s: &str, color: u32, bg: u32) {
    let tw = p.text_width(s).max(1);
    let lh = p.line_height().max(1);
    let mut tmp = Painter::new(tw as usize, lh as usize);
    tmp.fill(tmp.bounds(), bg);
    tmp.text(0, 0, s, color);
    let clip = p.clip();
    for ty in 0..lh {
        for tx in 0..tw {
            // (tx, ty) -> (x + ty, y_bottom - tx)
            let (dx, dy) = (x + ty, y_bottom - tx);
            if clip.contains(dx, dy) {
                let i = dy as usize * p.width + dx as usize;
                p.pixels[i] = tmp.pixels[ty as usize * tw as usize + tx as usize];
            }
        }
    }
}

const ROW: i32 = 20;

struct Layout {
    x: i32,
    w: i32,
    y: i32,
}

/// A number field that records one undo step per edit gesture.
#[allow(clippy::too_many_arguments)]
fn num(ui: &mut Ui, s: &mut State, d: &mut Doc, key: &str, r: Rect, label: &str, v: &mut f32, step: f32, unit: &str, undo: &str) -> bool {
    let wid = id(key);
    let changed = ui.number(wid, r, label, v, step, unit);
    if changed {
        let dragging = ui.st.active == Some(wid);
        if !(dragging && s.undo_widget == Some(wid)) {
            d.push_undo(undo);
        }
        s.undo_widget = if dragging { Some(wid) } else { None };
    }
    changed
}

fn label(ui: &mut Ui, l: &mut Layout, text: &str) {
    ui.p.label(Rect::new(l.x, l.y, l.w, ROW), text, t::TEXT, Align::Left);
    l.y += ROW;
}

fn panel(ui: &mut Ui, l: &mut Layout, title: &str, open: &mut bool) -> bool {
    let r = Rect::new(l.x - 6, l.y, l.w + 12, 24);
    ui.p.round_rect_alpha(r, 4, t::PANEL_HEADER, 245);
    ui.p.arrow((r.x + 12) as f32, (r.y + r.h / 2) as f32, 8.0, *open, t::TEXT_DIM);
    ui.p.label(Rect::new(r.x + 22, r.y, r.w - 28, r.h), title, t::TEXT, Align::Left);
    if ui.clicked(r) {
        *open = !*open;
    }
    l.y += 28;
    *open
}

/// Three stacked X/Y/Z fields; returns which one changed and its new value.
#[allow(clippy::too_many_arguments)]
fn xyz(ui: &mut Ui, s: &mut State, d: &mut Doc, l: &mut Layout, key: &str, v: [f32; 3], step: f32, unit: &str, undo: &str) -> Option<(usize, f32)> {
    let mut out = None;
    for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
        let mut val = v[k];
        let r = Rect::new(l.x, l.y, l.w, ROW - 1);
        if num(ui, s, d, &format!("{key}.{k}"), r, axis, &mut val, step, unit, undo) {
            out = Some((k, val));
        }
        l.y += ROW;
    }
    l.y += 6;
    out
}

pub fn draw(s: &mut State, d: &mut Doc, ui: &mut Ui, sr: Rect) {
    if ui.st.active.is_none() {
        s.undo_widget = None;
    }
    // The region: translucent over the view, as Blender's region overlap.
    ui.p.fill_alpha(sr, 0x1d1d1d, 120);
    // Tabs on the outer edge.
    let tabs = Rect::new(sr.right() - TAB_W, sr.y, TAB_W, sr.h);
    ui.p.fill_alpha(tabs, 0x161616, 200);
    let mut ty = sr.y + 6;
    for (tab, name) in [(SideTab::Item, "Item"), (SideTab::Tool, "Tool"), (SideTab::View, "View")] {
        let h = ui.p.text_width(name) + 18;
        let tr = Rect::new(tabs.x + 1, ty, TAB_W - 2, h);
        let on = s.side_tab == tab && d.python.side_tab.is_none();
        let bg = if on { t::TAB_ACTIVE } else if ui.hovered(tr) { 0x2a2a2a } else { 0x1f1f1f };
        ui.p.round_rect(tr, 4, bg);
        let lh = ui.p.line_height();
        vertical_text(ui.p, tr.x + (tr.w - lh) / 2, tr.bottom() - 9, name, if on { t::TEXT_HI } else { t::TEXT_DIM }, bg);
        if ui.clicked(tr) {
            s.side_tab = tab;
            d.python.side_tab = None;
        }
        ty += h + 2;
    }
    // Tabs of panels scripts and add-ons registered (bl_category).
    for name in d.python.sidebar_categories() {
        let h = ui.p.text_width(&name) + 18;
        let tr = Rect::new(tabs.x + 1, ty, TAB_W - 2, h);
        let on = d.python.side_tab.as_deref() == Some(name.as_str());
        let bg = if on { t::TAB_ACTIVE } else if ui.hovered(tr) { 0x2a2a2a } else { 0x1f1f1f };
        ui.p.round_rect(tr, 4, bg);
        let lh = ui.p.line_height();
        vertical_text(ui.p, tr.x + (tr.w - lh) / 2, tr.bottom() - 9, &name, if on { t::TEXT_HI } else { t::TEXT_DIM }, bg);
        if ui.clicked(tr) {
            d.python.side_tab = Some(name.clone());
        }
        ty += h + 2;
    }
    let body = Rect::new(sr.x + 4, sr.y, sr.w - TAB_W - 8, sr.h);
    ui.p.push_clip(body);
    let mut l = Layout { x: body.x + 10, w: body.w - 20, y: body.y + 6 };
    match (d.python.side_tab.clone(), s.side_tab) {
        (Some(cat), _) => {
            let mut y = l.y;
            crate::python::draw_sidebar(ui, d, &cat, l.x, l.w, &mut y);
        }
        (None, SideTab::Item) => item_tab(s, d, ui, &mut l),
        (None, SideTab::Tool) => tool_tab(s, d, ui, &mut l),
        (None, SideTab::View) => view_tab(s, d, ui, &mut l),
    }
    ui.p.pop_clip();
    // Clicks on the sidebar's empty space stay in the sidebar.
    if let Some(crate::input::Input::Down { x, y, .. }) = ui.input {
        if sr.contains(x, y) {
            ui.input = None;
        }
    }
}

fn item_tab(s: &mut State, d: &mut Doc, ui: &mut Ui, l: &mut Layout) {
    let mut open = s.panels[0];
    let open_now = panel(ui, l, "Transform", &mut open);
    s.panels[0] = open;
    if !open_now {
        return;
    }
    if d.mode == Mode::Edit {
        median(s, d, ui, l);
        return;
    }
    let Some(a) = d.active() else {
        label(ui, l, "Nothing selected");
        return;
    };
    let tr = d.scene.objects[a].transform;
    label(ui, l, "Location:");
    if let Some((k, v)) = xyz(ui, s, d, l, &format!("v3d/{a}/loc"), tr.location.to_array(), 0.1, "m", "Location") {
        let mut p = tr.location.to_array();
        p[k] = v;
        d.scene.objects[a].transform.location = Vec3::from_array(p);
        d.touch();
    }
    label(ui, l, "Rotation:");
    let e = quat_to_euler(tr.rotation);
    let deg = [e.x.to_degrees(), e.y.to_degrees(), e.z.to_degrees()];
    if let Some((k, v)) = xyz(ui, s, d, l, &format!("v3d/{a}/rot"), deg, 5.0, "°", "Rotation") {
        let mut r = deg;
        r[k] = v;
        d.scene.objects[a].transform.rotation = Quat::from_euler_xyz(r[0].to_radians(), r[1].to_radians(), r[2].to_radians());
        d.touch();
    }
    let er = Rect::new(l.x, l.y, l.w, ROW - 1);
    if ui.dropdown(er, "XYZ Euler") {
        ui.open_menu(er, "", vec![MenuItem::Check("XYZ Euler".into(), true, Op::new("wm.redraw")), MenuItem::op("Quaternion (WXYZ)", "", Op::new("object.rotation_mode_set"))]);
    }
    l.y += ROW + 8;
    label(ui, l, "Scale:");
    if let Some((k, v)) = xyz(ui, s, d, l, &format!("v3d/{a}/scale"), tr.scale.to_array(), 0.05, "", "Scale") {
        let mut sc = tr.scale.to_array();
        sc[k] = v;
        d.scene.objects[a].transform.scale = Vec3::from_array(sc);
        d.touch();
    }
    // Dimensions: the evaluated mesh's size times the scale.
    let size = s.dims_cache(d, a);
    if let Some(size) = size {
        label(ui, l, "Dimensions:");
        let dims = [size.x * tr.scale.x.abs(), size.y * tr.scale.y.abs(), size.z * tr.scale.z.abs()];
        if let Some((k, v)) = xyz(ui, s, d, l, &format!("v3d/{a}/dim"), dims, 0.1, "m", "Dimensions") {
            let base = size.to_array()[k];
            if base > 1e-9 {
                let mut sc = tr.scale.to_array();
                sc[k] = v / base * if sc[k] < 0.0 { -1.0 } else { 1.0 };
                d.scene.objects[a].transform.scale = Vec3::from_array(sc);
                d.touch();
            }
        }
    }
}

fn median(s: &mut State, d: &mut Doc, ui: &mut Ui, l: &mut Layout) {
    let Some((oi, mi)) = crate::edit::active_mesh(d) else { return };
    let verts = crate::edit::selected_verts(d);
    if verts.is_empty() {
        label(ui, l, "Nothing selected");
        return;
    }
    let w = d.scene.world_matrix(oi);
    let local = verts.iter().fold(Vec3::ZERO, |a, &v| a + d.scene.meshes[mi].positions[v as usize]) / verts.len() as f32;
    let shown = if s.median_global { w.transform_point(local) } else { local };
    label(ui, l, if verts.len() == 1 { "Vertex:" } else { "Median:" });
    if let Some((k, v)) = xyz(ui, s, d, l, "v3d/median", shown.to_array(), 0.1, "m", "Move Vertices") {
        let mut p = shown.to_array();
        p[k] = v;
        let target = Vec3::from_array(p);
        let delta = if s.median_global { w.inverse().transform_vector(target - shown) } else { target - shown };
        for v in verts {
            d.scene.meshes[mi].positions[v as usize] += delta;
        }
        d.touch();
    }
    let mut g = if s.median_global { 1 } else { 0 };
    if ui.segmented(Rect::new(l.x, l.y, l.w, ROW - 1), &["Local", "Global"], &mut g) {
        s.median_global = g == 1;
    }
    l.y += ROW + 6;
}

fn tool_tab(s: &mut State, d: &mut Doc, ui: &mut Ui, l: &mut Layout) {
    if super::sculptview::active(d) {
        l.y = super::sculptview::tool_panels(s, d, ui, l.x, l.w, l.y);
        return;
    }
    let mut open = s.panels[1];
    if panel(ui, l, "Active Tool", &mut open) {
        let r = Rect::new(l.x, l.y, l.w, 28);
        super::icons::tool(ui.p, d.tool, (r.x + 14) as f32, (r.y + 14) as f32, t::TEXT);
        ui.p.label(Rect::new(r.x + 32, r.y, r.w - 32, r.h), d.tool.label(), t::TEXT, Align::Left);
        l.y += 32;
        if d.tool == Tool::SelectBox {
            label(ui, l, "Mode");
            let modes = ["SET", "EXTEND", "SUB", "XOR", "AND"];
            let mut i = match d.box_mode {
                crate::doc::BoxMode::Set => 0,
                crate::doc::BoxMode::Extend => 1,
                crate::doc::BoxMode::Subtract => 2,
                crate::doc::BoxMode::Invert => 3,
                crate::doc::BoxMode::Intersect => 4,
            };
            if ui.segmented(Rect::new(l.x, l.y, l.w, ROW - 1), &["New", "Add", "Sub", "Inv", "And"], &mut i) {
                d.queue.push(Op::with("wm.context_set", &format!("box_mode={}", modes[i])));
            }
            l.y += ROW + 6;
        }
        if matches!(d.tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) {
            label(ui, l, "Orientation");
            let r = Rect::new(l.x, l.y, l.w, ROW - 1);
            if ui.dropdown(r, d.orientation.label()) {
                ui.open_menu(r, "", super::menus::orientation_items(d));
            }
            l.y += ROW + 6;
        }
    }
    s.panels[1] = open;
    let mut open = s.panels[2];
    if panel(ui, l, "Options", &mut open) {
        label(ui, l, "Transform");
        let mut snap = d.snap;
        if ui.checkbox(Rect::new(l.x, l.y, l.w, ROW), "Snapping", &mut snap) {
            d.snap = snap;
        }
        l.y += ROW;
        let mut prop = d.proportional;
        if ui.checkbox(Rect::new(l.x, l.y, l.w, ROW), "Proportional Editing", &mut prop) {
            d.proportional = prop;
        }
        l.y += ROW + 2;
        let r = Rect::new(l.x, l.y, l.w, ROW - 1);
        if ui.dropdown(r, d.falloff.label()) {
            ui.open_menu(r, "", super::menus::falloff_items(d));
        }
        l.y += ROW + 2;
        let mut size = d.proportional_size;
        if num(ui, s, d, "v3d/propsize", Rect::new(l.x, l.y, l.w, ROW - 1), "Size", &mut size, 0.05, "m", "Proportional Size") {
            d.proportional_size = size.max(1e-3);
        }
        l.y += ROW + 6;
    }
    s.panels[2] = open;
}

fn view_tab(s: &mut State, d: &mut Doc, ui: &mut Ui, l: &mut Layout) {
    let mut open = s.panels[3];
    if panel(ui, l, "View", &mut open) {
        let mut lens = s.lens;
        let r = Rect::new(l.x, l.y, l.w, ROW - 1);
        if ui.number(id("v3d/lens"), r, "Focal Length", &mut lens, 1.0, "mm") {
            s.lens = lens.clamp(1.0, 5000.0);
        }
        l.y += ROW + 6;
        label(ui, l, "Clip");
        let mut cs = s.clip_start;
        if ui.number(id("v3d/clip0"), Rect::new(l.x, l.y, l.w, ROW - 1), "Start", &mut cs, 0.01, "m") {
            s.clip_start = cs.max(1e-4);
        }
        l.y += ROW;
        let mut ce = s.clip_end;
        if ui.number(id("v3d/clip1"), Rect::new(l.x, l.y, l.w, ROW - 1), "End", &mut ce, 10.0, "m") {
            s.clip_end = ce.max(s.clip_start + 1e-3);
        }
        l.y += ROW + 8;
    }
    s.panels[3] = open;
    let mut open = s.panels[4];
    if panel(ui, l, "3D Cursor", &mut open) {
        label(ui, l, "Location:");
        if let Some((k, v)) = xyz(ui, s, d, l, "v3d/cursor", d.cursor.to_array(), 0.1, "m", "Cursor") {
            let mut p = d.cursor.to_array();
            p[k] = v;
            d.cursor = Vec3::from_array(p);
        }
        label(ui, l, "Rotation:");
        let r = d.cursor_rotation;
        let deg = [r.x.to_degrees(), r.y.to_degrees(), r.z.to_degrees()];
        if let Some((k, v)) = xyz(ui, s, d, l, "v3d/cursor_rot", deg, 5.0, "°", "Cursor") {
            let mut a = r.to_array();
            a[k] = v.to_radians();
            d.cursor_rotation = Vec3::from_array(a);
        }
    }
    s.panels[4] = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_round_trip() {
        for e in [Vec3::new(0.3, -0.2, 1.1), Vec3::new(0.0, 0.0, 1.5707964), Vec3::new(-1.0, 0.5, 0.25)] {
            let q = Quat::from_euler_xyz(e.x, e.y, e.z);
            let back = quat_to_euler(q);
            assert!(back.approx_eq(e, 1e-4), "{e:?} -> {back:?}");
        }
    }
}
