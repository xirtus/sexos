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
    pressure();
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
    pressure();
}
/// Pages programs could still take.
pub fn available() -> u64 { GLOBAL_ALLOCATOR.free_frames().saturating_sub(FLOOR) }
pub fn lent() -> u64 { LENT.load(Ordering::Relaxed) }
pub fn peak() -> u64 { PEAK.load(Ordering::Relaxed) }

/// How short memory is, for programs and services to act on before an allocation
/// fails: drop caches, shrink buffers, refuse new work. Advisory, never a kill.
pub const NORMAL: u64 = sex_pdx::program_memory::PRESSURE_NORMAL;
pub const LOW: u64 = sex_pdx::program_memory::PRESSURE_LOW;
pub const CRITICAL: u64 = sex_pdx::program_memory::PRESSURE_CRITICAL;
/// Most pages programs could take at once: the free count before they took any.
static BASE: AtomicU64 = AtomicU64::new(0);
static LEVEL: AtomicU64 = AtomicU64::new(NORMAL);

fn level_of(free: u64, base: u64) -> u64 {
    // Low below an eighth of the machine (at least 32 MiB), critical below a
    // thirty-second (at least 8 MiB): the same shares on 512 MiB as on 64 GiB.
    if free < (base / 32).max(2048) { CRITICAL } else if free < (base / 8).max(8192) { LOW } else { NORMAL }
}
/// The current level. It rises as soon as free memory crosses a mark, and falls only
/// once free memory is a quarter above it, so a program at the edge doesn't flap.
pub fn pressure() -> u64 {
    let free = available();
    let base = BASE.fetch_max(free, Ordering::Relaxed).max(free);
    let now = LEVEL.load(Ordering::Relaxed);
    let level = level_of(free, base).max(now.min(level_of(free - free / 5, base)));
    if level != now && LEVEL.compare_exchange(now, level, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        crate::serial_println!("[memory.pressure] level={} free_pages={} base_pages={}", level, free, base);
    }
    level
}
