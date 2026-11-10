// SPDX-License-Identifier: GPL-3.0-only
//! Frozen 9px Geist Sans advances, kerning and quarter-pixel coverage for pixel oracles.
fn main() {
    let font = sex_font::Font::new(sex_font::GEIST_SANS).unwrap();
    let glyphs: Vec<_> = (32..127).map(|c| font.glyph(c).unwrap()).collect();
    let csv = |v: Vec<i32>| println!("{}", v.iter().map(ToString::to_string).collect::<Vec<_>>().join(","));
    csv(glyphs.iter().map(|g| font.advance(*g, 9 * 64)).collect());
    for a in &glyphs { csv(glyphs.iter().map(|b| font.kerning(*a, *b, 9 * 64)).collect()); }
    let mut acc = vec![0; sex_font::raster::scratch_len(18, 9)];
    let mut out = vec![0u8; 18 * 9];
    for glyph in glyphs {
        for phase in 0..4 {
            assert!(font.draw(glyph, 9 * 64, 2 * 256 + phase * 64, 7 * 256, 18, 9, &mut acc, &mut out));
            println!("{}", out.iter().map(ToString::to_string).collect::<Vec<_>>().join(","));
        }
    }
}
