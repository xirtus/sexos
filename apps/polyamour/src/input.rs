// SPDX-License-Identifier: GPL-3.0-only
//! Input as Polyamour sees it, independent of where it came from (Shell on SexOS,
//! scripted events in host tests).

/// A key, named the way Blender's keymap names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter (lowercase) or digit on the main keyboard, or punctuation.
    Char(char),
    /// F1..F12.
    F(u8),
    /// Numpad 0..9.
    Num(u8),
    NumDot,
    NumPlus,
    NumMinus,
    NumStar,
    NumSlash,
    NumEnter,
    Tab,
    Esc,
    Enter,
    Space,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
    pub const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
    pub const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
    pub const ALT: Mods = Mods { shift: false, ctrl: false, alt: true };
    pub const CTRL_SHIFT: Mods = Mods { shift: true, ctrl: true, alt: false };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Key { key: Key, mods: Mods, text: Option<char> },
    Move { x: i32, y: i32 },
    Down { x: i32, y: i32, button: Button, mods: Mods },
    Up { x: i32, y: i32, button: Button },
    /// Positive scrolls up / zooms in, like a wheel turned away from you.
    Wheel { x: i32, y: i32, steps: i32, mods: Mods },
    Resize { width: usize, height: usize },
    /// Nothing happened; redraw (timers, progressive render).
    Tick,
}

impl Input {
    pub fn key(key: Key) -> Input {
        Input::Key { key, mods: Mods::NONE, text: None }
    }
    pub fn key_mods(key: Key, mods: Mods) -> Input {
        Input::Key { key, mods, text: None }
    }
    pub fn pos(&self) -> Option<(i32, i32)> {
        match *self {
            Input::Move { x, y } | Input::Down { x, y, .. } | Input::Up { x, y, .. } | Input::Wheel { x, y, .. } => Some((x, y)),
            _ => None,
        }
    }
}
