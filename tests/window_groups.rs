// SPDX-License-Identifier: GPL-3.0-only
//! Host tests for tabbed window groups, the dwindle split tree and
//! per-workspace layout memory (slice S3): the exact
//! `servers/silk-shell/src/layouts.rs` the Shell compiles, with the chords
//! `servers/silk-shell/src/wm.rs` binds for them.
#[path = "../servers/silk-shell/src/wm.rs"]
mod wm;
#[path = "../servers/silk-shell/src/layouts.rs"]
mod layouts;

use layouts::*;
use wm::{binding, code, Chord, Chords, Layout, Mods, Rect, WmAction};

const AREA: Rect = Rect::new(0, 48, 1280, 752);

fn right(r: &Rect) -> i32 { r.x + r.w as i32 }
fn bottom(r: &Rect) -> i32 { r.y + r.h as i32 }
fn apart(a: &Rect, b: &Rect) -> bool { right(a) <= b.x || right(b) <= a.x || bottom(a) <= b.y || bottom(b) <= a.y }
fn inside(r: &Rect) -> bool { r.x >= AREA.x && r.y >= AREA.y && right(r) <= right(&AREA) && bottom(r) <= bottom(&AREA) }

fn rects(t: &Dwindle, ids: &[u32], gi: u32, go: u32) -> Vec<Placed> {
    ids.iter().map(|&id| t.place(AREA, gi, go, id).expect("placed")).collect()
}

#[test]
fn chords_are_free_keys_and_do_not_move_existing_ones() {
    let m = Mods::default();
    let s = Mods { shift: true, ctrl: false };
    assert_eq!(binding(code::G, m), Some(WmAction::Group));
    assert_eq!(binding(code::G, s), Some(WmAction::Ungroup));
    assert_eq!(binding(code::PERIOD, m), Some(WmAction::NextTab));
    assert_eq!(binding(code::COMMA, m), Some(WmAction::PrevTab));
    assert_eq!(binding(code::T, m), Some(WmAction::ToggleDwindle));
    // What was there before stays.
    assert_eq!(binding(code::TAB, m), Some(WmAction::NextWorkspace));
    assert_eq!(binding(code::BRACKET_RIGHT, m), Some(WmAction::GrowMaster));
    assert_eq!(binding(code::BRACKET_LEFT, m), Some(WmAction::ShrinkMaster));
    assert_eq!(binding(code::SPACE, m), Some(WmAction::CycleLayout));
    assert_eq!(binding(code::V, s), Some(WmAction::ToggleFloating));
    assert_eq!(binding(code::DIGIT_1, m), Some(WmAction::Workspace(0)));
    // Group chords do not repeat while held.
    for a in [WmAction::Group, WmAction::Ungroup, WmAction::NextTab, WmAction::PrevTab, WmAction::ToggleDwindle] { assert!(!wm::repeats(a)); }
    let mut c = Chords::new();
    assert_eq!(c.event(code::SUPER_LEFT, 1), Chord::Consume);
    assert_eq!(c.event(code::G, 1), Chord::Act(WmAction::Group));
    assert_eq!(c.event(code::G, 1), Chord::Consume);
    assert_eq!(c.event(code::G, 0), Chord::Consume);
    assert_eq!(c.event(code::SHIFT_LEFT, 1), Chord::Pass);
    assert_eq!(c.event(code::G, 1), Chord::Act(WmAction::Ungroup));
    assert_eq!(c.event(code::G, 0), Chord::Consume);
    assert_eq!(c.event(code::SHIFT_LEFT, 0), Chord::Pass);
    assert_eq!(c.event(code::SUPER_LEFT, 0), Chord::Consume);
}

#[test]
fn a_fresh_tree_is_the_dwindle_spiral_and_splits_alternate() {
    for n in 1..=MAX_LEAVES as u32 {
        let ids: Vec<u32> = (10..10 + n).collect();
        let mut t = Dwindle::new();
        assert!(t.sync(&ids, Some(10)));
        assert_eq!(t.len(), n as usize);
        let p = rects(&t, &ids, 6, 8);
        // Halving 24 times leaves no room; up to 8 windows every tile is real.
        let roomy = n <= 8;
        for (i, a) in p.iter().enumerate() {
            assert!(inside(&a.rect), "{a:?}");
            if roomy { for b in &p[i + 1..] { assert!(apart(&a.rect, &b.rect), "{a:?} {b:?}"); } }
            // Window i sits i splits deep (the last two share a depth), and
            // the split that made it alternates with depth.
            let want = if n == 1 { 0 } else { (i + 1).min(n as usize - 1) };
            assert_eq!(a.depth as usize, want);
            if a.depth > 0 { assert_eq!(a.split, Some(Split::at_depth(a.depth - 1))); }
        }
        // Each next window lies right of (even step) or below (odd step) the one before.
        for i in 0..if roomy { p.len().saturating_sub(1) } else { 0 } {
            let (a, b) = (&p[i].rect, &p[i + 1].rect);
            if i % 2 == 0 { assert!(b.x >= right(a), "step {i}: {a:?} {b:?}"); } else { assert!(b.y >= bottom(a), "step {i}: {a:?} {b:?}"); }
        }
    }
}

#[test]
fn one_window_fills_the_area_inside_the_outer_gap() {
    let mut t = Dwindle::new();
    t.sync(&[7], None);
    let p = t.place(AREA, 6, 8, 7).unwrap();
    assert_eq!(p.rect, AREA.inset(8));
    assert_eq!((p.depth, p.split), (0, None));
}

#[test]
fn a_new_window_splits_the_focused_tile_and_neighbours_keep_the_gap() {
    let mut t = Dwindle::new();
    t.sync(&[1, 2, 3], None); // 1 | (2 / 3)
    // A fourth window opened with 1 focused splits 1's tile, stacked.
    t.sync(&[1, 2, 3, 4], Some(1));
    let p = rects(&t, &[1, 2, 3, 4], 6, 0);
    assert_eq!(p[0].depth, 2);
    assert_eq!(p[3].depth, 2);
    assert_eq!(p[3].split, Some(Split::Stack));
    assert_eq!(p[0].rect.x, p[3].rect.x);
    assert_eq!(p[0].rect.w, p[3].rect.w);
    assert_eq!(p[3].rect.y - bottom(&p[0].rect), 6, "inner gap between stacked tiles");
    assert_eq!(p[1].rect.x - right(&p[0].rect), 6, "inner gap between side tiles");
    // 2 and 3 did not move.
    let mut before = Dwindle::new();
    before.sync(&[1, 2, 3], None);
    assert_eq!(before.place(AREA, 6, 0, 2), t.place(AREA, 6, 0, 2));
    assert_eq!(before.place(AREA, 6, 0, 3), t.place(AREA, 6, 0, 3));
}

#[test]
fn closing_a_window_gives_its_sibling_the_tile_and_keeps_alternation() {
    let mut t = Dwindle::new();
    t.sync(&[1, 2, 3, 4], None); // 1 | (2 / (3 | 4))
    assert!(t.remove(2));
    assert!(!t.remove(2));
    assert_eq!(t.len(), 3);
    // (3 | 4) moved up a level: now stacked, as its new depth says.
    let p = rects(&t, &[1, 3, 4], 0, 0);
    assert_eq!(p[1].depth, 2);
    assert_eq!(p[1].split, Some(Split::Stack));
    assert!(p[2].rect.y >= bottom(&p[1].rect));
    assert_eq!(p[1].rect.x, right(&p[0].rect));
    // Removing down to nothing and building again works.
    assert!(t.sync(&[], None));
    assert!(t.is_empty());
    assert!(t.sync(&[5, 6], None));
    assert_eq!(t.len(), 2);
}

#[test]
fn sync_drops_gone_windows_and_swaps_trade_tiles() {
    let mut t = Dwindle::new();
    t.sync(&[1, 2, 3], None);
    let p2 = t.place(AREA, 0, 0, 2).unwrap();
    let p3 = t.place(AREA, 0, 0, 3).unwrap();
    assert!(t.swap(2, 3));
    assert_eq!(t.place(AREA, 0, 0, 3).unwrap(), p2);
    assert_eq!(t.place(AREA, 0, 0, 2).unwrap(), p3);
    assert!(!t.swap(2, 2));
    assert!(!t.swap(2, 99));
    // Nothing changes when nothing changed.
    assert!(!t.sync(&[1, 2, 3], Some(1)));
    assert!(t.sync(&[1, 3], Some(1)));
    assert!(!t.contains(2));
    assert!(t.place(AREA, 0, 0, 2).is_none());
    // Zero and duplicates are never leaves; the tree holds at most MAX_LEAVES.
    assert!(!t.insert(0, None));
    assert!(!t.insert(1, None));
    let ids: Vec<u32> = (1..=MAX_LEAVES as u32 + 4).collect();
    t.sync(&ids, Some(1));
    assert_eq!(t.len(), MAX_LEAVES);
}

#[test]
fn group_tabs_join_cycle_and_leave() {
    let mut tabs: [Option<u64>; 8] = [Some(301), None, None, None, None, None, None, None];
    let mut count = 1u8;
    assert!(append(&mut tabs, &mut count, &[Some(302), None]));
    assert!(append(&mut tabs, &mut count, &[Some(303)]));
    assert_eq!(count, 3);
    assert_eq!(&tabs[..4], &[Some(301), Some(302), Some(303), None]);
    assert_eq!(Members::new(tabs.iter().flatten().copied()).to_string(), "301,302,303");
    // Cycling visits every tab in order and wraps, both ways.
    let mut active = 0u8;
    let mut seen = vec![];
    for _ in 0..3 { active = cycle(active, count, 1); seen.push(tabs[active as usize].unwrap()); }
    assert_eq!(seen, [302, 303, 301]);
    assert_eq!(cycle(0, 3, -1), 2);
    assert_eq!(cycle(2, 3, 1), 0);
    assert_eq!(cycle(0, 0, 1), 0);
    // A full group refuses a join whole, moving nothing.
    let mut full = [Some(1u64); 8];
    let mut n = 8u8;
    assert!(!append(&mut full, &mut n, &[Some(9)]));
    assert_eq!(n, 8);
    assert!(!append(&mut tabs, &mut count, &[]));
    // The active tab leaves: the next one slides into its place.
    let mut active = 1u8;
    assert_eq!(take(&mut tabs, &mut count, &mut active, 1), Some(302));
    assert_eq!((count, active), (2, 1));
    assert_eq!(&tabs[..3], &[Some(301), Some(303), None]);
    // The last tab leaves while active: the new last one is active.
    tabs[2] = Some(304); count = 3; active = 2;
    assert_eq!(take(&mut tabs, &mut count, &mut active, 2), Some(304));
    assert_eq!((count, active), (2, 1));
    // Another tab leaves: the active window stays active.
    tabs[2] = Some(305); count = 3; active = 2;
    assert_eq!(take(&mut tabs, &mut count, &mut active, 0), Some(301));
    assert_eq!((count, active, tabs[active as usize]), (2, 1, Some(305)));
    // A lone window is no group.
    let mut lone = [Some(1u64), None];
    let (mut c, mut a) = (1u8, 0u8);
    assert_eq!(take(&mut lone, &mut c, &mut a, 0), None);
    assert_eq!(c, 1);
}

#[test]
fn super_g_pulls_the_window_before_or_for_the_first_the_one_after() {
    assert_eq!(join_target(&[4, 5, 6], 6), Some(5));
    assert_eq!(join_target(&[4, 6], 6), Some(4));
    assert_eq!(join_target(&[4, 5, 6], 4), Some(5));
    assert_eq!(join_target(&[4], 4), None);
    assert_eq!(join_target(&[4, 5], 9), None);
}

#[test]
fn each_workspace_remembers_its_layout_and_tree() {
    let mut d: Desk<4> = Desk::new();
    // Workspace 0 goes dwindle: its tree starts afresh once.
    assert_eq!(d.toggle(0, Layout::Grid), Layout::Dwindle);
    assert!(d.entered(0, true));
    d.tree(0).sync(&[1, 2, 3], None);
    assert!(!d.entered(0, true));
    // Workspace 1 tiles with its own layout and tree; workspace 0's is kept.
    assert!(!d.entered(1, false));
    assert!(d.tree(1).is_empty());
    assert!(!d.entered(0, true));
    assert_eq!(d.tree_ref(0).len(), 3);
    // Super+T again: back to the layout before; a later return to dwindle rebuilds.
    assert_eq!(d.toggle(0, Layout::Dwindle), Layout::Grid);
    assert!(!d.entered(0, false));
    assert!(d.entered(0, true));
    assert!(d.tree(0).is_empty());
    // A workspace that was always dwindle toggles to master stack.
    assert_eq!(d.toggle(2, Layout::Dwindle), Layout::MasterStack);
    // Out of range workspaces use the last one rather than panic.
    assert!(d.entered(99, true));
}
