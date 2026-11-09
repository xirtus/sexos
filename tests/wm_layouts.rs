//! Host tests for the desktop's window manager policy: the exact
//! `servers/silk-shell/src/wm.rs` the shell compiles.
#[path = "../servers/silk-shell/src/wm.rs"]
mod wm;
use wm::*;

const AREA: Rect = Rect::new(0, 50, 1280, 750);

fn apart(a: &Rect, b: &Rect) -> bool {
    a.x + a.w as i32 <= b.x || b.x + b.w as i32 <= a.x || a.y + a.h as i32 <= b.y || b.y + b.h as i32 <= a.y
}
fn inside(r: &Rect, area: &Rect) -> bool {
    r.x >= area.x && r.y >= area.y && r.x + r.w as i32 <= area.x + area.w as i32 && r.y + r.h as i32 <= area.y + area.h as i32
}

#[test]
fn tiled_windows_never_overlap_and_keep_the_gaps() {
    for layout in [Layout::MasterStack, Layout::Dwindle, Layout::Grid] {
        for n in 1..=16 {
            let t = Tiling { layout, ..Tiling::DEFAULT };
            let mut out = [Rect::default(); 16];
            tile(&t, AREA, &mut out[..n]);
            let work = AREA.inset(t.gap_outer);
            for (i, a) in out[..n].iter().enumerate() {
                assert!(inside(a, &work), "{layout:?} n={n} {a:?} leaves the work area");
                assert!(a.w >= 1 && a.h >= 1);
                for b in &out[i + 1..n] {
                    assert!(apart(a, b), "{layout:?} n={n}: {a:?} overlaps {b:?}");
                }
            }
            // Area is used: the windows plus gaps cover most of the work area.
            let used: u64 = out[..n].iter().map(|r| r.w as u64 * r.h as u64).sum();
            assert!(used * 100 / (work.w as u64 * work.h as u64) >= 85, "{layout:?} n={n} wastes space");
        }
    }
}

#[test]
fn one_window_and_monocle_fill_the_work_area() {
    let work = AREA.inset(Tiling::DEFAULT.gap_outer);
    let mut one = [Rect::default(); 1];
    tile(&Tiling::DEFAULT, AREA, &mut one);
    assert_eq!(one[0], work);
    let mut many = [Rect::default(); 4];
    tile(&Tiling { layout: Layout::Monocle, ..Tiling::DEFAULT }, AREA, &mut many);
    assert!(many.iter().all(|r| *r == work));
}

#[test]
fn master_stack_follows_the_ratio_and_neighbours_are_one_gap_apart() {
    let mut t = Tiling::DEFAULT;
    let mut out = [Rect::default(); 3];
    tile(&t, AREA, &mut out);
    assert_eq!(out[1].x - (out[0].x + out[0].w as i32), t.gap_inner as i32);
    assert_eq!(out[2].y - (out[1].y + out[1].h as i32), t.gap_inner as i32);
    let before = out[0].w;
    assert!(t.grow_master(MASTER_STEP as i8));
    tile(&t, AREA, &mut out);
    assert!(out[0].w > before);
    for _ in 0..40 { t.grow_master(MASTER_STEP as i8); }
    assert_eq!(t.master_pct, MASTER_MAX);
    assert!(!t.grow_master(MASTER_STEP as i8));
}

#[test]
fn dwindle_halves_along_the_longer_side() {
    let mut out = [Rect::default(); 3];
    tile(&Tiling { layout: Layout::Dwindle, ..Tiling::DEFAULT }, AREA, &mut out);
    // Wide screen: first split is left/right, then the right half splits top/bottom.
    assert!(out[1].x > out[0].x);
    assert_eq!(out[1].x, out[2].x);
    assert!(out[2].y > out[1].y);
}

#[test]
fn focus_moves_to_the_window_in_that_direction() {
    let mut out = [Rect::default(); 3];
    tile(&Tiling::DEFAULT, AREA, &mut out);
    // 0 = master (left), 1 = top right, 2 = bottom right.
    assert_eq!(neighbour(&out, 0, Dir::Right), Some(1));
    assert_eq!(neighbour(&out, 1, Dir::Down), Some(2));
    assert_eq!(neighbour(&out, 2, Dir::Up), Some(1));
    assert_eq!(neighbour(&out, 2, Dir::Left), Some(0));
    assert_eq!(neighbour(&out, 0, Dir::Left), None);
    assert_eq!(neighbour(&out, 1, Dir::Up), None);
}

#[test]
fn layouts_cycle_and_monocle_toggles_back() {
    let mut t = Tiling::DEFAULT;
    t.cycle();
    assert_eq!(t.layout, Layout::Dwindle);
    t.toggle_monocle();
    assert_eq!(t.layout, Layout::Monocle);
    t.toggle_monocle();
    assert_eq!(t.layout, Layout::Dwindle);
    for _ in 0..4 { t.cycle(); }
    assert_eq!(t.layout, Layout::Dwindle);
}

#[test]
fn tiling_order_swaps_without_touching_ids() {
    let mut o: Order<4> = Order::new();
    let mut frames = [12, 10, 11];
    o.sort(&mut frames);
    assert_eq!(frames, [12, 10, 11]);
    assert!(o.swap(12, 11));
    let mut frames = [10, 11, 12];
    o.sort(&mut frames);
    assert_eq!(frames, [11, 10, 12]);
    o.retain(|id| id != 10);
    let mut frames = [12, 11, 13];
    o.sort(&mut frames);
    assert_eq!(frames, [11, 12, 13]);
}

#[test]
fn super_chords_belong_to_the_desktop() {
    let mut c = Chords::new();
    // Plain typing passes through.
    assert_eq!(c.event(code::Q, 1), Chord::Pass);
    assert_eq!(c.event(code::Q, 0), Chord::Pass);
    // Super+3 switches; its repeat and release never reach a program.
    assert_eq!(c.event(code::SUPER_LEFT, 1), Chord::Consume);
    assert_eq!(c.event(0x04, 1), Chord::Act(WmAction::Workspace(2)));
    assert_eq!(c.event(0x04, 2), Chord::Consume);
    assert_eq!(c.event(code::SUPER_LEFT, 0), Chord::Consume);
    assert_eq!(c.event(0x04, 0), Chord::Consume);
    // Super+Shift+2 moves the window; Shift itself still reaches programs.
    assert_eq!(c.event(code::SHIFT_LEFT, 1), Chord::Pass);
    assert_eq!(c.event(code::SUPER_RIGHT, 1), Chord::Consume);
    assert_eq!(c.event(0x03, 1), Chord::Act(WmAction::MoveToWorkspace(1)));
    assert_eq!(c.event(0x03, 0), Chord::Consume);
    assert_eq!(c.event(code::H, 1), Chord::Act(WmAction::Swap(Dir::Left)));
    assert_eq!(c.event(code::H, 0), Chord::Consume);
    assert_eq!(c.event(code::SUPER_RIGHT, 0), Chord::Consume);
    assert_eq!(c.event(code::SHIFT_LEFT, 0), Chord::Pass);
    // Super alone opens the launcher.
    assert_eq!(c.event(code::SUPER_LEFT, 1), Chord::Consume);
    assert_eq!(c.event(code::SUPER_LEFT, 0), Chord::Act(WmAction::Launcher));
    // Focus repeats while held; unknown chords are swallowed.
    assert_eq!(c.event(code::SUPER_LEFT, 1), Chord::Consume);
    assert_eq!(c.event(code::L, 1), Chord::Act(WmAction::Focus(Dir::Right)));
    assert_eq!(c.event(code::L, 2), Chord::Act(WmAction::Focus(Dir::Right)));
    assert_eq!(c.event(code::L, 0), Chord::Consume);
    assert_eq!(c.event(0x2C, 1), Chord::Consume);
    assert_eq!(c.event(0x2C, 0), Chord::Consume);
    assert_eq!(c.event(code::SUPER_LEFT, 0), Chord::Consume);
    // Ctrl+arrows with Super walk workspaces, KDE style.
    assert_eq!(c.event(code::CTRL_LEFT, 1), Chord::Pass);
    assert_eq!(c.event(code::SUPER_LEFT, 1), Chord::Consume);
    assert_eq!(c.event(code::RIGHT, 1), Chord::Act(WmAction::NextWorkspace));
    assert_eq!(c.event(code::RIGHT, 0), Chord::Consume);
    assert_eq!(c.event(code::SUPER_LEFT, 0), Chord::Consume);
    assert_eq!(c.event(code::CTRL_LEFT, 0), Chord::Pass);
}

#[test]
fn bindings_cover_the_hyprland_and_kde_basics() {
    let m = Mods::default();
    let s = Mods { shift: true, ctrl: false };
    assert_eq!(binding(code::Q, m), Some(WmAction::Close));
    assert_eq!(binding(code::F, m), Some(WmAction::Fullscreen));
    assert_eq!(binding(code::V, m), Some(WmAction::ToggleFloating));
    assert_eq!(binding(code::SPACE, s), Some(WmAction::ToggleFloating));
    assert_eq!(binding(code::SPACE, m), Some(WmAction::CycleLayout));
    assert_eq!(binding(code::ENTER, m), Some(WmAction::Terminal));
    assert_eq!(binding(code::S, m), Some(WmAction::Scratchpad));
    assert_eq!(binding(code::S, s), Some(WmAction::ToScratchpad));
    assert_eq!(binding(code::DIGIT_9, m), Some(WmAction::Workspace(8)));
    assert_eq!(binding(code::DOWN_USB, m), Some(WmAction::Focus(Dir::Down)));
}

#[test]
fn overlapping_chords_and_ps2_repeats_stay_with_the_desktop() {
    let mut c = Chords::new();
    assert_eq!(c.event(code::SUPER_LEFT, 1), Chord::Consume);
    assert_eq!(c.event(code::L, 1), Chord::Act(WmAction::Focus(Dir::Right)));
    // PS/2 typematic repeat: another make byte, value 1.
    assert_eq!(c.event(code::L, 1), Chord::Act(WmAction::Focus(Dir::Right)));
    assert_eq!(c.event(code::H, 1), Chord::Act(WmAction::Focus(Dir::Left)));
    assert_eq!(c.event(code::H, 0), Chord::Consume);
    assert_eq!(c.event(code::SUPER_LEFT, 0), Chord::Consume);
    // L still held after Super went up: its repeats and release never reach a program.
    assert_eq!(c.event(code::L, 1), Chord::Consume);
    assert_eq!(c.event(code::L, 2), Chord::Consume);
    assert_eq!(c.event(code::L, 0), Chord::Consume);
    assert_eq!(c.event(code::L, 1), Chord::Pass);
    // A held non-repeating chord does its action once.
    assert_eq!(c.event(code::L, 0), Chord::Pass);
    assert_eq!(c.event(code::SUPER_LEFT, 1), Chord::Consume);
    assert_eq!(c.event(code::Q, 1), Chord::Act(WmAction::Close));
    assert_eq!(c.event(code::Q, 1), Chord::Consume);
    assert_eq!(c.event(code::Q, 0), Chord::Consume);
    assert_eq!(c.event(code::SUPER_LEFT, 0), Chord::Consume);
}
