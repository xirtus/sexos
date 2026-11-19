// SPDX-License-Identifier: GPL-3.0-only
use batik_core::adjust::{Adjustment, Curves, Levels};
use batik_core::blend::split_argb;
use batik_core::filters::{box_blur, gaussian_blur, pixelate, unsharp_mask};

#[test]
fn test_levels_lut() {
    let levels = Levels {
        in_black: 50,
        in_white: 200,
        gamma_fp: 1.0,
        out_black: 0,
        out_white: 255,
    };
    let lut = levels.build_lut();
    assert_eq!(lut[0], 0);
    assert_eq!(lut[50], 0);
    assert_eq!(lut[200], 255);
    assert_eq!(lut[255], 255);
    // Midpoint (125) should map to ~128
    assert!((126..=129).contains(&lut[125]));
}

#[test]
fn test_curves_lut() {
    // S-curve
    let pts = vec![(0, 0), (64, 32), (192, 224), (255, 255)];
    let lut = Curves::build_lut(&pts);
    assert_eq!(lut[0], 0);
    assert_eq!(lut[64], 32);
    assert_eq!(lut[192], 224);
    assert_eq!(lut[255], 255);
}

#[test]
fn test_invert_and_desaturate() {
    let p = 0xFF102030; // A=255, R=16, G=32, B=48
    let inv = Adjustment::Invert.apply_pixel(p);
    let (a, r, g, b) = split_argb(inv);
    assert_eq!((a, r, g, b), (255, 255 - 16, 255 - 32, 255 - 48));

    let desat = Adjustment::Desaturate.apply_pixel(p);
    let (da, dr, dg, db) = split_argb(desat);
    assert_eq!(da, 255);
    assert_eq!(dr, dg);
    assert_eq!(dg, db);
}

#[test]
fn test_spatial_filters() {
    // 5x5 image with center white pixel, all others black
    let mut pixels = vec![0xFF000000u32; 25];
    pixels[12] = 0xFFFFFFFF; // (2, 2)

    // Box blur spreads white to neighbors
    let bb = box_blur(&pixels, 5, 5, 1);
    let (_a, r, _, _) = split_argb(bb[11]); // neighbor (1, 2)
    assert!(r > 0, "box blur should spread energy to neighbor");

    // Gaussian blur
    let gb = gaussian_blur(&pixels, 5, 5, 1.0);
    let (_ga, gr, _, _) = split_argb(gb[11]);
    assert!(gr > 0, "gaussian blur should spread energy to neighbor");

    // Unsharp mask
    let usm = unsharp_mask(&pixels, 5, 5, 1.0, 1.5, 0);
    assert_eq!(usm.len(), 25);

    // Pixelate 4x4 image with block size 2
    let img4 = vec![
        0xFF101010, 0xFF101010, 0xFF505050, 0xFF505050,
        0xFF101010, 0xFF101010, 0xFF505050, 0xFF505050,
        0xFF101010, 0xFF101010, 0xFF505050, 0xFF505050,
        0xFF101010, 0xFF101010, 0xFF505050, 0xFF505050,
    ];
    let pix = pixelate(&img4, 4, 4, 2);
    assert_eq!(pix[0], 0xFF101010);
    assert_eq!(pix[2], 0xFF505050);
}
