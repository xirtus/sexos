// SPDX-License-Identifier: GPL-3.0-only
//! The desktop's own colours follow the theme programs use (slice S6a).
//!
//! Settings' `appearance.theme` and `appearance.accent` reach Display in
//! `OP_DESKTOP_LOOK` (`sex_pdx::desktop_look::theme`). Here they become the
//! colours of the bar (`silkbar_model::Theme`), the window chrome, the menus
//! and popups, and the desktop cards, from the same `sex_theme` tokens Weft
//! and every program draw with. Dark keeps the desktop's long-standing navy
//! look; light gives light panels with dark ink; high contrast is black and
//! white with sex_theme's strong yellow. The accent marks what is active.
//!
//! Colours here are XRGB (`0x00RRGGBB`), as Display writes them.
use sex_theme::{mix, Mode};
use silkbar_model::Theme;

/// Every colour the desktop draws itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desk {
    pub mode: Mode,
    /// The accent as chosen (`0xRRGGBB`), for receipts.
    pub accent: u32,
    /// The bar. `bg_top`/`bg_bottom` stay the wallpaper's business.
    pub bar: Theme,
    /// Window chrome: the focus colour, rim and title bar divider, the title
    /// bar's glass, and the tabs on it.
    pub focus: u32,
    pub rim: u32,
    pub title_bar: u32,
    pub active_tab: u32,
    pub inactive_tab: u32,
    /// The quiet inset of a window's resize band.
    pub band: u32,
    /// Menus and popups (window menu, tray menus, switcher, program tray).
    pub menu: u32,
    pub menu_rim: u32,
    pub menu_picked: u32,
    pub menu_ink: u32,
    /// Headings, a step brighter than the ink; secondary and disabled text.
    pub menu_title: u32,
    pub menu_soft: u32,
    pub menu_dim: u32,
    pub menu_field: u32,
    pub menu_button: u32,
    /// The accent button on a popup (Send), and the ink on it.
    pub menu_send: u32,
    pub menu_on_send: u32,
    pub menu_close: u32,
    /// Desktop cards (clock, weather, system) over the wallpaper.
    pub card: u32,
    pub card_rim: u32,
    pub card_ink: u32,
    pub card_soft: u32,
    /// Tray icon inks: charging and a low battery.
    pub good: u32,
    pub warn: u32,
}

const fn x(c: u32) -> u32 { c & 0x00FF_FFFF }
/// `a` mixed toward `b` by `t` (0..1), as XRGB.
pub fn blend(a: u32, b: u32, t: f32) -> u32 { x(mix(a, b, t)) }

/// The desktop colours for `mode` with accent `accent` (`0xRRGGBB`).
pub fn desk(mode: Mode, accent: u32) -> Desk {
    let accent = accent & 0xFF_FFFF;
    let t = sex_theme::tokens(mode, accent);
    let a = x(t.accent);
    match mode {
        // Today's desktop: the navy palette (`silkbar_model::DEFAULT_THEME`
        // and the old menu/card constants), with the accent for what is active.
        Mode::Dark => Desk {
            mode, accent,
            bar: Theme {
                active: a, chip_fill: a,
                ..silkbar_model::DEFAULT_THEME
            },
            focus: a, rim: 0x00B4_BEFE, title_bar: 0x001E_1E2E,
            active_tab: blend(0x001E_1E2E, a, 0.45), inactive_tab: 0x0045_475A,
            band: sex_pdx::desktop_style::NATIVE_CANVAS,
            menu: 0x0018_1825, menu_rim: 0x0045_475A, menu_picked: 0x0031_3244,
            menu_ink: 0x00CD_D6F4, menu_title: 0x00FF_FFFF, menu_soft: 0x0093_99B2, menu_dim: 0x006C_7086, menu_field: 0x0011_111B, menu_button: 0x0031_3244,
            menu_send: a, menu_on_send: x(t.on_accent), menu_close: 0x00F3_8BA8,
            card: 0x0014_121A, card_rim: 0x0046_3E4C, card_ink: 0x00F4_EEF2, card_soft: 0x00C2_B6C0,
            good: 0x00A6_E3A1, warn: 0x00F3_8BA8,
        },
        Mode::Light => Desk {
            mode, accent,
            bar: Theme {
                bg_top: silkbar_model::DEFAULT_THEME.bg_top, bg_bottom: silkbar_model::DEFAULT_THEME.bg_bottom,
                panel_fill: x(t.window), panel_glow: x(t.border), text: x(t.text), muted: x(t.text_dim),
                active: a, urgent: x(t.danger), chip_fill: a, chip_border: x(t.border),
            },
            focus: a, rim: x(mix(t.border, t.text_dim, 0.35)), title_bar: x(t.control),
            active_tab: blend(t.surface, t.accent, 0.30), inactive_tab: x(t.control_pressed),
            band: x(t.window),
            menu: x(t.surface), menu_rim: x(t.border), menu_picked: x(t.control_hover),
            menu_ink: x(t.text), menu_title: x(t.text), menu_soft: x(t.text_dim), menu_dim: x(t.text_dim), menu_field: x(t.window), menu_button: x(t.control),
            menu_send: a, menu_on_send: x(t.on_accent), menu_close: x(t.danger),
            card: x(t.surface), card_rim: x(t.border), card_ink: x(t.text), card_soft: x(t.text_dim),
            good: 0x002E_7D32, warn: x(t.danger),
        },
        // Black and white; the accent is sex_theme's high-contrast yellow
        // whatever the person chose, as in every program.
        Mode::HighContrast => Desk {
            mode, accent,
            bar: Theme {
                bg_top: silkbar_model::DEFAULT_THEME.bg_top, bg_bottom: silkbar_model::DEFAULT_THEME.bg_bottom,
                panel_fill: x(t.window), panel_glow: x(t.border), text: x(t.text), muted: x(t.text_disabled),
                active: a, urgent: x(t.danger), chip_fill: a, chip_border: x(t.border),
            },
            focus: x(t.focus), rim: x(t.border), title_bar: x(t.window),
            active_tab: x(t.control_pressed), inactive_tab: x(t.window),
            band: x(t.window),
            menu: x(t.surface), menu_rim: x(t.border), menu_picked: x(t.control_pressed),
            menu_ink: x(t.text), menu_title: x(t.text), menu_soft: x(t.text), menu_dim: x(t.text_disabled), menu_field: x(t.field), menu_button: x(t.control_pressed),
            menu_send: a, menu_on_send: x(t.on_accent), menu_close: x(t.danger),
            card: x(t.surface), card_rim: x(t.border), card_ink: x(t.text), card_soft: x(t.text),
            good: 0x0000_FF44, warn: x(t.danger),
        },
    }
}

/// A tray icon's ink (`tray_icons::FG`/`DIM`/`GOOD`/`WARN`) in the theme's
/// colours, so the icons read on a light bar as well as a dark one.
pub fn tray_ink(ink: u32, d: &Desk) -> u32 {
    match ink {
        crate::tray_icons::FG => d.bar.text,
        crate::tray_icons::DIM => d.menu_dim,
        crate::tray_icons::GOOD => d.good,
        crate::tray_icons::WARN => d.warn,
        other => other,
    }
}

/// The colours for the theme Display was last told, worked out once per theme.
struct Current { key: Option<(u8, u32)>, desk: Desk }
static mut CURRENT: Current = Current {
    key: None,
    // Overwritten before anyone reads it: `key` None forces the first sum.
    desk: Desk {
        mode: Mode::Dark, accent: 0, bar: silkbar_model::DEFAULT_THEME,
        focus: 0, rim: 0, title_bar: 0, active_tab: 0, inactive_tab: 0, band: 0,
        menu: 0, menu_rim: 0, menu_picked: 0, menu_ink: 0, menu_title: 0, menu_soft: 0, menu_dim: 0, menu_field: 0, menu_button: 0,
        menu_send: 0, menu_on_send: 0, menu_close: 0, card: 0, card_rim: 0, card_ink: 0, card_soft: 0,
        good: 0, warn: 0,
    },
};

/// The desktop colours now. Display has one thread; the colours change only
/// between frames, when `OP_DESKTOP_LOOK` brings a new theme.
pub fn current() -> &'static Desk {
    let key = sex_pdx::desktop_look::theme();
    let c = unsafe { &mut *core::ptr::addr_of_mut!(CURRENT) };
    if c.key != Some(key) {
        c.desk = desk(Mode::from_setting(key.0 as i32), key.1);
        c.key = Some(key);
    }
    &c.desk
}

/// Takes the theme from `OP_DESKTOP_LOOK`: true (with the receipt) when the
/// colours changed and the whole desktop has to be painted again.
pub fn refresh() -> bool {
    let before = unsafe { (*core::ptr::addr_of!(CURRENT)).key };
    let d = current();
    if before == Some(sex_pdx::desktop_look::theme()) { return false; }
    sex_pdx::serial_record!("[display.theme] mode={} accent={:06x}", sex_pdx::desktop_look::theme_name(d.mode.setting() as u8), d.accent);
    true
}
