// SPDX-License-Identifier: GPL-3.0-only
//! Draws a [`Screen`] into a program's pixels with Geist Mono, row by row, only the rows
//! that changed. Pixels are 0x00RRGGBB, row-major, `stride` pixels per row.

use crate::{colors, Screen, STRIKE, UNDERLINE};
use sex_font::cache::{GlyphCache, Key};
use sex_font::Font;

/// Kept glyph bitmaps. Large: keep it in a `static`, not on the stack.
pub type Cache = GlyphCache<2048, { 512 * 1024 }>;

pub struct Painter {
    font: Font<'static>,
    /// One cell, in pixels.
    pub cell_w: usize,
    pub cell_h: usize,
    /// Scratch for the rasteriser: one accumulator per coverage byte.
    acc: alloc::vec::Vec<i32>,
    /// The row the cursor was drawn on last time, which must be drawn again without it.
    cursor_row: Option<usize>,
}

/// Cell sizes that keep Geist Mono's proportions: about 0.6 of the height wide.
pub fn cell_for_height(h: usize) -> (usize, usize) { let h = h.clamp(10, 64); ((h * 3 + 2) / 5, h) }

impl Painter {
    pub fn new(cell_w: usize, cell_h: usize) -> Painter {
        let font = Font::new(sex_font::GEIST_MONO).expect("Geist Mono parses");
        Painter { font, cell_w, cell_h, acc: alloc::vec![0; (cell_w + 2) * (cell_h + 2)], cursor_row: None }
    }
    /// How many columns and rows fit `width` by `height` pixels.
    pub fn grid(&self, width: usize, height: usize) -> (usize, usize) { ((width / self.cell_w).max(2), (height / self.cell_h).max(1)) }

    /// Draws the rows `dirty` marks (all of them when `dirty` is empty) and the cursor
    /// when `focused`. Returns how many rows were drawn.
    pub fn draw(&mut self, screen: &Screen, dirty: &[bool], cache: &mut Cache, pixels: &mut [u32], stride: usize, focused: bool) -> usize {
        let (cw, ch) = (self.cell_w, self.cell_h);
        screen.set_cell_pixels(cw, ch);
        let mut drawn = 0;
        let mut painted = alloc::vec![false; screen.rows()];
        let (cx, cy) = screen.cursor();
        let show_cursor = screen.cursor_visible && screen.view_offset() == 0;
        let old_cursor = self.cursor_row.take();
        if show_cursor { self.cursor_row = Some(cy); }
        for row in 0..screen.rows() {
            if !dirty.is_empty() && !dirty.get(row).copied().unwrap_or(true) && !(show_cursor && row == cy) && old_cursor != Some(row) { continue; }
            if (row + 1) * ch > pixels.len() / stride.max(1) { break; }
            drawn += 1;
            painted[row] = true;
            for col in 0..screen.cols() {
                if (col + 1) * cw > stride { break; }
                let cell = screen.visible(col, row);
                let (mut fg, mut bg) = colors(&cell);
                let selected = screen.selected(col, row);
                let cursor_here = show_cursor && row == cy && col == cx;
                if selected { bg = 0x45475A; }
                // A solid block when focused, an outline when not, as most terminals do.
                if cursor_here && focused { core::mem::swap(&mut fg, &mut bg); if fg == bg { fg = crate::DEFAULT_BG; bg = crate::DEFAULT_FG; } }
                let (x0, y0) = (col * cw, row * ch);
                self.glyph(cache, pixels, stride, x0, y0, cell.ch, fg, bg);
                let mut rule = |y: usize, color: u32| { let at = (y0 + y) * stride + x0; pixels[at..at + cw].fill(color); };
                if cell.flags & UNDERLINE != 0 { rule(ch - 2, fg); }
                if cell.flags & STRIKE != 0 { rule(ch / 2, fg); }
                if cursor_here && !focused {
                    rule(0, crate::DEFAULT_FG); rule(ch - 1, crate::DEFAULT_FG);
                    for y in 0..ch { pixels[(y0 + y) * stride + x0] = crate::DEFAULT_FG; pixels[(y0 + y) * stride + x0 + cw - 1] = crate::DEFAULT_FG; }
                }
            }
            // The sliver right of the last column.
            let used = screen.cols() * cw;
            if used < stride { for y in 0..ch { let at = (row * ch + y) * stride; pixels[at + used..at + stride].fill(crate::DEFAULT_BG); } }
        }
        // Inline images over (or under) the text of the rows just drawn.
        crate::images::composite(screen, cw, ch, &painted, pixels, stride);
        drawn
    }
}

impl Painter {
    /// One character cell at pixel `x0, y0`: the glyph in `fg` over `bg`. Editors and
    /// other grids of text use this directly.
    #[allow(clippy::too_many_arguments)]
    pub fn glyph(&mut self, cache: &mut Cache, pixels: &mut [u32], stride: usize, x0: usize, y0: usize, ch: char, fg: u32, bg: u32) {
        let (cw, h) = (self.cell_w, self.cell_h);
        if x0 + cw > stride || (y0 + h) * stride > pixels.len() { return; }
        let key = Key { font: 1, scalar: ch as u32, size: h as u16, phase: 0, w: cw as u8, h: h as u8 };
        let (font, acc) = (&self.font, &mut self.acc);
        let glyph = if ch == ' ' { None } else {
            cache.get(key, |out| { acc.iter_mut().for_each(|a| *a = 0); sex_font::draw_cell(font, ch as u32, cw, h, acc, out) })
        };
        for y in 0..h {
            let line = &mut pixels[(y0 + y) * stride + x0..(y0 + y) * stride + x0 + cw];
            match glyph {
                Some(g) => for (x, p) in line.iter_mut().enumerate() { *p = blend(bg, fg, g[y * cw + x]); },
                None => line.fill(bg),
            }
        }
    }
    /// A run of text from pixel `x0, y0`, one cell per character; returns the x after it.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, cache: &mut Cache, pixels: &mut [u32], stride: usize, mut x0: usize, y0: usize, text: &str, fg: u32, bg: u32) -> usize {
        for c in text.chars() { if x0 + self.cell_w > stride { break; } self.glyph(cache, pixels, stride, x0, y0, c, fg, bg); x0 += self.cell_w; }
        x0
    }
}

fn blend(bg: u32, fg: u32, a: u8) -> u32 {
    if a == 0 { return bg; }
    if a == 255 { return fg; }
    let a = a as u32;
    let mix = |s: u32| (((fg >> s) & 0xFF) * a + ((bg >> s) & 0xFF) * (255 - a)) / 255;
    mix(16) << 16 | mix(8) << 8 | mix(0)
}
