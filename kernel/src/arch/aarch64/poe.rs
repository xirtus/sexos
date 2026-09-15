//! Phase 6 — S1PIE/S1POE DETECTION ONLY. Plan Phase 6 scope, honestly
//! bounded:
//!   - Read ID_AA64MMFR3_EL1, print parsed fields. FIELD POSITIONS:
//!     VERIFIED 2026-07-22 (Arm-maintained TF-A arch.h constants,
//!     QEMU v11.0.2 implementation cross-check; prior provisional
//!     positions were wrong — see ARM64_PHASE_6B handoff). Behavior
//!     stays UNPROVEN; a zero field is always treated as
//!     feature-absent (fail-safe).
//!   - FAIL-CLOSED if required-but-absent (cfg feature arm64-require-poe),
//!     else continue with a NO-ISOLATION dev banner.
//!   - Page-tagging PLUMBING exists in mmu.rs (descriptor attrs accept a
//!     POIndex field), but every tag is POIndex 0 — safe on ALL hardware
//!     (POIndex=0 = default lane). Nonzero tags are NOT written: QEMU
//!     S1POE is UNVERIFIED and nonzero POIndex on non-S1POE silicon could
//!     be RES0 (translation faults). Zero claims of isolation.
//!   - NO POR_EL0/POR_EL1/TCR2_EL1 writes anywhere (gate-enforced).
//!   - Phase 7 (POR_EL0 switching) is NOT IMPLEMENTED here: the plan's
//!     own STOP FIRST rule requires Phase-6 runtime facts (POIndex width,
//!     POR lane encoding, PIE/POE interaction) that need probes P1-P6 on
//!     a QEMU-capable host. Writing switching code blind would be an
//!     unverifiable isolation claim — forbidden.

use super::registers::*;
use super::serial;

/// Read + parse + report the feature registers. Never traps: ID register
/// reads are always safe. FIELD POSITIONS: VERIFIED 2026-07-22 (Arm TF-A
/// arch.h constants, QEMU v11.0.2 cross-check) — see registers.rs.
/// BEHAVIOR remains UNPROVEN (an ID bit is not an implementation).
pub fn detect() {
    let id: u64;
    unsafe { core::arch::asm!("mrs {}, ID_AA64MMFR3_EL1", out(reg) id, options(nomem, nostack)) }
    let tcrx = (id >> ID_AA64MMFR3_TCRX_SHIFT) & 0xF;
    let sctlrx = (id >> ID_AA64MMFR3_SCTLRX_SHIFT) & 0xF;
    let s1pie = (id >> ID_AA64MMFR3_S1PIE_SHIFT) & 0xF;
    let s2pie = (id >> ID_AA64MMFR3_S2PIE_SHIFT) & 0xF;
    let s1poe = (id >> ID_AA64MMFR3_S1POE_SHIFT) & 0xF;
    serial::write_str("[sex.arm.poe] id_aa64mmfr3=0x");
    serial::write_hex(id);
    serial::write_str("\n[sex.arm.poe] fields=VERIFIED(arm_tfa+qemu_xcheck) behavior=UNPROVEN tcrx=");
    serial::write_dec(tcrx);
    serial::write_str(" sctlrx=");
    serial::write_dec(sctlrx);
    serial::write_str(" s1pie=");
    serial::write_dec(s1pie);
    serial::write_str(" s2pie=");
    serial::write_dec(s2pie);
    serial::write_str(" s1poe=");
    serial::write_dec(s1poe);
    serial::write_str("\n");

    if s1poe == 0 {
        #[cfg(feature = "arm64-require-poe")]
        {
            serial::write_str("SEX_ARM_FAILCLOSED_NO_S1POE\n");
            loop { core::hint::spin_loop() }
        }
        #[cfg(not(feature = "arm64-require-poe"))]
        {
            serial::write_str("[sex.arm.poe] isolation=none dev=1\n");
        }
    } else {
        // Detection is NOT proof: probes P1-P6 (POR r/w, TCR2 activation,
        // negative R/W/X aborts, POR-only-no-TLBI, EL0 trap) are required
        // before ANY isolation statement. Report honestly.
        serial::write_str("[sex.arm.poe] s1poe=present status=UNPROVEN isolation=none\n");
    }
}

/// Report tagging-plumbing status (all POIndex values are 0 = default
/// lane; nonzero tagging awaits runtime probes).
pub fn tag_plumbing_status() {
    serial::write_str("SEX_ARM_POE_TAGGED poindex_default=0 nonzero_tags=0\n");
}
