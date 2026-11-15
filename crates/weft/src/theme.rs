// SPDX-License-Identifier: GPL-3.0-only
//! Themes: light, dark or high contrast, an accent colour, a font scale and a density.
//!
//! A theme is plain data: every widget reads its colours and sizes from here, so a new
//! theme repaints the whole window at once ([`crate::Ui::set_theme`]). Settings arrive
//! as text, one `key = value` a line, the same lines Tatami writes:
//!
//! ```text
//! theme = dark          # light, dark, auto, high-contrast
//! accent = #e5487a
//! scale = 1.25          # text and controls together
//! density = compact     # compact, comfortable, spacious
//! language = de
//! ```

/// A straight-alpha colour, `0xAARRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const TRANSPARENT: Color = Color(0);
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self { Color(0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32) }
    pub const fn hex(rgb: u32) -> Self { Color(0xFF00_0000 | rgb) }
    pub const fn a(self) -> u8 { (self.0 >> 24) as u8 }
    pub const fn r(self) -> u8 { (self.0 >> 16) as u8 }
    pub const fn g(self) -> u8 { (self.0 >> 8) as u8 }
    pub const fn b(self) -> u8 { self.0 as u8 }
    pub const fn with_alpha(self, a: u8) -> Self { Color(self.0 & 0x00FF_FFFF | (a as u32) << 24) }
    /// `self` mixed toward `other` by `t` (0..1).
    pub fn mix(self, other: Color, t: f32) -> Color {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8;
        Color((m(self.a(), other.a()) as u32) << 24 | (m(self.r(), other.r()) as u32) << 16 | (m(self.g(), other.g()) as u32) << 8 | m(self.b(), other.b()) as u32)
    }
    /// Relative luminance (WCAG), 0..1.
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| { let c = c as f32 / 255.0; if c <= 0.03928 { c / 12.92 } else { libm::powf((c + 0.055) / 1.055, 2.4) } };
        0.2126 * lin(self.r()) + 0.7152 * lin(self.g()) + 0.0722 * lin(self.b())
    }
    /// WCAG contrast ratio between two colours (1..21).
    pub fn contrast(self, other: Color) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    /// `#rrggbb` or `#rgb`.
    pub fn parse(text: &str) -> Option<Color> {
        let h = text.trim().strip_prefix('#')?;
        let v = u32::from_str_radix(h, 16).ok()?;
        match h.len() {
            6 => Some(Color::hex(v)),
            3 => Some(Color::rgb(((v >> 8) & 0xF) as u8 * 17, ((v >> 4) & 0xF) as u8 * 17, (v & 0xF) as u8 * 17)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Light, Dark, HighContrast }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density { Compact, Comfortable, Spacious }

/// Every colour a widget uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The window background.
    pub window: Color,
    /// Raised surfaces: cards, menus, dialogs, the tab strip.
    pub surface: Color,
    /// Text fields, list backgrounds, sunken areas.
    pub field: Color,
    /// Buttons at rest.
    pub control: Color,
    pub control_hover: Color,
    pub control_pressed: Color,
    pub border: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_disabled: Color,
    pub accent: Color,
    /// Text on the accent colour.
    pub on_accent: Color,
    pub selection: Color,
    pub focus: Color,
    pub danger: Color,
    pub on_danger: Color,
    pub shadow: Color,
    pub scrim: Color,
}

/// The theme every widget reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub mode: Mode,
    pub palette: Palette,
    /// Device pixels per layout unit: 1 on ordinary screens, 2 on HiDPI, or any
    /// in-between factor the person chose. Text and controls grow together.
    pub scale: f32,
    pub density: Density,
    /// Body text size in layout units.
    pub font_size: f32,
    pub radius: f32,
    /// What the settings asked for; `Mode::Light` and `Mode::Dark` follow it with "auto".
    pub accent: Color,
}

impl Default for Theme {
    fn default() -> Self { Theme::new(Mode::Light, DEFAULT_ACCENT) }
}

/// SexOS pink.
pub const DEFAULT_ACCENT: Color = Color::hex(0xD6336C);

impl Theme {
    pub fn new(mode: Mode, accent: Color) -> Theme {
        Theme { mode, palette: palette(mode, accent), scale: 1.0, density: Density::Comfortable, font_size: 14.0, radius: 6.0, accent }
    }
    pub fn light() -> Theme { Theme::new(Mode::Light, DEFAULT_ACCENT) }
    pub fn dark() -> Theme { Theme::new(Mode::Dark, DEFAULT_ACCENT) }
    pub fn high_contrast() -> Theme { Theme::new(Mode::HighContrast, DEFAULT_ACCENT) }

    pub fn with_accent(mut self, accent: Color) -> Theme { self.accent = accent; self.palette = palette(self.mode, accent); self }
    pub fn with_scale(mut self, scale: f32) -> Theme { self.scale = scale.clamp(0.5, 4.0); self }
    pub fn with_density(mut self, density: Density) -> Theme { self.density = density; self }

    /// Height of buttons, fields and list rows, in layout units.
    pub fn control_height(&self) -> f32 {
        match self.density { Density::Compact => 26.0, Density::Comfortable => 32.0, Density::Spacious => 40.0 }
    }
    pub fn row_height(&self) -> f32 {
        match self.density { Density::Compact => 22.0, Density::Comfortable => 28.0, Density::Spacious => 34.0 }
    }
    /// The usual gap between controls.
    pub fn spacing(&self) -> f32 {
        match self.density { Density::Compact => 4.0, Density::Comfortable => 8.0, Density::Spacious => 12.0 }
    }
    pub fn padding(&self) -> f32 { self.spacing() + 4.0 }
    /// Focus rings and borders are wider in high contrast.
    pub fn border_width(&self) -> f32 { if self.mode == Mode::HighContrast { 2.0 } else { 1.0 } }

    /// The theme the settings text describes, on top of `self`. Unknown keys are
    /// ignored, so newer settings never break older programs. Returns the language, if
    /// the text names one.
    pub fn apply_settings(&mut self, text: &str) -> Option<alloc::string::String> {
        let mut language = None;
        let mut mode = self.mode;
        let mut accent = self.accent;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("");
            let Some((k, v)) = line.split_once('=') else {
                // `#` starts a comment, so colours come back here with their `#`.
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "theme" | "mode" => mode = match v { "dark" => Mode::Dark, "high-contrast" | "contrast" => Mode::HighContrast, _ => Mode::Light },
                "scale" | "font-scale" => if let Ok(s) = v.parse::<f32>() { self.scale = s.clamp(0.5, 4.0) },
                "density" => self.density = match v { "compact" => Density::Compact, "spacious" => Density::Spacious, _ => Density::Comfortable },
                "language" | "locale" => language = Some(v.into()),
                _ => {}
            }
        }
        // Colours carry `#`, which the comment rule above cut: read them separately.
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == "accent" {
                    if let Some(c) = Color::parse(v.split_whitespace().next().unwrap_or("")) { accent = c; }
                }
            }
        }
        self.mode = mode;
        self.accent = accent;
        self.palette = palette(mode, accent);
        language
    }
}

fn palette(mode: Mode, accent: Color) -> Palette {
    let white = Color::hex(0xFFFFFF);
    let black = Color::hex(0x000000);
    // Text on the accent: whichever of white or near-black reads better.
    let on = |c: Color| if c.contrast(white) >= c.contrast(Color::hex(0x111111)) { white } else { Color::hex(0x111111) };
    match mode {
        Mode::Light => Palette {
            window: Color::hex(0xF6F5F7),
            surface: Color::hex(0xFFFFFF),
            field: Color::hex(0xFFFFFF),
            control: Color::hex(0xEAE8EC),
            control_hover: Color::hex(0xDFDCE2),
            control_pressed: Color::hex(0xD2CED6),
            border: Color::hex(0xCFCBD3),
            text: Color::hex(0x1E1B22),
            text_dim: Color::hex(0x5F5966),
            text_disabled: Color::hex(0xA29CA8),
            accent,
            on_accent: on(accent),
            selection: accent.with_alpha(0x40),
            focus: accent,
            danger: Color::hex(0xC62828),
            on_danger: white,
            shadow: black.with_alpha(0x30),
            scrim: black.with_alpha(0x50),
        },
        Mode::Dark => {
            // A brighter accent reads better on dark grounds.
            let a = accent.mix(white, 0.12);
            Palette {
                window: Color::hex(0x1C1A20),
                surface: Color::hex(0x26232B),
                field: Color::hex(0x141217),
                control: Color::hex(0x35313B),
                control_hover: Color::hex(0x413C48),
                control_pressed: Color::hex(0x4C4654),
                border: Color::hex(0x45404C),
                text: Color::hex(0xF1EEF4),
                text_dim: Color::hex(0xB3ADB9),
                text_disabled: Color::hex(0x6E6875),
                accent: a,
                on_accent: on(a),
                selection: a.with_alpha(0x55),
                focus: a,
                danger: Color::hex(0xEF5350),
                on_danger: black,
                shadow: black.with_alpha(0x70),
                scrim: black.with_alpha(0x80),
            }
        }
        Mode::HighContrast => Palette {
            window: black,
            surface: black,
            field: black,
            control: black,
            control_hover: Color::hex(0x1A1A1A),
            control_pressed: Color::hex(0x333333),
            border: white,
            text: white,
            text_dim: white,
            text_disabled: Color::hex(0x9E9E9E),
            accent: Color::hex(0xFFE600),
            on_accent: black,
            selection: Color::hex(0xFFE600).with_alpha(0x80),
            focus: Color::hex(0x00FFFF),
            danger: Color::hex(0xFF6E6E),
            on_danger: black,
            shadow: black.with_alpha(0),
            scrim: black.with_alpha(0xC0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_text_sets_everything() {
        let mut t = Theme::default();
        let lang = t.apply_settings("theme = dark\naccent = #3584e4  # blue\nscale = 1.5\ndensity = compact\nlanguage = de\nunknown = 1\n");
        assert_eq!(t.mode, Mode::Dark);
        assert_eq!(t.accent, Color::hex(0x3584E4));
        assert_eq!(t.scale, 1.5);
        assert_eq!(t.density, Density::Compact);
        assert_eq!(lang.as_deref(), Some("de"));
    }
    #[test]
    fn body_text_meets_wcag_aa_in_every_mode() {
        for t in [Theme::light(), Theme::dark(), Theme::high_contrast()] {
            let p = t.palette;
            assert!(p.text.contrast(p.window) >= 7.0, "{:?}", t.mode);
            assert!(p.text.contrast(p.surface) >= 7.0, "{:?}", t.mode);
            assert!(p.text.contrast(p.control) >= 4.5, "{:?}", t.mode);
            assert!(p.text_dim.contrast(p.window) >= 4.5, "{:?}", t.mode);
            assert!(p.on_accent.contrast(p.accent) >= 4.5, "{:?}", t.mode);
        }
    }
}
