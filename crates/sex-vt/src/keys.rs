// SPDX-License-Identifier: GPL-3.0-only
//! Keys typed into a terminal, as the bytes an xterm sends for them.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 to F12.
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods { pub shift: bool, pub alt: bool, pub control: bool }

/// The bytes for `key`. `app_cursor` is the program's DECCKM choice (`ESC [ ? 1 h`).
pub fn encode(key: Key, mods: Mods, app_cursor: bool) -> Vec<u8> {
    let mut out = Vec::new();
    // xterm's modifier parameter: 1 + shift + 2*alt + 4*control.
    let m = 1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.control as u8;
    let csi = |out: &mut Vec<u8>, num: Option<u8>, end: u8| {
        out.extend_from_slice(b"\x1b[");
        match (num, m) {
            (Some(n), 1) => { push_num(out, n as u32); }
            (Some(n), m) => { push_num(out, n as u32); out.push(b';'); push_num(out, m as u32); }
            (None, 1) => {}
            (None, m) => { out.extend_from_slice(b"1;"); push_num(out, m as u32); }
        }
        out.push(end);
    };
    match key {
        Key::Char(c) => {
            if mods.alt { out.push(0x1b); }
            if mods.control {
                let b = c.to_ascii_lowercase();
                match b {
                    'a'..='z' => out.push(b as u8 - b'a' + 1),
                    '@' | ' ' | '2' => out.push(0),
                    '[' | '3' => out.push(0x1b),
                    '\\' | '4' => out.push(0x1c),
                    ']' | '5' => out.push(0x1d),
                    '^' | '6' => out.push(0x1e),
                    '_' | '-' | '7' => out.push(0x1f),
                    '?' | '8' => out.push(0x7f),
                    _ => push_char(&mut out, c),
                }
            } else { push_char(&mut out, c); }
        }
        Key::Enter => { if mods.alt { out.push(0x1b); } out.push(b'\r'); }
        Key::Backspace => { if mods.alt { out.push(0x1b); } out.push(if mods.control { 0x08 } else { 0x7f }); }
        Key::Tab => if mods.shift { out.extend_from_slice(b"\x1b[Z") } else { out.push(b'\t') },
        Key::Escape => out.push(0x1b),
        Key::Up | Key::Down | Key::Right | Key::Left | Key::Home | Key::End => {
            let end = match key { Key::Up => b'A', Key::Down => b'B', Key::Right => b'C', Key::Left => b'D', Key::Home => b'H', _ => b'F' };
            if app_cursor && m == 1 { out.extend_from_slice(&[0x1b, b'O', end]); } else { csi(&mut out, None, end); }
        }
        Key::Insert => csi(&mut out, Some(2), b'~'),
        Key::Delete => csi(&mut out, Some(3), b'~'),
        Key::PageUp => csi(&mut out, Some(5), b'~'),
        Key::PageDown => csi(&mut out, Some(6), b'~'),
        Key::F(n @ 1..=4) => {
            let end = b"PQRS"[n as usize - 1];
            if m == 1 { out.extend_from_slice(&[0x1b, b'O', end]); } else { csi(&mut out, None, end); }
        }
        Key::F(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            csi(&mut out, Some(CODES[n as usize - 5]), b'~');
        }
        Key::F(_) => {}
    }
    out
}

/// Text pasted into the terminal: wrapped in bracketed-paste marks when the program
/// asked for them, with newlines sent as Enter (carriage returns), and any embedded end
/// mark removed so pasted text can never end the paste early.
pub fn paste(text: &[u8], bracketed: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    if bracketed { out.extend_from_slice(b"\x1b[200~"); }
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(b"\x1b[201~") { i += 6; continue; }
        match text[i] { b'\r' if text.get(i + 1) == Some(&b'\n') => {} b'\n' => out.push(b'\r'), b => out.push(b) }
        i += 1;
    }
    if bracketed { out.extend_from_slice(b"\x1b[201~"); }
    out
}

fn push_num(out: &mut Vec<u8>, mut n: u32) {
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    loop { i -= 1; digits[i] = b'0' + (n % 10) as u8; n /= 10; if n == 0 { break; } }
    out.extend_from_slice(&digits[i..]);
}
fn push_char(out: &mut Vec<u8>, c: char) {
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}
