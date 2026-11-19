// SPDX-License-Identifier: GPL-3.0-only
//! Scheduling class of the calling task (system call 104).
//!
//! `deadline(period_us)` puts this task in the deadline class: each time it is woken it
//! runs ahead of interactive and normal work, earliest deadline first, and should be
//! done within `period_us` (an audio thread refilling a buffer: the buffer's length).
//! Deadline work may take half of each core per 10 ms window; past that it waits
//! behind everything else. Periods are clamped to 500 µs ..= 100 ms. Display and Bell
//! are in the class from boot.

/// Puts the calling task in the deadline class with this period, or takes it out with 0.
pub fn deadline(period_us: u64) -> bool {
    let ret: u64;
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") 104u64 => ret, in("rdi") 1u64, in("rsi") period_us,
            out("rcx") _, out("r11") _, options(nostack));
    }
    ret == 0
}
