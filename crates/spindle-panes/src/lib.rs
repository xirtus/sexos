// SPDX-License-Identifier: GPL-3.0-only
//! Tabs and split panes for Spindle's terminal window.
//!
//! A window holds [`Tabs`]; each [`Tab`] is a tree of splits whose leaves are panes,
//! named by a [`PaneId`] the window chooses. This crate knows nothing about screens or
//! programs: it only says where each pane goes ([`Tab::layout`]), which pane is next to
//! which ([`Tab::focus_toward`]), where the dividers are for the mouse ([`Tab::divider_at`])
//! and how a split changes when a pane is resized, zoomed or closed.
#![no_std]
extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

pub type PaneId = u32;

/// How a split divides its space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// Side by side: the first pane on the left.
    Across,
    /// One above the other: the first pane on top.
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward { Left, Right, Up, Down }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect { pub x: usize, pub y: usize, pub w: usize, pub h: usize }
impl Rect {
    pub const fn new(x: usize, y: usize, w: usize, h: usize) -> Rect { Rect { x, y, w, h } }
    pub fn contains(&self, x: usize, y: usize) -> bool { x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h }
    fn center(&self) -> (isize, isize) { ((self.x + self.w / 2) as isize, (self.y + self.h / 2) as isize) }
}

/// The smallest share of a split either side keeps, in thousandths.
const MIN_SHARE: u16 = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Pane(PaneId),
    /// `share` is the first side's part of the space, in thousandths.
    Split { split: Split, share: u16, first: Box<Node>, second: Box<Node> },
}

impl Node {
    fn contains(&self, pane: PaneId) -> bool {
        match self { Node::Pane(p) => *p == pane, Node::Split { first, second, .. } => first.contains(pane) || second.contains(pane) }
    }
    fn panes(&self, out: &mut Vec<PaneId>) {
        match self { Node::Pane(p) => out.push(*p), Node::Split { first, second, .. } => { first.panes(out); second.panes(out); } }
    }
    fn layout(&self, area: Rect, gap: usize, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Node::Pane(p) => out.push((*p, area)),
            Node::Split { split, share, first, second } => { let (a, b, _) = divide(area, *split, *share, gap); first.layout(a, gap, out); second.layout(b, gap, out); }
        }
    }
    /// Replaces the leaf `pane` with `with(pane)`. True if found.
    fn replace(&mut self, pane: PaneId, with: &mut Option<Node>) -> bool {
        match self {
            Node::Pane(p) if *p == pane => { if let Some(n) = with.take() { *self = n; } true }
            Node::Pane(_) => false,
            Node::Split { first, second, .. } => first.replace(pane, with) || second.replace(pane, with),
        }
    }
    /// Removes `pane`; its sibling takes its parent's place. Returns false if absent.
    fn remove(&mut self, pane: PaneId) -> bool {
        let Node::Split { first, second, .. } = self else { return false };
        if **first == Node::Pane(pane) { *self = core::mem::replace(&mut **second, Node::Pane(0)); return true; }
        if **second == Node::Pane(pane) { *self = core::mem::replace(&mut **first, Node::Pane(0)); return true; }
        first.remove(pane) || second.remove(pane)
    }
}

/// The two halves of `area` and the divider between them.
fn divide(area: Rect, split: Split, share: u16, gap: usize) -> (Rect, Rect, Rect) {
    match split {
        Split::Across => {
            let room = area.w.saturating_sub(gap);
            let a = room * share as usize / 1000;
            (Rect::new(area.x, area.y, a, area.h), Rect::new(area.x + a + gap, area.y, room - a, area.h), Rect::new(area.x + a, area.y, gap, area.h))
        }
        Split::Down => {
            let room = area.h.saturating_sub(gap);
            let a = room * share as usize / 1000;
            (Rect::new(area.x, area.y, area.w, a), Rect::new(area.x, area.y + a + gap, area.w, room - a), Rect::new(area.x, area.y + a, area.w, gap))
        }
    }
}

/// A divider under the mouse: which split, by the path of first/second turns from the
/// root, and the area that split divides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divider { pub path: Vec<bool>, pub split: Split, pub area: Rect, pub line: Rect }

/// One tab: a tree of panes, the focused one, and whether it fills the tab (zoom).
#[derive(Clone, Debug)]
pub struct Tab {
    root: Node,
    pub focus: PaneId,
    pub zoomed: bool,
}

impl Tab {
    pub fn new(pane: PaneId) -> Tab { Tab { root: Node::Pane(pane), focus: pane, zoomed: false } }
    pub fn panes(&self) -> Vec<PaneId> { let mut v = Vec::new(); self.root.panes(&mut v); v }
    pub fn contains(&self, pane: PaneId) -> bool { self.root.contains(pane) }
    pub fn count(&self) -> usize { self.panes().len() }

    /// Splits `pane`, putting `new` after it (right or below), sharing the space equally.
    /// The new pane gets the focus. False if `pane` is not in this tab.
    pub fn split(&mut self, pane: PaneId, split: Split, new: PaneId) -> bool {
        let mut with = Some(Node::Split { split, share: 500, first: Box::new(Node::Pane(pane)), second: Box::new(Node::Pane(new)) });
        if !self.root.replace(pane, &mut with) { return false; }
        self.focus = new;
        self.zoomed = false;
        true
    }

    /// Removes `pane`. Returns true when the tab has no panes left (it was the last).
    /// The focus moves to the pane that took its place.
    pub fn remove(&mut self, pane: PaneId, area: Rect) -> bool {
        if self.root == Node::Pane(pane) { return true; }
        // The neighbour the focus goes to: the nearest pane in the closed one's place.
        let place = self.rect_of(pane, area, 0);
        if !self.root.remove(pane) { return false; }
        self.zoomed = false;
        if self.focus == pane || !self.contains(self.focus) {
            let (cx, cy) = place.map(|r| r.center()).unwrap_or((0, 0));
            let layout = self.layout(area, 0);
            self.focus = layout.iter().min_by_key(|(_, r)| { let (x, y) = r.center(); (x - cx).abs() + (y - cy).abs() }).map(|(p, _)| *p).unwrap_or(self.focus);
        }
        false
    }

    /// Where each pane goes in `area`, with `gap` pixels between panes. A zoomed tab shows
    /// only its focused pane.
    pub fn layout(&self, area: Rect, gap: usize) -> Vec<(PaneId, Rect)> {
        if self.zoomed { return alloc::vec![(self.focus, area)]; }
        let mut out = Vec::new();
        self.root.layout(area, gap, &mut out);
        out
    }
    pub fn rect_of(&self, pane: PaneId, area: Rect, gap: usize) -> Option<Rect> { self.layout(area, gap).into_iter().find(|(p, _)| *p == pane).map(|(_, r)| r) }
    /// The pane at a point.
    pub fn pane_at(&self, area: Rect, gap: usize, x: usize, y: usize) -> Option<PaneId> { self.layout(area, gap).into_iter().find(|(_, r)| r.contains(x, y)).map(|(p, _)| p) }

    /// Every divider line, for drawing.
    pub fn dividers(&self, area: Rect, gap: usize) -> Vec<Rect> {
        let mut out = Vec::new();
        if self.zoomed { return out; }
        fn walk(n: &Node, area: Rect, gap: usize, out: &mut Vec<Rect>) {
            if let Node::Split { split, share, first, second } = n {
                let (a, b, line) = divide(area, *split, *share, gap);
                out.push(line);
                walk(first, a, gap, out);
                walk(second, b, gap, out);
            }
        }
        walk(&self.root, area, gap, &mut out);
        out
    }

    /// The divider under a point, `slop` pixels either side of the line counting.
    pub fn divider_at(&self, area: Rect, gap: usize, x: usize, y: usize, slop: usize) -> Option<Divider> {
        if self.zoomed { return None; }
        fn walk(n: &Node, area: Rect, gap: usize, x: usize, y: usize, slop: usize, path: &mut Vec<bool>) -> Option<Divider> {
            let Node::Split { split, share, first, second } = n else { return None };
            let (a, b, line) = divide(area, *split, *share, gap);
            let hit = match split {
                Split::Across => y >= line.y && y < line.y + line.h && x + slop >= line.x && x < line.x + line.w + slop,
                Split::Down => x >= line.x && x < line.x + line.w && y + slop >= line.y && y < line.y + line.h + slop,
            };
            if hit { return Some(Divider { path: path.clone(), split: *split, area, line }); }
            path.push(false);
            if let Some(d) = walk(first, a, gap, x, y, slop, path) { return Some(d); }
            path.pop();
            path.push(true);
            let d = walk(second, b, gap, x, y, slop, path);
            path.pop();
            d
        }
        walk(&self.root, area, gap, x, y, slop, &mut Vec::new())
    }

    /// Moves a divider (found by [`Tab::divider_at`]) so that it sits at `x, y`.
    pub fn drag_divider(&mut self, d: &Divider, x: usize, y: usize) {
        let mut n = &mut self.root;
        for turn in &d.path {
            let Node::Split { first, second, .. } = n else { return };
            n = if *turn { second } else { first };
        }
        if let Node::Split { share, .. } = n {
            let (at, len) = match d.split { Split::Across => (x.saturating_sub(d.area.x), d.area.w), Split::Down => (y.saturating_sub(d.area.y), d.area.h) };
            if len == 0 { return; }
            *share = ((at * 1000 / len) as u16).clamp(MIN_SHARE, 1000 - MIN_SHARE);
        }
    }

    /// Grows the focused pane `step` thousandths toward `toward`: moves the nearest
    /// divider on that side of it. False if there is none.
    pub fn resize(&mut self, toward: Toward, step: u16) -> bool {
        let want = match toward { Toward::Left | Toward::Right => Split::Across, _ => Split::Down };
        // The pane must be on the side of the split that grows toward `toward`: growing
        // right moves the divider right of a first child; growing left moves the one left
        // of a second child.
        let grow_first = matches!(toward, Toward::Right | Toward::Down);
        fn walk(n: &mut Node, pane: PaneId, want: Split, grow_first: bool, step: u16) -> Option<bool> {
            let Node::Split { split, share, first, second } = n else { return None };
            let in_first = first.contains(pane);
            if !in_first && !second.contains(pane) { return None; }
            // Deeper splits are nearer the pane.
            let deeper = if in_first { walk(first, pane, want, grow_first, step) } else { walk(second, pane, want, grow_first, step) };
            if deeper == Some(true) { return deeper; }
            if *split == want && in_first == grow_first {
                *share = if in_first { share.saturating_add(step) } else { share.saturating_sub(step) }.clamp(MIN_SHARE, 1000 - MIN_SHARE);
                return Some(true);
            }
            Some(false)
        }
        walk(&mut self.root, self.focus, want, grow_first, step) == Some(true)
    }

    /// Moves the focus to the pane next to it toward `toward`, nearest first. False if none.
    pub fn focus_toward(&mut self, area: Rect, toward: Toward) -> bool {
        let layout = self.layout(area, 0);
        let Some(&(_, from)) = layout.iter().find(|(p, _)| *p == self.focus) else { return false };
        let (fx, fy) = from.center();
        let best = layout.iter().filter(|(p, r)| *p != self.focus && match toward {
            Toward::Left => r.x + r.w <= from.x,
            Toward::Right => r.x >= from.x + from.w,
            Toward::Up => r.y + r.h <= from.y,
            Toward::Down => r.y >= from.y + from.h,
        } && match toward {
            // Overlapping on the other axis.
            Toward::Left | Toward::Right => r.y < from.y + from.h && from.y < r.y + r.h,
            _ => r.x < from.x + from.w && from.x < r.x + r.w,
        }).min_by_key(|(_, r)| { let (x, y) = r.center(); (x - fx).abs() + (y - fy).abs() });
        match best { Some(&(p, _)) => { self.focus = p; self.zoomed = false; true } None => false }
    }

    /// The next or previous pane in reading order.
    pub fn cycle(&mut self, forward: bool) {
        let panes = self.panes();
        let Some(i) = panes.iter().position(|p| *p == self.focus) else { return };
        let n = panes.len();
        self.focus = panes[if forward { (i + 1) % n } else { (i + n - 1) % n }];
    }

    /// Makes every split share its space equally.
    pub fn equalize(&mut self) {
        fn walk(n: &mut Node) { if let Node::Split { share, first, second, .. } = n { *share = 500; walk(first); walk(second); } }
        walk(&mut self.root);
    }
}

/// The tabs of one window.
#[derive(Clone, Debug, Default)]
pub struct Tabs {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Tabs {
    pub fn new(first: PaneId) -> Tabs { Tabs { tabs: alloc::vec![Tab::new(first)], active: 0 } }
    pub fn len(&self) -> usize { self.tabs.len() }
    pub fn is_empty(&self) -> bool { self.tabs.is_empty() }
    pub fn current(&self) -> &Tab { &self.tabs[self.active] }
    pub fn current_mut(&mut self) -> &mut Tab { &mut self.tabs[self.active] }
    pub fn focus(&self) -> PaneId { self.current().focus }
    /// Opens a tab after the current one and switches to it.
    pub fn open(&mut self, pane: PaneId) { self.active = (self.active + 1).min(self.tabs.len()); self.tabs.insert(self.active, Tab::new(pane)); }
    /// Closes the tab at `i`, returning its panes. When it was current, the tab that
    /// took its place (the one to its right, else the new last) becomes current.
    pub fn close(&mut self, i: usize) -> Vec<PaneId> {
        if i >= self.tabs.len() { return Vec::new(); }
        let tab = self.tabs.remove(i);
        if self.active > i || self.active >= self.tabs.len() { self.active = self.active.saturating_sub(1); }
        tab.panes()
    }
    /// Removes `pane` from whichever tab holds it, closing that tab if it is empty.
    pub fn remove_pane(&mut self, pane: PaneId, area: Rect) {
        let Some(i) = self.tabs.iter().position(|t| t.contains(pane)) else { return };
        if self.tabs[i].remove(pane, area) { self.close(i); }
    }
    pub fn select(&mut self, i: usize) { if i < self.tabs.len() { self.active = i; } }
    pub fn next(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n > 0 { self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n }; }
    }
    /// Moves the current tab one place left or right.
    pub fn shift(&mut self, right: bool) {
        let to = if right { self.active + 1 } else { self.active.wrapping_sub(1) };
        if to < self.tabs.len() { self.tabs.swap(self.active, to); self.active = to; }
    }
    pub fn tab_of(&self, pane: PaneId) -> Option<usize> { self.tabs.iter().position(|t| t.contains(pane)) }
    pub fn all_panes(&self) -> Vec<PaneId> { self.tabs.iter().flat_map(|t| t.panes()).collect() }
}

#[cfg(test)]
mod tests;
