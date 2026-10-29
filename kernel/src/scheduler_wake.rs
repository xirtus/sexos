// SPDX-License-Identifier: GPL-3.0-only
//! Lifecycle containment only; this does not publish an execution token.
use core::sync::atomic::{AtomicU32, Ordering};

// scheduler.rs pins these wire values to TaskState at compile time.
pub const READY: u32 = 0;
pub const BLOCKED: u32 = 2;

/// Change only Blocked to Ready. Running must stay Running so tick retains
/// its outgoing-task requeue rule; Exited and unknown states stay unchanged.
/// The CAS is the sole transition point, so a concurrent exit cannot be
/// overwritten by a stale wake. Repeated wakes of Ready are idempotent.
///
/// This records no pending wake: wake-before-park can still be lost, detached
/// tasks gain no queue token, and an attached fault wait still needs a local
/// resume protocol. Never add an enqueue here without full save/stack-release
/// and per-task admission ownership discipline.
pub fn wake_blocked(state: &AtomicU32) {
    let _ = state.compare_exchange(BLOCKED, READY, Ordering::AcqRel, Ordering::Acquire);
}

#[cfg(test)]
fn wake(state: &AtomicU32) { wake_blocked(state); }

#[cfg(test)]
#[path = "../../tests/scheduler_wake_lifecycle.rs"]
mod tests;
