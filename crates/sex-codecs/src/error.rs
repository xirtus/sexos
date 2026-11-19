// SPDX-License-Identifier: GPL-3.0-only
//! Error types for SexOS image decoders.

use core::fmt;

/// All errors returned by `sex-codecs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Format could not be determined from the input stream.
    UnknownFormat,
    /// Input byte slice exceeds `Limits::max_input_bytes`.
    InputTooLarge,
    /// Dimensions or pixel count exceeds allowed limits.
    TooLarge,
    /// Heap allocation failed under memory pressure.
    OutOfMemory,
    /// Bitstream syntax or container is malformed or truncated.
    CorruptData,
    /// Valid format features that are not supported (e.g. unsupported bit depth or colorspace).
    Unsupported,
    /// WebP decoding error.
    Webp,
    /// GIF decoding error.
    Gif,
    /// AVIF/AV1 decoding error.
    Avif,
    /// HEIC/HEVC decoding error.
    Heic,
    /// Underlying sex-image error.
    Image(sex_image::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => write!(f, "unknown or unrecognized image format"),
            Self::InputTooLarge => write!(f, "input file exceeds byte quota"),
            Self::TooLarge => write!(f, "image dimensions exceed allowed limits"),
            Self::OutOfMemory => write!(f, "memory allocation failed"),
            Self::CorruptData => write!(f, "corrupt or truncated bitstream"),
            Self::Unsupported => write!(f, "unsupported format feature"),
            Self::Webp => write!(f, "webp decoding failed"),
            Self::Gif => write!(f, "gif decoding failed"),
            Self::Avif => write!(f, "avif decoding failed"),
            Self::Heic => write!(f, "heic decoding failed"),
            Self::Image(e) => write!(f, "sex-image error: {e:?}"),
        }
    }
}

impl core::error::Error for Error {}

impl From<sex_image::Error> for Error {
    fn from(err: sex_image::Error) -> Self {
        match err {
            sex_image::Error::UnknownFormat => Error::UnknownFormat,
            sex_image::Error::InputTooLarge => Error::InputTooLarge,
            sex_image::Error::TooLarge => Error::TooLarge,
            sex_image::Error::OutOfMemory => Error::OutOfMemory,
            sex_image::Error::Unsupported => Error::Unsupported,
            sex_image::Error::Webp => Error::Webp,
            sex_image::Error::Gif => Error::Gif,
            other => Error::Image(other),
        }
    }
}
