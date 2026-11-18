// SPDX-License-Identifier: GPL-3.0-only
//! Hot corners (slice S5): pushing the pointer into a screen corner runs that
//! corner's action, as GNOME's Activities corner and KDE's screen edges do. By
//! default the top-left corner opens the window overview; Settings
//! (`desktop.corner_*`) gives each corner one of [`ACTIONS`].
//!
//! A corner fires once when the pointer reaches its corner pixel, and again
//! only after the pointer has left it by [`REARM`] pixels. It never fires
//! while a button is held (a window or selection dragged into the corner), and
//! not for a pointer that was already in the corner when tracking began (the
//! boot pointer sits at 0,0). Pure logic, host-tested in `tests/hot_corners.rs`.

/// Corner choices, in `sex_config::CORNER_ACTIONS` order (append-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { Nothing, Overview, Launcher, Notifications, Lock }
pub const ACTIONS: [Action; 5] = [Action::Nothing, Action::Overview, Action::Launcher, Action::Notifications, Action::Lock];

impl Action {
    /// The action for a `desktop.corner_*` setting value; unknown values do nothing.
    pub fn from_setting(v: i32) -> Action { ACTIONS.get(v as usize).copied().unwrap_or(Action::Nothing) }
    pub fn name(self) -> &'static str {
        match self { Action::Nothing => "none", Action::Overview => "overview", Action::Launcher => "launcher", Action::Notifications => "notifications", Action::Lock => "lock" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner { TopLeft, TopRight, BottomLeft, BottomRight }
pub const CORNERS: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];

impl Corner {
    pub fn index(self) -> usize { self as usize }
    pub fn name(self) -> &'static str {
        match self { Corner::TopLeft => "top_left", Corner::TopRight => "top_right", Corner::BottomLeft => "bottom_left", Corner::BottomRight => "bottom_right" }
    }
    /// The corner pixel of a `w`×`h` screen.
    pub fn point(self, w: i32, h: i32) -> (i32, i32) {
        match self { Corner::TopLeft => (0, 0), Corner::TopRight => (w - 1, 0), Corner::BottomLeft => (0, h - 1), Corner::BottomRight => (w - 1, h - 1) }
    }
}

/// Pixels the pointer must move away from every corner before one fires again.
pub const REARM: i32 = 8;

/// The corner whose pixel the pointer is on.
pub fn at(x: i32, y: i32, w: i32, h: i32) -> Option<Corner> {
    if w <= 0 || h <= 0 { return None; }
    CORNERS.into_iter().find(|c| c.point(w, h) == (x, y))
}

/// How far the pointer is from the nearest corner (the larger of the two
/// distances along the axes).
pub fn distance(x: i32, y: i32, w: i32, h: i32) -> i32 {
    CORNERS.iter().map(|c| { let (cx, cy) = c.point(w, h); (x - cx).abs().max((y - cy).abs()) }).min().unwrap_or(i32::MAX)
}

pub struct Tracker { armed: bool }

impl Tracker {
    /// Disarmed until the pointer is first seen away from the corners.
    pub const fn new() -> Self { Tracker { armed: false } }
    pub fn armed(&self) -> bool { self.armed }
    /// The pointer is at `x`,`y` on a `w`×`h` screen; `held`: a button is down
    /// or something holds the pointer. The corner that fires now, if one does.
    pub fn update(&mut self, x: i32, y: i32, w: i32, h: i32, held: bool) -> Option<Corner> {
        if distance(x, y, w, h) >= REARM { self.armed = true; return None; }
        let corner = at(x, y, w, h)?;
        if !core::mem::replace(&mut self.armed, false) || held { return None; }
        Some(corner)
    }
}
