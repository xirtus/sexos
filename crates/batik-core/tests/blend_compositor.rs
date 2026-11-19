// SPDX-License-Identifier: GPL-3.0-only
use batik_core::blend::{blend_pixel, pack_argb, split_argb, BlendMode};
use batik_core::document::{BackgroundMode, Document};
use batik_core::geom::Rect;
use batik_core::layer::{Layer, RasterLayer};
use sex_image::Limits;

#[test]
fn test_blend_modes_golden() {
    // Backdrop: 50% gray (128, 128, 128), fully opaque
    let backdrop = pack_argb(255, 128, 128, 128);
    // Source: pure blue (0, 0, 255), fully opaque
    let source = pack_argb(255, 0, 0, 255);

    // 1. Normal: should result in pure source
    let norm = blend_pixel(backdrop, source, BlendMode::Normal, 255, 255);
    assert_eq!(norm, source);

    // 2. Multiply: 128/255 * 0 = 0 for R and G; 128/255 * 1.0 = ~128 for B
    let mul = blend_pixel(backdrop, source, BlendMode::Multiply, 255, 255);
    let (_a, r, g, b) = split_argb(mul);
    assert_eq!((r, g), (0, 0));
    assert!((127..=129).contains(&b));

    // 3. Screen: 1 - (1 - 0.5)*(1 - S)
    let scr = blend_pixel(backdrop, source, BlendMode::Screen, 255, 255);
    let (_a, r, g, b) = split_argb(scr);
    assert!((127..=129).contains(&r));
    assert!((127..=129).contains(&g));
    assert_eq!(b, 255);

    // 4. Darken: min(128, S)
    let drk = blend_pixel(backdrop, source, BlendMode::Darken, 255, 255);
    let (_a, r, g, b) = split_argb(drk);
    assert_eq!((r, g), (0, 0));
    assert_eq!(b, 128);

    // 5. Lighten: max(128, S)
    let lit = blend_pixel(backdrop, source, BlendMode::Lighten, 255, 255);
    let (_a, r, g, b) = split_argb(lit);
    assert_eq!((r, g), (128, 128));
    assert_eq!(b, 255);

    // 6. Difference: |B - S| -> |128 - 0| = 128, |128 - 255| = 127
    let diff = blend_pixel(backdrop, source, BlendMode::Difference, 255, 255);
    let (_a, r, g, b) = split_argb(diff);
    assert_eq!(r, 128);
    assert_eq!(g, 128);
    assert!((127..=128).contains(&b));

    // 7. Exclusion: B + S - 2*B*S
    let excl = blend_pixel(backdrop, source, BlendMode::Exclusion, 255, 255);
    let (_a, r, g, b) = split_argb(excl);
    assert_eq!(r, 128);
    assert_eq!(g, 128);
    assert!((127..=128).contains(&b));
}

#[test]
fn test_straight_alpha_porter_duff_transparency() {
    // Transparent backdrop
    let backdrop = pack_argb(0, 0, 0, 0);
    // 50% opacity red source
    let source = pack_argb(255, 200, 0, 0);

    let result = blend_pixel(backdrop, source, BlendMode::Normal, 128, 255);
    let (a, r, g, b) = split_argb(result);
    assert!((126..=130).contains(&a));
    // Straight alpha unpremultiplied color remains pure red
    assert_eq!((r, g, b), (200, 0, 0));
}

#[test]
fn test_flatten_rect_dirty_tile() {
    let limits = Limits::SURFACE;
    let mut doc = Document::new(10, 10, BackgroundMode::Solid(0xFFFFFFFF), limits).unwrap();

    let mut raster = RasterLayer::new(10, 10, &limits).unwrap();
    // Fill 2x2 tile at (2, 2) with red
    for y in 2..4 {
        for x in 2..4 {
            raster.pixels[y * 10 + x] = 0xFFFF0000;
        }
    }

    doc.add_layer(Layer::new_raster(1, "Layer 1".into(), raster, BlendMode::Normal, 255));

    let full = doc.flatten().unwrap();
    assert_eq!(full.pixel(2, 2), Some(0xFFFF0000));
    assert_eq!(full.pixel(0, 0), Some(0xFFFFFFFF));

    // Flatten dirty sub-rect at (2, 2, 2, 2)
    let tile = doc.flatten_rect(Rect::new(2, 2, 2, 2)).unwrap();
    assert_eq!(tile.width, 2);
    assert_eq!(tile.height, 2);
    for p in tile.pixels {
        assert_eq!(p, 0xFFFF0000);
    }
}
