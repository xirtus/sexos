// SPDX-License-Identifier: GPL-3.0-only
//! Window decorations (slice S1), the pure part: which title bar button a
//! point is on, when two presses make a double click, and the window menu's
//! state and what each of its rows does. The Shell glue (`decorations_glue.rs`)
//! runs the actions with the window manager's own close, maximise, float and
//! move-to-workspace paths; Display draws the buttons and the menu from the
//! same `sex_pdx` geometry, so a click lands on what is drawn.
//!
//! Host-tested by tests/decorations.rs against a `sex_pdx` built from only
//! `desktop_style` (and `app_catalog`) and `window_menu`.

use sex_pdx::desktop_style::chrome::{Metrics, CONTROLS};
use sex_pdx::window_menu::{Hit, Item, Layout, Menu};

/// Title bar buttons, left to right, as Display paints them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button { Close, Minimise, Maximise, Float }

impl Button {
    pub const ALL: [Button; CONTROLS as usize] = [Button::Close, Button::Minimise, Button::Maximise, Button::Float];
}

/// Where a point falls on a window's title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleHit { Button(Button), Title, Outside }

/// (lx, ly) relative to the top-left corner of a `w` x `h` window with a title bar.
pub fn title_hit(m: Metrics, w: u32, h: u32, lx: i32, ly: i32) -> TitleHit {
    if lx < 0 || ly < 0 || lx as u32 >= w || ly as u32 >= h || ly as u32 >= m.title() { return TitleHit::Outside; }
    for (i, b) in Button::ALL.iter().enumerate() {
        if let Some(r) = m.control(i as u32) {
            // A button cut off by a narrow window is not there.
            if r.x + r.w <= w && r.contains(lx as u32, ly as u32) { return TitleHit::Button(*b); }
        }
    }
    TitleHit::Title
}

/// The centre of a button, relative to the window's top-left corner.
pub fn button_centre(m: Metrics, b: Button) -> (u32, u32) {
    let i = Button::ALL.iter().position(|x| *x == b).unwrap_or(0) as u32;
    let r = m.control(i).unwrap_or(sex_pdx::desktop_style::chrome::Rect { x: 0, y: 0, w: 0, h: 0 });
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// Two presses on the same title bar this close in time and place.
pub const DOUBLE_CLICK_MS: u64 = 400;
/// Logical pixels the pointer may move between the two presses.
pub const DOUBLE_CLICK_SLOP: i32 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DoubleClick { last: Option<(u32, u64, i32, i32)> }

impl DoubleClick {
    pub const fn new() -> Self { Self { last: None } }
    /// A press on `frame`'s title at `tick` (ms). True when it ends a double
    /// click; the pair is then forgotten, so a third press starts over.
    pub fn press(&mut self, frame: u32, tick: u64, x: i32, y: i32, slop: i32) -> bool {
        let double = matches!(self.last, Some((f, t, px, py)) if f == frame
            && tick >= t && tick - t <= DOUBLE_CLICK_MS
            && (x - px).abs() <= slop && (y - py).abs() <= slop);
        self.last = if double { None } else { Some((frame, tick, x, y)) };
        double
    }
    pub fn reset(&mut self) { self.last = None; }
}

/// What a button or a menu row does to its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { Close, Minimise, ToggleMaximise, ToggleFloat, MoveToWorkspace(u8) }

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Close => "close", Action::Minimise => "minimise", Action::ToggleMaximise => "maximise",
            Action::ToggleFloat => "float", Action::MoveToWorkspace(_) => "workspace",
        }
    }
}

pub fn button_action(b: Button) -> Action {
    match b { Button::Close => Action::Close, Button::Minimise => Action::Minimise, Button::Maximise => Action::ToggleMaximise, Button::Float => Action::ToggleFloat }
}

pub fn item_action(i: Item) -> Action {
    match i { Item::Close => Action::Close, Item::Maximise => Action::ToggleMaximise, Item::Float => Action::ToggleFloat, Item::Workspace(n) => Action::MoveToWorkspace(n) }
}

/// A left press while the menu shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// A row: the menu closes and its action runs on `frame`.
    Choose { frame: u32, action: Action },
    /// The menu's border or padding: nothing happens.
    Inside,
    /// Elsewhere: the menu closes and the press goes nowhere else.
    Dismiss,
}

/// The window menu: at most one, for one window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowMenu { open: Option<(u32, Menu)> }

impl WindowMenu {
    pub const fn new() -> Self { Self { open: None } }
    pub fn open(&mut self, frame: u32, menu: Menu) { self.open = Some((frame, menu)); }
    /// True when it was open.
    pub fn close(&mut self) -> bool { self.open.take().is_some() }
    pub fn is_open(&self) -> bool { self.open.is_some() }
    pub fn frame(&self) -> Option<u32> { self.open.map(|(f, _)| f) }
    /// The word Display is sent: zero hides it.
    pub fn wire(&self) -> u64 { self.open.map_or(0, |(_, m)| m.wire()) }

    pub fn layout(&self, w: usize, h: usize, scale: u32) -> Option<Layout> {
        self.open.and_then(|(_, m)| Layout::new(&m, w, h, scale))
    }

    pub fn press(&mut self, x: i32, y: i32, w: usize, h: usize, scale: u32) -> Press {
        let Some((frame, menu)) = self.open else { return Press::Dismiss };
        let hit = match (x, y, Layout::new(&menu, w, h, scale)) {
            (x, y, Some(l)) if x >= 0 && y >= 0 => l.hit(x as usize, y as usize),
            _ => Hit::Outside,
        };
        match hit {
            Hit::Inside => Press::Inside,
            Hit::Outside => { self.open = None; Press::Dismiss }
            Hit::Row(i) => {
                self.open = None;
                match menu.item(i) { Some(item) => Press::Choose { frame, action: item_action(item) }, None => Press::Dismiss }
            }
        }
    }

    /// The pointer moved: true when the highlighted row changed.
    pub fn hover(&mut self, x: i32, y: i32, w: usize, h: usize, scale: u32) -> bool {
        let Some((frame, menu)) = self.open else { return false };
        let row = match Layout::new(&menu, w, h, scale) {
            Some(l) if x >= 0 && y >= 0 => match l.hit(x as usize, y as usize) { Hit::Row(i) => Some(i as u8), _ => None },
            _ => None,
        };
        if row == menu.hover { return false; }
        self.open = Some((frame, Menu { hover: row, ..menu }));
        true
    }
}
