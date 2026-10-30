// SPDX-License-Identifier: GPL-3.0-only
//! Fixed BOOT_DAG document snapshot reservations, one per client incarnation.
//! These slots name existing Region capabilities; protocol bytes grant nothing.

/// Read-only client view. Provider-local slots distinguish three exact peers.
pub const CLIENT_SLOT: u64 = 64;
pub const PROVIDER_SLOTS: [u64; 3] = [64, 65, 66];
/// A 12 KiB document plus bounded protocol metadata, rounded to BOOT pages.
pub const BYTES: u64 = 16 * 1024;
pub const FIRST_REGION_ID: u32 = 6;
