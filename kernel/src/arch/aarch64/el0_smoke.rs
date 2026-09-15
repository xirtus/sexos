//! Phase-3 EL0 smoke payload — DEVIATION from plan Phase 3 ("load one EL0
//! ELF (sexsh)"): no aarch64 userland backend exists (sexsh cannot build
//! for aarch64 yet — userland parity is a later userland phase per plan
//! table B), and the raw -kernel dev path has no initrd/module loader.
//! Instead: a hand-encoded A64 payload (every word documented below) runs
//! the exact same proofs the gate needs: EL0 execution works, EL0 read of
//! kernel VA sync-aborts, EL0 write to RO kernel text sync-aborts, EL0
//! execute from RW data sync-aborts (W^X at EL0 — x86 has no PKU-execute
//! equivalent), and a final deliberate fault hands control back to the
//! warden for the completion marker.
//!
//! Real ELF loading (sexsh via initrd/Limine-module path) remains
//! outstanding and is recorded as a deviation in the Phase-3 handoff.

use super::registers::*;
use super::serial;

/// Magic written pre-MMU at user_data_pa()+8, read back via HHDM.
pub const HHDM_PROBE_MAGIC: u64 = 0x5EED_1A11_7E57_0003;

/// Mailbox tags (payload writes tag to mailbox[0] before each fault).
pub const TAG_READ_KERNEL: u32 = 1;
pub const TAG_WRITE_RO: u32 = 2;
pub const TAG_EXEC_DATA: u32 = 3;
pub const TAG_DONE: u32 = 4;

/// One page, 4 KiB aligned.
#[repr(C, align(4096))]
struct Page([u8; 4096]);

// Smoke pages live in kernel .bss (no frame allocator yet — Phase 4/5
// concern). They are DOUBLE-MAPPED: EL1 RW via kernel identity VA, and
// EL0 via the user VAs below. Documented dev-path shortcut.
static mut USER_CODE_PAGE: Page = Page([0; 4096]);
static mut USER_DATA_PAGE: Page = Page([0; 4096]);
static mut USER_STACK_PAGE: Page = Page([0; 4096]);

/// The payload: A64 instructions (LE u32 words). Mailbox = USER_DATA_VA.
/// x9 = 0x410000 (mailbox), one MOVZ; kernel target = 0x40080000 (one
/// MOVZ); exec target = mailbox page (UXN=1); final target = unmapped
/// 0x50000000. Encoding verification: each word disassembles to the
/// commented instruction (llvm-objdump verified at build time by gate).
#[rustfmt::skip]
pub const PAYLOAD: &[u32] = &[
    0xD2A00829, // movz x9,  #0x41,   lsl #16   ; x9 = USER_DATA_VA (mailbox)
    0xD2800020, // movz x0,  #1                  ; tag = READ_KERNEL
    0xF9000120, // str  x0,  [x9]               ; mailbox[0] = tag
    0xD2A8010A, // movz x10, #0x4008, lsl #16   ; x10 = kernel text VA
    0xF9400141, // ldr  x1,  [x10]              ; ABORT: EL0 read of kernel
    0xD2800040, // movz x0,  #2                  ; tag = WRITE_RO
    0xF9000120, // str  x0,  [x9]
    0xF9000140, // str  x0,  [x10]              ; ABORT: EL0 write to RO text
    0xD2800060, // movz x0,  #3                  ; tag = EXEC_DATA
    0xF9000120, // str  x0,  [x9]
    0xD2A0082B, // movz x11, #0x41,   lsl #16   ; x11 = USER_DATA_VA
    0xD61F0160, // br   x11                     ; ABORT: exec from UXN data
    0xD2800080, // movz x0,  #4                  ; tag = DONE
    0xF9000120, // str  x0,  [x9]
    0xD2AA000C, // movz x12, #0x5000, lsl #16   ; x12 = unmapped user VA
    0xF9400181, // ldr  x1,  [x12]              ; ABORT: final deliberate fault
    0x14000000, // b    .                       ; safety net (unreachable)
];

/// Map the three user pages at their smoke VAs in the TTBR0 space.
/// SAFETY: called from mmu::init() after T0_ROOT is installed.
pub(crate) unsafe fn map_user_pages() {
    unsafe {
        let code_pa = &raw const USER_CODE_PAGE as u64;
        let data_pa = &raw const USER_DATA_PAGE as u64;
        let stack_pa = &raw const USER_STACK_PAGE as u64;
        // Code: RO both ELs, EL0-executable (UXN=0), EL1 never executes it.
        super::mmu::map_page_t0(
            USER_CODE_VA, code_pa,
            D_AF | D_SH_INNER | D_AP_RO_BOTH | D_PXN | ATTR_NORMAL,
        );
        // Data (mailbox): RW both ELs, never executable anywhere.
        super::mmu::map_page_t0(
            USER_DATA_VA, data_pa,
            D_AF | D_SH_INNER | D_AP_RW_BOTH | D_UXN | D_PXN | ATTR_NORMAL,
        );
        // Stack: RW both ELs, NX.
        super::mmu::map_page_t0(
            USER_STACK_VA, stack_pa,
            D_AF | D_SH_INNER | D_AP_RW_BOTH | D_UXN | D_PXN | ATTR_NORMAL,
        );
    }
}

/// Physical (== link/identity) address of the user data page.
pub fn user_data_pa() -> u64 {
    unsafe { &raw const USER_DATA_PAGE as u64 }
}

/// Pre-MMU setup: copy payload into the code page (identity addressing,
/// still writable) and plant the HHDM probe magic + cleared mailbox tag.
/// SAFETY: must run BEFORE mmu::init() flips permissions.
pub unsafe fn prepare() {
    unsafe {
        let dst = &raw mut USER_CODE_PAGE as *mut u32;
        for (i, w) in PAYLOAD.iter().enumerate() {
            dst.add(i).write_volatile(*w);
        }
        let mb = &raw mut USER_DATA_PAGE as *mut u64;
        mb.write_volatile(0); // tag = 0
        mb.add(1).write_volatile(HHDM_PROBE_MAGIC); // offset 8
    }
}

/// Read the mailbox tag (called by the exception warden; EL1 can access
/// the user page — same global VAS, AP permits EL1 RW).
pub fn mailbox_tag() -> u32 {
    unsafe { (&raw const USER_DATA_PAGE as *const u32).read_volatile() }
}

/// Read mailbox u64 slot `i` (0-based, i.e. byte offset i*8).
pub fn mailbox_slot(i: usize) -> u64 {
    unsafe { (&raw const USER_DATA_PAGE as *const u64).add(i).read_volatile() }
}

core::arch::global_asm!(
    ".text",
    ".balign 4",
    ".global sex_arm_el0_enter",
    "sex_arm_el0_enter:",
    // x0 = user entry VA, x1 = user stack top VA. Never returns via ret:
    // control comes back only through the exception warden.
    "msr ELR_EL1, x0",
    "msr SP_EL0, x1",
    "mov x2, xzr",          // SPSR: EL0t (M[3:0]=0000), DAIF unmasked
    "msr SPSR_EL1, x2",
    "isb",
    "eret",
);

unsafe extern "C" {
    /// Drop to EL0 at `entry` with user stack `sp`. Does not return.
    fn sex_arm_el0_enter(entry: u64, sp: u64) -> !;
}

/// Enter the EL0 smoke payload. Prints the entry marker first (EL0 cannot
/// print — UART is EL1-only; reaching the first warden fault proves EL0
/// execution happened).
pub fn enter() -> ! {
    serial::write_str("[sex.arm.el0] enter=1\n");
    enter_at(USER_CODE_VA)
}

/// Enter EL0 at an arbitrary prepared user code VA (shared stack page).
/// Diverges; control returns only via the exception warden.
pub fn enter_at(entry: u64) -> ! {
    unsafe {
        sex_arm_el0_enter(entry, USER_STACK_VA + 0x1000);
    }
}
