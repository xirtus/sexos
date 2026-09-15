//! AArch64 architecture backend — Phase 2 (entry + PL011 serial),
//! Phase 3 (MMU / single global VAS + EL0 smoke proof), Phase 4
//! (GICv3 + CNTP timer + preemptive 2-task ping-pong + kill containment
//! + PSCI AP bring-up).
//!
//! No PDX yet, no POE/S1PIE/TCR2/POR (deferred — see
//! Sex_ArmV9/docs/ARM_S1POE_PORT_PLAN_V1.md).
//! Platform FIXED to QEMU `virt` for these phases.

pub mod context;
pub mod early;
pub mod el0_smoke;
pub mod exceptions;
pub mod gicv3;
pub mod mmu;
pub mod phase4;
pub mod phase5;
pub mod phase6;
pub mod poe;
pub mod registers;
pub mod sched;
pub mod serial;
pub mod smp;
pub mod timer;
