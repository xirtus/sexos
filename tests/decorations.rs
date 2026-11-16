// SPDX-License-Identifier: GPL-3.0-only
//! Host test for window decorations (slice S1): the title bar buttons' hit
//! targets (servers/silk-shell/src/decorations.rs), double-click timing, and
//! the window menu's wire, layout and rows (crates/sex-pdx/src/window_menu.rs),
//! built against a `sex_pdx` of only the modules they use.
#[path = "../servers/silk-shell/src/decorations.rs"]
mod decorations;

use decorations::*;
use sex_pdx::desktop_style::chrome::{Metrics, CONTROLS};
use sex_pdx::window_menu::{Hit, Item, Layout, Menu, MAX_ROWS, OP_WINDOW_MENU};

#[test]
fn four_buttons_left_to_right_then_title_at_every_scale() {
    assert_eq!(CONTROLS, 4);
    for scale in 1..=3u32 {
        let m = Metrics::new(scale);
        let (w, h) = (640, 480);
        let at = |x: u32| title_hit(m, w, h, x as i32, m.px(13) as i32);
        assert_eq!(at(m.px(13)), TitleHit::Button(Button::Close));
        assert_eq!(at(m.px(37)), TitleHit::Button(Button::Minimise));
        assert_eq!(at(m.px(61)), TitleHit::Button(Button::Maximise));
        assert_eq!(at(m.px(85)), TitleHit::Button(Button::Float));
        // The tab strip and title start right of the last button.
        assert_eq!(at(m.controls_end()), TitleHit::Title);
        assert_eq!(at(m.px(150)), TitleHit::Title);
        // The first logical pixel and the row under the title bar are not buttons.
        assert_eq!(title_hit(m, w, h, 0, 0), TitleHit::Title);
        assert_eq!(title_hit(m, w, h, m.px(13) as i32, m.title() as i32), TitleHit::Outside);
        // Outside the window.
        for (x, y) in [(-1, 5), (5, -1), (w as i32, 5), (i32::MIN, i32::MIN)] {
            assert_eq!(title_hit(m, w, h, x, y), TitleHit::Outside);
        }
        // Centres land on their own button.
        for b in Button::ALL {
            let (cx, cy) = button_centre(m, b);
            assert_eq!(title_hit(m, w, h, cx as i32, cy as i32), TitleHit::Button(b));
        }
    }
}

#[test]
fn a_window_too_narrow_for_a_button_has_no_such_button() {
    let m = Metrics::new(1);
    // 80 px wide: close, minimise and maximise fit; float (73..97) does not.
    assert_eq!(title_hit(m, 80, 200, 75, 10), TitleHit::Title);
    assert_eq!(title_hit(m, 80, 200, 61, 10), TitleHit::Button(Button::Maximise));
}

#[test]
fn double_click_needs_same_window_place_and_time() {
    let mut d = DoubleClick::new();
    assert!(!d.press(7, 1000, 100, 10, 4));
    assert!(d.press(7, 1000 + DOUBLE_CLICK_MS, 103, 12, 4));
    // A third press starts over.
    assert!(!d.press(7, 1500, 103, 12, 4));
    // Too slow.
    assert!(!d.press(7, 1500 + DOUBLE_CLICK_MS + 1, 103, 12, 4));
    // Moved too far.
    assert!(!d.press(7, 2400, 120, 12, 4));
    // Another window.
    assert!(!d.press(8, 2500, 120, 12, 4));
    assert!(d.press(8, 2600, 120, 12, 4));
    // A clock going backwards never makes a double click.
    assert!(!d.press(9, 5000, 0, 0, 4));
    assert!(!d.press(9, 4900, 0, 0, 4));
    d.reset();
    assert!(!d.press(9, 4950, 0, 0, 4));
}

#[test]
fn buttons_and_rows_map_to_window_manager_actions() {
    assert_eq!(button_action(Button::Close), Action::Close);
    assert_eq!(button_action(Button::Maximise), Action::ToggleMaximise);
    assert_eq!(button_action(Button::Float), Action::ToggleFloat);
    assert_eq!(button_action(Button::Minimise), Action::Minimise);
    assert_eq!(item_action(Item::Workspace(4)), Action::MoveToWorkspace(4));
    assert_eq!(item_action(Item::Close), Action::Close);
}

#[test]
fn menu_rows_skip_the_windows_own_workspace_and_name_the_state() {
    assert_eq!(OP_WINDOW_MENU, 0x2E7);
    let m = Menu::new(10, 20, false, false, 9, 2);
    assert_eq!(m.rows(), 11);
    assert_eq!(m.rows(), MAX_ROWS);
    let items: Vec<_> = (0..m.rows()).map(|i| m.item(i).unwrap()).collect();
    assert_eq!(items[0], Item::Maximise);
    assert_eq!(items[1], Item::Float);
    assert_eq!(&items[2..10], &[0, 1, 3, 4, 5, 6, 7, 8].map(Item::Workspace));
    assert_eq!(items[10], Item::Close);
    assert_eq!(m.item(11), None);
    assert_eq!(m.label(Item::Maximise), "Maximise");
    assert_eq!(m.label(Item::Float), "Float");
    assert_eq!(m.label(Item::Workspace(0)), "Move to workspace 1");
    assert_eq!(m.label(Item::Workspace(8)), "Move to workspace 9");
    let restore = Menu::new(0, 0, true, true, 1, 0);
    assert_eq!(restore.rows(), 3);
    assert_eq!(restore.label(Item::Maximise), "Restore");
    assert_eq!(restore.label(Item::Float), "Tile");
    assert_eq!(restore.item(2), Some(Item::Close));
}

#[test]
fn menu_wire_round_trips_and_rejects_bad_words() {
    for m in [Menu::new(0, 0, false, false, 9, 0), Menu::new(1279, 799, true, true, 9, 8), Menu { hover: Some(10), ..Menu::new(5, 6, false, true, 9, 3) }] {
        assert_eq!(Menu::decode(m.wire()), Some(m));
    }
    assert_eq!(Menu::decode(0), None);
    let good = Menu::new(5, 6, false, false, 4, 1).wire();
    assert!(Menu::decode(good).is_some());
    // Unknown bits, too many workspaces, current past the count, hover past the rows.
    assert_eq!(Menu::decode(good | 1 << 50), None);
    assert_eq!(Menu::decode(good | 1 << 42), None);
    assert_eq!(Menu::decode(good & !(0xF << 32) | 10 << 32), None);
    assert_eq!(Menu::decode(good & !(0xF << 36) | 4 << 36), None);
    assert_eq!(Menu::decode(good & !(0xF << 44) | 6 << 44), None);
    assert_eq!(Menu::decode(good & !(1 << 63)), None);
    // Positions clamp; workspaces and current are kept in range.
    let m = Menu::new(-5, 70000, false, false, 0, 7);
    assert_eq!((m.x, m.y, m.workspaces, m.current), (0, u16::MAX, 1, 0));
}

#[test]
fn menu_layout_stays_on_screen_and_hits_rows() {
    for scale in 1..=3 {
        let m = Menu::new(1270, 790, false, false, 9, 0);
        let l = Layout::new(&m, 1280 * scale as usize, 800 * scale as usize, scale).unwrap();
        let (x, y, w, h) = l.panel;
        assert!(x + w <= 1280 * scale as usize && y + h <= 800 * scale as usize);
        assert_eq!(l.rows, 11);
        for i in 0..l.rows {
            let r = l.row(i);
            assert_eq!(l.hit(r.0 + 1, r.1 + 1), Hit::Row(i));
            assert_eq!(l.hit(r.0 + r.2 - 1, r.1 + r.3 - 1), Hit::Row(i));
        }
        assert_eq!(l.hit(x, y), Hit::Inside);
        assert_eq!(l.hit(x.saturating_sub(1), y), Hit::Outside);
        assert_eq!(l.hit(x + w, y + h), Hit::Outside);
    }
    // A screen too small for the menu has none.
    assert_eq!(Layout::new(&Menu::new(0, 0, false, false, 9, 0), 100, 100, 1), None);
}

#[test]
fn window_menu_press_and_hover() {
    let (w, h) = (1280, 800);
    let mut menu = WindowMenu::new();
    assert!(!menu.is_open());
    assert_eq!(menu.wire(), 0);
    assert_eq!(menu.press(0, 0, w, h, 1), Press::Dismiss);
    menu.open(42, Menu::new(100, 100, false, false, 9, 0));
    assert_eq!(menu.frame(), Some(42));
    let l = menu.layout(w, h, 1).unwrap();
    // Hover follows the pointer; only a change is reported.
    let r1 = l.row(1);
    assert!(menu.hover(r1.0 as i32 + 2, r1.1 as i32 + 2, w, h, 1));
    assert!(!menu.hover(r1.0 as i32 + 3, r1.1 as i32 + 3, w, h, 1));
    assert_eq!(Menu::decode(menu.wire()).unwrap().hover, Some(1));
    assert!(menu.hover(-1, -1, w, h, 1));
    // The border does nothing and keeps the menu.
    assert_eq!(menu.press(l.panel.0 as i32, l.panel.1 as i32, w, h, 1), Press::Inside);
    assert!(menu.is_open());
    // Row 1 is Float; choosing it closes the menu.
    assert_eq!(menu.press(r1.0 as i32 + 2, r1.1 as i32 + 2, w, h, 1), Press::Choose { frame: 42, action: Action::ToggleFloat });
    assert!(!menu.is_open());
    // Move to workspace: the window is on workspace 0, so row 2 is workspace 1 (0-based).
    menu.open(42, Menu::new(100, 100, false, false, 9, 0));
    let r2 = l.row(2);
    assert_eq!(menu.press(r2.0 as i32 + 2, r2.1 as i32 + 2, w, h, 1), Press::Choose { frame: 42, action: Action::MoveToWorkspace(1) });
    // The last row closes the window.
    menu.open(42, Menu::new(100, 100, false, false, 9, 0));
    let last = l.row(l.rows - 1);
    assert_eq!(menu.press(last.0 as i32 + 2, last.1 as i32 + 2, w, h, 1), Press::Choose { frame: 42, action: Action::Close });
    // A press elsewhere dismisses it.
    menu.open(42, Menu::new(100, 100, false, false, 9, 0));
    assert_eq!(menu.press(5, 5, w, h, 1), Press::Dismiss);
    assert!(!menu.close());
}

#[test]
fn menu_shrinks_to_fit_a_short_screen_at_large_scale() {
    // 1280x800 at 3x: nine workspaces need 816 px at 3x, so 2x is used.
    let m = Menu::new(0, 0, false, false, 9, 0);
    let l = Layout::new(&m, 1280, 800, 3).unwrap();
    assert_eq!(l.scale, 2);
    assert!(l.panel.1 + l.panel.3 <= 800);
    assert_eq!(Layout::new(&m, 1280, 800, 1).unwrap().scale, 1);
    assert_eq!(Layout::new(&m, 3840, 2400, 3).unwrap().scale, 3);
}
