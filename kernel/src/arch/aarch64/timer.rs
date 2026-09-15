//! ARM Generic Timer (CNTP, non-secure physical, PPI 30) — Phase 4.
//! Programs a periodic tick by reloading TVAL each interrupt.

use super::registers::*;
use super::serial;

static mut FREQ: u64 = 0;

/// Read CNTFRQ, program the first tick. GIC PPI 30 must already be
/// enabled (gicv3::init_bsp).
pub fn init() {
    let f: u64;
    unsafe { core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) f, options(nomem, nostack)) }
    if f == 0 {
        serial::write_str("[sex.arm.timer] FATAL freq=0\n");
        loop { core::hint::spin_loop() }
    }
    unsafe { FREQ = f }
    serial::write_str("[sex.arm.timer] freq=");
    serial::write_dec(f);
    serial::write_str(" hz=");
    serial::write_dec(TIMER_HZ);
    serial::write_str("\n");
    reload();
    // ENABLE=1, IMASK=0.
    unsafe { core::arch::asm!("msr CNTP_CTL_EL0, {}", in(reg) 1u64, options(nostack)) }
}

/// Reload the countdown for the next tick (call from the IRQ path).
pub fn reload() {
    let tval = unsafe { FREQ } / TIMER_HZ;
    unsafe { core::arch::asm!("msr CNTP_TVAL_EL0, {}", in(reg) tval, options(nostack)) }
}

/// Disable the timer (Phase 5+: the SVC test and later phases run
/// uninterruptible; scheduling resumes only with a real scheduler port).
pub fn disable() {
    unsafe { core::arch::asm!("msr CNTP_CTL_EL0, {}", in(reg) 0u64, options(nostack)) }
    serial::write_str("[sex.arm.timer] off=1\n");
}
