use super::*;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// Opaque non-null tokens. Tests never dereference these pointer values.
fn token(n: usize) -> *mut usize { n as *mut usize }

#[test]
fn full_rejects_without_losing_fifo_or_ownership() {
    let queue = BoundedQueue::<usize, 4>::new();
    for id in 1..=4 { assert_eq!(queue.try_push(token(id)), Ok(())); }
    assert_eq!(queue.try_push(token(5)), Err(PushError::Full));
    assert_eq!(queue.steal(), token(1));
    assert_eq!(queue.try_push(token(5)), Ok(()));
    for id in 2..=5 { assert_eq!(queue.steal(), token(id)); }
    assert!(queue.steal().is_null());
}

#[test]
fn null_is_rejected_without_consuming_capacity() {
    let queue = BoundedQueue::<usize, 2>::new();
    assert_eq!(queue.try_push(ptr::null_mut()), Err(PushError::Null));
    assert_eq!(queue.bottom.load(Ordering::Relaxed), 0);
    queue.push(token(1));
    queue.push(token(2));
    assert_eq!(queue.steal(), token(1));
    assert_eq!(queue.steal(), token(2));
}

#[test]
#[should_panic(expected = "scheduler runqueue insertion failed")]
fn compatibility_push_fails_explicitly_on_saturation() {
    let queue = BoundedQueue::<usize, 2>::new();
    queue.push(token(1));
    queue.push(token(2));
    queue.push(token(3));
}

#[test]
fn positions_and_sequences_wrap_at_usize_max() {
    let queue = BoundedQueue::<usize, 4>::new();
    let start = usize::MAX - 1;
    queue.top.store(start, Ordering::Relaxed);
    queue.bottom.store(start, Ordering::Relaxed);
    for offset in 0..4 {
        let pos = start.wrapping_add(offset);
        queue.slots[pos & 3].sequence.store(pos, Ordering::Relaxed);
    }
    for cycle in 0..32 {
        for id in 1..=4 { queue.push(token(cycle * 4 + id)); }
        assert_eq!(queue.try_push(token(999)), Err(PushError::Full));
        for id in 1..=4 { assert_eq!(queue.steal(), token(cycle * 4 + id)); }
        assert!(queue.steal().is_null());
    }
}

#[test]
fn reserved_but_unpublished_producer_cannot_be_consumed() {
    let queue = BoundedQueue::<usize, 2>::new();
    // Pause producer 1 after reserving slot 0, then let producer 2 complete.
    queue.bottom.store(1, Ordering::Relaxed);
    assert_eq!(queue.try_push(token(2)), Ok(()));
    assert!(queue.steal().is_null());
    assert_eq!(queue.try_push(token(3)), Err(PushError::Full));
    queue.slots[0].value.store(token(1), Ordering::Relaxed);
    queue.slots[0].sequence.store(1, Ordering::Release);
    assert_eq!(queue.steal(), token(1));
    assert_eq!(queue.steal(), token(2));
}

#[test]
fn reserved_consumer_prevents_slot_reuse_until_read_finishes() {
    let queue = BoundedQueue::<usize, 2>::new();
    queue.push(token(1));
    queue.push(token(2));
    // Pause consumer 1 after reserving the head but before reading its value.
    queue.top.store(1, Ordering::Relaxed);
    assert_eq!(queue.steal(), token(2));
    assert_eq!(queue.try_push(token(3)), Err(PushError::Full));
    assert_eq!(queue.slots[0].value.load(Ordering::Relaxed), token(1));
    queue.slots[0].sequence.store(2, Ordering::Release);
    assert_eq!(queue.try_push(token(3)), Ok(()));
    assert_eq!(queue.steal(), token(3));
}

#[test]
fn concurrent_producers_and_consumers_transfer_each_pointer_exactly_once() {
    const PRODUCERS: usize = 4;
    const EACH: usize = 20_000;
    const TOTAL: usize = PRODUCERS * EACH;
    let queue = Arc::new(BoundedQueue::<usize, 8>::new());
    let seen = Arc::new((0..TOTAL).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
    let consumed = Arc::new(AtomicUsize::new(0));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut workers = Vec::new();
    for producer in 0..PRODUCERS {
        let queue = Arc::clone(&queue);
        workers.push(thread::spawn(move || {
            for item in 0..EACH {
                let value = token(producer * EACH + item + 1);
                loop {
                    match queue.try_push(value) {
                        Ok(()) => break,
                        Err(PushError::Full | PushError::Contended) => {
                            assert!(Instant::now() < deadline, "producer stalled");
                            thread::yield_now();
                        }
                        Err(PushError::Null) => panic!("non-null value rejected"),
                    }
                }
            }
        }));
    }
    for _ in 0..4 {
        let queue = Arc::clone(&queue);
        let seen = Arc::clone(&seen);
        let consumed = Arc::clone(&consumed);
        workers.push(thread::spawn(move || {
            while consumed.load(Ordering::Relaxed) < TOTAL {
                let value = queue.steal() as usize;
                if value == 0 {
                    assert!(Instant::now() < deadline, "consumer stalled or pointer lost");
                    thread::yield_now();
                    continue;
                }
                assert!(value <= TOTAL);
                assert_eq!(seen[value - 1].fetch_add(1, Ordering::Relaxed), 0,
                    "duplicate pointer transfer");
                consumed.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for worker in workers { worker.join().unwrap(); }
    assert_eq!(consumed.load(Ordering::Relaxed), TOTAL);
    assert!(seen.iter().all(|entry| entry.load(Ordering::Relaxed) == 1));
    assert!(queue.steal().is_null());
}

#[test]
fn publication_makes_initialized_payload_visible() {
    let queue = Arc::new(BoundedQueue::<[usize; 32], 2>::new());
    let reader = Arc::clone(&queue);
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let value = reader.steal();
            if !value.is_null() {
                let payload = unsafe { Box::from_raw(value) };
                assert_eq!(*payload, [0x5a5a; 32]);
                break;
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
    });
    queue.push(Box::into_raw(Box::new([0x5a5a; 32])));
    worker.join().unwrap();
}
