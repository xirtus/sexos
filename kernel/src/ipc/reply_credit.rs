// SPDX-License-Identifier: GPL-3.0-only
//! Kernel-recorded optional reply rights. Every accepted typed invocation
//! grants its callee exactly one reply to that caller. A domain ID alone
//! grants nothing. Counts are indexed by the immutable domain registry;
//! no caller is evicted and no accepted obligation is silently discarded.
//! Admission must fail before enqueue if a checked counter cannot advance.

/// Shared with the domain registry. Index zero is never a user caller.
pub const REPLY_CREDIT_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    /// One obligation reserved; withdraw it if enqueue fails.
    Added,
    /// Invalid caller or exhausted counter; unchanged, do not enqueue.
    Full,
}

pub struct ReplyCredits {
    counts: [u64; REPLY_CREDIT_CAPACITY],
}

impl ReplyCredits {
    pub const fn new() -> Self {
        Self { counts: [0; REPLY_CREDIT_CAPACITY] }
    }

    /// Reserve one reply before publishing an invocation. Registry membership
    /// and the BOOT-held call edge are checked by the caller of this helper.
    pub fn grant(&mut self, caller: u32) -> Grant {
        if caller == 0 { return Grant::Full; }
        let Some(count) = self.counts.get_mut(caller as usize) else { return Grant::Full; };
        let Some(next) = count.checked_add(1) else { return Grant::Full; };
        *count = next;
        Grant::Added
    }

    /// Undo exactly the reservation made for a failed enqueue.
    pub fn withdraw(&mut self, caller: u32) {
        let _ = self.consume(caller);
    }

    pub fn holds(&self, target: u32) -> bool {
        target != 0 && self.counts.get(target as usize).is_some_and(|count| *count != 0)
    }

    /// Spend one obligation only after publishing its reply.
    pub fn consume(&mut self, target: u32) -> bool {
        if target == 0 { return false; }
        let Some(count) = self.counts.get_mut(target as usize) else { return false; };
        if *count == 0 { return false; }
        *count -= 1;
        true
    }

    /// A retired callee's reply rights cannot outlive its incarnation.
    pub fn clear(&mut self) {
        self.counts = [0; REPLY_CREDIT_CAPACITY];
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
        credits.counts[3] = u64::MAX - 1;
        credits.counts[9] = u64::MAX;
        assert_eq!(credits.grant(3), Grant::Added);
        assert_eq!(credits.outstanding(), 2 * u64::MAX as u128);
        assert_eq!(credits.grant(3), Grant::Full);
        assert_eq!(credits.counts[3], u64::MAX);
        assert_eq!(credits.counts[9], u64::MAX);
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
}
