// SPDX-License-Identifier: GPL-3.0-only
//! Paints egui's demo with the SexOS rasterizer and writes a PNG:
//! `cargo run -p sex-egui --example screenshot -- out.png [width height scale]`.
use sex_egui::raster::Target;
use sex_egui::{raw_input, Painter};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or_else(|| "egui-demo.png".into());
    let w: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1280);
    let h: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(800);
    let scale: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    let mut demo = egui_demo_lib::DemoWindows::default();
    let mut painter = Painter::new();
    let mut pixels = vec![0u32; w * h];
    let mut ms = 0.0;
    for i in 0..4 {
        let began = std::time::Instant::now();
        let out = ctx.run_ui(raw_input(w, h, scale, 1.0, i as f64 * 0.1, vec![]), |u| demo.ui(u));
        let stats = painter.paint(&ctx, out, &mut Target { pixels: &mut pixels, width: w, height: h, stride: w });
        ms = began.elapsed().as_secs_f64() * 1000.0;
        if i == 3 { eprintln!("{stats:?} {ms:.1} ms"); }
    }
    let _ = ms;
    let file = std::fs::File::create(&path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    let mut writer = enc.write_header().unwrap();
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    writer.write_image_data(&bytes).unwrap();
}
