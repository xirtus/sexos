// SPDX-License-Identifier: GPL-3.0-only
//! Complex OpenType shaping and font fallback cascade.
//!
//! Maps itemized text runs to appropriate fonts in the fallback cascade, invokes
//! HarfBuzz-grade shaping with `rustybuzz` for complex scripts (Arabic, Hebrew,
//! Indic, Thai, Latin ligatures/kerning), and computes em-scaled glyph advances.

extern crate alloc;

use alloc::vec::Vec;
use rustybuzz::{Direction as RbDirection, Face as RbFace, Feature, UnicodeBuffer};
use ttf_parser::Tag;
use sex_font::fallback::{
    NOTO_ARABIC, NOTO_CJK, NOTO_DEVANAGARI, NOTO_HEBREW, NOTO_MONO, NOTO_SYMBOLS, NOTO_THAI,
};
use sex_font::{Font, GEIST_MONO, GEIST_SANS};
use unicode_script::Script;

use crate::bidi::Direction;
use crate::emoji::is_emoji_char;
use crate::itemize::ItemRun;

pub const FONT_ID_PRIMARY: usize = 0;
pub const FONT_ID_MONO: usize = 1;
pub const FONT_ID_ARABIC: usize = 2;
pub const FONT_ID_HEBREW: usize = 3;
pub const FONT_ID_DEVANAGARI: usize = 4;
pub const FONT_ID_THAI: usize = 5;
pub const FONT_ID_CJK: usize = 6;
pub const FONT_ID_SYMBOLS: usize = 7;
pub const FONT_ID_NOTO_MONO: usize = 8;
pub const FONT_ID_EMOJI: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub font_id: usize,
    pub glyph_id: u16,
    pub cluster: u32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub is_emoji: bool,
    pub scalar: u32,
}

pub struct Shaper {
    primary_data: &'static [u8],
    primary_font: Font<'static>,
    mono_font: Font<'static>,
    arabic_font: Font<'static>,
    hebrew_font: Font<'static>,
    devanagari_font: Font<'static>,
    thai_font: Font<'static>,
    cjk_font: Font<'static>,
    symbols_font: Font<'static>,
    noto_mono_font: Font<'static>,
    custom_fonts: Vec<(&'static str, &'static [u8], Font<'static>)>,
}

impl Default for Shaper {
    fn default() -> Self {
        Self::new(GEIST_SANS)
    }
}

impl Shaper {
    pub fn new(primary_font_data: &'static [u8]) -> Self {
        let primary_font = Font::new(primary_font_data)
            .or_else(|| Font::new(GEIST_SANS))
            .expect("Primary font must parse");
        let mono_font = Font::new(GEIST_MONO).expect("GEIST_MONO must parse");
        let arabic_font = Font::new(NOTO_ARABIC).expect("NOTO_ARABIC must parse");
        let hebrew_font = Font::new(NOTO_HEBREW).expect("NOTO_HEBREW must parse");
        let devanagari_font = Font::new(NOTO_DEVANAGARI).expect("NOTO_DEVANAGARI must parse");
        let thai_font = Font::new(NOTO_THAI).expect("NOTO_THAI must parse");
        let cjk_font = Font::new(NOTO_CJK).expect("NOTO_CJK must parse");
        let symbols_font = Font::new(NOTO_SYMBOLS).expect("NOTO_SYMBOLS must parse");
        let noto_mono_font = Font::new(NOTO_MONO).expect("NOTO_MONO must parse");

        Self {
            primary_data: primary_font_data,
            primary_font,
            mono_font,
            arabic_font,
            hebrew_font,
            devanagari_font,
            thai_font,
            cjk_font,
            symbols_font,
            noto_mono_font,
            custom_fonts: Vec::new(),
        }
    }

    pub fn new_mono() -> Self {
        Self::new(GEIST_MONO)
    }

    pub fn register_custom_font(&mut self, name: &'static str, data: &'static [u8]) -> Option<usize> {
        let f = Font::new(data)?;
        let id = 10 + self.custom_fonts.len();
        self.custom_fonts.push((name, data, f));
        Some(id)
    }

    pub fn font_by_id(&self, id: usize) -> Option<&Font<'static>> {
        match id {
            FONT_ID_PRIMARY => Some(&self.primary_font),
            FONT_ID_MONO => Some(&self.mono_font),
            FONT_ID_ARABIC => Some(&self.arabic_font),
            FONT_ID_HEBREW => Some(&self.hebrew_font),
            FONT_ID_DEVANAGARI => Some(&self.devanagari_font),
            FONT_ID_THAI => Some(&self.thai_font),
            FONT_ID_CJK => Some(&self.cjk_font),
            FONT_ID_SYMBOLS => Some(&self.symbols_font),
            FONT_ID_NOTO_MONO => Some(&self.noto_mono_font),
            n if n >= 10 && n - 10 < self.custom_fonts.len() => Some(&self.custom_fonts[n - 10].2),
            _ => None,
        }
    }

    pub fn font_data_by_id(&self, id: usize) -> Option<&'static [u8]> {
        match id {
            FONT_ID_PRIMARY => Some(self.primary_data),
            FONT_ID_MONO => Some(GEIST_MONO),
            FONT_ID_ARABIC => Some(NOTO_ARABIC),
            FONT_ID_HEBREW => Some(NOTO_HEBREW),
            FONT_ID_DEVANAGARI => Some(NOTO_DEVANAGARI),
            FONT_ID_THAI => Some(NOTO_THAI),
            FONT_ID_CJK => Some(NOTO_CJK),
            FONT_ID_SYMBOLS => Some(NOTO_SYMBOLS),
            FONT_ID_NOTO_MONO => Some(NOTO_MONO),
            n if n >= 10 && n - 10 < self.custom_fonts.len() => Some(self.custom_fonts[n - 10].1),
            _ => None,
        }
    }

    /// Resolves the fallback font ID and font for character `c` given its script.
    pub fn resolve_font_for_char(&self, c: char, script: Script) -> (usize, &'static [u8], &Font<'static>) {
        let s = c as u32;

        if is_emoji_char(c) {
            return (
                FONT_ID_EMOJI,
                &[],
                self.font_by_id(FONT_ID_PRIMARY).unwrap_or(&self.primary_font),
            );
        }

        // Script-specific preferred fonts
        let candidate_id = match script {
            Script::Arabic => Some(FONT_ID_ARABIC),
            Script::Hebrew => Some(FONT_ID_HEBREW),
            Script::Devanagari => Some(FONT_ID_DEVANAGARI),
            Script::Thai => Some(FONT_ID_THAI),
            Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul => Some(FONT_ID_CJK),
            _ => None,
        };

        if let Some(id) = candidate_id {
            if let (Some(f), Some(data)) = (self.font_by_id(id), self.font_data_by_id(id)) {
                if f.glyph(s).is_some() {
                    return (id, data, f);
                }
            }
        }

        // Check primary font
        if self.primary_font.glyph(s).is_some() {
            return (FONT_ID_PRIMARY, self.primary_data, &self.primary_font);
        }

        // Check custom registered fonts
        for (i, (_, data, f)) in self.custom_fonts.iter().enumerate() {
            if f.glyph(s).is_some() {
                return (10 + i, data, f);
            }
        }

        // Fallback cascade: CJK -> Symbols -> Noto Mono -> Primary fallback
        for id in [FONT_ID_CJK, FONT_ID_SYMBOLS, FONT_ID_NOTO_MONO, FONT_ID_PRIMARY] {
            if let (Some(f), Some(data)) = (self.font_by_id(id), self.font_data_by_id(id)) {
                if f.glyph(s).is_some() {
                    return (id, data, f);
                }
            }
        }

        (
            FONT_ID_PRIMARY,
            self.primary_data,
            self.font_by_id(FONT_ID_PRIMARY).unwrap_or(&self.primary_font),
        )
    }

    /// Shapes an itemized run of text into positioned glyphs.
    pub fn shape_run(&self, run: &ItemRun<'_>, font_size_px: f32) -> Vec<ShapedGlyph> {
        let text = run.text;
        if text.is_empty() {
            return Vec::new();
        }

        let font_size_px = if font_size_px.is_finite() && font_size_px > 0.0 {
            font_size_px.clamp(1.0, 512.0)
        } else {
            16.0
        };

        // If run is pure emoji, return direct emoji glyphs
        if text.chars().all(is_emoji_char) {
            let mut glyphs = Vec::new();
            for (byte_offset, c) in text.char_indices() {
                glyphs.push(ShapedGlyph {
                    font_id: FONT_ID_EMOJI,
                    glyph_id: 0,
                    cluster: (run.byte_range.start + byte_offset) as u32,
                    x_advance: font_size_px,
                    y_advance: 0.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                    is_emoji: true,
                    scalar: c as u32,
                });
            }
            return glyphs;
        }

        // Resolve font for the first character of the run
        let first_char = text.chars().next().unwrap_or(' ');
        let (font_id, font_data, _font) = self.resolve_font_for_char(first_char, run.script);

        // Try OpenType shaping via rustybuzz
        if let Some(rb_face) = RbFace::from_slice(font_data, 0) {
            let mut buf = UnicodeBuffer::new();
            buf.push_str(text);
            buf.set_direction(if run.direction == Direction::Rtl {
                RbDirection::RightToLeft
            } else {
                RbDirection::LeftToRight
            });

            if let Some(tag) = run.script.short_name().as_bytes().try_into().ok().map(Tag::from_bytes) {
                if let Some(rb_script) = rustybuzz::Script::from_iso15924_tag(tag) {
                    buf.set_script(rb_script);
                }
            }

            let features = [Feature::new(Tag::from_bytes(b"kern"), 1, ..)];
            let shaped = rustybuzz::shape(&rb_face, &features, buf);
            let upem = rb_face.units_per_em() as f32;
            let scale = if upem > 0.0 { font_size_px / upem } else { 1.0 };

            let glyph_infos = shaped.glyph_infos();
            let glyph_positions = shaped.glyph_positions();
            let mut out = Vec::with_capacity(glyph_infos.len());

            for (info, pos) in glyph_infos.iter().zip(glyph_positions) {
                let char_at_cluster = text
                    .get(info.cluster as usize..)
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                let is_emoji = is_emoji_char(char_at_cluster);

                out.push(ShapedGlyph {
                    font_id,
                    glyph_id: info.glyph_id as u16,
                    cluster: (run.byte_range.start + info.cluster as usize) as u32,
                    x_advance: if is_emoji { font_size_px } else { pos.x_advance as f32 * scale },
                    y_advance: pos.y_advance as f32 * scale,
                    x_offset: pos.x_offset as f32 * scale,
                    y_offset: pos.y_offset as f32 * scale,
                    is_emoji,
                    scalar: char_at_cluster as u32,
                });
            }

            return out;
        }

        // Fallback: per-character simple layout with sex_font kerning
        let mut out = Vec::new();
        let size_units = (font_size_px * 64.0 + 0.5) as u32;
        let mut prev_glyph = None;

        for (byte_offset, c) in text.char_indices() {
            let (f_id, _, f) = self.resolve_font_for_char(c, run.script);
            let is_emoji = is_emoji_char(c);

            let gid = f.glyph(c as u32).unwrap_or(sex_font::GlyphId(0));
            let mut adv = if is_emoji {
                font_size_px
            } else {
                f.advance(gid, size_units) as f32 / 256.0
            };

            if let Some((p_id, p_gid)) = prev_glyph {
                if p_id == f_id && !is_emoji {
                    let kern = f.kerning(p_gid, gid, size_units) as f32 / 256.0;
                    adv += kern;
                }
            }

            out.push(ShapedGlyph {
                font_id: f_id,
                glyph_id: gid.0,
                cluster: (run.byte_range.start + byte_offset) as u32,
                x_advance: adv,
                y_advance: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
                is_emoji,
                scalar: c as u32,
            });

            prev_glyph = Some((f_id, gid));
        }

        out
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::bidi::{BaseDirection, Paragraph};
    use crate::itemize::itemize_paragraph;

    #[test]
    fn test_shape_arabic_lam_alef() {
        let shaper = Shaper::default();
        let text = "لا"; // Lam + Alef ligature
        let paras = Paragraph::split_and_resolve(text, BaseDirection::Auto);
        let items = itemize_paragraph(&paras[0]);
        let shaped = shaper.shape_run(&items[0], 16.0);
        assert!(!shaped.is_empty());
        assert_eq!(shaped[0].font_id, FONT_ID_ARABIC);
    }

    #[test]
    fn test_shape_emoji_fallback() {
        let shaper = Shaper::default();
        let text = "🚀";
        let paras = Paragraph::split_and_resolve(text, BaseDirection::Auto);
        let items = itemize_paragraph(&paras[0]);
        let shaped = shaper.shape_run(&items[0], 24.0);
        assert_eq!(shaped.len(), 1);
        assert!(shaped[0].is_emoji);
        assert_eq!(shaped[0].x_advance, 24.0);
    }
}
