// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Text that is more than one character after another.
//!
//! - [`bidi`]: the visual order of a line holding right-to-left text (Hebrew, Arabic)
//!   by the Unicode Bidirectional Algorithm, without allocating.
//! - [`arabic`]: the joining form (isolated, initial, medial, final) of each Arabic
//!   letter, for text drawn one character a cell.
//! - `shape` (feature `shaping`): OpenType shaping with rustybuzz, for proportional
//!   text: ligatures, combining marks, Arabic and Indic scripts.
#![no_std]
#[cfg(feature = "shaping")]
extern crate alloc;

pub mod arabic;
mod arabic_forms;
pub mod bidi;
mod bidi_classes;
mod wide_table;
pub mod cells;
#[cfg(feature = "shaping")]
pub mod shape;
