//! Host test for window animations (`crates/sex-pdx/src/window_motion.rs`).
#[path = "../crates/sex-pdx/src/window_motion.rs"]
mod window_motion;
use window_motion::*;

const A: Geom = Geom { x: 0, y: 40, w: 600, h: 400 };
const B: Geom = Geom { x: 640, y: 40, w: 600, h: 700 };

#[test]
fn ease_runs_from_zero_to_one_and_never_back() {
    assert_eq!(ease(0, DURATION), 0);
    assert_eq!(ease(DURATION, DURATION), 1024);
    let mut last = 0;
    for e in 0..=DURATION { let k = ease(e, DURATION); assert!(k >= last && k <= 1024); last = k; }
    assert!(ease(DURATION / 2, DURATION) > 512, "ease-out is past half way at half time");
}

#[test]
fn moves_glide_and_finish() {
    let mut m: Motion<4> = Motion::new();
    m.moved(7, A, B, 1000);
    assert!(m.active(1000));
    assert_eq!(m.current(7, 1000), Some(A));
    let mid = m.current(7, 1000 + DURATION / 2).unwrap();
    assert!(mid.x > A.x && mid.x < B.x && mid.h > A.h && mid.h < B.h);
    assert_eq!(m.current(7, 1000 + DURATION), None);
    assert_eq!(m.span(), Some((0, 40, 1240, 740)));
    assert!(m.progress(1000 + DURATION / 2).is_some_and(|k| k > 512 && k < 1024));
    m.clear_finished(1000 + DURATION);
    assert_eq!(m.span(), None);
    assert!(!m.active(1000 + DURATION));
    // A second move continues from where the window is drawn.
    m.moved(7, A, B, 2000);
    let at = m.current(7, 2050).unwrap();
    m.moved(7, B, A, 2050);
    assert_eq!(m.current(7, 2050), Some(at));
}

#[test]
fn opens_rise_or_slide_after_a_workspace_switch() {
    let mut m: Motion<4> = Motion::new();
    m.opened(1, A, 0);
    assert_eq!(m.current(1, 0), Some(Geom { y: A.y + RISE, ..A }));
    m.workspace(1, 1280, 500);
    m.opened(2, A, 550);
    assert_eq!(m.current(2, 550), Some(Geom { x: A.x + 1280, ..A }));
    m.opened(3, A, 500 + SLIDE_WINDOW + 1);
    assert_eq!(m.current(3, 500 + SLIDE_WINDOW + 1).unwrap().x, A.x, "late opens rise again");
}

#[test]
fn off_means_single_frame_and_full_table_recycles() {
    let mut m: Motion<2> = Motion::new();
    m.set_enabled(false);
    m.moved(1, A, B, 0);
    assert!(!m.active(0));
    assert_eq!(m.current(1, 0), None);
    m.set_enabled(true);
    m.moved(1, A, B, 0);
    m.moved(2, A, B, 10);
    m.moved(3, A, B, 20);
    assert!(m.current(3, 20).is_some());
    assert!(m.current(1, 20).is_none(), "the oldest gave way");
    m.forget(3);
    assert!(m.current(3, 20).is_none());
    m.moved(4, A, A, 0);
    assert!(m.current(4, 0).is_none(), "no move, no tween");
}
