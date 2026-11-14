// SPDX-License-Identifier: GPL-3.0-only
//! Tests for GIF decoding.

use sex_codecs::{decode, decode_into, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn gif_still_decode() {
    let bytes = fixture("still.gif");
    let img = decode(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (16, 16));
    assert_eq!(img.pixels.len(), 256);

    // Alpha is opaque
    for &p in &img.pixels {
        assert_eq!(p >> 24, 0xFF);
    }
}

#[test]
fn gif_interlaced_decode() {
    let bytes = fixture("interlaced.gif");
    let img = decode(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (16, 16));
    assert_eq!(img.pixels.len(), 256);
}

#[test]
fn gif_noise_lzw_expansion() {
    let bytes = fixture("noise.gif");
    let img = decode(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (120, 90));
    assert_eq!(img.pixels.len(), 120 * 90);
}

#[test]
fn gif_decode_into_slice() {
    let bytes = fixture("still.gif");
    let mut canvas = vec![0u32; 16 * 16];
    let (w, h) = decode_into(&bytes, &Limits::PHOTO, &mut canvas).unwrap();
    assert_eq!((w, h), (16, 16));
    assert_ne!(canvas[0], 0);
}
