// SPDX-License-Identifier: GPL-3.0-only
//! Who may pair, and the user's answer to "does this number match?".
//!
//! TODO(W2b, desktop prompt): there is no desktop pairing prompt yet. The answer should
//! come from the Shell (a `sex_pdx` prompt protocol: Cufflinks asks with the device name
//! and the six digits, the Shell shows Pair / Cancel and answers), which needs a Shell
//! module and two BOOT routes. Until then pairing is bounded instead of asked:
//!
//! - bonded devices always come back (their keys prove they paired before);
//! - a new keyboard or mouse is connected, and its pairing accepted, only inside a
//!   pairing window: the first `WINDOW_TICKS` after a controller is ready, and only
//!   when no device is bonded yet (first setup: put the keyboard in pairing mode, boot);
//! - outside the window a new device's numeric comparison is refused.
//!
//! Every decision is logged (`[cufflinks.pair] ... accepted=auto window=1` or
//! `accepted=0 reason=no_prompt`), so the x17 receipt shows what happened.

/// About two minutes at the kernel's 1 ms tick.
pub const WINDOW_TICKS: u64 = 120_000;

pub struct Policy { until: u64 }

impl Policy {
    /// A controller is ready: open the window if nothing is bonded.
    pub fn new(now: u64, bonded: usize) -> Policy { Policy { until: if bonded == 0 { now + WINDOW_TICKS } else { 0 } } }
    pub fn open(&self, now: u64) -> bool { now < self.until }
    pub fn window_seconds(&self, now: u64) -> u64 { self.until.saturating_sub(now) / 1000 }
    /// May a new (unbonded) device that looks like a keyboard or mouse be connected?
    pub fn connect_new(&self, now: u64) -> bool { self.open(now) }
    /// The answer to a numeric comparison.
    pub fn accept_compare(&self, now: u64) -> bool { self.open(now) }
}
