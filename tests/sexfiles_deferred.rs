#![allow(dead_code)]
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"]
pub mod app_lifecycle;
#[path = "../servers/sexfiles/src/defer_queue.rs"]
mod queue;
use app_lifecycle::OP_APP_EXIT;
#[test]
fn full_ordinary_backlog_cannot_consume_kernel_terminal_capacity() {
    let mut q = queue::Deferred::new();
    let requests: Vec<_> = (0..8).map(|i| (0x4D, 9, i, 0, 0)).collect();
    for &m in &requests {
        assert!(q.push(m));
    }
    assert!(!q.push((0x4D, 16, 99, 0, 0)));
    for pd in [9, 12, 15, 16, 17] {
        assert!(q.push((OP_APP_EXIT, 0, pd, pd, 0)));
    }
    assert_eq!(q.len(), 13);
    // Replayed hints coalesce and cannot crowd out another terminal subject.
    for _ in 0..100 {
        assert!(q.push((OP_APP_EXIT, 0, 9, 9, 0)));
    }
    for pd in [9, 12, 15, 16, 17] {
        assert_eq!(q.pop(), Some((OP_APP_EXIT, 0, pd, pd, 0)));
    }
    for m in requests {
        assert_eq!(q.pop(), Some(m));
    }
    assert_eq!(q.pop(), None);
}
#[test]
fn forged_or_wide_exit_payloads_have_no_priority_and_queue_is_reusable() {
    let mut q = queue::Deferred::new();
    let fake = (OP_APP_EXIT, 3, 9, 9, 0);
    assert!(q.push(fake));
    assert!(q.push((OP_APP_EXIT, 0, 16, 16, 0)));
    assert_eq!(q.pop(), Some((OP_APP_EXIT, 0, 16, 16, 0)));
    assert_eq!(q.pop(), Some(fake));
    for m in [
        (OP_APP_EXIT, 0, 0, 9, 0),
        (OP_APP_EXIT, 0, 9, 0, 0),
        (OP_APP_EXIT, 0, 1u64 << 32, 9, 0),
        (OP_APP_EXIT, 0, 9, 9, 1),
    ] {
        assert!(q.push(m));
        assert!(q.push((OP_APP_EXIT, 0, 9, 9, 0)));
        assert_eq!(q.pop(), Some((OP_APP_EXIT, 0, 9, 9, 0)));
        assert_eq!(q.pop(), Some(m));
    }
    assert_eq!(q.len(), 0);
}
