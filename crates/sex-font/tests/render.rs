// SPDX-License-Identifier: GPL-3.0-only
//! Light hinting and stem darkening (`Render`) against the bare outline.
use sex_font::{darkening, raster::scratch_len, Font, Render, Synthetic, GEIST_SANS};

fn draw(font: &Font, ch: char, px: u32, x: i32, render: Render) -> (Vec<u8>, usize) {
    let (w, h) = (px as usize * 2, px as usize * 2);
    let mut acc = vec![0; scratch_len(w, h)];
    let mut out = vec![0; w * h];
    let g = font.glyph(ch as u32).unwrap();
    assert!(font.draw_rendered(g, px * 64, x, (px as i32 * 3 / 2) * 256 + 77, w, h, &mut acc, &mut out, Synthetic::NONE, render));
    (out, w)
}

#[test]
fn plain_render_is_the_old_drawing() {
    let font = Font::new(GEIST_SANS).unwrap();
    let g = font.glyph('a' as u32).unwrap();
    let (w, h) = (26, 26);
    let mut acc = vec![0; scratch_len(w, h)];
    let mut old = vec![0; w * h];
    font.draw(g, 13 * 64, 300, 19 * 256 + 77, w, h, &mut acc, &mut old);
    let (new, _) = draw(&font, 'a', 13, 300, Render::PLAIN);
    assert_eq!(old, new);
}

#[test]
fn hinted_x_height_and_baseline_are_crisp() {
    let font = Font::new(GEIST_SANS).unwrap();
    // The flat top of `x`-height letters lands on a pixel row: the row above 'z'
    // (which has flat top and bottom) is empty and the top row is nearly solid.
    for px in [11, 12, 13, 14, 16] {
        let hint = Render { hint_below: 32 * 64, darken: false };
        let (out, w) = draw(&font, 'z', px, 256 * 2, hint);
        let rows: Vec<u32> = out.chunks(w).map(|r| r.iter().map(|v| *v as u32).sum()).collect();
        let inked: Vec<usize> = (0..rows.len()).filter(|i| rows[*i] > 0).collect();
        let (top, bottom) = (inked[0], *inked.last().unwrap());
        let peak = *rows.iter().max().unwrap();
        assert!(rows[top] * 10 >= peak * 7, "{px}px top row soft: {rows:?}");
        assert!(rows[bottom] * 10 >= peak * 7, "{px}px bottom row soft: {rows:?}");
    }
}

#[test]
fn darkening_adds_ink_not_width_and_fades_out() {
    let font = Font::new(GEIST_SANS).unwrap();
    let ink = |r: Render, px: u32| draw(&font, 'l', px, 256 * 4, r).0.iter().map(|v| *v as u32).sum::<u32>();
    let dark = Render { hint_below: 0, darken: true };
    assert!(ink(dark, 12) > ink(Render::PLAIN, 12) * 11 / 10);
    assert_eq!(ink(dark, 40), ink(Render::PLAIN, 40));
    assert!(darkening(8 * 64) > darkening(16 * 64) && darkening(32 * 64) == 0);
}

#[test]
fn ui_render_darkens_only_dark_ink() {
    assert!(Render::ui_for(0x1f1b1c).darken);
    assert!(!Render::ui_for(0xf2ebe7).darken);
}
