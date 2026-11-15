// SPDX-License-Identifier: GPL-3.0-only
//! Blender's Image Editor and UV Editor.
//!
//! The Image Editor shows the last F12 render ("Render Result") through the
//! scene's colour management, fitted to the area, over a checkerboard where it is
//! transparent; the wheel zooms at the pointer, middle-drag pans, Home fits and
//! Numpad 1 shows it 1:1. Image > Save As writes it as a PNG with the small
//! encoder below (stored deflate: no compression crate on SexOS).
//!
//! The UV Editor draws the active mesh's face-corner UVs as a wireframe over
//! the 0..1 grid, in Blender's UV editor colours; click selects a UV face.
use super::header_menus;
use crate::anim::{self, ReportKind};
use crate::doc::Doc;
use crate::draw::{Align, Painter, Rect};
use crate::editors::EditorKind;
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};
use sex_3d::render::to_display8;
use sex_3d::scene::ObjectData;

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
    /// UV Editor: selected faces of the active mesh.
    pub uv_selected: Vec<usize>,
    pub select_mode: usize,
    last_rect: Rect,
}

impl State {
    pub fn new(kind: EditorKind) -> State {
        State { kind, id: anim::editor_id(), zoom: 1.0, pan: (0.0, 0.0), fit: true, panning: None, cache: None, uv_selected: Vec::new(), select_mode: 2, last_rect: Rect::default() }
    }
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
    let img = d.render.as_ref()?;
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
    let uv = vec![MenuItem::op("Unwrap", "U", Op::new("uv.unwrap")), MenuItem::op("Smart UV Project", "", Op::new("uv.smart_project")), MenuItem::op("Reset", "", Op::new("uv.reset"))];
    let sel = vec![MenuItem::op("All", "A", anim::mail("uv.select_all", s.id, "SELECT")), MenuItem::op("None", "Alt A", anim::mail("uv.select_all", s.id, "DESELECT"))];
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
    let name = if s.kind == EditorKind::ImageEditor && d.render.is_some() { "Render Result" } else { "" };
    let br = Rect::new(x, y, 36, h);
    if ui.dropdown(br, "") {
        let mut items = Vec::new();
        if d.render.is_some() {
            items.push(MenuItem::op("Render Result", "", Op::new("image.browse")));
        } else {
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
            "uv.select_all" => {
                s.uv_selected = if arg == "SELECT" { (0..uv_mesh(d).map_or(0, |m| m.faces.len())).collect() } else { Vec::new() };
            }
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

fn uv_body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if s.fit {
        s.fit = false;
        s.zoom = (r.w.min(r.h) as f32 - 60.0).max(50.0);
        s.pan = (0.0, 0.0);
    }
    navigate(s, ui, r);
    // UV (u, v) to screen: the 0..1 square centred, v up.
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0 + s.pan.0, r.y as f32 + r.h as f32 / 2.0 + s.pan.1);
    let z = s.zoom;
    let to = move |u: f32, v: f32| (cx + (u - 0.5) * z, cy - (v - 0.5) * z);
    let faces: Vec<Vec<(f32, f32)>> = uv_mesh(d).map(|m| m.faces.iter().map(|f| f.uvs.iter().map(|uv| to(uv.x, uv.y)).collect()).collect()).unwrap_or_default();
    if let Some(Input::Down { x, y, button: Button::Left, mods }) = ui.input {
        if r.contains(x, y) && ui.p.clip().contains(x, y) {
            let hit = faces.iter().rposition(|f| f.len() >= 3 && inside_poly(f, x as f32, y as f32));
            if !mods.shift {
                s.uv_selected.clear();
            }
            if let Some(h) = hit {
                if let Some(p) = s.uv_selected.iter().position(|&f| f == h) {
                    s.uv_selected.remove(p);
                } else {
                    s.uv_selected.push(h);
                }
            }
            ui.input = None;
        }
    }
    let p = &mut *ui.p;
    p.fill(r, 0x303030);
    // Grid: fine lines every 1/8 across the view, the 0..1 tile darker.
    let (x0, y0) = to(0.0, 1.0);
    let (x1, y1) = to(1.0, 0.0);
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
    if faces.is_empty() {
        p.label(Rect::new(r.x, r.bottom() - 40, r.w, 20), "The active object has no UVs", t::TEXT_DIM, Align::Center);
        return;
    }
    for (i, f) in faces.iter().enumerate() {
        let sel = s.uv_selected.contains(&i);
        fill_poly(p, f, if sel { 0xff8500 } else { 0xffffff }, if sel { 70 } else { 22 });
    }
    for (i, f) in faces.iter().enumerate() {
        let sel = s.uv_selected.contains(&i);
        for k in 0..f.len() {
            let (a, b) = (f[k], f[(k + 1) % f.len()]);
            p.line(a.0, a.1, b.0, b.1, if sel { 0xff8500 } else { 0x0a0a0a });
        }
    }
    let n = faces.len();
    ui.p.label(Rect::new(r.x + 10, r.bottom() - 22, 300, 20), &format!("{n} faces, {} selected", s.uv_selected.len()), t::TEXT_DIM, Align::Left);
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    match (key, mods) {
        (Key::Home, Mods::NONE) => s.fit = true,
        (Key::Num(1), Mods::NONE) if s.kind == EditorKind::ImageEditor => {
            s.zoom = 1.0;
            s.pan = (0.0, 0.0);
        }
        (Key::NumPlus, Mods::NONE) => s.zoom *= 1.25,
        (Key::NumMinus, Mods::NONE) => s.zoom /= 1.25,
        (Key::Char('a'), Mods::NONE) if s.kind == EditorKind::UvEditor => s.uv_selected = (0..uv_mesh(d).map_or(0, |m| m.faces.len())).collect(),
        (Key::Char('a'), Mods::ALT) if s.kind == EditorKind::UvEditor => s.uv_selected.clear(),
        (Key::Char('s'), Mods::ALT) | (Key::Char('s'), Mods::CTRL_SHIFT) if s.kind == EditorKind::ImageEditor => save_render(s, d),
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

    #[test]
    fn uv_editor_draws_and_selects_cube_uvs() {
        let mut app = App::new(1280, 720);
        app.doc.want_workspace = Some("UV Editing".into());
        app.frame(Input::Tick);
        app.frame(Input::Tick);
        let a = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::UvEditor).unwrap().rect;
        let body = Rect::new(a.x, a.y + t::HEADER_H + 1, a.w, a.h - t::HEADER_H - 1);
        let (cx, cy) = body.center();
        app.frame(Input::Move { x: cx, y: cy });
        app.frame(Input::Down { x: cx, y: cy, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Up { x: cx, y: cy, button: Button::Left });
        if let crate::editors::Editor::Image(s) = &app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::UvEditor).unwrap().editor {
            assert_eq!(s.uv_selected.len(), 1);
        }
        let orange = app.painter.pixels.iter().filter(|&&p| p == 0xff8500).count();
        assert!(orange > 20, "{orange}");
    }
}
