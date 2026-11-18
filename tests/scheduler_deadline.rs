// Host tests for kernel/src/scheduler_deadline.rs (scripts/scheduler_deadline_host_gate.sh).
#[path = "../kernel/src/scheduler_deadline.rs"]
mod deadline;
use deadline::*;

fn token(n: usize) -> *mut u8 { (n * 16) as *mut u8 }

#[test]
fn earliest_deadline_first_then_empty() {
    let set = DeadlineSet::<u8>::new();
    assert!(set.is_empty());
    set.try_push(token(1), 300).unwrap();
    set.try_push(token(2), 100).unwrap();
    set.try_push(token(3), 200).unwrap();
    assert!(!set.is_empty());
    assert_eq!(set.take(), token(2));
    assert_eq!(set.take(), token(3));
    assert_eq!(set.take(), token(1));
    assert!(set.take().is_null());
    assert!(set.is_empty());
}

#[test]
fn deadlines_compare_across_clock_wrap() {
    let set = DeadlineSet::<u8>::new();
    set.try_push(token(1), 5).unwrap();
    set.try_push(token(2), u64::MAX - 5).unwrap();
    assert_eq!(set.take(), token(2), "a deadline just before the wrap is earlier");
}

#[test]
fn full_set_hands_the_task_back() {
    let set = DeadlineSet::<u8>::new();
    for i in 1..=SLOTS { set.try_push(token(i), i as u64).unwrap(); }
    assert_eq!(set.try_push(token(99), 0), Err(token(99)));
    assert_eq!(set.take(), token(1));
    set.try_push(token(99), 0).unwrap();
    assert_eq!(set.take(), token(99));
}

#[test]
fn concurrent_publishers_lose_nothing() {
    use std::sync::Arc;
    let set = Arc::new(DeadlineSet::<u8>::new());
    let threads: Vec<_> = (0..4).map(|t| { let set = set.clone(); std::thread::spawn(move || {
        for i in 0..2 { while set.try_push(token(1 + t * 2 + i), (t * 2 + i) as u64).is_err() { std::thread::yield_now(); } }
    }) }).collect();
    for t in threads { t.join().unwrap(); }
    let mut got: Vec<usize> = std::iter::from_fn(|| { let t = set.take(); (!t.is_null()).then_some(t as usize / 16) }).collect();
    got.sort();
    assert_eq!(got, (1..=8).collect::<Vec<_>>());
}

#[test]
fn budget_caps_deadline_share_per_window() {
    let b = Budget::new();
    // Window 0 starts at tick 10: the first full window measures 10 ms as 10M cycles.
    b.switched(10, 1_000_000, false);
    b.switched(20, 11_000_000, false);
    assert!(!b.exhausted());
    assert_eq!(b.cycles(1000), 1_000_000, "1 ms at 1 GHz");
    // Deadline work runs 4M cycles: under half the window.
    b.switched(21, 12_000_000, true);
    b.switched(22, 16_000_000, false);
    assert!(!b.exhausted());
    // 2M more: 6M of 10M, over half.
    b.switched(23, 17_000_000, true);
    assert!(b.running());
    b.switched(24, 19_000_000, false);
    assert!(b.exhausted());
    // The next window starts clean.
    b.switched(30, 21_000_000, false);
    assert!(!b.exhausted());
}

#[test]
fn periods_clamp_and_defaults_cover_display_and_sound() {
    assert_eq!(clamp_period_us(0), 0);
    assert_eq!(clamp_period_us(1), PERIOD_MIN_US);
    assert_eq!(clamp_period_us(10_000_000), PERIOD_MAX_US);
    assert_eq!(default_period_us(1), 16_667);
    assert_eq!(default_period_us(10), 5_000);
    assert_eq!(default_period_us(11), 0);
}
