//! Phase 5 — generic SVC call mechanism on AArch64 (NO POE, NO PDX-PARITY
//! CLAIMS). Plan Phase 5's gate (sexsh→sexfiles PDX round trip) cannot run:
//! no aarch64 userland backend exists (recorded deviation). This module
//! proves the MECHANISM PDX needs, with the plan's negative gate intact:
//!   1. EL0 → SVC → EL1 → return-to-EL0 round trip with args (x0-x1) and
//!      result (x0), using the full 272-byte frame save/restore.
//!   2. bad cap id → ERR_CAP_INVALID — the EXACT sex-pdx ABI constant
//!      (0xFFFF_FFFF_FFFF_FFFC, mirrored in registers.rs, gate-verified
//!      against crates/sex-pdx/src/lib.rs every gate run).
//!   3. good cap id → 0.
//! Results are checked by the EL0 warden (tag 5) from the mailbox and
//! printed as markers. The real PDX port (capability.rs/ipc.rs compile
//! for aarch64 + userland PDs) is a later phase; protection.rs stays dead.

use super::registers::*;
use super::serial;

/// Mailbox tag signaling "Phase-5 sequence done" (final fault follows).
pub const TAG_P5_DONE: u32 = 5;

#[repr(C, align(4096))]
struct Page([u8; 4096]);
static mut USER_CODE2_PAGE: Page = Page([0; 4096]);

/// The SVC test payload (A64, LE words; each verified against clang
/// assembler output by the phase-5 gate).
/// x9 = mailbox (USER_DATA_VA). Protocol: run 3 SVC calls, store each
/// result in the mailbox, then tag + deliberate final fault.
#[rustfmt::skip]
pub const PAYLOAD2: &[u32] = &[
    0xD2A00829, // movz x9,  #0x41,   lsl #16  ; x9 = mailbox
    0xD2800020, // movz x0,  #1                ; SEL_ECHO
    0xD2800521, // movz x1,  #41               ; arg = 41
    0xD4000001, // svc  #0                     ; -> x0 = 42
    0xF9000920, // str  x0,  [x9, #16]         ; mailbox[2] = echo result
    0xD2800040, // movz x0,  #2                ; SEL_CAP
    0xD29BD5A1, // movz x1,  #0xDEAD           ; bad cap id
    0xD4000001, // svc  #0                     ; -> x0 = ERR_CAP_INVALID
    0xF9000D20, // str  x0,  [x9, #24]         ; mailbox[3]
    0xD2800040, // movz x0,  #2                ; SEL_CAP
    0xD2800541, // movz x1,  #42               ; VALID_TEST_CAP
    0xD4000001, // svc  #0                     ; -> x0 = 0
    0xF9001120, // str  x0,  [x9, #32]         ; mailbox[4]
    0xD28000A0, // movz x0,  #5                ; tag = P5_DONE
    0xF9000120, // str  x0,  [x9]
    0xD2AC000C, // movz x12, #0x6000, lsl #16  ; x12 = 0x60000000
    0xF9400181, // ldr  x1,  [x12]             ; ABORT: final deliberate fault
    0x14000000, // b    .                      ; unreachable
];

/// Pre-MMU copy (same constraint as the Phase-3 payload: the page becomes
/// RO for EL1 too once mapped). Called from early.rs alongside prepare().
/// SAFETY: pre-MMU only.
pub unsafe fn prepare() {
    unsafe {
        let dst = &raw mut USER_CODE2_PAGE as *mut u32;
        for (i, w) in PAYLOAD2.iter().enumerate() {
            dst.add(i).write_volatile(*w);
        }
    }
}

/// Map the phase-5 code page (RX, user). Reuses the phase-3 mailbox/stack.
/// SAFETY: called from mmu::init() after T0_ROOT install.
pub(crate) unsafe fn map_user_pages() {
    unsafe {
        super::mmu::map_page_t0(
            USER_CODE2_VA, &raw const USER_CODE2_PAGE as u64,
            super::registers::D_AF | super::registers::D_SH_INNER
                | super::registers::D_AP_RO_BOTH | super::registers::D_PXN
                | super::registers::ATTR_NORMAL,
        );
    }
}

/// SVC dispatch — called from the exception handler with the full frame.
/// x0 = selector, x1 = arg; result written back to the frame's x0 slot.
/// Returns the same frame (signature matches the switch convention).
#[unsafe(no_mangle)]
extern "C" fn sex_arm_svc(frame: u64) -> u64 {
    unsafe {
        let f = frame as *mut u64;
        let sel = f.add(0).read_volatile();
        let arg = f.add(1).read_volatile();
        let result = match sel {
            SVC_SEL_ECHO => arg.wrapping_add(1),
            SVC_SEL_CAP => {
                if arg == VALID_TEST_CAP { 0 } else { ERR_CAP_INVALID }
            }
            _ => ERR_CAP_INVALID, // unknown selector: same error class
        };
        f.add(0).write_volatile(result);
        frame
    }
}

/// Phase-5 entry: called from task A after SEX_ARM_PHASE4_OK. Disables
/// the timer (no further scheduling; the SVC test is uninterruptible),
/// then drops to EL0 with the test payload. Diverges — the warden's
/// final-tag path chains to Phase 6.
pub fn run() -> ! {
    serial::write_str("[sex.arm.p5] entry=1\n");
    super::timer::disable();
    serial::write_str("[sex.arm.svc] test=echo,cap_bad,cap_good\n");
    serial::write_str("[sex.arm.el0] enter2=1\n");
    super::el0_smoke::enter_at(USER_CODE2_VA)
}
