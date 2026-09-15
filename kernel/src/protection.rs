//! Architecture-neutral semantic protection types — Phase 1 DEAD CODE.
//!
//! Introduced by ARM64_ARCH_SEAM_AND_X86_BASELINE_FREEZE_V1.
//! NOT wired into any call path. No shared struct layout changes.
//! Wiring happens at arch-backend boundaries in Phases 2–7 only.
//!
//! Plan: Sex_ArmV9/docs/ARM_S1POE_PORT_PLAN_V1.md §D.

#![allow(dead_code)]

/// Logical protection-domain key. Matches the current `pku_key` width.
///
/// Arm note: the logical DomainKey space is unbounded. The number of
/// simultaneous HARDWARE indexes on AArch64 (POIndex) is PROVISIONAL
/// until Phase 6 proves descriptor encoding and usable index count
/// (plan §C.5). No domain→index constant may be frozen before then.
pub type DomainKey = u8;

/// Hardware overlay permission mask. Matches the current PKRU width
/// (32-bit) on x86_64. AArch64 POR_EL0/POR_EL1 are 64-bit; any width
/// migration is confined to the arch backend in Phase 7 and must never
/// change shared struct layouts (STOP FIRST otherwise).
pub type ProtectionMask = u32;

/// Boot-time detected protection-hardware feature set.
pub struct ProtectionFeatures {
    /// Intel MPK/PKU present (CPUID.(EAX=7,ECX=0):ECX.PKU).
    pub pku: bool,
}
