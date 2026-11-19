// SPDX-License-Identifier: GPL-3.0-only
use batik_core::geom::Rect;
use batik_core::selection::{SelectionCombine, SelectionMask};
use batik_core::transform::{
    flip_horizontal, flip_vertical, resize_bilinear, rotate_180, rotate_90_ccw, rotate_90_cw,
    transform_affine,
};

#[test]
fn test_selection_primitives() {
    let mut sel = SelectionMask::new(10, 10).unwrap();
    assert!(!sel.is_active());
    assert_eq!(sel.sample(0, 0), 255); // inactive returns 255

    // Rect selection
    sel.select_rect(Rect::new(2, 2, 4, 4), SelectionCombine::Replace);
    assert!(sel.is_active());
    assert_eq!(sel.sample(2, 2), 255);
    assert_eq!(sel.sample(5, 5), 255);
    assert_eq!(sel.sample(0, 0), 0);
    assert_eq!(sel.sample(6, 6), 0);

    // Invert
    sel.invert();
    assert_eq!(sel.sample(2, 2), 0);
    assert_eq!(sel.sample(0, 0), 255);

    // Ellipse selection
    sel.select_ellipse(Rect::new(0, 0, 10, 10), SelectionCombine::Replace);
    assert_eq!(sel.sample(5, 5), 255); // Center
    assert_eq!(sel.sample(0, 0), 0);   // Corner outside circle
}

#[test]
fn test_feathering() {
    let mut sel = SelectionMask::new(20, 20).unwrap();
    sel.select_rect(Rect::new(5, 5, 10, 10), SelectionCombine::Replace);
    assert_eq!(sel.sample(5, 5), 255);
    assert_eq!(sel.sample(4, 5), 0);

    sel.feather(2);
    // Boundary pixel at (4, 5) should now be partially selected (feathered)
    let b = sel.sample(4, 5);
    assert!(b > 0 && b < 255, "feathered boundary should be in 1..254, got {b}");
}

#[test]
fn test_magic_wand() {
    let mut pixels = vec![0xFFFFFFFFu32; 100]; // 10x10 white
    // Make a 3x3 red square at (2, 2)
    for y in 2..5 {
        for x in 2..5 {
            pixels[y * 10 + x] = 0xFFFF0000;
        }
    }

    let mut sel = SelectionMask::new(10, 10).unwrap();
    sel.magic_wand(&pixels, 10, 10, 3, 3, 10, true, SelectionCombine::Replace);

    assert_eq!(sel.sample(3, 3), 255);
    assert_eq!(sel.sample(2, 2), 255);
    assert_eq!(sel.sample(0, 0), 0);
    assert_eq!(sel.sample(5, 5), 0);
}

#[test]
fn test_transforms() {
    let mut pixels = vec![
        1, 2,
        3, 4,
    ];
    flip_horizontal(&mut pixels, 2, 2);
    assert_eq!(pixels, vec![2, 1, 4, 3]);

    flip_vertical(&mut pixels, 2, 2);
    assert_eq!(pixels, vec![4, 3, 2, 1]);

    rotate_180(&mut pixels, 2, 2);
    assert_eq!(pixels, vec![1, 2, 3, 4]);

    let cw = rotate_90_cw(&pixels, 2, 2);
    assert_eq!(cw, vec![3, 1, 4, 2]);

    let ccw = rotate_90_ccw(&pixels, 2, 2);
    assert_eq!(ccw, vec![2, 4, 1, 3]);

    // Bilinear scaling 2x2 to 4x4
    let scaled = resize_bilinear(&pixels, 2, 2, 4, 4).unwrap();
    assert_eq!(scaled.len(), 16);

    // Affine transform identity
    let ident = transform_affine(&pixels, 2, 2, 2, 2, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    assert_eq!(ident, pixels);
}
