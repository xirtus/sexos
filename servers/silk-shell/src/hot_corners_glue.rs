// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `hot_corners.rs` (slice S5): the main loop's look at the
//! pointer runs a corner's action through `wm_action`, the path its keyboard
//! chord takes (Super+W for the overview).
use crate::hot_corners::{Action, Corner, Tracker};
use crate::wm::WmAction;
use sex_config::id;

static mut TRACKER: Tracker = Tracker::new();

fn tracker() -> &'static mut Tracker { unsafe { &mut *core::ptr::addr_of_mut!(TRACKER) } }

fn setting(c: Corner) -> u16 {
    match c { Corner::TopLeft => id::CORNER_TOP_LEFT, Corner::TopRight => id::CORNER_TOP_RIGHT,
        Corner::BottomLeft => id::CORNER_BOTTOM_LEFT, Corner::BottomRight => id::CORNER_BOTTOM_RIGHT }
}

/// Main loop: the pointer as it is now.
pub unsafe fn poll() {
    if crate::lock::locked() { return; }
    let held = crate::POINTER_BUTTONS != 0 || crate::POINTER_CAPTURE != 0
        || !matches!(crate::INTERACTION, crate::InteractionState::Idle);
    let Some(corner) = tracker().update(crate::POINTER_X, crate::POINTER_Y, crate::desktop_width(), crate::desktop_height(), held) else { return };
    let action = Action::from_setting(crate::config_watch::value(setting(corner)));
    let wm = match action {
        Action::Nothing => None,
        Action::Overview => Some(WmAction::Overview),
        Action::Launcher => Some(WmAction::Launcher),
        Action::Notifications => Some(WmAction::Notifications),
        Action::Lock => Some(WmAction::Lock),
    };
    let done = wm.is_some_and(|a| crate::wm_action(a));
    sex_pdx::serial_record!("[shell.hot_corner] corner={} action={} done={}", corner.name(), action.name(), done as u8);
}
