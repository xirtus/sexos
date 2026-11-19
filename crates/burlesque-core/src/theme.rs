// SPDX-License-Identifier: GPL-3.0-only
//! Presentation themes: palettes, typography, and styling rules.
//!
//! Provides 6 rich, distinct Keynote-class themes:
//! - Burlesque Velvet: Deep cabaret plum, warm ivory, crimson rose, champagne gold
//! - Editorial Light: Warm newsprint alabaster, obsidian ink, terracotta vermilion, emerald
//! - Midnight Slate: Midnight navy slate, ice white, electric sky cyan, indigo glow
//! - Champagne Ivory: Luminous ivory silk, warm charcoal, burnished amber gold, bronze
//! - Cyber Prism: Void black-indigo, luminescent aqua, hot neon magenta, cyan laser
//! - Swiss Modern: International stark white, dense typographic black, Swiss signal red

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
        Self::editorial_light()
    }

    pub const fn dark() -> Self {
        Self::midnight_slate()
    }

    pub const fn burlesque() -> Self {
        Self::burlesque_velvet()
    }

    /// Cabaret deep plum, warm ivory, crimson rose, champagne gold.
    pub const fn burlesque_velvet() -> Self {
        Self {
            background: Color::rgb(0x18, 0x08, 0x28),       // Deep cabaret plum
            foreground: Color::rgb(0xFD, 0xF4, 0xE3),       // Warm ivory
            primary_accent: Color::rgb(0xF4, 0x3F, 0x5E),   // Crimson rose
            secondary_accent: Color::rgb(0xF5, 0x9E, 0x0B), // Champagne gold
            muted: Color::rgb(0xC0, 0x84, 0xFC),            // Lavender shimmer
            surface: Color::rgb(0x2A, 0x11, 0x45),          // Royal velvet card
            border: Color::rgb(0x58, 0x1C, 0x87),           // Dark violet border
        }
    }

    /// Warm newsprint alabaster, obsidian ink, terracotta vermilion, emerald.
    pub const fn editorial_light() -> Self {
        Self {
            background: Color::rgb(0xFA, 0xF7, 0xF2),       // Warm newsprint alabaster
            foreground: Color::rgb(0x1C, 0x19, 0x17),       // Obsidian ink
            primary_accent: Color::rgb(0xC2, 0x41, 0x0C),   // Terracotta vermilion
            secondary_accent: Color::rgb(0x04, 0x78, 0x57), // Deep emerald
            muted: Color::rgb(0x78, 0x71, 0x6C),            // Warm stone
            surface: Color::rgb(0xFF, 0xFF, 0xFF),          // Pure white card
            border: Color::rgb(0xE7, 0xE5, 0xE4),           // Editorial hairline border
        }
    }

    /// Midnight navy slate, ice white, electric sky cyan, indigo glow.
    pub const fn midnight_slate() -> Self {
        Self {
            background: Color::rgb(0x0B, 0x11, 0x20),       // Midnight navy slate
            foreground: Color::rgb(0xF8, 0xFA, 0xFC),       // Ice white
            primary_accent: Color::rgb(0x38, 0xBD, 0xF8),   // Electric sky cyan
            secondary_accent: Color::rgb(0x81, 0x8C, 0xF8), // Indigo glow
            muted: Color::rgb(0x94, 0xA3, 0xB8),            // Slate mist
            surface: Color::rgb(0x1E, 0x29, 0x3B),          // Slate card surface
            border: Color::rgb(0x33, 0x41, 0x55),           // Steel border
        }
    }

    /// Luminous ivory silk, warm charcoal, burnished amber gold, bronze.
    pub const fn champagne_ivory() -> Self {
        Self {
            background: Color::rgb(0xFB, 0xF8, 0xF1),       // Luminous ivory silk
            foreground: Color::rgb(0x29, 0x25, 0x24),       // Warm charcoal
            primary_accent: Color::rgb(0xD9, 0x77, 0x06),   // Burnished amber gold
            secondary_accent: Color::rgb(0xB4, 0x53, 0x09), // Warm bronze
            muted: Color::rgb(0xA8, 0xA2, 0x9E),            // Cashmere grey
            surface: Color::rgb(0xFF, 0xFF, 0xFF),          // Pure ivory card
            border: Color::rgb(0xE5, 0xD5, 0xB8),           // Champagne gold border
        }
    }

    /// Void black-indigo, luminescent aqua, hot neon magenta, cyan laser.
    pub const fn cyber_prism() -> Self {
        Self {
            background: Color::rgb(0x09, 0x07, 0x1A),       // Void black-indigo
            foreground: Color::rgb(0xF0, 0xFD, 0xFA),       // Luminescent aqua
            primary_accent: Color::rgb(0xEC, 0x48, 0x99),   // Hot neon magenta
            secondary_accent: Color::rgb(0x06, 0xB6, 0xD4), // Cyan laser
            muted: Color::rgb(0xA8, 0x55, 0xF7),            // Neon violet shimmer
            surface: Color::rgb(0x13, 0x0F, 0x2E),          // Cyber matrix card
            border: Color::rgb(0x6D, 0x28, 0xD9),           // Prism neon border
        }
    }

    /// International stark white, dense typographic black, Swiss signal red.
    pub const fn swiss_modern() -> Self {
        Self {
            background: Color::rgb(0xFA, 0xFA, 0xFA),       // Stark white
            foreground: Color::rgb(0x11, 0x18, 0x27),       // Dense typographic black
            primary_accent: Color::rgb(0xE1, 0x1D, 0x48),   // Swiss signal red
            secondary_accent: Color::rgb(0x25, 0x63, 0xEB), // Bauhaus cobalt blue
            muted: Color::rgb(0x6B, 0x72, 0x80),            // Neutral grey
            surface: Color::rgb(0xFF, 0xFF, 0xFF),          // Clean white surface
            border: Color::rgb(0x11, 0x18, 0x27),           // Architectural solid rule
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
    pub show_master_chrome: bool,
}

impl Theme {
    pub fn burlesque_velvet() -> Self {
        Self {
            name: String::from("Burlesque Velvet"),
            palette: Palette::burlesque_velvet(),
            typography: Typography {
                title_size: 58.0,
                subtitle_size: 32.0,
                heading_size: 42.0,
                body_size: 26.0,
                ..Typography::default()
            },
            show_master_chrome: true,
        }
    }

    pub fn editorial_light() -> Self {
        Self {
            name: String::from("Editorial Light"),
            palette: Palette::editorial_light(),
            typography: Typography {
                title_size: 54.0,
                subtitle_size: 30.0,
                heading_size: 38.0,
                body_size: 25.0,
                ..Typography::default()
            },
            show_master_chrome: true,
        }
    }

    pub fn midnight_slate() -> Self {
        Self {
            name: String::from("Midnight Slate"),
            palette: Palette::midnight_slate(),
            typography: Typography {
                title_size: 56.0,
                subtitle_size: 32.0,
                heading_size: 40.0,
                body_size: 26.0,
                ..Typography::default()
            },
            show_master_chrome: true,
        }
    }

    pub fn champagne_ivory() -> Self {
        Self {
            name: String::from("Champagne Ivory"),
            palette: Palette::champagne_ivory(),
            typography: Typography {
                title_size: 56.0,
                subtitle_size: 30.0,
                heading_size: 40.0,
                body_size: 26.0,
                ..Typography::default()
            },
            show_master_chrome: true,
        }
    }

    pub fn cyber_prism() -> Self {
        Self {
            name: String::from("Cyber Prism"),
            palette: Palette::cyber_prism(),
            typography: Typography {
                title_size: 60.0,
                subtitle_size: 32.0,
                heading_size: 44.0,
                body_size: 26.0,
                ..Typography::default()
            },
            show_master_chrome: true,
        }
    }

    pub fn swiss_modern() -> Self {
        Self {
            name: String::from("Swiss Modern"),
            palette: Palette::swiss_modern(),
            typography: Typography {
                title_size: 64.0,
                subtitle_size: 34.0,
                heading_size: 46.0,
                body_size: 28.0,
                ..Typography::default()
            },
            show_master_chrome: true,
        }
    }

    pub fn burlesque() -> Self {
        Self::burlesque_velvet()
    }

    pub fn default_light() -> Self {
        Self::editorial_light()
    }

    pub fn default_dark() -> Self {
        Self::midnight_slate()
    }

    /// Return all 6 distinct Keynote-class themes.
    pub fn all_themes() -> [Theme; 6] {
        [
            Self::burlesque_velvet(),
            Self::editorial_light(),
            Self::midnight_slate(),
            Self::champagne_ivory(),
            Self::cyber_prism(),
            Self::swiss_modern(),
        ]
    }

    /// Find theme by exact or case-insensitive name match.
    pub fn by_name(name: &str) -> Option<Theme> {
        for t in Self::all_themes() {
            if t.name.eq_ignore_ascii_case(name) {
                return Some(t);
            }
        }
        None
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::burlesque_velvet()
    }
}
