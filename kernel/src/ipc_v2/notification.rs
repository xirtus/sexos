//! Notification: a 64-bit word that signalers OR bits into and one thread waits on.
//!
//! Signals never block and never queue: repeated signals of the same bit merge.
//! Device interrupts are bound to a notification bit through [`IrqRoutes`], so a
//! driver waits on its notification instead of polling an input ring.
//!
//! A wake may be spurious: if a signal lands while the waiter is parking, the
//! waiter returns the bits itself and the signaler's wake finds it running. The
//! scheduler treats waking a runnable task as a no-op.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::TaskId;

const NO_WAITER: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalOutcome {
    /// Bits recorded; nobody was waiting.
    Pending,
    /// Bits recorded and this waiter must be woken.
    Wake(TaskId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    /// These bits were pending and are now cleared.
    Ready(u64),
    /// Nothing pending: the task is the waiter and should block.
    Parked,
    /// Another thread already waits on this notification.
    Busy,
}

pub struct Notification {
    word: AtomicU64,
    waiter: AtomicU32,
}

impl Default for Notification {
    fn default() -> Self { Self::new() }
}

impl Notification {
    pub const fn new() -> Self { Self { word: AtomicU64::new(0), waiter: AtomicU32::new(NO_WAITER) } }

    pub fn signal(&self, bits: u64) -> SignalOutcome {
        if bits == 0 {
            return SignalOutcome::Pending;
        }
        self.word.fetch_or(bits, Ordering::AcqRel);
        match self.waiter.swap(NO_WAITER, Ordering::AcqRel) {
            NO_WAITER => SignalOutcome::Pending,
            w => SignalOutcome::Wake(w - 1),
        }
    }

    /// Takes the pending bits, or parks `task` as the waiter when there are none.
    pub fn wait(&self, task: TaskId) -> WaitOutcome {
        debug_assert!(task < u32::MAX);
        let bits = self.word.swap(0, Ordering::AcqRel);
        if bits != 0 {
            return WaitOutcome::Ready(bits);
        }
        match self.waiter.compare_exchange(NO_WAITER, task + 1, Ordering::AcqRel, Ordering::Acquire) {
            // Already the waiter (woken spuriously, waiting again): stay parked.
            Ok(_) => {}
            Err(w) if w == task + 1 => {}
            Err(_) => return WaitOutcome::Busy,
        }
        // A signal between the swap and the park saw no waiter: take its bits now.
        let late = self.word.swap(0, Ordering::AcqRel);
        if late == 0 {
            return WaitOutcome::Parked;
        }
        // Either we unpark ourselves, or the signaler already took us and its wake
        // will be spurious. The bits are ours in both cases.
        let _ = self.waiter.compare_exchange(task + 1, NO_WAITER, Ordering::AcqRel, Ordering::Acquire);
        WaitOutcome::Ready(late)
    }

    /// Takes the pending bits without waiting (zero when none).
    pub fn poll(&self) -> u64 { self.word.swap(0, Ordering::AcqRel) }

    /// The waiter stops waiting. False means a signal already claimed it.
    pub fn cancel_wait(&self, task: TaskId) -> bool {
        self.waiter.compare_exchange(task + 1, NO_WAITER, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub fn waiter(&self) -> Option<TaskId> {
        match self.waiter.load(Ordering::Acquire) {
            NO_WAITER => None,
            w => Some(w - 1),
        }
    }

    pub fn pending(&self) -> u64 { self.word.load(Ordering::Acquire) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    BadLine,
    BadBit,
    /// The line already signals some notification.
    Bound,
}

/// Device interrupt line -> (notification id, bit). One word per line, so the
/// interrupt handler finds its target with one load.
pub struct IrqRoutes<const LINES: usize> {
    routes: [AtomicU64; LINES],
}

impl<const LINES: usize> Default for IrqRoutes<LINES> {
    fn default() -> Self { Self::new() }
}

impl<const LINES: usize> IrqRoutes<LINES> {
    #[allow(clippy::declare_interior_mutable_const)]
    const EMPTY: AtomicU64 = AtomicU64::new(0);

    pub const fn new() -> Self { Self { routes: [Self::EMPTY; LINES] } }

    const fn pack(notification: u32, bit: u32) -> u64 { ((notification as u64 + 1) << 8) | bit as u64 }

    pub fn bind(&self, line: usize, notification: u32, bit: u32) -> Result<(), BindError> {
        let slot = self.routes.get(line).ok_or(BindError::BadLine)?;
        if bit >= 64 || notification == u32::MAX {
            return Err(BindError::BadBit);
        }
        slot.compare_exchange(0, Self::pack(notification, bit), Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| BindError::Bound)
    }

    /// Unbinds `line` only if it still routes to `notification`.
    pub fn unbind(&self, line: usize, notification: u32) -> bool {
        let Some(slot) = self.routes.get(line) else { return false };
        let current = slot.load(Ordering::Acquire);
        current != 0
            && (current >> 8) == notification as u64 + 1
            && slot.compare_exchange(current, 0, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// The notification is going away: drops every line bound to it.
    pub fn unbind_all(&self, notification: u32) -> usize {
        (0..LINES).filter(|&line| self.unbind(line, notification)).count()
    }

    /// Where an interrupt on `line` goes: (notification id, bit mask to signal).
    pub fn route(&self, line: usize) -> Option<(u32, u64)> {
        match self.routes.get(line)?.load(Ordering::Acquire) {
            0 => None,
            w => Some((((w >> 8) - 1) as u32, 1u64 << (w & 0x3F))),
        }
    }
}
