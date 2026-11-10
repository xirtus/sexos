#[path = "../apps/web/src/viewport.rs"]
mod viewport;
use viewport::{Frame, Surface};

#[test]
fn actual_grant_bounds_overflow_and_zero_are_not_fixed_fallback_dimensions() {
    let s = Surface::checked(1264, 706, 1_048_576).unwrap();
    assert_eq!(
        (s.w, s.h, s.pixels, s.document(30)),
        (1264, 706, 892384, (1264, 676))
    );
    assert_eq!(Surface::checked(0, 706, 1), Some(Surface::EMPTY));
    assert_eq!(Surface::checked(1264, 0, 1), Some(Surface::EMPTY));
    for (w, h, capacity) in [
        (1264, 706, 892383),
        (usize::MAX, 2, usize::MAX),
        (65536, 65536, usize::MAX),
    ] {
        assert!(Surface::checked(w, h, capacity).is_none());
    }
    assert_eq!(Surface::checked(5, 8, 40).unwrap().document(30), (5, 0));
}

#[test]
fn iframe_copy_clips_left_top_and_toolbar_and_preserves_uncovered_pixels() {
    let surface = Surface::checked(4, 4, 16).unwrap();
    let frame = Frame::visible(surface, [-1, 0, 4, 3], 1, 1, 16).unwrap();
    assert_eq!(
        (frame.sx, frame.sy, frame.dx, frame.dy, frame.w, frame.h),
        (1, 1, 0, 1, 3, 2)
    );
    let source: Vec<u32> = (100..112).collect();
    let mut dest = [7; 16];
    assert!(frame.copy(surface, &mut dest, &source));
    assert_eq!(
        dest,
        [7, 7, 7, 7, 105, 106, 107, 7, 109, 110, 111, 7, 7, 7, 7, 7]
    );
}

#[test]
fn iframe_copy_clips_right_bottom_and_refuses_impossible_scratch_or_buffers() {
    let surface = Surface::checked(4, 4, 16).unwrap();
    let frame = Frame::visible(surface, [3, 2, 3, 3], 0, 1, 16).unwrap();
    assert_eq!(
        (frame.sx, frame.sy, frame.dx, frame.dy, frame.w, frame.h),
        (0, 0, 3, 3, 1, 1)
    );
    let mut dest = [9; 16];
    assert!(frame.copy(surface, &mut dest, &[1, 2, 3, 4, 5, 6, 7, 8, 9]));
    assert_eq!(dest, [9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 1]);
    let before = dest;
    assert!(!frame.copy(surface, &mut dest, &[1]));
    assert_eq!(dest, before);
    assert!(Frame::visible(surface, [0, 0, 5, 5], 0, 1, 16).is_none());
    assert!(Frame::visible(surface, [i32::MAX, i32::MAX, 4, 4], i32::MIN, 1, 16).is_none());
    assert!(Frame::visible(surface, [i32::MIN, i32::MIN, 4, 4], i32::MAX, 1, 16).is_none());
}
