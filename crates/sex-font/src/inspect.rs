// SPDX-License-Identifier: GPL-3.0-only
//! OpenType/TrueType metadata inspection for SexOS (`OmaFont` / Filigree parity).

use alloc::string::String;
use alloc::vec::Vec;
use ttf_parser::name::{name_id, Name, PlatformId};
use ttf_parser::{Face, Tag};

/// Coarse script / symbol buckets tracked in [`UnicodeCoverage`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScriptRange {
    BasicLatin,
    LatinExt,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Devanagari,
    Cjk,
    Hangul,
    MathSymbols,
    BoxDrawing,
    Emoji,
}

impl ScriptRange {
    /// All tracked script buckets in canonical order.
    pub const ALL: [ScriptRange; 12] = [
        ScriptRange::BasicLatin,
        ScriptRange::LatinExt,
        ScriptRange::Cyrillic,
        ScriptRange::Greek,
        ScriptRange::Arabic,
        ScriptRange::Hebrew,
        ScriptRange::Devanagari,
        ScriptRange::Cjk,
        ScriptRange::Hangul,
        ScriptRange::MathSymbols,
        ScriptRange::BoxDrawing,
        ScriptRange::Emoji,
    ];

    /// Single-bit mask for this script range.
    pub const fn bit(self) -> u32 {
        match self {
            ScriptRange::BasicLatin => UnicodeCoverage::BASIC_LATIN,
            ScriptRange::LatinExt => UnicodeCoverage::LATIN_EXT,
            ScriptRange::Cyrillic => UnicodeCoverage::CYRILLIC,
            ScriptRange::Greek => UnicodeCoverage::GREEK,
            ScriptRange::Arabic => UnicodeCoverage::ARABIC,
            ScriptRange::Hebrew => UnicodeCoverage::HEBREW,
            ScriptRange::Devanagari => UnicodeCoverage::DEVANAGARI,
            ScriptRange::Cjk => UnicodeCoverage::CJK,
            ScriptRange::Hangul => UnicodeCoverage::HANGUL,
            ScriptRange::MathSymbols => UnicodeCoverage::MATH_SYMBOLS,
            ScriptRange::BoxDrawing => UnicodeCoverage::BOX_DRAWING,
            ScriptRange::Emoji => UnicodeCoverage::EMOJI,
        }
    }

    /// Human-readable label for this script bucket.
    pub const fn name(self) -> &'static str {
        match self {
            ScriptRange::BasicLatin => "BasicLatin",
            ScriptRange::LatinExt => "LatinExt",
            ScriptRange::Cyrillic => "Cyrillic",
            ScriptRange::Greek => "Greek",
            ScriptRange::Arabic => "Arabic",
            ScriptRange::Hebrew => "Hebrew",
            ScriptRange::Devanagari => "Devanagari",
            ScriptRange::Cjk => "Cjk",
            ScriptRange::Hangul => "Hangul",
            ScriptRange::MathSymbols => "MathSymbols",
            ScriptRange::BoxDrawing => "BoxDrawing",
            ScriptRange::Emoji => "Emoji",
        }
    }
}

/// Bitmask of Unicode script and symbol coverage derived from the font's `cmap`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct UnicodeCoverage(u32);

impl UnicodeCoverage {
    pub const BASIC_LATIN: u32 = 1 << 0;
    pub const LATIN_EXT: u32 = 1 << 1;
    pub const CYRILLIC: u32 = 1 << 2;
    pub const GREEK: u32 = 1 << 3;
    pub const ARABIC: u32 = 1 << 4;
    pub const HEBREW: u32 = 1 << 5;
    pub const DEVANAGARI: u32 = 1 << 6;
    pub const CJK: u32 = 1 << 7;
    pub const HANGUL: u32 = 1 << 8;
    pub const MATH_SYMBOLS: u32 = 1 << 9;
    pub const BOX_DRAWING: u32 = 1 << 10;
    pub const EMOJI: u32 = 1 << 11;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn insert(&mut self, script: ScriptRange) {
        self.0 |= script.bit();
    }

    pub const fn has(self, script: ScriptRange) -> bool {
        (self.0 & script.bit()) != 0
    }

    pub const fn contains_all(self, required: UnicodeCoverage) -> bool {
        (self.0 & required.0) == required.0
    }

    pub fn active_scripts(self) -> Vec<ScriptRange> {
        ScriptRange::ALL
            .iter()
            .copied()
            .filter(|&s| self.has(s))
            .collect()
    }

    pub fn script_names(self) -> Vec<&'static str> {
        ScriptRange::ALL
            .iter()
            .copied()
            .filter(|&s| self.has(s))
            .map(ScriptRange::name)
            .collect()
    }
}

/// High-level typographic classification derived from PANOSE and face flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontClass {
    Serif,
    Sans,
    Script,
    Decorative,
    Monospace,
    Unknown,
}

impl FontClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            FontClass::Serif => "serif",
            FontClass::Sans => "sans",
            FontClass::Script => "script",
            FontClass::Decorative => "decorative",
            FontClass::Monospace => "monospace",
            FontClass::Unknown => "unknown",
        }
    }
}

/// 10-byte `OS/2` PANOSE classification descriptor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Panose {
    pub bytes: [u8; 10],
}

impl Panose {
    pub const fn new(bytes: [u8; 10]) -> Self {
        Self { bytes }
    }

    pub const fn family_type(self) -> u8 {
        self.bytes[0]
    }

    pub const fn serif_style(self) -> u8 {
        self.bytes[1]
    }

    pub const fn weight(self) -> u8 {
        self.bytes[2]
    }

    pub const fn proportion(self) -> u8 {
        self.bytes[3]
    }

    pub const fn contrast(self) -> u8 {
        self.bytes[4]
    }

    pub const fn stroke_variation(self) -> u8 {
        self.bytes[5]
    }

    pub const fn arm_style(self) -> u8 {
        self.bytes[6]
    }

    pub const fn letterform(self) -> u8 {
        self.bytes[7]
    }

    pub const fn midline(self) -> u8 {
        self.bytes[8]
    }

    pub const fn x_height(self) -> u8 {
        self.bytes[9]
    }

    /// True when PANOSE designates Latin Text & Display with Monospaced proportion (`9`).
    pub const fn is_monospace(self) -> bool {
        self.bytes[0] == 2 && self.bytes[3] == 9
    }

    /// Classify from PANOSE fields alone (`serif`, `sans`, `script`, `decorative`, `monospace`).
    pub const fn classification(self) -> FontClass {
        match self.bytes[0] {
            // Latin Text & Display
            2 => {
                if self.bytes[3] == 9 {
                    FontClass::Monospace
                } else {
                    match self.bytes[1] {
                        2..=10 => FontClass::Serif,
                        11..=15 => FontClass::Sans,
                        _ => FontClass::Unknown,
                    }
                }
            }
            // Latin Hand Written / Script
            3 => FontClass::Script,
            // Latin Decorative (and Pictorial symbols)
            4 | 5 => FontClass::Decorative,
            _ => FontClass::Unknown,
        }
    }
}

/// Extracted OpenType/TrueType metadata for one font face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontMetadata {
    pub face_index: u32,
    /// Typographic family (`name` ID 16) with fallback to legacy family (ID 1).
    pub family: String,
    /// Typographic subfamily (`name` ID 17) with fallback to legacy subfamily (ID 2).
    pub subfamily: String,
    /// Legacy family (`name` ID 1), if present.
    pub legacy_family: String,
    /// Legacy subfamily (`name` ID 2), if present.
    pub legacy_subfamily: String,
    pub postscript_name: Option<String>,
    pub designer: Option<String>,
    pub manufacturer: Option<String>,
    pub license: Option<String>,
    pub version: Option<String>,
    /// `OS/2` weight class normalized to `100..=900`.
    pub weight: u16,
    /// `OS/2` width class (`1..=9`).
    pub width_class: u8,
    pub is_italic: bool,
    pub is_oblique: bool,
    pub is_bold: bool,
    pub is_monospace: bool,
    pub is_variable: bool,
    pub panose: Panose,
    pub glyph_count: u16,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub x_height: Option<i16>,
    pub cap_height: Option<i16>,
    pub coverage: UnicodeCoverage,
}

impl FontMetadata {
    /// Best-effort typographic classification combining PANOSE, monospace flags, and family name.
    pub fn classification(&self) -> FontClass {
        if self.is_monospace || self.panose.is_monospace() {
            return FontClass::Monospace;
        }
        let by_panose = self.panose.classification();
        if by_panose != FontClass::Unknown {
            return by_panose;
        }
        let lower = self.family.to_ascii_lowercase();
        if lower.contains("mono") || lower.contains("code") || lower.contains("console") {
            FontClass::Monospace
        } else if lower.contains("script") || lower.contains("cursive") || lower.contains("hand") {
            FontClass::Script
        } else if lower.contains("display") || lower.contains("decor") || lower.contains("stencil") {
            FontClass::Decorative
        } else if lower.contains("serif") && !lower.contains("sans") {
            FontClass::Serif
        } else if lower.contains("sans") {
            FontClass::Sans
        } else {
            FontClass::Unknown
        }
    }
}

/// Number of faces inside a `.ttc` / `.otc` font collection, or `None` for non-collection bytes.
pub fn font_collection_count(bytes: &[u8]) -> Option<u32> {
    ttf_parser::fonts_in_collection(bytes)
}

/// Inspect OpenType/TrueType metadata for `face_index` inside `bytes`.
pub fn inspect_font(bytes: &[u8], face_index: u32) -> Option<FontMetadata> {
    let face = Face::parse(bytes, face_index).ok()?;
    let upem = face.units_per_em();
    if upem == 0 {
        return None;
    }

    let legacy_family = read_name(&face, name_id::FAMILY).unwrap_or_default();
    let legacy_subfamily = read_name(&face, name_id::SUBFAMILY).unwrap_or_default();
    let family = read_name(&face, name_id::TYPOGRAPHIC_FAMILY)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| legacy_family.clone());
    let subfamily = read_name(&face, name_id::TYPOGRAPHIC_SUBFAMILY)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| legacy_subfamily.clone());

    let postscript_name = read_name(&face, name_id::POST_SCRIPT_NAME);
    let designer = read_name(&face, name_id::DESIGNER);
    let manufacturer = read_name(&face, name_id::MANUFACTURER);
    let license = read_name(&face, name_id::LICENSE);
    let version = read_name(&face, name_id::VERSION);

    let raw_weight = face.weight().to_number();
    let weight = if raw_weight == 0 {
        400
    } else {
        raw_weight.clamp(100, 900)
    };
    let width_class = (face.width().to_number() as u8).clamp(1, 9);

    let panose = read_panose(&face);
    let is_monospace = face.is_monospaced() || panose.is_monospace();
    let is_variable = face
        .raw_face()
        .table(Tag::from_bytes(b"fvar"))
        .is_some_and(|d| d.len() >= 16 && u16::from_be_bytes([d[8], d[9]]) > 0);

    let coverage = scan_unicode_coverage(&face);

    Some(FontMetadata {
        face_index,
        family,
        subfamily,
        legacy_family,
        legacy_subfamily,
        postscript_name,
        designer,
        manufacturer,
        license,
        version,
        weight,
        width_class,
        is_italic: face.is_italic(),
        is_oblique: face.is_oblique(),
        is_bold: face.is_bold(),
        is_monospace,
        is_variable,
        panose,
        glyph_count: face.number_of_glyphs(),
        units_per_em: upem,
        ascender: face.ascender(),
        descender: face.descender(),
        line_gap: face.line_gap(),
        x_height: face.x_height(),
        cap_height: face.capital_height(),
        coverage,
    })
}

fn read_panose(face: &Face<'_>) -> Panose {
    // OS/2 panose[10] sits at byte offsets 32..42 (present in all OS/2 versions 0..=5).
    if let Some(os2) = face.raw_face().table(Tag::from_bytes(b"OS/2")) {
        if let Some(slice) = os2.get(32..42) {
            if let Ok(bytes) = <[u8; 10]>::try_from(slice) {
                return Panose::new(bytes);
            }
        }
    }
    Panose::default()
}

fn read_name(face: &Face<'_>, id: u16) -> Option<String> {
    let mut best: Option<(u8, String)> = None;
    for rec in face.names() {
        if rec.name_id != id {
            continue;
        }
        let rank = name_priority(&rec);
        if rank == 0 {
            continue;
        }
        if best.as_ref().is_some_and(|(r, _)| *r >= rank) {
            continue;
        }
        if let Some(decoded) = decode_name_record(&rec) {
            let trimmed = decoded.trim();
            if !trimmed.is_empty() {
                best = Some((rank, String::from(trimmed)));
                if rank == 3 {
                    break;
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

fn name_priority(rec: &Name<'_>) -> u8 {
    if rec.is_unicode() {
        if rec.platform_id == PlatformId::Windows && rec.language_id == 0x0409 {
            3
        } else {
            2
        }
    } else if rec.platform_id == PlatformId::Macintosh && rec.encoding_id == 0 {
        1
    } else {
        0
    }
}

fn decode_name_record(rec: &Name<'_>) -> Option<String> {
    if rec.is_unicode() {
        if rec.name.len() % 2 != 0 {
            return None;
        }
        let units = rec
            .name
            .chunks_exact(2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]));
        let mut out = String::new();
        for ch in core::char::decode_utf16(units) {
            let c = ch.unwrap_or('\u{FFFD}');
            if c != '\0' {
                out.push(c);
            }
        }
        Some(out)
    } else if rec.platform_id == PlatformId::Macintosh && rec.encoding_id == 0 {
        let mut out = String::with_capacity(rec.name.len());
        for &b in rec.name {
            if b == 0 {
                continue;
            }
            out.push(decode_mac_roman(b));
        }
        Some(out)
    } else {
        None
    }
}

const MAC_ROMAN_HIGH: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è',
    'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü',
    '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø',
    '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø',
    '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{00A0}', 'À', 'Ã', 'Õ', 'Œ', 'œ',
    '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ',
    '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô',
    '\u{F8FF}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
];

fn decode_mac_roman(b: u8) -> char {
    if b < 0x80 {
        b as char
    } else {
        MAC_ROMAN_HIGH[(b - 0x80) as usize]
    }
}

fn classify_codepoint_script(cp: u32) -> Option<ScriptRange> {
    match cp {
        0x0020..=0x007E => Some(ScriptRange::BasicLatin),
        0x0080..=0x024F | 0x1E00..=0x1EFF => Some(ScriptRange::LatinExt),
        0x0400..=0x052F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Some(ScriptRange::Cyrillic),
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Some(ScriptRange::Greek),
        0x0600..=0x06FF
        | 0x0750..=0x077F
        | 0x08A0..=0x08FF
        | 0xFB50..=0xFDFF
        | 0xFE70..=0xFEFF => Some(ScriptRange::Arabic),
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Some(ScriptRange::Hebrew),
        0x0900..=0x097F | 0xA8E0..=0xA8FF => Some(ScriptRange::Devanagari),
        0x2E80..=0x2FDF
        | 0x3000..=0x30FF
        | 0x3190..=0x319F
        | 0x31C0..=0x31EF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0x20000..=0x2A6DF => Some(ScriptRange::Cjk),
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Some(ScriptRange::Hangul),
        0x2200..=0x22FF | 0x27C0..=0x27EF | 0x2980..=0x2AFF => Some(ScriptRange::MathSymbols),
        0x2500..=0x257F => Some(ScriptRange::BoxDrawing),
        0x2600..=0x27BF | 0x1F300..=0x1FAFF => Some(ScriptRange::Emoji),
        _ => None,
    }
}

fn scan_unicode_coverage(face: &Face<'_>) -> UnicodeCoverage {
    let mut coverage = UnicodeCoverage::empty();
    let Some(cmap) = face.tables().cmap else {
        return coverage;
    };
    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|cp| {
            if let Some(script) = classify_codepoint_script(cp) {
                if !coverage.has(script) && subtable.glyph_index(cp).is_some() {
                    coverage.insert(script);
                }
            }
        });
    }
    coverage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fallback::{NOTO_ARABIC, NOTO_CJK, NOTO_DEVANAGARI, NOTO_HEBREW, NOTO_MONO};
    use crate::{GEIST_MONO, GEIST_SANS, GEIST_SANS_BOLD, GEIST_SANS_ITALIC, JETBRAINS_MONO};
    use alloc::vec;

    fn build_two_face_ttc(f1: &[u8], f2: &[u8]) -> Vec<u8> {
        let header_len = 20usize; // ttcf(4) + ver(4) + count(4) + 2*offset(8)
        let off1 = header_len;
        let off2 = header_len + f1.len();
        let mut out = Vec::with_capacity(off2 + f2.len());
        out.extend_from_slice(b"ttcf");
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&2u32.to_be_bytes());
        out.extend_from_slice(&(off1 as u32).to_be_bytes());
        out.extend_from_slice(&(off2 as u32).to_be_bytes());

        let mut push_shifted = |sfnt: &[u8], delta: u32| {
            let start = out.len();
            out.extend_from_slice(sfnt);
            let num_tables = u16::from_be_bytes([sfnt[4], sfnt[5]]) as usize;
            for i in 0..num_tables {
                let rec = start + 12 + i * 16;
                let old = u32::from_be_bytes([
                    out[rec + 8],
                    out[rec + 9],
                    out[rec + 10],
                    out[rec + 11],
                ]);
                out[rec + 8..rec + 12].copy_from_slice(&(old + delta).to_be_bytes());
            }
        };
        push_shifted(f1, off1 as u32);
        push_shifted(f2, off2 as u32);
        out
    }

    fn build_minimal_sfnt(tables: &[([u8; 4], &[u8])]) -> Vec<u8> {
        let n = tables.len();
        let dir_size = 12 + n * 16;
        let mut out = vec![0u8; dir_size];
        out[0..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        out[4..6].copy_from_slice(&(n as u16).to_be_bytes());
        for (i, (tag, data)) in tables.iter().enumerate() {
            let off = out.len() as u32;
            out.extend_from_slice(data);
            while out.len() % 4 != 0 {
                out.push(0);
            }
            let rec = 12 + i * 16;
            out[rec..rec + 4].copy_from_slice(tag);
            out[rec + 8..rec + 12].copy_from_slice(&off.to_be_bytes());
            out[rec + 12..rec + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        }
        out
    }

    fn make_name_table(records: &[(u16, &str)]) -> Vec<u8> {
        let count = records.len();
        let storage_off = 6 + count * 12;
        let mut table = vec![0u8; storage_off];
        table[2..4].copy_from_slice(&(count as u16).to_be_bytes());
        table[4..6].copy_from_slice(&(storage_off as u16).to_be_bytes());
        for (i, &(id, s)) in records.iter().enumerate() {
            let str_off = (table.len() - storage_off) as u16;
            let utf16: Vec<u16> = s.encode_utf16().collect();
            for u in &utf16 {
                table.extend_from_slice(&u.to_be_bytes());
            }
            let rec = 6 + i * 12;
            table[rec..rec + 2].copy_from_slice(&3u16.to_be_bytes()); // Windows
            table[rec + 2..rec + 4].copy_from_slice(&1u16.to_be_bytes()); // Unicode BMP
            table[rec + 4..rec + 6].copy_from_slice(&0x0409u16.to_be_bytes()); // en-US
            table[rec + 6..rec + 8].copy_from_slice(&id.to_be_bytes());
            table[rec + 8..rec + 10].copy_from_slice(&((utf16.len() * 2) as u16).to_be_bytes());
            table[rec + 10..rec + 12].copy_from_slice(&str_off.to_be_bytes());
        }
        table
    }

    #[test]
    fn inspects_geist_sans_and_mono_faces() {
        let sans = inspect_font(GEIST_SANS, 0).expect("Geist Sans");
        assert_eq!(sans.family, "Geist");
        assert_eq!(sans.weight, 400);
        assert!(!sans.is_bold);
        assert!(!sans.is_italic);
        assert!(!sans.is_monospace);
        assert!(sans.glyph_count > 100);
        assert_eq!(sans.units_per_em, 1000);
        assert!(sans.coverage.has(ScriptRange::BasicLatin));
        assert!(sans.version.is_some());

        let bold = inspect_font(GEIST_SANS_BOLD, 0).expect("Geist Bold");
        assert_eq!(bold.weight, 700);
        assert!(bold.is_bold);

        let italic = inspect_font(GEIST_SANS_ITALIC, 0).expect("Geist Italic");
        assert!(italic.is_italic);

        let mono = inspect_font(GEIST_MONO, 0).expect("Geist Mono");
        assert!(mono.is_monospace);
        assert_eq!(mono.classification(), FontClass::Monospace);

        let jb = inspect_font(JETBRAINS_MONO, 0).expect("JetBrains Mono");
        assert!(jb.is_monospace);
        assert!(jb.coverage.has(ScriptRange::BasicLatin));
    }

    #[test]
    fn inspects_noto_script_coverage() {
        let ar = inspect_font(NOTO_ARABIC, 0).expect("Noto Arabic");
        assert!(ar.coverage.has(ScriptRange::Arabic));
        assert!(!ar.coverage.has(ScriptRange::Hangul));

        let he = inspect_font(NOTO_HEBREW, 0).expect("Noto Hebrew");
        assert!(he.coverage.has(ScriptRange::Hebrew));

        let deva = inspect_font(NOTO_DEVANAGARI, 0).expect("Noto Devanagari");
        assert!(deva.coverage.has(ScriptRange::Devanagari));

        let cjk = inspect_font(NOTO_CJK, 0).expect("Noto CJK");
        assert!(cjk.coverage.has(ScriptRange::Cjk));

        let mono = inspect_font(NOTO_MONO, 0).expect("Noto Mono");
        assert!(mono.coverage.has(ScriptRange::LatinExt));
        assert!(mono.coverage.has(ScriptRange::Cyrillic));
        assert!(mono.coverage.has(ScriptRange::Greek));
    }

    #[test]
    fn panose_classifies_all_five_categories() {
        assert_eq!(
            Panose::new([2, 2, 5, 3, 0, 0, 0, 0, 0, 0]).classification(),
            FontClass::Serif
        );
        assert_eq!(
            Panose::new([2, 11, 5, 3, 0, 0, 0, 0, 0, 0]).classification(),
            FontClass::Sans
        );
        assert_eq!(
            Panose::new([3, 1, 5, 0, 0, 0, 0, 0, 0, 0]).classification(),
            FontClass::Script
        );
        assert_eq!(
            Panose::new([4, 1, 5, 0, 0, 0, 0, 0, 0, 0]).classification(),
            FontClass::Decorative
        );
        assert_eq!(
            Panose::new([2, 11, 5, 9, 0, 0, 0, 0, 0, 0]).classification(),
            FontClass::Monospace
        );
        assert_eq!(Panose::default().classification(), FontClass::Unknown);
    }

    #[test]
    fn prefers_typographic_name_ids_16_17_over_1_2() {
        let mut head = [0u8; 54];
        head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3C85u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());

        let mut hhea = [0u8; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[34..36].copy_from_slice(&1u16.to_be_bytes());

        let mut maxp = [0u8; 6];
        maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&2u16.to_be_bytes());

        let name = make_name_table(&[
            (name_id::FAMILY, "Cabaret SemiBold"),
            (name_id::SUBFAMILY, "Regular"),
            (name_id::TYPOGRAPHIC_FAMILY, "Cabaret"),
            (name_id::TYPOGRAPHIC_SUBFAMILY, "SemiBold"),
            (name_id::DESIGNER, "SexOS Type Foundry"),
            (name_id::LICENSE, "SIL OFL 1.1"),
            (name_id::VERSION, "Version 2.10"),
        ]);

        let sfnt = build_minimal_sfnt(&[
            (*b"head", &head),
            (*b"hhea", &hhea),
            (*b"maxp", &maxp),
            (*b"name", &name),
        ]);
        let meta = inspect_font(&sfnt, 0).expect("minimal sfnt parses");
        assert_eq!(meta.family, "Cabaret");
        assert_eq!(meta.subfamily, "SemiBold");
        assert_eq!(meta.legacy_family, "Cabaret SemiBold");
        assert_eq!(meta.legacy_subfamily, "Regular");
        assert_eq!(meta.designer.as_deref(), Some("SexOS Type Foundry"));
        assert_eq!(meta.license.as_deref(), Some("SIL OFL 1.1"));
        assert_eq!(meta.version.as_deref(), Some("Version 2.10"));
    }

    #[test]
    fn inspects_ttc_collection_faces() {
        assert_eq!(font_collection_count(GEIST_SANS), None);
        let ttc = build_two_face_ttc(GEIST_SANS, GEIST_MONO);
        assert_eq!(font_collection_count(&ttc), Some(2));
        let m0 = inspect_font(&ttc, 0).expect("face 0");
        let m1 = inspect_font(&ttc, 1).expect("face 1");
        assert!(!m0.is_monospace);
        assert!(m1.is_monospace);
        assert!(inspect_font(&ttc, 2).is_none());
    }
}
