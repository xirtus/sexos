//! Host test for the screenshot selection (`crates/sex-pdx/src/screenshot.rs`),
//! which the Shell, Display and Spindle compile.
#[path = "../crates/sex-pdx/src/screenshot.rs"]
mod screenshot;
use screenshot::*;

#[test]
fn rect_wire_round_trips() {
    let r = Rect { x: 10, y: 20, w: 300, h: 400 };
    assert_eq!(Rect::decode(r.wire()), r);
    assert!(Rect::decode(0).is_full());
    assert_eq!(Rect::span(50, 60, 10, 20), Rect { x: 10, y: 20, w: 40, h: 40 });
    assert_eq!(Rect::span(-5, -5, 10, 10), Rect { x: 0, y: 0, w: 10, h: 10 });
}

#[test]
fn clamp_keeps_inside_the_picture() {
    assert_eq!(Rect::FULL.clamp(1280, 800), Some((0, 0, 1280, 800)));
    assert_eq!(Rect { x: 1200, y: 700, w: 200, h: 200 }.clamp(1280, 800), Some((1200, 700, 80, 100)));
    assert_eq!(Rect { x: 1280, y: 0, w: 5, h: 5 }.clamp(1280, 800), None);
    assert_eq!(Rect::FULL.clamp(0, 800), None);
    assert!(Rect { x: 2, y: 2, w: 2, h: 2 }.contains(3, 3));
    assert!(!Rect { x: 2, y: 2, w: 2, h: 2 }.contains(4, 3));
}

#[test]
fn crop_index_walks_rows() {
    let region = (2, 1, 3, 2);
    let got: Vec<usize> = (0..6).map(|i| crop_index(region, 10, i)).collect();
    assert_eq!(got, vec![12, 13, 14, 22, 23, 24]);
}

#[test]
fn selector_drag_click_keys() {
    let mut s = Selector::new();
    s.press(5, 5);
    assert_eq!(s.release(50, 50), None, "closed selector ignores the pointer");
    s.open();
    assert!(s.is_open());
    assert_eq!(s.selection(), Rect::default());
    s.press(100, 100);
    assert!(s.motion(200, 150));
    assert!(!s.motion(200, 150));
    assert_eq!(s.selection(), Rect { x: 100, y: 100, w: 100, h: 50 });
    assert_eq!(s.release(200, 150), Some(Pick::Region(Rect { x: 100, y: 100, w: 100, h: 50 })));
    assert!(!s.is_open());
    s.open(); s.press(40, 40);
    assert_eq!(s.release(42, 41), Some(Pick::WindowAt(42, 41)));
    s.open();
    assert_eq!(s.key(true), Some(Pick::Full));
    s.open();
    assert_eq!(s.key(false), Some(Pick::Cancel));
    assert_eq!(s.key(true), None);
}

#[test]
fn file_names_count_up() {
    let mut out = [0u8; 32];
    let n = file_name(1, &mut out);
    assert_eq!(&out[..n], b"/Screenshot-1.png");
    let n = file_name(4294967295, &mut out);
    assert_eq!(&out[..n], b"/Screenshot-4294967295.png");
}
