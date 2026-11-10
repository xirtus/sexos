//! Window manager policy in the style of Hyprland, river and KDE: Super key
//! bindings, tiling layouts with gaps, and focus by direction.
//!
//! Pure data and arithmetic with no IPC, so the host tests in
//! `tests/wm_layouts.rs` run this exact file. Shell owns frames and scenes;
//! this module only says where windows go and what a key chord means.

/// Key codes are the set-1 codes sexinput delivers for both PS/2 and USB.
pub mod code {
    pub const SUPER_LEFT: u8 = 0x5B;
    pub const SUPER_RIGHT: u8 = 0x5C;
    pub const SHIFT_LEFT: u8 = 0x2A;
    pub const SHIFT_RIGHT: u8 = 0x36;
    pub const CTRL_LEFT: u8 = 0x1D;
    pub const CTRL_RIGHT_USB: u8 = 0x61;
    pub const ESC: u8 = 0x01;
    pub const DIGIT_1: u8 = 0x02;
    pub const DIGIT_9: u8 = 0x0A;
    pub const TAB: u8 = 0x0F;
    pub const Q: u8 = 0x10;
    pub const W: u8 = 0x11;
    pub const BRACKET_LEFT: u8 = 0x1A;
    pub const BRACKET_RIGHT: u8 = 0x1B;
    pub const ENTER: u8 = 0x1C;
    pub const S: u8 = 0x1F;
    pub const D: u8 = 0x20;
    pub const F: u8 = 0x21;
    pub const H: u8 = 0x23;
    pub const J: u8 = 0x24;
    pub const K: u8 = 0x25;
    pub const L: u8 = 0x26;
    pub const V: u8 = 0x2F;
    pub const N: u8 = 0x31;
    pub const M: u8 = 0x32;
    pub const SPACE: u8 = 0x39;
    pub const UP: u8 = 0x48;
    pub const LEFT: u8 = 0x4B;
    pub const RIGHT: u8 = 0x4D;
    pub const DOWN: u8 = 0x50;
    /// USB keyboards deliver Up and Down with these codes today.
    pub const UP_USB: u8 = 0x67;
    pub const DOWN_USB: u8 = 0x6C;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir { Left, Right, Up, Down }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One master column on the left, the rest stacked on the right (dwm, river).
    MasterStack,
    /// Each new window halves the space the last one had (Hyprland's default).
    Dwindle,
    /// Every window fills the work area; only the focused one shows.
    Monocle,
    /// Rows and columns as square as the count allows.
    Grid,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::MasterStack, Layout::Dwindle, Layout::Monocle, Layout::Grid];
    pub fn next(self) -> Self {
        match self {
            Self::MasterStack => Self::Dwindle,
            Self::Dwindle => Self::Monocle,
            Self::Monocle => Self::Grid,
            Self::Grid => Self::MasterStack,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::MasterStack => "master",
            Self::Dwindle => "dwindle",
            Self::Monocle => "monocle",
            Self::Grid => "grid",
        }
    }
    pub fn index(self) -> u8 {
        match self { Self::MasterStack => 0, Self::Dwindle => 1, Self::Monocle => 2, Self::Grid => 3 }
    }
    pub fn from_index(i: u8) -> Option<Self> { Self::ALL.get(i as usize).copied() }
}

/// What a Super chord asks the desktop to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmAction {
    /// Super pressed and released alone, or Super+D: the launcher.
    Launcher,
    /// Super+Enter: a terminal.
    Terminal,
    /// Super+1..9.
    Workspace(u8),
    /// Super+Shift+1..9: send the focused window there.
    MoveToWorkspace(u8),
    /// Super+Ctrl+Right/Left and Super+Tab / Super+Shift+Tab.
    NextWorkspace,
    PrevWorkspace,
    /// Super+H/J/K/L or arrows.
    Focus(Dir),
    /// Super+Shift+H/J/K/L or arrows: swap places with that neighbour.
    Swap(Dir),
    /// Super+Q.
    Close,
    /// Super+F.
    Fullscreen,
    /// Super+V or Super+Shift+Space.
    ToggleFloating,
    /// Super+Space: the next layout on this workspace.
    CycleLayout,
    /// Super+M: monocle on this workspace, or back to the previous layout.
    ToggleMonocle,
    /// Super+] / Super+[: give the master column more or less room.
    GrowMaster,
    ShrinkMaster,
    /// Super+S: show or hide the scratchpad window.
    Scratchpad,
    /// Super+Shift+S: send the focused window to the scratchpad.
    ToScratchpad,
    /// Super+W: the workspace overview.
    Overview,
    /// Super+Escape: the session lock.
    Lock,
    /// Super+N: the notification history.
    Notifications,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods { pub shift: bool, pub ctrl: bool }

fn dir_of(code: u8) -> Option<Dir> {
    match code {
        code::H | code::LEFT => Some(Dir::Left),
        code::L | code::RIGHT => Some(Dir::Right),
        code::K | code::UP | code::UP_USB => Some(Dir::Up),
        code::J | code::DOWN | code::DOWN_USB => Some(Dir::Down),
        _ => None,
    }
}

/// The default binding table. `None` means the chord does nothing, and the
/// key still never reaches the program: Super belongs to the desktop.
pub fn binding(key: u8, m: Mods) -> Option<WmAction> {
    if (code::DIGIT_1..=code::DIGIT_9).contains(&key) {
        let n = key - code::DIGIT_1;
        return Some(if m.shift { WmAction::MoveToWorkspace(n) } else { WmAction::Workspace(n) });
    }
    if m.ctrl {
        return match key {
            code::RIGHT | code::L => Some(WmAction::NextWorkspace),
            code::LEFT | code::H => Some(WmAction::PrevWorkspace),
            _ => None,
        };
    }
    if let Some(d) = dir_of(key) {
        return Some(if m.shift { WmAction::Swap(d) } else { WmAction::Focus(d) });
    }
    Some(match (key, m.shift) {
        (code::TAB, false) => WmAction::NextWorkspace,
        (code::TAB, true) => WmAction::PrevWorkspace,
        (code::ENTER, _) => WmAction::Terminal,
        (code::D, _) => WmAction::Launcher,
        (code::Q, _) => WmAction::Close,
        (code::F, _) => WmAction::Fullscreen,
        (code::V, _) | (code::SPACE, true) => WmAction::ToggleFloating,
        (code::SPACE, false) => WmAction::CycleLayout,
        (code::M, _) => WmAction::ToggleMonocle,
        (code::BRACKET_RIGHT, _) => WmAction::GrowMaster,
        (code::BRACKET_LEFT, _) => WmAction::ShrinkMaster,
        (code::S, false) => WmAction::Scratchpad,
        (code::S, true) => WmAction::ToScratchpad,
        (code::W, _) => WmAction::Overview,
        (code::N, _) => WmAction::Notifications,
        (code::ESC, _) => WmAction::Lock,
        _ => return None,
    })
}

/// Actions that keep firing while their key is held.
pub fn repeats(a: WmAction) -> bool {
    matches!(a, WmAction::GrowMaster | WmAction::ShrinkMaster | WmAction::Focus(_))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self { Self { x, y, w, h } }
    fn cx(&self) -> i64 { self.x as i64 * 2 + self.w as i64 }
    fn cy(&self) -> i64 { self.y as i64 * 2 + self.h as i64 }
    fn right(&self) -> i64 { self.x as i64 + self.w as i64 }
    fn bottom(&self) -> i64 { self.y as i64 + self.h as i64 }
    /// Shrinks every side by `by`, keeping at least one pixel.
    pub fn inset(self, by: u32) -> Self {
        let bx = by.min(self.w.saturating_sub(1) / 2);
        let byy = by.min(self.h.saturating_sub(1) / 2);
        Self { x: self.x + bx as i32, y: self.y + byy as i32, w: self.w - 2 * bx, h: self.h - 2 * byy }
    }
    fn split_h(self, left_w: u32) -> (Self, Self) {
        let lw = left_w.min(self.w);
        (Self { w: lw, ..self }, Self { x: self.x + lw as i32, w: self.w - lw, ..self })
    }
    fn split_v(self, top_h: u32) -> (Self, Self) {
        let th = top_h.min(self.h);
        (Self { h: th, ..self }, Self { y: self.y + th as i32, h: self.h - th, ..self })
    }
}

/// Per-workspace tiling settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tiling {
    pub layout: Layout,
    /// The layout Super+M returns to.
    pub before_monocle: Layout,
    /// Master column width in percent of the work area, 15..=85.
    pub master_pct: u8,
    /// Space between windows, and between windows and the screen edge.
    pub gap_inner: u32,
    pub gap_outer: u32,
}

pub const MASTER_MIN: u8 = 15;
pub const MASTER_MAX: u8 = 85;
pub const MASTER_STEP: u8 = 5;

impl Tiling {
    pub const DEFAULT: Self = Self { layout: Layout::MasterStack, before_monocle: Layout::MasterStack, master_pct: 55, gap_inner: 6, gap_outer: 8 };
    pub fn grow_master(&mut self, by: i8) -> bool {
        let next = (self.master_pct as i16 + by as i16).clamp(MASTER_MIN as i16, MASTER_MAX as i16) as u8;
        core::mem::replace(&mut self.master_pct, next) != next
    }
    pub fn cycle(&mut self) {
        self.layout = self.layout.next();
        if self.layout != Layout::Monocle { self.before_monocle = self.layout; }
    }
    pub fn toggle_monocle(&mut self) {
        if self.layout == Layout::Monocle { self.layout = self.before_monocle; } else { self.before_monocle = self.layout; self.layout = Layout::Monocle; }
    }
}

/// Places `out.len()` windows in `area` (the work area below the panel).
/// Windows are in tiling order: index 0 is the master.
pub fn tile(t: &Tiling, area: Rect, out: &mut [Rect]) {
    let n = out.len();
    if n == 0 { return; }
    let work = area.inset(t.gap_outer);
    // Cells partition the work area exactly; then each side that faces a
    // neighbour gives up part of the inner gap, so neighbours end up exactly
    // `gap_inner` apart and the outer gap stays exact.
    let g = t.gap_inner;
    let cell = |r: Rect| -> Rect {
        let l = if r.x > work.x { g - g / 2 } else { 0 };
        let rt = if r.right() < work.right() { g / 2 } else { 0 };
        let tp = if r.y > work.y { g - g / 2 } else { 0 };
        let bt = if r.bottom() < work.bottom() { g / 2 } else { 0 };
        let w = r.w.saturating_sub(l + rt).max(1);
        let h = r.h.saturating_sub(tp + bt).max(1);
        Rect::new(r.x + l as i32, r.y + tp as i32, w, h)
    };
    match (t.layout, n) {
        (Layout::Monocle, _) | (_, 1) => { for r in out.iter_mut() { *r = work; } }
        (Layout::MasterStack, _) => {
            let mw = (work.w as u64 * t.master_pct as u64 / 100) as u32;
            let (master, stack) = work.split_h(mw);
            out[0] = cell(master);
            let rows = (n - 1) as u32;
            let mut rest = stack;
            for i in 1..n {
                let left = rows - (i as u32 - 1);
                let (top, below) = rest.split_v(rest.h / left);
                out[i] = cell(top);
                rest = below;
            }
        }
        (Layout::Dwindle, _) => {
            let mut rest = work;
            for i in 0..n {
                if i + 1 == n { out[i] = cell(rest); break; }
                // Split along the longer side, as Hyprland does.
                let (a, b) = if rest.w >= rest.h { rest.split_h(rest.w / 2) } else { rest.split_v(rest.h / 2) };
                out[i] = cell(a);
                rest = b;
            }
        }
        (Layout::Grid, _) => {
            let mut cols = 1u32;
            while cols * cols < n as u32 { cols += 1; }
            let rows = (n as u32).div_ceil(cols);
            let ch = work.h / rows;
            for i in 0..n {
                let r = i as u32 / cols;
                // The last row may be short: its windows share the full width.
                let in_row = if r + 1 == rows { n as u32 - r * cols } else { cols };
                let c = i as u32 - r * cols;
                let cw = work.w / in_row;
                let w = if c + 1 == in_row { work.w - cw * c } else { cw };
                let h = if r + 1 == rows { work.h - ch * r } else { ch };
                out[i] = cell(Rect::new(work.x + (cw * c) as i32, work.y + (ch * r) as i32, w, h));
            }
        }
    }
}

/// The window to move focus to from `from` in direction `dir`: the nearest one
/// whose centre lies that way and which overlaps on the other axis, else the
/// nearest centre that way at all.
pub fn neighbour(rects: &[Rect], from: usize, dir: Dir) -> Option<usize> {
    let a = rects.get(from)?;
    let mut best: Option<(bool, i64, usize)> = None;
    for (i, b) in rects.iter().enumerate() {
        if i == from { continue; }
        let (ahead, along, across, overlap) = match dir {
            Dir::Left => (b.cx() < a.cx(), a.cx() - b.cx(), (a.cy() - b.cy()).abs(), b.y as i64 <= a.bottom() && b.bottom() >= a.y as i64),
            Dir::Right => (b.cx() > a.cx(), b.cx() - a.cx(), (a.cy() - b.cy()).abs(), b.y as i64 <= a.bottom() && b.bottom() >= a.y as i64),
            Dir::Up => (b.cy() < a.cy(), a.cy() - b.cy(), (a.cx() - b.cx()).abs(), b.x as i64 <= a.right() && b.right() >= a.x as i64),
            Dir::Down => (b.cy() > a.cy(), b.cy() - a.cy(), (a.cx() - b.cx()).abs(), b.x as i64 <= a.right() && b.right() >= a.x as i64),
        };
        if !ahead { continue; }
        let score = along + across * 2;
        let key = (!overlap, score, i);
        if best.map_or(true, |(o, s, j)| key < (o, s, j)) { best = Some(key); }
    }
    best.map(|(_, _, i)| i)
}

/// A floating window's default place: centred, `pct` percent of the work area.
pub fn centred(area: Rect, pct: u32) -> Rect {
    let w = (area.w as u64 * pct as u64 / 100) as u32;
    let h = (area.h as u64 * pct as u64 / 100) as u32;
    Rect::new(area.x + ((area.w - w) / 2) as i32, area.y + ((area.h - h) / 2) as i32, w, h)
}

/// Tiling order of frames, by frame id. Swapping changes order only, never
/// the frame table, so frame ids and their slots stay as they were.
pub struct Order<const N: usize> { ids: [u32; N], len: usize }

impl<const N: usize> Order<N> {
    pub const fn new() -> Self { Self { ids: [0; N], len: 0 } }
    pub fn position(&self, id: u32) -> Option<usize> { self.ids[..self.len].iter().position(|&x| x == id) }
    /// Puts a new frame last; a known one keeps its place.
    pub fn insert(&mut self, id: u32) {
        if id == 0 || self.position(id).is_some() { return; }
        if self.len == N { self.ids.copy_within(1.., 0); self.len -= 1; }
        self.ids[self.len] = id;
        self.len += 1;
    }
    pub fn remove(&mut self, id: u32) {
        if let Some(p) = self.position(id) { self.ids.copy_within(p + 1..self.len, p); self.len -= 1; }
    }
    /// Keeps only the ids `live` accepts, in order.
    pub fn retain(&mut self, mut live: impl FnMut(u32) -> bool) {
        let mut n = 0;
        for i in 0..self.len { if live(self.ids[i]) { self.ids[n] = self.ids[i]; n += 1; } }
        self.len = n;
    }
    pub fn swap(&mut self, a: u32, b: u32) -> bool {
        match (self.position(a), self.position(b)) {
            (Some(i), Some(j)) if i != j => { self.ids.swap(i, j); true }
            _ => false,
        }
    }
    /// Sorts `frames` into tiling order; unknown ids are added at the end.
    pub fn sort(&mut self, frames: &mut [u32]) {
        for &f in frames.iter() { self.insert(f); }
        frames.sort_unstable_by_key(|&f| self.position(f).unwrap_or(usize::MAX));
    }
}

/// Recognises Super chords. Super itself and every key pressed while it is
/// held stay with the desktop, including their repeats and releases.
pub struct Chords {
    super_down: u8,
    shift: u8,
    ctrl: u8,
    /// A key went with this Super press, so releasing Super is not a tap.
    used: bool,
    /// Every key pressed with Super that is still held, with its action, so
    /// overlapping chords never let a held key's repeats or release leak.
    captured: [Option<(u8, Option<WmAction>)>; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord { Pass, Consume, Act(WmAction) }

impl Chords {
    pub const fn new() -> Self { Self { super_down: 0, shift: 0, ctrl: 0, used: false, captured: [None; 4] } }
    pub fn super_held(&self) -> bool { self.super_down != 0 }
    pub fn event(&mut self, key: u8, value: u64) -> Chord {
        let bit = |k: u8| -> u8 { if k == code::SUPER_LEFT || k == code::SHIFT_LEFT || k == code::CTRL_LEFT { 1 } else { 2 } };
        match key {
            code::SHIFT_LEFT | code::SHIFT_RIGHT => { if value == 1 { self.shift |= bit(key) } else if value == 0 { self.shift &= !bit(key) } return Chord::Pass; }
            code::CTRL_LEFT | code::CTRL_RIGHT_USB => { if value == 1 { self.ctrl |= bit(key) } else if value == 0 { self.ctrl &= !bit(key) } return Chord::Pass; }
            code::SUPER_LEFT | code::SUPER_RIGHT => {
                match value {
                    1 => { if self.super_down == 0 { self.used = false; } self.super_down |= bit(key); }
                    0 => {
                        self.super_down &= !bit(key);
                        if self.super_down == 0 && !core::mem::replace(&mut self.used, true) && self.captured.iter().all(|c| c.is_none()) {
                            return Chord::Act(WmAction::Launcher);
                        }
                    }
                    _ => {}
                }
                return Chord::Consume;
            }
            _ => {}
        }
        // PS/2 typematic repeats arrive as further presses (value 1), USB ones
        // may arrive as value 2: both are repeats of a captured key.
        if let Some(i) = self.captured.iter().position(|c| matches!(c, Some((k, _)) if *k == key)) {
            let action = self.captured[i].and_then(|c| c.1);
            return match value {
                0 => { self.captured[i] = None; Chord::Consume }
                _ => match action { Some(a) if repeats(a) && self.super_down != 0 => Chord::Act(a), _ => Chord::Consume },
            };
        }
        if self.super_down == 0 || value != 1 { return Chord::Pass; }
        self.used = true;
        let action = binding(key, Mods { shift: self.shift != 0, ctrl: self.ctrl != 0 });
        // Four held chord keys at once is plenty; a fifth replaces the oldest slot.
        let slot = self.captured.iter().position(|c| c.is_none()).unwrap_or(0);
        self.captured[slot] = Some((key, action));
        match action { Some(a) => Chord::Act(a), None => Chord::Consume }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect::new(0, 50, 1280, 750);

    fn no_overlap(rs: &[Rect]) {
        for (i, a) in rs.iter().enumerate() {
            assert!(a.x >= AREA.x && a.y >= AREA.y && a.right() <= AREA.right() && a.bottom() <= AREA.bottom(), "{a:?} leaves the area");
            for b in &rs[i + 1..] {
                let apart = a.right() <= b.x as i64 || b.right() <= a.x as i64 || a.bottom() <= b.y as i64 || b.bottom() <= a.y as i64;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn every_layout_keeps_windows_apart_and_inside() {
        for layout in Layout::ALL {
            if layout == Layout::Monocle { continue; }
            for n in 1..=12 {
                let mut out = [Rect::default(); 12];
                tile(&Tiling { layout, ..Tiling::DEFAULT }, AREA, &mut out[..n]);
                no_overlap(&out[..n]);
            }
        }
    }

    #[test]
    fn master_stack_uses_the_ratio() {
        let mut out = [Rect::default(); 3];
        tile(&Tiling::DEFAULT, AREA, &mut out);
        assert!(out[0].w > out[1].w);
        assert_eq!(out[1].x, out[2].x);
        assert!(out[2].y > out[1].y);
    }
}
