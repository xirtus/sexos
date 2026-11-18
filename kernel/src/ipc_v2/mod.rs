//! IPC v2 objects (kernel plan PR #405, phase 2): endpoints, one-shot reply
//! capabilities and notifications.
//!
//! These are pure state machines over task ids: they decide *who* runs next and
//! *what* is delivered, and leave the switch itself to the syscall layer. That keeps
//! them testable on the host (tests/ipc_v2_*.rs include these files directly) and
//! keeps the fastpath free of run queues: a call that finds its server waiting
//! takes it with one compare-and-swap. Nothing here is wired to syscalls yet; that
//! comes after PR #402 with the IPC owner's agreement.

pub mod endpoint;
pub mod notification;
pub mod reply;

/// A task as these objects see it: a small dense id the scheduler owns.
pub type TaskId = u32;
