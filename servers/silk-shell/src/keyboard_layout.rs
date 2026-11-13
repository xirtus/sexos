// SPDX-License-Identifier: GPL-3.0-only
//! The person's keyboard layouts. Shell sees every key edge before any program, so
//! it keeps the one `sex_keymap::Keyboard` (modifiers, Caps Lock, dead keys,
//! Compose) and hands the focused program the character each press typed, with the
//! key code as before. Programs never keep a layout table of their own.
//! Super+Ctrl+Space steps to the next chosen layout.
use sex_pdx::app_key::Key;

/// The layouts Super+Ctrl+Space steps through until Settings chooses them (B1):
/// US first, then the common European ones and Russian.
pub const DEFAULT_CHOSEN: [&str; 6] = ["us", "de", "fr", "es", "gb", "ru"];

pub static mut KEYBOARD: sex_keymap::Keyboard = sex_keymap::Keyboard::new();
static mut CHOSEN: bool = false;

#[allow(static_mut_refs)]
fn keyboard() -> &'static mut sex_keymap::Keyboard {
    let k = unsafe { &mut *core::ptr::addr_of_mut!(KEYBOARD) };
    unsafe { if !CHOSEN { CHOSEN = true; k.choose(DEFAULT_CHOSEN); } }
    k
}

/// Every key edge. True for keys that only change what later keys type
/// (AltGr, Caps Lock, Compose): programs do not get those.
pub fn observe(code: u8, value: u64) -> bool {
    let k = keyboard();
    let held = k.observe(code, value);
    // Shift, Ctrl and Alt stay keys of their own for Shell's other users.
    held && matches!(code, sex_keymap::code::ALT_GR | sex_keymap::code::CAPS_LOCK | sex_keymap::code::MENU)
}

/// The key a program gets for a press of `raw` (the code before any per-program
/// renumbering), carrying what it typed under the active layout.
pub fn typing(key: Key, raw: u8) -> Key {
    let typed = match keyboard().press(raw) {
        sex_keymap::Typed::Char(c) => Some(c),
        _ => None,
    };
    key.typing(typed)
}

/// Steps to the next chosen layout; returns its panel label and title.
pub fn next() -> (&'static str, &'static str) {
    let def = keyboard().next_layout();
    (def.label, def.title)
}

pub fn active_label() -> &'static str { keyboard().layout().label }

/// Back to the state at boot: nothing held, the first chosen layout.
#[allow(dead_code)]
pub fn reset() {
    unsafe { *core::ptr::addr_of_mut!(KEYBOARD) = sex_keymap::Keyboard::new(); CHOSEN = false; }
}
