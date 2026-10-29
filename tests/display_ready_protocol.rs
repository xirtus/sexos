//! Execute the actual startup protocol with deterministic syscall boundary doubles.
#[path = "../crates/sex-pdx/src/display_ready.rs"]
mod display_ready;

use display_ready::*;
use std::cell::RefCell;
use std::collections::VecDeque;

pub const SLOT_DISPLAY: u64 = 5;
pub const SLOT_SHELL: u64 = 6;
pub const SLOT_SILKBAR: u64 = 7;
pub const ERR_SERVICE_NOT_READY: u64 = u64::MAX - 1;
const ERR_CAP_INVALID: u64 = u64::MAX - 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdxMessage {
    pub type_id: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub caller_pd: u32,
}

#[derive(Default)]
struct Boundary {
    incoming: VecDeque<PdxMessage>,
    outcomes: VecDeque<Result<u64, u64>>,
    calls: Vec<[u64; 5]>,
    yields: usize,
}
thread_local! { static BOUNDARY: RefCell<Boundary> = RefCell::new(Boundary::default()); }

pub fn pdx_call_checked(slot: u64, op: u64, a0: u64, a1: u64, a2: u64) -> Result<u64, u64> {
    BOUNDARY.with(|state| {
        let mut state = state.borrow_mut();
        state.calls.push([slot, op, a0, a1, a2]);
        state.outcomes.pop_front().unwrap_or(Ok(0))
    })
}
pub fn pdx_try_listen_raw(slot: u64) -> Option<PdxMessage> {
    assert_eq!(slot, 0);
    BOUNDARY.with(|state| state.borrow_mut().incoming.pop_front())
}
pub fn sys_yield() {
    BOUNDARY.with(|state| {
        let mut state = state.borrow_mut();
        state.yields += 1;
        assert!(state.yields <= 1000, "startup wait stopped making progress");
    });
}
fn ready() -> PdxMessage {
    PdxMessage { type_id: OP_DISPLAY_READY, arg0: DISPLAY_READY_VERSION,
        arg1: 0, arg2: 0, caller_pd: DISPLAY_PD_ID }
}
fn ordinary(id: u64) -> PdxMessage {
    PdxMessage { type_id: 0x202, arg0: id, arg1: id + 1,
        arg2: id + 2, caller_pd: 4 }
}

#[test]
fn readiness_requires_authenticated_display_and_exact_versioned_shape() {
    assert!(is_display_ready(&ready()));
    for msg in [
        PdxMessage { caller_pd: 0, ..ready() },
        PdxMessage { caller_pd: 3, ..ready() },
        PdxMessage { type_id: 0, ..ready() },
        PdxMessage { type_id: 1, ..ready() },
        PdxMessage { arg0: 0, ..ready() },
        PdxMessage { arg0: DISPLAY_READY_VERSION + 1, ..ready() },
        PdxMessage { arg1: 1, ..ready() },
        PdxMessage { arg2: 1, ..ready() },
    ] { assert!(!is_display_ready(&msg), "accepted {msg:?}"); }
}

#[test]
fn wait_preserves_unrelated_fifo_and_rejects_forged_ready() {
    BOUNDARY.with(|state| state.borrow_mut().incoming.extend([
        ordinary(10), PdxMessage { caller_pd: 9, ..ready() }, ordinary(20), ready(), ordinary(30),
    ]));
    let mut inbox = StartupInbox::<4>::new();
    assert_eq!(inbox.wait_for_display(), Ok(()));
    assert!(!inbox.is_empty());
    assert_eq!(inbox.pop(), Some(ordinary(10)));
    assert_eq!(inbox.pop(), Some(ordinary(20)));
    assert_eq!(inbox.pop(), None);
    assert!(inbox.is_empty());
    assert_eq!(pdx_try_listen_raw(0), Some(ordinary(30)));
    BOUNDARY.with(|state| assert_eq!(state.borrow().yields, 4));
}

#[test]
fn full_wait_fails_before_dequeue_and_keeps_all_deferred_work() {
    BOUNDARY.with(|state| state.borrow_mut().incoming.extend([ordinary(1), ordinary(2), ready()]));
    let mut inbox = StartupInbox::<2>::new();
    assert_eq!(inbox.wait_for_display(), Err(WaitError::InboxFull));
    assert_eq!(inbox.pop(), Some(ordinary(1)));
    assert_eq!(inbox.pop(), Some(ordinary(2)));
    assert_eq!(pdx_try_listen_raw(0), Some(ready()));
}

#[test]
fn overflow_returns_message_and_fifo_wrap_preserves_order() {
    let mut inbox = StartupInbox::<2>::new();
    assert_eq!(inbox.observe(ordinary(1)), Ok(Observe::Deferred));
    assert_eq!(inbox.observe(ordinary(2)), Ok(Observe::Deferred));
    assert_eq!(inbox.observe(ordinary(3)), Err(ordinary(3)));
    assert_eq!(inbox.pop(), Some(ordinary(1)));
    assert_eq!(inbox.observe(ordinary(3)), Ok(Observe::Deferred));
    assert_eq!(inbox.pop(), Some(ordinary(2)));
    assert_eq!(inbox.pop(), Some(ordinary(3)));
    assert_eq!(inbox.pop(), None);
}

#[test]
fn duplicate_ready_is_idempotent_without_occupying_deferred_capacity() {
    let mut inbox = StartupInbox::<1>::new();
    assert_eq!(inbox.observe(ready()), Ok(Observe::Ready));
    assert_eq!(inbox.observe(ready()), Ok(Observe::Ready));
    assert_eq!(inbox.observe(ordinary(1)), Ok(Observe::Deferred));
    assert_eq!(inbox.pop(), Some(ordinary(1)));
}

#[test]
fn publisher_retries_only_failed_recipient_and_never_duplicates_success() {
    BOUNDARY.with(|state| state.borrow_mut().outcomes.extend([
        Err(ERR_SERVICE_NOT_READY), Ok(0), Ok(0),
    ]));
    let mut publisher = ReadyPublisher::new();
    assert_eq!(publisher.poll(), 0b10);
    assert_eq!(publisher.poll(), 0b01);
    assert_eq!(publisher.poll(), 0);
    BOUNDARY.with(|state| assert_eq!(state.borrow().calls, [
        [SLOT_SHELL, OP_DISPLAY_READY, DISPLAY_READY_VERSION, 0, 0],
        [SLOT_SILKBAR, OP_DISPLAY_READY, DISPLAY_READY_VERSION, 0, 0],
        [SLOT_SHELL, OP_DISPLAY_READY, DISPLAY_READY_VERSION, 0, 0],
    ]));
}

#[test]
fn saturated_publisher_does_bounded_work_without_losing_pending_notifications() {
    BOUNDARY.with(|state| state.borrow_mut().outcomes.extend([Err(ERR_SERVICE_NOT_READY); 4]));
    let mut publisher = ReadyPublisher::new();
    assert_eq!(publisher.poll(), 0);
    assert_eq!(publisher.poll(), 0);
    assert_eq!(publisher.poll(), 0b11);
    assert_eq!(publisher.poll(), 0);
    BOUNDARY.with(|state| {
        assert_eq!(state.borrow().calls.len(), 6);
        assert_eq!(state.borrow().yields, 0);
    });
}

#[test]
fn startup_send_retries_same_arguments_and_propagates_authority_failure() {
    BOUNDARY.with(|state| state.borrow_mut().outcomes.extend([
        Err(ERR_SERVICE_NOT_READY), Err(ERR_SERVICE_NOT_READY), Ok(17), Err(ERR_CAP_INVALID),
    ]));
    assert_eq!(send_startup_display(0xec, 99, 101, 103), Ok(17));
    assert_eq!(send_startup_display(0xec, 99, 101, 103), Err(ERR_CAP_INVALID));
    BOUNDARY.with(|state| {
        assert_eq!(state.borrow().calls, [[SLOT_DISPLAY, 0xec, 99, 101, 103]; 4]);
        assert_eq!(state.borrow().yields, 2);
    });
}

#[test]
fn startup_send_has_explicit_retry_exhaustion() {
    BOUNDARY.with(|state| state.borrow_mut().outcomes.extend(
        [Err(ERR_SERVICE_NOT_READY); STARTUP_SEND_ATTEMPTS]));
    assert_eq!(send_startup_display(0xec, 99, 101, 103), Err(ERR_SERVICE_NOT_READY));
    BOUNDARY.with(|state| {
        assert_eq!(state.borrow().calls.len(), STARTUP_SEND_ATTEMPTS);
        assert_eq!(state.borrow().yields, STARTUP_SEND_ATTEMPTS);
    });
    assert_eq!(STARTUP_INBOX_CAPACITY, 256);
}
