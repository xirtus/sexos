// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
use sex_pdx::console_relay::*;

#[test]
fn ring_wraps_and_fills() {
    let mut r = Ring::<8>::new();
    assert_eq!(r.push(b"abcdef"), 6);
    let mut o = [0; 4];
    assert_eq!(r.pop(&mut o), 4);
    assert_eq!(&o, b"abcd");
    assert_eq!(r.push(b"ghijklmn"), 6);
    let mut all = [0; 16];
    let n = r.pop(&mut all);
    assert_eq!(&all[..n], b"efghijkl");
}
#[test]
fn grants_and_paths_dropped() {
    let mut o = [0; 64];
    let n = plain_args(b"+net  -c @/data x +packages print(1)", &mut o);
    assert_eq!(&o[..n], b"-c x print(1)");
}
#[test]
fn status_round_trip() {
    let s = Status { state: ENDED, mode: RAW | NO_ECHO, code: 3, target: 42, task: 7 };
    let mut b = [0; STATUS_BYTES];
    s.encode(&mut b);
    assert_eq!(Status::decode(&b), s);
}
#[test]
fn status_reads_as_before_modes() {
    // A status written before modes and tasks existed: byte 1 and bytes 12..16 were 0.
    let mut b = [0u8; STATUS_BYTES];
    b[0] = RUNNING;
    b[4..8].copy_from_slice(&5u32.to_le_bytes());
    b[8..12].copy_from_slice(&40u32.to_le_bytes());
    assert_eq!(Status::decode(&b), Status { state: RUNNING, mode: 0, code: 5, target: 40, task: 0 });
    // Unknown mode bits are dropped.
    b[1] = 0xF0 | RAW;
    assert_eq!(Status::decode(&b).mode, RAW);
}
#[test]
fn slot_in_offset() {
    for op in [ATTACHED, WRITE, READ, EXIT, PUT, TAKE, INTERRUPT, HANG_UP, RELEASE, SIZE, SET_MODE, POLL, SET_SIZE] {
        assert!(op < 1 << SLOT_SHIFT);
        // Slot 0 is the operation alone: callers from before slots keep working.
        assert_eq!(at_slot(op, 0), op);
        assert_eq!(split(op), Some((op, 0)));
        for slot in 0..SLOTS { assert_eq!(split(at_slot(op, slot)), Some((op, slot))); }
    }
    assert_eq!(split(at_slot(TAKE, SLOTS)), None);
    assert_eq!(split(1 << 40 | TAKE), None);
    assert_eq!(at_slot(PUT, 3), 0x304);
}
#[test]
fn size_words() {
    assert_eq!(size_word(80, 24), 80 | 24 << 16);
    assert_eq!(split_size(size_word(132, 50)), Some((132, 50)));
    assert_eq!(split_size(1 << 32), None);
    let mut s = Size::default();
    assert!(!s.known());
    s.set(80, 24);
    s.set(80, 24);
    assert_eq!(s, Size { cols: 80, rows: 24, changes: 1 });
    s.set(100, 30);
    assert!(s.known() && s.changes == 2);
    let mut b = [0u8; SIZE_BYTES];
    s.encode(&mut b);
    assert_eq!(Size::decode(&b), s);
    assert_eq!(Size::decode(&b[..4]), Size::default());
}
#[test]
fn mode_bits_distinct() {
    assert_eq!(RAW & NO_ECHO, 0);
    assert_eq!((RAW | NO_ECHO) & NO_SIGNALS, 0);
    assert_eq!(MODES, 7);
}
#[test]
fn pool_hands_out_each_pair_once() {
    let mut p = Pool::<3>::new();
    assert_eq!(p.free(), 3);
    let (a, b, c) = (p.take().unwrap(), p.take().unwrap(), p.take().unwrap());
    assert_eq!((a, b, c), (0, 1, 2));
    assert_eq!(p.take(), None);
    p.give(1);
    assert_eq!(p.free(), 1);
    assert_eq!(p.take(), Some(1));
    p.give(7); // out of range: ignored
    assert_eq!(p.free(), 0);
}
