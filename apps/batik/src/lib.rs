// SPDX-License-Identifier: GPL-3.0-only
//! Batik: Weft Image Editor & Digital Painting App over `batik-core`.

#![cfg_attr(feature = "sexos", no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use batik_core::adjust::Adjustment;
use batik_core::blend::{split_argb, BlendMode};
use batik_core::brush::{
    bucket_fill, bucket_fill_mask, Brush, BrushPreset, PointerSample, StrokeEngine,
};
use batik_core::document::{BackgroundMode, Document};
use batik_core::error::BatikError;
use batik_core::filters::{blend_with_mask, box_blur, emboss, sharpen};
use batik_core::geom::Rect;
use batik_core::history::{History, HistoryAction};
use batik_core::jpeg::encode_jpeg;
use batik_core::layer::LayerKind;
use batik_core::ora::{read_ora, write_ora};
use batik_core::png::encode_rgba_png;
use batik_core::psd::{encode_psd, parse_psd};
use batik_core::selection::{SelectionCombine, SelectionMask};
use batik_core::webp::encode_webp;

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
    Lasso,
    MagicWand,
}

/// Simulated pressure dynamics curve along brush strokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressureCurve {
    Taper,
    Full,
    Light,
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
    pub pressure_curve: PressureCurve,
    pub edit_mask: bool,
    pub wand_tolerance: u8,

    pub is_painting: bool,
    pub stroke_step: usize,
    pub pre_stroke_pixels: Option<Vec<u32>>,
    pub pre_stroke_mask: Option<Vec<u8>>,
    pub select_start: Option<(f32, f32)>,
    pub lasso_points: Vec<(i32, i32)>,
    pub ants_phase: usize,

    // Widget IDs - Canvas
    pub canvas_id: Id,

    // Widget IDs - Tools
    pub tool_brush: Id,
    pub tool_eraser: Id,
    pub tool_bucket: Id,
    pub tool_select: Id,
    pub tool_lasso: Id,
    pub tool_wand: Id,

    // Widget IDs - Brush Presets & Curves
    pub btn_preset_soft: Id,
    pub btn_preset_hard: Id,
    pub btn_preset_chisel: Id,
    pub btn_preset_eraser: Id,
    pub btn_curve_taper: Id,
    pub btn_curve_full: Id,
    pub btn_curve_light: Id,

    // Widget IDs - Selections
    pub btn_invert_sel: Id,
    pub btn_feather: Id,
    pub btn_deselect: Id,

    // Widget IDs - Filters
    pub btn_blur: Id,
    pub btn_sharpen: Id,
    pub btn_emboss: Id,
    pub btn_sepia: Id,
    pub btn_posterize: Id,
    pub btn_invert: Id,
    pub btn_auto_levels: Id,

    // Widget IDs - Layers & Blend Modes
    pub btn_new_layer: Id,
    pub btn_dup_layer: Id,
    pub btn_merge_down: Id,
    pub btn_delete_layer: Id,
    pub btn_toggle_visibility: Id,
    pub btn_opacity_down: Id,
    pub btn_opacity_up: Id,
    pub btn_blend_cycle: Id,
    pub btn_blend_normal: Id,
    pub btn_blend_multiply: Id,
    pub btn_blend_screen: Id,
    pub btn_blend_overlay: Id,
    pub btn_blend_diff: Id,
    pub btn_blend_softlight: Id,
    pub btn_layer_prev: Id,
    pub btn_layer_next: Id,

    // Widget IDs - Layer Masks
    pub btn_add_mask: Id,
    pub btn_mask_from_sel: Id,
    pub btn_invert_mask: Id,
    pub btn_edit_mask: Id,

    // Widget IDs - Codecs & IO
    pub btn_export_png: Id,
    pub btn_export_jpg: Id,
    pub btn_export_webp: Id,
    pub btn_export_psd: Id,
    pub btn_save_ora: Id,
    pub btn_import_png: Id,
    pub btn_import_jpg: Id,
    pub btn_import_webp: Id,
    pub btn_import_psd: Id,

    // Widget IDs - History & UI
    pub btn_undo: Id,
    pub btn_redo: Id,
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
        let brush = Brush::from_preset(BrushPreset::HardRound);
        let stroke_engine = StrokeEngine::new(brush);

        // Register commands and shortcuts
        for c in [
            Command::new("batik.export-png", "Export PNG").shortcut("Ctrl+S"),
            Command::new("batik.export-jpg", "Export JPEG"),
            Command::new("batik.export-webp", "Export WebP"),
            Command::new("batik.export-psd", "Export PSD"),
            Command::new("batik.save-ora", "Save ORA"),
            Command::new("batik.import-png", "Import PNG"),
            Command::new("batik.import-jpg", "Import JPEG"),
            Command::new("batik.import-webp", "Import WebP"),
            Command::new("batik.import-psd", "Import PSD"),
            Command::new("batik.undo", "Undo").shortcut("Ctrl+Z"),
            Command::new("batik.redo", "Redo").shortcut("Ctrl+Y"),
            Command::new("batik.invert", "Invert").shortcut("Ctrl+I"),
            Command::new("batik.auto-levels", "Auto-Levels").shortcut("Ctrl+L"),
            Command::new("batik.blur", "Blur"),
            Command::new("batik.sharpen", "Sharpen"),
            Command::new("batik.emboss", "Emboss"),
            Command::new("batik.sepia", "Sepia"),
            Command::new("batik.posterize", "Posterize"),
            Command::new("batik.new-layer", "New Layer").shortcut("Ctrl+Shift+N"),
            Command::new("batik.dup-layer", "Duplicate Layer"),
            Command::new("batik.merge-down", "Merge Down").shortcut("Ctrl+E"),
            Command::new("batik.delete-layer", "Delete Layer"),
            Command::new("batik.add-mask", "Add Mask"),
            Command::new("batik.mask-from-sel", "Mask from Selection"),
            Command::new("batik.invert-mask", "Invert Mask"),
            Command::new("batik.edit-mask", "Edit Mask Toggle"),
            Command::new("batik.brush", "Brush"),
            Command::new("batik.eraser", "Eraser"),
            Command::new("batik.bucket", "Bucket Fill"),
            Command::new("batik.select", "Rect Select"),
            Command::new("batik.lasso", "Lasso Select"),
            Command::new("batik.wand", "Magic Wand"),
            Command::new("batik.deselect", "Deselect").shortcut("Ctrl+D"),
            Command::new("batik.invert-sel", "Invert Selection"),
            Command::new("batik.feather", "Feather Selection"),
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
                    .command("batik.export-png")
                    .command("batik.export-jpg")
                    .command("batik.export-webp")
                    .command("batik.export-psd")
                    .command("batik.save-ora")
                    .separator()
                    .command("batik.import-png")
                    .command("batik.import-jpg")
                    .command("batik.import-webp")
                    .command("batik.import-psd")
                    .separator()
                    .command("weft.close"),
                Menu::new("Edit")
                    .command("batik.undo")
                    .command("batik.redo")
                    .separator()
                    .command("batik.invert")
                    .command("batik.auto-levels")
                    .command("batik.blur")
                    .command("batik.sharpen")
                    .command("batik.emboss")
                    .command("batik.sepia")
                    .command("batik.posterize"),
                Menu::new("Select")
                    .command("batik.deselect")
                    .command("batik.invert-sel")
                    .command("batik.feather"),
                Menu::new("Layer")
                    .command("batik.new-layer")
                    .command("batik.dup-layer")
                    .command("batik.merge-down")
                    .command("batik.delete-layer")
                    .separator()
                    .command("batik.add-mask")
                    .command("batik.mask-from-sel")
                    .command("batik.invert-mask")
                    .command("batik.edit-mask"),
            ],
        );

        // Toolbar Row 1: Tools, Presets, Pressure Curves, History
        let row_tools = ui.row(root);
        let tool_brush = ui.button(row_tools, "Brush");
        let tool_eraser = ui.button(row_tools, "Eraser");
        let tool_bucket = ui.button(row_tools, "Bucket");
        let tool_select = ui.button(row_tools, "Select");
        let tool_lasso = ui.button(row_tools, "Lasso");
        let tool_wand = ui.button(row_tools, "Wand");

        ui.separator(row_tools);

        let btn_preset_soft = ui.button(row_tools, "Soft");
        let btn_preset_hard = ui.button(row_tools, "Hard");
        let btn_preset_chisel = ui.button(row_tools, "Chisel");
        let btn_preset_eraser = ui.button(row_tools, "EraserB");

        ui.separator(row_tools);

        let btn_curve_taper = ui.button(row_tools, "Taper");
        let btn_curve_full = ui.button(row_tools, "Full");
        let btn_curve_light = ui.button(row_tools, "Light");

        ui.separator(row_tools);

        let btn_undo = ui.button(row_tools, "Undo");
        let btn_redo = ui.button(row_tools, "Redo");

        // Toolbar Row 2: Selections and Spatial/Color Filters
        let row_filters = ui.row(root);
        let btn_invert_sel = ui.button(row_filters, "InvSel");
        let btn_feather = ui.button(row_filters, "Feather");
        let btn_deselect = ui.button(row_filters, "Desel");

        ui.separator(row_filters);

        let btn_blur = ui.button(row_filters, "Blur");
        let btn_sharpen = ui.button(row_filters, "Sharpen");
        let btn_emboss = ui.button(row_filters, "Emboss");
        let btn_sepia = ui.button(row_filters, "Sepia");
        let btn_posterize = ui.button(row_filters, "Posterize");
        let btn_invert = ui.button(row_filters, "Invert");
        let btn_auto_levels = ui.button(row_filters, "AutoLevels");

        // Toolbar Row 3: Layers & Blend Modes
        let row_layers = ui.row(root);
        let btn_new_layer = ui.button(row_layers, "NewLayer");
        let btn_dup_layer = ui.button(row_layers, "DupLayer");
        let btn_merge_down = ui.button(row_layers, "MergeDown");
        let btn_delete_layer = ui.button(row_layers, "DelLayer");
        let btn_toggle_visibility = ui.button(row_layers, "Visible");
        let btn_opacity_down = ui.button(row_layers, "Opac-");
        let btn_opacity_up = ui.button(row_layers, "Opac+");

        ui.separator(row_layers);

        let btn_blend_cycle = ui.button(row_layers, "BlendCycle");
        let btn_blend_normal = ui.button(row_layers, "Norm");
        let btn_blend_multiply = ui.button(row_layers, "Mul");
        let btn_blend_screen = ui.button(row_layers, "Scrn");
        let btn_blend_overlay = ui.button(row_layers, "Over");
        let btn_blend_diff = ui.button(row_layers, "Diff");
        let btn_blend_softlight = ui.button(row_layers, "Soft");

        // Toolbar Row 4: Layer Masks, Navigation, Codecs
        let row_codecs = ui.row(root);
        let btn_add_mask = ui.button(row_codecs, "AddMask");
        let btn_mask_from_sel = ui.button(row_codecs, "MaskSel");
        let btn_invert_mask = ui.button(row_codecs, "InvMask");
        let btn_edit_mask = ui.button(row_codecs, "EditMask");

        ui.separator(row_codecs);

        let btn_layer_prev = ui.button(row_codecs, "LyrPrev");
        let btn_layer_next = ui.button(row_codecs, "LyrNext");

        ui.separator(row_codecs);

        let btn_export_png = ui.button(row_codecs, "ExpPNG");
        let btn_export_jpg = ui.button(row_codecs, "ExpJPG");
        let btn_export_webp = ui.button(row_codecs, "ExpWebP");
        let btn_export_psd = ui.button(row_codecs, "ExpPSD");
        let btn_save_ora = ui.button(row_codecs, "SaveORA");

        // Color Swatches & Import bar
        let color_row = ui.row(root);
        ui.label(color_row, "Color:");
        let swatch_black = ui.button(color_row, "Black");
        let swatch_red = ui.button(color_row, "Red");
        let swatch_green = ui.button(color_row, "Green");
        let swatch_blue = ui.button(color_row, "Blue");
        let swatch_yellow = ui.button(color_row, "Yellow");
        let swatch_white = ui.button(color_row, "White");

        ui.separator(color_row);

        let btn_import_png = ui.button(color_row, "ImpPNG");
        let btn_import_jpg = ui.button(color_row, "ImpJPG");
        let btn_import_webp = ui.button(color_row, "ImpWebP");
        let btn_import_psd = ui.button(color_row, "ImpPSD");

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
        let layer_list = ui.list(sidebar, &["  [V]     Layer 1 (100%) Norm"]);
        ui.set_layout(layer_list, Layout::default().width(240.0).height(256.0));

        // Bottom status label
        let status_label = ui.label(root, "Ready");

        let controls = vec![
            // Tools
            ("brush", tool_brush),
            ("tool_brush", tool_brush),
            ("eraser", tool_eraser),
            ("tool_eraser", tool_eraser),
            ("bucket", tool_bucket),
            ("tool_bucket", tool_bucket),
            ("select", tool_select),
            ("tool_select", tool_select),
            ("lasso", tool_lasso),
            ("tool_lasso", tool_lasso),
            ("wand", tool_wand),
            ("tool_wand", tool_wand),
            // Presets
            ("preset_soft", btn_preset_soft),
            ("preset_hard", btn_preset_hard),
            ("preset_chisel", btn_preset_chisel),
            ("preset_eraser", btn_preset_eraser),
            // Pressure Curves
            ("curve_taper", btn_curve_taper),
            ("curve_full", btn_curve_full),
            ("curve_light", btn_curve_light),
            // Selections
            ("invert_sel", btn_invert_sel),
            ("feather_sel", btn_feather),
            ("deselect", btn_deselect),
            // Filters
            ("blur", btn_blur),
            ("sharpen", btn_sharpen),
            ("emboss", btn_emboss),
            ("sepia", btn_sepia),
            ("posterize", btn_posterize),
            ("invert", btn_invert),
            ("auto_levels", btn_auto_levels),
            // Layers & Blend
            ("new_layer", btn_new_layer),
            ("dup_layer", btn_dup_layer),
            ("merge_down", btn_merge_down),
            ("delete_layer", btn_delete_layer),
            ("toggle_visibility", btn_toggle_visibility),
            ("opacity_down", btn_opacity_down),
            ("opacity_up", btn_opacity_up),
            ("blend_cycle", btn_blend_cycle),
            ("blend_normal", btn_blend_normal),
            ("blend_multiply", btn_blend_multiply),
            ("blend_screen", btn_blend_screen),
            ("blend_overlay", btn_blend_overlay),
            ("blend_diff", btn_blend_diff),
            ("blend_softlight", btn_blend_softlight),
            ("layer_prev", btn_layer_prev),
            ("layer_next", btn_layer_next),
            // Layer Masks
            ("add_mask", btn_add_mask),
            ("mask_from_sel", btn_mask_from_sel),
            ("invert_mask", btn_invert_mask),
            ("edit_mask", btn_edit_mask),
            // Codecs Export
            ("export_png", btn_export_png),
            ("save_png", btn_export_png),
            ("export_jpg", btn_export_jpg),
            ("export_webp", btn_export_webp),
            ("export_psd", btn_export_psd),
            ("save_ora", btn_save_ora),
            // Codecs Import
            ("import_png", btn_import_png),
            ("import_jpg", btn_import_jpg),
            ("import_webp", btn_import_webp),
            ("import_psd", btn_import_psd),
            // History
            ("undo", btn_undo),
            ("redo", btn_redo),
            // Swatches
            ("color_black", swatch_black),
            ("color_red", swatch_red),
            ("color_green", swatch_green),
            ("color_blue", swatch_blue),
            ("color_yellow", swatch_yellow),
            ("color_white", swatch_white),
        ];

        let mut app = Self {
            doc,
            history,
            stroke_engine,
            selection,
            tool: Tool::Brush,
            active_layer_id,
            current_color: 0xFF00_0000,
            brush_size: 16.0,
            brush_opacity: 1.0,
            pressure_curve: PressureCurve::Taper,
            edit_mask: false,
            wand_tolerance: 16,
            is_painting: false,
            stroke_step: 0,
            pre_stroke_pixels: None,
            pre_stroke_mask: None,
            select_start: None,
            lasso_points: Vec::new(),
            ants_phase: 0,
            canvas_id,
            tool_brush,
            tool_eraser,
            tool_bucket,
            tool_select,
            tool_lasso,
            tool_wand,
            btn_preset_soft,
            btn_preset_hard,
            btn_preset_chisel,
            btn_preset_eraser,
            btn_curve_taper,
            btn_curve_full,
            btn_curve_light,
            btn_invert_sel,
            btn_feather,
            btn_deselect,
            btn_blur,
            btn_sharpen,
            btn_emboss,
            btn_sepia,
            btn_posterize,
            btn_invert,
            btn_auto_levels,
            btn_new_layer,
            btn_dup_layer,
            btn_merge_down,
            btn_delete_layer,
            btn_toggle_visibility,
            btn_opacity_down,
            btn_opacity_up,
            btn_blend_cycle,
            btn_blend_normal,
            btn_blend_multiply,
            btn_blend_screen,
            btn_blend_overlay,
            btn_blend_diff,
            btn_blend_softlight,
            btn_layer_prev,
            btn_layer_next,
            btn_add_mask,
            btn_mask_from_sel,
            btn_invert_mask,
            btn_edit_mask,
            btn_export_png,
            btn_export_jpg,
            btn_export_webp,
            btn_export_psd,
            btn_save_ora,
            btn_import_png,
            btn_import_jpg,
            btn_import_webp,
            btn_import_psd,
            btn_undo,
            btn_redo,
            swatches,
            layer_list,
            status_label,
            controls,
            canvas_w,
            canvas_h,
            undo_actions: 0,
            cached_flatten: flat.pixels,
        };

        app.refresh_canvas(ui);
        app
    }

    pub fn refresh_canvas(&mut self, ui: &mut Ui) {
        if let Ok(flat) = self.doc.flatten() {
            self.cached_flatten = flat.pixels.clone();

            let mut display = flat.pixels;
            if self.selection.is_active() {
                let w = self.canvas_w;
                let h = self.canvas_h;
                for y in 0..h {
                    let row = y * w;
                    for x in 0..w {
                        let s = self.selection.sample(x, y);
                        if s >= 128 {
                            let is_edge = (x == 0 || self.selection.sample(x - 1, y) < 128)
                                || (x + 1 >= w || self.selection.sample(x + 1, y) < 128)
                                || (y == 0 || self.selection.sample(x, y - 1) < 128)
                                || (y + 1 >= h || self.selection.sample(x, y + 1) < 128);
                            if is_edge {
                                let ant = ((x + y + self.ants_phase) / 3) % 2;
                                display[row + x] = if ant == 0 {
                                    0xFF00_0000
                                } else {
                                    0xFFFF_FFFF
                                };
                            }
                        }
                    }
                }
            }

            ui.set_image(self.canvas_id, self.canvas_w, self.canvas_h, display);
        }
        self.update_layer_list(ui);
        self.update_status(ui);
    }

    fn update_layer_list(&self, ui: &mut Ui) {
        let mut names = Vec::new();
        for l in self.doc.layers.iter().rev() {
            let active_mark = if l.id == self.active_layer_id { "> " } else { "  " };
            let vis_mark = if l.visible { "[V]" } else { "[H]" };
            let mask_mark = match &l.kind {
                LayerKind::Raster(r) if r.mask.is_some() => "[M]",
                _ => "   ",
            };
            let blend_name = match l.blend_mode {
                BlendMode::Normal => "Norm",
                BlendMode::Multiply => "Mul",
                BlendMode::Screen => "Scrn",
                BlendMode::Overlay => "Over",
                BlendMode::Difference => "Diff",
                BlendMode::SoftLight => "Soft",
                _ => "Oth",
            };
            names.push(format!(
                "{}{} {} {} ({}%) {}",
                active_mark,
                vis_mark,
                mask_mark,
                l.name,
                (l.opacity as u32 * 100) / 255,
                blend_name
            ));
        }
        let rows = Rows(names.into_iter().map(|s| vec![s]).collect());
        ui.set_rows(self.layer_list, rows);
    }

    fn update_status(&self, ui: &mut Ui) {
        let mask_str = if self.active_has_mask() {
            if self.edit_mask {
                "Mask (EDIT)"
            } else {
                "Mask"
            }
        } else {
            "NoMask"
        };
        let sel_str = if self.selection.is_active() {
            "Active"
        } else {
            "None"
        };
        let msg = format!(
            "Tool: {} | Curve: {:?} | Color: #{:08x} | Layers: {} (Blend: {}, {}) | Sel: {} | Undo: {}",
            self.tool_name(),
            self.pressure_curve,
            self.current_color,
            self.layer_count(),
            self.active_blend_name(),
            mask_str,
            sel_str,
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
            Tool::Lasso => "Lasso",
            Tool::MagicWand => "Wand",
        }
    }

    pub fn active_blend_name(&self) -> &'static str {
        if let Some(l) = self.doc.get_layer(self.active_layer_id) {
            match l.blend_mode {
                BlendMode::Normal => "Normal",
                BlendMode::Multiply => "Multiply",
                BlendMode::Screen => "Screen",
                BlendMode::Overlay => "Overlay",
                BlendMode::Darken => "Darken",
                BlendMode::Lighten => "Lighten",
                BlendMode::ColorDodge => "ColorDodge",
                BlendMode::ColorBurn => "ColorBurn",
                BlendMode::HardLight => "HardLight",
                BlendMode::SoftLight => "SoftLight",
                BlendMode::Difference => "Difference",
                BlendMode::Exclusion => "Exclusion",
            }
        } else {
            "Normal"
        }
    }

    pub fn active_has_mask(&self) -> bool {
        if let Some(l) = self.doc.get_layer(self.active_layer_id) {
            if let LayerKind::Raster(ref r) = l.kind {
                return r.mask.is_some();
            }
        }
        false
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

    pub fn load_document(&mut self, doc: Document, ui: &mut Ui) {
        self.canvas_w = doc.width;
        self.canvas_h = doc.height;
        self.active_layer_id = doc.layers.last().map(|l| l.id).unwrap_or(0);
        self.doc = doc;
        if let Ok(sel) = SelectionMask::new(self.canvas_w, self.canvas_h) {
            self.selection = sel;
        }
        self.refresh_canvas(ui);
    }

    fn current_stroke_pressure(&self) -> f32 {
        match self.pressure_curve {
            PressureCurve::Full => 1.0,
            PressureCurve::Light => 0.35,
            PressureCurve::Taper => {
                let p = 1.0 - (self.stroke_step as f32 * 0.10);
                p.clamp(0.15, 1.0)
            }
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
        let cx = (pt.x - cr.x).clamp(0.0, (self.canvas_w.saturating_sub(1)) as f32);
        let cy = (pt.y - cr.y).clamp(0.0, (self.canvas_h.saturating_sub(1)) as f32);

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

                        self.stroke_step = 0;
                        let pressure = self.current_stroke_pressure();
                        let sample = PointerSample::new(cx, cy, pressure);

                        if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                            if let LayerKind::Raster(ref mut raster) = l.kind {
                                if self.edit_mask {
                                    if raster.mask.is_none() {
                                        let _ = raster.add_mask(255);
                                    }
                                    self.pre_stroke_mask = raster.mask.clone();
                                    let mask_val = if is_eraser {
                                        0
                                    } else {
                                        let (_, r, g, b) = split_argb(self.current_color);
                                        ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8)
                                            as u8
                                    };
                                    self.stroke_engine.render_mask_dab(
                                        raster,
                                        &self.selection,
                                        sample,
                                        mask_val,
                                    );
                                } else {
                                    self.pre_stroke_pixels = Some(raster.pixels.clone());
                                    let _ = self.stroke_engine.paint_to(
                                        raster,
                                        &self.selection,
                                        sample,
                                    );
                                }
                            }
                        }
                        self.is_painting = true;
                        self.refresh_canvas(ui);
                        true
                    }
                    Tool::BucketFill => {
                        if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                            if let LayerKind::Raster(ref mut raster) = l.kind {
                                if self.edit_mask {
                                    if raster.mask.is_none() {
                                        let _ = raster.add_mask(255);
                                    }
                                    let old_mask = raster.mask.clone();
                                    let (_, r, g, b) = split_argb(self.current_color);
                                    let fill_val = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29)
                                        >> 8) as u8;
                                    if bucket_fill_mask(
                                        raster,
                                        cx as usize,
                                        cy as usize,
                                        fill_val,
                                        self.wand_tolerance,
                                        &self.selection,
                                    )
                                    .is_ok()
                                    {
                                        let new_mask = raster.mask.clone();
                                        self.history.push_action(HistoryAction::LayerMaskDelta {
                                            layer_id: self.active_layer_id,
                                            old_mask,
                                            new_mask,
                                        });
                                        self.undo_actions += 1;
                                        self.refresh_canvas(ui);
                                        return true;
                                    }
                                } else {
                                    let old_pixels = raster.pixels.clone();
                                    if bucket_fill(
                                        raster,
                                        cx as usize,
                                        cy as usize,
                                        self.current_color,
                                        self.wand_tolerance,
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
                        }
                        false
                    }
                    Tool::RectSelect => {
                        self.select_start = Some((cx, cy));
                        true
                    }
                    Tool::Lasso => {
                        self.lasso_points = vec![(cx as i32, cy as i32)];
                        self.is_painting = true;
                        true
                    }
                    Tool::MagicWand => {
                        let px = (cx as usize).min(self.canvas_w.saturating_sub(1));
                        let py = (cy as usize).min(self.canvas_h.saturating_sub(1));
                        self.selection.select_magic_wand(
                            &self.cached_flatten,
                            px,
                            py,
                            self.wand_tolerance,
                            true,
                            SelectionCombine::Replace,
                        );
                        self.refresh_canvas(ui);
                        true
                    }
                }
            }
            PointerKind::Move => {
                if self.is_painting {
                    match self.tool {
                        Tool::Brush | Tool::Eraser => {
                            self.stroke_step += 1;
                            let pressure = self.current_stroke_pressure();
                            let sample = PointerSample::new(cx, cy, pressure);

                            if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                                if let LayerKind::Raster(ref mut raster) = l.kind {
                                    if self.edit_mask {
                                        let mask_val = if self.tool == Tool::Eraser {
                                            0
                                        } else {
                                            let (_, r, g, b) = split_argb(self.current_color);
                                            ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8)
                                                as u8
                                        };
                                        self.stroke_engine.render_mask_dab(
                                            raster,
                                            &self.selection,
                                            sample,
                                            mask_val,
                                        );
                                    } else {
                                        let _ = self.stroke_engine.paint_to(
                                            raster,
                                            &self.selection,
                                            sample,
                                        );
                                    }
                                }
                            }
                            self.refresh_canvas(ui);
                            true
                        }
                        Tool::Lasso => {
                            self.lasso_points.push((cx as i32, cy as i32));
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            }
            PointerKind::Up(Button::Left) => {
                if self.is_painting {
                    self.is_painting = false;
                    match self.tool {
                        Tool::Brush | Tool::Eraser => {
                            self.stroke_engine.finish_stroke();
                            if self.edit_mask {
                                if let (Some(pre), Some(l)) = (
                                    self.pre_stroke_mask.take(),
                                    self.doc.get_layer(self.active_layer_id),
                                ) {
                                    if let LayerKind::Raster(ref raster) = l.kind {
                                        let post = raster.mask.clone();
                                        self.history.push_action(HistoryAction::LayerMaskDelta {
                                            layer_id: self.active_layer_id,
                                            old_mask: Some(pre),
                                            new_mask: post,
                                        });
                                        self.undo_actions += 1;
                                    }
                                }
                            } else if let (Some(pre), Some(l)) = (
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
                        }
                        Tool::Lasso => {
                            if self.lasso_points.len() >= 3 {
                                self.selection
                                    .select_lasso(&self.lasso_points, SelectionCombine::Replace);
                            }
                            self.lasso_points.clear();
                            self.refresh_canvas(ui);
                            true
                        }
                        _ => false,
                    }
                } else if let Some((sx, sy)) = self.select_start.take() {
                    let rx = libm::fminf(sx, cx) as i32;
                    let ry = libm::fminf(sy, cy) as i32;
                    let rw = (libm::fabsf(cx - sx) as usize).max(1);
                    let rh = (libm::fabsf(cy - sy) as usize).max(1);
                    self.selection
                        .select_rect(Rect::new(rx, ry, rw, rh), SelectionCombine::Replace);
                    self.refresh_canvas(ui);
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
            } else if *id == self.tool_lasso {
                command = Some("batik.lasso");
            } else if *id == self.tool_wand {
                command = Some("batik.wand");
            } else if *id == self.btn_preset_soft {
                command = Some("batik.preset-soft");
            } else if *id == self.btn_preset_hard {
                command = Some("batik.preset-hard");
            } else if *id == self.btn_preset_chisel {
                command = Some("batik.preset-chisel");
            } else if *id == self.btn_preset_eraser {
                command = Some("batik.preset-eraser");
            } else if *id == self.btn_curve_taper {
                command = Some("batik.curve-taper");
            } else if *id == self.btn_curve_full {
                command = Some("batik.curve-full");
            } else if *id == self.btn_curve_light {
                command = Some("batik.curve-light");
            } else if *id == self.btn_invert_sel {
                command = Some("batik.invert-sel");
            } else if *id == self.btn_feather {
                command = Some("batik.feather");
            } else if *id == self.btn_deselect {
                command = Some("batik.deselect");
            } else if *id == self.btn_blur {
                command = Some("batik.blur");
            } else if *id == self.btn_sharpen {
                command = Some("batik.sharpen");
            } else if *id == self.btn_emboss {
                command = Some("batik.emboss");
            } else if *id == self.btn_sepia {
                command = Some("batik.sepia");
            } else if *id == self.btn_posterize {
                command = Some("batik.posterize");
            } else if *id == self.btn_invert {
                command = Some("batik.invert");
            } else if *id == self.btn_auto_levels {
                command = Some("batik.auto-levels");
            } else if *id == self.btn_new_layer {
                command = Some("batik.new-layer");
            } else if *id == self.btn_dup_layer {
                command = Some("batik.dup-layer");
            } else if *id == self.btn_merge_down {
                command = Some("batik.merge-down");
            } else if *id == self.btn_delete_layer {
                command = Some("batik.delete-layer");
            } else if *id == self.btn_toggle_visibility {
                command = Some("batik.toggle-visibility");
            } else if *id == self.btn_opacity_down {
                command = Some("batik.opacity-down");
            } else if *id == self.btn_opacity_up {
                command = Some("batik.opacity-up");
            } else if *id == self.btn_blend_cycle {
                command = Some("batik.blend-cycle");
            } else if *id == self.btn_blend_normal {
                command = Some("batik.blend-normal");
            } else if *id == self.btn_blend_multiply {
                command = Some("batik.blend-multiply");
            } else if *id == self.btn_blend_screen {
                command = Some("batik.blend-screen");
            } else if *id == self.btn_blend_overlay {
                command = Some("batik.blend-overlay");
            } else if *id == self.btn_blend_diff {
                command = Some("batik.blend-diff");
            } else if *id == self.btn_blend_softlight {
                command = Some("batik.blend-softlight");
            } else if *id == self.btn_layer_prev {
                command = Some("batik.layer-prev");
            } else if *id == self.btn_layer_next {
                command = Some("batik.layer-next");
            } else if *id == self.btn_add_mask {
                command = Some("batik.add-mask");
            } else if *id == self.btn_mask_from_sel {
                command = Some("batik.mask-from-sel");
            } else if *id == self.btn_invert_mask {
                command = Some("batik.invert-mask");
            } else if *id == self.btn_edit_mask {
                command = Some("batik.edit-mask");
            } else if *id == self.btn_undo {
                command = Some("batik.undo");
            } else if *id == self.btn_redo {
                command = Some("batik.redo");
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
            "batik.lasso" => {
                self.tool = Tool::Lasso;
                self.update_status(ui);
                true
            }
            "batik.wand" => {
                self.tool = Tool::MagicWand;
                self.update_status(ui);
                true
            }
            "batik.preset-soft" => {
                self.stroke_engine.brush = Brush::from_preset(BrushPreset::SoftAirbrush);
                self.stroke_engine.brush.color = self.current_color;
                self.brush_size = self.stroke_engine.brush.size;
                self.brush_opacity = self.stroke_engine.brush.opacity;
                self.update_status(ui);
                true
            }
            "batik.preset-hard" => {
                self.stroke_engine.brush = Brush::from_preset(BrushPreset::HardRound);
                self.stroke_engine.brush.color = self.current_color;
                self.brush_size = self.stroke_engine.brush.size;
                self.brush_opacity = self.stroke_engine.brush.opacity;
                self.update_status(ui);
                true
            }
            "batik.preset-chisel" => {
                self.stroke_engine.brush = Brush::from_preset(BrushPreset::Calligraphy);
                self.stroke_engine.brush.color = self.current_color;
                self.brush_size = self.stroke_engine.brush.size;
                self.brush_opacity = self.stroke_engine.brush.opacity;
                self.update_status(ui);
                true
            }
            "batik.preset-eraser" => {
                self.stroke_engine.brush = Brush::from_preset(BrushPreset::Eraser);
                self.tool = Tool::Eraser;
                self.brush_size = self.stroke_engine.brush.size;
                self.brush_opacity = self.stroke_engine.brush.opacity;
                self.update_status(ui);
                true
            }
            "batik.curve-taper" => {
                self.pressure_curve = PressureCurve::Taper;
                self.update_status(ui);
                true
            }
            "batik.curve-full" => {
                self.pressure_curve = PressureCurve::Full;
                self.update_status(ui);
                true
            }
            "batik.curve-light" => {
                self.pressure_curve = PressureCurve::Light;
                self.update_status(ui);
                true
            }
            "batik.invert-sel" => {
                self.selection.invert();
                self.refresh_canvas(ui);
                true
            }
            "batik.feather" => {
                self.selection.feather(3);
                self.refresh_canvas(ui);
                true
            }
            "batik.deselect" => {
                self.selection.select_none();
                self.refresh_canvas(ui);
                true
            }
            "batik.add-mask" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_mask = raster.mask.clone();
                        if raster.add_mask(255).is_ok() {
                            let new_mask = raster.mask.clone();
                            self.history.push_action(HistoryAction::LayerMaskDelta {
                                layer_id: self.active_layer_id,
                                old_mask,
                                new_mask,
                            });
                            self.undo_actions += 1;
                            self.refresh_canvas(ui);
                            return true;
                        }
                    }
                }
                false
            }
            "batik.mask-from-sel" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_mask = raster.mask.clone();
                        if raster.mask_from_selection(&self.selection).is_ok() {
                            let new_mask = raster.mask.clone();
                            self.history.push_action(HistoryAction::LayerMaskDelta {
                                layer_id: self.active_layer_id,
                                old_mask,
                                new_mask,
                            });
                            self.undo_actions += 1;
                            self.refresh_canvas(ui);
                            return true;
                        }
                    }
                }
                false
            }
            "batik.invert-mask" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_mask = raster.mask.clone();
                        raster.invert_mask();
                        let new_mask = raster.mask.clone();
                        self.history.push_action(HistoryAction::LayerMaskDelta {
                            layer_id: self.active_layer_id,
                            old_mask,
                            new_mask,
                        });
                        self.undo_actions += 1;
                        self.refresh_canvas(ui);
                        return true;
                    }
                }
                false
            }
            "batik.edit-mask" => {
                self.edit_mask = !self.edit_mask;
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
                            if self.selection.is_active() {
                                Some(&self.selection)
                            } else {
                                None
                            },
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
            "batik.auto-levels" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        let auto = Adjustment::auto_levels(&raster.pixels);
                        Adjustment::Levels(auto).apply_to_slice(
                            &mut raster.pixels,
                            self.canvas_w,
                            if self.selection.is_active() {
                                Some(&self.selection)
                            } else {
                                None
                            },
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
            "batik.sepia" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        Adjustment::Sepia.apply_to_slice(
                            &mut raster.pixels,
                            self.canvas_w,
                            if self.selection.is_active() {
                                Some(&self.selection)
                            } else {
                                None
                            },
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
            "batik.posterize" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        Adjustment::Posterize { levels: 4 }.apply_to_slice(
                            &mut raster.pixels,
                            self.canvas_w,
                            if self.selection.is_active() {
                                Some(&self.selection)
                            } else {
                                None
                            },
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
                        let mut blended = vec![0u32; raster.pixels.len()];
                        blend_with_mask(
                            &old_pixels,
                            &blurred,
                            &mut blended,
                            self.canvas_w,
                            if self.selection.is_active() {
                                Some(&self.selection)
                            } else {
                                None
                            },
                        );
                        raster.pixels = blended;
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
            "batik.sharpen" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        let sharp = sharpen(&raster.pixels, self.canvas_w, self.canvas_h);
                        let mut blended = vec![0u32; raster.pixels.len()];
                        blend_with_mask(
                            &old_pixels,
                            &sharp,
                            &mut blended,
                            self.canvas_w,
                            if self.selection.is_active() {
                                Some(&self.selection)
                            } else {
                                None
                            },
                        );
                        raster.pixels = blended;
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
            "batik.emboss" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    if let LayerKind::Raster(ref mut raster) = l.kind {
                        let old_pixels = raster.pixels.clone();
                        let emb = emboss(&raster.pixels, self.canvas_w, self.canvas_h);
                        let mut blended = vec![0u32; raster.pixels.len()];
                        blend_with_mask(
                            &old_pixels,
                            &emb,
                            &mut blended,
                            self.canvas_w,
                            if self.selection.is_active() {
                                Some(&self.selection)
                            } else {
                                None
                            },
                        );
                        raster.pixels = blended;
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
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.dup-layer" => {
                if let Ok(new_id) = self.doc.duplicate_layer(self.active_layer_id) {
                    self.active_layer_id = new_id;
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.merge-down" => {
                if let Ok(Some(bottom_id)) = self.doc.merge_down(self.active_layer_id) {
                    self.active_layer_id = bottom_id;
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.delete-layer" => {
                if self.doc.layers.len() > 1 {
                    self.doc.remove_layer(self.active_layer_id);
                    if let Some(last) = self.doc.layers.last() {
                        self.active_layer_id = last.id;
                    }
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.toggle-visibility" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    let old_v = l.visible;
                    l.visible = !l.visible;
                    self.history.push_action(HistoryAction::LayerPropertyChange {
                        layer_id: self.active_layer_id,
                        old_opacity: l.opacity,
                        new_opacity: l.opacity,
                        old_blend: l.blend_mode,
                        new_blend: l.blend_mode,
                        old_visible: old_v,
                        new_visible: l.visible,
                    });
                    self.undo_actions += 1;
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.opacity-down" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    let old_op = l.opacity;
                    let new_op = old_op.saturating_sub(32);
                    l.opacity = new_op;
                    self.history.push_action(HistoryAction::LayerPropertyChange {
                        layer_id: self.active_layer_id,
                        old_opacity: old_op,
                        new_opacity: new_op,
                        old_blend: l.blend_mode,
                        new_blend: l.blend_mode,
                        old_visible: l.visible,
                        new_visible: l.visible,
                    });
                    self.undo_actions += 1;
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.opacity-up" => {
                if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
                    let old_op = l.opacity;
                    let new_op = old_op.saturating_add(32);
                    l.opacity = new_op;
                    self.history.push_action(HistoryAction::LayerPropertyChange {
                        layer_id: self.active_layer_id,
                        old_opacity: old_op,
                        new_opacity: new_op,
                        old_blend: l.blend_mode,
                        new_blend: l.blend_mode,
                        old_visible: l.visible,
                        new_visible: l.visible,
                    });
                    self.undo_actions += 1;
                    self.refresh_canvas(ui);
                    true
                } else {
                    false
                }
            }
            "batik.blend-cycle" => {
                let next_mode = match self
                    .doc
                    .get_layer(self.active_layer_id)
                    .map(|l| l.blend_mode)
                    .unwrap_or(BlendMode::Normal)
                {
                    BlendMode::Normal => BlendMode::Multiply,
                    BlendMode::Multiply => BlendMode::Screen,
                    BlendMode::Screen => BlendMode::Overlay,
                    BlendMode::Overlay => BlendMode::Difference,
                    BlendMode::Difference => BlendMode::SoftLight,
                    _ => BlendMode::Normal,
                };
                self.set_active_blend_mode(next_mode, ui)
            }
            "batik.blend-normal" => self.set_active_blend_mode(BlendMode::Normal, ui),
            "batik.blend-multiply" => self.set_active_blend_mode(BlendMode::Multiply, ui),
            "batik.blend-screen" => self.set_active_blend_mode(BlendMode::Screen, ui),
            "batik.blend-overlay" => self.set_active_blend_mode(BlendMode::Overlay, ui),
            "batik.blend-diff" => self.set_active_blend_mode(BlendMode::Difference, ui),
            "batik.blend-softlight" => self.set_active_blend_mode(BlendMode::SoftLight, ui),
            "batik.layer-prev" => {
                if let Some(pos) = self.doc.layers.iter().position(|l| l.id == self.active_layer_id) {
                    if pos > 0 {
                        self.active_layer_id = self.doc.layers[pos - 1].id;
                        self.refresh_canvas(ui);
                        return true;
                    }
                }
                false
            }
            "batik.layer-next" => {
                if let Some(pos) = self.doc.layers.iter().position(|l| l.id == self.active_layer_id) {
                    if pos + 1 < self.doc.layers.len() {
                        self.active_layer_id = self.doc.layers[pos + 1].id;
                        self.refresh_canvas(ui);
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub fn set_active_blend_mode(&mut self, mode: BlendMode, ui: &mut Ui) -> bool {
        if let Some(l) = self.doc.get_layer_mut(self.active_layer_id) {
            let old_b = l.blend_mode;
            l.blend_mode = mode;
            self.history.push_action(HistoryAction::LayerPropertyChange {
                layer_id: self.active_layer_id,
                old_opacity: l.opacity,
                new_opacity: l.opacity,
                old_blend: old_b,
                new_blend: mode,
                old_visible: l.visible,
                new_visible: l.visible,
            });
            self.undo_actions += 1;
            self.refresh_canvas(ui);
            true
        } else {
            false
        }
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, BatikError> {
        let flat = self.doc.flatten()?;
        encode_rgba_png(&flat.pixels, flat.width, flat.height)
    }

    pub fn encode_jpeg(&self, quality: u8) -> Result<Vec<u8>, BatikError> {
        let flat = self.doc.flatten()?;
        encode_jpeg(&flat.pixels, flat.width, flat.height, quality)
    }

    pub fn encode_webp(&self) -> Result<Vec<u8>, BatikError> {
        let flat = self.doc.flatten()?;
        encode_webp(&flat.pixels, flat.width, flat.height)
    }

    pub fn encode_psd(&self) -> Result<Vec<u8>, BatikError> {
        encode_psd(&self.doc)
    }

    pub fn encode_ora(&self) -> Result<Vec<u8>, BatikError> {
        write_ora(&self.doc)
    }

    pub fn export_bytes(&self, fmt: &str) -> Result<Vec<u8>, BatikError> {
        match fmt {
            "png" => self.encode_png(),
            "jpg" | "jpeg" => self.encode_jpeg(90),
            "webp" => self.encode_webp(),
            "psd" => self.encode_psd(),
            "ora" => self.encode_ora(),
            _ => Err(BatikError::Unsupported("unsupported format")),
        }
    }

    pub fn import_bytes(&mut self, bytes: &[u8], ui: &mut Ui) -> Result<(), BatikError> {
        let limits = self.doc.limits;
        if bytes.starts_with(b"8BPS") {
            let new_doc = parse_psd(bytes, &limits)?;
            self.load_document(new_doc, ui);
            Ok(())
        } else if bytes.starts_with(b"PK\x03\x04") {
            let new_doc = read_ora(bytes, &limits)?;
            self.load_document(new_doc, ui);
            Ok(())
        } else {
            let img = sex_image::decode(bytes, &limits)?;
            let mut new_doc = Document::new(
                img.width,
                img.height,
                BackgroundMode::Solid(0xFFFF_FFFF),
                limits,
            )?;
            let id = new_doc.add_raster_layer("Imported", BlendMode::Normal, 255)?;
            if let Some(l) = new_doc.get_layer_mut(id) {
                if let LayerKind::Raster(ref mut r) = l.kind {
                    r.pixels = img.pixels;
                }
            }
            self.load_document(new_doc, ui);
            Ok(())
        }
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
