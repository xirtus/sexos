mod common;

use common::*;
use gaze_codecs::{dms_to_degrees, find_exif, DateTime, Exif, Rational};

fn r(num: u32, den: u32) -> Rational {
    Rational { num, den }
}

/// A camera-like EXIF block: IFD0, Exif IFD, GPS IFD (the Eiffel Tower, 330 m).
fn camera(be: bool) -> Vec<u8> {
    let exif = Ifd::new()
        .with(0x829a, Val::Rational(vec![(1, 250)]))
        .with(0x829d, Val::Rational(vec![(28, 10)]))
        .with(0x8827, Val::Short(vec![400]))
        .with(0x9003, Val::Ascii("2024:07:14 21:30:05"))
        .with(0x9004, Val::Ascii("2024:07:14 21:30:06"))
        .with(0x9011, Val::Ascii("+02:00"))
        .with(0x9291, Val::Ascii("25"))
        .with(0x920a, Val::Rational(vec![(50, 1)]))
        .with(0xa405, Val::Short(vec![75]))
        .with(0xa002, Val::Long(vec![6000]))
        .with(0xa003, Val::Long(vec![4000]))
        .with(0xa433, Val::Ascii("FUJIFILM"))
        .with(0xa434, Val::Ascii("XF35mmF1.4 R"));
    let gps = Ifd::new()
        .with(0x0000, Val::Byte(vec![2, 3, 0, 0]))
        .with(0x0001, Val::Ascii("N"))
        .with(0x0002, Val::Rational(vec![(48, 1), (51, 1), (296, 10)]))
        .with(0x0003, Val::Ascii("E"))
        .with(0x0004, Val::Rational(vec![(2, 1), (17, 1), (402, 10)]))
        .with(0x0005, Val::Byte(vec![0]))
        .with(0x0006, Val::Rational(vec![(330, 1)]));
    let ifd0 = Ifd::new()
        .with(0x010f, Val::Ascii("FUJIFILM"))
        .with(0x0110, Val::Ascii("X-T5  "))
        .with(0x0112, Val::Short(vec![6]))
        .with(0x0131, Val::Ascii("Digital Camera X-T5 Ver2.00"))
        .with(0x0132, Val::Ascii("2024:07:15 08:00:00"))
        .with(0x8769, Val::Sub(exif))
        .with(0x8825, Val::Sub(gps));
    tiff(&ifd0, be)
}

fn check_camera(e: &Exif) {
    assert_eq!(e.orientation, Some(6));
    assert_eq!(e.make.as_deref(), Some("FUJIFILM"));
    assert_eq!(e.model.as_deref(), Some("X-T5"));
    assert_eq!(e.lens.as_deref(), Some("FUJIFILM XF35mmF1.4 R"));
    assert_eq!(e.software.as_deref(), Some("Digital Camera X-T5 Ver2.00"));
    assert_eq!(e.exposure_time, Some(r(1, 250)));
    assert_eq!(e.f_number, Some(r(28, 10)));
    assert_eq!(e.f_number.unwrap().to_f64(), Some(2.8));
    assert_eq!(e.iso, Some(400));
    assert_eq!(e.focal_length, Some(r(50, 1)));
    assert_eq!(e.focal_length_35mm, Some(75));
    assert_eq!((e.width, e.height), (Some(6000), Some(4000)));
    assert_eq!(
        e.date_taken,
        Some(DateTime { year: 2024, month: 7, day: 14, hour: 21, minute: 30, second: 5, millis: Some(250), utc_offset_minutes: Some(120) })
    );
    let g = e.gps.expect("gps");
    assert!((g.latitude - 48.858_222_2).abs() < 1e-6, "{}", g.latitude);
    assert!((g.longitude - 2.294_5).abs() < 1e-6, "{}", g.longitude);
    assert_eq!(g.altitude, Some(330.0));
}

#[test]
fn tiff_little_and_big_endian() {
    check_camera(&Exif::parse_tiff(&camera(false)).unwrap());
    check_camera(&Exif::parse_tiff(&camera(true)).unwrap());
}

#[test]
fn every_container() {
    let t = camera(false);
    let jpeg = jpeg_with_exif(SMALL_JPEG, &t);
    let png = png(&[[1, 2, 3, 255]; 4], 2, 2, Some(&t));
    let mut png_prefixed_exif = b"Exif\0\0".to_vec();
    png_prefixed_exif.extend_from_slice(&t);
    let png2 = common::png(&[[1, 2, 3, 255]; 4], 2, 2, Some(&png_prefixed_exif));
    let webp = webp_with_exif(WEBP, &t);
    let jxl = jxl_container(&[0xff, 0x0a, 0, 0], Some(&t));
    for (name, file) in [("jpeg", &jpeg), ("png", &png), ("png Exif prefix", &png2), ("webp", &webp), ("jxl", &jxl), ("tiff", &t)] {
        assert_eq!(find_exif(file), Some(&t[..]), "{name}");
        check_camera(&Exif::read(file).unwrap_or_else(|| panic!("{name}")));
    }
    // The containers still decode as pictures.
    assert_eq!(sex_image::dimensions(&jpeg).unwrap().1, 64);
    assert_eq!(sex_image::decode(&png, &sex_image::Limits::PHOTO).unwrap().pixel(1, 1), Some(0xff01_0203));
    assert!(sex_image::decode(&webp, &sex_image::Limits::PHOTO).is_ok());
}

#[test]
fn no_exif() {
    assert_eq!(Exif::read(SMALL_JPEG), None);
    assert_eq!(Exif::read(WEBP), None);
    assert_eq!(Exif::read(&png(&[[0; 4]], 1, 1, None)), None);
    assert_eq!(Exif::read(b"hello"), None);
    assert_eq!(Exif::read(&[]), None);
}

#[test]
fn gps_south_west_below_sea_level() {
    // Dead Sea shore (S/W flipped for the test): 31°30'0", 35°30'0", -430.5 m.
    let gps = Ifd::new()
        .with(0x0001, Val::Ascii("S"))
        .with(0x0002, Val::Rational(vec![(31, 1), (30, 1), (0, 1)]))
        .with(0x0003, Val::Ascii("W"))
        .with(0x0004, Val::Rational(vec![(35, 1), (3000, 100), (0, 1)]))
        .with(0x0005, Val::Byte(vec![1]))
        .with(0x0006, Val::Rational(vec![(4305, 10)]));
    let e = Exif::parse_tiff(&tiff(&Ifd::new().with(0x8825, Val::Sub(gps)), true)).unwrap();
    let g = e.gps.unwrap();
    assert_eq!((g.latitude, g.longitude, g.altitude), (-31.5, -35.5, Some(-430.5)));
}

#[test]
fn gps_conversion_values() {
    // Degrees + decimal minutes, as many phones write it: 37° 46.494', 122° 25.164'.
    assert!((dms_to_degrees(r(37, 1), r(46494, 1000), r(0, 1)).unwrap() - 37.774_9).abs() < 1e-9);
    assert!((dms_to_degrees(r(122, 1), r(25164, 1000), r(0, 1)).unwrap() - 122.419_4).abs() < 1e-9);
    // 35° 39' 31.25" (Tokyo Tower area).
    assert!((dms_to_degrees(r(35, 1), r(39, 1), r(3125, 100)).unwrap() - 35.658_680_555_555_6).abs() < 1e-12);
    assert_eq!(dms_to_degrees(r(1, 0), r(0, 1), r(0, 1)), None);
    // Out-of-range latitude is dropped, and the GPS block with it.
    let gps = Ifd::new().with(0x0002, Val::Rational(vec![(91, 1), (0, 1), (0, 1)])).with(0x0004, Val::Rational(vec![(1, 1)]));
    assert_eq!(Exif::parse_tiff(&tiff(&Ifd::new().with(0x8825, Val::Sub(gps)), false)).unwrap().gps, None);
    // Only degrees.
    let gps = Ifd::new().with(0x0002, Val::Rational(vec![(10, 1)])).with(0x0004, Val::Rational(vec![(20, 1)]));
    let g = Exif::parse_tiff(&tiff(&Ifd::new().with(0x8825, Val::Sub(gps)), false)).unwrap().gps.unwrap();
    assert_eq!((g.latitude, g.longitude, g.altitude), (10.0, 20.0, None));
}

#[test]
fn fallbacks_and_odd_values() {
    // No Exif IFD: date from IFD0 DateTime, size from ImageWidth/Length, ISO as LONG elsewhere absent.
    let ifd0 = Ifd::new()
        .with(0x0100, Val::Long(vec![640]))
        .with(0x0101, Val::Short(vec![480]))
        .with(0x0112, Val::Short(vec![9]))
        .with(0x0132, Val::Ascii("2001:02:03 04:05:06"));
    let e = Exif::parse_tiff(&tiff(&ifd0, false)).unwrap();
    assert_eq!(e.orientation, None, "9 is out of range");
    assert_eq!((e.width, e.height), (Some(640), Some(480)));
    let d = e.date_taken.unwrap();
    assert_eq!((d.year, d.month, d.day, d.hour, d.minute, d.second, d.millis), (2001, 2, 3, 4, 5, 6, None));

    // Unset camera clock, zero exposure denominator, lens model already naming its maker.
    let exif = Ifd::new()
        .with(0x9003, Val::Ascii("0000:00:00 00:00:00"))
        .with(0x829a, Val::Rational(vec![(1, 0)]))
        .with(0x8827, Val::Long(vec![12800]))
        .with(0xa433, Val::Ascii("Canon"))
        .with(0xa434, Val::Ascii("Canon EF 50mm f/1.8 STM"))
        .with(0x9011, Val::Ascii("-05:30"));
    let ifd0 = Ifd::new().with(0x0132, Val::Ascii("2010:10:10 10:10:10")).with(0x8769, Val::Sub(exif));
    let e = Exif::parse_tiff(&tiff(&ifd0, true)).unwrap();
    assert_eq!(e.exposure_time, None);
    assert_eq!(e.iso, Some(12800));
    assert_eq!(e.lens.as_deref(), Some("Canon EF 50mm f/1.8 STM"));
    // DateTimeOriginal is unset, so IFD0's DateTime is used (its OffsetTime is absent).
    let d = e.date_taken.unwrap();
    assert_eq!((d.year, d.utc_offset_minutes), (2010, None));
}

#[test]
fn hostile_input_never_panics() {
    let t = camera(false);
    // Every truncation.
    for n in 0..t.len() {
        let _ = Exif::parse_tiff(&t[..n]);
    }
    // Every single-byte corruption in the structure part.
    for i in 0..t.len().min(400) {
        for v in [0x00, 0xff, 0x7f] {
            let mut c = t.clone();
            c[i] = v;
            let _ = Exif::parse_tiff(&c);
            let _ = gaze_codecs::extract_preview(&c);
            let _ = gaze_codecs::sniff(&c);
        }
    }
    // An IFD chain pointing at itself, and a sub-IFD pointing at IFD0.
    let mut looped = orientation_tiff(3);
    let n = u16::from_le_bytes([looped[8], looped[9]]) as usize;
    let next = 10 + 12 * n;
    looped[next..next + 4].copy_from_slice(&8u32.to_le_bytes());
    assert_eq!(Exif::parse_tiff(&looped).unwrap().orientation, Some(3));
    let _ = gaze_codecs::extract_preview(&looped);
    // A JPEG whose APP1 length runs past the end.
    let mut j = jpeg_with_exif(SMALL_JPEG, &t);
    j.truncate(40);
    assert_eq!(Exif::read(&j), None);
}
