// SPDX-License-Identifier: GPL-3.0-only
//! Model of the scheduler's park/wake/handoff custody (kernel pass 2, K6). Built from
//! the real kernel sources: kernel/src/task_ownership.rs (the one custody word) and
//! kernel/src/self_wait.rs (timed-wait deadlines and the BOOT inventory).
//!
//! - Every state the custody word can reach from `new()` under every operation the
//!   scheduler performs, on two cores, with the run-queue token counted, is visited
//!   (breadth-first, no depth bound). Each state must keep the custody invariants, and
//!   from each one a wake must lead to the task running again, a fault wait must hold
//!   until the pager completes it, and an exit must end in Dead with nothing dispatched.
//! - Deadlines and the BOOT inventory are checked against small reference models over
//!   every short operation sequence.
//! - `SCHEDULER_PROTOCOL` lists the custody and deadline calls each scheduler.rs
//!   function makes. scripts/check_self_wait.py requires kernel/src/scheduler.rs to
//!   make exactly these, so a protocol change has to come here, next to its models.
//!   The loom harnesses (tests/model_loom_scheduler.rs) follow these sequences, and
//!   tests/model_kani.rs proves the invariants over symbolic operation sequences.
//!
//! These replace the self-wait source pins of the three files above. Host-only:
//! rustc --edition=2021 --test tests/model_scheduler_wait.rs (scripts/scheduler_model_gate.sh)
#![allow(dead_code)]
#[path = "../kernel/src/task_ownership.rs"]
mod ownership;
#[path = "../kernel/src/self_wait.rs"]
mod self_wait;

use ownership::{Admission, ParkResult, Phase, Snapshot, TaskOwnership, WakeResult};
use std::collections::{HashMap, VecDeque};

/// What each scheduler.rs function that waits or changes task custody calls, in
/// source order with its match arms: custody-word operations (snapshots, which only
/// read, are left out), deadline and BOOT-inventory calls, interrupt masking,
/// admission and the wait entry points. One function per line.
pub const SCHEDULER_PROTOCOL: &str = "
tick: state.begin_dispatch state.exit state.cancel_dispatch state.attach state.begin_switch_out
admit: Admission::NotRunnable state.try_admit Admission::Published Admission::Published Admission::Published
activate_task: state.activate .admit_task
retry_admission: state.needs_admission .admit_task
proof_kernel_timer_wake: unpark_thread
scheduler_finish_switch: state.release_after_stack_switch .admit
leave_dead_task: state.begin_switch_out
wait_for_local_resume_or_switch: interrupts::disable state.resume_attached
wait_for_message: interrupts::disable state.park ParkResult::WakeConsumed ParkResult::Blocked yield_and_switch ParkResult::Exited yield_and_switch ParkResult::NotOwner
park_current_unless: interrupts::disable state.park ParkResult::WakeConsumed ParkResult::Blocked yield_and_switch ParkResult::Exited yield_and_switch ParkResult::NotOwner
wait_for_message_ticks: interrupts::disable SELF_WAIT_TASKS.contains wait_deadline.cancel wait_deadline.arm state.park ParkResult::WakeConsumed wait_deadline.cancel ParkResult::Blocked yield_and_switch ParkResult::Exited wait_deadline.cancel yield_and_switch ParkResult::NotOwner wait_deadline.cancel
poll_self_wait_deadlines: SELF_WAIT_TASKS.visit wait_deadline.take_due wake_owned_thread
wake_record: wait_deadline.cancel wake_owned_thread
wake_record_if_due: wait_deadline.take_due wake_owned_thread
park_current_thread: state.park
unpark_thread: wait_deadline.cancel wake_owned_thread
unpark_for_call: unpark_thread
wake_owned_thread: state.wake WakeResult::Recorded WakeResult::NeedsAdmission .admit_task Admission::Published
resume_resolved_fault: state.complete_fault .admit_task
seal_self_wait_inventory: SELF_WAIT_TASKS.register SELF_WAIT_TASKS.seal
finish_current_task: interrupts::disable wait_deadline.cancel state.exit yield_and_switch
yield_and_switch: interrupts::disable wait_for_local_resume_or_switch
";

const CORES: [u32; 2] = [0, 1];

/// The operations the scheduler performs on a task's custody word. `Pop` is a
/// dispatcher taking the run-queue token and calling begin_dispatch, the only way
/// the scheduler calls it. `Admit(false)` is a full queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Activate,
    Admit(bool),
    Pop(u32),
    Attach(u32),
    CancelDispatch(u32),
    Park(u32),
    ParkFault(u32),
    CompleteFault,
    Resume(u32),
    SwitchOut(u32),
    Release(u32),
    Wake,
    Exit,
}

fn ops() -> Vec<Op> {
    let mut all = vec![Op::Activate, Op::Admit(true), Op::Admit(false), Op::CompleteFault, Op::Wake, Op::Exit];
    for c in CORES {
        all.extend([Op::Pop(c), Op::Attach(c), Op::CancelDispatch(c), Op::Park(c), Op::ParkFault(c),
                    Op::Resume(c), Op::SwitchOut(c), Op::Release(c)]);
    }
    all
}

/// A task's custody word plus the run-queue tokens that point at it.
struct World {
    t: TaskOwnership,
    tokens: u32,
}

/// The whole custody word (every bit is in the snapshot) plus the token count.
type Key = (u32, bool, bool, bool, bool, Option<u32>, u32);

fn key(s: Snapshot, tokens: u32) -> Key {
    (s.phase as u32, s.pending, s.blocked, s.fault_blocked, s.exited, s.owner_core, tokens)
}

impl World {
    fn new() -> Self { Self { t: TaskOwnership::new(), tokens: 0 } }
    fn snap(&self) -> Snapshot { self.t.snapshot() }
    fn key(&self) -> Key { key(self.snap(), self.tokens) }
    /// One publication, wherever it lands (a queue, or a handoff slot on this core or
    /// the task's home core): the kernel's three Published branches only choose which
    /// interrupt follows (preempt, preempt_for_partner, kick), not custody.
    fn admit(&mut self, room: bool) -> Admission {
        let tokens = &mut self.tokens;
        self.t.try_admit(|| if room { *tokens += 1; Ok(()) } else { Err(()) })
    }
    /// Applies one operation; the result is folded to a number for the checks.
    fn apply(&mut self, op: Op) -> u32 {
        match op {
            Op::Activate => self.t.activate() as u32,
            Op::Admit(room) => self.admit(room) as u32,
            Op::Pop(c) => {
                if self.tokens == 0 { return 2; }
                self.tokens -= 1;
                self.t.begin_dispatch(c) as u32
            }
            Op::Attach(c) => self.t.attach(c) as u32,
            Op::CancelDispatch(c) => self.t.cancel_dispatch(c) as u32,
            Op::Park(c) => self.t.park(c) as u32,
            Op::ParkFault(c) => self.t.park_fault(c) as u32,
            Op::CompleteFault => (unsafe { self.t.complete_fault() }) as u32,
            Op::Resume(c) => self.t.resume_attached(c) as u32,
            Op::SwitchOut(c) => self.t.begin_switch_out(c) as u32,
            Op::Release(c) => (unsafe { self.t.release_after_stack_switch(c) }) as u32,
            Op::Wake => self.t.wake() as u32,
            Op::Exit => self.t.exit() as u32,
        }
    }
}

fn replay(path: &[Op]) -> World {
    let mut w = World::new();
    for &op in path { w.apply(op); }
    w
}

/// Every reachable state, with one shortest path to it.
fn reachable() -> Vec<Vec<Op>> {
    let ops = ops();
    let mut seen: HashMap<Key, ()> = HashMap::new();
    let mut order = Vec::new();
    let mut queue = VecDeque::from([Vec::new()]);
    seen.insert(World::new().key(), ());
    while let Some(path) = queue.pop_front() {
        for &op in &ops {
            let mut w = replay(&path);
            w.apply(op);
            // A broken invariant stops the search here, so a bug that grows state
            // without bound (a second queue token, say) still ends the model.
            invariants(w.snap(), w.tokens).unwrap_or_else(|e| panic!("{e}: {path:?} then {op:?}"));
            if seen.insert(w.key(), ()).is_none() {
                let mut next = path.clone();
                next.push(op);
                queue.push_back(next);
            }
        }
        order.push(path);
    }
    order
}

/// Custody invariants of one state.
fn invariants(s: Snapshot, tokens: u32) -> Result<(), &'static str> {
    let owned = matches!(s.phase, Phase::Dispatching | Phase::Attached | Phase::SwitchingOut);
    if tokens > 1 { return Err("two run-queue tokens for one task"); }
    if (tokens == 1) != (s.phase == Phase::Queued) { return Err("run-queue token without Queued, or Queued without its token"); }
    if owned != s.owner_core.is_some() { return Err("owner core recorded outside an owned phase, or missing in one"); }
    if s.phase == Phase::Dead && !s.exited { return Err("Dead without exit"); }
    if s.exited && (s.pending || s.blocked || s.fault_blocked) { return Err("an exited task keeps a wake or a wait"); }
    if s.fault_blocked && !s.blocked { return Err("fault wait without a block"); }
    if matches!(s.phase, Phase::Queued | Phase::Dispatching) && s.blocked { return Err("a queued or dispatching task still blocked"); }
    if s.phase == Phase::Constructed && s.blocked { return Err("a constructed task blocked"); }
    Ok(())
}

/// Drives the scheduler's own steps (no wake, no exit) until the task runs, at most
/// a few steps. Returns false if it gets stuck: a lost wake.
fn runs_again(w: &mut World) -> bool {
    for _ in 0..6 {
        let s = w.snap();
        match (s.phase, s.owner_core) {
            (Phase::Attached, Some(c)) => return w.t.resume_attached(c),
            (Phase::SwitchingOut, Some(c)) => { if !unsafe { w.t.release_after_stack_switch(c) } { return false; } }
            (Phase::Detached, _) => {
                if !w.t.needs_admission() || w.admit(true) != Admission::Published { return false; }
            }
            (Phase::Queued, _) => { w.tokens -= 1; if !w.t.begin_dispatch(0) { return false; } }
            (Phase::Dispatching, Some(c)) => { if !w.t.attach(c) { return false; } }
            _ => return false,
        }
    }
    false
}

/// Drives custody release after an exit. Returns false if a dead task is dispatched
/// or custody never ends.
fn dies(w: &mut World) -> bool {
    for _ in 0..4 {
        let s = w.snap();
        match (s.phase, s.owner_core) {
            (Phase::Dead, _) => return w.tokens == 0,
            (Phase::Queued, _) => { w.tokens -= 1; if w.t.begin_dispatch(0) { return false; } }
            (Phase::Dispatching, Some(c)) => { if w.t.attach(c) { return false; } }
            (Phase::Attached, Some(c)) => { if !w.t.begin_switch_out(c) { return false; } }
            (Phase::SwitchingOut, Some(c)) => { if !unsafe { w.t.release_after_stack_switch(c) } { return false; } }
            _ => return false,
        }
    }
    false
}

#[test]
fn every_reachable_custody_state_keeps_the_invariants() {
    let states = reachable();
    assert!(states.len() > 30, "the model explores the custody graph");
    let ops = ops();
    for path in &states {
        let w = replay(path);
        let (before, tokens) = (w.snap(), w.tokens);
        invariants(before, tokens).unwrap_or_else(|e| panic!("{e}: {path:?}"));
        for &op in &ops {
            let mut next = replay(path);
            let result = next.apply(op);
            let after = next.snap();
            invariants(after, next.tokens).unwrap_or_else(|e| panic!("{e}: {path:?} then {op:?}"));
            assert!(!before.exited || after.exited, "exit undone: {path:?} then {op:?}");
            if before.phase == Phase::Dead {
                assert_eq!(key(after, next.tokens), key(before, tokens), "Dead changed: {path:?} then {op:?}");
            }
            if let Op::Pop(_) = op {
                if result == 1 { assert!(!before.exited, "a dead task dispatched: {path:?}"); }
            }
            if op == Op::Wake && before.phase == Phase::Constructed {
                assert!(!next.t.needs_admission(), "a wake activated a constructed task: {path:?}");
            }
        }
    }
}

#[test]
fn a_wake_is_never_lost_from_any_state() {
    for path in reachable() {
        let mut w = replay(&path);
        let s = w.snap();
        if s.exited || s.fault_blocked || s.phase == Phase::Constructed { continue; }
        let woke = w.t.wake();
        assert_eq!(woke == WakeResult::NeedsAdmission, s.phase == Phase::Detached, "wake result: {path:?}");
        assert!(runs_again(&mut w), "a wake was lost: {path:?}");
    }
}

#[test]
fn park_consumes_exactly_the_pending_wake() {
    for path in reachable() {
        let w = replay(&path);
        let s = w.snap();
        let (Phase::Attached, Some(c)) = (s.phase, s.owner_core) else { continue };
        if s.exited || s.fault_blocked { continue; }
        let parked = w.t.park(c);
        let after = w.snap();
        assert_eq!(parked == ParkResult::WakeConsumed, s.pending, "park result: {path:?}");
        assert!(!after.pending, "a pending wake survived park: {path:?}");
        assert_eq!(after.blocked, parked == ParkResult::Blocked, "block state: {path:?}");
        // A second park with no wake in between always blocks.
        if parked == ParkResult::WakeConsumed { assert_eq!(w.t.park(c), ParkResult::Blocked); }
    }
}

#[test]
fn a_fault_wait_holds_until_the_pager_completes_it() {
    for path in reachable() {
        let mut w = replay(&path);
        let s = w.snap();
        if !s.fault_blocked || s.exited { continue; }
        w.t.wake();
        assert!(!w.t.needs_admission(), "an ordinary wake admitted a faulted task: {path:?}");
        if let (Phase::Attached, Some(c)) = (s.phase, s.owner_core) {
            assert!(!w.t.resume_attached(c), "an ordinary wake resumed a faulted task: {path:?}");
        }
        assert!(unsafe { w.t.complete_fault() }, "the pager could not complete: {path:?}");
        assert!(runs_again(&mut w), "a completed fault did not run again: {path:?}");
    }
}

#[test]
fn an_exit_from_any_state_ends_in_dead_without_dispatch() {
    for path in reachable() {
        let mut w = replay(&path);
        let was = w.snap().exited;
        assert_eq!(w.t.exit(), !was, "exit result: {path:?}");
        assert!(dies(&mut w), "exit did not end custody: {path:?}");
        assert_eq!(w.t.wake(), WakeResult::Exited);
        assert_eq!(w.admit(true), Admission::NotRunnable);
    }
}

#[test]
fn a_task_reaches_every_custody_phase() {
    let phases: std::collections::HashSet<u32> = reachable().iter().map(|p| replay(p).snap().phase as u32).collect();
    assert_eq!(phases.len(), 7, "all seven phases reachable: {phases:?}");
}

/// Reference timed-wait deadline: None, or the tick it falls due.
#[test]
fn deadlines_match_the_reference_over_every_short_sequence() {
    #[derive(Clone, Copy, Debug)]
    enum D { Arm(u64, u64), Cancel, Take(u64) }
    let mut alphabet = vec![D::Cancel];
    for now in [0, 1, 3, 70, u64::MAX - 1] {
        for ticks in [0, 1, 2, 64, 65] { alphabet.push(D::Arm(now, ticks)); }
    }
    for now in [0, 1, 2, 3, 66, 134, u64::MAX] { alphabet.push(D::Take(now)); }
    let n = alphabet.len();
    for code in 0..n * n * n {
        let seq = [alphabet[code % n], alphabet[code / n % n], alphabet[code / n / n % n]];
        let d = self_wait::Deadline::new();
        let mut reference: Option<u64> = None;
        for op in seq {
            match op {
                D::Arm(now, ticks) => {
                    let want = if reference.is_none() && (1..=self_wait::MAX_TICKS).contains(&ticks) { now.checked_add(ticks) } else { None };
                    assert_eq!(d.arm(now, ticks), want, "{seq:?}");
                    if want.is_some() { reference = want; }
                }
                D::Cancel => { d.cancel(); reference = None; }
                D::Take(now) => {
                    let want = reference.filter(|&at| now >= at);
                    assert_eq!(d.take_due(now), want, "{seq:?}");
                    if want.is_some() { reference = None; }
                }
            }
        }
    }
}

#[test]
fn boot_inventory_matches_the_reference_over_every_short_sequence() {
    #[derive(Clone, Copy, Debug)]
    enum B { Register(usize, usize), Seal }
    let mut cells = [10u8, 11, 12];
    let ptrs: Vec<*mut u8> = cells.iter_mut().map(|c| c as *mut u8).collect();
    let mut alphabet = vec![B::Seal];
    for index in 0..4 { for task in 0..4 { alphabet.push(B::Register(index, task)); } }
    let n = alphabet.len();
    for code in 0..n * n * n {
        let seq = [alphabet[code % n], alphabet[code / n % n], alphabet[code / n / n % n]];
        let inv = self_wait::BootInventory::<u8, 3>::new();
        let (mut slots, mut sealed) = ([None::<usize>; 3], false);
        let pointer = |task: usize| if task < 3 { ptrs[task] } else { std::ptr::null_mut() };
        for op in seq {
            match op {
                B::Seal => { assert_eq!(inv.seal(), !sealed, "{seq:?}"); sealed = true; }
                B::Register(index, task) => {
                    let want = !sealed && index < 3 && task < 3 && slots[index].is_none() && !slots.contains(&Some(task));
                    assert_eq!(inv.register(index, pointer(task)), want, "{seq:?}");
                    if want { slots[index] = Some(task); }
                }
            }
        }
        let mut seen = Vec::new();
        inv.visit(|p| seen.push(p));
        let want: Vec<*mut u8> = if sealed { slots.iter().flatten().map(|&t| ptrs[t]).collect() } else { Vec::new() };
        assert_eq!(seen, want, "{seq:?}");
        for task in 0..4 {
            assert_eq!(inv.contains(pointer(task)), sealed && slots.contains(&Some(task)), "{seq:?}");
        }
    }
}

#[test]
fn the_protocol_table_names_each_function_once() {
    let mut names = std::collections::HashSet::new();
    for line in SCHEDULER_PROTOCOL.lines().filter(|l| !l.trim().is_empty()) {
        let (name, calls) = line.split_once(':').expect("name: calls");
        assert!(names.insert(name.trim()), "{name} listed twice");
        assert!(!calls.trim().is_empty(), "{name} lists no calls");
    }
    assert!(names.contains("wait_for_message_ticks") && names.contains("wake_owned_thread"));
}
