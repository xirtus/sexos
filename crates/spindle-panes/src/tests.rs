// SPDX-License-Identifier: GPL-3.0-only
extern crate std;
use super::*;

const AREA: Rect = Rect::new(0, 0, 1000, 600);

#[test]
fn split_across_then_down_lays_out_three_panes() {
    let mut t = Tab::new(1);
    assert!(t.split(1, Split::Across, 2));
    assert_eq!(t.focus, 2);
    assert!(t.split(2, Split::Down, 3));
    let l = t.layout(AREA, 0);
    assert_eq!(l, alloc::vec![(1, Rect::new(0, 0, 500, 600)), (2, Rect::new(500, 0, 500, 300)), (3, Rect::new(500, 300, 500, 300))]);
    assert_eq!(t.dividers(AREA, 0).len(), 2);
}

#[test]
fn gap_is_left_between_panes() {
    let mut t = Tab::new(1);
    t.split(1, Split::Across, 2);
    let l = t.layout(Rect::new(0, 0, 1002, 600), 2);
    assert_eq!(l[0].1, Rect::new(0, 0, 500, 600));
    assert_eq!(l[1].1, Rect::new(502, 0, 500, 600));
}

#[test]
fn focus_moves_by_direction() {
    let mut t = Tab::new(1);
    t.split(1, Split::Across, 2);
    t.split(2, Split::Down, 3);
    assert!(t.focus_toward(AREA, Toward::Up));
    assert_eq!(t.focus, 2);
    assert!(t.focus_toward(AREA, Toward::Left));
    assert_eq!(t.focus, 1);
    assert!(!t.focus_toward(AREA, Toward::Left));
    assert!(t.focus_toward(AREA, Toward::Right));
    assert!(t.focus == 2 || t.focus == 3);
}

#[test]
fn closing_a_pane_gives_its_space_to_the_sibling() {
    let mut t = Tab::new(1);
    t.split(1, Split::Across, 2);
    t.split(2, Split::Down, 3);
    assert!(!t.remove(2, AREA));
    assert_eq!(t.focus, 3);
    assert_eq!(t.layout(AREA, 0), alloc::vec![(1, Rect::new(0, 0, 500, 600)), (3, Rect::new(500, 0, 500, 600))]);
    assert!(!t.remove(1, AREA));
    assert_eq!(t.layout(AREA, 0), alloc::vec![(3, AREA)]);
    assert!(t.remove(3, AREA));
}

#[test]
fn resize_moves_the_nearest_divider_on_that_side() {
    let mut t = Tab::new(1);
    t.split(1, Split::Across, 2);
    t.focus = 1;
    assert!(t.resize(Toward::Right, 100));
    assert_eq!(t.layout(AREA, 0)[0].1.w, 600);
    assert!(!t.resize(Toward::Up, 100));
    t.focus = 2;
    assert!(t.resize(Toward::Left, 100));
    assert_eq!(t.layout(AREA, 0)[0].1.w, 500);
    // Never below the minimum share.
    for _ in 0..20 { t.resize(Toward::Left, 100); }
    assert_eq!(t.layout(AREA, 0)[0].1.w, 50);
}

#[test]
fn dragging_a_divider() {
    let mut t = Tab::new(1);
    t.split(1, Split::Across, 2);
    t.split(2, Split::Down, 3);
    let d = t.divider_at(AREA, 2, 750, 300, 3).expect("the inner divider");
    assert_eq!(d.split, Split::Down);
    assert_eq!(d.path, alloc::vec![true]);
    t.drag_divider(&d, 750, 450);
    let l = t.layout(AREA, 2);
    assert_eq!(l[1].1.h, 448);
    let d = t.divider_at(AREA, 2, 500, 10, 3).expect("the outer divider");
    t.drag_divider(&d, 250, 10);
    assert_eq!(t.layout(AREA, 2)[0].1.w, 249);
    assert!(t.divider_at(AREA, 2, 100, 100, 3).is_none());
}

#[test]
fn zoom_shows_only_the_focused_pane() {
    let mut t = Tab::new(1);
    t.split(1, Split::Across, 2);
    t.zoomed = true;
    assert_eq!(t.layout(AREA, 2), alloc::vec![(2, AREA)]);
    assert!(t.dividers(AREA, 2).is_empty());
    t.cycle(true);
    assert_eq!(t.focus, 1);
}

#[test]
fn tabs_open_close_move() {
    let mut tabs = Tabs::new(1);
    tabs.open(2);
    tabs.open(3);
    assert_eq!((tabs.len(), tabs.active, tabs.focus()), (3, 2, 3));
    tabs.select(0);
    tabs.open(4);
    assert_eq!(tabs.all_panes(), alloc::vec![1, 4, 2, 3]);
    tabs.shift(true);
    assert_eq!(tabs.all_panes(), alloc::vec![1, 2, 4, 3]);
    assert_eq!(tabs.active, 2);
    tabs.current_mut().split(4, Split::Down, 5);
    assert_eq!(tabs.close(2), alloc::vec![4, 5]);
    assert_eq!((tabs.active, tabs.focus()), (2, 3));
    tabs.remove_pane(2, AREA);
    assert_eq!(tabs.all_panes(), alloc::vec![1, 3]);
    assert_eq!(tabs.focus(), 3);
    tabs.next(false);
    assert_eq!(tabs.focus(), 1);
    tabs.close(1);
    assert_eq!((tabs.active, tabs.focus()), (0, 1));
}

#[test]
fn config_reads_settings_and_reports_problems() {
    let c = config::parse("# Spindle\ntheme = Solarized Dark\nfont-size = 20\npadding=8 # roomy\nbackground = #101010 # dark\ncolor1 = #ff0000\nwhat = 1\nfont-size = 99\nnonsense\n");
    assert_eq!(c.theme, "solarized-dark");
    assert_eq!((c.font_size, c.padding, c.scrollback), (20, 8, 10_000));
    assert_eq!(c.colors, alloc::vec![(alloc::string::String::from("background"), 0x101010), (alloc::string::String::from("color1"), 0xFF0000)]);
    assert_eq!(c.problems.len(), 3, "{:?}", c.problems);
    let t = config::with_setting("# x\ntheme = nord\npadding = 2\n", "theme", "latte");
    assert_eq!(t, "# x\ntheme = latte\npadding = 2\n");
    assert_eq!(config::with_setting("", "theme", "nord"), "theme = nord\n");
}
