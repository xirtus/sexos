// SPDX-License-Identifier: GPL-3.0-only
//! Shortcut keys the person chose (Settings' Shortcuts page, `shortcuts.*`): which
//! Super chord means which shortcut. Each setting is a place in
//! `sex_config::SHORTCUT_KEYS`: "default" keeps the shortcut's own key, "none" gives
//! it no key, and a letter, Enter or Escape moves it there. A moved shortcut frees
//! its own key, and a chosen key wins over the built-in table (`wm::binding`). Only
//! plain Super chords are changed: Shift and Ctrl chords stay the built-in ones.
//! Pure, host-tested (`tests/shortcuts.rs`); `shortcuts_glue.rs` reads the settings.

/// Set-1 key codes of `SHORTCUT_KEYS` from "a" on: a..z, Enter, Escape.
pub const CODES: [u8; 28] = [
    0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32,
    0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
    0x1C, 0x01,
];

/// A shortcut's key under a setting's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key { Own, Off, Code(u8) }

pub fn key(choice: i32) -> Key {
    match choice {
        0 => Key::Own,
        1 => Key::Off,
        n => CODES.get((n - 2) as usize).map_or(Key::Own, |c| Key::Code(*c)),
    }
}

/// What a plain Super+`code` chord means: `Some(Some(i))` shortcut `i`, `Some(None)`
/// nothing (the key's own shortcut moved away), `None` whatever the built-in table says.
/// `own[i]` is shortcut `i`'s own key, `chosen[i]` its setting.
pub fn pick(code: u8, own: &[u8], chosen: &[i32]) -> Option<Option<usize>> {
    if let Some(i) = chosen.iter().position(|c| key(*c) == Key::Code(code)) { return Some(Some(i)); }
    own.iter().zip(chosen).any(|(o, c)| *o == code && key(*c) != Key::Own).then_some(None)
}
