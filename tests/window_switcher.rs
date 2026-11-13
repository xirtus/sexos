//! Host test for the Alt+Tab switcher's wire, layout and order
//! (`crates/sex-pdx/src/window_switcher.rs`), which the Shell and Display compile.
#[path = "../crates/sex-pdx/src/window_switcher.rs"]
mod window_switcher;
use window_switcher::*;

#[test]
fn wire_round_trips_and_rejects_bad_words() {
    let s = State::new(&[300, 156, 0x99, 401, 402, 403, 404, 405], 5).unwrap();
    let [a, b, c] = s.wire();
    assert_eq!(State::decode(a, b, c), Some(s));
    assert_eq!(s.ids().collect::<Vec<_>>(), [300, 156, 0x99, 401, 402, 403, 404, 405]);
    assert_eq!(State::decode(0, 0, 0), Some(State::HIDDEN));
    assert!(!State::HIDDEN.visible());
    // Selected past the end, a hole, a stray id after the count, high bits.
    assert_eq!(State::decode(300, 0, 1 | 1 << 8), None);
    assert_eq!(State::decode(300, 0, 2), None);
    assert_eq!(State::decode(300 | 156 << 16, 0, 1), None);
    assert_eq!(State::decode(300, 0, 1 | 1 << 12), None);
    assert_eq!(State::new(&[70000], 0), None);
    assert_eq!(State::new(&[1; 9], 0), None);
}

#[test]
fn recent_order_puts_the_last_focused_first() {
    let mut r = Recent::<4>::new();
    for id in [10, 20, 30, 20] { r.touch(id); }
    assert_eq!(r.as_slice(), [20, 30, 10]);
    for id in [40, 50] { r.touch(id); }
    assert_eq!(r.as_slice(), [50, 40, 20, 30]);
    r.forget(40);
    assert_eq!(r.as_slice(), [50, 20, 30]);
    let mut out = [0; 8];
    // Windows on screen: 30 and a never-focused 60; 50 is on another workspace.
    let n = r.order(&[60, 30, 20], &mut out);
    assert_eq!(&out[..n], [20, 30, 60]);
}

#[test]
fn alt_tab_steps_and_alt_release_keeps_escape_goes_back() {
    let mut s = Session::new();
    assert_eq!(s.step(&[], 0, 1), None);
    assert!(!s.is_open());
    // Quick Alt+Tab: the previous window.
    assert_eq!(s.step(&[1, 2, 3], 1, 1), Some(2));
    assert_eq!(s.state().wire()[2], 3 | 1 << 8);
    assert_eq!(s.end(true), Some(2));
    assert_eq!(s.state(), State::HIDDEN);
    // Tab, Tab, Shift+Tab, Tab wraps; Escape returns to where it started.
    assert_eq!(s.step(&[2, 1, 3], 2, 1), Some(1));
    assert_eq!(s.step(&[9], 9, 1), Some(3));
    assert_eq!(s.step(&[], 0, 1), Some(2));
    assert_eq!(s.step(&[], 0, -1), Some(3));
    assert_eq!(s.end(false), Some(2));
    // A click picks a card; a window closing while open drops out.
    s.step(&[5, 6, 7], 5, 1);
    s.forget(6);
    assert_eq!(s.state().ids().collect::<Vec<_>>(), [5, 7]);
    assert_eq!(s.select(1), Some(7));
    assert_eq!(s.select(2), None);
    assert_eq!(s.end(true), Some(7));
    assert_eq!(s.end(true), None);
    // Focus on a panel: Tab lands on the most recent window, Escape returns to the panel.
    assert_eq!(s.step(&[5, 7], 0x93, 1), Some(5));
    assert_eq!(s.end(false), Some(0x93));
}

#[test]
fn layout_fits_cards_on_small_and_hidpi_screens() {
    for (w, h, scale) in [(1280, 800, 1), (1024, 768, 1), (2560, 1600, 2), (800, 600, 1)] {
        for n in 1..=MAX_ENTRIES {
            let l = Layout::new(w, h, n, scale).unwrap();
            let (px, py, pw, ph) = l.panel;
            assert!(px + pw <= w && py + ph <= h, "{w}x{h} n={n}");
            let (lx, _, lw, _) = l.card(n - 1);
            assert!(lx + lw < px + pw);
            let (cx, cy, _, _) = l.card(0);
            assert_eq!(l.card_at(cx + 1, cy + 1), Some(0));
            assert_eq!(l.card_at(px, py), None);
        }
    }
    assert_eq!(Layout::new(1280, 800, 0, 1), None);
    assert_eq!(Layout::new(100, 60, 8, 1), None);
}

#[test]
fn overview_grid_wire_layout_and_navigation() {
    let mut s = Session::new();
    assert!(!s.open_grid(&[], 1));
    assert!(s.open_grid(&[4, 5, 6, 7, 8], 4));
    let state = s.state();
    assert!(state.grid && state.selected == 0);
    let [a, b, c] = state.wire();
    assert_eq!(c, 5 | 1 << 16);
    assert_eq!(State::decode(a, b, c), Some(state));
    assert_eq!(State::decode(0, 0, 1 << 16), None);
    assert_eq!(s.move_by(3), Some(7));
    assert_eq!(s.move_by(3), Some(5));
    assert_eq!(s.move_by(-2), Some(8));
    assert_eq!(s.end(false), Some(4));
    for (w, h, bar, scale) in [(1280, 800, 28, 1), (1024, 768, 28, 1), (2560, 1600, 56, 2)] {
        for n in 1..=MAX_ENTRIES {
            let l = Layout::grid(w, h, bar, n, scale).unwrap();
            assert!(l.cols >= 1 && l.cols <= n);
            for i in 0..n {
                let (x, y, cw, ch) = l.card(i);
                assert!(x + cw <= w && y >= bar && y + ch <= h, "{w}x{h} n={n} i={i}");
                assert_eq!(l.card_at(x + cw / 2, y + ch / 2), Some(i));
            }
        }
    }
    // Four windows on a wide screen: a 2x2 or 4x1 grid, never one tall column.
    assert!(Layout::grid(1280, 800, 28, 4, 1).unwrap().cols >= 2);
}
