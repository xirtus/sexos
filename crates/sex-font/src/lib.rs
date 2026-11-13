// SPDX-License-Identifier: GPL-3.0-only
//! Fonts drawn at run time, at any size, from the TrueType files themselves.
//!
//! `ttf-parser` (MIT/Apache-2.0) reads the font without allocating; [`raster`] turns
//! the outlines into 8-bit coverage in integer arithmetic; [`cache`] keeps what was
//! drawn. Nothing here needs a heap, floating-point hardware or any authority: a
//! font is bytes, a glyph is a function of those bytes and a size.
#![no_std]

pub mod cache;
pub mod fallback;
pub mod raster;

pub use ttf_parser::GlyphId;
use raster::{div_round, Raster, FX};
use ttf_parser::{Face, OutlineBuilder};

/// Geist Mono and Geist Sans by Vercel, SIL Open Font License 1.1 (assets/fonts/geist/OFL.txt).
pub static GEIST_MONO: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-Regular.ttf");
pub static GEIST_SANS: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Regular.ttf");

/// A parsed font. Sizes are in 1/64 pixel per em, positions in 1/256 pixel.
pub struct Font<'a> {
    face: Face<'a>,
    upem: i64,
}

impl<'a> Font<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        let face = Face::parse(data, 0).ok()?;
        let upem = face.units_per_em() as i64;
        (upem > 0).then_some(Self { face, upem })
    }

    /// The glyph for a scalar, or `None` when the font lacks it (rather than .notdef).
    pub fn glyph(&self, scalar: u32) -> Option<GlyphId> {
        let ch = char::from_u32(scalar)?;
        self.face.glyph_index(ch).filter(|g| g.0 != 0)
    }

    /// Font units to 1/256 pixel at `size` (1/64 pixel per em).
    fn fx(&self, units: i64, size: u32) -> i32 {
        div_round(units * size as i64 * 4, self.upem) as i32
    }

    pub fn advance(&self, glyph: GlyphId, size: u32) -> i32 {
        self.fx(self.face.glyph_hor_advance(glyph).unwrap_or(0) as i64, size)
    }
    pub fn ascender(&self, size: u32) -> i32 { self.fx(self.face.ascender() as i64, size) }
    pub fn descender(&self, size: u32) -> i32 { self.fx(self.face.descender() as i64, size) }

    /// Height of capitals and depth of descenders, in font units (`H` and `g` ink).
    pub fn ink_extent(&self) -> (i16, i16) {
        let top = |c: u8| self.glyph(c as u32).and_then(|g| self.face.glyph_bounding_box(g));
        let cap = self.face.capital_height().filter(|h| *h > 0).or_else(|| top(b'H').map(|b| b.y_max)).unwrap_or(700);
        let desc = top(b'g').map(|b| b.y_min).unwrap_or(self.face.descender());
        (cap, desc.min(0))
    }

    /// Pair kerning between two glyphs from GPOS `kern` lookups, else the `kern` table.
    pub fn kerning(&self, left: GlyphId, right: GlyphId, size: u32) -> i32 {
        self.fx(self.kern_units(left, right) as i64, size)
    }

    fn kern_units(&self, left: GlyphId, right: GlyphId) -> i16 {
        use ttf_parser::gpos::{PairAdjustment, PositioningSubtable};
        let tables = self.face.tables();
        if let Some(gpos) = tables.gpos {
            for feature in gpos.features {
                if feature.tag != ttf_parser::Tag::from_bytes(b"kern") { continue; }
                for index in feature.lookup_indices {
                    let Some(lookup) = gpos.lookups.get(index) else { continue };
                    for sub in 0..lookup.subtables.len() {
                        let Some(PositioningSubtable::Pair(pair)) = lookup.subtables.get::<PositioningSubtable>(sub) else { continue };
                        let Some(first) = pair.coverage().get(left) else { continue };
                        let value = match pair {
                            PairAdjustment::Format1 { sets, .. } => sets.get(first).and_then(|set| set.get(right)),
                            PairAdjustment::Format2 { classes, matrix, .. } => matrix.get((classes.0.get(left), classes.1.get(right))),
                        };
                        // The first pair subtable that covers the glyph decides, kerned or not.
                        if let Some((value, _)) = value { return value.x_advance; }
                        if matches!(pair, PairAdjustment::Format2 { .. }) { return 0; }
                    }
                }
                return 0;
            }
        }
        tables.kern.and_then(|kern| kern.subtables.into_iter().find(|s| s.horizontal && !s.variable))
            .and_then(|s| s.glyphs_kerning(left, right)).unwrap_or(0)
    }

    /// Draw `glyph` at `size` into a `w` x `h` coverage bitmap, its origin at
    /// (`x`, `baseline`) in 1/256 pixel from the bitmap's top left. Ink outside the
    /// bitmap is clipped. `acc` needs [`raster::scratch_len`]`(w, h)` cells.
    pub fn draw(&self, glyph: GlyphId, size: u32, x: i32, baseline: i32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
        let Some(raster) = Raster::new(w, h, acc) else { return false };
        if out.len() < w * h { return false; }
        let mut pen = Pen { raster, font: self, size, x, baseline, start: (0, 0), at: (0, 0) };
        self.face.outline_glyph(glyph, &mut pen);
        pen.raster.finish(out);
        true
    }
}

/// Outline callbacks in font units, converted at once to fixed-point pixels.
struct Pen<'f, 'a, 'r> {
    raster: Raster<'r>,
    font: &'f Font<'a>,
    size: u32,
    x: i32,
    baseline: i32,
    start: (i32, i32),
    at: (i32, i32),
}

impl Pen<'_, '_, '_> {
    fn point(&self, x: f32, y: f32) -> (i32, i32) {
        // Sixteenths of a font unit keep composite-glyph offsets exact enough.
        let units = |v: f32| (v * 16.0 + if v < 0.0 { -0.5 } else { 0.5 }) as i64;
        let scale = |v: f32| div_round(units(v) * self.size as i64 * 4, self.font.upem * 16) as i32;
        (self.x + scale(x), self.baseline - scale(y))
    }
}

impl OutlineBuilder for Pen<'_, '_, '_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.at = self.point(x, y);
        self.start = self.at;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.raster.line(self.at.0, self.at.1, p.0, p.1);
        self.at = p;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (c, p) = (self.point(x1, y1), self.point(x, y));
        self.raster.quad(self.at, c, p);
        self.at = p;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1, c2, p) = (self.point(x1, y1), self.point(x2, y2), self.point(x, y));
        self.raster.cubic(self.at, c1, c2, p);
        self.at = p;
    }
    fn close(&mut self) {
        if self.at != self.start { self.raster.line(self.at.0, self.at.1, self.start.0, self.start.1); }
        self.at = self.start;
    }
}

/// Where a monospaced font sits in a `w` x `h` text cell: as large as the cell
/// allows while the advance keeps a sliver of the width free and capitals plus
/// descenders fit the height; the advance centred in the width and the ink box
/// (cap height to descender) centred in the height. Accents above capitals may clip.
/// Returns (size in 1/64 px, x, baseline) in 1/256 pixel.
pub fn cell_layout(font: &Font, w: usize, h: usize) -> (u32, i32, i32) {
    let probe = 64 * 64;
    let m = font.glyph(b'M' as u32).unwrap_or(GlyphId(0));
    let advance = font.advance(m, probe).max(1) as i64;
    let (cap, desc) = font.ink_extent();
    let ink = font.fx((cap - desc) as i64, probe).max(1) as i64;
    // At 12x18 this is Geist Mono at 19 px, the size the grid was designed around.
    let by_width = w as i64 * FX as i64 * 95 * probe as i64 / (advance * 100);
    let by_height = (h as i64 * FX as i64 - FX as i64 / 2) * probe as i64 / ink;
    let size = by_width.min(by_height).max(64) as u32;
    let x = (w as i32 * FX - font.advance(m, size)) / 2;
    let ink_h = font.fx((cap - desc) as i64, size);
    let baseline = (h as i32 * FX - ink_h) / 2 + font.fx(cap as i64, size);
    (size, x, baseline)
}

/// One character of a monospaced grid: `w * h` coverage bytes, or `false` when the
/// font lacks the scalar. The same function feeds Display and the pixel oracles.
pub fn draw_cell(font: &Font, scalar: u32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
    let Some(glyph) = font.glyph(scalar) else { return false };
    let (size, x, baseline) = cell_layout(font, w, h);
    font.draw(glyph, size, x, baseline, w, h, acc, out)
}
