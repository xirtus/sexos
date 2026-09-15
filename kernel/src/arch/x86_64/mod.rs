//! x86_64 architecture backend — Phase 1 re-export shims ONLY.
//!
//! Pure aliases to the existing top-level modules. Nothing moved,
//! nothing renamed, no call site changed. The canonical paths remain
//! `crate::pku`, `crate::gdt`, … until a later phase performs reviewed
//! moves. `crate::pku::PKU_ENABLED` is referenced by name from naked
//! assembly; its path MUST keep resolving (see
//! docs/handoff/ARM_S1POE_AUDIT_V1.md).

pub use crate::apic;
pub use crate::gdt;
pub use crate::interrupts;
pub use crate::pku;
pub use crate::serial;
pub use crate::smp;
