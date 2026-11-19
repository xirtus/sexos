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
            // A brighter accent reads better on dark grounds.
            let a = mix(accent, white, 0.12);
            Tokens {
                mode,
                window: solid(0x1C1A20), surface: solid(0x26232B), field: solid(0x141217),
                control: solid(0x35313B), control_hover: solid(0x413C48), control_pressed: solid(0x4C4654),
                border: solid(0x45404C),
                text: solid(0xF1EEF4), text_dim: solid(0xB3ADB9), text_disabled: solid(0x6E6875),
                accent: a, on_accent: on(a), selection: alpha(a, 0x55), focus: a,
                danger: solid(0xEF5350), on_danger: black,
                shadow: alpha(black, 0x70), scrim: alpha(black, 0x80),
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
}
