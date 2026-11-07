// SPDX-License-Identifier: GPL-3.0-only
//! Kernel-recorded optional reply rights. Every accepted typed invocation
//! grants its callee exactly one reply to that caller. A domain ID alone
//! grants nothing. Counts are indexed by the immutable domain registry;
//! no caller is evicted and no accepted obligation is silently discarded.
//! Admission must fail before enqueue if a checked counter cannot advance.

/// Domain identities the registry can hold. Index zero is never a user caller.
pub const REPLY_CREDIT_CAPACITY: usize = 8192;
/// Distinct callers one domain can owe at once. A table per domain, not a counter
/// per possible caller, so the cost does not grow with the number of domains.
pub const REPLY_CALLERS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    /// One obligation reserved; withdraw it if enqueue fails.
    Added,
    /// Invalid caller or exhausted counter; unchanged, do not enqueue.
    Full,
}

pub struct ReplyCredits {
    callers: [u32; REPLY_CALLERS],
    counts: [u64; REPLY_CALLERS],
}

impl ReplyCredits {
    pub const fn new() -> Self {
        Self { callers: [0; REPLY_CALLERS], counts: [0; REPLY_CALLERS] }
    }

    /// The caller's entry, or the free entry where it would go. Entries stay until
    /// `clear`, so a probe never passes a caller that is present.
    fn slot(&self, caller: u32) -> Option<usize> {
        let start = caller as usize % REPLY_CALLERS;
        (0..REPLY_CALLERS).map(|i| (start + i) % REPLY_CALLERS)
            .find(|&i| self.callers[i] == caller || self.callers[i] == 0)
    }

    /// Reserve one reply before publishing an invocation. Registry membership
    /// and the BOOT-held call edge are checked by the caller of this helper.
    pub fn grant(&mut self, caller: u32) -> Grant {
        if caller == 0 || caller as usize >= REPLY_CREDIT_CAPACITY { return Grant::Full; }
        let Some(slot) = self.slot(caller) else { return Grant::Full; };
        let Some(next) = self.counts[slot].checked_add(1) else { return Grant::Full; };
        self.callers[slot] = caller;
        self.counts[slot] = next;
        Grant::Added
    }

    /// Undo exactly the reservation made for a failed enqueue.
    pub fn withdraw(&mut self, caller: u32) {
        let _ = self.consume(caller);
    }

    pub fn holds(&self, target: u32) -> bool {
        target != 0 && self.slot(target).is_some_and(|slot| self.callers[slot] == target && self.counts[slot] != 0)
    }

    /// Spend one obligation only after publishing its reply.
    pub fn consume(&mut self, target: u32) -> bool {
        if target == 0 { return false; }
        let Some(slot) = self.slot(target) else { return false; };
        if self.callers[slot] != target || self.counts[slot] == 0 { return false; }
        self.counts[slot] -= 1;
        true
    }

    /// A retired callee's reply rights cannot outlive its incarnation.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Exact diagnostic sum, including when multiple u64 counters are full.
    pub fn outstanding(&self) -> u128 {
        self.counts.iter().map(|count| *count as u128).sum()
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn checked_limit_refuses_without_loss_and_can_consume_then_regrant() {
        let mut credits = ReplyCredits::new();
        for (caller, count) in [(3u32, u64::MAX - 1), (9, u64::MAX)] {
            assert_eq!(credits.grant(caller), Grant::Added);
            let slot = credits.slot(caller).unwrap();
            credits.counts[slot] = count;
        }
        assert_eq!(credits.grant(3), Grant::Added);
        assert_eq!(credits.outstanding(), 2 * u64::MAX as u128);
        assert_eq!(credits.grant(3), Grant::Full);
        assert_eq!(credits.counts[credits.slot(3).unwrap()], u64::MAX);
        assert_eq!(credits.counts[credits.slot(9).unwrap()], u64::MAX);
        assert_eq!(credits.outstanding(), 2 * u64::MAX as u128);
        assert!(credits.consume(3));
        assert_eq!(credits.grant(3), Grant::Added);
        assert_eq!(credits.grant(3), Grant::Full);
        assert_eq!(credits.outstanding(), 2 * u64::MAX as u128);
        credits.clear();
        assert_eq!(credits.outstanding(), 0);
        assert!(!credits.holds(3));
        assert!(!credits.holds(9));
    }

    #[test]
    fn a_full_table_refuses_new_callers_and_keeps_old_ones() {
        let mut credits = ReplyCredits::new();
        for caller in 1..=REPLY_CALLERS as u32 { assert_eq!(credits.grant(caller * 7), Grant::Added); }
        assert_eq!(credits.grant(9000), Grant::Full);
        assert_eq!(credits.grant(8192), Grant::Full);
        assert!(!credits.holds(9000));
        for caller in 1..=REPLY_CALLERS as u32 {
            assert!(credits.holds(caller * 7));
            assert!(credits.consume(caller * 7));
            assert!(!credits.consume(caller * 7));
        }
        assert_eq!(credits.grant(14), Grant::Added);
    }
}
