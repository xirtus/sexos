// SPDX-License-Identifier: GPL-3.0-only
//! The I panel: what the photo's EXIF says, as short lines of text.
use gaze_codecs::{Exif, Format, Rational};

/// One line of the panel: a label and its value.
pub type Line = (&'static str, String);

/// Exposure time as photographers write it: `1/250 s`, `0.4 s`, `2 s`.
pub fn exposure(t: Rational) -> Option<String> {
    let v = t.to_f64()?;
    if v <= 0.0 {
        return None;
    }
    Some(if v < 0.3 {
        format!("1/{} s", (1.0 / v).round() as u64)
    } else if (v - v.round()).abs() < 0.05 {
        format!("{} s", v.round() as u64)
    } else {
        format!("{v:.1} s")
    })
}

fn decimal(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// The panel's lines for a picture, its format and its stored size.
pub fn lines(exif: Option<&Exif>, format: Option<Format>, size: Option<(usize, usize)>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    if let Some(f) = format {
        let mut s = f.media_type().to_string();
        if let Some((w, h)) = size { s = format!("{w} x {h}, {s}"); }
        out.push(("Picture", s));
    }
    let Some(e) = exif else {
        out.push(("EXIF", "none".into()));
        return out;
    };
    let camera = match (&e.make, &e.model) {
        (Some(make), Some(model)) if model.starts_with(make.as_str()) => Some(model.clone()),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (a, b) => a.clone().or_else(|| b.clone()),
    };
    if let Some(c) = camera { out.push(("Camera", c)); }
    if let Some(l) = &e.lens { out.push(("Lens", l.clone())); }
    let mut shot = Vec::new();
    if let Some(t) = e.exposure_time.and_then(exposure) { shot.push(t); }
    if let Some(f) = e.f_number.and_then(Rational::to_f64) { shot.push(format!("f/{}", decimal(f, 1))); }
    if let Some(iso) = e.iso { shot.push(format!("ISO {iso}")); }
    if let Some(mm) = e.focal_length.and_then(Rational::to_f64) {
        let mut s = format!("{} mm", decimal(mm, 1));
        if let Some(f35) = e.focal_length_35mm.filter(|f| *f as f64 != mm.round()) { s += &format!(" ({f35} mm full frame)"); }
        shot.push(s);
    }
    if !shot.is_empty() { out.push(("Exposure", shot.join("  "))); }
    if let Some(d) = e.date_taken {
        let mut s = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", d.year, d.month, d.day, d.hour, d.minute, d.second);
        if let Some(off) = d.utc_offset_minutes {
            let sign = if off < 0 { '-' } else { '+' };
            s += &format!(" {sign}{:02}:{:02}", off.unsigned_abs() / 60, off.unsigned_abs() % 60);
        }
        out.push(("Taken", s));
    }
    if let Some(g) = e.gps {
        let ns = if g.latitude < 0.0 { 'S' } else { 'N' };
        let ew = if g.longitude < 0.0 { 'W' } else { 'E' };
        let mut s = format!("{:.5} {ns}, {:.5} {ew}", g.latitude.abs(), g.longitude.abs());
        if let Some(alt) = g.altitude { s += &format!(", {} m", decimal(alt, 1)); }
        out.push(("GPS", s));
    }
    if let Some(o) = e.orientation.filter(|o| *o > 1) { out.push(("Turned", format!("upright from EXIF orientation {o}"))); }
    if let Some(s) = &e.software { out.push(("Software", s.clone())); }
    out
}

/// One line of the serial record with the panel's values, for gates and crash reports.
pub fn record(lines: &[Line]) -> String {
    let mut s = String::from("[view.info] shown=1");
    for (label, value) in lines {
        s += &format!(" {}={}", label.to_ascii_lowercase(), value.replace(' ', "_"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaze_codecs::{DateTime, Gps};

    #[test]
    fn exposure_times() {
        assert_eq!(exposure(Rational { num: 1, den: 250 }).unwrap(), "1/250 s");
        assert_eq!(exposure(Rational { num: 10, den: 600 }).unwrap(), "1/60 s");
        assert_eq!(exposure(Rational { num: 4, den: 10 }).unwrap(), "0.4 s");
        assert_eq!(exposure(Rational { num: 2, den: 1 }).unwrap(), "2 s");
        assert_eq!(exposure(Rational { num: 1, den: 0 }), None);
    }

    #[test]
    fn every_field() {
        let e = Exif {
            orientation: Some(6),
            date_taken: Some(DateTime { year: 2024, month: 5, day: 1, hour: 14, minute: 3, second: 22, millis: None, utc_offset_minutes: Some(120) }),
            make: Some("Canon".into()),
            model: Some("Canon EOS R5".into()),
            lens: Some("RF24-70mm F2.8 L IS USM".into()),
            exposure_time: Some(Rational { num: 1, den: 250 }),
            f_number: Some(Rational { num: 28, den: 10 }),
            iso: Some(100),
            focal_length: Some(Rational { num: 50, den: 1 }),
            focal_length_35mm: None,
            gps: Some(Gps { latitude: 48.858370, longitude: -2.294481, altitude: Some(35.0) }),
            ..Exif::default()
        };
        let l = lines(Some(&e), Some(Format::Jpeg), Some((240, 160)));
        let get = |k: &str| l.iter().find(|(a, _)| *a == k).map(|(_, v)| v.as_str()).unwrap();
        assert_eq!(get("Picture"), "240 x 160, image/jpeg");
        assert_eq!(get("Camera"), "Canon EOS R5");
        assert_eq!(get("Lens"), "RF24-70mm F2.8 L IS USM");
        assert_eq!(get("Exposure"), "1/250 s  f/2.8  ISO 100  50 mm");
        assert_eq!(get("Taken"), "2024-05-01 14:03:22 +02:00");
        assert_eq!(get("GPS"), "48.85837 N, 2.29448 W, 35 m");
        assert_eq!(get("Turned"), "upright from EXIF orientation 6");
        let r = record(&l);
        assert!(r.contains(" camera=Canon_EOS_R5 "), "{r}");
        assert!(r.contains(" gps=48.85837_N,_2.29448_W,_35_m"), "{r}");
    }

    #[test]
    fn no_exif() {
        let l = lines(None, Some(Format::Png), None);
        assert_eq!(l, vec![("Picture", "image/png".to_string()), ("EXIF", "none".to_string())]);
        let only_make = Exif { make: Some("Nikon".into()), ..Exif::default() };
        assert_eq!(lines(Some(&only_make), None, None), vec![("Camera", "Nikon".to_string())]);
    }
}
