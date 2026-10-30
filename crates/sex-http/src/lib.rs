// SPDX-License-Identifier: GPL-3.0-only
//! Bounded pure HTTP framing under immutable BOOT hostname/port data. No route,
//! capability, TLS, allocator or publication authority is provided by parsing.
//! Fixed resources select their own request path and expected MIME; legacy
//! entry points retain the text/plain profile and exact original wire bytes.
#![no_std]
#[cfg(test)]
extern crate std;
mod codec;
pub use codec::*;
