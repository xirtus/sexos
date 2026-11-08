// SPDX-License-Identifier: GPL-3.0-only
//! Behaviour tests: fixtures were written by Pillow and ImageMagick, so these
//! check zune's output against an independent encoder.

use sex_image::{decode, dimensions, sniff, Error, Format, Image, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn split(p: u32) -> [i32; 4] {
    [(p >> 24) as i32, ((p >> 16) & 0xff) as i32, ((p >> 8) & 0xff) as i32, (p & 0xff) as i32]
}

/// The fixtures' source picture: red across, green down, blue 128.
fn gradient(x: usize, y: usize) -> [i32; 4] {
    [255, (x * 255 / 63) as i32, (y * 255 / 47) as i32, 128]
}

fn assert_gradient(image: &Image, tolerance: i32) {
    assert_eq!((image.width, image.height), (64, 48));
    assert_eq!(image.pixels.len(), 64 * 48);
    let mut worst = 0;
    for y in 0..48 {
        for x in 0..64 {
            let got = split(image.pixel(x, y).unwrap());
            let want = gradient(x, y);
            for c in 0..4 {
                worst = worst.max((got[c] - want[c]).abs());
            }
        }
    }
    assert!(worst <= tolerance, "max channel error {worst} > {tolerance}");
}

#[test]
fn png_rgb_is_exact() {
    assert_gradient(&decode(&fixture("gradient.png"), &Limits::PHOTO).unwrap(), 0);
}

#[test]
fn png_adam7_is_exact() {
    assert_gradient(&decode(&fixture("interlaced.png"), &Limits::PHOTO).unwrap(), 0);
}

#[test]
fn png_alpha_is_unpremultiplied() {
    let image = decode(&fixture("alpha.png"), &Limits::PHOTO).unwrap();
    assert_eq!((image.width, image.height), (5, 3));
    assert_eq!(image.pixel(0, 0), Some(0xffff_0000));
    assert_eq!(image.pixel(1, 0), Some(0x8000_ff00));
    assert_eq!(image.pixel(2, 0).map(|p| p >> 24), Some(0));
    assert_eq!(image.pixel(4, 2), Some(0xffff_0000));
    assert_eq!(image.pixel(5, 0), None);
}

#[test]
fn png_gray_palette_and_sixteen_bit() {
    let gray = decode(&fixture("gray.png"), &Limits::PHOTO).unwrap();
    let p = split(gray.pixel(63, 0).unwrap());
    assert_eq!(p[0], 255);
    assert_eq!(p[1], p[2]);
    assert_eq!(p[2], p[3]);

    let palette = decode(&fixture("palette.png"), &Limits::PHOTO).unwrap();
    assert_eq!((palette.width, palette.height), (64, 48));
    assert!(palette.pixels.iter().all(|p| p >> 24 == 0xff));

    let deep = decode(&fixture("gray16.png"), &Limits::PHOTO).unwrap();
    // 40000 / 257 = 155.6, so the high byte is 156 (0x9c).
    assert_eq!(deep.pixel(0, 0), Some(0xff9c_9c9c));
}

#[test]
fn jpeg_baseline_444() {
    assert_gradient(&decode(&fixture("gradient_q95.jpg"), &Limits::PHOTO).unwrap(), 6);
}

#[test]
fn jpeg_subsampled_420() {
    assert_gradient(&decode(&fixture("gradient_420.jpg"), &Limits::PHOTO).unwrap(), 10);
}

#[test]
fn jpeg_progressive() {
    assert_gradient(&decode(&fixture("gradient_progressive.jpg"), &Limits::PHOTO).unwrap(), 10);
}

#[test]
fn jpeg_cmyk() {
    assert_gradient(&decode(&fixture("cmyk.jpg"), &Limits::PHOTO).unwrap(), 10);
}

#[test]
fn jpeg_grayscale() {
    let image = decode(&fixture("gray.jpg"), &Limits::PHOTO).unwrap();
    assert_eq!((image.width, image.height), (64, 48));
    for &p in &image.pixels {
        let [a, r, g, b] = split(p);
        assert_eq!(a, 255);
        assert!((r - g).abs() <= 1 && (g - b).abs() <= 1);
    }
}

#[test]
fn sniff_and_dimensions() {
    assert_eq!(sniff(&fixture("gradient.png")), Some(Format::Png));
    assert_eq!(sniff(&fixture("gray.jpg")), Some(Format::Jpeg));
    assert_eq!(sniff(b"GIF89a"), None);
    assert_eq!(dimensions(&fixture("big.jpg")), Ok((Format::Jpeg, 1200, 900)));
    assert_eq!(dimensions(&fixture("big.png")), Ok((Format::Png, 1200, 900)));
    assert_eq!(decode(b"hello", &Limits::PHOTO), Err(Error::UnknownFormat));
}

#[test]
fn limits_refuse_before_allocating() {
    assert_eq!(decode(&fixture("big.jpg"), &Limits::SURFACE), Err(Error::TooLarge));
    assert!(decode(&fixture("big.png"), &Limits::SURFACE).is_err());
    let tight = Limits { max_pixels: 1000, ..Limits::PHOTO };
    assert_eq!(decode(&fixture("big.jpg"), &tight), Err(Error::TooLarge));
    let tiny_input = Limits { max_input_bytes: 100, ..Limits::PHOTO };
    assert_eq!(decode(&fixture("big.jpg"), &tiny_input), Err(Error::InputTooLarge));
}

#[test]
fn fit_within_keeps_aspect_and_colour() {
    let image = decode(&fixture("big.jpg"), &Limits::PHOTO).unwrap();
    let fitted = image.fit_within(800, 600).unwrap();
    assert_eq!((fitted.width, fitted.height), (800, 600));
    let [a, r, g, b] = split(fitted.pixel(400, 300).unwrap());
    assert_eq!(a, 255);
    assert!((r - 10).abs() <= 3 && (g - 20).abs() <= 3 && (b - 30).abs() <= 3);

    let wide = Image { width: 400, height: 100, pixels: vec![0xff00_00ff; 40_000] };
    let fitted = wide.fit_within(100, 100).unwrap();
    assert_eq!((fitted.width, fitted.height), (100, 25));
    assert!(fitted.pixels.iter().all(|&p| p == 0xff00_00ff));

    let small = Image { width: 2, height: 2, pixels: vec![1, 2, 3, 4] };
    assert_eq!(small.clone().fit_within(10, 10).unwrap(), small);
}

/// Cheap deterministic generator so the test needs no extra crates.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Damaged files must come back as errors or pictures, never as panics,
/// hangs or allocations past the limits.
#[test]
fn corrupted_files_never_panic() {
    let limits = Limits { max_width: 2048, max_height: 2048, max_pixels: 2048 * 2048, max_input_bytes: 1 << 20 };
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    for name in [
        "gradient.png", "interlaced.png", "alpha.png", "palette.png", "gray16.png",
        "gradient_q95.jpg", "gradient_420.jpg", "gradient_progressive.jpg", "cmyk.jpg", "gray.jpg",
    ] {
        let original = fixture(name);
        for round in 0..300 {
            let mut bytes = original.clone();
            match round % 3 {
                0 => {
                    let flips = 1 + (rng.next() % 8) as usize;
                    for _ in 0..flips {
                        let i = (rng.next() as usize) % bytes.len();
                        bytes[i] ^= 1 << (rng.next() % 8);
                    }
                }
                1 => bytes.truncate((rng.next() as usize) % bytes.len()),
                _ => {
                    let i = 8 + (rng.next() as usize) % (bytes.len() - 8);
                    bytes[i] = rng.next() as u8;
                    let last = bytes.len() - 1;
                    bytes[(i + 1).min(last)] = 0xff;
                }
            }
            if let Ok(image) = decode(&bytes, &limits) {
                assert_eq!(image.pixels.len(), image.width * image.height);
                assert!(image.width <= limits.max_width && image.height <= limits.max_height);
            }
        }
    }
}
