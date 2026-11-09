// SPDX-License-Identifier: GPL-3.0-only
//! Code a program writes for itself: the base a JIT engine needs. A run turns pages of
//! its own heap into code and back, and a page is never writable and executable at once.
//!
//! The call is self-only, like `program_memory`. Only heap pages the current run was
//! given can change; the program's file image and its stack never do.
//!
//! Sealing checks every byte first. Code may not hold the instructions that rewrite the
//! protection-key register (WRPKRU, XRSTOR, XRSTORS), at any offset, because a jump into
//! the middle of another instruction would run them. That keeps a program's own code
//! inside the key the kernel gave it, which is something Linux does not promise: there
//! any code can change its keys. A JIT that meets a refusal re-encodes the constant.
//!
//! rdi = op, rsi = first page's address, rdx = pages
//! → rax = status, rsi = byte offset of a refused instruction (BLOCKED), rdx = pages changed.
pub const SYS_PROGRAM_CODE: u64 = 82;

/// Pages become code: executable, no longer writable.
pub const SEAL: u64 = 1;
/// Pages become data again: writable, not executable. For patching or reuse.
pub const OPEN: u64 = 2;

pub const OK: u64 = 0;
/// Bad op, address not page-aligned, zero pages, or more than one call changes.
pub const INVALID: u64 = 1;
/// Try again: another core holds the program table.
pub const RETRY: u64 = 2;
/// Not this row's current run, or a page that is not the run's heap.
pub const DENIED: u64 = 3;
/// The bytes hold an instruction code may not contain; rsi gives its offset.
pub const BLOCKED: u64 = 7;

/// Most pages one call changes, so the kernel's interrupts-off time stays short.
pub const MAX_PAGES_PER_CALL: u64 = 256;

/// Whether the three bytes starting with 0x0F form a refused instruction:
/// WRPKRU (0F 01 EF), XRSTOR (0F AE /5, memory form) or XRSTORS (0F C7 /3, memory form).
/// The register forms of 0F AE /5 are LFENCE and harmless.
pub const fn refused(b1: u8, b2: u8) -> bool {
    let (reg, memory) = ((b2 >> 3) & 7, b2 >> 6 != 3);
    (b1 == 0x01 && b2 == 0xEF) || (b1 == 0xAE && reg == 5 && memory) || (b1 == 0xC7 && reg == 3 && memory)
}

/// Checks code a piece at a time, so pages need not be contiguous in memory.
/// Every refused instruction is three bytes, so two bytes of history are enough.
#[derive(Clone, Copy, Debug)]
pub struct Scan { history: [u8; 2], seen: u64 }
impl Scan {
    pub const fn new() -> Self { Self { history: [0; 2], seen: 0 } }
    /// Starts with bytes that come before the checked range (an executable neighbour),
    /// so an instruction across the boundary is still found. They count as offset < 0.
    pub fn primed(before: [u8; 2]) -> Self { Self { history: before, seen: 2 } }
    /// Feeds the next bytes. Returns the offset, from the first byte fed after priming,
    /// of the first refused instruction's 0x0F; it may be negative when it starts in
    /// the priming bytes.
    pub fn feed(&mut self, bytes: &[u8], primed: bool) -> Option<i64> {
        for &b in bytes {
            let [h0, h1] = self.history;
            if self.seen >= 2 && h0 == 0x0F && refused(h1, b) {
                let start = self.seen as i64 - 2;
                return Some(if primed { start - 2 } else { start });
            }
            self.history = [h1, b];
            self.seen += 1;
        }
        None
    }
}

/// Whole-slice check, for hosts and tests.
pub fn check(code: &[u8]) -> Result<(), usize> {
    match Scan::new().feed(code, false) { Some(at) => Err(at as usize), None => Ok(()) }
}

/// Seals or opens `pages` pages from `va`. Retries while another core holds the table.
/// Returns pages changed, or (status, offset) where offset is set for BLOCKED.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn call(op: u64, va: u64, pages: u64) -> Result<u64, (u64, u64)> {
    loop {
        let (status, offset, changed): (u64, u64, u64);
        unsafe {
            core::arch::asm!("syscall",
                inlateout("rax") SYS_PROGRAM_CODE => status,
                inlateout("rdi") op => _,
                inlateout("rsi") va => offset, inlateout("rdx") pages => changed,
                lateout("r10") _, lateout("r8") _,
                out("rcx") _, out("r11") _, options(nostack));
        }
        match status {
            OK => return Ok(changed),
            RETRY => crate::sys_yield(),
            e => return Err((e, offset)),
        }
    }
}
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub fn call(_op: u64, _va: u64, _pages: u64) -> Result<u64, (u64, u64)> { Err((u64::MAX, 0)) }

/// Makes the pages code.
pub fn seal(va: u64, pages: u64) -> Result<u64, (u64, u64)> { call(SEAL, va, pages) }
/// Makes the pages writable data again.
pub fn open(va: u64, pages: u64) -> Result<u64, (u64, u64)> { call(OPEN, va, pages) }
