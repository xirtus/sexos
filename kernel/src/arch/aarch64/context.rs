//! AArch64 task context + switch machinery — Phase 4.
//!
//! Frame layout (272 bytes, built on the interrupted/current stack by the
//! exception handlers; initial frames are synthesized the same way):
//!   [0..239]  x0-x29 (15 stp pairs)
//!   [240]     x30
//!   [248]     ELR_EL1  (resume PC)
//!   [256]     SPSR_EL1
//!   size 272 (16-byte aligned)
//! The switch is "ERET-based": handlers save a frame, the scheduler
//! returns the next frame pointer, a shared restore tail loads it.
//! This mirrors the x86 TaskContext-switch contract, NOT its layout
//! (audit: per-arch layout, do NOT unify).

use super::registers::SPSR_EL1H_TASK;

pub const FRAME_SIZE: usize = 272;
pub const OFF_ELR: usize = 248;
pub const OFF_SPSR: usize = 256;

/// Per-task kernel stacks (static, no heap at this phase).
#[repr(align(16))]
pub struct TaskStack([u8; 16384]);
pub static mut STACK_A: TaskStack = TaskStack([0; 16384]);
pub static mut STACK_B: TaskStack = TaskStack([0; 16384]);

/// Synthesize an initial frame at the top of `stack` for `entry`.
/// Returns the frame pointer (to hand to the restore path).
/// SAFETY: boot-time, single core, stacks are static.
pub unsafe fn build_frame(stack: *mut TaskStack, entry: u64) -> u64 {
    unsafe {
        let top = (stack as u64) + 16384;
        let frame = top - FRAME_SIZE as u64;
        let p = frame as *mut u64;
        for i in 0..(FRAME_SIZE / 8) {
            p.add(i).write_volatile(0);
        }
        p.add(OFF_ELR / 8).write_volatile(entry);
        p.add(OFF_SPSR / 8).write_volatile(SPSR_EL1H_TASK);
        frame
    }
}

core::arch::global_asm!(
    ".text",
    ".balign 4",
    // Shared restore tail: expects SP = frame. Loads SPSR/ELR from the
    // frame (scratch x9 clobbered BEFORE restoring x9 from the frame),
    // restores all GPRs, ERETs.
    ".global sex_arm_restore_eret",
    "sex_arm_restore_eret:",
    "ldr x9, [sp, #256]",
    "msr SPSR_EL1, x9",
    "ldr x9, [sp, #248]",
    "msr ELR_EL1, x9",
    "ldp x0,  x1,  [sp, #0]",
    "ldp x2,  x3,  [sp, #16]",
    "ldp x4,  x5,  [sp, #32]",
    "ldp x6,  x7,  [sp, #48]",
    "ldp x8,  x9,  [sp, #64]",
    "ldp x10, x11, [sp, #80]",
    "ldp x12, x13, [sp, #96]",
    "ldp x14, x15, [sp, #112]",
    "ldp x16, x17, [sp, #128]",
    "ldp x18, x19, [sp, #144]",
    "ldp x20, x21, [sp, #160]",
    "ldp x22, x23, [sp, #176]",
    "ldp x24, x25, [sp, #192]",
    "ldp x26, x27, [sp, #208]",
    "ldp x28, x29, [sp, #224]",
    "ldr x30, [sp, #240]",
    "add sp, sp, #272",
    "eret",

    // First entry into a task: x0 = frame. Never returns.
    ".global sex_arm_first",
    "sex_arm_first:",
    "mov sp, x0",
    "b sex_arm_restore_eret",
);

unsafe extern "C" {
    /// Enter the first scheduled task (frame in x0). Diverges.
    pub fn sex_arm_first(frame: u64) -> !;
}
