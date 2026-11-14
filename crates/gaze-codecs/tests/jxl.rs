//! JPEG XL fixtures are encoded at test time with zune-jpegxl (lossless), so
//! decoding must give back the exact pixels.
#![cfg(feature = "jxl")]

mod common;

use common::*;
use gaze_codecs::{decode, decode_jxl, jxl_dimensions, sniff, Error, Exif, Format, Limits};
use zune_core::bit_depth::BitDepth;
use zune_core::colorspace::ColorSpace;
use zune_core::options::EncoderOptions;
use zune_jpegxl::JxlSimpleEncoder;

fn encode(data: &[u8], w: usize, h: usize, cs: ColorSpace) -> Vec<u8> {
    let mut out = Vec::new();
    JxlSimpleEncoder::new(data, EncoderOptions::new(w, h, cs, BitDepth::Eight)).encode(&mut out).unwrap();
    out
}

fn pattern(w: usize, h: usize, ch: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let px = [(x * 255 / (w - 1)) as u8, (y * 255 / (h - 1)) as u8, ((x * 7 + y * 13) % 256) as u8, (128 + x * 4) as u8];
            v.extend_from_slice(&px[..ch]);
        }
    }
    v
}

#[test]
fn rgba_round_trip() {
    let (w, h) = (24, 9);
    let src = pattern(w, h, 4);
    let file = encode(&src, w, h, ColorSpace::RGBA);
    assert_eq!(sniff(&file), Some(Format::Jxl));
    assert_eq!(jxl_dimensions(&file), Ok((w, h)));
    let img = decode_jxl(&file, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (w, h));
    let got: Vec<u8> = img.pixels.iter().flat_map(|&p| rgba(p)).collect();
    assert_eq!(got, src);
}

#[test]
fn rgb_and_gray_round_trip() {
    let (w, h) = (16, 5);
    let rgb = pattern(w, h, 3);
    let img = decode(&encode(&rgb, w, h, ColorSpace::RGB), &Limits::PHOTO).unwrap();
    for (i, &p) in img.pixels.iter().enumerate() {
        assert_eq!(rgba(p), [rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], 255]);
    }
    let gray: Vec<u8> = (0..w * h).map(|i| (i * 3) as u8).collect();
    let img = decode(&encode(&gray, w, h, ColorSpace::Luma), &Limits::PHOTO).unwrap();
    for (i, &p) in img.pixels.iter().enumerate() {
        assert_eq!(rgba(p), [gray[i], gray[i], gray[i], 255]);
    }
}

#[test]
fn container_with_exif_and_limits() {
    let (w, h) = (8, 4);
    let src = pattern(w, h, 4);
    let cs = encode(&src, w, h, ColorSpace::RGBA);
    let file = jxl_container(&cs, Some(&orientation_tiff(6)));
    assert_eq!(sniff(&file), Some(Format::Jxl));
    assert_eq!(Exif::read(&file).unwrap().orientation, Some(6));
    // The codestream's own orientation (1 here) wins: EXIF is not applied again.
    let img = decode(&file, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (w, h));
    assert_eq!(img.pixels.iter().flat_map(|&p| rgba(p)).collect::<Vec<_>>(), src);

    let tiny = Limits { max_width: 4, max_height: 4, max_pixels: 16, max_input_bytes: 1 << 20 };
    assert_eq!(decode_jxl(&cs, &tiny), Err(Error::TooLarge));
    assert_eq!(decode_jxl(&cs[..cs.len() / 2], &Limits::PHOTO).map(|_| ()).is_err(), true);
    assert_eq!(decode_jxl(&[0xff, 0x0a, 1, 2, 3], &Limits::PHOTO), Err(Error::Jxl));
}
