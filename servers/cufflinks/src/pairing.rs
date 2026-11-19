// SPDX-License-Identifier: GPL-3.0-only
//! Who may pair, and how the user's answer to "does this number match?" is got.
//!
//! W2b: the Shell's Bluetooth panel (`sex_pdx::bt_ui`) is the prompt. While a Shell is
//! attached to the panel region and Cufflinks's wake reaches it ("prompt available"):
//!
//! - a new device is connected only when the user picks Pair in the panel; scan
//!   results are listed, never connected on their own;
//! - every numeric comparison is asked in the panel ("Does it show 123456?"), whether
//!   the user started the pairing or a device asked us; only Yes accepts it, and no
//!   answer within `ANSWER_TICKS` refuses it;
//! - bonded devices always come back (their keys prove they paired before).
//!
//! Without a prompt (no Shell attached, or the wake did not reach it) the bounded
//! first-setup rule from W2 stays as the fallback: a new keyboard, mouse or headset is
//! connected, and its comparison accepted, only inside a window of `WINDOW_TICKS`
//! after a controller is ready, and only while nothing is bonded. Once anything is
//! bonded, nothing new pairs without the user: strangers are never auto-accepted.
//!
//! Every decision is logged by the caller (`[cufflinks.pair] ... accepted=user`,
//! `asked=1`, `accepted=auto window=1` or `accepted=0 reason=...`).
//! This file has no dependencies, so `tests/cufflinks_pairing.rs` tests it as is.

/// About two minutes at the kernel's 1 ms tick.
pub const WINDOW_TICKS: u64 = 120_000;
/// How long a comparison waits for the user.
pub const ANSWER_TICKS: u64 = 30_000;
/// How long a Pair from the panel may take before it is called failed.
pub const CONNECT_TICKS: u64 = 40_000;

/// What to do with a numeric comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer { Ask, Accept, Refuse }

pub struct Policy { until: u64 }

impl Policy {
    /// A controller is ready: open the fallback window if nothing is bonded.
    pub fn new(now: u64, bonded: usize) -> Policy { Policy { until: if bonded == 0 { now + WINDOW_TICKS } else { 0 } } }
    pub fn window_open(&self, now: u64) -> bool { now < self.until }
    pub fn window_seconds(&self, now: u64) -> u64 { self.until.saturating_sub(now) / 1000 }
    /// May a new (unbonded) device found by scanning be connected on our own?
    /// With a prompt, never: the user picks it in the panel.
    pub fn connect_new(&self, now: u64, prompt: bool) -> bool { !prompt && self.window_open(now) }
    /// A numeric comparison arrived. With a prompt the user decides; without one the
    /// first-setup window does.
    pub fn compare(&self, now: u64, prompt: bool) -> Answer {
        if prompt { Answer::Ask } else if self.window_open(now) { Answer::Accept } else { Answer::Refuse }
    }
}

/// The one comparison shown to the user, waiting for an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending { pub handle: u16, pub addr: [u8; 6], pub value: u32, pub ticket: u32, pub since: u64 }

impl Pending {
    /// The user's answer, if it is for this comparison (a stale ticket is ignored).
    pub fn answers(&self, ticket: u32) -> bool { ticket == self.ticket }
    pub fn expired(&self, now: u64) -> bool { now.saturating_sub(self.since) >= ANSWER_TICKS }
}

/// Devices the user asked to pair from the panel, until they pair, fail or time out.
pub struct Requests { list: [Option<([u8; 6], u64)>; 4] }

impl Default for Requests { fn default() -> Self { Self::new() } }

impl Requests {
    pub const fn new() -> Requests { Requests { list: [None; 4] } }
    /// Records a request (the oldest gives way when four are waiting).
    pub fn add(&mut self, addr: [u8; 6], now: u64) {
        self.done(&addr);
        let at = self.list.iter().position(|r| r.is_none())
            .unwrap_or_else(|| (0..4).min_by_key(|&i| self.list[i].map_or(0, |r| r.1)).unwrap_or(0));
        self.list[at] = Some((addr, now));
    }
    pub fn waiting(&self, addr: &[u8; 6]) -> bool { self.list.iter().flatten().any(|r| r.0 == *addr) }
    /// It paired, failed or was forgotten. True if it was waiting.
    pub fn done(&mut self, addr: &[u8; 6]) -> bool {
        let mut was = false;
        for r in self.list.iter_mut() { if r.is_some_and(|r| r.0 == *addr) { *r = None; was = true; } }
        was
    }
    /// One request that took longer than `CONNECT_TICKS`, removed.
    pub fn expired(&mut self, now: u64) -> Option<[u8; 6]> {
        let r = self.list.iter_mut().find(|r| r.is_some_and(|r| now.saturating_sub(r.1) >= CONNECT_TICKS))?;
        r.take().map(|r| r.0)
    }
}
