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
#[test]
fn pipe_prepare_round_trip() {
    let word = stdio(PIPED, NULL, INHERIT);
    assert_eq!(stream(word, 0), PIPED);
    assert_eq!(stream(word, 1), NULL);
    assert_eq!(stream(word, 2), INHERIT);
    for slot in 0..SLOTS {
        let offset = pipe_prepare(slot, word);
        assert_eq!(split_pipe_prepare(offset), Some((slot, word)));
        // Never a terminal's PREPARE (low byte 0) nor a plain launch's.
        assert_eq!(offset & 0xFF, sex_pdx::fs::PREPARE_PIPED);
    }
    assert_eq!(split_pipe_prepare(sex_pdx::fs::PREPARE_LAUNCH), None);
    assert_eq!(split_pipe_prepare(at_slot(0, 3)), None);
    assert_eq!(split_pipe_prepare(pipe_prepare(0, 0) | (SLOTS as u64) << SLOT_SHIFT), None);
    assert_eq!(split_pipe_prepare(pipe_prepare(0, 0) | 1 << 40), None);
}
#[test]
fn env_block_reads_in_pieces() {
    let (cwd, env) = (b"@/data/a/work".as_slice(), b"A=1\0LANG=C.UTF-8\0".as_slice());
    let mut whole = Vec::new();
    let mut piece = [0u8; 5];
    loop {
        let n = env_block(cwd, env, whole.len(), &mut piece);
        if n == 0 { break; }
        whole.extend_from_slice(&piece[..n]);
    }
    assert_eq!(whole, b"@/data/a/work\0A=1\0LANG=C.UTF-8\0");
    let (folder, entries) = split_env_block(&whole);
    assert_eq!(folder, b"@/data/a/work");
    assert_eq!(entries.collect::<Vec<_>>(), vec![b"A=1".as_slice(), b"LANG=C.UTF-8".as_slice()]);
    let (folder, entries) = split_env_block(b"\0");
    assert!(folder.is_empty());
    assert_eq!(entries.count(), 0);
    assert_eq!(split_env_block(b"").0, b"");
}
#[test]
fn a_yarn_ring_keeps_the_newest_bytes() {
    let mut r: Ring<8> = Ring::new();
    r.push_over(b"abcde");
    r.push_over(b"fghij");
    let mut out = [0u8; 8];
    assert_eq!(r.pop(&mut out), 8);
    assert_eq!(&out, b"cdefghij");
    // More than the ring holds at once: only its tail.
    r.push_over(b"0123456789AB");
    assert_eq!(r.pop(&mut out), 8);
    assert_eq!(&out, b"456789AB");
    // Room left: nothing dropped, and it wraps.
    r.push(b"xyz");
    r.pop(&mut out[..2]);
    r.push_over(b"1234567");
    assert_eq!(r.len(), 8);
    assert_eq!(r.pop(&mut out), 8);
    assert_eq!(&out, b"z1234567");
}
#[test]
fn detach_words_and_yarn_records() {
    assert_eq!(split_detach(detach_word(20_000, 12)), Some((20_000, 12)));
    assert_eq!(split_detach(detach_word(0, LABEL_BYTES + 1)), None);
    let mut label = [0u8; LABEL_BYTES];
    label[..14].copy_from_slice(b"stdhello count");
    let y = Yarn { name: 7, state: RUNNING, code: 0, target: 40, label, label_len: 14 };
    let mut out = [0u8; YARN_BYTES];
    y.encode(&mut out);
    assert_eq!(Yarn::decode(&out), Some(y));
    assert_eq!(y.label(), b"stdhello count");
    assert_eq!(Yarn::decode(&out[..YARN_BYTES - 1]), None);
}
