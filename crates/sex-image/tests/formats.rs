// SPDX-License-Identifier: GPL-3.0-only
//! WebP and GIF: fixtures written by Pillow and ImageMagick; `.rgba` files hold
//! Pillow's own decode of a GIF, to compare against.

use sex_image::{decode, dimensions, sniff, Animation, Error, Format, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn rgba(p: u32) -> [u8; 4] {
    [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]
}

fn worst_vs_gradient(pixels: &[u32]) -> i32 {
    let mut worst = 0;
    for y in 0..48 {
        for x in 0..64 {
            let got = rgba(pixels[y * 64 + x]);
            let want = [(x * 255 / 63) as i32, (y * 255 / 47) as i32, 128, 255];
            for c in 0..4 {
                worst = worst.max((got[c] as i32 - want[c]).abs());
            }
        }
    }
    worst
}

#[test]
fn webp_is_recognised() {
    let b = fixture("gradient_lossless.webp");
    assert_eq!(sniff(&b), Some(Format::Webp));
    assert_eq!(dimensions(&b), Ok((Format::Webp, 64, 48)));
}

#[test]
fn webp_lossless_is_exact() {
    let image = decode(&fixture("gradient_lossless.webp"), &Limits::PHOTO).unwrap();
    assert_eq!((image.width, image.height), (64, 48));
    assert_eq!(worst_vs_gradient(&image.pixels), 0);
}

#[test]
fn webp_lossy_is_close() {
    let image = decode(&fixture("gradient_lossy.webp"), &Limits::PHOTO).unwrap();
    let worst = worst_vs_gradient(&image.pixels);
    assert!(worst <= 12, "max channel error {worst}");
}

#[test]
fn webp_alpha_is_unpremultiplied() {
    let image = decode(&fixture("alpha.webp"), &Limits::PHOTO).unwrap();
    assert_eq!((image.width, image.height), (5, 3));
    assert_eq!(image.pixel(0, 0), Some(0xffff_0000));
    assert_eq!(image.pixel(1, 0), Some(0x8000_ff00));
    assert_eq!(image.pixel(2, 0).map(|p| p >> 24), Some(0));
    // Lossy colour bleeds between such tiny neighbours; Pillow (libwebp) decodes
    // these same values.
    let image = decode(&fixture("alpha_lossy.webp"), &Limits::PHOTO).unwrap();
    let row: Vec<[u8; 4]> = (0..5).map(|x| rgba(image.pixel(x, 0).unwrap())).collect();
    let pillow = [[206, 26, 0, 255], [255, 90, 71, 128], [247, 12, 5, 0], [255, 1, 0, 255], [255, 0, 0, 255]];
    for (got, want) in row.iter().zip(pillow) {
        for c in 0..4 {
            assert!((got[c] as i32 - want[c] as i32).abs() <= 2, "{row:?}");
        }
    }
}

#[test]
fn webp_animation_plays_each_frame() {
    let b = fixture("anim.webp");
    let mut a = Animation::open(&b, &Limits::PHOTO).unwrap();
    assert_eq!((a.width, a.height, a.frames(), a.loops()), (8, 6, 3, None));
    let mut canvas = vec![0u32; 48];
    for (delay, colour) in [(100, 0xFFFF_0000u32), (200, 0xFF00_FF00), (300, 0xFF00_00FF), (100, 0xFFFF_0000)] {
        assert_eq!(a.next_into(&mut canvas), Ok(delay));
        assert!(canvas.iter().all(|&p| p == colour), "{colour:08x}: {:08x}", canvas[0]);
    }
}

fn assert_like_pillow(name: &str) {
    let b = fixture(name);
    let want = fixture(&format!("{name}.rgba"));
    let image = decode(&b, &Limits::PHOTO).unwrap();
    assert_eq!(image.pixels.len() * 4, want.len(), "{name}");
    for (i, p) in image.pixels.iter().enumerate() {
        assert_eq!(rgba(*p), want[i * 4..i * 4 + 4], "{name} pixel {i}");
    }
}

#[test]
fn gif_matches_pillow() {
    assert_eq!(sniff(&fixture("still.gif")), Some(Format::Gif));
    assert_like_pillow("still.gif");
    assert_like_pillow("interlaced.gif");
    // 120x90 with 256 colours: the code table fills and clears many times.
    assert_like_pillow("noise.gif");
}

#[test]
fn gif_animation_disposes_and_loops() {
    let b = fixture("anim.gif");
    let mut a = Animation::open(&b, &Limits::PHOTO).unwrap();
    assert_eq!((a.width, a.height, a.frames(), a.loops()), (8, 6, 3, None));
    let mut canvas = vec![0u32; 48];
    let frames = [(50, 0, 0xFFFF_0000u32), (120, 2, 0xFF00_FF00), (300, 4, 0xFF00_00FF), (50, 0, 0xFFFF_0000)];
    for (delay, from, colour) in frames {
        assert_eq!(a.next_into(&mut canvas), Ok(delay));
        for y in 0..6 {
            for x in 0..8 {
                // Disposal 2 clears the last frame, so only this one's columns show.
                let want = if (from..from + 3).contains(&x) { colour } else { 0 };
                assert_eq!(canvas[y * 8 + x], want, "frame from {from}, ({x}, {y})");
            }
        }
    }
}

#[test]
fn stills_play_as_one_frame() {
    let b = fixture("gradient.png");
    let mut a = Animation::open(&b, &Limits::PHOTO).unwrap();
    assert_eq!((a.frames(), a.loops()), (1, Some(0)));
    let mut canvas = vec![0u32; 64 * 48];
    assert_eq!(a.next_into(&mut canvas), Ok(0));
    assert_eq!(worst_vs_gradient(&canvas), 0);
}

#[test]
fn damaged_and_oversized_files_are_refused() {
    let small = Limits { max_width: 10, max_height: 10, max_pixels: 100, max_input_bytes: 1 << 20 };
    assert_eq!(decode(&fixture("noise.gif"), &small), Err(Error::TooLarge));
    assert_eq!(decode(&fixture("gradient_lossy.webp"), &small), Err(Error::TooLarge));
    for name in ["noise.gif", "anim.gif", "gradient_lossy.webp", "gradient_lossless.webp", "anim.webp"] {
        let b = fixture(name);
        // Cut short, and with bytes flipped: never a panic.
        for cut in [13, 20, 40, b.len() / 2, b.len() - 1] {
            let _ = decode(&b[..cut.min(b.len())], &Limits::PHOTO);
        }
        let mut seed = 7u32;
        for _ in 0..200 {
            let mut d = b.clone();
            for _ in 0..4 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let i = 12 + (seed as usize >> 8) % (d.len() - 12);
                d[i] ^= (seed >> 3) as u8 | 1;
            }
            if let Ok(mut a) = Animation::open(&d, &Limits::PHOTO) {
                let mut canvas = vec![0u32; a.width * a.height];
                for _ in 0..4 {
                    let _ = a.next_into(&mut canvas);
                }
            }
        }
    }
}
