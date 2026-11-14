// SPDX-License-Identifier: GPL-3.0-only
//! The fonts drawn when Geist lacks a character, tried in order: Noto Sans Mono
//! (Latin extended, Greek, Cyrillic, IPA), Noto Sans Symbols 2, Noto Sans CJK SC
//! (a cut of about 14,000 Chinese, Japanese and Korean characters) and the emoji of
//! Noto Color Emoji as shaded 36x36 coverage; Noto Sans Arabic, Hebrew, Devanagari
//! and Thai before the symbols. All SIL Open Font License 1.1
//! (assets/fonts/noto/README.md); scripts/gen_fallback_fonts.py builds them.
//!
//! A grid cell holds one character, so a wide character (an ideograph, an emoji) is
//! drawn fitted into that cell.
use crate::raster::FX;
use crate::Font;

pub static NOTO_MONO: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansMono-Regular.ttf");
pub static NOTO_SYMBOLS: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansSymbols2-Regular.ttf");
pub static NOTO_CJK: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansCJK-SC-Subset.otf");
pub static NOTO_ARABIC: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansArabic-Regular.ttf");
pub static NOTO_HEBREW: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansHebrew-Regular.ttf");
pub static NOTO_DEVANAGARI: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansDevanagari-Regular.ttf");
pub static NOTO_THAI: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansThai-Regular.ttf");
pub static EMOJI: &[u8] = include_bytes!("../../../assets/fonts/noto/emoji-36.sxe");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source { Mono, Symbols, Cjk, Emoji, Script(usize) }

/// Emoji coverage: "SXE1", u16 size, u16 count, sorted u32 scalars, then
/// size*size/2 bytes per emoji (two 4-bit levels a byte, high nibble first).
pub struct Emoji<'a> { size: usize, scalars: &'a [u8], bitmaps: &'a [u8] }
impl<'a> Emoji<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.get(..4)? != b"SXE1" { return None; }
        let size = u16::from_le_bytes([data[4], data[5]]) as usize;
        let count = u16::from_le_bytes([data[6], data[7]]) as usize;
        let scalars = data.get(8..8 + count * 4)?;
        let bitmaps = data.get(8 + count * 4..8 + count * 4 + count * size * size / 2)?;
        (size > 1 && size % 2 == 0).then_some(Self { size, scalars, bitmaps })
    }
    fn index(&self, scalar: u32) -> Option<usize> {
        let (mut lo, mut hi) = (0, self.scalars.len() / 4);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let s = u32::from_le_bytes(self.scalars[mid * 4..mid * 4 + 4].try_into().ok()?);
            if s == scalar { return Some(mid); }
            if s < scalar { lo = mid + 1 } else { hi = mid }
        }
        None
    }
    pub fn has(&self, scalar: u32) -> bool { self.index(scalar).is_some() }
    /// Source level 0..=255 at (x, y) in the emoji's own pixels.
    fn level(&self, bitmap: &[u8], x: usize, y: usize) -> u32 {
        let i = y * self.size + x;
        let b = bitmap[i / 2];
        let nibble = if i % 2 == 0 { b >> 4 } else { b & 15 };
        nibble as u32 * 17
    }
    /// The emoji as a square as large as the cell's smaller side, centred, scaled
    /// bilinearly. False if there is no such emoji.
    pub fn draw_cell(&self, scalar: u32, w: usize, h: usize, out: &mut [u8]) -> bool {
        let Some(i) = self.index(scalar) else { return false };
        if out.len() < w * h { return false; }
        let n = self.size * self.size / 2;
        let bitmap = &self.bitmaps[i * n..(i + 1) * n];
        out[..w * h].fill(0);
        let side = w.min(h);
        if side == 0 { return true; }
        let (ox, oy) = ((w - side) / 2, (h - side) / 2);
        let s = self.size as u32;
        for y in 0..side {
            // Pixel centres in 1/256 source pixel.
            let sy = ((2 * y as u32 + 1) * s * 128 / side as u32).saturating_sub(128).min((s - 1) * 256);
            let (y0, fy) = ((sy / 256) as usize, sy % 256);
            let y1 = (y0 + 1).min(self.size - 1);
            for x in 0..side {
                let sx = ((2 * x as u32 + 1) * s * 128 / side as u32).saturating_sub(128).min((s - 1) * 256);
                let (x0, fx) = ((sx / 256) as usize, sx % 256);
                let x1 = (x0 + 1).min(self.size - 1);
                let top = self.level(bitmap, x0, y0) * (256 - fx) + self.level(bitmap, x1, y0) * fx;
                let bottom = self.level(bitmap, x0, y1) * (256 - fx) + self.level(bitmap, x1, y1) * fx;
                out[(oy + y) * w + ox + x] = ((top * (256 - fy) + bottom * fy) >> 16) as u8;
            }
        }
        true
    }
}

/// Arabic (with its presentation forms), Hebrew, Devanagari, Thai.
const SCRIPTS: usize = 4;
pub struct Fallback<'a> { mono: Font<'a>, symbols: Font<'a>, cjk: Font<'a>, emoji: Emoji<'a>, scripts: [Font<'a>; SCRIPTS] }

impl Fallback<'static> {
    /// The embedded fonts, parsed.
    pub fn embedded() -> Option<Self> {
        Some(Self {
            mono: Font::new(NOTO_MONO)?, symbols: Font::new(NOTO_SYMBOLS)?, cjk: Font::new(NOTO_CJK)?, emoji: Emoji::new(EMOJI)?,
            scripts: [Font::new(NOTO_ARABIC)?, Font::new(NOTO_HEBREW)?, Font::new(NOTO_DEVANAGARI)?, Font::new(NOTO_THAI)?],
        })
    }

    /// [`Self::embedded`] written straight into `slot`, one font at a time: the whole
    /// set is about 12 KB, too much to build on a small stack and then copy.
    /// Returns whether every font parsed; on `false`, `slot` holds nothing.
    pub fn embedded_into(slot: &mut core::mem::MaybeUninit<Self>) -> bool {
        let p = slot.as_mut_ptr();
        // SAFETY: each field is written exactly once through a raw pointer into
        // `slot`; the caller treats `slot` as initialised only when this returns true.
        unsafe {
            let fonts: [(&'static [u8], *mut Font<'static>); 7] = [
                (NOTO_MONO, core::ptr::addr_of_mut!((*p).mono)),
                (NOTO_SYMBOLS, core::ptr::addr_of_mut!((*p).symbols)),
                (NOTO_CJK, core::ptr::addr_of_mut!((*p).cjk)),
                (NOTO_ARABIC, core::ptr::addr_of_mut!((*p).scripts[0])),
                (NOTO_HEBREW, core::ptr::addr_of_mut!((*p).scripts[1])),
                (NOTO_DEVANAGARI, core::ptr::addr_of_mut!((*p).scripts[2])),
                (NOTO_THAI, core::ptr::addr_of_mut!((*p).scripts[3])),
            ];
            for (data, field) in fonts {
                match Font::new(data) { Some(f) => field.write(f), None => return false }
            }
            match Emoji::new(EMOJI) { Some(e) => core::ptr::addr_of_mut!((*p).emoji).write(e), None => return false }
        }
        true
    }
}

impl<'a> Fallback<'a> {
    /// Which fallback draws `scalar`, if any does. Emoji come before the symbol
    /// font so a smiley is a face, not a dingbat.
    pub fn source(&self, scalar: u32) -> Option<Source> {
        if self.mono.glyph(scalar).is_some() { Some(Source::Mono) }
        else if self.emoji.has(scalar) { Some(Source::Emoji) }
        else if let Some(i) = self.scripts.iter().position(|f| f.glyph(scalar).is_some()) { Some(Source::Script(i)) }
        else if self.symbols.glyph(scalar).is_some() { Some(Source::Symbols) }
        else if self.cjk.glyph(scalar).is_some() { Some(Source::Cjk) }
        else { None }
    }

    /// For proportional text: the outline font (by a small id, 1 and up) that has
    /// `scalar`, in fallback order. Emoji are not outlines: see [`Self::emoji`].
    pub fn outline(&self, scalar: u32) -> Option<(u8, &Font<'a>)> {
        match self.source(scalar)? {
            Source::Mono => Some((1, &self.mono)),
            Source::Symbols => Some((2, &self.symbols)),
            Source::Cjk => Some((3, &self.cjk)),
            Source::Script(i) => Some((4 + i as u8, &self.scripts[i])),
            Source::Emoji => None,
        }
    }
    /// The outline font with this id (see [`Self::outline`]).
    pub fn font(&self, id: u8) -> Option<&Font<'a>> {
        match id { 1 => Some(&self.mono), 2 => Some(&self.symbols), 3 => Some(&self.cjk), 4.. => self.scripts.get(id as usize - 4), _ => None }
    }
    pub fn emoji(&self) -> &Emoji<'a> { &self.emoji }

    /// One grid cell for `scalar` from the first fallback that has it.
    pub fn draw_cell(&self, scalar: u32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
        match self.source(scalar) {
            Some(Source::Mono) => crate::draw_cell(&self.mono, scalar, w, h, acc, out),
            Some(Source::Emoji) => self.emoji.draw_cell(scalar, w, h, out),
            Some(Source::Symbols) => fitted(&self.symbols, scalar, w, h, acc, out),
            Some(Source::Cjk) => fitted(&self.cjk, scalar, w, h, acc, out),
            Some(Source::Script(i)) => fitted(&self.scripts[i], scalar, w, h, acc, out),
            None => false,
        }
    }
}

/// A glyph whose em square is drawn as large as fits the cell, centred.
fn fitted(font: &Font, scalar: u32, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
    let Some(glyph) = font.glyph(scalar) else { return false };
    let px = (w as i64 * 64).min(h as i64 * 64 * 85 / 100).max(64) as u32;
    let advance = font.advance(glyph, px).min(w as i32 * FX);
    let x = (w as i32 * FX - advance) / 2;
    let (asc, desc) = (font.ascender(px), font.descender(px));
    let baseline = (h as i32 * FX - (asc - desc)) / 2 + asc;
    font.draw(glyph, px, x, baseline, w, h, acc, out)
}
