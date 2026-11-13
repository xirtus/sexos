// SPDX-License-Identifier: GPL-3.0-only
//! The person's keyboard layouts. Shell sees every key edge before any program, so
//! it keeps the one `sex_keymap::Keyboard` (modifiers, Caps Lock, dead keys,
//! Compose) and hands the focused program the character each press typed, with the
//! key code as before. Programs never keep a layout table of their own.
//! Super+Ctrl+Space steps to the next chosen layout.
use sex_pdx::app_key::Key;

/// The layouts Super+Ctrl+Space steps through until Settings chooses them (B1):
/// US first, then the common European ones, Russian and Chinese (Pinyin).
pub const DEFAULT_CHOSEN: [&str; 7] = ["us", "de", "fr", "es", "gb", "ru", "zh-pinyin"];

pub static mut KEYBOARD: sex_keymap::Keyboard = sex_keymap::Keyboard::new();
/// The Pinyin input method, used while the `zh-pinyin` layout is active.
static mut PINYIN: sex_ime::Pinyin = sex_ime::Pinyin::new();
#[allow(static_mut_refs)]
fn pinyin() -> &'static mut sex_ime::Pinyin { unsafe { &mut *core::ptr::addr_of_mut!(PINYIN) } }
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

/// While the Pinyin layout is active, offers the press to the input method.
/// `None`: not the method's, deliver the key as usual. `Some(changed)`: the
/// method took it; each committed character went to `push` as a
/// [`sex_pdx::app_key::KEY_TEXT`] key, and `changed` says the pre-edit or its
/// candidates changed (Shell shows them, see [`preedit_line`]).
pub fn compose(key: Key, raw: u8, mut push: impl FnMut(Key)) -> Option<bool> {
    use sex_ime::{Input, Outcome};
    if keyboard().layout().name != "zh-pinyin" || key.control { return None; }
    let input = match raw {
        0x01 => Input::Escape, 0x0E => Input::Backspace, 0x1C => Input::Enter, 0x39 => Input::Space,
        0x0C => Input::PrevPage, 0x0D => Input::NextPage,
        0x02..=0x0A if !key.shift => Input::Digit(raw - 1),
        _ => match keyboard().sym(raw) {
            sex_keymap::Sym::Char(c) if c.is_ascii_lowercase() && !key.shift => Input::Letter(c as u8),
            _ => Input::Other,
        },
    };
    let ime = pinyin();
    match ime.key(input) {
        Outcome::Pass => None,
        Outcome::Changed => Some(true),
        Outcome::Commit => {
            for c in ime.committed().chars() {
                push(Key { code: sex_pdx::app_key::KEY_TEXT, shift: false, control: false, text: sex_pdx::app_key::Text::Char(c) });
            }
            Some(true)
        }
    }
}

/// What the panel shows while composing: `nihao  1 你好 2 ...`, or empty.
pub fn preedit_line(out: &mut [u8]) -> usize {
    let ime = pinyin();
    let mut n = 0;
    let mut put = |s: &str, n: &mut usize| { let b = s.as_bytes(); if *n + b.len() <= out.len() { out[*n..*n + b.len()].copy_from_slice(b); *n += b.len(); } };
    if !ime.active() { return 0; }
    put(ime.preedit(), &mut n);
    for (i, word) in ime.candidates().enumerate() {
        let digit = [b' ', b' ', b'1' + i as u8, b' '];
        put(core::str::from_utf8(&digit).unwrap_or(" "), &mut n);
        put(word, &mut n);
    }
    n
}

/// Steps to the next chosen layout; returns its panel label and title.
pub fn next() -> (&'static str, &'static str) {
    pinyin().key(sex_ime::Input::Escape);
    let def = keyboard().next_layout();
    (def.label, def.title)
}

pub fn active_label() -> &'static str { keyboard().layout().label }

/// Back to the state at boot: nothing held, the first chosen layout.
#[allow(dead_code)]
pub fn reset() {
    unsafe { *core::ptr::addr_of_mut!(KEYBOARD) = sex_keymap::Keyboard::new(); CHOSEN = false; }
    *pinyin() = sex_ime::Pinyin::new();
}
