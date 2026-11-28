// SPDX-License-Identifier: GPL-3.0-only
//! Bold and italic: the real Geist faces, and synthetic ones for fonts without them.
use sex_font::raster::scratch_len;
use sex_font::{
    geist, geist_weight, Font, Style, Synthetic, Weight, GEIST_MONO_MEDIUM, GEIST_MONO_SEMIBOLD,
    GEIST_SANS_MEDIUM, GEIST_SANS_SEMIBOLD,
};

const W: usize = 48;
const H: usize = 48;

/// Coverage of `c` at 32 px, its baseline 36 px down, pen 8 px in.
fn draw(font: &Font, c: char, synth: Synthetic) -> Vec<u8> {
    let g = font.glyph(c as u32).unwrap();
    let mut acc = vec![0; scratch_len(W, H)];
    let mut out = vec![0; W * H];
    assert!(font.draw_synthetic(g, 32 * 64, 8 * 256, 36 * 256, W, H, &mut acc, &mut out, synth));
    out
}

fn ink(cov: &[u8]) -> u32 { cov.iter().map(|&c| c as u32).sum() }

/// Mean x of the ink in the rows `rows`, in pixels.
fn centre(cov: &[u8], rows: core::ops::Range<usize>) -> f32 {
    let (mut sum, mut total) = (0f32, 0f32);
    for y in rows { for x in 0..W { let c = cov[y * W + x] as f32; sum += c * x as f32; total += c; } }
    sum / total.max(1.0)
}

#[test]
fn every_geist_face_parses_and_says_what_it_is() {
    for mono in [false, true] {
        for style in [Style::REGULAR, Style::BOLD, Style::ITALIC, Style::BOLD_ITALIC] {
            let f = Font::new(geist(mono, style)).unwrap_or_else(|| panic!("{mono} {style:?}"));
            assert_eq!(f.style(), style, "mono={mono}");
            assert!(f.glyph('A' as u32).is_some());
        }
    }
}

#[test]
fn bold_faces_carry_more_ink() {
    for mono in [false, true] {
        let regular = Font::new(geist(mono, Style::REGULAR)).unwrap();
        let bold = Font::new(geist(mono, Style::BOLD)).unwrap();
        let (r, b) = (ink(&draw(&regular, 'n', Synthetic::NONE)), ink(&draw(&bold, 'n', Synthetic::NONE)));
        assert!(b as f32 > r as f32 * 1.3, "mono={mono}: regular {r}, bold {b}");
    }
}

#[test]
fn italic_faces_lean_right() {
    let regular = Font::new(geist(false, Style::REGULAR)).unwrap();
    let italic = Font::new(geist(false, Style::ITALIC)).unwrap();
    let lean = |cov: &[u8]| centre(cov, 6..16) - centre(cov, 26..36);
    let (r, i) = (lean(&draw(&regular, 'l', Synthetic::NONE)), lean(&draw(&italic, 'l', Synthetic::NONE)));
    assert!(r.abs() < 0.6, "regular l stands upright: {r}");
    assert!(i > 1.5, "italic l leans: {i}");
}

#[test]
fn synthetic_styles_lean_and_thicken() {
    let f = Font::new(geist(false, Style::REGULAR)).unwrap();
    let plain = draw(&f, 'l', Synthetic::NONE);
    let slanted = draw(&f, 'l', Synthetic::for_style(Style::ITALIC));
    let bold = draw(&f, 'l', Synthetic::for_style(Style::BOLD));
    let lean = centre(&slanted, 6..16) - centre(&slanted, 26..36);
    assert!(lean > 1.5, "synthetic italic leans: {lean}");
    // Shearing keeps the area.
    let (p, s) = (ink(&plain) as f32, ink(&slanted) as f32);
    assert!((s - p).abs() < p * 0.05, "{p} vs {s}");
    assert!(ink(&bold) as f32 > p * 1.25, "synthetic bold thickens: {} vs {p}", ink(&bold));
    let g = f.glyph('l' as u32).unwrap();
    assert!(f.advance_synthetic(g, 32 * 64, Synthetic::for_style(Style::BOLD)) > f.advance(g, 32 * 64));
    assert_eq!(f.advance_synthetic(g, 32 * 64, Synthetic::for_style(Style::ITALIC)), f.advance(g, 32 * 64));
}

#[test]
fn synthetic_bold_keeps_counters_open() {
    // The hole in `o` narrows but stays: the middle of the glyph is still empty.
    let f = Font::new(geist(false, Style::REGULAR)).unwrap();
    let bold = draw(&f, 'o', Synthetic::for_style(Style::BOLD));
    let (mut x0, mut x1) = (W, 0);
    let row = 26;
    for x in 0..W { if bold[row * W + x] > 128 { x0 = x0.min(x); x1 = x1.max(x); } }
    let mid = (x0 + x1) / 2;
    assert!(bold[row * W + mid] < 64, "counter filled at x={mid}");
}

#[test]
fn geist_medium_and_semibold_parse_and_order_coverage_cleanly() {
    // Verify raw TTF headers + ttf-parser metadata for all 4 newly bundled faces.
    for (bytes, expected_weight) in [
        (GEIST_SANS_MEDIUM, Weight::Medium),
        (GEIST_SANS_SEMIBOLD, Weight::SemiBold),
        (GEIST_MONO_MEDIUM, Weight::Medium),
        (GEIST_MONO_SEMIBOLD, Weight::SemiBold),
    ] {
        let face = ttf_parser::Face::parse(bytes, 0).expect("ttf-parser parses Geist Medium/SemiBold");
        assert_eq!(face.weight().to_number(), expected_weight.number());
        assert!(face.number_of_glyphs() > 900);
        assert!(face.glyph_index('A').is_some());
        assert!(face.glyph_index('g').is_some());

        let font = Font::new(bytes).expect("sex_font::Font parses");
        assert_eq!(font.weight(), expected_weight);
        assert_eq!(font.weight_number(), expected_weight.number());
    }

    // Verify coverage ordering Regular < Medium < SemiBold < Bold at 12px, 14px, 17px
    // for both Geist Sans and Geist Mono.
    for mono in [false, true] {
        let regular = Font::new(geist_weight(mono, Weight::Regular, false)).unwrap();
        let medium = Font::new(geist_weight(mono, Weight::Medium, false)).unwrap();
        let semibold = Font::new(geist_weight(mono, Weight::SemiBold, false)).unwrap();
        let bold = Font::new(geist_weight(mono, Weight::Bold, false)).unwrap();

        for px in [12u32, 14, 17] {
            let cov = |font: &Font, ch: char| -> u32 {
                let g = font.glyph(ch as u32).unwrap();
                let (w, h) = (px as usize * 2, px as usize * 2);
                let mut acc = vec![0; scratch_len(w, h)];
                let mut out = vec![0u8; w * h];
                assert!(font.draw(g, px * 64, 4 * 256, (px as i32 * 3 / 2) * 256, w, h, &mut acc, &mut out));
                let sum = out.iter().map(|&v| v as u32).sum::<u32>();
                assert!(sum > 0, "non-empty bitmap for {ch} at {px}px");
                sum
            };

            for ch in ['H', 'a', 'e', 'n', '0'] {
                let r = cov(&regular, ch);
                let m = cov(&medium, ch);
                let s = cov(&semibold, ch);
                let b = cov(&bold, ch);
                assert!(
                    m > r,
                    "mono={mono} {px}px '{ch}': Medium ({m}) must exceed Regular ({r})"
                );
                assert!(
                    s > m,
                    "mono={mono} {px}px '{ch}': SemiBold ({s}) must exceed Medium ({m})"
                );
                assert!(
                    s < b,
                    "mono={mono} {px}px '{ch}': SemiBold ({s}) must be lighter than Bold ({b})"
                );
            }
        }
    }
}
