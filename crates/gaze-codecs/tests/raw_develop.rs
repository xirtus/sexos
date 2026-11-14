//! Full RAW development through rawler. The DNG fixture is written at test
//! time: a 32x24 RGGB Bayer mosaic, 12-bit samples above a black level of
//! 64, camera RGB equal to linear sRGB (ColorMatrix1 = XYZ to sRGB, D65),
//! an as-shot white balance that halves red, and a 2-pixel default crop.
//! Left half: linear (0.5, 0.02, 0.02), a red. Right half: 18% grey.
#![cfg(feature = "raw")]

mod common;

use common::*;
use gaze_codecs::{decode, decode_raw, decode_with, extract_preview, sniff, Error, Exif, Format, Limits, Options, RawSize};

const W: usize = 32;
const H: usize = 24;
const BLACK: u32 = 64;
const WHITE: u32 = 4095;

/// XYZ (D65) to linear sRGB, as DNG SRATIONALs over 10^7.
const XYZ_TO_SRGB: [f64; 9] = [3.2404542, -1.5371385, -0.4985314, -0.9692660, 1.8760108, 0.0415560, 0.0556434, -0.2040259, 1.0572252];

fn scene(x: usize) -> [f64; 3] {
    if x < W / 2 {
        [0.5, 0.02, 0.02]
    } else {
        [0.18, 0.18, 0.18]
    }
}

fn dng(orientation: u16, with_preview: bool) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let c = match (y % 2, x % 2) {
                (0, 0) => 0,
                (1, 1) => 2,
                _ => 1,
            };
            // The camera sees red at half strength; AsShotNeutral 0.5 undoes it.
            let v = scene(x)[c] * if c == 0 { 0.5 } else { 1.0 };
            let s = BLACK + (v * (WHITE - BLACK) as f64).round() as u32;
            data.extend_from_slice(&(s as u16).to_le_bytes());
        }
    }
    let matrix = XYZ_TO_SRGB.iter().map(|&m| ((m * 1e7).round() as i32, 10_000_000)).collect();
    let mut ifd0 = Ifd::new()
        .with(0x00fe, Val::Long(vec![0]))
        .with(0x0100, Val::Long(vec![W as u32]))
        .with(0x0101, Val::Long(vec![H as u32]))
        .with(0x0102, Val::Short(vec![16]))
        .with(0x0103, Val::Short(vec![1]))
        .with(0x0106, Val::Short(vec![32803]))
        .with(0x010f, Val::Ascii("SexOS"))
        .with(0x0110, Val::Ascii("Test Bayer"))
        .with(0x0111, Val::Blob(data, 0x0117))
        .with(0x0112, Val::Short(vec![orientation]))
        .with(0x0115, Val::Short(vec![1]))
        .with(0x0116, Val::Long(vec![H as u32]))
        .with(0x011c, Val::Short(vec![1]))
        .with(0x828d, Val::Short(vec![2, 2]))
        .with(0x828e, Val::Byte(vec![0, 1, 1, 2]))
        .with(0xc612, Val::Byte(vec![1, 4, 0, 0]))
        .with(0xc614, Val::Ascii("SexOS Test Bayer"))
        .with(0xc61a, Val::Long(vec![BLACK]))
        .with(0xc61d, Val::Long(vec![WHITE]))
        .with(0xc61f, Val::Long(vec![2, 2]))
        .with(0xc620, Val::Long(vec![W as u32 - 4, H as u32 - 4]))
        .with(0xc621, Val::SRational(matrix))
        .with(0xc628, Val::Rational(vec![(1, 2), (1, 1), (1, 1)]))
        .with(0xc65a, Val::Short(vec![21]));
    if with_preview {
        // A reduced-size preview in a SubIFD, as real DNGs carry.
        let pv = Ifd::new().with(0x00fe, Val::Long(vec![1])).with(0x0103, Val::Short(vec![7])).with(0x0111, Val::Blob(SMALL_JPEG.to_vec(), 0x0117));
        ifd0 = ifd0.with(0x014a, Val::Subs(vec![pv]));
    }
    tiff(&ifd0, false)
}

/// Linear to sRGB-gamma 8-bit.
fn srgb8(v: f64) -> i32 {
    let g = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (g * 255.0).round() as i32
}

fn assert_near(got: u32, want: [f64; 3], what: &str) {
    let [r, g, b, a] = rgba(got);
    let w = want.map(srgb8);
    let ok = a == 255 && (r as i32 - w[0]).abs() <= 3 && (g as i32 - w[1]).abs() <= 3 && (b as i32 - w[2]).abs() <= 3;
    assert!(ok, "{what}: got {:?}, want {:?}", [r, g, b, a], w);
}

#[test]
fn half_size_superpixel() {
    let file = dng(1, false);
    assert_eq!(sniff(&file), Some(Format::Dng));
    let img = decode_raw(&file, &Limits::PHOTO, RawSize::Half).unwrap();
    // Default crop 28x20, halved.
    assert_eq!((img.width, img.height), (14, 10));
    assert_near(img.pixel(2, 5).unwrap(), [0.5, 0.02, 0.02], "red");
    assert_near(img.pixel(11, 5).unwrap(), [0.18; 3], "grey");
}

#[test]
fn full_size_ppg() {
    let file = dng(1, false);
    let img = decode_raw(&file, &Limits::PHOTO, RawSize::Full).unwrap();
    assert_eq!((img.width, img.height), (28, 20));
    // Away from the colour edge (x 14 in the cropped picture) and the borders.
    assert_near(img.pixel(5, 10).unwrap(), [0.5, 0.02, 0.02], "red");
    assert_near(img.pixel(22, 10).unwrap(), [0.18; 3], "grey");
}

#[test]
fn decode_picks_preview_unless_asked() {
    // With a preview: decode() shows it (64x48 JPEG), oriented by the DNG's EXIF.
    let file = dng(6, true);
    assert_eq!(extract_preview(&file).map(|p| p.width), Some(64));
    assert_eq!(Exif::read(&file).unwrap().model.as_deref(), Some("Test Bayer"));
    let img = decode(&file, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (48, 64));
    // Asking for development turns the sensor data upright too.
    let opt = Options { raw_develop: Some(RawSize::Half), ..Options::default() };
    let img = decode_with(&file, &opt).unwrap();
    assert_eq!((img.width, img.height), (10, 14));
    // Orientation 6: the stored left (red) column is now the top row.
    assert_near(img.pixel(5, 2).unwrap(), [0.5, 0.02, 0.02], "red on top");
    assert_near(img.pixel(5, 11).unwrap(), [0.18; 3], "grey below");
    // Without a preview, decode() develops at half size.
    let img = decode(&dng(1, false), &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (14, 10));
}

#[test]
fn limits_and_garbage() {
    let tiny = Limits { max_width: 16, max_height: 16, max_pixels: 256, max_input_bytes: 1 << 20 };
    assert_eq!(decode_raw(&dng(1, false), &tiny, RawSize::Half), Err(Error::TooLarge));
    assert_eq!(decode_raw(b"not a raw file", &Limits::PHOTO, RawSize::Half), Err(Error::Raw));
    let mut cut = dng(1, false);
    cut.truncate(300);
    assert!(decode_raw(&cut, &Limits::PHOTO, RawSize::Full).is_err());
}
