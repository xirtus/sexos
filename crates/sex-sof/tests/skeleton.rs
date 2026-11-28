// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The crate builds as a host library; each stage adds its own test file.
#[test]
fn crate_links() { let _ = core::mem::size_of::<u8>(); }
