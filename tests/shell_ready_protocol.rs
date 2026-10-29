//! Execute the actual startup protocol with deterministic syscall boundary doubles.
#[cfg(not(shell_ready_baseline))]
#[path = "../crates/sex-pdx/src/shell_ready.rs"]
mod shell_ready;

#[cfg(not(shell_ready_baseline))]
use shell_ready::*;
use std::cell::RefCell;
use std::collections::VecDeque;

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

#[cfg(not(shell_ready_baseline))]
mod protocol_tests {
use super::*;
fn ready() -> PdxMessage {
    PdxMessage { type_id: OP_SHELL_READY, arg0: SHELL_READY_VERSION,
        arg1: 0, arg2: 0, caller_pd: SHELL_PD_ID }
}
fn ordinary(id: u64) -> PdxMessage {
    PdxMessage { type_id: 0x260, arg0: id, arg1: id + 1,
        arg2: id + 2, caller_pd: 5 }
}

#[test]
fn readiness_requires_authenticated_shell_and_exact_versioned_shape() {
    assert!(is_shell_ready(&ready()));
    for msg in [
        PdxMessage { caller_pd: 0, ..ready() },
        PdxMessage { caller_pd: 1, ..ready() },
        PdxMessage { type_id: 0, ..ready() },
        PdxMessage { type_id: 1, ..ready() },
        PdxMessage { arg0: 0, ..ready() },
        PdxMessage { arg0: SHELL_READY_VERSION + 1, ..ready() },
        PdxMessage { arg1: 1, ..ready() },
        PdxMessage { arg2: 1, ..ready() },
    ] { assert!(!is_shell_ready(&msg), "accepted {msg:?}"); }
}

#[test]
fn wait_preserves_unrelated_fifo_and_rejects_forged_ready() {
    BOUNDARY.with(|state| state.borrow_mut().incoming.extend([
        ordinary(10), PdxMessage { caller_pd: 9, ..ready() }, ordinary(20), ready(), ordinary(30),
    ]));
    let mut inbox = StartupInbox::<4>::new();
    assert_eq!(inbox.wait_for_shell(), Ok(()));
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
    assert_eq!(inbox.wait_for_shell(), Err(WaitError::InboxFull));
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
fn publisher_retries_failure_and_never_duplicates_success() {
    BOUNDARY.with(|state| state.borrow_mut().outcomes.extend([
        Err(ERR_SERVICE_NOT_READY), Err(ERR_CAP_INVALID), Ok(0),
    ]));
    let mut publisher = ReadyPublisher::new();
    assert!(publisher.is_pending());
    assert!(!publisher.poll());
    assert!(!publisher.poll());
    assert!(publisher.poll());
    assert!(!publisher.is_pending());
    assert!(!publisher.poll());
    BOUNDARY.with(|state| {
        assert_eq!(state.borrow().calls, [[SLOT_SHELL_INPUT_READY, OP_SHELL_READY, SHELL_READY_VERSION, 0, 0]; 3]);
        assert_eq!(state.borrow().yields, 0);
    });
}

#[test]
fn full_target_bounds_each_poll_and_allows_other_messages_to_drain() {
    BOUNDARY.with(|state| {
        let mut state = state.borrow_mut();
        state.outcomes.extend([Err(ERR_SERVICE_NOT_READY); 4]);
        state.incoming.extend([ordinary(1), ordinary(2), ordinary(3), ordinary(4)]);
    });
    let mut publisher = ReadyPublisher::new();
    for n in 1..=4 {
        assert!(!publisher.poll());
        // The publisher never waits inside poll; the owner's normal receive
        // can progress even when its notification target is still full.
        assert_eq!(pdx_try_listen_raw(0), Some(ordinary(n)));
        BOUNDARY.with(|state| assert_eq!(state.borrow().calls.len(), n as usize));
    }
    assert!(publisher.poll());
    assert!(!publisher.poll());
}

#[test]
fn invalid_controls_cannot_unlock_wait_or_consume_ordinary_capacity() {
    BOUNDARY.with(|state| state.borrow_mut().incoming.extend([
        PdxMessage { caller_pd: 1, ..ready() },
        PdxMessage { arg0: 2, ..ready() },
        PdxMessage { arg1: 1, ..ready() },
        PdxMessage { arg2: 1, ..ready() },
        ordinary(12), ready(),
    ]));
    let mut inbox = StartupInbox::<2>::new();
    assert_eq!(inbox.wait_for_shell(), Ok(()));
    assert_eq!(inbox.pop(), Some(ordinary(12)));
    assert_eq!(inbox.pop(), None);
    BOUNDARY.with(|state| assert_eq!(state.borrow().yields, 6));
}

#[test]
fn pre_ready_usb_keyboard_and_pointer_payloads_are_retained_exactly() {
    let pointer = PdxMessage { type_id: 0x260, arg0: u64::MAX, arg1: 0x8000_0001,
        arg2: 1 << 32, caller_pd: 5 };
    let key = PdxMessage { type_id: 0x261, arg0: 0x1234, arg1: 0xabcd,
        arg2: u64::MAX, caller_pd: 5 };
    BOUNDARY.with(|state| state.borrow_mut().incoming.extend([pointer, key, ready()]));
    let mut inbox = StartupInbox::<256>::new();
    assert_eq!(inbox.wait_for_shell(), Ok(()));
    assert_eq!(inbox.pop(), Some(pointer));
    assert_eq!(inbox.pop(), Some(key));
    assert_eq!(inbox.pop(), None);
    assert_eq!(STARTUP_INBOX_CAPACITY, 256);
    assert_eq!(INPUT_PD_ID, 4);
}
}

#[test]
fn input_startup_wait_precedes_all_synthetic_and_real_producers() {
    let source = include_str!("../servers/sexinput/src/main.rs");
    let start = source.split("pub extern \"C\" fn _start()").nth(1).unwrap();
    let wait = start.find("wait_for_shell()").expect("missing authenticated startup wait");
    assert!(wait < start.find("loop {").unwrap());
    assert!(wait < start.find("pdx_call(SLOT_SHELL, OP_HID_EVENT").unwrap());
    assert!(start.contains("pop_startup_message().or_else(|| pdx_try_listen_raw(0))"));
    assert_eq!(source.matches("pdx_try_listen_raw(0)").count(), 1);
    assert!(start.contains("[shell.ready.wait.failed] reason=inbox_full"));
}

#[test]
fn publisher_starts_after_full_shell_ready_and_retries_in_main_loop() {
    let source = include_str!("../servers/silk-shell/src/main.rs");
    let start = source.split("pub extern \"C\" fn _start()").nth(1).unwrap();
    let ready = start.find("[silkshell.ready]").unwrap();
    let publisher = start.find("sex_pdx::shell_ready::ReadyPublisher::new()").expect("missing ready publisher");
    let main_loop = start[publisher..].find("loop {").unwrap() + publisher;
    let poll = start.find("shell_ready_publisher.poll()").unwrap();
    assert!(ready < publisher && publisher < main_loop && main_loop < poll);
    assert!(poll < start.find("if !startup_messages_pending() {").unwrap());
    let idle = start.split("unsafe { flush_pending_cursor_send(); }").nth(1).unwrap();
    assert!(idle.find("if shell_ready_publisher.is_pending()").unwrap()
        < idle.find("pdx_listen_raw(0)").unwrap());
    assert!(idle.contains("sys_yield();\n                    continue;"));
}

#[test]
fn reverse_edge_is_granted_at_boot_with_identity_and_collision_checks() {
    let source = include_str!("../kernel/src/init.rs");
    let grants = source.split("[bootgraph.phase25.begin]").nth(1).unwrap()
        .split("[bootgraph.phase25.complete]").next().unwrap();
    assert!(grants.contains("assert_eq!(silkshell_id, sex_pdx::shell_ready::SHELL_PD_ID"));
    assert!(grants.contains("assert_eq!(sexinput_id, sex_pdx::shell_ready::INPUT_PD_ID"));
    assert!(grants.contains("find_capability(sex_pdx::shell_ready::SLOT_SHELL_INPUT_READY as u32).is_none()"));
    assert!(grants.contains("pd.grant_capability(sex_pdx::shell_ready::SLOT_SHELL_INPUT_READY, crate::capability::CapabilityData::Domain(sexinput_id))"));
}
