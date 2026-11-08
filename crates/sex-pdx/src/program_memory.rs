// SPDX-License-Identifier: GPL-3.0-only
//! More memory for a running program. The pages come from its own row's window,
//! reserved at BOOT and mapped on the row's hardware key, so asking mints nothing:
//! no frames, no capability, no new key. A program gets pages only for itself, they
//! follow straight after its image, and they go, zeroed, when the run ends.
//!
//! The call is self-only. The kernel finds the row by the calling domain, and only
//! the row's current run may grow. Everything else is refused.

/// rdi = pages wanted (0 only asks), → rax = status, rsi = first page's address,
/// rdx = pages the run holds after the call.
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
