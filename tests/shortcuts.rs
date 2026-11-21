// SPDX-License-Identifier: GPL-3.0-only
//! Host test for the person's shortcut keys (R3): servers/silk-shell/src/shortcuts.rs
//! against the sex-config schema Settings writes.
#[path = "../servers/silk-shell/src/shortcuts.rs"]
mod shortcuts;
use shortcuts::*;

const D: u8 = 0x20;
const W: u8 = 0x11;
const ESC: u8 = 0x01;
const OWN: [u8; 3] = [D, W, ESC];
fn place(name: &str) -> i32 { sex_config::SHORTCUT_KEYS.iter().position(|k| *k == name).unwrap() as i32 }

#[test]
fn every_key_name_has_a_code() {
    let names = sex_config::SHORTCUT_KEYS;
    assert_eq!(names.len(), CODES.len() + 2);
    assert_eq!((key(0), key(1)), (Key::Own, Key::Off));
    assert_eq!(key(place("d")), Key::Code(D));
    assert_eq!(key(place("w")), Key::Code(W));
    assert_eq!(key(place("escape")), Key::Code(ESC));
    assert_eq!(key(place("enter")), Key::Code(0x1C));
    assert_eq!(key(999), Key::Own, "a newer system's key keeps the default");
    let mut codes = CODES.to_vec();
    codes.sort();
    codes.dedup();
    assert_eq!(codes.len(), CODES.len());
}

#[test]
fn defaults_leave_the_built_in_table() {
    for code in [D, W, ESC, 0x10] { assert_eq!(pick(code, &OWN, &[0, 0, 0]), None); }
}

#[test]
fn a_moved_shortcut_takes_its_new_key_and_frees_its_own() {
    // The launcher on Super+A: Super+A opens it, Super+D does nothing.
    let chosen = [place("a"), 0, 0];
    assert_eq!(pick(0x1E, &OWN, &chosen), Some(Some(0)));
    assert_eq!(pick(D, &OWN, &chosen), Some(None));
    assert_eq!(pick(W, &OWN, &chosen), None);
    // Turned off: its key does nothing.
    assert_eq!(pick(ESC, &OWN, &[0, 0, 1]), Some(None));
    // Two shortcuts swapped.
    let swapped = [place("w"), place("d"), 0];
    assert_eq!((pick(D, &OWN, &swapped), pick(W, &OWN, &swapped)), (Some(Some(1)), Some(Some(0))));
    // Chosen keys win over another shortcut's own key.
    assert_eq!(pick(W, &OWN, &[place("w"), 0, 0]), Some(Some(0)));
}
