// SPDX-License-Identifier: GPL-3.0-only
//! Theme tokens: the one source of every program's colours and sizes.
//!
//! Weft builds its `Theme` from these, and programs that draw their own pixels
//! (Files, Spreadeagle, Splicy...) read the same tokens through
//! `sex_graphics::theme` and `sex_app::settings::theme()`, so switching the theme in
//! Settings (`appearance.theme`, `appearance.accent`) changes them all together.
//!
//! Colours are `0xAARRGGBB`; solid ones have alpha `FF`. No heap.
#![no_std]

/// Light, dark or high contrast: `appearance.theme` 0, 1 and 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode { #[default] Light, Dark, HighContrast }

impl Mode {
    /// The mode for the `appearance.theme` setting's value.
    pub const fn from_setting(v: i32) -> Mode { match v { 1 => Mode::Dark, 2 => Mode::HighContrast, _ => Mode::Light } }
    pub const fn setting(self) -> i32 { match self { Mode::Light => 0, Mode::Dark => 1, Mode::HighContrast => 2 } }
    /// The names Settings shows and the settings file holds, in setting order.
    pub const NAMES: &'static [&'static str] = &["light", "dark", "high-contrast"];
}

/// SexOS pink, the default accent (`appearance.accent`).
pub const DEFAULT_ACCENT: u32 = 0xD6336C;

/// Four curated macOS Noir accent tints (`0..=3`), mapped to `accent_tint_idx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum NoirAccent {
    /// SexOS Crimson / Rose (`#F43F5E` / `#E11D48`) — Default Noir signature accent.
    #[default]
    Crimson = 0,
    /// Graphite / Silver (`#94A3B8` / `#CBD5E1`) — Pure monochrome pro Noir.
    Graphite = 1,
    /// Noir Lavender (`#A78BFA`) — Refined obsidian-lavender.
    Lavender = 2,
    /// Ice Cyan (`#38BDF8`) — Crisp studio accent.
    IceCyan = 3,
}

impl NoirAccent {
    /// All four curated Noir accent tints in `accent_tint_idx` order.
    pub const ALL: [NoirAccent; 4] = [
        NoirAccent::Crimson,
        NoirAccent::Graphite,
        NoirAccent::Lavender,
        NoirAccent::IceCyan,
    ];

    /// Maps an `accent_tint_idx` (wrapping cleanly across 0..=3) to [`NoirAccent`].
    pub const fn from_index(idx: u8) -> Self {
        match idx % 4 {
            0 => NoirAccent::Crimson,
            1 => NoirAccent::Graphite,
            2 => NoirAccent::Lavender,
            _ => NoirAccent::IceCyan,
        }
    }

    pub const fn index(self) -> u8 {
        self as u8
    }

    pub const fn name(self) -> &'static str {
        match self {
            NoirAccent::Crimson => "SexOS Crimson",
            NoirAccent::Graphite => "Graphite",
            NoirAccent::Lavender => "Noir Lavender",
            NoirAccent::IceCyan => "Ice Cyan",
        }
    }

    /// Primary vibrant highlight / rim / indicator accent (`0xAARRGGBB`).
    pub const fn primary(self) -> u32 {
        match self {
            NoirAccent::Crimson => solid(0xF4_3F5E),
            NoirAccent::Graphite => solid(0x94_A3B8),
            NoirAccent::Lavender => solid(0xA7_8BFA),
            NoirAccent::IceCyan => solid(0x38_BDF8),
        }
    }

    /// Solid button / pill fill accent (`0xAARRGGBB`) guaranteed WCAG-readable with [`Self::on_accent`].
    pub const fn fill(self) -> u32 {
        match self {
            NoirAccent::Crimson => solid(0xE1_1D48),
            NoirAccent::Graphite => solid(0x94_A3B8),
            NoirAccent::Lavender => solid(0xA7_8BFA),
            NoirAccent::IceCyan => solid(0x38_BDF8),
        }
    }

    /// Luminous specular / hover accent (`0xAARRGGBB`).
    pub const fn glow(self) -> u32 {
        match self {
            NoirAccent::Crimson => solid(0xFB_7185),
            NoirAccent::Graphite => solid(0xCB_D5E1),
            NoirAccent::Lavender => solid(0xC4_B5FD),
            NoirAccent::IceCyan => solid(0x7D_D3FC),
        }
    }

    /// High-contrast text ink drawn on top of [`Self::fill`] (`0xAARRGGBB`).
    pub const fn on_accent(self) -> u32 {
        match self {
            NoirAccent::Crimson => solid(0xFF_FFFF),
            NoirAccent::Graphite | NoirAccent::Lavender | NoirAccent::IceCyan => solid(0x0B_0D12),
        }
    }
}

/// Every colour and size a program's interface uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tokens {
    pub mode: Mode,
    /// The window background.
    pub window: u32,
    /// Raised surfaces: cards, menus, dialogs, tab strips, tiles.
    pub surface: u32,
    /// Text fields, list backgrounds, sunken areas.
    pub field: u32,
    /// Buttons at rest, and while hovered and pressed.
    pub control: u32,
    pub control_hover: u32,
    pub control_pressed: u32,
    pub border: u32,
    pub text: u32,
    pub text_dim: u32,
    pub text_disabled: u32,
    pub accent: u32,
    /// Text on the accent colour.
    pub on_accent: u32,
    /// Selected text and rows (translucent accent).
    pub selection: u32,
    pub focus: u32,
    pub danger: u32,
    pub on_danger: u32,
    pub shadow: u32,
    pub scrim: u32,
    /// Corner radius of controls and of cards/windows, in layout units.
    pub radius: u32,
    pub radius_large: u32,
    /// Body text size, in layout units.
    pub font_size: u32,
    /// The usual gap between controls, comfortable density.
    pub spacing: u32,
}

const fn solid(rgb: u32) -> u32 { 0xFF00_0000 | (rgb & 0xFF_FFFF) }
const fn alpha(c: u32, a: u32) -> u32 { (c & 0xFF_FFFF) | a << 24 }

/// `a` mixed toward `b` by `t` (0..1), alpha too.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let m = |s: u32| { let (x, y) = ((a >> s) & 0xFF, (b >> s) & 0xFF); ((x as f32 + (y as f32 - x as f32) * t + 0.5) as u32) << s };
    m(24) | m(16) | m(8) | m(0)
}

/// Relative luminance (WCAG), 0..1.
pub fn luminance(c: u32) -> f32 {
    let lin = |v: u32| { let v = (v & 0xFF) as f32 / 255.0; if v <= 0.03928 { v / 12.92 } else { libm::powf((v + 0.055) / 1.055, 2.4) } };
    0.2126 * lin(c >> 16) + 0.7152 * lin(c >> 8) + 0.0722 * lin(c)
}

/// WCAG contrast ratio between two colours (1..21).
pub fn contrast(a: u32, b: u32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The tokens for `mode` with accent `accent` (`0xRRGGBB`).
pub fn tokens(mode: Mode, accent: u32) -> Tokens {
    let accent = solid(accent);
    let (white, black) = (solid(0xFFFFFF), solid(0x000000));
    // Text on the accent: whichever of white or near-black reads better.
    let on = |c: u32| if contrast(c, white) >= contrast(c, solid(0x111111)) { white } else { solid(0x111111) };
    let sizes = |t: Tokens| Tokens { radius: 6, radius_large: 10, font_size: 14, spacing: 8, ..t };
    sizes(match mode {
        Mode::Light => Tokens {
            mode,
            window: solid(0xF6F5F7), surface: solid(0xFFFFFF), field: solid(0xFFFFFF),
            control: solid(0xEAE8EC), control_hover: solid(0xDFDCE2), control_pressed: solid(0xD2CED6),
            border: solid(0xCFCBD3),
            text: solid(0x1E1B22), text_dim: solid(0x5F5966), text_disabled: solid(0xA29CA8),
            accent, on_accent: on(accent), selection: alpha(accent, 0x40), focus: accent,
            danger: solid(0xC62828), on_danger: white,
            shadow: alpha(black, 0x30), scrim: alpha(black, 0x50),
            radius: 0, radius_large: 0, font_size: 0, spacing: 0,
        },
        Mode::Dark => {
            // macOS Noir Obsidian surfaces; a slightly brighter accent reads crisply on dark glass.
            let a = mix(accent, white, 0.12);
            Tokens {
                mode,
                window: solid(0x0B_0D12), surface: solid(0x13_161F), field: solid(0x08_0A0E),
                control: solid(0x1B_202C), control_hover: solid(0x23_2938), control_pressed: solid(0x2D_3546),
                border: solid(0x2A_3142),
                text: solid(0xF1_F5F9), text_dim: solid(0x94_A3B8), text_disabled: solid(0x64_748B),
                accent: a, on_accent: on(a), selection: alpha(a, 0x55), focus: a,
                danger: solid(0xFB_7185), on_danger: solid(0x0B_0D12),
                shadow: alpha(black, 0x80), scrim: alpha(black, 0x90),
                radius: 0, radius_large: 0, font_size: 0, spacing: 0,
            }
        }
        Mode::HighContrast => Tokens {
            mode,
            window: black, surface: black, field: black,
            control: black, control_hover: solid(0x1A1A1A), control_pressed: solid(0x333333),
            border: white,
            text: white, text_dim: white, text_disabled: solid(0x9E9E9E),
            accent: solid(0xFFE600), on_accent: black, selection: alpha(0xFFE600, 0x80), focus: solid(0x00FFFF),
            danger: solid(0xFF6E6E), on_danger: black,
            shadow: alpha(black, 0), scrim: alpha(black, 0xC0),
            radius: 0, radius_large: 0, font_size: 0, spacing: 0,
        },
    })
}

/// The tokens the two appearance settings describe.
pub fn from_settings(theme: i32, accent: i32) -> Tokens { tokens(Mode::from_setting(theme), accent as u32 & 0xFF_FFFF) }

/// macOS Noir Obsidian Glass palette tokens (`0xAARRGGBB`).
/// Deep layered obsidian materials (`#0B0D12` base, `#13161F` elevated, `#1B202C`
/// hover/inputs, `#2A3142` 1px top specular rim highlights) paired with the four
/// selectable [`NoirAccent`] tints.
pub mod noir {
    use super::solid;

    /// Core obsidian dark materials.
    pub const DARK_BASE: u32 = solid(0x0B_0D12);
    pub const DARK_PANEL: u32 = solid(0x10_131B);
    pub const DARK_SURFACE: u32 = solid(0x13_161F);
    pub const DARK_ELEVATED: u32 = solid(0x1B_202C);
    pub const DARK_HOVER: u32 = solid(0x23_2938);
    pub const DARK_WELL: u32 = solid(0x08_0A0E);
    pub const DARK_RIM: u32 = solid(0x2A_3142);
    pub const DARK_HAIRLINE: u32 = solid(0x22_2836);
    pub const DARK_SPECULAR: u32 = solid(0xE2_E8F0);
    pub const DARK_TEXT: u32 = solid(0xF1_F5F9);
    pub const DARK_SUBTEXT: u32 = solid(0x94_A3B8);
    pub const DARK_DISABLED: u32 = solid(0x64_748B);
    pub const DARK_DANGER: u32 = solid(0xFB_7185);

    /// SilkBar obsidian glass tokens (dark & light).
    pub const DARK_BAR_TOP: u32 = DARK_BASE;
    pub const DARK_BAR_BOTTOM: u32 = DARK_SURFACE;
    pub const DARK_PILL_FILL: u32 = DARK_ELEVATED;
    pub const DARK_PILL_RIM: u32 = DARK_RIM;

    pub const LIGHT_BAR_TOP: u32 = solid(0xF1_E8FF);
    pub const LIGHT_BAR_BOTTOM: u32 = solid(0xE4_D6FC);
    pub const LIGHT_SPECULAR: u32 = solid(0xFD_FAFF);
    pub const LIGHT_HAIRLINE: u32 = solid(0xA7_8BFA);
    pub const LIGHT_PILL_FILL: u32 = solid(0xFA_F6FF);
    pub const LIGHT_PILL_RIM: u32 = solid(0xB8_9CF8);
    pub const LIGHT_TEXT: u32 = solid(0x1E_162E);

    /// Default signature Noir accent (SexOS Crimson / Rose).
    pub const NOIR_ACCENT: u32 = solid(0xF4_3F5E);
    pub const NOIR_ACCENT_FILL: u32 = solid(0xE1_1D48);
    pub const NOIR_ACCENT_GLOW: u32 = solid(0xFB_7185);

    /// Frosted obsidian dropdowns & popovers.
    pub const DARK_MENU_BG: u32 = DARK_SURFACE;
    pub const DARK_MENU_RIM: u32 = DARK_RIM;
    pub const DARK_MENU_HOVER: u32 = DARK_ELEVATED;
    pub const DARK_MENU_DIVIDER: u32 = DARK_HAIRLINE;
    pub const DARK_MENU_SUBTEXT: u32 = DARK_SUBTEXT;
    pub const DARK_MENU_DANGER: u32 = DARK_DANGER;

    pub const LIGHT_MENU_BG: u32 = solid(0xF6_F6FA);
    pub const LIGHT_MENU_RIM: u32 = solid(0xC4_C9D6);
    pub const LIGHT_MENU_HOVER: u32 = solid(0xE8_EBF2);
    pub const LIGHT_MENU_DIVIDER: u32 = solid(0xDD_E1EC);
    pub const LIGHT_MENU_SUBTEXT: u32 = solid(0x46_4E61);
    pub const LIGHT_MENU_DANGER: u32 = solid(0xC2_185B);

    /// Frosted Spotlight Launcher tokens (dark & light).
    pub const DARK_SPOTLIGHT_INPUT: u32 = DARK_BASE;
    pub const DARK_SPOTLIGHT_TILE: u32 = DARK_SURFACE;
    pub const DARK_SPOTLIGHT_SELECTED: u32 = DARK_HOVER;
    pub const DARK_SPOTLIGHT_PILL: u32 = DARK_ELEVATED;
    pub const LIGHT_SPOTLIGHT_INPUT: u32 = solid(0xFF_FFFF);
    pub const LIGHT_SPOTLIGHT_TILE: u32 = solid(0xEF_F1F6);
    pub const LIGHT_SPOTLIGHT_SELECTED: u32 = solid(0xE2_E6F0);
    pub const LIGHT_SPOTLIGHT_PILL: u32 = solid(0xEB_EEF5);
    pub const PILL_ACTIVE_CRIMSON: u32 = NOIR_ACCENT_FILL;
    pub const ON_PILL_ACTIVE: u32 = solid(0xFF_FFFF);
}

/// Legacy Candy & Lavender namespace updated around macOS Noir Obsidian dark
/// surfaces so existing callers automatically inherit the unified Noir look.
pub mod candy_lavender {
    use super::noir;

    pub const DARK_BAR_TOP: u32 = noir::DARK_BAR_TOP;
    pub const DARK_BAR_BOTTOM: u32 = noir::DARK_BAR_BOTTOM;
    pub const DARK_SPECULAR: u32 = noir::DARK_SPECULAR;
    pub const DARK_HAIRLINE: u32 = noir::DARK_HAIRLINE;
    pub const DARK_PILL_FILL: u32 = noir::DARK_PILL_FILL;
    pub const DARK_PILL_RIM: u32 = noir::DARK_PILL_RIM;
    pub const DARK_TEXT: u32 = noir::DARK_TEXT;

    pub const LIGHT_BAR_TOP: u32 = noir::LIGHT_BAR_TOP;
    pub const LIGHT_BAR_BOTTOM: u32 = noir::LIGHT_BAR_BOTTOM;
    pub const LIGHT_SPECULAR: u32 = noir::LIGHT_SPECULAR;
    pub const LIGHT_HAIRLINE: u32 = noir::LIGHT_HAIRLINE;
    pub const LIGHT_PILL_FILL: u32 = noir::LIGHT_PILL_FILL;
    pub const LIGHT_PILL_RIM: u32 = noir::LIGHT_PILL_RIM;
    pub const LIGHT_TEXT: u32 = noir::LIGHT_TEXT;

    pub const ORCHID_ACCENT: u32 = noir::NOIR_ACCENT;
    pub const ORCHID_GLOW: u32 = noir::NOIR_ACCENT_GLOW;

    pub const DARK_MENU_BG: u32 = noir::DARK_MENU_BG;
    pub const DARK_MENU_RIM: u32 = noir::DARK_MENU_RIM;
    pub const DARK_MENU_HOVER: u32 = noir::DARK_MENU_HOVER;
    pub const DARK_MENU_DIVIDER: u32 = noir::DARK_MENU_DIVIDER;
    pub const DARK_MENU_SUBTEXT: u32 = noir::DARK_MENU_SUBTEXT;
    pub const DARK_MENU_DANGER: u32 = noir::DARK_MENU_DANGER;

    pub const LIGHT_MENU_BG: u32 = noir::LIGHT_MENU_BG;
    pub const LIGHT_MENU_RIM: u32 = noir::LIGHT_MENU_RIM;
    pub const LIGHT_MENU_HOVER: u32 = noir::LIGHT_MENU_HOVER;
    pub const LIGHT_MENU_DIVIDER: u32 = noir::LIGHT_MENU_DIVIDER;
    pub const LIGHT_MENU_SUBTEXT: u32 = noir::LIGHT_MENU_SUBTEXT;
    pub const LIGHT_MENU_DANGER: u32 = noir::LIGHT_MENU_DANGER;

    pub const DARK_SPOTLIGHT_INPUT: u32 = noir::DARK_SPOTLIGHT_INPUT;
    pub const DARK_SPOTLIGHT_TILE: u32 = noir::DARK_SPOTLIGHT_TILE;
    pub const DARK_SPOTLIGHT_SELECTED: u32 = noir::DARK_SPOTLIGHT_SELECTED;
    pub const DARK_SPOTLIGHT_PILL: u32 = noir::DARK_SPOTLIGHT_PILL;
    pub const LIGHT_SPOTLIGHT_INPUT: u32 = noir::LIGHT_SPOTLIGHT_INPUT;
    pub const LIGHT_SPOTLIGHT_TILE: u32 = noir::LIGHT_SPOTLIGHT_TILE;
    pub const LIGHT_SPOTLIGHT_SELECTED: u32 = noir::LIGHT_SPOTLIGHT_SELECTED;
    pub const LIGHT_SPOTLIGHT_PILL: u32 = noir::LIGHT_SPOTLIGHT_PILL;
    pub const ORCHID_PILL_ACTIVE: u32 = noir::PILL_ACTIVE_CRIMSON;
    pub const ON_ORCHID_ACTIVE: u32 = noir::ON_PILL_ACTIVE;
}

pub mod candy_ui;
pub mod noir_ui;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_meets_wcag_in_every_mode() {
        for m in [Mode::Light, Mode::Dark, Mode::HighContrast] {
            let t = tokens(m, DEFAULT_ACCENT);
            assert!(contrast(t.text, t.window) >= 7.0, "{m:?}");
            assert!(contrast(t.text_dim, t.window) >= 4.5, "{m:?}");
            assert!(contrast(t.on_accent, t.accent) >= 4.5, "{m:?}");
            assert_eq!(Mode::from_setting(m.setting()), m);
        }
        assert_eq!(from_settings(1, 0x3584E4).mode, Mode::Dark);
        assert_eq!(from_settings(7, 0).mode, Mode::Light);
    }

    #[test]
    fn candy_lavender_silkbar_meets_wcag_aa() {
        use candy_lavender::*;
        // Dark candy-lavender bar and frosted pill surfaces meet WCAG AAA (>= 7.0).
        assert!(contrast(DARK_TEXT, DARK_BAR_TOP) >= 7.0);
        assert!(contrast(DARK_TEXT, DARK_BAR_BOTTOM) >= 7.0);
        assert!(contrast(DARK_TEXT, DARK_PILL_FILL) >= 7.0);
        // Light candy-lavender bar and frosted pill surfaces meet WCAG AAA (>= 7.0).
        assert!(contrast(LIGHT_TEXT, LIGHT_BAR_TOP) >= 7.0);
        assert!(contrast(LIGHT_TEXT, LIGHT_BAR_BOTTOM) >= 7.0);
        assert!(contrast(LIGHT_TEXT, LIGHT_PILL_FILL) >= 7.0);
        // Frosted candy-lavender menu/popover surfaces meet WCAG AA/AAA.
        assert!(contrast(DARK_TEXT, DARK_MENU_BG) >= 7.0);
        assert!(contrast(DARK_TEXT, DARK_MENU_HOVER) >= 7.0);
        assert!(contrast(DARK_MENU_SUBTEXT, DARK_MENU_BG) >= 7.0);
        assert!(contrast(DARK_MENU_DANGER, DARK_MENU_BG) >= 4.5);
        assert!(contrast(LIGHT_TEXT, LIGHT_MENU_BG) >= 7.0);
        assert!(contrast(LIGHT_TEXT, LIGHT_MENU_HOVER) >= 7.0);
        assert!(contrast(LIGHT_MENU_SUBTEXT, LIGHT_MENU_BG) >= 7.0);
        assert!(contrast(LIGHT_MENU_DANGER, LIGHT_MENU_BG) >= 4.5);
        // Spotlight search input, tiles, and active category pill meet WCAG AA/AAA.
        assert!(contrast(DARK_TEXT, DARK_SPOTLIGHT_INPUT) >= 7.0);
        assert!(contrast(DARK_TEXT, DARK_SPOTLIGHT_TILE) >= 7.0);
        assert!(contrast(DARK_TEXT, DARK_SPOTLIGHT_SELECTED) >= 7.0);
        assert!(contrast(LIGHT_TEXT, LIGHT_SPOTLIGHT_INPUT) >= 7.0);
        assert!(contrast(LIGHT_TEXT, LIGHT_SPOTLIGHT_TILE) >= 7.0);
        assert!(contrast(LIGHT_TEXT, LIGHT_SPOTLIGHT_SELECTED) >= 7.0);
        assert!(contrast(ON_ORCHID_ACTIVE, ORCHID_PILL_ACTIVE) >= 4.5);
    }

    #[test]
    fn noir_tokens_and_accents_meet_wcag() {
        use noir::*;
        for tint in NoirAccent::ALL {
            assert!(contrast(tint.on_accent(), tint.fill()) >= 4.5, "{tint:?}");
            assert!(contrast(tint.primary(), DARK_BASE) >= 4.5, "{tint:?} primary on base");
            assert!(contrast(tint.primary(), DARK_SURFACE) >= 4.5, "{tint:?} primary on surface");
        }
        assert_eq!(NoirAccent::from_index(0), NoirAccent::Crimson);
        assert_eq!(NoirAccent::from_index(1), NoirAccent::Graphite);
        assert_eq!(NoirAccent::from_index(2), NoirAccent::Lavender);
        assert_eq!(NoirAccent::from_index(3), NoirAccent::IceCyan);
        assert_eq!(NoirAccent::from_index(4), NoirAccent::Crimson);
        assert!(contrast(DARK_TEXT, DARK_BASE) >= 7.0);
        assert!(contrast(DARK_SUBTEXT, DARK_SURFACE) >= 7.0);
    }
}
