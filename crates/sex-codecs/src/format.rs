// SPDX-License-Identifier: GPL-3.0-only
//! Image format recognition and non-decoding header probing.

use crate::Error;

/// Image formats recognised by `sex-codecs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Format {
    Webp,
    Gif,
    Avif,
    Heic,
    Png,
    Jpeg,
}

impl Format {
    /// Canonical MIME media type for this format.
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Avif => "image/avif",
            Self::Heic => "image/heic",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

/// Metadata discovered without decoding pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Probe {
    pub format: Format,
    pub width: usize,
    pub height: usize,
    pub has_alpha: bool,
    pub is_animated: bool,
    pub frame_count: Option<usize>,
    pub bit_depth: u8,
}

/// Sniffs an image format by inspecting magic signatures and ISO container brands.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(Format::Webp);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return sniff_isobmff(bytes);
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some(Format::Png);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(Format::Jpeg);
    }
    None
}

/// Recognises ISOBMFF brands (AVIF, HEIC, MIAF).
fn sniff_isobmff(bytes: &[u8]) -> Option<Format> {
    if bytes.len() < 12 {
        return None;
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let end = if size == 0 || size > bytes.len() {
        bytes.len().min(4096)
    } else {
        size.clamp(12, bytes.len().min(4096))
    };

    let major = &bytes[8..12];
    match major {
        b"avif" | b"avis" => return Some(Format::Avif),
        b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" => return Some(Format::Heic),
        _ => {}
    }

    if end > 16 {
        for chunk in bytes[16..end].chunks_exact(4) {
            match chunk {
                b"avif" | b"avis" => return Some(Format::Avif),
                b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" => return Some(Format::Heic),
                _ => {}
            }
        }
    }

    None
}

/// Quickly inspects file headers to gather metadata without decoding pixel buffers.
pub fn probe(bytes: &[u8]) -> Result<Probe, Error> {
    let fmt = sniff(bytes).ok_or(Error::UnknownFormat)?;
    match fmt {
        Format::Webp => crate::webp::probe_webp(bytes),
        Format::Gif => crate::gif::probe_gif(bytes),
        Format::Avif => crate::avif::probe_avif(bytes),
        Format::Heic => crate::heic::probe_heic(bytes),
        Format::Png => {
            let (f, w, h) = sex_image::dimensions(bytes)?;
            if f != sex_image::Format::Png {
                return Err(Error::UnknownFormat);
            }
            Ok(Probe {
                format: Format::Png,
                width: w,
                height: h,
                has_alpha: true,
                is_animated: false,
                frame_count: Some(1),
                bit_depth: 8,
            })
        }
        Format::Jpeg => {
            let (f, w, h) = sex_image::dimensions(bytes)?;
            if f != sex_image::Format::Jpeg {
                return Err(Error::UnknownFormat);
            }
            Ok(Probe {
                format: Format::Jpeg,
                width: w,
                height: h,
                has_alpha: false,
                is_animated: false,
                frame_count: Some(1),
                bit_depth: 8,
            })
        }
    }
}
