// SPDX-License-Identifier: GPL-3.0-only
//! Host test for Wasm layout plugins (slice S4 follow-up): the wire in
//! crates/sex-pdx/src/wasm_layout.rs and the Shell's bookkeeping in
//! servers/silk-shell/src/layout_plugin.rs (which workspace uses which plugin,
//! generations, stale answers, the checks an answer must pass).
#[path = "../servers/silk-shell/src/wm.rs"]
mod wm;
#[path = "../servers/silk-shell/src/layout_plugin.rs"]
mod layout_plugin;
#[path = "../crates/sex-pdx/src/wasm_layout.rs"]
mod wasm_layout;

use layout_plugin::*;
use wasm_layout as wire;
use wm::Rect;

const W: u32 = 1280;
const H: u32 = 752;
const MIN: (u32, u32) = (120, 80);

fn raw(x: i32, y: i32, w: u32, h: u32) -> Raw { Raw { x, y, w, h } }

/// The built-in spiral's answer for three windows in 1280x752.
fn spiral3() -> [Raw; 3] { [raw(4, 4, 632, 744), raw(644, 4, 632, 368), raw(644, 380, 632, 368)] }

fn answer(p: &mut Plugins<9>, slot: usize, generation: u32, rects: &[Raw]) -> Got {
    let mut last = Got::More;
    for (i, r) in rects.iter().enumerate() { last = p.rect(slot, generation, i, rects.len(), *r, MIN.0, MIN.1); }
    last
}

#[test]
fn the_wire_round_trips_and_refuses_nonsense() {
    let a = wire::Ask { generation: 7, count: 3, w: W, h: H };
    assert_eq!(wire::Ask::decode(a.wire()), Some(a));
    assert_eq!(wire::Ask::decode([7, 0, a.wire()[2]]), None, "no windows");
    assert_eq!(wire::Ask::decode([7, 25, a.wire()[2]]), None, "more than the Shell has");
    assert_eq!(wire::Ask::decode([7, 3, 0]), None, "no area");
    assert_eq!(wire::Ask::decode([1 << 32, 3, a.wire()[2]]), None, "generation past 32 bits");
    let m = wire::RectMsg { generation: u32::MAX, index: 2, count: 3, rect: wire::Rect { x: -5, y: 9, w: 600, h: u32::MAX } };
    assert_eq!(wire::RectMsg::decode(m.wire()), Some(m));
    assert_eq!(wire::RectMsg::decode(wire::RectMsg { index: 3, ..m }.wire()), None, "index past count");
    assert_eq!(wire::RectMsg::decode(wire::RectMsg { count: 0, index: 0, ..m }.wire()), None);
    assert_eq!(wire::RectMsg::decode(wire::RectMsg { count: 25, index: 0, ..m }.wire()), None);
    for f in wire::Fail::ALL { assert_eq!(wire::Fail::decode(f.wire(9)), Some((9, f))); }
    assert_eq!(wire::Fail::decode([9, 0, 0]), None);
    assert_eq!(wire::Fail::decode([9, 6, 0]), None);
    assert_eq!(wire::Fail::Fuel.name(), "fuel");
    // Opcodes no other wire to the Shell uses.
    assert_eq!((wire::OP_LAYOUT_ASK, wire::OP_LAYOUT_RECT, wire::OP_LAYOUT_FAIL), (0x310, 0x311, 0x312));
}

#[test]
fn answers_are_checked_whole() {
    assert_eq!(check(&spiral3(), 3, W, H, MIN.0, MIN.1), Ok(()));
    // One rectangle per window, no more, no fewer.
    assert_eq!(check(&spiral3()[..2], 3, W, H, MIN.0, MIN.1), Err(Reject::Count));
    assert_eq!(check(&[], 0, W, H, MIN.0, MIN.1), Err(Reject::Count));
    // Inside the work area, on every side.
    let mut r = spiral3();
    r[1].x = W as i32 - 100;
    assert_eq!(check(&r, 3, W, H, MIN.0, MIN.1), Err(Reject::Outside(1)));
    r = spiral3(); r[0].x = -1;
    assert_eq!(check(&r, 3, W, H, MIN.0, MIN.1), Err(Reject::Outside(0)));
    r = spiral3(); r[2].h = H;
    assert_eq!(check(&r, 3, W, H, MIN.0, MIN.1), Err(Reject::Outside(2)));
    r = spiral3(); r[2].w = u32::MAX;
    assert_eq!(check(&r, 3, W, H, MIN.0, MIN.1), Err(Reject::Outside(2)));
    // At least the smallest window.
    r = spiral3(); r[1].h = 79;
    assert_eq!(check(&r, 3, W, H, MIN.0, MIN.1), Err(Reject::Small(1)));
    r = spiral3(); r[0].w = 0;
    assert_eq!(check(&r, 3, W, H, MIN.0, MIN.1), Err(Reject::Small(0)));
    assert_eq!(Reject::Small(1).reason(), "small");
    assert_eq!(Reject::Outside(2).index(), 2);
    // Placed on the screen: the work area starts below the bar.
    assert_eq!(placed(Rect::new(0, 48, W, H), raw(644, 4, 632, 368)), Rect::new(644, 52, 632, 368));
}

#[test]
fn a_workspace_asks_then_applies_the_answer_for_exactly_that_tiling() {
    let mut p = Plugins::<9>::new();
    let key = 0xABC;
    assert_eq!(p.set(0, key, 0, 100), Ok((0, true)));
    // Until the run is there and answers, the previous layout tiles; one ask is out.
    let Some(Tile::Ask { slot: 0, generation: g1 }) = p.tile(0, 3, W, H) else { panic!() };
    assert_eq!(p.tile(0, 3, W, H), Some(Tile::Wait), "the same question is not asked twice");
    assert!(p.applied(0).is_none());
    p.slots[0].as_mut().unwrap().run = Run::Running { target: 70, task: 5 };
    assert_eq!(p.slot_of_run(70, 5), Some(0));
    assert_eq!(p.slot_of_run(70, 6), None, "another task of that row is not the plugin's run");
    let r = spiral3();
    assert_eq!(p.rect(0, g1, 0, 3, r[0], MIN.0, MIN.1), Got::More);
    assert_eq!(p.rect(0, g1, 0, 3, r[0], MIN.0, MIN.1), Got::Bad, "a piece twice");
    assert_eq!(p.rect(0, g1, 1, 2, r[1], MIN.0, MIN.1), Got::Bad, "another count");
    assert_eq!(p.rect(0, g1, 1, 3, r[1], MIN.0, MIN.1), Got::More);
    assert_eq!(p.rect(0, g1, 2, 3, r[2], MIN.0, MIN.1), Got::Done { scene: 0, generation: g1, result: Ok(()) });
    assert_eq!(p.applied(0), Some(&r[..]));
    assert_eq!(p.tile(0, 3, W, H), Some(Tile::Apply));
    // A fourth window: a new question, while the old answer stays (the glue
    // tiles with the fallback until the new one is in).
    let Some(Tile::Ask { generation: g2, .. }) = p.tile(0, 4, W, H) else { panic!() };
    assert!(g2 > g1);
    assert_eq!(p.tile(0, 3, W, H), Some(Tile::Apply), "back to three: the kept answer fits again");
}

#[test]
fn stale_and_refused_answers_change_nothing() {
    let mut p = Plugins::<9>::new();
    p.set(1, 7, 1, 0).unwrap();
    p.slots[0].as_mut().unwrap().run = Run::Running { target: 70, task: 5 };
    let Some(Tile::Ask { generation: g1, .. }) = p.tile(1, 3, W, H) else { panic!() };
    // Another retile before the answer: the first answer is stale.
    let Some(Tile::Ask { generation: g2, .. }) = p.tile(1, 2, W, H) else { panic!() };
    assert_eq!(answer(&mut p, 0, g1, &spiral3()), Got::Stale { generation: g1 });
    assert!(p.applied(1).is_none());
    // A refused answer (outside the area) is not kept, and the next tiling asks again.
    let outside = [raw(W as i32 + 100, 0, 300, 200), raw(W as i32 + 100, 0, 300, 200)];
    assert_eq!(answer(&mut p, 0, g2, &outside), Got::Done { scene: 1, generation: g2, result: Err(Reject::Outside(0)) });
    assert!(p.applied(1).is_none());
    let Some(Tile::Ask { generation: g3, .. }) = p.tile(1, 2, W, H) else { panic!() };
    // The run could not answer (fuel): the workspace keeps its fallback.
    assert_eq!(p.failed(0, g3), Some(1));
    assert_eq!(p.failed(0, g3), None, "only once");
    assert!(p.applied(1).is_none());
    // A failure for an older ask is stale.
    let Some(Tile::Ask { generation: g4, .. }) = p.tile(1, 2, W, H) else { panic!() };
    assert_eq!(p.failed(0, g3), None);
    let ok = [raw(0, 0, 640, H), raw(640, 0, 640, H)];
    assert_eq!(answer(&mut p, 0, g4, &ok), Got::Done { scene: 1, generation: g4, result: Ok(()) });
    // An answer for a workspace that left the plugin meanwhile is stale too.
    let Some(Tile::Ask { generation: g5, .. }) = p.tile(1, 3, W, H) else { panic!() };
    assert_eq!(p.clear(1), Some(0), "nothing else uses it: its run is to end");
    assert_eq!(answer(&mut p, 0, g5, &spiral3()), Got::Stale { generation: g5 });
    assert!(p.applied(1).is_none());
    assert_eq!(p.tile(1, 3, W, H), None, "no plugin: the workspace's own layout");
}

#[test]
fn one_run_per_plugin_shared_by_workspaces_and_dropped_when_unused() {
    let mut p = Plugins::<9>::new();
    assert_eq!(p.set(0, 11, 0, 0), Ok((0, true)));
    assert_eq!(p.set(4, 11, 3, 0), Ok((0, false)), "the same plugin reuses its run");
    assert_eq!(p.set(2, 22, 0, 0), Ok((1, true)));
    assert_eq!(p.clear(0), None, "workspace 5 still uses plugin 11");
    assert_eq!(p.clear(4), Some(0));
    // A run asked to close comes back into use if a workspace names it again.
    p.slots[0].as_mut().unwrap().run = Run::Stopping { target: 70, task: 5, since: 9 };
    assert_eq!(p.set(3, 11, 2, 0), Ok((0, false)));
    assert_eq!(p.slots[0].unwrap().run, Run::Running { target: 70, task: 5 });
    // Its run ended (a crash): every workspace using it goes back to its own layout.
    p.set(5, 11, 0, 0).unwrap();
    assert_eq!(p.drop_plugin(0), 2);
    assert!(p.uses[3].is_none() && p.uses[5].is_none());
    assert!(p.uses[2].is_some(), "another plugin's workspace stays");
    // Four plugins at most.
    p.set(0, 33, 0, 0).unwrap();
    p.set(1, 44, 0, 0).unwrap();
    p.set(6, 55, 0, 0).unwrap();
    assert_eq!(p.set(7, 66, 0, 0), Err(()));
    assert_eq!(p.set(9, 33, 0, 0), Err(()), "no such workspace");
    // Switching plugins keeps the layout the workspace had before any plugin.
    p.set(8, 33, 2, 0).unwrap();
    p.set(8, 44, 0, 0).unwrap();
    assert_eq!(p.uses[8].unwrap().fallback, 2);
}
