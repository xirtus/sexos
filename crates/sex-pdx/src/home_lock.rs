// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The encrypted home: Files requests (same wire as [`crate::fs`]) that set it up, unlock
//! and lock it, and the status a request under `/home` gets while it is locked.
//!
//! The password goes in the request's data area, `length` bytes, never in the path. Only
//! a run holding the whole filesystem (`@/`) may send these.

/// `/home` itself: a plain folder while locked, the encrypted volume's root once unlocked.
pub const HOME: &[u8] = b"/home";
/// The encrypted volume, a file of the SexOS filesystem.
pub const IMAGE: &[u8] = b"/.home.sexcrypt";

/// Data: the password. `offset`: the volume's size in MiB. Fails EXISTS if there is one.
pub const SETUP: u8 = 24;
/// Data: the password. Answers OK, or WRONG_PASSWORD, or NOT_FOUND with no volume.
pub const UNLOCK: u8 = 25;
/// Writes out and forgets the key; every grant into `/home` ends.
pub const LOCK: u8 = 26;
/// The reply's value: one of the states below.
pub const STATE: u8 = 27;

pub const STATE_NONE: u32 = 0;
pub const STATE_LOCKED: u32 = 1;
pub const STATE_UNLOCKED: u32 = 2;

/// A path under `/home` while it is locked.
pub const LOCKED: u8 = 17;
pub const WRONG_PASSWORD: u8 = 18;
/// Shortest password SETUP takes.
pub const PASSWORD_MIN: usize = 8;

pub fn message(status: u8) -> &'static [u8] {
    match status {
        LOCKED => b"home is locked",
        WRONG_PASSWORD => b"wrong password",
        s => crate::fs::message(s),
    }
}
