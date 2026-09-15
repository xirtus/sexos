//! Sex Microkernel — AArch64 Phase 2 image (entry + serial).
//!
//! This is a SEPARATE cargo bin target (`sex-kernel-arm64`, feature-gated
//! by `arm64` so x86 builds skip it). The x86 `src/main.rs` is untouched.
//!
//! Boot contract (QEMU virt, -kernel):
//!   x0 = physical address of the DTB. CPU enters at EL2 (default when
//!   virtualization is enabled) or EL1. No other register state, no
//!   valid stack, MMU off.
//!
//! Exit markers (ordered): see docs/handoff/ARM64_PHASE_2_ENTRY_SERIAL_V1.md

#![no_std]
#![no_main]

#[cfg(not(target_arch = "aarch64"))]
compile_error!("sex-kernel-arm64 is an AArch64-only image");

// ── Reset entry ─────────────────────────────────────────────────────────────
// Kept as ONE bounded global_asm block (global_asm cannot reference Rust
// constants/symbols except by name — documented limitation; the SCTLR/SPSR
// literals below mirror registers::SCTLR_EL1_SAFE / SPSR_EL1H_DAIF /
// HCR_EL2_RW and are cross-checked by the phase gate).
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
    .section .text.boot, "ax"
    .global _start
    .type _start, %function
_start:
    // Preserve DTB pointer (x0) in callee-saved x20.
    mov     x20, x0
    // Capture ENTERED exception level (CurrentEL[3:2]) in x21.
    mrs     x21, CurrentEL
    lsr     x21, x21, #2
    and     x21, x21, #0x3

    cmp     x21, #2
    b.eq    .Lel2_to_el1
    cmp     x21, #1
    b.eq    .Lin_el1
    b       .Lunexpected_el

.Lel2_to_el1:
    // HCR_EL2.RW = 1: EL1 executes AArch64.
    mov     x9, #(1 << 31)
    msr     hcr_el2, x9
    // CNTHCTL_EL2 = 3: EL1 may access CNTP timer + counter (Phase 4;
    // reset value would otherwise be implementation-defined).
    mov     x9, #3
    msr     cnthctl_el2, x9
    // SCTLR_EL1 = safe reset state: MMU off, caches off, RES1 bits
    // (0x30D00800 == registers::SCTLR_EL1_SAFE).
    ldr     x9, =0x30D00800
    msr     sctlr_el1, x9
    // CPACR_EL1 = 0: FP/SIMD trapped at EL0/EL1 (softfloat kernel).
    msr     cpacr_el1, xzr
    // SPSR_EL2 = 0x3C5: return to EL1h (M=0101), DAIF masked.
    ldr     x9, =0x3C5
    msr     spsr_el2, x9
    // ELR_EL2 = EL1 continuation.
    adr     x9, .Lin_el1
    msr     elr_el2, x9
    // SP_EL1 = boot stack top.
    adrp    x9, __boot_stack_top
    add     x9, x9, :lo12:__boot_stack_top
    msr     sp_el1, x9
    eret

.Lin_el1:
    // Install the boot stack on the CURRENT stack pointer (SP_EL1 when
    // EL1h, which is the mode both paths run in).
    adrp    x9, __boot_stack_top
    add     x9, x9, :lo12:__boot_stack_top
    mov     sp, x9

    // Clear .bss (span includes the boot stack; no live stack data yet).
    adrp    x10, __bss_start
    add     x10, x10, :lo12:__bss_start
    adrp    x11, __bss_end
    add     x11, x11, :lo12:__bss_end
.Lbss_clear:
    cmp     x10, x11
    b.hs    .Lbss_done
    str     xzr, [x10], #8
    b       .Lbss_clear
.Lbss_done:

    // Call Rust: x0 = DTB, x1 = entered EL.
    mov     x0, x20
    mov     x1, x21
    bl      arm64_early_main

.Lhalt:
    wfe
    b       .Lhalt

.Lunexpected_el:
    // EL0/EL3 at reset: no safe continuation. Best-effort 'E' on PL011
    // (base fixed to QEMU virt, MMIO is Device-nGnRnE with MMU off),
    // then halt.
    mov     x9, #0x09000000
    mov     w10, #'E'
    str     w10, [x9]
    b       .Lhalt
.size _start, . - _start
"#
);

// ── Panic path (AArch64 image only; x86 panic handler in src/main.rs is
// untouched) ─────────────────────────────────────────────────────────────────
#[cfg(target_arch = "aarch64")]
#[panic_handler]
fn arm64_panic(_info: &core::panic::PanicInfo) -> ! {
    sex_kernel::arch::aarch64::serial::write_str("[sex.arm.panic]\n");
    loop {
        // SAFETY: WFE is a hint instruction; safe at EL1 with no setup.
        unsafe { core::arch::asm!("wfe", options(nomem, nostack)) }
    }
}
