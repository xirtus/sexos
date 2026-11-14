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
    let s = Status { state: ENDED, code: 3, target: 42 };
    let mut b = [0; STATUS_BYTES];
    s.encode(&mut b);
    assert_eq!(Status::decode(&b), s);
}
