// SPDX-License-Identifier: GPL-3.0-only
//! CPU time by domain, for the system monitor. Each core notes, at every switch, who it
//! ran since the last one and charges those TSC cycles to that domain or to its own idle
//! count. Reading is open to any task: it shows numbers, never memory or authority.
//! A program may name itself (sixteen bytes) so the monitor can list it by name.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use sex_pdx::system_stats as wire;

const DOMAINS: usize = crate::ipc::MAX_DOMAINS;
const CORES: usize = 128;
const IDLE: u32 = u32::MAX;
static CYCLES: [AtomicU64; DOMAINS] = [const { AtomicU64::new(0) }; DOMAINS];
static NAMES: [[AtomicU64; 2]; DOMAINS] = [const { [const { AtomicU64::new(0) }; 2] }; DOMAINS];
static IDLE_CYCLES: [AtomicU64; CORES] = [const { AtomicU64::new(0) }; CORES];
static SINCE: [AtomicU64; CORES] = [const { AtomicU64::new(0) }; CORES];
static SYSCALLS: [AtomicU64; DOMAINS] = [const { AtomicU64::new(0) }; DOMAINS];
static RUNNING: [AtomicU32; CORES] = [const { AtomicU32::new(IDLE) }; CORES];

fn now() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

/// This core now runs `pd` (or nothing, `None`): charge whoever ran until now.
pub fn switch(core: u32, pd: Option<u32>) {
    let core = core as usize;
    if core >= CORES { return; }
    let t = now();
    let was = RUNNING[core].swap(pd.unwrap_or(IDLE), Ordering::Relaxed);
    let since = SINCE[core].swap(t, Ordering::Relaxed);
    if since == 0 || t < since { return; }
    let spent = t - since;
    match was {
        IDLE => { IDLE_CYCLES[core].fetch_add(spent, Ordering::Relaxed); }
        pd if (pd as usize) < DOMAINS => { CYCLES[pd as usize].fetch_add(spent, Ordering::Relaxed); }
        _ => {}
    }
}

/// One more system call by the domain this core runs.
pub fn count_syscall() {
    let pd = crate::core_local::CoreLocal::get().current_pd() as usize;
    if pd < DOMAINS { SYSCALLS[pd].fetch_add(1, Ordering::Relaxed); }
}

fn cores() -> usize { (crate::smp::ONLINE.load(Ordering::Acquire) as usize).clamp(1, CORES) }

/// Charged cycles plus the run in progress on any core; `IDLE` sums every core's idle time.
fn cycles_of(pd: u32, t: u64) -> u64 {
    let mut total = if pd == IDLE { 0 } else { CYCLES[pd as usize].load(Ordering::Relaxed) };
    for core in 0..cores() {
        if pd == IDLE { total += IDLE_CYCLES[core].load(Ordering::Relaxed); }
        if RUNNING[core].load(Ordering::Relaxed) != pd { continue; }
        let since = SINCE[core].load(Ordering::Relaxed);
        if since != 0 && t > since { total += t - since; }
    }
    total
}

fn named(pd: usize) -> bool { NAMES[pd][0].load(Ordering::Relaxed) != 0 }

pub fn syscall(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let caller = crate::core_local::CoreLocal::get().current_pd();
    let (op, arg, arg2) = (regs.rdi, regs.rsi, regs.rdx);
    regs.rsi = 0; regs.rdx = 0; regs.r10 = 0; regs.r8 = 0;
    let t = now();
    match op {
        wire::OP_OVERVIEW => {
            regs.rsi = t;
            regs.rdx = cores() as u64;
            regs.r10 = cycles_of(IDLE, t);
            regs.r8 = crate::program_frames::available();
            wire::OK
        }
        wire::OP_DOMAIN => {
            if arg as usize >= DOMAINS || arg == 0 { return wire::INVALID; }
            let pd = arg as u32;
            regs.rsi = cycles_of(pd, t);
            let (task, pages) = crate::app_resources::held_pages(pd).unwrap_or((0, wire::UNKNOWN));
            regs.rdx = pages;
            regs.r10 = task as u64;
            regs.r8 = SYSCALLS[pd as usize].load(Ordering::Relaxed);
            wire::OK
        }
        wire::OP_NAME => {
            if arg as usize >= DOMAINS || arg == 0 { return wire::INVALID; }
            regs.rsi = NAMES[arg as usize][0].load(Ordering::Relaxed);
            regs.rdx = NAMES[arg as usize][1].load(Ordering::Relaxed);
            wire::OK
        }
        wire::OP_NEXT => {
            let start = (arg as usize).saturating_add(1);
            for pd in start.max(1)..DOMAINS {
                if CYCLES[pd].load(Ordering::Relaxed) != 0 || named(pd)
                    || (0..cores()).any(|c| RUNNING[c].load(Ordering::Relaxed) == pd as u32) {
                    regs.rsi = pd as u64;
                    return wire::OK;
                }
            }
            wire::NONE
        }
        wire::OP_SET_NAME => {
            if caller == 0 || caller as usize >= DOMAINS || arg == 0 { return wire::INVALID; }
            NAMES[caller as usize][0].store(arg, Ordering::Relaxed);
            NAMES[caller as usize][1].store(arg2, Ordering::Relaxed);
            wire::OK
        }
        _ => wire::INVALID,
    }
}
