// SPDX-License-Identifier: GPL-3.0-only
//! Frames lent to program runs: a run's image and heap. They come from the memory every
//! program shares, zeroed, one page at a time as the run asks, and go back when the run
//! ends. Asking mints no authority: the row's arena, key and right to grow were fixed
//! at BOOT, and this only decides whether the machine has a page to spare.
//!
//! Programs never take the last of memory: `FLOOR` pages stay for the kernel and its
//! services, so a program that asks for too much is refused, and nothing else breaks.
use core::sync::atomic::{AtomicU64, Ordering};
use crate::memory::allocator::GLOBAL_ALLOCATOR;

/// 16 MiB kept back from programs.
pub const FLOOR: u64 = 4096;
static LENT: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

/// A zeroed frame for a program run, or None when memory is down to the floor.
pub fn take() -> Option<u64> {
    if GLOBAL_ALLOCATOR.free_frames() <= FLOOR { return None; }
    let frame = GLOBAL_ALLOCATOR.alloc(0)?;
    crate::boot_memory::zero_frame(frame);
    let lent = LENT.fetch_add(1, Ordering::Relaxed) + 1;
    PEAK.fetch_max(lent, Ordering::Relaxed);
    Some(frame)
}
/// Returns a frame `take` gave. Its leaf must be clear and flushed on every core.
pub fn give(frame: u64) {
    GLOBAL_ALLOCATOR.free(frame, 0);
    LENT.fetch_sub(1, Ordering::Relaxed);
}
/// Pages programs could still take.
pub fn available() -> u64 { GLOBAL_ALLOCATOR.free_frames().saturating_sub(FLOOR) }
pub fn lent() -> u64 { LENT.load(Ordering::Relaxed) }
pub fn peak() -> u64 { PEAK.load(Ordering::Relaxed) }
