// SPDX-License-Identifier: GPL-3.0-only
//! Pixel buffer allocation, layout and color space conversions.

extern crate alloc;

use alloc::vec::Vec;
pub use sex_image::Image;

use crate::Error;

/// Safely allocates an uninitialized pixel buffer bounded by `count`.
/// Returns `Error::OutOfMemory` if reservation fails.
pub fn alloc_pixels(count: usize) -> Result<Vec<u32>, Error> {
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count).map_err(|_| Error::OutOfMemory)?;
    pixels.resize(count, 0);
    Ok(pixels)
}

/// Packs 8-bit straight ARGB channels into native 32-bit `0xAARRGGBB` word.
#[inline(always)]
pub const fn pack_argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    u32::from_be_bytes([a, r, g, b])
}

/// Unpacks native 32-bit `0xAARRGGBB` word into [a, r, g, b] bytes.
#[inline(always)]
pub const fn unpack_argb(pixel: u32) -> [u8; 4] {
    pixel.to_be_bytes()
}

/// Matrix coefficients as defined in ISO/IEC 23001-8 (CICP) Table 4.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum MatrixCoefficients {
    Identity = 0,
    Bt709 = 1,
    Unspecified = 2,
    Fcc = 4,
    Bt601 = 6,
    Smpte240 = 7,
    Ycgco = 8,
    Bt2020NonConstant = 9,
    Bt2020Constant = 10,
}

impl From<u16> for MatrixCoefficients {
    fn from(val: u16) -> Self {
        match val {
            0 => Self::Identity,
            1 => Self::Bt709,
            4 => Self::Fcc,
            6 => Self::Bt601,
            7 => Self::Smpte240,
            8 => Self::Ycgco,
            9 => Self::Bt2020NonConstant,
            10 => Self::Bt2020Constant,
            _ => Self::Bt601, // Safe fallback
        }
    }
}

/// Converts 8-bit YCbCr samples into a 32-bit `0xAARRGGBB` pixel with alpha.
#[inline]
pub fn ycbcr_to_argb(
    y: u8,
    cb: u8,
    cr: u8,
    a: u8,
    matrix: MatrixCoefficients,
    full_range: bool,
) -> u32 {
    let (r, g, b) = if full_range {
        let yf = y as i32;
        let cbf = cb as i32 - 128;
        let crf = cr as i32 - 128;

        match matrix {
            MatrixCoefficients::Bt709 => {
                let r = yf + ((crf * 403) >> 8);
                let g = yf - ((cbf * 48 + crf * 120) >> 8);
                let b = yf + ((cbf * 475) >> 8);
                (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
            }
            MatrixCoefficients::Bt2020NonConstant | MatrixCoefficients::Bt2020Constant => {
                let r = yf + ((crf * 377) >> 8);
                let g = yf - ((cbf * 42 + crf * 146) >> 8);
                let b = yf + ((cbf * 482) >> 8);
                (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
            }
            _ => {
                // Default BT.601 full range
                let r = yf + ((crf * 359) >> 8);
                let g = yf - ((cbf * 88 + crf * 183) >> 8);
                let b = yf + ((cbf * 454) >> 8);
                (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
            }
        }
    } else {
        // Limited range: Y in 16..235, Cb/Cr in 16..240
        let yf = ((y as i32 - 16).max(0) * 298) >> 8;
        let cbf = cb as i32 - 128;
        let crf = cr as i32 - 128;

        match matrix {
            MatrixCoefficients::Bt709 => {
                let r = yf + ((crf * 459) >> 8);
                let g = yf - ((cbf * 55 + crf * 136) >> 8);
                let b = yf + ((cbf * 541) >> 8);
                (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
            }
            MatrixCoefficients::Bt2020NonConstant | MatrixCoefficients::Bt2020Constant => {
                let r = yf + ((crf * 429) >> 8);
                let g = yf - ((cbf * 48 + crf * 166) >> 8);
                let b = yf + ((cbf * 550) >> 8);
                (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
            }
            _ => {
                // Default BT.601 limited range
                let r = yf + ((crf * 409) >> 8);
                let g = yf - ((cbf * 100 + crf * 208) >> 8);
                let b = yf + ((cbf * 516) >> 8);
                (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
            }
        }
    };

    pack_argb(a, r, g, b)
}

/// Converts 10-bit YCbCr samples into 32-bit `0xAARRGGBB` pixel.
#[inline]
pub fn ycbcr10_to_argb(
    y10: u16,
    cb10: u16,
    cr10: u16,
    a10: u16,
    matrix: MatrixCoefficients,
    full_range: bool,
) -> u32 {
    let y = (y10 >> 2) as u8;
    let cb = (cb10 >> 2) as u8;
    let cr = (cr10 >> 2) as u8;
    let a = (a10 >> 2) as u8;
    ycbcr_to_argb(y, cb, cr, a, matrix, full_range)
}
