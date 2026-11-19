// SPDX-License-Identifier: GPL-3.0-only
//! Tests for unified animation streaming decoder across GIF and WebP.

use sex_codecs::{AnimationDecoder, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn gif_animation_streaming_frames() {
    let bytes = fixture("anim.gif");
    let mut anim = AnimationDecoder::open(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((anim.width(), anim.height()), (8, 6));
    assert_eq!(anim.frame_count(), 3);
    assert_eq!(anim.loop_count(), None); // Loops forever

    let count = anim.width() * anim.height();
    let mut canvas = vec![0u32; count];

    // Frame 1: delay 50 ms
    let f1 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f1.duration_ms, 50);
    assert!(!f1.is_last);
    assert_eq!(canvas[0], 0xFFFF_0000); // Red

    // Frame 2: delay 120 ms
    let f2 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f2.duration_ms, 120);
    assert!(!f2.is_last);

    // Frame 3: delay 300 ms, is_last = true
    let f3 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f3.duration_ms, 300);
    assert!(f3.is_last);

    // Loops back to Frame 1
    let f4 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f4.duration_ms, 50);
    assert!(!f4.is_last);
    assert_eq!(canvas[0], 0xFFFF_0000);
}

#[test]
fn webp_animation_streaming_frames() {
    let bytes = fixture("anim.webp");
    let mut anim = AnimationDecoder::open(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!((anim.width(), anim.height()), (8, 6));
    assert_eq!(anim.frame_count(), 3);

    let count = anim.width() * anim.height();
    let mut canvas = vec![0u32; count];

    // Read all 3 frames
    let f1 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f1.duration_ms, 100);
    assert_eq!(canvas[0], 0xFFFF_0000); // Red

    let f2 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f2.duration_ms, 200);
    assert_eq!(canvas[0], 0xFF00_FF00); // Green

    let f3 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f3.duration_ms, 300);
    assert_eq!(canvas[0], 0xFF00_00FF); // Blue

    // Loop back
    let f4 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f4.duration_ms, 100);
    assert_eq!(canvas[0], 0xFFFF_0000);
}

#[test]
fn still_image_as_single_frame_animation() {
    let bytes = fixture("smpte32.avif");
    let mut anim = AnimationDecoder::open(&bytes, &Limits::PHOTO).unwrap();
    assert_eq!(anim.frame_count(), 1);

    let count = anim.width() * anim.height();
    let mut canvas = vec![0u32; count];

    let f1 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert_eq!(f1.duration_ms, 0);
    assert!(f1.is_last);

    // Stills do not loop
    assert!(anim.next_frame(&mut canvas).unwrap().is_none());

    // Resetting allows viewing again
    anim.reset().unwrap();
    let f2 = anim.next_frame(&mut canvas).unwrap().unwrap();
    assert!(f2.is_last);
}
