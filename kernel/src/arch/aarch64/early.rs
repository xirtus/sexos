//! AArch64 early main — reached from entry global_asm after the EL
//! transition, stack install and BSS clear.
//!
//! ABI contract with the entry assembly (src/arch/aarch64/main.rs):
//!   x0 = DTB physical pointer (preserved from reset in callee-saved x20)
//!   x1 = EL number the CPU was in at reset (captured before any drop)
//! No other incoming register state is assumed.
//!
//! Constraints: no allocator, no locks, no formatting heap, no panic
//! control flow, no x86 modules, no Limine, no kernel_init().

use super::registers;
use super::serial;

/// DTB magic (big-endian on disk): 0xD00DFEED.
const DTB_MAGIC: u32 = 0xD00D_FEED;

#[unsafe(no_mangle)]
pub extern "C" fn arm64_early_main(dtb: usize, entered_el: u8) -> ! {
    // 1. Entry banner: EL observed at reset (2 expected on QEMU virt,
    //    which boots kernels at EL2 when virtualization is enabled).
    serial::write_str("[sex.arm.entry] current_el=");
    serial::write_dec(entered_el as u64);
    serial::write_str("\n");

    // 2. Confirm we are now executing at EL1.
    let now_el = registers::current_el();
    if now_el == 1 {
        serial::write_str("[sex.arm.el1] ok=1\n");
    } else {
        serial::write_str("[sex.arm.el1] ok=0 el=");
        serial::write_dec(now_el as u64);
        serial::write_str("\n");
    }

    // 3. DTB pointer + bounded magic validation. RUNTIME FACT (2026-07-22,
    //    QEMU 11.0.2, both EL1-direct and EL2 boots, full 512 MiB guest-RAM
    //    scan via monitor pmemsave): for an ELF -kernel, QEMU starts the CPU
    //    at e_entry with x0=0 AND LOADS NO FDT INTO GUEST RAM AT ALL (the
    //    x0=FDT contract + FDT load apply to the raw Image boot path). The
    //    kernel platform is FIXED to QEMU virt (all addresses are constants),
    //    so the DTB is informational only: when x0==0, report present=0 and
    //    skip validation; when x0!=0 (alternate boot method), validate.
    let magic_ok = if dtb == 0 {
        serial::write_str("[sex.arm.dtb] x0=0 present=0 note=elf_boot_no_fdt\n");
        false
    } else {
        serial::write_str("[sex.arm.dtb] ptr=0x");
        serial::write_hex(dtb as u64);
        serial::write_str("\n");
        let ok = if (dtb & 7) == 0 {
            // SAFETY: single aligned volatile u32 read of the DTB header
            // (address came from x0 per the arm64 boot convention).
            let raw = unsafe { core::ptr::read_volatile(dtb as *const u32) };
            u32::from_be(raw) == DTB_MAGIC
        } else {
            false
        };
        if ok {
            serial::write_str("[sex.arm.dtb] magic=ok\n");
        } else {
            serial::write_str("[sex.arm.dtb] magic=bad\n");
        }
        ok
    };
    let _ = magic_ok;

    // 4. UART self-evident confirmation (this line itself proves PL011).
    serial::write_str("[sex.arm.uart] pl011=ok\n");

    // 5. Deterministic alive marker.
    serial::write_str("SEX_ARM_ALIVE\n");

    // ── Phase 3: MMU / single global VAS + EL0 smoke proof ────────────
    // 6. Pre-MMU: copy the EL0 smoke payloads into their pages and plant
    //    the HHDM probe magic (identity addressing still writable).
    unsafe { super::el0_smoke::prepare() };
    unsafe { super::phase5::prepare() };

    // 7. Build page tables and enable the MMU (prints tables=ok, on=1,
    //    wx=1, hhdm=ok; any failure prints FATAL and halts inside).
    super::mmu::init();
    serial::write_str("SEX_ARM_MMU_OK\n");

    // 8. Install the minimal Phase-3 vector table (EL0 sync warden).
    super::exceptions::init();

    // 9. Enter EL0 smoke payload. Never returns: the warden validates
    //    the four expected faults and halts with SEX_ARM_PHASE3_OK (or
    //    SEX_ARM_PHASE3_FAIL on any deviation).
    super::el0_smoke::enter();
}
