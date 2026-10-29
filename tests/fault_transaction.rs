#[path = "../kernel/src/fault_transaction.rs"]
mod fault;
#[path = "../kernel/src/task_ownership.rs"]
mod ownership;
use fault::*;
use ownership::{Admission, ParkResult, Phase, TaskOwnership};

fn record(token: u64) -> FaultRecord {
    FaultRecord { token, target_pd: 15, pager_pd: 14, task_id: 0x8015,
        fault_addr: 0x400123, page_addr: 0x400000, error_code: 6,
        saved_rip: 0x10100, saved_frame: 0x21000 }
}
fn pending() -> FaultSlot {
    let slot = FaultSlot::new();
    slot.prepare(record(1)).unwrap(); slot.publish(1).unwrap(); slot
}
fn attached() -> TaskOwnership {
    let state = TaskOwnership::new(); state.activate();
    assert_eq!(state.try_admit(|| Ok::<(), ()>(())), Admission::Published);
    assert!(state.begin_dispatch(0)); assert!(state.attach(0)); state
}

#[test]
fn prepared_notice_cannot_be_claimed_before_enqueue_acceptance() {
    let slot = FaultSlot::new(); slot.prepare(record(1)).unwrap();
    assert_eq!(slot.query(14, 1), Err(RETRY));
    assert_eq!(slot.claim(14, 1), Err(RETRY));
    slot.publish(1).unwrap();
    assert_eq!(slot.query(14, 1), Ok(record(1)));
}

#[test]
fn full_queue_cancellation_does_not_leave_a_live_request() {
    let slot = FaultSlot::new(); slot.prepare(record(1)).unwrap(); slot.cancel_prepared(1).unwrap();
    assert_eq!(slot.claim(14, 1), Err(INVALID));
    assert_eq!(slot.prepare(record(1)), Err(INVALID));
    slot.prepare(record(2)).unwrap();
}

#[test]
fn wrong_pager_stale_token_and_duplicate_claim_do_not_take_request() {
    let slot = pending();
    assert_eq!(slot.claim(13, 1), Err(DENIED));
    assert_eq!(slot.claim(14, 2), Err(INVALID));
    assert_eq!(slot.claim(14, 1), Ok(record(1)));
    assert_eq!(slot.claim(14, 1), Err(RETRY));
    assert_eq!(slot.reject(14, 1), Err(RETRY));
}

#[test]
fn no_effect_rollback_retains_exact_context_and_token() {
    let slot = pending(); slot.claim(14, 1).unwrap(); slot.rollback(14, 1).unwrap();
    assert_eq!(slot.query(14, 1).unwrap().saved_frame, 0x21000);
    assert_eq!(slot.claim(14, 1).unwrap().task_id, 0x8015);
}

#[test]
fn duplicate_completion_cannot_consume_a_new_fault() {
    let slot = pending(); slot.claim(14, 1).unwrap();
    assert_eq!(slot.complete(14, 1), Ok(Some(record(1))));
    slot.prepare(record(2)).unwrap(); slot.publish(2).unwrap();
    assert_eq!(slot.complete(14, 1), Ok(None));
    assert_eq!(slot.complete(13, 1), Err(DENIED));
    assert_eq!(slot.query(14, 2), Ok(record(2)));
}

#[test]
fn reject_consumes_only_authenticated_pending_request() {
    let slot = pending();
    assert_eq!(slot.reject(13, 1), Err(DENIED));
    assert_eq!(slot.reject(14, 1), Ok(record(1)));
    assert_eq!(slot.reject(14, 1), Err(INVALID));
    assert_eq!(slot.complete(14, 1), Err(INVALID));
}

#[test]
fn fault_completion_preserves_all_three_cpu_custody_boundaries() {
    for phase in [Phase::Attached, Phase::SwitchingOut, Phase::Detached] {
        let state = attached();
        assert_eq!(state.park_fault(0), ParkResult::Blocked);
        state.wake();
        assert!(!state.resume_attached(0), "generic event cannot resolve fault");
        if phase != Phase::Attached { assert!(state.begin_switch_out(0)); }
        if phase == Phase::Detached { assert!(unsafe { state.release_after_stack_switch(0) }); }
        let slot = pending(); slot.claim(14, 1).unwrap();
        assert!(slot.complete(14, 1).unwrap().is_some());
        assert!(unsafe { state.complete_fault() });
        assert_eq!(state.snapshot().phase, phase);
        assert!(state.snapshot().pending, "ordinary event remains owed");
        if phase != Phase::Detached {
            assert_eq!(state.try_admit::<()>(|| panic!("still attached stack published")), Admission::NotRunnable);
        } else {
            assert_eq!(state.try_admit(|| Ok::<(), ()>(())), Admission::Published);
        }
        assert!(!unsafe { state.complete_fault() }, "completion grants one resume only");
    }
}

#[test]
fn completion_never_resurrects_an_exited_task() {
    let state = attached(); state.park_fault(0); state.exit();
    assert!(!unsafe { state.complete_fault() });
    assert!(state.snapshot().exited);
}

#[test]
fn concurrent_claim_has_exactly_one_winner() {
    let slot = std::sync::Arc::new(pending());
    let mut workers = Vec::new();
    for _ in 0..16 {
        let slot = slot.clone(); workers.push(std::thread::spawn(move || slot.claim(14, 1).is_ok()));
    }
    assert_eq!(workers.into_iter().filter(|worker| worker.thread().id() != std::thread::current().id())
        .map(|worker| worker.join().unwrap() as usize).sum::<usize>(), 1);
}

#[test]
fn concurrent_tokens_are_unique_and_nonzero() {
    let issuer = std::sync::Arc::new(TokenIssuer::new());
    let mut workers = Vec::new();
    for _ in 0..8 {
        let issuer = issuer.clone();
        workers.push(std::thread::spawn(move || (0..1000).map(|_| issuer.issue().unwrap()).collect::<Vec<_>>()));
    }
    let mut tokens: Vec<_> = workers.into_iter().flat_map(|worker| worker.join().unwrap()).collect();
    tokens.sort_unstable(); tokens.dedup();
    assert_eq!(tokens, (1..=8000).collect::<Vec<_>>());
}

#[test]
fn only_user_nonpresent_data_faults_are_eligible() {
    for error in [4,6] { assert!(eligible_user_fault(0x4000,error,0x23)); }
    for cs in [0,8,0x10,0x21,0x22] { assert!(!eligible_user_fault(0x4000,4,cs)); }
    for error in [0,1,2,3,5,7,12,14,20,22,36,38,u64::MAX] {
        assert!(!eligible_user_fault(0x4000,error,3));
        let mut bad=record(1);bad.error_code=error;assert_eq!(FaultSlot::new().prepare(bad),Err(INVALID));
    }
    for address in [0,1<<47,0xffff800000000000,u64::MAX] { assert!(!eligible_user_fault(address,4,3)); }
}

#[test]
fn exited_retirement_consumes_prepared_pending_claimed_and_completed_without_reopening() {
 for phase in 0..4 {
  let slot=FaultSlot::new();slot.prepare(record(1)).unwrap();
  if phase>=1{slot.publish(1).unwrap();}if phase>=2{slot.claim(14,1).unwrap();}if phase>=3{slot.complete(14,1).unwrap();}
  assert_eq!(unsafe{slot.retire_exited(15,14,0x8015)},Ok(()));
  assert_eq!(unsafe{slot.retire_exited(15,14,0x8015)},Ok(()));
  assert_eq!(slot.complete(14,1),Err(INVALID));assert_eq!(slot.query(14,1),Err(INVALID));assert_eq!(slot.prepare(record(2)),Err(DENIED));
 }
}
#[test]
fn foreign_exit_retirement_preserves_exact_claimed_transaction() {
 let slot=pending();slot.claim(14,1).unwrap();
 for (target,pager,task) in [(16,14,0x8015),(15,13,0x8015),(15,14,15)]{
  assert_eq!(unsafe{slot.retire_exited(target,pager,task)},Err(DENIED));
 }
 assert_eq!(slot.complete(14,1),Ok(Some(record(1))));
}
