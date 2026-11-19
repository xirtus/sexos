// SPDX-License-Identifier: GPL-3.0-only
//! Dell (and Alienware) hotkey events over WMI GUID `9DBB5994-...`, notify value 0xD0.
//!
//! `_WED(0xD0)` returns a buffer of little-endian 16-bit words holding one or more records:
//! `[length, type, code, extra...]`, where `length` counts the words after itself. Type
//! 0x0000 is one key or event, 0x0010/0x0011 a sequence of keys/events, 0x0012 a
//! system event (privacy switches, keyboard light changes). Most keys also arrive as
//! keyboard scancodes or ACPI video notifies; only the keys that do not are acted on.
//! Codes match Linux's `dell-wmi` keymap.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    MicMute,
    Mute,
    BrightnessDown,
    BrightnessUp,
    /// The keyboard light changed (the firmware already did it).
    KeyboardLight,
    /// Radio switch (the firmware already did it).
    Radio,
    /// The display switch key (Fn+F8 on Dell, video output toggle).
    DisplaySwitch,
    /// Programmable keys: the Alienware Command Center key and its siblings.
    Program(u8),
    /// A code this table does not name.
    Other(u16),
}

impl Key {
    pub fn name(self) -> &'static str {
        match self {
            Key::MicMute => "mic_mute",
            Key::Mute => "mute",
            Key::BrightnessDown => "brightness_down",
            Key::BrightnessUp => "brightness_up",
            Key::KeyboardLight => "keyboard_light",
            Key::Radio => "radio",
            Key::DisplaySwitch => "display_switch",
            Key::Program(_) => "program",
            Key::Other(_) => "other",
        }
    }
    /// True when the desktop should act: the firmware or another path handles the rest.
    pub fn actionable(self) -> bool { matches!(self, Key::MicMute | Key::DisplaySwitch | Key::Program(_)) }
}

pub fn key(record_type: u16, code: u16) -> Key {
    match (record_type, code) {
        (_, 0x0150) => Key::MicMute,
        (_, 0x0109 | 0xE020) => Key::Mute,
        (_, 0xE005 | 0x0057) => Key::BrightnessDown,
        (_, 0xE006 | 0x0058) => Key::BrightnessUp,
        (_, 0xE00C | 0x0152) => Key::KeyboardLight,
        (_, 0xE008 | 0x0153) => Key::Radio,
        (_, 0xE00B | 0xE027) => Key::DisplaySwitch,
        (_, 0x0850) => Key::Program(1),
        (_, 0x0851) => Key::Program(2),
        (_, 0x0852) => Key::Program(3),
        (_, 0xE025) => Key::Program(4),
        (_, c) => Key::Other(c),
    }
}

/// The keys in one `_WED` buffer, at most `out.len()`. Malformed records end the walk.
pub fn decode(buffer: &[u8], out: &mut [Key]) -> usize {
    let words = buffer.len() / 2;
    let w = |i: usize| u16::from_le_bytes([buffer[2 * i], buffer[2 * i + 1]]);
    let (mut at, mut n) = (0usize, 0usize);
    while at < words && n < out.len() {
        let len = w(at) as usize;
        if len == 0 || at + 1 + len > words { break; }
        let kind = if len >= 1 { w(at + 1) } else { 0 };
        match kind {
            0x0000 | 0x0012 if len >= 2 => { out[n] = key(kind, w(at + 2)); n += 1; }
            0x0010 | 0x0011 => {
                for i in 2..=len {
                    if n == out.len() { break; }
                    out[n] = key(kind, w(at + i));
                    n += 1;
                }
            }
            _ => {}
        }
        at += 1 + len;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(words: &[u16]) -> Vec<u8> { words.iter().flat_map(|w| w.to_le_bytes()).collect() }

    #[test]
    fn single_and_sequence() {
        let mut out = [Key::Other(0); 8];
        assert_eq!(decode(&buf(&[2, 0x0000, 0x0150, 0, 0]), &mut out), 1);
        assert_eq!(out[0], Key::MicMute);
        assert!(out[0].actionable());
        let n = decode(&buf(&[3, 0x0010, 0xE005, 0x0850, 2, 0x0000, 0xE00C]), &mut out);
        assert_eq!(&out[..n], &[Key::BrightnessDown, Key::Program(1), Key::KeyboardLight]);
        assert!(!Key::BrightnessDown.actionable());
    }

    #[test]
    fn malformed() {
        let mut out = [Key::Other(0); 4];
        assert_eq!(decode(&buf(&[9, 0x0000, 0x0150]), &mut out), 0);
        assert_eq!(decode(&[0x02], &mut out), 0);
        assert_eq!(decode(&buf(&[0, 0x0000, 0x0150]), &mut out), 0);
        // A full output array stops cleanly.
        let mut one = [Key::Other(0); 1];
        assert_eq!(decode(&buf(&[3, 0x0010, 0xE005, 0xE006]), &mut one), 1);
        assert_eq!(key(0, 0x1234), Key::Other(0x1234));
    }
}
