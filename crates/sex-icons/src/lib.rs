// SPDX-License-Identifier: GPL-3.0-only
//! The SexOS icon set, drawn at run time from its one source, `assets/icons/icons.txt`.
//!
//! Three kinds share one line style (2-unit strokes, round caps and joins, a
//! 24-unit grid): `line` icons in one ink for interface chrome (Weft, SilkBar),
//! `app` icons with a rose-gold glyph on a charcoal badge (launcher), and `file`
//! icons with the glyph on a sheet or as a folder (Files). The colours are the lips
//! brand's (docs/BRAND.md); `scripts/brand/icons.py` writes the same icons as SVG
//! and PNG with the same palette, so every form matches.
//!
//! No heap, no floating-point hardware assumptions beyond `f32` arithmetic: paths
//! are flattened and stroked into polygons and scan-converted by `sex_font::raster`.
//! The caller lends the buffers.
#![no_std]

use sex_font::raster::{scratch_len, Raster, FX};

/// Every icon, as text: `KIND NAME PATH` per line.
pub static SOURCE: &str = include_str!("../../../assets/icons/icons.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Line, App, File }

/// Every icon: (kind, name, path data).
pub fn all() -> impl Iterator<Item = (Kind, &'static str, &'static str)> {
    SOURCE.lines().filter_map(|l| {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') { return None; }
        let (kind, rest) = l.split_once(' ')?;
        let (name, data) = rest.split_once(' ')?;
        let kind = match kind { "line" => Kind::Line, "app" => Kind::App, "file" => Kind::File, _ => return None };
        Some((kind, name, data))
    })
}

/// The path data of icon `name` of `kind`.
pub fn find(kind: Kind, name: &str) -> Option<&'static str> {
    all().find(|(k, n, _)| *k == kind && *n == name).map(|(_, _, d)| d)
}

/// The app icon for a program name or one of its aliases (`files` is Loom,
/// `term` is Spindle...), `None` for a program without one.
pub fn app_for(program: &str) -> Option<&'static str> {
    let name = match program {
        "files" => "loom", "term" | "terminal" => "spindle", "settings" => "tatami", "monitor" => "mesh",
        "web" | "browser" => "kaleidoscope", "play" | "music" => "qupid", "sex-calc" | "calc" => "abacus",
        "calendar" => "date", "pdf" => "vellum", "view" => "gaze", "store" | "sex-shop" => "sexshop",
        "search" => "voyeur", "shapes" => "gestalt", "talk" => "flirt", other => other,
    };
    all().find(|(k, n, _)| *k == Kind::App && *n == name).map(|(_, n, _)| n)
}

/// The file icon for a name: by extension, or `folder`.
pub fn file_for(name: &[u8], dir: bool) -> &'static str {
    if dir { return "folder"; }
    let Some(dot) = name.iter().rposition(|b| *b == b'.') else { return "file" };
    let ext = &name[dot + 1..];
    let mut e = [0u8; 5];
    if ext.len() > e.len() { return "file"; }
    for (o, b) in e.iter_mut().zip(ext) { *o = b.to_ascii_lowercase(); }
    match &e[..ext.len()] {
        b"png" | b"jpg" | b"jpeg" | b"gif" | b"webp" | b"bmp" | b"qoi" | b"svg" => "image",
        b"pdf" => "pdf",
        b"dng" | b"cr2" | b"cr3" | b"nef" | b"arw" | b"orf" | b"rw2" | b"pef" | b"raf" => "raw",
        b"mp4" | b"webm" | b"mkv" | b"mov" | b"avi" | b"m4v" => "video",
        b"mp3" | b"ogg" | b"opus" | b"flac" | b"wav" | b"m4a" => "audio",
        b"txt" | b"md" | b"rs" | b"toml" | b"json" | b"csv" | b"log" | b"html" | b"css" | b"js" => "text",
        b"zip" | b"tar" | b"gz" | b"xz" | b"zst" => "archive",
        _ => "file",
    }
}

// ---- Path data -------------------------------------------------------------

/// What a path walk reports, in grid units.
pub trait Sink {
    /// A subpath starts; `fill` when an `F` came before its `M`.
    fn begin(&mut self, x: f32, y: f32, fill: bool);
    fn line_to(&mut self, x: f32, y: f32);
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32);
    fn cubic_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32);
    fn close(&mut self);
    /// `O` (stroked) or `D` (filled) circle.
    fn circle(&mut self, cx: f32, cy: f32, r: f32, filled: bool);
}

/// Walks path data into `sink`. `false` if it does not parse (the sink may have
/// seen part of it).
pub fn walk(data: &str, sink: &mut impl Sink) -> bool {
    let b = data.as_bytes();
    let mut i = 0;
    let mut cmd = 0u8;
    let (mut x, mut y, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
    let mut fill_next = false;
    let mut first_of_m = false;
    let skip = |i: &mut usize| while *i < b.len() && (b[*i] == b' ' || b[*i] == b',') { *i += 1 };
    let mut nums = [0f32; 6];
    loop {
        skip(&mut i);
        if i >= b.len() { return true; }
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
            match cmd {
                b'F' => { fill_next = true; continue; }
                b'Z' | b'z' => { sink.close(); x = sx; y = sy; continue; }
                b'M' | b'm' => first_of_m = true,
                _ => {}
            }
        }
        let arity = match cmd.to_ascii_uppercase() {
            b'M' | b'L' => 2, b'H' | b'V' => 1, b'Q' => 4, b'C' => 6, b'O' | b'D' => 3, _ => return false,
        };
        for n in nums.iter_mut().take(arity) {
            skip(&mut i);
            let start = i;
            if i < b.len() && b[i] == b'-' { i += 1; }
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') { i += 1; }
            let Some(v) = core::str::from_utf8(&b[start..i]).ok().and_then(|s| s.parse().ok()) else { return false };
            *n = v;
        }
        let rel = cmd.is_ascii_lowercase();
        let (bx, by) = if rel { (x, y) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' if first_of_m => {
                (x, y) = (bx + nums[0], by + nums[1]);
                (sx, sy) = (x, y);
                sink.begin(x, y, fill_next);
                fill_next = false;
                first_of_m = false;
            }
            b'M' | b'L' => { (x, y) = (bx + nums[0], by + nums[1]); sink.line_to(x, y); }
            b'H' => { x = if rel { x + nums[0] } else { nums[0] }; sink.line_to(x, y); }
            b'V' => { y = if rel { y + nums[0] } else { nums[0] }; sink.line_to(x, y); }
            b'Q' => { sink.quad_to(bx + nums[0], by + nums[1], bx + nums[2], by + nums[3]); (x, y) = (bx + nums[2], by + nums[3]); }
            b'C' => {
                sink.cubic_to(bx + nums[0], by + nums[1], bx + nums[2], by + nums[3], bx + nums[4], by + nums[5]);
                (x, y) = (bx + nums[4], by + nums[5]);
            }
            b'O' | b'D' => sink.circle(nums[0], nums[1], nums[2], cmd.to_ascii_uppercase() == b'D'),
            _ => return false,
        }
    }
}

// ---- Rasterising -----------------------------------------------------------

/// Grid units to device pixels: `device = offset + grid * scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place { pub scale: f32, pub x: f32, pub y: f32 }

impl Place {
    /// The 24-unit grid filling a `side` pixel square.
    pub fn square(side: usize) -> Place { Place { scale: side as f32 / 24.0, x: 0.0, y: 0.0 } }
    /// Scaled by `k` about the grid's centre (an app glyph inside its badge).
    pub fn inner(self, k: f32) -> Place {
        Place { scale: self.scale * k, x: self.x + self.scale * 12.0 * (1.0 - k), y: self.y + self.scale * 12.0 * (1.0 - k) }
    }
}

const MAX_POINTS: usize = 400;

/// Turns a path into filled polygons on a raster: fills directly, strokes as a
/// quad per segment and a disc at every point. All polygons are wound the same
/// way, so where they overlap the coverage simply saturates.
struct Stroker<'r, 'a> {
    raster: &'r mut Raster<'a>,
    place: Place,
    half: f32,
    pts: [(f32, f32); MAX_POINTS],
    n: usize,
    fill: bool,
    closed: bool,
}

/// A unit circle, 32 points.
const CIRCLE: [(f32, f32); 32] = {
    // cos and sin of k * 11.25 degrees, from the first octant mirrored.
    const C: [f32; 9] = [1.0, 0.980_785_3, 0.923_879_5, 0.831_469_6, 0.707_106_8, 0.555_570_2, 0.382_683_4, 0.195_090_3, 0.0];
    let mut out = [(0.0, 0.0); 32];
    let mut k = 0;
    while k < 32 {
        let q = k / 8;
        let j = k % 8;
        let (c, s) = (C[j], C[8 - j]);
        out[k] = match q { 0 => (c, s), 1 => (-s, c), 2 => (-c, -s), _ => (s, -c) };
        k += 1;
    }
    out
};

impl Stroker<'_, '_> {
    fn dev(&self, (x, y): (f32, f32)) -> (i32, i32) {
        let f = |v: f32| { let v = v * FX as f32; (if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32 };
        (f(self.place.x + x * self.place.scale), f(self.place.y + y * self.place.scale))
    }

    /// A polygon in grid units, wound so its signed area is positive.
    fn polygon(&mut self, pts: &[(f32, f32)]) {
        if pts.len() < 3 { return; }
        let mut area = 0.0;
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            area += a.0 * b.1 - b.0 * a.1;
        }
        let n = pts.len();
        let at = |i: usize| if area >= 0.0 { pts[i] } else { pts[n - 1 - i] };
        for i in 0..n {
            let (a, b) = (self.dev(at(i)), self.dev(at((i + 1) % n)));
            self.raster.line(a.0, a.1, b.0, b.1);
        }
    }

    fn disc(&mut self, (cx, cy): (f32, f32), r: f32) {
        let mut p = [(0.0, 0.0); 32];
        for (o, (c, s)) in p.iter_mut().zip(CIRCLE) { *o = (cx + c * r, cy + s * r); }
        self.polygon(&p);
    }

    fn push(&mut self, p: (f32, f32)) {
        if self.n > 0 && self.pts[self.n - 1] == p { return; }
        if self.n < MAX_POINTS { self.pts[self.n] = p; self.n += 1; }
    }

    /// Draws the gathered subpath and forgets it.
    fn flush(&mut self) {
        let n = core::mem::take(&mut self.n);
        if n == 0 { return; }
        let pts = self.pts;
        if self.fill {
            self.polygon(&pts[..n]);
            return;
        }
        let half = self.half;
        let segs = if self.closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = libm::sqrtf(dx * dx + dy * dy);
            if len < 1e-4 { continue; }
            let (nx, ny) = (-dy / len * half, dx / len * half);
            self.polygon(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)]);
        }
        for p in &pts[..n] { self.disc(*p, half); }
    }

    fn last(&self) -> (f32, f32) { if self.n > 0 { self.pts[self.n - 1] } else { (0.0, 0.0) } }

    /// Segments for a curve of this many grid units at this scale: about one per 1.5 device pixels.
    fn steps(&self, len: f32) -> usize { ((len * self.place.scale / 1.5) as usize).clamp(4, 32) }
}

impl Sink for Stroker<'_, '_> {
    fn begin(&mut self, x: f32, y: f32, fill: bool) {
        self.flush();
        self.fill = fill;
        self.closed = false;
        self.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) { self.push((x, y)); }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last();
        let len = (libm::fabsf(x1 - p0.0) + libm::fabsf(y1 - p0.1) + libm::fabsf(x - x1) + libm::fabsf(y - y1)).max(1.0);
        let n = self.steps(len);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.push((u * u * p0.0 + 2.0 * u * t * x1 + t * t * x, u * u * p0.1 + 2.0 * u * t * y1 + t * t * y));
        }
    }
    fn cubic_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last();
        let len = (libm::fabsf(x1 - p0.0) + libm::fabsf(y1 - p0.1) + libm::fabsf(x2 - x1) + libm::fabsf(y2 - y1)
            + libm::fabsf(x - x2) + libm::fabsf(y - y2)).max(1.0);
        let n = self.steps(len);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push((a * p0.0 + b * x1 + c * x2 + d * x, a * p0.1 + b * y1 + c * y2 + d * y));
        }
    }
    fn close(&mut self) {
        if self.n > 1 && self.pts[0] == self.pts[self.n - 1] { self.n -= 1; }
        self.closed = true;
        self.flush();
    }
    fn circle(&mut self, cx: f32, cy: f32, r: f32, filled: bool) {
        self.flush();
        if filled { self.disc((cx, cy), r); return; }
        self.fill = false;
        self.closed = true;
        for (c, s) in CIRCLE { self.push((cx + c * r, cy + s * r)); }
        self.flush();
    }
}

/// Coverage of `data` stroked `width` grid units wide, placed by `place`, into a
/// `w` x `h` mask. `acc` needs `sex_font::raster::scratch_len(w, h)` cells.
#[allow(clippy::too_many_arguments)]
pub fn coverage(data: &str, place: Place, width: f32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
    if out.len() < w * h { return false; }
    let Some(mut raster) = Raster::new(w, h, acc) else { return false };
    let mut s = Stroker { raster: &mut raster, place, half: width / 2.0, pts: [(0.0, 0.0); MAX_POINTS], n: 0, fill: false, closed: false };
    let ok = walk(data, &mut s);
    s.flush();
    raster.finish(&mut out[..w * h]);
    ok
}

// ---- Colour ----------------------------------------------------------------

/// The colours icons are drawn in, `0xRRGGBB`. Must match scripts/brand/icons.py.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Line icons.
    pub ink: u32,
    pub badge_top: u32,
    pub badge_bottom: u32,
    pub rim: u32,
    /// The rose-gold sweep, left to right.
    pub glyph: [u32; 3],
    pub sheet_top: u32,
    pub sheet_bottom: u32,
    pub folder: [u32; 3],
}

impl Palette {
    /// The lips brand (docs/BRAND.md): charcoal badges, the bright rose-gold sweep.
    pub const BRAND: Palette = Palette {
        ink: 0xe2a084,
        badge_top: 0x3d3637, badge_bottom: 0x221e1f, rim: 0xffc4a6,
        glyph: [0xb07a66, 0xe2a084, 0xffcaae],
        sheet_top: 0x423b3c, sheet_bottom: 0x2e2a2b,
        folder: [0x9c6e5e, 0xe2a084, 0xf5aa8d],
    };
    /// The brand palette with line icons in `ink` (a theme's text colour).
    pub const fn with_ink(ink: u32) -> Palette { Palette { ink, ..Palette::BRAND } }
}

fn lerp(a: u32, b: u32, t: u32) -> u32 {
    let ch = |s: u32| { let (x, y) = ((a >> s) & 0xff, (b >> s) & 0xff); ((x * (256 - t) + y * t) >> 8) << s };
    ch(16) | ch(8) | ch(0)
}

/// A three-stop sweep at `t` of 256 (stops at 0, 128 and 256).
fn sweep(stops: [u32; 3], t: i32) -> u32 {
    let t = t.clamp(0, 256) as u32;
    if t < 128 { lerp(stops[0], stops[1], t * 2) } else { lerp(stops[1], stops[2], (t - 128) * 2) }
}

/// `color` at `alpha` (of 255) over the non-premultiplied `0xAARRGGBB` pixel `dst`.
fn over(dst: u32, color: u32, alpha: u32) -> u32 {
    if alpha == 0 { return dst; }
    let da = dst >> 24;
    let oa = alpha + da * (255 - alpha) / 255;
    if oa == 0 { return 0; }
    let ch = |s: u32| {
        let (c, d) = ((color >> s) & 0xff, (dst >> s) & 0xff);
        ((c * alpha * 255 + d * da * (255 - alpha)) / (oa * 255)).min(255) << s
    };
    oa << 24 | ch(16) | ch(8) | ch(0)
}

/// How a layer is coloured across the icon (in grid units).
enum Fill { Solid(u32, u32), Vertical(u32, u32, f32, f32), Sweep([u32; 3], f32, f32) }

fn layer(data: &str, place: Place, width: f32, side: usize, fill: Fill, acc: &mut [i32], mask: &mut [u8], out: &mut [u32]) -> bool {
    if !coverage(data, place, width, side, side, acc, mask) { return false; }
    for y in 0..side {
        for x in 0..side {
            let a = mask[y * side + x] as u32;
            if a == 0 { continue; }
            let gx = (x as f32 + 0.5 - place.x) / place.scale;
            let gy = (y as f32 + 0.5 - place.y) / place.scale;
            let (c, alpha) = match fill {
                Fill::Solid(c, al) => (c, al),
                Fill::Vertical(top, bottom, y0, y1) => (lerp(top, bottom, (((gy - y0) / (y1 - y0)).clamp(0.0, 1.0) * 256.0) as u32), 255),
                Fill::Sweep(stops, x0, x1) => (sweep(stops, ((gx - x0) / (x1 - x0) * 256.0) as i32), 255),
            };
            out[y * side + x] = over(out[y * side + x], c, a * alpha / 255);
        }
    }
    true
}

const BADGE: &str = "F M6.5 1 H17.5 C20.54 1 23 3.46 23 6.5 V17.5 C23 20.54 20.54 23 17.5 23 H6.5 C3.46 23 1 20.54 1 17.5 V6.5 C1 3.46 3.46 1 6.5 1 Z";
const BADGE_RIM: &str = "M6.5 1.25 H17.5 C20.4 1.25 22.75 3.6 22.75 6.5 V17.5 C22.75 20.4 20.4 22.75 17.5 22.75 H6.5 C3.6 22.75 1.25 20.4 1.25 17.5 V6.5 C1.25 3.6 3.6 1.25 6.5 1.25 Z";
const FOLDER: &str = "F M2 6 Q2 4 4 4 H9.5 L11.5 6 H20 Q22 6 22 8 V18 Q22 20 20 20 H4 Q2 20 2 18 Z";
const SHEET: &str = "F M5 2 H15 L20 7 V22 H5 Z";
const SHEET_EDGE: &str = "M5 2 H15 L20 7 V22 H5 Z";
const SHEET_FOLD: &str = "M15 2 V7 H20";
/// The app glyph's share of its badge.
pub const APP_GLYPH_SCALE: f32 = 0.64;

/// Bytes of scratch [`draw`] needs for a `side` pixel icon: (acc cells, mask bytes).
pub const fn scratch(side: usize) -> (usize, usize) { (scratch_len(side, side), side * side) }

/// Draws icon `name` of `kind` as a `side` x `side` image of non-premultiplied
/// `0xAARRGGBB` pixels into `out` (cleared first). `false` for an unknown icon or
/// buffers that are too small.
pub fn draw(kind: Kind, name: &str, side: usize, palette: &Palette, acc: &mut [i32], mask: &mut [u8], out: &mut [u32]) -> bool {
    let Some(data) = find(kind, name) else { return false };
    if out.len() < side * side || mask.len() < side * side { return false; }
    out[..side * side].fill(0);
    let place = Place::square(side);
    // Never thinner than a device pixel and a bit, so small sizes keep their lines.
    let stroke = |units: f32, p: Place| (units * p.scale).max(1.3) / p.scale;
    match kind {
        Kind::Line => layer(data, place, stroke(2.0, place), side, Fill::Solid(palette.ink, 255), acc, mask, out),
        Kind::App => {
            let glyph = place.inner(APP_GLYPH_SCALE);
            layer(BADGE, place, 0.0, side, Fill::Vertical(palette.badge_top, palette.badge_bottom, 1.0, 23.0), acc, mask, out)
                && layer(BADGE_RIM, place, stroke(0.5, place).min(1.0 / place.scale), side, Fill::Solid(palette.rim, 56), acc, mask, out)
                && layer(data, glyph, stroke(2.0, glyph), side, Fill::Sweep(palette.glyph, 2.0, 22.0), acc, mask, out)
        }
        Kind::File if name == "folder" => {
            layer(FOLDER, place, 0.0, side, Fill::Sweep(palette.folder, 3.0, 21.0), acc, mask, out)
                && layer("M2 9.5 H22", place, 1.0, side, Fill::Solid(palette.badge_bottom, 90), acc, mask, out)
        }
        Kind::File => {
            layer(SHEET, place, 0.0, side, Fill::Vertical(palette.sheet_top, palette.sheet_bottom, 3.0, 21.0), acc, mask, out)
                && layer(SHEET_EDGE, place, stroke(0.6, place).min(1.0 / place.scale), side, Fill::Solid(palette.rim, 77), acc, mask, out)
                && layer(SHEET_FOLD, place, stroke(0.6, place).min(1.0 / place.scale), side, Fill::Solid(palette.rim, 128), acc, mask, out)
                && layer(data, place, stroke(1.6, place), side, Fill::Sweep(palette.glyph, 2.0, 22.0), acc, mask, out)
        }
    }
}
