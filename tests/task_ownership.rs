// SPDX-License-Identifier: GPL-3.0-only
//! Tests execute production ownership and bounded-queue code. The explicit
//! saved/stack flags below are a boundary oracle, not privileged assembly proof.
#[path = "../kernel/src/task_ownership.rs"]
mod ownership;
#[path = "../kernel/src/scheduler_queue.rs"]
mod queue;
use ownership::{Admission, ParkResult, Phase, TaskOwnership, WakeResult};
use queue::BoundedQueue;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

fn attached(core: u32) -> TaskOwnership {
    let state = TaskOwnership::new();
    assert!(state.activate());
    assert_eq!(state.try_admit(|| Ok::<(), ()>(())), Admission::Published);
    assert!(state.begin_dispatch(core));
    assert!(state.attach(core));
    state
}

#[test]
fn constructed_tasks_require_explicit_activation_and_cannot_be_admitted_twice() {
    let state = TaskOwnership::new();
    assert_eq!(state.wake(), WakeResult::Recorded);
    assert_eq!(state.try_admit::<()>(|| panic!("unactivated task published")), Admission::NotRunnable);
    assert_eq!(state.snapshot().phase, Phase::Constructed);
    assert!(state.activate());
    assert!(!state.activate());
    assert_eq!(state.try_admit(|| Ok::<(), ()>(())), Admission::Published);
    assert_eq!(state.try_admit::<()>(|| panic!("duplicate token")), Admission::NotRunnable);
    assert!(state.begin_dispatch(1));
    assert!(!state.begin_dispatch(0));
    assert!(!state.attach(0));
    assert!(state.attach(1));
    assert_eq!(state.load(Ordering::Acquire), 1);
    assert_eq!(state.park(1), ParkResult::WakeConsumed, "pending survives dispatch");
}

#[test]
fn wake_on_either_side_of_park_resumes_same_owner_without_queue_token() {
    for wake_first in [false, true] {
        let state = attached(3);
        if wake_first { state.wake(); }
        assert_eq!(state.park(3), if wake_first { ParkResult::WakeConsumed } else { ParkResult::Blocked });
        if !wake_first { state.wake(); }
        assert!(state.resume_attached(3));
        assert_eq!(state.load(Ordering::Acquire), 1);
        assert_eq!(state.snapshot().phase, Phase::Attached);
        assert!(!state.snapshot().pending);
        assert_eq!(state.try_admit::<()>(|| panic!("attached owner published")), Admission::NotRunnable);
        assert!(!state.resume_attached(4));
    }
}

#[test]
fn every_wake_boundary_between_park_and_reattach_retains_progress() {
    // Wake once at every boundary of a real helper transition sequence:
    // park, switch intent, final save, stack departure, release, admission,
    // dequeue reservation, attach. Boundary 0 also tests wake-before-park.
    for wake_at in 0..=8 {
        let state = attached(0);
        let mut saved = false;
        let mut stack_in_use = true;
        let queue = BoundedQueue::<u8, 2>::new();
        let mut published = false;
        for step in 0..=8 {
            if step == wake_at { state.wake(); }
            match step {
                0 => { state.park(0); }
                1 => { assert!(state.begin_switch_out(0)); }
                2 => { saved = true; }
                3 => { stack_in_use = false; }
                4 => {
                    assert!(saved && !stack_in_use);
                    assert!(unsafe { state.release_after_stack_switch(0) });
                }
                5 => {
                    published = state.try_admit(|| queue.try_push(1usize as *mut u8)) == Admission::Published;
                }
                6 => { if published { assert!(!queue.steal().is_null()); assert!(state.begin_dispatch(1)); } }
                7 => { if published { assert!(state.attach(1)); } }
                _ => {}
            }
            if matches!(state.snapshot().phase, Phase::Attached | Phase::SwitchingOut) && step < 4 {
                assert_eq!(state.try_admit::<()>(|| panic!("published before release at boundary {wake_at}")), Admission::NotRunnable);
            }
        }
        if !published {
            assert!(state.needs_admission(), "wake lost at boundary {wake_at}: {:?}", state.snapshot());
            assert_eq!(state.try_admit(|| queue.try_push(1usize as *mut u8)), Admission::Published);
            assert!(!queue.steal().is_null());
            assert!(state.begin_dispatch(1));
            assert!(state.attach(1));
        }
        assert_eq!(state.snapshot().owner_core, Some(1));
        assert_eq!(state.load(Ordering::Acquire), 1);
    }
}

#[test]
fn queue_full_and_contention_retain_retry_custody_and_concurrent_flags() {
    let queue = BoundedQueue::<u8, 2>::new();
    queue.try_push(1usize as *mut u8).unwrap();
    queue.try_push(2usize as *mut u8).unwrap();
    let state = attached(0);
    assert_eq!(state.park(0), ParkResult::Blocked);
    assert!(state.begin_switch_out(0));
    assert!(unsafe { state.release_after_stack_switch(0) });
    assert_eq!(state.wake(), WakeResult::NeedsAdmission);
    assert_eq!(state.try_admit(|| queue.try_push(3usize as *mut u8)), Admission::Deferred);
    assert!(state.needs_admission(), "satisfied blocked wake must survive full rollback");
    assert_eq!(state.try_admit(|| {
        state.wake();
        Err(queue::PushError::Contended)
    }), Admission::Deferred);
    assert!(state.snapshot().pending);
    assert!(!queue.steal().is_null());
    assert_eq!(state.try_admit(|| queue.try_push(3usize as *mut u8)), Admission::Published);
    assert_eq!(queue.steal(), 2usize as *mut u8);
    assert_eq!(queue.steal(), 3usize as *mut u8);
    assert!(queue.steal().is_null());
    assert!(state.begin_dispatch(1));
    assert!(state.attach(1));
    assert_eq!(state.park(1), ParkResult::WakeConsumed);
}

#[test]
fn exit_during_failed_or_successful_admission_never_resurrects() {
    for success in [false, true] {
        let state = TaskOwnership::new();
        state.activate();
        assert_eq!(state.try_admit(|| {
            assert!(state.exit());
            assert_eq!(state.wake(), WakeResult::Exited);
            if success { Ok(()) } else { Err(()) }
        }), if success { Admission::Published } else { Admission::Deferred });
        if success { assert!(!state.begin_dispatch(0)); }
        assert_eq!(state.snapshot().phase, Phase::Dead);
        assert!(!state.needs_admission());
        assert_eq!(state.load(Ordering::Acquire), 3);
    }
}

#[test]
fn exit_at_each_owned_phase_preserves_custody_until_release() {
    let state = TaskOwnership::new();
    state.activate();
    state.try_admit(|| Ok::<(), ()>(()));
    assert!(state.begin_dispatch(0));
    state.exit();
    assert_eq!(state.snapshot().phase, Phase::Dispatching);
    assert!(!state.attach(0));
    assert_eq!(state.snapshot().phase, Phase::Dead);
    for switch_first in [false, true] {
        let state = attached(2);
        if switch_first { state.begin_switch_out(2); }
        state.exit();
        assert_eq!(state.snapshot().owner_core, Some(2));
        assert_eq!(state.wake(), WakeResult::Exited);
        assert_eq!(state.park(2), ParkResult::Exited);
        assert!(!state.resume_attached(2));
        if !switch_first { assert!(state.begin_switch_out(2)); }
        assert!(!unsafe { state.release_after_stack_switch(1) });
        assert!(unsafe { state.release_after_stack_switch(2) });
        assert_eq!(state.snapshot().phase, Phase::Dead);
    }
}

#[test]
fn canceled_dispatch_retains_pending_and_wrong_core_cannot_release_owner() {
    let state = TaskOwnership::new();
    state.activate();
    state.try_admit(|| Ok::<(), ()>(()));
    state.begin_dispatch(9);
    state.wake();
    assert!(!state.cancel_dispatch(8));
    assert!(state.cancel_dispatch(9));
    assert!(state.needs_admission());
    assert!(state.snapshot().pending);
    state.try_admit(|| Ok::<(), ()>(()));
    state.begin_dispatch(9);
    state.attach(9);
    assert!(!state.begin_switch_out(8));
    assert_eq!(state.park(8), ParkResult::NotOwner);
    assert!(!unsafe { state.release_after_stack_switch(9) });
    assert_eq!(state.snapshot().phase, Phase::Attached);
}

#[test]
fn generic_wakes_cannot_release_fault_wait_at_any_owner_boundary() {
    for wake_before in [false, true] {
        let state = attached(0);
        if wake_before { state.wake(); }
        assert_eq!(state.park_fault(0), ParkResult::Blocked);
        assert!(!state.snapshot().pending);
        for _ in 0..3 { state.wake(); }
        assert!(state.snapshot().fault_blocked);
        assert_eq!(state.load(Ordering::Acquire), 2);
        assert!(!state.resume_attached(0));
        assert_eq!(state.park(0), ParkResult::Blocked);
        state.begin_switch_out(0);
        state.wake();
        unsafe { state.release_after_stack_switch(0); }
        state.wake();
        assert!(!state.needs_admission());
        assert_eq!(state.try_admit::<()>(|| panic!("unresolved fault admitted")), Admission::NotRunnable);
        assert!(state.exit());
        assert_eq!(state.snapshot().phase, Phase::Dead);
    }
}

#[derive(Debug, Clone, Copy)]
enum Event { Park, Wake, Exit, Switch, Save, LeaveStack, Release, Admit, Dispatch, Attach, Resume, Cancel, Activate, Fault }
const EVENTS: [Event; 12] = [Event::Park, Event::Wake, Event::Exit, Event::Switch,
    Event::Save, Event::LeaveStack, Event::Release, Event::Admit, Event::Dispatch,
    Event::Attach, Event::Resume, Event::Cancel];

fn replay_seed(seed: Phase, trace: &[Event]) -> (ownership::Snapshot, usize, bool, bool) {
    let queue = BoundedQueue::<u8, 2>::new();
    let state = TaskOwnership::new();
    let mut tokens = 0;
    let mut saved = true;
    let mut stack_in_use = false;
    if seed == Phase::Dead { state.exit(); }
    else if seed != Phase::Constructed {
        state.activate();
        if seed != Phase::Detached {
            state.try_admit(|| queue.try_push(1usize as *mut u8)); tokens = 1;
            if seed != Phase::Queued {
                assert!(!queue.steal().is_null()); tokens = 0;
                state.begin_dispatch(0);
                if seed != Phase::Dispatching {
                    state.attach(0); saved = false; stack_in_use = true;
                    if seed == Phase::SwitchingOut { state.begin_switch_out(0); }
                }
            }
        }
    }
    for (index, event) in trace.iter().enumerate() {
        match event {
            Event::Activate => { state.activate(); }
            Event::Fault => { state.park_fault(0); }
            Event::Park => { state.park(0); }
            Event::Wake => { state.wake(); }
            Event::Exit => { state.exit(); }
            Event::Switch => { state.begin_switch_out(0); }
            Event::Save => { if state.snapshot().phase == Phase::SwitchingOut { saved = true; } }
            Event::LeaveStack => { if saved { stack_in_use = false; } }
            Event::Release => {
                // The oracle supplies the physical runtime preconditions.
                // Calling this unsafe API without these is not a valid test
                // of the helper and cannot prove an assembly implementation.
                if saved && !stack_in_use { unsafe { state.release_after_stack_switch(0); } }
            }
            Event::Admit => {
                if state.try_admit(|| {
                    assert!(saved && !stack_in_use, "early publish seed={seed:?}: {:?}", &trace[..=index]);
                    queue.try_push(1usize as *mut u8)
                }) == Admission::Published { tokens += 1; }
            }
            Event::Dispatch => {
                if !queue.steal().is_null() { tokens -= 1; state.begin_dispatch(0); }
            }
            Event::Attach => { if state.attach(0) { saved = false; stack_in_use = true; } }
            Event::Resume => { state.resume_attached(0); }
            Event::Cancel => { state.cancel_dispatch(0); }
        }
        let s = state.snapshot();
        assert!(tokens <= 1, "duplicate token seed={seed:?}: {:?}", &trace[..=index]);
        assert_eq!(tokens == 1, s.phase == Phase::Queued, "custody mismatch seed={seed:?}: {:?} {s:?}", &trace[..=index]);
        assert_eq!(s.owner_core.is_some(), matches!(s.phase, Phase::Dispatching | Phase::Attached | Phase::SwitchingOut),
            "owner mismatch seed={seed:?}: {:?} {s:?}", &trace[..=index]);
        if s.exited { assert_eq!(state.wake(), WakeResult::Exited); assert_eq!(state.load(Ordering::Acquire), 3); }
        if s.phase == Phase::Dead { assert!(s.exited); }
    }
    (state.snapshot(), tokens, saved, stack_in_use)
}

fn replay(trace: &[Event]) -> (ownership::Snapshot, usize, bool, bool) {
    replay_seed(Phase::Attached, trace)
}

#[test]
fn exhaust_all_five_event_traces_and_replay_exact_state() {
    let mut traces = 0usize;
    let mut trace = [Event::Wake; 5];
    for encoded in 0..12usize.pow(5) {
        let mut n = encoded;
        for event in &mut trace { *event = EVENTS[n % EVENTS.len()]; n /= EVENTS.len(); }
        let first = replay(&trace);
        assert_eq!(replay(&trace), first, "replay diverged for trace {trace:?}");
        traces += 1;
    }
    assert_eq!(traces, 248_832);
    eprintln!("[task.ownership.exhaustive] traces={traces} depth=5 events=12 scope=production_helper_boundaries");
}


#[test]
fn exhaust_all_four_event_traces_from_every_custody_phase() {
    let phases = [Phase::Constructed, Phase::Detached, Phase::Queued,
        Phase::Dispatching, Phase::Attached, Phase::SwitchingOut, Phase::Dead];
    let mut events = EVENTS.to_vec();
    events.extend([Event::Activate, Event::Fault]);
    let mut traces = 0usize;
    let mut trace = [Event::Wake; 4];
    for seed in phases {
        for encoded in 0..14usize.pow(4) {
            let mut n = encoded;
            for event in &mut trace { *event = events[n % events.len()]; n /= events.len(); }
            let first = replay_seed(seed, &trace);
            assert_eq!(replay_seed(seed, &trace), first, "seed {seed:?}, trace {trace:?}");
            traces += 1;
        }
    }
    assert_eq!(traces, 268_912);
    eprintln!("[task.ownership.exhaustive] traces={traces} depth=4 events=14 seeds=7 scope=production_helper_boundaries");
}

#[test]
fn concurrent_park_wake_and_exit_cannot_erase_event_or_terminal_state() {
    for round in 0..512 {
        let state = Arc::new(attached(0));
        let barrier = Arc::new(Barrier::new(3));
        std::thread::scope(|scope| {
            let (a, b) = (state.clone(), barrier.clone());
            scope.spawn(move || { b.wait(); a.park(0); });
            let (a, b) = (state.clone(), barrier.clone());
            scope.spawn(move || { b.wait(); a.wake(); });
            barrier.wait();
        });
        assert!(state.resume_attached(0), "lost park/wake round {round}: {:?}", state.snapshot());
        std::thread::scope(|scope| {
            let a = state.clone(); scope.spawn(move || { for _ in 0..8 { a.wake(); } });
            let a = state.clone(); scope.spawn(move || { a.exit(); });
        });
        assert_eq!(state.load(Ordering::Acquire), 3);
        assert!(!state.resume_attached(0));
    }
}

#[test]
fn concurrent_duplicate_admission_has_exactly_one_token() {
    for _ in 0..128 {
        let state = Arc::new(TaskOwnership::new());
        state.activate();
        let queue = Arc::new(BoundedQueue::<u8, 2>::new());
        let barrier = Arc::new(Barrier::new(9));
        let count = AtomicU64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let (s, q, b, count) = (state.clone(), queue.clone(), barrier.clone(), &count);
                scope.spawn(move || {
                    s.wake(); b.wait();
                    if s.try_admit(|| q.try_push(1usize as *mut u8)) == Admission::Published {
                        count.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
            barrier.wait();
        });
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(!queue.steal().is_null());
        assert!(queue.steal().is_null());
        assert!(state.begin_dispatch(0));
        assert!(state.attach(0));
    }
}

#[test]
fn context_payload_visible_after_release_admission_and_dispatch() {
    struct Shared { state: TaskOwnership, payload: AtomicU64, queue: BoundedQueue<u8, 2> }
    let shared = Arc::new(Shared { state: attached(0), payload: AtomicU64::new(0), queue: BoundedQueue::new() });
    let reader = shared.clone();
    let writer = shared.clone();
    const ROUNDS: u64 = 10_000;
    let consumer = std::thread::spawn(move || {
        for expected in 1..=ROUNDS {
            while reader.queue.steal().is_null() { std::hint::spin_loop(); }
            assert!(reader.state.begin_dispatch(1));
            assert_eq!(reader.payload.load(Ordering::Relaxed), expected);
            assert!(reader.state.attach(1));
            assert!(reader.state.begin_switch_out(1));
            unsafe { assert!(reader.state.release_after_stack_switch(1)); }
        }
    });
    for value in 1..=ROUNDS {
        if value > 1 {
            while !writer.state.needs_admission() { std::hint::spin_loop(); }
            // The prior consumer released ownership; reserve/attach the producer.
            assert_eq!(writer.state.try_admit(|| Ok::<(), ()>(())), Admission::Published);
            assert!(writer.state.begin_dispatch(0));
            assert!(writer.state.attach(0));
        }
        assert!(writer.state.begin_switch_out(0));
        writer.payload.store(value, Ordering::Relaxed); // simulated final context store
        unsafe { assert!(writer.state.release_after_stack_switch(0)); }
        assert_eq!(writer.state.try_admit(|| writer.queue.try_push(1usize as *mut u8)), Admission::Published);
    }
    consumer.join().unwrap();
}

#[test]
fn remote_admitter_acquires_final_context_stores_from_owner_release() {
    struct Shared { state: TaskOwnership, payload: AtomicU64, turns: AtomicU64 }
    let shared = Arc::new(Shared { state: attached(0), payload: AtomicU64::new(0), turns: AtomicU64::new(0) });
    let remote = shared.clone();
    const ROUNDS: u64 = 10_000;
    let worker = std::thread::spawn(move || {
        for expected in 1..=ROUNDS {
            // The turn counter is relaxed and carries no context-publication
            // fence: visibility must come from Detached observation/admission.
            while remote.turns.load(Ordering::Relaxed) != expected * 2 - 1 { std::hint::spin_loop(); }
            loop {
                let admitted = remote.state.try_admit(|| {
                    assert_eq!(remote.payload.load(Ordering::Relaxed), expected);
                    Ok::<(), ()>(())
                });
                if admitted == Admission::Published { break; }
                std::hint::spin_loop();
            }
            assert!(remote.state.begin_dispatch(1));
            assert!(remote.state.attach(1));
            assert!(remote.state.begin_switch_out(1));
            unsafe { assert!(remote.state.release_after_stack_switch(1)); }
            remote.turns.store(expected * 2, Ordering::Relaxed);
        }
    });
    for value in 1..=ROUNDS {
        if value > 1 {
            while shared.turns.load(Ordering::Relaxed) != (value - 1) * 2 { std::hint::spin_loop(); }
            assert_eq!(shared.state.try_admit(|| Ok::<(), ()>(())), Admission::Published);
            assert!(shared.state.begin_dispatch(0));
            assert!(shared.state.attach(0));
        }
        assert!(shared.state.begin_switch_out(0));
        shared.payload.store(value, Ordering::Relaxed);
        unsafe { assert!(shared.state.release_after_stack_switch(0)); }
        shared.turns.store(value * 2 - 1, Ordering::Relaxed);
    }
    worker.join().unwrap();
}
