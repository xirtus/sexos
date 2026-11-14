//! Recognising picture files by their content, never by their name.

use crate::tiff::Tiff;

/// Every picture format Gaze knows about, decodable or not.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Format {
    // Decoded by sex-image.
    Png,
    Jpeg,
    Webp,
    Gif,
    // Decoded here (behind features).
    Jxl,
    Svg,
    // Camera RAW: the embedded JPEG preview is shown.
    Dng,
    Cr2,
    Nef,
    Arw,
    Orf,
    Rw2,
    Pef,
    Raf,
    // Recognised only: decoders come later (FFmpeg for AVIF and HEIF).
    Avif,
    Heic,
    /// HEIF with a codec other than HEVC or AV1, or brands this crate does not name.
    Heif,
    /// Canon CR3 (ISO base media); its previews are not extracted yet.
    Cr3,
    Tiff,
    Bmp,
    Ico,
    Qoi,
    Psd,
    Exr,
    Hdr,
    Pnm,
}

/// What Gaze can do with a format today.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Support {
    /// Full decode (sex-image, or this crate with its feature on).
    Decode,
    /// The embedded JPEG preview of a RAW file is decoded.
    Preview,
    /// Recognised, but no decoder yet.
    Detected,
}

impl Format {
    pub fn is_raw(self) -> bool {
        matches!(self, Format::Dng | Format::Cr2 | Format::Cr3 | Format::Nef | Format::Arw | Format::Orf | Format::Rw2 | Format::Pef | Format::Raf)
    }

    /// Support in this build (JPEG XL and SVG depend on the `jxl` / `svg` features).
    pub fn support(self) -> Support {
        match self {
            Format::Png | Format::Jpeg | Format::Webp | Format::Gif => Support::Decode,
            Format::Jxl if cfg!(feature = "jxl") => Support::Decode,
            Format::Svg if cfg!(feature = "svg") => Support::Decode,
            // CR3 has no preview extraction here; rawler (feature `raw`) develops it.
            Format::Cr3 if cfg!(feature = "raw") => Support::Decode,
            Format::Cr3 => Support::Detected,
            f if f.is_raw() => Support::Preview,
            _ => Support::Detected,
        }
    }

    /// The media type name, for "open with" and sharing.
    pub fn media_type(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
            Format::Webp => "image/webp",
            Format::Gif => "image/gif",
            Format::Jxl => "image/jxl",
            Format::Svg => "image/svg+xml",
            Format::Dng => "image/x-adobe-dng",
            Format::Cr2 => "image/x-canon-cr2",
            Format::Cr3 => "image/x-canon-cr3",
            Format::Nef => "image/x-nikon-nef",
            Format::Arw => "image/x-sony-arw",
            Format::Orf => "image/x-olympus-orf",
            Format::Rw2 => "image/x-panasonic-rw2",
            Format::Pef => "image/x-pentax-pef",
            Format::Raf => "image/x-fuji-raf",
            Format::Avif => "image/avif",
            Format::Heic => "image/heic",
            Format::Heif => "image/heif",
            Format::Tiff => "image/tiff",
            Format::Bmp => "image/bmp",
            Format::Ico => "image/vnd.microsoft.icon",
            Format::Qoi => "image/qoi",
            Format::Psd => "image/vnd.adobe.photoshop",
            Format::Exr => "image/x-exr",
            Format::Hdr => "image/vnd.radiance",
            Format::Pnm => "image/x-portable-anymap",
        }
    }
}

/// Recognises a picture by its leading bytes (and, for TIFF-based RAW, its
/// first directory). `None` for anything else.
pub fn sniff(b: &[u8]) -> Option<Format> {
    if let Some(f) = sex_image::sniff(b) {
        return Some(match f {
            sex_image::Format::Png => Format::Png,
            sex_image::Format::Jpeg => Format::Jpeg,
            sex_image::Format::Webp => Format::Webp,
            sex_image::Format::Gif => Format::Gif,
        });
    }
    if b.starts_with(&[0xff, 0x0a]) || b.starts_with(&[0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a]) {
        return Some(Format::Jxl);
    }
    if b.starts_with(b"FUJIFILMCCD-RAW ") {
        return Some(Format::Raf);
    }
    if b.len() >= 12 && &b[4..8] == b"ftyp" {
        return isobmff(b);
    }
    if Tiff::parse(b).is_some() {
        return Some(tiff_kind(b));
    }
    if b.starts_with(b"BM") && b.len() >= 26 {
        return Some(Format::Bmp);
    }
    if b.starts_with(&[0, 0, 1, 0]) && b.len() >= 6 && b[4..6] != [0, 0] {
        return Some(Format::Ico);
    }
    if b.starts_with(b"qoif") {
        return Some(Format::Qoi);
    }
    if b.starts_with(b"8BPS") {
        return Some(Format::Psd);
    }
    if b.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        return Some(Format::Exr);
    }
    if b.starts_with(b"#?RADIANCE") || b.starts_with(b"#?RGBE") {
        return Some(Format::Hdr);
    }
    if b.len() >= 3 && b[0] == b'P' && (b'1'..=b'7').contains(&b[1]) && b[2].is_ascii_whitespace() {
        return Some(Format::Pnm);
    }
    if is_svg(b) {
        return Some(Format::Svg);
    }
    None
}

/// ISO base media files (HEIF, AVIF, CR3): judged by major and compatible brands.
fn isobmff(b: &[u8]) -> Option<Format> {
    let size = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
    // max() keeps the bounds ordered for inputs shorter than 16 bytes (fuzz #259).
    let end = size.clamp(16, b.len().min(4096).max(16));
    let brands = core::iter::once(&b[8..12]).chain(b.get(16..end).unwrap_or(&[]).chunks_exact(4));
    let (mut heic, mut heif) = (false, false);
    for brand in brands {
        match brand {
            b"crx " => return Some(Format::Cr3),
            b"avif" | b"avis" => return Some(Format::Avif),
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" => heic = true,
            b"mif1" | b"msf1" => heif = true,
            _ => {}
        }
    }
    if heic {
        Some(Format::Heic)
    } else if heif {
        Some(Format::Heif)
    } else {
        None
    }
}

fn tiff_kind(b: &[u8]) -> Format {
    match b.get(2..4) {
        Some(b"RO") | Some(b"RS") | Some(b"OR") | Some(b"SR") => return Format::Orf,
        Some([b'U', 0]) | Some([0, b'U']) => return Format::Rw2,
        _ => {}
    }
    if b.get(8..10) == Some(b"CR") {
        return Format::Cr2;
    }
    let Some(t) = Tiff::parse(b) else { return Format::Tiff };
    let Some((ifd0, _)) = t.ifd(t.first_ifd) else { return Format::Tiff };
    if ifd0.get(0xc612).is_some() {
        return Format::Dng;
    }
    let make = ifd0.ascii(0x010f).unwrap_or("");
    let has = |s: &str| make.len() >= s.len() && make.as_bytes()[..s.len()].eq_ignore_ascii_case(s.as_bytes());
    if has("NIKON") {
        Format::Nef
    } else if has("SONY") {
        Format::Arw
    } else if has("PENTAX") || has("RICOH") {
        Format::Pef
    } else if has("OLYMPUS") || has("OM DIGITAL") {
        Format::Orf
    } else {
        Format::Tiff
    }
}

fn is_svg(b: &[u8]) -> bool {
    let head = &b[..b.len().min(4096)];
    let head = head.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(head);
    let start = head.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(head.len());
    let head = &head[start..];
    if !head.starts_with(b"<") {
        return false;
    }
    if head.starts_with(b"<svg") {
        return true;
    }
    // An XML declaration, comments or a doctype may come before the root element.
    (head.starts_with(b"<?xml") || head.starts_with(b"<!")) && head.windows(4).any(|w| w == b"<svg")
}
