// SPDX-License-Identifier: GPL-3.0-only
//! SexOS keys and pointer events as egui input events.
//!
//! Shell sends a key's PS/2 set 1 code (or Input's normalized code for the extended
//! keys), Shift and Ctrl as they were held, and what the person's keyboard layout typed.
//! It sends presses only, so each press becomes a press and a release for egui.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};

/// The egui key for a SexOS key code, if egui names one.
pub fn key(code: u8) -> Option<Key> {
    use Key::*;
    const LETTERS: [(u8, Key); 26] = [
        (0x1E, A), (0x30, B), (0x2E, C), (0x20, D), (0x12, E), (0x21, F), (0x22, G), (0x23, H), (0x17, I),
        (0x24, J), (0x25, K), (0x26, L), (0x32, M), (0x31, N), (0x18, O), (0x19, P), (0x10, Q), (0x13, R),
        (0x1F, S), (0x14, T), (0x16, U), (0x2F, V), (0x11, W), (0x2D, X), (0x15, Y), (0x2C, Z),
    ];
    if let Some((_, k)) = LETTERS.iter().find(|(c, _)| *c == code) { return Some(*k); }
    Some(match code {
        0x02 => Num1, 0x03 => Num2, 0x04 => Num3, 0x05 => Num4, 0x06 => Num5,
        0x07 => Num6, 0x08 => Num7, 0x09 => Num8, 0x0A => Num9, 0x0B => Num0,
        0x01 => Escape, 0x0C => Minus, 0x0D => Equals, 0x0E => Backspace, 0x0F => Tab,
        0x1A => OpenBracket, 0x1B => CloseBracket, 0x1C => Enter, 0x27 => Semicolon, 0x28 => Quote,
        0x29 => Backtick, 0x2B => Backslash, 0x33 => Comma, 0x34 => Period, 0x35 => Slash, 0x39 => Space,
        0x3B => F1, 0x3C => F2, 0x3D => F3, 0x3E => F4, 0x3F => F5, 0x40 => F6, 0x41 => F7, 0x42 => F8,
        0x43 => F9, 0x44 => F10, 0x57 => F11, 0x58 => F12,
        0x48 | 0x67 => ArrowUp, 0x50 | 0x6C => ArrowDown, 0x4B | 0x69 => ArrowLeft, 0x4D | 0x6A => ArrowRight,
        0x47 | 0x66 => Home, 0x4F | 0x6B => End, 0x49 | 0x68 => PageUp, 0x51 | 0x6D => PageDown,
        0x52 | 0x6E => Insert, 0x53 | 0x6F => Delete,
        _ => return None,
    })
}

pub fn modifiers(shift: bool, control: bool) -> Modifiers {
    Modifiers { alt: false, ctrl: control, shift, mac_cmd: false, command: control }
}

/// The events one key press becomes: the key pressed and released, the text it typed,
/// and for Ctrl+C, Ctrl+X and Ctrl+V the clipboard events egui's text fields read.
pub fn key_events(code: u8, shift: bool, control: bool, typed: Option<char>, clipboard: &str, out: &mut Vec<Event>) {
    let m = modifiers(shift, control);
    if out.last().map_or(true, |e| !matches!(e, Event::ModifiersChanged(x) if *x == m)) { out.push(Event::ModifiersChanged(m)); }
    let k = key(code);
    if control {
        match k {
            Some(Key::C) => out.push(Event::Copy),
            Some(Key::X) => out.push(Event::Cut),
            Some(Key::V) => out.push(Event::Paste(clipboard.into())),
            _ => {}
        }
    }
    if let Some(k) = k {
        out.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
        out.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
    }
    if !control {
        if let Some(c) = typed.filter(|c| !c.is_control()) { out.push(Event::Text(c.into())); }
    }
}

/// What a pointer event in the window's pixels is to egui, at `ppp` pixels per point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerKind { Move, Down(u8), Up(u8), Wheel(i8), Leave }

pub fn pointer_events(x: i16, y: i16, kind: PointerKind, ppp: f32, m: Modifiers, out: &mut Vec<Event>) {
    let pos = Pos2::new(x as f32 / ppp, y as f32 / ppp);
    let button = |b: u8| match b { 1 => PointerButton::Middle, 2 => PointerButton::Secondary, _ => PointerButton::Primary };
    match kind {
        PointerKind::Move => out.push(Event::PointerMoved(pos)),
        PointerKind::Down(b) => {
            out.push(Event::PointerMoved(pos));
            out.push(Event::PointerButton { pos, button: button(b), pressed: true, modifiers: m });
        }
        PointerKind::Up(b) => out.push(Event::PointerButton { pos, button: button(b), pressed: false, modifiers: m }),
        // Positive steps move toward the document's start: the content moves down.
        PointerKind::Wheel(steps) => out.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, steps as f32),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: m,
        }),
        PointerKind::Leave => out.push(Event::PointerGone),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn letters_digits_and_extended_keys_map() {
        assert_eq!(key(0x1E), Some(Key::A));
        assert_eq!(key(0x2C), Some(Key::Z));
        assert_eq!(key(0x0B), Some(Key::Num0));
        assert_eq!(key(0x4B), Some(Key::ArrowLeft));
        assert_eq!(key(0x69), Some(Key::ArrowLeft));
        assert_eq!(key(0x57), Some(Key::F11));
        assert_eq!(key(0x7E), None);
    }
    #[test]
    fn a_press_is_press_release_and_text() {
        let mut out = Vec::new();
        key_events(0x1E, true, false, Some('A'), "", &mut out);
        assert!(matches!(out[1], Event::Key { key: Key::A, pressed: true, .. }));
        assert!(matches!(out[2], Event::Key { key: Key::A, pressed: false, .. }));
        assert_eq!(out[3], Event::Text("A".into()));
    }
    #[test]
    fn ctrl_v_pastes_the_clipboard_and_types_nothing() {
        let mut out = Vec::new();
        key_events(0x2F, false, true, Some('v'), "hello", &mut out);
        assert!(out.contains(&Event::Paste("hello".into())));
        assert!(!out.iter().any(|e| matches!(e, Event::Text(_))));
    }
    #[test]
    fn pointer_positions_are_points() {
        let mut out = Vec::new();
        pointer_events(200, 100, PointerKind::Down(0), 2.0, Modifiers::NONE, &mut out);
        assert_eq!(out[0], Event::PointerMoved(Pos2::new(100.0, 50.0)));
        assert!(matches!(out[1], Event::PointerButton { button: PointerButton::Primary, pressed: true, .. }));
    }
}
