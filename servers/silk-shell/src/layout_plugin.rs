// SPDX-License-Identifier: GPL-3.0-only
//! Wasm layout plugins (slice S4 follow-up), the Shell's bookkeeping: which
//! workspace uses which plugin, the plugin runs, the generation of each ask,
//! and whether an answer may be used.
//!
//! A plugin never runs in the Shell. `layout_plugin_glue.rs` starts one run of
//! the `wasm` program per plugin (`wasm layout @FILE`), sends it the tile count
//! and work area when a workspace that uses it retiles, and keeps tiling with
//! the workspace's previous layout meanwhile. The run's rectangles come back as
//! its own calls, one per rectangle; this module puts them together and
//! [`check`]s the whole answer before any of it is used. An answer for an older
//! generation (the workspace retiled again since) is dropped.
//!
//! Pure data and arithmetic with no IPC, so the host tests in
//! `tests/layout_plugin.rs` run this exact file.

use crate::wm::Rect;

/// Plugin runs at once (each is a program row).
pub const MAX_PLUGINS: usize = 4;
/// The most windows one answer places (the Shell's frames).
pub const MAX_RECTS: usize = 24;

/// Why an answer was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// Not one rectangle for each window.
    Count,
    /// A rectangle leaves the work area.
    Outside(u8),
    /// A rectangle is narrower or lower than a window may be.
    Small(u8),
}
impl Reject {
    pub fn reason(self) -> &'static str { match self { Reject::Count => "count", Reject::Outside(_) => "outside", Reject::Small(_) => "small" } }
    pub fn index(self) -> i32 { match self { Reject::Count => -1, Reject::Outside(i) | Reject::Small(i) => i as i32 } }
}

/// A rectangle as the plugin sent it: relative to the work area, unchecked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raw { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

/// Whether `rects` may tile `count` windows in a `w` by `h` work area: exactly
/// one rectangle per window, each inside the area and at least `min_w` by `min_h`.
pub fn check(rects: &[Raw], count: usize, w: u32, h: u32, min_w: u32, min_h: u32) -> Result<(), Reject> {
    if rects.len() != count || count == 0 || count > MAX_RECTS { return Err(Reject::Count); }
    for (i, r) in rects.iter().enumerate() {
        let (right, bottom) = (r.x as i64 + r.w as i64, r.y as i64 + r.h as i64);
        if r.x < 0 || r.y < 0 || right > w as i64 || bottom > h as i64 { return Err(Reject::Outside(i as u8)); }
        if r.w < min_w || r.h < min_h { return Err(Reject::Small(i as u8)); }
    }
    Ok(())
}

/// A plugin's run, as far as the Shell knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    /// Files is reading `wasm` for it (since this tick).
    Starting { since: u64 },
    Running { target: u32, task: u32 },
    /// Asked to close, as no workspace uses it any more (since this tick).
    Stopping { target: u32, task: u32, since: u64 },
}

/// The ask a plugin run owes an answer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub generation: u32,
    pub scene: u8,
    pub count: u8,
    pub w: u32,
    pub h: u32,
    /// Sent to the run; false while it still has to go.
    pub sent: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Plugin {
    pub key: u64,
    pub run: Run,
    pub pending: Option<Pending>,
    got: [Raw; MAX_RECTS],
    have: u32,
}

/// A workspace's plugin: the layout it falls back to and the last answer used.
#[derive(Clone, Copy, Debug)]
pub struct Use {
    pub key: u64,
    /// `wm::Layout::index` of the layout tiled until an answer is in, and
    /// whenever one is refused. A change of the workspace's layout (a chord, or
    /// `silk layout WS NAME`) ends the plugin's use.
    pub fallback: u8,
    /// The generation of the newest ask for this workspace.
    pub generation: u32,
    /// The answer in use: for how many windows, which area, and where.
    pub applied: Option<(u8, u32, u32)>,
    pub rects: [Raw; MAX_RECTS],
}

/// What tiling should do for a workspace with a plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    /// Use the answer kept for exactly this count and area.
    Apply,
    /// Ask plugin `slot` with this generation; tile with the fallback meanwhile.
    Ask { slot: usize, generation: u32 },
    /// The same question is already out: the fallback until it is answered.
    Wait,
}

/// What a rectangle from a plugin run did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Got {
    /// Kept; more of the answer is to come.
    More,
    /// For an ask this run does not owe (an older generation): dropped.
    Stale { generation: u32 },
    /// Not one of this answer's pieces (another count, an index twice): dropped.
    Bad,
    /// The whole answer, checked: in use for `scene` now, or refused.
    Done { scene: u8, generation: u32, result: Result<(), Reject> },
}

pub struct Plugins<const S: usize> {
    pub slots: [Option<Plugin>; MAX_PLUGINS],
    pub uses: [Option<Use>; S],
    next_generation: u32,
}

impl<const S: usize> Plugins<S> {
    pub const fn new() -> Self { Self { slots: [None; MAX_PLUGINS], uses: [None; S], next_generation: 0 } }

    /// The live slot of plugin `key`. A run already asked to close is not
    /// one: its Close cannot be taken back, so choosing the plugin again
    /// starts a fresh run in another slot while the old one retires.
    pub fn slot_of(&self, key: u64) -> Option<usize> {
        self.slots.iter().position(|p| p.is_some_and(|p| p.key == key && !matches!(p.run, Run::Stopping { .. })))
    }
    /// The plugin whose run is `(target, task)`.
    pub fn slot_of_run(&self, target: u32, task: u32) -> Option<usize> {
        self.slots.iter().position(|p| p.is_some_and(|p| matches!(p.run, Run::Running { target: t, task: k } if t == target && k == task)))
    }
    pub fn used(&self, key: u64) -> bool { self.uses.iter().flatten().any(|u| u.key == key) }

    /// Workspace `scene` uses plugin `key` from now, falling back to layout
    /// `fallback`. `Ok((slot, true))` when the plugin needs a run started,
    /// `Ok((slot, false))` when its run is already there; `Err(())` when every
    /// plugin slot is taken by another plugin in use.
    pub fn set(&mut self, scene: usize, key: u64, fallback: u8, now: u64) -> Result<(usize, bool), ()> {
        if scene >= S { return Err(()); }
        let (slot, fresh) = match self.slot_of(key) {
            Some(i) => (i, false),
            None => {
                let i = self.slots.iter().position(|p| p.is_none()).ok_or(())?;
                self.slots[i] = Some(Plugin { key, run: Run::Starting { since: now }, pending: None, got: [Raw::default(); MAX_RECTS], have: 0 });
                (i, true)
            }
        };
        let old = self.uses[scene].map(|u| u.fallback);
        // Moving from one plugin to another keeps the layout the workspace had first.
        self.uses[scene] = Some(Use { key, fallback: old.unwrap_or(fallback), generation: 0, applied: None, rects: [Raw::default(); MAX_RECTS] });
        Ok((slot, fresh))
    }

    /// Workspace `scene` stops using its plugin. The plugin's slot, if no
    /// workspace uses it any more (its run is to end).
    pub fn clear(&mut self, scene: usize) -> Option<usize> {
        let key = self.uses.get_mut(scene)?.take()?.key;
        if self.used(key) { return None; }
        self.slot_of(key)
    }

    /// Every workspace using plugin `slot` stops (its run ended or would not
    /// start), and the slot is freed. Returns how many workspaces used it.
    pub fn drop_plugin(&mut self, slot: usize) -> usize {
        let Some(p) = self.slots.get_mut(slot).and_then(|s| s.take()) else { return 0 };
        let mut n = 0;
        for u in self.uses.iter_mut() { if u.is_some_and(|u| u.key == p.key) { *u = None; n += 1; } }
        n
    }

    /// Tiling `count` windows in a `w` by `h` work area on workspace `scene`.
    pub fn tile(&mut self, scene: usize, count: usize, w: u32, h: u32) -> Option<Tile> {
        let u = self.uses.get(scene)?.as_ref()?;
        if count == 0 || count > MAX_RECTS { return None; }
        let want = (count as u8, w, h);
        if u.applied == Some(want) { return Some(Tile::Apply); }
        let slot = self.slot_of(u.key)?;
        let p = self.slots[slot].as_mut()?;
        if let Some(q) = p.pending {
            if q.scene as usize == scene && q.generation == u.generation && (q.count, q.w, q.h) == want { return Some(Tile::Wait); }
        }
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let generation = self.next_generation;
        // One ask out per run: a newer one replaces it, and the older answer is stale.
        p.pending = Some(Pending { generation, scene: scene as u8, count: count as u8, w, h, sent: false });
        p.have = 0;
        if let Some(u) = self.uses[scene].as_mut() { u.generation = generation; }
        Some(Tile::Ask { slot, generation })
    }

    /// A rectangle from plugin `slot`'s run, with `min_w` by `min_h` the
    /// smallest window.
    pub fn rect(&mut self, slot: usize, generation: u32, index: usize, count: usize, r: Raw, min_w: u32, min_h: u32) -> Got {
        let Some(p) = self.slots.get_mut(slot).and_then(|s| s.as_mut()) else { return Got::Bad };
        let Some(q) = p.pending.filter(|q| q.generation == generation) else { return Got::Stale { generation } };
        if count != q.count as usize || index >= count || p.have & 1 << index != 0 { return Got::Bad; }
        p.got[index] = r;
        p.have |= 1 << index;
        if p.have != (1u32 << count) - 1 { return Got::More; }
        p.pending = None;
        p.have = 0;
        let rects = p.got;
        let scene = q.scene as usize;
        // The workspace asked again, or left the plugin, since: nothing to use it for.
        let Some(u) = self.uses.get_mut(scene).and_then(|u| u.as_mut()).filter(|u| u.generation == generation) else {
            return Got::Stale { generation };
        };
        let result = check(&rects[..count], count, q.w, q.h, min_w, min_h);
        if result.is_ok() {
            u.rects = rects;
            u.applied = Some((q.count, q.w, q.h));
        }
        Got::Done { scene: q.scene, generation, result }
    }

    /// Plugin `slot`'s run could not answer generation `generation`: the
    /// workspace it was for, when that ask is still the newest.
    pub fn failed(&mut self, slot: usize, generation: u32) -> Option<u8> {
        let p = self.slots.get_mut(slot)?.as_mut()?;
        let q = p.pending.filter(|q| q.generation == generation)?;
        p.pending = None;
        p.have = 0;
        let u = self.uses.get(q.scene as usize)?.as_ref()?;
        (u.generation == generation).then_some(q.scene)
    }

    /// Workspace `scene`'s answer in use, relative to the work area.
    pub fn applied(&self, scene: usize) -> Option<&[Raw]> {
        let u = self.uses.get(scene)?.as_ref()?;
        let (n, _, _) = u.applied?;
        Some(&u.rects[..n as usize])
    }
}

/// `r`, relative to the work area, on the screen.
pub fn placed(area: Rect, r: Raw) -> Rect { Rect::new(area.x + r.x, area.y + r.y, r.w, r.h) }
