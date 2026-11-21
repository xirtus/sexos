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
    assert!(set(0, 0) && night_level() == 0);
    assert!(shows_mark() && wallpaper() == Some((WALLPAPERS[0].1, WALLPAPERS[0].2)), "the default is the SexOS wallpaper");
    assert!(set(2, 3) && !shows_mark());
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

#[test]
fn theme_word_round_trips_every_mode_and_accent() {
    for mode in 0..THEME_MODES {
        for accent in [0, DEFAULT_ACCENT, 0x3584E4, 0xFF_FFFF] {
            let word = theme_word(mode, accent);
            assert!(word != 0 && word >> 32 == 0, "fits arg2 with room");
            assert_eq!(decode_theme(word), Ok(Some((mode, accent))));
        }
    }
    assert_eq!(theme_word(1, 0x1D6_336C), theme_word(1, 0xD6336C), "only the colour's 24 bits travel");
    assert_eq!(decode_theme(0), Ok(None), "0 keeps the theme: an older sender");
    assert_eq!((theme_name(0), theme_name(1), theme_name(2)), ("light", "dark", "high-contrast"));
}

#[test]
fn unknown_theme_words_are_refused() {
    let _turn = LOOK.lock().unwrap();
    assert!(set_look(0, 0, theme_word(0, DEFAULT_ACCENT)));
    // Bit 32 is Reduce transparency; it never makes a theme word on its own,
    // and every bit above it is still unknown.
    for bad in [THEME_MODES as u64 + 1, 0xFF, 1 << 32, theme_word(1, 0x123456) | 1 << 40, theme_word(1, 0x123456) | 1 << 33, 0x0000_1200] {
        assert_eq!(decode_theme(bad), Err(()), "{bad:#x}");
        assert!(!set_look(3, 2, bad), "{bad:#x}");
    }
    assert_eq!((wallpaper_index(), night_level(), theme()), (0, 0, (0, DEFAULT_ACCENT)), "a refused look changes nothing");
    // A bad wallpaper refuses the theme with it.
    assert!(!set_look(WALLPAPERS.len() as u64, 0, theme_word(2, 0)));
    assert_eq!(theme(), (0, DEFAULT_ACCENT));
}

#[test]
fn look_carries_the_theme_and_zero_keeps_it() {
    let _turn = LOOK.lock().unwrap();
    assert!(set_look(0, 0, theme_word(0, DEFAULT_ACCENT)));
    assert!(set_look(1, 2, theme_word(1, 0x3584E4)));
    assert_eq!((wallpaper_index(), night_level(), theme()), (1, 2, (1, 0x3584E4)));
    assert!(set_look(4, 0, 0) && theme() == (1, 0x3584E4), "arg2 = 0 leaves the theme");
    assert!(set(2, 1) && theme() == (1, 0x3584E4), "the old two-value set leaves it too");
    assert!(set_look(0, 0, theme_word(0, DEFAULT_ACCENT)));
    assert_eq!(theme(), (0, DEFAULT_ACCENT));
}

#[test]
fn reduce_transparency_rides_with_the_theme() {
    let _turn = LOOK.lock().unwrap();
    assert!(set_look(0, 0, theme_word(0, DEFAULT_ACCENT)) && !reduce_transparency());
    let word = theme_word_reduced(1, 0x3584E4, true);
    assert_eq!(word, theme_word(1, 0x3584E4) | REDUCE_TRANSPARENCY);
    assert_eq!(decode_theme(word), Ok(Some((1, 0x3584E4))), "the theme itself is the same");
    assert!(set_look(0, 0, word) && reduce_transparency() && theme() == (1, 0x3584E4));
    assert!(set_look(2, 0, 0) && reduce_transparency(), "arg2 = 0 keeps it");
    assert!(!set_look(0, 0, word | 1 << 33) && reduce_transparency(), "a refused word changes nothing");
    assert!(set_look(0, 0, theme_word_reduced(0, DEFAULT_ACCENT, false)) && !reduce_transparency());
}
