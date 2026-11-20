// SPDX-License-Identifier: GPL-3.0-only
//! When the wired card's interrupt is armed, SexNet sleeps until a frame arrives
//! instead of waking every 2 ticks. This is the bookkeeping, kept pure so the host
//! tests can drive it: whether a sleep may be long, and when to give the interrupt
//! up because it never comes (a card or platform whose MSI is not delivered, e.g.
//! behind a remapping we have not set up). Giving up returns the service to the
//! 2-tick poll it always had, so a silent interrupt costs latency, never frames.

/// Ticks (1 ms) a quiet service sleeps with the interrupt armed. Transport timers
/// (TCP retransmission, DNS, ARP) only need this granularity.
pub const QUIET_TICKS: u64 = 10;
/// Frames found after a sleep that ended without an interrupt, with no interrupt
/// ever seen, before the interrupt is declared silent.
pub const SILENT_LIMIT: u32 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetIrq {
    pub armed: bool,
    pub gave_up: bool,
    /// Interrupt bits taken while armed.
    pub wakes: u64,
    /// Received frames that followed a timed-out quiet sleep, while `wakes == 0`.
    pub missed: u32,
    /// The last quiet sleep ended by its deadline, not by the interrupt.
    timed_out: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict { Keep, Silent }

impl NetIrq {
    pub fn may_arm(&self) -> bool { !self.armed && !self.gave_up }
    pub fn armed(&mut self) { self.armed = true; self.timed_out = false; }
    /// The service's interrupt bits were taken; true when the card's causes must be
    /// acknowledged before the ring is drained.
    pub fn took(&mut self, bits: u64) -> bool {
        if !self.armed || bits == 0 { return false; }
        self.wakes = self.wakes.saturating_add(1);
        self.timed_out = false;
        true
    }
    /// A quiet sleep ended; `bits` is what the take after it returned.
    pub fn slept(&mut self, bits: u64) { if self.armed { self.timed_out = bits == 0; } }
    /// A frame came off the receive ring.
    pub fn received(&mut self) -> Verdict {
        if !self.armed || !self.timed_out { return Verdict::Keep; }
        self.timed_out = false;
        if self.wakes != 0 { return Verdict::Keep; }
        self.missed += 1;
        if self.missed >= SILENT_LIMIT { self.armed = false; self.gave_up = true; Verdict::Silent } else { Verdict::Keep }
    }
    /// The card went away or stopped: forget the arming, allow a later card to arm.
    pub fn reset(&mut self) { *self = NetIrq { gave_up: self.gave_up, ..NetIrq::default() }; }
}
