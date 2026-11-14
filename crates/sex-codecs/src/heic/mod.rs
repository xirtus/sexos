// SPDX-License-Identifier: GPL-3.0-only
//! HEIC / HEVC still picture decoder.

pub mod decode;
pub mod nal;

pub use decode::{decode_heic, decode_heic_into, probe_heic};
