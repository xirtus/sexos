//! Phase-4 orchestrator — reached by ERET chain from the Phase-3 EL0
//! warden (final tag), at EL1h with DAIF masked on the boot stack.

use super::serial;

#[unsafe(no_mangle)]
pub extern "C" fn arm64_phase4_entry() -> ! {
    serial::write_str("[sex.arm.p4] entry=1\n");
    unsafe { core::arch::asm!("msr TPIDR_EL1, {}", in(reg) 0u64, options(nostack)) }
    super::gicv3::init_bsp();
    super::sched::init();
    super::smp::boot_ap();
    // Timer LAST: starting it earlier risks a pre-scheduler expiry, which
    // would make the a_run/b_run interleave nondeterministic (documented
    // in the Phase-4 handoff).
    super::timer::init();
    super::sched::start(); // diverges into task A
}
