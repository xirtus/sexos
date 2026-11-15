// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Network time: sexnet asks a time server (SNTP, `sex_ntp`) once the machine is online
//! and every hour after, and sets civil time (`clock_source::civil::SET`; sexnet is the
//! one time keeper). The CMOS clock is never written.
//!
//! rdi = `OP`, rsi = action. Shell turns it on or off (`time.network_time`); anyone may
//! ask how it went (it says nothing about who or where). One word back.

pub const OP: u64 = 0x2E9;
/// Shell only: rdx = 1 on (the default), 0 off. Replies `OK`.
pub const SET_ENABLED: u64 = 1;
/// Shell only: ask again now (the person pressed "Sync now"). Replies `OK`.
pub const SYNC_NOW: u64 = 2;
/// Replies `status_word`.
pub const STATUS: u64 = 3;

pub const OK: u64 = 0;
pub const DENIED: u64 = 1 << 63;
/// Where the servers are asked: `pool.ntp.org`, a name every network's resolver knows.
pub const SERVER: &[u8] = b"pool.ntp.org";
/// How often a synced clock is checked again (ms).
pub const RESYNC_MS: u64 = 3_600_000;

/// Status: what network time is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Off = 0, Waiting = 1, Asking = 2, Synced = 3, Failed = 4 }
/// The status word: state, the last step taken (ms, signed, saturating at ±2^31) and
/// minutes since the last sync (0xFFFF none).
pub fn status_word(state: State, step_ms: i64, minutes: u64) -> u64 {
    (state as u64) << 56 | (minutes.min(0xFFFF)) << 32 | (step_ms.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32 as u64)
}
pub fn decode_status(word: u64) -> Option<(State, i64, Option<u64>)> {
    if word & DENIED != 0 { return None; }
    let state = match word >> 56 { 0 => State::Off, 1 => State::Waiting, 2 => State::Asking, 3 => State::Synced, 4 => State::Failed, _ => return None };
    let minutes = (word >> 32) & 0xFFFF;
    Some((state, word as u32 as i32 as i64, (minutes != 0xFFFF).then_some(minutes)))
}
