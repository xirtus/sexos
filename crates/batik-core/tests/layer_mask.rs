// SPDX-License-Identifier: GPL-3.0-only
use batik_core::blend::{split_argb, BlendMode};
use batik_core::brush::{Brush, BrushPreset, PointerSample, StrokeEngine};
use batik_core::document::{BackgroundMode, Document};
use batik_core::geom::Rect;
use batik_core::layer::RasterLayer;
use batik_core::selection::{SelectionCombine, SelectionMask};
use sex_image::Limits;

#[test]
fn test_layer_mask_workflow() {
    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(10, 10, &limits).unwrap();
    // Fill layer with opaque red (0xFFFF0000)
    for p in &mut raster.pixels {
        *p = 0xFFFF0000;
    }

    assert!(raster.mask.is_none());

    // Add mask initialized to 255 (reveal all)
    raster.add_mask(255).unwrap();
    assert!(raster.mask.is_some());
    assert_eq!(raster.mask.as_ref().unwrap().len(), 100);
    assert!(raster.mask.as_ref().unwrap().iter().all(|&v| v == 255));

    // Paint a black dab on the mask (hiding pixels)
    let mut brush = Brush::from_preset(BrushPreset::HardRound);
    brush.size = 4.0;
    let engine = StrokeEngine::new(brush);
    let sample = PointerSample::new(5.0, 5.0, 1.0);
    let sel = SelectionMask::new(10, 10).unwrap();
    engine.render_mask_dab(&mut raster, &sel, sample, 0);

    // Center pixel mask value should be 0 (hidden)
    let mask_val = raster.mask.as_ref().unwrap()[5 * 10 + 5];
    assert_eq!(mask_val, 0, "Center pixel of mask should be 0");

    // Invert mask
    raster.invert_mask();
    assert_eq!(raster.mask.as_ref().unwrap()[5 * 10 + 5], 255);

    // Apply mask to pixels
    raster.apply_mask();
    assert!(raster.mask.is_none());
    // At (0, 0), original mask was 255, inverted was 0, so alpha should now be 0
    let (a, _, _, _) = split_argb(raster.pixels[0]);
    assert_eq!(a, 0);
    // At (5, 5), inverted mask was 255, so alpha should be 255
    let (a5, r5, _, _) = split_argb(raster.pixels[5 * 10 + 5]);
    assert_eq!(a5, 255);
    assert_eq!(r5, 255);
}

#[test]
fn test_layer_mask_compositing_with_document() {
    let limits = Limits::SURFACE;
    let mut doc = Document::new(8, 8, BackgroundMode::Solid(0xFFFFFFFF), limits).unwrap();
    // Add layer 1, filled with solid red
    let id1 = doc.add_raster_layer("Red Layer", BlendMode::Normal, 255).unwrap();
    if let Some(layer) = doc.get_layer_mut(id1) {
        if let Some(r) = layer.raster_mut() {
            for p in &mut r.pixels {
                *p = 0xFFFF0000;
            }
            // Add mask initialized to 0 (hide all red)
            r.add_mask(0).unwrap();
            // Paint white on mask at (2, 2)
            r.mask_mut().unwrap()[2 * 8 + 2] = 255;
        }
    }

    let flat = doc.render_flattened();
    // At (0, 0), red layer mask is 0, so background white shines through
    assert_eq!(flat[0], 0xFFFFFFFF);
    // At (2, 2), red layer mask is 255, so red shows
    assert_eq!(flat[2 * 8 + 2], 0xFFFF0000);
}

#[test]
fn test_mask_from_selection() {
    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(10, 10, &limits).unwrap();
    let mut sel = SelectionMask::new(10, 10).unwrap();
    sel.select_rect(Rect::new(2, 2, 4, 4), SelectionCombine::Replace);

    raster.mask_from_selection(&sel).unwrap();
    assert!(raster.mask.is_some());
    let m = raster.mask.as_ref().unwrap();
    assert_eq!(m[2 * 10 + 2], 255);
    assert_eq!(m[0], 0);

    // Delete mask
    raster.delete_mask();
    assert!(raster.mask.is_none());
}
