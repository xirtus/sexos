//! AArch64 exception support — Phase-3 minimal warden + Phase-4 expansion.
//!
//! Phase 3 (documented deviation): EL0 sync warden validates the smoke
//! payload's expected faults (tag + EC + FAR) and chains to Phase 4 on
//! the final tag.
//! Phase 4: IRQ-from-EL1h handler (timer tick → frame save → sched
//! switch → shared restore) and the EL1h sync kill-task containment
//! warden. All other vectors remain halt stubs (Phase 5 fills SVC).

use super::el0_smoke;
use super::registers::*;
use super::serial;

core::arch::global_asm!(
    ".text",
    ".balign 2048",
    ".global sex_arm_vectors",
    "sex_arm_vectors:",
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x000 sync SP0
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x080 irq  SP0
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x100 fiq  SP0
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x180 serr SP0
    "b sex_arm_el1_sync",        ".space 0x7C", // 0x200 sync EL1h
    "b sex_arm_irq_el1",         ".space 0x7C", // 0x280 irq  EL1h
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x300 fiq  EL1h
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x380 serr EL1h
    "b sex_arm_el0_sync",        ".space 0x7C", // 0x400 sync lower A64
    "b sex_arm_irq_el1",         ".space 0x7C", // 0x480 irq  lower A64
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x500 fiq  lower A64
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x580 serr lower A64
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x600 sync lower A32
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x680 irq  lower A32
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x700 fiq  lower A32
    "b sex_arm_halt_unexpected", ".space 0x7C", // 0x780 serr lower A32

    // ── Phase-3 EL0 sync warden (smoke payload) ──
    // (slot 0x400 pre-dispatch: EC==0x15 SVC64 goes to the Phase-5 SVC
    // path; everything else falls through to this warden. ESR is read
    // via x9 with save/restore — x9 is live in the payloads.)
    "sex_arm_el0_sync:",
    "sub sp, sp, #16",
    "str x9, [sp]",
    "mrs x9, ESR_EL1",
    "lsr x9, x9, #26",
    "and x9, x9, #0x3F",
    "cmp x9, #0x15",
    "ldr x9, [sp]",
    "add sp, sp, #16",
    "b.eq sex_arm_svc_path",
    "sub sp, sp, #160",
    "stp x0,  x1,  [sp, #0]",
    "stp x2,  x3,  [sp, #16]",
    "stp x4,  x5,  [sp, #32]",
    "stp x6,  x7,  [sp, #48]",
    "stp x8,  x9,  [sp, #64]",
    "stp x10, x11, [sp, #80]",
    "stp x12, x13, [sp, #96]",
    "stp x14, x15, [sp, #112]",
    "stp x30, xzr, [sp, #128]",
    "mrs x0, ESR_EL1",
    "mrs x1, FAR_EL1",
    "mrs x2, ELR_EL1",
    "bl sex_arm_rust_el0_warden",
    "cbz w0, .Lel0_skip",
    "cmp w0, #3",
    "b.eq .Lel0_skip_code",
    "cmp w0, #2",
    "b.eq .Lel0_chain_p4",
    "cmp w0, #4",
    "b.eq .Lel0_chain_p6",
    ".Lel0_halt:",
    "wfe",
    "b .Lel0_halt",
    ".Lel0_skip:",
    "mrs x0, ELR_EL1",
    "add x0, x0, #4",
    "msr ELR_EL1, x0",
    "b .Lel0_restore",
    // Tag-3 (exec-data) redirect: the faulting PC is ON the non-executable
    // data page, so ELR+4 would fault again (runtime-found 2026-07-22:
    // double insn-abort far=0x410004). Continue at the payload instruction
    // after `br x11` = USER_CODE_VA+0x30 (payload word 12; literal mirrors
    // registers::USER_CODE_VA + 12*4, gate-pinned).
    ".Lel0_skip_code:",
    "ldr x0, =0x400030",
    "msr ELR_EL1, x0",
    ".Lel0_restore:",
    "ldp x0,  x1,  [sp, #0]",
    "ldp x2,  x3,  [sp, #16]",
    "ldp x4,  x5,  [sp, #32]",
    "ldp x6,  x7,  [sp, #48]",
    "ldp x8,  x9,  [sp, #64]",
    "ldp x10, x11, [sp, #80]",
    "ldp x12, x13, [sp, #96]",
    "ldp x14, x15, [sp, #112]",
    "ldp x30, xzr, [sp, #128]",
    "add sp, sp, #160",
    "eret",
    // Chain: Phase 3 finished cleanly — ERET to arm64_phase4_entry at
    // EL1h, DAIF masked, boot stack (SP_EL1 already correct).
    ".Lel0_chain_p4:",
    "add sp, sp, #160",
    "ldr x0, =arm64_phase4_entry",
    "msr ELR_EL1, x0",
    "mov x1, #0x3C5",
    "msr SPSR_EL1, x1",
    "eret",
    // Chain: Phase 5 SVC test verified — ERET to arm64_phase6_entry.
    ".Lel0_chain_p6:",
    "add sp, sp, #160",
    "ldr x0, =arm64_phase6_entry",
    "msr ELR_EL1, x0",
    "mov x1, #0x3C5",
    "msr SPSR_EL1, x1",
    "eret",

    // ── Phase-5 SVC path (EC=0x15 from lower EL) ──
    // Full 272-byte frame; Rust writes the result into the frame's x0
    // slot; the frame's ELR is advanced past the svc instruction.
    "sex_arm_svc_path:",
    "sub sp, sp, #272",
    "stp x0,  x1,  [sp, #0]",
    "stp x2,  x3,  [sp, #16]",
    "stp x4,  x5,  [sp, #32]",
    "stp x6,  x7,  [sp, #48]",
    "stp x8,  x9,  [sp, #64]",
    "stp x10, x11, [sp, #80]",
    "stp x12, x13, [sp, #96]",
    "stp x14, x15, [sp, #112]",
    "stp x16, x17, [sp, #128]",
    "stp x18, x19, [sp, #144]",
    "stp x20, x21, [sp, #160]",
    "stp x22, x23, [sp, #176]",
    "stp x24, x25, [sp, #192]",
    "stp x26, x27, [sp, #208]",
    "stp x28, x29, [sp, #224]",
    "str x30, [sp, #240]",
    "mrs x9, ELR_EL1",
    "str x9, [sp, #248]",
    "mrs x9, SPSR_EL1",
    "str x9, [sp, #256]",
    "mov x0, sp",
    "bl sex_arm_svc",
    "mov sp, x0",
    // NO ELR advance here: for SVC (unlike aborts) ELR_EL1 already holds
    // the address of the instruction FOLLOWING the svc — runtime-found
    // 2026-07-22 (+4 skipped the result store word; mailbox slots read 0).
    "b sex_arm_restore_eret",

    // ── Phase-4 IRQ handler (EL1h, slot 0x280) ──
    // Full 272-byte frame, IAR read, tick switch, EOI, shared restore.
    "sex_arm_irq_el1:",
    "sub sp, sp, #272",
    "stp x0,  x1,  [sp, #0]",
    "stp x2,  x3,  [sp, #16]",
    "stp x4,  x5,  [sp, #32]",
    "stp x6,  x7,  [sp, #48]",
    "stp x8,  x9,  [sp, #64]",
    "stp x10, x11, [sp, #80]",
    "stp x12, x13, [sp, #96]",
    "stp x14, x15, [sp, #112]",
    "stp x16, x17, [sp, #128]",
    "stp x18, x19, [sp, #144]",
    "stp x20, x21, [sp, #160]",
    "stp x22, x23, [sp, #176]",
    "stp x24, x25, [sp, #192]",
    "stp x26, x27, [sp, #208]",
    "stp x28, x29, [sp, #224]",
    "str x30, [sp, #240]",
    "mrs x9, ELR_EL1",
    "str x9, [sp, #248]",
    "mrs x9, SPSR_EL1",
    "str x9, [sp, #256]",
    "mrs x19, ICC_IAR1_EL1",   // x19 = intid (frame already saved it)
    "cmp x19, #1023",
    "b.eq .Lirq_restore",      // spurious: no EOI, no tick
    "mov x0, sp",
    "mov x1, x19",
    "bl sex_arm_tick",          // -> x0 = frame to resume
    "mov sp, x0",
    "msr ICC_EOIR1_EL1, x19",  // EOI before restore (x19 callee-saved)
    ".Lirq_restore:",
    "b sex_arm_restore_eret",

    // ── Phase-4 EL1h sync: kill-task containment warden ──
    "sex_arm_el1_sync:",
    "sub sp, sp, #272",
    "stp x0,  x1,  [sp, #0]",
    "stp x2,  x3,  [sp, #16]",
    "stp x4,  x5,  [sp, #32]",
    "stp x6,  x7,  [sp, #48]",
    "stp x8,  x9,  [sp, #64]",
    "stp x10, x11, [sp, #80]",
    "stp x12, x13, [sp, #96]",
    "stp x14, x15, [sp, #112]",
    "stp x16, x17, [sp, #128]",
    "stp x18, x19, [sp, #144]",
    "stp x20, x21, [sp, #160]",
    "stp x22, x23, [sp, #176]",
    "stp x24, x25, [sp, #192]",
    "stp x26, x27, [sp, #208]",
    "stp x28, x29, [sp, #224]",
    "str x30, [sp, #240]",
    "mrs x9, ELR_EL1",
    "str x9, [sp, #248]",
    "mrs x9, SPSR_EL1",
    "str x9, [sp, #256]",
    "mrs x0, ESR_EL1",
    "mrs x1, FAR_EL1",
    "mrs x2, ELR_EL1",
    "mov x3, sp",
    "bl sex_arm_kill_warden",   // -> x0 = resume frame, or 0 = fatal
    "cbz x0, .Lel1_halt",
    "mov sp, x0",
    "b sex_arm_restore_eret",
    ".Lel1_halt:",
    "wfe",
    "b .Lel1_halt",

    // Any other vector: raw 'U' to PL011 (identity-mapped post-MMU).
    "sex_arm_halt_unexpected:",
    "mov x9, #0x0000",
    "movk x9, #0x0900, lsl #16",
    "mov w10, #0x55", // 'U'
    "str w10, [x9]",
    ".Lunexp_halt:",
    "wfe",
    "b .Lunexp_halt",
);

unsafe extern "C" {
    static sex_arm_vectors: u8;
}

/// Install the vector table. Called post-MMU on each core.
pub fn init() {
    let v = &raw const sex_arm_vectors as u64;
    unsafe {
        core::arch::asm!("msr VBAR_EL1, {v}", "isb", v = in(reg) v, options(nostack));
    }
    serial::write_str("[sex.arm.exc] vbar=ok\n");
}

// ESR_EL1.EC values (bits [31:26]).
const EC_INSN_ABORT_LOWER: u64 = 0x20;
const EC_DATA_ABORT_LOWER: u64 = 0x24;

/// EL0 sync warden. Returns 0 → skip faulting insn (ELR+4) + ERET;
/// 3 → redirect ELR to the code-page continuation (instruction-abort
/// skips); 2 → chain to Phase 4; 4 → chain to Phase 6; anything else →
/// halt.
#[unsafe(no_mangle)]
extern "C" fn sex_arm_rust_el0_warden(esr: u64, far: u64, elr: u64) -> u32 {
    let ec = (esr >> 26) & 0x3F;
    let tag = el0_smoke::mailbox_tag();
    let ok = match tag {
        el0_smoke::TAG_READ_KERNEL => {
            serial::write_str("[sex.arm.neg] el0_read_kernel=aborted\n");
            ec == EC_DATA_ABORT_LOWER && far == KERNEL_PHYS_BASE
        }
        el0_smoke::TAG_WRITE_RO => {
            serial::write_str("[sex.arm.neg] el0_write_ro=aborted\n");
            ec == EC_DATA_ABORT_LOWER && far == KERNEL_PHYS_BASE
        }
        el0_smoke::TAG_EXEC_DATA => {
            serial::write_str("[sex.arm.neg] el0_exec_data=aborted\n");
            if ec == EC_INSN_ABORT_LOWER && far == USER_DATA_VA {
                // Instruction abort: PC is on the NX data page. ELR+4
                // re-faults; redirect to the code-page continuation.
                return 3;
            }
            fail_detail(tag, ec, far, elr);
            return 1;
        }
        el0_smoke::TAG_DONE => {
            serial::write_str("[sex.arm.el0] done=1\n");
            if ec == EC_DATA_ABORT_LOWER && far == USER_UNMAPPED_VA {
                serial::write_str("SEX_ARM_PHASE3_OK\n");
                return 2; // chain to Phase 4
            }
            fail_detail(tag, ec, far, elr);
            return 1;
        }
        super::phase5::TAG_P5_DONE => {
            // Phase-5 completion: verify the three SVC results the
            // payload stored in the mailbox, plus the expected final
            // fault shape.
            let echo = el0_smoke::mailbox_slot(2);
            let cap_bad = el0_smoke::mailbox_slot(3);
            let cap_good = el0_smoke::mailbox_slot(4);
            serial::write_str("[sex.arm.el0] done2=1\n");
            if ec == EC_DATA_ABORT_LOWER && far == USER_UNMAPPED2_VA
                && echo == 42
                && cap_bad == ERR_CAP_INVALID
                && cap_good == 0
            {
                serial::write_str("[sex.arm.svc] echo=ok\n");
                serial::write_str("[sex.arm.svc] cap_bad=ERR_CAP_INVALID\n");
                serial::write_str("[sex.arm.svc] cap_good=ok\n");
                serial::write_str("SEX_ARM_PHASE5_OK\n");
                return 4; // chain to Phase 6
            }
            fail_detail(tag, ec, far, elr);
            serial::write_str("[sex.arm.svc] echo=0x");
            serial::write_hex(echo);
            serial::write_str(" cap_bad=0x");
            serial::write_hex(cap_bad);
            serial::write_str(" cap_good=0x");
            serial::write_hex(cap_good);
            serial::write_str("\nSEX_ARM_PHASE5_FAIL\n");
            return 1;
        }
        _ => {
            serial::write_str("[sex.arm.neg] unexpected=1\n");
            fail_detail(tag, ec, far, elr);
            return 1;
        }
    };
    if ok {
        0
    } else {
        fail_detail(tag, ec, far, elr);
        1
    }
}

fn fail_detail(tag: u32, ec: u64, far: u64, elr: u64) {
    serial::write_str("[sex.arm.neg] MISMATCH tag=");
    serial::write_dec(tag as u64);
    serial::write_str(" ec=0x");
    serial::write_hex(ec);
    serial::write_str(" far=0x");
    serial::write_hex(far);
    serial::write_str(" elr=0x");
    serial::write_hex(elr);
    serial::write_str("\nSEX_ARM_PHASE3_FAIL\n");
}
