// SPDX-License-Identifier: GPL-3.0-only
//! Tests for pixel conversion: YUY2, NV12, and MJPEG.

use sex_uvc::pixel::{nv12_to_argb, yuy2_to_argb, yuv_to_argb, PixelError};

#[test]
fn test_yuv_to_argb_known_colors() {
    // Black: Y=16, U=128, V=128 -> RGB (0, 0, 0)
    let black = yuv_to_argb(16, 128, 128);
    assert_eq!(black, 0xFF000000, "Black should be 0xFF000000");

    // White: Y=235, U=128, V=128 -> RGB (255, 255, 255)
    let white = yuv_to_argb(235, 128, 128);
    assert_eq!(white, 0xFFFFFFFF, "White should be 0xFFFFFFFF");

    // Red: Y=81, U=90, V=240 -> high R, low G, low B
    let red = yuv_to_argb(81, 90, 240);
    let r = (red >> 16) & 0xFF;
    let g = (red >> 8) & 0xFF;
    let b = red & 0xFF;
    assert!(r > 200, "Red R should be > 200, got {}", r);
    assert!(g < 50, "Red G should be < 50, got {}", g);
    assert!(b < 50, "Red B should be < 50, got {}", b);

    // Green: Y=145, U=54, V=34
    let green = yuv_to_argb(145, 54, 34);
    let r = (green >> 16) & 0xFF;
    let g = (green >> 8) & 0xFF;
    let b = green & 0xFF;
    assert!(g > 200, "Green G should be > 200, got {}", g);
    assert!(r < 50, "Green R should be < 50, got {}", r);
    assert!(b < 50, "Green B should be < 50, got {}", b);
}

#[test]
fn test_yuy2_to_argb_conversion() {
    let w = 4;
    let h = 2;
    // 4x2 = 8 pixels = 4 pairs = 16 bytes
    let yuy2 = vec![
        16, 128, 16, 128, // 2 black pixels
        235, 128, 235, 128, // 2 white pixels
        81, 90, 81, 90,   // 2 reddish pixels
        145, 54, 145, 54, // 2 greenish pixels
    ];
    let mut out = vec![0u32; w * h];
    yuy2_to_argb(&yuy2, w, h, &mut out).expect("Conversion failed");

    assert_eq!(out[0], 0xFF000000);
    assert_eq!(out[1], 0xFF000000);
    assert_eq!(out[2], 0xFFFFFFFF);
    assert_eq!(out[3], 0xFFFFFFFF);
}

#[test]
fn test_nv12_to_argb_conversion() {
    let w = 4;
    let h = 2;
    // Y plane: 8 bytes
    // UV plane: 4 bytes
    let mut nv12 = vec![16u8; 8]; // Y = black
    nv12.extend_from_slice(&[128, 128, 128, 128]); // U=128, V=128

    let mut out = vec![0u32; w * h];
    nv12_to_argb(&nv12, w, h, &mut out).expect("Conversion failed");

    for p in out {
        assert_eq!(p, 0xFF000000);
    }
}

#[test]
fn test_invalid_dimensions_handling() {
    let mut out = vec![0u32; 16];
    assert_eq!(yuy2_to_argb(&[0; 32], 3, 2, &mut out), Err(PixelError::InvalidDimensions));
    assert_eq!(yuy2_to_argb(&[0; 32], 0, 2, &mut out), Err(PixelError::InvalidDimensions));
    assert_eq!(nv12_to_argb(&[0; 32], 4, 3, &mut out), Err(PixelError::InvalidDimensions));
}
