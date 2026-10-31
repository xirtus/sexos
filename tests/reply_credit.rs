//! Native tests of the production kernel reply-obligation table.
#[path = "../kernel/src/ipc/reply_credit.rs"]
mod reply_credit;

use reply_credit::{Grant, ReplyCredits, REPLY_CREDIT_CAPACITY};

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
    assert_eq!(Grant::Added, credits.grant(15));
    assert_eq!(Grant::Added, credits.grant(15));
    assert_eq!(Grant::Added, credits.grant(9));
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
    assert_eq!(Grant::Added, credits.grant(15));
    credits.withdraw(15);
    assert!(!credits.holds(15));
    // Withdrawing an absent caller cannot underflow another entry.
    assert_eq!(Grant::Added, credits.grant(9));
    credits.withdraw(15);
    assert!(credits.holds(9));
    assert_eq!(credits.outstanding(), 1);
}

#[test]
fn full_table_records_no_obligation_for_a_new_caller_without_evicting() {
    let mut credits = ReplyCredits::new();
    for caller in 1..=REPLY_CREDIT_CAPACITY as u32 {
        assert_eq!(Grant::Added, credits.grant(caller));
    }
    assert_eq!(credits.grant(5000), Grant::Untracked);
    assert!(!credits.holds(5000), "no reply authority for an untracked caller");
    for caller in 1..=REPLY_CREDIT_CAPACITY as u32 {
        assert!(credits.holds(caller));
    }
    assert_eq!(Grant::Added, credits.grant(1));
    assert!(credits.consume(2));
    assert_eq!(Grant::Added, credits.grant(5000));
    assert!(credits.consume(5000));
}

#[test]
fn one_way_invocations_saturate_and_never_refuse() {
    // The 1800-second compact session: Shell sent Display more than 65,535
    // never-answered invocations and every later one was refused.
    let mut credits = ReplyCredits::new();
    for _ in 0..u16::MAX { assert_eq!(credits.grant(3), Grant::Added); }
    for _ in 0..100_000 { assert_eq!(credits.grant(3), Grant::Saturated); }
    assert_eq!(credits.outstanding(), u16::MAX as u32);
    assert!(credits.consume(3), "replies stay authorized at the bound");
    assert_eq!(credits.grant(3), Grant::Added);
}

#[test]
fn kernel_never_refuses_an_invocation_for_reply_bookkeeping() {
    let ipc = include_str!("../kernel/src/ipc.rs");
    let enqueue = ipc.split("GraphEdge::AsyncEnqueue { callee } =>").nth(1).unwrap().split("GraphEdge::AsyncDequeue").next().unwrap();
    let grant = enqueue.find("let granted = credits.grant(caller_pd);").unwrap();
    let publish = enqueue.find("(*callee.message_ring).enqueue(msg)").unwrap();
    assert!(grant < publish);
    // The only refusal is a full ring.
    assert_eq!(enqueue.matches("return Err(sex_pdx::ERR_SERVICE_NOT_READY)").count(), 1);
    assert!(enqueue.contains("if granted == crate::ipc::reply_credit::Grant::Added { credits.withdraw(caller_pd); }"));
}

#[test]
fn retired_callee_owes_nothing() {
    let mut credits = ReplyCredits::new();
    for caller in [3, 9, 15] {
        assert_eq!(Grant::Added, credits.grant(caller));
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
