// SPDX-License-Identifier: GPL-3.0-only
//! What the office apps share until they move onto egui (workstream M11): a [`Canvas`]
//! over a window's 0x00RRGGBB pixels and a [`Pen`] that draws text with sex-font,
//! keeping every glyph it has drawn so a redraw of a full grid costs only copies.
//!
//! No heap beyond `alloc`, no floating point, no authority: it runs in std and no_std
//! programs and in host tests alike.
#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use sex_font::Font;

/// A rectangle of pixels, rows `stride` apart.
pub struct Canvas<'a> {
    pub px: &'a mut [u32],
    pub stride: usize,
    pub width: usize,
    pub height: usize,
}

impl<'a> Canvas<'a> {
    pub fn new(px: &'a mut [u32], width: usize, height: usize) -> Self {
        let height = height.min(px.len() / width.max(1));
        Canvas { px, stride: width, width, height }
    }
    /// Fills the part of (`x`, `y`, `w`, `h`) inside the canvas.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let x1 = (x.saturating_add(w).max(0) as usize).min(self.width);
        let y1 = (y.saturating_add(h).max(0) as usize).min(self.height);
        if x0 >= x1 { return; }
        for row in y0..y1 { self.px[row * self.stride + x0..row * self.stride + x1].fill(color); }
    }
    /// A one-pixel outline.
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.fill(x, y, w, 1, color);
        self.fill(x, y + h - 1, w, 1, color);
        self.fill(x, y, 1, h, color);
        self.fill(x + w - 1, y, 1, h, color);
    }
    /// Mixes `color` over the pixel at (`x`, `y`) by `alpha` (0–255).
    fn blend(&mut self, x: i32, y: i32, color: u32, alpha: u32) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height { return; }
        let d = &mut self.px[y as usize * self.stride + x as usize];
        *d = mix(*d, color, alpha);
    }
}

/// `over` on `under` by `alpha` out of 255, per channel.
pub fn mix(under: u32, over: u32, alpha: u32) -> u32 {
    let ch = |s: u32| {
        let (u, o) = ((under >> s) & 0xff, (over >> s) & 0xff);
        ((o * alpha + u * (255 - alpha) + 127) / 255) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// One glyph's coverage, drawn once at a whole-pixel origin.
struct Glyph { left: i32, top: i32, w: usize, h: usize, advance: i32, cover: Vec<u8> }

/// Where text goes: which font, at which pixel size, with drawn glyphs kept.
pub struct Pen {
    font: Font<'static>,
    /// Pixel size of the em.
    pub size: u32,
    ascent: i32,
    glyphs: BTreeMap<char, Glyph>,
    acc: Vec<i32>,
}

impl Pen {
    /// Geist Sans at `size` pixels per em.
    pub fn sans(size: u32) -> Self { Self::new(sex_font::GEIST_SANS, size) }
    /// Geist Mono at `size` pixels per em.
    pub fn mono(size: u32) -> Self { Self::new(sex_font::GEIST_MONO, size) }
    pub fn new(data: &'static [u8], size: u32) -> Self {
        let font = Font::new(data).expect("bundled font parses");
        let ascent = (font.ascender(size * 64) + 255) / 256;
        Pen { font, size, ascent, glyphs: BTreeMap::new(), acc: Vec::new() }
    }
    /// The height of a line of text, in pixels.
    pub fn line_height(&self) -> i32 { (self.size as i32 * 5 + 3) / 4 + 1 }

    fn glyph(&mut self, c: char) -> &Glyph {
        if !self.glyphs.contains_key(&c) {
            let size = self.size * 64;
            let id = self.font.glyph(c as u32).or_else(|| self.font.glyph('?' as u32));
            let advance = id.map_or(0, |g| self.font.advance(g, size));
            // A box from one em above the baseline to half an em below, with a margin.
            let s = self.size as usize;
            let (w, h) = (s * 2 + 4, s * 2);
            let (left, top) = (-2, -(self.ascent + 2));
            let mut cover = vec![0u8; w * h];
            if let Some(g) = id.filter(|_| !c.is_whitespace()) {
                self.acc.resize(sex_font::raster::scratch_len(w, h), 0);
                self.font.draw(g, size, -left * 256, -top * 256, w, h, &mut self.acc, &mut cover);
            }
            self.glyphs.insert(c, Glyph { left, top, w, h, advance, cover });
        }
        &self.glyphs[&c]
    }

    /// How wide `text` is, in pixels.
    pub fn measure(&mut self, text: &str) -> i32 {
        text.chars().map(|c| self.glyph(c).advance).sum::<i32>() / 256
    }

    /// Draws `text` with its top-left at (`x`, `y`), clipped to `clip` (x, width) when
    /// given. Returns the pen's end x.
    pub fn text(&mut self, canvas: &mut Canvas, x: i32, y: i32, text: &str, color: u32, clip: Option<(i32, i32)>) -> i32 {
        let (cx0, cx1) = clip.map_or((i32::MIN, i32::MAX), |(cx, cw)| (cx, cx + cw));
        let baseline = y + self.ascent;
        let mut pen = x * 256;
        for c in text.chars() {
            let at = pen / 256;
            let g = self.glyph(c);
            let advance = g.advance;
            if at >= cx1 { break; }
            for gy in 0..g.h {
                for gx in 0..g.w {
                    let a = g.cover[gy * g.w + gx] as u32;
                    let px = at + g.left + gx as i32;
                    if a == 0 || px < cx0 || px >= cx1 { continue; }
                    let py = baseline + g.top + gy as i32;
                    // Re-borrow: `g` lives in `self`, the canvas does not.
                    canvas.blend(px, py, color, a);
                }
            }
            pen += advance;
        }
        pen / 256
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_leaves_ink_inside_its_clip_only() {
        let mut px = vec![0u32; 200 * 40];
        let mut canvas = Canvas::new(&mut px, 200, 40);
        let mut pen = Pen::sans(14);
        let w = pen.measure("Gingham 42");
        assert!(w > 40 && w < 140, "width {w}");
        pen.text(&mut canvas, 10, 10, "Gingham 42", 0xffffff, Some((10, 50)));
        let inked: Vec<usize> = (0..200).filter(|x| (0..40).any(|y| px[y * 200 + x] != 0)).collect();
        assert!(!inked.is_empty());
        assert!(inked.iter().all(|x| (10..60).contains(x)), "{inked:?}");
    }

    #[test]
    fn fill_clips_to_the_canvas() {
        let mut px = vec![0u32; 10 * 10];
        let mut canvas = Canvas::new(&mut px, 10, 10);
        canvas.fill(-5, 8, 100, 100, 7);
        assert_eq!(px.iter().filter(|p| **p == 7).count(), 20);
    }

    #[test]
    fn mix_ends() {
        assert_eq!(mix(0x000000, 0xffffff, 255), 0xffffff);
        assert_eq!(mix(0x123456, 0xffffff, 0), 0x123456);
    }
}
