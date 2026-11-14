//! EXIF photo metadata: found inside JPEG (APP1), PNG (eXIf), WebP (EXIF),
//! JPEG XL (Exif box), TIFF and TIFF-based camera RAW files, then read from
//! its TIFF structure.

use alloc::string::{String, ToString};

use crate::tiff::{Ifd, Tiff};

/// An unsigned EXIF rational, kept exact (exposure time 1/250 stays 1/250).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    /// The value as a float; `None` for a zero denominator.
    pub fn to_f64(self) -> Option<f64> {
        (self.den != 0).then(|| self.num as f64 / self.den as f64)
    }
}

/// A date and time as the camera's clock showed it, from
/// `YYYY:MM:DD HH:MM:SS`, plus the optional EXIF 2.31 offset from UTC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Milliseconds from the SubSecTime tag, when present.
    pub millis: Option<u16>,
    /// Minutes east of UTC from the OffsetTime tag, when present.
    pub utc_offset_minutes: Option<i16>,
}

impl DateTime {
    pub(crate) fn parse(s: &str) -> Option<DateTime> {
        let b = s.as_bytes();
        if b.len() < 19 {
            return None;
        }
        let num = |r: core::ops::Range<usize>| -> Option<u32> {
            let mut v = 0u32;
            for &c in &b[r] {
                if !c.is_ascii_digit() {
                    return None;
                }
                v = v * 10 + (c - b'0') as u32;
            }
            Some(v)
        };
        let d = DateTime {
            year: num(0..4)? as u16,
            month: num(5..7)? as u8,
            day: num(8..10)? as u8,
            hour: num(11..13)? as u8,
            minute: num(14..16)? as u8,
            second: num(17..19)? as u8,
            millis: None,
            utc_offset_minutes: None,
        };
        // Cameras with an unset clock write zeros (or blanks, refused above).
        if d.year == 0 || !(1..=12).contains(&d.month) || !(1..=31).contains(&d.day) || d.hour > 23 || d.minute > 59 || d.second > 60 {
            return None;
        }
        Some(d)
    }

    fn with_subsec(mut self, s: Option<&str>) -> DateTime {
        if let Some(s) = s {
            let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).take(3).collect();
            if !digits.is_empty() {
                let mut v: u16 = digits.parse().unwrap_or(0);
                for _ in digits.len()..3 {
                    v *= 10;
                }
                self.millis = Some(v);
            }
        }
        self
    }

    fn with_offset(mut self, s: Option<&str>) -> DateTime {
        // "+02:00" / "-05:30"
        if let Some(b) = s.map(str::as_bytes).filter(|b| b.len() >= 6 && b[3] == b':') {
            let sign = match b[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return self,
            };
            let d = |i: usize| b[i].is_ascii_digit().then(|| (b[i] - b'0') as i16);
            if let (Some(h1), Some(h2), Some(m1), Some(m2)) = (d(1), d(2), d(4), d(5)) {
                self.utc_offset_minutes = Some(sign * ((h1 * 10 + h2) * 60 + m1 * 10 + m2));
            }
        }
        self
    }
}

/// GPS position in decimal degrees (north and east positive) and metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gps {
    pub latitude: f64,
    pub longitude: f64,
    /// Metres above sea level (negative below).
    pub altitude: Option<f64>,
}

/// The photo metadata Gaze shows. Every field is optional: phones, cameras
/// and editors each write a different subset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exif {
    /// 1 to 8, as `apply_orientation` takes it. `None` when absent or out of range.
    pub orientation: Option<u8>,
    /// DateTimeOriginal, falling back to DateTimeDigitized, then DateTime.
    pub date_taken: Option<DateTime>,
    pub make: Option<String>,
    pub model: Option<String>,
    /// LensModel, prefixed by LensMake when that is not already in it.
    pub lens: Option<String>,
    pub exposure_time: Option<Rational>,
    pub f_number: Option<Rational>,
    pub iso: Option<u32>,
    /// Millimetres, as the lens reports it.
    pub focal_length: Option<Rational>,
    pub focal_length_35mm: Option<u16>,
    pub gps: Option<Gps>,
    /// PixelXDimension / PixelYDimension, else ImageWidth / ImageLength of IFD0.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub software: Option<String>,
}

// IFD0
const IMAGE_WIDTH: u16 = 0x0100;
const IMAGE_LENGTH: u16 = 0x0101;
const MAKE: u16 = 0x010f;
const MODEL: u16 = 0x0110;
const ORIENTATION: u16 = 0x0112;
const SOFTWARE: u16 = 0x0131;
const DATE_TIME: u16 = 0x0132;
const EXIF_IFD: u16 = 0x8769;
const GPS_IFD: u16 = 0x8825;
// Exif IFD
const EXPOSURE_TIME: u16 = 0x829a;
const F_NUMBER: u16 = 0x829d;
const ISO: u16 = 0x8827;
const DATE_ORIGINAL: u16 = 0x9003;
const DATE_DIGITIZED: u16 = 0x9004;
const OFFSET_TIME: u16 = 0x9010;
const OFFSET_ORIGINAL: u16 = 0x9011;
const OFFSET_DIGITIZED: u16 = 0x9012;
const SUBSEC: u16 = 0x9290;
const SUBSEC_ORIGINAL: u16 = 0x9291;
const SUBSEC_DIGITIZED: u16 = 0x9292;
const FOCAL_LENGTH: u16 = 0x920a;
const PIXEL_X: u16 = 0xa002;
const PIXEL_Y: u16 = 0xa003;
const FOCAL_35MM: u16 = 0xa405;
const LENS_MAKE: u16 = 0xa433;
const LENS_MODEL: u16 = 0xa434;
// GPS IFD
const GPS_LAT_REF: u16 = 0x0001;
const GPS_LAT: u16 = 0x0002;
const GPS_LON_REF: u16 = 0x0003;
const GPS_LON: u16 = 0x0004;
const GPS_ALT_REF: u16 = 0x0005;
const GPS_ALT: u16 = 0x0006;

impl Exif {
    /// Reads a TIFF block (starting at its `II`/`MM` byte order mark), the
    /// payload of a JPEG APP1 after `Exif\0\0`. `None` when it is not TIFF.
    pub fn parse_tiff(tiff: &[u8]) -> Option<Exif> {
        let t = Tiff::parse(tiff)?;
        let (ifd0, _) = t.ifd(t.first_ifd)?;
        let exif = ifd0.sub(EXIF_IFD, 0);
        let gps = ifd0.sub(GPS_IFD, 0);
        let both = |tag: u16| exif.and_then(|e| e.ascii(tag)).or_else(|| ifd0.ascii(tag));

        let mut out = Exif {
            orientation: ifd0.uint(ORIENTATION).filter(|o| (1..=8).contains(o)).map(|o| o as u8),
            make: ifd0.ascii(MAKE).map(ToString::to_string),
            model: ifd0.ascii(MODEL).map(ToString::to_string),
            software: ifd0.ascii(SOFTWARE).map(ToString::to_string),
            width: exif.and_then(|e| e.uint(PIXEL_X)).or_else(|| ifd0.uint(IMAGE_WIDTH)),
            height: exif.and_then(|e| e.uint(PIXEL_Y)).or_else(|| ifd0.uint(IMAGE_LENGTH)),
            ..Exif::default()
        };

        let dates = [
            (exif.and_then(|e| e.ascii(DATE_ORIGINAL)), SUBSEC_ORIGINAL, OFFSET_ORIGINAL),
            (exif.and_then(|e| e.ascii(DATE_DIGITIZED)), SUBSEC_DIGITIZED, OFFSET_DIGITIZED),
            (ifd0.ascii(DATE_TIME), SUBSEC, OFFSET_TIME),
        ];
        out.date_taken = dates.iter().find_map(|&(s, sub, off)| {
            Some(DateTime::parse(s?)?.with_subsec(both(sub)).with_offset(both(off)))
        });

        if let Some(e) = exif {
            let rat = |tag| e.rational(tag).map(|(num, den)| Rational { num, den }).filter(|r| r.den != 0);
            out.exposure_time = rat(EXPOSURE_TIME);
            out.f_number = rat(F_NUMBER);
            out.focal_length = rat(FOCAL_LENGTH);
            out.iso = e.uint(ISO).filter(|&i| i != 0);
            out.focal_length_35mm = e.uint(FOCAL_35MM).filter(|&f| f != 0).map(|f| f as u16);
            out.lens = match (e.ascii(LENS_MAKE), e.ascii(LENS_MODEL)) {
                (Some(mk), Some(md)) if !md.to_ascii_lowercase().contains(&mk.to_ascii_lowercase()) => {
                    let mut s = String::from(mk);
                    s.push(' ');
                    s.push_str(md);
                    Some(s)
                }
                (_, Some(md)) => Some(md.to_string()),
                _ => None,
            };
        }

        out.gps = gps.and_then(read_gps);
        Some(out)
    }

    /// Finds and reads the EXIF block of a JPEG, PNG, WebP, JPEG XL, TIFF or
    /// TIFF-based RAW file.
    pub fn read(file: &[u8]) -> Option<Exif> {
        Exif::parse_tiff(find_exif(file)?)
    }
}

/// Degrees, minutes and seconds (three rationals) to decimal degrees.
pub fn dms_to_degrees(d: Rational, m: Rational, s: Rational) -> Option<f64> {
    Some(d.to_f64()? + m.to_f64()? / 60.0 + s.to_f64()? / 3600.0)
}

fn read_gps(g: Ifd<'_>) -> Option<Gps> {
    let coord = |tag: u16, ref_tag: u16, neg: u8, max: f64| -> Option<f64> {
        let e = g.get(tag)?;
        let r = |i| g.rational_n(&e, i).map(|(num, den)| Rational { num, den });
        // Some writers give only degrees, or degrees and decimal minutes.
        let zero = Rational { num: 0, den: 1 };
        let v = dms_to_degrees(r(0)?, r(1).unwrap_or(zero), r(2).unwrap_or(zero))?;
        if !(0.0..=max).contains(&v) {
            return None;
        }
        let sign = match g.ascii(ref_tag).and_then(|s| s.bytes().next()) {
            Some(c) if c.to_ascii_uppercase() == neg => -1.0,
            _ => 1.0,
        };
        Some(sign * v)
    };
    let latitude = coord(GPS_LAT, GPS_LAT_REF, b'S', 90.0)?;
    let longitude = coord(GPS_LON, GPS_LON_REF, b'W', 180.0)?;
    let altitude = g.rational(GPS_ALT).filter(|r| r.1 != 0).map(|(n, d)| {
        let a = n as f64 / d as f64;
        if g.uint(GPS_ALT_REF) == Some(1) {
            -a
        } else {
            a
        }
    });
    Some(Gps { latitude, longitude, altitude })
}

const EXIF_PREFIX: &[u8] = b"Exif\0\0";

fn strip_prefix(b: &[u8]) -> &[u8] {
    b.strip_prefix(EXIF_PREFIX).unwrap_or(b)
}

/// The raw TIFF bytes of a file's EXIF block, located by container format.
pub fn find_exif(file: &[u8]) -> Option<&[u8]> {
    if file.starts_with(&[0xff, 0xd8]) {
        jpeg_exif(file)
    } else if file.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        png_exif(file)
    } else if file.len() >= 12 && file.starts_with(b"RIFF") && &file[8..12] == b"WEBP" {
        webp_exif(file)
    } else if file.starts_with(&[0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a]) {
        jxl_exif(file)
    } else if Tiff::parse(file).is_some() {
        Some(file)
    } else {
        None
    }
}

/// Walks JPEG markers to the first APP1 holding `Exif\0\0`, stopping at the scan.
pub(crate) fn jpeg_exif(file: &[u8]) -> Option<&[u8]> {
    let mut i = 2;
    loop {
        // Skip fill bytes before the marker.
        while file.get(i) == Some(&0xff) && file.get(i + 1) == Some(&0xff) {
            i += 1;
        }
        if *file.get(i)? != 0xff {
            return None;
        }
        let marker = *file.get(i + 1)?;
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([*file.get(i + 2)?, *file.get(i + 3)?]) as usize;
        if len < 2 {
            return None;
        }
        let body = file.get(i + 4..i + 2 + len)?;
        if marker == 0xe1 && body.starts_with(EXIF_PREFIX) {
            return Some(&body[EXIF_PREFIX.len()..]);
        }
        i += 2 + len;
    }
}

fn png_exif(file: &[u8]) -> Option<&[u8]> {
    let mut i = 8;
    while i + 8 <= file.len() {
        let len = u32::from_be_bytes(file[i..i + 4].try_into().ok()?) as usize;
        let kind = &file[i + 4..i + 8];
        let body = file.get(i + 8..(i + 8).checked_add(len)?)?;
        // eXIf should precede IDAT, but some writers append it: look up to IEND.
        match kind {
            b"eXIf" => return Some(strip_prefix(body)),
            b"IEND" => return None,
            _ => {}
        }
        i = i + 12 + len;
    }
    None
}

fn webp_exif(file: &[u8]) -> Option<&[u8]> {
    let mut i = 12;
    while i + 8 <= file.len() {
        let len = u32::from_le_bytes(file[i + 4..i + 8].try_into().ok()?) as usize;
        let body = file.get(i + 8..(i + 8).checked_add(len)?)?;
        if &file[i..i + 4] == b"EXIF" {
            return Some(strip_prefix(body));
        }
        i += 8 + len + (len & 1);
    }
    None
}

/// JPEG XL container: the `Exif` box holds a 4-byte offset to the TIFF header.
fn jxl_exif(file: &[u8]) -> Option<&[u8]> {
    let mut i = 0;
    while i + 8 <= file.len() {
        let size32 = u32::from_be_bytes(file[i..i + 4].try_into().ok()?) as u64;
        let kind = &file[i + 4..i + 8];
        let (header, size) = match size32 {
            0 => (8, (file.len() - i) as u64),
            1 => (16, u64::from_be_bytes(file.get(i + 8..i + 16)?.try_into().ok()?)),
            s => (8, s),
        };
        let size = usize::try_from(size).ok()?;
        if size < header {
            return None;
        }
        let body = file.get(i + header..i.checked_add(size)?)?;
        if kind == b"Exif" {
            let off = u32::from_be_bytes(body.get(..4)?.try_into().ok()?) as usize;
            return body.get(4usize.checked_add(off)?..);
        }
        i += size;
    }
    None
}
