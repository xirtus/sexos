// SPDX-License-Identifier: GPL-3.0-only
//! Text: Geist Sans for interface text, Geist Mono for code and numbers, measured and
//! drawn at any size with sex-font's outline rasterizer, glyphs kept once drawn.
//!
//! Characters Geist lacks come from Geist Mono, then (with the `fallback-fonts`
//! feature) from the Noto fallback set: CJK, Arabic, Hebrew, Devanagari, Thai and
//! symbols. Shaping and bidi (sex-shape) join later; interface strings are drawn in
//! logical order with pair kerning.
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use sex_font::{Font, GlyphId, Render, Style, Synthetic};

/// A typeface: Geist Sans or Geist Mono, regular, bold, italic or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Face { pub mono: bool, pub style: Style }

#[allow(non_upper_case_globals)]
impl Face {
    pub const Sans: Face = Face { mono: false, style: Style::REGULAR };
    pub const Mono: Face = Face { mono: true, style: Style::REGULAR };
    /// This face in bold.
    pub const fn bold(self) -> Face { Face { mono: self.mono, style: Style { bold: true, italic: self.style.italic } } }
    /// This face in italic.
    pub const fn italic(self) -> Face { Face { mono: self.mono, style: Style { bold: self.style.bold, italic: true } } }
    /// This face with `style`.
    pub const fn with(self, style: Style) -> Face { Face { mono: self.mono, style } }
    /// Font id of the Geist file: 0..4 Sans, 4..8 Mono.
    fn id(self) -> u8 { (self.mono as u8) * 4 + self.style.index() as u8 }
}

/// One drawn glyph: coverage bytes and where they sit relative to the pen.
pub struct Glyph {
    pub w: usize,
    pub h: usize,
    /// Pixels from the pen position to the bitmap's left edge (negative: left of it).
    pub left: i32,
    /// Pixels from the baseline up to the bitmap's top edge.
    pub top: i32,
    pub coverage: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key { font: u8, glyph: u16, size: u32, phase: u8, style: u8, dark: bool }

/// Subpixel positions a glyph is drawn at: quarter pixels.
const PHASES: i32 = 4;

pub struct Fonts {
    /// The eight Geist faces, by [`Face::id`].
    geist: [Font<'static>; 8],
    #[cfg(feature = "fallback-fonts")]
    fallback: Option<alloc::boxed::Box<sex_font::fallback::Fallback<'static>>>,
    cache: BTreeMap<Key, Glyph>,
    cached_bytes: usize,
    acc: Vec<i32>,
}

/// Glyph bitmaps kept before the cache starts over (a window of text at three sizes
/// fits several times over).
const CACHE_BYTES: usize = 4 << 20;

impl Default for Fonts {
    fn default() -> Self { Self::new() }
}

impl Fonts {
    pub fn new() -> Fonts {
        Fonts {
            geist: core::array::from_fn(|i| {
                let style = Style { bold: i & 1 != 0, italic: i & 2 != 0 };
                Font::new(sex_font::geist(i >= 4, style)).expect("Geist parses")
            }),
            #[cfg(feature = "fallback-fonts")]
            fallback: sex_font::fallback::Fallback::embedded().map(alloc::boxed::Box::new),
            cache: BTreeMap::new(),
            cached_bytes: 0,
            acc: Vec::new(),
        }
    }

    fn primary(&self, face: Face) -> &Font<'static> { &self.geist[face.id() as usize] }

    fn font(&self, id: u8) -> Option<&Font<'static>> {
        match id {
            0..=7 => Some(&self.geist[id as usize]),
            #[cfg(feature = "fallback-fonts")]
            n => self.fallback.as_ref().and_then(|f| f.font(n - 10)),
            #[cfg(not(feature = "fallback-fonts"))]
            _ => None,
        }
    }

    /// Which font draws `c` in `face`, and its glyph. Geist fonts come in every
    /// style; the fallbacks are regular and get a synthetic one (see [`Self::synth`]).
    fn resolve(&self, face: Face, c: char) -> Option<(u8, GlyphId)> {
        let first = face.id();
        if let Some(g) = self.primary(face).glyph(c as u32) { return Some((first, g)); }
        let other = Face { mono: !face.mono, style: face.style }.id();
        if let Some(g) = self.font(other).and_then(|f| f.glyph(c as u32)) { return Some((other, g)); }
        #[cfg(feature = "fallback-fonts")]
        if let Some(fb) = &self.fallback {
            if let Some((id, f)) = fb.outline(c as u32) { return f.glyph(c as u32).map(|g| (10 + id, g)); }
        }
        None
    }

    /// The synthetic style font `id` needs to draw `face`: none for Geist.
    fn synth(id: u8, face: Face) -> Synthetic { if id < 8 { Synthetic::NONE } else { Synthetic::for_style(face.style) } }

    /// Size in sex-font's units (1/64 pixel per em) for `px` pixels.
    fn units(px: f32) -> u32 { (px * 64.0 + 0.5).max(64.0) as u32 }

    /// Ascent and descent (positive, below the baseline) in pixels.
    pub fn metrics(&self, face: Face, px: f32) -> (f32, f32) {
        let f = self.primary(face);
        let s = Self::units(px);
        (f.ascender(s) as f32 / 256.0, -(f.descender(s) as f32) / 256.0)
    }

    /// The height of one line of text in pixels.
    pub fn line_height(&self, face: Face, px: f32) -> f32 {
        let (a, d) = self.metrics(face, px);
        libm::ceilf((a + d) * 1.15)
    }

    /// The advance of each character of `text` in pixels, kerning included.
    pub fn advances(&self, face: Face, px: f32, text: &str, out: &mut Vec<f32>) {
        let s = Self::units(px);
        let mut prev: Option<(u8, GlyphId)> = None;
        for c in text.chars() {
            let r = self.resolve(face, c);
            let mut adv = match r {
                Some((id, g)) => self.font(id).map_or(0, |f| f.advance_synthetic(g, s, Self::synth(id, face))),
                // Missing glyphs take the width of a box.
                None => (px * 0.6 * 256.0) as i32,
            };
            if let (Some((pi, pg)), Some((id, g))) = (prev, r) {
                if pi == id { if let Some(f) = self.font(id) { adv += f.kerning(pg, g, s); } }
            }
            out.push(adv as f32 / 256.0);
            prev = r;
        }
    }

    /// The width of `text` in pixels.
    pub fn measure(&self, face: Face, px: f32, text: &str) -> f32 {
        let mut v = Vec::new();
        self.advances(face, px, text, &mut v);
        v.iter().sum()
    }

    /// The glyph for `c` at `px` with the pen `phase` quarters of a pixel right of a
    /// whole pixel, drawn on first use, for dark ink (stems darkened).
    pub fn glyph(&mut self, face: Face, c: char, px: f32, phase: u8) -> Option<&Glyph> {
        self.glyph_for(face, c, px, phase, 0)
    }

    /// [`Fonts::glyph`] for ink of colour `ink` (`0xRRGGBB`): light hinting at small
    /// sizes, and stem darkening only for dark ink (`sex_font::Render::ui_for`).
    pub fn glyph_for(&mut self, face: Face, c: char, px: f32, phase: u8, ink: u32) -> Option<&Glyph> {
        let (id, g) = self.resolve(face, c)?;
        let size = Self::units(px);
        let synth = Self::synth(id, face);
        let render = Render::ui_for(ink);
        let key = Key { font: id, glyph: g.0, size, phase, style: if id < 8 { 0 } else { face.style.index() as u8 }, dark: render.darken };
        if !self.cache.contains_key(&key) {
            if self.cached_bytes > CACHE_BYTES { self.cache.clear(); self.cached_bytes = 0; }
            let font = self.font(id)?;
            let asc = font.ascender(size) / 256 + 2;
            let desc = -font.descender(size) / 256 + 2;
            let adv = font.advance_synthetic(g, size, synth) / 256;
            // Room for overhang, and for a synthetic lean (about a fifth of the ascent).
            let pad = (px as i32) / 3 + 2 + if synth.slant != 0 { asc / 4 } else { 0 };
            let w = (adv + 2 * pad).max(1) as usize;
            let h = (asc + desc).max(1) as usize;
            let need = sex_font::raster::scratch_len(w, h);
            let mut acc = core::mem::take(&mut self.acc);
            if acc.len() < need { acc.resize(need, 0); }
            let mut coverage = vec![0u8; w * h];
            let x = pad * 256 + phase as i32 * 256 / PHASES;
            let drawn = self.font(id).map_or(false, |font| font.draw_rendered(g, size, x, asc * 256, w, h, &mut acc[..need], &mut coverage, synth, render));
            self.acc = acc;
            if !drawn { return None; }
            self.cached_bytes += coverage.len() + 32;
            self.cache.insert(key, Glyph { w, h, left: -pad, top: asc, coverage });
        }
        self.cache.get(&key)
    }

    pub fn phase_of(x: f32) -> (i32, u8) {
        let whole = libm::floorf(x);
        let phase = ((x - whole) * PHASES as f32) as i32;
        (whole as i32, phase.clamp(0, PHASES - 1) as u8)
    }

    /// Where to break `text` to fit `width` pixels: byte ranges of each line. Breaks
    /// at spaces and after hyphens; a word longer than the width is cut where it must be.
    pub fn wrap(&self, face: Face, px: f32, text: &str, width: f32) -> Vec<(usize, usize)> {
        let mut lines = Vec::new();
        for (para_start, para) in split_keep(text) {
            let mut adv = Vec::new();
            self.advances(face, px, para, &mut adv);
            let idx: Vec<usize> = para.char_indices().map(|(i, _)| i).collect();
            let mut start = 0usize; // char index
            while start < idx.len() {
                let mut x = 0.0;
                let mut last_break: Option<usize> = None;
                let mut end = start;
                while end < idx.len() {
                    let c = para[idx[end]..].chars().next().unwrap();
                    if x + adv[end] > width && end > start {
                        break;
                    }
                    x += adv[end];
                    if c == ' ' || c == '-' { last_break = Some(end + 1); }
                    end += 1;
                }
                let cut = if end >= idx.len() { idx.len() } else { last_break.filter(|b| *b > start).unwrap_or(end.max(start + 1)) };
                let byte = |ci: usize| if ci >= idx.len() { para.len() } else { idx[ci] };
                let (mut a, mut b) = (byte(start), byte(cut));
                // Spaces at a break belong to no line.
                while b > a && para.as_bytes()[b - 1] == b' ' { b -= 1; }
                while a < b && para.as_bytes()[a] == b' ' && start > 0 { a += 1; }
                lines.push((para_start + a, para_start + b));
                start = cut;
            }
            if para.is_empty() { lines.push((para_start, para_start)); }
        }
        lines
    }

    /// `text` cut to `width` pixels with an ellipsis, as a byte length to keep.
    pub fn fit(&self, face: Face, px: f32, text: &str, width: f32) -> Option<usize> {
        if self.measure(face, px, text) <= width { return None; }
        let ell = self.measure(face, px, "…");
        let mut adv = Vec::new();
        self.advances(face, px, text, &mut adv);
        let mut x = 0.0;
        for ((i, _), a) in text.char_indices().zip(adv) {
            if x + a + ell > width { return Some(i); }
            x += a;
        }
        Some(text.len())
    }
}

/// Paragraphs of `text` (split at `\n`) with their byte offsets.
fn split_keep(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut at = 0;
    text.split('\n').map(move |p| { let s = at; at += p.len() + 1; (s, p) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measures_and_draws_geist() {
        let mut f = Fonts::new();
        let w = f.measure(Face::Sans, 14.0, "Hello");
        assert!(w > 25.0 && w < 45.0, "{w}");
        assert!(f.measure(Face::Sans, 28.0, "Hello") > w * 1.9);
        let g = f.glyph(Face::Sans, 'H', 14.0, 0).unwrap();
        assert!(g.coverage.iter().filter(|c| **c > 128).count() > 10);
    }
    #[test]
    fn bold_and_italic_faces() {
        let mut f = Fonts::new();
        let ink = |f: &mut Fonts, face: Face| f.glyph(face, 'n', 20.0, 0).unwrap().coverage.iter().map(|c| *c as u32).sum::<u32>();
        let (r, b) = (ink(&mut f, Face::Sans), ink(&mut f, Face::Sans.bold()));
        assert!(b as f32 > r as f32 * 1.3, "{r} {b}");
        assert!(f.measure(Face::Sans.bold(), 14.0, "Hello") > f.measure(Face::Sans, 14.0, "Hello"));
        assert!(ink(&mut f, Face::Mono.bold().italic()) > ink(&mut f, Face::Mono));
        // Each face caches its own glyphs.
        assert_ne!(ink(&mut f, Face::Sans.italic()), r);
    }
    #[test]
    fn wraps_at_spaces() {
        let f = Fonts::new();
        let text = "one two three four five six seven";
        let lines = f.wrap(Face::Sans, 14.0, text, 80.0);
        assert!(lines.len() >= 3);
        for (a, b) in &lines {
            assert!(f.measure(Face::Sans, 14.0, &text[*a..*b]) <= 80.0, "{:?}", &text[*a..*b]);
            assert!(!text[*a..*b].starts_with(' ') && !text[*a..*b].ends_with(' '));
        }
        assert_eq!(f.wrap(Face::Sans, 14.0, "a\n\nb", 100.0).len(), 3);
    }
    #[test]
    fn fits_with_an_ellipsis() {
        let f = Fonts::new();
        assert_eq!(f.fit(Face::Sans, 14.0, "short", 200.0), None);
        let keep = f.fit(Face::Sans, 14.0, "a rather long tab title here", 80.0).unwrap();
        assert!(keep > 3 && keep < 20);
    }
}
