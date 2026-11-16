// SPDX-License-Identifier: GPL-3.0-only
//! The 3D Viewport's own widgets: the header, the toolbar, the active tool's
//! transform gizmo and the navigation gizmo with its buttons.
use super::icons::{self, Icon};
use super::proj::{seg_dist, Proj};
use super::{menus, modal_header, set_axis_view, tag, tag_open, Modal, Shading, State};
use crate::doc::{Doc, Mode, SelectMode, Tool};
use crate::draw::{Align, Rect};
use crate::editors::header_menus;
use crate::input::{Button, Input};
use crate::theme as t;
use crate::transform::{orientation_basis, TMode};
use crate::ui::{MenuItem, Op, Ui};
use sex_3d::Vec3;

// ---------------------------------------------------------------- header

/// A toggle button with an icon (Blender's header icon toggles).
fn icon_toggle(ui: &mut Ui, r: Rect, i: Icon, on: bool) -> bool {
    let hover = ui.hovered(r);
    ui.p.round_rect(r, 4, if on { t::SELECTED } else if hover { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
    let (cx, cy) = r.center();
    icons::icon(ui.p, i, cx as f32, cy as f32, if on { t::TEXT_HI } else { t::TEXT });
    ui.clicked(r)
}

/// The small dropdown arrow button beside a toggle.
fn arrow_button(ui: &mut Ui, r: Rect) -> bool {
    let hover = ui.hovered(r);
    ui.p.round_rect(r, 4, if hover { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
    ui.p.arrow((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32, 7.0, true, t::TEXT_DIM);
    ui.clicked(r)
}

/// One segment of a joined button row: rounded only at the row's ends.
fn segment_bg(ui: &mut Ui, seg: Rect, i: usize, n: usize, color: u32) {
    let l = if i == 0 { 0 } else { 6 };
    let rr = if i + 1 == n { 0 } else { 6 };
    ui.p.push_clip(seg);
    ui.p.round_rect(Rect::new(seg.x - l, seg.y, seg.w + l + rr, seg.h), 4, color);
    ui.p.pop_clip();
}

/// A joined row of icon buttons; returns the clicked index.
fn icon_row(ui: &mut Ui, r: Rect, items: &[(Icon, bool)]) -> Option<usize> {
    let n = items.len();
    let w = r.w / n as i32;
    let mut out = None;
    for (i, (ic, on)) in items.iter().enumerate() {
        let seg = Rect::new(r.x + i as i32 * w, r.y, if i + 1 == n { r.w - w * (n as i32 - 1) } else { w - 1 }, r.h);
        let hover = ui.hovered(seg);
        segment_bg(ui, seg, i, n, if *on { t::SELECTED } else if hover { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
        let (cx, cy) = seg.center();
        icons::icon(ui.p, *ic, cx as f32, cy as f32, if *on { t::TEXT_HI } else { t::TEXT });
        if ui.clicked(seg) {
            out = Some(i);
        }
    }
    out
}

/// A dropdown with an icon and (optionally) a label.
fn icon_dropdown(ui: &mut Ui, r: Rect, i: Icon, label: &str) -> bool {
    let hover = ui.hovered(r);
    ui.p.round_rect(r, 4, if hover { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
    icons::icon(ui.p, i, (r.x + 12) as f32, (r.y + r.h / 2) as f32, t::TEXT);
    if !label.is_empty() {
        ui.p.label(Rect::new(r.x + 24, r.y, r.w - 38, r.h), label, t::TEXT, Align::Left);
    }
    ui.p.arrow((r.right() - 9) as f32, (r.y + r.h / 2) as f32, 7.0, true, t::TEXT_DIM);
    ui.clicked(r)
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let at = (hr.x + 1, hr.bottom() + 2);
    if let Some(text) = modal_header(s) {
        ui.p.label(Rect::new(hr.x + 8, hr.y, hr.w - 16, hr.h), &text, t::TEXT, Align::Left);
        return;
    }
    let y = hr.y + 3;
    let h = hr.h - 6;
    // Mode dropdown with its icon.
    let mr = Rect::new(hr.x, y, 124, h);
    if icon_dropdown(ui, mr, if d.mode == Mode::Edit { Icon::EditMode } else { Icon::ObjectMode }, d.mode.label()) {
        ui.open_menu(mr, "", menus::mode_items(d));
    }
    let mut x = mr.right() + 6;
    if super::rigview::mesh_edit(d) {
        let sm = d.select_mode;
        let r = Rect::new(x, y, 78, h);
        if let Some(i) = icon_row(ui, r, &[(Icon::VertSel, sm == SelectMode::Vertex), (Icon::EdgeSel, sm == SelectMode::Edge), (Icon::FaceSel, sm == SelectMode::Face)]) {
            d.queue.push(Op::with("mesh.select_mode", ["VERT", "EDGE", "FACE"][i]));
        }
        x = r.right() + 6;
    }
    let flags = menus::ViewFlags { toolbar: s.show_toolbar, sidebar: s.show_sidebar };
    let view = move |d: &Doc| menus::view_menu(d, &flags);
    let rig_menus = super::rigview::header_menus(d);
    let paint_menus = super::sculptview::header_menus(d);
    let gp_menus = super::gpview::header_menus(d);
    let menu_end = if let Some(extra) = &gp_menus {
        let mut list: Vec<(&str, &dyn Fn(&Doc) -> Vec<MenuItem>)> = vec![("View", &view)];
        for (t, f) in extra {
            list.push((t, f));
        }
        let end = header_menus(ui, hr, x, &list, d);
        super::gpview::tool_header(s, d, ui, Rect::new(end + 12, y, (hr.right() - end - 12 - 340).max(0), h))
    } else if let Some(extra) = &paint_menus {
        let mut list: Vec<(&str, &dyn Fn(&Doc) -> Vec<MenuItem>)> = vec![("View", &view)];
        for (t, f) in extra {
            list.push((t, f));
        }
        let end = header_menus(ui, hr, x, &list, d);
        super::sculptview::tool_header(s, d, ui, Rect::new(end + 12, y, (hr.right() - end - 12 - 340).max(0), h))
    } else if let Some(extra) = &rig_menus {
        let mut list: Vec<(&str, &dyn Fn(&Doc) -> Vec<MenuItem>)> = vec![("View", &view)];
        for (t, f) in extra {
            list.push((t, f));
        }
        let end = header_menus(ui, hr, x, &list, d);
        weight_tool_header(d, ui, Rect::new(end + 12, y, hr.right() - end - 12, h))
    } else if super::rigview::mesh_edit(d) {
        let list: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 8] = [
            ("View", &view),
            ("Select", &menus::select_menu),
            ("Add", &menus::add_menu),
            ("Mesh", &menus::mesh_menu),
            ("Vertex", &menus::vertex_menu),
            ("Edge", &menus::edge_menu),
            ("Face", &menus::face_menu),
            ("UV", &menus::uv_menu),
        ];
        header_menus(ui, hr, x, &list, d)
    } else {
        let list: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 4] = [("View", &view), ("Select", &menus::select_menu), ("Add", &menus::add_menu), ("Object", &menus::object_menu)];
        header_menus(ui, hr, x, &list, d)
    };
    tag_open(ui, at);

    // Right side, from the right edge leftwards.
    let mut rx = hr.right() - 4;
    let mut take = |w: i32, gap: i32| {
        rx -= w;
        let r = Rect::new(rx, y, w, h);
        rx -= gap;
        r
    };
    let shading_arrow = take(18, 1);
    let shading = take(100, 6);
    let xray = take(28, 8);
    let overlay_arrow = take(18, 1);
    let overlay = take(28, 4);
    let gizmo_arrow = take(18, 1);
    let gizmo = take(28, 8);
    let vis = take(40, 10);
    let right_start = rx;
    let sh = s.shading;
    if let Some(i) = icon_row(ui, shading, &[(Icon::Wireframe, sh == Shading::Wireframe), (Icon::Solid, sh == Shading::Solid), (Icon::Material, sh == Shading::Material), (Icon::Rendered, sh == Shading::Rendered)]) {
        s.shading = [Shading::Wireframe, Shading::Solid, Shading::Material, Shading::Rendered][i];
    }
    if arrow_button(ui, shading_arrow) {
        let items = vec![
            MenuItem::Heading("Viewport Shading".into()),
            MenuItem::Check("Backface Culling".into(), s.backface_culling, Op::with("view3d.overlay", "backface")),
            MenuItem::Check("X-Ray".into(), s.xray, Op::new("view3d.toggle_xray")),
            MenuItem::Separator,
            solid_lighting_menu(s),
            solid_color_menu(s),
            MenuItem::Check("Specular Lighting".into(), s.solid.specular, Op::with("view3d.solid", "specular")),
            MenuItem::Check("Shadow".into(), s.solid.shadow, Op::with("view3d.solid", "shadow")),
            MenuItem::Check("Cavity".into(), s.solid.cavity, Op::with("view3d.solid", "cavity")),
            MenuItem::Check("Outline".into(), s.solid.outline, Op::with("view3d.solid", "outline")),
            MenuItem::Separator,
            MenuItem::op("Wireframe", "", Op::with("view3d.shading", "WIREFRAME")),
            MenuItem::op("Solid", "", Op::with("view3d.shading", "SOLID")),
            MenuItem::op("Material Preview", "", Op::with("view3d.shading", "MATERIAL")),
            MenuItem::op("Rendered", "", Op::with("view3d.shading", "RENDERED")),
        ];
        open_tagged(ui, shading_arrow, items, at);
    }
    if icon_toggle(ui, xray, Icon::XRay, s.xray_on()) {
        s.xray = !s.xray;
    }
    if icon_toggle(ui, overlay, Icon::Overlays, s.show_overlays) {
        s.show_overlays = !s.show_overlays;
    }
    if arrow_button(ui, overlay_arrow) {
        let c = |l: &str, on: bool, a: &str| MenuItem::Check(l.into(), on, Op::with("view3d.overlay", a));
        let items = vec![
            MenuItem::Heading("Viewport Overlays".into()),
            c("Floor", s.show_floor, "floor"),
            c("Text Info", s.show_text, "text"),
            c("Statistics", s.show_stats, "stats"),
            c("3D Cursor", s.show_cursor, "cursor"),
            c("Wireframe", s.show_wire, "wire"),
        ];
        open_tagged(ui, overlay_arrow, items, at);
    }
    if icon_toggle(ui, gizmo, Icon::Gizmo, s.show_gizmo) {
        s.show_gizmo = !s.show_gizmo;
    }
    if arrow_button(ui, gizmo_arrow) {
        let c = |l: &str, on: bool, a: &str| MenuItem::Check(l.into(), on, Op::with("view3d.overlay", a));
        let items = vec![MenuItem::Heading("Viewport Gizmos".into()), c("Navigate", s.show_nav_gizmo, "gizmo_nav"), c("Active Tools", s.show_tool_gizmo, "gizmo_tool")];
        open_tagged(ui, gizmo_arrow, items, at);
    }
    if icon_dropdown(ui, vis, Icon::Visibility, "") {
        let na = |l: &str| MenuItem::Check(l.into(), true, Op::new("view3d.object_type_visibility"));
        open_tagged(ui, vis, vec![MenuItem::Heading("Object Types Visibility".into()), na("Mesh"), na("Camera"), na("Light"), na("Empty")], at);
    }

    // The middle: orientation, pivot, snapping, proportional editing.
    if paint_menus.is_some() {
        return;
    }
    let cw = 96 + 4 + 36 + 8 + 28 + 1 + 34 + 8 + 28 + 1 + 18;
    let cx = (hr.x + hr.w / 2 - cw / 2).max(menu_end + 12);
    if cx + cw > right_start {
        return;
    }
    let mut x = cx;
    let or = Rect::new(x, y, 96, h);
    if icon_dropdown(ui, or, Icon::Orientation, d.orientation.label()) {
        ui.open_menu(or, "", menus::orientation_items(d));
    }
    x += 96 + 4;
    let pr = Rect::new(x, y, 36, h);
    if icon_dropdown(ui, pr, Icon::Pivot, "") {
        ui.open_menu(pr, "", menus::pivot_items(d));
    }
    x += 36 + 8;
    if icon_toggle(ui, Rect::new(x, y, 28, h), Icon::Magnet, d.snap) {
        d.snap = !d.snap;
    }
    x += 29;
    let sr = Rect::new(x, y, 34, h);
    if icon_dropdown(ui, sr, Icon::Grid, "") {
        ui.open_menu(sr, "", menus::snap_target_items(d));
    }
    x += 34 + 8;
    if icon_toggle(ui, Rect::new(x, y, 28, h), Icon::Proportional, d.proportional) {
        d.proportional = !d.proportional;
    }
    x += 29;
    let fr = Rect::new(x, y, 18, h);
    if arrow_button(ui, fr) {
        ui.open_menu(fr, "", menus::falloff_items(d));
    }
}

fn open_tagged(ui: &mut Ui, r: Rect, items: Vec<MenuItem>, at: (i32, i32)) {
    let mut items = items;
    tag(&mut items, at);
    // Right-aligned under its button, like Blender's popovers.
    ui.open_menu(r, "", items);
    if let Some(p) = &mut ui.st.open_popup {
        p.x = r.right() - 200;
    }
}

// ---------------------------------------------------------------- toolbar

const BS: i32 = 28;
const GAP: i32 = 8;

/// The toolbar's tools in order; None is a gap between groups.
pub fn toolbar_tools(d: &Doc) -> Vec<Option<Tool>> {
    use Tool as T;
    let mut v = vec![Some(T::SelectBox), Some(T::Cursor), None, Some(T::Move), Some(T::Rotate), Some(T::Scale), Some(T::Transform), None, Some(T::Annotate), Some(T::Measure), None, Some(T::AddCube)];
    if d.mode == Mode::Edit {
        v.extend([
            None,
            Some(T::Extrude),
            Some(T::Inset),
            Some(T::Bevel),
            Some(T::LoopCut),
            Some(T::Knife),
            Some(T::PolyBuild),
            Some(T::Spin),
            None,
            Some(T::Smooth),
            Some(T::EdgeSlide),
            Some(T::ShrinkFatten),
            Some(T::Shear),
            Some(T::Rip),
        ]);
    }
    v
}

pub fn toolbar_rect(d: &Doc, r: Rect) -> Rect {
    if super::gpview::active(d) {
        return super::gpview::toolbar_rect(d, r);
    }
    if super::sculptview::active(d) {
        return super::sculptview::toolbar_rect(d, r);
    }
    let tools = toolbar_tools(d);
    let h: i32 = tools.iter().map(|t| if t.is_some() { BS + 2 } else { GAP }).sum::<i32>() + 8;
    Rect::new(r.x + 4, r.y + 6, BS + 8, h.min(r.h - 10))
}

pub fn toolbar(_s: &mut State, d: &mut Doc, ui: &mut Ui, tr: Rect, _r: Rect) {
    if super::gpview::active(d) {
        super::gpview::toolbar(d, ui, tr);
        return;
    }
    if super::sculptview::active(d) {
        super::sculptview::toolbar(d, ui, tr);
        return;
    }
    let mut y = tr.y + 4;
    let x = tr.x + 4;
    let mut hover_tip: Option<(Rect, &'static str)> = None;
    // Each group sits on its own dark rounded column.
    let tools = toolbar_tools(d);
    let mut group: Vec<(Rect, Tool)> = Vec::new();
    let mut groups: Vec<Vec<(Rect, Tool)>> = Vec::new();
    for t in tools {
        match t {
            Some(t) => {
                group.push((Rect::new(x, y, BS, BS), t));
                y += BS + 2;
            }
            None => {
                groups.push(core::mem::take(&mut group));
                y += GAP;
            }
        }
    }
    groups.push(group);
    ui.p.push_clip(tr);
    for g in &groups {
        if let (Some(f), Some(l)) = (g.first(), g.last()) {
            ui.p.round_rect_alpha(Rect::new(f.0.x - 2, f.0.y - 2, BS + 4, l.0.bottom() - f.0.y + 4), 5, 0x1d1d1d, 200);
        }
    }
    for g in groups {
        for (br, tool) in g {
            let on = d.tool == tool;
            let hover = ui.hovered(br);
            if on {
                ui.p.round_rect(br, 4, t::SELECTED);
            } else if hover {
                ui.p.round_rect(br, 4, 0x3a3a3a);
            }
            let (cx, cy) = br.center();
            icons::tool(ui.p, tool, cx as f32, cy as f32, if on { t::TEXT_HI } else { t::TEXT });
            if hover {
                hover_tip = Some((br, tool.label()));
            }
            if ui.clicked(br) {
                d.tool = tool;
            }
        }
    }
    ui.p.pop_clip();
    if let Some((br, tip)) = hover_tip {
        let w = ui.p.text_width(tip) + 14;
        let r = Rect::new(br.right() + 8, br.y + 3, w, 22);
        ui.p.round_rect(r, 4, t::MENU_BG);
        ui.p.label(r, tip, t::TEXT, Align::Center);
    }
    // Clicks on the toolbar's empty space stay in it.
    if let Some(Input::Down { x, y, .. }) = ui.input {
        if tr.contains(x, y) {
            ui.input = None;
        }
    }
}

// ---------------------------------------------------------------- tool gizmo

#[derive(Clone, Debug)]
enum Shape {
    /// Arrow or line handle from a to b (screen).
    Seg((f32, f32), (f32, f32)),
    /// A ring as screen points (front half only).
    Ring(Vec<(f32, f32)>),
    /// A screen circle (view ring / scale ring).
    Circle((f32, f32), f32),
    /// The filled centre (free move).
    Dot((f32, f32), f32),
    /// A small plane square.
    Square((f32, f32), f32),
}

#[derive(Clone, Debug)]
struct Handle {
    mode: TMode,
    axis: Option<usize>,
    plane: bool,
    shape: Shape,
    color: u32,
    /// Draw a cone (move) or a box (scale) at the end of a Seg.
    tip: u8,
}

const AXIS_COL: [u32; 3] = [t::AXIS_X, t::AXIS_Y, t::AXIS_Z];

fn gizmo_center(d: &Doc) -> Option<Vec3> {
    match d.pivot {
        crate::transform::Pivot::Cursor => Some(d.cursor),
        _ => d.selection_center(),
    }
}

fn handles(s: &State, d: &Doc, p: &Proj) -> Vec<Handle> {
    let mut out = Vec::new();
    let tool = d.tool;
    if !matches!(tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) || matches!(s.modal, Modal::Transform(_)) {
        return out;
    }
    let Some(c) = gizmo_center(d) else { return out };
    let Some(cs) = p.to_screen(c) else { return out };
    let basis = orientation_basis(d, d.orientation, p);
    let k = p.world_per_pixel(c);
    let len = 80.0 * k;
    let mv = matches!(tool, Tool::Move | Tool::Transform);
    let sc = matches!(tool, Tool::Scale | Tool::Transform);
    let rt = matches!(tool, Tool::Rotate | Tool::Transform);
    for (i, axis) in basis.iter().enumerate() {
        let col = AXIS_COL[i];
        if mv {
            if let (Some(a), Some(b)) = (p.to_screen(c + *axis * (len * 0.25)), p.to_screen(c + *axis * len)) {
                out.push(Handle { mode: TMode::Translate, axis: Some(i), plane: false, shape: Shape::Seg(a, b), color: col, tip: 1 });
            }
            // Plane handles: a square between the other two axes.
            let (u, v) = (basis[(i + 1) % 3], basis[(i + 2) % 3]);
            if tool == Tool::Move {
                if let Some(q) = p.to_screen(c + (u + v) * (len * 0.32)) {
                    out.push(Handle { mode: TMode::Translate, axis: Some(i), plane: true, shape: Shape::Square(q, 5.0), color: col, tip: 0 });
                }
            }
        }
        if sc {
            let l = if tool == Tool::Transform { len * 0.6 } else { len };
            if let (Some(a), Some(b)) = (p.to_screen(c + *axis * (len * 0.2)), p.to_screen(c + *axis * l)) {
                out.push(Handle { mode: TMode::Resize, axis: Some(i), plane: false, shape: Shape::Seg(a, b), color: col, tip: 2 });
            }
        }
    }
    if rt {
        let rr = len * 0.85;
        for (i, axis) in basis.iter().enumerate() {
            let (u, v) = (basis[(i + 1) % 3], basis[(i + 2) % 3]);
            let mut pts = Vec::new();
            let n = 64;
            for kk in 0..=n {
                let a = kk as f32 / n as f32 * core::f32::consts::TAU;
                let q = c + (u * a.cos() + v * a.sin()) * rr;
                // Only the half facing the viewer, like Blender.
                let facing = if p.ortho { (q - c).dot(p.back) >= -1e-4 * rr } else { (q - c).dot((p.eye - c).normalize()) >= -1e-4 * rr };
                if facing {
                    if let Some(sq) = p.to_screen(q) {
                        pts.push(sq);
                        continue;
                    }
                }
                if pts.len() >= 2 {
                    out.push(Handle { mode: TMode::Rotate, axis: Some(i), plane: false, shape: Shape::Ring(core::mem::take(&mut pts)), color: AXIS_COL[i], tip: 0 });
                }
                pts.clear();
            }
            if pts.len() >= 2 {
                out.push(Handle { mode: TMode::Rotate, axis: Some(i), plane: false, shape: Shape::Ring(pts), color: AXIS_COL[i], tip: 0 });
            }
            let _ = axis;
        }
        out.push(Handle { mode: TMode::Rotate, axis: None, plane: false, shape: Shape::Circle(cs, 80.0 * 0.85 + 14.0), color: 0xdddddd, tip: 0 });
    }
    if tool == Tool::Scale {
        out.push(Handle { mode: TMode::Resize, axis: None, plane: false, shape: Shape::Circle(cs, 80.0 * 0.85 + 10.0), color: 0xdddddd, tip: 0 });
    }
    if mv {
        out.push(Handle { mode: TMode::Translate, axis: None, plane: false, shape: Shape::Dot(cs, 9.0), color: 0xdddddd, tip: 0 });
    }
    out
}

fn hit(h: &Handle, x: f32, y: f32) -> f32 {
    match &h.shape {
        Shape::Seg(a, b) => seg_dist(x, y, *a, *b),
        Shape::Ring(pts) => pts.windows(2).map(|w| seg_dist(x, y, w[0], w[1])).fold(f32::INFINITY, f32::min),
        Shape::Circle(c, r) => ((x - c.0).hypot(y - c.1) - r).abs(),
        Shape::Dot(c, r) => ((x - c.0).hypot(y - c.1) - r).max(0.0),
        Shape::Square(c, r) => ((x - c.0).abs().max((y - c.1).abs()) - r).max(0.0),
    }
}

/// The gizmo handle under (x, y): (mode, axis, plane).
pub fn gizmo_hit(s: &State, d: &Doc, p: &Proj, x: f32, y: f32) -> Option<(TMode, Option<usize>, bool)> {
    let hs = handles(s, d, p);
    let mut best: Option<(f32, usize)> = None;
    for (i, h) in hs.iter().enumerate() {
        let dist = hit(h, x, y);
        if dist < 7.0 && best.is_none_or(|b| dist < b.0) {
            best = Some((dist, i));
        }
    }
    best.map(|(_, i)| (hs[i].mode, hs[i].axis, hs[i].plane))
}

pub fn draw_tool_gizmo(s: &State, d: &Doc, ui: &mut Ui, p: &Proj) {
    let hs = handles(s, d, p);
    let (mx, my) = ui.mouse();
    let hot = gizmo_hit(s, d, p, mx as f32, my as f32);
    for h in &hs {
        let is_hot = hot.is_some_and(|(m, a, pl)| m == h.mode && a == h.axis && pl == h.plane);
        let col = if is_hot { crate::draw_mix(0xffffff, h.color, 110) } else { h.color };
        match &h.shape {
            Shape::Seg(a, b) => {
                icons::thick(ui.p, a.0, a.1, b.0, b.1, 2.0, col);
                match h.tip {
                    1 => icons::head(ui.p, b.0 + (b.0 - a.0) * 0.18, b.1 + (b.1 - a.1) * 0.18, b.0 - a.0, b.1 - a.1, 12.0, col),
                    2 => ui.p.fill(Rect::new(b.0 as i32 - 4, b.1 as i32 - 4, 9, 9), col),
                    _ => {}
                }
            }
            Shape::Ring(pts) => {
                for w in pts.windows(2) {
                    icons::thick(ui.p, w[0].0, w[0].1, w[1].0, w[1].1, 2.0, col);
                }
            }
            Shape::Circle(c, r) => ui.p.ring(c.0, c.1, *r, col),
            Shape::Dot(c, r) => {
                ui.p.ring(c.0, c.1, *r, col);
                if is_hot {
                    ui.p.circle(c.0, c.1, *r - 2.0, 0xffffff);
                }
            }
            Shape::Square(c, r) => {
                ui.p.fill_alpha(Rect::new((c.0 - r) as i32, (c.1 - r) as i32, (2.0 * r) as i32 + 1, (2.0 * r) as i32 + 1), col, 200);
            }
        }
    }
}

// ---------------------------------------------------------------- navigation gizmo

const NAV_R: f32 = 40.0;

fn nav_center(nav: Rect) -> (f32, f32) {
    ((nav.right() - 52) as f32, (nav.y + 50) as f32)
}

fn nav_buttons(nav: Rect) -> [(f32, f32); 4] {
    let (cx, cy) = nav_center(nav);
    let x = cx + 22.0;
    let y0 = cy + NAV_R + 26.0;
    [(x, y0), (x, y0 + 34.0), (x, y0 + 68.0), (x, y0 + 102.0)]
}

/// Whether (x, y) is on the navigation gizmo or its buttons.
pub fn nav_hit_area(nav: Rect, x: i32, y: i32) -> bool {
    if nav.w == 0 {
        return false;
    }
    let (cx, cy) = nav_center(nav);
    let (fx, fy) = (x as f32, y as f32);
    if (fx - cx).hypot(fy - cy) <= NAV_R + 8.0 {
        return true;
    }
    nav_buttons(nav).iter().any(|b| (fx - b.0).hypot(fy - b.1) <= 15.0)
}

pub fn nav_gizmo(s: &mut State, d: &Doc, ui: &mut Ui, nav: Rect, r: Rect) {
    let (cx, cy) = nav_center(nav);
    let (mx, my) = ui.mouse();
    let (fx, fy) = (mx as f32, my as f32);
    let over = (fx - cx).hypot(fy - cy) <= NAV_R + 8.0 && r.contains(mx, my);
    if over || matches!(s.modal, Modal::Orbit) && ui.st.mouse == (mx, my) && over {
        ui.p.circle(cx, cy, NAV_R + 8.0, crate::draw_mix(0xffffff, t::VIEWPORT, 30));
    }
    let right = s.cam.right();
    let up = s.cam.up();
    let back = s.cam.back();
    let axes = [(Vec3::X, t::AXIS_X, "X", "RIGHT", "LEFT"), (Vec3::Y, t::AXIS_Y, "Y", "BACK", "FRONT"), (Vec3::Z, t::AXIS_Z, "Z", "TOP", "BOTTOM")];
    // Balls: (depth, screen x, y, colour, label, positive, view name).
    let mut balls: Vec<(f32, f32, f32, u32, &str, bool, &str)> = Vec::new();
    for (v, c, l, pos, neg) in axes {
        let (px, py) = (cx + v.dot(right) * NAV_R, cy - v.dot(up) * NAV_R);
        balls.push((v.dot(back), px, py, c, l, true, pos));
        balls.push((-v.dot(back), cx - v.dot(right) * NAV_R, cy + v.dot(up) * NAV_R, c, l, false, neg));
    }
    balls.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
    let mut hot: Option<&str> = None;
    for &(_, px, py, _, _, positive, name) in balls.iter().rev() {
        if (fx - px).hypot(fy - py) <= if positive { 10.0 } else { 8.0 } {
            hot = Some(name);
            break;
        }
    }
    for &(_, px, py, c, l, positive, name) in &balls {
        let is_hot = hot == Some(name);
        if positive {
            ui.p.line(cx, cy, px, py, c);
            ui.p.circle(px, py, 9.0, if is_hot { crate::draw_mix(0xffffff, c, 120) } else { c });
            let w = ui.p.text_width(l);
            ui.p.text(px as i32 - w / 2, py as i32 - ui.p.line_height() / 2, l, 0x000000);
        } else {
            ui.p.circle(px, py, 7.5, crate::draw_mix(c, 0x262626, 110));
            ui.p.ring(px, py, 7.0, if is_hot { 0xffffff } else { crate::draw_mix(c, 0x262626, 200) });
        }
    }
    // The buttons: zoom, pan (drag), camera view, perspective/orthographic.
    let btns = nav_buttons(nav);
    let icons_ = [Icon::Zoom, Icon::Hand, Icon::Camera, Icon::Grid];
    for (i, b) in btns.iter().enumerate() {
        let hover = (fx - b.0).hypot(fy - b.1) <= 15.0;
        let on = (i == 2 && s.camera_view) || (i == 3 && s.cam.ortho);
        ui.p.circle(b.0, b.1, 14.0, if on { t::SELECTED } else if hover { 0x5a5a5a } else { crate::draw_mix(0x000000, t::VIEWPORT, 70) });
        icons::icon(ui.p, icons_[i], b.0, b.1, t::TEXT_HI);
    }
    // Clicks.
    if let Some(Input::Down { x, y, button: Button::Left, .. }) = ui.input {
        let (px, py) = (x as f32, y as f32);
        if let Some(name) = hot.filter(|_| (px, py) == (fx, fy)) {
            ui.input = None;
            let same = s.axis_view.is_some_and(|a| a.to_uppercase() == name);
            set_axis_view(s, if same { super::opposite_axis(s.axis_view.unwrap_or("Front")) } else { name });
            return;
        }
        if (px - cx).hypot(py - cy) <= NAV_R + 8.0 {
            ui.input = None;
            s.last = (x, y);
            s.modal = Modal::Orbit;
            return;
        }
        for (i, b) in btns.iter().enumerate() {
            if (px - b.0).hypot(py - b.1) <= 15.0 {
                ui.input = None;
                s.last = (x, y);
                match i {
                    0 => s.modal = Modal::Zoom,
                    1 => s.modal = Modal::Pan,
                    2 => {
                        if d.scene.active_camera.is_some() {
                            s.camera_view = !s.camera_view;
                        }
                    }
                    _ => {
                        s.cam.ortho = !s.cam.ortho;
                    }
                }
                return;
            }
        }
    }
}

/// Weight Paint's tool settings in the header: brush, weight, radius,
/// strength. Returns the x after them.
fn weight_tool_header(d: &mut Doc, ui: &mut Ui, r: Rect) -> i32 {
    if d.mode != Mode::WeightPaint || r.w < 420 {
        return r.x;
    }
    let mut x = r.x;
    let br = Rect::new(x, r.y, 84, r.h);
    if ui.dropdown(br, d.rig.wp.brush.label()) {
        ui.open_menu(br, "Brush", super::rigview::weight_brush_items(d));
    }
    x += 90;
    let wp = &mut d.rig.wp;
    let mut fields: [(&str, &mut f32, f32, &str); 3] = [("Weight", &mut wp.weight, 0.01, ""), ("Radius", &mut wp.radius, 1.0, " px"), ("Strength", &mut wp.strength, 0.01, "")];
    for (label, v, step, unit) in fields.iter_mut() {
        let fr = Rect::new(x, r.y, 104, r.h);
        if ui.number(crate::ui::id(&format!("wp/{label}")), fr, label, v, *step, unit) {
            **v = if *label == "Radius" { v.clamp(1.0, 500.0) } else { v.clamp(0.0, 1.0) };
        }
        x += 108;
    }
    x
}

/// Solid shading: Lighting (Studio, Flat, MatCap and the matcaps).
fn solid_lighting_menu(s: &State) -> MenuItem {
    use sex_3d::render::workbench::{Lighting, MATCAPS};
    let mut items: Vec<MenuItem> = Lighting::ALL.iter().map(|l| MenuItem::Check(l.label().into(), s.solid.lighting == *l, Op::with("view3d.solid", &format!("lighting:{}", l.label())))).collect();
    items.push(MenuItem::Separator);
    for (i, m) in MATCAPS.iter().enumerate() {
        items.push(MenuItem::Check(format!("MatCap {m}"), s.solid.lighting == Lighting::MatCap && s.solid.matcap == i, Op::with("view3d.solid", &format!("matcap:{i}"))));
    }
    MenuItem::Sub(format!("Lighting: {}", s.solid.lighting.label()), items)
}

/// Solid shading: Color (Material, Object, Attribute, Single, Random, Texture).
fn solid_color_menu(s: &State) -> MenuItem {
    use sex_3d::render::workbench::ColorType;
    let items = ColorType::ALL.iter().map(|c| MenuItem::Check(c.label().into(), s.solid.color == *c, Op::with("view3d.solid", &format!("color:{}", c.label())))).collect();
    MenuItem::Sub(format!("Color: {}", s.solid.color.label()), items)
}
