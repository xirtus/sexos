//! Render two windows on a desktop to a binary PPM, for eyeballing on a host:
//! `cargo run -p sex-canvas --example desktop -- out.ppm`
use sex_canvas::{Canvas, Color, Rect, WindowStyle};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "desktop.ppm".into());
    let (w, h) = (800u32, 500u32);
    let mut buf = vec![0u32; (w * h) as usize];
    let mut c = Canvas::new(&mut buf, w, h).unwrap();
    c.fill_round_rect_gradient(Rect::new(0.0, 0.0, w as f32, h as f32), 0.0, Color::rgb(0x2B, 0x3A, 0x67), Color::rgb(0x8E, 0x5B, 0x9C));
    let style = WindowStyle::default();
    let back = c.window(Rect::new(60.0, 50.0, 380.0, 260.0), &style, false);
    c.fill_round_rect(back.inset(20.0), 6.0, Color::rgb(0xE6, 0xE8, 0xEE));
    let front = c.window(Rect::new(300.0, 150.0, 420.0, 290.0), &style, true);
    let body = front.inset(24.0);
    c.fill_round_rect(Rect::new(body.x, body.y, 120.0, 36.0), 18.0, Color::rgb(0x3A, 0x7B, 0xF6));
    c.fill_circle(body.x + 200.0, body.y + 90.0, 50.0, Color::rgba(0xFF, 0x7A, 0x45, 220));
    c.stroke_circle(body.x + 200.0, body.y + 90.0, 64.0, 3.0, Color::rgba(0x3A, 0x7B, 0xF6, 160));
    for i in 0..6 {
        let x = body.x + 10.0 + i as f32 * 22.0;
        c.line(x, body.bottom() - 10.0, x + 60.0, body.y + 60.0, 2.5, Color::rgba(0x20, 0x20, 0x30, 200));
    }
    let mut out = format!("P6 {w} {h} 255\n").into_bytes();
    for p in c.pixels() {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    std::fs::write(&path, out).unwrap();
    println!("wrote {path}");
}
