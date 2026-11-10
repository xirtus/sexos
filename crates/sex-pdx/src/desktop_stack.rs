// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! User-space presentation policy shared by Shell and Display. These identities
//! and caller-supplied facts never establish ownership, liveness or capability.

/// Linen is the base desktop window. Native app families follow in immutable
/// BOOT catalog order, then the remaining built-in windows. Allocation/reuse of
/// Display slots must never change this ordering.
pub const BUILTINS: [u64; 8] = [200, 202, 203, 204, 103, 102, 101, 100];
pub const ORDER_LEN: usize = crate::app_catalog::COUNT + BUILTINS.len();
const fn order() -> [u64; ORDER_LEN] {
    let mut result = [0; ORDER_LEN];
    result[0] = BUILTINS[0];
    let mut i = 0;
    while i < crate::app_catalog::COUNT {
        result[1 + i] = crate::app_catalog::INCARNATIONS[i].frame();
        i += 1;
    }
    i = 1;
    while i < BUILTINS.len() {
        result[crate::app_catalog::COUNT + i] = BUILTINS[i];
        i += 1;
    }
    i = 0;
    while i < ORDER_LEN {
        assert!(result[i] != 0 && result[i] != 152);
        let mut j = 0;
        while j < i { assert!(result[i] != result[j]); j += 1; }
        i += 1;
    }
    result
}
pub const ORDER: [u64; ORDER_LEN] = order();
const fn max_id() -> usize {
    let mut max = 204;
    let mut i = 0;
    while i < crate::app_catalog::COUNT {
        let row = &crate::app_catalog::INCARNATIONS[i];
        if row.frame() > max { max = row.frame(); }
        if row.surface > max { max = row.surface; }
        let mut j = 0;
        while j < row.children.len() {
            if row.children[j] > max { max = row.children[j]; }
            j += 1;
        }
        i += 1;
    }
    // A catalog edit cannot silently create an unbounded presentation table.
    assert!(max < 4096 && ORDER_LEN < u16::MAX as usize);
    max as usize
}
const fn ranks() -> [u16; max_id() + 1] {
    let mut result = [u16::MAX; max_id() + 1];
    let mut i = 0;
    while i < ORDER_LEN { result[ORDER[i] as usize] = i as u16; i += 1; }
    i = 0;
    while i < crate::app_catalog::COUNT {
        let row = &crate::app_catalog::INCARNATIONS[i];
        let rank = (1 + i) as u16;
        assert!(result[row.surface as usize] == u16::MAX || result[row.surface as usize] == rank);
        result[row.surface as usize] = rank;
        let mut j = 0;
        while j < row.children.len() {
            assert!(result[row.children[j] as usize] == u16::MAX || result[row.children[j] as usize] == rank);
            result[row.children[j] as usize] = rank; j += 1;
        }
        i += 1;
    }
    result
}
const fn owners() -> [u32; max_id() + 1] {
    let mut result = [u32::MAX; max_id() + 1];
    let mut i = 0;
    while i < BUILTINS.len() {
        // Legacy 100..103 are explicitly owned/mapped standalone descriptors;
        // their creator is checked by admission/held Shell state, not this rank.
        result[BUILTINS[i] as usize] = if BUILTINS[i] >= 100 && BUILTINS[i] <= 103 { 0 } else { 3 };
        i += 1;
    }
    i = 0;
    while i < crate::app_catalog::COUNT {
        let row = &crate::app_catalog::INCARNATIONS[i];
        result[row.frame() as usize] = if row.shell_surface.is_some() { 3 } else { row.target };
        result[row.surface as usize] = row.target;
        let mut j = 0;
        while j < row.children.len() { result[row.children[j] as usize] = row.target; j += 1; }
        i += 1;
    }
    result
}
static RANKS: [u16; max_id() + 1] = ranks();
static OWNERS: [u32; max_id() + 1] = owners();
pub fn rank(surface: u64) -> Option<u16> {
    RANKS.get(usize::try_from(surface).ok()?).copied().filter(|rank| *rank != u16::MAX)
}
/// Callers normalize focus only through their authenticated bound-child state.
/// Unknown surfaces have a deterministic final position, never slot priority.
pub fn key(focus: u64, owner: u32, surface: u64) -> (u8, u16, u64) {
    if owner == 3 && surface == 152 { return (0, 0, surface); }
    if surface == focus { return (1, 0, surface); }
    let rank = rank(surface).filter(|_| OWNERS.get(surface as usize).is_some_and(|expected| *expected == 0 || *expected == owner));
    (2, rank.unwrap_or(u16::MAX), surface)
}
/// Catalog association only. The caller still has to attest the exact live
/// registration and an actual parent frame before using this as a focus alias.
pub fn catalog_frame(surface: u64) -> u64 {
    crate::app_catalog::INCARNATIONS.iter()
        .find(|row| row.surface == surface || row.children.contains(&surface))
        .map_or(surface, |row| row.frame())
}

#[derive(Clone, Copy)]
pub struct FrameFacts { pub active_tab: bool, pub accepts_input: bool }
#[derive(Clone, Copy)]
pub struct InputFacts {
    pub registered: bool, pub alive: bool, pub lifecycle_ready: bool, pub in_scene: bool,
    pub frame: Option<FrameFacts>, pub standalone_owned: bool,
}
/// Visible lifecycle labels alone are insufficient. Facts must come from
/// Shell's held registration/frame state, never an app's requested geometry.
pub fn input_allowed(facts: InputFacts) -> bool {
    facts.registered && facts.alive && facts.lifecycle_ready && facts.in_scene
        && match facts.frame {
            Some(frame) => frame.active_tab && frame.accepts_input,
            None => facts.standalone_owned,
        }
}
