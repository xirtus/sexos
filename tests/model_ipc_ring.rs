// SPDX-License-Identifier: GPL-3.0-only
//! Model-based property test of the kernel IPC message ring (kernel/src/ipc_ring.rs,
//! kernel plan phase 7). Sequential op sequences must match a bounded VecDeque
//! exactly; a threaded run checks many producers into one consumer (the real use:
//! every client enqueues into a server's single ring) with per-producer order kept.
//! Interleavings at every atomic are checked by loom in model_loom_queues.rs.
//! Run: cargo test --manifest-path fuzz/models/Cargo.toml --test model_ipc_ring
use proptest::prelude::*;
use std::collections::VecDeque;

#[path = "../kernel/src/ipc_ring.rs"]
#[allow(dead_code)]
mod ipc_ring;
use ipc_ring::RingBuffer;

/// A message the size and shape of an IPC call (several words, padding-free).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Msg { id: u64, a: u64, b: u64, c: u32, d: u32 }
fn msg(id: u64) -> Msg { Msg { id, a: id ^ 0xA5A5, b: !id, c: id as u32, d: (id >> 32) as u32 } }

#[derive(Clone, Debug)]
enum Op { Enqueue(u64), Dequeue, Len }

fn ops() -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(prop_oneof![
        5 => any::<u64>().prop_map(Op::Enqueue),
        5 => Just(Op::Dequeue),
        1 => Just(Op::Len),
    ], 0..600)
}

fn check<const N: usize>(ops: &[Op]) -> Result<(), TestCaseError> {
    let ring = RingBuffer::<Msg, N>::new();
    let mut model = VecDeque::new();
    for (step, op) in ops.iter().enumerate() {
        match *op {
            Op::Enqueue(v) => {
                let want = model.len() < N;
                if want { model.push_back(msg(v)); }
                prop_assert_eq!(ring.enqueue(msg(v)).is_ok(), want, "step {} {:?}", step, op);
            }
            Op::Dequeue => prop_assert_eq!(ring.dequeue(), model.pop_front(), "step {} {:?}", step, op),
            Op::Len => prop_assert_eq!(ring.len(), model.len()),
        }
    }
    Ok(())
}

proptest! {
    #[test] fn matches_a_bounded_fifo_n1(ops in ops()) { check::<1>(&ops)?; }
    #[test] fn matches_a_bounded_fifo_n2(ops in ops()) { check::<2>(&ops)?; }
    #[test] fn matches_a_bounded_fifo_n3(ops in ops()) { check::<3>(&ops)?; }
    #[test] fn matches_a_bounded_fifo_n256(ops in ops()) { check::<256>(&ops)?; }

    #[test]
    fn many_producers_one_consumer_lose_nothing(producers in 1usize..6, each in 1u64..500) {
        use std::sync::Arc;
        let ring = Arc::new(RingBuffer::<Msg, 4>::new());
        let handles: Vec<_> = (0..producers as u64).map(|p| {
            let ring = ring.clone();
            std::thread::spawn(move || for i in 0..each {
                while ring.enqueue(msg(p << 32 | i)).is_err() { std::thread::yield_now(); }
            })
        }).collect();
        let total = producers as u64 * each;
        let mut last = vec![None::<u64>; producers];
        let mut got = 0;
        while got < total {
            match ring.dequeue() {
                Some(m) => {
                    prop_assert_eq!(m, msg(m.id), "torn message");
                    let (p, i) = ((m.id >> 32) as usize, m.id & 0xFFFF_FFFF);
                    prop_assert!(last[p].map_or(true, |l| l + 1 == i), "producer {} skipped or reordered", p);
                    last[p] = Some(i);
                    got += 1;
                }
                None => std::thread::yield_now(),
            }
        }
        for h in handles { h.join().unwrap(); }
        prop_assert_eq!(ring.dequeue(), None);
        prop_assert_eq!(ring.len(), 0);
    }
}
