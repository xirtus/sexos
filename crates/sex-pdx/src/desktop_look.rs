//! Wallpaper and night light: the Shell chooses, Display draws.
//!
//! The Shell sends `OP_DESKTOP_LOOK` with the wallpaper and the night light
//! level; Display keeps them here and paints every frame with them. The
//! wallpaper is a vertical gradient; night light warms the finished frame by
//! scaling green and blue down, the way redshift/gammastep and Plasma's Night
//! Color do, so programs need not know about it.

use core::sync::atomic::{AtomicU8, Ordering};

/// Shell -> Display. arg0 = wallpaper index, arg1 = night light level.
pub const OP_DESKTOP_LOOK: u64 = 0x2DA;

/// Gradients (top, bottom). Index 0 is the theme's own background.
pub const WALLPAPERS: [(&str, u32, u32); 5] = [
    ("Theme", 0, 0),
    ("Dusk", 0x002A1B3D, 0x00A8505A),
    ("Forest", 0x000F2A20, 0x004E7A4A),
    ("Ocean", 0x00081A33, 0x00287A9A),
    ("Slate", 0x001C1E26, 0x004A4E5C),
];
/// Night light off, low, medium, high.
pub const NIGHT_LEVELS: u8 = 4;

static WALLPAPER: AtomicU8 = AtomicU8::new(0);
static NIGHT: AtomicU8 = AtomicU8::new(0);
static PUBLISHED_NIGHT: AtomicU8 = AtomicU8::new(0);

/// Takes a new look; false (and no change) when either value is out of range.
pub fn set(wallpaper: u64, night: u64) -> bool {
    if wallpaper >= WALLPAPERS.len() as u64 || night >= NIGHT_LEVELS as u64 { return false; }
    WALLPAPER.store(wallpaper as u8, Ordering::Relaxed);
    NIGHT.store(night as u8, Ordering::Relaxed);
    true
}
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
    match WALLPAPERS.get(wallpaper_index() as usize) {
        Some(&(_, top, bottom)) if wallpaper_index() != 0 => Some((top, bottom)),
        _ => None,
    }
}

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
