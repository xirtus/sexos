// SPDX-License-Identifier: GPL-3.0-only
//! Atomic scheduling custody and park/wake handshake.
//!
//! All lifecycle reads derive from this one word; a separate Ready/Running
//! atomic would race the ownership/pending decision. Queued includes the sole
//! producer's admission reservation until its bounded queue publication ends.
//! Detached runnable tasks retain retry custody in the scheduler's task inventory.
//! This module cannot establish physical stack departure: its unsafe release
//! operation must be called by the runtime only after the final context save
//! and after execution has moved off the outgoing task's stack.
use core::sync::atomic::{AtomicU32, Ordering};

const PHASE_MASK: u32 = 7;
const PENDING: u32 = 1 << 3;
const BLOCKED: u32 = 1 << 4;
const EXITED: u32 = 1 << 5;
const FAULT_WAIT: u32 = 1 << 6;
const OWNER_SHIFT: u32 = 8;
const OWNER_MASK: u32 = 0xff << OWNER_SHIFT;
pub const MAX_CORES: u32 = 128;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Constructed = 0,
    Detached = 1,
    Queued = 2,
    Dispatching = 3,
    Attached = 4,
    SwitchingOut = 5,
    Dead = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    pub pending: bool,
    pub blocked: bool,
    pub fault_blocked: bool,
    pub exited: bool,
    pub owner_core: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission { Published, Deferred, NotRunnable }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkResult { Blocked, WakeConsumed, NotOwner, Exited }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeResult { Recorded, NeedsAdmission, Exited }

pub struct TaskOwnership { word: AtomicU32 }

fn phase(word: u32) -> Phase {
    match word & PHASE_MASK {
        0 => Phase::Constructed, 1 => Phase::Detached, 2 => Phase::Queued,
        3 => Phase::Dispatching, 4 => Phase::Attached, 5 => Phase::SwitchingOut,
        6 => Phase::Dead, _ => unreachable!("invalid task ownership phase"),
    }
}
fn with_phase(word: u32, next: Phase) -> u32 {
    (word & !(PHASE_MASK | OWNER_MASK)) | next as u32
}
fn owned(word: u32, expected: Phase, core: u32) -> bool {
    core < MAX_CORES && phase(word) == expected
        && (word & OWNER_MASK) >> OWNER_SHIFT == core + 1
}
fn runnable(word: u32) -> bool {
    word & (EXITED | FAULT_WAIT) == 0 && (word & BLOCKED == 0 || word & PENDING != 0)
}

impl TaskOwnership {
    pub const fn new() -> Self { Self { word: AtomicU32::new(Phase::Constructed as u32) } }

    /// Legacy TaskState values: Ready=0, Running=1, Blocked=2, Exited=3.
    /// Dispatch reservations and switching-out custody are not Running.
    pub fn load(&self, order: Ordering) -> u32 {
        let word = self.word.load(order);
        if word & EXITED != 0 || phase(word) == Phase::Dead { 3 }
        else if word & BLOCKED != 0 { 2 }
        else if phase(word) == Phase::Attached { 1 }
        else { 0 }
    }

    pub fn snapshot(&self) -> Snapshot {
        let word = self.word.load(Ordering::Acquire);
        let owner = (word & OWNER_MASK) >> OWNER_SHIFT;
        Snapshot { phase: phase(word), pending: word & PENDING != 0,
            blocked: word & BLOCKED != 0, fault_blocked: word & FAULT_WAIT != 0, exited: word & EXITED != 0,
            owner_core: if owner == 0 { None } else { Some(owner - 1) } }
    }

    pub fn needs_admission(&self) -> bool {
        let word = self.word.load(Ordering::Acquire);
        phase(word) == Phase::Detached && runnable(word)
    }

    /// Explicit first admission only. A wake of Constructed never activates it.
    pub fn activate(&self) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if phase(old) != Phase::Constructed || old & EXITED != 0 { return false; }
            let new = with_phase(old, Phase::Detached);
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true, Err(now) => old = now,
            }
        }
    }

    /// Claim the only admission reservation, then publish one pointer token.
    /// `publish` must be the bounded queue's try_push: Ok iff it published
    /// exactly one token, Err iff it published none. It must not unwind.
    /// Consumers must call begin_dispatch only after removing that token.
    pub fn try_admit<E>(&self, publish: impl FnOnce() -> Result<(), E>) -> Admission {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if phase(old) != Phase::Detached || !runnable(old) { return Admission::NotRunnable; }
            let mut new = with_phase(old, Phase::Queued);
            // Satisfy a blocked wait exactly once. A wake of an already
            // runnable task remains pending across admission and dispatch.
            if old & BLOCKED != 0 { new &= !(BLOCKED | PENDING); }
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break, Err(now) => old = now,
            }
        }
        if publish().is_ok() { return Admission::Published; }
        // No pointer escaped. Concurrent wake/exit can change flags, but no
        // dispatcher can own this un-published reservation. Preserve flags.
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            assert_eq!(phase(old), Phase::Queued, "failed admission lost reservation");
            let next = if old & EXITED != 0 { Phase::Dead } else { Phase::Detached };
            let new = with_phase(old, next);
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Admission::Deferred, Err(now) => old = now,
            }
        }
    }

    /// Consume one actual dequeued token. The acquire pairs with context/stack
    /// release before admission. Exited tokens are consumed without dispatch.
    pub fn begin_dispatch(&self, core: u32) -> bool {
        if core >= MAX_CORES { return false; }
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if phase(old) != Phase::Queued { return false; }
            let exited = old & EXITED != 0;
            let new = if exited { with_phase(old, Phase::Dead) }
                else { with_phase(old, Phase::Dispatching) | ((core + 1) << OWNER_SHIFT) };
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return !exited, Err(now) => old = now,
            }
        }
    }

    pub fn attach(&self, core: u32) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if !owned(old, Phase::Dispatching, core) { return false; }
            let exited = old & EXITED != 0;
            let new = if exited { with_phase(old, Phase::Dead) }
                else { (old & !PHASE_MASK) | Phase::Attached as u32 };
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return !exited, Err(now) => old = now,
            }
        }
    }

    pub fn cancel_dispatch(&self, core: u32) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if !owned(old, Phase::Dispatching, core) { return false; }
            let next = if old & EXITED != 0 { Phase::Dead } else { Phase::Detached };
            match self.word.compare_exchange_weak(old, with_phase(old, next), Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true, Err(now) => old = now,
            }
        }
    }

    /// The pending test and park are one CAS: wake-before-park is consumed,
    /// while wake-after-park remains pending for local resume or admission.
    pub fn park(&self, core: u32) -> ParkResult {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if old & EXITED != 0 { return ParkResult::Exited; }
            if !owned(old, Phase::Attached, core) { return ParkResult::NotOwner; }
            if old & FAULT_WAIT != 0 { return ParkResult::Blocked; }
            let pending = old & PENDING != 0;
            let new = if pending { old & !(BLOCKED | PENDING) } else { old | BLOCKED };
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return if pending { ParkResult::WakeConsumed } else { ParkResult::Blocked },
                Err(now) => old = now,
            }
        }
    }

    /// A page fault is a distinct wait condition. Ordinary IRQ/reply wakes
    /// cannot authorize retry of an unresolved faulting instruction. Only the
    /// authenticated pager transaction may call complete_fault.
    pub fn park_fault(&self, core: u32) -> ParkResult {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if old & EXITED != 0 { return ParkResult::Exited; }
            if !owned(old, Phase::Attached, core) { return ParkResult::NotOwner; }
            let new = (old | BLOCKED | FAULT_WAIT) & !PENDING;
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return ParkResult::Blocked, Err(now) => old = now,
            }
        }
    }

    /// # Safety
    /// Caller has authenticated and retired this task's unique fault transaction
    /// after its authorized mapping became visible. Generic wake is insufficient.
    /// Custody is preserved: Attached/SwitchingOut must never be queued here.
    pub unsafe fn complete_fault(&self) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if old & EXITED != 0 || old & FAULT_WAIT == 0
                || !matches!(phase(old), Phase::Attached | Phase::SwitchingOut | Phase::Detached) {
                return false;
            }
            // A pending ordinary event still belongs to its next ordinary wait.
            let new = old & !(FAULT_WAIT | BLOCKED);
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true, Err(now) => old = now,
            }
        }
    }

    /// Resume the still-attached owner without inventing a queue token.
    pub fn resume_attached(&self, core: u32) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if !owned(old, Phase::Attached, core) || !runnable(old) { return false; }
            if old & BLOCKED == 0 { return true; }
            let new = old & !(BLOCKED | PENDING);
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true, Err(now) => old = now,
            }
        }
    }

    pub fn begin_switch_out(&self, core: u32) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if !owned(old, Phase::Attached, core) { return false; }
            let new = (old & !PHASE_MASK) | Phase::SwitchingOut as u32;
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true, Err(now) => old = now,
            }
        }
    }

    /// # Safety
    /// The calling core must have finished every outgoing context write and
    /// moved execution off this task's kernel stack. No further old-stack or
    /// context writes may occur before this task is next attached. Phase alone
    /// does not prove those hardware/runtime obligations.
    pub unsafe fn release_after_stack_switch(&self, core: u32) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if !owned(old, Phase::SwitchingOut, core) { return false; }
            let next = if old & EXITED != 0 { Phase::Dead } else { Phase::Detached };
            match self.word.compare_exchange_weak(old, with_phase(old, next), Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true, Err(now) => old = now,
            }
        }
    }

    pub fn wake(&self) -> WakeResult {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if old & EXITED != 0 || phase(old) == Phase::Dead { return WakeResult::Exited; }
            let new = old | PENDING;
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return if phase(new) == Phase::Detached { WakeResult::NeedsAdmission } else { WakeResult::Recorded },
                Err(now) => old = now,
            }
        }
    }

    /// Terminal intent cannot be undone by wake or a stale park operation.
    /// Queued/owned tasks retain custody until their token/owner releases it.
    pub fn exit(&self) -> bool {
        let mut old = self.word.load(Ordering::Acquire);
        loop {
            if old & EXITED != 0 { return false; }
            let mut new = (old | EXITED) & !(PENDING | BLOCKED | FAULT_WAIT);
            if matches!(phase(old), Phase::Constructed | Phase::Detached) {
                new = with_phase(new, Phase::Dead);
            }
            match self.word.compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true, Err(now) => old = now,
            }
        }
    }
}
