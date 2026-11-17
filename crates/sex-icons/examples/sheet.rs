// SPDX-License-Identifier: GPL-3.0-only
//! Every icon drawn natively at one size on charcoal, to compare with
//! assets/icons/preview.png (the SVG rendering).
//! `cargo run -p sex-icons --example sheet -- out.ppm [side]`
use sex_icons::{all, draw, scratch, Kind, Palette};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "icons.ppm".into());
    let side: usize = args.next().map_or(64, |s| s.parse().unwrap());
    let icons: Vec<_> = all().collect();
    let cols = 12;
    let cell = side + side / 4;
    let rows = icons.len().div_ceil(cols);
    let (w, h) = (cols * cell + side / 4, rows * cell + side / 4);
    let mut img = vec![0x221e1fu32; w * h];
    let (a, m) = scratch(side);
    let (mut acc, mut mask, mut px) = (vec![0; a], vec![0; m], vec![0u32; side * side]);
    for (n, (kind, name, _)) in icons.iter().enumerate() {
        let pal = if *kind == Kind::Line { Palette::with_ink(0xe2a084) } else { Palette::BRAND };
        assert!(draw(*kind, name, side, &pal, &mut acc, &mut mask, &mut px), "{name}");
        let (ox, oy) = (side / 4 + (n % cols) * cell, side / 4 + (n / cols) * cell);
        for y in 0..side { for x in 0..side {
            let p = px[y * side + x];
            let al = p >> 24;
            let d = &mut img[(oy + y) * w + ox + x];
            let ch = |s: u32| ((((p >> s) & 255) * al + ((*d >> s) & 255) * (255 - al)) / 255) << s;
            *d = ch(16) | ch(8) | ch(0);
        }}
    }
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in img { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]); }
    std::fs::write(path, out).unwrap();
}
