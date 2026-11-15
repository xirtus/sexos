// SPDX-License-Identifier: GPL-3.0-only
use batik_core::blend::{pack_argb, split_argb, BlendMode};
use batik_core::document::{BackgroundMode, Document};
use batik_core::jpeg::encode_jpeg;
use batik_core::layer::LayerKind;
use batik_core::png::encode_rgba_png;
use batik_core::psd::{encode_psd, parse_psd};
use batik_core::webp::encode_webp;
use sex_image::Limits;

#[test]
fn test_webp_lossless_roundtrip_bit_exact() {
    let width = 16;
    let height = 16;
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let a = ((x * 15 + y * 7) % 256) as u8;
            let r = (x * 16) as u8;
            let g = (y * 16) as u8;
            let b = ((x + y) * 8) as u8;
            pixels.push(pack_argb(a, r, g, b));
        }
    }

    let webp_bytes = encode_webp(&pixels, width, height).expect("encode webp");
    assert!(webp_bytes.starts_with(b"RIFF"));
    assert_eq!(&webp_bytes[8..12], b"WEBP");
    assert_eq!(&webp_bytes[12..16], b"VP8L");

    let decoded = sex_image::decode(&webp_bytes, &Limits::PHOTO).expect("decode webp");
    assert_eq!(decoded.width, width);
    assert_eq!(decoded.height, height);
    assert_eq!(decoded.pixels, pixels, "WebP lossless must decode bit-exact!");
}

#[test]
fn test_jpeg_baseline_roundtrip() {
    let width = 32;
    let height = 32;
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 8) % 256) as u8;
            let g = ((y * 8) % 256) as u8;
            let b = (((x + y) * 4) % 256) as u8;
            pixels.push(pack_argb(255, r, g, b));
        }
    }

    let jpeg_bytes = encode_jpeg(&pixels, width, height, 90).expect("encode jpeg");
    assert!(jpeg_bytes.starts_with(&[0xFF, 0xD8]));
    assert!(jpeg_bytes.ends_with(&[0xFF, 0xD9]));

    let decoded = sex_image::decode(&jpeg_bytes, &Limits::PHOTO).expect("decode jpeg");
    assert_eq!(decoded.width, width);
    assert_eq!(decoded.height, height);

    // Compute mean absolute error across color channels
    let mut total_diff: u64 = 0;
    for (orig, dec) in pixels.iter().zip(decoded.pixels.iter()) {
        let (_, or, og, ob) = split_argb(*orig);
        let (_, dr, dg, db) = split_argb(*dec);
        total_diff += (or as i32 - dr as i32).abs() as u64;
        total_diff += (og as i32 - dg as i32).abs() as u64;
        total_diff += (ob as i32 - db as i32).abs() as u64;
    }
    let avg_diff = (total_diff as f64) / (width * height * 3) as f64;
    assert!(avg_diff < 10.0, "JPEG quality 90 should have low average channel error: {avg_diff}");
}

#[test]
fn test_png_roundtrip() {
    let width = 8;
    let height = 8;
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.push(pack_argb(200, (x * 30) as u8, (y * 30) as u8, 128));
        }
    }

    let png_bytes = encode_rgba_png(&pixels, width, height).expect("encode png");
    let decoded = sex_image::decode(&png_bytes, &Limits::PHOTO).expect("decode png");
    assert_eq!(decoded.width, width);
    assert_eq!(decoded.height, height);
    assert_eq!(decoded.pixels, pixels, "PNG must decode bit-exact");
}

#[test]
fn test_psd_multi_layer_roundtrip() {
    let width = 8;
    let height = 8;
    let limits = Limits::PHOTO;
    let mut doc = Document::new(width, height, BackgroundMode::Transparent, limits).unwrap();

    // Layer 1: Base layer with full opacity
    let l1_id = doc.add_raster_layer("Base", BlendMode::Normal, 255).unwrap();
    if let Some(l1) = doc.get_layer_mut(l1_id) {
        if let LayerKind::Raster(ref mut r) = l1.kind {
            r.pixels.fill(0xFFFF0000); // Red
        }
    }

    // Layer 2: Overlay layer with Screen blend mode, 180 opacity, and a layer mask
    let l2_id = doc.add_raster_layer("Overlay", BlendMode::Screen, 180).unwrap();
    if let Some(l2) = doc.get_layer_mut(l2_id) {
        if let LayerKind::Raster(ref mut r) = l2.kind {
            r.pixels.fill(0xFF00FF00); // Green
            r.add_mask(128).unwrap(); // 50% mask
        }
    }

    let psd_bytes = encode_psd(&doc).expect("encode psd");
    assert!(psd_bytes.starts_with(b"8BPS"));

    let doc2 = parse_psd(&psd_bytes, &limits).expect("parse psd");
    assert_eq!(doc2.width, width);
    assert_eq!(doc2.height, height);
    assert_eq!(doc2.layers.len(), 2, "must decode 2 layers");

    // Check layer 1
    let l1_dec = &doc2.layers[0];
    assert_eq!(l1_dec.name, "Base");
    assert_eq!(l1_dec.blend_mode, BlendMode::Normal);
    assert_eq!(l1_dec.opacity, 255);
    if let LayerKind::Raster(ref r) = l1_dec.kind {
        assert_eq!(r.pixels[0], 0xFFFF0000);
        assert!(r.mask.is_none());
    } else {
        panic!("Layer 1 not raster");
    }

    // Check layer 2
    let l2_dec = &doc2.layers[1];
    assert_eq!(l2_dec.name, "Overlay");
    assert_eq!(l2_dec.blend_mode, BlendMode::Screen);
    assert_eq!(l2_dec.opacity, 180);
    if let LayerKind::Raster(ref r) = l2_dec.kind {
        assert_eq!(r.pixels[0], 0xFF00FF00);
        assert!(r.mask.is_some(), "Layer 2 mask must be preserved!");
        assert_eq!(r.mask.as_ref().unwrap()[0], 128);
    } else {
        panic!("Layer 2 not raster");
    }
}
