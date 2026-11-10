//! Decoding of WebP images.
//!
//! Vendored from image-webp 0.2.4 (MIT OR Apache-2.0) for SexOS: no_std + alloc,
//! the encoder and tests left out, `std::io` replaced by the in-memory `io` module.

#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs, deprecated, clippy::all)]
extern crate alloc;

pub use self::decoder::{
    DecodingError, LoopCount, UpsamplingMethod, WebPDecodeOptions, WebPDecoder,
};
pub use self::io::Cursor;

pub mod io;
mod alpha_blending;
mod decoder;
mod extended;
mod huffman;
mod loop_filter;
mod lossless;
mod lossless_transform;
mod transform;
mod vp8_arithmetic_decoder;
mod yuv;

pub mod vp8;
