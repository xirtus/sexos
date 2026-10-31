#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]
macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
macro_rules! serial_println{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
const EV_REL: u64 = 2;
const EV_ABS: u64 = 3;
const EV_BTN: u64 = 4;
const EV_WHEEL: u64 = 5;
#[path="../crates/sex-pdx/src/app_scroll.rs"]pub mod app_scroll;
#[derive(Clone, Copy)]
struct HidPointerRawReport {
    dx: i16,
    dy: i16,
    buttons: u8,
    wheel: i8,
    is_abs: bool,
}
include!(env!("ABS_POINTER_PRODUCTION"));
#[test]
fn absolute_first_move_and_edges_keep_exact_coordinates_for_any_geometry() {
    let mut last = 0;
    let mut calls = vec![];
    let r = HidPointerRawReport {
        dx: 18651,
        dy: 4880,
        buttons: 1,
        wheel: 0,
        is_abs: true,
    };
    normalize_pointer_report_v1(r, &mut last, |a, b, c| calls.push((a, b, c)));
    assert_eq!(calls, vec![(18651, 4880, EV_ABS), (1, 1, EV_BTN)]);
    calls.clear();
    normalize_pointer_report_v1(
        HidPointerRawReport { buttons: 0, ..r },
        &mut last,
        |a, b, c| calls.push((a, b, c)),
    );
    assert_eq!(calls, vec![(18651, 4880, EV_ABS), (1, 0, EV_BTN)]);
}
#[test]
fn relative_mouse_still_emits_signed_deltas_without_absolute_scaling() {
    let mut last = 0;
    let mut calls = vec![];
    normalize_pointer_report_v1(
        HidPointerRawReport {
            dx: -7,
            dy: 12,
            buttons: 0,
            wheel: 0,
            is_abs: false,
        },
        &mut last,
        |a, b, c| calls.push((a, b, c)),
    );
    assert_eq!(calls, vec![((-7i64) as u64, 12, EV_REL)]);
}
