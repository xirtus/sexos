// SPDX-License-Identifier: GPL-3.0-only
//! Blender's Image Editor and UV Editor.
//!
//! The Image Editor shows the last F12 render ("Render Result") through the
//! scene's colour management, fitted to the area, over a checkerboard where it is
//! transparent; the wheel zooms at the pointer, middle-drag pans, Home fits and
//! Numpad 1 shows it 1:1. Image > Save As writes it as a PNG with the small
//! encoder below (stored deflate: no compression crate on SexOS).
//!
//! The UV Editor draws the active mesh's face-corner UVs over the 0..1 grid in
//! Blender's UV editor colours: in Edit Mode the selected faces (Blender's
//! default without sync selection), otherwise every face. UVs are selected
//! per face corner in vertex, edge or face mode (click, Shift-click, drag for
//! a box, L / Ctrl L for linked islands, A / Alt A), with Blender's "Shared
//! Location" sticky selection. G, R and S move, rotate and scale the selection
//! (X/Y constrain; click or Enter confirms, right-click or Esc cancels), P and
//! Alt P pin and unpin, V stitches, M merges. The unwrap operators live in
//! `sex_3d::uv` and `crate::uvedit`.
use super::header_menus;
use crate::anim::{self, ReportKind};
use crate::doc::{Doc, Mode};
use crate::draw::{Align, Painter, Rect};
use crate::editors::EditorKind;
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};
use sex_3d::render::to_display8;
use sex_3d::scene::ObjectData;
use sex_3d::uv::{self as uvops, Corner};
use sex_3d::{Mesh, Vec2};
use std::collections::BTreeSet;

pub struct State {
    pub kind: EditorKind,
    id: u64,
    /// Screen pixels per image pixel (Image Editor) or per UV unit (UV Editor).
    pub zoom: f32,
    /// Offset of the image centre from the area centre, in screen pixels.
    pub pan: (f32, f32),
    fit: bool,
    panning: Option<(i32, i32)>,
    /// Converted render: (key, width, height, RGBA8).
    cache: Option<(u64, u32, u32, Vec<u8>)>,
    /// UV Editor: selected UV corners (face, corner) of the active mesh.
    pub uv_sel: BTreeSet<Corner>,
    /// 0 vertex, 1 edge, 2 face (Blender's UV select modes).
    pub select_mode: usize,
    last_rect: Rect,
    /// Pointer position at the last draw.
    mouse: (i32, i32),
    /// Left button pressed in the UV area: where, with which modifiers, and
    /// the box corner once it is dragged.
    press: Option<(i32, i32, Mods, Option<(i32, i32)>)>,
    /// A running G/R/S transform.
    pub modal: Option<UvTransform>,
}

impl State {
    pub fn new(kind: EditorKind) -> State {
        State {
            kind,
            id: anim::editor_id(),
            zoom: 1.0,
            pan: (0.0, 0.0),
            fit: true,
            panning: None,
            cache: None,
            uv_sel: BTreeSet::new(),
            select_mode: 0,
            last_rect: Rect::default(),
            mouse: (0, 0),
            press: None,
            modal: None,
        }
    }
    /// Faces whose every corner is selected.
    pub fn selected_faces(&self, m: &Mesh) -> Vec<u32> {
        (0..m.faces.len() as u32).filter(|&f| face_selected(&self.uv_sel, m, f)).collect()
    }
}

/// G, R or S on UVs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvTool {
    Move,
    Rotate,
    Scale,
}

/// A modal UV transform in progress.
#[derive(Clone, Debug)]
pub struct UvTransform {
    pub tool: UvTool,
    start: (i32, i32),
    pivot: Vec2,
    orig: Vec<(Corner, Vec2)>,
    /// Constrained to UV axis 0 (U) or 1 (V).
    axis: Option<usize>,
}

// ---------------------------------------------------------------- PNG

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, e) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *e = c;
    }
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffff_ffff
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// Encode RGBA8 pixels as a PNG: filter 0 scanlines in stored (uncompressed)
/// deflate blocks inside a zlib stream. Larger than a compressed PNG, but any
/// PNG reader opens it and it needs no compression code.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in rgba.chunks(width as usize * 4).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(b) = blocks.next() {
        z.push(if blocks.peek().is_none() { 1 } else { 0 });
        let n = b.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

// ---------------------------------------------------------------- render cache

fn render_key(img: &sex_3d::scene::Image) -> u64 {
    let mut h: u64 = (img.width as u64) << 32 | img.height as u64;
    let n = img.pixels.len();
    let step = (n / 1024).max(1);
    for i in (0..n).step_by(step) {
        for c in img.pixels[i] {
            h = h.wrapping_mul(0x100000001b3) ^ c.to_bits() as u64;
        }
    }
    h
}

fn render_rgba(s: &mut State, d: &Doc) -> Option<(u32, u32)> {
    let img = if d.show_viewer { d.viewer.as_ref().or(d.render.as_ref()) } else { d.render.as_ref().or(d.viewer.as_ref()) }?;
    let cm = d.settings.color_management();
    let key = render_key(img) ^ (cm.view as u64) << 3 ^ (cm.look as u64) << 9 ^ (cm.exposure.to_bits() as u64) << 17 ^ (cm.gamma.to_bits() as u64) << 31;
    if s.cache.as_ref().map(|c| c.0) != Some(key) {
        s.cache = Some((key, img.width, img.height, to_display8(img, &cm)));
        s.fit = true;
    }
    s.cache.as_ref().map(|c| (c.1, c.2))
}

// ---------------------------------------------------------------- header

fn image_menus(s: &State) -> (Vec<MenuItem>, Vec<MenuItem>) {
    let m = |l: &str, k: &str, op: &str| MenuItem::op(l, k, anim::mail(op, s.id, ""));
    let view = vec![
        m("Frame All", "Home", "image.view_all"),
        m("Zoom 1:1", "Numpad 1", "image.view_zoom_1"),
        m("Zoom In", "Numpad +", "image.view_zoom_in"),
        m("Zoom Out", "Numpad -", "image.view_zoom_out"),
        MenuItem::Separator,
        MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area")),
    ];
    let image = vec![
        MenuItem::op("New", "Alt N", Op::new("image.new")),
        MenuItem::op("Open...", "Alt O", Op::new("image.open")),
        MenuItem::Separator,
        m("Save", "Alt S", "image.save_as"),
        m("Save As...", "Shift Alt S", "image.save_as"),
    ];
    (view, image)
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let y = hr.y + 3;
    let h = hr.h - 6;
    let mut x = hr.x;
    if s.kind == EditorKind::UvEditor {
        let r = Rect::new(x, y, 72, h);
        ui.segmented(r, &["V", "E", "F"], &mut s.select_mode);
        x = r.right() + 8;
    }
    let (view, image) = image_menus(s);
    let mail = |l: &str, k: &str, op: &str, a: &str| MenuItem::op(l, k, anim::mail(op, s.id, a));
    let uv = vec![
        MenuItem::op("Unwrap", "U", Op::new("uv.unwrap")),
        MenuItem::op("Unwrap (Conformal)", "", Op::with("uv.unwrap", "CONFORMAL")),
        MenuItem::op("Smart UV Project", "", Op::new("uv.smart_project")),
        MenuItem::op("Lightmap Pack", "", Op::new("uv.lightmap_pack")),
        MenuItem::op("Follow Active Quads", "", Op::new("uv.follow_active_quads")),
        MenuItem::Separator,
        MenuItem::op("Pack Islands", "Ctrl P", Op::new("uv.pack_islands")),
        MenuItem::op("Average Islands Scale", "Ctrl A", Op::new("uv.average_islands_scale")),
        MenuItem::Separator,
        MenuItem::op("Cube Projection", "", Op::new("uv.cube_project")),
        MenuItem::op("Cylinder Projection", "", Op::new("uv.cylinder_project")),
        MenuItem::op("Sphere Projection", "", Op::new("uv.sphere_project")),
        MenuItem::Separator,
        mail("Pin", "P", "uv.pin", ""),
        mail("Unpin", "Alt P", "uv.pin", "CLEAR"),
        MenuItem::Separator,
        mail("Stitch", "V", "uv.stitch", ""),
        MenuItem::Sub("Merge".into(), vec![mail("At Center", "", "uv.weld", ""), mail("By Distance", "", "uv.remove_doubles", "")]),
        MenuItem::Separator,
        MenuItem::op("Seams from Islands", "", Op::new("uv.seams_from_islands")),
        MenuItem::op("Mark Seam", "", Op::new("mesh.mark_seam")),
        MenuItem::op("Clear Seam", "", Op::with("mesh.mark_seam", "CLEAR")),
        MenuItem::Separator,
        MenuItem::op("Reset", "", Op::new("uv.reset")),
    ];
    let sel = vec![
        mail("All", "A", "uv.select_all", "SELECT"),
        mail("None", "Alt A", "uv.select_all", "DESELECT"),
        mail("Invert", "Ctrl I", "uv.select_all", "INVERT"),
        MenuItem::Separator,
        mail("Select Linked", "Ctrl L", "uv.select_linked", ""),
        mail("Select Pinned", "Shift P", "uv.select_pinned", ""),
    ];
    let vf = move |_: &Doc| view.clone();
    let imf = move |_: &Doc| image.clone();
    let uvf = move |_: &Doc| uv.clone();
    let self_ = move |_: &Doc| sel.clone();
    let end = if s.kind == EditorKind::UvEditor {
        let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 4] = [("View", &vf), ("Select", &self_), ("Image", &imf), ("UV", &uvf)];
        header_menus(ui, hr, x, &menus, d)
    } else {
        let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 2] = [("View", &vf), ("Image", &imf)];
        header_menus(ui, hr, x, &menus, d)
    };
    x = end + 10;
    // The image datablock.
    let name = match (s.kind == EditorKind::ImageEditor, d.render.is_some(), d.viewer.is_some()) {
        (true, _, true) if d.show_viewer => "Viewer Node",
        (true, true, _) => "Render Result",
        (true, _, true) => "Viewer Node",
        _ => "",
    };
    let br = Rect::new(x, y, 36, h);
    if ui.dropdown(br, "") {
        let mut items = Vec::new();
        if d.render.is_some() {
            items.push(MenuItem::op("Render Result", "", Op::with("image.browse", "Render Result")));
        }
        if d.viewer.is_some() {
            items.push(MenuItem::op("Viewer Node", "", Op::with("image.browse", "Viewer Node")));
        }
        if items.is_empty() {
            items.push(MenuItem::Heading("No images (F12 renders one)".into()));
        }
        ui.open_menu(br, "", items);
    }
    image_icon(ui.p, br);
    x = br.right() + 2;
    if name.is_empty() {
        let nb = Rect::new(x, y, 60, h);
        if ui.button(nb, "+ New") {
            d.queue.push(Op::new("image.new"));
        }
        let ob = Rect::new(nb.right() + 2, y, 60, h);
        if ui.button(ob, "Open") {
            d.queue.push(Op::new("image.open"));
        }
    } else {
        let nr = Rect::new(x, y, 140, h);
        ui.p.round_rect(nr, 4, t::BUTTON_DARK);
        ui.p.label(Rect::new(nr.x + 6, nr.y, nr.w - 8, nr.h), name, t::TEXT, Align::Left);
        let sr = Rect::new(nr.right() + 8, y, 70, h);
        if ui.dropdown(sr, "Slot 1") {
            ui.open_menu(sr, "Slot", (1..=8).map(|i| MenuItem::op(&format!("Slot {i}"), "", Op::new("image.slot"))).collect());
        }
        let vr = Rect::new(sr.right() + 6, y, 96, h);
        ui.dropdown(vr, "ViewLayer");
        let cr = Rect::new(vr.right() + 6, y, 96, h);
        ui.dropdown(cr, "Combined");
    }
}

fn image_icon(p: &mut Painter, r: Rect) {
    let b = Rect::new(r.x + 5, r.y + r.h / 2 - 5, 12, 10);
    p.frame(b, t::TEXT_DIM);
    p.line((b.x + 2) as f32, (b.bottom() - 2) as f32, (b.x + 5) as f32, (b.y + 4) as f32, t::TEXT_DIM);
    p.line((b.x + 5) as f32, (b.y + 4) as f32, (b.right() - 2) as f32, (b.bottom() - 2) as f32, t::TEXT_DIM);
}

// ---------------------------------------------------------------- body

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    s.last_rect = r;
    for (op, arg) in anim::take_mail(d, s.id) {
        match op.as_str() {
            "image.view_all" => s.fit = true,
            "image.view_zoom_1" => {
                s.zoom = 1.0;
                s.pan = (0.0, 0.0);
            }
            "image.view_zoom_in" => s.zoom *= 1.25,
            "image.view_zoom_out" => s.zoom /= 1.25,
            "image.save_as" => save_render(s, d),
            _ if s.kind == EditorKind::UvEditor => uv_mail(s, d, &op, &arg),
            _ => {}
        }
    }
    if s.kind == EditorKind::UvEditor {
        uv_body(s, d, ui, r);
    } else {
        image_body(s, d, ui, r);
    }
}

/// Pointer navigation shared by both editors. `size` is the content size in
/// content units; returns nothing, changes zoom and pan.
fn navigate(s: &mut State, ui: &mut Ui, r: Rect) {
    let Some(input) = ui.input else { return };
    let clip = ui.p.clip();
    let inside = |x: i32, y: i32| r.contains(x, y) && clip.contains(x, y);
    match input {
        Input::Wheel { x, y, steps, mods } if inside(x, y) => {
            if mods.ctrl {
                s.pan.0 += steps as f32 * 40.0;
            } else if mods.shift {
                s.pan.1 += steps as f32 * 40.0;
            } else {
                let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0 + s.pan.0, r.y as f32 + r.h as f32 / 2.0 + s.pan.1);
                let f = 1.25f32.powi(steps);
                let nz = (s.zoom * f).clamp(0.02, 64.0);
                let f = nz / s.zoom;
                s.zoom = nz;
                // Keep the point under the pointer fixed.
                s.pan.0 += (x as f32 - cx) * (1.0 - f);
                s.pan.1 += (y as f32 - cy) * (1.0 - f);
            }
            ui.input = None;
        }
        Input::Down { x, y, button: Button::Middle, .. } if inside(x, y) => {
            s.panning = Some((x, y));
            ui.input = None;
        }
        Input::Move { x, y } if s.panning.is_some() => {
            let (lx, ly) = s.panning.unwrap_or((x, y));
            s.pan.0 += (x - lx) as f32;
            s.pan.1 += (y - ly) as f32;
            s.panning = Some((x, y));
            ui.input = None;
        }
        Input::Up { button: Button::Middle, .. } if s.panning.is_some() => {
            s.panning = None;
            ui.input = None;
        }
        _ => {}
    }
}

fn image_body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    let Some((w, h)) = render_rgba(s, d) else {
        ui.p.label(r, "Render Result appears here after F12", t::TEXT_DIM, Align::Center);
        return;
    };
    if s.fit {
        s.fit = false;
        s.zoom = ((r.w as f32 - 40.0) / w as f32).min((r.h as f32 - 40.0) / h as f32).clamp(0.02, 64.0);
        s.pan = (0.0, 0.0);
    }
    navigate(s, ui, r);
    let Some((_, _, _, rgba)) = &s.cache else { return };
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0 + s.pan.0, r.y as f32 + r.h as f32 / 2.0 + s.pan.1);
    let dw = w as f32 * s.zoom;
    let dh = h as f32 * s.zoom;
    let dx = cx - dw / 2.0;
    let dy = cy - dh / 2.0;
    let dest = Rect::new(dx.floor() as i32, dy.floor() as i32, dw.ceil() as i32 + 1, dh.ceil() as i32 + 1);
    let p = &mut *ui.p;
    let c = dest.intersect(&p.clip()).intersect(&r);
    let inv = 1.0 / s.zoom;
    let pw = p.width;
    for y in c.y..c.bottom() {
        let sy = ((y as f32 + 0.5 - dy) * inv) as i64;
        if sy < 0 || sy >= h as i64 {
            continue;
        }
        let row = y as usize * pw;
        for x in c.x..c.right() {
            let sx = ((x as f32 + 0.5 - dx) * inv) as i64;
            if sx < 0 || sx >= w as i64 {
                continue;
            }
            let i = (sy as usize * w as usize + sx as usize) * 4;
            let a = rgba[i + 3] as u32;
            let px = (rgba[i] as u32) << 16 | (rgba[i + 1] as u32) << 8 | rgba[i + 2] as u32;
            p.pixels[row + x as usize] = if a >= 255 {
                px
            } else {
                // Blender's transparency checkerboard.
                let check = if ((x - dest.x) / 8 + (y - dest.y) / 8) % 2 == 0 { 0x4d4d4d } else { 0x333333 };
                crate::draw_mix(px, check, a)
            };
        }
    }
    // Sample info line like Blender's footer: size and zoom.
    let info = format!("{w} x {h}   {:.0}%", s.zoom * 100.0);
    ui.p.label(Rect::new(r.x + 10, r.bottom() - 22, 300, 20), &info, t::TEXT_DIM, Align::Left);
}

fn save_render(s: &mut State, d: &mut Doc) {
    if render_rgba(s, d).is_none() {
        anim::report(d, ReportKind::Warning, "No image to save: render with F12 first");
        return;
    }
    let Some((_, w, h, rgba)) = &s.cache else { return };
    let stem = d.path.as_deref().map(|p| p.rsplit_once('.').map_or(p, |x| x.0).to_string()).unwrap_or_else(|| "untitled".into());
    let name = format!("{stem}.png");
    match std::fs::write(&name, encode_png(*w, *h, rgba)) {
        Ok(()) => anim::report(d, ReportKind::Info, &format!("Saved image \"{name}\"")),
        Err(e) => anim::report(d, ReportKind::Error, &format!("Cannot write {name}: {e}")),
    }
}

// ---------------------------------------------------------------- UV editor

fn uv_mesh(d: &Doc) -> Option<&sex_3d::Mesh> {
    let a = d.active()?;
    match d.scene.objects[a].data {
        ObjectData::Mesh(mi) => d.scene.meshes.get(mi),
        _ => None,
    }
}

/// The active mesh, mutably (for live transforms that are not undo steps yet).
fn uv_mesh_mut(d: &mut Doc) -> Option<&mut sex_3d::Mesh> {
    let a = d.active()?;
    match d.scene.objects[a].data {
        ObjectData::Mesh(mi) => d.scene.meshes.get_mut(mi),
        _ => None,
    }
}

fn fill_poly(p: &mut Painter, pts: &[(f32, f32)], color: u32, alpha: u32) {
    if pts.len() < 3 {
        return;
    }
    let ymin = pts.iter().map(|q| q.1).fold(f32::INFINITY, f32::min).floor() as i32;
    let ymax = pts.iter().map(|q| q.1).fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
    let clip = p.clip();
    for y in ymin.max(clip.y)..ymax.min(clip.bottom()) {
        let yc = y as f32 + 0.5;
        let mut xs: Vec<f32> = Vec::new();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            if (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc) {
                xs.push(a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks(2) {
            if let [x0, x1] = pair {
                let a = x0.round() as i32;
                p.fill_alpha(Rect::new(a, y, x1.round() as i32 - a, 1), color, alpha);
            }
        }
    }
}

fn inside_poly(pts: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let n = pts.len();
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + n - 1) % n]);
        if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    inside
}

/// Faces the UV editor shows and edits: in Edit Mode the selected faces
/// (Blender without UV sync selection), otherwise all; only faces with UVs.
fn shown_faces(d: &Doc, m: &Mesh) -> Vec<u32> {
    let base: Vec<u32> = if d.mode == Mode::Edit { crate::uvedit::op_faces(d) } else { (0..m.faces.len() as u32).collect() };
    base.into_iter().filter(|&f| m.faces.get(f as usize).is_some_and(|f| f.uvs.len() == f.verts.len() && f.verts.len() >= 3)).collect()
}

fn face_selected(sel: &BTreeSet<Corner>, m: &Mesh, f: u32) -> bool {
    let n = m.faces[f as usize].verts.len() as u32;
    n > 0 && (0..n).all(|c| sel.contains(&(f, c)))
}

/// UV <-> screen for the UV editor body: the 0..1 square centred, v up.
#[derive(Clone, Copy, Debug)]
struct UvView {
    cx: f32,
    cy: f32,
    z: f32,
}

impl UvView {
    fn of(s: &State, r: Rect) -> UvView {
        UvView { cx: r.x as f32 + r.w as f32 / 2.0 + s.pan.0, cy: r.y as f32 + r.h as f32 / 2.0 + s.pan.1, z: s.zoom }
    }
    fn to(&self, u: Vec2) -> (f32, f32) {
        (self.cx + (u.x - 0.5) * self.z, self.cy - (u.y - 0.5) * self.z)
    }
}

fn corner_uv(m: &Mesh, c: Corner) -> Vec2 {
    m.faces[c.0 as usize].uvs[c.1 as usize]
}

/// The UV corner nearest to (x, y) within 12 pixels.
fn pick_corner(m: &Mesh, shown: &[u32], v: UvView, x: f32, y: f32) -> Option<Corner> {
    let mut best = (12.0f32, None);
    for &f in shown {
        for (k, &u) in m.faces[f as usize].uvs.iter().enumerate() {
            let (sx, sy) = v.to(u);
            let dd = (sx - x).hypot(sy - y);
            if dd < best.0 {
                best = (dd, Some((f, k as u32)));
            }
        }
    }
    best.1
}

/// The UV edge (two corners of one face) nearest to (x, y) within 10 pixels.
fn pick_edge(m: &Mesh, shown: &[u32], v: UvView, x: f32, y: f32) -> Option<(Corner, Corner)> {
    let mut best = (10.0f32, None);
    for &f in shown {
        let uvs = &m.faces[f as usize].uvs;
        for k in 0..uvs.len() {
            let k2 = (k + 1) % uvs.len();
            let dd = crate::editors::view3d::proj::seg_dist(x, y, v.to(uvs[k]), v.to(uvs[k2]));
            if dd < best.0 {
                best = (dd, Some(((f, k as u32), (f, k2 as u32))));
            }
        }
    }
    best.1
}

/// The topmost face under (x, y), or the one whose centre is nearest within 16 pixels.
fn pick_face(m: &Mesh, shown: &[u32], v: UvView, x: f32, y: f32) -> Option<u32> {
    let polys: Vec<(u32, Vec<(f32, f32)>)> = shown.iter().map(|&f| (f, m.faces[f as usize].uvs.iter().map(|&u| v.to(u)).collect())).collect();
    if let Some((f, _)) = polys.iter().rev().find(|(_, p)| inside_poly(p, x, y)) {
        return Some(*f);
    }
    let mut best = (16.0f32, None);
    for (f, p) in &polys {
        let c = p.iter().fold((0.0, 0.0), |a, q| (a.0 + q.0 / p.len() as f32, a.1 + q.1 / p.len() as f32));
        let dd = (c.0 - x).hypot(c.1 - y);
        if dd < best.0 {
            best = (dd, Some(*f));
        }
    }
    best.1
}

fn face_corners(m: &Mesh, f: u32) -> impl Iterator<Item = Corner> {
    (0..m.faces[f as usize].verts.len() as u32).map(move |c| (f, c))
}

/// Click select: in vertex and edge mode with shared-location stickiness.
fn select_click(s: &mut State, m: &Mesh, shown: &[u32], v: UvView, x: f32, y: f32, mods: Mods) {
    let hits: Vec<Corner> = match s.select_mode {
        0 => pick_corner(m, shown, v, x, y).map(|c| uvops::sticky(m, shown, c)).unwrap_or_default(),
        1 => pick_edge(m, shown, v, x, y).map(|(a, b)| uvops::sticky(m, shown, a).into_iter().chain(uvops::sticky(m, shown, b)).collect()).unwrap_or_default(),
        _ => pick_face(m, shown, v, x, y).map(|f| face_corners(m, f).collect()).unwrap_or_default(),
    };
    if hits.is_empty() {
        if !mods.shift {
            s.uv_sel.clear();
        }
        return;
    }
    if mods.shift {
        if hits.iter().all(|c| s.uv_sel.contains(c)) {
            for c in &hits {
                s.uv_sel.remove(c);
            }
        } else {
            s.uv_sel.extend(hits);
        }
    } else {
        s.uv_sel.clear();
        s.uv_sel.extend(hits);
    }
}

/// Box select between two screen points. Shift adds, Ctrl removes.
fn select_box(s: &mut State, m: &Mesh, shown: &[u32], v: UvView, a: (i32, i32), b: (i32, i32), mods: Mods) {
    let (x0, x1) = (a.0.min(b.0) as f32, a.0.max(b.0) as f32);
    let (y0, y1) = (a.1.min(b.1) as f32, a.1.max(b.1) as f32);
    let inside = |p: (f32, f32)| p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1;
    let mut hits = Vec::new();
    for &f in shown {
        let uvs = &m.faces[f as usize].uvs;
        if s.select_mode == 2 {
            let c = uvs.iter().fold(Vec2::ZERO, |a, &u| a + u) * (1.0 / uvs.len() as f32);
            if inside(v.to(c)) {
                hits.extend(face_corners(m, f));
            }
        } else {
            for (k, &u) in uvs.iter().enumerate() {
                if inside(v.to(u)) {
                    hits.push((f, k as u32));
                }
            }
        }
    }
    if mods.ctrl {
        for c in &hits {
            s.uv_sel.remove(c);
        }
    } else {
        if !mods.shift {
            s.uv_sel.clear();
        }
        s.uv_sel.extend(hits);
    }
}

/// Select (or deselect) the UV islands holding any of `seeds`.
fn select_linked(s: &mut State, m: &Mesh, shown: &[u32], seeds: &[u32], deselect: bool) {
    for isl in uvops::uv_islands(m, shown) {
        if isl.iter().any(|f| seeds.contains(f)) {
            for &f in &isl {
                for c in face_corners(m, f) {
                    if deselect {
                        s.uv_sel.remove(&c);
                    } else {
                        s.uv_sel.insert(c);
                    }
                }
            }
        }
    }
}

/// Selected corners on faces the editor shows.
fn live_selection(s: &State, m: &Mesh, shown: &[u32]) -> Vec<Corner> {
    let set: std::collections::HashSet<u32> = shown.iter().copied().collect();
    s.uv_sel.iter().copied().filter(|c| set.contains(&c.0) && (c.1 as usize) < m.faces[c.0 as usize].uvs.len()).collect()
}

/// Operators the UV editor's menus mail to it.
fn uv_mail(s: &mut State, d: &mut Doc, op: &str, arg: &str) {
    let Some(m) = uv_mesh(d) else { return };
    let shown = shown_faces(d, m);
    match op {
        "uv.select_all" => select_all(s, m, &shown, arg),
        "uv.select_linked" => {
            let seeds: Vec<u32> = live_selection(s, m, &shown).iter().map(|c| c.0).collect();
            select_linked(s, m, &shown, &seeds, false);
        }
        "uv.select_pinned" => {
            let pins: Vec<Corner> = shown.iter().flat_map(|&f| m.uv_pins.range((f, 0)..(f + 1, 0)).copied()).collect();
            s.uv_sel.extend(pins);
        }
        "uv.pin" => pin(s, d, arg == "CLEAR"),
        "uv.stitch" => stitch(s, d),
        "uv.weld" => weld(s, d),
        "uv.remove_doubles" => remove_doubles(s, d),
        _ => {}
    }
}

fn select_all(s: &mut State, m: &Mesh, shown: &[u32], action: &str) {
    let all: Vec<Corner> = shown.iter().flat_map(|&f| face_corners(m, f)).collect();
    let action = if action == "TOGGLE" {
        if all.iter().any(|c| s.uv_sel.contains(c)) {
            "DESELECT"
        } else {
            "SELECT"
        }
    } else {
        action
    };
    match action {
        "SELECT" => s.uv_sel.extend(all),
        "INVERT" => {
            for c in all {
                if !s.uv_sel.remove(&c) {
                    s.uv_sel.insert(c);
                }
            }
        }
        _ => s.uv_sel.clear(),
    }
}

fn pin(s: &mut State, d: &mut Doc, clear: bool) {
    let Some(m) = uv_mesh(d) else { return };
    let sel = live_selection(s, m, &shown_faces(d, m));
    if sel.is_empty() {
        return;
    }
    crate::uvedit::edit_uvs(d, if clear { "Unpin" } else { "Pin" }, |m| {
        for c in sel {
            if clear {
                m.uv_pins.remove(&c);
            } else {
                m.uv_pins.insert(c);
            }
        }
    });
}

fn stitch(s: &mut State, d: &mut Doc) {
    let Some(m) = uv_mesh(d) else { return };
    let shown = shown_faces(d, m);
    let sel = live_selection(s, m, &shown);
    if sel.is_empty() {
        d.report = "Nothing selected to stitch".into();
        return;
    }
    let mut n = 0;
    crate::uvedit::edit_uvs(d, "Stitch", |m| n = uvops::stitch(m, &shown, &sel));
    d.report = format!("Stitched {n} UV(s)");
}

fn weld(s: &mut State, d: &mut Doc) {
    let Some(m) = uv_mesh(d) else { return };
    let sel = live_selection(s, m, &shown_faces(d, m));
    if sel.is_empty() {
        return;
    }
    crate::uvedit::edit_uvs(d, "Weld", |m| uvops::weld(m, &sel));
}

fn remove_doubles(s: &mut State, d: &mut Doc) {
    let Some(m) = uv_mesh(d) else { return };
    let sel = live_selection(s, m, &shown_faces(d, m));
    if sel.is_empty() {
        return;
    }
    let mut n = 0;
    crate::uvedit::edit_uvs(d, "Merge UVs by Distance", |m| n = uvops::merge_by_distance(m, &sel, 0.02));
    d.report = format!("Merged {n} UV(s)");
}

// ---- G / R / S

fn start_transform(s: &mut State, d: &Doc, tool: UvTool) -> bool {
    let Some(m) = uv_mesh(d) else { return false };
    let sel = live_selection(s, m, &shown_faces(d, m));
    if sel.is_empty() {
        return false;
    }
    let orig: Vec<(Corner, Vec2)> = sel.iter().map(|&c| (c, corner_uv(m, c))).collect();
    let (mut mn, mut mx) = (Vec2::new(f32::MAX, f32::MAX), Vec2::new(f32::MIN, f32::MIN));
    for (_, u) in &orig {
        mn = Vec2::new(mn.x.min(u.x), mn.y.min(u.y));
        mx = Vec2::new(mx.x.max(u.x), mx.y.max(u.y));
    }
    // Pivot: the selection's bounding box centre (the UV editor's default).
    s.modal = Some(UvTransform { tool, start: s.mouse, pivot: (mn + mx) * 0.5, orig, axis: None });
    true
}

/// The transformed position of each selected corner for the pointer at `mouse`.
fn transformed(t: &UvTransform, v: UvView, mouse: (i32, i32)) -> Vec<(Corner, Vec2)> {
    let (sx, sy) = (t.start.0 as f32, t.start.1 as f32);
    let (mx, my) = (mouse.0 as f32, mouse.1 as f32);
    let (px, py) = v.to(t.pivot);
    let keep = |d: Vec2| match t.axis {
        Some(0) => Vec2::new(d.x, 0.0),
        Some(_) => Vec2::new(0.0, d.y),
        None => d,
    };
    match t.tool {
        UvTool::Move => {
            let dlt = keep(Vec2::new((mx - sx) / v.z, -(my - sy) / v.z));
            t.orig.iter().map(|&(c, u)| (c, u + dlt)).collect()
        }
        UvTool::Rotate => {
            let a0 = (-(sy - py)).atan2(sx - px);
            let a1 = (-(my - py)).atan2(mx - px);
            let (sn, cs) = (a1 - a0).sin_cos();
            t.orig
                .iter()
                .map(|&(c, u)| {
                    let q = u - t.pivot;
                    (c, t.pivot + Vec2::new(q.x * cs - q.y * sn, q.x * sn + q.y * cs))
                })
                .collect()
        }
        UvTool::Scale => {
            let d0 = (sx - px).hypot(sy - py);
            let f = if d0 > 1.0 { (mx - px).hypot(my - py) / d0 } else { 1.0 };
            let k = match t.axis {
                Some(0) => (f, 1.0),
                Some(_) => (1.0, f),
                None => (f, f),
            };
            t.orig.iter().map(|&(c, u)| (c, t.pivot + Vec2::new((u.x - t.pivot.x) * k.0, (u.y - t.pivot.y) * k.1))).collect()
        }
    }
}

fn write_uvs(m: &mut Mesh, list: &[(Corner, Vec2)]) {
    for &((f, c), u) in list {
        if let Some(x) = m.faces.get_mut(f as usize).and_then(|f| f.uvs.get_mut(c as usize)) {
            *x = u;
        }
    }
}

fn update_transform(s: &mut State, d: &mut Doc) {
    let Some(t) = &s.modal else { return };
    let list = transformed(t, UvView::of(s, s.last_rect), s.mouse);
    if let Some(m) = uv_mesh_mut(d) {
        write_uvs(m, &list);
    }
}

/// End G/R/S: keep the result as one undo step, or put the UVs back.
fn finish_transform(s: &mut State, d: &mut Doc, confirm: bool) {
    let Some(t) = s.modal.take() else { return };
    let fin = transformed(&t, UvView::of(s, s.last_rect), s.mouse);
    if let Some(m) = uv_mesh_mut(d) {
        write_uvs(m, &t.orig);
    }
    if confirm {
        let label = match t.tool {
            UvTool::Move => "Move",
            UvTool::Rotate => "Rotate",
            UvTool::Scale => "Resize",
        };
        crate::uvedit::edit_uvs(d, label, |m| write_uvs(m, &fin));
    }
    d.touch();
}

pub fn is_modal(s: &State) -> bool {
    s.modal.is_some() || s.press.is_some()
}

fn uv_events(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    let Some(input) = ui.input else { return };
    let v = UvView::of(s, r);
    if s.modal.is_some() {
        match input {
            Input::Move { .. } => update_transform(s, d),
            Input::Down { button: Button::Left, .. } => finish_transform(s, d, true),
            Input::Down { button: Button::Right, .. } => finish_transform(s, d, false),
            _ => return,
        }
        ui.input = None;
        return;
    }
    let inside = |x: i32, y: i32| r.contains(x, y) && ui.p.clip().contains(x, y);
    match input {
        Input::Down { x, y, button: Button::Left, mods } if inside(x, y) => {
            s.press = Some((x, y, mods, None));
            ui.input = None;
        }
        Input::Move { x, y } if s.press.is_some() => {
            if let Some(p) = &mut s.press {
                if p.3.is_some() || (x - p.0).abs() + (y - p.1).abs() > 4 {
                    p.3 = Some((x, y));
                }
            }
            ui.input = None;
        }
        Input::Up { button: Button::Left, .. } if s.press.is_some() => {
            let (px, py, mods, end) = s.press.take().expect("press");
            if let Some(m) = uv_mesh(d) {
                let shown = shown_faces(d, m);
                match end {
                    Some(e) => select_box(s, m, &shown, v, (px, py), e, mods),
                    None => select_click(s, m, &shown, v, px as f32, py as f32, mods),
                }
            }
            ui.input = None;
        }
        _ => {}
    }
}

fn uv_body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if s.fit {
        s.fit = false;
        s.zoom = (r.w.min(r.h) as f32 - 60.0).max(50.0);
        s.pan = (0.0, 0.0);
    }
    s.mouse = ui.mouse();
    if let Some(m) = uv_mesh(d) {
        let faces = &m.faces;
        s.uv_sel.retain(|&(f, c)| faces.get(f as usize).is_some_and(|f| (c as usize) < f.verts.len()));
    }
    if s.modal.is_none() {
        navigate(s, ui, r);
    }
    uv_events(s, d, ui, r);
    let v = UvView::of(s, r);
    let z = v.z;
    let to = |u: Vec2| v.to(u);
    let p = &mut *ui.p;
    p.fill(r, 0x303030);
    // Grid: fine lines every 1/8 across the view, the 0..1 tile darker.
    let (x0, y0) = to(Vec2::new(0.0, 1.0));
    let (x1, y1) = to(Vec2::new(1.0, 0.0));
    let tile = Rect::new(x0.round() as i32, y0.round() as i32, (x1 - x0).round() as i32, (y1 - y0).round() as i32);
    p.fill(tile, 0x2a2a2a);
    let step = z / 8.0;
    if step > 4.0 {
        let mut k = ((r.x as f32 - x0) / step).floor();
        while x0 + k * step < r.right() as f32 {
            let x = (x0 + k * step).round() as i32;
            p.fill(Rect::new(x, r.y, 1, r.h), if (k as i32).rem_euclid(8) == 0 { 0x4a4a4a } else { 0x383838 });
            k += 1.0;
        }
        let mut k = ((r.y as f32 - y0) / step).floor();
        while y0 + k * step < r.bottom() as f32 {
            let y = (y0 + k * step).round() as i32;
            p.fill(Rect::new(r.x, y, r.w, 1), if (k as i32).rem_euclid(8) == 0 { 0x4a4a4a } else { 0x383838 });
            k += 1.0;
        }
    }
    p.frame(Rect::new(tile.x, tile.y, tile.w + 1, tile.h + 1), 0x5a5a5a);
    let Some(m) = uv_mesh(d) else {
        p.label(Rect::new(r.x, r.bottom() - 40, r.w, 20), "The active object has no UVs", t::TEXT_DIM, Align::Center);
        return;
    };
    let shown = shown_faces(d, m);
    if shown.is_empty() {
        let msg = if d.mode == Mode::Edit && m.has_uvs() { "Select faces in the 3D Viewport to edit their UVs" } else { "The active object has no UVs" };
        p.label(Rect::new(r.x, r.bottom() - 40, r.w, 20), msg, t::TEXT_DIM, Align::Center);
        return;
    }
    let sel = &s.uv_sel;
    let polys: Vec<(u32, Vec<(f32, f32)>)> = shown.iter().map(|&f| (f, m.faces[f as usize].uvs.iter().map(|&u| to(u)).collect())).collect();
    for (f, pts) in &polys {
        let fs = face_selected(sel, m, *f);
        fill_poly(p, pts, if fs { 0xff8500 } else { 0xffffff }, if fs { 70 } else { 22 });
    }
    for pass in 0..2 {
        for (f, pts) in &polys {
            for k in 0..pts.len() {
                let k2 = (k + 1) % pts.len();
                let es = sel.contains(&(*f, k as u32)) && sel.contains(&(*f, k2 as u32));
                if (pass == 1) != es {
                    continue;
                }
                let (a, b) = (pts[k], pts[k2]);
                p.line(a.0, a.1, b.0, b.1, if es { 0xff8500 } else { 0x0a0a0a });
            }
        }
    }
    match s.select_mode {
        0 => {
            for (f, pts) in &polys {
                for (k, q) in pts.iter().enumerate() {
                    let on = sel.contains(&(*f, k as u32));
                    p.fill(Rect::new(q.0.round() as i32 - 2, q.1.round() as i32 - 2, 4, 4), if on { 0xff8500 } else { 0x000000 });
                }
            }
        }
        2 => {
            for (f, pts) in &polys {
                let c = pts.iter().fold((0.0, 0.0), |a, q| (a.0 + q.0 / pts.len() as f32, a.1 + q.1 / pts.len() as f32));
                let on = face_selected(sel, m, *f);
                p.fill(Rect::new(c.0.round() as i32 - 2, c.1.round() as i32 - 2, 4, 4), if on { 0xff8500 } else { 0x000000 });
            }
        }
        _ => {}
    }
    // Pinned UVs: Blender's red dots.
    for (f, pts) in &polys {
        for (k, q) in pts.iter().enumerate() {
            if m.uv_pins.contains(&(*f, k as u32)) {
                p.fill(Rect::new(q.0.round() as i32 - 3, q.1.round() as i32 - 3, 6, 6), 0xff0000);
            }
        }
    }
    if let Some((px, py, _, Some((ex, ey)))) = s.press {
        let b = Rect::new(px.min(ex), py.min(ey), (ex - px).abs(), (ey - py).abs());
        p.fill_alpha(b, 0xffffff, 18);
        p.frame(b, 0xffffff);
    }
    let info = match &s.modal {
        Some(tr) => {
            let list = transformed(tr, v, s.mouse);
            let (c, u) = list.first().copied().unwrap_or(((0, 0), Vec2::ZERO));
            let o = tr.orig.iter().find(|o| o.0 == c).map_or(Vec2::ZERO, |o| o.1);
            let ax = match tr.axis {
                Some(0) => " along U",
                Some(_) => " along V",
                None => "",
            };
            match tr.tool {
                UvTool::Move => format!("D: {:.4}  {:.4}{ax}", u.x - o.x, u.y - o.y),
                UvTool::Rotate => "Rotate (click or Enter to confirm, Esc to cancel)".to_string(),
                UvTool::Scale => format!("Scale{ax}"),
            }
        }
        None => format!("{} faces, {} selected", shown.len(), shown.iter().filter(|&&f| face_selected(sel, m, f)).count()),
    };
    ui.p.label(Rect::new(r.x + 10, r.bottom() - 22, 400, 20), &info, t::TEXT_DIM, Align::Left);
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    let uv = s.kind == EditorKind::UvEditor;
    if uv && s.modal.is_some() {
        match key {
            Key::Esc => finish_transform(s, d, false),
            Key::Enter | Key::NumEnter | Key::Space => finish_transform(s, d, true),
            Key::Char('x') | Key::Char('y') => {
                let a = if key == Key::Char('x') { 0 } else { 1 };
                if let Some(t) = &mut s.modal {
                    t.axis = if t.axis == Some(a) { None } else { Some(a) };
                }
                update_transform(s, d);
            }
            _ => {}
        }
        return true;
    }
    if uv {
        let handled = uv_hotkey(s, d, key, mods);
        if handled {
            return true;
        }
    }
    match (key, mods) {
        (Key::Home, Mods::NONE) => s.fit = true,
        (Key::Num(1), Mods::NONE) if s.kind == EditorKind::ImageEditor => {
            s.zoom = 1.0;
            s.pan = (0.0, 0.0);
        }
        (Key::NumPlus, Mods::NONE) => s.zoom *= 1.25,
        (Key::NumMinus, Mods::NONE) => s.zoom /= 1.25,
        (Key::Char('s'), Mods::ALT) | (Key::Char('s'), Mods::CTRL_SHIFT) if s.kind == EditorKind::ImageEditor => save_render(s, d),
        _ => return false,
    }
    true
}

/// The UV editor's keymap (Blender 2.8+).
fn uv_hotkey(s: &mut State, d: &mut Doc, key: Key, mods: Mods) -> bool {
    let (mx, my) = s.mouse;
    let v = UvView::of(s, s.last_rect);
    match (key, mods) {
        (Key::Char('a'), Mods::NONE) | (Key::Char('a'), Mods::ALT) | (Key::Char('i'), Mods::CTRL) => {
            let action = match mods {
                Mods::NONE => "SELECT",
                Mods::ALT => "DESELECT",
                _ => "INVERT",
            };
            if let Some(m) = uv_mesh(d) {
                let shown = shown_faces(d, m);
                select_all(s, m, &shown, action);
            }
        }
        (Key::Char('1'), Mods::NONE) => s.select_mode = 0,
        (Key::Char('2'), Mods::NONE) => s.select_mode = 1,
        (Key::Char('3'), Mods::NONE) => s.select_mode = 2,
        (Key::Char('l'), Mods::NONE) | (Key::Char('l'), Mods::SHIFT) => {
            if let Some(m) = uv_mesh(d) {
                let shown = shown_faces(d, m);
                let (x, y) = (mx as f32, my as f32);
                let f = pick_face(m, &shown, v, x, y).or_else(|| pick_corner(m, &shown, v, x, y).map(|c| c.0));
                if let Some(f) = f {
                    select_linked(s, m, &shown, &[f], mods.shift);
                }
            }
        }
        (Key::Char('l'), Mods::CTRL) => uv_mail(s, d, "uv.select_linked", ""),
        (Key::Char('g'), Mods::NONE) => {
            start_transform(s, d, UvTool::Move);
        }
        (Key::Char('r'), Mods::NONE) => {
            start_transform(s, d, UvTool::Rotate);
        }
        (Key::Char('s'), Mods::NONE) => {
            start_transform(s, d, UvTool::Scale);
        }
        (Key::Char('p'), Mods::NONE) => pin(s, d, false),
        (Key::Char('p'), Mods::ALT) => pin(s, d, true),
        (Key::Char('p'), Mods::SHIFT) => uv_mail(s, d, "uv.select_pinned", ""),
        (Key::Char('p'), Mods::CTRL) => d.queue.push(Op::new("uv.pack_islands")),
        (Key::Char('a'), Mods::CTRL) => d.queue.push(Op::new("uv.average_islands_scale")),
        (Key::Char('v'), Mods::NONE) => stitch(s, d),
        (Key::Char('m'), Mods::NONE) => {
            let items = vec![MenuItem::op("At Center", "", anim::mail("uv.weld", s.id, "")), MenuItem::op("By Distance", "", anim::mail("uv.remove_doubles", s.id, ""))];
            d.menu = Some(crate::ui::Popup::new("Merge", items, mx - 20, my - 10));
        }
        (Key::Char('u'), Mods::NONE) => {
            let items = vec![
                MenuItem::op("Unwrap", "", Op::new("uv.unwrap")),
                MenuItem::op("Smart UV Project", "", Op::new("uv.smart_project")),
                MenuItem::op("Lightmap Pack", "", Op::new("uv.lightmap_pack")),
                MenuItem::op("Follow Active Quads", "", Op::new("uv.follow_active_quads")),
                MenuItem::Separator,
                MenuItem::op("Cube Projection", "", Op::new("uv.cube_project")),
                MenuItem::op("Cylinder Projection", "", Op::new("uv.cylinder_project")),
                MenuItem::op("Sphere Projection", "", Op::new("uv.sphere_project")),
                MenuItem::Separator,
                MenuItem::op("Reset", "", Op::new("uv.reset")),
            ];
            d.menu = Some(crate::ui::Popup::new("Unwrap", items, mx - 20, my - 10));
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::App;

    #[test]
    fn png_round_trips_through_the_png_crate() {
        let (w, h) = (300u32, 250u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, (i / 300) as u8, (i * 7 % 251) as u8, (255 - i % 200) as u8]).collect();
        let bytes = encode_png(w, h, &rgba);
        let dec = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (w, h));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&buf[..info.buffer_size()], &rgba[..]);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn image_editor_shows_the_render() {
        let mut app = App::new(1280, 720);
        app.frame(Input::Tick);
        let mut img = sex_3d::scene::Image::new("Render Result", 160, 90);
        for (i, px) in img.pixels.iter_mut().enumerate() {
            *px = [(i % 160) as f32 / 160.0, 0.9, 0.2, 1.0];
        }
        app.doc.render = Some(img);
        app.doc.want_workspace = Some("Rendering".into());
        app.frame(Input::Tick);
        app.frame(Input::Tick);
        let a = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::ImageEditor).unwrap().rect;
        let body = Rect::new(a.x, a.y + t::HEADER_H + 1, a.w, a.h - t::HEADER_H - 1);
        let mut greenish = 0;
        for y in body.y..body.bottom() {
            for x in body.x..body.right() {
                let p = app.painter.pixels[y as usize * app.painter.width + x as usize];
                let (r, g, b) = (p >> 16 & 0xff, p >> 8 & 0xff, p & 0xff);
                if g > r + 8 && g > b + 20 {
                    greenish += 1;
                }
            }
        }
        // The 160x90 render is fitted to the area: most of it is image.
        let (mx, my) = body.center();
        assert!(greenish > body.w * body.h / 3, "{greenish} {:06x}", app.painter.pixels[my as usize * app.painter.width + mx as usize]);
        // Numpad 1 shows it 1:1.
        app.frame(Input::Move { x: body.x + 50, y: body.y + 50 });
        app.frame(Input::key(Key::Num(1)));
        if let crate::editors::Editor::Image(s) = &app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::ImageEditor).unwrap().editor {
            assert_eq!(s.zoom, 1.0);
        }
    }

    fn uv_app() -> (App, Rect) {
        let mut app = App::new(1280, 720);
        app.doc.want_workspace = Some("UV Editing".into());
        app.frame(Input::Tick);
        app.frame(Input::Tick);
        let a = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::UvEditor).unwrap().rect;
        let body = Rect::new(a.x, a.y + t::HEADER_H + 1, a.w, a.h - t::HEADER_H - 1);
        (app, body)
    }

    fn uv_state(app: &App) -> &State {
        match &app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::UvEditor).unwrap().editor {
            crate::editors::Editor::Image(s) => s,
            _ => unreachable!(),
        }
    }

    fn click(app: &mut App, x: i32, y: i32, mods: Mods) {
        app.frame(Input::Move { x, y });
        app.frame(Input::Down { x, y, button: Button::Left, mods });
        app.frame(Input::Up { x, y, button: Button::Left });
    }

    fn uvs(app: &App) -> Vec<Vec<Vec2>> {
        uv_mesh(&app.doc).unwrap().faces.iter().map(|f| f.uvs.clone()).collect()
    }

    #[test]
    fn uv_editor_draws_and_selects_cube_uvs() {
        let (mut app, body) = uv_app();
        let (cx, cy) = body.center();
        app.frame(Input::Move { x: cx, y: cy });
        app.frame(Input::key(Key::Char('3')));
        assert_eq!(uv_state(&app).select_mode, 2);
        click(&mut app, cx, cy, Mods::NONE);
        let m = uv_mesh(&app.doc).unwrap();
        assert_eq!(uv_state(&app).selected_faces(m).len(), 1);
        let orange = app.painter.pixels.iter().filter(|&&p| p == 0xff8500).count();
        assert!(orange > 20, "{orange}");
        // Clicking empty space deselects.
        click(&mut app, body.x + 5, body.y + 5, Mods::NONE);
        assert!(uv_state(&app).uv_sel.is_empty());
    }

    #[test]
    fn uv_box_select_linked_and_vertex_pick() {
        let (mut app, body) = uv_app();
        let (cx, cy) = body.center();
        // Vertex mode (the default): box around the whole 0..1 tile.
        let z = uv_state(&app).zoom as i32;
        let h = z / 2 + 10;
        app.frame(Input::Move { x: cx - h, y: cy - h });
        app.frame(Input::Down { x: cx - h, y: cy - h, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Move { x: cx + h, y: cy + h });
        app.frame(Input::Up { x: cx + h, y: cy + h, button: Button::Left });
        assert_eq!(uv_state(&app).uv_sel.len(), 24, "all corners of the cube");
        // Click a corner of the tile: every corner sharing that location and vertex.
        let (x0, y0) = (cx - z / 2, cy + z / 2); // UV (0, 0)
        click(&mut app, x0, y0, Mods::NONE);
        let n = uv_state(&app).uv_sel.len();
        assert!(n >= 1 && n < 24, "{n}");
        // L over the tile: the island under the pointer (each cube face is its own here).
        app.frame(Input::key_mods(Key::Char('a'), Mods::ALT));
        app.frame(Input::Move { x: cx, y: cy });
        app.frame(Input::key(Key::Char('l')));
        assert_eq!(uv_state(&app).uv_sel.len(), 4);
    }

    #[test]
    fn uv_grab_rotate_scale_pin_and_undo() {
        let (mut app, body) = uv_app();
        let (cx, cy) = body.center();
        app.frame(Input::Move { x: cx, y: cy });
        app.frame(Input::key(Key::Char('a')));
        let before = uvs(&app);
        let z = uv_state(&app).zoom;
        // G, move 40 px right, confirm with a click.
        app.frame(Input::key(Key::Char('g')));
        assert!(uv_state(&app).modal.is_some());
        app.frame(Input::Move { x: cx + 40, y: cy });
        app.frame(Input::Down { x: cx + 40, y: cy, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Up { x: cx + 40, y: cy, button: Button::Left });
        assert!(uv_state(&app).modal.is_none());
        let after = uvs(&app);
        let dx = after[0][0].x - before[0][0].x;
        assert!((dx - 40.0 / z).abs() < 1e-4, "{dx}");
        assert!((after[0][0].y - before[0][0].y).abs() < 1e-6);
        assert_eq!(app.doc.undo.undo_label(), Some("Move"));
        // S constrained to V, cancelled with Esc: nothing changes.
        app.frame(Input::key(Key::Char('s')));
        app.frame(Input::key(Key::Char('y')));
        app.frame(Input::Move { x: cx + 100, y: cy + 30 });
        app.frame(Input::key(Key::Esc));
        assert_eq!(uvs(&app), after);
        // R by a quarter turn about the selection centre.
        app.frame(Input::Move { x: cx + 100, y: cy });
        app.frame(Input::key(Key::Char('r')));
        app.frame(Input::Move { x: cx, y: cy - 100 });
        app.frame(Input::key(Key::Enter));
        let rot = uvs(&app);
        assert_ne!(rot, after);
        // P pins, Alt P unpins.
        app.frame(Input::key(Key::Char('p')));
        assert_eq!(uv_mesh(&app.doc).unwrap().uv_pins.len(), 24);
        app.frame(Input::key_mods(Key::Char('p'), Mods::ALT));
        assert!(uv_mesh(&app.doc).unwrap().uv_pins.is_empty());
        // Undo back to the start.
        for _ in 0..4 {
            app.doc.run(&Op::new("ed.undo"));
        }
        assert_eq!(uvs(&app), before);
    }

    #[test]
    fn uv_weld_and_stitch_keys() {
        let (mut app, body) = uv_app();
        let (cx, cy) = body.center();
        app.frame(Input::Move { x: cx, y: cy });
        app.frame(Input::key(Key::Char('a')));
        // M opens the Merge menu; At Center welds every selected UV.
        app.frame(Input::key(Key::Char('m')));
        assert!(app.popup.is_some(), "Merge menu");
        app.frame(Input::key(Key::Esc));
        assert!(app.popup.is_none());
        app.doc.info.mailbox.push(anim::mail("uv.weld", uv_state(&app).id, ""));
        app.frame(Input::Tick);
        let u = uvs(&app);
        assert!(u.iter().flatten().all(|x| (*x - u[0][0]).x.abs() < 1e-6 && (*x - u[0][0]).y.abs() < 1e-6));
        app.doc.run(&Op::new("ed.undo"));
        app.frame(Input::key(Key::Char('v')));
        assert!(app.doc.report.starts_with("Stitched"), "{}", app.doc.report);
    }

    #[test]
    fn edit_mode_seams_unwrap_and_uv_editor_shows_selection() {
        let (mut app, body) = uv_app();
        app.doc.run(&Op::with("object.mode_set", "EDIT"));
        app.doc.run(&Op::with("mesh.select_all", "SELECT"));
        app.doc.run(&Op::new("mesh.mark_seam"));
        app.doc.run(&Op::new("uv.unwrap"));
        assert!(app.doc.report.is_empty(), "{}", app.doc.report);
        let m = uv_mesh(&app.doc).unwrap();
        assert_eq!(uvops::uv_islands(m, &(0..6).collect::<Vec<_>>()).len(), 6);
        app.frame(Input::Tick);
        // The unwrapped islands fill much of the tile: lots of face tint drawn.
        let (cx, cy) = body.center();
        app.frame(Input::Move { x: cx, y: cy });
        app.frame(Input::key(Key::Char('a')));
        app.frame(Input::Tick);
        let orange = app.painter.pixels.iter().filter(|&&p| p == 0xff8500).count();
        assert!(orange > 100, "{orange}");
    }
}
