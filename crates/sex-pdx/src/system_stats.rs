// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! System statistics (syscall 87): CPU time by domain and how much memory each run
//! holds, for the system monitor. Reading needs no right: it is numbers only. A program
//! names itself here so the monitor can show it (`sex_app::Window::title` does it).
pub const SYS_STATS: u64 = 87;
/// rsi = TSC now, rdx = cores, r10 = idle cycles of all cores, r8 = free program pages.
pub const OP_OVERVIEW: u64 = 0;
/// rsi(pd) -> rsi = cycles, rdx = pages held (UNKNOWN if not an app row), r10 = live task, r8 = system calls made.
pub const OP_DOMAIN: u64 = 1;
/// rsi(pd) -> rsi, rdx = sixteen bytes of name, little-endian.
pub const OP_NAME: u64 = 2;
/// rsi(after) -> rsi = the next domain that ran or has a name; NONE when there is none.
pub const OP_NEXT: u64 = 3;
/// rsi, rdx = sixteen bytes of the caller's own name.
pub const OP_SET_NAME: u64 = 4;
pub const OK: u64 = 0;
pub const INVALID: u64 = 1;
pub const NONE: u64 = 2;
pub const UNKNOWN: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overview { pub tsc: u64, pub cores: u64, pub idle_cycles: u64, pub free_pages: u64 }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain { pub pd: u32, pub cycles: u64, pub pages: u64, pub task: u32, pub syscalls: u64 }

#[cfg(target_arch = "x86_64")]
fn call(op: u64, a: u64, b: u64) -> (u64, [u64; 4]) {
    let (status, r1, r2, r3, r4): (u64, u64, u64, u64, u64);
    unsafe { core::arch::asm!("syscall", inlateout("rax") SYS_STATS => status,
        in("rdi") op, inlateout("rsi") a => r1, inlateout("rdx") b => r2,
        lateout("r10") r3, lateout("r8") r4, out("rcx") _, out("r11") _, options(nostack)); }
    (status, [r1, r2, r3, r4])
}
#[cfg(not(target_arch = "x86_64"))]
fn call(_op: u64, _a: u64, _b: u64) -> (u64, [u64; 4]) { (INVALID, [0; 4]) }

pub fn overview() -> Option<Overview> {
    let (s, r) = call(OP_OVERVIEW, 0, 0);
    (s == OK).then_some(Overview { tsc: r[0], cores: r[1], idle_cycles: r[2], free_pages: r[3] })
}
pub fn domain(pd: u32) -> Option<Domain> {
    let (s, r) = call(OP_DOMAIN, pd as u64, 0);
    (s == OK).then_some(Domain { pd, cycles: r[0], pages: r[1], task: r[2] as u32, syscalls: r[3] })
}
pub fn name(pd: u32) -> [u8; 16] {
    let (s, r) = call(OP_NAME, pd as u64, 0);
    let mut out = [0u8; 16];
    if s == OK { out[..8].copy_from_slice(&r[0].to_le_bytes()); out[8..].copy_from_slice(&r[1].to_le_bytes()); }
    out
}
pub fn next(after: u32) -> Option<u32> {
    let (s, r) = call(OP_NEXT, after as u64, 0);
    (s == OK).then_some(r[0] as u32)
}
/// Names the calling domain; at most sixteen bytes are kept.
pub fn set_name(name: &[u8]) {
    let mut b = [0u8; 16];
    let n = name.len().min(16);
    b[..n].copy_from_slice(&name[..n]);
    let _ = call(OP_SET_NAME, u64::from_le_bytes(b[..8].try_into().unwrap()), u64::from_le_bytes(b[8..].try_into().unwrap()));
}
/// Percent of one core, in tenths, used between two samples of the same domain.
pub fn tenths_of_core(cycles_before: u64, cycles_now: u64, tsc_before: u64, tsc_now: u64) -> u64 {
    let span = tsc_now.saturating_sub(tsc_before);
    if span == 0 { return 0; }
    (cycles_now.saturating_sub(cycles_before) as u128 * 1000 / span as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn busy_core_is_a_thousand_tenths() {
        assert_eq!(tenths_of_core(0, 500, 1000, 1500), 1000);
        assert_eq!(tenths_of_core(100, 150, 0, 1000), 50);
        assert_eq!(tenths_of_core(5, 1, 0, 0), 0);
    }
}
