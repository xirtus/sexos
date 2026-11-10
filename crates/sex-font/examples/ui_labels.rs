// SPDX-License-Identifier: GPL-3.0-only
//! Independent 13px Geist Sans atlas for native UI pixel oracles.
fn main() {
    let scale: usize = std::env::args().nth(1).unwrap_or_else(|| "1".into()).parse().unwrap();
    assert!((1..=3).contains(&scale));
    let (width, height) = (32 * scale, 16 * scale);
    let font = sex_font::Font::new(sex_font::GEIST_SANS).unwrap();
    let glyphs: Vec<_> = (32..127).map(|c| font.glyph(c).unwrap()).collect();
    let csv = |v: Vec<i32>| println!("{}", v.iter().map(ToString::to_string).collect::<Vec<_>>().join(","));
    let size = (13 * scale * 64) as u32;
    csv(glyphs.iter().map(|g| font.advance(*g, size)).collect());
    for a in &glyphs { csv(glyphs.iter().map(|b| font.kerning(*a, *b, size)).collect()); }
    let mut acc = vec![0; sex_font::raster::scratch_len(width, height)];
    let mut out = vec![0u8; width * height];
    for glyph in glyphs {
        for phase in 0..4 {
            assert!(font.draw(glyph, size, (2 * scale * 256 + phase * 64) as i32,
                (13 * scale * 256) as i32, width, height, &mut acc, &mut out));
            println!("{}", out.iter().map(ToString::to_string).collect::<Vec<_>>().join(","));
        }
    }
}
