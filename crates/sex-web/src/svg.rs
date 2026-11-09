// SPDX-License-Identifier: GPL-3.0-only
//! SVG pictures: `<svg>` in a page and `.svg` files, drawn with tiny-skia.
//!
//! The parts icons, logos and simple charts use: `rect`, `circle`, `ellipse`, `line`,
//! `polyline`, `polygon` and `path` (every command, arcs included), grouped with `g`,
//! `svg`, `a` and `use` (including `symbol`), with `fill`, `stroke`, `stroke-width`,
//! opacity, `fill-rule`, line caps and joins, `transform` and the outer `viewBox`.
//! Paint comes from attributes and the `style` attribute; `currentColor` is the
//! element's CSS colour. Text, gradients, masks and filters are not drawn.
use crate::Image;
use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::tiny_skia::{self as sk, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};
use sex_html::dom::{Dom, Kind, NodeId};

/// Largest side drawn.
const MAX_SIDE: u32 = 2048;

#[derive(Clone, Copy)]
struct State {
    fill: Option<u32>,
    stroke: Option<u32>,
    width: f32,
    opacity: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
    even_odd: bool,
    cap: LineCap,
    join: LineJoin,
    current: u32,
}

/// The size an `<svg>` asks for: its width and height, or its viewBox.
pub fn natural_size(dom: &Dom, svg: NodeId) -> Option<(i32, i32)> {
    let vb = view_box(dom, svg);
    let len = |name: &str| dom.attr(svg, name).and_then(|v| number(v.trim().trim_end_matches("px")));
    match (len("width"), len("height"), vb) {
        (Some(w), Some(h), _) => Some((w as i32, h as i32)),
        (Some(w), None, Some(v)) if v[2] > 0.0 => Some((w as i32, (w * v[3] / v[2]) as i32)),
        (None, Some(h), Some(v)) if v[3] > 0.0 => Some(((h * v[2] / v[3]) as i32, h as i32)),
        (None, None, Some(v)) => Some((v[2] as i32, v[3] as i32)),
        _ => None,
    }
}

/// Draws an SVG file at its own size (fitted within `max_w` x `max_h`).
pub fn decode(text: &str, max_w: usize, max_h: usize) -> Option<Image> {
    let dom = sex_html::parse::parse(text);
    let svg = dom.descendants(Dom::ROOT).into_iter().find(|&n| dom.tag(n) == Some("svg"))?;
    let (w, h) = natural_size(&dom, svg).unwrap_or((300, 150));
    let (mut w, mut h) = (w.max(1) as usize, h.max(1) as usize);
    if w > max_w { h = (h * max_w / w).max(1); w = max_w; }
    if h > max_h { w = (w * max_h / h).max(1); h = max_h; }
    render(&dom, svg, w as u32, h as u32, 0xFF00_0000)
}

/// Draws `svg` (an element of `dom`) into a `width` x `height` straight-alpha picture;
/// `color` is its CSS colour (for `currentColor`).
pub fn render(dom: &Dom, svg: NodeId, width: u32, height: u32, color: u32) -> Option<Image> {
    let (w, h) = (width.clamp(1, MAX_SIDE), height.clamp(1, MAX_SIDE));
    let mut pm = Pixmap::new(w, h)?;
    let mut ts = Transform::identity();
    if let Some([vx, vy, vw, vh]) = view_box(dom, svg).filter(|v| v[2] > 0.0 && v[3] > 0.0) {
        // preserveAspectRatio xMidYMid meet (or none).
        let (sx, sy) = (w as f32 / vw, h as f32 / vh);
        if dom.attr(svg, "preserveaspectratio").is_some_and(|p| p.trim() == "none") {
            ts = Transform::from_row(sx, 0.0, 0.0, sy, -vx * sx, -vy * sy);
        } else {
            let s = sx.min(sy);
            let (ox, oy) = ((w as f32 - vw * s) / 2.0, (h as f32 - vh * s) / 2.0);
            ts = Transform::from_row(s, 0.0, 0.0, s, ox - vx * s, oy - vy * s);
        }
    }
    let state = State { fill: Some(0xFF00_0000), stroke: None, width: 1.0, opacity: 1.0, fill_opacity: 1.0, stroke_opacity: 1.0, even_odd: false, cap: LineCap::Butt, join: LineJoin::Miter, current: color };
    let state = paint_state(dom, svg, state);
    let mut r = Renderer { dom, pm: &mut pm, depth: 0 };
    for c in dom.nodes[svg].children.clone() { r.node(c, ts, state); }
    let mut img = Image { width: w as usize, height: h as usize, pixels: alloc::vec![0; (w * h) as usize] };
    for (d, p) in img.pixels.iter_mut().zip(pm.data().chunks_exact(4)) {
        let a = p[3] as u32;
        if a == 0 { continue; }
        let un = |c: u8| if a == 255 { c as u32 } else { ((c as u32 * 255 + a / 2) / a).min(255) };
        *d = (a << 24) | (un(p[0]) << 16) | (un(p[1]) << 8) | un(p[2]);
    }
    Some(img)
}

fn view_box(dom: &Dom, n: NodeId) -> Option<[f32; 4]> {
    let v = numbers(dom.attr(n, "viewbox")?);
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

struct Renderer<'a> {
    dom: &'a Dom,
    pm: &'a mut Pixmap,
    depth: u32,
}

impl Renderer<'_> {
    fn node(&mut self, n: NodeId, ts: Transform, parent: State) {
        let Kind::Element { tag, .. } = &self.dom.nodes[n].kind else { return };
        if self.depth > 64 { return; }
        let tag = tag.as_str();
        if matches!(tag, "defs" | "symbol" | "clippath" | "mask" | "lineargradient" | "radialgradient" | "pattern" | "marker" | "title" | "desc" | "style" | "script" | "text" | "filter" | "metadata") { return; }
        if self.dom.attr(n, "display").is_some_and(|d| d.trim() == "none") || style_value(self.dom, n, "display").is_some_and(|d| d == "none") { return; }
        let st = paint_state(self.dom, n, parent);
        let mut ts = ts;
        if let Some(t) = self.dom.attr(n, "transform") { ts = ts.pre_concat(transform(t)); }
        let f = |name: &str| self.dom.attr(n, name).and_then(|v| number(v.trim().trim_end_matches("px"))).unwrap_or(0.0);
        let path = match tag {
            "g" | "a" | "svg" | "switch" => {
                let mut ts = ts;
                if tag == "svg" { ts = ts.pre_translate(f("x"), f("y")); }
                self.depth += 1;
                for c in self.dom.nodes[n].children.clone() { self.node(c, ts, st); }
                self.depth -= 1;
                return;
            }
            "use" => {
                let href = self.dom.attr(n, "href").or_else(|| self.dom.attr(n, "xlink:href")).unwrap_or("");
                let Some(id) = href.strip_prefix('#') else { return };
                let Some(target) = self.dom.descendants(Dom::ROOT).into_iter().find(|&t| self.dom.attr(t, "id") == Some(id)) else { return };
                let mut ts = ts.pre_translate(f("x"), f("y"));
                if self.dom.tag(target) == Some("symbol") {
                    // A symbol's viewBox maps onto the use's width and height.
                    if let (Some([vx, vy, vw, vh]), w, h) = (view_box(self.dom, target), f("width"), f("height")) {
                        if vw > 0.0 && vh > 0.0 && w > 0.0 && h > 0.0 {
                            let s = (w / vw).min(h / vh);
                            ts = ts.pre_concat(Transform::from_row(s, 0.0, 0.0, s, (w - vw * s) / 2.0 - vx * s, (h - vh * s) / 2.0 - vy * s));
                        }
                    }
                    let st = paint_state(self.dom, target, st);
                    self.depth += 1;
                    for c in self.dom.nodes[target].children.clone() { self.node(c, ts, st); }
                    self.depth -= 1;
                } else {
                    self.depth += 1;
                    self.node(target, ts, st);
                    self.depth -= 1;
                }
                return;
            }
            "rect" => {
                let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
                if w <= 0.0 || h <= 0.0 { return; }
                let (mut rx, mut ry) = (f("rx"), f("ry"));
                if rx == 0.0 { rx = ry; }
                if ry == 0.0 { ry = rx; }
                let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
                if rx > 0.0 {
                    let mut pb = PathBuilder::new();
                    let k = 0.552_284_8;
                    pb.move_to(x + rx, y);
                    pb.line_to(x + w - rx, y);
                    pb.cubic_to(x + w - rx + rx * k, y, x + w, y + ry - ry * k, x + w, y + ry);
                    pb.line_to(x + w, y + h - ry);
                    pb.cubic_to(x + w, y + h - ry + ry * k, x + w - rx + rx * k, y + h, x + w - rx, y + h);
                    pb.line_to(x + rx, y + h);
                    pb.cubic_to(x + rx - rx * k, y + h, x, y + h - ry + ry * k, x, y + h - ry);
                    pb.line_to(x, y + ry);
                    pb.cubic_to(x, y + ry - ry * k, x + rx - rx * k, y, x + rx, y);
                    pb.close();
                    pb.finish()
                } else { sk::Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect) }
            }
            "circle" => { let r = f("r"); if r <= 0.0 { return; } PathBuilder::from_circle(f("cx"), f("cy"), r) }
            "ellipse" => {
                let (rx, ry) = (f("rx"), f("ry"));
                if rx <= 0.0 || ry <= 0.0 { return; }
                sk::Rect::from_xywh(f("cx") - rx, f("cy") - ry, 2.0 * rx, 2.0 * ry).and_then(PathBuilder::from_oval)
            }
            "line" => { let mut pb = PathBuilder::new(); pb.move_to(f("x1"), f("y1")); pb.line_to(f("x2"), f("y2")); pb.finish() }
            "polyline" | "polygon" => {
                let v = numbers(self.dom.attr(n, "points").unwrap_or(""));
                if v.len() < 4 { return; }
                let mut pb = PathBuilder::new();
                pb.move_to(v[0], v[1]);
                for p in v[2..].chunks_exact(2) { pb.line_to(p[0], p[1]); }
                if tag == "polygon" { pb.close(); }
                pb.finish()
            }
            "path" => path_data(self.dom.attr(n, "d").unwrap_or("")),
            _ => {
                // Unknown containers (foreignObject aside) may still hold shapes.
                if tag != "foreignobject" { self.depth += 1; for c in self.dom.nodes[n].children.clone() { self.node(c, ts, st); } self.depth -= 1; }
                return;
            }
        };
        let Some(path) = path else { return };
        if let Some(c) = st.fill.filter(|_| tag != "line") {
            let paint = paint(c, st.opacity * st.fill_opacity);
            let rule = if st.even_odd { FillRule::EvenOdd } else { FillRule::Winding };
            self.pm.fill_path(&path, &paint, rule, ts, None);
        }
        if let Some(c) = st.stroke {
            if st.width > 0.0 {
                let paint = paint(c, st.opacity * st.stroke_opacity);
                let stroke = Stroke { width: st.width, line_cap: st.cap, line_join: st.join, ..Stroke::default() };
                self.pm.stroke_path(&path, &paint, &stroke, ts, None);
            }
        }
    }
}

fn paint(c: u32, opacity: f32) -> Paint<'static> {
    let mut p = Paint::default();
    let a = ((c >> 24) as f32 * opacity.clamp(0.0, 1.0)) as u8;
    p.set_color_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, a);
    p.anti_alias = true;
    p
}

/// One property from the `style` attribute.
fn style_value(dom: &Dom, n: NodeId, name: &str) -> Option<String> {
    let style = dom.attr(n, "style")?;
    style.split(';').filter_map(|d| d.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| String::from(v.trim()))
}

fn paint_state(dom: &Dom, n: NodeId, mut st: State) -> State {
    let get = |name: &str| style_value(dom, n, name).or_else(|| dom.attr(n, name).map(|v| String::from(v.trim())));
    if let Some(c) = get("color").and_then(|v| sex_html::style::parse_color_with(&v.to_ascii_lowercase(), st.current)) { st.current = c; }
    let color = |v: &str, cur: u32| -> Option<Option<u32>> {
        let v = v.to_ascii_lowercase();
        if v == "none" || v == "transparent" { return Some(None); }
        if v.starts_with("url(") { return Some(Some(0xFF80_8080)); }
        sex_html::style::parse_color_with(&v, cur).map(Some)
    };
    if let Some(v) = get("fill").and_then(|v| color(&v, st.current)) { st.fill = v; }
    if let Some(v) = get("stroke").and_then(|v| color(&v, st.current)) { st.stroke = v; }
    if let Some(w) = get("stroke-width").and_then(|v| number(v.trim_end_matches("px"))) { st.width = w.max(0.0); }
    if let Some(o) = get("opacity").and_then(|v| number(&v)) { st.opacity *= o.clamp(0.0, 1.0); }
    if let Some(o) = get("fill-opacity").and_then(|v| number(&v)) { st.fill_opacity = o.clamp(0.0, 1.0); }
    if let Some(o) = get("stroke-opacity").and_then(|v| number(&v)) { st.stroke_opacity = o.clamp(0.0, 1.0); }
    if let Some(r) = get("fill-rule") { st.even_odd = r == "evenodd"; }
    if let Some(c) = get("stroke-linecap") { st.cap = match c.as_str() { "round" => LineCap::Round, "square" => LineCap::Square, _ => LineCap::Butt }; }
    if let Some(j) = get("stroke-linejoin") { st.join = match j.as_str() { "round" => LineJoin::Round, "bevel" => LineJoin::Bevel, _ => LineJoin::Miter }; }
    st
}

fn number(s: &str) -> Option<f32> {
    let s = s.trim();
    let (s, pct) = match s.strip_suffix('%') { Some(r) => (r, true), None => (s, false) };
    let v: f32 = s.parse().ok()?;
    Some(if pct { v / 100.0 } else { v })
}

/// Numbers in SVG's compact syntax: `1.5.5-2e1` is 1.5, .5, -20.
fn numbers(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut lex = Lexer { b: s.as_bytes(), i: 0 };
    while let Some(v) = lex.number() { out.push(v); }
    out
}

struct Lexer<'a> { b: &'a [u8], i: usize }

impl Lexer<'_> {
    fn skip(&mut self) { while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r' | b',') { self.i += 1; } }
    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let b = self.b;
        let mut i = self.i;
        if i < b.len() && matches!(b[i], b'+' | b'-') { i += 1; }
        let mut digits = false;
        while i < b.len() && b[i].is_ascii_digit() { i += 1; digits = true; }
        if i < b.len() && b[i] == b'.' { i += 1; while i < b.len() && b[i].is_ascii_digit() { i += 1; digits = true; } }
        if !digits { return None; }
        if i < b.len() && matches!(b[i], b'e' | b'E') {
            let mut j = i + 1;
            if j < b.len() && matches!(b[j], b'+' | b'-') { j += 1; }
            if j < b.len() && b[j].is_ascii_digit() { while j < b.len() && b[j].is_ascii_digit() { j += 1; } i = j; }
        }
        self.i = i;
        core::str::from_utf8(&b[start..i]).ok()?.parse().ok()
    }
    /// An arc flag: a single 0 or 1, possibly packed against the next number.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.b.get(self.i)?;
        if c == b'0' || c == b'1' { self.i += 1; Some(c == b'1') } else { None }
    }
}

fn path_data(d: &str) -> Option<sk::Path> {
    let mut lex = Lexer { b: d.as_bytes(), i: 0 };
    let mut pb = PathBuilder::new();
    let (mut cx, mut cy, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
    let mut last_ctrl: Option<(f32, f32, u8)> = None;
    let mut cmd = 0u8;
    let mut open = false;
    loop {
        lex.skip();
        if lex.i >= lex.b.len() { break; }
        let c = lex.b[lex.i];
        if c.is_ascii_alphabetic() { cmd = c; lex.i += 1; } else if cmd == 0 { return None; }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        let up = cmd.to_ascii_uppercase();
        macro_rules! n { () => { match lex.number() { Some(v) => v, None => break } } }
        match up {
            b'M' => {
                let (x, y) = (n!() + ox, n!() + oy);
                pb.move_to(x, y);
                open = true;
                (cx, cy, sx, sy) = (x, y, x, y);
                // Further pairs are line-tos.
                cmd = if rel { b'l' } else { b'L' };
                last_ctrl = None;
            }
            b'L' => { let (x, y) = (n!() + ox, n!() + oy); pb.line_to(x, y); (cx, cy) = (x, y); last_ctrl = None; }
            b'H' => { let x = n!() + ox; pb.line_to(x, cy); cx = x; last_ctrl = None; }
            b'V' => { let y = n!() + if rel { cy } else { 0.0 }; pb.line_to(cx, y); cy = y; last_ctrl = None; }
            b'C' => {
                let (x1, y1, x2, y2, x, y) = (n!() + ox, n!() + oy, n!() + ox, n!() + oy, n!() + ox, n!() + oy);
                pb.cubic_to(x1, y1, x2, y2, x, y);
                last_ctrl = Some((x2, y2, b'C'));
                (cx, cy) = (x, y);
            }
            b'S' => {
                let (x1, y1) = match last_ctrl { Some((px, py, b'C')) => (2.0 * cx - px, 2.0 * cy - py), _ => (cx, cy) };
                let (x2, y2, x, y) = (n!() + ox, n!() + oy, n!() + ox, n!() + oy);
                pb.cubic_to(x1, y1, x2, y2, x, y);
                last_ctrl = Some((x2, y2, b'C'));
                (cx, cy) = (x, y);
            }
            b'Q' => {
                let (x1, y1, x, y) = (n!() + ox, n!() + oy, n!() + ox, n!() + oy);
                pb.quad_to(x1, y1, x, y);
                last_ctrl = Some((x1, y1, b'Q'));
                (cx, cy) = (x, y);
            }
            b'T' => {
                let (x1, y1) = match last_ctrl { Some((px, py, b'Q')) => (2.0 * cx - px, 2.0 * cy - py), _ => (cx, cy) };
                let (x, y) = (n!() + ox, n!() + oy);
                pb.quad_to(x1, y1, x, y);
                last_ctrl = Some((x1, y1, b'Q'));
                (cx, cy) = (x, y);
            }
            b'A' => {
                let (rx, ry, rot) = (n!(), n!(), n!());
                let Some(large) = lex.flag() else { break };
                let Some(sweep) = lex.flag() else { break };
                let (x, y) = (n!() + ox, n!() + oy);
                arc(&mut pb, cx, cy, rx, ry, rot, large, sweep, x, y);
                (cx, cy) = (x, y);
                last_ctrl = None;
            }
            b'Z' => {
                if open { pb.close(); }
                (cx, cy) = (sx, sy);
                last_ctrl = None;
                // Only a new command may follow Z.
                cmd = 0;
                lex.skip();
                if lex.i < lex.b.len() && !lex.b[lex.i].is_ascii_alphabetic() { break; }
                // The next subpath starts at the closed one's start.
                pb.move_to(sx, sy);
            }
            _ => break,
        }
    }
    pb.finish()
}

/// An elliptical arc from (x1, y1) to (x2, y2) as cubic Béziers (SVG implementation notes F.6).
#[allow(clippy::too_many_arguments)]
fn arc(pb: &mut PathBuilder, x1: f32, y1: f32, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, x2: f32, y2: f32) {
    let (mut rx, mut ry) = (libm::fabsf(rx), libm::fabsf(ry));
    if rx == 0.0 || ry == 0.0 || (x1 == x2 && y1 == y2) { pb.line_to(x2, y2); return; }
    let phi = rot_deg.to_radians();
    let (sin, cos) = (libm::sinf(phi), libm::cosf(phi));
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (x1p, y1p) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 { let s = libm::sqrtf(lambda); rx *= s; ry *= s; }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den == 0.0 { 0.0 } else { libm::sqrtf((num / den).max(0.0)) };
    if large == sweep { coef = -coef; }
    let (cxp, cyp) = (coef * rx * y1p / ry, -coef * ry * x1p / rx);
    let (cx, cy) = (cos * cxp - sin * cyp + (x1 + x2) / 2.0, sin * cxp + cos * cyp + (y1 + y2) / 2.0);
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| libm::atan2f(ux * vy - uy * vx, ux * vx + uy * vy);
    let theta1 = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut delta = angle((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let tau = core::f32::consts::TAU;
    if !sweep && delta > 0.0 { delta -= tau; } else if sweep && delta < 0.0 { delta += tau; }
    let segments = libm::ceilf(libm::fabsf(delta) / (core::f32::consts::FRAC_PI_2)).max(1.0) as i32;
    let step = delta / segments as f32;
    let k = 4.0 / 3.0 * libm::tanf(step / 4.0);
    let point = |t: f32| {
        let (ct, st) = (libm::cosf(t), libm::sinf(t));
        (cx + rx * ct * cos - ry * st * sin, cy + rx * ct * sin + ry * st * cos, -rx * st * cos - ry * ct * sin, -rx * st * sin + ry * ct * cos)
    };
    let mut t = theta1;
    for _ in 0..segments {
        let (ax, ay, adx, ady) = point(t);
        let (bx, by, bdx, bdy) = point(t + step);
        pb.cubic_to(ax + k * adx, ay + k * ady, bx - k * bdx, by - k * bdy, bx, by);
        t += step;
    }
}

/// The `transform` attribute's functions, composed left to right.
fn transform(v: &str) -> Transform {
    let mut ts = Transform::identity();
    let mut rest = v;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let a = numbers(&rest[open + 1..open + close]);
        let g = |i: usize, d: f32| a.get(i).copied().unwrap_or(d);
        let t = match name {
            "translate" => Transform::from_translate(g(0, 0.0), g(1, 0.0)),
            "scale" => Transform::from_scale(g(0, 1.0), g(1, g(0, 1.0))),
            "rotate" => {
                let r = Transform::from_rotate(g(0, 0.0));
                if a.len() >= 3 { Transform::from_translate(a[1], a[2]).pre_concat(r).pre_translate(-a[1], -a[2]) } else { r }
            }
            "skewx" | "skewX" => Transform::from_row(1.0, 0.0, libm::tanf(g(0, 0.0).to_radians()), 1.0, 0.0, 0.0),
            "skewy" | "skewY" => Transform::from_row(1.0, libm::tanf(g(0, 0.0).to_radians()), 0.0, 1.0, 0.0, 0.0),
            "matrix" if a.len() == 6 => Transform::from_row(a[0], a[1], a[2], a[3], a[4], a[5]),
            _ => Transform::identity(),
        };
        ts = ts.pre_concat(t);
        rest = &rest[open + close + 1..];
    }
    ts
}

/// Whether `bytes` look like an SVG file.
pub fn is_svg(bytes: &[u8]) -> bool { bytes[..bytes.len().min(1024)].windows(4).any(|w| w == b"<svg") }

/// Draws every inline `<svg>` the layout placed (pictures named `svg:<node>`) at its
/// laid-out size, replacing earlier drawings; SVG files are redrawn at their laid-out
/// size too, so they stay sharp.
pub fn draw_inline(page: &sex_html::layout::Page, dom: &Dom, styles: &[Option<sex_html::style::Style>], fonts: &mut crate::Fonts) {
    fonts.images.retain(|(s, _)| !s.starts_with("svg:"));
    for it in &page.items {
        let sex_html::layout::Item::Image { w, h, src, .. } = it else { continue };
        if let Some((_, text, _)) = fonts.svgs.iter().find(|(s, _, _)| s == src) {
            let text = text.clone();
            let have = fonts.images.iter().position(|(s, _)| s == src);
            if have.is_some_and(|i| fonts.images[i].1.width == *w as usize && fonts.images[i].1.height == *h as usize) { continue; }
            if *w <= 0 || *h <= 0 || *w > 2048 || *h > 2048 { continue; }
            let file = sex_html::parse::parse(&text);
            let Some(root) = file.descendants(Dom::ROOT).into_iter().find(|&n| file.tag(n) == Some("svg")) else { continue };
            if let Some(img) = render(&file, root, *w as u32, *h as u32, 0xFF00_0000) {
                let img = alloc::rc::Rc::new(img);
                match have { Some(i) => fonts.images[i].1 = img, None => fonts.images.push((src.clone(), img)) }
            }
            continue;
        }
        let Some(n) = src.strip_prefix("svg:").and_then(|n| n.parse::<usize>().ok()) else { continue };
        if n >= dom.nodes.len() || dom.tag(n) != Some("svg") || fonts.images.iter().any(|(s, _)| s == src) { continue; }
        let color = styles.get(n).and_then(|s| s.as_ref()).map_or(0xFF00_0000, |s| s.color);
        if let Some(img) = render(dom, n, *w as u32, *h as u32, color) { fonts.images.push((src.clone(), alloc::rc::Rc::new(img))); }
    }
}
