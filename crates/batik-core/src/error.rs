// SPDX-License-Identifier: GPL-3.0-only
//! Error types for Batik image editor engine.

use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatikError {
    /// Underlying image decode error from sex-image.
    Image(sex_image::Error),
    /// Underlying archive error from sex-archive.
    Archive(sex_archive::Error),
    /// Image or layer exceeded memory or dimension limits.
    TooLarge,
    /// Memory allocation failed.
    OutOfMemory,
    /// Dimensions are invalid (e.g. 0 width or height).
    InvalidDimensions,
    /// Unsupported format or feature.
    Unsupported(&'static str),
    /// Malformed or corrupt PSD file.
    CorruptPsd(&'static str),
    /// Malformed or corrupt OpenRaster file.
    CorruptOra(&'static str),
    /// Layer is locked (pixels or alpha).
    LayerLocked,
    /// Specified layer was not found.
    LayerNotFound(u64),
    /// Invalid parameter supplied.
    InvalidParameter(&'static str),
    /// Compression or decompression error.
    CompressionError,
}

impl From<sex_image::Error> for BatikError {
    fn from(err: sex_image::Error) -> Self {
        match err {
            sex_image::Error::OutOfMemory => BatikError::OutOfMemory,
            sex_image::Error::TooLarge | sex_image::Error::InputTooLarge => BatikError::TooLarge,
            _ => BatikError::Image(err),
        }
    }
}

impl From<sex_archive::Error> for BatikError {
    fn from(err: sex_archive::Error) -> Self {
        match err {
            sex_archive::Error::TooLarge => BatikError::TooLarge,
            _ => BatikError::Archive(err),
        }
    }
}

impl fmt::Display for BatikError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BatikError::Image(e) => write!(f, "image error: {e:?}"),
            BatikError::Archive(e) => write!(f, "archive error: {e}"),
            BatikError::TooLarge => write!(f, "dimensions or file size exceed limits"),
            BatikError::OutOfMemory => write!(f, "out of memory"),
            BatikError::InvalidDimensions => write!(f, "invalid dimensions"),
            BatikError::Unsupported(msg) => write!(f, "unsupported: {msg}"),
            BatikError::CorruptPsd(msg) => write!(f, "corrupt PSD: {msg}"),
            BatikError::CorruptOra(msg) => write!(f, "corrupt ORA: {msg}"),
            BatikError::LayerLocked => write!(f, "layer is locked"),
            BatikError::LayerNotFound(id) => write!(f, "layer id {id} not found"),
            BatikError::InvalidParameter(msg) => write!(f, "invalid parameter: {msg}"),
            BatikError::CompressionError => write!(f, "compression or decompression failure"),
        }
    }
}
