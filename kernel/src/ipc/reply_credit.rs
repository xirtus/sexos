// SPDX-License-Identifier: GPL-3.0-only
//! Kernel-recorded reply obligations. An accepted typed invocation from a
//! caller grants its callee exactly one reply to that caller; nothing else
//! does. A domain ID alone is therefore not a return capability. The table is
//! finite and owned by the callee's protection domain: a full table refuses
//! the invocation before enqueue instead of evicting another obligation.

pub const REPLY_CREDIT_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Credit {
    caller: u32,
    count: u16,
}

pub struct ReplyCredits {
    entries: [Credit; REPLY_CREDIT_CAPACITY],
}

impl ReplyCredits {
    pub const fn new() -> Self {
        Self { entries: [Credit { caller: 0, count: 0 }; REPLY_CREDIT_CAPACITY] }
    }

    /// Reserve one reply for `caller` before its invocation is published.
    /// False means no effect: the caller's invocation must not be enqueued.
    pub fn grant(&mut self, caller: u32) -> bool {
        if let Some(credit) = self.entries.iter_mut().find(|c| c.count != 0 && c.caller == caller) {
            if credit.count == u16::MAX { return false; }
            credit.count += 1;
            return true;
        }
        match self.entries.iter_mut().find(|c| c.count == 0) {
            Some(free) => { *free = Credit { caller, count: 1 }; true }
            None => false,
        }
    }

    /// Undo a reservation whose invocation was not accepted.
    pub fn withdraw(&mut self, caller: u32) {
        let _ = self.consume(caller);
    }

    pub fn holds(&self, target: u32) -> bool {
        self.entries.iter().any(|c| c.count != 0 && c.caller == target)
    }

    /// Spend one obligation after its reply was actually published.
    pub fn consume(&mut self, target: u32) -> bool {
        let Some(credit) = self.entries.iter_mut().find(|c| c.count != 0 && c.caller == target) else {
            return false;
        };
        credit.count -= 1;
        if credit.count == 0 { credit.caller = 0; }
        true
    }

    /// A retired callee owes nothing; its obligations cannot outlive it.
    pub fn clear(&mut self) {
        self.entries = [Credit { caller: 0, count: 0 }; REPLY_CREDIT_CAPACITY];
    }

    pub fn outstanding(&self) -> u32 {
        self.entries.iter().map(|c| c.count as u32).sum()
    }
}
