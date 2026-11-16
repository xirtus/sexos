// SPDX-License-Identifier: GPL-3.0-only
//! Host test for Display's magnifier and colour filters (crates/sex-pdx/src/access_view.rs).
#[path = "../crates/sex-pdx/src/access_view.rs"]
mod access_view;
use access_view::*;

fn frame(w: usize, h: usize) -> Vec<u32> {
    (0..w * h).map(|i| ((i % w) as u32) << 8 | (i / w) as u32).collect()
}

#[test]
fn set_rejects_out_of_range() {
    assert!(!set(3, 0));
    assert!(!set(33, 0));
    assert!(!set(4, 6));
    assert!(set(8, 1));
    assert_eq!((zoom(), filter()), (8, Filter::HighContrast));
    assert!(active());
    assert!(set(4, 0));
    assert!(!active());
}

#[test]
fn magnifier_zooms_2x_around_pointer() {
    let (w, h) = (64usize, 48usize);
    let f = frame(w, h);
    let mut out = vec![0u32; w];
    // Pointer at (40, 30): the desktop point under it stays under it.
    let v = view(8, w, h, 40, 30);
    row(&f, w, h, 30, &v, Filter::None, &mut out);
    assert_eq!(out[40], f[30 * w + 40]);
    // Every source pixel covers 2x2 output pixels.
    assert_eq!(out[40], out[41]);
    assert_ne!(out[41], out[42]);
    assert_eq!(v.source_x(42), 41);
    let mut below = vec![0u32; w];
    row(&f, w, h, 31, &v, Filter::None, &mut below);
    assert_eq!(out, below);
    // The view covers half the screen in each direction.
    let span = v.source_x(w - 1) - v.source_x(0) + 1;
    assert_eq!(span, w / 2);
}

#[test]
fn magnifier_reaches_every_edge() {
    let (w, h) = (100usize, 80usize);
    for z in [5u8, 8, 12, 32] {
        let tl = view(z, w, h, 0, 0);
        assert_eq!((tl.source_x(0), tl.source_y(0)), (0, 0));
        let br = view(z, w, h, w - 1, h - 1);
        assert_eq!((br.source_x(w - 1), br.source_y(h - 1)), (w - 1, h - 1), "zoom {z}");
        for x in 0..w { assert!(br.source_x(x) < w); }
    }
    // 1x is the identity.
    let one = view(ZOOM_ONE, w, h, 50, 40);
    for x in 0..w { assert_eq!(one.source_x(x), x); }
}

#[test]
fn filters() {
    let white = 0x00FF_FFFF;
    let black = 0;
    assert_eq!(apply(white, Filter::Inverted), black);
    assert_eq!(apply(black, Filter::Inverted), white);
    // Inverted keeps hue: pure red stays reddish.
    let red = apply(0x00FF_0000, Filter::Inverted);
    assert!((red >> 16 & 0xFF) > (red >> 8 & 0xFF));
    let g = apply(0x0080_4020, Filter::Grayscale);
    assert_eq!(g >> 16 & 0xFF, g & 0xFF);
    assert_eq!(g >> 8 & 0xFF, g & 0xFF);
    // High contrast pushes mid greys apart.
    let dim = apply(0x0060_6060, Filter::HighContrast) & 0xFF;
    let light = apply(0x00A0_A0A0, Filter::HighContrast) & 0xFF;
    assert!(dim < 0x60 && light > 0xA0);
    assert_eq!(apply(white, Filter::HighContrast), white);
    // Colour-blind aids leave greys alone and change red/green or blue/yellow.
    for f in [Filter::RedGreen, Filter::BlueYellow] { assert_eq!(apply(0x0080_8080, f), 0x0080_8080); }
    assert_ne!(apply(0x00FF_0000, Filter::RedGreen), 0x00FF_0000);
    assert_ne!(apply(0x0000_00FF, Filter::BlueYellow), 0x0000_00FF);
    assert_eq!(apply(0xFF12_3456, Filter::None), 0x0012_3456);
}

#[test]
fn change_tracking() {
    // Under zoom, pointer motion moves the view: a change; at 1x it is not.
    set(4, 0);
    take_change();
    set_pointer(10, 10);
    assert!(!take_change());
    set(8, 0);
    assert!(take_change());
    assert!(!take_change());
    set_pointer(11, 10);
    assert!(take_change());
    set(4, 0);
    assert!(take_change());
}
