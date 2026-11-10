// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Fixed BOOT_DAG text regions, one per app incarnation. The app writes a whole
//! text once and Display copies it, instead of 8 bytes per call. These slots name
//! existing Region capabilities: nothing is minted, and the transaction hash is
//! still checked on Display's own copy, so a later write by the app changes nothing.
use crate::app_catalog::{COUNT, INCARNATIONS};

pub const BYTES: u64 = 8 * 1024;
const _: () = assert!(crate::text_batch::CAPACITY as u64 <= BYTES);
/// The app's view, writable by that app only.
pub const CLIENT_SLOT: u64 = 99;
/// Display's read-only views, one slot per catalog row.
pub const DISPLAY_FIRST_SLOT: u64 = 200;
pub const fn display_slot(index: usize) -> u64 { DISPLAY_FIRST_SLOT + index as u64 }
pub const fn region_id(index: usize) -> u32 { assert!(index < COUNT); 0x3000 + index as u32 }
/// Descriptive row lookup. The kernel-held capability is the authority.
pub fn index_of(target: u32) -> Option<usize> { INCARNATIONS.iter().position(|row| row.target == target) }
