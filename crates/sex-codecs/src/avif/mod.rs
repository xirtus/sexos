// SPDX-License-Identifier: GPL-3.0-only
//! AVIF still picture and MIAF sequence decoder.

pub mod decode;
pub mod obu;

pub use decode::{decode_avif, decode_avif_into, probe_avif};
