// SPDX-License-Identifier: GPL-3.0-only
//! FilmCraft's engine, headless, on SexOS: open FilmCraft's demo project, look at its
//! sequence, make an edit and undo it, then export part of it as H.264 MP4 and as WAV
//! with FilmCraft's own pure-Rust encoders. Each step prints one `[splicy.probe.*]` line
//! for the QEMU gate. Splicy's window follows once egui runs on SexOS.
// SexOS's std runtime: program entry, files, threads and time.
#[cfg(target_os = "hermit")]
extern crate sex_std;

use filmcraft_engine::Session;
use serde_json::{json, Value};
use std::time::Instant;

/// One line for the person (stdout) and, on SexOS, for the serial record the gate reads.
fn say(line: String) {
    println!("{line}");
    #[cfg(target_os = "hermit")]
    sex_std::record(&line);
}

fn step(s: &mut Session, id: &str, params: Value) -> Value {
    let t = Instant::now();
    match s.execute(id, params) {
        Ok(v) => {
            say(format!("[splicy.probe.ok] {id} ms={}", t.elapsed().as_millis()));
            v
        }
        Err(e) => {
            say(format!("[splicy.probe.failed] {id} {e}"));
            std::process::exit(1);
        }
    }
}

fn size(path: &str) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn main() {
    let seconds: f64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(2.0);
    let mut s = Session::default();
    step(&mut s, "file.openDemoProject", json!({}));
    let all = step(&mut s, "edit.selectAll", json!({}));
    say(format!("[splicy.probe.project] selected={}", all.to_string().len()));
    step(&mut s, "edit.rippleDelete", json!({}));
    step(&mut s, "edit.undo", json!({}));
    step(&mut s, "edit.deselectAll", json!({}));
    for (path, format) in [("probe.wav", "wav"), ("probe.mp4", "h264")] {
        let t = Instant::now();
        let r = step(&mut s, "file.exportMedia", json!({"path": path, "format": format, "range": "custom", "startSeconds": 0, "endSeconds": seconds, "wait": true}));
        let ok = r["result"].is_object();
        say(format!("[splicy.probe.export] format={format} file={path} bytes={} ms={} result={}", size(path), t.elapsed().as_millis(), ok as u8));
    }
    say(format!("[splicy.probe.done]"));
}
