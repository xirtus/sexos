// SPDX-License-Identifier: GPL-3.0-only
use batik_core::blend::BlendMode;
use batik_core::layer::LayerKind;
use batik_core::psd::parse_psd;
use sex_image::Limits;

fn create_test_psd() -> Vec<u8> {
    let mut out = Vec::new();

    // 1. Header (26 bytes)
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes()); // version 1
    out.extend_from_slice(&[0u8; 6]); // reserved
    out.extend_from_slice(&3u16.to_be_bytes()); // 3 channels
    out.extend_from_slice(&4u32.to_be_bytes()); // height 4
    out.extend_from_slice(&4u32.to_be_bytes()); // width 4
    out.extend_from_slice(&8u16.to_be_bytes()); // depth 8
    out.extend_from_slice(&3u16.to_be_bytes()); // color mode 3 (RGB)

    // 2. Color Mode Data
    out.extend_from_slice(&0u32.to_be_bytes());

    // 3. Image Resources
    out.extend_from_slice(&0u32.to_be_bytes());

    // 4. Layer & Mask Info section
    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&2i16.to_be_bytes()); // 2 layers

    // Layer 0: "Base", bounds (0, 0, 4, 4), Raw channels
    layer_info.extend_from_slice(&0i32.to_be_bytes()); // top
    layer_info.extend_from_slice(&0i32.to_be_bytes()); // left
    layer_info.extend_from_slice(&4i32.to_be_bytes()); // bottom
    layer_info.extend_from_slice(&4i32.to_be_bytes()); // right
    layer_info.extend_from_slice(&4u16.to_be_bytes()); // 4 channels

    // Channel infos: 0=R, 1=G, 2=B, -1=A. Length = 2 + 16 = 18 bytes
    for id in [0i16, 1, 2, -1] {
        layer_info.extend_from_slice(&id.to_be_bytes());
        layer_info.extend_from_slice(&18u32.to_be_bytes());
    }
    layer_info.extend_from_slice(b"8BIM");
    layer_info.extend_from_slice(b"norm");
    layer_info.push(255); // opacity
    layer_info.push(0); // clipping
    layer_info.push(0); // flags: visible
    layer_info.push(0); // filler

    // Extra data: mask (4) + ranges (4) + name (8) = 16 bytes
    layer_info.extend_from_slice(&16u32.to_be_bytes());
    layer_info.extend_from_slice(&0u32.to_be_bytes()); // mask len
    layer_info.extend_from_slice(&0u32.to_be_bytes()); // ranges len
    layer_info.push(4); // name len
    layer_info.extend_from_slice(b"Base");
    layer_info.extend_from_slice(&[0u8; 3]); // pad to 8 bytes

    // Layer 1: "Overlay", bounds (1, 1, 3, 3), PackBits RLE channels
    layer_info.extend_from_slice(&1i32.to_be_bytes()); // top
    layer_info.extend_from_slice(&1i32.to_be_bytes()); // left
    layer_info.extend_from_slice(&3i32.to_be_bytes()); // bottom
    layer_info.extend_from_slice(&3i32.to_be_bytes()); // right
    layer_info.extend_from_slice(&4u16.to_be_bytes()); // 4 channels

    // Channel infos: length = 2 (comp 1) + 4 (table) + 4 (PackBits) = 10 bytes
    for id in [0i16, 1, 2, -1] {
        layer_info.extend_from_slice(&id.to_be_bytes());
        layer_info.extend_from_slice(&10u32.to_be_bytes());
    }
    layer_info.extend_from_slice(b"8BIM");
    layer_info.extend_from_slice(b"mul ");
    layer_info.push(200); // opacity
    layer_info.push(0); // clipping
    layer_info.push(0); // flags: visible
    layer_info.push(0); // filler

    // Extra data: mask (4) + ranges (4) + name (8) = 16 bytes
    layer_info.extend_from_slice(&16u32.to_be_bytes());
    layer_info.extend_from_slice(&0u32.to_be_bytes());
    layer_info.extend_from_slice(&0u32.to_be_bytes());
    layer_info.push(7); // name len
    layer_info.extend_from_slice(b"Overlay"); // 1 + 7 = 8 bytes (no pad needed)

    // Channel data for Layer 0 (Raw: comp = 0, 16 bytes each)
    // R: 0, G: 0, B: 255, A: 255
    for val in [0x00u8, 0x00, 0xFF, 0xFF] {
        layer_info.extend_from_slice(&0u16.to_be_bytes()); // comp 0
        layer_info.extend_from_slice(&[val; 16]);
    }

    // Channel data for Layer 1 (PackBits: comp = 1, height = 2, width = 2)
    // Repeat run for 2 bytes: n = -1 (0xFF), followed by value
    // R: 255, G: 0, B: 0, A: 255
    for val in [0xFFu8, 0x00, 0x00, 0xFF] {
        layer_info.extend_from_slice(&1u16.to_be_bytes()); // comp 1
        layer_info.extend_from_slice(&2u16.to_be_bytes()); // line 0 len = 2
        layer_info.extend_from_slice(&2u16.to_be_bytes()); // line 1 len = 2
        layer_info.extend_from_slice(&[0xFF, val, 0xFF, val]); // PackBits runs
    }

    // Layer info length prefix
    let mut layer_section = Vec::new();
    layer_section.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
    layer_section.extend_from_slice(&layer_info);

    // Layer & mask section length prefix
    out.extend_from_slice(&(layer_section.len() as u32).to_be_bytes());
    out.extend_from_slice(&layer_section);

    out
}

#[test]
fn test_psd_multi_layer_fixture_import() {
    let psd_bytes = create_test_psd();
    let doc = parse_psd(&psd_bytes, &Limits::SURFACE).unwrap();

    assert_eq!(doc.width, 4);
    assert_eq!(doc.height, 4);
    assert_eq!(doc.layers.len(), 2, "PSD fixture layers count should be 2");

    // Verify Layer 0: Base
    let l0 = &doc.layers[0];
    assert_eq!(l0.name, "Base");
    assert_eq!(l0.blend_mode, BlendMode::Normal);
    assert_eq!(l0.opacity, 255);
    assert!(l0.visible);
    if let LayerKind::Raster(ref r0) = l0.kind {
        assert_eq!(r0.width, 4);
        assert_eq!(r0.height, 4);
        assert_eq!(r0.offset_x, 0);
        assert_eq!(r0.offset_y, 0);
        // All pixels are opaque blue (0xFF0000FF)
        for p in &r0.pixels {
            assert_eq!(*p, 0xFF0000FF);
        }
    } else {
        panic!("Layer 0 is not raster");
    }

    // Verify Layer 1: Overlay
    let l1 = &doc.layers[1];
    assert_eq!(l1.name, "Overlay");
    assert_eq!(l1.blend_mode, BlendMode::Multiply);
    assert_eq!(l1.opacity, 200);
    assert!(l1.visible);
    if let LayerKind::Raster(ref r1) = l1.kind {
        assert_eq!(r1.width, 2);
        assert_eq!(r1.height, 2);
        assert_eq!(r1.offset_x, 1);
        assert_eq!(r1.offset_y, 1);
        // PackBits decompressed pixels are opaque red (0xFFFF0000)
        for p in &r1.pixels {
            assert_eq!(*p, 0xFFFF0000);
        }
    } else {
        panic!("Layer 1 is not raster");
    }
}
