// SPDX-License-Identifier: GPL-3.0-only
//! Desktop widgets (slice S5b): the Shell turns Display's wallpaper cards
//! (`sex_pdx::desktop_widgets`) on or off from the `desktop.widgets` setting
//! and Super+Shift+W, with the weather card while `desktop.weather` is on.
//! Display fills the clock and system cards itself; `weather_glue` brings the
//! weather.
use sex_pdx::desktop_widgets as dw;

static mut ON: bool = false;
/// Display has not taken the latest state yet.
static mut OWED: bool = false;

unsafe fn send() {
    let cards = if !ON { 0 } else if crate::weather_glue::on() { dw::ALL } else { dw::CLOCK | dw::SYSTEM };
    crate::weather_glue::cards(ON);
    OWED = sex_pdx::pdx_call(sex_pdx::SLOT_DISPLAY, dw::OP_DESKTOP_WIDGETS, cards, 0, 0).0 != 0;
    sex_pdx::serial_record!("[shell.widgets] on={} ok={}", ON as u8, !OWED as u8);
}

/// From settings.
pub unsafe fn set(on: bool) {
    if ON == on && !OWED { return; }
    ON = on;
    send();
}

/// Super+Shift+W: flip, and keep it in settings.
pub unsafe fn toggle() -> bool {
    ON = !ON;
    send();
    crate::config_watch::changed_here(sex_config::id::WIDGETS, ON as i32);
    true
}

/// `desktop.weather` changed: the weather card comes or goes with the others.
pub unsafe fn weather(on: bool) {
    if crate::weather_glue::on() == on { return; }
    crate::weather_glue::setting(on);
    if ON { send(); }
}

/// Display started again: show them again if they were on.
pub fn display_restarted() { unsafe { OWED = ON; } }

/// Main loop: retry a refused send.
pub unsafe fn poll() { if OWED { send(); } }
