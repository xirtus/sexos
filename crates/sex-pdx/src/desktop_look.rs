//! Wallpaper and night light: the Shell chooses, Display draws.
//!
//! The Shell sends `OP_DESKTOP_LOOK` with the wallpaper and the night light
//! level; Display keeps them here and paints every frame with them. The
//! wallpaper is a vertical gradient, and the first one, the SexOS wallpaper,
//! also carries the lips mark (`brand.rs`) in its middle; night light warms the finished frame by
//! scaling green and blue down, the way redshift/gammastep and Plasma's Night
//! Color do, so programs need not know about it.
//!
//! The same message carries the theme (slice S6a): Settings'
//! `appearance.theme` and `appearance.accent`, which programs already follow
//! through `sex_theme`, so the bar, window chrome, menus and desktop cards
//! repaint to match them.

use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

/// Shell -> Display. arg0 = wallpaper index, arg1 = night light level,
/// arg2 = theme word (`theme_word`), or 0 to keep the theme as it is.
pub const OP_DESKTOP_LOOK: u64 = 0x2DA;

/// Light, dark and high contrast: `appearance.theme` 0, 1 and 2.
pub const THEME_MODES: u8 = 3;
/// The mode names, in setting order, for receipts.
pub const THEME_NAMES: [&str; THEME_MODES as usize] = ["light", "dark", "high-contrast"];
/// SexOS pink, the default accent (`sex_theme::DEFAULT_ACCENT`).
pub const DEFAULT_ACCENT: u32 = 0xD6336C;

/// The theme word for arg2: bits 0..8 hold the mode plus one (so 0 means no
/// theme), bits 8..32 the accent `0xRRGGBB`; every higher bit is zero.
pub const fn theme_word(mode: u8, accent: u32) -> u64 {
    (mode as u64 + 1) | ((accent as u64 & 0xFF_FFFF) << 8)
}
/// The (mode, accent) a theme word names: None for 0 (keep the theme), and
/// Err for anything that is not a theme word.
pub const fn decode_theme(word: u64) -> Result<Option<(u8, u32)>, ()> {
    if word == 0 { return Ok(None); }
    let mode = (word & 0xFF) as u8;
    if word >> 32 != 0 || mode == 0 || mode > THEME_MODES { return Err(()); }
    Ok(Some((mode - 1, (word >> 8) as u32 & 0xFF_FFFF)))
}

/// Gradients (top, bottom). Index 0, the default, is the SexOS wallpaper:
/// warm charcoal (`brand::WALL_TOP`/`WALL_BOTTOM`) with the lips mark on it.
/// Append only: settings files keep the index by name (`sex_config::WALLPAPERS`).
pub const WALLPAPERS: [(&str, u32, u32); 6] = [
    ("SexOS", 0x00332E2F, 0x001C1919),
    ("Dusk", 0x002A1B3D, 0x00A8505A),
    ("Forest", 0x000F2A20, 0x004E7A4A),
    ("Ocean", 0x00081A33, 0x00287A9A),
    ("Slate", 0x001C1E26, 0x004A4E5C),
    // The desktop theme's own navy gradient, the default before the lips.
    ("Midnight", 0x00081424, 0x001E1E2E),
];
/// The wallpaper that carries the mark.
pub const MARK_WALLPAPER: u8 = 0;
/// Night light off, low, medium, high.
pub const NIGHT_LEVELS: u8 = 4;

static WALLPAPER: AtomicU8 = AtomicU8::new(0);
static NIGHT: AtomicU8 = AtomicU8::new(0);
static PUBLISHED_NIGHT: AtomicU8 = AtomicU8::new(0);
/// The theme Display paints with, as a theme word; it starts at the settings'
/// defaults (light, SexOS pink), which is what the Shell holds before it has
/// read the person's settings.
static THEME: AtomicU32 = AtomicU32::new(theme_word(0, DEFAULT_ACCENT) as u32);

/// Takes a new look; false (and no change) when either value is out of range.
pub fn set(wallpaper: u64, night: u64) -> bool {
    if wallpaper >= WALLPAPERS.len() as u64 || night >= NIGHT_LEVELS as u64 { return false; }
    WALLPAPER.store(wallpaper as u8, Ordering::Relaxed);
    NIGHT.store(night as u8, Ordering::Relaxed);
    true
}
/// Takes a whole `OP_DESKTOP_LOOK`: false (and no change at all) when any of
/// the three is out of range or the theme word has unknown bits.
pub fn set_look(wallpaper: u64, night: u64, theme: u64) -> bool {
    let Ok(theme) = decode_theme(theme) else { return false };
    if wallpaper >= WALLPAPERS.len() as u64 || night >= NIGHT_LEVELS as u64 { return false; }
    if let Some((mode, accent)) = theme { THEME.store(theme_word(mode, accent) as u32, Ordering::Relaxed); }
    set(wallpaper, night)
}
/// The theme Display paints with: (mode, accent `0xRRGGBB`).
pub fn theme() -> (u8, u32) {
    match decode_theme(THEME.load(Ordering::Relaxed) as u64) { Ok(Some(t)) => t, _ => (0, DEFAULT_ACCENT) }
}
/// The receipt name of a theme mode.
pub fn theme_name(mode: u8) -> &'static str { THEME_NAMES.get(mode as usize).copied().unwrap_or("light") }
pub fn wallpaper_index() -> u8 { WALLPAPER.load(Ordering::Relaxed) }
pub fn night_level() -> u8 { NIGHT.load(Ordering::Relaxed) }
/// True once after the night light level changes, for the publisher: the
/// frame on screen was warmed by the old level, so all of it is written again.
pub fn take_night_change() -> bool {
    let now = night_level();
    PUBLISHED_NIGHT.swap(now, Ordering::Relaxed) != now
}

/// The chosen gradient, or None for the theme's own.
pub fn wallpaper() -> Option<(u32, u32)> {
    WALLPAPERS.get(wallpaper_index() as usize).map(|&(_, top, bottom)| (top, bottom))
}
/// True when the wallpaper on screen carries the lips mark.
pub fn shows_mark() -> bool { wallpaper_index() == MARK_WALLPAPER }

/// Green and blue kept per level, out of 256: about 4500 K, 3700 K, 2900 K.
const KEEP: [(u32, u32); NIGHT_LEVELS as usize] = [(256, 256), (236, 200), (218, 156), (198, 112)];

/// One XRGB pixel warmed by `level` (0 leaves it as it is).
#[inline]
pub fn night(pixel: u32, level: u8) -> u32 {
    let (g, b) = KEEP[(level as usize).min(KEEP.len() - 1)];
    let green = ((pixel >> 8 & 0xFF) * g) >> 8;
    let blue = ((pixel & 0xFF) * b) >> 8;
    (pixel & 0x00FF_0000) | green << 8 | blue
}
