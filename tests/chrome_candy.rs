// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for `servers/sexdisplay/src/chrome_candy.rs`:
//! - Per-window harmonious Candy UI palette & matching full-frame derivation
//! - Clear 4 colored window buttons with micro-glyphs and hover ring
//! - Modern Apple-style SilkBar glass in Light, Dark, HighContrast, and Reduce-Transparency modes
//! - Rounded workspace capsules and clean bell icon silhouette

#[path = "../servers/sexdisplay/src/chrome_candy.rs"]
mod chrome_candy;

use chrome_candy::*;
use sex_pdx::desktop_style::chrome::Metrics;

fn luma(c: u32) -> f32 {
    let r = ((c >> 16) & 0xFF) as f32;
    let g = ((c >> 8) & 0xFF) as f32;
    let b = (c & 0xFF) as f32;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

#[test]
fn per_window_auto_hue_and_explicit_swatches() {
    // Core apps get distinct harmonious candy hues in Auto mode (0).
    let q = window_candy_hue(201, 0); // Quil
    let s = window_candy_hue(153, 0); // Spindle
    let l = window_candy_hue(200, 0); // Linen
    assert_ne!(q, s);
    assert_ne!(s, l);
    assert_ne!(q, l);
    // Explicit swatches 1..=8 override Auto deterministically.
    for swatch in 1..=CANDY_COUNT as u8 {
        assert_eq!(window_candy_hue(201, swatch), candy_swatch(swatch));
        assert_eq!(window_candy_hue(153, swatch), candy_swatch(swatch));
    }
}

#[test]
fn matching_window_frame_harmonizes_with_title_bar_in_dark_and_light() {
    let berry = candy_swatch(1);
    let sky = candy_swatch(3);
    for mode in [MODE_DARK, MODE_LIGHT] {
        let p_berry = window_palette(mode, berry, true, false);
        let p_sky = window_palette(mode, sky, true, false);
        // Different candy hues produce distinct matching title bars AND matching frame bands/rims.
        assert_ne!(p_berry.title_bar, p_sky.title_bar);
        assert_ne!(p_berry.band, p_sky.band);
        assert_ne!(p_berry.rim, p_sky.rim);
        // Focused windows are richer/crisper than unfocused ones.
        let p_unfocused = window_palette(mode, berry, false, false);
        assert_ne!(p_berry.title_bar, p_unfocused.title_bar);
        assert_ne!(p_berry.rim, p_unfocused.rim);
        // Hover brightens the rim.
        let p_hover = window_palette(mode, berry, true, true);
        assert_ne!(p_hover.rim, p_berry.rim);
    }
    // Dark mode stays dark, light mode stays light.
    let dark = window_palette(MODE_DARK, berry, true, false);
    let light = window_palette(MODE_LIGHT, berry, true, false);
    assert!(luma(dark.title_bar) < 100.0 && luma(dark.band) < 90.0);
    assert!(luma(light.title_bar) > 170.0 && luma(light.band) > 170.0);
    // High contrast ignores candy hue.
    assert_eq!(window_palette(MODE_HIGH_CONTRAST, berry, true, false), window_palette(MODE_HIGH_CONTRAST, sky, true, false));
}

#[test]
fn four_window_buttons_render_distinct_glyphs_and_hover_feedback() {
    let m = Metrics::new(1);
    let bg = 0x0024_1E2E;
    for index in 0..4u32 {
        let r = m.control(index).unwrap();
        let mut rest_pixels = 0usize;
        let mut hover_diffs = 0usize;
        for ly in r.y as usize..(r.y + r.h) as usize {
            for lx in r.x as usize..(r.x + r.w) as usize {
                let p_rest = control_button_pixel(lx, ly, bg, m, true, false, 0, true);
                let p_hover = control_button_pixel(lx, ly, bg, m, true, true, index as u8, true);
                if p_rest != bg { rest_pixels += 1; }
                if p_hover != p_rest { hover_diffs += 1; }
            }
        }
        assert!(rest_pixels >= 80, "button {index} should draw a crisp 12px circle ({rest_pixels} px)");
        assert!(hover_diffs >= 40, "button {index} should show unmistakable hover feedback ({hover_diffs} px)");
    }
}

#[test]
fn apple_style_silkbar_glass_adapts_to_dark_light_and_high_contrast() {
    let wall = 0x002A_2426;
    let light_px = bar_glass_pixel(14, 28, wall, MODE_LIGHT, 0x00F6_F5F7, false);
    let dark_px = bar_glass_pixel(14, 28, wall, MODE_DARK, 0x0018_1825, false);
    // Light mode SilkBar is airy and bright; dark mode is deep obsidian glass.
    assert!(luma(light_px) > 190.0, "light bar luma={}", luma(light_px));
    assert!(luma(dark_px) < 45.0, "dark bar luma={}", luma(dark_px));
    assert!(luma(light_px) > luma(dark_px) + 120.0);
    // Bottom hairline separator differs from body in every mode.
    assert_ne!(bar_glass_pixel(27, 28, wall, MODE_DARK, 0x0018_1825, false), dark_px);
    assert_ne!(bar_glass_pixel(27, 28, wall, MODE_LIGHT, 0x00F6_F5F7, false), light_px);
    assert_eq!(bar_glass_pixel(27, 28, wall, MODE_HIGH_CONTRAST, 0, false), 0x00FF_FFFF);
    assert_eq!(bar_glass_pixel(14, 28, wall, MODE_HIGH_CONTRAST, 0, false), 0x0000_0000);
    // Reduce transparency uses opaque panel_fill.
    assert_eq!(bar_glass_pixel(14, 28, wall, MODE_DARK, 0x0018_1825, true), 0x0018_1825);
}

#[test]
fn workspace_pills_and_bell_icon_render_cleanly() {
    let bg = 0x0015_1520;
    let mut active_px = 0usize;
    let mut inactive_px = 0usize;
    for y in 2..26 {
        for x in 100..124 {
            if workspace_pill_pixel(x, y, 100, 2, 24, 24, 1, true, false, 0x00D6_336C, bg).is_some() { active_px += 1; }
            if workspace_pill_pixel(x, y, 100, 2, 24, 24, 1, false, false, 0x0088_8899, bg).is_some() { inactive_px += 1; }
        }
    }
    // Active capsule is wider than inactive dot.
    assert!(active_px > inactive_px * 2, "active={active_px} inactive={inactive_px}");
    // Bell icon draws a crisp silhouette and lights up badge when notifications exist.
    let mut bell_px = 0usize;
    let mut badge_px = 0usize;
    for y in 2..26 {
        for x in 200..224 {
            if let Some(c) = bell_icon_pixel(x, y, 200, 2, 24, 24, 1, true, 2, 0, 0x00FF_FFFF, 0x0088_8899, bg) {
                bell_px += 1;
                if c == 0x00FF_D700 { badge_px += 1; }
            }
        }
    }
    assert!(bell_px >= 40 && badge_px >= 6, "bell={bell_px} badge={badge_px}");
}
