//! Minimal GICv3 driver — Phase 4 scope: exactly what the CNTP timer tick
//! needs, nothing more. QEMU virt gic-version=3, system-register CPU
//! interface (ICC_*_EL1), Group-1 non-secure. No ITS, no LPIs, no SPI
//! routing table (Phase-later drivers add their own).
//!
//! Access via the HHDM device alias (TTBR1) — the GIC region is NOT
//! identity-mapped in TTBR0.

use super::registers::*;
use super::serial;
use core::ptr::{read_volatile, write_volatile};

#[inline]
fn gicd(off: u64) -> *mut u32 { (HHDM_BASE + GICD_PA + off) as *mut u32 }
#[inline]
fn gicr(core: u64, off: u64) -> *mut u32 { (HHDM_BASE + GICR_PA + core * GICR_STRIDE + off) as *mut u32 }

// GICD offsets
const GICD_CTLR: u64 = 0x000;
const GICD_TYPER: u64 = 0x004;
// GICR offsets (RD frame / SGI frame)
const GICR_WAKER: u64 = 0x014;
const GICR_ISENABLER0: u64 = GICR_SGI_OFF + 0x100;
const GICR_IGROUPR0: u64 = GICR_SGI_OFF + 0x080;
const GICR_IPRIORITYR: u64 = GICR_SGI_OFF + 0x400;

/// Initialize distributor + this core's redistributor + CPU interface,
/// then enable the timer PPI. Boot core only (Phase 4).
pub fn init_bsp() {
    unsafe {
        // 1. Distributor: ARE_NS (bit 4) + EnableGrp1NS (bit 1).
        write_volatile(gicd(GICD_CTLR), (1 << 4) | (1 << 1));
        dsb_sy();
        let typer = read_volatile(gicd(GICD_TYPER));
        if read_volatile(gicd(GICD_CTLR)) & (1 << 1) == 0 {
            serial::write_str("[sex.arm.gic] FATAL gicd_grp1ns_clear\n");
            loop { core::hint::spin_loop() }
        }
        let _ = typer;
        serial::write_str("[sex.arm.gic] gicd=ok\n");

        // 2. Redistributor: wake core 0 (ProcessorSleep=0, poll
        //    ChildrenAsleep=0). DSB before dependent GICD/GICR accesses.
        write_volatile(gicr(0, GICR_WAKER), 0);
        dsb_sy();
        let mut spins: u32 = 0;
        while read_volatile(gicr(0, GICR_WAKER)) & (1 << 2) != 0 {
            spins += 1;
            if spins > 1_000_000 {
                serial::write_str("[sex.arm.gic] FATAL gicr_asleep\n");
                loop { core::hint::spin_loop() }
            }
            core::hint::spin_loop();
        }
        serial::write_str("[sex.arm.gic] gicr=ok\n");

        // 3. CPU interface (system registers): SRE=1, PMR=0xFF (all
        //    priorities visible), IGRPEN1=1.
        let mut sre: u64;
        core::arch::asm!("mrs {}, ICC_SRE_EL1", out(reg) sre, options(nomem, nostack));
        sre |= 1;
        core::arch::asm!(
            "msr ICC_SRE_EL1, {}",
            "isb",
            in(reg) sre, options(nostack));
        core::arch::asm!("msr ICC_PMR_EL1, {}", in(reg) 0xFFu64, options(nostack));
        core::arch::asm!("msr ICC_IGRPEN1_EL1, {}", "isb", in(reg) 1u64, options(nostack));
        serial::write_str("[sex.arm.gic] cpuif=ok\n");

        // 4. Timer PPI 30: Group 1 (resets Group 0, and with only
        //    IGRPEN1=1 a Group-0 PPI is NEVER signaled — runtime-found
        //    2026-07-22: no tick fired, task A monopolized), priority
        //    0x80 (byte per INTID), enable.
        write_volatile(gicr(0, GICR_IGROUPR0), 1 << INTID_TIMER);
        let prio = (HHDM_BASE + GICR_PA + GICR_IPRIORITYR + INTID_TIMER as u64) as *mut u8;
        write_volatile(prio, 0x80);
        write_volatile(gicr(0, GICR_ISENABLER0), 1 << INTID_TIMER);
        dsb_sy();
        serial::write_str("[sex.arm.gic] ppi30=on\n");
    }
}
