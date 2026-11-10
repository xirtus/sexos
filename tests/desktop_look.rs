//! Host tests for wallpaper and night light: the exact
//! `crates/sex-pdx/src/desktop_look.rs` Display and the Shell compile.
#[path = "../crates/sex-pdx/src/desktop_look.rs"]
mod desktop_look;
use desktop_look::*;
/// The look is one shared state; tests that change it take turns.
static LOOK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn night_light_off_keeps_every_pixel() {
    for p in [0u32, 0x00FF_FFFF, 0x0012_3456, 0x00A8_505A] { assert_eq!(night(p, 0), p); }
}

#[test]
fn night_light_keeps_red_and_warms_more_each_level() {
    let white = 0x00FF_FFFFu32;
    let mut last = white;
    for level in 1..NIGHT_LEVELS {
        let p = night(white, level);
        assert_eq!(p >> 16, 0xFF, "red stays");
        assert!(p & 0xFF < last & 0xFF && (p >> 8 & 0xFF) <= (last >> 8 & 0xFF), "level {level} warmer");
        assert!(p & 0xFF < (p >> 8 & 0xFF), "blue drops more than green");
        last = p;
    }
    assert_eq!(night(white, 99), night(white, NIGHT_LEVELS - 1));
}

#[test]
fn look_takes_only_known_values() {
    let _turn = LOOK.lock().unwrap();
    assert!(set(0, 0) && wallpaper().is_none() && night_level() == 0);
    assert!(set(2, 3));
    assert_eq!(wallpaper(), Some((WALLPAPERS[2].1, WALLPAPERS[2].2)));
    assert!(!set(WALLPAPERS.len() as u64, 0) && !set(0, NIGHT_LEVELS as u64));
    assert_eq!((wallpaper_index(), night_level()), (2, 3), "a refused look changes nothing");
    assert!(set(0, 0));
}

#[test]
fn publisher_sees_each_night_change_once() {
    let _turn = LOOK.lock().unwrap();
    assert!(set(0, 0)); take_night_change();
    assert!(!take_night_change());
    assert!(set(0, 2));
    assert!(take_night_change() && !take_night_change());
    assert!(set(0, 0) && take_night_change());
}
