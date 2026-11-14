// SPDX-License-Identifier: GPL-3.0-only
//! Draws a [`Screen`] into a program's pixels with Geist Mono, row by row, only the rows
//! that changed. Pixels are 0x00RRGGBB, row-major, `stride` pixels per row.

use crate::hints::Hint;
use crate::{colors, Highlight, Screen, STRIKE, UNDERLINE};
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
/// Search matches, the current one, and the copy-mode cursor and hint labels.
pub const MATCH_BG: u32 = 0x5C5034;
pub const CURRENT_BG: u32 = 0xF9E2AF;
pub const CURRENT_FG: u32 = 0x11111B;
pub const COPY_CURSOR: u32 = 0xF5C2E7;
pub const HINT_BG: u32 = 0xF9E2AF;
pub const HINT_FG: u32 = 0x11111B;
pub const HINT_TYPED_BG: u32 = 0x6C7086;

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
        let mut drawn = 0;
        let (cx, cy) = screen.cursor();
        let show_cursor = screen.cursor_visible && screen.view_offset() == 0;
        let old_cursor = self.cursor_row.take();
        if show_cursor { self.cursor_row = Some(cy); }
        // Copy mode's cursor, on screen.
        let copy = screen.copy_cursor().and_then(|p| {
            let top = screen.point_at(0, 0).row;
            (p.row >= top && p.row < top + screen.rows()).then_some((p.col, p.row - top))
        });
        for row in 0..screen.rows() {
            if !dirty.is_empty() && !dirty.get(row).copied().unwrap_or(true) && !(show_cursor && row == cy) && old_cursor != Some(row) { continue; }
            if (row + 1) * ch > pixels.len() / stride.max(1) { break; }
            drawn += 1;
            for col in 0..screen.cols() {
                if (col + 1) * cw > stride { break; }
                let cell = screen.visible(col, row);
                let (mut fg, mut bg) = colors(&cell);
                let selected = screen.selected(col, row);
                let cursor_here = show_cursor && row == cy && col == cx;
                if selected { bg = 0x45475A; }
                match screen.highlight(col, row) {
                    Highlight::None => {}
                    Highlight::Match => if !selected { bg = MATCH_BG },
                    Highlight::Current => { bg = CURRENT_BG; fg = CURRENT_FG; }
                }
                // A solid block when focused, an outline when not, as most terminals do.
                if cursor_here && focused { core::mem::swap(&mut fg, &mut bg); if fg == bg { fg = crate::DEFAULT_BG; bg = crate::DEFAULT_FG; } }
                let (x0, y0) = (col * cw, row * ch);
                self.glyph(cache, pixels, stride, x0, y0, cell.ch, fg, bg);
                let mut rule = |y: usize, color: u32| { let at = (y0 + y) * stride + x0; pixels[at..at + cw].fill(color); };
                if cell.flags & UNDERLINE != 0 { rule(ch - 2, fg); }
                // A hyperlink is underlined faintly, so it can be told from text the program underlined.
                else if cell.link != 0 { rule(ch - 1, blend(bg, fg, 110)); }
                if cell.flags & STRIKE != 0 { rule(ch / 2, fg); }
                if cursor_here && !focused { hollow(pixels, stride, x0, y0, cw, ch, crate::DEFAULT_FG); }
                if copy == Some((col, row)) { hollow(pixels, stride, x0, y0, cw, ch, COPY_CURSOR); }
            }
            // The sliver right of the last column.
            let used = screen.cols() * cw;
            if used < stride { for y in 0..ch { let at = (row * ch + y) * stride; pixels[at + used..at + stride].fill(crate::DEFAULT_BG); } }
        }
        drawn
    }
}

impl Painter {
    /// Draws quick-select hint labels over the screen as drawn by [`Painter::draw`]: each
    /// label's letters over the first cells of its hint, the letters already `typed`
    /// dimmed, and the rest of the hint underlined. Hints whose label does not start with
    /// `typed` are left alone. After the hints go away, draw the whole screen again.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_hints(&mut self, hints: &[Hint], typed: &str, cache: &mut Cache, pixels: &mut [u32], stride: usize, cols: usize, rows: usize) {
        let (cw, ch) = (self.cell_w, self.cell_h);
        let len = pixels.len();
        let fits = move |col: usize, row: usize| col < cols && row < rows && (col + 1) * cw <= stride && (row + 1) * ch * stride <= len;
        for h in hints.iter().filter(|h| h.label.starts_with(typed)) {
            // Underline the whole hint, row by row.
            let (mut col, mut row) = (h.col, h.row);
            loop {
                if fits(col, row) { let at = (row * ch + ch - 1) * stride + col * cw; pixels[at..at + cw].fill(HINT_BG); }
                if (row, col) == (h.end_row, h.end_col) || row > h.end_row { break; }
                col += 1;
                if col >= cols { col = 0; row += 1; }
            }
            let (mut col, mut row) = (h.col, h.row);
            for (i, c) in h.label.chars().enumerate() {
                if !fits(col, row) { break; }
                let bg = if i < typed.chars().count() { HINT_TYPED_BG } else { HINT_BG };
                self.glyph(cache, pixels, stride, col * cw, row * ch, c.to_ascii_uppercase(), HINT_FG, bg);
                col += 1;
                if col >= cols { col = 0; row += 1; }
            }
        }
    }

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

/// A one-pixel box around a cell.
fn hollow(pixels: &mut [u32], stride: usize, x0: usize, y0: usize, cw: usize, ch: usize, color: u32) {
    pixels[y0 * stride + x0..y0 * stride + x0 + cw].fill(color);
    pixels[(y0 + ch - 1) * stride + x0..(y0 + ch - 1) * stride + x0 + cw].fill(color);
    for y in 0..ch { pixels[(y0 + y) * stride + x0] = color; pixels[(y0 + y) * stride + x0 + cw - 1] = color; }
}

fn blend(bg: u32, fg: u32, a: u8) -> u32 {
    if a == 0 { return bg; }
    if a == 255 { return fg; }
    let a = a as u32;
    let mix = |s: u32| (((fg >> s) & 0xFF) * a + ((bg >> s) & 0xFF) * (255 - a)) / 255;
    mix(16) << 16 | mix(8) << 8 | mix(0)
}
