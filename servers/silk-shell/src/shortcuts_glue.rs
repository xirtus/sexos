// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `shortcuts.rs`: the Super chord table with the person's keys, from
//! the settings the Shell holds (`config_watch`). `WM_CHORDS` asks it for every chord.
use crate::shortcuts;
use crate::wm::{self, code, Mods, WmAction};
use sex_config::id;

/// The shortcuts Settings can move: setting, own key, action. Order is `shortcuts::pick`'s.
const SHORTCUTS: [(u16, u8, WmAction); 6] = [
    (id::SHORTCUT_LAUNCHER, code::D, WmAction::Launcher),
    (id::SHORTCUT_TERMINAL, code::ENTER, WmAction::Terminal),
    (id::SHORTCUT_OVERVIEW, code::W, WmAction::Overview),
    (id::SHORTCUT_NOTIFICATIONS, code::N, WmAction::Notifications),
    (id::SHORTCUT_LOCK, code::ESC, WmAction::Lock),
    (id::SHORTCUT_CLOSE, code::Q, WmAction::Close),
];

pub fn binding(key: u8, m: Mods) -> Option<WmAction> {
    if m.shift || m.ctrl { return wm::binding(key, m); }
    let own = SHORTCUTS.map(|s| s.1);
    let chosen = SHORTCUTS.map(|s| crate::config_watch::value(s.0));
    match shortcuts::pick(key, &own, &chosen) {
        Some(Some(i)) => Some(SHORTCUTS[i].2),
        Some(None) => None,
        None => wm::binding(key, m),
    }
}
