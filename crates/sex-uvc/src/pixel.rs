// SPDX-License-Identifier: GPL-3.0-only
//! Pixel decoders converting YUY2 (4:2:2), NV12 (4:2:0), and MJPEG into 0xFFRRGGBB.
//!
//! Uses pure integer BT.601 math with zero float panics and strict bounds checking.

use crate::descriptors::FormatKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelError {
    BufferTooShort,
    BufferTooSmall,
    InvalidDimensions,
    MjpegDecodeFailed,
    UnsupportedFormat,
}

#[inline(always)]
pub fn yuv_to_argb(y: u8, u: u8, v: u8) -> u32 {
    let y = y as i32;
    let u = u as i32 - 128;
    let v = v as i32 - 128;
    let c = (y - 16).max(0);

    let r = (298 * c + 409 * v + 128) >> 8;
    let g = (298 * c - 100 * u - 208 * v + 128) >> 8;
    let b = (298 * c + 516 * u + 128) >> 8;

    let r = r.clamp(0, 255) as u32;
    let g = g.clamp(0, 255) as u32;
    let b = b.clamp(0, 255) as u32;

    0xFF00_0000 | (r << 16) | (g << 8) | b
}

/// Converts YUY2 (YUV 4:2:2, 2 bytes/pixel) to 0xFFRRGGBB.
/// 4 bytes: `[Y0, U0, Y1, V0]` produces two pixels: `(Y0, U0, V0)` and `(Y1, U0, V0)`.
pub fn yuy2_to_argb(yuy2: &[u8], width: usize, height: usize, out: &mut [u32]) -> Result<(), PixelError> {
    if width == 0 || height == 0 || (width & 1) != 0 {
        return Err(PixelError::InvalidDimensions);
    }
    let total_pixels = width.checked_mul(height).ok_or(PixelError::InvalidDimensions)?;
    let needed_bytes = total_pixels.checked_mul(2).ok_or(PixelError::InvalidDimensions)?;

    if yuy2.len() < needed_bytes {
        return Err(PixelError::BufferTooShort);
    }
    if out.len() < total_pixels {
        return Err(PixelError::BufferTooSmall);
    }

    let pairs = total_pixels / 2;
    for i in 0..pairs {
        let src = i * 4;
        let y0 = yuy2[src];
        let u = yuy2[src + 1];
        let y1 = yuy2[src + 2];
        let v = yuy2[src + 3];

        let dst = i * 2;
        out[dst] = yuv_to_argb(y0, u, v);
        out[dst + 1] = yuv_to_argb(y1, u, v);
    }

    Ok(())
}

/// Converts NV12 (YUV 4:2:0 planar/semi-planar) to 0xFFRRGGBB.
/// Y plane: `width * height` bytes.
/// UV plane: `(width / 2) * (height / 2) * 2 = width * height / 2` bytes interleaved `[U, V]`.
pub fn nv12_to_argb(nv12: &[u8], width: usize, height: usize, out: &mut [u32]) -> Result<(), PixelError> {
    if width == 0 || height == 0 || (width & 1) != 0 || (height & 1) != 0 {
        return Err(PixelError::InvalidDimensions);
    }
    let total_pixels = width.checked_mul(height).ok_or(PixelError::InvalidDimensions)?;
    let y_size = total_pixels;
    let uv_size = total_pixels / 2;
    let total_needed = y_size.checked_add(uv_size).ok_or(PixelError::InvalidDimensions)?;

    if nv12.len() < total_needed {
        return Err(PixelError::BufferTooShort);
    }
    if out.len() < total_pixels {
        return Err(PixelError::BufferTooSmall);
    }

    let y_plane = &nv12[..y_size];
    let uv_plane = &nv12[y_size..y_size + uv_size];

    for row in 0..height {
        let uv_row = row / 2;
        let y_row_start = row * width;
        let uv_row_start = uv_row * width;

        for col in 0..width {
            let y_val = y_plane[y_row_start + col];
            let uv_col = col & !1;
            let u_val = uv_plane[uv_row_start + uv_col];
            let v_val = uv_plane[uv_row_start + uv_col + 1];

            out[y_row_start + col] = yuv_to_argb(y_val, u_val, v_val);
        }
    }

    Ok(())
}

/// Standard 420-byte ITU-T T.81 Annex K Huffman table (`DHT`) segment for MJPEG frames
/// that omit Huffman tables (standard for UVC webcams).
/// Contains DC luminance (K.3), AC luminance (K.5), DC chrominance (K.4), and AC chrominance (K.6).
const STANDARD_MJPEG_DHT: [u8; 420] = [
    0xFF, 0xC4, 0x01, 0xA2,
    // DC Luminance (Tc=0, Th=0)
    0x00,
    0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
    // AC Luminance (Tc=1, Th=0)
    0x10,
    0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D,
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
    0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
    // DC Chrominance (Tc=0, Th=1)
    0x01,
    0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
    // AC Chrominance (Tc=1, Th=1)
    0x11,
    0x00, 0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77,
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0,
    0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
    0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
    0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3,
    0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
    0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];

/// Decodes MJPEG bytes directly into `out` as `0xFFRRGGBB` via `sex_image::decode_into`,
/// injecting standard ITU-T T.81 Annex K Huffman tables (`DHT`) before `SOS` and appending
/// `EOI` (`0xFF 0xD9`) when omitted by UVC webcams. Returns `(width, height)`.
pub fn mjpeg_to_argb(mjpeg: &[u8], out: &mut [u32]) -> Result<(usize, usize), PixelError> {
    if mjpeg.len() >= 4 && mjpeg[0] == 0xFF && mjpeg[1] == 0xD8 {
        let mut pos = 2usize;
        let mut has_dht = false;
        let mut sos_pos: Option<usize> = None;

        while pos + 1 < mjpeg.len() {
            if mjpeg[pos] != 0xFF {
                break;
            }
            let marker_start = pos;
            while pos < mjpeg.len() && mjpeg[pos] == 0xFF {
                pos += 1;
            }
            if pos >= mjpeg.len() {
                break;
            }
            let marker = mjpeg[pos];
            pos += 1;
            if marker == 0x00 || marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
                continue;
            }
            if marker == 0xD9 {
                break;
            }
            if marker == 0xDA {
                sos_pos = Some(marker_start);
                break;
            }
            if marker == 0xC4 {
                has_dht = true;
            }
            if pos + 2 > mjpeg.len() {
                break;
            }
            let seg_len = u16::from_be_bytes([mjpeg[pos], mjpeg[pos + 1]]) as usize;
            if seg_len < 2 || pos + seg_len > mjpeg.len() {
                break;
            }
            pos += seg_len;
        }

        let has_eoi = mjpeg.len() >= 2
            && mjpeg[mjpeg.len() - 2] == 0xFF
            && mjpeg[mjpeg.len() - 1] == 0xD9;
        let needs_dht = !has_dht && sos_pos.is_some();
        let needs_eoi = !has_eoi && sos_pos.is_some();

        if needs_dht || needs_eoi {
            let extra = if needs_dht { STANDARD_MJPEG_DHT.len() } else { 0 }
                + if needs_eoi { 2 } else { 0 };
            let mut patched = alloc::vec::Vec::with_capacity(mjpeg.len() + extra);
            if let Some(sos) = sos_pos {
                if needs_dht {
                    patched.extend_from_slice(&mjpeg[..sos]);
                    patched.extend_from_slice(&STANDARD_MJPEG_DHT);
                    patched.extend_from_slice(&mjpeg[sos..]);
                } else {
                    patched.extend_from_slice(mjpeg);
                }
            }
            if needs_eoi {
                patched.extend_from_slice(&[0xFF, 0xD9]);
            }
            return sex_image::decode_into(&patched, &sex_image::Limits::PHOTO, out).map_err(|e| {
                match e {
                    sex_image::Error::TooLarge => PixelError::BufferTooSmall,
                    _ => PixelError::MjpegDecodeFailed,
                }
            });
        }
    }

    sex_image::decode_into(mjpeg, &sex_image::Limits::PHOTO, out).map_err(|e| match e {
        sex_image::Error::TooLarge => PixelError::BufferTooSmall,
        _ => PixelError::MjpegDecodeFailed,
    })
}

/// Scales `src` (`src_w` x `src_h`) into `dst` (`dst_w` x `dst_h`) using nearest-neighbor
/// sampling, preserving aspect ratio with letterboxing inside `(dst_w, dst_h)`.
/// Returns the destination sub-rectangle `(ox, oy, rw, rh)` that was drawn.
pub fn scale_argb_fit(
    src: &[u32],
    src_w: usize,
    src_h: usize,
    dst: &mut [u32],
    dst_w: usize,
    dst_h: usize,
) -> (usize, usize, usize, usize) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (0, 0, 0, 0);
    }
    if src.len() < src_w.saturating_mul(src_h) || dst.len() < dst_w.saturating_mul(dst_h) {
        return (0, 0, 0, 0);
    }
    let (rw, rh) = if src_w.saturating_mul(dst_h) >= src_h.saturating_mul(dst_w) {
        (dst_w, (src_h.saturating_mul(dst_w) / src_w).max(1).min(dst_h))
    } else {
        ((src_w.saturating_mul(dst_h) / src_h).max(1).min(dst_w), dst_h)
    };
    let ox = (dst_w - rw) / 2;
    let oy = (dst_h - rh) / 2;
    for dy in 0..rh {
        let sy = (dy * src_h) / rh;
        let src_row = sy * src_w;
        let dst_row = (oy + dy) * dst_w + ox;
        for dx in 0..rw {
            let sx = (dx * src_w) / rw;
            dst[dst_row + dx] = src[src_row + sx];
        }
    }
    (ox, oy, rw, rh)
}

/// General frame decoder based on negotiated FormatKind.
pub fn decode_frame(
    format: FormatKind,
    raw: &[u8],
    width: usize,
    height: usize,
    out: &mut [u32],
) -> Result<(usize, usize), PixelError> {
    match format {
        FormatKind::Yuy2 => {
            yuy2_to_argb(raw, width, height, out)?;
            Ok((width, height))
        }
        FormatKind::Nv12 => {
            nv12_to_argb(raw, width, height, out)?;
            Ok((width, height))
        }
        FormatKind::Mjpeg => mjpeg_to_argb(raw, out),
        FormatKind::Unknown(_) => Err(PixelError::UnsupportedFormat),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Valid 1x1 baseline JPEG (reddish/gray) with standard Huffman table.
    const TINY_JPEG_1X1: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00,
        0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00,
        0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14,
        0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12, 0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A,
        0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C,
        0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32,
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00,
        0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A,
        0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05,
        0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31,
        0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42,
        0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18,
        0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43,
        0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A,
        0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8,
        0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6,
        0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2,
        0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xFB, 0xD5, 0x00, 0x00, 0xFF, 0xD9,
    ];

    #[test]
    fn yuy2_decodes_white_and_black_pixels() {
        // Y0=235 (white), U=128, Y1=16 (black), V=128
        let yuy2 = [235u8, 128, 16, 128];
        let mut out = [0u32; 2];
        yuy2_to_argb(&yuy2, 2, 1, &mut out).expect("yuy2 decode");
        assert_eq!(out[0], 0xFFFF_FFFF);
        assert_eq!(out[1], 0xFF00_0000);
    }

    #[test]
    fn mjpeg_decodes_valid_frame_and_checks_buffer() {
        let mut out = [0u32; 4];
        let (w, h) = decode_frame(FormatKind::Mjpeg, TINY_JPEG_1X1, 1, 1, &mut out).expect("mjpeg decode");
        assert_eq!((w, h), (1, 1));
        assert_eq!(out[0] >> 24, 0xFF);
        // Too-small output buffer is rejected cleanly
        let mut empty = [];
        assert_eq!(mjpeg_to_argb(TINY_JPEG_1X1, &mut empty), Err(PixelError::BufferTooSmall));
    }

    #[test]
    fn mjpeg_decodes_dht_stripped_frame_and_missing_eoi() {
        // Strip both DHT segments from TINY_JPEG_1X1 to mimic UVC MJPEG frames.
        let mut stripped = Vec::new();
        stripped.extend_from_slice(&TINY_JPEG_1X1[..2]);
        let mut pos = 2usize;
        while pos + 1 < TINY_JPEG_1X1.len() {
            let start = pos;
            if TINY_JPEG_1X1[pos] != 0xFF {
                break;
            }
            let marker = TINY_JPEG_1X1[pos + 1];
            if marker == 0xDA {
                stripped.extend_from_slice(&TINY_JPEG_1X1[start..]);
                break;
            }
            let seg_len = u16::from_be_bytes([TINY_JPEG_1X1[pos + 2], TINY_JPEG_1X1[pos + 3]]) as usize;
            let end = pos + 2 + seg_len;
            if marker != 0xC4 {
                stripped.extend_from_slice(&TINY_JPEG_1X1[start..end]);
            }
            pos = end;
        }

        let mut expected = [0u32; 4];
        mjpeg_to_argb(TINY_JPEG_1X1, &mut expected).expect("baseline jpeg");

        // 1. DHT-stripped MJPEG with EOI
        let mut out = [0u32; 4];
        let (w, h) = mjpeg_to_argb(&stripped, &mut out).expect("dht-stripped mjpeg decode");
        assert_eq!((w, h), (1, 1));
        assert_eq!(out[0], expected[0]);

        // 2. DHT-stripped MJPEG also missing trailing EOI (0xFF 0xD9)
        let without_eoi = &stripped[..stripped.len() - 2];
        let mut out2 = [0u32; 4];
        let (w2, h2) = mjpeg_to_argb(without_eoi, &mut out2).expect("dht+eoi-stripped mjpeg decode");
        assert_eq!((w2, h2), (1, 1));
        assert_eq!(out2[0], expected[0]);
    }

    #[test]
    fn scale_argb_fit_preserves_aspect_ratio() {
        // 16:9 source (16x9) fitted into 4:3 destination (16x12) -> letterboxed 16x9 at oy=1
        let src = vec![0xFF11_2233u32; 16 * 9];
        let mut dst = vec![0u32; 16 * 12];
        let (ox, oy, rw, rh) = scale_argb_fit(&src, 16, 9, &mut dst, 16, 12);
        assert_eq!((ox, oy, rw, rh), (0, 1, 16, 9));
        assert_eq!(dst[0], 0);
        assert_eq!(dst[16], 0xFF11_2233);
        assert_eq!(dst[10 * 16], 0);
    }
}
