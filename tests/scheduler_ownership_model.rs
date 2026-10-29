// SPDX-License-Identifier: GPL-3.0-only
//! Deterministic scheduler ownership counterexamples, 2026-10-28 audit.
//!
//! Baseline is the pre-CAS-containment source observed at audit start.
//! This is an executable abstraction of source-visible transitions, NOT a test
//! of compiled scheduler.rs, x86 assembly, memory ordering, or QEMU execution.
//! Passing `baseline_*` tests means a known failure is reproducible in the model;
//! it does not mean the kernel is correct. Keep them as historical counterexamples
//! when replacing the implementation and add tests of any extracted real helper.
//!
//! Host-only invocation (not a kernel/image build route):
//! rustc --edition=2021 --test tests/scheduler_ownership_model.rs -o /tmp/sexos-scheduler-model
//! /tmp/sexos-scheduler-model

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Life { Ready, Running, Blocked, Exited }

#[derive(Clone, Copy, Debug)]
struct Baseline {
    life: Life,
    attached: bool,
    queue_tokens: usize,
    context_saved: bool,
    stack_in_use: bool,
}

impl Baseline {
    fn running() -> Self {
        Self { life: Life::Running, attached: true, queue_tokens: 0,
            context_saved: false, stack_in_use: true }
    }

    // scheduler.rs::unpark_thread, unconditional Ready store.
    fn wake(&mut self) { self.life = Life::Ready; }
    // scheduler.rs::park_current_thread; no pending-wake handshake.
    fn park(&mut self) { self.life = Life::Blocked; }

    // scheduler.rs::tick after a distinct concrete replacement was obtained.
    // The handler's save and switch_to's RSP transfer have not happened yet.
    fn tick_has_replacement(&mut self) {
        self.attached = false;
        if self.life == Life::Running {
            self.life = Life::Ready;
            self.queue_tokens += 1;
        }
    }

    // scheduler.rs::tick returns NoRunnable without mutating ownership/state.
    fn tick_no_replacement(&mut self) {}

    // interrupts.rs page-fault NoRunnable branch accepts Running only.
    fn attached_fault_can_return(&self) -> bool { self.life == Life::Running }
}

#[test]
fn baseline_running_wake_then_switch_orphans_task() {
    let mut task = Baseline::running();
    task.wake();
    task.tick_has_replacement();
    assert_eq!(task.life, Life::Ready);
    assert!(!task.attached);
    assert_eq!(task.queue_tokens, 0, "Ready task has no dispatcher route");
}

#[test]
fn baseline_wake_before_park_is_erased() {
    let mut task = Baseline::running();
    task.wake();
    task.park();
    task.tick_has_replacement();
    assert_eq!(task.life, Life::Blocked);
    assert_eq!(task.queue_tokens, 0);
}

#[test]
fn baseline_wake_resurrects_exited_lifecycle() {
    let mut task = Baseline::running();
    task.life = Life::Exited;
    task.wake();
    assert_eq!(task.life, Life::Ready);
}

#[test]
fn baseline_detached_wake_has_no_dispatch_token() {
    let mut task = Baseline::running();
    task.park();
    task.tick_has_replacement();
    task.context_saved = true;
    task.stack_in_use = false;
    task.wake();
    assert_eq!(task.life, Life::Ready);
    assert!(!task.attached);
    assert_eq!(task.queue_tokens, 0);
}

#[test]
fn baseline_attached_no_runnable_wake_never_satisfies_fault_wait() {
    let mut task = Baseline::running();
    task.park();
    task.tick_no_replacement();
    task.wake();
    assert!(task.attached);
    assert!(!task.attached_fault_can_return());
    // Kernel-mode timer interrupts return before sched.tick(), so repeated
    // ticks do not supply the missing Ready -> Running transition here.
}

#[test]
fn baseline_tick_publishes_before_context_save_and_stack_release() {
    let mut task = Baseline::running();
    task.tick_has_replacement();
    assert_eq!(task.queue_tokens, 1);
    assert!(!task.context_saved);
    assert!(task.stack_in_use);
    // A remote queue consumer can now acquire this token. Its slot acquire
    // cannot order context writes that the publisher has not performed yet.
}

#[test]
fn enqueue_on_wake_would_duplicate_attached_execution_ownership() {
    let mut task = Baseline::running();
    task.park();
    task.tick_no_replacement();
    task.wake();
    task.queue_tokens += 1; // rejected proposed "fix": push after wake
    assert!(task.attached && task.stack_in_use);
    assert_eq!(task.queue_tokens, 1);
}

#[test]
fn saved_context_alone_does_not_make_yield_publication_safe() {
    let mut task = Baseline::running();
    task.context_saved = true;
    task.life = Life::Ready;
    task.queue_tokens += 1; // yield_and_switch publishes before clearing owner
    assert!(task.attached && task.stack_in_use);
    task.attached = false;
    assert!(task.stack_in_use, "clearing current_task does not change RSP");
}

// Reference ownership requirements, deliberately not a runtime implementation.
// Each method below is one serialized abstract transition. Real code needs an
// atomic protocol and a proven post-stack-release boundary to implement them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner { Constructed, Attached, SwitchingOut, Detached, Queued, Dead }

#[derive(Debug)]
struct Required {
    owner: Owner,
    pending: bool,
    saved: bool,
    stack_released: bool,
    tokens: usize,
}

impl Required {
    fn attached() -> Self {
        Self { owner: Owner::Attached, pending: false, saved: false,
            stack_released: false, tokens: 0 }
    }
    fn wake(&mut self) {
        if self.owner != Owner::Dead { self.pending = true; }
    }
    fn admit(&mut self, capacity: bool) {
        if self.owner == Owner::Detached && self.pending && capacity {
            assert!(self.saved && self.stack_released);
            self.pending = false;
            self.owner = Owner::Queued;
            self.tokens += 1;
        }
    }
    fn release(&mut self) {
        assert_eq!(self.owner, Owner::SwitchingOut);
        assert!(self.saved && self.stack_released);
        self.owner = Owner::Detached;
    }
}

#[test]
fn required_wake_at_every_switch_boundary_retains_one_admission() {
    // Exhaust every position for two wakes relative to ordered operations:
    // begin switch, save context, leave stack, release owner, admit to queue.
    for first in 0..=5 {
        for second in first..=5 {
            let mut task = Required::attached();
            for boundary in 0..=5 {
                if boundary == first { task.wake(); }
                if boundary == second { task.wake(); }
                task.admit(true);
                match boundary {
                    0 => task.owner = Owner::SwitchingOut,
                    1 => task.saved = true,
                    2 => task.stack_released = true,
                    3 => task.release(),
                    _ => task.admit(true),
                }
                assert!(task.tokens <= 1);
                if task.tokens > 0 {
                    assert!(task.saved && task.stack_released);
                    assert_eq!(task.owner, Owner::Queued);
                }
            }
            task.admit(true);
            assert_eq!(task.tokens, 1, "wake positions {first}, {second}");
        }
    }
}

#[test]
fn required_queue_failure_keeps_pending_token_for_retry() {
    let mut task = Required::attached();
    task.owner = Owner::SwitchingOut;
    task.saved = true;
    task.stack_released = true;
    task.release();
    task.wake();
    for _ in 0..8 { task.admit(false); task.wake(); }
    assert!(task.pending);
    assert_eq!(task.tokens, 0);
    task.admit(true);
    assert_eq!(task.tokens, 1);
}

#[test]
fn required_constructed_task_is_never_admitted_by_generic_wake() {
    let mut task = Required::attached();
    task.owner = Owner::Constructed;
    task.wake();
    task.admit(true);
    assert_eq!(task.owner, Owner::Constructed);
    assert_eq!(task.tokens, 0, "unfinished zero-RIP trampoline is not runnable");
}

#[test]
fn required_dead_task_rejects_wake_and_admission() {
    let mut task = Required::attached();
    task.owner = Owner::Dead;
    task.wake();
    task.admit(true);
    assert!(!task.pending);
    assert_eq!(task.tokens, 0);
}
