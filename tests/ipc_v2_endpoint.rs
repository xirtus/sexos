//! Host tests of the IPC v2 endpoint (kernel/src/ipc_v2/endpoint.rs).
#[path = "../kernel/src/ipc_v2"]
mod ipc_v2 {
    pub type TaskId = u64;
    pub mod endpoint;
}

use ipc_v2::endpoint::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn call(task: u64, badge: u64) -> Sender { Sender { task, badge, generation: 7, kind: CallKind::Call } }

#[test]
fn parked_receiver_is_taken_by_the_fastpath() {
    let ep = Endpoint::new();
    assert_eq!(ep.recv(3), RecvOutcome::Parked);
    assert_eq!(ep.waiter(), Some(3));
    assert_eq!(ep.send(call(9, 0xB0)), SendOutcome::Rendezvous { receiver: 3 });
    assert_eq!(ep.waiter(), None);
    assert_eq!(ep.queued(), 0);
}

#[test]
fn callers_queue_in_order_and_carry_their_badges() {
    let ep = Endpoint::new();
    for t in 0..4 { assert_eq!(ep.send(call(10 + t, 100 + t as u64)), SendOutcome::Queued); }
    for t in 0..4 {
        match ep.recv(1) {
            RecvOutcome::Message(s) => { assert_eq!(s.task, 10 + t); assert_eq!(s.badge, 100 + t as u64); assert_eq!(s.generation, 7); }
            other => panic!("expected message, got {other:?}"),
        }
    }
    assert_eq!(ep.recv(1), RecvOutcome::Parked);
}

#[test]
fn one_receiver_at_a_time() {
    let ep = Endpoint::new();
    assert_eq!(ep.recv(1), RecvOutcome::Parked);
    assert_eq!(ep.recv(2), RecvOutcome::Busy);
    assert!(!ep.cancel_recv(2));
    assert!(ep.cancel_recv(1));
    assert_eq!(ep.send(call(5, 0)), SendOutcome::Queued, "a cancelled receiver is not taken");
}

#[test]
fn full_queue_refuses_without_losing_anyone() {
    let ep = Endpoint::new();
    for t in 0..QUEUE_DEPTH as u64 { assert_eq!(ep.send(call(t, 0)), SendOutcome::Queued); }
    assert_eq!(ep.send(call(99, 0)), SendOutcome::Full);
    assert_eq!(ep.queued(), QUEUE_DEPTH);
}

#[test]
fn cancelled_caller_leaves_the_others_in_order() {
    let ep = Endpoint::new();
    for t in [1, 2, 3, 4] { ep.send(call(t, 0)); }
    assert_eq!(ep.cancel_send(2).map(|s| s.task), Some(2));
    assert_eq!(ep.cancel_send(2), None);
    assert_eq!(ep.queued(), 3);
    let order: Vec<u64> = (0..3).map(|_| match ep.recv(0) { RecvOutcome::Message(s) => s.task, o => panic!("{o:?}") }).collect();
    assert_eq!(order, vec![1, 3, 4]);
}

#[test]
fn close_releases_everyone_and_refuses_later_calls() {
    let ep = Endpoint::new();
    ep.send(call(1, 0));
    ep.send(call(2, 0));
    let mut released = Vec::new();
    assert_eq!(ep.close(|r| released.push(r)), 2);
    assert!(matches!(released[0], Released::Caller(s) if s.task == 1));
    assert_eq!(ep.send(call(3, 0)), SendOutcome::Closed);
    assert_eq!(ep.recv(4), RecvOutcome::Closed);
    assert!(ep.reopen());
    assert_eq!(ep.recv(4), RecvOutcome::Parked);
    let mut released = Vec::new();
    assert_eq!(ep.close(|r| released.push(r)), 1);
    assert_eq!(released, vec![Released::Receiver(4)]);
    assert!(!ep.cancel_recv(4), "a closed endpoint has no waiter to cancel");
}

/// One server, several callers on real threads: every message is delivered exactly
/// once, whichever path (rendezvous or queue) it took. A lost wakeup hangs or
/// drops a count.
#[test]
fn no_message_is_lost_or_duplicated_under_contention() {
    const CALLERS: u32 = 4;
    const EACH: u32 = 5_000;
    let ep = Arc::new(Endpoint::new());
    // Stand-ins for the scheduler: a rendezvous hands the message to the parked
    // receiver through `inbox`.
    let inbox: Arc<Mutex<Vec<u64>>> = Arc::default();
    let sent = Arc::new(AtomicUsize::new(0));
    let callers: Vec<_> = (0..CALLERS).map(|c| {
        let (ep, inbox, sent) = (ep.clone(), inbox.clone(), sent.clone());
        std::thread::spawn(move || for i in 0..EACH {
            let badge = ((c as u64) << 32) | i as u64;
            loop {
                match ep.send(Sender { task: 100 + c as u64, badge, generation: 0, kind: CallKind::Send }) {
                    SendOutcome::Rendezvous { receiver } => { assert_eq!(receiver, 1); inbox.lock().unwrap().push(badge); break }
                    SendOutcome::Queued => break,
                    SendOutcome::Full => std::thread::yield_now(),
                    SendOutcome::Closed => panic!("closed"),
                }
            }
            sent.fetch_add(1, Ordering::SeqCst);
        })
    }).collect();
    let total = (CALLERS * EACH) as usize;
    let mut got = Vec::with_capacity(total);
    while got.len() < total {
        match ep.recv(1) {
            RecvOutcome::Message(s) => got.push(s.badge),
            RecvOutcome::Parked => loop {
                if let Some(b) = inbox.lock().unwrap().pop() { got.push(b); break }
                if ep.waiter().is_none() { continue } // taken: message is on its way
                if sent.load(Ordering::SeqCst) == total && ep.cancel_recv(1) { break }
                std::thread::yield_now();
            },
            other => panic!("{other:?}"),
        }
    }
    for c in callers { c.join().unwrap(); }
    got.extend(inbox.lock().unwrap().drain(..));
    got.sort_unstable();
    got.dedup();
    assert_eq!(got.len(), total);
}
