// SPDX-License-Identifier: GPL-3.0-only
//! What the 3D Viewport draws: the shaded image for each shading mode, and
//! the overlays on top (Edit Mode elements, origins, the 3D cursor, the
//! camera frame and passepartout, annotations, the text info, modal tool
//! feedback).
use super::proj::Proj;
use super::{ensure_cache, ensure_world, Layout, Modal, Shading, State};
use crate::doc::{Doc, Elem, Mode, SelectMode};
use crate::draw::{Painter, Rect};
use crate::theme as t;
use crate::ui::Ui;
use sex_3d::render::viewport::ViewportSettings;
use sex_3d::Vec3;

/// Blender 4.x's flat viewport background (#3d3d3d).
const BG: [f32; 3] = [0x3d as f32 / 255.0; 3];
const EDGE_SEL: u32 = 0xffa000;
const VERT_SEL: u32 = 0xff7a00;
const FACE_SEL: u32 = 0xffa733;
const WIRE_EDIT: u32 = 0x000000;
/// Blender's theme colours for UV seams and sharp edges.
pub const EDGE_SEAM: u32 = 0xdb2512;
pub const EDGE_SHARP: u32 = 0x00ffff;

pub fn scene_image(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    let settings = ViewportSettings {
        width: r.w as u32,
        height: r.h as u32,
        wireframe: s.show_wire && s.show_overlays,
        grid: s.show_floor && s.show_overlays,
        backface_culling: s.backface_culling,
        threads: 0,
        background: Some(BG),
        workbench: sex_3d::render::Workbench { xray: s.xray, ..s.solid },
        ..ViewportSettings::default()
    };
    match s.shading {
        Shading::Solid => {
            ensure_cache(s, d);
            let Some((_, full, _)) = &s.cache else { return };
            full.draw_into(&s.cam, &settings, &mut s.buf);
            // Grease Pencil over the shaded image, depth tested (in object order).
            s.gp_depth = sex_3d::grease_pencil::raster::viewport_depth_to_distance(&full.last_depth(), s.cam.ortho);
            let mut buf = core::mem::take(&mut s.buf);
            let depth = core::mem::take(&mut s.gp_depth);
            super::gpview::composite(s, d, &mut buf, r.w as u32, r.h as u32, (!s.xray).then_some(depth.as_slice()));
            s.gp_depth = depth;
            s.buf = buf;
            ui.p.blit_rgba(r, &s.buf, r.w as usize, r.h as usize);
        }
        Shading::Wireframe => {
            ensure_cache(s, d);
            ensure_world(s, d);
            let Some((_, _, shell)) = &s.cache else { return };
            shell.draw_into(&s.cam, &ViewportSettings { workbench: Default::default(), ..settings }, &mut s.buf);
            let mut buf = core::mem::take(&mut s.buf);
            super::gpview::composite(s, d, &mut buf, r.w as u32, r.h as u32, None);
            s.buf = buf;
            ui.p.blit_rgba(r, &s.buf, r.w as usize, r.h as usize);
            let p = Proj::new(&s.cam, r);
            let edit_obj = if d.mode == Mode::Edit { d.active() } else { None };
            if let Some((_, objs)) = &s.world {
                for w in objs {
                    let col = if Some(w.index) == edit_obj {
                        0x202020
                    } else if d.scene.active_object == Some(w.index) && d.is_selected(w.index) {
                        t::ACTIVE_OBJECT
                    } else if d.is_selected(w.index) {
                        t::SELECTED_OBJECT
                    } else {
                        0x0a0a0a
                    };
                    let sp: Vec<Option<(f32, f32)>> = w.points.iter().map(|&q| p.to_screen(q)).collect();
                    for e in &w.edges {
                        if let (Some(a), Some(b)) = (sp[e[0] as usize], sp[e[1] as usize]) {
                            ui.p.line(a.0, a.1, b.0, b.1, col);
                        }
                    }
                }
            }
        }
        Shading::Material | Shading::Rendered => {
            ui.p.fill(r, t::VIEWPORT);
            if s.show_overlays && s.render.is_some() {
                // Grid, gizmos, wires and selection outlines over the render,
                // Grease Pencil on top.
                ensure_cache(s, d);
                let (w, h) = (r.w.max(1) as usize, r.h.max(1) as usize);
                let mut img = {
                    let Some(rv) = &s.render else { return };
                    if (rv.w as usize, rv.h as usize) == (w, h) { rv.rgba.clone() } else { scale_rgba(&rv.rgba, rv.w as usize, rv.h as usize, w, h) }
                };
                super::gpview::composite(s, d, &mut img, w as u32, h as u32, None);
                let Some((_, full, _)) = &s.cache else { return };
                let st = ViewportSettings { wireframe: false, ..settings };
                full.draw_over(&s.cam, &st, &img, &mut s.buf);
                ui.p.blit_rgba(r, &s.buf, w, h);
            } else if let Some(rv) = &s.render {
                let mut img = rv.rgba.clone();
                let (w, h) = (rv.w, rv.h);
                super::gpview::composite(s, d, &mut img, w as u32, h as u32, None);
                blit_scaled(ui.p, r, &img, w as usize, h as usize);
            }
        }
    }
}

/// Nearest-pixel resize of an RGBA image.
fn scale_rgba(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 4];
    if sw == 0 || sh == 0 || src.len() < sw * sh * 4 {
        return out;
    }
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh - 1);
        for x in 0..dw {
            let sx = (x * sw / dw).min(sw - 1);
            let (i, o) = ((sy * sw + sx) * 4, (y * dw + x) * 4);
            out[o..o + 4].copy_from_slice(&src[i..i + 4]);
        }
    }
    out
}

/// Bilinear upscale of a small RGBA image into `r`.
fn blit_scaled(p: &mut Painter, r: Rect, rgba: &[u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return;
    }
    let c = r.intersect(&p.clip());
    let sx = w as f32 / r.w.max(1) as f32;
    let sy = h as f32 / r.h.max(1) as f32;
    for y in c.y..c.bottom() {
        let fy = ((y - r.y) as f32 + 0.5) * sy - 0.5;
        let y0 = fy.floor().clamp(0.0, (h - 1) as f32) as usize;
        let y1 = (y0 + 1).min(h - 1);
        let ty = (fy - y0 as f32).clamp(0.0, 1.0);
        let row = y as usize * p.width;
        for x in c.x..c.right() {
            let fx = ((x - r.x) as f32 + 0.5) * sx - 0.5;
            let x0 = fx.floor().clamp(0.0, (w - 1) as f32) as usize;
            let x1 = (x0 + 1).min(w - 1);
            let tx = (fx - x0 as f32).clamp(0.0, 1.0);
            let px = |xx: usize, yy: usize, k: usize| rgba[(yy * w + xx) * 4 + k] as f32;
            let mut out = 0u32;
            for k in 0..3 {
                let a = px(x0, y0, k) * (1.0 - tx) + px(x1, y0, k) * tx;
                let b = px(x0, y1, k) * (1.0 - tx) + px(x1, y1, k) * tx;
                out = out << 8 | (a * (1.0 - ty) + b * ty).round().clamp(0.0, 255.0) as u32;
            }
            p.pixels[row + x as usize] = out;
        }
    }
}

/// Camera view: darken outside the camera frame (passepartout) and outline it.
pub fn camera_frame(s: &State, d: &Doc, ui: &mut Ui, r: Rect) {
    let Some((hw, hh)) = super::camera_frame(s, d, r) else { return };
    let (cx, cy) = (r.x as f32 + r.w as f32 * 0.5, r.y as f32 + r.h as f32 * 0.5);
    let f = Rect::new((cx - hw).round() as i32, (cy - hh).round() as i32, (2.0 * hw).round() as i32, (2.0 * hh).round() as i32);
    let a = 128;
    ui.p.fill_alpha(Rect::new(r.x, r.y, r.w, f.y - r.y), 0x000000, a);
    ui.p.fill_alpha(Rect::new(r.x, f.bottom(), r.w, r.bottom() - f.bottom()), 0x000000, a);
    ui.p.fill_alpha(Rect::new(r.x, f.y, f.x - r.x, f.h), 0x000000, a);
    ui.p.fill_alpha(Rect::new(f.right(), f.y, r.right() - f.right(), f.h), 0x000000, a);
    ui.p.frame(f, 0x000000);
    ui.p.frame(Rect::new(f.x - 1, f.y - 1, f.w + 2, f.h + 2), 0x7a7a7a);
    // A dashed inner frame like Blender's camera frame.
    let name = d.scene.active_camera.and_then(|c| d.scene.objects.get(c)).map(|o| o.name.clone()).unwrap_or_default();
    ui.p.text(f.x + 4, f.y - ui.p.line_height() - 4, &name, t::TEXT);
}

/// Small dots at the origins of selected objects (Blender's origin overlay).
pub fn origins(d: &Doc, ui: &mut Ui, p: &Proj) {
    for &i in &d.scene.selected {
        if i >= d.scene.objects.len() || !d.scene.is_visible(i) {
            continue;
        }
        if let Some((x, y)) = p.to_screen(d.world_location(i)) {
            let active = d.scene.active_object == Some(i);
            ui.p.circle(x, y, 3.5, 0x000000);
            ui.p.circle(x, y, 2.5, if active { t::ACTIVE_OBJECT } else { t::SELECTED_OBJECT });
        }
    }
}

/// Blender's 3D cursor: a red/white dashed ring with crosshair lines.
pub fn cursor(ui: &mut Ui, p: &Proj, at: Vec3) {
    let Some((x, y)) = p.to_screen(at) else { return };
    let rad = 10.0;
    let n = 16;
    for k in 0..n {
        let a0 = k as f32 / n as f32 * core::f32::consts::TAU;
        let a1 = (k + 1) as f32 / n as f32 * core::f32::consts::TAU;
        let col = if k % 2 == 0 { 0xff3333 } else { 0xffffff };
        super::icons::thick(ui.p, x + rad * a0.cos(), y + rad * a0.sin(), x + rad * a1.cos(), y + rad * a1.sin(), 2.0, col);
    }
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        ui.p.line(x + dx * 5.0, y + dy * 5.0, x + dx * 18.0, y + dy * 18.0, 0x000000);
        ui.p.line(x + dx * 5.0 + dy, y + dy * 5.0 + dx, x + dx * 18.0 + dy, y + dy * 18.0 + dx, 0xffffff);
    }
}

pub fn annotations(s: &State, ui: &mut Ui, p: &Proj) {
    let col = 0x4fe3ff;
    for st in &s.annotations {
        let pts: Vec<Option<(f32, f32)>> = st.iter().map(|&q| p.to_screen(q)).collect();
        for w in pts.windows(2) {
            if let (Some(a), Some(b)) = (w[0], w[1]) {
                super::icons::thick(ui.p, a.0, a.1, b.0, b.1, 2.0, col);
            }
        }
    }
    if let Some((a, b)) = s.measure {
        if let (Some(sa), Some(sb)) = (p.to_screen(a), p.to_screen(b)) {
            super::icons::thick(ui.p, sa.0, sa.1, sb.0, sb.1, 2.0, 0xffffff);
            ui.p.circle(sa.0, sa.1, 3.0, 0xffffff);
            ui.p.circle(sb.0, sb.1, 3.0, 0xffffff);
            let txt = format!("{} m", crate::ui::trim_number(a.distance(b)));
            let (mx, my) = ((sa.0 + sb.0) * 0.5, (sa.1 + sb.1) * 0.5);
            let w = ui.p.text_width(&txt);
            ui.p.round_rect_alpha(Rect::new(mx as i32 - w / 2 - 4, my as i32 - 18, w + 8, 16), 3, 0x000000, 140);
            ui.p.text(mx as i32 - w / 2, my as i32 - 18, &txt, t::TEXT_HI);
        }
    }
}

/// Edit Mode: faces tinted when selected, edges and vertices (or face dots),
/// hidden behind the mesh unless X-ray is on.
pub fn edit_overlay(s: &mut State, d: &Doc, ui: &mut Ui, p: &Proj) {
    let xray = s.xray || s.shading == Shading::Wireframe;
    let Some(ev) = super::edit_view(s, d, p) else { return };
    let sel = &d.edit;
    let active_face = match sel.active {
        Some(Elem::Face(f)) => Some(f),
        _ => None,
    };
    // Selected faces: tint the pixels the face buffer says are theirs.
    if !sel.faces.is_empty() {
        let r = ev.rect;
        let clip = ui.p.clip().intersect(&r);
        for y in clip.y..clip.bottom() {
            let row = (y - r.y) as usize * r.w as usize;
            let prow = y as usize * ui.p.width;
            for x in clip.x..clip.right() {
                let f = ev.face[row + (x - r.x) as usize];
                if f != u32::MAX && sel.faces.contains(&f) {
                    let px = &mut ui.p.pixels[prow + x as usize];
                    let (c, a) = if active_face == Some(f) { (0xffffff, 60) } else { (FACE_SEL, 70) };
                    *px = sex_office::mix(*px, c, a);
                }
            }
        }
    }
    let hidden = &d.edit_hidden;
    let edge_sel = |a: u32, b: u32| {
        let k = if a < b { (a, b) } else { (b, a) };
        sel.edges.contains(&k)
    };
    let mesh = crate::edit::active_mesh(d).map(|(_, mi)| &d.scene.meshes[mi]);
    let flag_col = |a: u32, b: u32| match mesh {
        Some(m) if m.is_seam(a, b) => Some(EDGE_SEAM),
        Some(m) if m.is_sharp(a, b) => Some(EDGE_SHARP),
        _ => None,
    };
    // Edges, unselected first so selected ones draw on top.
    for pass in 0..2 {
        for e in &ev.edges {
            if hidden.contains(&e[0]) || hidden.contains(&e[1]) {
                continue;
            }
            let selected = edge_sel(e[0], e[1]);
            if (pass == 1) != selected {
                continue;
            }
            let (Some(a), Some(b)) = (ev.screen[e[0] as usize], ev.screen[e[1] as usize]) else { continue };
            let flagged = flag_col(e[0], e[1]);
            let col = if selected { EDGE_SEL } else { flagged.unwrap_or(WIRE_EDIT) };
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            let n = ((len / 5.0).ceil() as i32).clamp(1, 400);
            let mut run: Option<(f32, f32)> = None;
            for k in 0..=n {
                let tt = k as f32 / n as f32;
                let (x, y, z) = (a.0 + (b.0 - a.0) * tt, a.1 + (b.1 - a.1) * tt, a.2 + (b.2 - a.2) * tt);
                let vis = xray || ev.visible(x, y, z);
                match (vis, run) {
                    (true, None) => run = Some((x, y)),
                    (false, Some(st)) => {
                        ui.p.line(st.0, st.1, x, y, col);
                        run = None;
                    }
                    _ => {}
                }
            }
            if let Some(st) = run {
                ui.p.line(st.0, st.1, b.0, b.1, col);
                if selected || flagged.is_some() {
                    ui.p.line(st.0 + 0.5, st.1 + 0.5, b.0 + 0.5, b.1 + 0.5, col);
                }
            }
        }
    }
    match d.select_mode {
        SelectMode::Vertex => {
            let active = match sel.active {
                Some(Elem::Vert(v)) => Some(v),
                _ => None,
            };
            for (i, sp) in ev.screen.iter().enumerate() {
                let Some((x, y, _)) = *sp else { continue };
                let v = i as u32;
                if hidden.contains(&v) || !ev.vert_visible(v, xray) {
                    continue;
                }
                let col = if Some(v) == active {
                    0xffffff
                } else if sel.verts.contains(&v) {
                    VERT_SEL
                } else {
                    0x000000
                };
                ui.p.fill(Rect::new(x.round() as i32 - 2, y.round() as i32 - 2, 5, 5), col);
            }
        }
        SelectMode::Face => {
            for (i, c) in ev.center_screen.iter().enumerate() {
                let Some((x, y, z)) = *c else { continue };
                if ev.hidden_face[i] || !(xray || ev.visible(x, y, z)) {
                    continue;
                }
                let col = if sel.faces.contains(&(i as u32)) { VERT_SEL } else { 0x000000 };
                ui.p.fill(Rect::new(x.round() as i32 - 1, y.round() as i32 - 1, 4, 4), col);
            }
        }
        SelectMode::Edge => {}
    }
}

fn dashed(p: &mut Painter, a: (f32, f32), b: (f32, f32), col: u32) {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (len / 6.0).ceil().max(1.0) as i32;
    for k in (0..n).step_by(2) {
        let t0 = k as f32 / n as f32;
        let t1 = ((k + 1) as f32 / n as f32).min(1.0);
        p.line(a.0 + (b.0 - a.0) * t0, a.1 + (b.1 - a.1) * t0, a.0 + (b.0 - a.0) * t1, a.1 + (b.1 - a.1) * t1, col);
    }
}

pub fn modal_overlay(s: &State, d: &Doc, ui: &mut Ui, p: &Proj) {
    let mouse = ui.mouse();
    match &s.modal {
        Modal::Box { x0, y0, x1, y1, .. } => {
            let b = Rect::new(*x0.min(x1), *y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
            ui.p.fill_alpha(b, 0xffffff, 18);
            let (l, tp, rt, bt) = (b.x as f32, b.y as f32, b.right() as f32, b.bottom() as f32);
            for (a, c) in [((l, tp), (rt, tp)), ((rt, tp), (rt, bt)), ((rt, bt), (l, bt)), ((l, bt), (l, tp))] {
                ui.p.line(a.0, a.1, c.0, c.1, 0x000000);
                dashed(ui.p, a, c, 0xffffff);
            }
        }
        Modal::BoxWait => {
            let (x, y) = (mouse.0 as f32, mouse.1 as f32);
            dashed(ui.p, (p.rect.x as f32, y), (p.rect.right() as f32, y), 0xffffff);
            dashed(ui.p, (x, p.rect.y as f32), (x, p.rect.bottom() as f32), 0xffffff);
        }
        Modal::Transform(m) => {
            if let Some(c) = p.to_screen(m.center) {
                if m.mode != crate::transform::TMode::Translate {
                    dashed(ui.p, c, (mouse.0 as f32, mouse.1 as f32), 0x000000);
                }
                // The constraint axis line through the pivot.
                let dir = match m.constraint {
                    crate::transform::Constraint::Axis(a) => Some((m.basis[a], [t::AXIS_X, t::AXIS_Y, t::AXIS_Z][a])),
                    crate::transform::Constraint::Dir(n) => Some((n, 0xffffff)),
                    _ => None,
                };
                if let Some((dir, col)) = dir {
                    let k = p.world_per_pixel(m.center) * 4000.0;
                    if let (Some(a), Some(b)) = (p.to_screen(m.center - dir * k), p.to_screen(m.center + dir * k)) {
                        ui.p.line(a.0, a.1, b.0, b.1, col);
                    } else if let Some(b) = p.to_screen(m.center + dir * k * 0.01) {
                        ui.p.line(c.0, c.1, b.0, b.1, col);
                    }
                }
                if d.proportional && m.is_edit() {
                    let rad = d.proportional_size / p.world_per_pixel(m.center);
                    ui.p.ring(c.0, c.1, rad, 0xffffff);
                }
            }
        }
        Modal::MeshTool(_) => {
            if let Some(c) = d.selection_center().and_then(|c| p.to_screen(c)) {
                dashed(ui.p, c, (mouse.0 as f32, mouse.1 as f32), 0x000000);
            }
        }
        Modal::LoopCut { preview, .. } => {
            for (a, b) in preview {
                if let (Some(a), Some(b)) = (p.to_screen(*a), p.to_screen(*b)) {
                    super::icons::thick(ui.p, a.0, a.1, b.0, b.1, 2.0, 0xffff40);
                }
            }
        }
        Modal::Knife { pts } => {
            for w in pts.windows(2) {
                super::icons::thick(ui.p, w[0].0, w[0].1, w[1].0, w[1].1, 2.0, 0xff40ff);
            }
            if let Some(l) = pts.last() {
                dashed(ui.p, *l, (mouse.0 as f32, mouse.1 as f32), 0xff40ff);
            }
            for q in pts {
                ui.p.circle(q.0, q.1, 3.0, 0x40ff40);
            }
        }
        _ => {}
    }
}

fn shadow_text(p: &mut Painter, x: i32, y: i32, s: &str, col: u32) {
    p.text(x + 1, y + 1, s, 0x000000);
    p.text(x, y, s, col);
}

/// Top-left text: the view's name, then "(frame) Collection | Object".
pub fn text_info(s: &State, d: &Doc, ui: &mut Ui, r: Rect, lay: &Layout) {
    let x = if s.show_toolbar { lay.toolbar.right() + 12 } else { r.x + 12 } + 0;
    let mut y = r.y + 8;
    let lh = ui.p.line_height() + 2;
    let proj = if s.cam.ortho { "Orthographic" } else { "Perspective" };
    let name = if s.camera_view {
        "Camera Perspective".to_string()
    } else if let Some(a) = s.axis_view {
        format!("{a} {proj}")
    } else {
        format!("User {proj}")
    };
    shadow_text(ui.p, x, y, &name, t::TEXT);
    y += lh;
    let mut line = format!("({}) ", d.frame);
    if let Some(a) = d.active() {
        let o = &d.scene.objects[a];
        let coll = d.scene.collections.get(o.collection).map(|c| c.name.as_str()).unwrap_or("Scene Collection");
        line.push_str(&format!("{coll} | {}", o.name));
    } else {
        line.push_str("Scene Collection");
    }
    shadow_text(ui.p, x, y, &line, t::TEXT);
    y += lh;
    if let (true, Some(rv)) = (matches!(s.shading, Shading::Material | Shading::Rendered), &s.render) {
        let st = if rv.samples() >= rv.target { format!("Sample {}/{}", rv.samples(), rv.target) } else { format!("Sample {}/{}  {}", rv.samples(), rv.target, rv.activity()) };
        shadow_text(ui.p, x, y, &st, t::TEXT);
        y += lh;
    }
    if s.show_stats {
        let sc = &d.scene;
        let (mut v, mut f, mut tr) = (0usize, 0usize, 0usize);
        for i in 0..sc.objects.len() {
            if let sex_3d::ObjectData::Mesh(mi) = sc.objects[i].data {
                if sc.is_visible(i) {
                    let m = &sc.meshes[mi];
                    v += m.vertex_count();
                    f += m.face_count();
                    tr += m.faces.iter().map(|f| f.verts.len().saturating_sub(2)).sum::<usize>();
                }
            }
        }
        for l in [format!("Objects   {}/{}", sc.selected.len(), sc.objects.len()), format!("Vertices  {v}"), format!("Faces     {f}"), format!("Triangles {tr}")] {
            shadow_text(ui.p, x, y, &l, t::TEXT);
            y += lh;
        }
    }
}
