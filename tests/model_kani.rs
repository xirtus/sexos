// SPDX-License-Identifier: GPL-3.0-only
//! Kani bounded proofs over the real kernel sources (kernel plan phase 7). Built into
//! fuzz/models' library only under `cargo kani`; each harness explores every value
//! of its symbolic inputs, not samples. Sequential only: concurrency is loom's job
//! (model_loom_queues.rs).
//! Run: cd fuzz/models && cargo kani
use crate::ipc_ring::RingBuffer;
use crate::reply_credit::{Grant, ReplyCredits, REPLY_CALLERS, REPLY_CREDIT_CAPACITY};
use crate::scheduler_queue::{BoundedQueue, PushError};

// unwind(130): the reply table probes up to REPLY_CALLERS (128) entries per lookup.

/// No domain id, valid or not, can spend a reply on a fresh table.
#[kani::proof]
#[kani::unwind(130)]
#[kani::solver(cadical)]
fn reply_needs_an_invocation() {
    let mut credits = ReplyCredits::new();
    let target: u32 = kani::any();
    assert!(!credits.holds(target));
    assert!(!credits.consume(target));
}

/// One grant permits exactly one reply, to that caller only; invalid callers are
/// refused and change nothing.
#[kani::proof]
#[kani::unwind(130)]
#[kani::solver(cadical)]
fn one_grant_one_reply_to_that_caller() {
    let mut credits = ReplyCredits::new();
    let caller: u32 = kani::any();
    let other: u32 = kani::any();
    let granted = credits.grant(caller);
    let valid = caller != 0 && (caller as usize) < REPLY_CREDIT_CAPACITY;
    assert_eq!(granted == Grant::Added, valid);
    if other != caller { assert!(!credits.holds(other)); assert!(!credits.consume(other)); }
    assert_eq!(credits.consume(caller), valid);
    assert!(!credits.consume(caller));
    assert_eq!(credits.outstanding(), 0);
}

/// Two callers that collide on a probe chain keep separate counts, and withdraw
/// undoes exactly one reservation.
#[kani::proof]
#[kani::unwind(130)]
#[kani::solver(cadical)]
fn colliding_callers_stay_separate_and_withdraw_is_exact() {
    let mut credits = ReplyCredits::new();
    let a: u32 = kani::any();
    let k: u32 = kani::any();
    kani::assume(a > 0 && (a as usize) < REPLY_CREDIT_CAPACITY && k > 0 && k < 8);
    let b = a + k * REPLY_CALLERS as u32; // same home slot as a
    kani::assume((b as usize) < REPLY_CREDIT_CAPACITY);
    assert_eq!(credits.grant(a), Grant::Added);
    assert_eq!(credits.grant(b), Grant::Added);
    assert_eq!(credits.grant(b), Grant::Added);
    credits.withdraw(b);
    assert!(credits.consume(b));
    assert!(!credits.consume(b));
    assert!(credits.consume(a));
    assert!(!credits.holds(a) && !credits.holds(b));
}

/// Run queue, N=2: any 5-step sequence of push/steal behaves as a bounded FIFO,
/// through wraparound.
#[kani::proof]
#[kani::unwind(6)]
fn run_queue_is_a_bounded_fifo() {
    let q = BoundedQueue::<usize, 2>::new();
    let mut model = [0usize; 2];
    let (mut head, mut len) = (0usize, 0usize);
    let mut next = 1usize;
    for _ in 0..5 {
        if kani::any() {
            let pushed = q.try_push(next as *mut usize);
            if len == 2 { assert_eq!(pushed, Err(PushError::Full)); }
            else { assert_eq!(pushed, Ok(())); model[(head + len) % 2] = next; len += 1; }
            next += 1;
        } else {
            let got = q.steal() as usize;
            if len == 0 { assert_eq!(got, 0); }
            else { assert_eq!(got, model[head]); head = (head + 1) % 2; len -= 1; }
        }
    }
    assert_eq!(q.try_push(core::ptr::null_mut()), Err(PushError::Null));
}

/// IPC ring, N=2: any 5-step sequence of enqueue/dequeue with symbolic payloads
/// behaves as a bounded FIFO, and len agrees.
#[kani::proof]
#[kani::unwind(6)]
fn ipc_ring_is_a_bounded_fifo() {
    let ring = RingBuffer::<u64, 2>::new();
    let mut model = [0u64; 2];
    let (mut head, mut len) = (0usize, 0usize);
    for _ in 0..5 {
        if kani::any() {
            let v: u64 = kani::any();
            let ok = ring.enqueue(v).is_ok();
            assert_eq!(ok, len < 2);
            if ok { model[(head + len) % 2] = v; len += 1; }
        } else {
            let got = ring.dequeue();
            if len == 0 { assert_eq!(got, None); }
            else { assert_eq!(got, Some(model[head])); head = (head + 1) % 2; len -= 1; }
        }
        assert_eq!(ring.len(), len);
    }
}
