// SPDX-License-Identifier: GPL-3.0-only
//! Shuttle: bidirectional text, complex OpenType shaping, font fallback, and shared font picker for SexOS.
//!
//! Named following SexOS fabric conventions: a weaver's shuttle carries the thread back and forth across
//! left-to-right and right-to-left lines.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod bidi;
pub mod catalog;
pub mod emoji;
pub mod itemize;
pub mod layout;
pub mod picker;
pub mod shaper;

pub use bidi::{BaseDirection, Direction, Paragraph, VisualRun};
pub use catalog::{script_mask, FontCatalog, FontEntry, FontFilter};
pub use emoji::{is_emoji_char, rasterize_emoji, EmojiRaster};
pub use itemize::{itemize_paragraph, ItemRun};
pub use layout::{
    layout_paragraph, Alignment, CursorRect, HitResult, LayoutOptions, ParagraphLayout, VisualGlyph,
    VisualLine,
};
pub use picker::{FontPicker, FontSelection, PickerKey, PRESET_SIZES};
pub use shaper::{Shaper, ShapedGlyph};
