// SPDX-License-Identifier: GPL-3.0-only
//! The twenty Maya day signs drawn natively in their seal colours, to compare
//! with assets/maya/svg. `cargo run -p sex-icons --example maya_sheet -- out.ppm [side]`
use sex_icons::{maya, scratch};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "maya.ppm".into());
    let side: usize = args.next().map_or(64, |s| s.parse().unwrap());
    const COLOURS: [u32; 4] = [0xD8453E, 0xF2EFE9, 0x3F6FD1, 0xF2C230];
    let (cols, cell) = (10, side + side / 4);
    let (w, h) = (cols * cell + side / 4, 2 * cell + side / 4);
    let mut img = vec![0x221e1fu32; w * h];
    let (a, m) = scratch(side);
    let (mut acc, mut mask, mut px) = (vec![0; a], vec![0; m], vec![0u32; side * side]);
    for n in 0..20 {
        assert!(maya::draw(n as u8 + 1, side, COLOURS[n % 4], 0x1C1A1B, &mut acc, &mut mask, &mut px));
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
    for p in img { out.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]); }
    std::fs::write(path, out).unwrap();
}
