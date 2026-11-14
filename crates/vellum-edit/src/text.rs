//! Text strings, WinAnsi/Helvetica metrics for appearance streams, and PDF dates.

/// Decodes a PDF text string (UTF-16BE with BOM, UTF-8 with BOM, or PDFDocEncoding).
pub fn decode_text(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| pdfdoc_char(c)).collect()
}

fn pdfdoc_char(c: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}', '\u{2044}',
        '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}', '\u{201D}', '\u{2018}',
        '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}', '\u{0141}', '\u{0152}', '\u{0160}',
        '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}', '\u{0153}', '\u{0161}', '\u{017E}', '\u{FFFD}',
    ];
    match c {
        0x80..=0x9F => HIGH[(c - 0x80) as usize],
        0xA0 => '\u{20AC}',
        _ => c as char,
    }
}

/// Encodes a text string: PDFDocEncoding when it is plain ASCII/Latin-1, UTF-16BE otherwise.
pub fn encode_text(s: &str) -> Vec<u8> {
    if s.chars().all(|c| (c as u32) < 0x80 || (0xA1..=0xFF).contains(&(c as u32))) {
        return s.chars().map(|c| c as u8).collect();
    }
    let mut out = vec![0xFE, 0xFF];
    for u in s.encode_utf16() { out.extend_from_slice(&u.to_be_bytes()); }
    out
}

/// Maps a char to a WinAnsiEncoding byte for a simple Helvetica font (`?` when unmappable).
pub fn winansi(c: char) -> u8 {
    let u = c as u32;
    match u {
        0x20..=0x7E | 0xA0..=0xFF => u as u8,
        0x20AC => 0x80, 0x201A => 0x82, 0x0192 => 0x83, 0x201E => 0x84, 0x2026 => 0x85,
        0x2020 => 0x86, 0x2021 => 0x87, 0x02C6 => 0x88, 0x2030 => 0x89, 0x0160 => 0x8A,
        0x2039 => 0x8B, 0x0152 => 0x8C, 0x017D => 0x8E, 0x2018 => 0x91, 0x2019 => 0x92,
        0x201C => 0x93, 0x201D => 0x94, 0x2022 => 0x95, 0x2013 => 0x96, 0x2014 => 0x97,
        0x02DC => 0x98, 0x2122 => 0x99, 0x0161 => 0x9A, 0x203A => 0x9B, 0x0153 => 0x9C,
        0x017E => 0x9E, 0x0178 => 0x9F,
        0x09 => b' ',
        _ => b'?',
    }
}

/// Helvetica advance widths (1/1000 em) for WinAnsi 0x20..=0x7E (from the standard AFM).
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // space .. /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 0 .. ?
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // @ .. O
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // P .. _
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // ` .. o
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,      // p .. ~
];

/// Width of a WinAnsi byte in Helvetica, 1/1000 em.
pub fn helv_width(b: u8) -> f64 {
    match b {
        0x20..=0x7E => HELV[(b - 0x20) as usize] as f64,
        0xA0 => 278.0,
        _ => 556.0,
    }
}

/// Width of a string set in Helvetica at `size` points.
pub fn text_width(s: &str, size: f64) -> f64 {
    s.chars().map(|c| helv_width(winansi(c))).sum::<f64>() * size / 1000.0
}

/// Breaks text into lines that fit `width` at `size` (explicit newlines kept).
pub fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let para = para.trim_end_matches('\r');
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(&candidate, size) <= width || line.is_empty() {
                line = candidate;
                // A single overlong word: break by characters.
                while text_width(&line, size) > width && line.chars().count() > 1 {
                    let mut head = String::new();
                    for c in line.chars() {
                        if text_width(&format!("{head}{c}"), size) > width && !head.is_empty() { break; }
                        head.push(c);
                    }
                    let rest = line[head.len()..].to_string();
                    lines.push(head);
                    line = rest;
                }
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        lines.push(line);
    }
    lines
}

/// A string literal for a content stream, in WinAnsi bytes.
pub fn content_string(s: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in s.chars() {
        let b = winansi(c);
        match b {
            b'(' | b')' | b'\\' => { out.push(b'\\'); out.push(b); }
            0x20..=0x7E => out.push(b),
            _ => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
        }
    }
    out.push(b')');
    out
}

/// A PDF date (7.9.4). Vellum passes the wall-clock time in; this crate has no clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PdfDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Offset from UTC in minutes (east positive).
    pub utc_offset_minutes: i16,
}

impl PdfDate {
    /// `D:YYYYMMDDHHmmSS` followed by `Z` or `+HH'mm'`.
    pub fn to_pdf_string(&self) -> String {
        let mut s = format!("D:{:04}{:02}{:02}{:02}{:02}{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second);
        if self.utc_offset_minutes == 0 {
            s.push('Z');
        } else {
            let o = self.utc_offset_minutes.unsigned_abs();
            s.push_str(&format!("{}{:02}'{:02}'", if self.utc_offset_minutes < 0 { '-' } else { '+' }, o / 60, o % 60));
        }
        s
    }

    /// Parses a PDF date, tolerating missing trailing parts and a missing `D:`.
    pub fn parse(s: &str) -> Option<PdfDate> {
        let s = s.trim();
        let s = s.strip_prefix("D:").unwrap_or(s);
        let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
        if digits.len() < 4 { return None; }
        let part = |a: usize, b: usize, d: u32| digits.get(a..b).and_then(|p| p.parse::<u32>().ok()).unwrap_or(d);
        let rest = &s[digits.len()..];
        let mut off = 0i16;
        if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
            let nums: Vec<i16> = rest[1..].split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect();
            off = nums.first().copied().unwrap_or(0) * 60 + nums.get(1).copied().unwrap_or(0);
            if sign == '-' { off = -off; }
        }
        Some(PdfDate {
            year: part(0, 4, 0) as u16,
            month: part(4, 6, 1) as u8,
            day: part(6, 8, 1) as u8,
            hour: part(8, 10, 0) as u8,
            minute: part(10, 12, 0) as u8,
            second: part(12, 14, 0) as u8,
            utc_offset_minutes: off,
        })
    }
}
