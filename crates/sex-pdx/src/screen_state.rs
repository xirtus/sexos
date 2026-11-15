// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Lock screen, dim and screen-off: the Shell decides, Display paints.
//!
//! `OP_SCREEN_STATE` (Shell -> Display): arg0 = mode (`NORMAL`, `DIM`, `OFF`), arg1 =
//! the lock word ([`Lock::word`]). While locked, Display paints the lock surface over
//! the whole frame, so nothing of the desktop shows; dim halves every pixel on its way
//! out and off writes black, the way night light warms it.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

pub const OP_SCREEN_STATE: u64 = 0x2F2;

pub const NORMAL: u8 = 0;
pub const DIM: u8 = 1;
pub const OFF: u8 = 2;

/// What the lock screen says under the password dots.
pub const SAY_PROMPT: u8 = 0;
pub const SAY_WRONG: u8 = 1;
pub const SAY_CHECKING: u8 = 2;
pub const SAY_NO_PASSWORD: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Lock { pub locked: bool, pub typed: u8, pub say: u8 }
impl Lock {
    pub const fn word(self) -> u64 { self.locked as u64 | (self.typed as u64) << 8 | (self.say as u64) << 16 }
    pub const fn decode(word: u64) -> Option<Lock> {
        if word >> 24 != 0 || word & 0xFE != 0 || (word >> 16) as u8 > SAY_NO_PASSWORD { return None; }
        Some(Lock { locked: word & 1 == 1, typed: (word >> 8) as u8, say: (word >> 16) as u8 })
    }
}

static MODE: AtomicU8 = AtomicU8::new(NORMAL);
static LOCK: AtomicU64 = AtomicU64::new(0);
static PUBLISHED_MODE: AtomicU8 = AtomicU8::new(NORMAL);

/// Takes a new state; None (and no change) for a bad one, else whether it changed.
pub fn set(mode: u64, lock: u64) -> Option<bool> {
    let lock_state = Lock::decode(lock)?;
    if mode > OFF as u64 { return None; }
    let changed = MODE.swap(mode as u8, Ordering::Relaxed) != mode as u8;
    Some(LOCK.swap(lock_state.word(), Ordering::Relaxed) != lock_state.word() || changed)
}
pub fn mode() -> u8 { MODE.load(Ordering::Relaxed) }
pub fn lock() -> Lock { Lock::decode(LOCK.load(Ordering::Relaxed)).unwrap_or_default() }
/// True once after the mode changes, for the publisher: the whole frame goes out again.
pub fn take_mode_change() -> bool {
    let now = mode();
    PUBLISHED_MODE.swap(now, Ordering::Relaxed) != now
}
/// One XRGB pixel as the mode shows it.
#[inline]
pub fn shade(pixel: u32, mode: u8) -> u32 {
    match mode { DIM => (pixel >> 1) & 0x007F_7F7F, OFF => 0, _ => pixel }
}
