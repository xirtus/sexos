// SPDX-License-Identifier: GPL-3.0-only
//! Tests for format sniffing, probing, and limits admission.

use sex_codecs::{admit, probe, sniff, Error, Format, Limits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn sniff_all_supported_formats() {
    assert_eq!(sniff(&fixture("gradient_lossless.webp")), Some(Format::Webp));
    assert_eq!(sniff(&fixture("anim.webp")), Some(Format::Webp));
    assert_eq!(sniff(&fixture("still.gif")), Some(Format::Gif));
    assert_eq!(sniff(&fixture("anim.gif")), Some(Format::Gif));
    assert_eq!(sniff(&fixture("smpte32.avif")), Some(Format::Avif));
    assert_eq!(sniff(&fixture("smpte32.heic")), Some(Format::Heic));

    // Non-image bytes
    assert_eq!(sniff(b"not an image file"), None);
    assert_eq!(sniff(&[]), None);
    assert_eq!(sniff(&[0; 12]), None);
}

#[test]
fn probe_metadata_without_full_decode() {
    // WebP probe
    let p_webp = probe(&fixture("anim.webp")).unwrap();
    assert_eq!(p_webp.format, Format::Webp);
    assert_eq!((p_webp.width, p_webp.height), (8, 6));
    assert!(p_webp.is_animated);
    assert_eq!(p_webp.frame_count, Some(3));

    // GIF probe
    let p_gif = probe(&fixture("anim.gif")).unwrap();
    assert_eq!(p_gif.format, Format::Gif);
    assert_eq!((p_gif.width, p_gif.height), (8, 6));
    assert!(p_gif.is_animated);
    assert_eq!(p_gif.frame_count, Some(3));

    // AVIF probe
    let p_avif = probe(&fixture("smpte32.avif")).unwrap();
    assert_eq!(p_avif.format, Format::Avif);
    assert_eq!((p_avif.width, p_avif.height), (32, 32));
    assert!(!p_avif.is_animated);

    // HEIC probe
    let p_heic = probe(&fixture("smpte32.heic")).unwrap();
    assert_eq!(p_heic.format, Format::Heic);
    assert_eq!((p_heic.width, p_heic.height), (32, 32));
    assert!(!p_heic.is_animated);
}

#[test]
fn limits_admission_discipline() {
    assert!(admit(&Limits::PHOTO, 800, 600).is_ok());
    assert_eq!(admit(&Limits::SURFACE, 800, 600), Ok(480_000));

    // Zero side
    assert_eq!(admit(&Limits::PHOTO, 0, 100), Err(Error::TooLarge));
    assert_eq!(admit(&Limits::PHOTO, 100, 0), Err(Error::TooLarge));

    // Exceeding side limits
    assert_eq!(admit(&Limits::SURFACE, 801, 600), Err(Error::TooLarge));
    assert_eq!(admit(&Limits::SURFACE, 800, 601), Err(Error::TooLarge));

    // Arithmetic overflow in width * height
    assert_eq!(admit(&Limits::PHOTO, usize::MAX, 2), Err(Error::TooLarge));
}
