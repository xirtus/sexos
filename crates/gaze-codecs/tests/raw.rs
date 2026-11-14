mod common;

use common::*;
use gaze_codecs::{decode, extract_preview, sniff, Error, Exif, Format, Limits, Support};

/// A lossless-JPEG (SOF3) header claiming 4000x3000: sensor data, never a preview.
fn lossless_jpeg() -> Vec<u8> {
    let mut j = vec![0xff, 0xd8, 0xff, 0xc3, 0, 11, 8];
    j.extend_from_slice(&3000u16.to_be_bytes());
    j.extend_from_slice(&4000u16.to_be_bytes());
    j.extend_from_slice(&[1, 1, 0x11, 0]);
    j.extend_from_slice(&[0xff, 0xd9]);
    j
}

/// A NEF-shaped file: IFD0 with a small thumbnail strip and the camera's
/// tags, a SubIFD with the full-size preview (JPEGInterchangeFormat) and one
/// with lossless sensor data; IFD1 another thumbnail.
fn nef(orientation: u16) -> Vec<u8> {
    let preview = Ifd::new().with(0x00fe, Val::Long(vec![1])).with(0x0103, Val::Short(vec![6])).with(0x0201, Val::Blob(BIG_JPEG.to_vec(), 0x0202));
    let sensor = Ifd::new().with(0x00fe, Val::Long(vec![0])).with(0x0103, Val::Short(vec![7])).with(0x0111, Val::Blob(lossless_jpeg(), 0x0117));
    let ifd0 = Ifd::new()
        .with(0x010f, Val::Ascii("NIKON CORPORATION"))
        .with(0x0110, Val::Ascii("NIKON Z 6_2"))
        .with(0x0112, Val::Short(vec![orientation]))
        .with(0x0103, Val::Short(vec![6]))
        .with(0x0111, Val::Blob(SMALL_JPEG.to_vec(), 0x0117))
        .with(0x014a, Val::Subs(vec![preview, sensor]));
    let ifd1 = Ifd::new().with(0x0201, Val::Blob(SMALL_JPEG.to_vec(), 0x0202));
    tiff(&ifd0.then(ifd1), false)
}

#[test]
fn nef_largest_preview_and_orientation() {
    let file = nef(8);
    assert_eq!(sniff(&file), Some(Format::Nef));
    let p = extract_preview(&file).unwrap();
    assert_eq!((p.width, p.height), (1200, 900));
    assert_eq!(p.jpeg, BIG_JPEG);
    let e = Exif::read(&file).unwrap();
    assert_eq!((e.make.as_deref(), e.model.as_deref(), e.orientation), (Some("NIKON CORPORATION"), Some("NIKON Z 6_2"), Some(8)));
    let img = decode(&file, &Limits::PHOTO).unwrap();
    assert_eq!((img.width, img.height), (900, 1200));
    // Turned 90 degrees counter-clockwise: the stored top-right is now top-left.
    let plain = sex_image::decode(BIG_JPEG, &Limits::PHOTO).unwrap();
    assert_eq!(img.pixel(0, 0), plain.pixel(1199, 0));
    assert_eq!(img.pixel(899, 1199), plain.pixel(0, 899));
}

#[test]
fn dng_cr2_arw_pef_orf_rw2() {
    // DNG: IFD0 is a thumbnail, SubIFDs hold the preview (Compression 7 strip) and lossless raw.
    let pv = Ifd::new().with(0x0103, Val::Short(vec![7])).with(0x0111, Val::Blob(BIG_JPEG.to_vec(), 0x0117));
    let raw = Ifd::new().with(0x0103, Val::Short(vec![7])).with(0x0111, Val::Blob(lossless_jpeg(), 0x0117));
    let dng = tiff(&Ifd::new().with(0xc612, Val::Byte(vec![1, 4, 0, 0])).with(0x014a, Val::Subs(vec![raw, pv])), true);
    assert_eq!(sniff(&dng), Some(Format::Dng));
    assert_eq!(extract_preview(&dng).unwrap().jpeg, BIG_JPEG);

    // CR2: "CR" after the header; IFD0 strip is the JPEG preview; IFD3 lossless raw.
    let ifd3 = Ifd::new().with(0x0103, Val::Short(vec![6])).with(0x0111, Val::Blob(lossless_jpeg(), 0x0117));
    let ifd0 = Ifd::new().with(0x010f, Val::Ascii("Canon")).with(0x0103, Val::Short(vec![6])).with(0x0111, Val::Blob(SMALL_JPEG.to_vec(), 0x0117));
    let cr2 = tiff_with(&ifd0.then(ifd3), false, [42, 0], b"CR\x02\0\0\0\0\0");
    assert_eq!(sniff(&cr2), Some(Format::Cr2));
    let p = extract_preview(&cr2).unwrap();
    assert_eq!((p.width, p.height, p.jpeg), (64, 48, SMALL_JPEG));

    for (make, f) in [("SONY", Format::Arw), ("PENTAX", Format::Pef), ("RICOH IMAGING COMPANY, LTD.", Format::Pef), ("OLYMPUS IMAGING CORP.", Format::Orf)] {
        let t = tiff(&Ifd::new().with(0x010f, Val::Ascii(make)).with(0x0201, Val::Blob(SMALL_JPEG.to_vec(), 0x0202)), false);
        assert_eq!(sniff(&t), Some(f), "{make}");
        assert_eq!(f.support(), Support::Preview);
        assert_eq!(decode(&t, &Limits::PHOTO).unwrap().width, 64);
    }

    // ORF and RW2 use their own TIFF magic. RW2 stores the preview inline (tag 0x2e).
    let orf = tiff_with(&Ifd::new().with(0x0201, Val::Blob(SMALL_JPEG.to_vec(), 0x0202)), false, *b"RO", &[]);
    assert_eq!(sniff(&orf), Some(Format::Orf));
    assert!(extract_preview(&orf).is_some());
    let rw2 = tiff_with(&Ifd::new().with(0x002e, Val::Undefined(SMALL_JPEG.to_vec())), false, [b'U', 0], &[]);
    assert_eq!(sniff(&rw2), Some(Format::Rw2));
    assert_eq!(extract_preview(&rw2).unwrap().jpeg, SMALL_JPEG);

    // Plain TIFF with only lossless data: no preview.
    let none = tiff(&Ifd::new().with(0x0103, Val::Short(vec![7])).with(0x0111, Val::Blob(lossless_jpeg(), 0x0117)), false);
    assert_eq!(sniff(&none), Some(Format::Tiff));
    assert_eq!(extract_preview(&none), None);
}

#[test]
fn raf_preview_and_its_own_exif() {
    let jpeg = jpeg_with_exif(SMALL_JPEG, &orientation_tiff(3));
    let mut raf = b"FUJIFILMCCD-RAW 0201FF383501X-T5".to_vec();
    raf.resize(100, 0);
    let off = raf.len() as u32;
    raf[84..88].copy_from_slice(&off.to_be_bytes());
    raf[88..92].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
    raf.extend_from_slice(&jpeg);
    raf.extend_from_slice(&[0; 64]);
    assert_eq!(sniff(&raf), Some(Format::Raf));
    assert_eq!(extract_preview(&raf).unwrap().jpeg, &jpeg[..]);
    // The RAF header is not TIFF: orientation comes from the preview's EXIF.
    let img = decode(&raf, &Limits::PHOTO).unwrap();
    let plain = sex_image::decode(SMALL_JPEG, &Limits::PHOTO).unwrap();
    assert_eq!(img.pixel(0, 0), plain.pixel(63, 47));
    // Offsets past the end.
    raf[84..88].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(decode(&raf, &Limits::PHOTO), Err(Error::NoPreview));
}

fn heif(major: &[u8; 4], compat: &[&[u8; 4]]) -> Vec<u8> {
    let mut body = major.to_vec();
    body.extend_from_slice(&[0; 4]);
    for c in compat {
        body.extend_from_slice(*c);
    }
    let mut f = isobmff_box(b"ftyp", &body);
    f.extend(isobmff_box(b"meta", &[0; 8]));
    f
}

#[test]
fn sniff_every_format() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;
    let svg_decl = "\u{feff}  <?xml version=\"1.0\"?>\n<!-- c -->\n<!DOCTYPE svg>\n<svg/>".as_bytes();
    let cases: Vec<(Vec<u8>, Option<Format>)> = vec![
        (SMALL_JPEG.to_vec(), Some(Format::Jpeg)),
        (png(&[[0; 4]], 1, 1, None), Some(Format::Png)),
        (WEBP.to_vec(), Some(Format::Webp)),
        (b"GIF89a\x01\0\x01\0".to_vec(), Some(Format::Gif)),
        (vec![0xff, 0x0a, 0xfa, 0x10], Some(Format::Jxl)),
        (jxl_container(&[0xff, 0x0a], None), Some(Format::Jxl)),
        (svg.to_vec(), Some(Format::Svg)),
        (svg_decl.to_vec(), Some(Format::Svg)),
        (heif(b"avif", &[b"mif1", b"miaf"]), Some(Format::Avif)),
        (heif(b"mif1", &[b"mif1", b"avif"]), Some(Format::Avif)),
        (heif(b"avis", &[]), Some(Format::Avif)),
        (heif(b"heic", &[b"mif1"]), Some(Format::Heic)),
        (heif(b"mif1", &[b"mif1", b"heic"]), Some(Format::Heic)),
        (heif(b"heix", &[]), Some(Format::Heic)),
        (heif(b"mif1", &[b"mif1"]), Some(Format::Heif)),
        (heif(b"crx ", &[b"isom"]), Some(Format::Cr3)),
        (heif(b"isom", &[b"mp41"]), None),
        (tiff(&Ifd::new().with(0x0100, Val::Short(vec![1])), true), Some(Format::Tiff)),
        (b"BM\x3a\0\0\0\0\0\0\0\x36\0\0\0\x28\0\0\0\x01\0\0\0\x01\0\0\0".to_vec(), Some(Format::Bmp)),
        (vec![0, 0, 1, 0, 1, 0, 16, 16], Some(Format::Ico)),
        (b"qoif\0\0\0\x01\0\0\0\x01\x04\0".to_vec(), Some(Format::Qoi)),
        (b"8BPS\0\x01".to_vec(), Some(Format::Psd)),
        (vec![0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0], Some(Format::Exr)),
        (b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n".to_vec(), Some(Format::Hdr)),
        (b"P6\n1 1\n255\n\0\0\0".to_vec(), Some(Format::Pnm)),
        (b"<html><body>no</body></html>".to_vec(), None),
        (b"<?xml version=\"1.0\"?><feed/>".to_vec(), None),
        (b"plain text".to_vec(), None),
        (vec![], None),
    ];
    for (bytes, want) in cases {
        assert_eq!(sniff(&bytes), want, "{:?}", &bytes[..bytes.len().min(16)]);
    }
    assert_eq!(Format::Avif.support(), Support::Detected);
    assert_eq!(Format::Cr3.support(), Support::Detected);
    assert_eq!(Format::Jpeg.support(), Support::Decode);
    assert_eq!(Format::Heic.media_type(), "image/heic");
    assert_eq!(decode(&heif(b"avif", &[]), &Limits::PHOTO), Err(Error::NoDecoder(Format::Avif)));
    assert_eq!(decode(b"nope", &Limits::PHOTO), Err(Error::UnknownFormat));
}
