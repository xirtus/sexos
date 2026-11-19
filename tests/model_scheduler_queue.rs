// SPDX-License-Identifier: GPL-3.0-only
//! Model-based property test of the kernel run queue (kernel/src/scheduler_queue.rs,
//! kernel plan phase 7). Sequential op sequences must match a bounded VecDeque
//! exactly, through many wraparounds; a threaded run checks that every pushed token
//! comes out exactly once with per-producer order kept. Interleavings at every
//! atomic are checked by loom in model_loom_queues.rs.
//! Run: cargo test --manifest-path fuzz/models/Cargo.toml --test model_scheduler_queue
use proptest::prelude::*;
use std::collections::VecDeque;

#[path = "../kernel/src/scheduler_queue.rs"]
#[allow(dead_code)]
mod scheduler_queue;
use scheduler_queue::{BoundedQueue, PushError};

fn token(n: usize) -> *mut usize { n as *mut usize }

#[derive(Clone, Debug)]
enum Op { Push(usize), PushNull, Steal }

fn ops() -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(prop_oneof![
        5 => (1usize..1_000_000).prop_map(Op::Push),
        1 => Just(Op::PushNull),
        5 => Just(Op::Steal),
    ], 0..600)
}

fn check<const N: usize>(ops: &[Op]) -> Result<(), TestCaseError> {
    let q = BoundedQueue::<usize, N>::new();
    let mut model = VecDeque::new();
    for (step, op) in ops.iter().enumerate() {
        match *op {
            Op::Push(v) => {
                let want = if model.len() == N { Err(PushError::Full) } else { model.push_back(v); Ok(()) };
                prop_assert_eq!(q.try_push(token(v)), want, "step {} {:?}", step, op);
            }
            Op::PushNull => prop_assert_eq!(q.try_push(core::ptr::null_mut()), Err(PushError::Null)),
            Op::Steal => prop_assert_eq!(q.steal() as usize, model.pop_front().unwrap_or(0), "step {} {:?}", step, op),
        }
        // Positions are diagnostic but must still agree with the contents when quiet.
        let top = q.top.load(core::sync::atomic::Ordering::Relaxed);
        let bottom = q.bottom.load(core::sync::atomic::Ordering::Relaxed);
        prop_assert_eq!(bottom.wrapping_sub(top), model.len());
    }
    Ok(())
}

proptest! {
    #[test] fn matches_a_bounded_fifo_n2(ops in ops()) { check::<2>(&ops)?; }
    #[test] fn matches_a_bounded_fifo_n4(ops in ops()) { check::<4>(&ops)?; }
    #[test] fn matches_a_bounded_fifo_n64(ops in ops()) { check::<64>(&ops)?; }

    /// Producers and stealers on real threads: nothing lost, nothing doubled, and
    /// each producer's tokens leave in the order it pushed them.
    #[test]
    fn threads_move_each_token_once_in_producer_order(producers in 1usize..4, stealers in 1usize..4, each in 1usize..400) {
        use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
        let q = Arc::new(BoundedQueue::<usize, 8>::new());
        let taken = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let total = producers * each;
        let mut handles = Vec::new();
        for p in 0..producers {
            let q = q.clone();
            handles.push(std::thread::spawn(move || for i in 0..each {
                // Token encodes (producer, sequence); +1 keeps it non-null.
                let t = token(1 + p * 1_000_000 + i);
                while q.try_push(t).is_err() { std::thread::yield_now(); }
            }));
        }
        for _ in 0..stealers {
            let (q, taken, seen) = (q.clone(), taken.clone(), seen.clone());
            handles.push(std::thread::spawn(move || {
                let mut mine = Vec::new();
                while taken.load(Ordering::Acquire) < total {
                    let v = q.steal();
                    if v.is_null() { std::thread::yield_now(); continue; }
                    taken.fetch_add(1, Ordering::AcqRel);
                    mine.push(v as usize - 1);
                }
                seen.lock().unwrap().push(mine);
            }));
        }
        for h in handles { h.join().unwrap(); }
        let per_stealer = seen.lock().unwrap().clone();
        let mut all: Vec<usize> = per_stealer.iter().flatten().copied().collect();
        prop_assert_eq!(all.len(), total);
        // One stealer sees a producer's tokens in increasing order.
        for mine in &per_stealer {
            let mut last = vec![None; producers];
            for &v in mine {
                let (p, i) = (v / 1_000_000, v % 1_000_000);
                prop_assert!(last[p].map_or(true, |l| l < i), "producer {} order broken", p);
                last[p] = Some(i);
            }
        }
        all.sort_unstable();
        all.dedup();
        prop_assert_eq!(all.len(), total, "a token was doubled");
        prop_assert!(q.steal().is_null());
    }
}
