//! Host tests for the SexOS mark: the exact `crates/sex-pdx/src/brand.rs`
//! the kernel boot screen, Display and the Shell draw with.
#[path = "../crates/sex-pdx/src/brand.rs"]
mod brand;
#[path = "../crates/sex-pdx/src/desktop_look.rs"]
mod desktop_look;
use brand::*;


#[test]
fn mark_is_solid_in_the_middle_and_empty_around_it() {
    let w = 268;
    let mut row = [0u8; 268];
    // At 1 px per design unit: the upper lip's middle, the open mouth, the lower lip.
    coverage_row(w, 50, &mut row);
    assert_eq!(row[134], 255);
    coverage_row(w, 100, &mut row);
    assert_eq!(row[134], 0, "the mouth is open between the lips");
    coverage_row(w, 140, &mut row);
    assert_eq!(row[134], 255, "lower lip");
    assert_eq!(row[2], 0);
    coverage_row(w, 0, &mut row);
    assert_eq!(row[0], 0);
}

#[test]
fn mark_is_symmetric() {
    let w = 200;
    let mut row = [0u8; 200];
    for y in 0..height_for(w) {
        coverage_row(w, y, &mut row);
        for x in 0..100 {
            let (l, r) = (row[x] as i32, row[199 - x] as i32);
            assert!((l - r).abs() <= 40, "y {y} x {x}: {l} vs {r}");
        }
    }
}

#[test]
fn gradient_runs_dark_to_peach() {
    assert_eq!(color_at(Tone::Rich, 0, 100), RICH[0].1);
    assert_eq!(color_at(Tone::Rich, 99, 100), RICH[3].1);
    assert_eq!(color_at(Tone::Bright, 0, 100), BRIGHT[0].1);
    assert_eq!(blend(0xFFFFFF, 0xFF00_0000, 0), 0xFF00_0000);
    assert_eq!(blend(0x123456, 0xFF00_0000, 255), 0xFF12_3456);
}

#[test]
fn small_sizes_still_draw() {
    let mut row = [0u8; 16];
    let mut ink = 0u32;
    for y in 0..height_for(16) {
        coverage_row(16, y, &mut row);
        ink += row.iter().map(|&c| c as u32).sum::<u32>();
    }
    assert!(ink > 255 * 30, "a 16 px mark has about 50 px of ink, got {}", ink / 255);
}

#[test]
fn wallpaper_mark_sits_in_the_middle() {
    let (x, y, w) = wallpaper_place(1920, 1080);
    assert_eq!(w, 499);
    assert_eq!(x, (1920 - 499) / 2);
    assert!(y > 300 && y + height_for(w) < 800);
    assert_eq!(fit_width(40, 20), 33);
    assert_eq!(fit_width(20, 40), 20);
}

#[test]
fn draw_touches_only_the_box() {
    let mut n = 0;
    draw(64, Tone::Rich, |_, _| 0, |x, y, c| { assert!(x < 64 && y < height_for(64) && c != 0); n += 1; });
    assert!(n > 500, "{n}");
}

#[test]
fn sexos_wallpaper_wears_the_brand_charcoal() {
    let (name, top, bottom) = desktop_look::WALLPAPERS[desktop_look::MARK_WALLPAPER as usize];
    assert_eq!((name, top, bottom), ("SexOS", WALL_TOP, WALL_BOTTOM));
}
