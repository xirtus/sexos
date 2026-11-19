// SPDX-License-Identifier: GPL-3.0-only
//! PKU gates, kernel side (docs/handoff/PKU_GATES_STUDY_2026_11_18.md, P6.2a).
//!
//! A gate stub finds the running task's gate area with RDPID, which reads IA32_TSC_AUX.
//! `Scheduler::switch_to`, the one place a core starts running a task, loads TSC_AUX
//! from `TaskContext.gate_aux` once [`ON`] is set. Until gates are switched on that
//! costs one predicted branch per switch. Ring 3 cannot write TSC_AUX, and Drag guests
//! get neither RDTSCP nor the MSR, so a task's RDPID always names its own area.
use core::sync::atomic::{AtomicBool, Ordering};

pub const TSC_AUX: u32 = 0xC000_0103;

/// Set once gates may run: PKU on and RDPID present (study 4.1). Index 0 is "no area"
/// and is never mapped, so a stub run by a task without an area faults at once.
pub static ON: AtomicBool = AtomicBool::new(false);

fn rdpid() -> u64 {
    let v: u64;
    unsafe { core::arch::asm!("rdpid {}", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

/// Boot check on the first core: RDPID exists and returns what was written to TSC_AUX.
/// Gates stay off; the first granted gate switches them on (P6.3).
pub fn init() -> bool {
    let has = core::arch::x86_64::__cpuid_count(7, 0).ecx & (1 << 22) != 0;
    let ok = has && crate::pku::PKU_ENABLED.load(Ordering::Acquire) != 0 && unsafe {
        let mut m = x86_64::registers::model_specific::Msr::new(TSC_AUX);
        m.write(0x5A5);
        let seen = rdpid();
        m.write(0);
        seen == 0x5A5
    };
    crate::serial_println!("[gate.rdpid] cpu={} usable={} gates=off", has as u8, ok as u8);
    ok
}
