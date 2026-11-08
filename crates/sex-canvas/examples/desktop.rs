//! Render the demo scene to a binary PPM, for eyeballing or as a gate's reference:
//! `cargo run -p sex-canvas --example desktop -- out.ppm [width height dx dy]`
use sex_canvas::{demo, Canvas};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().cloned().unwrap_or_else(|| "desktop.ppm".into());
    let num = |i: usize, d: f32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (w, h) = (num(1, 800.0) as u32, num(2, 500.0) as u32);
    let mut buf = vec![0u32; (w * h) as usize];
    let mut c = Canvas::new(&mut buf, w, h).unwrap();
    demo(&mut c, num(3, 0.0), num(4, 0.0));
    let mut out = format!("P6 {w} {h} 255\n").into_bytes();
    for p in c.pixels() {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    std::fs::write(&path, out).unwrap();
    println!("wrote {path}");
}
