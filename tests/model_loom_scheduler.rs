// SPDX-License-Identifier: GPL-3.0-only
//! Loom models of the scheduler's park/wake/handoff custody (kernel pass 2, K6). The
//! code is the real kernel/src/task_ownership.rs and self_wait.rs, rewritten by
//! fuzz/models/build.rs only to use loom's atomics. Each thread follows the call
//! sequence that tests/model_scheduler_wait.rs's SCHEDULER_PROTOCOL records for the
//! scheduler.rs function it stands for (scripts/check_self_wait.py keeps the two equal).
//!
//! What must hold under every interleaving:
//!  * a wake racing a park is never lost: the park consumes it, the attached owner
//!    resumes, or the task is admitted with exactly one run-queue token;
//!  * racing wakes publish one token, never two;
//!  * an exit racing a wake, an admission and a dispatch ends in Dead, with custody
//!    released by whoever held it and no dead task attached;
//!  * a timer claims a deadline at most once and never the owner's later re-arm, and a
//!    timed wait raced by its timer and an IPC wake ends runnable, with at most a stale
//!    deadline left whose firing is only a spurious wake.
//! Run: RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 \
//!      cargo test --release --manifest-path fuzz/models/Cargo.toml --test model_loom_scheduler
#![cfg(loom)]
use loom::sync::atomic::{AtomicU32, Ordering};
use loom::sync::Arc;
use loom::thread;
use sexos_kernel_models::loom_scheduler_wait::ownership::{Admission, ParkResult, Phase, TaskOwnership, WakeResult};
use sexos_kernel_models::loom_scheduler_wait::self_wait::Deadline;

/// Assertions print no values: formatting overflows loom's coroutine stacks.
fn model<F: Fn() + Sync + Send + 'static>(f: F) {
    let mut b = loom::model::Builder::new();
    b.max_branches = 100_000;
    b.check(f);
}

/// A task and its run-queue: `queued` is the token count, `pushes` every token ever
/// published.
struct Task { t: TaskOwnership, queued: AtomicU32, pushes: AtomicU32 }

impl Task {
    fn new() -> Self { Self { t: TaskOwnership::new(), queued: AtomicU32::new(0), pushes: AtomicU32::new(0) } }
    /// `Scheduler::admit`: try_admit publishing into the run queue.
    fn admit(&self) -> Admission {
        self.t.try_admit(|| { self.queued.fetch_add(1, Ordering::AcqRel); self.pushes.fetch_add(1, Ordering::AcqRel); Ok::<(), ()>(()) })
    }
    /// `wake_owned_thread`: wake, then admit only if the task was Detached.
    fn wake_owned(&self) {
        if self.t.wake() == WakeResult::NeedsAdmission { self.admit(); }
    }
    /// A dispatcher taking the token: `tick`'s begin_dispatch then attach.
    fn dispatch(&self, core: u32) -> bool {
        self.queued.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire).is_ok()
            && self.t.begin_dispatch(core) && self.t.attach(core)
    }
    /// The outgoing half of a switch: begin_switch_out, then `scheduler_finish_switch`
    /// releasing custody off the old stack and admitting the task if it is runnable.
    fn switch_out(&self, core: u32) {
        assert!(self.t.begin_switch_out(core));
        assert!(unsafe { self.t.release_after_stack_switch(core) });
        self.admit();
    }
}

fn attached() -> Task {
    let task = Task::new();
    assert!(task.t.activate());
    assert_eq!(task.admit(), Admission::Published);
    assert!(task.dispatch(0));
    task
}

/// Parked and switched out: Detached and blocked, no token.
fn parked_away() -> Task {
    let task = attached();
    assert_eq!(task.t.park(0), ParkResult::Blocked);
    task.switch_out(0);
    assert_eq!(task.queued.load(Ordering::Acquire), 0);
    task
}

#[test]
fn a_wake_racing_park_and_switch_out_is_not_lost() {
    model(|| {
        let task = Arc::new(attached());
        let owner = { let task = task.clone(); thread::spawn(move || {
            // wait_for_message: park, and on Blocked switch to other work.
            let parked = task.t.park(0);
            if parked == ParkResult::Blocked { task.switch_out(0); }
            parked
        }) };
        let waker = { let task = task.clone(); thread::spawn(move || task.wake_owned()) };
        let parked = owner.join().unwrap();
        waker.join().unwrap();
        let s = task.t.snapshot();
        assert!(task.pushes.load(Ordering::Acquire) <= 2, "one token per admission");
        if parked == ParkResult::WakeConsumed {
            assert!(s.phase == Phase::Attached && !s.blocked && !s.pending, "consumed wake left the task blocked");
            assert_eq!(task.queued.load(Ordering::Acquire), 0);
        } else {
            assert!(s.phase == Phase::Queued && !s.blocked, "the wake was lost");
            assert_eq!(task.queued.load(Ordering::Acquire), 1);
        }
    });
}

#[test]
fn a_wake_racing_park_resumes_the_attached_owner() {
    model(|| {
        let task = Arc::new(attached());
        let owner = { let task = task.clone(); thread::spawn(move || {
            // wait_for_message, then wait_for_local_resume_or_switch with nothing else
            // to run: the owner keeps custody and looks for its wake.
            if task.t.park(0) == ParkResult::Blocked {
                while !task.t.resume_attached(0) { thread::yield_now(); }
            }
        }) };
        let waker = { let task = task.clone(); thread::spawn(move || task.t.wake()) };
        owner.join().unwrap();
        assert!(waker.join().unwrap() == WakeResult::Recorded, "an attached task needs no admission");
        let s = task.t.snapshot();
        assert!(s.phase == Phase::Attached && !s.blocked, "owner did not resume");
        assert_eq!(task.pushes.load(Ordering::Acquire), 1);
    });
}

#[test]
fn racing_wakes_publish_one_token() {
    model(|| {
        let task = Arc::new(parked_away());
        let a = { let task = task.clone(); thread::spawn(move || task.wake_owned()) };
        let b = { let task = task.clone(); thread::spawn(move || task.wake_owned()) };
        a.join().unwrap();
        b.join().unwrap();
        assert_eq!(task.queued.load(Ordering::Acquire), 1);
        assert!(task.t.snapshot().phase == Phase::Queued, "woken task not queued");
    });
}

#[test]
fn an_exit_racing_wake_admission_and_dispatch_ends_dead() {
    model(|| {
        let task = Arc::new(parked_away());
        let before = task.pushes.load(Ordering::Acquire);
        let waker = { let task = task.clone(); thread::spawn(move || task.wake_owned()) };
        let exiter = { let task = task.clone(); thread::spawn(move || task.t.exit()) };
        let dispatcher = { let task = task.clone(); thread::spawn(move || task.dispatch(1)) };
        waker.join().unwrap();
        assert!(exiter.join().unwrap(), "the only exit succeeds");
        let attached = dispatcher.join().unwrap();
        // Whoever holds custody gives it up: a leftover token is consumed without
        // dispatch, an attached owner switches out.
        if task.queued.load(Ordering::Acquire) == 1 { assert!(!task.dispatch(0), "a dead task dispatched"); }
        if attached { task.switch_out(1); }
        let s = task.t.snapshot();
        assert!(s.phase == Phase::Dead && s.exited, "custody did not end");
        assert!(task.pushes.load(Ordering::Acquire) - before <= 1, "two tokens");
        assert!(task.t.wake() == WakeResult::Exited, "a dead task woke");
    });
}

#[test]
fn a_timer_claims_a_deadline_once_and_never_a_later_arm() {
    model(|| {
        let d = Arc::new(Deadline::new());
        assert_eq!(d.arm(0, 5), Some(5));
        let timer = { let d = d.clone(); thread::spawn(move || d.take_due(10)) };
        let rival = { let d = d.clone(); thread::spawn(move || d.take_due(10)) };
        // wait_for_message_ticks on the owner: cancel the stale deadline, arm anew.
        let owner = { let d = d.clone(); thread::spawn(move || { d.cancel(); d.arm(10, 2) }) };
        let (a, b) = (timer.join().unwrap(), rival.join().unwrap());
        assert!(owner.join().unwrap() == Some(12), "the owner's arm failed");
        assert!(a.is_none() || a == Some(5), "a timer took the later arm");
        assert!(b.is_none() || b == Some(5), "a timer took the later arm");
        assert!(!(a.is_some() && b.is_some()), "a deadline claimed twice");
        assert!(d.take_due(12) == Some(12), "a stale claim removed the later arm");
    });
}

#[test]
fn a_timed_wait_raced_by_its_timer_and_an_ipc_wake_ends_runnable() {
    model(|| {
        let task = Arc::new(attached());
        let deadline = Arc::new(Deadline::new());
        let owner = { let (task, d) = (task.clone(), deadline.clone()); thread::spawn(move || {
            // wait_for_message_ticks.
            d.cancel();
            assert!(d.arm(0, 2) == Some(2), "arm failed");
            match task.t.park(0) {
                ParkResult::WakeConsumed => d.cancel(),
                ParkResult::Blocked => task.switch_out(0),
                _ => panic!("owner lost custody"),
            }
        }) };
        // poll_self_wait_deadlines at tick 2.
        let timer = { let (task, d) = (task.clone(), deadline.clone()); thread::spawn(move || {
            if d.take_due(2).is_some() { task.wake_owned(); }
        }) };
        // unpark_thread from an IPC reply.
        let ipc = { let (task, d) = (task.clone(), deadline.clone()); thread::spawn(move || { d.cancel(); task.wake_owned(); }) };
        owner.join().unwrap();
        timer.join().unwrap();
        ipc.join().unwrap();
        let s = task.t.snapshot();
        assert!(!s.blocked || s.pending, "the timed wait was never woken");
        assert!(task.queued.load(Ordering::Acquire) <= 1, "two tokens");
        // An IPC cancel can land just before the owner arms, leaving that deadline
        // armed after the IPC wake. The kernel accepts it (the owner's next wait
        // cancels it first): firing it later is only a spurious wake, never a second
        // token or custody change.
        let queued = task.queued.load(Ordering::Acquire);
        if deadline.take_due(u64::MAX).is_some() { task.wake_owned(); }
        assert!(task.queued.load(Ordering::Acquire) == queued, "a stale deadline admitted the task again");
        assert!(deadline.take_due(u64::MAX).is_none(), "a deadline claimed twice");
    });
}

