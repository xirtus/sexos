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
fn all_legal_registry_callers_retain_independent_reply_rights() {
    let mut credits = ReplyCredits::new();
    for caller in 1..REPLY_CREDIT_CAPACITY as u32 {
        assert_eq!(credits.grant(caller), Grant::Added);
    }
    assert_eq!(credits.outstanding(), (REPLY_CREDIT_CAPACITY - 1) as u128);
    for caller in (1..REPLY_CREDIT_CAPACITY as u32).rev() {
        assert!(credits.holds(caller));
        assert!(credits.consume(caller));
        assert!(!credits.consume(caller));
    }
    assert_eq!(credits.outstanding(), 0);
}

#[test]
fn invalid_caller_ids_cannot_create_or_spend_any_rights() {
    let mut credits = ReplyCredits::new();
    assert_eq!(credits.grant(3), Grant::Added);
    for caller in [0, REPLY_CREDIT_CAPACITY as u32, u32::MAX] {
        assert_eq!(credits.grant(caller), Grant::Full);
        assert!(!credits.holds(caller));
        assert!(!credits.consume(caller));
        credits.withdraw(caller);
        assert_eq!(credits.outstanding(), 1);
        assert!(credits.holds(3));
    }
}

#[test]
fn hundred_thousand_accepted_invocations_all_remain_consumable() {
    let mut credits = ReplyCredits::new();
    for _ in 0..100_000 { assert_eq!(credits.grant(3), Grant::Added); }
    assert_eq!(credits.outstanding(), 100_000);
    for remaining in (0..100_000).rev() {
        assert!(credits.consume(3));
        assert_eq!(credits.outstanding(), remaining);
    }
    assert!(!credits.consume(3));
    assert!(!credits.holds(3));
    assert_eq!(credits.outstanding(), 0);
}

#[test]
fn kernel_rejects_unrecordable_invocation_before_publication() {
    let ipc = include_str!("../kernel/src/ipc.rs");
    assert!(ipc.contains("pub const MAX_DOMAINS: usize = reply_credit::REPLY_CREDIT_CAPACITY;"));
    let enqueue = ipc.split("GraphEdge::AsyncEnqueue { callee } =>").nth(1).unwrap().split("GraphEdge::AsyncDequeue").next().unwrap();
    let grant = enqueue.find("credits.grant(caller_pd)").unwrap();
    let full = enqueue.find("Grant::Full").unwrap();
    let refuse = enqueue.find("return Err(sex_pdx::ERR_SERVICE_NOT_READY)").unwrap();
    let publish = enqueue.find("(*callee.message_ring).enqueue(msg)").unwrap();
    let withdraw = enqueue.find("credits.withdraw(caller_pd)").unwrap();
    assert!(grant < full && full < refuse && refuse < publish && publish < withdraw);
    assert_eq!(enqueue.matches("return Err(sex_pdx::ERR_SERVICE_NOT_READY)").count(), 2);
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
