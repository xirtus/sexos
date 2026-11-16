// SPDX-License-Identifier: GPL-3.0-only
//! Host test for the Shell's accessibility aids (servers/silk-shell/src/access.rs):
//! magnifier and filter chords, sticky keys, slow keys and bounce keys.
#[path = "../servers/silk-shell/src/access.rs"]
mod access;
#[path = "../crates/sex-pdx/src/access_view.rs"]
mod access_view;

use access::{code, Access, Change, Events, Verdict};

fn key(a: &mut Access, c: u8, v: u64, now: u64) -> (Verdict, Vec<(u8, u64)>) {
    let mut e = Events::new();
    let verdict = a.key(c, v, now, &mut e);
    (verdict, e.as_slice().to_vec())
}
fn tap(a: &mut Access, c: u8, now: u64) -> Vec<(Verdict, Vec<(u8, u64)>)> {
    vec![key(a, c, 1, now), key(a, c, 0, now + 10)]
}
fn chord(a: &mut Access, c: u8) -> Verdict {
    key(a, code::SUPER_LEFT, 1, 0);
    key(a, code::ALT_LEFT, 1, 0);
    let (v, _) = key(a, c, 1, 0);
    key(a, c, 0, 0);
    key(a, code::ALT_LEFT, 0, 0);
    key(a, code::SUPER_LEFT, 0, 0);
    v
}

#[test]
fn shell_constants_match_display() {
    assert_eq!(access::ZOOM_ONE, access_view::ZOOM_ONE);
    assert_eq!(access::ZOOM_MAX, access_view::ZOOM_MAX);
    assert_eq!(access::FILTER_COUNT as usize, access_view::FILTERS.len());
    for (i, name) in access::FILTER_NAMES.iter().enumerate() { assert_eq!(*name, access_view::FILTERS[i].0); }
    assert_eq!(access_view::Filter::from_index(access::FILTER_INVERTED as u64), Some(access_view::Filter::Inverted));
    for z in access::ZOOM_ONE..=access::ZOOM_MAX {
        for up in [true, false] { assert_eq!(access::zoom_step(z, up), access_view::zoom_step(z, up)); }
    }
}

#[test]
fn magnifier_chords() {
    let mut a = Access::new();
    assert_eq!(chord(&mut a, code::DIGIT_8), Verdict::Chord(Change::View { zoom: 8, filter: 0 }));
    assert_eq!(chord(&mut a, code::EQUAL), Verdict::Chord(Change::View { zoom: 10, filter: 0 }));
    assert_eq!(chord(&mut a, code::MINUS), Verdict::Chord(Change::View { zoom: 8, filter: 0 }));
    // Off, then back on at the zoom it had.
    assert_eq!(chord(&mut a, code::DIGIT_8), Verdict::Chord(Change::View { zoom: 4, filter: 0 }));
    assert_eq!(chord(&mut a, code::DIGIT_8), Verdict::Chord(Change::View { zoom: 8, filter: 0 }));
    for _ in 0..20 { chord(&mut a, code::EQUAL); }
    assert_eq!(a.zoom, access::ZOOM_MAX);
    for _ in 0..30 { chord(&mut a, code::MINUS); }
    assert_eq!(a.zoom, access::ZOOM_ONE);
    let mut text = [0u8; 64];
    let n = a.notice(Change::View { zoom: 10, filter: 1 }, &mut text);
    assert_eq!(&text[..n], b"Magnifier 2.5x  Colours: High contrast");
}

#[test]
fn filter_chords_cycle_and_invert() {
    let mut a = Access::new();
    for want in [1, 2, 3, 4, 5, 0] {
        assert_eq!(chord(&mut a, code::C), Verdict::Chord(Change::View { zoom: 4, filter: want }));
    }
    assert_eq!(chord(&mut a, code::I), Verdict::Chord(Change::View { zoom: 4, filter: 2 }));
    assert_eq!(chord(&mut a, code::I), Verdict::Chord(Change::View { zoom: 4, filter: 0 }));
}

#[test]
fn keys_without_super_alt_pass() {
    let mut a = Access::new();
    assert_eq!(key(&mut a, code::DIGIT_8, 1, 0).0, Verdict::Pass);
    key(&mut a, code::SUPER_LEFT, 1, 0);
    assert_eq!(key(&mut a, code::DIGIT_8, 1, 0).0, Verdict::Pass, "Super+8 is the workspace key");
    key(&mut a, code::SUPER_LEFT, 0, 0);
    key(&mut a, code::ALT_LEFT, 1, 0);
    assert_eq!(key(&mut a, code::C, 1, 0).0, Verdict::Pass, "Alt+C belongs to programs");
}

#[test]
fn five_shift_taps_toggle_sticky_keys() {
    let mut a = Access::new();
    for _ in 0..4 { assert_eq!(tap(&mut a, code::SHIFT_LEFT, 0)[0].0, Verdict::Pass); }
    assert_eq!(key(&mut a, code::SHIFT_LEFT, 1, 0).0, Verdict::Chord(Change::StickyKeys(true)));
    key(&mut a, code::SHIFT_LEFT, 0, 0);
    // Another key in between starts the count again.
    let mut b = Access::new();
    for _ in 0..3 { tap(&mut b, code::SHIFT_LEFT, 0); }
    tap(&mut b, 0x1E, 0);
    for _ in 0..2 { tap(&mut b, code::SHIFT_LEFT, 0); }
    assert!(!b.sticky);
}

#[test]
fn sticky_shift_applies_to_the_next_key_once() {
    let mut a = Access::new();
    assert_eq!(chord(&mut a, code::S), Verdict::Chord(Change::StickyKeys(true)));
    // Ctrl tapped alone: its press passes, its release is held back.
    assert_eq!(key(&mut a, code::CTRL_LEFT, 1, 0).0, Verdict::Pass);
    assert_eq!(key(&mut a, code::CTRL_LEFT, 0, 0).0, Verdict::Replace);
    // C becomes Ctrl+C: the modifier is pressed around it and released after.
    assert_eq!(key(&mut a, code::C, 1, 0), (Verdict::Replace, vec![(code::CTRL_LEFT, 1), (code::C, 1)]));
    assert_eq!(key(&mut a, code::C, 0, 0), (Verdict::Replace, vec![(code::C, 0), (code::CTRL_LEFT, 0)]));
    // Used up: the next key is plain.
    assert_eq!(key(&mut a, code::C, 1, 0).0, Verdict::Pass);
}

#[test]
fn sticky_double_tap_locks_until_third_tap() {
    let mut a = Access::new();
    chord(&mut a, code::S);
    tap(&mut a, code::SHIFT_LEFT, 0);
    tap(&mut a, code::SHIFT_LEFT, 0);
    for _ in 0..3 {
        assert_eq!(key(&mut a, 0x1E, 1, 0), (Verdict::Replace, vec![(code::SHIFT_LEFT, 1), (0x1E, 1)]));
        assert_eq!(key(&mut a, 0x1E, 0, 0), (Verdict::Replace, vec![(0x1E, 0), (code::SHIFT_LEFT, 0)]));
    }
    // Third tap: off, and its release passes so programs see Shift up.
    let taps = tap(&mut a, code::SHIFT_LEFT, 0);
    assert_eq!(taps[1].0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x1E, 1, 0).0, Verdict::Pass);
}

#[test]
fn sticky_held_modifier_works_normally() {
    let mut a = Access::new();
    chord(&mut a, code::S);
    assert_eq!(key(&mut a, code::SHIFT_LEFT, 1, 0).0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x1E, 1, 0).0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x1E, 0, 0).0, Verdict::Pass);
    // Released after use in a chord: not latched.
    assert_eq!(key(&mut a, code::SHIFT_LEFT, 0, 0).0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x1E, 1, 0).0, Verdict::Pass);
}

#[test]
fn slow_keys_type_only_held_keys() {
    let mut a = Access::new();
    assert_eq!(chord(&mut a, code::K), Verdict::Chord(Change::SlowKeys(true)));
    // A brush: pressed and released before the delay, nothing types.
    assert_eq!(key(&mut a, 0x1E, 1, 1000).0, Verdict::Replace);
    let mut e = Events::new();
    assert!(!a.poll(1100, &mut e));
    assert_eq!(key(&mut a, 0x1E, 0, 1150), (Verdict::Replace, vec![]));
    assert!(!a.poll(2000, &mut e));
    // Held: typed once the delay passes, released normally.
    key(&mut a, 0x1E, 1, 3000);
    assert!(!a.poll(3000 + access::SLOW_MS - 1, &mut e));
    assert!(a.poll(3000 + access::SLOW_MS, &mut e));
    assert_eq!(e.as_slice(), &[(0x1E, 1)]);
    // Hardware repeats while held pass on (typematic), the release too.
    assert_eq!(key(&mut a, 0x1E, 2, 3500).0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x1E, 0, 3600).0, Verdict::Pass);
}

#[test]
fn bounce_keys_drop_quick_second_presses() {
    let mut a = Access::new();
    assert_eq!(chord(&mut a, code::B), Verdict::Chord(Change::BounceKeys(true)));
    assert_eq!(key(&mut a, 0x1E, 1, 1000).0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x1E, 0, 1050).0, Verdict::Pass);
    // A bounce: the press and its release are both dropped.
    assert_eq!(key(&mut a, 0x1E, 1, 1100).0, Verdict::Replace);
    assert_eq!(key(&mut a, 0x1E, 0, 1120).0, Verdict::Replace);
    // Another key is unaffected; the same key later is fine.
    assert_eq!(key(&mut a, 0x30, 1, 1130).0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x30, 0, 1140).0, Verdict::Pass);
    assert_eq!(key(&mut a, 0x1E, 1, 1050 + access::BOUNCE_MS + 1).0, Verdict::Pass);
}
