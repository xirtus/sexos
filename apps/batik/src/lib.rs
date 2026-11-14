// SPDX-License-Identifier: GPL-3.0-only
//! Batik: Weft Image Editor & Digital Painting App over `batik-core`.

#![cfg_attr(feature = "sexos", no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use batik_core::adjust::{Adjustment, Levels};
use batik_core::blend::BlendMode;
use batik_core::brush::{bucket_fill, Brush, PointerSample, StrokeEngine};
use batik_core::document::{BackgroundMode, Document};
use batik_core::error::BatikError;
use batik_core::filters::box_blur;
use batik_core::geom::Rect;
use batik_core::history::{History, HistoryAction};
use batik_core::layer::LayerKind;
use batik_core::ora::write_ora;
use batik_core::png::encode_rgba_png;
use batik_core::selection::{SelectionCombine, SelectionMask};

pub mod ui_backend;
pub use ui_backend as weft;

use weft::geom::Point;
use weft::input::{Button, Event, PointerKind};
use weft::menu::{Command, Menu};
use weft::widget::{Id, Layout, Rows};
use weft::{Response, Ui};

/// Available painting and editing tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Brush,
    Eraser,
    BucketFill,
    RectSelect,
}

pub struct Batik {
    pub doc: Document,
    pub history: History,
    pub stroke_engine: StrokeEngine,
    pub selection: SelectionMask,
    pub tool: Tool,
    pub active_layer_id: u64,
    pub current_color: u32,
    pub brush_size: f32,
    pub brush_opacity: f32,

    pub is_painting: bool,
    pub pre_stroke_pixels: Option<Vec<u32>>,
    pub select_start: Option<(f32, f32)>,

    // Widget IDs
    pub canvas_id: Id,
    pub tool_brush: Id,
    pub tool_eraser: Id,
    pub tool_bucket: Id,
    pub tool_select: Id,
    pub btn_levels: Id,
    pub btn_invert: Id,
    pub btn_blur: Id,
    pub btn_undo: Id,
    pub btn_redo: Id,
    pub btn_new_layer: Id,
    pub btn_save_png: Id,
    pub btn_save_ora: Id,

    pub swatches: Vec<(Id, u32)>,
    pub layer_list: Id,
    pub status_label: Id,

    pub controls: Vec<(&'static str, Id)>,

    pub canvas_w: usize,
    pub canvas_h: usize,
    pub undo_actions: usize,
    pub cached_flatten: Vec<u32>,
}

impl Batik {
    pub fn build(ui: &mut Ui) -> Self {
        let canvas_w = 256;
        let canvas_h = 256;

        let limits = sex_image::Limits::PHOTO;

        let mut doc = Document::new(
            canvas_w,
            canvas_h,
            BackgroundMode::Solid(0xFFFF_FFFF),
            limits,
        )
        .expect("document creation");
        let active_layer_id = doc
            .add_raster_layer("Layer 1", BlendMode::Normal, 255)
            .expect("add base layer");

        // Fill initial layer with white for solid painting canvas
        if let Some(l) = doc.get_layer_mut(active_layer_id) {
            if let LayerKind::Raster(ref mut r) = l.kind {
                r.pixels.fill(0xFFFF_FFFF);
            }
        }

        let selection = SelectionMask::new(canvas_w, canvas_h).expect("selection mask");
        let history = History::new(8 * 1024 * 1024);
        let brush = Brush {
            size: 16.0,
            color: 0xFF00_0000,
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.8,
            spacing: 0.2,
            eraser: false,
            size_dynamics: true,
            opacity_dynamics: false,
        };
        let stroke_engine = StrokeEngine::new(brush);

        // Register commands and shortcuts
        for c in [
            Command::new("batik.save-png", "Save PNG").shortcut("Ctrl+S"),
            Command::new("batik.save-ora", "Save ORA"),
            Command::new("batik.undo", "Undo").shortcut("Ctrl+Z"),
            Command::new("batik.redo", "Redo").shortcut("Ctrl+Y"),
            Command::new("batik.invert", "Invert").shortcut("Ctrl+I"),
            Command::new("batik.levels", "Levels").shortcut("Ctrl+L"),
            Command::new("batik.blur", "Blur"),
            Command::new("batik.new-layer", "New Layer").shortcut("Ctrl+Shift+N"),
            Command::new("batik.brush", "Brush"),
            Command::new("batik.eraser", "Eraser"),
            Command::new("batik.bucket", "Bucket Fill"),
            Command::new("batik.select", "Rect Select"),
        ] {
            ui.commands.add(c);
        }

        let root = ui.root();
        ui.set_layout(root, Layout::column());

        // Top Menu Bar
        ui.menubar(
            root,
            vec![
                Menu::new("File")
                    .command("batik.save-png")
                    .command("batik.save-ora")
                    .separator()
                    .command("weft.close"),
                Menu::new("Edit")
                    .command("batik.undo")
                    .command("batik.redo")
                    .separator()
                    .command("batik.invert")
                    .command("batik.levels")
                    .command("batik.blur"),
                Menu::new("Layer").command("batik.new-layer"),
            ],
        );

        // Main Toolbar row 1: Tools & History
        let row_tools = ui.row(root);
        let tool_brush = ui.button(row_tools, "Brush");
        let tool_eraser = ui.button(row_tools, "Eraser");
        let tool_bucket = ui.button(row_tools, "Bucket");
        let tool_select = ui.button(row_tools, "Select");

        ui.separator(row_tools);

        let btn_undo = ui.button(row_tools, "Undo");
        let btn_redo = ui.button(row_tools, "Redo");

        // Main Toolbar row 2: Adjustments & File
        let row_adjust = ui.row(root);
        let btn_levels = ui.button(row_adjust, "Levels");
        let btn_invert = ui.button(row_adjust, "Invert");
        let btn_blur = ui.button(row_adjust, "Blur");

        ui.separator(row_adjust);

        let btn_new_layer = ui.button(row_adjust, "New Layer");
        let btn_save_png = ui.button(row_adjust, "Save PNG");
        let btn_save_ora = ui.button(row_adjust, "Save ORA");

        // Color Swatches bar
        let color_row = ui.row(root);
        ui.label(color_row, "Color:");
        let swatch_black = ui.button(color_row, "Black");
        let swatch_red = ui.button(color_row, "Red");
        let swatch_green = ui.button(color_row, "Green");
        let swatch_blue = ui.button(color_row, "Blue");
        let swatch_yellow = ui.button(color_row, "Yellow");
        let swatch_white = ui.button(color_row, "White");

        let swatches = vec![
            (swatch_black, 0xFF00_0000),
            (swatch_red, 0xFFFF_0000),
            (swatch_green, 0xFF00_FF00),
            (swatch_blue, 0xFF00_00FF),
            (swatch_yellow, 0xFFFF_FF00),
            (swatch_white, 0xFFFF_FFFF),
        ];

        // Central workspace row: Canvas on left, Layer stack on right
        let workspace = ui.row(root);

        // Canvas container
        let flat = doc.flatten().expect("initial flatten");
        let canvas_id = ui.image(workspace, canvas_w, canvas_h, flat.pixels.clone());
        ui.set_layout(
            canvas_id,
            Layout::default().width(canvas_w as f32).height(canvas_h as f32),
        );

        // Layers Sidebar
        let sidebar = ui.column(workspace);
        ui.label(sidebar, "Layers:");
        let layer_list = ui.list(sidebar, &["Layer 1"]);
        ui.set_layout(layer_list, Layout::default().width(120.0).height(200.0));

        // Bottom status label
        let status_label = ui.label(root, "Tool: Brush | Color: #000000 | Size: 16px | Layers: 1");

        let controls = vec![
            ("brush", tool_brush),
            ("eraser", tool_eraser),
            ("bucket", tool_bucket),
            ("select", tool_select),
            ("levels", btn_levels),
            ("invert", btn_invert),
            ("blur", btn_blur),
            ("undo", btn_undo),
            ("redo", btn_redo),
            ("new_layer", btn_new_layer),
            ("save_png", btn_save_png),
            ("save_ora", btn_save_ora),
            ("color_black", swatch_black),
            ("color_red", swatch_red),
            ("color_green", swatch_green),
            ("color_blue", swatch_blue),
            ("color_yellow", swatch_yellow),
            ("color_white", swatch_white),
        ];

        Self {
            doc,
            history,
            stroke_engine,
            selection,
            tool: Tool::Brush,
            active_layer_id,
            current_color: 0xFF00_0000,
            brush_size: 16.0,
            brush_opacity: 1.0,
            is_painting: false,
            pre_stroke_pixels: None,
            select_start: None,
            canvas_id,
            tool_brush,
            tool_eraser,
            tool_bucket,
            tool_select,
            btn_levels,
            btn_invert,
            btn_blur,
            btn_undo,
            btn_redo,
            btn_new_layer,
            btn_save_png,
            btn_save_ora,
            swatches,
            layer_list,
            status_label,
            controls,
            canvas_w,
            canvas_h,
            undo_actions: 0,
            cached_flatten: flat.pixels,
        }
    }

    pub fn refresh_canvas(&mut self, ui: &mut Ui) {
        if let Ok(flat) = self.doc.flatten() {
            self.cached_flatten = flat.pixels.clone();
            ui.set_image(self.canvas_id, self.canvas_w, self.canvas_h, flat.pixels);
        }
        self.update_status(ui);
    }

    fn update_status(&self, ui: &mut Ui) {
        let msg = format!(
            "Tool: {} | Color: #{:08x} | Size: {:.0}px | Layers: {} | Undo: {}",
            self.tool_name(),
            self.current_color,
            self.brush_size,
            self.layer_count(),
            self.undo_count()
        );
        ui.set_text(self.status_label, &msg);
    }

    pub fn layer_count(&self) -> usize {
        self.doc.layers.len()
    }

    pub fn tool_name(&self) -> &'static str {
        match self.tool {
            Tool::Brush => "Brush",
            Tool::Eraser => "Eraser",
            Tool::BucketFill => "Bucket",
            Tool::RectSelect => "Select",
        }
    }

    pub fn undo_count(&self) -> usize {
        self.undo_actions
    }

    pub fn pixel0(&self) -> u32 {
        self.cached_flatten.first().copied().unwrap_or(0)
    }

    pub fn pixel_at(&self, x: usize, y: usize) -> u32 {
        if x < self.canvas_w && y < self.canvas_h {
            self.cached_flatten[y * self.canvas_w + x]
        } else {
            0
        }
    }

    pub fn handle_event(&mut self, ui: &mut Ui, event: &Event) -> bool {
        let Event::Pointer(p) = event else {
            return false;
        };

        let scale = ui.theme().scale;
        let pt = Point::new(p.x / scale, p.y / scale);
        let cr = ui.rect(self.canvas_id);

        let in_canvas = cr.contains(pt);
        let cx = (pt.x - cr.x).clamp(0.0, (self.canvas_w - 1) as f32);
        let cy = (pt.y - cr.y).clamp(0.0, (self.canvas_h - 1) as f32);

        match p.kind {
            PointerKind::Down(Button::Left) => {
                if !in_canvas {
                    return false;
                }
                match self.tool {
                    Tool::Brush | Tool::Eraser => {
                        let is_eraser = self.tool == Tool::Eraser;
                        self.stroke_engine.brush.eraser = is_eraser;
                        self.stroke_engine.brush.color = self.current_color;
                        self.stroke_engine.brush.size = self.brush_size;
                        self.stroke_engine.brush.opacity = self.brush_opacity;

                        if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                            if let LayerKind::Raster(ref mut raster) = l.kind {
                                self.pre_stroke_pixels = Some(raster.pixels.clone());
                                let _ = self.stroke_engine.paint_to(
                                    raster,
                                    &self.selection,
                                    PointerSample::new(cx, cy, 1.0),
                                );
                            }
                        }
                        self.is_painting = true;
                        self.refresh_canvas(ui);
                        true
                    }
                    Tool::BucketFill => {
                        if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                            if let LayerKind::Raster(ref mut raster) = l.kind {
                                let old_pixels = raster.pixels.clone();
                                if bucket_fill(
                                    raster,
                                    cx as usize,
                                    cy as usize,
                                    self.current_color,
                                    16,
                                    &self.selection,
                                )
                                .is_ok()
                                {
                                    let new_pixels = raster.pixels.clone();
                                    self.history.push_action(HistoryAction::LayerPixelDelta {
                                        layer_id: self.active_layer_id,
                                        rect: Rect::new(0, 0, self.canvas_w, self.canvas_h),
                                        old_pixels,
                                        new_pixels,
                                    });
                                    self.undo_actions += 1;
                                    self.refresh_canvas(ui);
                                    return true;
                                }
                            }
                        }
                        false
                    }
                    Tool::RectSelect => {
                        self.select_start = Some((cx, cy));
                        true
                    }
                }
            }
            PointerKind::Move => {
                if self.is_painting && (self.tool == Tool::Brush || self.tool == Tool::Eraser) {
                    if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                        if let LayerKind::Raster(ref mut raster) = l.kind {
                            let _ = self.stroke_engine.paint_to(
                                raster,
                                &self.selection,
                                PointerSample::new(cx, cy, 1.0),
                            );
                        }
                    }
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            PointerKind::Up(Button::Left) => {
                if self.is_painting {
                    self.is_painting = false;
                    self.stroke_engine.finish_stroke();
                    if let (Some(pre), Some(l)) = (
                        self.pre_stroke_pixels.take(),
                        self.doc.get_layer(self.active_layer_id),
                    ) {
                        if let LayerKind::Raster(ref raster) = l.kind {
                            let post = raster.pixels.clone();
                            self.history.push_action(HistoryAction::LayerPixelDelta {
                                layer_id: self.active_layer_id,
                                rect: Rect::new(0, 0, self.canvas_w, self.canvas_h),
                                old_pixels: pre,
                                new_pixels: post,
                            });
                            self.undo_actions += 1;
                        }
                    }
                    self.refresh_canvas(ui);
                    true
                } else if let Some((sx, sy)) = self.select_start.take() {
                    let rx = libm::fminf(sx, cx) as i32;
                    let ry = libm::fminf(sy, cy) as i32;
                    let rw = (libm::fabsf(cx - sx) as usize).max(1);
                    let rh = (libm::fabsf(cy - sy) as usize).max(1);
                    self.selection.select_rect(
                        Rect::new(rx, ry, rw, rh),
                        SelectionCombine::Replace,
                    );
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn on_response(&mut self, ui: &mut Ui, response: &Response) -> bool {
        let mut command = None;

        if let Response::Clicked(id) = response {
            if *id == self.tool_brush {
                command = Some("batik.brush");
            } else if *id == self.tool_eraser {
                command = Some("batik.eraser");
            } else if *id == self.tool_bucket {
                command = Some("batik.bucket");
            } else if *id == self.tool_select {
                command = Some("batik.select");
            } else if *id == self.btn_levels {
                command = Some("batik.levels");
            } else if *id == self.btn_invert {
                command = Some("batik.invert");
            } else if *id == self.btn_blur {
                command = Some("batik.blur");
            } else if *id == self.btn_undo {
                command = Some("batik.undo");
            } else if *id == self.btn_redo {
                command = Some("batik.redo");
            } else if *id == self.btn_new_layer {
                command = Some("batik.new-layer");
            } else if *id == self.btn_save_png {
                command = Some("batik.save-png");
            } else if *id == self.btn_save_ora {
                command = Some("batik.save-ora");
            } else if let Some(&(_, color)) = self.swatches.iter().find(|(s_id, _)| s_id == id) {
                self.current_color = color;
                self.stroke_engine.brush.color = color;
                self.update_status(ui);
                return true;
            }
        }

        if let Response::Command(cmd) = response {
            command = Some(cmd.as_str());
        }

        let Some(cmd) = command else {
            return false;
        };

        match cmd {
            "batik.brush" => {
                self.tool = Tool::Brush;
                self.update_status(ui);
                true
            }
            "batik.eraser" => {
                self.tool = Tool::Eraser;
                self.update_status(ui);
                true
            }
            "batik.bucket" => {
                self.tool = Tool::BucketFill;
                self.update_status(ui);
                true
            }
            "batik.select" => {
                self.tool = Tool::RectSelect;
                self.update_status(ui);
                true
            }
            "batik.invert" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        Adjustment::Invert.apply_to_slice(
                            &mut raster.pixels,
                            self.canvas_w,
                            Some(&self.selection),
                        );
                        let new_pixels = raster.pixels.clone();
                        self.history.push_action(HistoryAction::LayerPixelDelta {
                            layer_id: self.active_layer_id,
                            rect: Rect::new(0, 0, self.canvas_w, self.canvas_h),
                            old_pixels,
                            new_pixels,
                        });
                        self.undo_actions += 1;
                        self.refresh_canvas(ui);
                        return true;
                    }
                }
                false
            }
            "batik.levels" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        Adjustment::Levels(Levels::default()).apply_to_slice(
                            &mut raster.pixels,
                            self.canvas_w,
                            Some(&self.selection),
                        );
                        let new_pixels = raster.pixels.clone();
                        self.history.push_action(HistoryAction::LayerPixelDelta {
                            layer_id: self.active_layer_id,
                            rect: Rect::new(0, 0, self.canvas_w, self.canvas_h),
                            old_pixels,
                            new_pixels,
                        });
                        self.undo_actions += 1;
                        self.refresh_canvas(ui);
                        return true;
                    }
                }
                false
            }
            "batik.blur" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        let blurred = box_blur(&raster.pixels, self.canvas_w, self.canvas_h, 2);
                        raster.pixels = blurred;
                        let new_pixels = raster.pixels.clone();
                        self.history.push_action(HistoryAction::LayerPixelDelta {
                            layer_id: self.active_layer_id,
                            rect: Rect::new(0, 0, self.canvas_w, self.canvas_h),
                            old_pixels,
                            new_pixels,
                        });
                        self.undo_actions += 1;
                        self.refresh_canvas(ui);
                        return true;
                    }
                }
                false
            }
            "batik.undo" => {
                if self.history.undo(&mut self.doc) {
                    self.undo_actions = self.undo_actions.saturating_sub(1);
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.redo" => {
                if self.history.redo(&mut self.doc) {
                    self.undo_actions += 1;
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.new-layer" => {
                let name = format!("Layer {}", self.doc.layers.len() + 1);
                if let Ok(id) = self.doc.add_raster_layer(&name, BlendMode::Normal, 255) {
                    self.active_layer_id = id;
                    let names: Vec<String> = self.doc.layers.iter().map(|l| l.name.clone()).collect();
                    let rows = Rows(names.into_iter().map(|s| vec![s]).collect());
                    ui.set_rows(self.layer_list, rows);
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, BatikError> {
        let flat = self.doc.flatten()?;
        encode_rgba_png(&flat.pixels, flat.width, flat.height)
    }

    pub fn encode_ora(&self) -> Result<Vec<u8>, BatikError> {
        write_ora(&self.doc)
    }

    pub fn step(&mut self, ui: &mut Ui, event: Event) -> Vec<Response> {
        let canvas_changed = self.handle_event(ui, &event);
        let responses = ui.handle(event);
        let mut any_changed = canvas_changed;
        for r in &responses {
            if self.on_response(ui, r) {
                any_changed = true;
            }
        }
        if any_changed {
            self.refresh_canvas(ui);
        }
        responses
    }
}
