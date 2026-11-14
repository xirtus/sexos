// SPDX-License-Identifier: GPL-3.0-only
//! Tests for HEIC decoding.

use sex_codecs::{decode, decode_into, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn heic_real_image_decode() {
    let bytes = fixture("smpte32.heic");
    let img = decode(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (32, 32));
    assert_eq!(img.pixels.len(), 32 * 32);

    // Assert alpha channel is fully opaque
    for &p in &img.pixels {
        assert_eq!(p >> 24, 0xFF);
    }

    // Verify center pixel has non-zero color channels
    let center = img.pixel(16, 16).unwrap();
    let r = (center >> 16) & 0xFF;
    let g = (center >> 8) & 0xFF;
    let b = center & 0xFF;
    assert!(r > 0 || g > 0 || b > 0);
}

#[test]
fn heic_decode_into_slice() {
    let bytes = fixture("smpte32.heic");
    let mut canvas = vec![0u32; 32 * 32];
    let (w, h) = decode_into(&bytes, &Limits::PHOTO, &mut canvas).unwrap();
    assert_eq!((w, h), (32, 32));
    assert_eq!(canvas.len(), 32 * 32);
    assert_eq!(canvas[0] >> 24, 0xFF);
}
