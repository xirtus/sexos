// SPDX-License-Identifier: GPL-3.0-only
//! Stack high-water mark of the Browser main task.
//!
//! BOOT maps 64 KiB of stack below a 64 KiB-aligned top; a deeper stack
//! needs a demand-paged page that the pager quota may refuse (feature ledger
//! F4). `paint` fills the unused part with a pattern once, early; `high_water`
//! reports how deep the stack has ever reached since. Diagnostic only.

const STACK: usize = 65536;
const PATTERN: u64 = 0x5e05_57ac_6e05_57ac;
/// Leave the lowest bytes alone and do not touch the live frame.
const GUARD: usize = 256;
static mut TOP: usize = 0;
static mut LOW: usize = 0;

#[inline(always)]
fn rsp() -> usize {
    let r: usize;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) r) };
    r
}

/// Paint the stack below the caller's frame. Call once, early.
#[inline(never)]
pub fn paint() {
    let sp = rsp();
    let top = (sp + 0xffff) & !0xffff;
    let low = top - STACK + GUARD;
    let end = sp.saturating_sub(1024);
    if end <= low { return; }
    let mut a = low;
    while a < end { unsafe { core::ptr::write_volatile(a as *mut u64, PATTERN) }; a += 8; }
    unsafe { TOP = top; LOW = low; PAINT_DEPTH = top - sp; }
}
static mut PAINT_DEPTH: usize = 0;

/// Stack depth of the frame that painted, and bytes still holding the pattern.
pub fn detail() -> (usize, usize) {
    let (top, low) = unsafe { (TOP, LOW) };
    let mut a = low;
    let mut untouched = 0;
    while a < top { if unsafe { core::ptr::read_volatile(a as *const u64) } == PATTERN { untouched += 8; } a += 8; }
    (unsafe { PAINT_DEPTH }, untouched)
}

/// Deepest stack use in bytes since `paint`, if painted.
pub fn high_water() -> Option<usize> {
    let (top, low) = unsafe { (TOP, LOW) };
    if top == 0 { return None; }
    let mut a = low;
    while a < top && unsafe { core::ptr::read_volatile(a as *const u64) } == PATTERN { a += 8; }
    Some(top - a)
}
