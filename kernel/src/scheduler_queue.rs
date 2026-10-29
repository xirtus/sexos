//! Bounded multi-producer/multi-consumer pointer queue.
//!
//! A slot's sequence transfers ownership separately from position reservation:
//! a producer publishes only after writing, and a consumer releases only after
//! reading. Positions alone cannot protect a slot across concurrent wraparound.
//! This queue transfers pointer values, not task execution/context ownership.
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

struct Slot<T> {
    sequence: AtomicUsize,
    value: AtomicPtr<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    Null,
    Full,
    Contended,
}

pub struct BoundedQueue<T, const N: usize> {
    // Diagnostic snapshots only: reservations can precede publication/release.
    pub top: AtomicUsize,
    pub bottom: AtomicUsize,
    slots: [Slot<T>; N],
}

impl<T, const N: usize> BoundedQueue<T, N> {
    pub const fn new() -> Self {
        assert!(N >= 2 && N.is_power_of_two() && N <= isize::MAX as usize);
        let mut slots = [const {
            Slot { sequence: AtomicUsize::new(0), value: AtomicPtr::new(ptr::null_mut()) }
        }; N];
        let mut i = 0;
        while i < N {
            slots[i].sequence = AtomicUsize::new(i);
            i += 1;
        }
        Self { top: AtomicUsize::new(0), bottom: AtomicUsize::new(0), slots }
    }

    /// On error no slot was reserved and the caller retains the pointer.
    /// Full also includes a slot still owned by an in-flight consumer.
    /// Work is bounded: retry Contended later, never spin in an interrupt.
    pub fn try_push(&self, value: *mut T) -> Result<(), PushError> {
        if value.is_null() { return Err(PushError::Null); }
        let mut pos = self.bottom.load(Ordering::Relaxed);
        for _ in 0..N {
            let slot = &self.slots[pos & (N - 1)];
            let sequence = slot.sequence.load(Ordering::Acquire);
            let difference = sequence.wrapping_sub(pos) as isize;
            if difference == 0 {
                match self.bottom.compare_exchange_weak(
                    pos, pos.wrapping_add(1), Ordering::Relaxed, Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        slot.value.store(value, Ordering::Relaxed);
                        slot.sequence.store(pos.wrapping_add(1), Ordering::Release);
                        return Ok(());
                    }
                    Err(actual) => pos = actual,
                }
            } else if difference < 0 {
                return Err(PushError::Full);
            } else {
                pos = self.bottom.load(Ordering::Relaxed);
            }
        }
        Err(PushError::Contended)
    }

    /// Compatibility for callers that cannot recover insertion ownership yet.
    /// Fail closed instead of silently orphaning a runnable task on saturation.
    pub fn push(&self, value: *mut T) {
        self.try_push(value).expect("scheduler runqueue insertion failed");
    }

    /// Null means no published head was obtained in this bounded attempt.
    /// It can mean empty, contention, or an earlier producer still publishing;
    /// callers must not use it as proof that all producers are quiescent.
    pub fn steal(&self) -> *mut T {
        let mut pos = self.top.load(Ordering::Relaxed);
        for _ in 0..N {
            let slot = &self.slots[pos & (N - 1)];
            let sequence = slot.sequence.load(Ordering::Acquire);
            let difference = sequence.wrapping_sub(pos.wrapping_add(1)) as isize;
            if difference == 0 {
                match self.top.compare_exchange_weak(
                    pos, pos.wrapping_add(1), Ordering::Relaxed, Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let value = slot.value.load(Ordering::Relaxed);
                        slot.sequence.store(pos.wrapping_add(N), Ordering::Release);
                        return value;
                    }
                    Err(actual) => pos = actual,
                }
            } else if difference < 0 {
                return ptr::null_mut();
            } else {
                pos = self.top.load(Ordering::Relaxed);
            }
        }
        ptr::null_mut()
    }
}

#[cfg(test)]
#[path = "scheduler_queue_tests.rs"]
mod tests;
