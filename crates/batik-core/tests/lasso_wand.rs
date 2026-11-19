// SPDX-License-Identifier: GPL-3.0-only
use batik_core::selection::{SelectionCombine, SelectionMask};

#[test]
fn test_lasso_polygon_selection() {
    let mut sel = SelectionMask::new(10, 10).unwrap();
    // Triangle: (2,2), (8,2), (5,8)
    let poly = vec![(2, 2), (8, 2), (5, 8)];
    sel.select_lasso(&poly, SelectionCombine::Replace);
    assert!(sel.is_active());

    // Inside triangle (5, 4) should be selected (255)
    assert_eq!(sel.sample(5, 4), 255);
    // Outside triangle (0, 0) should be 0
    assert_eq!(sel.sample(0, 0), 0);
    // (1, 9) should be 0
    assert_eq!(sel.sample(1, 9), 0);
}

#[test]
fn test_magic_wand_selection_and_tolerance() {
    let mut pixels = vec![0xFF000000u32; 100]; // 10x10 black
    // Place a 4x4 red patch at (3,3)
    for y in 3..7 {
        for x in 3..7 {
            pixels[y * 10 + x] = 0xFFFF0000;
        }
    }
    // Place a close-to-red pixel next to it (diff 10)
    pixels[3 * 10 + 2] = 0xFFF50000;

    let mut sel = SelectionMask::new(10, 10).unwrap();
    // Wand with tolerance 5 at (3,3): should select the exact red patch, but NOT diff 10
    sel.select_magic_wand(&pixels, 3, 3, 5, true, SelectionCombine::Replace);
    assert_eq!(sel.sample(3, 3), 255);
    assert_eq!(sel.sample(6, 6), 255);
    assert_eq!(sel.sample(2, 3), 0);
    assert_eq!(sel.sample(0, 0), 0);

    // Wand with tolerance 20 at (3,3): should expand to include (2, 3)
    let mut sel2 = SelectionMask::new(10, 10).unwrap();
    sel2.select_magic_wand(&pixels, 3, 3, 20, true, SelectionCombine::Replace);
    assert_eq!(sel2.sample(2, 3), 255);
}
