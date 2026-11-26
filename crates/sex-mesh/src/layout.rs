// SPDX-License-Identifier: GPL-3.0-only
//! Where things sit in Mesh's window, and what a key or a click means.
use crate::canvas::{CELL_H, CELL_W};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab { Processes, Performance, Grants, Hardware }

pub const TABS: [Tab; 4] = [Tab::Processes, Tab::Performance, Tab::Grants, Tab::Hardware];

impl Tab {
    pub fn label(self) -> &'static [u8] {
        match self { Tab::Processes => b"1 Programs", Tab::Performance => b"2 Performance", Tab::Grants => b"3 Grants", Tab::Hardware => b"4 Hardware" }
    }
    /// The word the serial record uses.
    pub fn name(self) -> &'static str {
        match self { Tab::Processes => "programs", Tab::Performance => "performance", Tab::Grants => "grants", Tab::Hardware => "hardware" }
    }
    pub fn index(self) -> usize { TABS.iter().position(|t| *t == self).unwrap_or(0) }
    /// Keys 1 to 4 choose a tab directly.
    pub fn from_key(c: u8) -> Option<Tab> { c.checked_sub(b'1').and_then(|i| TABS.get(i as usize).copied()) }
    pub fn next(self) -> Tab { TABS[(self.index() + 1) % TABS.len()] }
    pub fn prev(self) -> Tab { TABS[(self.index() + TABS.len() - 1) % TABS.len()] }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }
impl Rect {
    pub fn contains(self, x: i32, y: i32) -> bool { x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h }
}

pub const PAD: i32 = 8;
pub const TAB_H: i32 = CELL_H as i32 + 8;
/// Content starts below the tab strip.
pub const TOP: i32 = TAB_H + 6;
/// One line of a list.
pub const ROW_H: i32 = CELL_H as i32 + 2;

/// The tab's button in the strip.
pub fn tab_rect(tab: Tab) -> Rect {
    let mut x = PAD;
    for t in TABS {
        let w = t.label().len() as i32 * CELL_W as i32 + 2 * PAD;
        if t == tab { return Rect { x, y: 2, w, h: TAB_H - 2 }; }
        x += w + 4;
    }
    unreachable!()
}
pub fn tab_at(x: i32, y: i32) -> Option<Tab> { TABS.into_iter().find(|t| tab_rect(*t).contains(x, y)) }

/// The first list row sits below a header line.
pub fn list_top() -> i32 { TOP + ROW_H }
/// How many list rows fit in a window `h` high, above the button bar.
pub fn rows_fit(h: i32) -> usize { ((button_bar(h) - list_top()) / ROW_H).max(0) as usize }
/// The list row (counted from the first one shown) a click at `y` lands on.
pub fn row_at(y: i32, h: i32) -> Option<usize> {
    if y < list_top() || y >= button_bar(h) { return None; }
    let row = ((y - list_top()) / ROW_H) as usize;
    (row < rows_fit(h)).then_some(row)
}
/// The top of the bar at the window's bottom.
pub fn button_bar(h: i32) -> i32 { h - ROW_H - PAD }
pub const END_LABEL: &[u8] = b"End program (Del)";
/// The End button, at the bar's right.
pub fn end_button(w: i32, h: i32) -> Rect {
    let bw = END_LABEL.len() as i32 * CELL_W as i32 + 2 * PAD;
    Rect { x: w - PAD - bw, y: button_bar(h), w: bw, h: ROW_H + 2 }
}

/// What the person asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { Show(Tab), Select(usize), End, Scroll(i32) }

/// Which interactive control the pointer currently hovers over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoverTarget { Tab(Tab), End }

/// Returns the interactive control under `(x, y)` for hover/pressed styling.
pub fn hover(tab: Tab, x: i32, y: i32, w: i32, h: i32) -> Option<HoverTarget> {
    if let Some(t) = tab_at(x, y) { return Some(HoverTarget::Tab(t)); }
    if tab == Tab::Processes && end_button(w, h).contains(x, y) { return Some(HoverTarget::End); }
    None
}

/// A left click at (x, y) in a window `w` by `h`, with `tab` showing.
pub fn click(tab: Tab, x: i32, y: i32, w: i32, h: i32) -> Option<Action> {
    if let Some(t) = tab_at(x, y) { return Some(Action::Show(t)); }
    if tab != Tab::Processes { return None; }
    if end_button(w, h).contains(x, y) { return Some(Action::End); }
    row_at(y, h).map(Action::Select)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_cycling_choose_tabs() {
        assert_eq!(Tab::from_key(b'1'), Some(Tab::Processes));
        assert_eq!(Tab::from_key(b'3'), Some(Tab::Grants));
        assert_eq!(Tab::from_key(b'5'), None);
        assert_eq!(Tab::from_key(b'0'), None);
        assert_eq!(Tab::Hardware.next(), Tab::Processes);
        assert_eq!(Tab::Processes.prev(), Tab::Hardware);
        assert_eq!(Tab::Performance.name(), "performance");
    }

    #[test]
    fn tabs_sit_side_by_side_and_are_hit() {
        for t in TABS {
            let r = tab_rect(t);
            assert_eq!(tab_at(r.x + 1, r.y + 1), Some(t));
            assert_eq!(tab_at(r.x + r.w - 1, r.y + r.h - 1), Some(t));
        }
        let (a, b) = (tab_rect(Tab::Processes), tab_rect(Tab::Performance));
        assert!(a.x + a.w < b.x);
        // The gap between two tabs, and below the strip, are not tabs.
        assert_eq!(tab_at(a.x + a.w + 1, a.y + 2), None);
        assert_eq!(tab_at(a.x + 2, TAB_H + 2), None);
    }

    #[test]
    fn clicks_select_rows_and_end() {
        let (w, h) = (624, 371);
        let first = list_top() + 1;
        assert_eq!(click(Tab::Processes, 40, first, w, h), Some(Action::Select(0)));
        assert_eq!(click(Tab::Processes, 40, first + 2 * ROW_H, w, h), Some(Action::Select(2)));
        let e = end_button(w, h);
        assert!(e.x > 0 && e.y + e.h <= h);
        assert_eq!(click(Tab::Processes, e.x + 2, e.y + 2, w, h), Some(Action::End));
        assert_eq!(hover(Tab::Processes, e.x + 2, e.y + 2, w, h), Some(HoverTarget::End));
        // On another tab the list and button are not there.
        assert_eq!(click(Tab::Grants, e.x + 2, e.y + 2, w, h), None);
        assert_eq!(hover(Tab::Grants, e.x + 2, e.y + 2, w, h), None);
        assert_eq!(click(Tab::Grants, 40, first, w, h), None);
        let t = tab_rect(Tab::Hardware);
        assert_eq!(click(Tab::Grants, t.x + 3, t.y + 3, w, h), Some(Action::Show(Tab::Hardware)));
        assert_eq!(hover(Tab::Grants, t.x + 3, t.y + 3, w, h), Some(HoverTarget::Tab(Tab::Hardware)));
        // Header line and button bar are not rows.
        assert_eq!(row_at(TOP + 1, h), None);
        assert_eq!(row_at(button_bar(h) + 1, h), None);
        assert!(rows_fit(h) >= 10);
        assert_eq!(rows_fit(40), 0);
    }
}
