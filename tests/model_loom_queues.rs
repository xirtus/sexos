// SPDX-License-Identifier: GPL-3.0-only
//! Loom models of the kernel's lock-free queues (kernel plan phase 7). Loom runs each
//! scenario under every interleaving of atomic operations (bounded by
//! LOOM_MAX_PREEMPTIONS) and models weak memory ordering for the atomics, so a
//! missing Release/Acquire pair shows up as a wrong value.
//! The queues are the real kernel/src/scheduler_queue.rs and kernel/src/ipc_ring.rs,
//! rewritten by fuzz/models/build.rs only to use loom's atomics and, for the ring's
//! payload, a loom-tracked cell (an unordered payload access fails the model).
//! Mutation-checked: publishing a run-queue value before writing it, a Relaxed
//! sequence load in steal, a Relaxed ring publish, and a Relaxed read_idx release
//! each fail at least one model below.
//! Run: RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 \
//!      cargo test --release --manifest-path fuzz/models/Cargo.toml --test model_loom_queues
#![cfg(loom)]
use loom::sync::Arc;
use loom::thread;
use sexos_kernel_models::loom_ipc_ring::RingBuffer;
use sexos_kernel_models::loom_scheduler_queue::{BoundedQueue, PushError};

fn token(n: usize) -> *mut usize { n as *mut usize }

fn push_retry<const N: usize>(q: &BoundedQueue<usize, N>, v: usize) {
    loop {
        match q.try_push(token(v)) {
            Ok(()) => return,
            Err(PushError::Null) => unreachable!(),
            Err(_) => thread::yield_now(),
        }
    }
}

fn drain<const N: usize>(q: &BoundedQueue<usize, N>) -> Vec<usize> {
    let mut out = Vec::new();
    loop {
        let v = q.steal();
        if v.is_null() { return out; }
        out.push(v as usize);
    }
}

/// Two producers race for the same slot while a stealer runs: each token arrives
/// exactly once, and a stolen pointer is always one that was published.
#[test]
fn run_queue_two_producers_one_stealer() {
    loom::model(|| {
        let q = Arc::new(BoundedQueue::<usize, 2>::new());
        let a = { let q = q.clone(); thread::spawn(move || push_retry(&q, 1)) };
        let b = { let q = q.clone(); thread::spawn(move || push_retry(&q, 2)) };
        let early = q.steal() as usize;
        assert!(early == 0 || early == 1 || early == 2, "unpublished value {early:#x}");
        a.join().unwrap();
        b.join().unwrap();
        let mut got: Vec<usize> = drain(&q);
        if early != 0 { got.push(early); }
        got.sort_unstable();
        assert_eq!(got, [1, 2]);
    });
}

/// Two stealers race for one token while a producer refills across wraparound
/// (N=2, positions 0..3 reuse both slots): no token is lost or taken twice, and the
/// relaxed value store is ordered by the slot sequence.
#[test]
fn run_queue_two_stealers_across_wraparound() {
    loom::model(|| {
        let q = Arc::new(BoundedQueue::<usize, 2>::new());
        push_retry(&q, 1);
        push_retry(&q, 2);
        assert_eq!(q.steal() as usize, 1); // slot 0 now free for position 2
        let p = { let q = q.clone(); thread::spawn(move || push_retry(&q, 3)) };
        let s = { let q = q.clone(); thread::spawn(move || q.steal() as usize) };
        let mine = q.steal() as usize;
        p.join().unwrap();
        let theirs = s.join().unwrap();
        let mut got: Vec<usize> = [mine, theirs].into_iter().filter(|&v| v != 0).collect();
        got.extend(drain(&q));
        got.sort_unstable();
        assert_eq!(got, [2, 3]);
    });
}

/// A full queue refuses without reserving, even while a consumer is mid-release.
#[test]
fn run_queue_full_never_loses_ownership() {
    loom::model(|| {
        let q = Arc::new(BoundedQueue::<usize, 2>::new());
        push_retry(&q, 1);
        push_retry(&q, 2);
        let s = { let q = q.clone(); thread::spawn(move || q.steal() as usize) };
        let pushed = q.try_push(token(3));
        let stolen = s.join().unwrap();
        assert_eq!(stolen, 1);
        let mut got = drain(&q);
        match pushed {
            Ok(()) => assert_eq!(got, [2, 3]),
            Err(PushError::Full) | Err(PushError::Contended) => { got.sort_unstable(); assert_eq!(got, [2]) }
            Err(PushError::Null) => unreachable!(),
        }
    });
}

/// Two client PDs enqueue into one server ring while the server dequeues: the
/// 2026-07-18 lost-message bug (two producers writing one slot) must not return.
#[test]
fn ipc_ring_two_producers_one_consumer() {
    loom::model(|| {
        let ring = Arc::new(RingBuffer::<u64, 2>::new());
        let a = { let r = ring.clone(); thread::spawn(move || r.enqueue(0xA1).unwrap()) };
        let b = { let r = ring.clone(); thread::spawn(move || r.enqueue(0xB2).unwrap()) };
        let early = ring.dequeue();
        a.join().unwrap();
        b.join().unwrap();
        let mut got: Vec<u64> = early.into_iter().collect();
        while let Some(v) = ring.dequeue() { got.push(v); }
        got.sort_unstable();
        assert_eq!(got, [0xA1, 0xB2]);
        assert_eq!(ring.len(), 0);
    });
}

/// Slot reuse with one slot: the producer may only write after the consumer has
/// moved the old value out and erased it, so neither value is torn or zeroed.
#[test]
fn ipc_ring_reuse_after_erase() {
    loom::model(|| {
        let ring = Arc::new(RingBuffer::<u64, 1>::new());
        ring.enqueue(7).unwrap();
        let p = { let r = ring.clone(); thread::spawn(move || {
            while r.enqueue(9).is_err() { thread::yield_now(); }
        }) };
        let first = loop {
            if let Some(v) = ring.dequeue() { break v; }
            thread::yield_now();
        };
        p.join().unwrap();
        assert_eq!(first, 7);
        assert_eq!(ring.dequeue(), Some(9));
        assert_eq!(ring.dequeue(), None);
    });
}

/// Two consumers (the reader turn guard) and one producer: each value moves out once.
#[test]
fn ipc_ring_two_consumers_move_each_value_once() {
    loom::model(|| {
        let ring = Arc::new(RingBuffer::<u64, 2>::new());
        ring.enqueue(5).unwrap();
        let p = { let r = ring.clone(); thread::spawn(move || r.enqueue(6).unwrap()) };
        let c = { let r = ring.clone(); thread::spawn(move || r.dequeue()) };
        let mine = ring.dequeue();
        p.join().unwrap();
        let theirs = c.join().unwrap();
        let mut got: Vec<u64> = mine.into_iter().chain(theirs).collect();
        while let Some(v) = ring.dequeue() { got.push(v); }
        got.sort_unstable();
        assert_eq!(got, [5, 6]);
    });
}
