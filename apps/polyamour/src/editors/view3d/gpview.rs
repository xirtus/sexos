// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil in the 3D Viewport: drawing strokes and primitives, Fill,
//! Erase, Tint and the Eyedropper in Draw Mode; selection and G/R/S/E in
//! Edit Mode; sculpt, vertex-colour and weight brushes; the toolbar, tool
//! header and header menus of those modes; compositing the strokes into the
//! viewport image. Pixel work happens in the view rectangle's own
//! coordinates (`GpView`), so window positions are shifted by `s.rect`.
use super::{Modal, State};
use crate::doc::{Doc, Mode};
use crate::draw::{Align, Painter, Rect};
use crate::editors::properties::picker::Picker;
use crate::editors::properties::ColorTarget;
use crate::gpencil::ui::{GpOwner, PropColor};
use crate::gpencil::{self as g, DrawTool, EditTool};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{id, MenuItem, NumOpts, Op, Popup, Ui};
use sex_3d::grease_pencil::edit::{self as gedit, SelectAction, SelectMode};
use sex_3d::grease_pencil::raster::{self, SceneDraw};
use sex_3d::grease_pencil::sculpt::{self as gsculpt, Dab, SculptStroke, SculptTool, VertexTool, WeightTool};
use sex_3d::grease_pencil::tools::{self as gtools, EraseMode, Primitive, StrokeBuilder};
use sex_3d::grease_pencil::view::{DrawPlane, GpView, Placement, Projector};
use sex_3d::grease_pencil::{layer_world_matrix, GpStroke, GreasePencil};
use sex_3d::undo::Change;
use sex_3d::{Mat4, Quat, Vec3};

/// True in a Grease Pencil mode other than Object Mode.
pub fn active(d: &Doc) -> bool {
    d.mode != Mode::Object && g::in_mode(d)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DabKind {
    Erase,
    Tint,
    Sculpt,
    Vertex,
    Weight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XKind {
    Move,
    Rotate,
    Scale,
    Radius,
}

/// Grease Pencil modal states (positions are view-local pixels).
pub enum GModal {
    Draw { sb: Box<StrokeBuilder> },
    Prim { kind: Primitive, ctrl: Vec<(f32, f32)>, shift: bool, dragging: bool },
    Dab { kind: DabKind, before: Box<GreasePencil>, gi: usize, last: (f32, f32), sst: SculptStroke, invert: bool, changed: bool },
    Xform { kind: XKind, before: Box<GreasePencil>, gi: usize, start: (f32, f32), center: Vec3, axis: Option<usize>, mouse: (f32, f32) },
    Box { a: (f32, f32), b: (f32, f32), action: SelectAction, fresh: bool },
    Size { strength: bool, x0: f32, orig: f32 },
}

fn view(s: &State) -> GpView {
    GpView::from_viewport(&s.cam, s.rect.w.max(1) as u32, s.rect.h.max(1) as u32)
}

fn local(s: &State, x: i32, y: i32) -> (f32, f32) {
    ((x - s.rect.x) as f32, (y - s.rect.y) as f32)
}

/// The active layer if it can take new strokes: (object, data, layer).
fn target_layer(d: &mut Doc) -> Option<(usize, usize, usize)> {
    let (oi, gi) = g::active_gp(d)?;
    let gpd = &mut d.scene.grease_pencils[gi];
    if gpd.layers.is_empty() {
        gpd.add_layer("Layer");
    }
    let li = gpd.active_layer.min(gpd.layers.len() - 1);
    if !gpd.layer_visible(li) || gpd.layer_locked(li) {
        d.report = "Active layer is locked or hidden".into();
        return None;
    }
    Some((oi, gi, li))
}

fn cursor_rot(d: &Doc) -> Mat4 {
    let r = d.cursor_rotation;
    Mat4::from_trs(Vec3::ZERO, Quat::from_euler_xyz(r.x, r.y, r.z), Vec3::ONE)
}

/// Depth (distance along the view) of the viewport surface at a pixel.
fn depth_lookup(s: &State) -> impl Fn(f32, f32) -> Option<f32> + '_ {
    let w = s.rect.w.max(0) as usize;
    move |x: f32, y: f32| {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let i = y as usize * w + x as usize;
        s.gp_depth.get(i).copied().filter(|d| d.is_finite())
    }
}

/// Run `f` with a projector for layer `li` of object `oi`.
fn with_projector<R>(s: &State, d: &Doc, oi: usize, gi: usize, li: usize, f: impl FnOnce(&Projector<'_>) -> R) -> R {
    let v = view(s);
    let dl = depth_lookup(s);
    let proj = Projector {
        view: &v,
        plane: d.gp.plane,
        placement: d.gp.placement,
        object: layer_world_matrix(&d.scene, oi, &d.scene.grease_pencils[gi], li),
        cursor: d.cursor,
        cursor_rot: cursor_rot(d),
        depth_at: Some(&dl),
        surface_offset: 0.0,
    };
    f(&proj)
}

/// Add a finished stroke to the active layer's key at the current frame.
fn add_stroke(d: &mut Doc, gi: usize, li: usize, stroke: GpStroke, back: bool, label: &str) {
    let before = Box::new(d.scene.grease_pencils[gi].clone());
    let (frame, additive) = (d.frame, d.gp.additive);
    let l = &mut d.scene.grease_pencils[gi].layers[li];
    let fi = l.ensure_key(frame, additive);
    let strokes = &mut l.frames[fi].drawing.strokes;
    if back {
        strokes.insert(0, stroke);
    } else {
        strokes.push(stroke);
    }
    d.push_undo_patch(label, vec![Change::GreasePencil { gp: gi, data: before }]);
    d.touch();
}

fn material(d: &Doc, gi: usize) -> usize {
    let gpd = &d.scene.grease_pencils[gi];
    gpd.active_material.min(gpd.materials.len().saturating_sub(1))
}

fn start_dab(s: &mut State, d: &mut Doc, kind: DabKind, at: (f32, f32), invert: bool) {
    let Some((_, gi)) = g::active_gp(d) else { return };
    if kind == DabKind::Weight && d.scene.grease_pencils[gi].vertex_groups.is_empty() {
        d.scene.grease_pencils[gi].vertex_groups.push("Group".into());
    }
    let before = Box::new(d.scene.grease_pencils[gi].clone());
    let mut m = GModal::Dab { kind, before, gi, last: at, sst: SculptStroke::default(), invert, changed: false };
    dab(s, d, &mut m, at);
    s.modal = Modal::Gp(Box::new(m));
}

/// One brush dab at `at` on every editable drawing.
fn dab(s: &State, d: &mut Doc, m: &mut GModal, at: (f32, f32)) {
    let GModal::Dab { kind, last, sst, invert, changed, .. } = m else { return };
    let v = view(s);
    let mode = d.mode;
    let st = d.gp.clone();
    let size = match kind {
        DabKind::Erase => st.erase,
        DabKind::Tint => st.tint,
        DabKind::Sculpt => st.sculpt,
        DabKind::Vertex => st.vertex,
        DabKind::Weight => st.weight,
    };
    let dabv = Dab { at, prev: *last, radius: size.radius, strength: size.strength, invert: *invert };
    let group = g::active_gp(d).map_or(0, |(_, gi)| d.scene.grease_pencils[gi].active_vertex_group as u32);
    let kind = *kind;
    let mut any = false;
    let clip = st.clipboard.clone();
    g::each_drawing(d, |dr, w| {
        let c = match kind {
            DabKind::Erase => gtools::erase(dr, &v, w, at, size.radius, st.erase_mode, size.strength),
            DabKind::Tint => gtools::tint(dr, &v, w, at, size.radius, size.strength, st.tint_color, st.vertex_stroke, st.vertex_fill),
            DabKind::Sculpt => gsculpt::sculpt_dab(dr, &v, w, st.sculpt_tool, &dabv, sst, &clip),
            DabKind::Vertex => gsculpt::vertex_dab(dr, &v, w, st.vertex_tool, &dabv, st.vertex_color, st.vertex_fill),
            DabKind::Weight => gsculpt::weight_dab(dr, &v, w, st.weight_tool, &dabv, group, st.weight_value),
        };
        any |= c;
        c
    });
    let _ = mode;
    if any {
        *changed = true;
        d.touch();
    }
    *last = at;
}

/// Selected points' centre in world space.
fn selection_center(d: &Doc) -> Option<Vec3> {
    let (oi, gi) = g::active_gp(d)?;
    let mut sum = Vec3::ZERO;
    let mut n = 0.0;
    for (li, fi, m) in g::editable(d, oi, gi) {
        for s in &d.scene.grease_pencils[gi].layers[li].frames[fi].drawing.strokes {
            for p in s.points.iter().filter(|p| p.select) {
                sum += m.transform_point(p.position);
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| sum / n)
}

/// G, R, S or Alt+S on the selected points.
pub fn start_xform(s: &mut State, d: &mut Doc, kind: XKind, mouse: (f32, f32)) -> bool {
    let Some((_, gi)) = g::active_gp(d) else { return false };
    let Some(center) = selection_center(d) else {
        d.report = "Nothing selected".into();
        return true;
    };
    let before = Box::new(d.scene.grease_pencils[gi].clone());
    s.modal = Modal::Gp(Box::new(GModal::Xform { kind, before, gi, start: mouse, center, axis: None, mouse }));
    true
}

fn apply_xform(s: &State, d: &mut Doc, m: &GModal) {
    let GModal::Xform { kind, before, gi, start, center, axis, mouse } = m else { return };
    d.scene.grease_pencils[*gi] = (**before).clone();
    let v = view(s);
    let Some((cx, cy, _)) = v.project(*center) else { return };
    let (dx, dy) = (mouse.0 - start.0, mouse.1 - start.1);
    let center = *center;
    let axis_v = axis.map(|a| [Vec3::X, Vec3::Y, Vec3::Z][a]);
    match kind {
        XKind::Move => {
            let mut delta = v.pixel_delta(dx, dy, center);
            if let Some(a) = axis_v {
                delta = a * delta.dot(a);
            }
            g::each_drawing(d, |dr, w| {
                let inv = w.inverse();
                gedit::transform_selected(dr, &|p| inv.transform_point(w.transform_point(p) + delta));
                true
            });
        }
        XKind::Rotate => {
            let a0 = (start.1 - cy).atan2(start.0 - cx);
            let a1 = (mouse.1 - cy).atan2(mouse.0 - cx);
            // Screen y points down: a clockwise drag turns clockwise on screen.
            let ang = -(a1 - a0);
            let ax = axis_v.unwrap_or(v.back);
            let q = Quat::from_axis_angle(ax, ang);
            g::each_drawing(d, |dr, w| {
                let inv = w.inverse();
                gedit::transform_selected(dr, &|p| inv.transform_point(center + q.rotate(w.transform_point(p) - center)));
                true
            });
        }
        XKind::Scale | XKind::Radius => {
            let r0 = (start.0 - cx).hypot(start.1 - cy).max(1.0);
            let f = (mouse.0 - cx).hypot(mouse.1 - cy) / r0;
            if *kind == XKind::Radius {
                g::each_drawing(d, |dr, _| {
                    gedit::scale_radius(dr, f);
                    true
                });
            } else {
                let sv = match axis_v {
                    Some(a) => Vec3::ONE + a * (f - 1.0),
                    None => Vec3::splat(f),
                };
                g::each_drawing(d, |dr, w| {
                    let inv = w.inverse();
                    gedit::transform_selected(dr, &|p| {
                        let q = w.transform_point(p) - center;
                        inv.transform_point(center + Vec3::new(q.x * sv.x, q.y * sv.y, q.z * sv.z))
                    });
                    true
                });
            }
        }
    }
    d.touch();
}

/// The header text while a Grease Pencil modal runs.
pub fn header(m: &GModal) -> Option<String> {
    Some(match m {
        GModal::Xform { kind, axis, .. } => {
            let ax = axis.map_or(String::new(), |a| format!("  along {}", ["X", "Y", "Z"][a]));
            format!("{}{ax}   (X/Y/Z constrain, click or Enter to confirm, Esc to cancel)", match kind {
                XKind::Move => "Move",
                XKind::Rotate => "Rotate",
                XKind::Scale => "Resize",
                XKind::Radius => "Radius",
            })
        }
        GModal::Prim { kind: Primitive::Polyline, ctrl, .. } => format!("Polyline: click to add points ({}), Enter or right-click to finish, Esc to cancel", ctrl.len()),
        GModal::Prim { kind, .. } => format!("{}: drag, Shift to constrain", kind.label()),
        _ => return None,
    })
}

// ---------------------------------------------------------------- press

/// Left button down over the view. True when Grease Pencil took it.
pub fn press(s: &mut State, d: &mut Doc, x: i32, y: i32, mods: Mods) -> bool {
    if !active(d) {
        return false;
    }
    let at = local(s, x, y);
    match d.mode {
        Mode::Draw => press_draw(s, d, at, mods),
        Mode::Edit => press_edit(s, d, at, mods),
        Mode::Sculpt => start_dab(s, d, DabKind::Sculpt, at, mods.ctrl),
        Mode::VertexPaint => start_dab(s, d, DabKind::Vertex, at, mods.ctrl),
        Mode::WeightPaint => start_dab(s, d, DabKind::Weight, at, mods.ctrl),
        _ => return false,
    }
    true
}

fn press_draw(s: &mut State, d: &mut Doc, at: (f32, f32), mods: Mods) {
    let tool = d.gp.tool;
    match tool {
        DrawTool::Draw if mods.ctrl => start_dab(s, d, DabKind::Erase, at, false),
        DrawTool::Draw => {
            if target_layer(d).is_none() {
                return;
            }
            let mut b = d.gp.brush.clone();
            if mods.shift {
                b.use_stabilizer = !b.use_stabilizer;
            }
            s.modal = Modal::Gp(Box::new(GModal::Draw { sb: Box::new(StrokeBuilder::new(b, at)) }));
        }
        DrawTool::Erase => start_dab(s, d, DabKind::Erase, at, false),
        DrawTool::Tint => start_dab(s, d, DabKind::Tint, at, false),
        DrawTool::Fill => fill_at(s, d, at),
        DrawTool::Eyedropper => eyedropper(s, d, at),
        DrawTool::Primitive(kind) => {
            if target_layer(d).is_none() {
                return;
            }
            s.modal = Modal::Gp(Box::new(GModal::Prim { kind, ctrl: vec![at, at], shift: mods.shift, dragging: true }));
        }
    }
}

/// The Fill tool: close the region under the click with a fill stroke.
pub fn fill_at(s: &State, d: &mut Doc, at: (f32, f32)) {
    let Some((oi, gi, li)) = target_layer(d) else { return };
    let v = view(s);
    let mut bounds = Vec::new();
    let gpd = &d.scene.grease_pencils[gi];
    for l in 0..gpd.layers.len() {
        if !gpd.layer_visible(l) {
            continue;
        }
        if let Some(fi) = gpd.layers[l].frame_index_at(d.frame) {
            let w = layer_world_matrix(&d.scene, oi, gpd, l);
            bounds.extend(gtools::screen_strokes(&v, &w, &gpd.layers[l].frames[fi].drawing));
        }
    }
    let mat = fill_material(d, gi);
    let fs = d.gp.fill.clone();
    let brush = d.gp.brush.clone();
    let r = with_projector(s, d, oi, gi, li, |p| gtools::fill(&v, &bounds, at, &fs, &brush, p, mat));
    match r {
        Ok(stroke) => add_stroke(d, gi, li, stroke, fs.on_back, "Fill"),
        Err(gtools::FillError::Leak) => d.report = "Unable to fill unclosed areas".into(),
        Err(gtools::FillError::OnBoundary) => d.report = "Click inside a closed area to fill it".into(),
    }
}

/// A material that shows fills: the active one if it does, otherwise the
/// first that does, otherwise a new fill material.
fn fill_material(d: &mut Doc, gi: usize) -> usize {
    let a = material(d, gi);
    let gpd = &mut d.scene.grease_pencils[gi];
    if gpd.materials.get(a).is_some_and(|m| m.show_fill) {
        return a;
    }
    if let Some(i) = gpd.materials.iter().position(|m| m.show_fill) {
        return i;
    }
    gpd.materials.push(sex_3d::grease_pencil::GpMaterial::fill("Fill", [0.5, 0.5, 0.5, 1.0]));
    gpd.materials.len() - 1
}

fn eyedropper(s: &State, d: &mut Doc, at: (f32, f32)) {
    let w = s.rect.w.max(0) as usize;
    let i = (at.1 as usize * w + at.0 as usize) * 4;
    let Some(px) = s.buf.get(i..i + 3) else { return };
    let c = [px[0], px[1], px[2]].map(|v| sex_3d::render::srgb_decode(v as f32 / 255.0));
    d.gp.brush.vertex_color = [c[0], c[1], c[2], 1.0];
    d.gp.vertex_color = c;
    d.report = format!("Picked color {:.2}, {:.2}, {:.2}", c[0], c[1], c[2]);
}

fn press_edit(s: &mut State, d: &mut Doc, at: (f32, f32), mods: Mods) {
    match d.gp.edit_tool {
        EditTool::Cursor => {
            let v = view(s);
            let n = v.back;
            d.cursor = v.on_plane(at.0, at.1, d.cursor, n).unwrap_or(d.cursor);
        }
        EditTool::Extrude => {
            if extrude(d) {
                start_xform(s, d, XKind::Move, at);
            }
        }
        EditTool::Radius => {
            start_xform(s, d, XKind::Radius, at);
        }
        EditTool::SelectBox => {
            let action = if mods.ctrl { SelectAction::Deselect } else { SelectAction::Select };
            s.modal = Modal::Gp(Box::new(GModal::Box { a: at, b: at, action, fresh: !mods.shift && !mods.ctrl }));
        }
    }
}

fn extrude(d: &mut Doc) -> bool {
    d.push_undo("Extrude");
    let n = g::each_drawing(d, |dr, _| gedit::extrude(dr) > 0);
    if n {
        d.touch();
    }
    n
}

fn select_mode(d: &Doc) -> SelectMode {
    d.gp.select_mode
}

fn finish_box(s: &State, d: &mut Doc, a: (f32, f32), b: (f32, f32), action: SelectAction, fresh: bool) {
    let v = view(s);
    let mode = select_mode(d);
    let click = (a.0 - b.0).abs() < 3.0 && (a.1 - b.1).abs() < 3.0;
    if click {
        let mut hit = false;
        g::each_drawing(d, |dr, w| {
            hit |= gedit::select_pick(dr, &v, w, b, 12.0, !fresh, mode);
            true
        });
        if !hit && fresh {
            g::each_drawing(d, |dr, _| {
                gedit::select_all(dr, SelectAction::Deselect);
                true
            });
        }
    } else {
        let r = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
        g::each_drawing(d, |dr, w| {
            if fresh {
                gedit::select_all(dr, SelectAction::Deselect);
            }
            gedit::select_box(dr, &v, w, r, action, mode);
            true
        });
    }
    d.touch();
}

// ---------------------------------------------------------------- modal

/// Run the Grease Pencil modal; returns the modal that continues.
pub fn modal(s: &mut State, d: &mut Doc, inp: Input, m: GModal) -> Modal {
    let keep = |m: GModal| Modal::Gp(Box::new(m));
    let lp = |s: &State, x: i32, y: i32| local(s, x, y);
    match m {
        GModal::Draw { mut sb } => match inp {
            Input::Move { x, y } => {
                sb.add(lp(s, x, y));
                keep(GModal::Draw { sb })
            }
            Input::Up { button: Button::Left, x, y } => {
                sb.add(lp(s, x, y));
                if let Some((oi, gi, li)) = target_layer(d) {
                    let mat = material(d, gi);
                    let st = with_projector(s, d, oi, gi, li, |p| sb.finish(p, mat));
                    if let Some(st) = st {
                        add_stroke(d, gi, li, st, false, "Draw Stroke");
                    }
                }
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => Modal::None,
            _ => keep(GModal::Draw { sb }),
        },
        GModal::Prim { kind, mut ctrl, shift, dragging } => {
            let snap = |kind: Primitive, a: (f32, f32), b: (f32, f32), sh: bool| {
                if !sh {
                    b
                } else if matches!(kind, Primitive::Box | Primitive::Circle) {
                    gtools::snap_square(a, b)
                } else {
                    gtools::snap_angle(a, b)
                }
            };
            match inp {
                Input::Move { x, y } => {
                    let at = lp(s, x, y);
                    let n = ctrl.len();
                    let a = ctrl[n.saturating_sub(2)];
                    if let Some(l) = ctrl.last_mut() {
                        *l = snap(kind, a, at, shift);
                    }
                    keep(GModal::Prim { kind, ctrl, shift, dragging })
                }
                Input::Up { button: Button::Left, .. } if kind != Primitive::Polyline => {
                    commit_primitive(s, d, kind, &ctrl);
                    Modal::None
                }
                Input::Up { button: Button::Left, .. } => keep(GModal::Prim { kind, ctrl, shift, dragging: false }),
                Input::Down { button: Button::Left, x, y, .. } if kind == Primitive::Polyline => {
                    let at = lp(s, x, y);
                    if let Some(l) = ctrl.last_mut() {
                        *l = at;
                    }
                    ctrl.push(at);
                    keep(GModal::Prim { kind, ctrl, shift, dragging })
                }
                Input::Key { key: Key::Enter, .. } | Input::Down { button: Button::Right, .. } if kind == Primitive::Polyline => {
                    // The trailing point follows the mouse: drop it.
                    if ctrl.len() > 2 {
                        ctrl.pop();
                    }
                    commit_primitive(s, d, kind, &ctrl);
                    Modal::None
                }
                Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => Modal::None,
                _ => keep(GModal::Prim { kind, ctrl, shift, dragging }),
            }
        }
        GModal::Dab { kind, before, gi, last, sst, invert, changed } => {
            let mut m = GModal::Dab { kind, before, gi, last, sst, invert, changed };
            match inp {
                Input::Move { x, y } => {
                    let at = lp(s, x, y);
                    dab(s, d, &mut m, at);
                    keep(m)
                }
                Input::Up { button: Button::Left, .. } => {
                    if let GModal::Dab { before, gi, changed: true, kind, .. } = m {
                        let label = match kind {
                            DabKind::Erase => "Erase",
                            DabKind::Tint => "Tint",
                            DabKind::Sculpt => "Sculpt Stroke",
                            DabKind::Vertex => "Vertex Paint",
                            DabKind::Weight => "Weight Paint",
                        };
                        d.push_undo_patch(label, vec![Change::GreasePencil { gp: gi, data: before }]);
                        d.touch();
                    }
                    Modal::None
                }
                Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => {
                    if let GModal::Dab { before, gi, .. } = m {
                        d.scene.grease_pencils[gi] = *before;
                        d.touch();
                    }
                    Modal::None
                }
                _ => keep(m),
            }
        }
        GModal::Xform { kind, before, gi, start, center, mut axis, mouse } => match inp {
            Input::Move { x, y } => {
                let m = GModal::Xform { kind, before, gi, start, center, axis, mouse: lp(s, x, y) };
                apply_xform(s, d, &m);
                keep(m)
            }
            Input::Key { key: Key::Char(c @ ('x' | 'y' | 'z')), .. } => {
                let a = (c as u8 - b'x') as usize;
                axis = if axis == Some(a) { None } else { Some(a) };
                let m = GModal::Xform { kind, before, gi, start, center, axis, mouse };
                apply_xform(s, d, &m);
                keep(m)
            }
            Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => {
                let label = match kind {
                    XKind::Move => "Move",
                    XKind::Rotate => "Rotate",
                    XKind::Scale => "Resize",
                    XKind::Radius => "Radius",
                };
                d.push_undo_patch(label, vec![Change::GreasePencil { gp: gi, data: before }]);
                d.touch();
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => {
                d.scene.grease_pencils[gi] = *before;
                d.touch();
                Modal::None
            }
            _ => keep(GModal::Xform { kind, before, gi, start, center, axis, mouse }),
        },
        GModal::Box { a, b, action, fresh } => match inp {
            Input::Move { x, y } => keep(GModal::Box { a, b: lp(s, x, y), action, fresh }),
            Input::Up { button: Button::Left, x, y } => {
                finish_box(s, d, a, lp(s, x, y), action, fresh);
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } => Modal::None,
            _ => keep(GModal::Box { a, b, action, fresh }),
        },
        GModal::Size { strength, x0, orig } => {
            let set = |d: &mut Doc, x: f32| {
                let v = if strength { (orig + (x - x0) / 200.0).clamp(0.0, 1.0) } else { (orig + (x - x0)).max(1.0) };
                set_size(d, strength, v);
            };
            match inp {
                Input::Move { x, y } => {
                    set(d, lp(s, x, y).0);
                    keep(GModal::Size { strength, x0, orig })
                }
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => Modal::None,
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    set(d, x0);
                    Modal::None
                }
                _ => keep(GModal::Size { strength, x0, orig }),
            }
        }
    }
}

fn commit_primitive(s: &State, d: &mut Doc, kind: Primitive, ctrl: &[(f32, f32)]) {
    if ctrl.len() < 2 || (kind != Primitive::Polyline && (ctrl[0].0 - ctrl[1].0).abs() + (ctrl[0].1 - ctrl[1].1).abs() < 2.0) {
        return;
    }
    let Some((oi, gi, li)) = target_layer(d) else { return };
    let mat = material(d, gi);
    let (sub, brush) = (d.gp.primitive_subdiv, d.gp.brush.clone());
    if let Some(st) = with_projector(s, d, oi, gi, li, |p| gtools::primitive_stroke(kind, ctrl, sub, &brush, p, mat)) {
        add_stroke(d, gi, li, st, false, kind.label());
    }
}

/// Radius (px) or strength of the brush the mode and tool use.
fn size_of(d: &mut Doc, strength: bool) -> f32 {
    if d.mode == Mode::Draw && matches!(d.gp.tool, DrawTool::Draw | DrawTool::Primitive(_) | DrawTool::Fill) {
        return if strength { d.gp.brush.strength } else { d.gp.brush.radius };
    }
    let mode = d.mode;
    let b = d.gp.size_mut(mode);
    if strength {
        b.strength
    } else {
        b.radius
    }
}

fn set_size(d: &mut Doc, strength: bool, v: f32) {
    if d.mode == Mode::Draw && matches!(d.gp.tool, DrawTool::Draw | DrawTool::Primitive(_) | DrawTool::Fill) {
        if strength {
            d.gp.brush.strength = v;
        } else {
            d.gp.brush.radius = v;
        }
        return;
    }
    let mode = d.mode;
    let b = d.gp.size_mut(mode);
    if strength {
        b.strength = v;
    } else {
        b.radius = v;
    }
}

// ---------------------------------------------------------------- keys

/// Keys over the view in a Grease Pencil mode. Returns true when used.
pub fn key(s: &mut State, d: &mut Doc, ui: &Ui, key: Key, mods: Mods) -> bool {
    if !g::in_mode(d) {
        // Ctrl+Tab on a Grease Pencil object in Object Mode: the mode menu.
        if key == Key::Tab && mods == Mods::CTRL && g::active_gp(d).is_some() {
            mode_menu(d, ui);
            return true;
        }
        return false;
    }
    use Key as K;
    let (mx, my) = ui.mouse();
    let at = local(s, mx, my);
    let q = |d: &mut Doc, id: &str, arg: &str| d.queue.push(Op::with(id, arg));
    match (key, mods) {
        (K::Tab, Mods::CTRL) => {
            mode_menu(d, ui);
            return true;
        }
        (K::Char('f'), Mods::NONE) if d.mode != Mode::Edit => {
            let orig = size_of(d, false);
            s.modal = Modal::Gp(Box::new(GModal::Size { strength: false, x0: at.0, orig }));
            return true;
        }
        (K::Char('f'), Mods::SHIFT) if d.mode != Mode::Edit => {
            let orig = size_of(d, true);
            s.modal = Modal::Gp(Box::new(GModal::Size { strength: true, x0: at.0, orig }));
            return true;
        }
        (K::Char('e'), m) if m == Mods::CTRL => {
            q(d, "grease_pencil.interpolate", "");
            return true;
        }
        (K::Char('e'), m) if m.ctrl && m.shift => {
            q(d, "grease_pencil.interpolate_sequence", "");
            return true;
        }
        _ => {}
    }
    match d.mode {
        Mode::Draw => match (key, mods) {
            (K::Char('e'), Mods::NONE) => d.gp.tool = DrawTool::Erase,
            (K::Char('d'), Mods::NONE) => d.gp.tool = DrawTool::Draw,
            (K::Char('i'), Mods::NONE) => q(d, "grease_pencil.insert_blank_frame", ""),
            (K::Delete, _) | (K::Char('x'), Mods::NONE) => q(d, "grease_pencil.delete_frame", ""),
            _ => return false,
        },
        Mode::Edit => return edit_key(s, d, ui, key, mods, at),
        _ => return false,
    }
    true
}

fn edit_key(s: &mut State, d: &mut Doc, ui: &Ui, key: Key, mods: Mods, at: (f32, f32)) -> bool {
    use Key as K;
    let q = |d: &mut Doc, id: &str, arg: &str| d.queue.push(Op::with(id, arg));
    let alt = Mods { alt: true, ..Mods::NONE };
    match (key, mods) {
        (K::Char('g'), Mods::NONE) => {
            start_xform(s, d, XKind::Move, at);
        }
        (K::Char('r'), Mods::NONE) => {
            start_xform(s, d, XKind::Rotate, at);
        }
        (K::Char('s'), Mods::NONE) => {
            start_xform(s, d, XKind::Scale, at);
        }
        (K::Char('s'), m) if m == alt => {
            start_xform(s, d, XKind::Radius, at);
        }
        (K::Char('e'), Mods::NONE) => {
            if extrude(d) {
                start_xform(s, d, XKind::Move, at);
            }
        }
        (K::Char('d'), Mods::SHIFT) => {
            d.push_undo("Duplicate");
            if g::each_drawing(d, |dr, _| gedit::duplicate(dr) > 0) {
                d.touch();
                start_xform(s, d, XKind::Move, at);
            }
        }
        (K::Char('a'), Mods::NONE) => q(d, "grease_pencil.select_all", "SELECT"),
        (K::Char('a'), m) if m == alt => q(d, "grease_pencil.select_all", "DESELECT"),
        (K::Char('i'), Mods::CTRL) => q(d, "grease_pencil.select_all", "INVERT"),
        (K::Char('l'), Mods::CTRL) | (K::Char('l'), Mods::NONE) => q(d, "grease_pencil.select_linked", ""),
        (K::Char('1'), Mods::NONE) => d.gp.select_mode = SelectMode::Point,
        (K::Char('2'), Mods::NONE) => d.gp.select_mode = SelectMode::Stroke,
        (K::Char('3'), Mods::NONE) => d.gp.select_mode = SelectMode::Segment,
        (K::Char('x'), Mods::NONE) | (K::Delete, _) => {
            let items = vec![
                MenuItem::op("Delete Points", "", Op::with("grease_pencil.delete", "POINTS")),
                MenuItem::op("Delete Strokes", "", Op::with("grease_pencil.delete", "STROKES")),
                MenuItem::op("Delete Frame", "", Op::new("grease_pencil.delete_frame")),
                MenuItem::Separator,
                MenuItem::op("Dissolve", "", Op::new("grease_pencil.dissolve")),
            ];
            popup(d, ui, "Delete", items);
        }
        (K::Char('f'), Mods::NONE) => q(d, "grease_pencil.cyclical_set", "TOGGLE"),
        (K::Char('j'), Mods::CTRL) => q(d, "grease_pencil.stroke_join", ""),
        (K::Char('p'), Mods::NONE) => q(d, "grease_pencil.separate", ""),
        (K::Char('c'), Mods::CTRL) => q(d, "grease_pencil.copy", ""),
        (K::Char('v'), Mods::CTRL) => q(d, "grease_pencil.paste", ""),
        (K::Char('m'), Mods::NONE) => {
            let mut items: Vec<MenuItem> = g::active_gp(d)
                .map(|(_, gi)| d.scene.grease_pencils[gi].layers.iter().rev().map(|l| MenuItem::op(&l.name, "", Op::with("grease_pencil.move_to_layer", &l.name))).collect())
                .unwrap_or_default();
            items.push(MenuItem::Separator);
            items.push(MenuItem::op("New Layer", "", Op::with("grease_pencil.move_to_layer", "Layer")));
            popup(d, ui, "Move to Layer", items);
        }
        (K::Char('k'), Mods::NONE) => popup(d, ui, "Stroke", stroke_menu(d)),
        _ => return false,
    }
    true
}

fn popup(d: &mut Doc, ui: &Ui, title: &str, items: Vec<MenuItem>) {
    let (x, y) = ui.mouse();
    d.menu = Some(Popup::new(title, items, x - 20, y - 10));
}

/// Ctrl+Tab: the Grease Pencil modes.
fn mode_menu(d: &mut Doc, ui: &Ui) {
    let items = g::MODES.iter().map(|m| MenuItem::Check(m.label().into(), d.mode == *m, Op::with("object.mode_set", m.id()))).collect();
    popup(d, ui, "Mode", items);
}

// ---------------------------------------------------------------- menus

fn op(l: &str, k: &str, id: &str) -> MenuItem {
    MenuItem::op(l, k, Op::new(id))
}
fn opa(l: &str, k: &str, id: &str, a: &str) -> MenuItem {
    MenuItem::op(l, k, Op::with(id, a))
}

pub fn draw_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Interpolate", "Ctrl E", "grease_pencil.interpolate"),
        op("Interpolate Sequence", "Shift Ctrl E", "grease_pencil.interpolate_sequence"),
        MenuItem::Separator,
        op("Insert Blank Keyframe", "I", "grease_pencil.insert_blank_frame"),
        opa("Insert Blank Keyframe (All Layers)", "", "grease_pencil.insert_blank_frame", "ALL_LAYERS"),
        op("Duplicate Active Keyframe", "", "grease_pencil.frame_duplicate"),
        op("Delete Active Keyframe", "X", "grease_pencil.delete_frame"),
        MenuItem::Separator,
        op("Toggle Onion Skins", "", "gpencil.onion_toggle"),
    ]
}

pub fn select_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        opa("All", "A", "grease_pencil.select_all", "SELECT"),
        opa("None", "Alt A", "grease_pencil.select_all", "DESELECT"),
        opa("Invert", "Ctrl I", "grease_pencil.select_all", "INVERT"),
        MenuItem::Separator,
        op("Linked", "Ctrl L", "grease_pencil.select_linked"),
        op("More", "", "grease_pencil.select_more"),
        op("Alternated", "", "grease_pencil.select_alternate"),
    ]
}

pub fn gp_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Duplicate", "Shift D", "grease_pencil.duplicate"),
        op("Copy", "Ctrl C", "grease_pencil.copy"),
        op("Paste", "Ctrl V", "grease_pencil.paste"),
        MenuItem::Separator,
        op("Separate", "P", "grease_pencil.separate"),
        op("Join Strokes", "Ctrl J", "grease_pencil.stroke_join"),
        MenuItem::Separator,
        op("Interpolate", "Ctrl E", "grease_pencil.interpolate"),
        op("Insert Blank Keyframe", "", "grease_pencil.insert_blank_frame"),
        op("Duplicate Active Keyframe", "", "grease_pencil.frame_duplicate"),
        MenuItem::Separator,
        opa("Delete Points", "X", "grease_pencil.delete", "POINTS"),
        opa("Delete Strokes", "", "grease_pencil.delete", "STROKES"),
        op("Dissolve", "", "grease_pencil.dissolve"),
    ]
}

pub fn stroke_menu(d: &Doc) -> Vec<MenuItem> {
    let mats: Vec<MenuItem> = g::active_gp(d)
        .map(|(_, gi)| d.scene.grease_pencils[gi].materials.iter().enumerate().map(|(i, m)| opa(&m.name, "", "grease_pencil.stroke_material_set", &i.to_string())).collect())
        .unwrap_or_default();
    vec![
        op("Subdivide", "", "grease_pencil.stroke_subdivide"),
        op("Simplify", "", "grease_pencil.stroke_simplify"),
        op("Smooth", "", "grease_pencil.stroke_smooth"),
        MenuItem::Separator,
        opa("Toggle Caps", "", "grease_pencil.caps_set", "TOGGLE"),
        opa("Round Caps", "", "grease_pencil.caps_set", "ROUND"),
        opa("Flat Caps", "", "grease_pencil.caps_set", "FLAT"),
        op("Toggle Cyclic", "F", "grease_pencil.cyclical_set"),
        op("Switch Direction", "", "grease_pencil.stroke_switch_direction"),
        MenuItem::Separator,
        MenuItem::Sub("Assign Material".into(), mats),
        op("Reset Vertex Color", "", "grease_pencil.stroke_reset_vertex_color"),
        MenuItem::Separator,
        MenuItem::Sub("Move to Layer".into(), move_layer_items(d)),
    ]
}

fn move_layer_items(d: &Doc) -> Vec<MenuItem> {
    g::active_gp(d).map(|(_, gi)| d.scene.grease_pencils[gi].layers.iter().rev().map(|l| opa(&l.name, "", "grease_pencil.move_to_layer", &l.name)).collect()).unwrap_or_default()
}

pub fn paint_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![op("Reset Vertex Color", "", "grease_pencil.stroke_reset_vertex_color"), MenuItem::Separator, op("Interpolate", "Ctrl E", "grease_pencil.interpolate")]
}

pub fn weights_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![op("Add Vertex Group", "", "grease_pencil.vertex_group_add"), op("Assign to Group", "", "grease_pencil.vertex_group_assign")]
}

/// The header menus of a Grease Pencil mode, or None.
pub fn header_menus(d: &Doc) -> Option<Vec<(&'static str, fn(&Doc) -> Vec<MenuItem>)>> {
    if !active(d) {
        return None;
    }
    Some(match d.mode {
        Mode::Draw => vec![("Draw", draw_menu as fn(&Doc) -> Vec<MenuItem>)],
        Mode::Edit => vec![("Select", select_menu as fn(&Doc) -> Vec<MenuItem>), ("Grease Pencil", gp_menu), ("Stroke", stroke_menu)],
        Mode::WeightPaint => vec![("Weights", weights_menu as fn(&Doc) -> Vec<MenuItem>)],
        _ => vec![("Paint", paint_menu as fn(&Doc) -> Vec<MenuItem>)],
    })
}

// ---------------------------------------------------------------- tool header

/// The tool settings strip in the header. Returns where it ended.
pub fn tool_header(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) -> i32 {
    if r.w < 240 {
        return r.x;
    }
    let mut x = r.x;
    let h = r.h;
    let num = |ui: &mut Ui, x: &mut i32, w: i32, key: &str, label: &str, v: &mut f32, o: NumOpts<'_>| {
        let rr = Rect::new(*x, r.y, w, h);
        let c = ui.number_ex(id(&format!("gp_hdr/{key}")), rr, label, v, o);
        *x += w + 4;
        c
    };
    let px = NumOpts { step: 1.0, unit: " px", min: 1.0, max: 500.0, slider: false, int: true };
    let fac = NumOpts { step: 0.05, unit: "", min: 0.0, max: 1.0, slider: true, int: false };
    match d.mode {
        Mode::Draw => {
            let tool = d.gp.tool;
            match tool {
                DrawTool::Erase => {
                    let rr = Rect::new(x, r.y, 80, h);
                    if ui.dropdown(rr, d.gp.erase_mode.label()) {
                        let items = EraseMode::ALL.iter().map(|m| MenuItem::Check(m.label().into(), d.gp.erase_mode == *m, Op::with("gpencil.set", &format!("erase_mode={}", m.label())))).collect();
                        ui.open_menu(rr, "", items);
                    }
                    x += 84;
                    num(ui, &mut x, 90, "er", "Radius", &mut d.gp.erase.radius, px);
                    num(ui, &mut x, 100, "es", "Strength", &mut d.gp.erase.strength, fac);
                }
                DrawTool::Tint => {
                    num(ui, &mut x, 90, "tr", "Radius", &mut d.gp.tint.radius, px);
                    num(ui, &mut x, 100, "ts", "Strength", &mut d.gp.tint.strength, fac);
                    let sw = Rect::new(x, r.y + 2, 30, h - 4);
                    if ui.swatch(sw, d.gp.tint_color) {
                        s.picker = Some(Picker::new(ColorTarget::Prop(PropColor::new(GpOwner::Tool, "tint_color")), sw, d));
                    }
                    x += 34;
                }
                DrawTool::Fill => {
                    num(ui, &mut x, 110, "fl", "Leak Size", &mut d.gp.fill.leak_size, NumOpts { step: 1.0, unit: " px", min: 0.0, max: 50.0, slider: false, int: false });
                    let mut back = d.gp.fill.on_back;
                    if ui.checkbox(Rect::new(x, r.y, 80, h), "Back", &mut back) {
                        d.gp.fill.on_back = back;
                    }
                    x += 84;
                }
                _ => {
                    num(ui, &mut x, 90, "br", "Radius", &mut d.gp.brush.radius, px);
                    num(ui, &mut x, 100, "bs", "Strength", &mut d.gp.brush.strength, fac);
                    if let DrawTool::Primitive(_) = tool {
                        let mut sub = d.gp.primitive_subdiv as f32;
                        if num(ui, &mut x, 100, "ps", "Subdivisions", &mut sub, NumOpts { step: 1.0, unit: "", min: 0.0, max: 128.0, slider: false, int: true }) {
                            d.gp.primitive_subdiv = sub as u32;
                        }
                    } else {
                        let mut st = d.gp.brush.use_stabilizer;
                        if ui.checkbox(Rect::new(x, r.y, 90, h), "Stabilize", &mut st) {
                            d.gp.brush.use_stabilizer = st;
                        }
                        x += 94;
                    }
                    let c = d.gp.brush.vertex_color;
                    let sw = Rect::new(x, r.y + 2, 30, h - 4);
                    if ui.swatch(sw, [c[0], c[1], c[2]]) {
                        s.picker = Some(Picker::new(ColorTarget::Prop(PropColor::new(GpOwner::Tool, "brush_color")), sw, d));
                    }
                    x += 34;
                }
            }
            // Placement and plane.
            let pr = Rect::new(x, r.y, 76, h);
            if ui.dropdown(pr, d.gp.placement.label()) {
                let items = [Placement::Origin, Placement::Cursor, Placement::Surface, Placement::Stroke]
                    .iter()
                    .map(|p| MenuItem::Check(p.label().into(), d.gp.placement == *p, Op::with("gpencil.set", &format!("placement={}", p.id()))))
                    .collect();
                ui.open_menu(pr, "Stroke Placement", items);
            }
            x += 80;
            let pl = Rect::new(x, r.y, 70, h);
            if ui.dropdown(pl, d.gp.plane.label()) {
                let items = [DrawPlane::Front, DrawPlane::Side, DrawPlane::Top, DrawPlane::View, DrawPlane::Cursor]
                    .iter()
                    .map(|p| MenuItem::Check(p.label().into(), d.gp.plane == *p, Op::with("gpencil.set", &format!("plane={}", p.id()))))
                    .collect();
                ui.open_menu(pl, "Drawing Plane", items);
            }
            x += 74;
            let mut add = d.gp.additive;
            if ui.checkbox(Rect::new(x, r.y, 80, h), "Additive", &mut add) {
                d.gp.additive = add;
            }
            x += 84;
        }
        Mode::Edit => {
            let sm = d.gp.select_mode;
            for (i, (lab, m)) in [("Point", SelectMode::Point), ("Stroke", SelectMode::Stroke), ("Segment", SelectMode::Segment)].into_iter().enumerate() {
                let rr = Rect::new(x + i as i32 * 62, r.y, 60, h);
                if sm == m {
                    ui.p.round_rect(rr, 4, t::SELECTED);
                }
                if ui.button(rr, lab) {
                    d.gp.select_mode = m;
                }
            }
            x += 3 * 62 + 4;
        }
        mode => {
            let b = *d.gp.size_mut(mode);
            let (mut rad, mut st) = (b.radius, b.strength);
            num(ui, &mut x, 90, "sr", "Radius", &mut rad, px);
            num(ui, &mut x, 100, "ss", "Strength", &mut st, fac);
            let b = d.gp.size_mut(mode);
            b.radius = rad;
            b.strength = st;
            if mode == Mode::VertexPaint {
                let sw = Rect::new(x, r.y + 2, 30, h - 4);
                if ui.swatch(sw, d.gp.vertex_color) {
                    s.picker = Some(Picker::new(ColorTarget::Prop(PropColor::new(GpOwner::Tool, "vertex_color")), sw, d));
                }
                x += 34;
            }
            if mode == Mode::WeightPaint {
                num(ui, &mut x, 90, "ww", "Weight", &mut d.gp.weight_value, fac);
            }
        }
    }
    x
}

// ---------------------------------------------------------------- toolbar

const BS: i32 = 28;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Entry {
    Draw(DrawTool),
    Edit(EditTool),
    Sculpt(SculptTool),
    Vertex(VertexTool),
    Weight(WeightTool),
}

impl Entry {
    fn label(self) -> &'static str {
        match self {
            Entry::Draw(t) => t.label(),
            Entry::Edit(t) => t.label(),
            Entry::Sculpt(t) => t.label(),
            Entry::Vertex(t) => t.label(),
            Entry::Weight(t) => t.label(),
        }
    }
}

fn entries(d: &Doc) -> Vec<Entry> {
    match d.mode {
        Mode::Draw => DrawTool::ALL.iter().map(|&t| Entry::Draw(t)).collect(),
        Mode::Edit => EditTool::ALL.iter().map(|&t| Entry::Edit(t)).collect(),
        Mode::Sculpt => SculptTool::ALL.iter().map(|&t| Entry::Sculpt(t)).collect(),
        Mode::VertexPaint => VertexTool::ALL.iter().map(|&t| Entry::Vertex(t)).collect(),
        _ => WeightTool::ALL.iter().map(|&t| Entry::Weight(t)).collect(),
    }
}

pub fn toolbar_rect(d: &Doc, r: Rect) -> Rect {
    let n = entries(d).len() as i32;
    Rect::new(r.x + 4, r.y + 6, BS + 8, (n * (BS + 2) + 8).min(r.h - 10))
}

pub fn toolbar(d: &mut Doc, ui: &mut Ui, tr: Rect) {
    ui.p.push_clip(tr);
    ui.p.round_rect_alpha(tr, 5, 0x1d1d1d, 200);
    let mut tip = None;
    for (i, e) in entries(d).into_iter().enumerate() {
        let br = Rect::new(tr.x + 4, tr.y + 4 + i as i32 * (BS + 2), BS, BS);
        let on = match e {
            Entry::Draw(t) => d.gp.tool == t,
            Entry::Edit(t) => d.gp.edit_tool == t,
            Entry::Sculpt(t) => d.gp.sculpt_tool == t,
            Entry::Vertex(t) => d.gp.vertex_tool == t,
            Entry::Weight(t) => d.gp.weight_tool == t,
        };
        let hover = ui.hovered(br);
        if on {
            ui.p.round_rect(br, 4, t::SELECTED);
        } else if hover {
            ui.p.round_rect(br, 4, 0x3a3a3a);
        }
        let (cx, cy) = br.center();
        tool_icon(ui.p, e.label(), cx as f32, cy as f32, if on { t::TEXT_HI } else { t::TEXT });
        if hover {
            tip = Some((br, e.label()));
        }
        if ui.clicked(br) {
            match e {
                Entry::Draw(t) => d.gp.tool = t,
                Entry::Edit(t) => d.gp.edit_tool = t,
                Entry::Sculpt(t) => d.gp.sculpt_tool = t,
                Entry::Vertex(t) => d.gp.vertex_tool = t,
                Entry::Weight(t) => d.gp.weight_tool = t,
            }
        }
    }
    ui.p.pop_clip();
    if let Some((br, tip)) = tip {
        let w = ui.p.text_width(tip) + 14;
        let r = Rect::new(tr.right() + 8, br.y + 3, w, 22);
        ui.p.round_rect(r, 4, t::MENU_BG);
        ui.p.label(r, tip, t::TEXT, Align::Center);
    }
    if let Some(Input::Down { x, y, .. }) = ui.input {
        if tr.contains(x, y) {
            ui.input = None;
        }
    }
}

/// Our own small tool glyphs, drawn from lines and circles.
fn tool_icon(p: &mut Painter, name: &str, cx: f32, cy: f32, c: u32) {
    let l = |p: &mut Painter, a: (f32, f32), b: (f32, f32)| p.line(cx + a.0, cy + a.1, cx + b.0, cy + b.1, c);
    match name {
        "Draw" => {
            l(p, (-8.0, 8.0), (6.0, -6.0));
            l(p, (-6.0, 8.0), (8.0, -6.0));
            l(p, (-8.0, 8.0), (-6.0, 8.0));
            l(p, (6.0, -6.0), (8.0, -8.0));
        }
        "Fill" => {
            l(p, (-7.0, -2.0), (0.0, -9.0));
            l(p, (0.0, -9.0), (7.0, -2.0));
            l(p, (7.0, -2.0), (0.0, 5.0));
            l(p, (0.0, 5.0), (-7.0, -2.0));
            p.circle(cx + 7.0, cy + 6.0, 2.5, c);
        }
        "Erase" => {
            l(p, (-8.0, 2.0), (0.0, -6.0));
            l(p, (0.0, -6.0), (8.0, 2.0));
            l(p, (8.0, 2.0), (2.0, 8.0));
            l(p, (2.0, 8.0), (-2.0, 8.0));
            l(p, (-2.0, 8.0), (-8.0, 2.0));
            l(p, (-4.0, -2.0), (4.0, 6.0));
        }
        "Tint" => {
            p.ring(cx, cy, 7.0, c);
            p.circle(cx - 2.0, cy - 2.0, 3.0, c);
        }
        "Eyedropper" => {
            l(p, (-7.0, 7.0), (3.0, -3.0));
            p.circle(cx + 5.0, cy - 5.0, 3.0, c);
        }
        "Line" => l(p, (-8.0, 7.0), (8.0, -7.0)),
        "Polyline" => {
            l(p, (-8.0, 6.0), (-3.0, -6.0));
            l(p, (-3.0, -6.0), (2.0, 4.0));
            l(p, (2.0, 4.0), (8.0, -5.0));
        }
        "Arc" | "Curve" => {
            let mut prev = None;
            for i in 0..=12 {
                let a = std::f32::consts::PI * (i as f32 / 12.0);
                let pt = (-8.0 * a.cos(), 5.0 - if name == "Arc" { 10.0 * a.sin() } else { 10.0 * (2.0 * a).sin() * 0.6 });
                if let Some(q) = prev {
                    l(p, q, pt);
                }
                prev = Some(pt);
            }
        }
        "Box" | "Select Box" => {
            l(p, (-7.0, -6.0), (7.0, -6.0));
            l(p, (7.0, -6.0), (7.0, 6.0));
            l(p, (7.0, 6.0), (-7.0, 6.0));
            l(p, (-7.0, 6.0), (-7.0, -6.0));
        }
        "Circle" => p.ring(cx, cy, 7.5, c),
        "Cursor" => {
            p.ring(cx, cy, 6.0, c);
            l(p, (-9.0, 0.0), (-3.0, 0.0));
            l(p, (3.0, 0.0), (9.0, 0.0));
            l(p, (0.0, -9.0), (0.0, -3.0));
            l(p, (0.0, 3.0), (0.0, 9.0));
        }
        "Extrude" => {
            l(p, (-7.0, 6.0), (7.0, 6.0));
            l(p, (0.0, 6.0), (0.0, -7.0));
            l(p, (-3.0, -4.0), (0.0, -7.0));
            l(p, (3.0, -4.0), (0.0, -7.0));
        }
        "Radius" => {
            p.ring(cx, cy, 7.0, c);
            p.ring(cx, cy, 3.0, c);
        }
        _ => {
            // Brushes: a soft dot with the tool's initials.
            p.ring(cx, cy, 9.0, c);
            let s: String = name.chars().take(2).collect();
            p.label(Rect::new(cx as i32 - 10, cy as i32 - 8, 20, 16), &s, c, Align::Center);
        }
    }
}

// ---------------------------------------------------------------- drawing

/// Composite every Grease Pencil object over the viewport image `rgba`
/// (`w` x `h`), depth tested against `depth` (distances) when given.
pub fn composite(s: &State, d: &mut Doc, rgba: &mut [u8], w: u32, h: u32, depth: Option<&[f32]>) {
    if !d.scene.objects.iter().any(|o| matches!(o.data, sex_3d::scene::ObjectData::GreasePencil(_))) {
        return;
    }
    let ev = g::evaluated(d);
    let v = GpView::from_viewport(&s.cam, w, h);
    let lookup = |oi: usize| ev.iter().find(|e| e.0 == oi).map(|e| e.1.clone());
    let onion = g::active_gp(d).is_some_and(|(_, gi)| d.scene.grease_pencils[gi].onion.enabled) && !d.playing;
    let opts = SceneDraw { view: &v, depth, frame: d.frame, render: false, onion, evaluated: Some(&lookup) };
    raster::composite_srgb8(&d.scene, &opts, rgba);
}

/// Overlays: edit points, the brush circle and what a modal is drawing.
pub fn draw(s: &State, d: &Doc, ui: &mut Ui) {
    let r = s.rect;
    let (ox, oy) = (r.x as f32, r.y as f32);
    let v = view(s);
    if d.mode == Mode::Edit && g::in_mode(d) {
        if let Some((oi, gi)) = g::active_gp(d) {
            for (li, fi, m) in g::editable(d, oi, gi) {
                for st in &d.scene.grease_pencils[gi].layers[li].frames[fi].drawing.strokes {
                    let pts: Vec<Option<(f32, f32)>> = st.points.iter().map(|p| v.project(m.transform_point(p.position)).map(|q| (q.0 + ox, q.1 + oy))).collect();
                    let n = pts.len();
                    let segs = if st.cyclic { n } else { n.saturating_sub(1) };
                    for i in 0..segs {
                        if let (Some(a), Some(b)) = (pts[i], pts[(i + 1) % n]) {
                            ui.p.line(a.0, a.1, b.0, b.1, if st.select { 0xff9f40 } else { 0x7a7a7a });
                        }
                    }
                    for (p, q) in st.points.iter().zip(&pts) {
                        if let Some(q) = q {
                            let c = if p.select { 0xffa733 } else { 0x101010 };
                            ui.p.fill(Rect::new(q.0 as i32 - 2, q.1 as i32 - 2, 5, 5), c);
                        }
                    }
                }
            }
        }
    }
    if let Modal::Gp(m) = &s.modal {
        match &**m {
            GModal::Draw { sb } => {
                let c = sb.brush.vertex_color;
                let col = if c[3] > 0.0 { rgb(c) } else { 0x202020 };
                for w in sb.screen.windows(2) {
                    ui.p.line(w[0].0 + ox, w[0].1 + oy, w[1].0 + ox, w[1].1 + oy, col);
                }
            }
            GModal::Prim { kind, ctrl, .. } => {
                let pts = gtools::primitive_points(*kind, ctrl, d.gp.primitive_subdiv);
                for w in pts.windows(2) {
                    ui.p.line(w[0].0 + ox, w[0].1 + oy, w[1].0 + ox, w[1].1 + oy, 0x202020);
                }
                if kind.cyclic() && pts.len() > 2 {
                    let (a, b) = (pts[pts.len() - 1], pts[0]);
                    ui.p.line(a.0 + ox, a.1 + oy, b.0 + ox, b.1 + oy, 0x202020);
                }
                for c in ctrl {
                    ui.p.fill(Rect::new((c.0 + ox) as i32 - 3, (c.1 + oy) as i32 - 3, 7, 7), 0xffffff);
                }
            }
            GModal::Box { a, b, .. } => {
                let rr = Rect::new((a.0.min(b.0) + ox) as i32, (a.1.min(b.1) + oy) as i32, (a.0 - b.0).abs() as i32, (a.1 - b.1).abs() as i32);
                ui.p.frame(rr, 0xffffff);
            }
            GModal::Size { strength, .. } => {
                let (mx, my) = ui.mouse();
                let rad = if *strength { 40.0 } else { size_of_ro(d) };
                ui.p.ring(mx as f32, my as f32, rad, 0xffffff);
            }
            _ => {}
        }
    }
    // Brush circle for brush tools.
    let brush = match d.mode {
        Mode::Draw if matches!(d.gp.tool, DrawTool::Erase) => Some(d.gp.erase.radius),
        Mode::Draw if matches!(d.gp.tool, DrawTool::Tint) => Some(d.gp.tint.radius),
        Mode::Draw if matches!(d.gp.tool, DrawTool::Draw) => Some(d.gp.brush.radius),
        Mode::Sculpt => Some(d.gp.sculpt.radius),
        Mode::VertexPaint => Some(d.gp.vertex.radius),
        Mode::WeightPaint => Some(d.gp.weight.radius),
        _ => None,
    };
    if let (Some(rad), true) = (brush, active(d)) {
        let (mx, my) = ui.mouse();
        if r.contains(mx, my) && !matches!(s.modal, Modal::Gp(ref m) if matches!(**m, GModal::Size { .. })) {
            ui.p.ring(mx as f32, my as f32, rad, 0xe0e0e0);
        }
    }
}

fn size_of_ro(d: &Doc) -> f32 {
    match d.mode {
        Mode::Sculpt => d.gp.sculpt.radius,
        Mode::VertexPaint => d.gp.vertex.radius,
        Mode::WeightPaint => d.gp.weight.radius,
        _ => match d.gp.tool {
            DrawTool::Erase => d.gp.erase.radius,
            DrawTool::Tint => d.gp.tint.radius,
            _ => d.gp.brush.radius,
        },
    }
}

fn rgb(c: [f32; 4]) -> u32 {
    let e = |v: f32| (sex_3d::render::srgb_encode(v.clamp(0.0, 1.0)) * 255.0).round() as u32;
    e(c[0]) << 16 | e(c[1]) << 8 | e(c[2])
}
