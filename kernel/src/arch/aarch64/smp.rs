//! Minimal SMP bring-up — Phase 4 scope: PSCI CPU_ON for core 1, AP does
//! EL-handling + MMU-on with the SHARED page tables (single global VAS),
//! prints SEX_ARM_AP_OK, sets AP_DONE, WFEs. No per-AP scheduling yet.
//!
//! Cache-coherence protocol (documented, gate-listed): before CPU_ON the
//! BSP cleans+invalidates D-cache over the whole kernel image (page
//! tables were written with caches OFF so RAM is clean; post-MMU globals
//! like TTBR0_PA are dirty in D-cache and the AP reads them with caches
//! OFF) and invalidates I-cache. AP prints/sets AP_DONE only AFTER its
//! MMU is on (inner-shareable coherent with BSP).

use super::registers::*;
use super::serial;
use core::sync::atomic::{AtomicU64, Ordering};

/// AP handshake (post-MMU both sides → coherent).
static AP_DONE: AtomicU64 = AtomicU64::new(0);

/// AP's own 16 KiB stack.
#[repr(align(16))]
pub struct ApStack([u8; 16384]);
#[unsafe(no_mangle)]
pub static mut AP_STACK: ApStack = ApStack([0; 16384]);

core::arch::global_asm!(
    ".text",
    ".balign 4",
    ".global sex_arm_ap_entry",
    "sex_arm_ap_entry:",
    // Entered via PSCI CPU_ON: x0 = context id (unused), MMU off,
    // DAIF unknown, EL = implementation start EL (QEMU virt: EL2).
    "msr DAIFSet, #0xF",
    "mrs x21, CurrentEL",
    "lsr x21, x21, #2",
    "cmp x21, #2",
    "b.eq .Lap_el2",
    "b .Lap_el1",
    ".Lap_el2:",
    "mov x9, #3",
    "msr CNTHCTL_EL2, x9", // EL1 may use CNTP (parity with BSP entry)
    "mov x9, #(1 << 31)",  // HCR_EL2.RW
    "msr HCR_EL2, x9",
    "ldr x9, =0x30D00800", // SCTLR_EL1 safe (mirrors registers.rs)
    "msr SCTLR_EL1, x9",
    "msr CPACR_EL1, xzr",
    "mov x9, #0x3C5",      // EL1h, DAIF masked
    "msr SPSR_EL2, x9",
    "ldr x9, =.Lap_el1",
    "msr ELR_EL2, x9",
    "ldr x9, =AP_STACK",
    "add x9, x9, #16384",
    "msr SP_EL1, x9",
    "eret",
    ".Lap_el1:",
    "ldr x9, =AP_STACK",
    "add x9, x9, #16384",
    "mov sp, x9",
    "bl sex_arm_ap_main",
    ".Lap_halt:",
    "wfe",
    "b .Lap_halt",
);

unsafe extern "C" {
    fn sex_arm_ap_entry() -> !;
    static __kernel_start: u8;
    static __kernel_end: u8;
}

/// AP Rust entry (MMU still OFF here): enable MMU with the shared tables,
/// install VBAR, tag core id, signal, halt.
#[unsafe(no_mangle)]
extern "C" fn sex_arm_ap_main() -> ! {
    unsafe {
        let b0 = super::mmu::ttbr0_pa();
        let b1 = super::mmu::ttbr1_pa();
        dsb_sy();
        core::arch::asm!(
            "msr MAIR_EL1, {m}",
            "msr TCR_EL1, {t}",
            "msr TTBR0_EL1, {b0}",
            "msr TTBR1_EL1, {b1}",
            "isb",
            "msr SCTLR_EL1, {s}",
            "isb",
            m = in(reg) MAIR_EL1_V,
            t = in(reg) TCR_EL1_V,
            b0 = in(reg) b0,
            b1 = in(reg) b1,
            s = in(reg) SCTLR_EL1_MMU,
            options(nostack),
        );
        super::exceptions::init(); // VBAR (shared vectors)
        core::arch::asm!("msr TPIDR_EL1, {}", in(reg) 1u64, options(nostack));
        serial::write_str("SEX_ARM_AP_OK\n");
        AP_DONE.store(1, Ordering::Release);
        loop { wfe() }
    }
}

/// BSP side: clean caches, CPU_ON core 1, wait for the AP marker flag.
pub fn boot_ap() {
    unsafe {
        // Clean+invalidate D-cache over the kernel image (AP reads with
        // caches off), then I-cache, then DSB.
        let start = &raw const __kernel_start as u64;
        let end = &raw const __kernel_end as u64;
        let mut p = start & !0x3F;
        while p < end {
            core::arch::asm!("dc cvac, {}", in(reg) p, options(nostack));
            p += 64;
        }
        core::arch::asm!("dsb sy", "ic iallu", "dsb sy", options(nostack));
        let entry = sex_arm_ap_entry as u64; // physical == link address
        let mut ret: i64;
        // PSCI conduit: QEMU TCG implements PSCI via SMC (intercepted in
        // QEMU's CPU loop); HVC from EL1 traps to the GUEST's EL2 where no
        // vectors exist — runtime-found 2026-07-22 (HVC never returned,
        // BSP spun at VBAR_EL2=0). SMC matches QEMU virt's DTB-advertised
        // psci method under TCG. (KVM/real hardware uses HVC — platform
        // here is QEMU TCG.)
        core::arch::asm!(
            "mov x0, {psci}",
            "mov x1, {mpidr}",
            "mov x2, {entry}",
            "mov x3, xzr",
            "smc #0",
            "mov {ret}, x0",
            psci = in(reg) PSCI_CPU_ON,
            mpidr = in(reg) AP_MPIDR,
            entry = in(reg) entry,
            ret = out(reg) ret,
            out("x0") _, out("x1") _, out("x2") _, out("x3") _,
            options(nostack),
        );
        if ret != 0 {
            serial::write_str("[sex.arm.smp] FATAL psci_ret=");
            serial::write_dec(ret as u64);
            serial::write_str("\nSEX_ARM_PHASE4_FAIL\n");
            loop { core::hint::spin_loop() }
        }
        serial::write_str("[sex.arm.smp] ap_boot=1\n");
        let mut spins: u64 = 0;
        while AP_DONE.load(Ordering::Acquire) == 0 {
            spins += 1;
            if spins > 100_000_000 {
                serial::write_str("[sex.arm.smp] FATAL ap_timeout\nSEX_ARM_PHASE4_FAIL\n");
                loop { core::hint::spin_loop() }
            }
            core::hint::spin_loop();
        }
        serial::write_str("[sex.arm.smp] ap_ack=1\n");
    }
}
