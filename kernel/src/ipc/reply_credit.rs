// SPDX-License-Identifier: GPL-3.0-only
//! Kernel-recorded reply obligations. An accepted typed invocation from a
//! caller grants its callee exactly one reply to that caller; nothing else
//! does. A domain ID alone is therefore not a return capability. The table is
//! finite and owned by the callee's protection domain. Reply bookkeeping never
//! refuses an invocation: most invocations are one-way and are never answered,
//! so a caller's count saturates instead of failing, and a full table simply
//! records no obligation for a new caller (that caller cannot be replied to).

pub const REPLY_CREDIT_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Credit {
    caller: u32,
    count: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    /// One more obligation recorded; withdraw it if the invocation fails.
    Added,
    /// Already at the bound: replies stay authorized, nothing to withdraw.
    Saturated,
    /// No slot for a new caller: no reply authority is created.
    Untracked,
}

pub struct ReplyCredits {
    entries: [Credit; REPLY_CREDIT_CAPACITY],
}

impl ReplyCredits {
    pub const fn new() -> Self {
        Self { entries: [Credit { caller: 0, count: 0 }; REPLY_CREDIT_CAPACITY] }
    }

    /// Record one reply obligation for `caller` before its invocation is
    /// published. Never a reason to refuse the invocation.
    pub fn grant(&mut self, caller: u32) -> Grant {
        if let Some(credit) = self.entries.iter_mut().find(|c| c.count != 0 && c.caller == caller) {
            if credit.count == u16::MAX { return Grant::Saturated; }
            credit.count += 1;
            return Grant::Added;
        }
        match self.entries.iter_mut().find(|c| c.count == 0) {
            Some(free) => { *free = Credit { caller, count: 1 }; Grant::Added }
            None => Grant::Untracked,
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
