// SPDX-License-Identifier: GPL-3.0-only
//! Raster image editor and digital painting engine for SexOS.
//!
//! Provides layered canvas composition, straight-alpha Porter-Duff blend modes,
//! feathered selection masks, pressure-sensitive brush engine, non-destructive
//! adjustments and spatial filters, bounded history journaling, OpenRaster (.ora)
//! read/write, and Photoshop (.psd) multi-layer importing.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod adjust;
pub mod blend;
pub mod brush;
pub mod document;
pub mod error;
pub mod filters;
pub mod geom;
pub mod history;
pub mod jpeg;
pub mod layer;
pub mod ora;
pub mod png;
pub mod psd;
pub mod selection;
pub mod text;
pub mod transform;
pub mod webp;

pub use adjust::{
    auto_levels, Adjustment, AdjustmentKind, Curves, CurvesAdjustment, CurvesPreset, HueSatAdjustment, Levels,
};
pub use blend::{pack_argb, split_argb, BlendMode};
pub use brush::{bucket_fill, bucket_fill_mask, eyedropper, Brush, BrushPreset, PointerSample, StrokeEngine};
pub use document::{BackgroundMode, Document};
pub use error::BatikError;
pub use filters::{
    apply_filter_stack, box_blur, edge_detect, emboss, gaussian_blur, pixelate, sharpen, unsharp_mask,
    FilterNode, FilterSpec, FilterStack,
};
pub use geom::Rect;
pub use history::{History, HistoryAction};
pub use jpeg::encode_jpeg;
pub use layer::{AdjustmentLayer, GroupLayer, Layer, LayerKind, RasterLayer};
pub use ora::{read_ora, write_ora};
pub use png::encode_rgba_png;
pub use psd::{encode_psd, parse_psd};
pub use selection::{SelectionCombine, SelectionMask};
pub use text::TextLayer;
pub use webp::encode_webp;

/// Enforces `sex_image::Limits` on picture dimensions before allocating buffers.
pub fn admit_limits(
    limits: &sex_image::Limits,
    width: usize,
    height: usize,
) -> Result<usize, BatikError> {
    if width == 0 || height == 0 {
        return Err(BatikError::InvalidDimensions);
    }
    if width > limits.max_width || height > limits.max_height {
        return Err(BatikError::TooLarge);
    }
    let pixels = width.checked_mul(height).ok_or(BatikError::TooLarge)?;
    if pixels > limits.max_pixels {
        return Err(BatikError::TooLarge);
    }
    Ok(pixels)
}
