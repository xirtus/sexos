// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! More memory for a running program. The pages come from its own row's window,
//! reserved at BOOT and mapped on the row's hardware key, so asking mints nothing:
//! no frames, no capability, no new key. A program gets pages only for itself, they
//! follow straight after its image, and they go, zeroed, when the run ends.
//!
//! The call is self-only. The kernel finds the row by the calling domain, and only
//! the row's current run may grow. Everything else is refused.

/// rdi = pages wanted (0 only asks), → rax = status, rsi = first page's address,
/// rdx = pages the run holds after the call, r10 = memory pressure (any caller).
pub const SYS_PROGRAM_MEMORY: u64 = 81;
pub const PAGE: u64 = 4096;
/// Most pages one call grants, so the kernel's interrupts-off time stays short.
pub const MAX_PAGES_PER_CALL: u64 = 256;

pub const OK: u64 = 0;
/// More pages than one call grants.
pub const INVALID: u64 = 1;
/// Try again: another core holds the program table.
pub const RETRY: u64 = 2;
/// Not a program run, or not this row's current one.
pub const DENIED: u64 = 3;
/// The window has fewer pages left than asked for. Nothing was granted.
pub const FULL: u64 = 6;

/// Memory pressure, returned in r10 by every call: how short the whole machine is on
/// memory. Advisory: on LOW drop caches and shrink buffers, on CRITICAL also refuse
/// new work. The kernel never kills for it.
pub const PRESSURE_NORMAL: u64 = 0;
/// Free memory below an eighth of what programs could have (at least 32 MiB).
pub const PRESSURE_LOW: u64 = 1;
/// Below a thirty-second (at least 8 MiB): allocations will soon be refused.
pub const PRESSURE_CRITICAL: u64 = 2;
/// From the kernel (caller 0) to the Shell when pressure becomes critical:
/// arg0 = level, arg1 = free program pages. The Shell's out-of-memory policy acts on it.
pub const OP_PRESSURE: u64 = 0x3A1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    /// The first page the run was given. Pages follow it without gaps.
    pub base: u64,
    /// How many pages the run holds now.
    pub pages: u64,
}
impl Memory {
    pub const fn end(&self) -> u64 { self.base + self.pages * PAGE }
}

/// Asks for `pages` more pages, all or none. `grow(0)` reports what the run holds.
#[cfg(target_arch = "x86_64")]
pub fn grow(pages: u64) -> Result<Memory, u64> {
    loop {
        let (status, base, held): (u64, u64, u64);
        unsafe {
            core::arch::asm!("syscall",
                inlateout("rax") SYS_PROGRAM_MEMORY => status,
                inlateout("rdi") pages => _,
                lateout("rsi") base, lateout("rdx") held,
                lateout("r10") _, lateout("r8") _,
                out("rcx") _, out("r11") _, options(nostack));
        }
        match status {
            OK => return Ok(Memory { base, pages: held }),
            RETRY => crate::sys_yield(),
            e => return Err(e),
        }
    }
}

/// The machine's memory pressure now. Any domain may ask; it grants nothing.
#[cfg(target_arch = "x86_64")]
pub fn pressure() -> u64 {
    let level: u64;
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS_PROGRAM_MEMORY => _,
            inlateout("rdi") 0u64 => _,
            lateout("rsi") _, lateout("rdx") _,
            lateout("r10") level, lateout("r8") _,
            out("rcx") _, out("r11") _, options(nostack));
    }
    level
}

/// Host builds (tests) have no kernel to ask.
#[cfg(not(target_arch = "x86_64"))]
pub fn grow(_pages: u64) -> Result<Memory, u64> { Err(u64::MAX) }
#[cfg(not(target_arch = "x86_64"))]
pub fn pressure() -> u64 { PRESSURE_NORMAL }
