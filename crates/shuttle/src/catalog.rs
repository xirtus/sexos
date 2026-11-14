// SPDX-License-Identifier: GPL-3.0-only
//! Shared font catalog and metadata discovery.
//!
//! Indexes SexOS standard embedded fonts and dynamically registered font bytes,
//! extracts TrueType/OpenType metadata via `ttf-parser` (family, subfamily, weight,
//! monospace detection, supported script coverage), and provides flexible filtering.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use sex_font::fallback::{
    NOTO_ARABIC, NOTO_CJK, NOTO_DEVANAGARI, NOTO_HEBREW, NOTO_MONO, NOTO_SYMBOLS, NOTO_THAI,
};
use sex_font::{GEIST_MONO, GEIST_SANS};
use ttf_parser::{name_id, Face};

pub mod script_mask {
    pub const LATIN: u32 = 1 << 0;
    pub const CYRILLIC: u32 = 1 << 1;
    pub const GREEK: u32 = 1 << 2;
    pub const ARABIC: u32 = 1 << 3;
    pub const HEBREW: u32 = 1 << 4;
    pub const CJK: u32 = 1 << 5;
    pub const DEVANAGARI: u32 = 1 << 6;
    pub const THAI: u32 = 1 << 7;
    pub const SYMBOLS: u32 = 1 << 8;
    pub const EMOJI: u32 = 1 << 9;
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontEntry {
    pub id: usize,
    pub name: String,
    pub family: String,
    pub subfamily: String,
    pub weight: u16,
    pub is_italic: bool,
    pub is_monospaced: bool,
    pub scripts: u32,
    pub data: &'static [u8],
}

#[derive(Clone, Debug, Default)]
pub struct FontFilter {
    pub query: Option<String>,
    pub monospace_only: bool,
    pub required_script: Option<u32>,
    pub min_weight: Option<u16>,
    pub max_weight: Option<u16>,
}

pub struct FontCatalog {
    entries: Vec<FontEntry>,
}

impl Default for FontCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl FontCatalog {
    pub fn new() -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
        };

        catalog.register_builtin("Geist Sans", GEIST_SANS, script_mask::LATIN | script_mask::CYRILLIC);
        catalog.register_builtin("Geist Mono", GEIST_MONO, script_mask::LATIN | script_mask::CYRILLIC);
        catalog.register_builtin("Noto Sans Arabic", NOTO_ARABIC, script_mask::ARABIC);
        catalog.register_builtin("Noto Sans Hebrew", NOTO_HEBREW, script_mask::HEBREW);
        catalog.register_builtin("Noto Sans CJK SC", NOTO_CJK, script_mask::CJK);
        catalog.register_builtin("Noto Sans Devanagari", NOTO_DEVANAGARI, script_mask::DEVANAGARI);
        catalog.register_builtin("Noto Sans Thai", NOTO_THAI, script_mask::THAI);
        catalog.register_builtin("Noto Sans Mono", NOTO_MONO, script_mask::LATIN | script_mask::CYRILLIC | script_mask::GREEK);
        catalog.register_builtin("Noto Sans Symbols 2", NOTO_SYMBOLS, script_mask::SYMBOLS);

        catalog
    }

    fn register_builtin(&mut self, default_name: &str, data: &'static [u8], extra_scripts: u32) {
        let id = self.entries.len();
        if let Ok(face) = Face::parse(data, 0) {
            let family = extract_name(&face, name_id::FAMILY).unwrap_or_else(|| String::from(default_name));
            let subfamily = extract_name(&face, name_id::SUBFAMILY).unwrap_or_else(|| String::from("Regular"));
            let is_italic = face.is_italic();
            let is_monospaced = face.is_monospaced();
            let weight = face.weight().to_number();

            let mut scripts = extra_scripts;
            if face.glyph_index('A').is_some() {
                scripts |= script_mask::LATIN;
            }
            if face.glyph_index('Ж').is_some() {
                scripts |= script_mask::CYRILLIC;
            }
            if face.glyph_index('Ω').is_some() {
                scripts |= script_mask::GREEK;
            }
            if face.glyph_index('ب').is_some() {
                scripts |= script_mask::ARABIC;
            }
            if face.glyph_index('ש').is_some() {
                scripts |= script_mask::HEBREW;
            }
            if face.glyph_index('日').is_some() {
                scripts |= script_mask::CJK;
            }

            self.entries.push(FontEntry {
                id,
                name: String::from(default_name),
                family,
                subfamily,
                weight,
                is_italic,
                is_monospaced,
                scripts,
                data,
            });
        }
    }

    /// Registers dynamic font data and extracts OpenType metadata.
    pub fn register(&mut self, data: &'static [u8]) -> Option<usize> {
        let face = Face::parse(data, 0).ok()?;
        let id = self.entries.len();
        let family = extract_name(&face, name_id::FAMILY).unwrap_or_else(|| String::from("Custom Font"));
        let subfamily = extract_name(&face, name_id::SUBFAMILY).unwrap_or_else(|| String::from("Regular"));
        let is_italic = face.is_italic();
        let is_monospaced = face.is_monospaced();
        let weight = face.weight().to_number();

        let mut scripts = 0;
        if face.glyph_index('A').is_some() {
            scripts |= script_mask::LATIN;
        }
        if face.glyph_index('Ж').is_some() {
            scripts |= script_mask::CYRILLIC;
        }
        if face.glyph_index('ب').is_some() {
            scripts |= script_mask::ARABIC;
        }
        if face.glyph_index('ש').is_some() {
            scripts |= script_mask::HEBREW;
        }
        if face.glyph_index('日').is_some() {
            scripts |= script_mask::CJK;
        }

        self.entries.push(FontEntry {
            id,
            name: family.clone(),
            family,
            subfamily,
            weight,
            is_italic,
            is_monospaced,
            scripts,
            data,
        });

        Some(id)
    }

    pub fn entries(&self) -> &[FontEntry] {
        &self.entries
    }

    pub fn entry_by_id(&self, id: usize) -> Option<&FontEntry> {
        self.entries.get(id)
    }

    /// Queries fonts matching `filter`.
    pub fn query(&self, filter: &FontFilter) -> Vec<&FontEntry> {
        self.entries
            .iter()
            .filter(|e| {
                if filter.monospace_only && !e.is_monospaced {
                    return false;
                }
                if let Some(req_script) = filter.required_script {
                    if (e.scripts & req_script) == 0 {
                        return false;
                    }
                }
                if let Some(min_w) = filter.min_weight {
                    if e.weight < min_w {
                        return false;
                    }
                }
                if let Some(max_w) = filter.max_weight {
                    if e.weight > max_w {
                        return false;
                    }
                }
                if let Some(ref q) = filter.query {
                    let q_lower = q.to_lowercase();
                    let name_match = e.name.to_lowercase().contains(&q_lower);
                    let fam_match = e.family.to_lowercase().contains(&q_lower);
                    if !name_match && !fam_match {
                        return false;
                    }
                }
                true
            })
            .collect()
    }
}

fn extract_name(face: &Face<'_>, id: u16) -> Option<String> {
    for name in face.names() {
        if name.name_id == id {
            if name.is_unicode() {
                let u16_chars: Vec<u16> = name
                    .name
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                if let Ok(s) = String::from_utf16(&u16_chars) {
                    return Some(s);
                }
            } else if let Ok(s) = core::str::from_utf8(name.name) {
                return Some(String::from(s));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_builtins() {
        let catalog = FontCatalog::new();
        assert!(catalog.entries().len() >= 9);

        // Filter monospace
        let mono_filter = FontFilter {
            monospace_only: true,
            ..Default::default()
        };
        let monos = catalog.query(&mono_filter);
        assert!(!monos.is_empty());
        assert!(monos.iter().all(|e| e.is_monospaced));

        // Filter search query
        let query_filter = FontFilter {
            query: Some(String::from("arabic")),
            ..Default::default()
        };
        let arabics = catalog.query(&query_filter);
        assert_eq!(arabics.len(), 1);
        assert_eq!(arabics[0].name, "Noto Sans Arabic");
    }
}
