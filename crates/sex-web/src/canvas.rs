// SPDX-License-Identifier: GPL-3.0-only
//! Canvas 2D for pages: runs the command batches `sex-js` queues for each `<canvas>`
//! (fields split by U+001F, commands by U+001E) on a premultiplied surface with
//! tiny-skia (paths, fills, strokes, gradients) and Geist (text). The browser then
//! shows each surface as a picture named `canvas:<node>`.
use crate::{measure, text_run, Fonts, Image, Target};
use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::tiny_skia::{
    self as sk, BlendMode, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, PathBuilder, PixmapMut, Point,
    RadialGradient, Shader, SpreadMode, Stroke, Transform,
};
use sex_html::layout::{FontKey, Host};

/// Largest canvas side and area this browser draws.
pub const MAX_SIDE: u32 = 4096;
pub const MAX_PIXELS: u32 = 4 << 20;

#[derive(Clone)]
enum Style {
    Color(u32),
    Linear { p: [f32; 4], stops: Vec<(f32, u32)> },
    Radial { p: [f32; 6], stops: Vec<(f32, u32)> },
}

#[derive(Clone)]
struct State {
    ctm: Transform,
    fill: Style,
    stroke: Style,
    line_width: f32,
    alpha: f32,
    cap: LineCap,
    join: LineJoin,
    font: FontKey,
    align: u8,
    baseline: u8,
    blend: BlendMode,
}

impl State {
    fn new() -> Self {
        Self {
            ctm: Transform::identity(), fill: Style::Color(0xFF00_0000), stroke: Style::Color(0xFF00_0000), line_width: 1.0,
            alpha: 1.0, cap: LineCap::Butt, join: LineJoin::Miter, font: FontKey { size: 10, bold: false, italic: false, mono: false },
            align: 0, baseline: 0, blend: BlendMode::SourceOver,
        }
    }
}

/// One canvas: its pixels (premultiplied 0xAARRGGBB) and drawing state.
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    state: State,
    stack: Vec<State>,
    path: PathBuilder,
    /// The current point, in device space.
    at: Option<(f32, f32)>,
    /// The subpath's start, for closePath.
    start: Option<(f32, f32)>,
}

/// Parses a CSS `font` shorthand as far as Geist can follow it.
pub fn parse_font(css: &str) -> FontKey {
    let mut key = FontKey { size: 10, bold: false, italic: false, mono: false };
    let lower = css.to_ascii_lowercase();
    for t in lower.split_ascii_whitespace() {
        let t = t.split('/').next().unwrap_or(t);
        if let Some(px) = t.strip_suffix("px") { if let Ok(v) = px.parse::<f32>() { key.size = (v + 0.5) as i32; } }
        else if let Some(pt) = t.strip_suffix("pt") { if let Ok(v) = pt.parse::<f32>() { key.size = (v * 4.0 / 3.0 + 0.5) as i32; } }
        else if let Some(em) = t.strip_suffix("em") { if let Ok(v) = em.parse::<f32>() { key.size = (v * 16.0 + 0.5) as i32; } }
        else if t == "bold" || t == "bolder" || t.parse::<u32>().map_or(false, |w| w >= 600) { key.bold = true; }
        else if t == "italic" || t == "oblique" { key.italic = true; }
    }
    key.mono = lower.contains("mono") || lower.contains("courier") || lower.contains("consolas");
    key.size = key.size.clamp(1, 400);
    key
}

/// `measureText` for sex-js: (CSS font, text) to pixels.
pub fn measure_css(font: &str, text: &str) -> i32 { measure(text, parse_font(font)) }

fn color_of(css: &str) -> Option<u32> { sex_html::style::parse_color(css) }

fn skia_color(c: u32, alpha: f32) -> sk::Color {
    let a = ((c >> 24) as f32 * alpha.clamp(0.0, 1.0) + 0.5) as u8;
    // Red and blue swap: tiny-skia writes R,G,B,A bytes, SexOS words are B,G,R,A in memory.
    sk::Color::from_rgba8(c as u8, (c >> 8) as u8, (c >> 16) as u8, a)
}

fn f(fields: &[&str], i: usize) -> Option<f32> {
    let v: f32 = fields.get(i)?.parse().ok()?;
    v.is_finite().then_some(v)
}

fn rem(v: f32, m: f32) -> f32 { let r = libm::fmodf(v, m); if r < 0.0 { r + m } else { r } }

fn as_bytes(words: &mut [u32]) -> &mut [u8] {
    // SAFETY: u32 has no padding and any byte pattern is a valid u8; the length is exact.
    unsafe { core::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut u8, words.len() * 4) }
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width, height, pixels: alloc::vec![0; (width * height) as usize], state: State::new(), stack: Vec::new(),
            path: PathBuilder::new(), at: None, start: None,
        }
    }

    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        let t = &self.state.ctm;
        (t.sx * x + t.kx * y + t.tx, t.ky * x + t.sy * y + t.ty)
    }

    /// How much the transform scales lengths, for stroke widths and text sizes.
    fn scale(&self) -> f32 {
        let t = &self.state.ctm;
        libm::sqrtf((t.sx * t.sy - t.kx * t.ky).abs())
    }

    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.path.move_to(x, y);
        self.at = Some((x, y));
        self.start = Some((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        if self.at.is_none() { self.path.move_to(x, y); self.start = Some((x, y)); } else { self.path.line_to(x, y); }
        self.at = Some((x, y));
    }

    /// An elliptical arc as cubic Béziers of at most a quarter turn, transformed.
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, rot: f32, a0: f32, a1: f32, ccw: bool) {
        use core::f32::consts::PI;
        let tau = 2.0 * PI;
        let mut sweep = a1 - a0;
        if !ccw {
            if sweep >= tau { sweep = tau } else { sweep = rem(sweep, tau); if sweep == 0.0 && a1 != a0 { sweep = tau; } }
        } else if -sweep >= tau { sweep = -tau } else { sweep = -rem(-sweep, tau); if sweep == 0.0 && a1 != a0 { sweep = -tau; } }
        let (cr, sr) = (libm::cosf(rot), libm::sinf(rot));
        let point = |t: f32| { let (c, s) = (libm::cosf(t), libm::sinf(t)); (cx + rx * c * cr - ry * s * sr, cy + rx * c * sr + ry * s * cr) };
        let deriv = |t: f32| { let (c, s) = (libm::cosf(t), libm::sinf(t)); (-rx * s * cr - ry * c * sr, -rx * s * sr + ry * c * cr) };
        let (x0, y0) = point(a0);
        self.line_to(x0, y0);
        let n = libm::ceilf(sweep.abs() / (PI / 2.0)).max(1.0) as usize;
        let dt = sweep / n as f32;
        let k = 4.0 / 3.0 * libm::tanf(dt / 4.0);
        let mut t = a0;
        for _ in 0..n {
            let (p0, d0) = (point(t), deriv(t));
            let (p1, d1) = (point(t + dt), deriv(t + dt));
            let c1 = self.map(p0.0 + k * d0.0, p0.1 + k * d0.1);
            let c2 = self.map(p1.0 - k * d1.0, p1.1 - k * d1.1);
            let e = self.map(p1.0, p1.1);
            self.path.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
            self.at = Some(e);
            t += dt;
        }
    }

    fn shader(&self, style: &Style) -> Shader<'static> {
        let stops = |s: &Vec<(f32, u32)>| s.iter().map(|(o, c)| GradientStop::new(o.clamp(0.0, 1.0), skia_color(*c, self.state.alpha))).collect::<Vec<_>>();
        let solid = |c: u32| Shader::SolidColor(skia_color(c, self.state.alpha));
        match style {
            Style::Color(c) => solid(*c),
            Style::Linear { p, stops: s } => {
                let first = s.first().map_or(0, |x| x.1);
                LinearGradient::new(Point::from_xy(p[0], p[1]), Point::from_xy(p[2], p[3]), stops(s), SpreadMode::Pad, self.state.ctm).unwrap_or(solid(first))
            }
            Style::Radial { p, stops: s } => {
                let first = s.first().map_or(0, |x| x.1);
                RadialGradient::new(Point::from_xy(p[0], p[1]), Point::from_xy(p[3], p[4]), p[5], stops(s), SpreadMode::Pad, self.state.ctm).unwrap_or(solid(first))
            }
        }
    }

    fn paint(&self, style: &Style) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.shader = self.shader(style);
        paint.anti_alias = true;
        paint.blend_mode = self.state.blend;
        paint
    }

    fn pixmap(&mut self) -> Option<PixmapMut<'_>> { PixmapMut::from_bytes(as_bytes(&mut self.pixels), self.width, self.height) }

    fn fill(&mut self, rule: FillRule) {
        let Some(path) = self.path.clone().finish() else { return };
        let paint = self.paint(&self.state.fill.clone());
        if let Some(mut pm) = self.pixmap() { pm.fill_path(&path, &paint, rule, Transform::identity(), None); }
    }

    fn stroke(&mut self) {
        let Some(path) = self.path.clone().finish() else { return };
        let paint = self.paint(&self.state.stroke.clone());
        let stroke = Stroke { width: self.state.line_width * self.scale(), line_cap: self.state.cap, line_join: self.state.join, ..Stroke::default() };
        if let Some(mut pm) = self.pixmap() { pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None); }
    }

    fn rect_path(&self, x: f32, y: f32, w: f32, h: f32) -> Option<sk::Path> {
        let mut pb = PathBuilder::new();
        let pts = [self.map(x, y), self.map(x + w, y), self.map(x + w, y + h), self.map(x, y + h)];
        pb.move_to(pts[0].0, pts[0].1);
        for p in &pts[1..] { pb.line_to(p.0, p.1); }
        pb.close();
        pb.finish()
    }

    fn style(fields: &[&str]) -> Option<Style> {
        let stops = |at: usize| -> Option<Vec<(f32, u32)>> {
            let n: usize = fields.get(at)?.parse().ok()?;
            (0..n.min(64)).map(|i| Some((f(fields, at + 1 + 2 * i)?, color_of(fields.get(at + 2 + 2 * i)?)?))).collect()
        };
        match *fields.first()? {
            "c" => color_of(fields.get(1)?).map(Style::Color),
            "l" => Some(Style::Linear { p: [f(fields, 1)?, f(fields, 2)?, f(fields, 3)?, f(fields, 4)?], stops: stops(5)? }),
            "r" => Some(Style::Radial { p: [f(fields, 1)?, f(fields, 2)?, f(fields, 3)?, f(fields, 4)?, f(fields, 5)?, f(fields, 6)?], stops: stops(7)? }),
            _ => None,
        }
    }

    fn text(&mut self, fonts: &mut Fonts, text: &str, x: f32, y: f32) {
        let t = self.state.ctm;
        // A plain translate and uniform scale draws glyphs straight onto the surface; any
        // other transform draws them into a scratch surface mapped through the full matrix.
        let plain = t.kx == 0.0 && t.ky == 0.0 && t.sx > 0.0 && t.sx == t.sy;
        let s = if plain { t.sx } else { libm::hypotf(t.sx, t.ky).max(libm::hypotf(t.kx, t.sy)) };
        let mut font = self.state.font;
        font.size = ((font.size as f32 * s) + 0.5) as i32;
        if font.size < 1 || font.size > 1024 { return; }
        let width = fonts.text_width(text, font);
        let (asc, desc) = fonts.metrics(font);
        let dx = match self.state.align { 1 => -width / 2, 2 => -width, _ => 0 };
        let dy = match self.state.baseline { 1 => asc, 2 => (asc - desc) / 2, 3 => -desc, _ => 0 };
        let color = match &self.state.fill { Style::Color(c) => *c, Style::Linear { stops, .. } | Style::Radial { stops, .. } => stops.first().map_or(0xFF00_0000, |s| s.1) };
        let a = ((color >> 24) as f32 * self.state.alpha) as u32;
        let color = (color & 0x00FF_FFFF) | a.min(255) << 24;
        if plain {
            let (px, py) = self.map(x, y);
            let (w, h) = (self.width as i32, self.height as i32);
            let mut target = Target { pixels: &mut self.pixels, width: w, height: h, scroll: 0, top: 0, premul: true };
            text_run(&mut target, fonts, libm::roundf(px) as i32 + dx, libm::roundf(py) as i32 + dy, font, color, text);
            return;
        }
        if s <= 0.0 { return; }
        let pad = 2 + font.size / 4;
        let (tw, th) = ((width + 2 * pad).clamp(1, MAX_SIDE as i32) as u32, (asc + desc + 2 * pad).clamp(1, MAX_SIDE as i32) as u32);
        let mut scratch = alloc::vec![0u32; (tw * th) as usize];
        let mut target = Target { pixels: &mut scratch, width: tw as i32, height: th as i32, scroll: 0, top: 0, premul: true };
        text_run(&mut target, fonts, pad, pad + asc, font, color, text);
        // Scratch pixel (u, v) sits at user point (x, y) + ((u - pad + dx) / s, (v - pad - asc + dy) / s).
        let map = t.pre_translate(x, y).pre_scale(1.0 / s, 1.0 / s).pre_translate((dx - pad) as f32, (dy - pad - asc) as f32);
        let Some(src) = sk::PixmapRef::from_bytes(as_bytes(&mut scratch), tw, th) else { return };
        let paint = sk::PixmapPaint { opacity: 1.0, blend_mode: self.state.blend, quality: sk::FilterQuality::Bilinear };
        if let Some(mut pm) = self.pixmap() { pm.draw_pixmap(0, 0, src, &paint, map, None); }
    }

    /// Runs one command.
    fn command(&mut self, fonts: &mut Fonts, cmd: &str) -> Option<()> {
        let fields: Vec<&str> = cmd.split('\u{1f}').collect();
        let (name, a) = (fields[0], &fields[1..]);
        match name {
            "sv" => { if self.stack.len() < 512 { self.stack.push(self.state.clone()); } }
            "rs" => { if let Some(s) = self.stack.pop() { self.state = s; } }
            "re0" => { self.state = State::new(); self.stack.clear(); self.path = PathBuilder::new(); self.at = None; self.pixels.fill(0); }
            // Setting the canvas's width or height (even to the same value) clears it and its state.
            "sz" => {
                let (w, h) = ((f(a, 0)? as u32).clamp(1, MAX_SIDE), (f(a, 1)? as u32).clamp(1, MAX_SIDE));
                if w * h <= MAX_PIXELS { *self = Surface::new(w, h); }
            }
            "st" => self.state.ctm = Transform::from_row(f(a, 0)?, f(a, 1)?, f(a, 2)?, f(a, 3)?, f(a, 4)?, f(a, 5)?),
            "tf" => self.state.ctm = self.state.ctm.pre_concat(Transform::from_row(f(a, 0)?, f(a, 1)?, f(a, 2)?, f(a, 3)?, f(a, 4)?, f(a, 5)?)),
            "fs" => self.state.fill = Self::style(a)?,
            "ss" => self.state.stroke = Self::style(a)?,
            "lw" => self.state.line_width = f(a, 0)?,
            "ga" => self.state.alpha = f(a, 0)?,
            "lc" => self.state.cap = match *a.first()? { "round" => LineCap::Round, "square" => LineCap::Square, _ => LineCap::Butt },
            "lj" => self.state.join = match *a.first()? { "round" => LineJoin::Round, "bevel" => LineJoin::Bevel, _ => LineJoin::Miter },
            "fo" => self.state.font = parse_font(a.first()?),
            "ta" => self.state.align = match *a.first()? { "center" => 1, "right" | "end" => 2, _ => 0 },
            "tb" => self.state.baseline = match *a.first()? { "top" | "hanging" => 1, "middle" => 2, "bottom" | "ideographic" => 3, _ => 0 },
            "co" => self.state.blend = match *a.first()? {
                "copy" => BlendMode::Source, "destination-out" => BlendMode::DestinationOut, "destination-over" => BlendMode::DestinationOver,
                "source-in" => BlendMode::SourceIn, "source-atop" => BlendMode::SourceAtop, "destination-in" => BlendMode::DestinationIn,
                "lighter" => BlendMode::Plus, "xor" => BlendMode::Xor, "multiply" => BlendMode::Multiply, "screen" => BlendMode::Screen,
                "darken" => BlendMode::Darken, "lighten" => BlendMode::Lighten, "difference" => BlendMode::Difference,
                _ => BlendMode::SourceOver,
            },
            "bp" => { self.path = PathBuilder::new(); self.at = None; self.start = None; }
            "cp" => { if self.at.is_some() { self.path.close(); self.at = self.start; } }
            "mt" => self.move_to(f(a, 0)?, f(a, 1)?),
            "lt" => self.line_to(f(a, 0)?, f(a, 1)?),
            "qc" => {
                if self.at.is_none() { self.move_to(f(a, 0)?, f(a, 1)?); }
                let (c, e) = (self.map(f(a, 0)?, f(a, 1)?), self.map(f(a, 2)?, f(a, 3)?));
                self.path.quad_to(c.0, c.1, e.0, e.1);
                self.at = Some(e);
            }
            "bc" => {
                if self.at.is_none() { self.move_to(f(a, 0)?, f(a, 1)?); }
                let (c1, c2, e) = (self.map(f(a, 0)?, f(a, 1)?), self.map(f(a, 2)?, f(a, 3)?), self.map(f(a, 4)?, f(a, 5)?));
                self.path.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
                self.at = Some(e);
            }
            "el" => self.ellipse(f(a, 0)?, f(a, 1)?, f(a, 2)?, f(a, 3)?, f(a, 4)?, f(a, 5)?, f(a, 6)?, a.get(7) == Some(&"1")),
            // arcTo approximated by a line to its first point.
            "at" => self.line_to(f(a, 0)?, f(a, 1)?),
            "rc" => {
                let (x, y, w, h) = (f(a, 0)?, f(a, 1)?, f(a, 2)?, f(a, 3)?);
                self.move_to(x, y); self.line_to(x + w, y); self.line_to(x + w, y + h); self.line_to(x, y + h);
                self.path.close();
                self.move_to(x, y);
            }
            "rr" => {
                let (x, y, w, h, r) = (f(a, 0)?, f(a, 1)?, f(a, 2)?, f(a, 3)?, f(a, 4)?);
                let r = r.min(w.abs() / 2.0).min(h.abs() / 2.0).max(0.0);
                use core::f32::consts::PI;
                self.move_to(x + r, y);
                self.ellipse(x + w - r, y + r, r, r, 0.0, -PI / 2.0, 0.0, false);
                self.ellipse(x + w - r, y + h - r, r, r, 0.0, 0.0, PI / 2.0, false);
                self.ellipse(x + r, y + h - r, r, r, 0.0, PI / 2.0, PI, false);
                self.ellipse(x + r, y + r, r, r, 0.0, PI, 1.5 * PI, false);
                self.path.close();
            }
            "fi" => self.fill(if a.first() == Some(&"evenodd") { FillRule::EvenOdd } else { FillRule::Winding }),
            "sk" => self.stroke(),
            "fr" | "sr" | "cr" => {
                let path = self.rect_path(f(a, 0)?, f(a, 1)?, f(a, 2)?, f(a, 3)?)?;
                match name {
                    "fr" => { let p = self.paint(&self.state.fill.clone()); self.pixmap()?.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None); }
                    "sr" => {
                        let p = self.paint(&self.state.stroke.clone());
                        let st = Stroke { width: self.state.line_width * self.scale(), line_join: self.state.join, ..Stroke::default() };
                        self.pixmap()?.stroke_path(&path, &p, &st, Transform::identity(), None);
                    }
                    _ => {
                        let mut p = Paint::default();
                        p.blend_mode = BlendMode::Clear;
                        self.pixmap()?.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
                    }
                }
            }
            "ft" => self.text(fonts, a.first()?, f(a, 1)?, f(a, 2)?),
            _ => {}
        }
        Some(())
    }

    /// The surface as a straight-alpha picture for painting.
    pub fn image(&self) -> Image {
        let pixels = self.pixels.iter().map(|&p| {
            let a = p >> 24;
            if a == 0 { return 0; }
            if a == 255 { return p; }
            let un = |s: u32| (((p >> s) & 0xFF) * 255 + a / 2) / a;
            a << 24 | un(16).min(255) << 16 | un(8).min(255) << 8 | un(0).min(255)
        }).collect();
        Image { width: self.width as usize, height: self.height as usize, pixels }
    }
}

/// All canvases of a page.
#[derive(Default)]
pub struct Canvases {
    pub surfaces: Vec<(usize, Surface)>,
}

impl Canvases {
    pub fn new() -> Self { Self::default() }

    /// Runs `commands` on canvas `node`, `width` by `height` (a new size clears it), and
    /// refreshes its picture in `fonts`.
    pub fn run(&mut self, fonts: &mut Fonts, node: usize, width: u32, height: u32, commands: &str) {
        let (width, height) = (width.clamp(1, MAX_SIDE), height.clamp(1, MAX_SIDE));
        if width * height > MAX_PIXELS { return; }
        let i = match self.surfaces.iter().position(|(n, _)| *n == node) {
            Some(i) => i,
            None => { self.surfaces.push((node, Surface::new(width, height))); self.surfaces.len() - 1 }
        };
        let surface = &mut self.surfaces[i].1;
        if surface.width != width || surface.height != height { *surface = Surface::new(width, height); }
        for cmd in commands.split('\u{1e}') { let _ = surface.command(fonts, cmd); }
        let name = alloc::format!("canvas:{node}");
        let image = surface.image();
        match fonts.images.iter_mut().find(|(s, _)| *s == name) {
            Some(slot) => slot.1 = alloc::rc::Rc::new(image),
            None => fonts.images.push((name, alloc::rc::Rc::new(image))),
        }
    }

    /// Runs a page's queued batches; sizes come from each canvas's attributes.
    pub fn run_queue(&mut self, fonts: &mut Fonts, dom: &sex_html::dom::Dom, queue: Vec<(usize, String)>) -> bool {
        let any = !queue.is_empty();
        for (node, batch) in queue {
            let (w, h) = canvas_size(dom, node);
            self.run(fonts, node, w, h, &batch);
        }
        any
    }
}

/// Feeds WebGL canvases the pictures they wait for, and shows each canvas that drew a new
/// frame. True if any changed.
pub fn present_gl(fonts: &mut Fonts, gl: &mut sex_js::webgl::GlCanvases) -> bool {
    let mut any = false;
    for (node, canvas) in gl.by_node.iter_mut() {
        for (target, tex, src, flip, premul) in core::mem::take(&mut canvas.pictures) {
            let Some((_, img)) = fonts.images.iter().find(|(s, _)| *s == src) else { continue };
            canvas.ctx.upload_picture(target, tex, img.width as u32, img.height as u32, &img.pixels, flip, premul);
        }
        if canvas.ctx.frame == canvas.shown { continue; }
        canvas.shown = canvas.ctx.frame;
        let (width, height) = (canvas.ctx.width as usize, canvas.ctx.height as usize);
        let image = Image { width, height, pixels: canvas.ctx.present(canvas.alpha) };
        let name = alloc::format!("canvas:{node}");
        match fonts.images.iter_mut().find(|(s, _)| *s == name) {
            Some(slot) => slot.1 = alloc::rc::Rc::new(image),
            None => fonts.images.push((name, alloc::rc::Rc::new(image))),
        }
        any = true;
    }
    any
}

/// A canvas element's drawing size: its `width` and `height` attributes, 300 by 150 by default.
pub fn canvas_size(dom: &sex_html::dom::Dom, node: usize) -> (u32, u32) {
    let get = |name, default| dom.attr(node, name).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(default);
    (get("width", 300), get("height", 150))
}
