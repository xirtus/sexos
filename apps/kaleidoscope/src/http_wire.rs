// SPDX-License-Identifier: GPL-3.0-only
//! Source-path facade: runtime and standalone host harnesses compile the same
//! allocation-free codec as sex-http. Parsing grants no endpoint authority.
#[path="../../../crates/sex-http/src/codec.rs"]
mod codec;
pub use self::codec::*;
