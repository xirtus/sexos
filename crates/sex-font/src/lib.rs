// SPDX-License-Identifier: GPL-3.0-only
//! Fonts drawn at run time, at any size, from the TrueType files themselves.
//!
//! `ttf-parser` (MIT/Apache-2.0) reads the font without allocating; [`raster`] turns
//! the outlines into 8-bit coverage in integer arithmetic; [`cache`] keeps what was
//! drawn. Nothing here needs a heap, floating-point hardware or any authority: a
//! font is bytes, a glyph is a function of those bytes and a size.
#![no_std]

// Only the OpenType inspector allocates; glyph rendering stays allocation-free so
// servers without an allocator (sexdisplay) can keep linking this crate.
#[cfg(feature = "inspect")]
extern crate alloc;

pub mod cache;
pub mod fallback;
#[cfg(feature = "inspect")]
#[forbid(unsafe_code)]
pub mod inspect;
pub mod gamma;
mod gamma_tables;
pub mod raster;

#[cfg(feature = "inspect")]
pub use inspect::{
    font_collection_count, inspect_font, FontClass, FontMetadata, Panose, ScriptRange,
    UnicodeCoverage,
};
pub use ttf_parser::GlyphId;
use raster::{div_round, Raster, FX};
use ttf_parser::{Face, OutlineBuilder};

/// Geist Mono and Geist Sans by Vercel, SIL Open Font License 1.1 (assets/fonts/geist/OFL.txt).
pub static GEIST_MONO: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-Regular.ttf");
pub static GEIST_SANS: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Regular.ttf");
/// Medium (500) and SemiBold (600) faces for warm UI body text and crisp macOS-style UI chrome.
pub static GEIST_SANS_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Medium.ttf");
pub static GEIST_SANS_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-SemiBold.ttf");
pub static GEIST_MONO_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-Medium.ttf");
pub static GEIST_MONO_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-SemiBold.ttf");
/// The bold and italic faces of both. A program pays only for the faces it names:
/// the linker drops the rest.
pub static GEIST_SANS_BOLD: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Bold.ttf");
pub static GEIST_SANS_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Italic.ttf");
pub static GEIST_SANS_BOLD_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-BoldItalic.ttf");
pub static GEIST_MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-Bold.ttf");
pub static GEIST_MONO_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-Italic.ttf");
pub static GEIST_MONO_BOLD_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-BoldItalic.ttf");
/// JetBrains Mono, SIL Open Font License 1.1 (assets/fonts/jetbrains/OFL.txt): Spindle's
/// other terminal font, the one with coding ligatures.
pub static JETBRAINS_MONO: &[u8] = include_bytes!("../../../assets/fonts/jetbrains/JetBrainsMono-Regular.ttf");
pub static JETBRAINS_MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/jetbrains/JetBrainsMono-Bold.ttf");
pub static JETBRAINS_MONO_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/jetbrains/JetBrainsMono-Italic.ttf");
pub static JETBRAINS_MONO_BOLD_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/jetbrains/JetBrainsMono-BoldItalic.ttf");

/// Typographic weight of a face (`OS/2` weight class).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weight {
    #[default]
    Regular,
    Medium,
    SemiBold,
    Bold,
}

impl Weight {
    /// Numeric OpenType weight class (400, 500, 600, 700).
    pub const fn number(self) -> u16 {
        match self {
            Weight::Regular => 400,
            Weight::Medium => 500,
            Weight::SemiBold => 600,
            Weight::Bold => 700,
        }
    }

    /// Maps an OpenType `OS/2` weight class number to the nearest [`Weight`].
    pub const fn from_number(weight: u16) -> Self {
        match weight {
            0..=449 => Weight::Regular,
            450..=549 => Weight::Medium,
            550..=649 => Weight::SemiBold,
            650..=u16::MAX => Weight::Bold,
        }
    }

    /// True only for full Bold (700), matching TrueType's `OS/2` bold flag.
    pub const fn is_bold(self) -> bool {
        matches!(self, Weight::Bold)
    }

    /// 0 regular, 1 medium, 2 semibold, 3 bold.
    pub const fn index(self) -> usize {
        match self {
            Weight::Regular => 0,
            Weight::Medium => 1,
            Weight::SemiBold => 2,
            Weight::Bold => 3,
        }
    }

    /// Weight corresponding to a legacy [`Style`].
    pub const fn from_style(style: Style) -> Self {
        if style.bold { Weight::Bold } else { Weight::Regular }
    }

    /// Legacy [`Style`] representation (`SemiBold` and `Bold` map to `bold = false/true` respectively via `is_bold()`).
    pub const fn to_style(self, italic: bool) -> Style {
        Style { bold: self.is_bold(), italic }
    }
}

/// Weight and slant of a face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Style { pub bold: bool, pub italic: bool }

impl Style {
    pub const REGULAR: Style = Style { bold: false, italic: false };
    pub const BOLD: Style = Style { bold: true, italic: false };
    pub const ITALIC: Style = Style { bold: false, italic: true };
    pub const BOLD_ITALIC: Style = Style { bold: true, italic: true };
    /// 0 regular, 1 bold, 2 italic, 3 bold italic.
    pub fn index(self) -> usize { self.bold as usize | (self.italic as usize) << 1 }
    /// Equivalent [`Weight`] for this legacy [`Style`].
    pub const fn weight(self) -> Weight { Weight::from_style(self) }
    /// Builds a legacy [`Style`] from [`Weight`] and `italic` (`SemiBold` and `Bold` set `bold = true`).
    pub const fn from_weight(weight: Weight, italic: bool) -> Self {
        Style { bold: matches!(weight, Weight::SemiBold | Weight::Bold), italic }
    }
}

/// The Geist file for a face: Sans or Mono (`mono`) in `style`.
pub fn geist(mono: bool, style: Style) -> &'static [u8] {
    match (mono, style.bold, style.italic) {
        (false, false, false) => GEIST_SANS,
        (false, true, false) => GEIST_SANS_BOLD,
        (false, false, true) => GEIST_SANS_ITALIC,
        (false, true, true) => GEIST_SANS_BOLD_ITALIC,
        (true, false, false) => GEIST_MONO,
        (true, true, false) => GEIST_MONO_BOLD,
        (true, false, true) => GEIST_MONO_ITALIC,
        (true, true, true) => GEIST_MONO_BOLD_ITALIC,
    }
}

/// The Geist file for a face: Sans or Mono (`mono`) in [`Weight`] and `italic`.
pub fn geist_weight(mono: bool, weight: Weight, italic: bool) -> &'static [u8] {
    match (mono, weight, italic) {
        (false, Weight::Regular, false) => GEIST_SANS,
        (false, Weight::Medium, false) => GEIST_SANS_MEDIUM,
        (false, Weight::SemiBold, false) => GEIST_SANS_SEMIBOLD,
        (false, Weight::Bold, false) => GEIST_SANS_BOLD,
        (false, Weight::Regular | Weight::Medium, true) => GEIST_SANS_ITALIC,
        (false, Weight::SemiBold | Weight::Bold, true) => GEIST_SANS_BOLD_ITALIC,
        (true, Weight::Regular, false) => GEIST_MONO,
        (true, Weight::Medium, false) => GEIST_MONO_MEDIUM,
        (true, Weight::SemiBold, false) => GEIST_MONO_SEMIBOLD,
        (true, Weight::Bold, false) => GEIST_MONO_BOLD,
        (true, Weight::Regular | Weight::Medium, true) => GEIST_MONO_ITALIC,
        (true, Weight::SemiBold | Weight::Bold, true) => GEIST_MONO_BOLD_ITALIC,
    }
}

/// Bold and italic made from a regular face, for fonts that have no such faces
/// (the Noto fallbacks): italic leans the outline, bold smears it sideways.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Synthetic {
    /// Horizontal lean per unit of height, in 1/1024 (a 12-degree slant is 218).
    pub slant: i32,
    /// Extra stroke width in 1/1024 of an em (bold is about 32).
    pub embolden: i32,
}

impl Synthetic {
    pub const NONE: Synthetic = Synthetic { slant: 0, embolden: 0 };
    /// What `style` asks of a font that is itself regular.
    pub fn for_style(style: Style) -> Synthetic {
        Synthetic { slant: if style.italic { 218 } else { 0 }, embolden: if style.bold { 32 } else { 0 } }
    }
    /// What `weight` and `italic` ask of a regular fallback font.
    pub fn for_weight(weight: Weight, italic: bool) -> Synthetic {
        let embolden = match weight {
            Weight::Regular => 0,
            Weight::Medium => 10,
            Weight::SemiBold => 21,
            Weight::Bold => 32,
        };
        Synthetic { slant: if italic { 218 } else { 0 }, embolden }
    }
}

/// How a glyph is drawn beyond its bare outline. [`Render::PLAIN`] is the outline
/// as it is (what [`Font::draw`] does, and what the pixel oracles were cut from);
/// [`Render::UI`] is what interface text should use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Render {
    /// Light hinting below this size (1/64 pixel per em; 0 never): the baseline,
    /// x-height and cap height land on whole pixels, vertically only, so advances,
    /// kerning and line breaks stay exactly as unhinted.
    pub hint_below: u32,
    /// Stem darkening: thin strokes at small sizes get up to about a third of a
    /// pixel more weight, centred on the stroke, fading out by [`DARKEN_UNTIL`].
    pub darken: bool,
}

impl Render {
    pub const PLAIN: Render = Render { hint_below: 0, darken: false };
    /// Interface text: hinted up to 32 px, darkened at small sizes.
    pub const UI: Render = Render { hint_below: 32 * 64, darken: true };
    /// Interface text for ink of colour `rgb` (`0xRRGGBB`): darkening only for ink
    /// darker than mid grey. Mixed in linear light ([`gamma::blend`]) dark strokes on
    /// a light ground thin out and need it; light strokes on a dark ground already
    /// gain weight and would smear.
    pub fn ui_for(rgb: u32) -> Render {
        Render { darken: gamma::luminance(rgb) < 2048, ..Render::UI }
    }
}

/// The size (1/64 pixel per em) from which stems are no longer darkened.
pub const DARKEN_UNTIL: u32 = 32 * 64;
/// The most darkening, in 1/256 pixel, reached at 8 px and below.
pub const DARKEN_MAX: i32 = 88;
/// Size threshold (1/64 pixel per em) for subtle CoreText-style UI stem darkening.
pub const SUBTLE_DARKEN_UNTIL: u32 = 20 * 64;
/// Maximum subtle stem darkening, in 1/256 pixel (~7% warmth around 12-16 px).
pub const SUBTLE_DARKEN_MAX: i32 = 24;

/// How much wider stems are drawn at `size`, in 1/256 pixel.
pub fn darkening(size: u32) -> i32 {
    const FROM: u32 = 8 * 64;
    if size >= DARKEN_UNTIL { return 0; }
    let size = size.max(FROM);
    (DARKEN_MAX as i64 * (DARKEN_UNTIL - size) as i64 / (DARKEN_UNTIL - FROM) as i64) as i32
}

/// Subtle Apple CoreText-style stem widening for small UI / dark-mode text, in 1/256 pixel.
pub fn subtle_darkening(size: u32) -> i32 {
    const FROM: u32 = 8 * 64;
    if size >= SUBTLE_DARKEN_UNTIL { return 0; }
    let size = size.max(FROM);
    (SUBTLE_DARKEN_MAX as i64 * (SUBTLE_DARKEN_UNTIL - size) as i64 / (SUBTLE_DARKEN_UNTIL - FROM) as i64) as i32
}

/// Light hinting's vertical map, in 1/256 pixel above the baseline: each zone
/// (descenders and lowercase, x-height to cap height, above the caps) is scaled so
/// its top lands on a whole pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VMap { xh: i32, xh_to: i32, cap: i32, cap_to: i32 }

impl VMap {
    fn map(&self, y: i32) -> i32 {
        let lerp = |v: i32, from: i32, to: i32| (v as i64 * to as i64 / from.max(1) as i64) as i32;
        if y <= self.xh { lerp(y, self.xh, self.xh_to) }
        else if y <= self.cap { self.xh_to + lerp(y - self.xh, self.cap - self.xh, self.cap_to - self.xh_to) }
        else { self.cap_to + lerp(y - self.cap, self.cap, self.cap_to) }
    }
}

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

    /// The advance of a glyph drawn with `synth`: emboldening widens it by the smear.
    pub fn advance_synthetic(&self, glyph: GlyphId, size: u32, synth: Synthetic) -> i32 {
        self.advance(glyph, size) + Self::smear(size, synth)
    }

    /// The emboldening smear in 1/256 pixel.
    fn smear(size: u32, synth: Synthetic) -> i32 { (synth.embolden.max(0) as i64 * size as i64 * 4 / 1024) as i32 }

    /// What the face says of itself (OS/2 and head flags).
    pub fn style(&self) -> Style { Style { bold: self.face.is_bold(), italic: self.face.is_italic() || self.face.is_oblique() } }
    /// Numeric `OS/2` weight class (e.g. 400 Regular, 500 Medium, 600 SemiBold, 700 Bold).
    pub fn weight_number(&self) -> u16 { self.face.weight().to_number() }
    /// Classified weight bucket from the face's `OS/2` weight class.
    pub fn weight(&self) -> Weight { Weight::from_number(self.weight_number()) }
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
        self.draw_synthetic(glyph, size, x, baseline, w, h, acc, out, Synthetic::NONE)
    }

    /// [`Font::draw`] with a synthetic slant and weight. The lean pivots on the
    /// baseline, so ascenders move right and descenders left of the pen; emboldening
    /// draws the outline again at each step of the smear, and the coverage of the
    /// overlapping copies saturates, so strokes thicken and counters narrow.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_synthetic(&self, glyph: GlyphId, size: u32, x: i32, baseline: i32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8], synth: Synthetic) -> bool {
        self.draw_rendered(glyph, size, x, baseline, w, h, acc, out, synth, Render::PLAIN)
    }

    /// [`Font::draw_synthetic`] with hinting and stem darkening as `render` asks.
    /// With [`Render::PLAIN`] the result is bit-identical to `draw_synthetic`.
    /// Hinting moves the baseline to the nearest whole pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rendered(&self, glyph: GlyphId, size: u32, x: i32, baseline: i32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8], synth: Synthetic, render: Render) -> bool {
        if out.len() < w * h || acc.len() < raster::scratch_len(w, h) { return false; }
        let vmap = (size < render.hint_below).then(|| self.vmap(size)).flatten();
        let baseline = if vmap.is_some() { div_round(baseline as i64, FX as i64) as i32 * FX } else { baseline };
        let dark = if render.darken { darkening(size) } else { 0 };
        if dark == 0 {
            return self.pass(glyph, size, x, baseline, w, h, acc, out, synth, vmap, false);
        }
        // The outline drawn twice, a darkening apart and centred on the pen, and
        // the larger coverage kept: every stem gains the darkening, the advance none.
        self.pass(glyph, size, x - dark / 2, baseline, w, h, acc, out, synth, vmap, false)
            && self.pass(glyph, size, x + dark - dark / 2, baseline, w, h, acc, out, synth, vmap, true)
    }

    /// Draws `glyph` with Apple CoreText-style light vertical hinting (below `hint_below`)
    /// and subtle ~7% stem darkening at `<= 16–20 px` so Regular/Medium/SemiBold text
    /// stays warm, crisp, and effortlessly readable on dark macOS Noir backgrounds.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_humanized(&self, glyph: GlyphId, size: u32, x: i32, baseline: i32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8], synth: Synthetic, hint_below: u32) -> bool {
        if out.len() < w * h || acc.len() < raster::scratch_len(w, h) { return false; }
        let vmap = (size < hint_below).then(|| self.vmap(size)).flatten();
        let baseline = if vmap.is_some() { div_round(baseline as i64, FX as i64) as i32 * FX } else { baseline };
        let dark = subtle_darkening(size);
        let ok = if dark == 0 {
            self.pass(glyph, size, x, baseline, w, h, acc, out, synth, vmap, false)
        } else {
            self.pass(glyph, size, x - dark / 2, baseline, w, h, acc, out, synth, vmap, false)
                && self.pass(glyph, size, x + dark - dark / 2, baseline, w, h, acc, out, synth, vmap, true)
        };
        if ok && size <= 16 * 64 {
            let strength = if size <= 12 * 64 { 18 } else { 14 };
            raster::stem_darken_coverage(w, h, out, strength);
        }
        ok
    }

    /// Light hinting's zones at `size`, or `None` when the font has no usable x-height.
    fn vmap(&self, size: u32) -> Option<VMap> {
        let top = |c: u8| self.glyph(c as u32).and_then(|g| self.face.glyph_bounding_box(g)).map(|b| b.y_max);
        let xh = self.face.x_height().filter(|h| *h > 0).or_else(|| top(b'x'))?;
        let cap = self.face.capital_height().filter(|h| *h > xh).or_else(|| top(b'H')).filter(|h| *h > xh)?;
        let snap = |v: i32| (div_round(v as i64, FX as i64) as i32).max(1) * FX;
        let (xh, cap) = (self.fx(xh as i64, size), self.fx(cap as i64, size));
        let xh_to = snap(xh);
        Some(VMap { xh, xh_to, cap, cap_to: snap(cap).max(xh_to + FX) })
    }

    #[allow(clippy::too_many_arguments)]
    fn pass(&self, glyph: GlyphId, size: u32, x: i32, baseline: i32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8], synth: Synthetic, vmap: Option<VMap>, keep_max: bool) -> bool {
        let Some(raster) = Raster::new(w, h, acc) else { return false };
        let smear = Self::smear(size, synth);
        // Copies a quarter pixel apart at most, so the smear stays solid.
        let copies = if smear > 0 { (smear / 64).clamp(1, 16) + 1 } else { 1 };
        let mut pen = Pen { raster, font: self, size, x, baseline, slant: synth.slant, vmap, start: (0, 0), at: (0, 0) };
        for n in 0..copies {
            pen.x = x + if copies > 1 { smear * n / (copies - 1) } else { 0 };
            self.face.outline_glyph(glyph, &mut pen);
            pen.close();
        }
        if keep_max { pen.raster.finish_max(out) } else { pen.raster.finish(out) }
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
    /// Lean in 1/1024 per unit of height.
    slant: i32,
    /// Light hinting's vertical map, when hinted.
    vmap: Option<VMap>,
    start: (i32, i32),
    at: (i32, i32),
}

impl Pen<'_, '_, '_> {
    fn point(&self, x: f32, y: f32) -> (i32, i32) {
        // Sixteenths of a font unit keep composite-glyph offsets exact enough.
        let units = |v: f32| (v * 16.0 + if v < 0.0 { -0.5 } else { 0.5 }) as i64;
        let scale = |v: f32| div_round(units(v) * self.size as i64 * 4, self.font.upem * 16) as i32;
        let (sx, sy) = (scale(x), scale(y));
        let sy = self.vmap.map_or(sy, |m| m.map(sy));
        (self.x + sx + (sy as i64 * self.slant as i64 / 1024) as i32, self.baseline - sy)
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
/// font lacks the scalar. Applies subtle stem darkening at small cell sizes (`<= 19 px`)
/// for crisp Apple-terminal density while keeping Display and pixel oracles identical.
pub fn draw_cell(font: &Font, scalar: u32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
    let Some(glyph) = font.glyph(scalar) else { return false };
    let (size, x, baseline) = cell_layout(font, w, h);
    font.draw_humanized(glyph, size, x, baseline, w, h, acc, out, Synthetic::NONE, 0)
}

/// Bare-outline variant of [`draw_cell`] without subtle stem darkening.
pub fn draw_cell_plain(font: &Font, scalar: u32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
    let Some(glyph) = font.glyph(scalar) else { return false };
    let (size, x, baseline) = cell_layout(font, w, h);
    font.draw(glyph, size, x, baseline, w, h, acc, out)
}

/// [`draw_cell`] for a glyph found by shaping (a ligature's part), drawn `shift` cells to
/// the right of the bitmap: a glyph that reaches into the cells before its own (as
/// coding ligatures do) gives each of them its part this way.
pub fn draw_cell_glyph(font: &Font, glyph: GlyphId, shift: usize, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
    let (size, x, baseline) = cell_layout(font, w, h);
    font.draw(glyph, size, x + (shift * w) as i32 * FX, baseline, w, h, acc, out)
}
