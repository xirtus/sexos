// SPDX-License-Identifier: GPL-3.0-only
//! The desktop's own colours follow the theme programs use (slice S6a).
//!
//! Settings' `appearance.theme` and `appearance.accent` reach Display in
//! `OP_DESKTOP_LOOK` (`sex_pdx::desktop_look::theme`). Here they become the
//! colours of the bar (`silkbar_model::Theme`), the window chrome, the menus
//! and popups, and the desktop cards, from the same `sex_theme` tokens Weft
//! and every program draw with. Dark uses macOS Noir obsidian glass surfaces
//! (`#0B0D12` base, `#13161F` cards/menus, `#1B202C` elevated/hover, `#2A3142`
//! specular rims); light gives light panels with dark ink; high contrast is
//! black and white with sex_theme's strong yellow. The accent marks what is active.
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

impl Desk {
    /// The text drawn on a popup's background, which its frosted glass must
    /// keep readable (slice S6b, `glass::params`): body, headings, secondary
    /// text and Close. Disabled text (`menu_dim`) is exempt, as in WCAG.
    pub const fn glass_inks(&self) -> [u32; 4] { [self.menu_ink, self.menu_title, self.menu_soft, self.menu_close] }
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
        // macOS Noir Obsidian Glass surfaces (`#0B0D12` base, `#13161F` cards/menus,
        // `#1B202C` hover/buttons, `#2A3142` specular rim) with active Noir accent.
        Mode::Dark => Desk {
            mode, accent,
            bar: Theme {
                panel_fill: 0x000B_0D12,
                panel_glow: 0x002A_3142,
                text: 0x00F1_F5F9,
                muted: 0x0064_748B,
                active: a,
                chip_fill: a,
                ..silkbar_model::DEFAULT_THEME
            },
            focus: a, rim: 0x002A_3142, title_bar: 0x0013_161F,
            active_tab: blend(0x0013_161F, a, 0.42), inactive_tab: 0x000E_1118,
            band: sex_pdx::desktop_style::NATIVE_CANVAS,
            menu: 0x0013_161F, menu_rim: 0x002A_3142, menu_picked: 0x001B_202C,
            menu_ink: 0x00F1_F5F9, menu_title: 0x00FF_FFFF, menu_soft: 0x0094_A3B8, menu_dim: 0x0064_748B, menu_field: 0x0008_0A0E, menu_button: 0x001B_202C,
            menu_send: a, menu_on_send: x(t.on_accent), menu_close: 0x00FB_7185,
            card: 0x0013_161F, card_rim: 0x002A_3142, card_ink: 0x00F1_F5F9, card_soft: 0x0094_A3B8,
            good: 0x0034_D399, warn: 0x00FB_7185,
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

static TINT_IDX: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Updates the active curated Noir accent tint index (`0..=3`, from `SilkBar.phase1.accent_tint_idx`).
pub fn set_tint_idx(idx: u8) {
    TINT_IDX.store(idx % 4, core::sync::atomic::Ordering::Relaxed);
}

/// Active curated [`sex_theme::NoirAccent`] from the SilkBar accent tint index.
pub fn active_noir_accent() -> sex_theme::NoirAccent {
    sex_theme::NoirAccent::from_index(TINT_IDX.load(core::sync::atomic::Ordering::Relaxed))
}

/// Resolved highlight/rim accent (`0x00RRGGBB`) honoring Settings custom accents and
/// the four curated Noir Accent Tints when Settings uses the default accent.
pub fn resolved_accent() -> u32 {
    let d = current();
    if d.mode == Mode::HighContrast {
        return d.focus;
    }
    if d.accent == sex_theme::DEFAULT_ACCENT {
        active_noir_accent().primary() & 0x00FF_FFFF
    } else {
        d.focus
    }
}

/// Resolved solid button/pill fill + readable foreground ink `(fill, on_fill)` (`0x00RRGGBB`).
pub fn resolved_fill_and_ink() -> (u32, u32) {
    let d = current();
    if d.mode == Mode::HighContrast {
        return (d.menu_send, d.menu_on_send);
    }
    if d.accent == sex_theme::DEFAULT_ACCENT {
        let tint = active_noir_accent();
        (tint.fill() & 0x00FF_FFFF, tint.on_accent() & 0x00FF_FFFF)
    } else {
        (d.menu_send, d.menu_on_send)
    }
}
