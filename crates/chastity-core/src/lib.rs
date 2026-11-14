// SPDX-License-Identifier: GPL-3.0-only
//! Chastity, the SexOS password vault: everything but the window.
//!
//! * [`vault`]: KDBX4 vaults (keepass-rs), interchangeable with KeePassXC: create, open,
//!   save, entries (title, user name, password, URL, notes, TOTP), search.
//! * [`keeper`]: the one place the unlocked key material lives, zeroized on lock; the
//!   seam where Shibari, the planned RAM-only secret keeper, takes over.
//! * [`totp`]: RFC 6238 codes from `otpauth://` URIs.
//! * [`generator`]: random passwords from the system's entropy.
//! * [`autofill`]: the boundary a browser (Kaleidoscope) will ask through; not wired yet.
//!
//! Builds for the host (`cargo test -p chastity-core`) and for SexOS programs
//! (`x86_64-sex-std.json`, with `apps/chastity`).
pub mod autofill;
pub mod generator;
pub mod keeper;
pub mod totp;
pub mod vault;

pub use vault::{Kdf, NewEntry, OpenError, Summary, Vault};

/// FNV-1a, for records that must name a value without showing it (titles in the
/// serial record, as the gate checks them).
pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}
