// SPDX-License-Identifier: GPL-3.0-only
//! The program loop: window events in, egui frames out.
use std::time::{Duration, Instant};

use sex_std::sex_app::{self, Event as WinEvent};
use sex_std::sex_pdx::app_pointer::Kind;

use crate::keys::{self, PointerKind};
use crate::{raster, raw_input, Options, Painter};

/// Runs `ui` every frame with the window's root [`egui::Ui`], until the window closes.
pub fn run_ui(options: Options, mut ui: impl FnMut(&mut egui::Ui)) -> ! {
    run_inner(options, &mut |ctx, raw| ctx.run_ui(raw, |u| ui(u)))
}

/// Runs `update` every frame with the [`egui::Context`] (panels, windows, areas).
pub fn run_ctx(options: Options, mut update: impl FnMut(&egui::Context)) -> ! {
    run_inner(options, &mut |ctx, raw| ctx.run_pass(raw, |c| update(c)))
}

/// A frame no later than this after the last when egui animates.
const FRAME: Duration = Duration::from_millis(16);

fn run_inner(options: Options, frame: &mut dyn FnMut(&egui::Context, egui::RawInput) -> egui::FullOutput) -> ! {
    let window = sex_std::window();
    window.title(options.title.as_bytes());
    let ctx = egui::Context::default();
    let mut painter = Painter::new();
    let start = Instant::now();
    let mut clipboard = String::new();
    let mut modifiers = egui::Modifiers::NONE;
    let mut events: Vec<egui::Event> = Vec::new();
    let mut frames: u64 = 0;
    // When egui wants the next frame; None waits for input.
    let mut due: Option<Instant> = Some(start);
    let (target, task) = sex_std::identity();
    sex_std::sex_pdx::serial_record!("[egui.ready] target={} task={} scale={}", target, task, options.scale);
    loop {
        // Wait for input or the frame egui asked for, then take everything waiting.
        let first = match due {
            Some(at) => {
                let now = Instant::now();
                if at > now { window.next_within((at - now).as_millis().max(1) as u64) } else { window.poll() }
            }
            None => Some(window.next()),
        };
        let mut closing = false;
        let mut resized = false;
        let mut next = first;
        while let Some(event) = next {
            let ppp = options.scale * ctx.zoom_factor();
            match event {
                WinEvent::Close => closing = true,
                WinEvent::Resize { .. } => resized = true,
                WinEvent::Key(key) => {
                    modifiers = keys::modifiers(key.shift, key.control);
                    keys::key_events(key.code, key.shift, key.control, sex_app::text(key), &clipboard, &mut events);
                }
                WinEvent::Pointer(p) => {
                    let kind = match p.kind {
                        Kind::Move => PointerKind::Move,
                        Kind::Down { button } => PointerKind::Down(button),
                        Kind::Up { button } => PointerKind::Up(button),
                        Kind::Wheel { steps } => PointerKind::Wheel(steps),
                        Kind::Leave => PointerKind::Leave,
                    };
                    keys::pointer_events(p.x, p.y, kind, ppp, modifiers, &mut events);
                }
            }
            next = window.poll();
        }
        if closing {
            sex_std::sex_pdx::serial_record!("[egui.closed] target={} task={} frames={}", target, task, frames);
            window_close();
        }
        if !resized && events.is_empty() && due.map_or(true, |at| at > Instant::now()) { continue; }

        let (width, height) = window.size();
        let began = Instant::now();
        let raw = raw_input(width, height, options.scale, ctx.zoom_factor(), start.elapsed().as_secs_f64(), std::mem::take(&mut events));
        let output = frame(&ctx, raw);
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command { clipboard = text.clone(); }
        }
        let viewport = output.viewport_output.get(&egui::ViewportId::ROOT);
        let repaint = viewport.map_or(Duration::MAX, |v| v.repaint_delay);
        let mut close_asked = false;
        if let Some(v) = viewport {
            for c in &v.commands {
                match c {
                    egui::ViewportCommand::Close => close_asked = true,
                    egui::ViewportCommand::Title(t) => window.title(t.as_bytes()),
                    _ => {}
                }
            }
        }
        let pixels = window.pixels();
        let (w, h) = (width.min(pixels.len() / height.max(1)), height);
        let stats = painter.paint(&ctx, output, &mut raster::Target { pixels, width: w, height: h, stride: w });
        window.show_pixels(w, h);
        frames += 1;
        if frames == 1 || options.trace_frames {
            sex_std::sex_pdx::serial_record!(
                "[egui.frame] n={} w={} h={} meshes={} triangles={} pixels={} ms={}",
                frames, w, h, stats.meshes, stats.triangles, stats.pixels, began.elapsed().as_millis()
            );
        }
        if close_asked {
            sex_std::sex_pdx::serial_record!("[egui.closed] target={} task={} frames={}", target, task, frames);
            window_close();
        }
        due = if repaint == Duration::ZERO { Some(began + FRAME) } else if repaint == Duration::MAX { None } else { Some(Instant::now() + repaint) };
    }
}

fn window_close() -> ! { sex_std::sex_pdx::task_finish(0) }
