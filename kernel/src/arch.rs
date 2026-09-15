//! Compile-time architecture seam — Phase 1 (ARM64_ARCH_SEAM_AND_X86_BASELINE_FREEZE_V1).
//!
//! This file pre-existed as a one-line orphan (`pub mod x86_64;`) never
//! declared by lib.rs. Phase 1 activates it as the intended seam with
//! cfg gates. Behavior-neutral: on x86_64 it only re-exports existing
//! modules under an arch-namespaced path; no call site changes.
//! The aarch64 arm is added in Phase 2 (not now).
//!
//! Plan: Sex_ArmV9/docs/ARM_S1POE_PORT_PLAN_V1.md §D.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as current;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use aarch64 as current;
