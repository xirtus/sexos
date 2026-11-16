// SPDX-License-Identifier: GPL-3.0-only
//! Tabbed window groups, the dwindle split tree and per-workspace layout
//! memory (slice S3), in the style of Hyprland's groups and dwindle layout.
//!
//! Pure data and arithmetic with no IPC, so the host tests in
//! `tests/window_groups.rs` run this exact file. A group is a Shell frame
//! with more than one tab: the helpers here move tabs between tab arrays and
//! step through them; `layouts_glue.rs` applies them to the frames.
//!
//! The dwindle tree is a binary split tree whose leaves are frame ids. A new
//! window splits the focused window's tile in two; the split direction
//! alternates with depth (side by side at the root, then stacked, then side
//! by side again), so the tree never has to store a direction and closing a
//! window keeps the alternation.

use crate::wm::{Layout, Rect};

/// As many leaves as the Shell has frames.
pub const MAX_LEAVES: usize = 24;
const MAX_NODES: usize = MAX_LEAVES * 2;
const NONE: u8 = u8::MAX;

/// How a split divides its tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// Left and right halves.
    Side,
    /// Top and bottom halves.
    Stack,
}

impl Split {
    /// The direction of a split `depth` levels below the root.
    pub fn at_depth(depth: u8) -> Self { if depth % 2 == 0 { Split::Side } else { Split::Stack } }
    pub fn name(self) -> &'static str { match self { Split::Side => "h", Split::Stack => "v" } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Node {
    used: bool,
    /// The frame id for a leaf; 0 for a split.
    leaf: u32,
    a: u8,
    b: u8,
    parent: u8,
}

const FREE: Node = Node { used: false, leaf: 0, a: NONE, b: NONE, parent: NONE };

/// Where a window of the tree goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub rect: Rect,
    /// Splits between the root and this tile.
    pub depth: u8,
    /// The direction of the split that made this tile, if any.
    pub split: Option<Split>,
}

#[derive(Clone, Copy)]
pub struct Dwindle {
    nodes: [Node; MAX_NODES],
    root: u8,
    /// The window inserted last, which the next one splits when focus is
    /// not in the tree.
    last: u32,
    leaves: u8,
}

impl Dwindle {
    pub const fn new() -> Self { Self { nodes: [FREE; MAX_NODES], root: NONE, last: 0, leaves: 0 } }
    pub fn clear(&mut self) { *self = Self::new(); }
    pub fn len(&self) -> usize { self.leaves as usize }
    pub fn is_empty(&self) -> bool { self.root == NONE }
    fn find(&self, id: u32) -> Option<u8> {
        if id == 0 { return None; }
        self.nodes.iter().position(|n| n.used && n.leaf == id).map(|i| i as u8)
    }
    pub fn contains(&self, id: u32) -> bool { self.find(id).is_some() }
    fn alloc(&mut self, node: Node) -> Option<u8> {
        let i = self.nodes.iter().position(|n| !n.used)?;
        self.nodes[i] = Node { used: true, ..node };
        Some(i as u8)
    }
    fn first_leaf(&self) -> Option<u8> {
        self.nodes.iter().position(|n| n.used && n.leaf != 0).map(|i| i as u8)
    }
    /// Puts `old` where `new` was in its parent (or at the root).
    fn replace_child(&mut self, parent: u8, old: u8, new: u8) {
        if parent == NONE { self.root = new; return; }
        let p = &mut self.nodes[parent as usize];
        if p.a == old { p.a = new; } else if p.b == old { p.b = new; }
    }

    /// Adds window `id` by splitting the tile of `at` (when it is in the
    /// tree), else that of the window added last. The old window keeps the
    /// first half (left or top); the new one takes the second.
    pub fn insert(&mut self, id: u32, at: Option<u32>) -> bool {
        if id == 0 || self.contains(id) || self.len() >= MAX_LEAVES { return false; }
        if self.root == NONE {
            let Some(n) = self.alloc(Node { leaf: id, ..FREE }) else { return false };
            self.root = n; self.leaves = 1; self.last = id;
            return true;
        }
        let Some(target) = at.and_then(|a| self.find(a)).or_else(|| self.find(self.last)).or_else(|| self.first_leaf()) else { return false };
        let parent = self.nodes[target as usize].parent;
        let Some(split) = self.alloc(Node { leaf: 0, a: target, b: NONE, parent, ..FREE }) else { return false };
        let Some(leaf) = self.alloc(Node { leaf: id, parent: split, ..FREE }) else {
            self.nodes[split as usize] = FREE;
            return false;
        };
        self.nodes[split as usize].b = leaf;
        self.nodes[target as usize].parent = split;
        self.replace_child(parent, target, split);
        self.leaves += 1;
        self.last = id;
        true
    }

    /// Takes window `id` out; its sibling takes the whole of their tile.
    pub fn remove(&mut self, id: u32) -> bool {
        let Some(n) = self.find(id) else { return false };
        let parent = self.nodes[n as usize].parent;
        self.nodes[n as usize] = FREE;
        self.leaves -= 1;
        if self.last == id { self.last = 0; }
        if parent == NONE { self.root = NONE; return true; }
        let p = self.nodes[parent as usize];
        let sibling = if p.a == n { p.b } else { p.a };
        self.nodes[sibling as usize].parent = p.parent;
        self.replace_child(p.parent, parent, sibling);
        self.nodes[parent as usize] = FREE;
        true
    }

    /// Swaps two windows' tiles.
    pub fn swap(&mut self, a: u32, b: u32) -> bool {
        match (self.find(a), self.find(b)) {
            (Some(i), Some(j)) if i != j => { self.nodes[i as usize].leaf = b; self.nodes[j as usize].leaf = a; true }
            _ => false,
        }
    }

    /// Makes the tree hold exactly `present`: windows that left are taken
    /// out, new ones split the focused window's tile. A tree built from
    /// nothing splits the window added last each time, in `present` order,
    /// which gives the classic dwindle spiral.
    pub fn sync(&mut self, present: &[u32], focus: Option<u32>) -> bool {
        let mut changed = false;
        let mut gone = [0u32; MAX_LEAVES];
        let mut g = 0;
        for n in self.nodes.iter() {
            if n.used && n.leaf != 0 && !present.contains(&n.leaf) && g < MAX_LEAVES { gone[g] = n.leaf; g += 1; }
        }
        for &id in &gone[..g] { changed |= self.remove(id); }
        let fresh = self.is_empty();
        for &id in present {
            if !self.contains(id) { changed |= self.insert(id, if fresh { None } else { focus }); }
        }
        changed
    }

    /// The splits from the root down to window `id`: (depth, bits), where
    /// bit d is set when the path takes the second half at depth d.
    fn path(&self, id: u32) -> Option<(u8, u32)> {
        let mut n = self.find(id)?;
        let mut rev = [false; MAX_LEAVES];
        let mut depth = 0usize;
        loop {
            let parent = self.nodes[n as usize].parent;
            if parent == NONE { break; }
            if depth >= MAX_LEAVES { return None; }
            rev[depth] = self.nodes[parent as usize].b == n;
            depth += 1;
            n = parent;
        }
        let mut bits = 0u32;
        for d in 0..depth { if rev[depth - 1 - d] { bits |= 1 << d; } }
        Some((depth as u8, bits))
    }

    /// Window `id`'s tile in `area`, with `gap_outer` around the edge and
    /// `gap_inner` between neighbours, as `wm::tile` keeps them.
    pub fn place(&self, area: Rect, gap_inner: u32, gap_outer: u32, id: u32) -> Option<Placed> {
        let (depth, bits) = self.path(id)?;
        let work = area.inset(gap_outer);
        let mut r = work;
        for d in 0..depth {
            let second = bits & (1 << d) != 0;
            r = match Split::at_depth(d) {
                Split::Side => {
                    let first = r.w / 2;
                    if second { Rect::new(r.x + first as i32, r.y, r.w - first, r.h) } else { Rect::new(r.x, r.y, first, r.h) }
                }
                Split::Stack => {
                    let first = r.h / 2;
                    if second { Rect::new(r.x, r.y + first as i32, r.w, r.h - first) } else { Rect::new(r.x, r.y, r.w, first) }
                }
            };
        }
        Some(Placed { rect: gap(work, r, gap_inner), depth, split: if depth == 0 { None } else { Some(Split::at_depth(depth - 1)) } })
    }
}

/// Each side of `r` that faces a neighbour gives up part of the inner gap,
/// as in `wm::tile`, so neighbours end up exactly `g` apart.
fn gap(work: Rect, r: Rect, g: u32) -> Rect {
    let right = |q: &Rect| q.x as i64 + q.w as i64;
    let bottom = |q: &Rect| q.y as i64 + q.h as i64;
    let l = if r.x > work.x { g - g / 2 } else { 0 };
    let rt = if right(&r) < right(&work) { g / 2 } else { 0 };
    let tp = if r.y > work.y { g - g / 2 } else { 0 };
    let bt = if bottom(&r) < bottom(&work) { g / 2 } else { 0 };
    Rect::new(r.x + l as i32, r.y + tp as i32, r.w.saturating_sub(l + rt).max(1), r.h.saturating_sub(tp + bt).max(1))
}

// ── Tab groups ──────────────────────────────────────────────────────────────

/// The tab `step` places from `active` in a group of `count`, wrapping.
pub fn cycle(active: u8, count: u8, step: i8) -> u8 {
    if count == 0 { return 0; }
    ((active as i16 + step as i16).rem_euclid(count as i16)) as u8
}

/// Whether tab `ti` of a group of `count` whose shown tab is `active` is in
/// the background: hidden behind the active tab, never drawn, hit or focused.
pub fn hidden(count: u8, active: u8, ti: usize) -> bool { count > 1 && ti != active as usize }

/// The index of `sid` among a frame's tab surfaces `ids` when it is one of
/// the group's background tabs; `None` for the shown tab or a stranger.
pub fn behind(ids: impl IntoIterator<Item = Option<u64>>, count: u8, active: u8, sid: u64) -> Option<usize> {
    let ti = ids.into_iter().position(|t| t == Some(sid))?;
    if hidden(count, active, ti) { Some(ti) } else { None }
}

/// The window Super+G pulls into the focused one's group: the one before
/// it in tiling order, or for the first window the one after it.
pub fn join_target(order: &[u32], focused: u32) -> Option<u32> {
    let i = order.iter().position(|&f| f == focused)?;
    if i > 0 { Some(order[i - 1]) } else { order.get(1).copied() }
}

/// Appends every tab of `src` to `dst` (which holds `count`), keeping their
/// order. Nothing moves when they do not all fit.
pub fn append<T: Copy>(dst: &mut [Option<T>], count: &mut u8, src: &[Option<T>]) -> bool {
    let n = src.iter().flatten().count();
    let have = *count as usize;
    if n == 0 || have + n > dst.len() || dst[have..].iter().any(|t| t.is_some()) { return false; }
    for (k, t) in src.iter().flatten().enumerate() { dst[have + k] = Some(*t); }
    *count = (have + n) as u8;
    true
}

/// Takes tab `idx` out of a group, closing the gap. The active tab stays
/// the same window, or when it was the one taken, the tab that slid into
/// its place (or the new last one). A lone window is not a group: nothing
/// is taken from it.
pub fn take<T: Copy>(tabs: &mut [Option<T>], count: &mut u8, active: &mut u8, idx: u8) -> Option<T> {
    let (c, i) = (*count as usize, idx as usize);
    if c < 2 || i >= c || c > tabs.len() { return None; }
    let tab = tabs[i]?;
    tabs.copy_within(i + 1..c, i);
    tabs[c - 1] = None;
    *count -= 1;
    if *active == idx { *active = idx.min(*count - 1); } else if *active > idx { *active -= 1; }
    Some(tab)
}

/// Surface ids of a group, printed `a,b,c` for receipts.
pub struct Members { ids: [u64; 8], len: usize }

impl Members {
    pub fn new(ids: impl Iterator<Item = u64>) -> Self {
        let mut m = Self { ids: [0; 8], len: 0 };
        for id in ids { if m.len < 8 { m.ids[m.len] = id; m.len += 1; } }
        m
    }
    pub fn as_slice(&self) -> &[u64] { &self.ids[..self.len] }
}

impl core::fmt::Display for Members {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for (i, id) in self.as_slice().iter().enumerate() {
            if i > 0 { f.write_str(",")?; }
            write!(f, "{}", id)?;
        }
        Ok(())
    }
}

// ── Per-workspace layout memory ─────────────────────────────────────────────

/// Each workspace's dwindle tree, whether it was last tiled as dwindle, and
/// the layout Super+T returns to. The layout itself lives in the Shell's
/// per-workspace `wm::Tiling`; this keeps what dwindle and groups add.
pub struct Desk<const N: usize> {
    trees: [Dwindle; N],
    was_dwindle: [bool; N],
    before: [Layout; N],
}

impl<const N: usize> Desk<N> {
    pub const fn new() -> Self { Self { trees: [Dwindle::new(); N], was_dwindle: [false; N], before: [Layout::MasterStack; N] } }
    fn at(scene: usize) -> usize { scene.min(N.saturating_sub(1)) }
    pub fn tree(&mut self, scene: usize) -> &mut Dwindle { &mut self.trees[Self::at(scene)] }
    pub fn tree_ref(&self, scene: usize) -> &Dwindle { &self.trees[Self::at(scene)] }
    /// Called at each tiling of `scene`: true when the workspace has just
    /// come into dwindle from another layout, and its tree starts afresh.
    /// Switching workspaces keeps both the layout and the tree.
    pub fn entered(&mut self, scene: usize, dwindle: bool) -> bool {
        let s = Self::at(scene);
        let fresh = dwindle && !self.was_dwindle[s];
        self.was_dwindle[s] = dwindle;
        if fresh { self.trees[s].clear(); }
        fresh
    }
    /// Super+T: dwindle, or back to the layout the workspace had before.
    pub fn toggle(&mut self, scene: usize, current: Layout) -> Layout {
        let s = Self::at(scene);
        if current == Layout::Dwindle {
            let back = self.before[s];
            if back == Layout::Dwindle { Layout::MasterStack } else { back }
        } else {
            self.before[s] = current;
            Layout::Dwindle
        }
    }
}
