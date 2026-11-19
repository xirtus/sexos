// SPDX-License-Identifier: GPL-3.0-only
use batik_core::adjust::{Adjustment, auto_levels};
use batik_core::blend::split_argb;
use batik_core::filters::{edge_detect, emboss, sharpen};

#[test]
fn test_extended_convolution_filters() {
    // 5x5 image with center step edge
    let mut pixels = vec![0xFF000000u32; 25];
    for y in 0..5 {
        for x in 2..5 {
            pixels[y * 5 + x] = 0xFFFFFFFF;
        }
    }

    let sh = sharpen(&pixels, 5, 5);
    assert_eq!(sh.len(), 25);

    let em = emboss(&pixels, 5, 5);
    assert_eq!(em.len(), 25);

    let ed = edge_detect(&pixels, 5, 5);
    assert_eq!(ed.len(), 25);
    // Across the step edge (x=1 to x=2), edge detector should detect contrast
    let (_, r_edge, _, _) = split_argb(ed[2 * 5 + 2]);
    assert!(r_edge > 0, "Edge should be detected at step discontinuity");
}

#[test]
fn test_sepia_adjustment() {
    let p = 0xFF808080; // Gray
    let sep = Adjustment::Sepia.apply_pixel(p);
    let (a, r, g, b) = split_argb(sep);
    assert_eq!(a, 255);
    // Sepia tone has red > green > blue
    assert!(r >= g, "Sepia red should be >= green: r={}, g={}", r, g);
    assert!(g >= b, "Sepia green should be >= blue: g={}, b={}", g, b);
}

#[test]
fn test_auto_levels() {
    // Pixels ranging strictly from 50 to 200
    let pixels = vec![0xFF323232, 0xFF646464, 0xFFC8C8C8];
    let levels = auto_levels(&pixels);
    assert_eq!(levels.in_black, 50);
    assert_eq!(levels.in_white, 200);

    let lut = levels.build_lut();
    assert_eq!(lut[50], 0);
    assert_eq!(lut[200], 255);
}
