// SPDX-License-Identifier: GPL-3.0-only
//! Tests for WebP decoding.

use sex_codecs::{decode, decode_into, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn rgba(p: u32) -> [u8; 4] {
    [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]
}

#[test]
fn webp_lossless_exact_decode() {
    let bytes = fixture("gradient_lossless.webp");
    let img = decode(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (64, 48));
    assert_eq!(img.pixels.len(), 64 * 48);

    // Assert corner pixels of gradient
    assert_eq!(img.pixel(0, 0), Some(0xFF00_0080));
    assert_eq!(img.pixel(63, 0), Some(0xFFFF_0080));
}

#[test]
fn webp_lossy_decode() {
    let bytes = fixture("gradient_lossy.webp");
    let img = decode(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (64, 48));

    // Corner pixel should be very close to red=0, blue=128
    let p0 = rgba(img.pixel(0, 0).unwrap());
    assert!(p0[0] <= 10, "red channel got {}", p0[0]);
    assert!((p0[2] as i32 - 128).abs() <= 15, "blue channel got {}", p0[2]);
    assert_eq!(p0[3], 255);
}

#[test]
fn webp_alpha_straight_unpremultiplied() {
    let bytes = fixture("alpha.webp");
    let img = decode(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (5, 3));

    // Transparent pixel should have alpha = 0
    let a0 = img.pixel(2, 0).unwrap() >> 24;
    assert_eq!(a0, 0);

    // Opaque red pixel
    let p_red = img.pixel(0, 0).unwrap();
    assert_eq!(p_red, 0xFFFF_0000);

    // Semi-transparent pixel
    let a_half = img.pixel(1, 0).unwrap() >> 24;
    assert_eq!(a_half, 0x80);
}

#[test]
fn webp_decode_into_slice() {
    let bytes = fixture("gradient_lossless.webp");
    let mut canvas = vec![0u32; 64 * 48];
    let (w, h) = decode_into(&bytes, &Limits::PHOTO, &mut canvas).unwrap();
    assert_eq!((w, h), (64, 48));
    assert_eq!(canvas[0], 0xFF00_0080);
}
