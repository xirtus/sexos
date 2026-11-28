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
pub use sex_theme::{Mode, NoirAccent};

/// A line in the serial record (standard error on the host).
pub fn record(line: &str) {
    #[cfg(target_os = "hermit")]
    sex_std::record(line);
    #[cfg(not(target_os = "hermit"))]
    eprintln!("{line}");
}

pub mod keys;
pub mod raster;
pub mod text_edit;
#[cfg(target_os = "hermit")]
mod run;
#[cfg(target_os = "hermit")]
pub use run::{run_ctx, run_frames, run_ui};

pub use text_edit::{NoirTextEdit, SexUiExt, TextEdit, TextMode};

fn clipboard_id() -> egui::Id {
    egui::Id::NULL.with("sex_egui::clipboard")
}

/// Reads the window-local clipboard mirrored into egui context storage.
pub fn clipboard_text(ctx: &egui::Context) -> String {
    ctx.data(|d| d.get_temp::<String>(clipboard_id())).unwrap_or_default()
}

/// Stores the latest clipboard text in egui context storage for context-menu Paste.
pub fn set_clipboard(ctx: &egui::Context, text: &str) {
    ctx.data_mut(|d| d.insert_temp(clipboard_id(), text.to_owned()));
}

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

#[inline]
const fn c32(xrgb: u32) -> egui::Color32 {
    egui::Color32::from_rgb(((xrgb >> 16) & 0xFF) as u8, ((xrgb >> 8) & 0xFF) as u8, (xrgb & 0xFF) as u8)
}

/// Default humanized Geist fonts for `egui`:
/// - `Proportional` / `TextStyle::Body`: `Geist-Medium` (500) with light CoreText-style hinting
/// - `FontFamily::Name("SemiBold")` / `TextStyle::Heading` / `TextStyle::Button`: `Geist-SemiBold` (600)
/// - `Monospace` / `TextStyle::Monospace`: `GeistMono-Medium` (500)
/// Preserves egui's built-in icon and monochrome emoji fallbacks.
pub fn noir_font_definitions() -> egui::FontDefinitions {
    use egui::epaint::text::{HintingTarget, SmoothHinting};
    use std::sync::Arc;

    let mut fonts = egui::FontDefinitions::default();
    let tweak = egui::FontTweak {
        hinting_target: HintingTarget::Smooth(SmoothHinting {
            light: true,
            symmetric_rendering: true,
            preserve_linear_metrics: true,
        }),
        ..Default::default()
    };

    fonts.font_data.insert(
        "Geist-Medium".to_owned(),
        Arc::new(egui::FontData::from_static(sex_font::GEIST_SANS_MEDIUM).tweak(tweak.clone())),
    );
    fonts.font_data.insert(
        "Geist-SemiBold".to_owned(),
        Arc::new(egui::FontData::from_static(sex_font::GEIST_SANS_SEMIBOLD).tweak(tweak.clone())),
    );
    fonts.font_data.insert(
        "Geist-Regular".to_owned(),
        Arc::new(egui::FontData::from_static(sex_font::GEIST_SANS).tweak(tweak.clone())),
    );
    fonts.font_data.insert(
        "GeistMono-Medium".to_owned(),
        Arc::new(egui::FontData::from_static(sex_font::GEIST_MONO_MEDIUM).tweak(tweak)),
    );

    if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        prop.insert(0, "Geist-Regular".to_owned());
        prop.insert(0, "Geist-Medium".to_owned());
    }
    let semibold_chain = {
        let mut v = vec!["Geist-SemiBold".to_owned(), "Geist-Medium".to_owned()];
        if let Some(prop) = fonts.families.get(&egui::FontFamily::Proportional) {
            for f in prop {
                if f != "Geist-Medium" && f != "Geist-Regular" {
                    v.push(f.clone());
                }
            }
        }
        v
    };
    fonts.families.insert(egui::FontFamily::Name("SemiBold".into()), semibold_chain);

    if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        mono.insert(0, "GeistMono-Medium".to_owned());
    }
    fonts
}

/// macOS Noir Obsidian Glass dark [`egui::Visuals`] paired with a curated [`NoirAccent`].
pub fn noir_visuals(accent: NoirAccent) -> egui::Visuals {
    use egui::epaint::FontColorTransferFunction;
    use egui::{CornerRadius, Shadow, Stroke};
    use sex_theme::noir as nr;

    let base = c32(nr::DARK_BASE);
    let panel = c32(nr::DARK_PANEL);
    let surface = c32(nr::DARK_SURFACE);
    let elevated = c32(nr::DARK_ELEVATED);
    let hover = c32(nr::DARK_HOVER);
    let well = c32(nr::DARK_WELL);
    let rim = c32(nr::DARK_RIM);
    let text = c32(nr::DARK_TEXT);
    let subtext = c32(nr::DARK_SUBTEXT);

    let accent_primary = c32(accent.primary());
    let accent_fill = c32(accent.fill());
    let accent_glow = c32(accent.glow());
    let on_accent = c32(accent.on_accent());

    let mut v = egui::Visuals::dark();
    v.dark_mode = true;
    v.text_options.color_transfer_function = FontColorTransferFunction::DARK_MODE_DEFAULT;
    v.panel_fill = base;
    v.window_fill = surface;
    v.window_stroke = Stroke::new(1.0, rim);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = well;
    v.faint_bg_color = panel;
    v.code_bg_color = elevated;
    v.hyperlink_color = accent_glow;
    v.warn_fg_color = egui::Color32::from_rgb(0xFB, 0xBF, 0x24);
    v.error_fg_color = c32(nr::DARK_DANGER);
    v.window_shadow = Shadow {
        offset: [0, 12],
        blur: 20,
        spread: 0,
        color: egui::Color32::from_black_alpha(120),
    };
    v.popup_shadow = Shadow {
        offset: [0, 8],
        blur: 14,
        spread: 0,
        color: egui::Color32::from_black_alpha(110),
    };
    v.selection = egui::style::Selection {
        bg_fill: accent_fill.gamma_multiply(0.32),
        stroke: Stroke::new(1.0, accent_primary),
    };
    v.text_cursor.stroke = Stroke::new(2.0, accent_primary);
    v.widgets.noninteractive.weak_bg_fill = surface;
    v.widgets.noninteractive.bg_fill = surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, rim);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, subtext);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(6);

    v.widgets.inactive.weak_bg_fill = elevated;
    v.widgets.inactive.bg_fill = elevated;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, rim);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
    v.widgets.inactive.corner_radius = CornerRadius::same(7);

    v.widgets.hovered.weak_bg_fill = hover;
    v.widgets.hovered.bg_fill = hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, egui::Color32::from_rgb(0x3B, 0x45, 0x5E));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, egui::Color32::WHITE);
    v.widgets.hovered.corner_radius = CornerRadius::same(7);

    v.widgets.active.weak_bg_fill = accent_fill;
    v.widgets.active.bg_fill = accent_fill;
    v.widgets.active.bg_stroke = Stroke::new(1.0, accent_glow);
    v.widgets.active.fg_stroke = Stroke::new(1.0, on_accent);
    v.widgets.active.corner_radius = CornerRadius::same(7);

    v.widgets.open.weak_bg_fill = elevated;
    v.widgets.open.bg_fill = surface;
    v.widgets.open.bg_stroke = Stroke::new(1.0, accent_primary);
    v.widgets.open.fg_stroke = Stroke::new(1.0, text);
    v.widgets.open.corner_radius = CornerRadius::same(7);
    v
}

/// Applies the humanized Geist fonts + macOS Noir Obsidian visuals with SexOS Crimson default accent.
pub fn apply_noir_theme(ctx: &egui::Context) {
    apply_noir_theme_with_accent(ctx, NoirAccent::default());
}

/// Applies the humanized Geist fonts + macOS Noir Obsidian visuals with `accent`.
pub fn apply_noir_theme_with_accent(ctx: &egui::Context, accent: NoirAccent) {
    use egui::{FontFamily, FontId, TextStyle};

    ctx.set_fonts(noir_font_definitions());
    let mut style = (*ctx.global_style()).clone();
    style.visuals = noir_visuals(accent);
    style.text_styles = [
        (TextStyle::Small, FontId::new(10.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Name("SemiBold".into()))),
        (TextStyle::Heading, FontId::new(18.0, FontFamily::Name("SemiBold".into()))),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_global_style(style);
}

/// Creates an [`egui::Context`] preconfigured with humanized Geist fonts & macOS Noir dark visuals.
pub fn new_context() -> egui::Context {
    let ctx = egui::Context::default();
    apply_noir_theme(&ctx);
    ctx
}
