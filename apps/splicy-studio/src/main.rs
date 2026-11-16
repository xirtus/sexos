//! Splicy Studio: the SexOS video editor. Its editing core (core/: engine and egui
//! interface, which began as FilmCraft's MIT OR Apache-2.0 code) carries only Splicy's name;
//! sex-egui gives it the window, keys, pointer and a CPU painter, the way eframe does on a
//! desktop.
//!
//! On the host, `splicy-studio shot.png [WIDTH HEIGHT]` runs the same app through a few
//! frames and paints the last one with sex-egui's painter: what the window would show.
#[cfg(target_os = "hermit")]
extern crate sex_std;

use eframe::App;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use serde_json::{Value, json};

/// The editing session. With `--demo` it holds the demo project. Otherwise each
/// media file named (`run splicy-studio @talk.webm @talk.words`) is imported, the first
/// gets a sequence of its own, and a `.words` transcript (`START_MS END_MS WORD` lines,
/// Splicy's format) becomes that clip's transcript, which the Text panel's
/// Transcript tab edits: select words, then extract (ripple delete) or lift.
fn session() -> Session {
    let mut session = Session::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--demo") {
        match session.execute("file.openDemoProject", json!({})) {
            Ok(_) => say("[splicy.studio.demo] opened"),
            Err(e) => say(&format!("[splicy.studio.demo.failed] {e}")),
        }
        return session;
    }
    let media: Vec<&String> = args.iter().filter(|a| !a.starts_with("--") && !a.ends_with(".words") && !a.ends_with(".png") && a.parse::<usize>().is_err()).collect();
    if media.is_empty() {
        return session;
    }
    let first = match session.execute("file.import", json!({ "paths": media })) {
        Ok(r) => {
            let items: Vec<u64> = r["items"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
            say(&format!("[splicy.studio.imported] items={} errors={}", items.len(), r["errors"]));
            items.first().copied()
        }
        Err(e) => {
            say(&format!("[splicy.studio.import.failed] {e}"));
            None
        }
    };
    let Some(item) = first else { return session };
    match session.execute("file.newSequenceFromClip", json!({ "items": [item] })) {
        Ok(_) => say("[splicy.studio.sequence] made"),
        Err(e) => say(&format!("[splicy.studio.sequence.failed] {e}")),
    }
    if let Some(words) = args.iter().find(|a| a.ends_with(".words")) {
        match std::fs::read_to_string(words).map(|text| transcript(&text)) {
            Ok(t) => match session.execute("transcript.set", json!({ "item": item, "transcript": t })) {
                Ok(r) => say(&format!("[splicy.studio.transcript] words={}", r["words"])),
                Err(e) => say(&format!("[splicy.studio.transcript.failed] {e}")),
            },
            Err(e) => say(&format!("[splicy.studio.transcript.failed] {words}: {e}")),
        }
    }
    session
}

/// Splicy's `.words` lines as an engine transcript (times in the engine's ticks).
fn transcript(text: &str) -> Value {
    let tick = |ms: i64| ms * (filmcraft_time::TICKS_PER_SECOND as i64 / 1000);
    let words: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (start, end) = (f.next()?.parse::<i64>().ok()?, f.next()?.parse::<i64>().ok()?);
            let word = f.collect::<Vec<_>>().join(" ");
            (!word.is_empty() && end > start).then(|| json!({ "text": word, "start": tick(start), "end": tick(end) }))
        })
        .collect();
    json!({ "language": "en", "source": "imported", "words": words })
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
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--") && (a.ends_with(".png") || a.parse::<usize>().is_ok())).collect();
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
    // The interface loads its fonts and lays its docks out over the first frames.
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
