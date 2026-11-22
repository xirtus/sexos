// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bounded waiting. Every loop in a driver that waits on a device goes through
//! here, so every wait ends: on the deadline, or after [`MAX_POLLS`] looks even
//! if the clock never moves.

use crate::error::Error;
use crate::regs::{is_gone, Regs};

/// Looks at the device one wait may take, whatever the clock says. A real
/// device answers in far fewer; a wait that needs more is a driver bug.
pub const MAX_POLLS: u32 = 1_000_000;

/// Time as a driver sees it: milliseconds since boot, and a way to give the core
/// away while waiting. On SexOS `sleep_ms` also returns early when one of the
/// service's interrupt bits is set, so an interrupt-driven wait is a sleep.
pub trait Clock {
    fn now_ms(&mut self) -> u64;
    /// Wait about `ms` milliseconds (0 = just yield the core).
    fn sleep_ms(&mut self, ms: u64);
}

impl<C: Clock + ?Sized> Clock for &mut C {
    fn now_ms(&mut self) -> u64 { (**self).now_ms() }
    fn sleep_ms(&mut self, ms: u64) { (**self).sleep_ms(ms) }
}

/// Calls `look` until it returns `Some`, the clock passes `timeout_ms`, or it
/// has been called [`MAX_POLLS`] times. Between looks the core is yielded
/// (`sleep_ms(0)`); after the first 64 looks it sleeps 1 ms between them, so a
/// long wait does not burn a core.
pub fn wait_for<C: Clock, T>(clock: &mut C, timeout_ms: u64, mut look: impl FnMut() -> Option<T>) -> Result<T, Error> {
    let start = clock.now_ms();
    for n in 0..MAX_POLLS {
        if let Some(v) = look() { return Ok(v); }
        if clock.now_ms().saturating_sub(start) >= timeout_ms { return Err(Error::Timeout); }
        clock.sleep_ms(if n < 64 { 0 } else { 1 });
    }
    Err(Error::Timeout)
}

/// Waits until `read32(offset) & mask == want`; returns the last value read.
///
/// A read of all ones ends the wait with [`Error::Gone`], even when the bits
/// waited for would be set by it: a ready bit that "sets" because the device
/// fell off the bus must not count as ready. For the rare register that
/// legitimately reads all ones, wait with [`wait_for`] and your own check.
pub fn poll_until<R: Regs, C: Clock>(regs: &mut R, clock: &mut C, offset: u32, mask: u32, want: u32, timeout_ms: u64) -> Result<u32, Error> {
    let mut gone = false;
    let r = wait_for(clock, timeout_ms, || {
        let v = regs.read32(offset);
        if is_gone(v) {
            gone = true;
            return Some(v);
        }
        (v & mask == want).then_some(v)
    });
    if gone { Err(Error::Gone) } else { r }
}
