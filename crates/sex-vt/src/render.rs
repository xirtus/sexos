// SPDX-License-Identifier: GPL-3.0-only
//! Draws a [`Screen`] into a program's pixels with Geist Mono, row by row, only the rows
//! that changed. Pixels are 0x00RRGGBB, row-major, `stride` pixels per row.
//!
//! With [`Faces`] (a terminal sets them; a program's own console does not, to stay small)
//! bold and italic text use Geist Mono's bold and italic cuts, and characters Geist lacks
//! come from the Noto fallbacks: CJK, emoji, symbols and other scripts. A wide character
//! is drawn across its two cells.

use crate::hints::Hint;
use crate::{Highlight, Screen, BOLD, ITALIC, SPACER, STRIKE, UNDERLINE, WIDE};
use sex_font::fallback::Fallback;
use sex_font::cache::{GlyphCache, Key};
use sex_font::Font;

/// Kept glyph bitmaps. Large: keep it in a `static`, not on the stack.
pub type Cache = GlyphCache<2048, { 512 * 1024 }>;

/// Geist Mono's bold, italic and bold italic cuts and the fallback fonts. About 12 KB
/// parsed: keep it in a `static` or a `Box` and lend it to every [`Painter`].
pub struct Faces {
    styles: [Font<'static>; 3],
    fallback: Fallback<'static>,
}
impl Faces {
    /// The embedded fonts parsed straight into `slot`; on `false`, `slot` holds nothing.
    pub fn embedded_into(slot: &mut core::mem::MaybeUninit<Faces>) -> bool {
        let p = slot.as_mut_ptr();
        // SAFETY: each field is written once through a raw pointer into `slot`, which the
        // caller treats as initialised only when this returns true.
        unsafe {
            let fallback = &mut *(core::ptr::addr_of_mut!((*p).fallback) as *mut core::mem::MaybeUninit<Fallback<'static>>);
            if !Fallback::embedded_into(fallback) { return false; }
            let cuts = [sex_font::GEIST_MONO_BOLD, sex_font::GEIST_MONO_ITALIC, sex_font::GEIST_MONO_BOLD_ITALIC];
            for (i, data) in cuts.into_iter().enumerate() {
                match Font::new(data) { Some(f) => core::ptr::addr_of_mut!((*p).styles[i]).write(f), None => return false }
            }
        }
        true
    }
    /// The same, boxed.
    pub fn embedded() -> Option<alloc::boxed::Box<Faces>> {
        let mut b = alloc::boxed::Box::<Faces>::new_uninit();
        // SAFETY: initialised exactly when `embedded_into` says so.
        Self::embedded_into(&mut b).then(|| unsafe { b.assume_init() })
    }
}

pub struct Painter {
    font: Font<'static>,
    faces: Option<&'static Faces>,
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
        Painter { font, faces: None, cell_w, cell_h, acc: alloc::vec![0; (2 * cell_w + 2) * (cell_h + 2)], cursor_row: None }
    }
    /// Draws with these bold and italic cuts and fallback fonts from now on.
    pub fn with_faces(mut self, faces: &'static Faces) -> Painter { self.faces = Some(faces); self }
    pub fn set_faces(&mut self, faces: Option<&'static Faces>) { self.faces = faces; }
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
        // Copy mode's cursor, on screen.
        let copy = screen.copy_cursor().and_then(|p| {
            let top = screen.point_at(0, 0).row;
            (p.row >= top && p.row < top + screen.rows()).then_some((p.col, p.row - top))
        });
        for row in 0..screen.rows() {
            if !dirty.is_empty() && !dirty.get(row).copied().unwrap_or(true) && !(show_cursor && row == cy) && old_cursor != Some(row) { continue; }
            if (row + 1) * ch > pixels.len() / stride.max(1) { break; }
            drawn += 1;
            painted[row] = true;
            for col in 0..screen.cols() {
                if (col + 1) * cw > stride { break; }
                let cell = screen.visible(col, row);
                // A wide character's second cell was drawn with its first.
                if cell.ch == SPACER && col > 0 && screen.visible(col - 1, row).flags & WIDE != 0 { continue; }
                let cells = if cell.flags & WIDE != 0 && col + 1 < screen.cols() && screen.visible(col + 1, row).ch == SPACER && (col + 2) * cw <= stride { 2 } else { 1 };
                let w = cells * cw;
                let (mut fg, mut bg) = screen.colors(&cell);
                let selected = screen.selected(col, row);
                // The cursor can sit on either half of a wide character.
                let cursor_here = show_cursor && row == cy && (col == cx || cells == 2 && col + 1 == cx);
                if selected { bg = screen.theme().selection; }
                match screen.highlight(col, row) {
                    Highlight::None => {}
                    Highlight::Match => if !selected { bg = MATCH_BG },
                    Highlight::Current => { bg = CURRENT_BG; fg = CURRENT_FG; }
                }
                // A solid block when focused, an outline when not, as most terminals do.
                if cursor_here && focused { core::mem::swap(&mut fg, &mut bg); if fg == bg { fg = screen.theme().bg; bg = screen.theme().cursor; } }
                let (x0, y0) = (col * cw, row * ch);
                let style = (cell.flags & BOLD != 0) as u8 | ((cell.flags & ITALIC != 0) as u8) << 1;
                let shown = if cell.ch == SPACER { ' ' } else { cell.ch };
                self.cell(cache, pixels, stride, x0, y0, shown, style, cells, fg, bg);
                let mut rule = |y: usize, color: u32| { let at = (y0 + y) * stride + x0; pixels[at..at + w].fill(color); };
                if cell.flags & UNDERLINE != 0 { rule(ch - 2, fg); }
                // A hyperlink is underlined faintly, so it can be told from text the program underlined.
                else if cell.link != 0 { rule(ch - 1, blend(bg, fg, 110)); }
                if cell.flags & STRIKE != 0 { rule(ch / 2, fg); }
                if cursor_here && !focused { hollow(pixels, stride, x0, y0, w, ch, screen.theme().cursor); }
                if copy == Some((col, row)) || cells == 2 && copy == Some((col + 1, row)) { hollow(pixels, stride, x0, y0, w, ch, COPY_CURSOR); }
            }
            // The sliver right of the last column.
            let used = screen.cols() * cw;
            if used < stride { for y in 0..ch { let at = (row * ch + y) * stride; pixels[at + used..at + stride].fill(screen.theme().bg); } }
        }
        // Inline images over (or under) the text of the rows just drawn.
        crate::images::composite(screen, cw, ch, &painted, pixels, stride);
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
        self.cell(cache, pixels, stride, x0, y0, ch, 0, 1, fg, bg);
    }
    /// `cells` cells (1, or 2 for a wide character) at pixel `x0, y0`: `ch` in `style`
    /// (bit 0 bold, bit 1 italic), from the fallback fonts when Geist Mono lacks it.
    #[allow(clippy::too_many_arguments)]
    pub fn cell(&mut self, cache: &mut Cache, pixels: &mut [u32], stride: usize, x0: usize, y0: usize, ch: char, style: u8, cells: usize, fg: u32, bg: u32) {
        let (cw, h) = (self.cell_w * cells, self.cell_h);
        if x0 + cw > stride || (y0 + h) * stride > pixels.len() { return; }
        // Without faces every style is the regular cut, so they share bitmaps.
        let style = if self.faces.is_some() { style & 3 } else { 0 };
        let key = Key { font: 1 + style, scalar: ch as u32, size: h as u16, phase: 0, w: cw as u8, h: h as u8 };
        let (font, faces, acc) = (&self.font, self.faces, &mut self.acc);
        let glyph = if ch == ' ' { None } else {
            cache.get(key, |out| {
                let clear = |acc: &mut [i32]| acc.iter_mut().for_each(|a| *a = 0);
                clear(acc);
                let face = match (style, faces) { (1..=3, Some(f)) => &f.styles[style as usize - 1], _ => font };
                // A wide character is never Geist's (it has no CJK or emoji): only fallbacks fit it.
                if cells == 1 && sex_font::draw_cell(face, ch as u32, cw, h, acc, out) { return true; }
                let Some(f) = faces else { return false };
                clear(acc);
                f.fallback.draw_cell(ch as u32, cw, h, acc, out)
            })
        };
        for y in 0..h {
            let line = &mut pixels[(y0 + y) * stride + x0..(y0 + y) * stride + x0 + cw];
            match glyph {
                Some(g) => for (x, p) in line.iter_mut().enumerate() { *p = blend(bg, fg, g[y * cw + x]); },
                None => line.fill(bg),
            }
        }
    }
    /// A run of text from pixel `x0, y0`, one or two cells per character; returns the x after it.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, cache: &mut Cache, pixels: &mut [u32], stride: usize, mut x0: usize, y0: usize, text: &str, fg: u32, bg: u32) -> usize {
        for c in text.chars() {
            let cells = sex_shape::cells::width(c as u32);
            if cells == 0 { continue; }
            if x0 + cells * self.cell_w > stride { break; }
            self.cell(cache, pixels, stride, x0, y0, c, 0, cells, fg, bg);
            x0 += cells * self.cell_w;
        }
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

/// A scroll in progress, drawn over a few frames instead of in one jump. The view has
/// already moved; `left` pixels of the move are still to be shown (positive when the
/// text still has to come down). Rows beyond the new frame come from `from`, what was
/// on screen when the glide (re)started, `span` pixels before the end.
#[derive(Default)]
pub struct Glide {
    pub left: i32,
    pub span: i32,
    pub from: alloc::vec::Vec<u32>,
    pub shown: alloc::vec::Vec<u32>,
    /// The `left` that `shown` was composed with: [`Glide::step`] moves `left` on to the
    /// next frame before the window has shown this one.
    pub shown_left: i32,
}

impl Glide {
    /// Composes the frame `now` (the pane's pixels) offset by what is left into `shown`.
    pub fn compose(&mut self, now: &[u32], w: usize, h: usize) {
        self.shown.resize(w * h, 0);
        self.shown_left = self.left;
        for y in 0..h {
            // The new frame moved by `left` too much: row y shows its row y + left, and
            // past its edge, the old frame's (which was `span` further along).
            let src = y as i32 + self.left;
            let (rows, at): (&[u32], i32) = if src >= 0 && (src as usize) < h { (now, src) } else { (&self.from, src - self.span) };
            let at = at.clamp(0, h as i32 - 1) as usize;
            self.shown[y * w..y * w + w].copy_from_slice(&rows[at * w..at * w + w]);
        }
    }
    /// Whether a glide is on screen.
    pub fn active(&self) -> bool { self.left != 0 }
    /// Scrolls `px` more from what is on screen now (`on_screen`, `h` rows tall): a glide
    /// from there, or false when the move is a screen or more and should just jump.
    pub fn retarget(&mut self, px: i32, on_screen: &[u32], h: i32) -> bool {
        // The window shows the frame composed with `shown_left`, not the stepped `left`.
        let left = if self.active() { self.shown_left } else { 0 } + px;
        if left == 0 || left.abs() >= h { *self = Glide::default(); return false; }
        self.from = on_screen.to_vec();
        self.left = left;
        self.span = left;
        true
    }
    /// One frame further: most of what is left goes, so the scroll eases out.
    pub fn step(&mut self) { self.left -= self.left * 5 / 8 + self.left.signum(); if self.left.abs() < 2 { self.left = 0; } }
}
