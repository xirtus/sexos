// SPDX-License-Identifier: GPL-3.0-only
//! More of SYS_STATS (syscall 87), for Mesh: program memory and who holds what, as
//! numbers only. Kept apart from `system_stats`, which the self-wait review pins whole.
//! Answers use `system_stats`' OK, INVALID and NONE.
pub use crate::system_stats::{INVALID, NONE, OK};

/// rsi(core) -> rsi = idle cycles of that core, rdx = domain it runs now (0 idle), r10 =
/// TSC now. Reserved: kernels answer INVALID until the per-core idle counters, kept in the
/// self-wait-pinned `cpu_account`, are exposed after review.
pub const OP_CORE: u64 = 6;
/// rsi = pages lent to program runs now, rdx = most ever lent, r10 = most they could hold.
pub const OP_MEMORY: u64 = 7;
/// rsi(pd) -> rsi = the domain's PKU key (its organ), rdx = capabilities it holds.
pub const OP_ORGAN: u64 = 8;
/// rsi(pd), rdx(after) -> rsi = the next domain above `after` that `pd` holds a domain
/// capability to; NONE when there is none. IPC v2 endpoints name objects, not domains,
/// so they are counted by OP_ORGAN but not linked. Numbers only, never the capability.
pub const OP_LINK: u64 = 9;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Core { pub idle_cycles: u64, pub running: u32, pub tsc: u64 }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory { pub lent_pages: u64, pub peak_pages: u64, pub limit_pages: u64 }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Organ { pub pku_key: u8, pub capabilities: u64 }

#[cfg(target_arch = "x86_64")]
fn call(op: u64, a: u64, b: u64) -> (u64, [u64; 3]) {
    let (status, r1, r2, r3): (u64, u64, u64, u64);
    unsafe { core::arch::asm!("syscall", inlateout("rax") crate::system_stats::SYS_STATS => status,
        in("rdi") op, inlateout("rsi") a => r1, inlateout("rdx") b => r2,
        lateout("r10") r3, lateout("r8") _, out("rcx") _, out("r11") _, options(nostack)); }
    (status, [r1, r2, r3])
}
#[cfg(not(target_arch = "x86_64"))]
fn call(_op: u64, _a: u64, _b: u64) -> (u64, [u64; 3]) { (INVALID, [0; 3]) }

/// One core's idle time and what it runs; `None` past the last online core, or while the
/// kernel does not answer OP_CORE.
pub fn core(index: u32) -> Option<Core> {
    let (s, r) = call(OP_CORE, index as u64, 0);
    (s == OK).then_some(Core { idle_cycles: r[0], running: r[1] as u32, tsc: r[2] })
}
pub fn memory() -> Option<Memory> {
    let (s, r) = call(OP_MEMORY, 0, 0);
    (s == OK).then_some(Memory { lent_pages: r[0], peak_pages: r[1], limit_pages: r[2] })
}
pub fn organ(pd: u32) -> Option<Organ> {
    let (s, r) = call(OP_ORGAN, pd as u64, 0);
    (s == OK).then_some(Organ { pku_key: r[0] as u8, capabilities: r[1] })
}
/// The next domain above `after` that `pd` may call.
pub fn link(pd: u32, after: u32) -> Option<u32> {
    let (s, r) = call(OP_LINK, pd as u64, after as u64);
    (s == OK).then_some(r[0] as u32)
}
