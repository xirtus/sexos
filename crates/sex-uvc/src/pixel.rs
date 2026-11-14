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

/// Decodes MJPEG bytes into 0xFFRRGGBB via `sex_image::decode`.
/// Returns `(width, height)` of the decoded image.
pub fn mjpeg_to_argb(mjpeg: &[u8], out: &mut [u32]) -> Result<(usize, usize), PixelError> {
    let image = sex_image::decode(mjpeg, &sex_image::Limits::PHOTO)
        .map_err(|_| PixelError::MjpegDecodeFailed)?;

    let total = image.width.checked_mul(image.height).ok_or(PixelError::InvalidDimensions)?;
    if out.len() < total {
        return Err(PixelError::BufferTooSmall);
    }

    out[..total].copy_from_slice(&image.pixels);
    Ok((image.width, image.height))
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
