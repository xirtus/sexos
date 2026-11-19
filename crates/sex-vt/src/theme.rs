// SPDX-License-Identifier: GPL-3.0-only
//! Colour themes: the default foreground and background, the cursor and selection, and
//! the sixteen named colours a program picks from. A screen keeps its own theme
//! ([`crate::Screen::set_theme`]); the 256-colour cube and the grey ramp stay fixed, as
//! in every xterm-compatible terminal.

/// A terminal colour scheme. Colours are 0x00RRGGBB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub fg: u32,
    pub bg: u32,
    pub cursor: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
}

impl Theme {
    /// The 256-colour palette with this theme's sixteen named colours.
    pub fn palette(&self, i: u8) -> u32 {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v as u32 };
                level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6)
            }
            _ => { let g = 8 + 10 * (i as u32 - 232); g << 16 | g << 8 | g }
        }
    }
    /// A built-in theme by name (case and `-`/`_`/space ignored).
    pub fn named(name: &str) -> Option<Theme> {
        let want: alloc::string::String = name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
        THEMES.iter().find(|t| t.name.chars().filter(|c| c.is_ascii_alphanumeric()).eq(want.chars())).copied()
    }
    /// Whether the background is light (for choosing overlay colours).
    pub fn is_light(&self) -> bool {
        let (r, g, b) = ((self.bg >> 16) & 0xFF, (self.bg >> 8) & 0xFF, self.bg & 0xFF);
        r * 299 + g * 587 + b * 114 > 128_000
    }
}

/// Parses `#rrggbb`, `rrggbb` or `0xrrggbb`.
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    let hex = s.strip_prefix('#').or_else(|| s.strip_prefix("0x")).unwrap_or(s);
    if hex.len() != 6 { return None; }
    u32::from_str_radix(hex, 16).ok()
}

pub const MOCHA: Theme = Theme {
    name: "mocha", fg: 0xCDD6F4, bg: 0x11111B, cursor: 0xCDD6F4, selection: 0x45475A,
    ansi: [0x1E1E2E, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xBAC2DE,
           0x585B70, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xA6ADC8],
};
pub const LATTE: Theme = Theme {
    name: "latte", fg: 0x4C4F69, bg: 0xEFF1F5, cursor: 0xDC8A78, selection: 0xACB0BE,
    ansi: [0x5C5F77, 0xD20F39, 0x40A02B, 0xDF8E1D, 0x1E66F5, 0xEA76CB, 0x179299, 0xACB0BE,
           0x6C6F85, 0xD20F39, 0x40A02B, 0xDF8E1D, 0x1E66F5, 0xEA76CB, 0x179299, 0xBCC0CC],
};
pub const GRUVBOX: Theme = Theme {
    name: "gruvbox", fg: 0xEBDBB2, bg: 0x282828, cursor: 0xEBDBB2, selection: 0x504945,
    ansi: [0x282828, 0xCC241D, 0x98971A, 0xD79921, 0x458588, 0xB16286, 0x689D6A, 0xA89984,
           0x928374, 0xFB4934, 0xB8BB26, 0xFABD2F, 0x83A598, 0xD3869B, 0x8EC07C, 0xEBDBB2],
};
pub const NORD: Theme = Theme {
    name: "nord", fg: 0xD8DEE9, bg: 0x2E3440, cursor: 0xD8DEE9, selection: 0x434C5E,
    ansi: [0x3B4252, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x88C0D0, 0xE5E9F0,
           0x4C566A, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x8FBCBB, 0xECEFF4],
};
pub const SOLARIZED_DARK: Theme = Theme {
    name: "solarized-dark", fg: 0x839496, bg: 0x002B36, cursor: 0x93A1A1, selection: 0x073642,
    ansi: [0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5,
           0x002B36, 0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3],
};
pub const SOLARIZED_LIGHT: Theme = Theme {
    name: "solarized-light", fg: 0x657B83, bg: 0xFDF6E3, cursor: 0x586E75, selection: 0xEEE8D5,
    ansi: SOLARIZED_DARK.ansi,
};
pub const TOKYO_NIGHT: Theme = Theme {
    name: "tokyo-night", fg: 0xC0CAF5, bg: 0x1A1B26, cursor: 0xC0CAF5, selection: 0x33467C,
    ansi: [0x15161E, 0xF7768E, 0x9ECE6A, 0xE0AF68, 0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xA9B1D6,
           0x414868, 0xF7768E, 0x9ECE6A, 0xE0AF68, 0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xC0CAF5],
};
pub const DRACULA: Theme = Theme {
    name: "dracula", fg: 0xF8F8F2, bg: 0x282A36, cursor: 0xF8F8F2, selection: 0x44475A,
    ansi: [0x21222C, 0xFF5555, 0x50FA7B, 0xF1FA8C, 0xBD93F9, 0xFF79C6, 0x8BE9FD, 0xF8F8F2,
           0x6272A4, 0xFF6E6E, 0x69FF94, 0xFFFFA5, 0xD6ACFF, 0xFF92DF, 0xA4FFFF, 0xFFFFFF],
};
/// Black on white with pure colours, for low vision and bright rooms.
pub const HIGH_CONTRAST: Theme = Theme {
    name: "high-contrast", fg: 0x000000, bg: 0xFFFFFF, cursor: 0x000000, selection: 0xFFD700,
    ansi: [0x000000, 0xC00000, 0x007000, 0x806000, 0x0000C0, 0x900090, 0x007070, 0x404040,
           0x404040, 0xFF0000, 0x008800, 0x997700, 0x0000FF, 0xB000B0, 0x008888, 0x000000],
};

pub const THEMES: [Theme; 9] = [MOCHA, LATTE, GRUVBOX, NORD, SOLARIZED_DARK, SOLARIZED_LIGHT, TOKYO_NIGHT, DRACULA, HIGH_CONTRAST];
