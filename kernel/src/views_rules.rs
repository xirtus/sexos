// SPDX-License-Identifier: GPL-3.0-only
//! The page-table rules of protection views (`views`), free of hardware so host tests
//! (tests/protection_views.rs) run the same code on tables in ordinary memory.
//!
//! A view mirrors the canonical tables: its root and every table on the way down to a
//! span it opens are private copies; everything else, and every page table (the leaves),
//! is the canonical one. Permission bits live in the entry that points at a subtree, so
//! a span is opened by the user bit of its page-directory entry in the view's own copy.
pub const FRAME: u64 = 0x000f_ffff_ffff_f000;
pub const PRESENT: u64 = 1;
pub const USER: u64 = 1 << 2;
pub const HUGE: u64 = 1 << 7;
/// CR3 bit 63: keep this PCID's translations.
pub const NOFLUSH: u64 = 1 << 63;
pub const SPAN: u64 = 2 * 1024 * 1024;

/// Table memory: tables are named by physical address.
pub trait Tables {
    fn read(&self, table: u64, index: usize) -> u64;
    fn write(&mut self, table: u64, index: usize, value: u64);
    /// A new table holding `from`'s entries.
    fn copy(&mut self, from: u64) -> Option<u64>;
}
/// Level 3 indexes the root (PML4), 2 a PDPT, 1 a page directory.
pub fn index(va: u64, level: u32) -> usize { ((va >> (12 + 9 * level)) & 511) as usize }

/// Gives the view rooted at `root` its own tables down to `va`'s span and opens the span
/// there. `made(canonical, private)` hears each table copied. Ok(true) when the view held
/// the span closed before, so cores may still cache the closed entry and must flush.
pub fn open<T: Tables>(t: &mut T, canonical: u64, root: u64, va: u64, made: &mut dyn FnMut(u64, u64)) -> Result<bool, ()> {
    let (mut c, mut v) = (canonical, root);
    for level in [3, 2] {
        let i = index(va, level);
        let ce = t.read(c, i);
        if ce & PRESENT == 0 || ce & HUGE != 0 { return Err(()); }
        let ve = t.read(v, i);
        let next = ce & FRAME;
        v = if ve & PRESENT == 0 || ve & FRAME == next {
            let copy = t.copy(next).ok_or(())?;
            t.write(v, i, copy | (ce & !FRAME));
            made(next, copy);
            copy
        } else { ve & FRAME };
        c = next;
    }
    let i = index(va, 1);
    let ce = t.read(c, i);
    if ce & PRESENT == 0 || ce & HUGE != 0 { return Err(()); }
    let before = t.read(v, i);
    t.write(v, i, ce | USER);
    Ok(before & PRESENT != 0 && before & USER == 0)
}

/// A private table's entry after the canonical one changed: a new canonical entry is
/// taken as is, the view keeps a span it opened, and an entry leading to a private copy
/// keeps pointing there with the canonical flags.
pub fn merged(canonical: u64, private: u64) -> u64 {
    if private & PRESENT == 0 { canonical }
    else if private & FRAME == canonical & FRAME { canonical | (private & USER) }
    else { (private & FRAME) | (canonical & !FRAME) }
}

/// What CR3 gets to enter a view: with PCIDs the view's index is its tag, and a view no
/// invalidation touched since this core last ran it keeps its translations.
pub fn cr3(root: u64, view: usize, pcid: bool, stale: bool) -> u64 {
    if !pcid { return root; }
    root | view as u64 | if stale { 0 } else { NOFLUSH }
}

/// Views a core must flush on their next entry after an invalidation ran under `current`:
/// INVLPG and a CR3 reload reach the current PCID only.
pub fn stale_after_invalidate(current: usize) -> u64 { !(1u64 << current) }
