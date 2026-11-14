// SPDX-License-Identifier: GPL-3.0-only
//! Pixels for a sex-app window: filled boxes, pictures and Geist Mono text in grid cells,
//! anti-aliased by sex-font and blended over what is already there. The glyphs of
//! ASCII are drawn once per face and kept; anything else is drawn each time.
use alloc::vec;
use alloc::vec::Vec;
use sex_font::{raster::scratch_len, Font};

pub struct Face {
    pub w: usize,
    pub h: usize,
    font: Font<'static>,
    cache: Vec<u8>,
    ready: [bool; 95],
    acc: Vec<i32>,
    scratch: Vec<u8>,
}

impl Face {
    pub fn new(w: usize, h: usize) -> Face {
        let font = Font::new(sex_font::GEIST_MONO).unwrap_or_else(|| sex_pdx::task_finish(4));
        Face { w, h, font, cache: vec![0; 95 * w * h], ready: [false; 95], acc: vec![0; scratch_len(w, h)], scratch: vec![0; w * h] }
    }
    /// The glyph's coverage, `w * h` bytes, or None for a blank or missing one.
    fn coverage(&mut self, c: char) -> Option<&[u8]> {
        let cell = self.w * self.h;
        let ascii = (c as u32).wrapping_sub(32);
        if c == ' ' { return None; }
        if ascii < 95 {
            let i = ascii as usize;
            if !self.ready[i] {
                self.acc.fill(0);
                let out = &mut self.cache[i * cell..(i + 1) * cell];
                sex_font::draw_cell(&self.font, c as u32, self.w, self.h, &mut self.acc, out);
                self.ready[i] = true;
            }
            return Some(&self.cache[i * cell..(i + 1) * cell]);
        }
        self.acc.fill(0);
        self.scratch.fill(0);
        sex_font::draw_cell(&self.font, c as u32, self.w, self.h, &mut self.acc, &mut self.scratch).then_some(&self.scratch[..])
    }
}

/// Mixes `color` over `under` by coverage `a` (0..=255), per channel.
fn blend(under: u32, color: u32, a: u32) -> u32 {
    let mix = |s: u32| ((((color >> s) & 0xFF) * a + ((under >> s) & 0xFF) * (255 - a)) / 255) << s;
    mix(16) | mix(8) | mix(0)
}

pub struct Canvas<'a> { pub px: &'a mut [u32], pub w: usize, pub h: usize }

impl Canvas<'_> {
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let (x1, y1) = ((x + w).clamp(0, self.w as i32) as usize, (y + h).clamp(0, self.h as i32) as usize);
        for row in y0..y1 { self.px[row * self.w + x0.min(x1)..row * self.w + x1].fill(color); }
    }
    /// Text from (`x`, `y`), its cells' top left, cut at `max_w` pixels wide.
    /// Returns where the next character would go.
    pub fn text(&mut self, face: &mut Face, x: i32, y: i32, text: &str, color: u32, max_w: i32) -> i32 {
        let (fw, fh) = (face.w as i32, face.h as i32);
        let mut at = x;
        for c in text.chars() {
            if at + fw > x + max_w { break; }
            if let Some(cov) = face.coverage(c) {
                for gy in 0..fh {
                    let py = y + gy;
                    if py < 0 || py >= self.h as i32 { continue; }
                    for gx in 0..fw {
                        let px = at + gx;
                        let a = cov[(gy * fw + gx) as usize] as u32;
                        if a == 0 || px < 0 || px >= self.w as i32 { continue; }
                        let p = &mut self.px[py as usize * self.w + px as usize];
                        *p = blend(*p, color, a);
                    }
                }
            }
            at += fw;
        }
        at
    }
}

/// Text cut to `n` characters, on a character boundary.
pub fn clip(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) { Some((i, _)) => &text[..i], None => text }
}

impl Canvas<'_> {
    /// A picture scaled (nearest pixel) to fit the box, centred in it, keeping its shape.
    pub fn picture(&mut self, x: i32, y: i32, w: i32, h: i32, pw: usize, ph: usize, pixels: &[u32]) {
        if pw == 0 || ph == 0 || w <= 0 || h <= 0 { return; }
        // The largest scale, in 1/1024ths, at which the whole picture fits.
        let scale = ((w as i64 * 1024) / pw as i64).min((h as i64 * 1024) / ph as i64).max(1);
        let (dw, dh) = ((pw as i64 * scale / 1024) as i32, (ph as i64 * scale / 1024) as i32);
        let (ox, oy) = (x + (w - dw) / 2, y + (h - dh) / 2);
        for dy in 0..dh {
            let py = oy + dy;
            if py < 0 || py >= self.h as i32 { continue; }
            let sy = ((dy as i64 * 1024 / scale) as usize).min(ph - 1);
            for dx in 0..dw {
                let px = ox + dx;
                if px < 0 || px >= self.w as i32 { continue; }
                let sx = ((dx as i64 * 1024 / scale) as usize).min(pw - 1);
                self.px[py as usize * self.w + px as usize] = pixels[sy * pw + sx] & 0x00FF_FFFF;
            }
        }
    }
}
