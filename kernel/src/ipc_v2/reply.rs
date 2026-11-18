//! One-shot reply capabilities and the incarnation generations that cancel them.
//!
//! A call hands the server a reply capability naming the caller and the caller's
//! generation at call time. It lives in the server thread's [`ReplySlot`] (one per
//! thread, no table, no lock) and is consumed by the reply. When a task dies its
//! generation moves on, so every reply capability still naming it stops working:
//! the server's reply is refused instead of reaching whatever runs in that slot
//! next. Servers with deferred work check [`ReplySlot::is_live`] before doing it,
//! which reads the generation word and makes no syscall.

use core::sync::atomic::{AtomicU64, Ordering};

use super::TaskId;
use super::endpoint::{CallKind, Sender};

/// Where a caller's live incarnation generation is read from. The kernel's
/// generation registry (authority v2, capability_table.rs) implements it; these
/// objects keep no generation table of their own.
pub trait Incarnations {
    fn is_current(&self, task: TaskId, generation: u32) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyCap {
    pub caller: TaskId,
    pub generation: u32,
}

impl ReplyCap {
    /// The reply capability a call creates; one-way sends create none.
    pub fn for_sender(sender: &Sender) -> Option<Self> {
        match sender.kind {
            CallKind::Call => Some(Self { caller: sender.task, generation: sender.generation }),
            CallKind::Send => None,
        }
    }

    const fn pack(self) -> u64 { ((self.caller as u64 + 1) << 32) | self.generation as u64 }

    const fn unpack(word: u64) -> Option<Self> {
        match word {
            0 => None,
            w => Some(Self { caller: ((w >> 32) - 1) as u32, generation: w as u32 }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyError {
    /// No reply capability held: nothing to answer.
    Empty,
    /// The thread still holds an unanswered call.
    Occupied,
    /// The caller died after calling; the capability was consumed and dropped.
    CallerGone,
}

/// A server thread's single reply capability.
pub struct ReplySlot {
    word: AtomicU64,
}

impl Default for ReplySlot {
    fn default() -> Self { Self::new() }
}

impl ReplySlot {
    pub const fn new() -> Self { Self { word: AtomicU64::new(0) } }

    /// Holds the capability from a call just received.
    pub fn install(&self, cap: ReplyCap) -> Result<(), ReplyError> {
        debug_assert!(cap.caller < u32::MAX);
        self.word
            .compare_exchange(0, cap.pack(), Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| ReplyError::Occupied)
    }

    /// Consumes the capability to reply. Either way the slot is empty afterwards,
    /// so a capability is used at most once.
    pub fn take(&self, generations: &impl Incarnations) -> Result<ReplyCap, ReplyError> {
        let cap = ReplyCap::unpack(self.word.swap(0, Ordering::AcqRel)).ok_or(ReplyError::Empty)?;
        if generations.is_current(cap.caller, cap.generation) {
            Ok(cap)
        } else {
            Err(ReplyError::CallerGone)
        }
    }

    /// Whether the held capability can still be answered (for deferred work).
    pub fn is_live(&self, generations: &impl Incarnations) -> bool {
        ReplyCap::unpack(self.word.load(Ordering::Acquire))
            .is_some_and(|cap| generations.is_current(cap.caller, cap.generation))
    }

    pub fn peek(&self) -> Option<ReplyCap> { ReplyCap::unpack(self.word.load(Ordering::Acquire)) }

    /// The server thread died holding a call: returns the caller to wake with an
    /// error, if it is still the same incarnation.
    pub fn discard(&self, generations: &impl Incarnations) -> Option<ReplyCap> {
        self.take(generations).ok()
    }
}
