// SPDX-License-Identifier: GPL-3.0-only
//! Fixed BOOT_DAG document snapshot reservations, one per client incarnation.
//! These slots name existing Region capabilities; protocol bytes grant nothing.

/// Read-only client view. Provider-local slots distinguish the exact catalogued Quil peers.
pub const CLIENT_SLOT: u64 = 64;
pub const CLIENT_COUNT: usize = crate::app_catalog::QUIL_COUNT;
pub const PROVIDER_SLOTS: [u64; CLIENT_COUNT] = {
    let mut slots = [0; CLIENT_COUNT]; let mut i = 0;
    while i < CLIENT_COUNT { slots[i] = CLIENT_SLOT + i as u64; i += 1; }
    slots
};
/// A 12 KiB document plus bounded protocol metadata, rounded to BOOT pages.
pub const BYTES: u64 = 16 * 1024;
pub const FIRST_REGION_ID: u32 = 6;

/// Preserve the existing 6..8 identities. Additional document views use a
/// separate BOOT namespace, leaving UiIR's permanent regions 9/10 untouched.
pub const fn region_id(index: usize) -> u32 {
    assert!(index < crate::app_catalog::PRESENTATION_BITS);
    if index < 3 { FIRST_REGION_ID + index as u32 } else { 0x2000 + (index - 3) as u32 }
}
