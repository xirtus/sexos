// SPDX-License-Identifier: GPL-3.0-only
//! Batik Weft window host tests: widget events, painting, layers, adjustments, undo/redo, and rendering.

use batik::weft;
use batik::weft::input::{Button, Event, Pointer, PointerKind};
use batik::weft::{Strings, Theme, Ui};
use batik::{Batik, PressureCurve, Tool};
use batik_core::layer::LayerKind;

fn click_widget(id: weft::widget::Id, ui: &mut Ui, batik: &mut Batik) {
    ui.layout();
    let c = ui.rect(id).center();
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Move,
        }),
    );
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Down(Button::Left),
        }),
    );
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Up(Button::Left),
        }),
    );
}

fn drag_stroke(
    ui: &mut Ui,
    batik: &mut Batik,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    steps: usize,
) {
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: x1,
            y: y1,
            kind: PointerKind::Down(Button::Left),
        }),
    );
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let cx = x1 + t * (x2 - x1);
        let cy = y1 + t * (y2 - y1);
        let _ = batik.step(
            ui,
            Event::Pointer(Pointer {
                x: cx,
                y: cy,
                kind: PointerKind::Move,
            }),
        );
    }
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: x2,
            y: y2,
            kind: PointerKind::Up(Button::Left),
        }),
    );
}

#[test]
fn test_batik_window_ui_interactions() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    // 1. Initial State
    assert_eq!(batik.layer_count(), 1);
    assert_eq!(batik.tool, Tool::Brush);
    assert_eq!(batik.undo_count(), 0);
    assert_eq!(batik.pixel0(), 0xFFFF_FFFF); // initial solid white

    // 2. Select Red Color
    let swatch_red = batik
        .controls
        .iter()
        .find(|(name, _)| *name == "color_red")
        .unwrap()
        .1;
    click_widget(swatch_red, &mut ui, &mut batik);
    assert_eq!(batik.current_color, 0xFFFF_0000);

    // 3. Paint a brush stroke on the canvas
    let cr = ui.rect(batik.canvas_id);
    let start_x = cr.x + 20.0;
    let start_y = cr.y + 20.0;

    drag_stroke(&mut ui, &mut batik, start_x, start_y, start_x + 10.0, start_y + 10.0, 5);

    assert_eq!(batik.undo_count(), 1);
    let painted_pixel = batik.pixel_at(20, 20);
    assert_eq!(painted_pixel, 0xFFFF_0000, "stroke painted red");

    // 4. Invert Adjustment
    let btn_invert = batik.btn_invert;
    click_widget(btn_invert, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 2);
    let inverted_pixel = batik.pixel_at(20, 20);
    assert_eq!(inverted_pixel, 0xFF00_FFFF, "red inverted to cyan");

    // 5. Undo restores painted red
    let btn_undo = batik.btn_undo;
    click_widget(btn_undo, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 1);
    assert_eq!(batik.pixel_at(20, 20), 0xFFFF_0000, "undo restored red");

    // 6. Redo restores inverted cyan
    let btn_redo = batik.btn_redo;
    click_widget(btn_redo, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 2);
    assert_eq!(batik.pixel_at(20, 20), 0xFF00_FFFF, "redo restored cyan");

    // 7. Add new layer
    let btn_new_layer = batik.btn_new_layer;
    click_widget(btn_new_layer, &mut ui, &mut batik);
    assert_eq!(batik.layer_count(), 2);

    // 8. Test PNG and ORA encoding
    let png_bytes = batik.encode_png().expect("encode png");
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let ora_bytes = batik.encode_ora().expect("encode ora");
    assert!(ora_bytes.starts_with(b"PK\x03\x04"));

    // 9. Paint interface into pixel buffer
    let mut pixels = vec![0u32; 800 * 600];
    assert!(ui.paint(&mut pixels).is_some());

    // Verify non-blank rendered pixels
    let mut unique_colors = std::collections::HashSet::new();
    for p in &pixels {
        unique_colors.insert(*p);
    }
    assert!(
        unique_colors.len() > 8,
        "expected rich UI rendering, got {} distinct colors",
        unique_colors.len()
    );
}

#[test]
fn test_batik_layers_and_blend_modes() {
    let mut ui = Ui::new(1280, 800, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    // Fill base layer with solid blue (0xFF0000FF)
    if let Some(l) = batik.doc.get_layer_mut(batik.active_layer_id) {
        if let LayerKind::Raster(ref mut r) = l.kind {
            r.pixels.fill(0xFF0000FF);
        }
    }
    batik.refresh_canvas(&mut ui);
    assert_eq!(batik.pixel_at(10, 10), 0xFF0000FF);

    // 1. Add new layer (Layer 2)
    click_widget(batik.btn_new_layer, &mut ui, &mut batik);
    assert_eq!(batik.layer_count(), 2);

    // Fill Layer 2 with solid yellow (0xFFFFFF00)
    if let Some(l) = batik.doc.get_layer_mut(batik.active_layer_id) {
        if let LayerKind::Raster(ref mut r) = l.kind {
            r.pixels.fill(0xFFFFFF00);
        }
    }
    batik.refresh_canvas(&mut ui);
    assert_eq!(batik.pixel_at(10, 10), 0xFFFFFF00);

    // 2. Change blend mode to Multiply
    click_widget(batik.btn_blend_multiply, &mut ui, &mut batik);
    assert_eq!(batik.active_blend_name(), "Multiply");
    // Blue (0, 0, 255) * Yellow (255, 255, 0) = Black (0, 0, 0)
    let mult_pixel = batik.pixel_at(10, 10);
    assert_eq!(mult_pixel, 0xFF000000, "Yellow multiplied over Blue gives Black");

    // 3. Change blend mode to Screen
    click_widget(batik.btn_blend_screen, &mut ui, &mut batik);
    assert_eq!(batik.active_blend_name(), "Screen");
    // Blue Screen Yellow = White (0xFFFFFFFF)
    let screen_pixel = batik.pixel_at(10, 10);
    assert_eq!(screen_pixel, 0xFFFFFFFF, "Yellow screened over Blue gives White");

    // 4. Test Duplicate Layer
    click_widget(batik.btn_dup_layer, &mut ui, &mut batik);
    assert_eq!(batik.layer_count(), 3);

    // 5. Test Opacity down
    click_widget(batik.btn_opacity_down, &mut ui, &mut batik);
    if let Some(l) = batik.doc.get_layer(batik.active_layer_id) {
        assert!(l.opacity < 255);
    }

    // 6. Test Visibility toggle
    click_widget(batik.btn_toggle_visibility, &mut ui, &mut batik);
    if let Some(l) = batik.doc.get_layer(batik.active_layer_id) {
        assert!(!l.visible);
    }

    // 7. Merge Down
    click_widget(batik.btn_merge_down, &mut ui, &mut batik);
    assert_eq!(batik.layer_count(), 2);

    // 8. Delete Layer
    click_widget(batik.btn_delete_layer, &mut ui, &mut batik);
    assert_eq!(batik.layer_count(), 1);
}

#[test]
fn test_batik_layer_mask_workflow() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    // Base layer: solid red (0xFFFF0000)
    if let Some(l) = batik.doc.get_layer_mut(batik.active_layer_id) {
        if let LayerKind::Raster(ref mut r) = l.kind {
            r.pixels.fill(0xFFFF0000);
        }
    }

    // Add layer 2: solid green (0xFF00FF00)
    click_widget(batik.btn_new_layer, &mut ui, &mut batik);
    if let Some(l) = batik.doc.get_layer_mut(batik.active_layer_id) {
        if let LayerKind::Raster(ref mut r) = l.kind {
            r.pixels.fill(0xFF00FF00);
        }
    }
    batik.refresh_canvas(&mut ui);
    assert_eq!(batik.pixel_at(10, 10), 0xFF00FF00);

    // Add mask to Layer 2 (reveals all green)
    click_widget(batik.btn_add_mask, &mut ui, &mut batik);
    assert!(batik.active_has_mask());

    // Toggle Edit Mask mode
    click_widget(batik.btn_edit_mask, &mut ui, &mut batik);
    assert!(batik.edit_mask);

    // Select Black color (masks away green, revealing red beneath)
    let swatch_black = batik
        .controls
        .iter()
        .find(|(name, _)| *name == "color_black")
        .unwrap()
        .1;
    click_widget(swatch_black, &mut ui, &mut batik);

    // Paint on mask at (50, 50)
    let cr = ui.rect(batik.canvas_id);
    let mx = cr.x + 50.0;
    let my = cr.y + 50.0;
    drag_stroke(&mut ui, &mut batik, mx, my, mx + 5.0, my + 5.0, 3);

    // Where mask was painted black, green is hidden and base red is revealed!
    let revealed_pixel = batik.pixel_at(50, 50);
    assert_eq!(revealed_pixel, 0xFFFF0000, "painted mask reveals underlying red layer");

    // Invert mask
    click_widget(batik.btn_invert_mask, &mut ui, &mut batik);
    // At (50, 50), inverted mask becomes 255 (green shows again)
    let green_restored = batik.pixel_at(50, 50);
    assert_eq!(green_restored, 0xFF00FF00, "inverted mask restores green");
}

#[test]
fn test_batik_pressure_and_presets() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    // 1. Select Chisel Preset
    click_widget(batik.btn_preset_chisel, &mut ui, &mut batik);
    assert!(batik.stroke_engine.brush.chisel_ratio < 0.5);

    // 2. Select Soft Preset
    click_widget(batik.btn_preset_soft, &mut ui, &mut batik);
    assert!(batik.stroke_engine.brush.hardness < 0.2);

    // 3. Select Pressure Curves
    click_widget(batik.btn_curve_light, &mut ui, &mut batik);
    assert_eq!(batik.pressure_curve, PressureCurve::Light);

    click_widget(batik.btn_curve_full, &mut ui, &mut batik);
    assert_eq!(batik.pressure_curve, PressureCurve::Full);

    click_widget(batik.btn_curve_taper, &mut ui, &mut batik);
    assert_eq!(batik.pressure_curve, PressureCurve::Taper);

    // 4. Draw a tapered stroke
    let cr = ui.rect(batik.canvas_id);
    drag_stroke(&mut ui, &mut batik, cr.x + 30.0, cr.y + 30.0, cr.x + 80.0, cr.y + 80.0, 10);
    assert!(batik.undo_count() >= 1);
}

#[test]
fn test_batik_selections_lasso_wand_marching_ants() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    assert!(!batik.selection.is_active());

    // 1. Lasso Tool selection
    click_widget(batik.tool_lasso, &mut ui, &mut batik);
    assert_eq!(batik.tool, Tool::Lasso);

    let cr = ui.rect(batik.canvas_id);
    // Drag triangle
    let _ = batik.step(&mut ui, Event::Pointer(Pointer { x: cr.x + 20.0, y: cr.y + 20.0, kind: PointerKind::Down(Button::Left) }));
    let _ = batik.step(&mut ui, Event::Pointer(Pointer { x: cr.x + 80.0, y: cr.y + 20.0, kind: PointerKind::Move }));
    let _ = batik.step(&mut ui, Event::Pointer(Pointer { x: cr.x + 50.0, y: cr.y + 80.0, kind: PointerKind::Move }));
    let _ = batik.step(&mut ui, Event::Pointer(Pointer { x: cr.x + 50.0, y: cr.y + 80.0, kind: PointerKind::Up(Button::Left) }));

    assert!(batik.selection.is_active(), "Lasso selection must be active");
    assert_eq!(batik.selection.sample(50, 35), 255);

    // 2. Feather selection
    click_widget(batik.btn_feather, &mut ui, &mut batik);
    assert!(batik.selection.is_active());

    // 3. Deselect
    click_widget(batik.btn_deselect, &mut ui, &mut batik);
    assert!(!batik.selection.is_active());

    // 4. Magic Wand tool
    click_widget(batik.tool_wand, &mut ui, &mut batik);
    assert_eq!(batik.tool, Tool::MagicWand);
    // Click on canvas center
    let _ = batik.step(&mut ui, Event::Pointer(Pointer { x: cr.x + 100.0, y: cr.y + 100.0, kind: PointerKind::Down(Button::Left) }));
    assert!(batik.selection.is_active(), "Wand must activate selection");
}

#[test]
fn test_batik_filters() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    // 1. Sharpen
    click_widget(batik.btn_sharpen, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 1);

    // 2. Emboss
    click_widget(batik.btn_emboss, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 2);

    // 3. Sepia
    click_widget(batik.btn_sepia, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 3);

    // 4. Posterize
    click_widget(batik.btn_posterize, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 4);

    // 5. Auto-Levels
    click_widget(batik.btn_auto_levels, &mut ui, &mut batik);
    assert_eq!(batik.undo_count(), 5);
}

#[test]
fn test_batik_codecs_roundtrip() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    // Export PNG
    let png = batik.export_bytes("png").expect("export png");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

    // Export JPEG
    let jpg = batik.export_bytes("jpg").expect("export jpg");
    assert!(jpg.starts_with(&[0xFF, 0xD8, 0xFF]));

    // Export WebP
    let webp = batik.export_bytes("webp").expect("export webp");
    assert!(webp.starts_with(b"RIFF") && &webp[8..12] == b"WEBP");

    // Export PSD
    let psd = batik.export_bytes("psd").expect("export psd");
    assert!(psd.starts_with(b"8BPS"));

    // Re-import PSD
    batik.import_bytes(&psd, &mut ui).expect("import psd");
    assert_eq!(batik.layer_count(), 1);

    // Re-import PNG
    batik.import_bytes(&png, &mut ui).expect("import png");
    assert_eq!(batik.layer_count(), 1);

    // Re-import JPEG
    batik.import_bytes(&jpg, &mut ui).expect("import jpg");
    assert_eq!(batik.layer_count(), 1);

    // Re-import WebP
    batik.import_bytes(&webp, &mut ui).expect("import webp");
    assert_eq!(batik.layer_count(), 1);
}
