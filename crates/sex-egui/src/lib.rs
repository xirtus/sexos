// SPDX-License-Identifier: GPL-3.0-only
//! egui on SexOS.
//!
//! A program built for `x86_64-sex-std.json` (Rust `std`, see `crates/sex-std`) runs
//! unmodified egui code in its own window:
//!
//! ```ignore
//! fn main() {
//!     let mut count = 0;
//!     sex_egui::run_ui(sex_egui::Options::new("Counter"), move |ui| {
//!         if ui.button("Add one").clicked() { count += 1; }
//!         ui.label(format!("{count}"));
//!     });
//! }
//! ```
//!
//! What this crate does, and nothing more: Shell's keys (with what the person's layout
//! typed), pointer moves, presses and wheel steps become egui input ([`keys`]); egui's
//! tessellated meshes are painted by a CPU rasterizer ([`raster`]) into the window's
//! pixel surface and shown. Copy and paste work between egui text fields in the program;
//! the Shell clipboard is reached when programs get a clipboard grant. The window's
//! pixels per point come from [`Options::scale`] (Ctrl+Plus and Ctrl+Minus zoom, as in
//! egui everywhere). Repaints happen when input arrives or egui asks for one, so an idle
//! window costs nothing.
pub use egui;

/// A line in the serial record (standard error on the host).
pub fn record(line: &str) {
    #[cfg(target_os = "hermit")]
    sex_std::record(line);
    #[cfg(not(target_os = "hermit"))]
    eprintln!("{line}");
}

pub mod keys;
pub mod raster;
#[cfg(target_os = "hermit")]
mod run;
#[cfg(target_os = "hermit")]
pub use run::{run_ctx, run_ui};

/// How the window starts.
#[derive(Clone, Debug)]
pub struct Options {
    /// The window title (Shell shows up to sixteen characters).
    pub title: String,
    /// Pixels per point. 1 on ordinary screens, 2 on HiDPI.
    pub scale: f32,
    /// A line in the serial record for every frame, for gates and profiling.
    pub trace_frames: bool,
}

impl Options {
    pub fn new(title: &str) -> Self {
        let scale = std::env::args().find_map(|a| a.strip_prefix("--scale=").and_then(|s| s.parse().ok())).unwrap_or(1.0f32);
        Self { title: title.into(), scale: scale.clamp(0.5, 4.0), trace_frames: false }
    }
}

/// One frame of egui, painted into a pixel buffer. The program loop uses it, and so do
/// host tests and screenshots: the same code paints in both places.
pub struct Painter {
    pub textures: raster::Textures,
}

impl Default for Painter {
    fn default() -> Self { Self::new() }
}

impl Painter {
    pub fn new() -> Self { Self { textures: raster::Textures::new() } }

    /// Tessellates `output` and paints it over a background of egui's panel colour.
    pub fn paint(&mut self, ctx: &egui::Context, mut output: egui::FullOutput, target: &mut raster::Target) -> raster::Stats {
        let ppp = output.pixels_per_point;
        self.textures.set(&output.textures_delta);
        target.clear(ctx.global_style().visuals.panel_fill);
        let primitives = ctx.tessellate(output.shapes, ppp);
        let stats = raster::paint(target, &primitives, &self.textures, ppp);
        self.textures.free(&mut output.textures_delta);
        stats
    }
}

/// The input for one frame of a `width` by `height` pixel window at `ppp` native
/// pixels per point, with egui's own zoom factor `zoom` on top.
pub fn raw_input(width: usize, height: usize, ppp: f32, zoom: f32, time: f64, events: Vec<egui::Event>) -> egui::RawInput {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32) / (ppp * zoom))),
        time: Some(time),
        events,
        focused: true,
        max_texture_side: Some(8192),
        ..Default::default()
    };
    if let Some(info) = raw.viewports.get_mut(&raw.viewport_id) {
        info.native_pixels_per_point = Some(ppp);
        info.focused = Some(true);
    } else {
        let info = egui::ViewportInfo { native_pixels_per_point: Some(ppp), focused: Some(true), ..Default::default() };
        raw.viewports.insert(raw.viewport_id, info);
    }
    raw
}
