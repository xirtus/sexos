// SPDX-License-Identifier: GPL-3.0-only
//! Drawing into a window's pixels: rectangles, rounded shapes, lines, text and icons,
//! all clipped. Coordinates are layout units; the painter multiplies by the theme's
//! scale, so one widget paints crisply at 1x, 1.25x and 2x.
//!
//! Window pixels are `0x00RRGGBB` words (the top byte is kept 0xFF so tiny-skia, which
//! writes premultiplied R,G,B,A bytes, sees an opaque window; colours handed to it have
//! red and blue swapped so it draws straight into the words, as sex-canvas does).
use alloc::vec::Vec;

use tiny_skia::{FillRule, Mask, Paint, PathBuilder, PixmapMut, Stroke, Transform};

use crate::geom::Rect;
use crate::text::{Face, Fonts};
use crate::theme::Color;

pub struct Painter<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
    pub scale: f32,
    /// The clip in device pixels.
    clip: Rect,
    stack: Vec<Rect>,
    mask: Option<(Rect, Mask)>,
}

impl<'a> Painter<'a> {
    /// `pixels` holds `width * height` words, row after row.
    pub fn new(pixels: &'a mut [u32], width: usize, height: usize, scale: f32) -> Painter<'a> {
        let clip = Rect::new(0.0, 0.0, width as f32, height as f32);
        Painter { pixels: &mut pixels[..width * height], width, height, scale, clip, stack: Vec::new(), mask: None }
    }
    pub fn width(&self) -> usize { self.width }
    pub fn height(&self) -> usize { self.height }
    pub fn pixels(&self) -> &[u32] { self.pixels }

    fn dev(&self, r: Rect) -> Rect { r.scale(self.scale) }

    /// Narrows the clip to `r` (layout units) until the matching [`Self::pop_clip`].
    pub fn push_clip(&mut self, r: Rect) {
        self.stack.push(self.clip);
        self.clip = self.clip.intersect(&self.dev(r).round_out());
    }
    /// Narrows the clip to `r` in device pixels.
    pub fn push_clip_device(&mut self, r: Rect) {
        self.stack.push(self.clip);
        self.clip = self.clip.intersect(&r.round_out());
    }
    pub fn pop_clip(&mut self) { if let Some(c) = self.stack.pop() { self.clip = c; } }
    pub fn clip_visible(&self, r: Rect) -> bool { self.clip.intersects(&self.dev(r)) }

    fn clip_ints(&self) -> (i32, i32, i32, i32) {
        (self.clip.x.max(0.0) as i32, self.clip.y.max(0.0) as i32, (self.clip.right() as i32).min(self.width as i32), (self.clip.bottom() as i32).min(self.height as i32))
    }

    #[inline]
    fn blend(&mut self, i: usize, c: Color, cover: u32) {
        let a = c.a() as u32 * cover / 255;
        if a == 0 { return; }
        if a >= 255 { self.pixels[i] = 0xFF00_0000 | (c.0 & 0x00FF_FFFF); return; }
        let d = self.pixels[i];
        let m = |s: u32, d: u32| (s * a + d * (255 - a) + 127) / 255;
        self.pixels[i] = 0xFF00_0000 | m(c.r() as u32, d >> 16 & 0xFF) << 16 | m(c.g() as u32, d >> 8 & 0xFF) << 8 | m(c.b() as u32, d & 0xFF);
    }

    /// Fills whole device pixels of `r`; edges are snapped, so adjacent rectangles meet.
    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        let d = self.dev(r);
        let (cx0, cy0, cx1, cy1) = self.clip_ints();
        let x0 = (libm::roundf(d.x) as i32).max(cx0);
        let y0 = (libm::roundf(d.y) as i32).max(cy0);
        let x1 = (libm::roundf(d.right()) as i32).min(cx1);
        let y1 = (libm::roundf(d.bottom()) as i32).min(cy1);
        for y in y0..y1 {
            let row = y as usize * self.width;
            for x in x0..x1 { self.blend(row + x as usize, c, 255); }
        }
    }

    pub fn clear(&mut self, c: Color) {
        let w = 0xFF00_0000 | (c.0 & 0xFF_FFFF);
        let (x0, y0, x1, y1) = self.clip_ints();
        for y in y0..y1 { self.pixels[y as usize * self.width + x0 as usize..y as usize * self.width + x1 as usize].fill(w); }
    }

    fn skia_paint(c: Color) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color_rgba8(c.b(), c.g(), c.r(), c.a());
        p.anti_alias = true;
        p
    }

    /// Runs `f` with a tiny-skia pixmap over the window and the current clip mask.
    fn skia(&mut self, bounds: Rect, f: impl FnOnce(&mut PixmapMut, Option<&Mask>)) {
        let full = Rect::new(0.0, 0.0, self.width as f32, self.height as f32);
        let needs_mask = self.clip != full && !(bounds.x >= self.clip.x && bounds.y >= self.clip.y && bounds.right() <= self.clip.right() && bounds.bottom() <= self.clip.bottom());
        if needs_mask && self.mask.as_ref().map_or(true, |(r, _)| *r != self.clip) {
            let mut m = match Mask::new(self.width as u32, self.height as u32) { Some(m) => m, None => return };
            if let Some(rect) = tiny_skia::Rect::from_xywh(self.clip.x, self.clip.y, self.clip.w.max(0.0), self.clip.h.max(0.0)) {
                m.fill_path(&PathBuilder::from_rect(rect), FillRule::Winding, false, Transform::identity());
            }
            self.mask = Some((self.clip, m));
        }
        let (w, h) = (self.width as u32, self.height as u32);
        let bytes = unsafe { core::slice::from_raw_parts_mut(self.pixels.as_mut_ptr() as *mut u8, self.pixels.len() * 4) };
        let Some(mut pm) = PixmapMut::from_bytes(bytes, w, h) else { return };
        let mask = if needs_mask { self.mask.as_ref().map(|(_, m)| m) } else { None };
        if self.clip.is_empty() { return; }
        f(&mut pm, mask);
    }

    fn round_path(d: Rect, r: f32) -> Option<tiny_skia::Path> {
        let r = r.min(d.w / 2.0).min(d.h / 2.0).max(0.0);
        let mut pb = PathBuilder::new();
        if r <= 0.01 {
            return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(d.x, d.y, d.w, d.h)?));
        }
        let k = 0.552_284_8 * r;
        let (x0, y0, x1, y1) = (d.x, d.y, d.right(), d.bottom());
        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0);
        pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1);
        pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
        pb.line_to(x0, y0 + r);
        pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
        pb.close();
        pb.finish()
    }

    pub fn fill_round(&mut self, r: Rect, radius: f32, c: Color) {
        if c.a() == 0 { return; }
        let d = self.dev(r);
        if radius <= 0.0 { return self.fill_rect(r, c); }
        let Some(path) = Self::round_path(d, radius * self.scale) else { return };
        self.skia(d, |pm, mask| pm.fill_path(&path, &Self::skia_paint(c), FillRule::Winding, Transform::identity(), mask));
    }

    /// A border of `width` units drawn inside `r`.
    pub fn stroke_round(&mut self, r: Rect, radius: f32, width: f32, c: Color) {
        if c.a() == 0 { return; }
        let wd = width * self.scale;
        let d = self.dev(r).inset(wd / 2.0);
        let Some(path) = Self::round_path(d, (radius * self.scale - wd / 2.0).max(0.0)) else { return };
        let stroke = Stroke { width: wd, ..Stroke::default() };
        self.skia(d.inset(-wd), |pm, mask| pm.stroke_path(&path, &Self::skia_paint(c), &stroke, Transform::identity(), mask));
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, c: Color) {
        let s = self.scale;
        let Some(path) = PathBuilder::from_circle(cx * s, cy * s, radius * s) else { return };
        let b = Rect::new((cx - radius) * s, (cy - radius) * s, 2.0 * radius * s, 2.0 * radius * s);
        self.skia(b, |pm, mask| pm.fill_path(&path, &Self::skia_paint(c), FillRule::Winding, Transform::identity(), mask));
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: Color) {
        let s = self.scale;
        let mut pb = PathBuilder::new();
        pb.move_to(x0 * s, y0 * s);
        pb.line_to(x1 * s, y1 * s);
        let Some(path) = pb.finish() else { return };
        let stroke = Stroke { width: width * s, line_cap: tiny_skia::LineCap::Round, ..Stroke::default() };
        let b = Rect::new(x0.min(x1) * s - width * s, y0.min(y1) * s - width * s, (x1 - x0).abs() * s + 2.0 * width * s, (y1 - y0).abs() * s + 2.0 * width * s);
        self.skia(b, |pm, mask| pm.stroke_path(&path, &Self::skia_paint(c), &stroke, Transform::identity(), mask));
    }

    /// A soft shadow under a rounded rectangle: layered translucent outlines.
    pub fn shadow(&mut self, r: Rect, radius: f32, blur: f32, c: Color) {
        if c.a() == 0 { return; }
        let steps = 4;
        for i in (1..=steps).rev() {
            let grow = blur * i as f32 / steps as f32;
            let a = (c.a() as u32 / (steps as u32 + 1)) as u8;
            self.fill_round(Rect::new(r.x - grow / 2.0, r.y - grow / 4.0, r.w + grow, r.h + grow), radius + grow / 2.0, c.with_alpha(a));
        }
    }

    /// Runs `f` with the tiny-skia pixmap, a transform from layout units and the clip
    /// mask: icons and app drawings use it.
    pub fn skia_draw(&mut self, bounds: Rect, f: impl FnOnce(&mut PixmapMut, Transform, Option<&Mask>)) {
        let s = self.scale;
        let d = self.dev(bounds);
        self.skia(d, |pm, mask| f(pm, Transform::from_scale(s, s), mask));
    }

    /// Draws `text` on one line, its baseline at `baseline` and starting at `x`
    /// (layout units), at `size` units. Returns the advance in layout units.
    pub fn text(&mut self, fonts: &mut Fonts, face: Face, size: f32, x: f32, baseline: f32, text: &str, c: Color) -> f32 {
        let s = self.scale;
        let px = size * s;
        let mut adv = Vec::new();
        fonts.advances(face, px, text, &mut adv);
        let mut pen = x * s;
        let base = libm::roundf(baseline * s) as i32;
        let (cx0, cy0, cx1, cy1) = self.clip_ints();
        for (ch, a) in text.chars().zip(adv.iter()) {
            if pen > cx1 as f32 { break; }
            if ch != ' ' && pen + a >= cx0 as f32 - px {
                let (whole, phase) = Fonts::phase_of(pen);
                if let Some(g) = fonts.glyph(face, ch, px, phase) {
                    let gx = whole + g.left;
                    let gy = base - g.top;
                    for row in 0..g.h as i32 {
                        let y = gy + row;
                        if y < cy0 || y >= cy1 { continue; }
                        let line = &g.coverage[row as usize * g.w..(row as usize + 1) * g.w];
                        for (col, cover) in line.iter().enumerate() {
                            if *cover == 0 { continue; }
                            let x = gx + col as i32;
                            if x < cx0 || x >= cx1 { continue; }
                            let i = y as usize * self.width + x as usize;
                            self.blend(i, c, *cover as u32);
                        }
                    }
                } else {
                    // A box for a character no font has.
                    let h = px * 0.7;
                    let r = Rect::new(pen / s + 1.0, (base as f32 - h) / s, a / s - 2.0, h / s);
                    self.stroke_round(r, 0.0, 1.0, c);
                }
            }
            pen += a;
        }
        (pen / s) - x
    }

    /// Copies an image of `w` by `h` `0x00RRGGBB` pixels into `r`, scaled to fit.
    pub fn image(&mut self, r: Rect, w: usize, h: usize, px: &[u32]) {
        if w == 0 || h == 0 || px.len() < w * h { return; }
        let d = self.dev(r);
        let (cx0, cy0, cx1, cy1) = self.clip_ints();
        let x0 = (d.x as i32).max(cx0);
        let y0 = (d.y as i32).max(cy0);
        let x1 = (d.right() as i32).min(cx1);
        let y1 = (d.bottom() as i32).min(cy1);
        for y in y0..y1 {
            let sy = (((y as f32 - d.y) / d.h) * h as f32) as usize;
            for x in x0..x1 {
                let sx = (((x as f32 - d.x) / d.w) * w as f32) as usize;
                let p = px[sy.min(h - 1) * w + sx.min(w - 1)];
                let i = y as usize * self.width + x as usize;
                let a = p >> 24;
                if a == 0 || a == 0xFF { self.pixels[i] = 0xFF00_0000 | (p & 0xFF_FFFF); } else { self.blend(i, Color(p), 255); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangles_clip_and_blend() {
        let mut px = alloc::vec![0xFF000000u32; 20 * 10];
        let mut p = Painter::new(&mut px, 20, 10, 1.0);
        p.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        p.fill_rect(Rect::new(5.0, 2.0, 10.0, 3.0), Color::hex(0xFF0000));
        p.pop_clip();
        p.fill_rect(Rect::new(0.0, 8.0, 2.0, 1.0), Color::hex(0xFFFFFF).with_alpha(128));
        assert_eq!(px[2 * 20 + 5] & 0xFFFFFF, 0xFF0000);
        assert_eq!(px[2 * 20 + 10] & 0xFFFFFF, 0);
        assert_eq!(px[8 * 20] & 0xFFFFFF, 0x808080);
    }
    #[test]
    fn round_shapes_respect_the_clip_and_scale() {
        let mut px = alloc::vec![0xFF000000u32; 40 * 40];
        let mut p = Painter::new(&mut px, 40, 40, 2.0);
        p.push_clip(Rect::new(0.0, 0.0, 10.0, 20.0));
        p.fill_round(Rect::new(2.0, 2.0, 16.0, 16.0), 4.0, Color::hex(0x00FF00));
        p.pop_clip();
        assert_eq!(px[20 * 40 + 10] & 0xFFFFFF, 0x00FF00);
        assert_eq!(px[20 * 40 + 25] & 0xFFFFFF, 0);
        // The corner is rounded.
        assert_eq!(px[4 * 40 + 4] & 0xFFFFFF, 0);
    }
    #[test]
    fn text_draws_ink_in_its_colour() {
        let mut fonts = Fonts::new();
        let mut px = alloc::vec![0xFFFFFFFFu32; 100 * 30];
        let mut p = Painter::new(&mut px, 100, 30, 1.0);
        let adv = p.text(&mut fonts, Face::Sans, 16.0, 4.0, 20.0, "Weft", Color::hex(0x000000));
        assert!(adv > 25.0);
        let dark = px.iter().filter(|c| **c & 0xFF < 64).count();
        assert!(dark > 30, "{dark}");
    }
}
