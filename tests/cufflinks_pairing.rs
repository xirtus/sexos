//! Host test for Cufflinks's pairing policy (`servers/cufflinks/src/pairing.rs`), the
//! exact file the server compiles: with the desktop prompt a new device pairs only
//! when the user asks, and once anything is bonded nothing pairs without the user.
#[path = "../servers/cufflinks/src/pairing.rs"]
mod pairing;
use pairing::*;

const KB: [u8; 6] = [1, 2, 3, 4, 5, 6];
const HP: [u8; 6] = [9, 8, 7, 6, 5, 4];

#[test]
fn prompt_means_the_user_decides() {
    // First setup, nothing bonded, inside the window: still nothing on our own.
    let p = Policy::new(1_000, 0);
    assert!(p.window_open(1_000));
    assert!(!p.connect_new(1_000, true));
    assert_eq!(p.compare(1_000, true), Answer::Ask);
    // Bonded, window shut: still asked, never accepted on its own.
    let p = Policy::new(1_000, 2);
    assert!(!p.connect_new(1_000, true));
    assert_eq!(p.compare(1_000, true), Answer::Ask);
}

#[test]
fn without_a_prompt_only_first_setup_pairs() {
    let p = Policy::new(1_000, 0);
    assert_eq!(p.window_seconds(1_000), 120);
    assert!(p.connect_new(1_000 + WINDOW_TICKS - 1, false));
    assert_eq!(p.compare(1_000 + WINDOW_TICKS - 1, false), Answer::Accept);
    assert!(!p.connect_new(1_000 + WINDOW_TICKS, false));
    assert_eq!(p.compare(1_000 + WINDOW_TICKS, false), Answer::Refuse);
    // Something bonded: strangers are never auto-accepted, window or not.
    let p = Policy::new(1_000, 1);
    assert!(!p.window_open(1_000));
    assert_eq!(p.window_seconds(1_000), 0);
    assert!(!p.connect_new(1_000, false));
    assert_eq!(p.compare(1_000, false), Answer::Refuse);
}

#[test]
fn a_comparison_waits_for_its_own_answer() {
    let q = Pending { handle: 0x40, addr: KB, value: 123_456, ticket: 7, since: 5_000 };
    assert!(q.answers(7) && !q.answers(6), "a stale answer is ignored");
    assert!(!q.expired(5_000 + ANSWER_TICKS - 1));
    assert!(q.expired(5_000 + ANSWER_TICKS));
}

#[test]
fn requests_are_bounded_and_expire() {
    let mut r = Requests::new();
    r.add(KB, 0);
    r.add(HP, 10);
    assert!(r.waiting(&KB) && r.waiting(&HP));
    assert!(r.done(&KB) && !r.done(&KB));
    assert!(!r.waiting(&KB));
    assert_eq!(r.expired(10 + CONNECT_TICKS - 1), None);
    assert_eq!(r.expired(10 + CONNECT_TICKS), Some(HP));
    assert_eq!(r.expired(10 + CONNECT_TICKS), None);
    // Four at most: the oldest gives way.
    for i in 0..5u8 { r.add([i; 6], 100 + i as u64); }
    assert!(!r.waiting(&[0; 6]) && (1..5u8).all(|i| r.waiting(&[i; 6])));
    // Asking again restarts the clock.
    r.add([1; 6], 50_000);
    let mut gone = Vec::new();
    while let Some(a) = r.expired(104 + CONNECT_TICKS) { gone.push(a[0]); }
    gone.sort();
    assert_eq!(gone, [2, 3, 4]);
    assert!(r.waiting(&[1; 6]));
}
