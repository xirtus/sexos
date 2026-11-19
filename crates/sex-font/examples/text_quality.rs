// SPDX-License-Identifier: GPL-3.0-only
//! Before/after sheet of interface text: the plain outline mixed in sRGB (how text
//! was drawn) beside light hinting, stem darkening and the linear-light blend
//! (`Render::UI` + `gamma::blend`), in the light and dark themes.
//!
//! `cargo run -p sex-font --example text_quality -- out.ppm [scale]`
use sex_font::{gamma, Font, Render, Synthetic, GEIST_SANS};

const LINES: &[(&str, u32)] = &[
    ("Files  Settings  Kaleidoscope  Qupid  Spindle", 11),
    ("The quick brown fox jumps over the lazy dog 0123456789", 12),
    ("Silkbar: Wi-Fi connected, battery 82%, Tue 17 Nov 10:45", 13),
    ("Assuit - Untitled document (edited) - Spreadeagle", 14),
    ("Hamburgefonstiv  illegible minimum 1Il|", 16),
    ("Light hinting and stem darkening", 20),
];

fn plain_blend(fg: u32, bg: u32, a: u8) -> u32 {
    let (a, inv) = (a as u32, 255 - a as u32);
    let mix = |s: u32| ((((fg >> s) & 0xFF) * a + ((bg >> s) & 0xFF) * inv) / 255) << s;
    mix(16) | mix(8) | mix(0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "text_quality.ppm".into());
    let scale: u32 = args.next().map_or(1, |s| s.parse().unwrap());
    let font = Font::new(GEIST_SANS).unwrap();
    let col_w = 470 * scale as usize;
    let row_h: usize = LINES.iter().map(|(_, px)| (*px * scale) as usize * 2).sum::<usize>() + 24 * scale as usize;
    let (w, h) = (col_w * 2, row_h * 2);
    let mut img = vec![0u32; w * h];
    let themes = [(0xf7f4f2u32, 0x1f1b1cu32), (0x231f20, 0xf2ebe7)];
    for (t, (bg, fg)) in themes.iter().enumerate() {
        for (c, after) in [false, true].into_iter().enumerate() {
            let (ox, oy) = (c * col_w, t * row_h);
            for y in oy..oy + row_h { for x in ox..ox + col_w { img[y * w + x] = *bg; } }
            let mut base = oy + 12 * scale as usize;
            for (text, px) in LINES {
                let size = px * scale * 64;
                base += (*px * scale) as usize * 2;
                let render = if after { Render::ui_for(*fg) } else { Render::PLAIN };
                let mut pen = (ox + 10 * scale as usize) as i32 * 256;
                let mut prev = None;
                for ch in text.chars() {
                    let Some(g) = font.glyph(ch as u32) else { pen += size as i32 * 4 / 3; continue };
                    if let Some(p) = prev { pen += font.kerning(p, g, size); }
                    let (gw, gh) = ((size / 64 * 2) as usize, (size / 64 * 2) as usize);
                    let mut acc = vec![0; sex_font::raster::scratch_len(gw, gh)];
                    let mut cov = vec![0u8; gw * gh];
                    let gx = pen / 256 - (size / 64 / 2) as i32;
                    let top = base as i32 - (size / 64 * 3 / 2) as i32;
                    let local_x = pen - gx * 256;
                    let local_base = (base as i32 - top) * 256;
                    font.draw_rendered(g, size, local_x, local_base, gw, gh, &mut acc, &mut cov, Synthetic::NONE, render);
                    for yy in 0..gh { for xx in 0..gw {
                        let a = cov[yy * gw + xx];
                        if a == 0 { continue; }
                        let (px_x, px_y) = (gx + xx as i32, top + yy as i32);
                        if px_x < 0 || px_y < 0 || px_x as usize >= w || px_y as usize >= h { continue; }
                        let i = px_y as usize * w + px_x as usize;
                        img[i] = if after { gamma::blend(*fg, img[i], a) } else { plain_blend(*fg, img[i], a) };
                    }}
                    pen += font.advance(g, size);
                    prev = Some(g);
                }
            }
        }
    }
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in img { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]); }
    std::fs::write(path, out).unwrap();
}
