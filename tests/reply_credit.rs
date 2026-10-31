//! Native tests of the production kernel reply-obligation table.
#[path = "../kernel/src/ipc/reply_credit.rs"]
mod reply_credit;

use reply_credit::{ReplyCredits, REPLY_CREDIT_CAPACITY};

#[test]
fn domain_id_without_invocation_cannot_reply() {
    let mut credits = ReplyCredits::new();
    for target in [0, 1, 7, 13, 1023, u32::MAX] {
        assert!(!credits.holds(target));
        assert!(!credits.consume(target));
    }
    assert_eq!(credits.outstanding(), 0);
}

#[test]
fn each_accepted_invocation_permits_exactly_one_reply_to_its_caller() {
    let mut credits = ReplyCredits::new();
    assert!(credits.grant(15));
    assert!(credits.grant(15));
    assert!(credits.grant(9));
    assert!(!credits.holds(12), "another domain gains nothing");
    assert!(credits.consume(15));
    assert!(credits.consume(15));
    assert!(!credits.consume(15), "a third reply has no invocation");
    assert!(credits.holds(9));
    assert!(credits.consume(9));
    assert_eq!(credits.outstanding(), 0);
}

#[test]
fn withdrawn_reservation_grants_nothing() {
    let mut credits = ReplyCredits::new();
    assert!(credits.grant(15));
    credits.withdraw(15);
    assert!(!credits.holds(15));
    // Withdrawing an absent caller cannot underflow another entry.
    assert!(credits.grant(9));
    credits.withdraw(15);
    assert!(credits.holds(9));
    assert_eq!(credits.outstanding(), 1);
}

#[test]
fn full_table_refuses_new_callers_without_evicting_existing_obligations() {
    let mut credits = ReplyCredits::new();
    for caller in 1..=REPLY_CREDIT_CAPACITY as u32 {
        assert!(credits.grant(caller));
    }
    assert!(!credits.grant(5000));
    assert!(!credits.holds(5000));
    for caller in 1..=REPLY_CREDIT_CAPACITY as u32 {
        assert!(credits.holds(caller));
    }
    // An existing caller may still accumulate, and a freed entry is reused.
    assert!(credits.grant(1));
    assert!(credits.consume(2));
    assert!(credits.grant(5000));
    assert!(credits.consume(5000));
}

#[test]
fn per_caller_count_saturates_by_refusal() {
    let mut credits = ReplyCredits::new();
    for _ in 0..u16::MAX {
        assert!(credits.grant(15));
    }
    assert!(!credits.grant(15));
    assert_eq!(credits.outstanding(), u16::MAX as u32);
}

#[test]
fn retired_callee_owes_nothing() {
    let mut credits = ReplyCredits::new();
    for caller in [3, 9, 15] {
        assert!(credits.grant(caller));
    }
    credits.clear();
    for caller in [3, 9, 15] {
        assert!(!credits.holds(caller));
    }
    assert_eq!(credits.outstanding(), 0);
}

#[test]
fn kernel_wiring_grants_before_publication_and_spends_after_delivery() {
    let ipc = include_str!("../kernel/src/ipc.rs");
    let router = include_str!("../kernel/src/ipc/router.rs");
    let enqueue = ipc.split("GraphEdge::AsyncEnqueue { callee } =>").nth(1).unwrap();
    let grant = enqueue.find("credits.grant(caller_pd)").unwrap();
    let publish = enqueue.find("(*callee.message_ring).enqueue(msg)").unwrap();
    let withdraw = enqueue.find("credits.withdraw(caller_pd)").unwrap();
    assert!(grant < publish && publish < withdraw);
    let reply = router.split("pub fn send_reply(").nth(1).unwrap();
    let holds = reply.find("credits.holds(target_pd_id)").unwrap();
    let full = reply.find("return Err(IpcError::BufferFull)").unwrap();
    let push = reply.find("replies.push_back(").unwrap();
    let consume = reply.find("credits.consume(target_pd_id)").unwrap();
    assert!(holds < full && full < push && push < consume);
    let retire = include_str!("../kernel/src/app_resources.rs");
    assert!(retire.contains("target.reply_credits.lock().clear();"));
}
