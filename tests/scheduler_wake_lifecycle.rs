// SPDX-License-Identifier: GPL-3.0-only
// Shared assertions for the extracted pre-fix production wake body and the
// actual production helper. The harness provides wake(&AtomicU32).
use std::sync::atomic::{AtomicU32, Ordering};

#[test]
fn wake_preserves_running_for_tick_requeue() {
    let state = AtomicU32::new(1);
    super::wake(&state);
    assert_eq!(state.load(Ordering::Acquire), 1);
}

#[test]
fn wake_never_resurrects_exited() {
    let state = AtomicU32::new(3);
    super::wake(&state);
    assert_eq!(state.load(Ordering::Acquire), 3);
}

#[test]
fn wake_marks_blocked_ready() {
    let state = AtomicU32::new(2);
    super::wake(&state);
    assert_eq!(state.load(Ordering::Acquire), 0);
}

#[test]
fn repeated_wake_leaves_ready_unchanged() {
    let state = AtomicU32::new(0);
    for _ in 0..32 { super::wake(&state); }
    assert_eq!(state.load(Ordering::Acquire), 0);
}

#[test]
fn invalid_lifecycle_is_not_made_runnable() {
    for value in [4, 7, u32::MAX] {
        let state = AtomicU32::new(value);
        super::wake(&state);
        assert_eq!(state.load(Ordering::Acquire), value);
    }
}

#[test]
fn exit_before_wake_remains_terminal() {
    let state = AtomicU32::new(2);
    state.store(3, Ordering::Release);
    super::wake(&state);
    assert_eq!(state.load(Ordering::Acquire), 3);
}

#[test]
fn exit_after_wake_remains_terminal() {
    let state = AtomicU32::new(2);
    super::wake(&state);
    state.store(3, Ordering::Release);
    super::wake(&state);
    assert_eq!(state.load(Ordering::Acquire), 3);
}

#[test]
fn repeated_wakes_do_not_reclassify_an_attached_running_task() {
    let state = AtomicU32::new(1);
    for _ in 0..32 { super::wake(&state); }
    // tick's existing outgoing-task rule requeues precisely state == Running.
    assert!(state.load(Ordering::Acquire) == 1, "tick would orphan this task");
}
