// SPDX-License-Identifier: GPL-3.0-only
//! Integration tests for G-27 Non-Destructive Workflow:
//! 1. Curves and Hue/Saturation Adjustment Layers with Masking and Undo/Redo.
//! 2. Deterministic Hue/Saturation transformation mathematics.
//! 3. Scalable, editable TextLayers with live typography updates and PSD export.
//! 4. Non-Destructive FilterStack push/toggle/clear preserving original pixels bit-for-bit.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use batik_core::adjust::{
    AdjustmentKind, CurvesAdjustment, CurvesPreset, HueSatAdjustment,
};
use batik_core::blend::{pack_argb, split_argb, BlendMode};
use batik_core::document::{BackgroundMode, Document};
use batik_core::filters::FilterSpec;
use batik_core::history::{History, HistoryAction};
use batik_core::layer::{Layer, LayerKind, RasterLayer};
use batik_core::psd::{encode_psd, parse_psd};
use sex_image::Limits;

#[test]
fn test_curves_adjustment_layer_and_masking_and_undo() {
    let limits = Limits::PHOTO;
    let mut doc = Document::new(64, 64, BackgroundMode::Solid(0xFF808080), limits).unwrap();

    // Base raster layer with a horizontal gradient from black to white
    let mut grad_pixels = Vec::with_capacity(64 * 64);
    for _y in 0..64 {
        for x in 0..64 {
            let v = ((x as u32 * 255) / 63) as u8;
            grad_pixels.push(pack_argb(255, v, v, v));
        }
    }
    let raster = RasterLayer::with_pixels(64, 64, grad_pixels.clone(), &limits).unwrap();
    doc.add_layer(Layer::new_raster(1, "Gradient".into(), raster, BlendMode::Normal, 255));

    // 1. Add Curves Adjustment Layer with SCurveContrast
    let curves_id = doc.add_curves_preset_layer("Contrast Curves", CurvesPreset::SCurveContrast, 255);
    assert_eq!(curves_id, 2);

    let flat_contrast = doc.flatten().unwrap();
    // In S-Curve: shadow at x=8 (v ~ 32) should be compressed darker, highlight at x=56 (v ~ 226) should be brighter
    let (_a0, r_shadow_orig, _, _) = split_argb(grad_pixels[8]);
    let (_, r_shadow_adj, _, _) = split_argb(flat_contrast.pixels[8]);
    assert!(
        r_shadow_adj <= r_shadow_orig,
        "S-curve should darken shadows: orig={}, adj={}",
        r_shadow_orig,
        r_shadow_adj
    );

    let (_, r_high_orig, _, _) = split_argb(grad_pixels[56]);
    let (_, r_high_adj, _, _) = split_argb(flat_contrast.pixels[56]);
    assert!(
        r_high_adj >= r_high_orig,
        "S-curve should brighten highlights: orig={}, adj={}",
        r_high_orig,
        r_high_adj
    );

    // 2. Test LiftShadows preset
    doc.update_adjustment_layer(curves_id, AdjustmentKind::Curves(CurvesAdjustment::lift_shadows())).unwrap();
    let flat_lifted = doc.flatten().unwrap();
    let (_, r_black_adj, _, _) = split_argb(flat_lifted.pixels[0]);
    assert!(r_black_adj >= 45, "LiftShadows should lift black above 45: got {}", r_black_adj);

    // 3. Masked adjustment layer: affect only left half (x < 32: 255, x >= 32: 0)
    let mut mask = vec![0u8; 64 * 64];
    for y in 0..64 {
        for x in 0..32 {
            mask[y * 64 + x] = 255;
        }
    }
    let layer = doc.get_layer_mut(curves_id).unwrap();
    if let LayerKind::Adjustment(ref mut adj) = layer.kind {
        adj.mask = Some(mask);
    }

    let flat_masked = doc.flatten().unwrap();
    // Left side (x=0) was adjusted (shadow lifted)
    let (_, r_left, _, _) = split_argb(flat_masked.pixels[0]);
    assert!(r_left >= 45, "Masked left side should be adjusted");
    // Right side (x=63, white) had mask=0, so it matches original gradient pixel exactly!
    assert_eq!(flat_masked.pixels[63], grad_pixels[63], "Masked right side must be completely untouched");

    // 4. Undo/Redo journaling test
    let mut history = History::new(1024 * 1024);
    let old_kind = AdjustmentKind::Curves(CurvesAdjustment::lift_shadows());
    let new_kind = AdjustmentKind::Curves(CurvesAdjustment::solarize());
    doc.update_adjustment_layer(curves_id, new_kind.clone()).unwrap();
    history.push_action(HistoryAction::AdjustmentLayerChanged {
        layer_id: curves_id,
        old_kind: old_kind.clone(),
        new_kind: new_kind.clone(),
    });

    let layer = doc.get_layer(curves_id).unwrap();
    assert_eq!(layer.adjustment().unwrap().kind, new_kind);

    assert!(history.undo(&mut doc));
    let layer = doc.get_layer(curves_id).unwrap();
    assert_eq!(layer.adjustment().unwrap().kind, old_kind, "Undo must restore old curves preset");

    assert!(history.redo(&mut doc));
    let layer = doc.get_layer(curves_id).unwrap();
    assert_eq!(layer.adjustment().unwrap().kind, new_kind, "Redo must restore new curves preset");
}

#[test]
fn test_hue_saturation_adjustment_layer() {
    let limits = Limits::PHOTO;
    let mut doc = Document::new(32, 32, BackgroundMode::Transparent, limits).unwrap();

    // Fill base layer with pure red 0xFFFF0000
    let red_pixels = vec![0xFFFF0000u32; 32 * 32];
    let raster = RasterLayer::with_pixels(32, 32, red_pixels.clone(), &limits).unwrap();
    doc.add_layer(Layer::new_raster(1, "Red Base".into(), raster, BlendMode::Normal, 255));

    // 1. +120° hue shift turns pure red 0xFFFF0000 into pure green 0xFF00FF00
    let adj_id = doc.add_hue_saturation_layer("Hue Shift", 120, 0, 0, 255);
    let flat_green = doc.flatten().unwrap();
    let (a, r, g, b) = split_argb(flat_green.pixels[0]);
    assert_eq!((a, r, g, b), (255, 0, 255, 0), "+120° hue shift on red must yield pure green");

    // 2. -100% saturation desaturates red to neutral gray (R == G == B)
    doc.update_adjustment_layer(
        adj_id,
        AdjustmentKind::HueSaturation(HueSatAdjustment::new(0, -100, 0)),
    )
    .unwrap();
    let flat_gray = doc.flatten().unwrap();
    let (ga, gr, gg, gb) = split_argb(flat_gray.pixels[0]);
    assert_eq!(ga, 255);
    assert_eq!(gr, gg);
    assert_eq!(gg, gb);
    assert!((126..=129).contains(&gr), "Desaturated red should be neutral gray ~128: got {}", gr);

    // 3. Lightness +100% reaches pure white, -100% reaches pure black
    doc.update_adjustment_layer(
        adj_id,
        AdjustmentKind::HueSaturation(HueSatAdjustment::new(0, 0, 100)),
    )
    .unwrap();
    let flat_white = doc.flatten().unwrap();
    assert_eq!(flat_white.pixels[0], 0xFFFFFFFF, "Lightness +100% yields white");

    doc.update_adjustment_layer(
        adj_id,
        AdjustmentKind::HueSaturation(HueSatAdjustment::new(0, 0, -100)),
    )
    .unwrap();
    let flat_black = doc.flatten().unwrap();
    assert_eq!(flat_black.pixels[0], 0xFF000000, "Lightness -100% yields black");

    // 4. Layer Opacity & Blend Mode interaction
    let layer = doc.get_layer_mut(adj_id).unwrap();
    layer.opacity = 128; // ~50% opacity
    layer.kind = AdjustmentKind::HueSaturation(HueSatAdjustment::new(120, 0, 0)).into();
    let flat_blend = doc.flatten().unwrap();
    let (_, br, bg, _) = split_argb(flat_blend.pixels[0]);
    assert!(br > 50 && bg > 50, "50% opacity must blend red backdrop with green adjustment: r={}, g={}", br, bg);
}

#[test]
fn test_editable_text_layer_and_psd_roundtrip() {
    let limits = Limits::PHOTO;
    let mut doc = Document::new(160, 60, BackgroundMode::Solid(0xFF000000), limits).unwrap();

    // 1. Add TextLayer "SEXOS BATIK"
    let text_id = doc
        .add_text_layer(
            "Title",
            "SEXOS BATIK",
            10,
            15,
            24.0,
            0xFFFFFFFF,
            BlendMode::Normal,
            255,
        )
        .unwrap();

    let flat1 = doc.flatten().unwrap();
    let non_black_count = flat1.pixels.iter().filter(|&&p| p != 0xFF000000).count();
    assert!(
        non_black_count > 50,
        "Rendered text must produce crisp non-background glyph pixels: got {}",
        non_black_count
    );

    // 2. Edit TextLayer in place without destroying or recreating it
    doc.set_text_layer_content(text_id, "PIXELMATOR PRO").unwrap();
    doc.set_text_layer_style(text_id, 20.0, 0xFFFFCC00, true).unwrap();
    doc.move_text_layer(text_id, 5, 20).unwrap();

    let flat2 = doc.flatten().unwrap();
    let gold_pixels = flat2.pixels.iter().filter(|&&p| {
        let (_a, r, g, b) = split_argb(p);
        r > 200 && g > 150 && b < 100
    }).count();
    assert!(
        gold_pixels > 50,
        "Updated text must render with gold color: got {} pixels",
        gold_pixels
    );

    // 3. Multi-layer PSD Export Roundtrip
    let psd_bytes = encode_psd(&doc).unwrap();
    assert!(psd_bytes.starts_with(b"8BPS"), "PSD signature must be valid");
    let imported_doc = parse_psd(&psd_bytes, &limits).unwrap();
    assert!(
        imported_doc.layers.len() >= 1,
        "Imported PSD must preserve exported layers"
    );
}

#[test]
fn test_nondestructive_filter_stack() {
    let limits = Limits::PHOTO;
    let mut doc = Document::new(64, 64, BackgroundMode::Transparent, limits).unwrap();

    // Create distinctive pattern in base layer
    let mut pattern = vec![0xFF000000u32; 64 * 64];
    for y in 0..64 {
        for x in 0..64 {
            if (x / 8 + y / 8) % 2 == 0 {
                pattern[y * 64 + x] = 0xFFFF0055;
            } else {
                pattern[y * 64 + x] = 0xFF00AAFF;
            }
        }
    }
    let original_pixels = pattern.clone();
    let raster = RasterLayer::with_pixels(64, 64, pattern, &limits).unwrap();
    let layer_id = doc.add_layer(Layer::new_raster(1, "Base Pattern".into(), raster, BlendMode::Normal, 255));

    // 1. Push filters onto filter_stack: GaussianBlur + Posterize + Sepia
    doc.push_filter(layer_id, FilterSpec::GaussianBlur { radius: 2 }).unwrap();
    doc.push_filter(layer_id, FilterSpec::Posterize { levels: 4 }).unwrap();
    doc.push_filter(layer_id, FilterSpec::Sepia { intensity_pct: 100 }).unwrap();

    let flat_filtered = doc.flatten().unwrap();
    assert_ne!(
        flat_filtered.pixels, original_pixels,
        "Flattened output must reflect active filter stack"
    );

    // 2. Verify underlying raster pixels remain 100% BIT-IDENTICAL!
    let current_layer = doc.get_layer(layer_id).unwrap();
    let raster_ref = current_layer.raster().unwrap();
    assert_eq!(
        raster_ref.pixels, original_pixels,
        "Raster layer pixels must NEVER be mutated by non-destructive filter stack!"
    );

    // 3. Toggle filters off non-destructively
    doc.set_filter_enabled(layer_id, 0, false).unwrap();
    doc.set_filter_enabled(layer_id, 1, false).unwrap();
    doc.set_filter_enabled(layer_id, 2, false).unwrap();

    let flat_disabled = doc.flatten().unwrap();
    assert_eq!(
        flat_disabled.pixels, original_pixels,
        "When filter nodes are disabled, flattened output must match original pixels pixel-for-pixel!"
    );

    // 4. Clear filter stack
    doc.clear_filter_stack(layer_id).unwrap();
    let current_layer = doc.get_layer(layer_id).unwrap();
    assert!(current_layer.filter_stack.is_empty(), "Filter stack must be empty");
}
