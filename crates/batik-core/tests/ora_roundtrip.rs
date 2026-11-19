// SPDX-License-Identifier: GPL-3.0-only
use batik_core::blend::BlendMode;
use batik_core::document::{BackgroundMode, Document};
use batik_core::layer::{Layer, LayerKind, RasterLayer};
use batik_core::ora::{read_ora, write_ora};
use sex_image::Limits;

#[test]
fn test_ora_roundtrip() {
    let limits = Limits::SURFACE;
    let mut doc = Document::new(8, 8, BackgroundMode::Transparent, limits).unwrap();

    // Layer 1: 8x8 white background
    let mut l1_raster = RasterLayer::new(8, 8, &limits).unwrap();
    l1_raster.pixels.fill(0xFFFFFFFF);
    doc.add_layer(Layer::new_raster(1, "Background".into(), l1_raster, BlendMode::Normal, 255));

    // Layer 2: 4x4 red square at offset (2, 2) with opacity 200 and Multiply blend
    let mut l2_raster = RasterLayer::new(4, 4, &limits).unwrap();
    l2_raster.offset_x = 2;
    l2_raster.offset_y = 2;
    l2_raster.pixels.fill(0xFFFF0000);
    doc.add_layer(Layer::new_raster(2, "Red Square".into(), l2_raster, BlendMode::Multiply, 200));

    // Save to ORA
    let ora_bytes = write_ora(&doc).unwrap();
    assert!(!ora_bytes.is_empty());

    // Read back
    let loaded = read_ora(&ora_bytes, &limits).unwrap();

    assert_eq!(loaded.width, 8);
    assert_eq!(loaded.height, 8);
    assert_eq!(loaded.layers.len(), 2);

    // Verify Layer 0
    let l0 = &loaded.layers[0];
    assert_eq!(l0.name, "Background");
    assert_eq!(l0.opacity, 255);
    assert_eq!(l0.blend_mode, BlendMode::Normal);
    if let LayerKind::Raster(ref r) = l0.kind {
        assert_eq!(r.width, 8);
        assert_eq!(r.height, 8);
        assert_eq!(r.offset_x, 0);
        assert_eq!(r.offset_y, 0);
        assert_eq!(r.pixels[0], 0xFFFFFFFF);
    } else {
        panic!("Layer 0 is not raster");
    }

    // Verify Layer 1
    let l1 = &loaded.layers[1];
    assert_eq!(l1.name, "Red Square");
    assert_eq!(l1.opacity, 200);
    assert_eq!(l1.blend_mode, BlendMode::Multiply);
    if let LayerKind::Raster(ref r) = l1.kind {
        assert_eq!(r.width, 4);
        assert_eq!(r.height, 4);
        assert_eq!(r.offset_x, 2);
        assert_eq!(r.offset_y, 2);
        assert_eq!(r.pixels[0], 0xFFFF0000);
    } else {
        panic!("Layer 1 is not raster");
    }

    // Flattened outputs must match!
    let orig_flat = doc.flatten().unwrap();
    let loaded_flat = loaded.flatten().unwrap();
    assert_eq!(orig_flat.pixels, loaded_flat.pixels);
}
