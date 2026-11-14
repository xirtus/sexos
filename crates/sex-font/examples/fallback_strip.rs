//! Writes a PGM strip of fallback characters: cargo run -p sex-font --example fallback_strip -- out.pgm 24 36
use sex_font::fallback::Fallback;
use sex_font::raster::scratch_len;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (w, h): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let text = "Жизнь Ωmega 日本語 你好 한국어 あいう 😀👍❤🚀🐱 ♞⣿";
    let f = Fallback::embedded().unwrap();
    let chars: Vec<char> = text.chars().collect();
    let mut img = vec![255u8; w * chars.len() * h];
    let mut acc = vec![0; scratch_len(w, h)];
    let mut cell = vec![0u8; w * h];
    for (i, c) in chars.iter().enumerate() {
        if !f.draw_cell(*c as u32, w, h, &mut acc, &mut cell) { continue; }
        for y in 0..h { for x in 0..w { img[y * w * chars.len() + i * w + x] = 255 - cell[y * w + x]; } }
    }
    let mut out = format!("P5 {} {} 255\n", w * chars.len(), h).into_bytes();
    out.extend(img);
    std::fs::write(&a[1], out).unwrap();
}
