// SPDX-License-Identifier: GPL-3.0-only
//! The desktop theme (slice S6a): Settings' `appearance.theme` and
//! `appearance.accent`, which every program already follows through
//! `sex_theme`, go to Display with the wallpaper and night light
//! (`OP_DESKTOP_LOOK` arg2, `sex_pdx::desktop_look::theme_word`), so the bar,
//! window chrome, menus and desktop cards repaint to match the programs.
//! `config_watch` sends the look again whenever either setting changes.
//! The word also carries `accessibility.reduce_transparency` (slice S6b):
//! Display then draws menus and panels opaque instead of frosted glass.
use sex_pdx::config::schema::id;
use sex_pdx::desktop_look as look;

/// The theme Display last took (None before the first).
static mut SENT: Option<u64> = None;

/// The theme word for `OP_DESKTOP_LOOK` from the settings the Shell holds.
/// Unknown modes are light, as in `sex_theme::Mode::from_setting`.
pub fn word() -> u64 {
    let mode = match crate::config_watch::value(id::THEME) { 1 => 1, 2 => 2, _ => 0 };
    let reduce = crate::config_watch::value(id::REDUCE_TRANSPARENCY) != 0;
    look::theme_word_reduced(mode, crate::config_watch::value(id::ACCENT) as u32 & 0xFF_FFFF, reduce)
}

/// After a look went to Display carrying `word`: a receipt when the theme is
/// new to Display, or when Display did not take it (the look stays owed).
pub fn sent(word: u64, ok: bool) {
    let last = unsafe { &mut *core::ptr::addr_of_mut!(SENT) };
    if ok && *last == Some(word) { return; }
    if ok { *last = Some(word); }
    if let Ok(Some((mode, accent))) = look::decode_theme(word) {
        sex_pdx::serial_record!("[shell.theme] mode={} accent={:06x} status={} reduce_transparency={}", look::theme_name(mode), accent,
            if ok { "ok" } else { "owed" }, look::reduces_transparency(word) as u8);
    }
}

/// Display started again with the default theme: the next look is new to it.
pub fn display_restarted() { unsafe { *core::ptr::addr_of_mut!(SENT) = None; } }
