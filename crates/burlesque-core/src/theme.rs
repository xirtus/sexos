// SPDX-License-Identifier: GPL-3.0-only
//! Presentation themes: palettes, typography, and styling rules.

use alloc::string::String;
use sex_canvas::Color;

/// Color palette for a deck theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub primary_accent: Color,
    pub secondary_accent: Color,
    pub muted: Color,
    pub surface: Color,
    pub border: Color,
}

impl Palette {
    pub const fn light() -> Self {
        Self {
            background: Color::rgb(0xF8, 0xFA, 0xFC),
            foreground: Color::rgb(0x0F, 0x17, 0x2A),
            primary_accent: Color::rgb(0x7C, 0x3A, 0xED),
            secondary_accent: Color::rgb(0xEC, 0x48, 0x99),
            muted: Color::rgb(0x64, 0x74, 0x8B),
            surface: Color::rgb(0xFF, 0xFF, 0xFF),
            border: Color::rgb(0xE2, 0xE8, 0xF0),
        }
    }

    pub const fn dark() -> Self {
        Self {
            background: Color::rgb(0x0F, 0x17, 0x2A),
            foreground: Color::rgb(0xF8, 0xFA, 0xFC),
            primary_accent: Color::rgb(0x8B, 0x5C, 0xF6),
            secondary_accent: Color::rgb(0xF4, 0x3F, 0x5E),
            muted: Color::rgb(0x94, 0xA3, 0xB8),
            surface: Color::rgb(0x1E, 0x29, 0x3B),
            border: Color::rgb(0x33, 0x41, 0x55),
        }
    }

    pub const fn burlesque() -> Self {
        Self {
            background: Color::rgb(0x18, 0x08, 0x28),     // Deep cabaret plum
            foreground: Color::rgb(0xFD, 0xF4, 0xE3),     // Warm ivory
            primary_accent: Color::rgb(0xF4, 0x3F, 0x5E), // Crimson rose
            secondary_accent: Color::rgb(0xF5, 0x9E, 0x0B), // Champagne gold
            muted: Color::rgb(0xC0, 0x84, 0xFC),          // Lavender shimmer
            surface: Color::rgb(0x2A, 0x11, 0x45),        // Royal velvet card
            border: Color::rgb(0x58, 0x1C, 0x87),         // Dark violet border
        }
    }
}

/// Typography specification for slide text hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub struct Typography {
    pub title_font: &'static str,
    pub body_font: &'static str,
    pub code_font: &'static str,
    pub title_size: f32,
    pub subtitle_size: f32,
    pub heading_size: f32,
    pub body_size: f32,
    pub code_size: f32,
    pub caption_size: f32,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            title_font: "Geist Sans",
            body_font: "Geist Sans",
            code_font: "Geist Mono",
            title_size: 56.0,
            subtitle_size: 32.0,
            heading_size: 40.0,
            body_size: 26.0,
            code_size: 22.0,
            caption_size: 18.0,
        }
    }
}

/// Deck presentation theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub palette: Palette,
    pub typography: Typography,
}

impl Theme {
    pub fn default_light() -> Self {
        Self {
            name: String::from("Modern Light"),
            palette: Palette::light(),
            typography: Typography::default(),
        }
    }

    pub fn default_dark() -> Self {
        Self {
            name: String::from("Modern Dark"),
            palette: Palette::dark(),
            typography: Typography::default(),
        }
    }

    pub fn burlesque() -> Self {
        Self {
            name: String::from("Burlesque"),
            palette: Palette::burlesque(),
            typography: Typography::default(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::burlesque()
    }
}
