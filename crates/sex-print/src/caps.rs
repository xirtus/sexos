// SPDX-License-Identifier: GPL-3.0-only
//! What an IPP Everywhere (PWG 5100.14) or AirPrint printer says it can do, read
//! from a Get-Printer-Attributes response, and the choices Needle makes from it.
use crate::ipp::{self, Message, Value};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The attributes Needle asks for (IPP Everywhere's required printer description).
pub const REQUESTED: &[&str] = &[
    "printer-name", "printer-info", "printer-make-and-model", "printer-state", "printer-state-reasons",
    "printer-uri-supported", "printer-uuid", "document-format-supported", "document-format-default",
    "media-supported", "media-default", "sides-supported", "sides-default", "print-color-mode-supported",
    "print-color-mode-default", "printer-resolution-supported", "printer-resolution-default",
    "pwg-raster-document-resolution-supported", "pwg-raster-document-type-supported",
    "pwg-raster-document-sheet-back", "urf-supported", "color-supported", "copies-supported",
    "operations-supported", "ipp-versions-supported",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrinterCaps {
    pub name: String,
    pub info: String,
    pub make_and_model: String,
    pub state: i32,
    pub state_reasons: Vec<String>,
    pub uris: Vec<String>,
    pub uuid: String,
    pub formats: Vec<String>,
    pub media: Vec<String>,
    pub media_default: String,
    pub sides: Vec<String>,
    pub color_modes: Vec<String>,
    pub color: bool,
    /// printer-resolution-supported, dots per inch.
    pub resolutions: Vec<(i32, i32)>,
    /// pwg-raster-document-resolution-supported, dots per inch.
    pub raster_resolutions: Vec<(i32, i32)>,
    /// pwg-raster-document-type-supported: sgray_8, srgb_8, ...
    pub raster_types: Vec<String>,
    pub raster_sheet_back: String,
    /// urf-supported: V1.4, CP1, W8, SRGB24, RS300, DM1 ...
    pub urf: Vec<String>,
    pub copies_max: i32,
    pub operations: Vec<i32>,
    pub ipp_versions: Vec<String>,
}

fn dpi(v: &Value) -> Option<(i32, i32)> {
    match *v {
        // units 3 = dots per inch, 4 = dots per centimetre.
        Value::Resolution { x, y, units: 4 } => Some(((x * 254 + 50) / 100, (y * 254 + 50) / 100)),
        Value::Resolution { x, y, .. } => Some((x, y)),
        _ => None,
    }
}

impl PrinterCaps {
    pub fn from_response(m: &Message) -> Self {
        let mut c = PrinterCaps { copies_max: 1, ..Default::default() };
        for g in m.groups_of(ipp::group::PRINTER) {
            for a in &g.attributes {
                let strings = || a.strings().into_iter().map(String::from).collect::<Vec<_>>();
                let first = || a.first().and_then(Value::as_str).unwrap_or("").to_string();
                match a.name.as_str() {
                    "printer-name" => c.name = first(),
                    "printer-info" => c.info = first(),
                    "printer-make-and-model" => c.make_and_model = first(),
                    "printer-state" => c.state = a.first().and_then(Value::as_int).unwrap_or(0),
                    "printer-state-reasons" => c.state_reasons = strings(),
                    "printer-uri-supported" => c.uris = strings(),
                    "printer-uuid" => c.uuid = first(),
                    "document-format-supported" => c.formats = strings(),
                    "media-supported" => c.media = strings(),
                    "media-default" => c.media_default = first(),
                    "sides-supported" => c.sides = strings(),
                    "print-color-mode-supported" => c.color_modes = strings(),
                    "color-supported" => c.color = a.first().and_then(Value::as_bool).unwrap_or(false),
                    "printer-resolution-supported" => c.resolutions = a.values.iter().filter_map(dpi).collect(),
                    "pwg-raster-document-resolution-supported" => c.raster_resolutions = a.values.iter().filter_map(dpi).collect(),
                    "pwg-raster-document-type-supported" => c.raster_types = strings(),
                    "pwg-raster-document-sheet-back" => c.raster_sheet_back = first(),
                    "urf-supported" => c.urf = strings(),
                    "copies-supported" => if let Some(Value::Range(_, hi)) = a.first() { c.copies_max = *hi },
                    "operations-supported" => c.operations = a.values.iter().filter_map(Value::as_int).collect(),
                    "ipp-versions-supported" => c.ipp_versions = strings(),
                    _ => {}
                }
            }
        }
        c
    }

    pub fn supports(&self, format: &str) -> bool { self.formats.iter().any(|f| f.eq_ignore_ascii_case(format)) }
    pub fn duplex(&self) -> bool { self.sides.iter().any(|s| s.starts_with("two-sided")) }
    pub fn state_name(&self) -> &'static str { ipp::printer_state_name(self.state) }

    /// The page encoding Needle renders to: PWG raster first, then Apple URF.
    pub fn raster_format(&self) -> Option<&'static str> {
        if self.supports("image/pwg-raster") { Some("image/pwg-raster") } else if self.supports("image/urf") { Some("image/urf") } else { None }
    }

    /// The lowest resolution the raster format accepts (smallest job; enough for text).
    pub fn raster_dpi(&self) -> u32 {
        let from_urf = self.urf.iter().find_map(|k| k.strip_prefix("RS")).map(|r| {
            r.split('-').filter_map(|n| n.parse::<i32>().ok()).map(|n| (n, n)).collect::<Vec<_>>()
        });
        let list = if self.supports("image/pwg-raster") && !self.raster_resolutions.is_empty() {
            self.raster_resolutions.clone()
        } else {
            from_urf.unwrap_or_default()
        };
        list.iter().filter(|r| r.0 == r.1).map(|r| r.0).min().unwrap_or(300) as u32
    }

    /// Whether raster pages may be sent in colour (`srgb_8` / URF `SRGB24`).
    pub fn raster_color(&self) -> bool {
        self.raster_types.iter().any(|t| t == "srgb_8") || self.urf.iter().any(|k| k == "SRGB24")
    }

    /// The media Needle prints on: the default, else A4 or Letter if offered.
    pub fn media_choice(&self) -> &str {
        if !self.media_default.is_empty() { return &self.media_default; }
        self.media.iter().find(|m| m.starts_with("iso_a4") || m.starts_with("na_letter")).map(|s| s.as_str()).unwrap_or("iso_a4_210x297mm")
    }
}

/// A PWG 5101.1 self-describing media name's size in hundredths of millimetres:
/// `iso_a4_210x297mm`, `na_letter_8.5x11in`.
pub fn media_size(name: &str) -> Option<(u32, u32)> {
    let dims = name.rsplit('_').next()?;
    let (scale, dims) = if let Some(d) = dims.strip_suffix("mm") { (100u64, d) } else { (2540u64, dims.strip_suffix("in")?) };
    let (w, h) = dims.split_once('x')?;
    let fixed = |s: &str| -> Option<u64> {
        // Up to four decimals, kept integer.
        let (i, f) = s.split_once('.').unwrap_or((s, ""));
        let mut v: u64 = i.parse().ok()?;
        let mut div = 1u64;
        for ch in f.chars().take(4) { v = v * 10 + ch.to_digit(10)? as u64; div *= 10; }
        Some((v * scale + div / 2) / div)
    };
    Some((fixed(w)? as u32, fixed(h)? as u32))
}

/// A media size in hundredths of millimetres as PostScript points.
pub fn hmm_to_points(hmm: u32) -> u32 { ((hmm as u64 * 72 + 1270) / 2540) as u32 }
/// ... and as pixels at `dpi`.
pub fn hmm_to_pixels(hmm: u32, dpi: u32) -> u32 { ((hmm as u64 * dpi as u64 + 1270) / 2540) as u32 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_names() {
        assert_eq!(media_size("iso_a4_210x297mm"), Some((21000, 29700)));
        assert_eq!(media_size("na_letter_8.5x11in"), Some((21590, 27940)));
        assert_eq!(media_size("bogus"), None);
        assert_eq!(hmm_to_points(21000), 595);
        assert_eq!(hmm_to_points(27940), 792);
        assert_eq!(hmm_to_pixels(21000, 300), 2480);
    }
}
