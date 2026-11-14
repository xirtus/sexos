// SPDX-License-Identifier: GPL-3.0-only
use batik_core::document::{BackgroundMode, Document};
use batik_core::geom::Rect;
use batik_core::history::{History, HistoryAction};
use batik_core::layer::LayerKind;
use sex_image::Limits;

#[test]
fn test_history_undo_redo_and_budget_eviction() {
    let limits = Limits::SURFACE;
    let mut doc = Document::new(10, 10, BackgroundMode::Solid(0xFFFFFFFF), limits).unwrap();
    let layer_id = doc.add_raster_layer("Layer 1", batik_core::BlendMode::Normal, 255).unwrap();

    // 1000 bytes budget
    let mut history = History::new(1000);

    // Initial state: pixel at (0, 0) is 0
    let mut old_p = vec![0u32; 4];
    let mut new_p = vec![0xFFFF0000u32; 4]; // red

    // Push action
    history.push_action(HistoryAction::LayerPixelDelta {
        layer_id,
        rect: Rect::new(0, 0, 2, 2),
        old_pixels: old_p.clone(),
        new_pixels: new_p.clone(),
    });

    // Manually apply new pixels to doc
    if let Some(l) = doc.get_layer_mut(layer_id) {
        if let LayerKind::Raster(ref mut r) = l.kind {
            r.pixels[0] = 0xFFFF0000;
        }
    }

    assert!(history.can_undo());
    history.undo(&mut doc);

    // Pixel restored to 0
    if let Some(l) = doc.get_layer(layer_id) {
        if let LayerKind::Raster(ref r) = l.kind {
            assert_eq!(r.pixels[0], 0);
        }
    }

    assert!(history.can_redo());
    history.redo(&mut doc);

    // Pixel red again
    if let Some(l) = doc.get_layer(layer_id) {
        if let LayerKind::Raster(ref r) = l.kind {
            assert_eq!(r.pixels[0], 0xFFFF0000);
        }
    }

    // Now test budget eviction: push 50 actions to overflow 1000 byte budget
    for i in 0..50 {
        old_p[0] = i;
        new_p[0] = i + 1;
        history.push_action(HistoryAction::LayerPixelDelta {
            layer_id,
            rect: Rect::new(0, 0, 2, 2),
            old_pixels: old_p.clone(),
            new_pixels: new_p.clone(),
        });
    }

    assert!(history.current_bytes() <= 1000, "Current bytes {} exceeded max budget 1000", history.current_bytes());
}
