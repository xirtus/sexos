// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Denim: compressed memory. The Denim service keeps a program's cold heap pages packed
//! in its own memory, so a machine can run programs whose memory adds up to more than
//! its RAM. The kernel holds the mechanism only:
//!
//! - `SCAN` reads and clears the accessed bits of 64 heap pages of a run;
//! - `OUT` takes one heap page: its leaf becomes a non-present marker naming the row,
//!   every core drops the translation, the page's bytes are copied into a page of
//!   Denim's own memory (the window), and the frame goes back to the free pool;
//! - a fault on a marked page parks the task and the kernel tells Denim (`OP_FAULT`);
//! - `IN` copies a window page into a fresh frame, maps it where the marker was and,
//!   given the fault's token, lets the parked task go on.
//!
//! Which pages, when, and how they are stored is Denim's policy. Only the Denim domain
//! may call. Only writable, non-executable heap pages (from `program_memory::grow`) of a
//! live, single-threaded run go out: a run that starts a worker gets `RETRY` and Denim a
//! `OP_RECALL`, and code pages stay. A run's out pages simply end with it.

/// rdi = op, rsi = target domain, rdx/r10/r8 per op → rax = status, rsi..r8 per op.
pub const SYS: u64 = 88;
/// Denim's BOOT identity (domain and layout), outside the catalog's rows.
pub const DENIM_PD: u32 = 51;
/// The Memory organ, beside the pager (sext).
pub const DENIM_KEY: u8 = 14;

/// rdx = first heap page index → rsi = accessed bits (cleared by the scan), rdx = bits
/// of pages present and eligible to go out, r10 = heap pages | run << 32, r8 = heap base.
/// Bit i is heap page `first + i`.
pub const SCAN: u64 = 1;
/// rdx = heap page address, r10 = window (a page of Denim's own grown memory)
/// → rsi = pages of the run now out.
pub const OUT: u64 = 2;
/// rdx = heap page address, r10 = window holding its bytes, r8 = fault token (0 when
/// no task waits, as for a recall) → rsi = pages of the run still out.
pub const IN: u64 = 3;

pub const OK: u64 = 0;
pub const INVALID: u64 = 1;
/// Try again: the table is busy, or the page changed under the call.
pub const RETRY: u64 = 2;
/// Not Denim, not a live run, not a heap page that may go (or come) back.
pub const DENIED: u64 = 3;
/// No free frame to bring the page back into. The task stays parked; try later.
pub const FULL: u64 = 6;

/// From the kernel (caller 0): a run touched a page Denim holds. arg0 = target,
/// arg1 = page address, arg2 = token for `IN`.
pub const OP_FAULT: u64 = 0x3D1;
/// From the kernel (caller 0): bring back every page of arg0's run (it starts a worker
/// or turns heap into code).
pub const OP_RECALL: u64 = 0x3D2;

/// The leaf of a page Denim holds: not present, tagged, naming its row. Never a frame.
pub const OUT_TAG: u64 = 0x600;
pub const fn out_marker(target: u32) -> u64 { ((target as u64) << 12) | OUT_TAG }
pub const fn is_out(leaf: u64) -> bool { leaf & 0xFFF == OUT_TAG }
/// A heap page that may go out: present, user, writable, no-execute, on the row's key.
pub const fn heap_data(leaf: u64, pkey: u8) -> bool {
    leaf & 7 == 7 && leaf >> 63 == 1 && (leaf >> 59) & 15 == pkey as u64
}

#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "hermit")))]
pub fn call(op: u64, target: u32, a: u64, b: u64, c: u64) -> [u64; 5] {
    let (status, r1, r2, r3, r4);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS => status, inlateout("rdi") op => _,
            inlateout("rsi") target as u64 => r1, inlateout("rdx") a => r2,
            inlateout("r10") b => r3, inlateout("r8") c => r4,
            out("rcx") _, out("r11") _, options(nostack));
    }
    [status, r1, r2, r3, r4]
}
#[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "hermit"))))]
pub fn call(_op: u64, _target: u32, _a: u64, _b: u64, _c: u64) -> [u64; 5] { [u64::MAX, 0, 0, 0, 0] }

