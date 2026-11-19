// SPDX-License-Identifier: GPL-3.0-only
//! Draws a [`Screen`] into a program's pixels with Geist Mono, row by row, only the rows
//! that changed. Pixels are 0x00RRGGBB, row-major, `stride` pixels per row.
//!
//! With [`Faces`] (a terminal sets them; a program's own console does not, to stay small)
//! bold and italic text use Geist Mono's bold and italic cuts, and characters Geist lacks
//! come from the Noto fallbacks: CJK, emoji, symbols and other scripts. A wide character
//! is drawn across its two cells.
//!
//! [`Faces`] for [`Family::JetBrainsMono`] draw in JetBrains Mono instead, and with the
//! `ligatures` feature its coding ligatures (`->`, `=>`, `!=`, `<!--`): each run of
//! punctuation is shaped with rustybuzz, every cell keeps the one glyph shaping put
//! there, and the ink of a glyph that reaches back over the cells before it is drawn in
//! those cells too. Copying, selecting and the cursor still see one character a cell.

use crate::hints::Hint;
use crate::{Highlight, Screen, BOLD, ITALIC, SPACER, STRIKE, UNDERLINE, WIDE};
use sex_font::fallback::Fallback;
use sex_font::cache::{GlyphCache, Key};
use sex_font::Font;

/// Kept glyph bitmaps. Large: keep it in a `static`, not on the stack.
pub type Cache = GlyphCache<2048, { 512 * 1024 }>;

/// The monospaced font a terminal draws its text in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Family {
    /// Geist Mono, the default: no ligatures.
    #[default]
    GeistMono,
    /// JetBrains Mono, whose coding ligatures are drawn with the `ligatures` feature.
    JetBrainsMono,
}

impl Family {
    pub const ALL: [Family; 2] = [Family::GeistMono, Family::JetBrainsMono];
    /// The name settings files use: `geist-mono`, `jetbrains-mono`.
    pub fn name(self) -> &'static str { match self { Family::GeistMono => "geist-mono", Family::JetBrainsMono => "jetbrains-mono" } }
    pub fn named(name: &str) -> Option<Family> { Family::ALL.into_iter().find(|f| f.name() == name) }
    /// Regular, bold, italic and bold italic.
    fn files(self) -> [&'static [u8]; 4] {
        match self {
            Family::GeistMono => [sex_font::GEIST_MONO, sex_font::GEIST_MONO_BOLD, sex_font::GEIST_MONO_ITALIC, sex_font::GEIST_MONO_BOLD_ITALIC],
            Family::JetBrainsMono => [sex_font::JETBRAINS_MONO, sex_font::JETBRAINS_MONO_BOLD, sex_font::JETBRAINS_MONO_ITALIC, sex_font::JETBRAINS_MONO_BOLD_ITALIC],
        }
    }
}

/// A family's regular, bold, italic and bold italic cuts and the fallback fonts (and,
/// with ligatures, the shapers). Tens of KB parsed: keep it in a `static` or a `Box` and
/// lend it to every [`Painter`].
pub struct Faces {
    family: Family,
    cuts: [Font<'static>; 4],
    fallback: Fallback<'static>,
    /// One per cut, for a family with ligatures.
    #[cfg(feature = "ligatures")]
    shapers: [Option<rustybuzz::Face<'static>>; 4],
}
impl Faces {
    /// Geist Mono's faces parsed straight into `slot`; on `false`, `slot` holds nothing.
    pub fn embedded_into(slot: &mut core::mem::MaybeUninit<Faces>) -> bool { Self::family_into(slot, Family::GeistMono) }
    /// `family`'s faces parsed straight into `slot`; on `false`, `slot` holds nothing.
    pub fn family_into(slot: &mut core::mem::MaybeUninit<Faces>, family: Family) -> bool {
        let p = slot.as_mut_ptr();
        // SAFETY: each field is written once through a raw pointer into `slot`, which the
        // caller treats as initialised only when this returns true.
        unsafe {
            let fallback = &mut *(core::ptr::addr_of_mut!((*p).fallback) as *mut core::mem::MaybeUninit<Fallback<'static>>);
            if !Fallback::embedded_into(fallback) { return false; }
            core::ptr::addr_of_mut!((*p).family).write(family);
            for (i, data) in family.files().into_iter().enumerate() {
                match Font::new(data) { Some(f) => core::ptr::addr_of_mut!((*p).cuts[i]).write(f), None => return false }
                #[cfg(feature = "ligatures")]
                core::ptr::addr_of_mut!((*p).shapers[i]).write(match family {
                    Family::JetBrainsMono => rustybuzz::Face::from_slice(data, 0),
                    Family::GeistMono => None,
                });
            }
        }
        true
    }
    /// Geist Mono's, boxed.
    pub fn embedded() -> Option<alloc::boxed::Box<Faces>> { Self::boxed(Family::GeistMono) }
    /// `family`'s, boxed.
    pub fn boxed(family: Family) -> Option<alloc::boxed::Box<Faces>> {
        let mut b = alloc::boxed::Box::<Faces>::new_uninit();
        // SAFETY: initialised exactly when `family_into` says so.
        Self::family_into(&mut b, family).then(|| unsafe { b.assume_init() })
    }
    pub fn family(&self) -> Family { self.family }
}

/// One cell of a shaped run: the glyph shaping put there, whether it differs from the
/// character's own glyph (and so may reach into the cells before it), and where the run
/// ends. `gid` 0 means the cell is drawn by character as usual.
#[cfg(feature = "ligatures")]
#[derive(Clone, Copy, Default)]
struct Lig { gid: u16, shaped: bool, end: u16 }

/// Runs shaped before, by their text and what is around them: a scrolling `cat` of the
/// same lines shapes each kind of run once.
#[cfg(feature = "ligatures")]
const SHAPED_SLOTS: usize = 128;
#[cfg(feature = "ligatures")]
#[derive(Default)]
struct Shaped { key: alloc::vec::Vec<u8>, gids: alloc::vec::Vec<u16> }

pub struct Painter {
    font: Font<'static>,
    faces: Option<&'static Faces>,
    #[cfg(feature = "ligatures")]
    lig: alloc::vec::Vec<Lig>,
    #[cfg(feature = "ligatures")]
    shaped: alloc::vec::Vec<Shaped>,
    #[cfg(feature = "ligatures")]
    buffer: Option<rustybuzz::UnicodeBuffer>,
    /// A ligature cell's coverage, its parts added up.
    #[cfg(feature = "ligatures")]
    mix: alloc::vec::Vec<u8>,
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
        Painter {
            font, faces: None, cell_w, cell_h, acc: alloc::vec![0; (2 * cell_w + 2) * (cell_h + 2)], cursor_row: None,
            #[cfg(feature = "ligatures")]
            lig: alloc::vec::Vec::new(),
            #[cfg(feature = "ligatures")]
            shaped: alloc::vec::Vec::new(),
            #[cfg(feature = "ligatures")]
            buffer: None,
            #[cfg(feature = "ligatures")]
            mix: alloc::vec::Vec::new(),
        }
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
            #[cfg(feature = "ligatures")]
            self.shape_row(screen, row);
            #[cfg(feature = "ligatures")]
            let lig = core::mem::take(&mut self.lig);
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
                #[cfg(feature = "ligatures")]
                let drawn_shaped = lig.get(col).is_some_and(|l| l.gid != 0) && self.lig_cell(cache, pixels, stride, x0, y0, &lig[col..lig[col].end as usize], style, fg, bg);
                #[cfg(not(feature = "ligatures"))]
                let drawn_shaped = false;
                if !drawn_shaped { self.cell(cache, pixels, stride, x0, y0, shown, style, cells, fg, bg); }
                let mut rule = |y: usize, color: u32| { let at = (y0 + y) * stride + x0; pixels[at..at + w].fill(color); };
                if cell.flags & UNDERLINE != 0 { rule(ch - 2, fg); }
                // A hyperlink is underlined faintly, so it can be told from text the program underlined.
                else if cell.link != 0 { rule(ch - 1, blend(bg, fg, 110)); }
                if cell.flags & STRIKE != 0 { rule(ch / 2, fg); }
                if cursor_here && !focused { hollow(pixels, stride, x0, y0, w, ch, screen.theme().cursor); }
                if copy == Some((col, row)) || cells == 2 && copy == Some((col + 1, row)) { hollow(pixels, stride, x0, y0, w, ch, COPY_CURSOR); }
            }
            #[cfg(feature = "ligatures")]
            { self.lig = lig; }
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
        let key = Key { font: self.font_id(style), scalar: ch as u32, size: h as u16, phase: 0, w: cw as u8, h: h as u8 };
        let (font, faces, acc) = (&self.font, self.faces, &mut self.acc);
        let glyph = if ch == ' ' { None } else {
            cache.get(key, |out| {
                let clear = |acc: &mut [i32]| acc.iter_mut().for_each(|a| *a = 0);
                clear(acc);
                let face = match faces { Some(f) => &f.cuts[style as usize], None => font };
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
    /// The cache's font number for a cut: 1 to 4 for Geist Mono, 5 to 8 for JetBrains Mono.
    fn font_id(&self, style: u8) -> u8 { 1 + style + 4 * self.faces.map_or(0, |f| f.family as u8) }

    /// Shapes the runs of punctuation on `row` that a font with ligatures turns into
    /// other glyphs, into `self.lig` (empty when nothing on the row is shaped).
    #[cfg(feature = "ligatures")]
    fn shape_row(&mut self, screen: &Screen, row: usize) {
        self.lig.clear();
        let Some(faces) = self.faces else { return };
        if faces.shapers[0].is_none() { return; }
        let cols = screen.cols();
        let style_of = |c: &crate::Cell| (c.flags & BOLD != 0) as u8 | ((c.flags & ITALIC != 0) as u8) << 1;
        let punct = |c: &crate::Cell| c.ch.is_ascii_punctuation() && c.flags & WIDE == 0;
        let mut col = 0;
        while col < cols {
            let first = screen.visible(col, row);
            if !punct(&first) { col += 1; continue; }
            let (start, style) = (col, style_of(&first));
            while col < cols && { let c = screen.visible(col, row); punct(&c) && style_of(&c) == style } { col += 1; }
            if col - start < 2 { continue; }
            let Some(shaper) = faces.shapers[style as usize].as_ref() else { continue };
            // The run and a few characters either side: some alternates look at them.
            let mut key = alloc::vec::Vec::with_capacity(col - start + 12);
            key.push(style);
            let around = |from: usize, to: usize, out: &mut alloc::vec::Vec<u8>| for c in from..to {
                let ch = screen.visible(c, row).ch;
                if ch != SPACER { let mut b = [0; 4]; out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes()); }
            };
            around(start.saturating_sub(3), start, &mut key);
            let pre_end = key.len();
            key.push(0);
            for c in start..col { key.push(screen.visible(c, row).ch as u8); }
            let run_end = key.len();
            key.push(0);
            around(col, (col + 3).min(cols), &mut key);
            let slot = key.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)) as usize % SHAPED_SLOTS;
            if self.shaped.len() < SHAPED_SLOTS { self.shaped.resize_with(SHAPED_SLOTS, Shaped::default); }
            if self.shaped[slot].key != key {
                let text = |a: usize, b: usize| core::str::from_utf8(&key[a..b]).unwrap_or("");
                let mut buffer = self.buffer.take().unwrap_or_default();
                buffer.set_pre_context(text(1, pre_end));
                buffer.push_str(text(pre_end + 1, run_end));
                buffer.set_post_context(text(run_end + 1, key.len()));
                buffer.set_direction(rustybuzz::Direction::LeftToRight);
                let out = rustybuzz::shape(shaper, &[], buffer);
                let infos = out.glyph_infos();
                // One glyph a cell, or the run is left as characters.
                let gids = if infos.len() == col - start { infos.iter().map(|i| i.glyph_id as u16).collect() } else { alloc::vec::Vec::new() };
                self.buffer = Some(out.clear());
                self.shaped[slot] = Shaped { key, gids };
            }
            let font = &faces.cuts[style as usize];
            let gids = &self.shaped[slot].gids;
            let nominal = |i: usize| font.glyph(screen.visible(start + i, row).ch as u32).map_or(0, |g| g.0);
            if gids.is_empty() || gids.iter().enumerate().all(|(i, g)| *g == nominal(i)) { continue; }
            if self.lig.is_empty() { self.lig.resize(cols, Lig::default()); }
            for (i, g) in gids.iter().enumerate() {
                self.lig[start + i] = Lig { gid: *g, shaped: *g != nominal(i), end: col as u16 };
            }
        }
    }

    /// One cell of a shaped run (`run` starts at the cell): its own glyph and the parts
    /// of the glyphs after it that reach back into it, as JetBrains Mono's ligatures do.
    /// False when it could not be drawn so (draw the character instead).
    #[cfg(feature = "ligatures")]
    #[allow(clippy::too_many_arguments)]
    fn lig_cell(&mut self, cache: &mut Cache, pixels: &mut [u32], stride: usize, x0: usize, y0: usize, run: &[Lig], style: u8, fg: u32, bg: u32) -> bool {
        let (cw, h) = (self.cell_w, self.cell_h);
        let Some(faces) = self.faces else { return false };
        if x0 + cw > stride || (y0 + h) * stride > pixels.len() || run.is_empty() { return false; }
        let style = style & 3;
        let font_id = self.font_id(style);
        let face = &faces.cuts[style as usize];
        let (acc, mix) = (&mut self.acc, &mut self.mix);
        mix.clear();
        mix.resize(cw * h, 0);
        // A ligature reaches back at most a few cells (`<!--` is the longest).
        for (k, l) in run.iter().enumerate().take(5) {
            if k > 0 && !l.shaped { continue; }
            let key = Key { font: font_id, scalar: l.gid as u32, size: h as u16, phase: 1 + k as u8, w: cw as u8, h: h as u8 };
            let part = cache.get(key, |out| {
                acc.iter_mut().for_each(|a| *a = 0);
                sex_font::draw_cell_glyph(face, sex_font::GlyphId(l.gid), k, cw, h, acc, out)
            });
            if let Some(g) = part { for (m, a) in mix.iter_mut().zip(g) { *m = m.saturating_add(*a); } }
        }
        for y in 0..h {
            let line = &mut pixels[(y0 + y) * stride + x0..(y0 + y) * stride + x0 + cw];
            for (x, p) in line.iter_mut().enumerate() { *p = blend(bg, fg, mix[y * cw + x]); }
        }
        true
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
