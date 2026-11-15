// SPDX-License-Identifier: GPL-3.0-only
//! The canvas: VectorCraft's renderer draws the art, this puts it on a grey desk with a tool
//! strip on the left and a status line below, draws the tools' overlays (pen rubber band,
//! marquee, anchors, Smart Guides) and the selection's box, and turns SexOS keys and pointer
//! events into the engine's tool events and commands. The art is drawn again only when the
//! document, the view or the window changes.
use crate::drawing::{Drawing, TOOLS};
use sex_office::{Canvas, Pen};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_key::Key;
use sex_std::sex_pdx::app_pointer::{Kind, Pointer};
use serde_json::json;
use vectorcraft_geom::{BezPath, PathEl, Point};
use vectorcraft_tools::{Mods, Overlay, PointerKind, ToolKey};

const DESK: u32 = 0x00d5d7db;
const BAR_BG: u32 = 0x00f8f9fa;
const DIM: u32 = 0x005f6368;
const INK: u32 = 0x00202124;
const LINE: u32 = 0x00dadce0;
const SELECTED: u32 = 0x001a73e8;
const STATUS_H: i32 = 26;
const STRIP_W: i32 = 40;
const MARGIN: f64 = 24.0;

/// Set-1 scan codes Shell passes.
mod code {
    pub const ESC: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const TAB: u8 = 0x0F;
    pub const ENTER: u8 = 0x1C;
    pub const A: u8 = 0x1E;
    pub const S: u8 = 0x1F;
    pub const D: u8 = 0x20;
    pub const G: u8 = 0x22;
    pub const C: u8 = 0x2E;
    pub const V: u8 = 0x2F;
    pub const X: u8 = 0x2D;
    pub const Y: u8 = 0x15;
    pub const Z: u8 = 0x2C;
    pub const E: u8 = 0x12;
    /// Ctrl+Shift+1..4: Pathfinder.
    pub fn digit(c: u8) -> Option<u8> { (0x02..=0x0A).contains(&c).then(|| c - 1) }
}

const NONE: Mods = Mods { shift: false, alt: false, cmd: false, ctrl: false, space: false };

pub struct Ui {
    d: Drawing,
    name: String,
    /// Pixels a point.
    zoom: f64,
    /// The point at the canvas's top left.
    origin: Point,
    /// The art as last drawn: (revision key, width, height, pixels).
    art: Option<(String, i32, i32, Vec<u32>)>,
    message: String,
    pen: Pen,
    /// The pointer went down on the canvas with the left button (a tool gesture) or the
    /// middle one / the Hand tool (panning from this screen point).
    pressed: Option<Press>,
    /// The window size the view was fitted to.
    fitted: Option<(i32, i32)>,
}

#[derive(Clone, Copy)]
enum Press {
    Tool,
    Pan(i32, i32, Point),
}

impl Ui {
    pub fn new(d: Drawing, name: String) -> Ui {
        Ui { d, name, zoom: 1.0, origin: Point::new(0.0, 0.0), art: None, message: String::new(), pen: Pen::sans(13), pressed: None, fitted: None }
    }

    /// Fits the artboard into a canvas `w` by `h` pixels.
    fn fit(&mut self, w: i32, h: i32) {
        let a = self.d.artboard();
        let (cw, ch) = ((w - STRIP_W) as f64 - 2.0 * MARGIN, (h - STATUS_H) as f64 - 2.0 * MARGIN);
        self.zoom = (cw / a.width()).min(ch / a.height()).clamp(0.05, 64.0);
        let (vw, vh) = ((w - STRIP_W) as f64 / self.zoom, (h - STATUS_H) as f64 / self.zoom);
        self.origin = Point::new(a.center().x - vw / 2.0, a.center().y - vh / 2.0);
        self.fitted = Some((w, h));
    }

    /// Zooms by `factor`, keeping the point under canvas pixel (`x`, `y`) where it is.
    fn zoom_at(&mut self, factor: f64, x: f64, y: f64) {
        let at = self.to_doc(x, y);
        self.zoom = (self.zoom * factor).clamp(0.05, 64.0);
        self.origin = Point::new(at.x - x / self.zoom, at.y - y / self.zoom);
    }

    /// Canvas pixel → point.
    fn to_doc(&self, x: f64, y: f64) -> Point { Point::new(self.origin.x + x / self.zoom, self.origin.y + y / self.zoom) }

    /// Point → window pixel.
    fn to_screen(&self, p: Point) -> (i32, i32) {
        (STRIP_W + ((p.x - self.origin.x) * self.zoom).round() as i32, ((p.y - self.origin.y) * self.zoom).round() as i32)
    }

    pub fn draw(&mut self, window: &mut sex_app::Window) {
        let (width, height) = window.size();
        let (w, h) = (width as i32, height as i32);
        if self.fitted.is_none() { self.fit(w, h); }
        let (cw, ch) = ((w - STRIP_W).max(1), (h - STATUS_H).max(1));
        let key = format!("{}:{}:{}:{}:{}:{}", self.d.revision(), self.zoom, self.origin.x, self.origin.y, cw, ch);
        if self.art.as_ref().is_none_or(|a| a.0 != key) {
            let started = std::time::Instant::now();
            let px = self.d.render(cw as u32, ch as u32, self.zoom, self.origin, DESK);
            sex_std::record(&format!("[gestalt.render] width={cw} height={ch} zoom={:.3} objects={} ms={}",
                self.zoom, self.d.objects(), started.elapsed().as_millis()));
            self.art = Some((key, cw, ch, px));
        }
        let overlays = self.d.overlays(self.zoom);
        let bounds = self.d.selection_bounds();
        let mut canvas = Canvas::new(window.pixels(), width, height);
        canvas.fill(0, 0, w, h, DESK);
        if let Some((_, aw, _, px)) = &self.art { canvas.blit(STRIP_W, 0, px, *aw as usize, (STRIP_W, 0, w - STRIP_W, ch)); }
        // The selection's box and its eight handles.
        if let Some(b) = bounds {
            let (x0, y0) = self.to_screen(Point::new(b.x0, b.y0));
            let (x1, y1) = self.to_screen(Point::new(b.x1, b.y1));
            canvas.frame(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1), SELECTED);
            for (hx, hy) in [(x0, y0), ((x0 + x1) / 2, y0), (x1, y0), (x1, (y0 + y1) / 2), (x1, y1), ((x0 + x1) / 2, y1), (x0, y1), (x0, (y0 + y1) / 2)] {
                canvas.fill(hx - 3, hy - 3, 7, 7, SELECTED);
                canvas.fill(hx - 2, hy - 2, 5, 5, 0x00ffffff);
            }
        }
        for o in &overlays { self.overlay(&mut canvas, o, ch); }
        // The tool strip.
        canvas.fill(0, 0, STRIP_W, h, BAR_BG);
        canvas.fill(STRIP_W - 1, 0, 1, h, LINE);
        let active = self.d.tool_id();
        let row = 30;
        for (i, (letter, id, _)) in TOOLS.iter().enumerate() {
            let y = 6 + i as i32 * row;
            let on = *id == active;
            if on { canvas.fill(4, y, STRIP_W - 9, row - 4, SELECTED); }
            let label = letter.to_ascii_uppercase().to_string();
            let lw = self.pen.measure(&label);
            self.pen.text(&mut canvas, (STRIP_W - 1 - lw) / 2, y + 6, &label, if on { 0x00ffffff } else { INK }, None);
        }
        // The status line.
        canvas.fill(0, ch, w, STATUS_H, BAR_BG);
        canvas.fill(0, ch, w, 1, LINE);
        let tool = TOOLS.iter().find(|t| t.1 == active).map_or(active, |t| t.2);
        let status = if self.message.is_empty() {
            format!("{}{}   {tool}   {:.0}%   {} object{}, {} selected   Ctrl+S save  Ctrl+E PNG  Ctrl+Shift+E PDF  Ctrl+Z undo  Ctrl+G group  Ctrl+Shift+1-4 Pathfinder",
                self.name, if self.d.dirty() { " (changed)" } else { "" }, self.zoom * 100.0, self.d.objects(), if self.d.objects() == 1 { "" } else { "s" }, self.d.selected())
        } else {
            self.message.clone()
        };
        self.pen.text(&mut canvas, 10, ch + (STATUS_H - self.pen.line_height()) / 2, &status, DIM, Some((0, w - 10)));
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(15).collect();
        window.title(format!("{short}{}", if self.d.dirty() { "*" } else { "" }).as_bytes());
    }

    /// Draws one tool overlay, clipped above the status line.
    fn overlay(&mut self, c: &mut Canvas, o: &Overlay, clip_h: i32) {
        let rgb = |c: [u8; 3]| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
        let clip = (STRIP_W, 0, i32::MAX, clip_h);
        match o {
            Overlay::Marquee(r) | Overlay::SelectionMarquee { rect: r, .. } => {
                let (x0, y0) = self.to_screen(Point::new(r.x0, r.y0));
                let (x1, y1) = self.to_screen(Point::new(r.x1, r.y1));
                c.frame(x0.min(x1), y0.min(y1), (x1 - x0).abs().max(1), (y1 - y0).abs().max(1), DIM);
            }
            Overlay::Path { path, color, dashed, .. } => self.path(c, path, rgb(*color), *dashed, clip),
            Overlay::SharedPath { path, color, .. } => self.path(c, path, rgb(*color), false, clip),
            Overlay::Line { a, b, color, .. } | Overlay::Distance { a, b, color, .. } => {
                let (p, q) = (self.to_screen(*a), self.to_screen(*b));
                line(c, p, q, rgb(*color), clip);
            }
            Overlay::Anchor { p, color, filled, size } | Overlay::CurvePoint { p, color, filled, size } => {
                let (x, y) = self.to_screen(*p);
                let r = (*size as i32 / 2).max(2);
                c.fill(x - r, y - r, 2 * r + 1, 2 * r + 1, rgb(*color));
                if !filled { c.fill(x - r + 1, y - r + 1, 2 * r - 1, 2 * r - 1, 0x00ffffff); }
            }
            Overlay::Handle { p, color } => {
                let (x, y) = self.to_screen(*p);
                c.fill(x - 2, y - 2, 5, 5, rgb(*color));
            }
            Overlay::Label { p, text, color } => {
                let (x, y) = self.to_screen(*p);
                self.pen.text(c, x + 6, y + 4, text, rgb(*color), None);
            }
            Overlay::Measure { p, text } => {
                let (x, y) = self.to_screen(*p);
                let tw = self.pen.measure(text);
                c.fill(x + 8, y + 8, tw + 8, self.pen.line_height() + 4, 0x00505050);
                self.pen.text(c, x + 12, y + 10, text, 0x00ffffff, None);
            }
            Overlay::GridLine { a, b, color } => {
                let (p, q) = (self.to_screen(*a), self.to_screen(*b));
                line(c, p, q, rgb([color[0], color[1], color[2]]), clip);
            }
            Overlay::Highlight { quad, color } => {
                let pts: Vec<(i32, i32)> = quad.iter().map(|p| self.to_screen(*p)).collect();
                for i in 0..4 { line(c, pts[i], pts[(i + 1) % 4], rgb([color[0], color[1], color[2]]), clip); }
            }
            _ => {}
        }
    }

    /// Strokes a Bézier path one pixel wide, flattened to lines.
    fn path(&self, c: &mut Canvas, path: &BezPath, color: u32, dashed: bool, clip: (i32, i32, i32, i32)) {
        let mut start = None;
        let mut last = None;
        let mut n = 0;
        vectorcraft_geom::kurbo::flatten(path.iter(), 0.25 / self.zoom.max(0.01), |el| {
            match el {
                PathEl::MoveTo(p) => { let s = self.to_screen(p); start = Some(s); last = Some(s); }
                PathEl::LineTo(p) => {
                    let q = self.to_screen(p);
                    if let Some(a) = last { if !dashed || n % 2 == 0 { line(c, a, q, color, clip); } }
                    n += 1;
                    last = Some(q);
                }
                PathEl::ClosePath => { if let (Some(a), Some(s)) = (last, start) { line(c, a, s, color, clip); } last = start; }
                _ => {}
            }
        });
    }

    fn save(&mut self) {
        match self.d.save() {
            Ok(bytes) => {
                // An `.ai` or EPS drawing saves beside itself as SVG.
                self.name = self.d.path.clone();
                sex_std::record(&format!("[gestalt.saved] name={} bytes={bytes} objects={}", self.d.path, self.d.objects()));
                self.message = format!("Saved {} ({bytes} bytes)", self.name);
            }
            Err(e) => {
                sex_std::record(&format!("[gestalt.save-failed] {e}"));
                self.message = e;
            }
        }
    }

    fn export(&mut self, format: &str) {
        match self.d.export(format) {
            Ok((name, bytes)) => {
                sex_std::record(&format!("[gestalt.exported] name={name} format={format} bytes={bytes}"));
                self.message = format!("Exported {name} ({bytes} bytes)");
            }
            Err(e) => {
                sex_std::record(&format!("[gestalt.export-failed] format={format} {e}"));
                self.message = e;
            }
        }
    }

    /// Runs an engine command and records it, or says why not.
    fn command(&mut self, id: &str, params: serde_json::Value) {
        match self.d.command(id, params) {
            Ok(_) => sex_std::record(&format!("[gestalt.command] {id} objects={} selected={}", self.d.objects(), self.d.selected())),
            Err(e) => {
                sex_std::record(&format!("[gestalt.command-failed] {id} {e}"));
                self.message = e;
            }
        }
    }

    fn pick_tool(&mut self, id: &str) {
        if self.d.tool(id) { sex_std::record(&format!("[gestalt.tool] id={id}")); }
    }

    fn key(&mut self, key: Key, w: i32, h: i32) {
        self.message.clear();
        let mods = Mods { shift: key.shift, ctrl: key.control, ..NONE };
        if key.control {
            let typed = sex_app::typed(Key { control: false, ..key });
            match key.code {
                code::S => return self.save(),
                code::E => return self.export(if key.shift { "pdf" } else { "png" }),
                code::Z if key.shift => return self.command("edit.redo", json!({})),
                code::Z => return self.command("edit.undo", json!({})),
                code::Y => return self.command("edit.redo", json!({})),
                code::C => return self.command("edit.copy", json!({})),
                code::X => return self.command("edit.cut", json!({})),
                code::V => return self.command("edit.paste", json!({})),
                code::D => return self.command("edit.duplicate", json!({})),
                code::A => return self.command("select.all", json!({})),
                code::G if key.shift => return self.command("object.ungroup", json!({})),
                code::G => return self.command("object.group", json!({})),
                c if key.shift && code::digit(c).is_some_and(|d| (1..=4).contains(&d)) => {
                    let op = ["unite", "minusFront", "intersect", "exclude"][code::digit(c).unwrap() as usize - 1];
                    return self.command(&format!("object.pathfinder.{op}"), json!({}));
                }
                _ => {}
            }
            let (cx, cy) = ((w - STRIP_W) as f64 / 2.0, (h - STATUS_H) as f64 / 2.0);
            match typed {
                Some(b'=' | b'+') => return self.zoom_at(1.25, cx, cy),
                Some(b'-') => return self.zoom_at(0.8, cx, cy),
                Some(b'0') => return self.fit(w, h),
                Some(b'1') => return self.zoom_at(1.0 / self.zoom, cx, cy),
                _ => {}
            }
        }
        let tool_key = match (key.code, sex_app::nav(key)) {
            (code::ESC, _) => Some(ToolKey::Escape),
            (code::ENTER, _) => Some(ToolKey::Enter),
            (code::TAB, _) => Some(ToolKey::Tab),
            (code::BACKSPACE, _) => Some(ToolKey::Backspace),
            (_, Some(Nav::Delete)) => Some(ToolKey::Delete),
            (_, Some(Nav::Left)) => Some(ToolKey::Left),
            (_, Some(Nav::Right)) => Some(ToolKey::Right),
            (_, Some(Nav::Up)) => Some(ToolKey::Up),
            (_, Some(Nav::Down)) => Some(ToolKey::Down),
            (_, Some(Nav::Home)) => Some(ToolKey::Home),
            (_, Some(Nav::End)) => Some(ToolKey::End),
            _ => None,
        };
        if self.d.wants_text() {
            // The Type tool is taking text: keys go to it (Esc ends it).
            if let Some(k) = tool_key { let _ = self.d.key(k, mods, self.zoom); return; }
            if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) {
                let mut s = [0u8; 4];
                if let Err(e) = self.d.text(ch.encode_utf8(&mut s), self.zoom) { self.message = e; }
            }
            return;
        }
        if let Some(k) = tool_key {
            let before = self.d.revision();
            let _ = self.d.key(k, mods, self.zoom);
            if self.d.revision() != before { return; }
            // Not the tool's: Delete removes the selection, arrows nudge it.
            let step = if key.shift { 10.0 } else { 1.0 };
            match k {
                ToolKey::Delete | ToolKey::Backspace if self.d.selected() > 0 => self.command("edit.clear", json!({})),
                ToolKey::Left => self.command("object.move", json!({"dx": -step, "dy": 0})),
                ToolKey::Right => self.command("object.move", json!({"dx": step, "dy": 0})),
                ToolKey::Up => self.command("object.move", json!({"dx": 0, "dy": -step})),
                ToolKey::Down => self.command("object.move", json!({"dx": 0, "dy": step})),
                _ => {}
            }
            return;
        }
        if !key.control {
            if let Some(ch) = sex_app::text(key) {
                let lower = ch.to_ascii_lowercase();
                if lower == 'm' && key.shift { return self.pick_tool("shapeBuilder"); }
                if let Some(&(_, id, _)) = TOOLS.iter().find(|t| t.0 == lower) { self.pick_tool(id); }
            }
        }
    }

    /// The tool strip picks tools; on the canvas the left button drives the active tool, the
    /// middle button (or the Hand tool) pans, and the wheel scrolls.
    fn pointer(&mut self, p: Pointer, w: i32, h: i32) -> bool {
        let (x, y) = (p.x as i32, p.y as i32);
        let at = self.to_doc((x - STRIP_W) as f64, y as f64);
        match p.kind {
            Kind::Wheel { steps } => {
                self.origin.y -= steps as f64 * 48.0 / self.zoom;
                true
            }
            Kind::Down { button: 0 } if x < STRIP_W => {
                let i = (y - 6) / 30;
                if let Some(&(_, id, _)) = (i >= 0).then(|| TOOLS.get(i as usize)).flatten() { self.pick_tool(id); }
                true
            }
            Kind::Down { button } if y < h - STATUS_H && x >= STRIP_W => {
                self.message.clear();
                if button == 1 || self.d.tool_id() == "hand" {
                    self.pressed = Some(Press::Pan(x, y, self.origin));
                    return false;
                }
                if button != 0 { return false; }
                self.pressed = Some(Press::Tool);
                let _ = self.d.pointer(PointerKind::Down, at, NONE, self.zoom);
                true
            }
            Kind::Move => match self.pressed {
                Some(Press::Pan(x0, y0, o)) => {
                    self.origin = Point::new(o.x - (x - x0) as f64 / self.zoom, o.y - (y - y0) as f64 / self.zoom);
                    true
                }
                Some(Press::Tool) => { let _ = self.d.pointer(PointerKind::Drag, at, NONE, self.zoom); true }
                None if x >= STRIP_W && y < h - STATUS_H => {
                    // Hover: the pen's rubber band, Smart Guides' hints.
                    let before = self.d.overlays_len(self.zoom);
                    let _ = self.d.pointer(PointerKind::Move, at, NONE, self.zoom);
                    before > 0 || self.d.overlays_len(self.zoom) > 0
                }
                None => false,
            },
            Kind::Up { .. } => match self.pressed.take() {
                Some(Press::Tool) => {
                    let objects = self.d.objects();
                    let _ = self.d.pointer(PointerKind::Up, at, NONE, self.zoom);
                    sex_std::record(&format!("[gestalt.gesture] tool={} objects={} selected={}{}", self.d.tool_id(), self.d.objects(), self.d.selected(),
                        self.d.selection_bounds().map_or(String::new(), |b| format!(" box={:.0},{:.0},{:.0},{:.0}", b.x0, b.y0, b.width(), b.height()))));
                    if self.d.objects() != objects { self.message.clear(); }
                    true
                }
                Some(Press::Pan(..)) => true,
                None => false,
            },
            _ => { let _ = w; false }
        }
    }

    pub fn run(mut self, window: &mut sex_app::Window) {
        self.draw(window);
        let (target, task) = sex_std::identity();
        sex_std::record(&format!("[gestalt.ready] target={target} task={task}"));
        loop {
            let (w, h) = { let (w, h) = window.size(); (w as i32, h as i32) };
            match window.next() {
                Event::Close => break,
                Event::Resize { .. } => {
                    if self.fitted.is_some_and(|f| f != { let (w, h) = window.size(); (w as i32, h as i32) }) { self.fitted = None; }
                    self.draw(window)
                }
                Event::Key(key) => { self.key(key, w, h); self.draw(window); }
                Event::Pointer(p) => { if self.pointer(p, w, h) { self.draw(window); } }
                _ => {}
            }
        }
        sex_std::record("[gestalt.closed]");
    }
}

/// A one-pixel line from `a` to `b`, inside `clip` (x, y, w, h).
fn line(c: &mut Canvas, a: (i32, i32), b: (i32, i32), color: u32, clip: (i32, i32, i32, i32)) {
    let (mut x, mut y) = a;
    let (dx, dy) = ((b.0 - x).abs(), -(b.1 - y).abs());
    if dx > 8192 || -dy > 8192 { return; }
    let (sx, sy) = (if x < b.0 { 1 } else { -1 }, if y < b.1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        if x >= clip.0 && y >= clip.1 && y < clip.1 + clip.3 && x < clip.0.saturating_add(clip.2) { c.fill(x, y, 1, 1, color); }
        if (x, y) == b { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x += sx; }
        if e2 <= dx { err += dx; y += sy; }
    }
}
