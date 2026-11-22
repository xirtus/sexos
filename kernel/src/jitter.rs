// SPDX-License-Identifier: GPL-3.0-only
//! Race hunting (kernel pass 2, K5): a build with `SEXOS_JITTER=<seed>` perturbs timing
//! at the points where today's races lived, so a flake shows up on the PR that causes it.
//! Every perturbation is one the hardware may legally produce anyway:
//!
//! - **wake**: a random delay before a wake is published (a slow waker, an SMI);
//! - **kick**: a wake IPI to a parked core is dropped, so that core only looks again on
//!   its next timer tick (a late interrupt; never a lost wake, since the wake is recorded);
//! - **entry**: a random delay at system call entry (a slow core, a preemption just
//!   before the kernel ran).
//!
//! In any other build `ON` is false and every hook compiles to nothing.
use core::sync::atomic::{AtomicU64, Ordering};

pub const ON: bool = option_env!("SEXOS_JITTER").is_some();
const MAX_CORES: usize = crate::scheduler::task_ownership::MAX_CORES as usize;
/// Longest delay, in TSC cycles (~20 µs at 2.5 GHz).
const MAX_DELAY: u64 = 50_000;
/// One in this many sites is perturbed.
const ONE_IN: u64 = 8;

static STATE: [AtomicU64; MAX_CORES] = [const { AtomicU64::new(0) }; MAX_CORES];
pub static COUNTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
#[derive(Clone, Copy)]
pub enum Site { Wake = 0, Kick = 1, Entry = 2 }

/// Seeds every core's generator from the build's seed; logs it so a failure can be named.
pub fn init() {
    if !ON { return; }
    let seed = option_env!("SEXOS_JITTER").and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(1).max(1);
    for (core, state) in STATE.iter().enumerate() {
        state.store(seed ^ ((core as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)), Ordering::Relaxed);
    }
    crate::serial_println!("[jitter] seed={} one_in={} max_delay_cycles={}", seed, ONE_IN, MAX_DELAY);
}

fn next() -> u64 {
    let core = crate::core_local::CoreLocal::get().core_id as usize % MAX_CORES;
    let mut x = STATE[core].load(Ordering::Relaxed);
    if x == 0 { x = 0x2545_F491_4F6C_DD1D; }
    x ^= x << 13; x ^= x >> 7; x ^= x << 17;
    STATE[core].store(x, Ordering::Relaxed);
    x
}

/// True for one site in `ONE_IN`, counted.
#[inline(always)]
pub fn hit(site: Site) -> bool {
    if !ON { return false; }
    let yes = next() % ONE_IN == 0;
    if yes { COUNTS[site as usize].fetch_add(1, Ordering::Relaxed); }
    yes
}

/// Spins a random time at `site`, now and then.
#[inline(always)]
pub fn delay(site: Site) {
    if !ON || !hit(site) { return; }
    let until = unsafe { core::arch::x86_64::_rdtsc() } + next() % MAX_DELAY;
    while unsafe { core::arch::x86_64::_rdtsc() } < until { core::hint::spin_loop(); }
}
