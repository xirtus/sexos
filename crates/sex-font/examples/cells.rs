// SPDX-License-Identifier: GPL-3.0-only
//! Prints Geist Mono grid cells as Display draws them: `cells W H` writes one line per
//! printable ASCII character, `w*h` comma-separated coverage bytes. Pixel oracles
//! read this, so they check the screen against the very renderer Display runs.
fn main() {
    let args: Vec<usize> = std::env::args().skip(1).map(|a| a.parse().expect("W H")).collect();
    let (w, h) = (args[0], args[1]);
    let font = sex_font::Font::new(sex_font::GEIST_MONO).unwrap();
    let mut acc = vec![0; sex_font::raster::scratch_len(w, h)];
    let mut out = vec![0u8; w * h];
    for c in 32u32..127 {
        if !sex_font::draw_cell(&font, c, w, h, &mut acc, &mut out) { out.fill(0); }
        println!("{}", out.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
    }
}
