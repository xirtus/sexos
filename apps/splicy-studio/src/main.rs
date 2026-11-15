//! Splicy Studio: FilmCraft's editor (github.com/storytold/filmcraft, MIT OR Apache-2.0)
//! on SexOS. FilmCraft's engine and egui interface run unchanged; sex-egui gives them the
//! window, keys, pointer and a CPU painter, the way eframe does on a desktop.
//!
//! On the host, `splicy-studio shot.png [WIDTH HEIGHT]` runs the same app through a few
//! frames and paints the last one with sex-egui's painter: what the window would show.
#[cfg(target_os = "hermit")]
extern crate sex_std;

use eframe::App;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;

/// FilmCraft's session; with `--demo` among the arguments, holding FilmCraft's demo project.
fn session() -> Session {
    let mut session = Session::default();
    if std::env::args().any(|a| a == "--demo") {
        match session.execute("file.openDemoProject", serde_json::json!({})) {
            Ok(_) => say("[splicy.studio.demo] opened"),
            Err(e) => say(&format!("[splicy.studio.demo.failed] {e}")),
        }
    }
    session
}

fn say(line: &str) {
    println!("{line}");
    #[cfg(target_os = "hermit")]
    sex_std::record(line);
}

/// One eframe pass: input rewrite, logic, then the interface.
fn pass(app: &mut FilmcraftApp, frame: &mut eframe::Frame, ctx: &egui::Context, mut raw: egui::RawInput) -> egui::FullOutput {
    app.raw_input_hook(ctx, &mut raw);
    ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx(), frame);
        app.ui(ui, frame);
    })
}

#[cfg(target_os = "hermit")]
fn main() {
    let mut app = FilmcraftApp::new(session());
    let mut frame = eframe::Frame::new();
    say("[splicy.studio.ready]");
    sex_egui::run_frames(sex_egui::Options::new("Splicy"), move |ctx, raw| pass(&mut app, &mut frame, ctx, raw))
}

#[cfg(not(target_os = "hermit"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--demo").collect();
    let out = args.first().cloned().unwrap_or_else(|| "splicy-studio.png".into());
    let width: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(1280);
    let height: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(800);
    let mut app = FilmcraftApp::new(session());
    let mut frame = eframe::Frame::new();
    let ctx = egui::Context::default();
    let mut painter = sex_egui::Painter::new();
    let start = std::time::Instant::now();
    let mut pixels = vec![0u32; width * height];
    let mut stats = Default::default();
    // FilmCraft loads its fonts and lays its docks out over the first frames.
    for _ in 0..12 {
        let raw = sex_egui::raw_input(width, height, 1.0, ctx.zoom_factor(), start.elapsed().as_secs_f64(), Vec::new());
        let output = pass(&mut app, &mut frame, &ctx, raw);
        stats = painter.paint(&ctx, output, &mut sex_egui::raster::Target { pixels: &mut pixels, width, height, stride: width });
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("[splicy.studio.frame] w={width} h={height} meshes={} triangles={} ms={}", stats.meshes, stats.triangles, start.elapsed().as_millis());
    let file = std::fs::File::create(&out).expect("create the screenshot");
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = pixels.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    png.write_header().and_then(|mut w| w.write_image_data(&rgb)).expect("write the screenshot");
    println!("[splicy.studio.shot] {out}");
}
