// SPDX-License-Identifier: GPL-3.0-only
//! Sculpt Mode, Vertex Paint and Texture Paint in the 3D view: strokes from
//! the pointer (spacing, dots, drag dot, anchored, line, airbrush and the
//! stabilizer), F / Shift+F, Ctrl to invert and Shift to smooth, box and
//! lasso masking, hiding and face sets, the Mesh Filter, the brush cursor,
//! texture display while texture painting, the header's tool settings, the
//! toolbar of brushes and the sidebar's Tool tab.
use super::proj::Proj;
use super::{Modal, State};
use crate::doc::{Doc, Mode};
use crate::draw::{Align, Painter, Rect};
use crate::editors::properties::picker::{self, Picker};
use crate::editors::properties::ColorTarget;
use crate::input::{Button, Input, Key, Mods};
use crate::sculpt::{self, Kind, PaintStroke, RegionTool};
use crate::theme as t;
use crate::ui::{id, MenuItem, Op, Popup, Ui};
use sex_3d::paint::texture::TexBrush;
use sex_3d::paint::vertex::VpBrush;
use sex_3d::paint::BlendMode;
use sex_3d::sculpt::ops as sops;
use sex_3d::sculpt::{Brush, BrushKind, Curve, Dab, StrokeMethod};
use sex_3d::undo::Change;
use sex_3d::{Mat4, Vec3};

/// True in the three brush modes with a mesh to paint.
pub fn active(d: &Doc) -> bool {
    matches!(d.mode, Mode::Sculpt | Mode::VertexPaint | Mode::TexturePaint) && sculpt::active_mesh(d).is_some()
}

/// A stroke in progress.
pub struct StrokeState {
    st: PaintStroke,
    mode: Mode,
    brush: Brush,
    invert: bool,
    method: StrokeMethod,
    press: (f32, f32),
    /// Where the last dab went (screen).
    last: (f32, f32),
    /// The stabilized pointer.
    lazy: (f32, f32),
    mouse: (f32, f32),
    /// Surface point under the press (object, world), for grab-like
    /// brushes, anchored strokes and the Rotate brush.
    anchor: Option<(Vec3, Vec3)>,
    /// Object-space centre of the last dab (Snake Hook, step vectors).
    last_center: Option<Vec3>,
    radius_px: f32,
    dabs: usize,
}

/// The paint modes' modal states.
pub enum SModal {
    Stroke(Box<StrokeState>),
    /// F / Shift+F.
    Size { strength: bool, x0: i32, orig: f32, center: (i32, i32) },
    /// B or a region tool: waiting for the drag.
    RegionWait(RegionTool),
    Region { tool: RegionTool, pts: Vec<(f32, f32)>, clear: bool },
    Filter { orig: Vec<Vec3>, x0: i32 },
}

// ---------------------------------------------------------------- geometry

fn scale_of(w: &Mat4) -> f32 {
    let s = (w.transform_vector(Vec3::X).length() + w.transform_vector(Vec3::Y).length() + w.transform_vector(Vec3::Z).length()) / 3.0;
    if s > 1e-12 {
        s
    } else {
        1.0
    }
}

/// Object-space vector towards the viewer at world point `pw`.
fn view_dir(p: &Proj, inv: &Mat4, pw: Vec3) -> Vec3 {
    let wv = if p.ortho { p.back } else { (p.eye - pw).normalize() };
    inv.transform_vector(wv).normalize()
}

/// The point under screen (x, y) on the view plane through world point `pw`, in object space.
fn plane_point(p: &Proj, inv: &Mat4, pw: Vec3, x: f32, y: f32) -> Vec3 {
    inv.transform_point(p.at_depth(x, y, pw))
}

/// Build a dab at screen point `at`.
fn make_dab(d: &mut Doc, p: &Proj, ss: &mut StrokeState, at: (f32, f32)) -> Option<Dab> {
    let kind = ss.brush.kind;
    let (o, dir) = p.ray(at.0, at.1);
    let hit = sculpt::raycast(d, o, dir);
    let w = sculpt::world(d)?;
    let inv = w.inverse();
    let scale = scale_of(&w);
    let grab_like = kind.is_grab_like() || kind == BrushKind::Rotate;
    let (center, center_w) = if grab_like {
        match ss.anchor {
            Some(a) => a,
            None => {
                let (h, hw) = hit?;
                ss.anchor = Some((h.pos, hw));
                (h.pos, hw)
            }
        }
    } else if kind == BrushKind::SnakeHook {
        match (ss.last_center, hit) {
            (Some(c), _) => {
                let cw = w.transform_point(c);
                (plane_point(p, &inv, cw, at.0, at.1), p.at_depth(at.0, at.1, cw))
            }
            (None, Some((h, hw))) => (h.pos, hw),
            (None, None) => return None,
        }
    } else if ss.method == StrokeMethod::Anchored {
        match ss.anchor {
            Some(a) => a,
            None => {
                let (h, hw) = hit?;
                ss.anchor = Some((h.pos, hw));
                (h.pos, hw)
            }
        }
    } else {
        let (h, hw) = hit?;
        (h.pos, hw)
    };
    let radius_px = if ss.method == StrokeMethod::Anchored && !grab_like {
        ((at.0 - ss.press.0).hypot(at.1 - ss.press.1)).max(2.0)
    } else {
        ss.radius_px
    };
    let radius = if ss.brush.radius_in_scene { ss.brush.scene_radius } else { radius_px * p.world_per_pixel(center_w) / scale };
    let view = view_dir(p, &inv, center_w);
    let mut dab = Dab::at(center, radius.max(1e-6), view);
    dab.invert = ss.invert;
    if grab_like {
        let now = plane_point(p, &inv, center_w, ss.mouse.0, ss.mouse.1);
        dab.grab = now - center;
        if kind == BrushKind::Rotate {
            if let Some(c) = p.to_screen(center_w) {
                let a0 = (ss.press.1 - c.1).atan2(ss.press.0 - c.0);
                let a1 = (ss.mouse.1 - c.1).atan2(ss.mouse.0 - c.0);
                dab.angle = -(a1 - a0);
            }
        }
    }
    if let Some(lc) = ss.last_center {
        let lw = w.transform_point(lc);
        let prev = plane_point(p, &inv, lw, ss.last.0, ss.last.1);
        let now = plane_point(p, &inv, lw, at.0, at.1);
        dab.step = now - prev;
    }
    if ss.mode == Mode::Sculpt && d.sculpt.dyntopo.enabled {
        let upp = p.world_per_pixel(center_w) / scale;
        dab.detail = d.sculpt.dyntopo.edge_length(radius, upp);
        dab.dyntopo_subdivide = d.sculpt.dyntopo.subdivide;
        dab.dyntopo_collapse = d.sculpt.dyntopo.collapse;
    }
    dab.relax = kind == BrushKind::SlideRelax && ss.invert;
    Some(dab)
}

/// Object space to (screen x, y, depth with smaller nearer).
fn projector(p: &Proj, w: Mat4) -> impl Fn(Vec3) -> Option<(f32, f32, f32)> + '_ {
    move |q: Vec3| p.project(w.transform_point(q)).map(|(x, y, k)| (x, y, -k))
}

/// One dab of whatever the stroke paints, at screen `at`.
fn apply_dab(d: &mut Doc, p: &Proj, ss: &mut StrokeState, at: (f32, f32)) {
    match ss.st.kind {
        Kind::Texture { oi, .. } => {
            let w = d.scene.world_matrix(oi);
            let proj = projector(p, w);
            let step = (at.0 - ss.last.0, at.1 - ss.last.1);
            sculpt::texture_dab(d, &mut ss.st, &proj, at, 1.0, step, ss.invert);
            sculpt::note_live(d, oi, &[], true);
        }
        _ => {
            if let Some(dab) = make_dab(d, p, ss, at) {
                sculpt::dab(d, &mut ss.st, &dab);
                ss.last_center = Some(dab.center);
            }
        }
    }
    ss.last = at;
    ss.dabs += 1;
}

/// Redo the whole stroke from its start (anchored, line, drag dot).
fn restart(d: &mut Doc, p: &Proj, ss: &mut StrokeState, at: (f32, f32)) {
    sculpt::restore(d, &mut ss.st);
    let keep_anchor = ss.anchor;
    ss.last_center = None;
    match ss.method {
        StrokeMethod::Line => {
            let spacing = (ss.radius_px * ss.brush.spacing / 100.0).max(1.0);
            let (dx, dy) = (at.0 - ss.press.0, at.1 - ss.press.1);
            let len = dx.hypot(dy);
            let n = (len / spacing).floor() as usize;
            ss.last = ss.press;
            for i in 0..=n.min(2000) {
                let f = if n == 0 { 0.0 } else { i as f32 * spacing / len.max(1e-6) };
                apply_dab(d, p, ss, (ss.press.0 + dx * f, ss.press.1 + dy * f));
            }
        }
        _ => {
            ss.last = at;
            apply_dab(d, p, ss, at);
        }
    }
    ss.anchor = keep_anchor;
}

// ---------------------------------------------------------------- press and modal

/// Left press in a paint mode. Returns true when the press was ours.
pub fn press(s: &mut State, d: &mut Doc, p: &Proj, x: i32, y: i32, mods: Mods) -> bool {
    if !active(d) {
        return false;
    }
    let at = (x as f32, y as f32);
    // A region tool or a pending box.
    if let Some(SModal::RegionWait(tool)) = take_wait(s) {
        start_region(s, tool, at, mods.ctrl);
        return true;
    }
    if d.mode == Mode::Sculpt {
        if let Some(tool) = d.sculpt.region {
            if tool == RegionTool::MeshFilter {
                let orig = sculpt::with_mesh(d, |m, _| m.positions.clone()).unwrap_or_default();
                s.modal = Modal::Sculpt(Box::new(SModal::Filter { orig, x0: x }));
            } else {
                start_region(s, tool, at, mods.ctrl);
            }
            return true;
        }
    }
    // Vertex Paint: Ctrl+click... Blender samples with S; keep Ctrl for invert.
    let (st, brush, method, radius_px) = match d.mode {
        Mode::Sculpt => {
            let mut brush = d.sculpt.brush().clone();
            if mods.shift && brush.kind != BrushKind::Mask {
                let r = brush.radius;
                brush = d.sculpt.brush_of(BrushKind::Smooth).clone();
                brush.radius = r;
            }
            let method = brush.stroke;
            let r = brush.radius;
            let Some(st) = sculpt::begin_sculpt(d, &brush) else { return true };
            (st, brush, method, r)
        }
        Mode::VertexPaint => {
            let Some(st) = sculpt::begin_vertex(d) else { return true };
            let mut b = Brush::new(BrushKind::Draw);
            b.curve = d.sculpt.vp.curve;
            (st, b, StrokeMethod::Space, d.sculpt.vp.radius)
        }
        _ => {
            let Some(st) = sculpt::begin_texture(d) else { return true };
            let mut b = Brush::new(BrushKind::Draw);
            b.curve = d.sculpt.tp.curve;
            (st, b, StrokeMethod::Space, d.sculpt.tp.radius)
        }
    };
    let mut ss = StrokeState {
        st,
        mode: d.mode,
        invert: mods.ctrl,
        brush,
        method,
        press: at,
        last: at,
        lazy: at,
        mouse: at,
        anchor: None,
        last_center: None,
        radius_px,
        dabs: 0,
    };
    // The surface under the press anchors grab-like and anchored strokes.
    let (o, dir) = p.ray(at.0, at.1);
    if let Some((h, hw)) = sculpt::raycast(d, o, dir) {
        ss.anchor = Some((h.pos, hw));
    }
    let method = ss.method;
    if method != StrokeMethod::Line {
        apply_dab(d, p, &mut ss, at);
    }
    s.modal = Modal::Sculpt(Box::new(SModal::Stroke(Box::new(ss))));
    true
}

fn take_wait(s: &mut State) -> Option<SModal> {
    if let Modal::Sculpt(m) = &s.modal {
        if let SModal::RegionWait(t) = **m {
            s.modal = Modal::None;
            return Some(SModal::RegionWait(t));
        }
    }
    None
}

fn start_region(s: &mut State, tool: RegionTool, at: (f32, f32), clear: bool) {
    s.modal = Modal::Sculpt(Box::new(SModal::Region { tool, pts: vec![at, at], clear }));
}

/// Run the paint modals; returns the modal that continues.
pub fn modal(s: &mut State, d: &mut Doc, p: &Proj, inp: Input, m: SModal) -> Modal {
    let _ = s;
    let keep = |m: SModal| Modal::Sculpt(Box::new(m));
    match m {
        SModal::Stroke(mut ss) => match inp {
            Input::Move { x, y } => {
                let mouse = (x as f32, y as f32);
                ss.mouse = mouse;
                stroke_move(d, p, &mut ss, mouse);
                keep(SModal::Stroke(ss))
            }
            Input::Tick => {
                if ss.method == StrokeMethod::Airbrush {
                    let at = ss.last;
                    apply_dab(d, p, &mut ss, at);
                }
                keep(SModal::Stroke(ss))
            }
            Input::Up { button: Button::Left, x, y } => {
                if ss.method == StrokeMethod::Line {
                    restart(d, p, &mut ss, (x as f32, y as f32));
                }
                sculpt::end(d, ss.st);
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => {
                sculpt::restore(d, &mut ss.st);
                sculpt::end(d, ss.st);
                Modal::None
            }
            _ => keep(SModal::Stroke(ss)),
        },
        SModal::Size { strength, x0, orig, center } => {
            let mode = d.mode;
            let set = |d: &mut Doc, x: i32| {
                if strength {
                    d.sculpt.set_strength(mode, orig + (x - x0) as f32 / 200.0);
                } else {
                    d.sculpt.set_radius(mode, orig + (x - x0) as f32);
                }
            };
            match inp {
                Input::Move { x, .. } => {
                    set(d, x);
                    keep(SModal::Size { strength, x0, orig, center })
                }
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => Modal::None,
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    set(d, x0);
                    Modal::None
                }
                _ => keep(SModal::Size { strength, x0, orig, center }),
            }
        }
        SModal::RegionWait(tool) => match inp {
            Input::Down { button: Button::Left, x, y, mods } => {
                let at = (x as f32, y as f32);
                keep(SModal::Region { tool, pts: vec![at, at], clear: mods.ctrl })
            }
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => Modal::None,
            _ => keep(SModal::RegionWait(tool)),
        },
        SModal::Region { tool, mut pts, clear } => match inp {
            Input::Move { x, y } => {
                let at = (x as f32, y as f32);
                if tool.lasso() {
                    if pts.last().is_none_or(|l| (l.0 - at.0).hypot(l.1 - at.1) > 2.0) {
                        pts.push(at);
                    }
                } else if pts.len() >= 2 {
                    pts[1] = at;
                }
                keep(SModal::Region { tool, pts, clear })
            }
            Input::Up { button: Button::Left, .. } => {
                region_apply(d, p, tool, &pts, clear);
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => Modal::None,
            _ => keep(SModal::Region { tool, pts, clear }),
        },
        SModal::Filter { orig, x0 } => match inp {
            Input::Move { x, .. } => {
                filter_apply(d, &orig, (x - x0) as f32 / 100.0);
                keep(SModal::Filter { orig, x0 })
            }
            Input::Up { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => {
                filter_end(d, orig);
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => {
                filter_apply(d, &orig, 0.0);
                sculpt::touch_ours(d);
                Modal::None
            }
            _ => keep(SModal::Filter { orig, x0 }),
        },
    }
}

fn stroke_move(d: &mut Doc, p: &Proj, ss: &mut StrokeState, mouse: (f32, f32)) {
    let kind = ss.brush.kind;
    let grab_like = ss.mode == Mode::Sculpt && (kind.is_grab_like() || kind == BrushKind::Rotate);
    if grab_like {
        // Grab-like brushes deform from the original shape each time.
        let at = ss.press;
        apply_dab(d, p, ss, at);
        return;
    }
    match ss.method {
        StrokeMethod::Anchored | StrokeMethod::DragDot => {
            restart(d, p, ss, mouse);
            return;
        }
        StrokeMethod::Line => {
            restart(d, p, ss, mouse);
            return;
        }
        StrokeMethod::Dots => {
            apply_dab(d, p, ss, mouse);
            return;
        }
        _ => {}
    }
    // The stabilizer: the brush trails the pointer.
    let target = if ss.brush.lazy {
        let (dx, dy) = (mouse.0 - ss.lazy.0, mouse.1 - ss.lazy.1);
        let dist = dx.hypot(dy);
        let r = ss.brush.lazy_radius;
        if dist > r {
            let k = (dist - r) / dist * (1.0 - ss.brush.lazy_factor * 0.8);
            ss.lazy = (ss.lazy.0 + dx * k, ss.lazy.1 + dy * k);
        }
        ss.lazy
    } else {
        mouse
    };
    if ss.method == StrokeMethod::Airbrush {
        apply_dab(d, p, ss, target);
        return;
    }
    let spacing = (ss.radius_px * ss.brush.spacing / 100.0).max(1.0);
    let mut guard = 0;
    loop {
        let (dx, dy) = (target.0 - ss.last.0, target.1 - ss.last.1);
        let dist = dx.hypot(dy);
        if dist < spacing || guard > 400 {
            break;
        }
        let at = (ss.last.0 + dx / dist * spacing, ss.last.1 + dy / dist * spacing);
        apply_dab(d, p, ss, at);
        ss.last = at;
        guard += 1;
    }
}

/// Mask, hide or face-set the vertices inside a box or lasso.
fn region_apply(d: &mut Doc, p: &Proj, tool: RegionTool, pts: &[(f32, f32)], clear: bool) {
    let Some(w) = sculpt::world(d).or_else(|| sculpt::active_mesh(d).map(|(oi, _)| d.scene.world_matrix(oi))) else { return };
    let inv = w.inverse();
    let project = |q: Vec3| p.to_screen(w.transform_point(q));
    let (bx0, by0, bx1, by1) = match (pts.first(), pts.last()) {
        (Some(a), Some(b)) => (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)),
        _ => return,
    };
    if !tool.lasso() && (bx1 - bx0 < 2.0 || by1 - by0 < 2.0) {
        return;
    }
    let poly = pts.to_vec();
    let lasso = tool.lasso();
    let inside = move |x: f32, y: f32| if lasso { sops::in_lasso(&poly, x, y) } else { x >= bx0 && x <= bx1 && y >= by0 && y <= by1 };
    let eye = inv.transform_point(p.eye);
    let back = inv.transform_vector(p.back);
    let ortho = p.ortho;
    let front = move |q: Vec3| if ortho { back } else { eye - q };
    let label = tool.label();
    sculpt::mesh_op(d, label, |m, ss| {
        let sel = sops::verts_in_screen(m, ss, &project, &inside, Some(&front));
        if !sel.iter().any(|&s| s) {
            return false;
        }
        match tool {
            RegionTool::BoxMask | RegionTool::LassoMask => sops::mask_select(m, &sel, if clear { 0.0 } else { 1.0 }),
            RegionTool::BoxHide => {
                sops::hide_select(m, &sel, clear);
                ss.update_hidden(m);
            }
            _ => {
                sops::face_set_select(m, &sel);
            }
        }
        true
    });
}

fn filter_apply(d: &mut Doc, orig: &[Vec3], amount: f32) {
    let (kind, strength, axes) = (d.sculpt.filter, d.sculpt.filter_strength, d.sculpt.filter_axes);
    let seed = 7;
    let res = sculpt::with_mesh(d, |m, ss| {
        if m.positions.len() != orig.len() {
            return None;
        }
        m.positions.copy_from_slice(orig);
        let all: Vec<u32> = (0..m.positions.len() as u32).collect();
        ss.pbvh.mark_verts(&all);
        ss.pbvh.refit(&m.positions);
        ss.update_normals(m, &all);
        if amount != 0.0 {
            sops::mesh_filter(m, ss, kind, amount * strength, axes, seed);
        }
        Some(all)
    });
    if let (Some(Some(all)), Some((oi, _))) = (res, sculpt::active_mesh(d)) {
        sculpt::note_live(d, oi, &all, false);
    }
}

fn filter_end(d: &mut Doc, orig: Vec<Vec3>) {
    let Some((oi, mi, target)) = sculpt::target(d) else { return };
    let Some((_, m, _)) = sculpt::edited_mesh(d) else { return };
    if m.positions.len() != orig.len() {
        return;
    }
    let mut verts = Vec::new();
    let mut values = Vec::new();
    for (v, o) in orig.iter().enumerate() {
        if m.positions[v] != *o {
            verts.push(v as u32);
            values.push(*o);
        }
    }
    if verts.is_empty() {
        return;
    }
    match target {
        sculpt::Target::Mesh => d.push_undo_patch("Mesh Filter", vec![Change::Positions { mesh: mi, verts, values }]),
        sculpt::Target::Multires { md, level } => {
            let pos = m.positions.clone();
            let before = d.scene.objects[oi].modifiers[md].clone();
            let base = d.scene.meshes[mi].clone();
            if let sex_3d::modifiers::Modifier::Multires { data, .. } = &mut d.scene.objects[oi].modifiers[md] {
                data.store(&base, level, &pos);
            }
            d.push_undo_patch("Mesh Filter", vec![Change::Modifier { object: oi, index: md, data: Box::new(before) }]);
        }
    }
    sculpt::touch_ours(d);
}

// ---------------------------------------------------------------- keys

fn op(l: &str, k: &str, id: &str) -> MenuItem {
    MenuItem::op(l, k, Op::new(id))
}
fn opa(l: &str, k: &str, id: &str, a: &str) -> MenuItem {
    MenuItem::op(l, k, Op::with(id, a))
}

/// The face set under the pointer.
fn face_set_under(d: &mut Doc, p: &Proj, x: f32, y: f32) -> Option<i32> {
    let (o, dir) = p.ray(x, y);
    let (h, _) = sculpt::raycast(d, o, dir)?;
    let (_, m, _) = sculpt::edited_mesh(d)?;
    Some(m.face_set(h.face as usize))
}

/// Keys in the paint modes. Returns None when the key is not ours.
pub fn key(s: &mut State, d: &mut Doc, ui: &Ui, key: Key, mods: Mods, p: &Proj) -> Option<()> {
    if !active(d) {
        return None;
    }
    use Key as K;
    let (mx, my) = ui.mouse();
    let q = |d: &mut Doc, id: &str, arg: &str| d.queue.push(Op::with(id, arg));
    let alt = mods.alt && !mods.ctrl && !mods.shift;
    let mode = d.mode;
    match (key, mods) {
        (K::Char('f'), Mods::NONE) => {
            let orig = d.sculpt.radius_strength(mode).0;
            s.modal = Modal::Sculpt(Box::new(SModal::Size { strength: false, x0: mx, orig, center: (mx - orig as i32, my) }));
            return Some(());
        }
        (K::Char('f'), Mods::SHIFT) => {
            let (r, st) = d.sculpt.radius_strength(mode);
            s.modal = Modal::Sculpt(Box::new(SModal::Size { strength: true, x0: mx, orig: st, center: (mx - r as i32, my) }));
            return Some(());
        }
        _ => {}
    }
    match mode {
        Mode::Sculpt => match (key, mods) {
            (K::Char('d'), Mods::CTRL) => q(d, "sculpt.dynamic_topology_toggle", ""),
            (K::Char('r'), Mods::CTRL) => q(d, "object.voxel_remesh", ""),
            (K::Char('m'), _) if alt => q(d, "paint.mask_flood_fill", "VALUE 0"),
            (K::Char('i'), Mods::CTRL) => q(d, "paint.mask_flood_fill", "INVERT"),
            (K::Char('w'), Mods::CTRL) => {
                d.menu = Some(Popup::new("Init Face Sets", face_set_init_items(), mx - 20, my - 10));
            }
            (K::Char('h'), Mods::NONE) => {
                let fs = face_set_under(d, p, mx as f32, my as f32).unwrap_or(1);
                q(d, "sculpt.face_set_change_visibility", &format!("HIDE {fs}"));
            }
            (K::Char('h'), Mods::SHIFT) => {
                let fs = face_set_under(d, p, mx as f32, my as f32).unwrap_or(1);
                q(d, "sculpt.face_set_change_visibility", &format!("ISOLATE {fs}"));
            }
            (K::Char('h'), _) if alt => q(d, "sculpt.face_set_change_visibility", "SHOW"),
            (K::Char('b'), Mods::NONE) => s.modal = Modal::Sculpt(Box::new(SModal::RegionWait(RegionTool::BoxMask))),
            (K::Char('x'), Mods::NONE) => q(d, "sculpt.brush_select", "DRAW"),
            (K::Char('c'), Mods::NONE) => q(d, "sculpt.brush_select", "CLAY"),
            (K::Char('c'), Mods::SHIFT) => q(d, "sculpt.brush_select", "CLAY_STRIPS"),
            (K::Char('i'), Mods::NONE) => q(d, "sculpt.brush_select", "INFLATE"),
            (K::Char('l'), Mods::NONE) => q(d, "sculpt.brush_select", "LAYER"),
            (K::Char('p'), Mods::NONE) => q(d, "sculpt.brush_select", "PINCH"),
            (K::Char('g'), Mods::NONE) => q(d, "sculpt.brush_select", "GRAB"),
            (K::Char('k'), Mods::NONE) => q(d, "sculpt.brush_select", "SNAKE_HOOK"),
            (K::Char('s'), Mods::SHIFT) | (K::Char('s'), Mods::NONE) => q(d, "sculpt.brush_select", "SMOOTH"),
            (K::Char('t'), Mods::SHIFT) => q(d, "sculpt.brush_select", "FLATTEN"),
            (K::Char('m'), Mods::NONE) => q(d, "sculpt.brush_select", "MASK"),
            (K::Char('r'), Mods::NONE) | (K::Char('a'), Mods::NONE) | (K::Delete, _) => {}
            _ => return None,
        },
        Mode::VertexPaint => match (key, mods) {
            (K::Char('k'), Mods::SHIFT) => q(d, "paint.vertex_color_set", ""),
            (K::Char('s'), Mods::NONE) => sample_color(d, p, mx as f32, my as f32),
            (K::Char('x'), Mods::NONE) => {
                let vp = &mut d.sculpt.vp;
                std::mem::swap(&mut vp.color, &mut vp.secondary);
            }
            (K::Char('g'), Mods::NONE) | (K::Char('r'), Mods::NONE) | (K::Char('a'), Mods::NONE) | (K::Delete, _) => {}
            _ => return None,
        },
        _ => match (key, mods) {
            (K::Char('x'), Mods::NONE) => {
                let tp = &mut d.sculpt.tp;
                std::mem::swap(&mut tp.color, &mut tp.secondary);
            }
            (K::Char('s'), Mods::NONE) => sample_color(d, p, mx as f32, my as f32),
            (K::Char('s'), Mods::ALT) => q(d, "image.save", ""),
            (K::Char('g'), Mods::NONE) | (K::Char('r'), Mods::NONE) | (K::Char('a'), Mods::NONE) | (K::Delete, _) => {}
            _ => return None,
        },
    }
    Some(())
}

/// S: pick the colour under the pointer as the brush colour.
fn sample_color(d: &mut Doc, p: &Proj, x: f32, y: f32) {
    let (o, dir) = p.ray(x, y);
    let Some((h, _)) = sculpt::raycast(d, o, dir) else { return };
    let Some((_, mi)) = sculpt::active_mesh(d) else { return };
    let c = if d.mode == Mode::TexturePaint {
        let Some(img) = sculpt::paint_image(d) else { return };
        let Some(uv) = sex_3d::paint::texture::uv_at(&d.scene.meshes[mi], h.face as usize, h.pos) else { return };
        let c = d.scene.images[img].sample(uv.x, uv.y);
        [c[0], c[1], c[2]]
    } else {
        let m = &d.scene.meshes[mi];
        let Some(c) = sex_3d::paint::vertex::sample(m, m.active_color, h.face as usize, h.pos) else { return };
        [c[0], c[1], c[2]]
    };
    if d.mode == Mode::TexturePaint {
        d.sculpt.tp.color = c;
    } else {
        d.sculpt.vp.color = c;
    }
}

fn face_set_init_items() -> Vec<MenuItem> {
    vec![
        opa("By Loose Parts", "", "sculpt.face_sets_init", "LOOSE_PARTS"),
        opa("By Material", "", "sculpt.face_sets_init", "MATERIALS"),
        opa("By Normals", "", "sculpt.face_sets_init", "NORMALS"),
        opa("By UV Seams", "", "sculpt.face_sets_init", "UV_SEAMS"),
        opa("By Sharp Edges", "", "sculpt.face_sets_init", "SHARP_EDGES"),
    ]
}

// ---------------------------------------------------------------- menus

pub fn sculpt_menu(d: &Doc) -> Vec<MenuItem> {
    let filt = |k: sops::FilterKind| opa(k.label(), "", "sculpt.mesh_filter", k.id());
    vec![
        MenuItem::Check("Dynamic Topology".into(), d.sculpt.dyntopo.enabled, Op::new("sculpt.dynamic_topology_toggle")),
        opa("Detail Flood Fill", "", "sculpt.detail_flood_fill", ""),
        op("Voxel Remesh", "Ctrl R", "object.voxel_remesh"),
        MenuItem::Separator,
        MenuItem::Sub(
            "Mesh Filter".into(),
            vec![
                filt(sops::FilterKind::Smooth),
                filt(sops::FilterKind::Scale),
                filt(sops::FilterKind::Inflate),
                filt(sops::FilterKind::Sphere),
                filt(sops::FilterKind::Random),
                filt(sops::FilterKind::Sharpen),
                filt(sops::FilterKind::EnhanceDetails),
            ],
        ),
        MenuItem::Separator,
        MenuItem::Sub(
            "Multires".into(),
            vec![
                opa("Subdivide", "", "object.multires_subdivide", "CATMULL_CLARK"),
                opa("Simple", "", "object.multires_subdivide", "SIMPLE"),
                opa("Linear", "", "object.multires_subdivide", "LINEAR"),
                op("Unsubdivide", "", "object.multires_unsubdivide"),
                op("Apply Base", "", "object.multires_base_apply"),
                op("Delete Higher", "", "object.multires_higher_levels_delete"),
            ],
        ),
    ]
}

pub fn mask_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        opa("Invert Mask", "Ctrl I", "paint.mask_flood_fill", "INVERT"),
        opa("Fill Mask", "", "paint.mask_flood_fill", "VALUE 1"),
        opa("Clear Mask", "Alt M", "paint.mask_flood_fill", "VALUE 0"),
        MenuItem::Separator,
        opa("Box Mask", "B", "sculpt.tool_select", "BOX_MASK"),
        opa("Lasso Mask", "", "sculpt.tool_select", "LASSO_MASK"),
        MenuItem::Separator,
        opa("Grow Mask", "", "paint.mask_filter", "GROW"),
        opa("Shrink Mask", "", "paint.mask_filter", "SHRINK"),
        opa("Smooth Mask", "", "paint.mask_filter", "SMOOTH"),
    ]
}

pub fn face_sets_menu(_d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Sub("Initialize Face Sets".into(), face_set_init_items()), MenuItem::Separator];
    v.extend([
        opa("Box Face Set", "", "sculpt.tool_select", "BOX_FACE_SET"),
        opa("Lasso Face Set", "", "sculpt.tool_select", "LASSO_FACE_SET"),
        MenuItem::Separator,
        opa("Invert Visible Face Sets", "", "sculpt.face_set_change_visibility", "INVERT"),
        opa("Show All Face Sets", "Alt H", "sculpt.face_set_change_visibility", "SHOW"),
        opa("Box Hide", "", "sculpt.tool_select", "BOX_HIDE"),
    ]);
    v
}

pub fn paint_menu(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![op("Set Vertex Colors", "Shift K", "paint.vertex_color_set"), MenuItem::Separator];
    v.push(MenuItem::Sub("Brush".into(), VpBrush::ALL.iter().map(|b| MenuItem::Check(b.label().into(), d.sculpt.vp.brush == *b, Op::with("paint.vertex_brush", b.label()))).collect()));
    v.push(op("Add Color Attribute", "", "geometry.color_attribute_add"));
    v
}

pub fn texture_menu(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![opa("Add Texture Paint Slot", "", "paint.add_texture_paint_slot", "Base Color"), op("Save Image", "Alt S", "image.save"), MenuItem::Separator];
    v.push(MenuItem::Sub("Brush".into(), TexBrush::ALL.iter().map(|b| MenuItem::Check(b.label().into(), d.sculpt.tp.brush == *b, Op::with("paint.texture_brush", b.label()))).collect()));
    v
}

/// The header menus of a paint mode, or None.
pub fn header_menus(d: &Doc) -> Option<Vec<(&'static str, fn(&Doc) -> Vec<MenuItem>)>> {
    if !active(d) {
        return None;
    }
    Some(match d.mode {
        Mode::Sculpt => vec![("Sculpt", sculpt_menu), ("Mask", mask_menu), ("Face Sets", face_sets_menu)],
        Mode::VertexPaint => vec![("Paint", paint_menu)],
        _ => vec![("Texture", texture_menu)],
    })
}

fn brush_items(d: &Doc) -> Vec<MenuItem> {
    BrushKind::ALL.iter().map(|k| MenuItem::Check(k.label().into(), d.sculpt.region.is_none() && d.sculpt.kind == *k, Op::with("sculpt.brush_select", k.id()))).collect()
}

fn blend_items(d: &Doc) -> Vec<MenuItem> {
    let cur = if d.mode == Mode::TexturePaint { d.sculpt.tp.blend } else { d.sculpt.vp.blend };
    BlendMode::ALL.iter().map(|b| MenuItem::Check(b.label().into(), cur == *b, Op::with("paint.blend", b.id()))).collect()
}

/// The tool settings strip in the header. Returns where it ended.
pub fn tool_header(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) -> i32 {
    if r.w < 300 {
        return r.x;
    }
    let mut x = r.x;
    let mode = d.mode;
    let label = match mode {
        Mode::Sculpt => d.sculpt.region.map(|t| t.label()).unwrap_or(d.sculpt.kind.label()),
        Mode::VertexPaint => d.sculpt.vp.brush.label(),
        _ => d.sculpt.tp.brush.label(),
    };
    let br = Rect::new(x, r.y, 110, r.h);
    if ui.dropdown(br, label) {
        let items = match mode {
            Mode::Sculpt => brush_items(d),
            Mode::VertexPaint => VpBrush::ALL.iter().map(|b| MenuItem::Check(b.label().into(), d.sculpt.vp.brush == *b, Op::with("paint.vertex_brush", b.label()))).collect(),
            _ => TexBrush::ALL.iter().map(|b| MenuItem::Check(b.label().into(), d.sculpt.tp.brush == *b, Op::with("paint.texture_brush", b.label()))).collect(),
        };
        ui.open_menu(br, "Brush", items);
    }
    x += 116;
    if mode != Mode::Sculpt {
        let primary = if mode == Mode::TexturePaint { d.sculpt.tp.color } else { d.sculpt.vp.color };
        let secondary = if mode == Mode::TexturePaint { d.sculpt.tp.secondary } else { d.sculpt.vp.secondary };
        let cr = Rect::new(x, r.y + 2, 30, r.h - 4);
        if ui.swatch(cr, primary) {
            s.picker = Some(Picker::new(ColorTarget::Paint(false), cr, d));
        }
        let cr2 = Rect::new(x + 32, r.y + 6, 18, r.h - 12);
        if ui.swatch(cr2, secondary) {
            s.picker = Some(Picker::new(ColorTarget::Paint(true), cr2, d));
        }
        x += 56;
        let bl = Rect::new(x, r.y, 84, r.h);
        let cur = if mode == Mode::TexturePaint { d.sculpt.tp.blend } else { d.sculpt.vp.blend };
        if ui.dropdown(bl, cur.label()) {
            ui.open_menu(bl, "Blend", blend_items(d));
        }
        x += 90;
    }
    let (mut rad, mut st) = d.sculpt.radius_strength(mode);
    if x + 210 > r.right() {
        return x;
    }
    if ui.number(id("sculpt/radius"), Rect::new(x, r.y, 100, r.h), "Radius", &mut rad, 1.0, " px") {
        d.sculpt.set_radius(mode, rad);
    }
    x += 104;
    if ui.number(id("sculpt/strength"), Rect::new(x, r.y, 100, r.h), "Strength", &mut st, 0.01, "") {
        d.sculpt.set_strength(mode, st);
    }
    x += 106;
    if mode == Mode::Sculpt && d.sculpt.region.is_none() && x + 70 < r.right() {
        // Direction: Add / Subtract.
        let b = d.sculpt.brush_mut();
        let mut i = b.invert as usize;
        if ui.segmented(Rect::new(x, r.y, 64, r.h), &["+", "-"], &mut i) {
            b.invert = i == 1;
        }
        x += 70;
    }
    if mode == Mode::Sculpt && x + 120 < r.right() {
        // Symmetry X/Y/Z.
        for (k, a) in ["X", "Y", "Z"].iter().enumerate() {
            let rr = Rect::new(x + k as i32 * 24, r.y, 23, r.h);
            let on = d.sculpt.symmetry.mirror[k];
            ui.p.round_rect(rr, 4, if on { t::SELECTED } else if ui.hovered(rr) { t::BUTTON_HOVER } else { t::BUTTON });
            ui.p.label(rr, a, if on { t::TEXT_HI } else { t::TEXT }, Align::Center);
            if ui.clicked(rr) {
                d.sculpt.symmetry.mirror[k] = !on;
            }
        }
        x += 78;
        if x + 70 < r.right() {
            let rr = Rect::new(x, r.y, 64, r.h);
            let on = d.sculpt.dyntopo.enabled;
            ui.p.round_rect(rr, 4, if on { t::SELECTED } else if ui.hovered(rr) { t::BUTTON_HOVER } else { t::BUTTON });
            ui.p.label(rr, "Dyntopo", if on { t::TEXT_HI } else { t::TEXT }, Align::Center);
            if ui.clicked(rr) {
                d.queue.push(Op::new("sculpt.dynamic_topology_toggle"));
            }
            x += 70;
        }
    }
    x
}

// ---------------------------------------------------------------- toolbar

const BS: i32 = 28;

/// The paint-mode toolbar entries: brushes then region tools.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Entry {
    Sculpt(BrushKind),
    Region(RegionTool),
    Vp(VpBrush),
    Tex(TexBrush),
}

fn entries(d: &Doc) -> Vec<Option<Entry>> {
    match d.mode {
        Mode::Sculpt => {
            let mut v: Vec<Option<Entry>> = BrushKind::ALL.iter().filter(|k| !matches!(k, BrushKind::Mask | BrushKind::DrawFaceSets)).map(|&k| Some(Entry::Sculpt(k))).collect();
            v.push(None);
            v.push(Some(Entry::Sculpt(BrushKind::Mask)));
            v.push(Some(Entry::Region(RegionTool::BoxMask)));
            v.push(Some(Entry::Region(RegionTool::LassoMask)));
            v.push(None);
            v.push(Some(Entry::Region(RegionTool::BoxHide)));
            v.push(Some(Entry::Sculpt(BrushKind::DrawFaceSets)));
            v.push(Some(Entry::Region(RegionTool::BoxFaceSet)));
            v.push(Some(Entry::Region(RegionTool::LassoFaceSet)));
            v.push(None);
            v.push(Some(Entry::Region(RegionTool::MeshFilter)));
            v
        }
        Mode::VertexPaint => VpBrush::ALL.iter().map(|&b| Some(Entry::Vp(b))).collect(),
        _ => TexBrush::ALL.iter().map(|&b| Some(Entry::Tex(b))).collect(),
    }
}

/// Columns of the sculpt toolbar (it wraps into two when tall).
fn columns(d: &Doc, h: i32) -> usize {
    let n = entries(d).len() as i32;
    if n * (BS + 2) + 12 > h {
        2
    } else {
        1
    }
}

pub fn toolbar_rect(d: &Doc, r: Rect) -> Rect {
    let es = entries(d);
    let cols = columns(d, r.h - 10);
    let rows = es.len().div_ceil(cols) as i32;
    let h = rows * (BS + 2) + 8;
    Rect::new(r.x + 4, r.y + 6, cols as i32 * (BS + 2) + 6, h.min(r.h - 10))
}

pub fn toolbar(d: &mut Doc, ui: &mut Ui, tr: Rect) {
    let es = entries(d);
    let cols = columns(d, tr.h + 10);
    ui.p.push_clip(tr);
    ui.p.round_rect_alpha(tr, 5, 0x1d1d1d, 200);
    let mut tip: Option<(Rect, String)> = None;
    let mut i = 0usize;
    for e in es {
        let col = (i % cols) as i32;
        let row = (i / cols) as i32;
        i += 1;
        let Some(e) = e else { continue };
        let br = Rect::new(tr.x + 3 + col * (BS + 2), tr.y + 4 + row * (BS + 2), BS, BS);
        let on = match e {
            Entry::Sculpt(k) => d.sculpt.region.is_none() && d.sculpt.kind == k,
            Entry::Region(t) => d.sculpt.region == Some(t),
            Entry::Vp(b) => d.sculpt.vp.brush == b,
            Entry::Tex(b) => d.sculpt.tp.brush == b,
        };
        let hover = ui.hovered(br);
        if on {
            ui.p.round_rect(br, 4, t::SELECTED);
        } else if hover {
            ui.p.round_rect(br, 4, 0x3a3a3a);
        }
        let (cx, cy) = br.center();
        let c = if on { t::TEXT_HI } else { t::TEXT };
        let name = match e {
            Entry::Sculpt(k) => {
                brush_icon(ui.p, k, cx as f32, cy as f32, c);
                k.label()
            }
            Entry::Region(t) => {
                region_icon(ui.p, t, cx as f32, cy as f32, c);
                t.label()
            }
            Entry::Vp(b) => {
                paint_icon(ui.p, b.label(), cx as f32, cy as f32, c);
                b.label()
            }
            Entry::Tex(b) => {
                paint_icon(ui.p, b.label(), cx as f32, cy as f32, c);
                b.label()
            }
        };
        if hover {
            tip = Some((br, name.to_string()));
        }
        if ui.clicked(br) {
            match e {
                Entry::Sculpt(k) => d.queue.push(Op::with("sculpt.brush_select", k.id())),
                Entry::Region(t) => d.queue.push(Op::with("sculpt.tool_select", t.id())),
                Entry::Vp(b) => d.sculpt.vp.brush = b,
                Entry::Tex(b) => d.sculpt.tp.brush = b,
            }
        }
    }
    ui.p.pop_clip();
    if let Some((br, tip)) = tip {
        let w = ui.p.text_width(&tip) + 14;
        let r = Rect::new(tr.right() + 8, br.y + 3, w, 22);
        ui.p.round_rect(r, 4, t::MENU_BG);
        ui.p.label(r, &tip, t::TEXT, Align::Center);
    }
    if let Some(Input::Down { x, y, .. }) = ui.input {
        if tr.contains(x, y) {
            ui.input = None;
        }
    }
}

/// A brush profile over a ground line: what the brush does to a surface.
fn profile(p: &mut Painter, cx: f32, cy: f32, c: u32, f: impl Fn(f32) -> f32) {
    let mut prev: Option<(f32, f32)> = None;
    for i in 0..=16 {
        let u = i as f32 / 16.0 * 2.0 - 1.0;
        let pt = (cx + u * 10.0, cy + 5.0 - f(u) * 9.0);
        if let Some(q) = prev {
            p.line(q.0, q.1, pt.0, pt.1, c);
        }
        prev = Some(pt);
    }
}

/// Our own icons for the sculpt brushes.
pub fn brush_icon(p: &mut Painter, k: BrushKind, cx: f32, cy: f32, c: u32) {
    use BrushKind as K;
    let bump = |u: f32| (1.0 - u * u).max(0.0).powi(2);
    let dim = 0x707070;
    match k {
        K::Draw => profile(p, cx, cy, c, bump),
        K::DrawSharp => profile(p, cx, cy, c, |u: f32| (1.0 - u.abs()).max(0.0).powi(3)),
        K::Clay => profile(p, cx, cy, c, |u: f32| if u.abs() < 0.6 { 0.6 } else { 0.6 * (1.0 - (u.abs() - 0.6) / 0.4) }),
        K::ClayStrips => {
            profile(p, cx, cy, c, |u: f32| if u.abs() < 0.7 { 0.5 } else { 0.0 });
            p.line(cx - 7.0, cy - 7.0, cx + 7.0, cy - 7.0, c);
        }
        K::ClayThumb => {
            profile(p, cx, cy, c, |u: f32| (0.5 - 0.4 * u).max(0.0) * (1.0 - u * u));
        }
        K::Layer => {
            profile(p, cx, cy, c, |u: f32| if u.abs() < 0.8 { 0.45 } else { 0.0 });
            p.line(cx - 10.0, cy - 4.0, cx + 10.0, cy - 4.0, dim);
        }
        K::Inflate => {
            p.ring(cx, cy, 6.0, c);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                p.line(cx + dx * 7.0, cy + dy * 7.0, cx + dx * 11.0, cy + dy * 11.0, c);
            }
        }
        K::Blob => {
            p.circle(cx, cy, 6.0, c);
            p.line(cx - 10.0, cy + 7.0, cx + 10.0, cy + 7.0, c);
        }
        K::Crease => profile(p, cx, cy, c, |u: f32| -0.6 * (1.0 - u.abs()).max(0.0).powi(2)),
        K::Smooth => profile(p, cx, cy, c, |u: f32| 0.25 * (u * 6.0).sin() * (1.0 - u.abs())),
        K::Flatten => {
            profile(p, cx, cy, c, |u: f32| if u.abs() < 0.5 { 0.3 } else { 0.3 * (1.0 - (u.abs() - 0.5) * 2.0) });
            p.line(cx - 10.0, cy - 6.0, cx + 10.0, cy - 6.0, dim);
        }
        K::Fill => {
            profile(p, cx, cy, c, |u: f32| 0.6 * u * u);
            p.line(cx - 6.0, cy + 2.0, cx + 6.0, cy + 2.0, dim);
        }
        K::Scrape => {
            profile(p, cx, cy, c, |u: f32| (0.6 * bump(u)).min(0.25));
            p.line(cx - 10.0, cy + 2.0, cx + 10.0, cy + 2.0, dim);
        }
        K::MultiplaneScrape => {
            p.line(cx - 10.0, cy + 6.0, cx, cy - 4.0, c);
            p.line(cx, cy - 4.0, cx + 10.0, cy + 6.0, c);
        }
        K::Pinch => {
            p.line(cx - 10.0, cy, cx - 3.0, cy, c);
            p.line(cx + 10.0, cy, cx + 3.0, cy, c);
            p.line(cx - 6.0, cy - 3.0, cx - 3.0, cy, c);
            p.line(cx - 6.0, cy + 3.0, cx - 3.0, cy, c);
            p.line(cx + 6.0, cy - 3.0, cx + 3.0, cy, c);
            p.line(cx + 6.0, cy + 3.0, cx + 3.0, cy, c);
        }
        K::Grab => {
            p.ring(cx - 2.0, cy + 2.0, 5.0, dim);
            p.circle(cx + 3.0, cy - 3.0, 5.0, c);
        }
        K::ElasticDeform => {
            p.ring(cx, cy, 8.0, dim);
            p.circle(cx + 2.0, cy - 2.0, 4.0, c);
        }
        K::SnakeHook => {
            profile(p, cx, cy, c, |u: f32| if u > 0.2 { 1.0 - (u - 0.2) * 1.25 } else { (u + 1.0) / 1.2 * 0.2 });
        }
        K::Thumb => {
            p.circle(cx, cy, 5.0, c);
            p.line(cx - 10.0, cy + 6.0, cx + 10.0, cy + 6.0, c);
        }
        K::Pose => {
            p.line(cx - 8.0, cy + 8.0, cx, cy, c);
            p.line(cx, cy, cx + 8.0, cy - 6.0, c);
            p.circle(cx, cy, 2.5, c);
        }
        K::Nudge => {
            p.line(cx - 9.0, cy, cx + 9.0, cy, c);
            p.line(cx + 9.0, cy, cx + 5.0, cy - 4.0, c);
            p.line(cx + 9.0, cy, cx + 5.0, cy + 4.0, c);
        }
        K::Rotate => {
            p.ring(cx, cy, 7.0, c);
            p.line(cx + 7.0, cy, cx + 10.0, cy - 3.0, c);
        }
        K::SlideRelax => {
            p.line(cx - 9.0, cy + 4.0, cx + 9.0, cy + 4.0, c);
            p.line(cx - 5.0, cy - 2.0, cx + 5.0, cy - 2.0, c);
            p.line(cx + 5.0, cy - 2.0, cx + 2.0, cy - 5.0, c);
        }
        K::Simplify => {
            for (a, b) in [((-9.0, 6.0), (0.0, -7.0)), ((0.0, -7.0), (9.0, 6.0)), ((9.0, 6.0), (-9.0, 6.0)), ((-4.5, -0.5), (4.5, -0.5))] {
                p.line(cx + a.0, cy + a.1, cx + b.0, cy + b.1, c);
            }
        }
        K::Mask => {
            p.ring(cx, cy, 8.0, c);
            p.circle(cx - 2.0, cy, 5.5, dim);
        }
        K::DrawFaceSets => {
            p.fill(Rect::new(cx as i32 - 8, cy as i32 - 8, 8, 8), 0xd08080);
            p.fill(Rect::new(cx as i32, cy as i32 - 8, 8, 8), 0x80c080);
            p.fill(Rect::new(cx as i32 - 8, cy as i32, 16, 8), 0x8080d0);
        }
    }
}

fn region_icon(p: &mut Painter, t: RegionTool, cx: f32, cy: f32, c: u32) {
    let boxr = |p: &mut Painter| {
        p.frame(Rect::new(cx as i32 - 8, cy as i32 - 7, 16, 14), c);
    };
    let lasso = |p: &mut Painter| {
        let pts = [(-8.0, 0.0), (-4.0, -7.0), (5.0, -6.0), (8.0, 1.0), (2.0, 7.0), (-6.0, 5.0), (-8.0, 0.0)];
        for w in pts.windows(2) {
            p.line(cx + w[0].0, cy + w[0].1, cx + w[1].0, cy + w[1].1, c);
        }
    };
    match t {
        RegionTool::BoxMask => {
            boxr(p);
            p.fill(Rect::new(cx as i32 - 6, cy as i32 - 5, 6, 10), 0x707070);
        }
        RegionTool::LassoMask => {
            lasso(p);
            p.circle(cx - 1.0, cy, 3.5, 0x707070);
        }
        RegionTool::BoxHide => {
            boxr(p);
            p.line(cx - 8.0, cy + 7.0, cx + 8.0, cy - 7.0, c);
        }
        RegionTool::BoxFaceSet => {
            boxr(p);
            p.fill(Rect::new(cx as i32 - 6, cy as i32 - 5, 12, 10), 0x8080d0);
        }
        RegionTool::LassoFaceSet => {
            lasso(p);
            p.circle(cx, cy, 3.5, 0x8080d0);
        }
        RegionTool::MeshFilter => {
            p.ring(cx, cy, 7.0, c);
            p.line(cx - 10.0, cy, cx + 10.0, cy, c);
        }
    }
}

fn paint_icon(p: &mut Painter, name: &str, cx: f32, cy: f32, c: u32) {
    match name {
        "Draw" => {
            p.line(cx - 7.0, cy + 7.0, cx + 6.0, cy - 6.0, c);
            p.circle(cx - 7.0, cy + 7.0, 2.5, c);
        }
        "Blur" | "Soften" => {
            p.circle(cx, cy, 7.0, 0x606060);
            p.circle(cx, cy, 4.0, c);
        }
        "Average" => {
            p.ring(cx, cy, 7.0, c);
            p.line(cx - 4.0, cy, cx + 4.0, cy, c);
        }
        "Smear" => {
            p.line(cx - 8.0, cy + 3.0, cx + 8.0, cy - 3.0, c);
            p.line(cx - 8.0, cy - 1.0, cx + 4.0, cy - 5.0, 0x707070);
        }
        "Clone" => {
            p.ring(cx - 3.0, cy - 3.0, 5.0, c);
            p.ring(cx + 3.0, cy + 3.0, 5.0, c);
        }
        "Fill" => {
            p.fill(Rect::new(cx as i32 - 7, cy as i32 - 3, 14, 10), c);
            p.line(cx - 7.0, cy - 3.0, cx, cy - 9.0, c);
        }
        _ => {
            p.ring(cx, cy, 8.0, c);
            p.circle(cx - 2.0, cy, 5.5, 0x707070);
        }
    }
}

// ---------------------------------------------------------------- sidebar

/// The Tool tab's panels in a paint mode. Returns the new y.
pub fn tool_panels(s: &mut State, d: &mut Doc, ui: &mut Ui, x: i32, w: i32, mut y: i32) -> i32 {
    const ROW: i32 = 20;
    let mode = d.mode;
    let lab = |ui: &mut Ui, y: &mut i32, s: &str| {
        ui.p.label(Rect::new(x, *y, w, ROW), s, t::TEXT_DIM, Align::Left);
        *y += ROW;
    };
    let head = |ui: &mut Ui, y: &mut i32, s: &str| {
        let r = Rect::new(x - 6, *y, w + 12, 22);
        ui.p.round_rect_alpha(r, 4, t::PANEL_HEADER, 245);
        ui.p.label(Rect::new(r.x + 10, r.y, r.w - 16, r.h), s, t::TEXT, Align::Left);
        *y += 26;
    };
    head(ui, &mut y, "Brush Settings");
    let name = match mode {
        Mode::Sculpt => d.sculpt.region.map(|t| t.label()).unwrap_or(d.sculpt.kind.label()),
        Mode::VertexPaint => d.sculpt.vp.brush.label(),
        _ => d.sculpt.tp.brush.label(),
    };
    let br = Rect::new(x, y, w, ROW);
    if ui.dropdown(br, name) {
        let items = match mode {
            Mode::Sculpt => brush_items(d),
            Mode::VertexPaint => VpBrush::ALL.iter().map(|b| MenuItem::Check(b.label().into(), d.sculpt.vp.brush == *b, Op::with("paint.vertex_brush", b.label()))).collect(),
            _ => TexBrush::ALL.iter().map(|b| MenuItem::Check(b.label().into(), d.sculpt.tp.brush == *b, Op::with("paint.texture_brush", b.label()))).collect(),
        };
        ui.open_menu(br, "Brush", items);
    }
    y += ROW + 4;
    let (mut rad, mut st) = d.sculpt.radius_strength(mode);
    if ui.number(id("side/sculpt/radius"), Rect::new(x, y, w, ROW), "Radius", &mut rad, 1.0, " px") {
        d.sculpt.set_radius(mode, rad);
    }
    y += ROW + 2;
    if ui.number(id("side/sculpt/strength"), Rect::new(x, y, w, ROW), "Strength", &mut st, 0.01, "") {
        d.sculpt.set_strength(mode, st);
    }
    y += ROW + 4;
    if mode != Mode::Sculpt {
        lab(ui, &mut y, "Color");
        let primary = if mode == Mode::TexturePaint { d.sculpt.tp.color } else { d.sculpt.vp.color };
        let secondary = if mode == Mode::TexturePaint { d.sculpt.tp.secondary } else { d.sculpt.vp.secondary };
        let half = (w - 4) / 2;
        let a = Rect::new(x, y, half, ROW);
        if ui.swatch(a, primary) {
            s.picker = Some(Picker::new(ColorTarget::Paint(false), a, d));
        }
        let b = Rect::new(x + half + 4, y, half, ROW);
        if ui.swatch(b, secondary) {
            s.picker = Some(Picker::new(ColorTarget::Paint(true), b, d));
        }
        y += ROW + 4;
        lab(ui, &mut y, "Blend");
        let bl = Rect::new(x, y, w, ROW);
        let cur = if mode == Mode::TexturePaint { d.sculpt.tp.blend } else { d.sculpt.vp.blend };
        if ui.dropdown(bl, cur.label()) {
            ui.open_menu(bl, "Blend", blend_items(d));
        }
        y += ROW + 4;
        // The palette.
        lab(ui, &mut y, "Palette");
        let pal = d.sculpt.vp.palette.clone();
        let n = pal.len().max(1) as i32;
        let sw = ((w - (n - 1) * 3) / n).max(10);
        for (i, c) in pal.iter().enumerate() {
            let r = Rect::new(x + i as i32 * (sw + 3), y, sw, ROW);
            if ui.swatch(r, *c) {
                if mode == Mode::TexturePaint {
                    d.sculpt.tp.color = *c;
                } else {
                    d.sculpt.vp.color = *c;
                }
            }
        }
        y += ROW + 4;
        if ui.button(Rect::new(x, y, w, ROW), "Add Color to Palette") {
            let c = if mode == Mode::TexturePaint { d.sculpt.tp.color } else { d.sculpt.vp.color };
            if !d.sculpt.vp.palette.contains(&c) {
                d.sculpt.vp.palette.push(c);
                if d.sculpt.vp.palette.len() > 10 {
                    d.sculpt.vp.palette.remove(0);
                }
            }
        }
        y += ROW + 6;
        if mode == Mode::TexturePaint {
            head(ui, &mut y, "Texture Slots");
            if let Some((oi, _)) = sculpt::active_mesh(d) {
                let slots = sculpt::texture_slots(d, oi);
                for (i, sl) in slots.iter().enumerate() {
                    let r = Rect::new(x, y, w, ROW);
                    let on = i == d.sculpt.tex_slot.min(slots.len().saturating_sub(1));
                    ui.p.round_rect(r, 3, if on { t::SELECTED } else if ui.hovered(r) { t::BUTTON_HOVER } else { t::BUTTON_DARK });
                    let dirty = if d.sculpt.dirty_images.contains(&sl.image) { "*" } else { "" };
                    ui.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), &format!("{}{dirty}", sl.name), t::TEXT, Align::Left);
                    if ui.clicked(r) {
                        d.sculpt.tex_slot = i;
                    }
                    y += ROW + 1;
                }
            }
            if ui.button(Rect::new(x, y, w, ROW), "Add Texture Paint Slot") {
                d.queue.push(Op::with("paint.add_texture_paint_slot", "Base Color"));
            }
            y += ROW + 2;
            if ui.button(Rect::new(x, y, w, ROW), "Save Image") {
                d.queue.push(Op::new("image.save"));
            }
            y += ROW + 4;
            let mut bleed = d.sculpt.tp.bleed as f32;
            if ui.number(id("side/tp/bleed"), Rect::new(x, y, w, ROW), "Bleed", &mut bleed, 1.0, " px") {
                d.sculpt.tp.bleed = bleed.clamp(0.0, 64.0) as u32;
            }
            y += ROW + 2;
            let mut occ = d.sculpt.tp.occlude;
            if ui.checkbox(Rect::new(x, y, w, ROW), "Occlude", &mut occ) {
                d.sculpt.tp.occlude = occ;
            }
            y += ROW;
            let mut cull = d.sculpt.tp.backface_culling;
            if ui.checkbox(Rect::new(x, y, w, ROW), "Backface Culling", &mut cull) {
                d.sculpt.tp.backface_culling = cull;
            }
            y += ROW + 4;
        }
        return y;
    }
    if d.sculpt.region == Some(RegionTool::MeshFilter) {
        lab(ui, &mut y, "Filter Type");
        let fr = Rect::new(x, y, w, ROW);
        if ui.dropdown(fr, d.sculpt.filter.label()) {
            let items = [sops::FilterKind::Smooth, sops::FilterKind::Scale, sops::FilterKind::Inflate, sops::FilterKind::Sphere, sops::FilterKind::Random, sops::FilterKind::Sharpen, sops::FilterKind::EnhanceDetails]
                .iter()
                .map(|k| MenuItem::Check(k.label().into(), d.sculpt.filter == *k, Op::with("sculpt.filter_set", k.id())))
                .collect();
            ui.open_menu(fr, "Filter", items);
        }
        y += ROW + 4;
        let mut fs = d.sculpt.filter_strength;
        if ui.number(id("side/sculpt/filter"), Rect::new(x, y, w, ROW), "Strength", &mut fs, 0.01, "") {
            d.sculpt.filter_strength = fs.clamp(-10.0, 10.0);
        }
        y += ROW + 6;
    }
    // Brush details.
    {
        let b = d.sculpt.brush_mut();
        let mut inv = b.invert as usize;
        if ui.segmented(Rect::new(x, y, w, ROW), &["Add", "Subtract"], &mut inv) {
            b.invert = inv == 1;
        }
        y += ROW + 4;
        lab(ui, &mut y, "Falloff");
        let fr = Rect::new(x, y, w, ROW);
        let cur = d.sculpt.brush().curve;
        if ui.dropdown(fr, cur.label()) {
            let items = [Curve::Smooth, Curve::Smoother, Curve::Sphere, Curve::Root, Curve::Sharp, Curve::Linear, Curve::Sharper, Curve::InverseSquare, Curve::Constant]
                .iter()
                .map(|c| MenuItem::Check(c.label().into(), cur == *c, Op::with("sculpt.curve", c.id())))
                .collect();
            ui.open_menu(fr, "Falloff", items);
        }
        y += ROW + 4;
        lab(ui, &mut y, "Stroke Method");
        let sr = Rect::new(x, y, w, ROW);
        let cur = d.sculpt.brush().stroke;
        if ui.dropdown(sr, cur.label()) {
            let items = [StrokeMethod::Space, StrokeMethod::Dots, StrokeMethod::DragDot, StrokeMethod::Anchored, StrokeMethod::Line, StrokeMethod::Airbrush]
                .iter()
                .map(|m| MenuItem::Check(m.label().into(), cur == *m, Op::with("sculpt.stroke_method", m.id())))
                .collect();
            ui.open_menu(sr, "Stroke Method", items);
        }
        y += ROW + 4;
        let b = d.sculpt.brush_mut();
        if ui.number(id("side/sculpt/spacing"), Rect::new(x, y, w, ROW), "Spacing", &mut b.spacing, 1.0, " %") {
            b.spacing = b.spacing.clamp(1.0, 500.0);
        }
        y += ROW + 2;
        if ui.number(id("side/sculpt/hardness"), Rect::new(x, y, w, ROW), "Hardness", &mut b.hardness, 0.01, "") {
            b.hardness = b.hardness.clamp(0.0, 1.0);
        }
        y += ROW + 4;
        for (label, v) in [("Front Faces Only", &mut b.front_faces_only), ("Auto-Mask Topology", &mut b.automask_topology), ("Auto-Mask Face Sets", &mut b.automask_face_sets), ("Stabilize Stroke", &mut b.lazy), ("Radius in Scene", &mut b.radius_in_scene)] {
            ui.checkbox(Rect::new(x, y, w, ROW), label, v);
            y += ROW;
        }
        if b.radius_in_scene {
            if ui.number(id("side/sculpt/scene_radius"), Rect::new(x, y, w, ROW), "Scene Radius", &mut b.scene_radius, 0.01, " m") {
                b.scene_radius = b.scene_radius.max(1e-4);
            }
            y += ROW + 2;
        }
        if b.kind.uses_plane() {
            let mut i = b.plane as usize;
            let names = ["Area", "View", "X", "Y", "Z"];
            if ui.segmented(Rect::new(x, y, w, ROW), &names, &mut i) {
                b.plane = [sex_3d::sculpt::SculptPlane::Area, sex_3d::sculpt::SculptPlane::View, sex_3d::sculpt::SculptPlane::X, sex_3d::sculpt::SculptPlane::Y, sex_3d::sculpt::SculptPlane::Z][i.min(4)];
            }
            y += ROW + 2;
            if ui.number(id("side/sculpt/plane_offset"), Rect::new(x, y, w, ROW), "Plane Offset", &mut b.plane_offset, 0.01, "") {
                b.plane_offset = b.plane_offset.clamp(-2.0, 2.0);
            }
            y += ROW + 2;
            if ui.number(id("side/sculpt/area_radius"), Rect::new(x, y, w, ROW), "Area Radius", &mut b.area_radius, 0.01, "") {
                b.area_radius = b.area_radius.clamp(0.0, 2.0);
            }
            y += ROW + 2;
        }
        if ui.number(id("side/sculpt/normal_radius"), Rect::new(x, y, w, ROW), "Normal Radius", &mut b.normal_radius, 0.01, "") {
            b.normal_radius = b.normal_radius.clamp(0.0, 2.0);
        }
        y += ROW + 6;
    }
    head(ui, &mut y, "Symmetry");
    lab(ui, &mut y, "Mirror");
    let third = (w - 4) / 3;
    for k in 0..3 {
        let mut on = d.sculpt.symmetry.mirror[k];
        if ui.checkbox(Rect::new(x + k as i32 * (third + 2), y, third, ROW), ["X", "Y", "Z"][k], &mut on) {
            d.sculpt.symmetry.mirror[k] = on;
        }
    }
    y += ROW + 2;
    lab(ui, &mut y, "Lock");
    for k in 0..3 {
        let mut on = d.sculpt.symmetry.lock[k];
        if ui.checkbox(Rect::new(x + k as i32 * (third + 2), y, third, ROW), ["X", "Y", "Z"][k], &mut on) {
            d.sculpt.symmetry.lock[k] = on;
        }
    }
    y += ROW + 6;
    head(ui, &mut y, "Dyntopo");
    let mut on = d.sculpt.dyntopo.enabled;
    if ui.checkbox(Rect::new(x, y, w, ROW), "Dynamic Topology", &mut on) {
        d.queue.push(Op::new("sculpt.dynamic_topology_toggle"));
    }
    y += ROW + 2;
    let dt = &mut d.sculpt.dyntopo;
    let mut mi = dt.mode as usize;
    if ui.segmented(Rect::new(x, y, w, ROW), &["Relative", "Constant", "Brush"], &mut mi) {
        dt.mode = [sex_3d::sculpt::DetailMode::Relative, sex_3d::sculpt::DetailMode::Constant, sex_3d::sculpt::DetailMode::Brush][mi.min(2)];
    }
    y += ROW + 2;
    let (label, v, step, unit) = match dt.mode {
        sex_3d::sculpt::DetailMode::Relative => ("Detail Size", &mut dt.detail_size, 0.5, " px"),
        sex_3d::sculpt::DetailMode::Constant => ("Resolution", &mut dt.constant_detail, 0.5, ""),
        sex_3d::sculpt::DetailMode::Brush => ("Detail Percentage", &mut dt.detail_percent, 1.0, " %"),
    };
    if ui.number(id("side/sculpt/detail"), Rect::new(x, y, w, ROW), label, v, step, unit) {
        *v = v.clamp(0.5, 500.0);
    }
    y += ROW + 2;
    ui.checkbox(Rect::new(x, y, w / 2, ROW), "Subdivide", &mut dt.subdivide);
    ui.checkbox(Rect::new(x + w / 2, y, w / 2, ROW), "Collapse", &mut dt.collapse);
    y += ROW + 2;
    if ui.button(Rect::new(x, y, w, ROW), "Detail Flood Fill") {
        d.queue.push(Op::new("sculpt.detail_flood_fill"));
    }
    y += ROW + 6;
    head(ui, &mut y, "Remesh");
    let mut vs = d.sculpt.voxel_size;
    if ui.number(id("side/sculpt/voxel"), Rect::new(x, y, w, ROW), "Voxel Size", &mut vs, 0.005, " m") {
        d.sculpt.voxel_size = vs.clamp(1e-4, 10.0);
    }
    y += ROW + 2;
    ui.checkbox(Rect::new(x, y, w, ROW), "Preserve Volume", &mut d.sculpt.voxel_preserve_volume);
    y += ROW + 2;
    if ui.button(Rect::new(x, y, w, ROW), "Voxel Remesh") {
        d.queue.push(Op::new("object.voxel_remesh"));
    }
    y += ROW + 6;
    head(ui, &mut y, "Display");
    ui.checkbox(Rect::new(x, y, w, ROW), "Show Mask", &mut d.sculpt.show_mask);
    y += ROW;
    ui.checkbox(Rect::new(x, y, w, ROW), "Show Face Sets", &mut d.sculpt.show_face_sets);
    y += ROW + 4;
    y
}

// ---------------------------------------------------------------- drawing

/// The brush cursor.
pub fn draw_brush(s: &State, d: &Doc, ui: &mut Ui, r: Rect) {
    if !active(d) {
        return;
    }
    let (x, y) = ui.mouse();
    if let Modal::Sculpt(m) = &s.modal {
        match &**m {
            SModal::Size { center, .. } => {
                let (rad, st) = d.sculpt.radius_strength(d.mode);
                let (cx, cy) = (center.0 as f32, center.1 as f32);
                ui.p.ring(cx, cy, rad, 0xffffff);
                ui.p.ring(cx, cy, rad * st.clamp(0.0, 1.0), 0x909090);
                return;
            }
            SModal::Region { tool, pts, .. } => {
                if tool.lasso() {
                    for w in pts.windows(2) {
                        ui.p.line(w[0].0, w[0].1, w[1].0, w[1].1, 0xffffff);
                    }
                    if let (Some(a), Some(b)) = (pts.first(), pts.last()) {
                        ui.p.line_alpha(a.0, a.1, b.0, b.1, 0xffffff, 120);
                    }
                } else if let (Some(a), Some(b)) = (pts.first(), pts.last()) {
                    let rr = Rect::new(a.0.min(b.0) as i32, a.1.min(b.1) as i32, (a.0 - b.0).abs() as i32 + 1, (a.1 - b.1).abs() as i32 + 1);
                    ui.p.fill_alpha(rr, 0xffffff, 24);
                    ui.p.frame(rr, 0xffffff);
                }
                return;
            }
            SModal::RegionWait(_) => {
                ui.p.line(0f32.max(r.x as f32), y as f32, r.right() as f32, y as f32, 0x808080);
                ui.p.line(x as f32, r.y as f32, x as f32, r.bottom() as f32, 0x808080);
                return;
            }
            SModal::Stroke(ss) => {
                if let Some(a) = ss.anchor.filter(|_| ss.method == StrokeMethod::Anchored) {
                    let _ = a;
                    let rad = (ss.mouse.0 - ss.press.0).hypot(ss.mouse.1 - ss.press.1);
                    ui.p.ring(ss.press.0, ss.press.1, rad, 0xffffff);
                    return;
                }
                if ss.brush.lazy {
                    ui.p.ring(ss.lazy.0, ss.lazy.1, 3.0, 0xffffff);
                }
            }
            _ => {}
        }
    }
    if !r.contains(x, y) || (d.mode == Mode::Sculpt && d.sculpt.region.is_some()) {
        return;
    }
    let (rad, st) = d.sculpt.radius_strength(d.mode);
    let col = match d.mode {
        Mode::Sculpt if d.sculpt.brush().invert => 0x6080ff,
        Mode::Sculpt => 0xff9060,
        _ => 0xffffff,
    };
    ui.p.ring(x as f32, y as f32, rad, col);
    ui.p.ring(x as f32, y as f32, rad * st.clamp(0.0, 1.0), 0x909090);
}

/// Texture Paint shows the paint image on the mesh, z-buffered and lit.
pub fn draw_texture(d: &Doc, ui: &mut Ui, p: &Proj) {
    if d.mode != Mode::TexturePaint {
        return;
    }
    let Some((oi, mi)) = sculpt::active_mesh(d) else { return };
    let Some(img) = sculpt::paint_image(d).and_then(|i| d.scene.images.get(i)) else { return };
    if img.width == 0 || img.height == 0 || img.pixels.len() != (img.width * img.height) as usize {
        return;
    }
    let m = &d.scene.meshes[mi];
    let w = d.scene.world_matrix(oi);
    let world: Vec<Vec3> = m.positions.iter().map(|&q| w.transform_point(q)).collect();
    let proj: Vec<Option<(f32, f32, f32)>> = world.iter().map(|&q| p.project(q)).collect();
    let clip = ui.p.clip().intersect(&p.rect);
    if clip.is_empty() {
        return;
    }
    let (cw, ch) = (clip.w as usize, clip.h as usize);
    let mut zb = vec![f32::NEG_INFINITY; cw * ch];
    let width = ui.p.width;
    let pix = &mut ui.p.pixels;
    let (iw, ih) = (img.width as f32, img.height as f32);
    let enc = |c: f32| (sex_3d::render::srgb_encode(c.clamp(0.0, 1.0)) * 255.0) as u32;
    for t in m.triangulate() {
        let f = &m.faces[t.face as usize];
        if f.uvs.len() != f.verts.len() || m.is_face_hidden(t.face as usize) {
            continue;
        }
        let [a, b, c] = t.verts.map(|v| v as usize);
        let (Some(pa), Some(pb), Some(pc)) = (proj[a], proj[b], proj[c]) else { continue };
        let uv = t.corners.map(|k| f.uvs[k as usize]);
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
                // Perspective-correct UVs (the depth key is 1/w in perspective).
                let (u, v) = if p.ortho {
                    (uv[0].x * w0 + uv[1].x * w1 + uv[2].x * w2, uv[0].y * w0 + uv[1].y * w1 + uv[2].y * w2)
                } else {
                    let (q0, q1, q2) = (w0 * pa.2, w1 * pb.2, w2 * pc.2);
                    let s = (q0 + q1 + q2).max(1e-12);
                    ((uv[0].x * q0 + uv[1].x * q1 + uv[2].x * q2) / s, (uv[0].y * q0 + uv[1].y * q1 + uv[2].y * q2) / s)
                };
                let tx = ((u - u.floor()) * iw) as usize % img.width as usize;
                let ty = (((1.0 - (v - v.floor())) * ih) as usize).min(img.height as usize - 1);
                let px = img.pixels[ty * img.width as usize + tx];
                let ch = |k: usize| enc(px[k] * light);
                pix[y as usize * width + x as usize] = ch(0) << 16 | ch(1) << 8 | ch(2);
            }
        }
    }
    let _ = ch;
}

/// The colour picker popover: claims events first, drawn last.
pub fn picker_claims(s: &mut State, ui: &mut Ui) -> Option<Input> {
    let pk = s.picker.as_ref()?;
    match picker::claims(pk, ui) {
        picker::Claim::Mine => ui.input.take(),
        picker::Claim::Close => {
            s.picker = None;
            None
        }
        picker::Claim::No => None,
    }
}

pub fn picker_draw(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect, stash: Option<Input>) {
    let Some(pk) = &mut s.picker else { return };
    if stash.is_some() {
        ui.input = stash;
    }
    // Brush colours are not undo steps: the picker never records one.
    let mut undo_for = Some(ui.st.active);
    if !picker::draw(pk, d, ui, r, &mut undo_for) {
        s.picker = None;
    }
}

/// Whether a running stroke wants timer ticks (Airbrush).
pub fn wants_tick(s: &State) -> bool {
    matches!(&s.modal, Modal::Sculpt(m) if matches!(&**m, SModal::Stroke(ss) if ss.method == StrokeMethod::Airbrush))
}
