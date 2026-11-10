// SPDX-License-Identifier: GPL-3.0-only
//! Pages on screen: measures text for `sex-html` layout with Geist (Sans for text,
//! Mono for code) and paints a laid-out [`Page`] into a 0x00RRGGBB pixel buffer.
//!
//! Everything is integer arithmetic: glyphs come from `sex-font`'s integer rasterizer
//! through its glyph cache, boxes and rounded corners are filled here with 4x4
//! supersampled edges, pictures are scaled nearest-neighbour from `sex-image` output.
//! Text widths used for layout and the pen positions used for painting come from the
//! same function, so runs land exactly where layout put them.
#![no_std]
extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use sex_font::cache::{GlyphCache, Key};
use sex_font::{Font, GlyphId};
use sex_html::layout::{Bullet, FontKey, Host, Item, Page};
pub use sex_html;

pub mod canvas;
pub mod svg;
pub use sex_image::Image;

/// Glyph bitmaps kept between paints.
type Cache = GlyphCache<4096, { 768 * 1024 }>;
const SANS: u8 = 0;
const MONO: u8 = 1;
/// Largest text drawn at its own size; bigger text is drawn at this size.
const MAX_SIZE: i32 = 96;

pub struct Fonts {
    sans: Font<'static>,
    mono: Font<'static>,
    cache: Box<Cache>,
    acc: Vec<i32>,
    /// Decoded pictures by address, for layout sizes and painting.
    /// Decoded pictures by `src` (`canvas:<node>` for canvases), shared with scripts.
    pub images: Vec<(String, alloc::rc::Rc<Image>)>,
    /// SVG pictures by `src`: their text and own size, redrawn at the size laid out.
    pub svgs: Vec<(String, alloc::rc::Rc<str>, (i32, i32))>,
    /// Animated GIF and WebP pictures; each frame replaces its entry in `images`.
    pub moving: Vec<Moving>,
    /// Frames of animated pictures shown since the page loaded.
    pub frames_shown: u64,
}

/// Most animated pictures a page plays at once; the rest show their first frame.
const MAX_MOVING: usize = 16;

/// An animated picture: its frames and the canvas they draw on.
pub struct Moving {
    src: String,
    animation: sex_image::Animation<alloc::rc::Rc<[u8]>>,
    canvas: Vec<u32>,
    fit: (usize, usize),
    due: u64,
    shown: u64,
}

impl Fonts {
    pub fn new() -> Option<Self> {
        let sans = Font::new(sex_font::GEIST_SANS)?;
        let mono = Font::new(sex_font::GEIST_MONO)?;
        // The cache is all zeroes when empty; allocate it zeroed rather than build it on the stack.
        let cache = unsafe {
            let layout = core::alloc::Layout::new::<Cache>();
            let ptr = alloc::alloc::alloc_zeroed(layout) as *mut Cache;
            if ptr.is_null() { return None; }
            Box::from_raw(ptr)
        };
        Some(Fonts { sans, mono, cache, acc: alloc::vec![0; sex_font::raster::scratch_len(256, 256)], images: Vec::new(), svgs: Vec::new(), moving: Vec::new(), frames_shown: 0 })
    }

    fn face(&self, mono: bool) -> &Font<'static> { if mono { &self.mono } else { &self.sans } }


    fn draw_size(size: i32) -> i32 { size.clamp(1, MAX_SIZE) }

    fn advances(&self, text: &str, font: FontKey, each: impl FnMut(char, Option<(u8, GlyphId)>, i32)) -> i32 {
        Faces { sans: &self.sans, mono: &self.mono }.advances(text, font, each)
    }
}

/// The two faces, for measuring without a glyph cache.
struct Faces<'a> { sans: &'a Font<'static>, mono: &'a Font<'static> }

/// Text width in pixels, as layout measures it, without a [`Fonts`].
pub fn measure(text: &str, font: FontKey) -> i32 {
    let (Some(sans), Some(mono)) = (Font::new(sex_font::GEIST_SANS), Font::new(sex_font::GEIST_MONO)) else { return 0 };
    let faces = Faces { sans: &sans, mono: &mono };
    let mut any = false;
    let pen = faces.advances(text, font, |_, _, _| any = true);
    let bold = if font.bold && any { font.size * 256 / 24 } else { 0 };
    (pen - bold + 255) / 256
}

impl Faces<'_> {
    fn face(&self, mono: bool) -> &Font<'static> { if mono { self.mono } else { self.sans } }

    /// The glyph for `c`: from the requested face, else the other one. Spaces that
    /// should not show are mapped to the space glyph.
    fn glyph(&self, c: char, mono: bool) -> Option<(u8, GlyphId)> {
        let c = match c { '\u{A0}' | '\u{2002}' | '\u{2003}' | '\u{2009}' => ' ', '\u{2212}' => '-', c => c };
        let first = if mono { MONO } else { SANS };
        let second = if mono { SANS } else { MONO };
        for f in [first, second] {
            if let Some(g) = self.face(f == MONO).glyph(c as u32) { return Some((f, g)); }
        }
        None
    }

    /// Pen advances, in 1/256 pixel, for each character of `text`: what layout measures
    /// and where painting puts glyphs.
    fn advances(&self, text: &str, font: FontKey, mut each: impl FnMut(char, Option<(u8, GlyphId)>, i32)) -> i32 {
        let size = (Fonts::draw_size(font.size) * 64) as u32;
        let scale = |v: i32| if font.size > MAX_SIZE { v * font.size / MAX_SIZE } else { v };
        let bold = if font.bold { font.size * 256 / 24 } else { 0 };
        let mut pen = 0;
        let mut prev: Option<(u8, GlyphId)> = None;
        for c in text.chars() {
            if c == '\u{AD}' || c == '\u{200B}' || c == '\u{200C}' || c == '\u{200D}' || c == '\u{FE0F}' { continue; }
            let g = self.glyph(c, font.mono);
            let adv = match g {
                Some((f, id)) => {
                    let face = self.face(f == MONO);
                    let kern = match prev { Some((pf, pid)) if pf == f => face.kerning(pid, id, size), _ => 0 };
                    pen += scale(kern);
                    scale(face.advance(id, size)) + bold
                }
                None => font.size * 256 * 6 / 10,
            };
            each(c, g, pen);
            pen += adv;
            prev = g;
        }
        pen
    }
}

impl Host for Fonts {
    fn text_width(&self, text: &str, font: FontKey) -> i32 {
        // Synthetic bold widens each glyph's advance; the last one needs no room after it.
        let mut any = false;
        let pen = self.advances(text, font, |_, _, _| any = true);
        let bold = if font.bold && any { font.size * 256 / 24 } else { 0 };
        (pen - bold + 255) / 256
    }
    fn metrics(&self, font: FontKey) -> (i32, i32) {
        let size = (Self::draw_size(font.size) * 64) as u32;
        let face = self.face(font.mono);
        let up = |v: i32| (v * font.size.max(1) / Self::draw_size(font.size) + 255) / 256;
        (up(face.ascender(size)), up(-face.descender(size)))
    }
    fn image_size(&self, src: &str) -> Option<(i32, i32)> {
        if let Some((_, _, size)) = self.svgs.iter().find(|(s, _, _)| s == src) { return Some(*size); }
        self.images.iter().find(|(s, _)| s == src).map(|(_, i)| (i.width as i32, i.height as i32))
    }
}

/// A clipped view onto a pixel buffer, scrolled by `scroll` page pixels.
pub struct Target<'a> {
    pub pixels: &'a mut [u32],
    pub width: i32,
    pub height: i32,
    /// Page y shown at the target's top row.
    pub scroll: i32,
    /// Words are premultiplied with alpha (a canvas), not opaque screen pixels.
    pub premul: bool,
    /// Target row where the page starts (room for a toolbar above).
    pub top: i32,
    /// Page rectangle (x, y, width, height) drawing is limited to, if any.
    pub clip: Option<[i32; 4]>,
}

/// Source-over onto a premultiplied word, alpha included (canvas surfaces).
fn blend_premul(dst: u32, src: u32, alpha: u32) -> u32 {
    if alpha == 0 { return dst; }
    let src = (src & 0x00FF_FFFF) | 0xFF00_0000;
    let mix = |s: u32| ((((src >> s) & 0xFF) * alpha + ((dst >> s) & 0xFF) * (255 - alpha) + 127) / 255) << s;
    mix(24) | mix(16) | mix(8) | mix(0)
}

fn blend(dst: u32, src: u32, alpha: u32) -> u32 {
    if alpha >= 255 { return src & 0x00FF_FFFF; }
    if alpha == 0 { return dst; }
    let mix = |s: u32| ((((src >> s) & 0xFF) * alpha + ((dst >> s) & 0xFF) * (255 - alpha) + 127) / 255) << s;
    mix(16) | mix(8) | mix(0)
}

impl Target<'_> {
    /// Screen pixels drawing may touch: x0, y0, x1, y1.
    fn bounds(&self) -> (i32, i32, i32, i32) {
        let (mut x0, mut y0, mut x1, mut y1) = (0, self.top, self.width, self.height);
        if let Some([cx, cy, cw, ch]) = self.clip {
            let cy = cy - self.scroll + self.top;
            x0 = x0.max(cx); y0 = y0.max(cy); x1 = x1.min(cx + cw); y1 = y1.min(cy + ch);
        }
        (x0, y0, x1, y1)
    }

    /// Fills a page rectangle, rounding its corners by `radius`, at `color`'s alpha.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32, radius: i32) {
        let a = color >> 24;
        if a == 0 || w <= 0 || h <= 0 { return; }
        let y = y - self.scroll + self.top;
        let (bx0, by0, bx1, by1) = self.bounds();
        let (x0, y0) = (x.max(bx0), y.max(by0));
        let (x1, y1) = ((x + w).min(bx1), (y + h).min(by1));
        if x0 >= x1 || y0 >= y1 { return; }
        let r = radius.min(w / 2).min(h / 2).max(0);
        for py in y0..y1 {
            let row = (py * self.width) as usize;
            let dy = if py < y + r { y + r - py } else if py >= y + h - r { py - (y + h - r) + 1 } else { 0 };
            for px in x0..x1 {
                let dx = if px < x + r { x + r - px } else if px >= x + w - r { px - (x + w - r) + 1 } else { 0 };
                let cover = if dx > 0 && dy > 0 { corner_cover(dx, dy, r) } else { 16 };
                if cover == 0 { continue; }
                let p = &mut self.pixels[row + px as usize];
                *p = if self.premul { blend_premul(*p, color, a * cover / 16) } else { blend(*p, color, a * cover / 16) };
            }
        }
    }

    /// Blends a coverage bitmap. `smear` thickens strokes (synthetic bold); `slant`
    /// is the bitmap row of the baseline when rows above it lean right (synthetic italic).
    fn coverage(&mut self, x: i32, y: i32, w: usize, h: usize, bits: &[u8], color: u32, smear: i32, slant: Option<i32>) {
        let a = color >> 24;
        let (bx0, by0, bx1, by1) = self.bounds();
        for gy in 0..h as i32 {
            let py = y + gy;
            let x = x + slant.map_or(0, |base| (((base - gy) * 13) >> 6) - ((base * 13) >> 7));
            if py < by0 || py >= by1 { continue; }
            let row = (py * self.width) as usize;
            for gx in 0..w as i32 + smear {
                let px = x + gx;
                if px < bx0 || px >= bx1 { continue; }
                let mut c = 0u32;
                for s in 0..=smear {
                    let sx = gx - s;
                    if sx >= 0 && sx < w as i32 { c = c.max(bits[gy as usize * w + sx as usize] as u32); }
                }
                if c == 0 { continue; }
                let p = &mut self.pixels[row + px as usize];
                *p = if self.premul { blend_premul(*p, color, c * a / 255) } else { blend(*p, color, c * a / 255) };
            }
        }
    }
}

/// Sixteenths of pixel (`dx`, `dy`) covered by a quarter circle of radius `r`;
/// `dx`/`dy` count pixels from the corner's centre outwards, starting at 1.
fn corner_cover(dx: i32, dy: i32, r: i32) -> u32 {
    let mut n = 0;
    let r2 = (r * 8) * (r * 8);
    for sy in 0..4 {
        for sx in 0..4 {
            // Sample centres in 1/8 pixel, measured from the circle's centre.
            let fx = (dx - 1) * 8 + sx * 2 + 1;
            let fy = (dy - 1) * 8 + sy * 2 + 1;
            if fx * fx + fy * fy <= r2 { n += 1; }
        }
    }
    n
}

/// Paints `page` into `target`. `focus` outlines that link.
pub fn paint(page: &Page, fonts: &mut Fonts, target: &mut Target, focus: Option<usize>) {
    let bg = page.background;
    for y in target.top.max(0)..target.height {
        let row = (y * target.width) as usize;
        target.pixels[row..row + target.width as usize].fill(bg & 0x00FF_FFFF);
    }
    let top = target.scroll;
    let bottom = target.scroll + target.height - target.top;
    let outer = target.clip;
    for (i, item) in page.items.iter().enumerate() {
        let (a, b) = item.vertical_span();
        if b <= top || a >= bottom { continue; }
        target.clip = page.clip_of(i).or(outer);
        match item {
            Item::Rect { x, y, w, h, color, radius } => target.fill(*x, *y, *w, *h, *color, *radius),
            Item::Bullet { x, y, size, color, kind } => match kind {
                Bullet::Disc => target.fill(*x, *y, *size, *size, *color, *size / 2),
                Bullet::Square => target.fill(*x, *y, *size, *size, *color, 0),
                Bullet::Circle => {
                    target.fill(*x, *y, *size, *size, *color, *size / 2);
                    let t = (*size / 4).max(1);
                    target.fill(*x + t, *y + t, *size - 2 * t, *size - 2 * t, bg, (*size - 2 * t) / 2);
                }
            },
            Item::Image { x, y, w, h, src, alt } => image(target, fonts, *x, *y, *w, *h, src, alt),
            Item::Text { x, baseline, font, color, text, underline, strike } => {
                text_run(target, fonts, *x, *baseline, *font, *color, text);
                let width = fonts.text_width(text, *font);
                let thick = (font.size / 14).max(1);
                if *underline { target.fill(*x, *baseline + (font.size / 8).max(1), width, thick, *color, 0); }
                if *strike { target.fill(*x, *baseline - font.size * 3 / 10, width, thick, *color, 0); }
            }
        }
    }
    target.clip = outer;
    if let Some(l) = focus.and_then(|f| page.links.get(f)) {
        let ring = 0xFF0B_57D0;
        target.fill(l.x - 2, l.y - 1, l.w + 4, 2, ring, 0);
        target.fill(l.x - 2, l.y + l.h - 1, l.w + 4, 2, ring, 0);
        target.fill(l.x - 2, l.y - 1, 2, l.h + 2, ring, 0);
        target.fill(l.x + l.w, l.y - 1, 2, l.h + 2, ring, 0);
    }
}

/// Draws only the pictures named in `changed` (canvases that drew) over the last frame,
/// when each is opaque and so covers what was under it. False when that can't stand in
/// for a full `frame`.
pub fn repaint_pictures(page: &Page, fonts: &mut Fonts, pixels: &mut [u32], width: i32, height: i32, scroll: i32, changed: &[String]) -> bool {
    let mut t = Target { pixels, width, height, scroll, top: BAR, premul: false, clip: None };
    let (top, bottom) = (scroll, scroll + height - BAR);
    let mut items: Vec<(i32, i32, i32, i32, &str, Option<[i32; 4]>)> = Vec::new();
    for (i, item) in page.items.iter().enumerate() {
        let Item::Image { x, y, w, h, src, .. } = item else { continue };
        if !changed.iter().any(|c| c == src) || y + h <= top || *y >= bottom { continue; }
        // Anything painted later over the picture (a block pulled up by a negative
        // margin, say) would be lost: draw the whole frame then.
        let over = page.items[i + 1..].iter().any(|later| {
            let (a, b) = later.vertical_span();
            if b <= *y || a >= y + h { return false; }
            match later {
                Item::Rect { x: lx, w: lw, .. } | Item::Image { x: lx, w: lw, .. } => *lx < x + w && lx + lw > *x,
                Item::Bullet { x: lx, size, .. } => *lx < x + w && lx + size > *x,
                // A run's width isn't kept: any text starting left of the picture's
                // right edge may reach it.
                Item::Text { x: lx, .. } => *lx < x + w,
            }
        });
        if over { return false; }
        items.push((*x, *y, *w, *h, src.as_str(), page.clip_of(i)));
    }
    let opaque = |src: &str| fonts.images.iter().find(|(s, _)| s == src).is_some_and(|(_, img)| img.pixels.iter().all(|p| p >> 24 == 255));
    if !items.iter().all(|i| opaque(i.4)) { return false; }
    for (x, y, w, h, src, clip) in items { t.clip = clip; image(&mut t, fonts, x, y, w, h, src, ""); }
    true
}

fn image(target: &mut Target, fonts: &mut Fonts, x: i32, y: i32, w: i32, h: i32, src: &str, alt: &str) {
    let Some((_, img)) = fonts.images.iter().find(|(s, _)| s == src) else {
        if src.starts_with("canvas:") { return; }
        // A video with no picture yet is black, as in other browsers.
        if src.starts_with("video:") { target.fill(x, y, w, h, 0xFF00_0000, 0); return; }
        target.fill(x, y, w, h, 0xFFF1_F3F4, 0);
        target.fill(x, y, w, 1, 0xFFC4_C7C5, 0);
        target.fill(x, y + h - 1, w, 1, 0xFFC4_C7C5, 0);
        target.fill(x, y, 1, h, 0xFFC4_C7C5, 0);
        target.fill(x + w - 1, y, 1, h, 0xFFC4_C7C5, 0);
        if !alt.is_empty() && h > 16 {
            let font = FontKey { size: 13, bold: false, italic: false, mono: false };
            text_run(target, fonts, x + 4, y + 15, font, 0xFF5F_6368, alt);
        }
        return;
    };
    let (iw, ih) = (img.width as i64, img.height as i64);
    if iw == 0 || ih == 0 || w <= 0 || h <= 0 { return; }
    let sy0 = y - target.scroll + target.top;
    let (bx0, by0, bx1, by1) = target.bounds();
    for ty in sy0.max(by0)..(sy0 + h).min(by1) {
        let srow = ((ty - sy0) as i64 * ih / h as i64) as usize * img.width;
        let row = (ty * target.width) as usize;
        for tx in x.max(bx0)..(x + w).min(bx1) {
            let s = img.pixels[srow + ((tx - x) as i64 * iw / w as i64) as usize];
            let p = &mut target.pixels[row + tx as usize];
            *p = blend(*p, s, s >> 24);
        }
    }
}

/// Draws `text` with its pen starting at page (`x`, `baseline`).
pub fn text_run(target: &mut Target, fonts: &mut Fonts, x: i32, baseline: i32, font: FontKey, color: u32, text: &str) {
    let screen_base = baseline - target.scroll + target.top;
    let size = Fonts::draw_size(font.size);
    if screen_base + font.size < target.top || screen_base - font.size * 2 > target.height { return; }
    let pad = 2 + size / 6;
    let (asc, desc) = fonts.metrics(FontKey { size, ..font });
    let h = (asc + desc + 2 * pad).min(255) as usize;
    let smear = if font.bold { if font.size >= 28 { 2 } else { 1 } } else { 0 };
    let mut glyphs: Vec<(u8, GlyphId, char, i32)> = Vec::new();
    fonts.advances(text, font, |c, g, pen| if let Some((f, id)) = g { if !c.is_whitespace() { glyphs.push((f, id, c, pen)); } });
    let x256 = x * 256;
    for (f, id, c, pen) in glyphs {
        let at = x256 + pen;
        let phase = ((at & 255) / 64) as u8;
        let face = fonts.face(f == MONO);
        let size64 = (size * 64) as u32;
        let adv = (face.advance(id, size64) + 255) / 256;
        let w = (adv + 2 * pad).clamp(1, 255) as usize;
        let key = Key { font: f, scalar: c as u32, size: size as u16, phase, w: w as u8, h: h as u8 };
        let (sans, mono, acc) = (&fonts.sans, &fonts.mono, &mut fonts.acc);
        let face = if f == MONO { mono } else { sans };
        let bits = fonts.cache.get(key, |out| {
            face.draw(id, size64, pad * 256 + phase as i32 * 64, (pad + asc) * 256, w, h, acc, out)
        });
        let Some(bits) = bits else { continue };
        target.coverage((at >> 8) - pad, screen_base - asc - pad, w, h, bits, color, smear, font.italic.then_some(asc + pad));
    }
}

impl Fonts {
    /// Decodes the picture `src` from `bytes` (fitted within `max_w` x `max_h`) and keeps it
    /// for layout and painting.
    pub fn add_picture(&mut self, src: &str, bytes: &[u8], max_w: usize, max_h: usize) {
        let Some(img) = decode_image(bytes, max_w, max_h) else { return };
        if svg::is_svg(bytes) {
            if let Ok(text) = core::str::from_utf8(bytes) { self.svgs.push((String::from(src), alloc::rc::Rc::from(text), (img.width as i32, img.height as i32))); }
        }
        self.images.push((String::from(src), alloc::rc::Rc::new(img)));
        if self.moving.len() < MAX_MOVING && !svg::is_svg(bytes) {
            let Ok(animation) = sex_image::Animation::open(alloc::rc::Rc::from(bytes), &PICTURE_LIMITS) else { return };
            if animation.frames() < 2 { return; }
            let count = animation.width * animation.height;
            let mut canvas = Vec::new();
            if canvas.try_reserve_exact(count).is_err() { return; }
            canvas.resize(count, 0);
            self.moving.push(Moving { src: String::from(src), animation, canvas, fit: (max_w, max_h), due: 0, shown: 0 });
        }
    }

    /// Shows each animated picture's next frame when it is due, at `now` (ms).
    /// Returns whether any picture changed, and when the next frame is due.
    pub fn step_pictures(&mut self, now: u64) -> (bool, Option<u64>) {
        let (mut changed, mut next) = (false, None::<u64>);
        let Fonts { moving, images, frames_shown, .. } = self;
        moving.retain_mut(|m| {
            if now >= m.due {
                let frames = m.animation.frames() as u64;
                if m.animation.loops().is_some_and(|loops| m.shown >= frames * (loops as u64 + 1)) { return false; }
                let Ok(delay) = m.animation.next_into(&mut m.canvas) else { return false };
                let frame = Image { width: m.animation.width, height: m.animation.height, pixels: m.canvas.clone() };
                let Ok(frame) = frame.fit_within(m.fit.0, m.fit.1) else { return false };
                if let Some(slot) = images.iter_mut().find(|(s, _)| *s == m.src) { slot.1 = alloc::rc::Rc::new(frame); }
                m.shown += 1;
                *frames_shown += 1;
                // Browsers hold each frame at least 20 ms.
                m.due = now + (delay as u64).max(20);
                changed = true;
            }
            next = Some(next.map_or(m.due, |n| n.min(m.due)));
            true
        });
        (changed, next)
    }
}

/// Decodes a picture for a page, shrunk to at most `max_w` x `max_h`.
pub fn decode_image(bytes: &[u8], max_w: usize, max_h: usize) -> Option<Image> {
    if svg::is_svg(bytes) {
        return svg::decode(core::str::from_utf8(bytes).ok()?, max_w, max_h);
    }
    if sex_image::sniff(bytes).is_none() && is_avif(bytes) {
        // Safety: set once at start, before any page loads; programs are single-threaded here.
        let decode = unsafe { AVIF_DECODER }?;
        return decode(bytes, max_w, max_h);
    }
    sex_image::decode(bytes, &PICTURE_LIMITS).ok()?.fit_within(max_w, max_h).ok()
}

/// Decodes an AVIF picture fitted within a box. sex-image does not read AV1, so a
/// program that links a video decoder (Web, through sex-media) hands one in.
pub type AvifDecoder = fn(&[u8], usize, usize) -> Option<Image>;
static mut AVIF_DECODER: Option<AvifDecoder> = None;

/// Sets the decoder [`decode_image`] uses for AVIF pictures.
pub fn set_avif_decoder(decode: AvifDecoder) {
    unsafe { AVIF_DECODER = Some(decode) };
}

/// Whether `bytes` start like an AVIF file: an ISO `ftyp` box whose major or a
/// compatible brand is `avif` (a still picture) or `avis` (a picture sequence).
pub fn is_avif(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" { return false; }
    let size = (u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize).clamp(16, bytes.len().min(256));
    let brand = |b: &[u8]| b == b"avif" || b == b"avis";
    brand(&bytes[8..12]) || bytes[16..size].chunks_exact(4).any(brand)
}

/// What a page's picture may be.
const PICTURE_LIMITS: sex_image::Limits = sex_image::Limits { max_width: 4096, max_height: 4096, max_pixels: 2_000_000, max_input_bytes: 4 << 20 };

/// Height of the bar above the page.
pub const BAR: i32 = 30;

/// The whole window: a bar showing `bar_text` (the address, or what is being typed,
/// with a caret when `editing`), then `page` scrolled by `scroll`.
pub fn frame(page: &Page, fonts: &mut Fonts, pixels: &mut [u32], width: i32, height: i32, scroll: i32, focus: Option<usize>, bar_text: &str, editing: bool) {
    let mut t = Target { pixels, width, height, scroll, top: BAR, premul: false, clip: None };
    paint(page, fonts, &mut t, focus);
    let mut bar = Target { pixels: t.pixels, width, height: BAR, scroll: 0, top: 0, premul: false, clip: None };
    bar.fill(0, 0, width, BAR, 0xFF0D_1420, 0);
    bar.fill(6, 4, width - 12, BAR - 8, if editing { 0xFFFF_FFFF } else { 0xFF22_2C3A }, 8);
    let font = FontKey { size: 13, bold: false, italic: false, mono: false };
    let color = if editing { 0xFF1F_1F1F } else { 0xFFD7_E3F0 };
    // Keep the end of a long address in view while typing.
    let room = width - 40;
    let mut shown = bar_text;
    while fonts.text_width(shown, font) > room && !shown.is_empty() {
        let cut = shown.char_indices().nth(1).map_or(shown.len(), |(i, _)| i);
        shown = if editing { &shown[cut..] } else { &shown[..shown.char_indices().last().map_or(0, |(i, _)| i)] };
    }
    text_run(&mut bar, fonts, 16, 19, font, color, shown);
    if editing {
        let x = 16 + fonts.text_width(shown, font) + 1;
        bar.fill(x, 8, 1, BAR - 16, 0xFF0B_57D0, 0);
    }
}
