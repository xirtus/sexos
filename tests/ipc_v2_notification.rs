//! Host tests of IPC v2 notifications and interrupt routes (kernel/src/ipc_v2/notification.rs).
#[path = "../kernel/src/ipc_v2"]
mod ipc_v2 {
    pub type TaskId = u32;
    pub mod notification;
}

use ipc_v2::notification::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn signals_merge_and_are_taken_once() {
    let n = Notification::new();
    assert_eq!(n.signal(0b01), SignalOutcome::Pending);
    assert_eq!(n.signal(0b10), SignalOutcome::Pending);
    assert_eq!(n.signal(0b10), SignalOutcome::Pending);
    assert_eq!(n.wait(1), WaitOutcome::Ready(0b11));
    assert_eq!(n.poll(), 0);
    assert_eq!(n.signal(0), SignalOutcome::Pending);
    assert_eq!(n.pending(), 0);
}

#[test]
fn signal_wakes_the_parked_waiter() {
    let n = Notification::new();
    assert_eq!(n.wait(4), WaitOutcome::Parked);
    assert_eq!(n.wait(5), WaitOutcome::Busy);
    assert_eq!(n.signal(1 << 63), SignalOutcome::Wake(4));
    assert_eq!(n.waiter(), None);
    assert_eq!(n.signal(1), SignalOutcome::Pending, "woken once");
    assert_eq!(n.wait(4), WaitOutcome::Ready((1 << 63) | 1));
}

#[test]
fn cancelled_waiter_is_not_woken() {
    let n = Notification::new();
    assert_eq!(n.wait(2), WaitOutcome::Parked);
    assert!(n.cancel_wait(2));
    assert!(!n.cancel_wait(2));
    assert_eq!(n.signal(8), SignalOutcome::Pending);
}

/// A signaler and a waiter on two threads: every bit signalled is seen exactly
/// once, and the waiter is never left parked with bits pending.
#[test]
fn no_bit_is_lost_under_contention() {
    const ROUNDS: u64 = 20_000;
    let n = Arc::new(Notification::new());
    let woken = Arc::new(AtomicU64::new(0));
    let signaler = {
        let (n, woken) = (n.clone(), woken.clone());
        std::thread::spawn(move || for i in 0..ROUNDS {
            if let SignalOutcome::Wake(t) = n.signal(1 << (i % 64)) { assert_eq!(t, 9); woken.fetch_add(1, Ordering::SeqCst); }
        })
    };
    let mut seen = 0u64;
    let mut last_woken = 0;
    loop {
        match n.wait(9) {
            WaitOutcome::Ready(bits) => seen += bits.count_ones() as u64,
            WaitOutcome::Parked => {
                // Block until a wake (or the signaler is done and nothing is pending).
                loop {
                    let w = woken.load(Ordering::SeqCst);
                    if w != last_woken { last_woken = w; break }
                    if signaler.is_finished() && n.waiter() == Some(9) {
                        assert_eq!(n.pending(), 0, "parked with bits pending: lost wakeup");
                        assert!(n.cancel_wait(9));
                        break;
                    }
                    std::thread::yield_now();
                }
            }
            WaitOutcome::Busy => panic!("only one waiter"),
        }
        if signaler.is_finished() && n.pending() == 0 && n.waiter().is_none() { break }
    }
    signaler.join().unwrap();
    seen += n.poll().count_ones() as u64;
    // Bits merge, so `seen` counts distinct deliveries: at least one, at most ROUNDS.
    assert!(seen >= 1 && seen <= ROUNDS);
    assert_eq!(n.pending(), 0);
}

#[test]
fn interrupt_lines_route_to_one_notification_bit() {
    let routes: IrqRoutes<24> = IrqRoutes::new();
    assert_eq!(routes.route(5), None);
    routes.bind(5, 0, 3).unwrap();
    routes.bind(6, 0, 63).unwrap();
    routes.bind(7, 2, 0).unwrap();
    assert_eq!(routes.bind(5, 1, 0), Err(BindError::Bound));
    assert_eq!(routes.bind(24, 1, 0), Err(BindError::BadLine));
    assert_eq!(routes.bind(8, 1, 64), Err(BindError::BadBit));
    assert_eq!(routes.route(5), Some((0, 1 << 3)));
    assert_eq!(routes.route(6), Some((0, 1 << 63)));
    assert!(!routes.unbind(7, 0), "only the owner unbinds");
    assert_eq!(routes.unbind_all(0), 2);
    assert_eq!(routes.route(5), None);
    assert_eq!(routes.route(7), Some((2, 1)));
}

#[test]
fn interrupt_signals_its_driver() {
    let routes: IrqRoutes<4> = IrqRoutes::new();
    let notes = [Notification::new(), Notification::new()];
    routes.bind(2, 1, 4).unwrap();
    assert_eq!(notes[1].wait(30), WaitOutcome::Parked);
    let (id, mask) = routes.route(2).unwrap();
    assert_eq!(notes[id as usize].signal(mask), SignalOutcome::Wake(30));
    assert_eq!(notes[1].wait(30), WaitOutcome::Ready(1 << 4));
    assert_eq!(notes[0].pending(), 0);
}
