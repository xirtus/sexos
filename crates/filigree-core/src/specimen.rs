// SPDX-License-Identifier: GPL-3.0-only
//! Repertoire inspector, waterfall specimen calculator, 6-family side-by-side
//! comparator, and Spindle monospace handoff for `/system/settings.toml`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use sex_font::{inspect_font, UnicodeCoverage};
use ttf_parser::Face;

use crate::catalog::FontEntry;

/// Maximum number of families compared side-by-side.
pub const MAX_COMPARE_SLOTS: usize = 6;

/// Default waterfall specimen pixel sizes.
pub const DEFAULT_WATERFALL_SIZES: &[u32] = &[10, 12, 14, 18, 24, 36, 48, 72];

/// Default SexOS system settings TOML path updated by Spindle handoff.
pub const DEFAULT_SETTINGS_PATH: &str = "/system/settings.toml";

/// Inclusive Unicode scalar range `[start, end]` supported by a face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScalarRange {
    pub start: u32,
    pub end: u32,
}

impl ScalarRange {
    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start) + 1
    }

    pub const fn is_empty(self) -> bool {
        self.end < self.start
    }

    pub const fn contains(self, cp: u32) -> bool {
        cp >= self.start && cp <= self.end
    }
}

/// One paged slice of supported codepoints for repertoire browsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodepointPage {
    pub page_index: u32,
    pub page_size: u32,
    pub block_start: u32,
    pub codepoints: Vec<u32>,
    pub total_supported: usize,
}

/// Complete Unicode repertoire summary for a single font face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repertoire {
    pub total_codepoints: usize,
    pub ranges: Vec<ScalarRange>,
    pub coverage: UnicodeCoverage,
}

impl Repertoire {
    /// Whether `cp` is mapped to a non-`.notdef` glyph in this repertoire.
    pub fn contains(&self, cp: u32) -> bool {
        self.ranges
            .binary_search_by(|r| {
                if cp < r.start {
                    core::cmp::Ordering::Greater
                } else if cp > r.end {
                    core::cmp::Ordering::Less
                } else {
                    core::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

/// Enumerate all supported Unicode scalar ranges for `face_index` in `bytes`.
pub fn inspect_repertoire(bytes: &[u8], face_index: u32) -> Option<Repertoire> {
    let face = Face::parse(bytes, face_index).ok()?;
    let meta = inspect_font(bytes, face_index)?;
    let cmap = face.tables().cmap?;

    let mut scalars: Vec<u32> = Vec::new();
    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|cp| {
            if let Some(ch) = char::from_u32(cp) {
                if face.glyph_index(ch).is_some_and(|g| g.0 != 0) {
                    scalars.push(cp);
                }
            }
        });
    }

    scalars.sort_unstable();
    scalars.dedup();

    let mut ranges: Vec<ScalarRange> = Vec::new();
    for &cp in &scalars {
        if let Some(last) = ranges.last_mut() {
            if cp == last.end + 1 {
                last.end = cp;
                continue;
            }
        }
        ranges.push(ScalarRange { start: cp, end: cp });
    }

    Some(Repertoire {
        total_codepoints: scalars.len(),
        ranges,
        coverage: meta.coverage,
    })
}

/// Extract one fixed-size Unicode codepoint page (e.g. `page_size = 256`) from `repertoire`.
pub fn codepoint_page(
    repertoire: &Repertoire,
    page_size: u32,
    page_index: u32,
) -> CodepointPage {
    let page_size = page_size.max(1);
    let block_start = page_index.saturating_mul(page_size);
    let block_end = block_start.saturating_add(page_size - 1);
    let mut codepoints = Vec::new();

    for r in &repertoire.ranges {
        if r.end < block_start {
            continue;
        }
        if r.start > block_end {
            break;
        }
        let s = r.start.max(block_start);
        let e = r.end.min(block_end);
        for cp in s..=e {
            codepoints.push(cp);
        }
    }

    CodepointPage {
        page_index,
        page_size,
        block_start,
        codepoints,
        total_supported: repertoire.total_codepoints,
    }
}

/// Convenience helper: inspect a face and return one paged block directly.
pub fn inspect_codepoint_page(
    bytes: &[u8],
    face_index: u32,
    page_size: u32,
    page_index: u32,
) -> Option<CodepointPage> {
    let rep = inspect_repertoire(bytes, face_index)?;
    Some(codepoint_page(&rep, page_size, page_index))
}

/// Return all page indices (for `page_size`) that contain at least one supported scalar.
pub fn nonempty_pages(repertoire: &Repertoire, page_size: u32) -> Vec<u32> {
    let page_size = page_size.max(1);
    let mut pages = Vec::new();
    for r in &repertoire.ranges {
        let first_page = r.start / page_size;
        let last_page = r.end / page_size;
        for p in first_page..=last_page {
            if pages.last().copied() != Some(p) {
                pages.push(p);
            }
        }
    }
    pages
}

/// One line in a waterfall specimen layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaterfallRow {
    pub size_px: u32,
    pub y_offset_px: i32,
    pub baseline_y_px: i32,
    pub line_height_px: i32,
    pub ascender_px: i32,
    pub descender_px: i32,
    pub estimated_advance_px: i32,
    pub text: String,
}

/// Multi-size waterfall specimen layout for one face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaterfallLayout {
    pub family: String,
    pub subfamily: String,
    pub total_height_px: i32,
    pub max_width_px: i32,
    pub rows: Vec<WaterfallRow>,
}

/// Compute waterfall row coordinates and advances from actual face metrics.
pub fn calculate_waterfall(
    bytes: &[u8],
    face_index: u32,
    sample: &str,
    sizes_px: &[u32],
    row_gap_px: i32,
) -> Option<WaterfallLayout> {
    let face = Face::parse(bytes, face_index).ok()?;
    let meta = inspect_font(bytes, face_index)?;
    let upem = face.units_per_em() as i64;
    if upem <= 0 {
        return None;
    }

    let sample_advance_units = measure_text_advance_units(&face, sample);
    let mut rows = Vec::with_capacity(sizes_px.len());
    let mut y = 0i32;
    let mut max_width = 0i32;

    for &raw_size in sizes_px {
        let size_px = raw_size.max(1);
        let asc_px = scale_round(meta.ascender as i64, size_px, upem).max(1);
        let desc_px = scale_round(meta.descender as i64, size_px, upem).min(0);
        let gap_px = scale_round(meta.line_gap.max(0) as i64, size_px, upem);
        let line_height_px = (asc_px - desc_px + gap_px).max(size_px as i32);
        let baseline_y_px = y + asc_px;
        let estimated_advance_px = scale_round(sample_advance_units, size_px, upem);

        max_width = max_width.max(estimated_advance_px);
        rows.push(WaterfallRow {
            size_px,
            y_offset_px: y,
            baseline_y_px,
            line_height_px,
            ascender_px: asc_px,
            descender_px: desc_px,
            estimated_advance_px,
            text: String::from(sample),
        });
        y = y.saturating_add(line_height_px).saturating_add(row_gap_px.max(0));
    }

    let total_height_px = if rows.is_empty() {
        0
    } else {
        y.saturating_sub(row_gap_px.max(0))
    };

    Some(WaterfallLayout {
        family: meta.family,
        subfamily: meta.subfamily,
        total_height_px,
        max_width_px: max_width,
        rows,
    })
}

/// Errors from the side-by-side comparator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompareError {
    Empty,
    TooManySlots(usize),
    InvalidFont { slot: usize },
}

impl fmt::Display for CompareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompareError::Empty => f.write_str("no fonts provided to compare"),
            CompareError::TooManySlots(n) => {
                write!(f, "cannot compare {n} fonts (maximum is {MAX_COMPARE_SLOTS})")
            }
            CompareError::InvalidFont { slot } => write!(f, "invalid font bytes at slot {slot}"),
        }
    }
}

/// Font-unit and pixel-scaled vertical/horizontal metrics for one compared face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedMetrics {
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub cap_height: i16,
    pub x_height: i16,
    pub ascender_px: i32,
    pub descender_px: i32,
    pub cap_height_px: i32,
    pub x_height_px: i32,
    pub sample_width_px: i32,
    pub missing_chars: usize,
}

/// One slot in a [`ComparisonBoard`] with shared-baseline-aligned Y guide coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareSlot {
    pub slot_index: usize,
    pub family: String,
    pub subfamily: String,
    pub weight: u16,
    pub is_italic: bool,
    pub metrics: NormalizedMetrics,
    pub baseline_y_px: i32,
    pub x_height_y_px: i32,
    pub cap_height_y_px: i32,
    pub ascender_y_px: i32,
    pub descender_y_px: i32,
}

/// Up-to-6-family side-by-side metric comparison board aligned on one baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparisonBoard {
    pub sample_word: String,
    pub size_px: u32,
    pub shared_baseline_y_px: i32,
    pub total_height_px: i32,
    pub slots: Vec<CompareSlot>,
}

/// Build an up-to-6-family metric-aligned side-by-side comparison board.
pub fn build_comparison(
    faces: &[(&[u8], u32)],
    sample_word: &str,
    size_px: u32,
) -> Result<ComparisonBoard, CompareError> {
    if faces.is_empty() {
        return Err(CompareError::Empty);
    }
    if faces.len() > MAX_COMPARE_SLOTS {
        return Err(CompareError::TooManySlots(faces.len()));
    }
    let size_px = size_px.max(1);

    struct RawSlot {
        family: String,
        subfamily: String,
        weight: u16,
        is_italic: bool,
        metrics: NormalizedMetrics,
    }

    let mut raw_slots = Vec::with_capacity(faces.len());
    let mut max_asc_px = 0i32;
    let mut max_desc_depth_px = 0i32;

    for (slot_index, &(bytes, face_index)) in faces.iter().enumerate() {
        let face = Face::parse(bytes, face_index)
            .map_err(|_| CompareError::InvalidFont { slot: slot_index })?;
        let meta = inspect_font(bytes, face_index)
            .ok_or(CompareError::InvalidFont { slot: slot_index })?;
        let upem = meta.units_per_em as i64;

        let cap_height = meta
            .cap_height
            .filter(|&h| h > 0)
            .or_else(|| {
                face.glyph_index('H')
                    .and_then(|g| face.glyph_bounding_box(g))
                    .map(|b| b.y_max)
            })
            .unwrap_or(((upem * 7) / 10) as i16);

        let x_height = meta
            .x_height
            .filter(|&h| h > 0)
            .or_else(|| {
                face.glyph_index('x')
                    .and_then(|g| face.glyph_bounding_box(g))
                    .map(|b| b.y_max)
            })
            .unwrap_or(((upem * 5) / 10) as i16);

        let ascender_px = scale_round(meta.ascender as i64, size_px, upem);
        let descender_px = scale_round(meta.descender as i64, size_px, upem).min(0);
        let cap_height_px = scale_round(cap_height as i64, size_px, upem);
        let x_height_px = scale_round(x_height as i64, size_px, upem);

        let sample_advance_units = measure_text_advance_units(&face, sample_word);
        let sample_width_px = scale_round(sample_advance_units, size_px, upem);
        let missing_chars = sample_word
            .chars()
            .filter(|&c| face.glyph_index(c).is_none_or(|g| g.0 == 0))
            .count();

        max_asc_px = max_asc_px.max(ascender_px);
        max_desc_depth_px = max_desc_depth_px.max(-descender_px);

        raw_slots.push(RawSlot {
            family: meta.family,
            subfamily: meta.subfamily,
            weight: meta.weight,
            is_italic: meta.is_italic || meta.is_oblique,
            metrics: NormalizedMetrics {
                units_per_em: meta.units_per_em,
                ascender: meta.ascender,
                descender: meta.descender,
                cap_height,
                x_height,
                ascender_px,
                descender_px,
                cap_height_px,
                x_height_px,
                sample_width_px,
                missing_chars,
            },
        });
    }

    let shared_baseline_y_px = max_asc_px.max(0);
    let total_height_px = shared_baseline_y_px + max_desc_depth_px.max(0);

    let slots = raw_slots
        .into_iter()
        .enumerate()
        .map(|(slot_index, s)| CompareSlot {
            slot_index,
            family: s.family,
            subfamily: s.subfamily,
            weight: s.weight,
            is_italic: s.is_italic,
            baseline_y_px: shared_baseline_y_px,
            x_height_y_px: shared_baseline_y_px - s.metrics.x_height_px,
            cap_height_y_px: shared_baseline_y_px - s.metrics.cap_height_px,
            ascender_y_px: shared_baseline_y_px - s.metrics.ascender_px,
            descender_y_px: shared_baseline_y_px - s.metrics.descender_px,
            metrics: s.metrics,
        })
        .collect();

    Ok(ComparisonBoard {
        sample_word: String::from(sample_word),
        size_px,
        shared_baseline_y_px,
        total_height_px,
        slots,
    })
}

/// Errors returned when validating a font for Spindle terminal handoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpindleHandoffError {
    InactiveEntry,
    NotMonospace { family: String },
    InvalidFont,
    NoPrintableAscii,
    NonUniformAdvance {
        min_advance: u16,
        max_advance: u16,
        min_char: char,
        max_char: char,
    },
}

impl fmt::Display for SpindleHandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpindleHandoffError::InactiveEntry => {
                f.write_str("cannot hand off a deactivated font to Spindle")
            }
            SpindleHandoffError::NotMonospace { family } => {
                write!(f, "font '{family}' is not marked monospaced")
            }
            SpindleHandoffError::InvalidFont => f.write_str("font bytes failed to parse"),
            SpindleHandoffError::NoPrintableAscii => {
                f.write_str("font lacks printable ASCII glyphs")
            }
            SpindleHandoffError::NonUniformAdvance {
                min_advance,
                max_advance,
                min_char,
                max_char,
            } => write!(
                f,
                "non-uniform monospace advance: '{min_char}'={min_advance} vs '{max_char}'={max_advance}"
            ),
        }
    }
}

/// Validated Spindle terminal monospace handoff configuration for `/system/settings.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpindleFontConfig {
    pub family: String,
    pub path: String,
    pub face_index: u32,
    pub advance_units: Option<u16>,
    pub units_per_em: u16,
    pub sha256_hex: String,
    pub toml_fragment: String,
}

impl SpindleFontConfig {
    /// Merge this `[spindle]` monospace configuration into an existing `/system/settings.toml`,
    /// preserving unrelated sections, keys, and comments.
    pub fn apply_to_settings_toml(&self, existing_toml: &str) -> String {
        const SPINDLE_KEYS: &[&str] = &[
            "monospace_font",
            "monospace_font_path",
            "monospace_face_index",
            "monospace_sha256",
        ];

        let mut out = String::new();
        let mut in_spindle = false;
        let mut saw_spindle = false;
        let mut wrote_keys = false;

        for line in existing_toml.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                if in_spindle && !wrote_keys {
                    push_spindle_kv(&mut out, self);
                    wrote_keys = true;
                }
                let sec = &trimmed[1..trimmed.len() - 1].trim();
                in_spindle = *sec == "spindle";
                if in_spindle {
                    saw_spindle = true;
                }
                out.push_str(line);
                out.push('\n');
                continue;
            }

            if in_spindle {
                if let Some((k, _)) = trimmed.split_once('=') {
                    let key = k.trim();
                    if SPINDLE_KEYS.contains(&key) {
                        continue;
                    }
                }
            }

            out.push_str(line);
            out.push('\n');
        }

        if in_spindle && !wrote_keys {
            push_spindle_kv(&mut out, self);
        } else if !saw_spindle {
            if !out.is_empty() && !out.ends_with("\n\n") {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
            out.push_str(&self.toml_fragment);
        }

        out
    }
}

/// Validate `entry` and its font `bytes` for Spindle monospace advance uniformity
/// and emit the `/system/settings.toml` monospace config update.
pub fn spindle_font_config(
    entry: &FontEntry,
    bytes: &[u8],
) -> Result<SpindleFontConfig, SpindleHandoffError> {
    if !entry.active {
        return Err(SpindleHandoffError::InactiveEntry);
    }
    let face = Face::parse(bytes, entry.face_index).map_err(|_| SpindleHandoffError::InvalidFont)?;
    if !(entry.metadata.is_monospace || face.is_monospaced() || entry.metadata.panose.is_monospace())
    {
        return Err(SpindleHandoffError::NotMonospace {
            family: entry.canonical_family(),
        });
    }

    let mut min_adv = u16::MAX;
    let mut max_adv = 0u16;
    let mut min_char = ' ';
    let mut max_char = ' ';
    let mut seen = 0usize;

    for code in 0x20u32..=0x7Eu32 {
        let Some(ch) = char::from_u32(code) else {
            continue;
        };
        let Some(gid) = face.glyph_index(ch).filter(|g| g.0 != 0) else {
            continue;
        };
        let Some(adv) = face.glyph_hor_advance(gid) else {
            continue;
        };
        seen += 1;
        if adv < min_adv {
            min_adv = adv;
            min_char = ch;
        }
        if adv > max_adv {
            max_adv = adv;
            max_char = ch;
        }
    }

    if seen == 0 {
        return Err(SpindleHandoffError::NoPrintableAscii);
    }
    if min_adv == 0 || min_adv != max_adv {
        return Err(SpindleHandoffError::NonUniformAdvance {
            min_advance: min_adv,
            max_advance: max_adv,
            min_char,
            max_char,
        });
    }

    Ok(build_spindle_config(entry, Some(min_adv)))
}

/// Metadata-only Spindle handoff check when raw font bytes are not loaded.
pub fn spindle_font_config_entry(entry: &FontEntry) -> Result<SpindleFontConfig, SpindleHandoffError> {
    if !entry.active {
        return Err(SpindleHandoffError::InactiveEntry);
    }
    if !(entry.metadata.is_monospace || entry.metadata.panose.is_monospace()) {
        return Err(SpindleHandoffError::NotMonospace {
            family: entry.canonical_family(),
        });
    }
    Ok(build_spindle_config(entry, None))
}

fn build_spindle_config(entry: &FontEntry, advance_units: Option<u16>) -> SpindleFontConfig {
    let family = entry.canonical_family();
    let mut frag = String::from("[spindle]\n");
    let mut dummy = SpindleFontConfig {
        family: family.clone(),
        path: entry.path.clone(),
        face_index: entry.face_index,
        advance_units,
        units_per_em: entry.metadata.units_per_em,
        sha256_hex: entry.sha256_hex.clone(),
        toml_fragment: String::new(),
    };
    push_spindle_kv(&mut frag, &dummy);
    dummy.toml_fragment = frag;
    dummy
}

fn push_spindle_kv(out: &mut String, cfg: &SpindleFontConfig) {
    out.push_str(&format!("monospace_font = \"{}\"\n", escape_toml_str(&cfg.family)));
    out.push_str(&format!(
        "monospace_font_path = \"{}\"\n",
        escape_toml_str(&cfg.path)
    ));
    out.push_str(&format!("monospace_face_index = {}\n", cfg.face_index));
    out.push_str(&format!(
        "monospace_sha256 = \"{}\"\n",
        escape_toml_str(&cfg.sha256_hex)
    ));
}

fn escape_toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn measure_text_advance_units(face: &Face<'_>, text: &str) -> i64 {
    let mut total = 0i64;
    for ch in text.chars() {
        if let Some(gid) = face.glyph_index(ch) {
            total += face.glyph_hor_advance(gid).unwrap_or(0) as i64;
        }
    }
    total
}

fn scale_round(units: i64, size_px: u32, upem: i64) -> i32 {
    let num = units * size_px as i64;
    if num >= 0 {
        ((num + upem / 2) / upem) as i32
    } else {
        ((num - upem / 2) / upem) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sex_font::fallback::NOTO_ARABIC;
    use sex_font::{ScriptRange, GEIST_MONO, GEIST_SANS, JETBRAINS_MONO};

    #[test]
    fn repertoire_inspector_ranges_and_codepoint_pages() {
        let rep = inspect_repertoire(GEIST_MONO, 0).expect("Geist Mono repertoire");
        assert!(rep.total_codepoints > 95);
        assert!(rep.contains('A' as u32));
        assert!(rep.contains('~' as u32));
        assert!(rep.coverage.has(ScriptRange::BasicLatin));

        let page0 = codepoint_page(&rep, 256, 0);
        assert_eq!(page0.block_start, 0);
        assert!(page0.codepoints.contains(&('A' as u32)));
        let pages = nonempty_pages(&rep, 256);
        assert!(pages.contains(&0));

        let ar = inspect_repertoire(NOTO_ARABIC, 0).expect("Arabic repertoire");
        assert!(ar.coverage.has(ScriptRange::Arabic));
        assert!(ar.contains(0x0627)); // ARABIC LETTER ALEF
    }

    #[test]
    fn waterfall_layout_monotonic_y_and_scaling() {
        let wf = calculate_waterfall(
            GEIST_SANS,
            0,
            "Hamburgefonstiv",
            DEFAULT_WATERFALL_SIZES,
            4,
        )
        .expect("waterfall");
        assert_eq!(wf.rows.len(), DEFAULT_WATERFALL_SIZES.len());
        for pair in wf.rows.windows(2) {
            assert!(pair[1].y_offset_px > pair[0].y_offset_px);
            assert!(pair[1].estimated_advance_px >= pair[0].estimated_advance_px);
            assert!(pair[1].line_height_px >= pair[0].line_height_px);
        }
        assert!(wf.total_height_px > 0);
        assert!(wf.max_width_px > 0);
    }

    #[test]
    fn side_by_side_comparator_aligns_baseline_and_enforces_max_6() {
        assert_eq!(
            build_comparison(&[], "abc", 48).unwrap_err(),
            CompareError::Empty
        );
        let too_many = [(GEIST_SANS, 0); 7];
        assert_eq!(
            build_comparison(&too_many, "abc", 48).unwrap_err(),
            CompareError::TooManySlots(7)
        );

        let board = build_comparison(
            &[(GEIST_SANS, 0), (GEIST_MONO, 0), (JETBRAINS_MONO, 0)],
            "Hamburgefonstiv",
            48,
        )
        .expect("3-slot comparison");
        assert_eq!(board.slots.len(), 3);
        let b0 = board.slots[0].baseline_y_px;
        assert_eq!(b0, board.shared_baseline_y_px);
        for slot in &board.slots {
            assert_eq!(slot.baseline_y_px, b0);
            assert!(slot.cap_height_y_px < slot.x_height_y_px);
            assert!(slot.x_height_y_px < slot.baseline_y_px);
            assert!(slot.descender_y_px >= slot.baseline_y_px);
            assert_eq!(slot.metrics.missing_chars, 0);
        }
    }

    #[test]
    fn spindle_handoff_validates_uniform_monospace_and_rejects_proportional() {
        let mut mono_entry =
            FontEntry::from_bytes("/system/fonts/GeistMono-Regular.ttf", 0, GEIST_MONO).unwrap();
        let cfg = spindle_font_config(&mono_entry, GEIST_MONO).expect("Geist Mono handoff");
        assert_eq!(cfg.family, "Geist Mono");
        assert!(cfg.advance_units.unwrap() > 0);

        let existing = "[appearance]\naccent = \"#3d7eff\"\n\n[spindle]\nmonospace_font = \"Old\"\nother_key = 42\n";
        let merged = cfg.apply_to_settings_toml(existing);
        assert!(merged.contains("[appearance]\naccent = \"#3d7eff\""));
        assert!(merged.contains("other_key = 42"));
        assert!(merged.contains("monospace_font = \"Geist Mono\""));
        assert!(!merged.contains("monospace_font = \"Old\""));

        let jb_entry =
            FontEntry::from_bytes("/system/fonts/JetBrainsMono-Regular.ttf", 0, JETBRAINS_MONO)
                .unwrap();
        assert!(spindle_font_config(&jb_entry, JETBRAINS_MONO).is_ok());

        let sans_entry =
            FontEntry::from_bytes("/system/fonts/Geist-Regular.ttf", 0, GEIST_SANS).unwrap();
        assert!(matches!(
            spindle_font_config(&sans_entry, GEIST_SANS),
            Err(SpindleHandoffError::NotMonospace { .. })
        ));

        mono_entry.active = false;
        assert_eq!(
            spindle_font_config(&mono_entry, GEIST_MONO).unwrap_err(),
            SpindleHandoffError::InactiveEntry
        );

        // Tamper hmtx advance for 'A' so monospace uniformity check catches it.
        mono_entry.active = true;
        let mut tampered = GEIST_MONO.to_vec();
        let gid_a = Face::parse(&tampered, 0)
            .unwrap()
            .glyph_index('A')
            .unwrap()
            .0 as usize;
        let num_tables = u16::from_be_bytes([tampered[4], tampered[5]]) as usize;
        for i in 0..num_tables {
            let rec = 12 + i * 16;
            if &tampered[rec..rec + 4] == b"hmtx" {
                let off = u32::from_be_bytes([
                    tampered[rec + 8],
                    tampered[rec + 9],
                    tampered[rec + 10],
                    tampered[rec + 11],
                ]) as usize;
                let g_off = off + gid_a * 4;
                let orig = u16::from_be_bytes([tampered[g_off], tampered[g_off + 1]]);
                tampered[g_off..g_off + 2].copy_from_slice(&(orig + 123).to_be_bytes());
                break;
            }
        }
        assert!(matches!(
            spindle_font_config(&mono_entry, &tampered),
            Err(SpindleHandoffError::NonUniformAdvance { .. })
        ));
    }
}
