// SPDX-License-Identifier: GPL-3.0-only
//! Fast, memory-bounded, fuzzed image decoders for WebP, AVIF, HEIC and GIF animation.
//!
//! Designed for SexOS confined programs (Gaze picture viewer/editor and Kaleidoscope web browser)
//! operating on the scalar soft-float or SSE userspace target (`x86_64-sex-app`).
//!
//! * Pure `#![no_std]` + `alloc`, with `#![forbid(unsafe_code)]` across the crate.
//! * Memory quota discipline: bounds check against [`Limits`] before any pixel buffer allocation.
//! * Standardized pixel representation: 32-bit `0xAARRGGBB` straight alpha.
//! * Unified streaming [`AnimationDecoder`] for Animated GIF, Animated WebP, and stills.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod animation;
pub mod avif;
pub mod error;
pub mod format;
pub mod gif;
pub mod heic;
pub mod image;
pub mod isobmff;
pub mod limits;
pub mod webp;

pub use animation::{AnimationDecoder, FrameTiming};
pub use error::Error;
pub use format::{probe, sniff, Format, Probe};
pub use image::{
    alloc_pixels, pack_argb, unpack_argb, ycbcr10_to_argb, ycbcr_to_argb, Image,
    MatrixCoefficients,
};
pub use limits::{admit, Limits};

/// Decodes an image into a newly allocated `Image` layout (`0xAARRGGBB` straight alpha).
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let fmt = sniff(bytes).ok_or(Error::UnknownFormat)?;
    match fmt {
        Format::Webp => webp::decode_webp(bytes, limits),
        Format::Gif => gif::decode_gif(bytes, limits),
        Format::Avif => avif::decode_avif(bytes, limits),
        Format::Heic => heic::decode_heic(bytes, limits),
        Format::Png => Ok(sex_image::decode(bytes, limits)?),
        Format::Jpeg => Ok(sex_image::decode(bytes, limits)?),
    }
}

/// Decodes an image directly into a caller-supplied pixel slice (`out.len() >= width * height`).
pub fn decode_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let fmt = sniff(bytes).ok_or(Error::UnknownFormat)?;
    match fmt {
        Format::Webp => webp::decode_webp_into(bytes, limits, out),
        Format::Gif => gif::decode_gif_into(bytes, limits, out),
        Format::Avif => avif::decode_avif_into(bytes, limits, out),
        Format::Heic => heic::decode_heic_into(bytes, limits, out),
        Format::Png => Ok(sex_image::decode_into(bytes, limits, out)?),
        Format::Jpeg => Ok(sex_image::decode_into(bytes, limits, out)?),
    }
}
