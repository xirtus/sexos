//! Host tests for the desktop theme (slice S6a): the exact
//! `servers/sexdisplay/src/desk_theme.rs` Display compiles, over the exact
//! `crates/sex-theme` tokens every program draws with.
//!
//! The crate root stands in for the crates the module names: `sex_theme`
//! (its source, with `libm::powf` from std), `sex_pdx` (the real
//! `desktop_look` and `desktop_style`, and a quiet `serial_record!`), and
//! Display's `tray_icons` inks.
extern crate self as sex_theme;
extern crate self as sex_pdx;
extern crate self as libm;

#[path = "../crates/sex-theme/src/lib.rs"]
mod theme_source;
pub use theme_source::*;
pub fn powf(x: f32, y: f32) -> f32 { x.powf(y) }

#[path = "../crates/sex-pdx/src/desktop_look.rs"]
pub mod desktop_look;
#[path = "../crates/sex-pdx/src/app_catalog.rs"]
pub mod app_catalog;
#[path = "../crates/sex-pdx/src/desktop_style.rs"]
pub mod desktop_style;
#[macro_export]
macro_rules! serial_record { ($($t:tt)*) => {{ let _ = format_args!($($t)*); }}; }

/// Display's tray icon inks (`servers/sexdisplay/src/tray_icons.rs`).
mod tray_icons {
    pub const FG: u32 = 0x00F1EEF4;
    pub const DIM: u32 = 0x006E6875;
    pub const WARN: u32 = 0x00F38BA8;
    pub const GOOD: u32 = 0x00A6E3A1;
}

#[path = "../servers/sexdisplay/src/desk_theme.rs"]
mod desk_theme;
use desk_theme::{desk, tray_ink, Desk};
use silkbar_model::DEFAULT_THEME;

const MODES: [Mode; 3] = [Mode::Light, Mode::Dark, Mode::HighContrast];
const ACCENTS: [u32; 4] = [DEFAULT_ACCENT, 0x3584E4, 0x2EC27E, 0xF6D32D];

fn colours(d: &Desk) -> Vec<(&'static str, u32)> {
    let b = &d.bar;
    vec![("panel_fill", b.panel_fill), ("panel_glow", b.panel_glow), ("text", b.text), ("muted", b.muted),
        ("active", b.active), ("urgent", b.urgent), ("chip_fill", b.chip_fill), ("chip_border", b.chip_border),
        ("focus", d.focus), ("rim", d.rim), ("title_bar", d.title_bar), ("active_tab", d.active_tab),
        ("inactive_tab", d.inactive_tab), ("band", d.band), ("menu", d.menu), ("menu_rim", d.menu_rim),
        ("menu_picked", d.menu_picked), ("menu_ink", d.menu_ink), ("menu_title", d.menu_title),
        ("menu_soft", d.menu_soft), ("menu_dim", d.menu_dim), ("menu_field", d.menu_field),
        ("menu_button", d.menu_button), ("menu_send", d.menu_send), ("menu_on_send", d.menu_on_send),
        ("menu_close", d.menu_close), ("card", d.card), ("card_rim", d.card_rim), ("card_ink", d.card_ink),
        ("card_soft", d.card_soft), ("good", d.good), ("warn", d.warn)]
}

#[test]
fn every_colour_is_xrgb_as_display_writes_it() {
    for m in MODES { for a in ACCENTS {
        for (name, c) in colours(&desk(m, a)) { assert_eq!(c >> 24, 0, "{m:?} {name} {c:08x}"); }
    } }
}

#[test]
fn dark_keeps_the_desktop_it_had() {
    let d = desk(Mode::Dark, DEFAULT_ACCENT);
    assert_eq!((d.bar.panel_fill, d.bar.panel_glow, d.bar.text, d.bar.muted, d.bar.urgent),
        (DEFAULT_THEME.panel_fill, DEFAULT_THEME.panel_glow, DEFAULT_THEME.text, DEFAULT_THEME.muted, DEFAULT_THEME.urgent));
    assert_eq!((d.bar.bg_top, d.bar.bg_bottom), (DEFAULT_THEME.bg_top, DEFAULT_THEME.bg_bottom), "the wallpaper stays the wallpaper's");
    // The old window menu and card constants.
    assert_eq!((d.menu, d.menu_rim, d.menu_picked, d.menu_ink, d.menu_close), (0x181825, 0x45475A, 0x313244, 0xCDD6F4, 0xF38BA8));
    assert_eq!((d.card, d.card_rim, d.card_ink, d.card_soft), (0x14121A, 0x463E4C, 0xF4EEF2, 0xC2B6C0));
    assert_eq!((d.title_bar, d.rim, d.band), (0x1E1E2E, 0xB4BEFE, desktop_style::NATIVE_CANVAS));
}

#[test]
fn light_has_light_panels_and_dark_ink() {
    for a in ACCENTS {
        let d = desk(Mode::Light, a);
        for (name, c) in [("bar", d.bar.panel_fill), ("menu", d.menu), ("card", d.card), ("title bar", d.title_bar)] {
            assert!(luminance(c) > 0.7, "{name} {c:06x} is light");
        }
        assert!(luminance(d.bar.text) < 0.05 && luminance(d.menu_ink) < 0.05 && luminance(d.card_ink) < 0.05);
        // Focused tab and title bar read apart from the light panels around them.
        assert_ne!(d.active_tab, d.title_bar);
        assert!(contrast(d.rim, d.title_bar) >= 1.8, "the title bar's divider and window outline show");
    }
}

#[test]
fn high_contrast_is_black_and_white_whatever_the_accent() {
    let d = desk(Mode::HighContrast, DEFAULT_ACCENT);
    assert_eq!((d.bar.panel_fill, d.bar.text, d.menu, d.menu_ink, d.card, d.card_ink), (0, 0xFFFFFF, 0, 0xFFFFFF, 0, 0xFFFFFF));
    let t = tokens(Mode::HighContrast, DEFAULT_ACCENT);
    assert_eq!(d.bar.active, t.accent & 0xFF_FFFF, "sex_theme's high-contrast accent");
    for a in ACCENTS {
        let other = desk(Mode::HighContrast, a);
        assert_eq!(colours(&other), colours(&d), "accent {a:06x} changes nothing in high contrast");
    }
}

#[test]
fn the_accent_marks_what_is_active() {
    for m in [Mode::Light, Mode::Dark] { for a in ACCENTS {
        let d = desk(m, a);
        let accent = tokens(m, a).accent & 0xFF_FFFF;
        assert_eq!((d.bar.active, d.bar.chip_fill, d.focus, d.menu_send), (accent, accent, accent, accent), "{m:?} {a:06x}");
        assert_eq!(d.accent, a);
    } }
    assert_ne!(desk(Mode::Light, 0x3584E4).bar.active, desk(Mode::Light, DEFAULT_ACCENT).bar.active);
}

#[test]
fn text_reads_on_what_it_is_drawn_on() {
    for m in MODES { for a in ACCENTS {
        let d = desk(m, a);
        let ok = |name: &str, ink: u32, under: u32, min: f32| {
            let c = contrast(ink, under);
            assert!(c >= min, "{m:?} {a:06x}: {name} {ink:06x} on {under:06x} is {c:.2}, under {min}");
        };
        ok("bar text", d.bar.text, d.bar.panel_fill, 7.0);
        ok("menu ink", d.menu_ink, d.menu, 7.0);
        ok("menu ink on picked row", d.menu_ink, d.menu_picked, 4.5);
        ok("menu title", d.menu_title, d.menu, 7.0);
        ok("menu soft", d.menu_soft, d.menu, 4.5);
        ok("menu dim", d.menu_dim, d.menu, 3.0);
        ok("field ink", d.menu_ink, d.menu_field, 7.0);
        ok("button ink", d.menu_ink, d.menu_button, 4.5);
        ok("Send", d.menu_on_send, d.menu_send, 4.5);
        ok("Close", d.menu_close, d.menu, 3.0);
        ok("card ink", d.card_ink, d.card, 7.0);
        ok("card soft", d.card_soft, d.card, 4.5);
        ok("workspace dot", d.bar.muted, d.bar.panel_fill, if m == Mode::Dark { 1.0 } else { 3.0 });
        ok("tray icon", tray_ink(tray_icons::FG, &d), d.bar.panel_fill, 7.0);
        ok("low battery", tray_ink(tray_icons::WARN, &d), d.bar.panel_fill, 3.0);
        ok("charging", tray_ink(tray_icons::GOOD, &d), d.bar.panel_fill, 3.0);
        assert_ne!(d.menu_picked, d.menu, "{m:?}: the hovered row shows");
    } }
}

#[test]
fn tray_icons_take_the_theme_ink() {
    let d = desk(Mode::Light, DEFAULT_ACCENT);
    assert_eq!(tray_ink(tray_icons::FG, &d), d.bar.text);
    assert_eq!(tray_ink(tray_icons::DIM, &d), d.menu_dim);
    assert_eq!(tray_ink(0x123456, &d), 0x123456, "other inks pass");
}

#[test]
fn display_follows_the_look_it_is_told() {
    // Display starts in the settings' defaults: light, SexOS pink.
    assert_eq!(desktop_look::theme(), (0, DEFAULT_ACCENT));
    assert_eq!(*desk_theme::current(), desk(Mode::Light, DEFAULT_ACCENT));
    assert!(desktop_look::set_look(0, 0, desktop_look::theme_word(1, 0x3584E4)));
    assert!(desk_theme::refresh(), "a new theme repaints");
    assert_eq!(*desk_theme::current(), desk(Mode::Dark, 0x3584E4));
    assert!(!desk_theme::refresh(), "once");
    assert!(desktop_look::set_look(2, 0, 0), "a look without a theme keeps it");
    assert!(!desk_theme::refresh());
    assert_eq!(desk_theme::current().mode, Mode::Dark);
    assert!(desktop_look::set_look(0, 0, desktop_look::theme_word(0, DEFAULT_ACCENT)) && desk_theme::refresh());
    assert_eq!(desk_theme::current().mode, Mode::Light);
}
