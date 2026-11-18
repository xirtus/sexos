// SPDX-License-Identifier: GPL-3.0-only
//! Loom models of the IPC v2 objects and the authority v2 capability slots (kernel
//! plan phase 7). The code is the real kernel/src/ipc_v2/{endpoint,notification}.rs
//! and kernel/src/capability_table.rs, rewritten by fuzz/models/build.rs only to use
//! loom's atomics, a loom-tracked cell for the endpoint queue, yielding spin loops
//! and (for the table) 4 slots and 4 objects.
//!
//! What must hold under every interleaving:
//!  * endpoint: no lost wakeup (a parked receiver never sits beside queued callers),
//!    each caller is delivered once, and a cancelled receive either wins or a caller
//!    has taken the receiver;
//!  * notification: a waiter that parks is always woken by the signal that follows,
//!    and the signalled bit is delivered exactly once;
//!  * capability slots: a reader racing revoke + install sees the old capability,
//!    nothing, or the new one, never a mix of the two (the seqlock), and racing
//!    installs never share a slot.
//! Run: RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 \
//!      cargo test --release --manifest-path fuzz/models/Cargo.toml --test model_loom_v2
#![cfg(loom)]
use loom::sync::Arc;
use loom::thread;
use sexos_kernel_models::loom_authority::capability::CapabilityData as D;
use sexos_kernel_models::loom_authority::capability_table::{CapabilityTable, Generations, ObjectCap};
use sexos_kernel_models::loom_ipc_v2::endpoint::{CallKind, Endpoint, RecvOutcome, Released, SendOutcome, Sender};
use sexos_kernel_models::loom_ipc_v2::notification::{Notification, SignalOutcome, WaitOutcome};

/// The endpoint's spin lock makes long but finite paths, so loom's default branch cap
/// (1,000) is raised. Assertions here print no values: formatting overflows
/// loom's small coroutine stacks and hides the failure behind an abort.
fn model<F: Fn() + Sync + Send + 'static>(f: F) {
    let mut b = loom::model::Builder::new();
    b.max_branches = 100_000;
    b.check(f);
}

fn caller(task: u64) -> Sender { Sender { task, badge: task * 10, generation: 1, kind: CallKind::Call } }

#[test]
fn endpoint_caller_and_receiver_lose_nobody() {
    model(|| {
        let ep = Arc::new(Endpoint::new());
        let c = { let ep = ep.clone(); thread::spawn(move || ep.send(caller(1))) };
        let got = ep.recv(0);
        let sent = c.join().unwrap();
        match (got, sent) {
            // The caller queued first and the receiver took it.
            (RecvOutcome::Message(s), SendOutcome::Queued) => { assert_eq!(s, caller(1)); assert_eq!(ep.queued(), 0); }
            // The receiver parked first and the caller took it.
            (RecvOutcome::Parked, SendOutcome::Rendezvous { receiver: 0 }) => assert_eq!(ep.queued(), 0),
            other => panic!("lost or duplicated: {other:?} queued={} waiter={:?}", ep.queued(), ep.waiter()),
        }
        // No lost wakeup: never a parked receiver next to a waiting caller.
        assert!(ep.waiter().is_none() && ep.queued() == 0);
    });
}

#[test]
fn endpoint_two_callers_race_for_one_parked_receiver() {
    model(|| {
        let ep = Arc::new(Endpoint::new());
        assert_eq!(ep.recv(0), RecvOutcome::Parked);
        let c = { let ep = ep.clone(); thread::spawn(move || ep.send(caller(1))) };
        let mine = ep.send(caller(2));
        let theirs = c.join().unwrap();
        // Exactly one takes the receiver; the other queues behind it.
        let taken = [mine, theirs].iter().filter(|o| **o == SendOutcome::Rendezvous { receiver: 0 }).count();
        assert_eq!(taken, 1, "{mine:?} {theirs:?}");
        assert!([mine, theirs].contains(&SendOutcome::Queued));
        assert_eq!(ep.queued(), 1);
        assert!(ep.waiter().is_none());
    });
}

#[test]
fn endpoint_cancelled_receive_races_a_caller() {
    model(|| {
        let ep = Arc::new(Endpoint::new());
        assert_eq!(ep.recv(0), RecvOutcome::Parked);
        let c = { let ep = ep.clone(); thread::spawn(move || ep.send(caller(1))) };
        let cancelled = ep.cancel_recv(0);
        let sent = c.join().unwrap();
        // Exactly one wins: the cancel, and the caller queues; or the caller, which
        // took the receiver and will hand it the message.
        if cancelled {
            assert_eq!(sent, SendOutcome::Queued);
            assert_eq!(ep.queued(), 1);
        } else {
            assert_eq!(sent, SendOutcome::Rendezvous { receiver: 0 });
            assert_eq!(ep.queued(), 0);
        }
        assert!(ep.waiter().is_none());
    });
}

#[test]
fn endpoint_close_races_a_caller() {
    model(|| {
        let ep = Arc::new(Endpoint::new());
        assert_eq!(ep.recv(0), RecvOutcome::Parked);
        let c = { let ep = ep.clone(); thread::spawn(move || ep.send(caller(1))) };
        let mut released = Vec::new();
        ep.close(|x| released.push(x));
        let sent = c.join().unwrap();
        // The parked receiver is released by close unless the caller took it first;
        // a caller after the close is refused, never left queued on a dead endpoint.
        match sent {
            SendOutcome::Rendezvous { receiver: 0 } => assert!(released.is_empty()),
            SendOutcome::Closed => assert_eq!(released, vec![Released::Receiver(0)]),
            other => panic!("unexpected {other:?} {released:?}"),
        }
        assert!(ep.is_closed() && ep.queued() == 0 && ep.waiter().is_none());
    });
}

#[test]
fn notification_waiter_that_parks_is_always_woken() {
    model(|| {
        let n = Arc::new(Notification::new());
        let s = { let n = n.clone(); thread::spawn(move || n.signal(1)) };
        let waited = n.wait(7);
        let signalled = s.join().unwrap();
        match waited {
            WaitOutcome::Parked => {
                // The signal saw the waiter and must wake it; the kernel then polls the bit.
                assert_eq!(signalled, SignalOutcome::Wake(7));
                assert_eq!(n.poll(), 1);
            }
            WaitOutcome::Ready(bits) => {
                assert_eq!(bits, 1);
                assert_eq!(n.pending(), 0, "the bit is delivered once");
            }
            WaitOutcome::Busy => panic!("one waiter"),
        }
        assert!(n.waiter().is_none());
    });
}

#[test]
fn notification_two_signals_one_waiter() {
    model(|| {
        let n = Arc::new(Notification::new());
        let a = { let n = n.clone(); thread::spawn(move || n.signal(1)) };
        let b = { let n = n.clone(); thread::spawn(move || n.signal(2)) };
        let mut bits = match n.wait(3) { WaitOutcome::Ready(b) => b, WaitOutcome::Parked => 0, WaitOutcome::Busy => panic!() };
        let outs = [a.join().unwrap(), b.join().unwrap()];
        let woken = outs.iter().filter(|o| **o == SignalOutcome::Wake(3)).count();
        assert!(woken <= 1, "woken twice: {outs:?}");
        if bits == 0 { assert_eq!(woken, 1, "parked and never woken"); }
        bits |= n.poll();
        assert_eq!(bits, 3, "every bit delivered");
        assert!(n.waiter().is_none());
    });
}

const A: ObjectCap = ObjectCap { object: 1, generation: 2, rights: 0b0_1001, badge: 0xAAAA_0000_AAAA };
const B: ObjectCap = ObjectCap { object: 2, generation: 5, rights: 0b1_0110, badge: 0xBBBB_0000_BBBB };

#[test]
fn slot_reader_never_sees_a_torn_capability() {
    model(|| {
        let t = Arc::new(CapabilityTable::loom_new());
        t.seal();
        assert_eq!(t.install(D::Endpoint(A)), Ok(1));
        let r = { let t = t.clone(); thread::spawn(move || t.find(1).map(|c| c.data)) };
        assert_eq!(t.revoke(1).map(|c| c.data), Some(D::Endpoint(A)));
        assert_eq!(t.install(D::Notification(B)), Ok(1));
        let seen = r.join().unwrap();
        assert!(matches!(seen, None | Some(D::Endpoint(A)) | Some(D::Notification(B))), "torn read");
        assert_eq!(t.find(1).map(|c| c.data), Some(D::Notification(B)));
    });
}

#[test]
fn racing_installs_never_share_a_slot() {
    model(|| {
        let t = Arc::new(CapabilityTable::loom_new());
        t.seal();
        let h = { let t = t.clone(); thread::spawn(move || t.install(D::Endpoint(A)).unwrap()) };
        let mine = t.install(D::Endpoint(B)).unwrap();
        let theirs = h.join().unwrap();
        assert_ne!(mine, theirs);
        assert_eq!(t.object(mine), Some(B));
        assert_eq!(t.object(theirs), Some(A));
    });
}

#[test]
fn racing_creates_get_distinct_objects_and_revoke_kills_old_copies() {
    model(|| {
        let g = Arc::new(Generations::new());
        let h = { let g = g.clone(); thread::spawn(move || g.create(1).unwrap()) };
        let mine = g.create(2).unwrap();
        let theirs = h.join().unwrap();
        assert_ne!(mine.object, theirs.object);
        let r = { let g = g.clone(); thread::spawn(move || g.revoke(theirs.object).unwrap()) };
        let live_before = g.live(&theirs);
        let fresh = r.join().unwrap();
        assert!(!g.live(&theirs) && g.live(&fresh) && g.live(&mine));
        let _ = live_before; // either answer is fine while the revoke races
    });
}
