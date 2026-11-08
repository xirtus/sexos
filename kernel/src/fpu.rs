// SPDX-License-Identifier: GPL-3.0-only
//! Floating point and SSE for programs. The kernel itself is soft-float and never
//! touches x87, MMX or XMM state, so a task's registers stay in the CPU across every
//! kernel entry and are saved only when the core switches to another task: the
//! outgoing task's state goes to its own area before the task is released to other
//! cores, and the incoming task's comes back before it runs. A task never sees
//! another task's registers, whatever key or core either ran on.
//!
//! Only x87 and SSE (FXSAVE's 512 bytes) are enabled. XCR0 is held to those two, so
//! AVX and later register files stay off and can't carry state between tasks.
use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

/// One task's x87 and SSE registers, in FXSAVE layout.
#[repr(C, align(16))]
pub struct FpuState(pub [u8; 512]);

impl FpuState {
    /// What `fninit` and a reset MXCSR give: round to nearest, every exception masked,
    /// empty x87 stack, zeroed XMM registers.
    pub const INIT: FpuState = {
        let mut bytes = [0u8; 512];
        bytes[0] = 0x7F; // FCW = 0x037F
        bytes[1] = 0x03;
        bytes[24] = 0x80; // MXCSR = 0x1F80
        bytes[25] = 0x1F;
        bytes[28] = 0xFF; // MXCSR_MASK: informative only, fxrstor ignores it
        bytes[29] = 0xFF;
        FpuState(bytes)
    };
}

/// Turns on SSE for ring 3 on this core. BSP: before the other cores copy CR0/CR4/XCR0.
pub unsafe fn init_core() {
    let mut cr0 = Cr0::read();
    cr0.remove(Cr0Flags::EMULATE_COPROCESSOR | Cr0Flags::TASK_SWITCHED);
    cr0.insert(Cr0Flags::MONITOR_COPROCESSOR | Cr0Flags::NUMERIC_ERROR);
    Cr0::write(cr0);
    let mut cr4 = Cr4::read();
    cr4.insert(Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
    Cr4::write(cr4);
    if cr4.contains(Cr4Flags::OSXSAVE) {
        use x86_64::registers::xcontrol::{XCr0, XCr0Flags};
        XCr0::write(XCr0Flags::X87 | XCr0Flags::SSE);
    }
    core::arch::asm!("fninit", options(nostack, preserves_flags));
}

/// Saves this core's x87/SSE registers into `state`.
#[inline(always)]
pub unsafe fn save(state: *mut FpuState) {
    core::arch::asm!("fxsave64 [{}]", in(reg) state, options(nostack, preserves_flags));
}

/// Loads `state` into this core's x87/SSE registers.
#[inline(always)]
pub unsafe fn restore(state: *const FpuState) {
    core::arch::asm!("fxrstor64 [{}]", in(reg) state, options(nostack, preserves_flags, readonly));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn init_masks_every_exception() {
        let s = FpuState::INIT;
        assert_eq!(u16::from_le_bytes([s.0[0], s.0[1]]), 0x037F);
        assert_eq!(u32::from_le_bytes([s.0[24], s.0[25], s.0[26], s.0[27]]), 0x1F80);
        assert_eq!(core::mem::align_of::<FpuState>(), 16);
    }
}
