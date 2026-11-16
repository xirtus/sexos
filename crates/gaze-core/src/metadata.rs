// SPDX-License-Identifier: GPL-3.0-only
//! Unified photo metadata extraction (EXIF + PNG text chunks + filesystem mtime)
//! and deterministic JPEG EXIF APP1 fixture builder.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::gazetteer::resolve_place;

/// Civil calendar timestamp extracted from EXIF `DateTimeOriginal`, PNG text chunks, or `mtime`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhotoDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl PhotoDate {
    pub const fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<Self> {
        if year == 0 || month == 0 || month > 12 || day == 0 || day > 31 || hour > 23 || minute > 59 || second > 60 {
            return None;
        }
        Some(Self { year, month, day, hour, minute, second })
    }

    /// Converts Unix epoch timestamp (seconds, milliseconds, or nanoseconds) into UTC `PhotoDate`
    /// using Howard Hinnant's `civil_from_days` algorithm. Returns `None` for `0`.
    pub fn from_unix_timestamp(raw: u64) -> Option<Self> {
        if raw == 0 {
            return None;
        }
        let secs = if raw >= 100_000_000_000_000_000 {
            raw / 1_000_000_000
        } else if raw >= 100_000_000_000 {
            raw / 1_000
        } else {
            raw
        };
        // Ignore small monotonic tick values that are not real calendar epochs (< 1973-03-03).
        if secs < 100_000_000 {
            return None;
        }
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        let hour = (rem / 3600) as u8;
        let minute = ((rem % 3600) / 60) as u8;
        let second = (rem % 60) as u8;

        // Howard Hinnant civil_from_days
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = (yoe as i64) + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = (doy - (153 * mp + 2) / 5 + 1) as u8; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8; // [1, 12]
        let year_i64 = if m <= 2 { y + 1 } else { y };
        if !(1..=9999).contains(&year_i64) {
            return None;
        }
        Self::new(year_i64 as u16, m, d, hour, minute, second)
    }

    /// Parses EXIF (`2026:05:14 16:30:00`), ISO-8601 (`2026-05-14T16:30:00Z`), or `YYYY-MM-DD`.
    pub fn parse_flexible(input: &str) -> Option<Self> {
        let s = input.trim();
        let b = s.as_bytes();
        if b.len() < 10 {
            return None;
        }
        let digits = |slice: &[u8]| -> Option<u32> {
            if slice.is_empty() {
                return None;
            }
            let mut acc = 0u32;
            for &ch in slice {
                if !ch.is_ascii_digit() {
                    return None;
                }
                acc = acc.checked_mul(10)?.checked_add((ch - b'0') as u32)?;
            }
            Some(acc)
        };
        if !matches!(b[4], b':' | b'-' | b'/') || !matches!(b[7], b':' | b'-' | b'/') {
            return None;
        }
        let year = digits(&b[0..4])? as u16;
        let month = digits(&b[5..7])? as u8;
        let day = digits(&b[8..10])? as u8;

        let (hour, minute, second) = if b.len() >= 19
            && matches!(b[10], b' ' | b'T' | b't' | b'_')
            && b[13] == b':'
            && b[16] == b':'
        {
            (
                digits(&b[11..13])? as u8,
                digits(&b[14..16])? as u8,
                digits(&b[17..19])? as u8,
            )
        } else if b.len() >= 16
            && matches!(b[10], b' ' | b'T' | b't' | b'_')
            && b[13] == b':'
        {
            (digits(&b[11..13])? as u8, digits(&b[14..16])? as u8, 0)
        } else {
            (0, 0, 0)
        };

        Self::new(year, month, day, hour, minute, second)
    }

    pub fn format_ymd(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn format_ym(&self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    pub fn format_datetime(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    pub fn format_short_datetime(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }

    pub fn sort_key(&self) -> u64 {
        ((self.year as u64) << 32)
            | ((self.month as u64) << 24)
            | ((self.day as u64) << 16)
            | ((self.hour as u64) << 10)
            | ((self.minute as u64) << 4)
            | ((self.second as u64) & 0xF)
    }
}

/// Unified faces-free photo metadata stored in the Gaze library catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhotoMeta {
    pub width: u32,
    pub height: u32,
    pub format_name: String,
    pub date: Option<PhotoDate>,
    pub gps: Option<(f64, f64)>,
    /// Display place name, e.g. `"Paris, France"`.
    pub place: Option<String>,
    /// Region + continent tokens for rich offline smart search, e.g. `"Ile-de-France Europe"`.
    pub place_region: Option<String>,
    pub camera: Option<String>,
    /// Lens model, e.g. `"EF 50mm f/1.8"`.
    pub lens: Option<String>,
    /// Shutter, aperture, ISO and focal length in one line, e.g. `"1/250 s · f/2.8 · ISO 400 · 35 mm"`.
    pub exposure: Option<String>,
}

/// Extracts dimensions, format, date taken, GPS, offline-resolved place, and camera model
/// from image bytes and optional filesystem `mtime`.
pub fn extract_metadata(bytes: &[u8], mtime_raw: Option<u64>) -> PhotoMeta {
    let mut meta = PhotoMeta::default();

    if let Some(fmt) = gaze_codecs::sniff(bytes) {
        meta.format_name = match fmt {
            gaze_codecs::Format::Png => "PNG",
            gaze_codecs::Format::Jpeg => "JPEG",
            gaze_codecs::Format::Webp => "WebP",
            gaze_codecs::Format::Gif => "GIF",
            gaze_codecs::Format::Jxl => "JPEG XL",
            gaze_codecs::Format::Svg => "SVG",
            gaze_codecs::Format::Tiff => "TIFF",
            gaze_codecs::Format::Avif => "AVIF",
            gaze_codecs::Format::Heic => "HEIC",
            _ => "RAW",
        }
        .to_string();
    }

    if let Ok((_, w, h)) = sex_image::dimensions(bytes) {
        meta.width = w as u32;
        meta.height = h as u32;
    }

    if let Some(exif) = gaze_codecs::Exif::read(bytes) {
        if meta.width == 0 || meta.height == 0 {
            if let (Some(w), Some(h)) = (exif.width, exif.height) {
                meta.width = w;
                meta.height = h;
            }
        }
        if let Some(o) = exif.orientation {
            if gaze_codecs::swaps_axes(o) && meta.width > 0 && meta.height > 0 {
                core::mem::swap(&mut meta.width, &mut meta.height);
            }
        }
        if let Some(dt) = exif.date_taken {
            meta.date = PhotoDate::new(dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second);
        }
        if let Some(gps) = exif.gps {
            meta.gps = Some((gps.latitude, gps.longitude));
            if let Some(resolved) = resolve_place(gps.latitude, gps.longitude) {
                meta.place = Some(resolved.display);
                let mut reg = resolved.region;
                if !resolved.continent.is_empty() {
                    if !reg.is_empty() {
                        reg.push(' ');
                    }
                    reg.push_str(&resolved.continent);
                }
                if !reg.is_empty() {
                    meta.place_region = Some(reg);
                }
            }
        }
        meta.camera = match (exif.make.as_deref(), exif.model.as_deref()) {
            (Some(mk), Some(md)) if !md.to_ascii_lowercase().contains(&mk.to_ascii_lowercase()) => {
                Some(format!("{} {}", mk.trim(), md.trim()))
            }
            (_, Some(md)) if !md.trim().is_empty() => Some(md.trim().to_string()),
            (Some(mk), None) if !mk.trim().is_empty() => Some(mk.trim().to_string()),
            _ => exif.software.clone().filter(|s| !s.trim().is_empty()),
        };
        meta.lens = exif.lens.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(String::from);
        meta.exposure = exposure_line(&exif);
    }

    // Also inspect uncompressed PNG tEXt / iTXt metadata chunks if PNG
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        scan_png_text_chunks(bytes, &mut meta);
    }

    // Fallback to filesystem modification timestamp when embedded date is absent
    if meta.date.is_none() {
        if let Some(raw) = mtime_raw {
            meta.date = PhotoDate::from_unix_timestamp(raw);
        }
    }

    meta
}

/// Shutter, aperture, ISO and focal length, whichever the photo records, as one line.
pub fn exposure_line(exif: &gaze_codecs::Exif) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = exif.exposure_time.filter(|t| t.num > 0 && t.den > 0) {
        parts.push(if t.num >= t.den {
            let secs = t.num as f64 / t.den as f64;
            if secs == (secs as u64) as f64 { format!("{} s", secs as u64) } else { format!("{:.1} s", secs) }
        } else {
            format!("1/{} s", (t.den as u64 + t.num as u64 / 2) / t.num as u64)
        });
    }
    if let Some(f) = exif.f_number.and_then(|f| f.to_f64()).filter(|f| *f > 0.0) {
        parts.push(if f == (f as u64) as f64 { format!("f/{}", f as u64) } else { format!("f/{:.1}", f) });
    }
    if let Some(iso) = exif.iso.filter(|i| *i > 0) {
        parts.push(format!("ISO {}", iso));
    }
    if let Some(mm) = exif.focal_length.and_then(|f| f.to_f64()).filter(|f| *f > 0.0) {
        parts.push(if mm == (mm as u64) as f64 { format!("{} mm", mm as u64) } else { format!("{:.1} mm", mm) });
    }
    if parts.is_empty() { None } else { Some(parts.join(" \u{b7} ")) }
}

fn scan_png_text_chunks(bytes: &[u8], meta: &mut PhotoMeta) {
    let mut pos = 8usize;
    let mut chunks_seen = 0usize;
    while pos.saturating_add(12) <= bytes.len() && chunks_seen < 256 {
        chunks_seen += 1;
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let tag = &bytes[pos + 4..pos + 8];
        let Some(data_end) = (pos + 8).checked_add(len) else { break };
        let Some(next_pos) = data_end.checked_add(4) else { break };
        if next_pos > bytes.len() {
            break;
        }
        let data = &bytes[pos + 8..data_end];
        if tag == b"tEXt" {
            if let Some(nul) = data.iter().position(|&b| b == 0) {
                if let (Ok(k), Ok(v)) = (
                    core::str::from_utf8(&data[..nul]),
                    core::str::from_utf8(&data[nul + 1..]),
                ) {
                    apply_png_kv(k.trim(), v.trim(), meta);
                }
            }
        } else if tag == b"iTXt" {
            if let Some(nul) = data.iter().position(|&b| b == 0) {
                // iTXt: keyword \0 comp_flag(1) comp_method(1) lang \0 translated \0 text
                if nul + 3 <= data.len() && data[nul + 1] == 0 {
                    let rest = &data[nul + 3..];
                    if let Some(n1) = rest.iter().position(|&b| b == 0) {
                        let rest2 = &rest[n1 + 1..];
                        if let Some(n2) = rest2.iter().position(|&b| b == 0) {
                            if let (Ok(k), Ok(v)) = (
                                core::str::from_utf8(&data[..nul]),
                                core::str::from_utf8(&rest2[n2 + 1..]),
                            ) {
                                apply_png_kv(k.trim(), v.trim(), meta);
                            }
                        }
                    }
                }
            }
        } else if tag == b"IEND" {
            break;
        }
        pos = next_pos;
    }
}

fn apply_png_kv(key: &str, val: &str, meta: &mut PhotoMeta) {
    if val.is_empty() {
        return;
    }
    if key.eq_ignore_ascii_case("Creation Time")
        || key.eq_ignore_ascii_case("Date")
        || key.eq_ignore_ascii_case("DateTimeOriginal")
        || key.eq_ignore_ascii_case("DateTime")
    {
        if meta.date.is_none() {
            meta.date = PhotoDate::parse_flexible(val);
        }
    } else if key.eq_ignore_ascii_case("GPS") || key.eq_ignore_ascii_case("Coordinates") {
        if meta.gps.is_none() {
            if let Some((lat, lon)) = parse_lat_lon(val) {
                meta.gps = Some((lat, lon));
                if meta.place.is_none() {
                    if let Some(resolved) = resolve_place(lat, lon) {
                        meta.place = Some(resolved.display);
                        meta.place_region = Some(format!("{} {}", resolved.region, resolved.continent));
                    }
                }
            }
        }
    } else if key.eq_ignore_ascii_case("Place") || key.eq_ignore_ascii_case("Location") {
        meta.place = Some(val.to_string());
    } else if key.eq_ignore_ascii_case("Camera") || key.eq_ignore_ascii_case("Model") {
        if meta.camera.is_none() {
            meta.camera = Some(val.to_string());
        }
    }
}

fn parse_lat_lon(s: &str) -> Option<(f64, f64)> {
    let mut parts = s.split(',');
    let lat: f64 = parts.next()?.trim().parse().ok()?;
    let lon: f64 = parts.next()?.trim().parse().ok()?;
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

/// Builds a valid JPEG APP1 (`FF E1 <len> Exif\0\0`) segment containing little-endian TIFF IFDs
/// for `DateTimeOriginal` (`"YYYY:MM:DD HH:MM:SS"`), WGS84 `GPSLatitude`/`GPSLongitude` DMS rationals,
/// and optional `Model` tag.
pub fn build_jpeg_exif_app1(
    date_exif: &str,
    lat: f64,
    lon: f64,
    camera_model: Option<&str>,
) -> Vec<u8> {
    let mut tiff = Vec::with_capacity(256);
    // Little-endian TIFF header: "II", magic 42, IFD0 at offset 8
    tiff.extend_from_slice(b"II\x2a\x00\x08\x00\x00\x00");

    let model_str = camera_model.unwrap_or("SexOS Cam");
    let mut model_bytes = model_str.as_bytes().to_vec();
    model_bytes.push(0);
    if model_bytes.len() % 2 == 1 {
        model_bytes.push(0);
    }

    let mut date_bytes = date_exif.as_bytes().to_vec();
    date_bytes.push(0);
    if date_bytes.len() % 2 == 1 {
        date_bytes.push(0);
    }

    // IFD0 has 3 entries: Model (0x0110), ExifIFD (0x8769), GPSIFD (0x8825)
    // Offset 8: u16 count (2) + 3 * 12 (36) + u32 next_ifd (4) = 42 bytes -> ends at 50
    let model_offset = 50u32;
    let exif_ifd_offset = model_offset + model_bytes.len() as u32;
    // ExifIFD has 1 entry: DateTimeOriginal (0x9003) -> 2 + 12 + 4 = 18 bytes
    let date_offset = exif_ifd_offset + 18;
    let gps_ifd_offset = date_offset + date_bytes.len() as u32;
    // GPSIFD has 4 entries: LatRef(1), Lat(2), LonRef(3), Lon(4) -> 2 + 48 + 4 = 54 bytes
    let gps_lat_offset = gps_ifd_offset + 54;
    let gps_lon_offset = gps_lat_offset + 24;

    let push_u16 = |buf: &mut Vec<u8>, v: u16| buf.extend_from_slice(&v.to_le_bytes());
    let push_u32 = |buf: &mut Vec<u8>, v: u32| buf.extend_from_slice(&v.to_le_bytes());
    let push_entry = |buf: &mut Vec<u8>, tag: u16, ty: u16, count: u32, val_or_off: u32| {
        push_u16(buf, tag);
        push_u16(buf, ty);
        push_u32(buf, count);
        push_u32(buf, val_or_off);
    };

    // IFD0
    push_u16(&mut tiff, 3);
    push_entry(&mut tiff, 0x0110, 2, (model_str.len() + 1) as u32, model_offset);
    push_entry(&mut tiff, 0x8769, 4, 1, exif_ifd_offset);
    push_entry(&mut tiff, 0x8825, 4, 1, gps_ifd_offset);
    push_u32(&mut tiff, 0);
    tiff.extend_from_slice(&model_bytes);

    // Exif SubIFD
    push_u16(&mut tiff, 1);
    push_entry(&mut tiff, 0x9003, 2, (date_exif.len() + 1) as u32, date_offset);
    push_u32(&mut tiff, 0);
    tiff.extend_from_slice(&date_bytes);

    // GPS SubIFD
    let lat_ref = if lat >= 0.0 { b'N' } else { b'S' };
    let lon_ref = if lon >= 0.0 { b'E' } else { b'W' };
    push_u16(&mut tiff, 4);
    push_entry(&mut tiff, 0x0001, 2, 2, u32::from_le_bytes([lat_ref, 0, 0, 0]));
    push_entry(&mut tiff, 0x0002, 5, 3, gps_lat_offset);
    push_entry(&mut tiff, 0x0003, 2, 2, u32::from_le_bytes([lon_ref, 0, 0, 0]));
    push_entry(&mut tiff, 0x0004, 5, 3, gps_lon_offset);
    push_u32(&mut tiff, 0);

    let write_dms = |buf: &mut Vec<u8>, deg_val: f64| {
        let abs = libm::fabs(deg_val);
        let d = libm::floor(abs) as u32;
        let rem_min = (abs - d as f64) * 60.0;
        let m = libm::floor(rem_min) as u32;
        let sec_x10000 = libm::round((rem_min - m as f64) * 60.0 * 10_000.0) as u32;
        push_u32(buf, d);
        push_u32(buf, 1);
        push_u32(buf, m);
        push_u32(buf, 1);
        push_u32(buf, sec_x10000);
        push_u32(buf, 10_000);
    };
    write_dms(&mut tiff, lat);
    write_dms(&mut tiff, lon);

    let payload_len = 6 + tiff.len(); // "Exif\0\0" + tiff
    let seg_len = (2 + payload_len) as u16;
    let mut app1 = Vec::with_capacity(2 + seg_len as usize);
    app1.extend_from_slice(&[0xFF, 0xE1]);
    app1.extend_from_slice(&seg_len.to_be_bytes());
    app1.extend_from_slice(b"Exif\0\0");
    app1.extend_from_slice(&tiff);
    app1
}

/// Injects an EXIF APP1 block immediately after the JPEG SOI (`FF D8`) marker.
pub fn inject_jpeg_exif(
    jpeg_bytes: &[u8],
    date_exif: &str,
    lat: f64,
    lon: f64,
    camera_model: Option<&str>,
) -> Vec<u8> {
    if jpeg_bytes.len() < 2 || jpeg_bytes[0] != 0xFF || jpeg_bytes[1] != 0xD8 {
        return jpeg_bytes.to_vec();
    }
    let app1 = build_jpeg_exif_app1(date_exif, lat, lon, camera_model);
    let mut out = Vec::with_capacity(jpeg_bytes.len() + app1.len());
    out.extend_from_slice(&jpeg_bytes[..2]);
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg_bytes[2..]);
    out
}
