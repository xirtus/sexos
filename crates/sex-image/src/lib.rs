// SPDX-License-Identifier: GPL-3.0-only
//! PNG and JPEG decoding for confined SexOS programs.
//!
//! This crate wraps the pure Rust `zune-png` and `zune-jpeg` decoders
//! (MIT/Apache-2.0/Zlib) with `no_std` + `alloc` and no SIMD, so it runs in
//! a SexOS program on the scalar soft-float target. Decoding is meant to
//! happen inside the program that asked for the picture, never in Display or
//! a server: a hostile file can at worst fault the program that opened it.
//!
//! Every decode is bounded by [`Limits`] before the pixel buffer is
//! allocated, and that allocation is fallible, so an oversized or lying
//! header returns an error instead of exhausting the program's heap.
//!
//! Pixels come out as unpremultiplied `0xAARRGGBB`, the same layout as
//! `sex_graphics::png` and `sex_graphics::qoi`.

#![no_std]
#![forbid(unsafe_op_in_unsafe_fn)]

extern crate alloc;

use alloc::vec::Vec;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

/// Largest side any header may announce (the JPEG maximum).
const HEADER_MAX: usize = u16::MAX as usize;

/// The picture formats this crate recognises.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The bytes are neither PNG nor JPEG.
    UnknownFormat,
    /// The file is larger than `Limits::max_input_bytes`.
    InputTooLarge,
    /// The header announces a picture larger than the limits allow.
    TooLarge,
    /// The pixel buffer could not be allocated.
    OutOfMemory,
    /// The PNG decoder refused the file.
    Png,
    /// The JPEG decoder refused the file.
    Jpeg,
    /// The decoder produced a colour layout this crate does not convert.
    Unsupported,
}

/// Upper bounds checked before any pixel memory is reserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_width: usize,
    pub max_height: usize,
    /// Width times height; bounds memory independently of the aspect ratio.
    pub max_pixels: usize,
    pub max_input_bytes: usize,
}

impl Limits {
    /// Enough for a 12 megapixel photo (48 MiB of pixels).
    pub const PHOTO: Limits = Limits {
        max_width: 8192,
        max_height: 8192,
        max_pixels: 12_000_000,
        max_input_bytes: 32 << 20,
    };

    /// A program's 800x600 pixel surface.
    pub const SURFACE: Limits = Limits {
        max_width: 800,
        max_height: 600,
        max_pixels: 800 * 600,
        max_input_bytes: 8 << 20,
    };

    fn admit(&self, width: usize, height: usize) -> Result<usize, Error> {
        if width == 0 || height == 0 {
            return Err(Error::TooLarge);
        }
        if width > self.max_width || height > self.max_height {
            return Err(Error::TooLarge);
        }
        let pixels = width.checked_mul(height).ok_or(Error::TooLarge)?;
        if pixels > self.max_pixels {
            return Err(Error::TooLarge);
        }
        Ok(pixels)
    }
}

/// A decoded picture: `pixels.len() == width * height`, rows top to bottom.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Image {
    /// The pixel at (`x`, `y`), or `None` outside the picture.
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x < self.width && y < self.height {
            self.pixels.get(y * self.width + x).copied()
        } else {
            None
        }
    }

    /// Shrinks the picture to fit inside `max_width` x `max_height`, keeping
    /// its aspect ratio, by averaging each source box (alpha weighted). A
    /// picture that already fits is returned unchanged.
    pub fn fit_within(self, max_width: usize, max_height: usize) -> Result<Image, Error> {
        if max_width == 0 || max_height == 0 {
            return Err(Error::TooLarge);
        }
        if self.width <= max_width && self.height <= max_height {
            return Ok(self);
        }
        // Scale by the tighter of the two ratios, rounding to at least 1.
        let (w, h) = if self.width * max_height >= self.height * max_width {
            (max_width, (self.height * max_width / self.width).max(1))
        } else {
            ((self.width * max_height / self.height).max(1), max_height)
        };
        let mut out = Vec::new();
        out.try_reserve_exact(w * h).map_err(|_| Error::OutOfMemory)?;
        for oy in 0..h {
            let y0 = oy * self.height / h;
            let y1 = ((oy + 1) * self.height / h).max(y0 + 1);
            for ox in 0..w {
                let x0 = ox * self.width / w;
                let x1 = ((ox + 1) * self.width / w).max(x0 + 1);
                let (mut a, mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
                for y in y0..y1 {
                    let row = &self.pixels[y * self.width..(y + 1) * self.width];
                    for &p in &row[x0..x1] {
                        let pa = (p >> 24) as u64;
                        a += pa;
                        r += pa * ((p >> 16) & 0xff) as u64;
                        g += pa * ((p >> 8) & 0xff) as u64;
                        b += pa * (p & 0xff) as u64;
                        n += 1;
                    }
                }
                let pixel = if a == 0 {
                    0
                } else {
                    let ca = (a + n / 2) / n;
                    let cr = (r + a / 2) / a;
                    let cg = (g + a / 2) / a;
                    let cb = (b + a / 2) / a;
                    ((ca as u32) << 24) | ((cr as u32) << 16) | ((cg as u32) << 8) | cb as u32
                };
                out.push(pixel);
            }
        }
        Ok(Image { width: w, height: h, pixels: out })
    }
}

/// Recognises a file by its leading signature bytes.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Format::Jpeg)
    } else {
        None
    }
}

/// Reads only the header: the format and the picture's width and height.
pub fn dimensions(bytes: &[u8]) -> Result<(Format, usize, usize), Error> {
    match sniff(bytes).ok_or(Error::UnknownFormat)? {
        Format::Png => {
            let mut decoder = zune_png::PngDecoder::new_with_options(ZCursor::new(bytes), png_options());
            decoder.decode_headers().map_err(|_| Error::Png)?;
            let (w, h) = decoder.dimensions().ok_or(Error::Png)?;
            Ok((Format::Png, w, h))
        }
        Format::Jpeg => {
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), jpeg_options());
            decoder.decode_headers().map_err(|_| Error::Jpeg)?;
            let info = decoder.info().ok_or(Error::Jpeg)?;
            Ok((Format::Jpeg, info.width as usize, info.height as usize))
        }
    }
}

/// Decodes a PNG or JPEG file, chosen by its signature, into a new buffer.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let (_, width, height) = dimensions(bytes)?;
    let mut pixels = reserve_pixels(limits.admit(width, height)?)?;
    decode_into(bytes, limits, &mut pixels)?;
    Ok(Image { width, height, pixels })
}

/// Decodes into memory the caller already holds, such as a program's pixel
/// surface, and returns the picture's width and height. Rows are packed
/// (`width` pixels each) at the front of `out`; a picture with more pixels
/// than `out` holds is refused as `TooLarge`. The decoders still allocate
/// their own working memory from the heap.
pub fn decode_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    match sniff(bytes).ok_or(Error::UnknownFormat)? {
        Format::Png => png_into(bytes, limits, out),
        Format::Jpeg => jpeg_into(bytes, limits, out),
    }
}

fn png_options() -> DecoderOptions {
    // zune's own size check reports a generic format error; let any header
    // through and apply `Limits::admit` right after it, before the pixel
    // buffer exists, so callers get `Error::TooLarge`.
    DecoderOptions::default()
        .set_max_width(HEADER_MAX)
        .set_max_height(HEADER_MAX)
        .png_set_strip_to_8bit(true)
        .png_set_confirm_crc(true)
}

fn jpeg_options() -> DecoderOptions {
    // zune's own size check reports a generic format error; let any header
    // through and apply `Limits::admit` right after it, before the pixel
    // buffer exists, so callers get `Error::TooLarge`.
    DecoderOptions::default()
        .set_max_width(HEADER_MAX)
        .set_max_height(HEADER_MAX)
        // JPEG carries no alpha. RGB rather than RGBA also avoids a zune-jpeg
        // 0.5 bug: its CMYK to RGBA path writes 3-byte strides into 4-byte
        // pixels. `expand_in_place` adds the opaque alpha.
        .jpeg_set_out_colorspace(ColorSpace::RGB)
}

/// Decodes a PNG (any colour type and depth, palette, tRNS, Adam7). For an
/// animated PNG this is the first frame.
fn png_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    let mut decoder = zune_png::PngDecoder::new_with_options(ZCursor::new(bytes), png_options());
    decoder.decode_headers().map_err(|_| Error::Png)?;
    let (width, height) = decoder.dimensions().ok_or(Error::Png)?;
    let count = limits.admit(width, height)?;
    let colorspace = decoder.colorspace().ok_or(Error::Png)?;
    let size = decoder.output_buffer_size().ok_or(Error::Png)?;
    let bytes_out = target(out, count, size)?;
    decoder.decode_into(&mut bytes_out[..size]).map_err(|_| Error::Png)?;
    expand_in_place(bytes_out, count, colorspace)?;
    Ok((width, height))
}

/// Decodes a baseline or progressive JPEG (grayscale, YCbCr, CMYK, YCCK).
fn jpeg_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), jpeg_options());
    decoder.decode_headers().map_err(|_| Error::Jpeg)?;
    let info = decoder.info().ok_or(Error::Jpeg)?;
    let (width, height) = (info.width as usize, info.height as usize);
    let count = limits.admit(width, height)?;
    let colorspace = decoder.output_colorspace().ok_or(Error::Jpeg)?;
    let size = decoder.output_buffer_size().ok_or(Error::Jpeg)?;
    let bytes_out = target(out, count, size)?;
    decoder.decode_into(&mut bytes_out[..size]).map_err(|_| Error::Jpeg)?;
    expand_in_place(bytes_out, count, colorspace)?;
    Ok((width, height))
}

/// The first `count` pixels of `out` as bytes, if they fit and can hold the
/// decoder's `size` bytes of packed samples.
fn target(out: &mut [u32], count: usize, size: usize) -> Result<&mut [u8], Error> {
    let pixels = out.get_mut(..count).ok_or(Error::TooLarge)?;
    let bytes = as_bytes(pixels);
    if size > bytes.len() {
        return Err(Error::Unsupported);
    }
    Ok(bytes)
}

fn reserve_pixels(count: usize) -> Result<Vec<u32>, Error> {
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count).map_err(|_| Error::OutOfMemory)?;
    pixels.resize(count, 0);
    Ok(pixels)
}

fn as_bytes(pixels: &mut [u32]) -> &mut [u8] {
    let len = core::mem::size_of_val(pixels);
    // SAFETY: u8 has alignment 1 and every byte pattern is a valid u32, so
    // viewing the initialised u32 buffer as bytes for its full length is sound.
    unsafe { core::slice::from_raw_parts_mut(pixels.as_mut_ptr().cast::<u8>(), len) }
}

/// Turns `count` packed samples of `colorspace` at the front of `buf` into
/// native-endian `0xAARRGGBB` words filling the whole buffer. Works from the
/// last pixel back, so a pixel's 4-byte destination never overlaps an
/// earlier pixel's still unread source bytes.
fn expand_in_place(buf: &mut [u8], count: usize, colorspace: ColorSpace) -> Result<(), Error> {
    let channels = match colorspace {
        ColorSpace::Luma => 1,
        ColorSpace::LumaA => 2,
        ColorSpace::RGB => 3,
        ColorSpace::RGBA => 4,
        _ => return Err(Error::Unsupported),
    };
    if buf.len() < count * 4 {
        return Err(Error::Unsupported);
    }
    for i in (0..count).rev() {
        let s = &buf[i * channels..i * channels + channels];
        let (r, g, b, a) = match channels {
            1 => (s[0], s[0], s[0], 0xff),
            2 => (s[0], s[0], s[0], s[1]),
            3 => (s[0], s[1], s[2], 0xff),
            _ => (s[0], s[1], s[2], s[3]),
        };
        let word = u32::from_be_bytes([a, r, g, b]);
        buf[i * 4..i * 4 + 4].copy_from_slice(&word.to_ne_bytes());
    }
    Ok(())
}
