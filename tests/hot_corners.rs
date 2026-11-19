// SPDX-License-Identifier: GPL-3.0-only
//! Host test for hot corners (slice S5): servers/silk-shell/src/hot_corners.rs.
#[path = "../servers/silk-shell/src/hot_corners.rs"]
mod hot_corners;
use hot_corners::*;

const W: i32 = 1280;
const H: i32 = 800;

#[test]
fn corner_pixels_only() {
    assert_eq!(at(0, 0, W, H), Some(Corner::TopLeft));
    assert_eq!(at(W - 1, 0, W, H), Some(Corner::TopRight));
    assert_eq!(at(0, H - 1, W, H), Some(Corner::BottomLeft));
    assert_eq!(at(W - 1, H - 1, W, H), Some(Corner::BottomRight));
    assert_eq!(at(1, 0, W, H), None);
    assert_eq!(at(0, 1, W, H), None);
    assert_eq!(at(640, 400, W, H), None);
    assert_eq!(at(0, 0, 0, 0), None);
}

#[test]
fn boot_pointer_in_the_corner_does_not_fire() {
    let mut t = Tracker::new();
    assert_eq!(t.update(0, 0, W, H, false), None);
    assert_eq!(t.update(0, 0, W, H, false), None);
    assert!(!t.armed());
}

#[test]
fn fires_once_then_rearms_after_leaving() {
    let mut t = Tracker::new();
    assert_eq!(t.update(300, 300, W, H, false), None);
    assert_eq!(t.update(3, 2, W, H, false), None, "near is not in");
    assert_eq!(t.update(0, 0, W, H, false), Some(Corner::TopLeft));
    assert_eq!(t.update(0, 0, W, H, false), None, "staying does not repeat");
    assert_eq!(t.update(4, 4, W, H, false), None);
    assert_eq!(t.update(0, 0, W, H, false), None, "not far enough away to rearm");
    assert_eq!(t.update(REARM, 0, W, H, false), None);
    assert_eq!(t.update(0, 0, W, H, false), Some(Corner::TopLeft));
}

#[test]
fn every_corner_fires() {
    for c in CORNERS {
        let mut t = Tracker::new();
        t.update(640, 400, W, H, false);
        let (x, y) = c.point(W, H);
        assert_eq!(t.update(x, y, W, H, false), Some(c));
    }
}

#[test]
fn a_held_button_never_fires_and_disarms() {
    let mut t = Tracker::new();
    t.update(640, 400, W, H, false);
    assert_eq!(t.update(0, 0, W, H, true), None, "dragging into the corner");
    assert_eq!(t.update(0, 0, W, H, false), None, "released there: still nothing");
    t.update(100, 100, W, H, false);
    assert_eq!(t.update(0, 0, W, H, false), Some(Corner::TopLeft));
}

#[test]
fn settings_map_to_actions() {
    assert_eq!(Action::from_setting(0), Action::Nothing);
    assert_eq!(Action::from_setting(1), Action::Overview);
    assert_eq!(Action::from_setting(4), Action::Lock);
    assert_eq!(Action::from_setting(99), Action::Nothing);
    assert_eq!(Action::from_setting(-1), Action::Nothing);
    let names: Vec<&str> = ACTIONS.iter().map(|a| a.name()).collect();
    assert_eq!(names, ["none", "overview", "launcher", "notifications", "lock"], "sex_config::CORNER_ACTIONS order");
}
