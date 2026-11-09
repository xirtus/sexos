// SPDX-License-Identifier: GPL-3.0-only
use sex_web::canvas::Canvases;
use sex_web::Fonts;

const FS: char = '\u{1f}';

fn ink(c: &Canvases) -> (usize, i32, i32, i32, i32) {
    let s = &c.surfaces[0].1;
    let (mut n, mut x0, mut y0, mut x1, mut y1) = (0, i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (i, p) in s.pixels.iter().enumerate() {
        if p >> 24 > 64 {
            let (x, y) = ((i as u32 % s.width) as i32, (i as u32 / s.width) as i32);
            n += 1; x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y);
        }
    }
    (n, x0, y0, x1, y1)
}

#[test]
fn rotated_text_is_drawn_rotated() {
    let mut fonts = Fonts::new().unwrap();
    let mut flat = Canvases::new();
    flat.run(&mut fonts, 1, 200, 200, &format!("fo{FS}20px sans-serif\u{1e}ft{FS}HHHHHH{FS}50{FS}100"));
    let (n, x0, y0, x1, y1) = ink(&flat);
    assert!(n > 50 && x1 - x0 > 2 * (y1 - y0), "flat text is wide: {:?}", (x0, y0, x1, y1));

    // Rotated a quarter turn about (100, 100), the same text runs downwards.
    let mut turned = Canvases::new();
    turned.run(&mut fonts, 1, 200, 200, &format!("fo{FS}20px sans-serif\u{1e}tf{FS}0{FS}1{FS}-1{FS}0{FS}100{FS}100\u{1e}ft{FS}HHHHHH{FS}-50{FS}0"));
    let (m, x0, y0, x1, y1) = ink(&turned);
    assert!(m > n / 2 && y1 - y0 > 2 * (x1 - x0), "rotated text is tall: {:?}", (x0, y0, x1, y1));
    assert!(y0 >= 45 && x0 >= 98, "rotated text runs down from (100, 50), right of the pivot: {:?}", (x0, y0, x1, y1));

    // A nonuniform scale stretches the glyphs.
    let mut wide = Canvases::new();
    wide.run(&mut fonts, 1, 400, 200, &format!("fo{FS}20px sans-serif\u{1e}tf{FS}2{FS}0{FS}0{FS}0.5{FS}0{FS}0\u{1e}ft{FS}HHHHHH{FS}10{FS}200"));
    let (_, wx0, wy0, wx1, wy1) = ink(&wide);
    let (_, fx0, fy0, fx1, fy1) = ink(&flat);
    assert!((wx1 - wx0) > 3 * (fx1 - fx0) / 2 && (wy1 - wy0) < 3 * (fy1 - fy0) / 4);
}

#[test]
fn resizing_clears_the_canvas() {
    let mut fonts = Fonts::new().unwrap();
    let mut c = Canvases::new();
    c.run(&mut fonts, 1, 20, 10, &format!("fr{FS}0{FS}0{FS}20{FS}10"));
    assert_eq!(ink(&c).0, 200);
    c.run(&mut fonts, 1, 20, 10, &format!("lw{FS}5\u{1e}sz{FS}20{FS}10"));
    assert_eq!(ink(&c).0, 0);
}

#[test]
fn draw_image_copies_another_canvas() {
    let mut fonts = Fonts::new().unwrap();
    let mut c = Canvases::new();
    // Canvas 1: red left half, green right half.
    c.run(&mut fonts, 1, 20, 10, &format!("fs{FS}c{FS}#ff0000\u{1e}fr{FS}0{FS}0{FS}10{FS}10\u{1e}fs{FS}c{FS}#00ff00\u{1e}fr{FS}10{FS}0{FS}10{FS}10"));
    // Canvas 2 copies the green half to its left edge, at the same size, then the whole
    // of canvas 1 stretched over its lower half.
    c.run(&mut fonts, 2, 20, 20, &format!("di{FS}canvas:1{FS}10{FS}0{FS}10{FS}10{FS}0{FS}0{FS}10{FS}10\u{1e}di{FS}canvas:1{FS}0{FS}0{FS}20{FS}10{FS}0{FS}10{FS}20{FS}10"));
    let s = &c.surfaces[1].1;
    let at = |x: u32, y: u32| s.pixels[(y * s.width + x) as usize];
    assert_eq!(at(0, 0), 0xFF00_FF00, "copied green");
    assert_eq!(at(9, 9), 0xFF00_FF00);
    assert_eq!(at(10, 0) >> 24, 0, "nothing drawn beside the copy");
    assert_eq!(at(2, 15), 0xFFFF_0000, "red on the left of the stretched copy");
    assert_eq!(at(17, 15), 0xFF00_FF00);
    // A missing picture draws nothing.
    c.run(&mut fonts, 3, 4, 4, &format!("di{FS}canvas:9{FS}0{FS}0{FS}4{FS}4{FS}0{FS}0{FS}4{FS}4"));
    assert!(c.surfaces[2].1.pixels.iter().all(|&p| p == 0));
}
