// SPDX-License-Identifier: GPL-3.0-only
//! ISO Base Media File Format and Multi-Image Application Format parsing.

pub mod box_parser;
pub mod transforms;

pub use box_parser::{BoxHeader, BoxIter, ItemLocation, ItemProperties, ItemReference, MiafMeta};
pub use transforms::{apply_clap, apply_imir, apply_irot, CleanAperture};

