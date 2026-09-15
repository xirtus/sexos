//! AArch64 system-register constants and minimal accessors.
//!
//! All architecture-specific hex values live HERE (single source of
//! truth). Do not scatter literals across other modules.

/// PL011 UART0 base — QEMU `virt` machine. Phase 2 is explicitly
/// platform-fixed to QEMU virt; DTB-driven discovery is a later phase.
pub const PL011_BASE: usize = 0x0900_0000;

/// Safe reset-state SCTLR_EL1 used by the EL2→EL1 transition in entry
/// assembly: MMU off (M=0), caches off (I/C=0), alignment check off,
/// plus the architecturally-required RES1 bits for Armv8.0
/// (0x30D00800). Kept in sync with the literal in the entry global_asm
/// (global_asm cannot reference Rust constants — documented limitation).
pub const SCTLR_EL1_SAFE: u64 = 0x30D00800;

/// SPSR value for ERET to EL1h with DAIF masked:
/// M[3:0]=0b0101 (EL1h, SP_EL1), D/A/I/F[9:6]=0b1111 (masked).
pub const SPSR_EL1H_DAIF: u64 = 0x3C5;

/// HCR_EL2.RW: EL1 Execution state is AArch64.
pub const HCR_EL2_RW: u64 = 1 << 31;

/// Read CurrentEL (bits [3:2]) and return the EL number (0..3).
#[inline(always)]
pub fn current_el() -> u8 {
    let v: u64;
    // SAFETY: MRS from CurrentEL is readable at EL1+ and has no side
    // effects. Phase 2 always runs at EL1 when this is called.
    unsafe {
        core::arch::asm!("mrs {}, CurrentEL", out(reg) v, options(nomem, nostack));
    }
    ((v >> 2) & 0b11) as u8
}

// ── Phase 3: MMU / global VAS constants ────────────────────────────────────
// Design: ONE global address space (Sex SAS model, mirrors x86 single CR3).
//   TTBR0_EL1 = lower 48-bit: kernel image (EL1-only) + MMIO window +
//               user mappings (all PDs together — never switched).
//   TTBR1_EL1 = upper 48-bit: HHDM window (all RAM + device MMIO alias),
//               EL1-only.
// NO per-PD TTBR (plan Phase-3 STOP FIRST condition is NOT triggered).
// POE/TCR2/POR registers are NOT touched here (Phase 6+).

/// HHDM base in the TTBR1 (upper) half. bits[63:48]=0xFFFF with T1SZ=16.
pub const HHDM_BASE: u64 = 0xFFFF_8000_0000_0000;

/// MAIR_EL1: index 0 = Device-nGnRnE (0x00), index 1 = Normal WB RA WA (0xFF).
pub const MAIR_EL1_V: u64 = 0x0000_0000_0000_FF00;
pub const ATTR_DEVICE: u64 = 0 << 2;
pub const ATTR_NORMAL: u64 = 1 << 2;

/// TCR_EL1: T0SZ=16, T1SZ=16 (48-bit each half), TG0=4K, TG1=4K,
/// IRGN/ORGN=WB RA WA both, SH=inner both, IPS=40-bit (1 TiB PA), A1=0.
pub const TCR_EL1_V: u64 = 0x0000_0002_B510_3510;

/// SCTLR_EL1 with MMU on: SAFE base | M | C | I | WXN | SA.
pub const SCTLR_EL1_MMU: u64 = SCTLR_EL1_SAFE | 0x1 | 0x4 | 0x8 | (1 << 12) | (1 << 19);

// Descriptor attribute bits (block/page).
pub const D_VALID_TABLE: u64 = 0b11; // valid table or L3 page
pub const D_VALID_BLOCK: u64 = 0b01; // L1/L2 block
pub const D_AF: u64 = 1 << 10; // access flag (no AF-fault handling)
pub const D_SH_INNER: u64 = 3 << 8;
pub const D_AP_RW_EL1: u64 = 0b00 << 6;
pub const D_AP_RW_BOTH: u64 = 0b01 << 6;
pub const D_AP_RO_EL1: u64 = 0b10 << 6;
pub const D_AP_RO_BOTH: u64 = 0b11 << 6;
pub const D_UXN: u64 = 1 << 54;
pub const D_PXN: u64 = 1 << 53;

/// QEMU virt DRAM start (physical).
pub const DRAM_BASE: u64 = 0x4000_0000;
/// QEMU virt: first 1 GiB (MMIO: GIC 0x08000000, UART 0x09000000, virtio…)
/// is device; DRAM [0x40000000, 0x80000000) covered by one 1 GiB block
/// (platform-fixed; DTB memory parsing is Phase 4+).
pub const MMIO_BLOCK_BASE: u64 = 0x0000_0000;

/// User smoke-payload VAs (chosen to encode in single MOVZ, documented in
/// el0_smoke.rs). NOT final userland layout — Phase 3 smoke only.
pub const USER_CODE_VA: u64 = 0x0040_0000;
pub const USER_DATA_VA: u64 = 0x0041_0000; // mailbox lives here
pub const USER_STACK_VA: u64 = 0x0042_0000;
/// Deliberate final unmapped user VA for the smoke payload's last fault.
pub const USER_UNMAPPED_VA: u64 = 0x5000_0000;

/// Kernel link/load base (mirrors aarch64-linker.ld PHYS_BASE).
pub const KERNEL_PHYS_BASE: u64 = 0x4008_0000;

// ── Phase 4: exceptions / GICv3 / timer / scheduler constants ──────────────

/// SPSR for kernel tasks: EL1h (M[3:0]=0101), D/A/F masked, IRQ UNMASKED.
pub const SPSR_EL1H_TASK: u64 = 0x345;

/// QEMU virt GICv3 (physical; access via HHDM alias).
pub const GICD_PA: u64 = 0x0800_0000;
pub const GICR_PA: u64 = 0x080A_0000; // core 0 redistributor (RD+SGI: 128 KiB)
pub const GICR_STRIDE: u64 = 0x2_0000; // per-core redistributor stride
pub const GICR_SGI_OFF: u64 = 0x1_0000; // SGI/PPI frame within a redistributor

/// Non-secure physical timer (CNTP) PPI.
pub const INTID_TIMER: u32 = 30;
/// Spurious INTID.
pub const INTID_SPURIOUS: u32 = 1023;

/// Timer tick rate for Phase 4.
pub const TIMER_HZ: u64 = 100;

/// PSCI (SMC64 function IDs via HVC conduit on QEMU virt).
pub const PSCI_CPU_ON: u64 = 0xC400_0003;
pub const AP_MPIDR: u64 = 1; // core 1

/// PSTATE.DAIF set/clr values.
pub const DAIF_ALL: u64 = 0xF;

#[inline(always)]
pub fn dsb_sy() { unsafe { core::arch::asm!("dsb sy", options(nostack)) } }
#[inline(always)]
pub fn isb() { unsafe { core::arch::asm!("isb", options(nostack)) } }
#[inline(always)]
pub fn wfe() { unsafe { core::arch::asm!("wfe", options(nomem, nostack)) } }

// ── Phase 5: SVC / PDX-mechanism constants ─────────────────────────────────

/// Mirrors the sex-pdx ABI constant EXACTLY (crates/sex-pdx/src/lib.rs:684).
/// sex-pdx is an x86-target dependency (not compiled for aarch64 yet); the
/// gate cross-checks this literal against the crate source every run.
pub const ERR_CAP_INVALID: u64 = 0xFFFF_FFFF_FFFF_FFFC;

/// Phase-5 SVC selectors (mechanism proof only — NOT the PDX ABI).
pub const SVC_SEL_ECHO: u64 = 1; // returns arg + 1
pub const SVC_SEL_CAP: u64 = 2;  // returns 0 if arg == VALID_TEST_CAP else ERR_CAP_INVALID
/// The one "valid" test capability id for the mechanism harness.
pub const VALID_TEST_CAP: u64 = 42;

/// Phase-5 EL0 SVC-test payload VA and its final deliberate-fault VA.
pub const USER_CODE2_VA: u64 = 0x0043_0000;
pub const USER_UNMAPPED2_VA: u64 = 0x6000_0000;

// ── Phase 6: S1PIE/S1POE detection constants ───────────────────────────────
// FIELD POSITIONS VERIFIED 2026-07-22 against Arm-maintained architectural
// constants (Arm Trusted Firmware include/arch/aarch64/arch.h:
// ID_AA64MMFR3_EL1_S1POE_SHIFT = 16) and cross-checked for implementation
// truth against QEMU v11.0.2 target/arm/cpu-features.h lines 333-347
// (QEMU source is implementation truth, NOT encoding truth; it agrees with
// the Arm-maintained layout). The earlier provisional positions (S1POE=4,
// TCR2=12) were WRONG and have been corrected; [7:4] is SCTLRX, [15:12]
// is S2PIE, the TCR2 feature field is TCRX at [3:0].
// Fail-closed logic treats "field reads 0" as feature-absent.
pub const ID_AA64MMFR3_TCRX_SHIFT: u64 = 0;
pub const ID_AA64MMFR3_SCTLRX_SHIFT: u64 = 4;
pub const ID_AA64MMFR3_S1PIE_SHIFT: u64 = 8;
pub const ID_AA64MMFR3_S2PIE_SHIFT: u64 = 12;
pub const ID_AA64MMFR3_S1POE_SHIFT: u64 = 16;
