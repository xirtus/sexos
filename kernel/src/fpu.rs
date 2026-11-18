// SPDX-License-Identifier: GPL-3.0-only
//! Floating point, SSE and AVX for programs. The kernel itself is soft-float and never
//! touches x87, MMX, XMM, YMM or ZMM state, so a task's registers stay in the CPU across
//! every kernel entry and are saved only when the core switches to another task: the
//! outgoing task's state goes to its own area before the task is released to other
//! cores, and the incoming task's comes back before it runs. A task never sees
//! another task's registers, whatever key or core either ran on.
//!
//! XCR0 holds x87 and SSE, plus AVX (YMM upper halves) where the CPU has it, plus the
//! three AVX-512 components (opmask, ZMM_Hi256, Hi16_ZMM) where it has those. PKRU
//! (XCR0 bit 9) is never enabled, so no XRSTOR, the kernel's or a program's, can load a
//! protection-key register; the loader's XRSTOR ban stays as the second wall.
//! Each task's save area is its own 64-byte-aligned heap block, sized from CPUID for
//! the enabled set, so `Task` stays small: it is built on a program's 16 KiB kernel
//! stack (worker spawn), and an inline 2.7 KiB area once grew that path enough to
//! overrun a stack (PR #427).
//! Saving uses XSAVEC (compacted, skips registers still in their reset state), else
//! XSAVEOPT, else XSAVE, else FXSAVE on CPUs without XSAVE. XSAVEOPT's skip of registers
//! unchanged since the last XRSTOR from the same address is safe here: a task's area
//! is written by the core holding its registers or reset while no core holds them
//! (the scheduler then forgets which task each core has loaded).
use core::arch::x86_64::__cpuid_count as cpuid;
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

/// Most bytes any enabled set may need: x87/SSE legacy area, XSAVE header, AVX, (MPX
/// gap), opmask, ZMM_Hi256, Hi16_ZMM, in standard layout. Compacted is no larger.
pub const AREA_MAX: usize = 2688;
const X87_SSE: u64 = 0b11;
const AVX: u64 = 1 << 2;
const AVX512: u64 = 0b111 << 5;

/// One task's register state: a heap block in FXSAVE layout followed by the XSAVE
/// header and the extended components, 64-byte aligned for XSAVE.
pub struct FpuState { area: core::ptr::NonNull<u8>, bytes: u32 }
// The block belongs to one task; the scheduler's custody protocol orders access.
unsafe impl Send for FpuState {}
unsafe impl Sync for FpuState {}


impl FpuState {
    /// A fresh area in the reset state. Allocates: never on the switch path.
    pub fn new() -> FpuState {
        let bytes = SIZE.load(Ordering::Relaxed);
        let p = unsafe { alloc::alloc::alloc(Self::layout(bytes)) };
        let area = core::ptr::NonNull::new(p).unwrap_or_else(|| alloc::alloc::handle_alloc_error(Self::layout(bytes)));
        let mut s = FpuState { area, bytes };
        s.reset();
        s
    }
    /// What `fninit` and a reset MXCSR give: round to nearest, every exception masked,
    /// empty x87 stack, zeroed vector registers. Standard form (XCOMP_BV = 0), x87 and
    /// SSE marked present so XRSTOR takes FCW/MXCSR from here, everything else reset.
    pub fn reset(&mut self) {
        init_bytes(unsafe { core::slice::from_raw_parts_mut(self.area.as_ptr(), self.bytes as usize) });
    }
    fn layout(bytes: u32) -> core::alloc::Layout { core::alloc::Layout::from_size_align(bytes as usize, 64).unwrap() }
}

impl Drop for FpuState {
    fn drop(&mut self) { unsafe { alloc::alloc::dealloc(self.area.as_ptr(), Self::layout(self.bytes)) } }
}

fn init_bytes(bytes: &mut [u8]) {
    bytes.fill(0);
    bytes[0] = 0x7F; // FCW = 0x037F
    bytes[1] = 0x03;
    bytes[24] = 0x80; // MXCSR = 0x1F80
    bytes[25] = 0x1F;
    bytes[28] = 0xFF; // MXCSR_MASK: informative only, the restores ignore it
    bytes[29] = 0xFF;
    if bytes.len() > 512 { bytes[512] = X87_SSE as u8; } // XSTATE_BV
}

const FXSAVE: u8 = 0;
const XSAVE: u8 = 1;
const XSAVEOPT: u8 = 2;
const XSAVEC: u8 = 3;
static MODE: AtomicU8 = AtomicU8::new(FXSAVE);
static XCR0_BITS: AtomicU64 = AtomicU64::new(X87_SSE);
/// Bytes per save area; the largest possible until `init_core` measures it, so an area
/// made earlier is never too small.
static SIZE: AtomicU32 = AtomicU32::new(AREA_MAX as u32);

/// Turns on x87/SSE/AVX for ring 3 on this core. BSP: before the other cores copy
/// CR0/CR4/XCR0 (they copy XCR0 as written here, so every core saves the same set).
pub unsafe fn init_core() {
    let mut cr0 = Cr0::read();
    cr0.remove(Cr0Flags::EMULATE_COPROCESSOR | Cr0Flags::TASK_SWITCHED);
    cr0.insert(Cr0Flags::MONITOR_COPROCESSOR | Cr0Flags::NUMERIC_ERROR);
    Cr0::write(cr0);
    let mut cr4 = Cr4::read();
    cr4.insert(Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
    let leaf1 = cpuid(1, 0).ecx;
    if leaf1 & (1 << 26) != 0 && cpuid(0, 0).eax >= 0xD { cr4.insert(Cr4Flags::OSXSAVE); }
    Cr4::write(cr4);
    if cr4.contains(Cr4Flags::OSXSAVE) {
        let offered = { let l = cpuid(0xD, 0); l.eax as u64 | (l.edx as u64) << 32 };
        let mut xcr0 = X87_SSE;
        if leaf1 & (1 << 28) != 0 && offered & AVX != 0 { xcr0 |= AVX; }
        if xcr0 & AVX != 0 && cpuid(7, 0).ebx & (1 << 16) != 0 && offered & AVX512 == AVX512 { xcr0 |= AVX512; }
        x86_64::registers::xcontrol::XCr0::write_raw(xcr0);
        let saves = cpuid(0xD, 1).eax;
        let (standard, compacted) = (cpuid(0xD, 0).ebx, cpuid(0xD, 1).ebx);
        if xcr0 & AVX512 != 0 && standard.max(compacted) as usize > AREA_MAX {
            xcr0 &= !AVX512; // A layout we did not plan for: keep AVX, drop AVX-512.
            x86_64::registers::xcontrol::XCr0::write_raw(xcr0);
        }
        if (cpuid(0xD, 0).ebx as usize) > AREA_MAX { xcr0 = X87_SSE; x86_64::registers::xcontrol::XCr0::write_raw(xcr0); }
        let mode = if saves & 2 != 0 { XSAVEC } else if saves & 1 != 0 { XSAVEOPT } else { XSAVE };
        // Standard size covers both forms (XRSTOR reads whichever XCOMP_BV says).
        let size = cpuid(0xD, 0).ebx.max(cpuid(0xD, 1).ebx).next_multiple_of(64).max(576);
        MODE.store(mode, Ordering::Relaxed);
        XCR0_BITS.store(xcr0, Ordering::Relaxed);
        SIZE.store(size, Ordering::Relaxed);
    }
    core::arch::asm!("fninit", options(nostack, preserves_flags));
}

/// For the boot record: enabled register files, save instruction, bytes per save.
pub fn describe() -> (&'static str, &'static str, u32) {
    let bits = XCR0_BITS.load(Ordering::Relaxed);
    let files = if bits & AVX512 != 0 { "x87+sse+avx+avx512" } else if bits & AVX != 0 { "x87+sse+avx" } else { "x87+sse" };
    let mode = ["fxsave", "xsave", "xsaveopt", "xsavec"][MODE.load(Ordering::Relaxed) as usize];
    (files, mode, SIZE.load(Ordering::Relaxed))
}

/// Saves this core's register state into `state` (every enabled component).
#[inline(always)]
pub unsafe fn save(state: *mut FpuState) {
    let (state, all) = ((*state).area.as_ptr(), u32::MAX);
    match MODE.load(Ordering::Relaxed) {
        XSAVEC => core::arch::asm!("xsavec64 [{}]", in(reg) state, in("eax") all, in("edx") all, options(nostack, preserves_flags)),
        XSAVEOPT => core::arch::asm!("xsaveopt64 [{}]", in(reg) state, in("eax") all, in("edx") all, options(nostack, preserves_flags)),
        XSAVE => core::arch::asm!("xsave64 [{}]", in(reg) state, in("eax") all, in("edx") all, options(nostack, preserves_flags)),
        _ => core::arch::asm!("fxsave64 [{}]", in(reg) state, options(nostack, preserves_flags)),
    }
}

/// Loads `state` into this core's registers. Takes either XSAVE form, by XCOMP_BV.
#[inline(always)]
pub unsafe fn restore(state: *const FpuState) {
    let (state, all) = ((*state).area.as_ptr() as *const u8, u32::MAX);
    if MODE.load(Ordering::Relaxed) == FXSAVE {
        core::arch::asm!("fxrstor64 [{}]", in(reg) state, options(nostack, preserves_flags, readonly));
    } else {
        core::arch::asm!("xrstor64 [{}]", in(reg) state, in("eax") all, in("edx") all, options(nostack, preserves_flags, readonly));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn init_masks_every_exception() {
        let mut s = [0xAAu8; 832];
        init_bytes(&mut s);
        assert_eq!(u16::from_le_bytes([s[0], s[1]]), 0x037F);
        assert_eq!(u32::from_le_bytes([s[24], s[25], s[26], s[27]]), 0x1F80);
        // Standard form, x87+SSE present, reserved header bytes zero (XRSTOR #GPs otherwise).
        assert_eq!(s[512], 0b11);
        assert!(s[513..576].iter().all(|&b| b == 0));
    }
    #[test]
    fn area_max_fits_avx512_standard_layout() {
        // Hi16_ZMM is component 7 at standard offset 1664, 1024 bytes; PKRU (2688) is excluded.
        assert_eq!(AREA_MAX, 1664 + 1024);
        assert_eq!(core::mem::size_of::<FpuState>(), 16);
    }
}
