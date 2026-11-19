// SPDX-License-Identifier: GPL-3.0-only
//! Text with colour emoji in Weft, light and dark, at 1x and 2x.
//! `cargo run -p weft --features color-emoji --example emoji_text -- out.ppm`
use weft::paint::Painter;
use weft::text::{Face, Fonts};
use weft::theme::Theme;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "emoji.ppm".into());
    let (w, h) = (900usize, 300usize);
    let mut px = vec![0u32; w * h];
    let mut fonts = Fonts::new();
    for (i, theme) in [Theme::light(), Theme::dark()].iter().enumerate() {
        let p = theme.palette;
        let (y0, y1) = (i * h / 2, (i + 1) * h / 2);
        for y in y0..y1 { for x in 0..w { px[y * w + x] = p.window.0; } }
        let mut painter = Painter::new(&mut px, w, h, 1.0);
        let y = y0 as f32;
        painter.text(&mut fonts, Face::Sans, 14.0, 16.0, y + 34.0, "Party tonight 🎉 bring snacks 🍕🍩 and good vibes 😀💋❤️🔥", p.text);
        painter.text(&mut fonts, Face::Sans, 28.0, 16.0, y + 90.0, "Kiss 💋 Love ❤️ Fire 🔥 Wink 😉", p.text);
        painter.text(&mut fonts, Face::Sans, 12.0, 16.0, y + 125.0, "Small: 👍 ok 🙂 sure 🌈 lovely 🌙", p.text_dim);
    }
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]); }
    std::fs::write(path, out).unwrap();
}
