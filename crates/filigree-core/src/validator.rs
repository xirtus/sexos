// SPDX-License-Identifier: GPL-3.0-only
//! Pre-install font & bundle validator for Filigree.
//!
//! Validates raw `.ttf`, `.otf`, `.ttc`/`.otc`, and `.zip` bundles in memory
//! before any bytes are written to disk.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use sex_archive::{no_progress, Archive, EntryKind, Format};
use sex_font::{font_collection_count, inspect_font, FontMetadata};
use sha2::{Digest, Sha256};
use ttf_parser::Face;

/// Default per-member decompression cap (64 MiB) to guard against malformed archives.
pub const DEFAULT_MAX_ARCHIVE_MEMBER_BYTES: u64 = 64 * 1024 * 1024;

/// Recognized container / font file formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFormat {
    Ttf,
    Otf,
    Ttc,
    Otc,
    Zip,
}

impl InputFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            InputFormat::Ttf => "ttf",
            InputFormat::Otf => "otf",
            InputFormat::Ttc => "ttc",
            InputFormat::Otc => "otc",
            InputFormat::Zip => "zip",
        }
    }
}

/// Errors emitted by pre-install validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    EmptyInput,
    UnrecognizedFormat,
    EmptyCollection,
    TruncatedTable { face_index: u32, tag: [u8; 4] },
    CorruptFont { face_index: u32, reason: String },
    ZeroGlyphFace { face_index: u32 },
    UnsafeArchivePath(String),
    ArchiveError(String),
    NoFontsInArchive,
    OversizedMember { path: String, size: u64 },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::EmptyInput => f.write_str("empty input"),
            ValidationError::UnrecognizedFormat => f.write_str("unrecognized font or archive format"),
            ValidationError::EmptyCollection => f.write_str("font collection contains zero faces"),
            ValidationError::TruncatedTable { face_index, tag } => {
                let tag_str = core::str::from_utf8(tag).unwrap_or("????");
                write!(f, "face {face_index}: truncated SFNT table '{tag_str}'")
            }
            ValidationError::CorruptFont { face_index, reason } => {
                write!(f, "face {face_index}: corrupt font ({reason})")
            }
            ValidationError::ZeroGlyphFace { face_index } => {
                write!(f, "face {face_index}: font has zero glyphs")
            }
            ValidationError::UnsafeArchivePath(path) => {
                write!(f, "unsafe archive path rejected: {path}")
            }
            ValidationError::ArchiveError(msg) => write!(f, "archive error: {msg}"),
            ValidationError::NoFontsInArchive => f.write_str("archive contains no valid font files"),
            ValidationError::OversizedMember { path, size } => {
                write!(f, "archive member '{path}' exceeds size limit ({size} bytes)")
            }
        }
    }
}

/// Validated metadata for a single face within a font file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedFace {
    pub face_index: u32,
    pub metadata: FontMetadata,
}

/// Structured pre-install preview card returned before any file is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPreview {
    pub source_name: String,
    pub format: InputFormat,
    pub sha256: [u8; 32],
    pub sha256_hex: String,
    pub byte_len: usize,
    pub faces: Vec<ValidatedFace>,
}

/// A validated font file (raw or extracted from a bundle) ready to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedFontAsset {
    pub preview: InstallPreview,
    pub bytes: Vec<u8>,
}

/// Compute raw SHA-256 digest.
pub fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Compute lowercase hexadecimal SHA-256 digest.
pub fn sha256_hex(bytes: &[u8]) -> String {
    encode_hex(&sha256_digest(bytes))
}

pub(crate) fn encode_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Validate raw `.ttf`, `.otf`, `.ttc`, or `.otc` bytes without touching the filesystem.
pub fn validate_font_bytes(bytes: &[u8], source_name: &str) -> Result<InstallPreview, ValidationError> {
    if bytes.is_empty() {
        return Err(ValidationError::EmptyInput);
    }
    let magic = bytes.get(..4).ok_or(ValidationError::UnrecognizedFormat)?;

    let (format, face_offsets) = if magic == b"ttcf" {
        let num_fonts = font_collection_count(bytes).ok_or(ValidationError::CorruptFont {
            face_index: 0,
            reason: String::from("invalid TTC header"),
        })?;
        if num_fonts == 0 {
            return Err(ValidationError::EmptyCollection);
        }
        let header_end = 12usize
            .checked_add((num_fonts as usize).saturating_mul(4))
            .ok_or(ValidationError::EmptyCollection)?;
        if bytes.len() < header_end {
            return Err(ValidationError::TruncatedTable {
                face_index: 0,
                tag: *b"ttcf",
            });
        }
        let mut offsets = Vec::with_capacity(num_fonts as usize);
        for i in 0..num_fonts as usize {
            let pos = 12 + i * 4;
            let off = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                as usize;
            offsets.push(off);
        }
        let first_magic = bytes.get(offsets[0]..offsets[0] + 4);
        let fmt = if first_magic == Some(b"OTTO") {
            InputFormat::Otc
        } else {
            InputFormat::Ttc
        };
        (fmt, offsets)
    } else if magic == [0x00, 0x01, 0x00, 0x00] || magic == b"true" || magic == b"typ1" {
        (InputFormat::Ttf, alloc::vec![0usize])
    } else if magic == b"OTTO" {
        (InputFormat::Otf, alloc::vec![0usize])
    } else {
        return Err(ValidationError::UnrecognizedFormat);
    };

    let mut faces = Vec::with_capacity(face_offsets.len());
    for (idx_usize, &sfnt_offset) in face_offsets.iter().enumerate() {
        let face_index = idx_usize as u32;
        validate_sfnt_directory(bytes, sfnt_offset, face_index)?;

        let face = Face::parse(bytes, face_index).map_err(|e| ValidationError::CorruptFont {
            face_index,
            reason: format!("{e:?}"),
        })?;

        if face.number_of_glyphs() == 0 {
            return Err(ValidationError::ZeroGlyphFace { face_index });
        }
        if face.units_per_em() == 0 {
            return Err(ValidationError::CorruptFont {
                face_index,
                reason: String::from("units_per_em is zero"),
            });
        }

        let metadata = inspect_font(bytes, face_index).ok_or_else(|| ValidationError::CorruptFont {
            face_index,
            reason: String::from("metadata inspection failed"),
        })?;

        faces.push(ValidatedFace {
            face_index,
            metadata,
        });
    }

    let sha256 = sha256_digest(bytes);
    let sha256_hex = encode_hex(&sha256);

    Ok(InstallPreview {
        source_name: String::from(source_name),
        format,
        sha256,
        sha256_hex,
        byte_len: bytes.len(),
        faces,
    })
}

fn validate_sfnt_directory(
    bytes: &[u8],
    sfnt_offset: usize,
    face_index: u32,
) -> Result<(), ValidationError> {
    let header_end = sfnt_offset
        .checked_add(12)
        .ok_or(ValidationError::TruncatedTable {
            face_index,
            tag: *b"sfnt",
        })?;
    if bytes.len() < header_end {
        return Err(ValidationError::TruncatedTable {
            face_index,
            tag: *b"sfnt",
        });
    }
    let sfnt_tag = &bytes[sfnt_offset..sfnt_offset + 4];
    let valid_sfnt = sfnt_tag == [0x00, 0x01, 0x00, 0x00]
        || sfnt_tag == b"OTTO"
        || sfnt_tag == b"true"
        || sfnt_tag == b"typ1";
    if !valid_sfnt {
        return Err(ValidationError::CorruptFont {
            face_index,
            reason: String::from("invalid SFNT signature"),
        });
    }

    let num_tables = u16::from_be_bytes([bytes[sfnt_offset + 4], bytes[sfnt_offset + 5]]) as usize;
    if num_tables == 0 {
        return Err(ValidationError::CorruptFont {
            face_index,
            reason: String::from("zero SFNT tables"),
        });
    }
    let dir_end = header_end
        .checked_add(num_tables.saturating_mul(16))
        .ok_or(ValidationError::TruncatedTable {
            face_index,
            tag: *b"dir ",
        })?;
    if bytes.len() < dir_end {
        return Err(ValidationError::TruncatedTable {
            face_index,
            tag: *b"dir ",
        });
    }

    for i in 0..num_tables {
        let base = header_end + i * 16;
        let tag = [bytes[base], bytes[base + 1], bytes[base + 2], bytes[base + 3]];
        let offset =
            u32::from_be_bytes([bytes[base + 8], bytes[base + 9], bytes[base + 10], bytes[base + 11]])
                as usize;
        let length = u32::from_be_bytes([
            bytes[base + 12],
            bytes[base + 13],
            bytes[base + 14],
            bytes[base + 15],
        ]) as usize;

        let end = offset
            .checked_add(length)
            .ok_or(ValidationError::TruncatedTable { face_index, tag })?;
        if end > bytes.len() {
            return Err(ValidationError::TruncatedTable { face_index, tag });
        }

        if &tag == b"maxp" && length >= 6 {
            let num_glyphs = u16::from_be_bytes([bytes[offset + 4], bytes[offset + 5]]);
            if num_glyphs == 0 {
                return Err(ValidationError::ZeroGlyphFace { face_index });
            }
        }
        if &tag == b"head" && length < 54 {
            return Err(ValidationError::TruncatedTable { face_index, tag });
        }
    }

    Ok(())
}

/// Validate and extract font files from a `.zip` bundle using `sex-archive`.
///
/// Rejects the entire bundle if any entry contains path traversal / unsafe paths
/// or if any included font file fails validation.
pub fn validate_bundle(
    zip_bytes: &[u8],
    _source_name: &str,
) -> Result<Vec<ValidatedFontAsset>, ValidationError> {
    validate_bundle_with_limit(zip_bytes, DEFAULT_MAX_ARCHIVE_MEMBER_BYTES)
}

/// Like [`validate_bundle`] with an explicit maximum decompressed member byte limit.
pub fn validate_bundle_with_limit(
    zip_bytes: &[u8],
    max_member_bytes: u64,
) -> Result<Vec<ValidatedFontAsset>, ValidationError> {
    if zip_bytes.is_empty() {
        return Err(ValidationError::EmptyInput);
    }
    let archive = Archive::open(zip_bytes)
        .map_err(|e| ValidationError::ArchiveError(format!("{e}")))?;

    if archive.format() != Format::Zip {
        return Err(ValidationError::UnrecognizedFormat);
    }

    // Pass 1: fail closed on ANY unsafe / path-traversal entry before reading payloads.
    for entry in archive.entries() {
        if !entry.safe || has_unsafe_path_syntax(&entry.raw_path) {
            return Err(ValidationError::UnsafeArchivePath(entry.raw_path.clone()));
        }
    }

    // Pass 2: extract & validate all font files in memory.
    let mut assets = Vec::new();
    for (idx, entry) in archive.entries().iter().enumerate() {
        if entry.kind != EntryKind::File {
            continue;
        }
        if is_ignored_bundle_metadata(&entry.path) || !is_font_extension(&entry.path) {
            continue;
        }
        if entry.size > max_member_bytes {
            return Err(ValidationError::OversizedMember {
                path: entry.path.clone(),
                size: entry.size,
            });
        }
        let mut buf = Vec::with_capacity(entry.size.min(1024 * 1024) as usize);
        let extracted = archive
            .extract(idx, &mut buf, &mut no_progress)
            .map_err(|e| match e {
                sex_archive::Error::UnsafePath(p) => ValidationError::UnsafeArchivePath(p),
                other => ValidationError::ArchiveError(format!("{other}")),
            })?;
        if (buf.len() as u64) > max_member_bytes {
            return Err(ValidationError::OversizedMember {
                path: extracted.path,
                size: buf.len() as u64,
            });
        }

        let preview = validate_font_bytes(&buf, &extracted.path)?;
        assets.push(ValidatedFontAsset { preview, bytes: buf });
    }

    if assets.is_empty() {
        return Err(ValidationError::NoFontsInArchive);
    }
    Ok(assets)
}

/// Auto-detect `.zip` archive vs raw font file (`.ttf`, `.otf`, `.ttc`, `.otc`) and validate.
pub fn validate_input(
    bytes: &[u8],
    source_name: &str,
) -> Result<Vec<ValidatedFontAsset>, ValidationError> {
    if bytes.is_empty() {
        return Err(ValidationError::EmptyInput);
    }
    let looks_zip = bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
        || source_name.to_ascii_lowercase().ends_with(".zip");
    if looks_zip {
        validate_bundle(bytes, source_name)
    } else {
        let preview = validate_font_bytes(bytes, source_name)?;
        Ok(alloc::vec![ValidatedFontAsset {
            preview,
            bytes: bytes.to_vec(),
        }])
    }
}

fn has_unsafe_path_syntax(raw: &str) -> bool {
    if raw.is_empty() || raw.contains('\0') || raw.starts_with('/') || raw.starts_with('\\') {
        return true;
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    raw.split(['/', '\\']).any(|seg| seg == "..")
}

fn is_ignored_bundle_metadata(path: &str) -> bool {
    if path.starts_with("__MACOSX/") || path == "__MACOSX" {
        return true;
    }
    let base = path.rsplit('/').next().unwrap_or(path);
    base.starts_with("._")
}

fn is_font_extension(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".ttf")
        || lower.ends_with(".otf")
        || lower.ends_with(".ttc")
        || lower.ends_with(".otc")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sex_archive::{ArchiveBuilder, OutputFormat};
    use sex_font::fallback::NOTO_CJK;
    use sex_font::{GEIST_MONO, GEIST_SANS};

    fn make_zip(items: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = ArchiveBuilder::new(OutputFormat::Zip);
        for &(name, data) in items {
            builder
                .add_file(name, data.len() as u64, 0, data)
                .expect("add_file");
        }
        let mut out = Vec::new();
        builder.write(&mut out, &mut no_progress).expect("write zip");
        out
    }

    fn build_two_face_ttc(f1: &[u8], f2: &[u8]) -> Vec<u8> {
        let header_len = 20usize;
        let off1 = header_len;
        let off2 = header_len + f1.len();
        let mut out = Vec::with_capacity(off2 + f2.len());
        out.extend_from_slice(b"ttcf");
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&2u32.to_be_bytes());
        out.extend_from_slice(&(off1 as u32).to_be_bytes());
        out.extend_from_slice(&(off2 as u32).to_be_bytes());
        for (sfnt, delta) in [(f1, off1 as u32), (f2, off2 as u32)] {
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
        }
        out
    }

    #[test]
    fn validates_ttf_and_otf_with_sha256_preview() {
        let ttf = validate_font_bytes(GEIST_SANS, "Geist-Regular.ttf").expect("TTF valid");
        assert_eq!(ttf.format, InputFormat::Ttf);
        assert_eq!(ttf.faces.len(), 1);
        assert_eq!(ttf.sha256_hex.len(), 64);
        assert_eq!(ttf.sha256, sha256_digest(GEIST_SANS));

        let otf = validate_font_bytes(NOTO_CJK, "NotoSansCJK.otf").expect("OTF valid");
        assert_eq!(otf.format, InputFormat::Otf);
        assert_eq!(otf.faces.len(), 1);
    }

    #[test]
    fn validates_ttc_collection_preview() {
        let ttc = build_two_face_ttc(GEIST_SANS, GEIST_MONO);
        let preview = validate_font_bytes(&ttc, "Geist.ttc").expect("TTC valid");
        assert_eq!(preview.format, InputFormat::Ttc);
        assert_eq!(preview.faces.len(), 2);
        assert!(!preview.faces[0].metadata.is_monospace);
        assert!(preview.faces[1].metadata.is_monospace);
    }

    #[test]
    fn rejects_truncated_sfnt_table() {
        let truncated = &GEIST_SANS[..256];
        let err = validate_font_bytes(truncated, "broken.ttf").unwrap_err();
        assert!(matches!(err, ValidationError::TruncatedTable { .. }));
    }

    #[test]
    fn rejects_zero_glyph_face() {
        let mut patched = GEIST_SANS.to_vec();
        let num_tables = u16::from_be_bytes([patched[4], patched[5]]) as usize;
        for i in 0..num_tables {
            let rec = 12 + i * 16;
            if &patched[rec..rec + 4] == b"maxp" {
                let off = u32::from_be_bytes([
                    patched[rec + 8],
                    patched[rec + 9],
                    patched[rec + 10],
                    patched[rec + 11],
                ]) as usize;
                patched[off + 4..off + 6].copy_from_slice(&0u16.to_be_bytes());
                break;
            }
        }
        let err = validate_font_bytes(&patched, "zero.ttf").unwrap_err();
        assert_eq!(err, ValidationError::ZeroGlyphFace { face_index: 0 });
    }

    #[test]
    fn validates_and_extracts_zip_bundle() {
        let zip = make_zip(&[
            ("Geist/Geist-Regular.ttf", GEIST_SANS),
            ("Geist/GeistMono-Regular.ttf", GEIST_MONO),
            ("Geist/OFL.txt", b"SIL Open Font License"),
            ("__MACOSX/._Geist-Regular.ttf", b"not a font"),
        ]);
        let assets = validate_input(&zip, "Geist.zip").expect("zip bundle valid");
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].preview.source_name, "Geist/Geist-Regular.ttf");
        assert_eq!(assets[1].preview.source_name, "Geist/GeistMono-Regular.ttf");
        assert_eq!(assets[0].bytes, GEIST_SANS);
    }

    #[test]
    fn rejects_zip_with_path_traversal() {
        let mut zip = make_zip(&[("aa/evil.ttf", GEIST_SANS)]);
        // Replace both local header and central directory occurrences of "aa/evil.ttf" with "../evil.ttf"
        let needle = b"aa/evil.ttf";
        let replacement = b"../evil.ttf";
        let mut replaced = 0usize;
        for i in 0..=zip.len().saturating_sub(needle.len()) {
            if &zip[i..i + needle.len()] == needle {
                zip[i..i + needle.len()].copy_from_slice(replacement);
                replaced += 1;
            }
        }
        assert_eq!(replaced, 2);
        let err = validate_bundle(&zip, "evil.zip").unwrap_err();
        assert!(matches!(err, ValidationError::UnsafeArchivePath(_)));
    }

    #[test]
    fn rejects_zip_with_corrupt_font_atomically() {
        let zip = make_zip(&[
            ("good.ttf", GEIST_SANS),
            ("corrupt.ttf", &GEIST_SANS[..128]),
        ]);
        let err = validate_bundle(&zip, "mixed.zip").unwrap_err();
        assert!(matches!(err, ValidationError::TruncatedTable { .. }));
    }
}
