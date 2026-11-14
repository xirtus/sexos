// SPDX-License-Identifier: GPL-3.0-only
//! eSCL (AirScan, Mopria eSCL 2.x): driverless scanning over HTTP.
//!
//! The flow: `GET {root}/ScannerCapabilities` (read with [`Capabilities::parse`]),
//! `POST {root}/ScanJobs` with a ScanSettings document ([`ScanSettings::to_xml`]),
//! which answers `201 Created` with a `Location:` for the job; then
//! `GET {location}/NextDocument` once per page until it answers 404.
use crate::xml::{self, Element};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const NS_SCAN: &str = "http://schemas.hp.com/imaging/escl/2011/05/03";
pub const NS_PWG: &str = "http://www.pwg.org/schemas/2010/12/sm";

/// What one input (platen or feeder) offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputCaps {
    /// In 1/300 inch.
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
    /// RGB24, Grayscale8, BlackAndWhite1.
    pub color_modes: Vec<String>,
    /// MIME types (pwg:DocumentFormat and scan:DocumentFormatExt).
    pub formats: Vec<String>,
    /// Discrete resolutions in dots per inch (x, y).
    pub resolutions: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub version: String,
    pub make_and_model: String,
    pub uuid: String,
    pub platen: Option<InputCaps>,
    pub adf: Option<InputCaps>,
}

fn num(e: &Element, name: &str) -> u32 { e.text_of(name).and_then(|t| t.parse().ok()).unwrap_or(0) }

fn input_caps(e: &Element) -> InputCaps {
    let mut c = InputCaps {
        min_width: num(e, "MinWidth"), max_width: num(e, "MaxWidth"),
        min_height: num(e, "MinHeight"), max_height: num(e, "MaxHeight"),
        ..Default::default()
    };
    let mut found = Vec::new();
    e.find_all("ColorMode", &mut found);
    for m in found.drain(..) { let t = m.text.trim().to_string(); if !t.is_empty() && !c.color_modes.contains(&t) { c.color_modes.push(t); } }
    e.find_all("DocumentFormat", &mut found);
    e.find_all("DocumentFormatExt", &mut found);
    for m in found.drain(..) { let t = m.text.trim().to_string(); if !t.is_empty() && !c.formats.contains(&t) { c.formats.push(t); } }
    e.find_all("DiscreteResolution", &mut found);
    for r in found.drain(..) {
        let v = (num(r, "XResolution"), num(r, "YResolution"));
        if v.0 > 0 && !c.resolutions.contains(&v) { c.resolutions.push(v); }
    }
    c
}

impl Capabilities {
    pub fn parse(doc: &str) -> Result<Self, xml::Error> {
        let root = xml::parse(doc)?;
        if root.name != "ScannerCapabilities" { return Err(xml::Error); }
        let mut c = Capabilities {
            version: root.text_of("Version").unwrap_or("").to_string(),
            make_and_model: root.text_of("MakeAndModel").unwrap_or("").to_string(),
            uuid: root.text_of("UUID").unwrap_or("").to_string(),
            ..Default::default()
        };
        c.platen = root.path(&["Platen", "PlatenInputCaps"]).map(input_caps);
        c.adf = root.path(&["Adf", "AdfSimplexInputCaps"]).map(input_caps);
        Ok(c)
    }

    /// The input to scan from: the platen if there is one, else the feeder.
    pub fn input(&self) -> Option<(&'static str, &InputCaps)> {
        self.platen.as_ref().map(|p| ("Platen", p)).or_else(|| self.adf.as_ref().map(|a| ("Feeder", a)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanSettings {
    pub version: String,
    /// "Platen" or "Feeder".
    pub input_source: String,
    pub color_mode: String,
    pub format: String,
    pub resolution: u32,
    /// In 1/300 inch.
    pub x_offset: u32,
    pub y_offset: u32,
    pub width: u32,
    pub height: u32,
}

impl ScanSettings {
    /// Settings for a whole page from `caps`: `want_format` if offered (else PNG,
    /// JPEG, PDF, whatever is first), the lowest resolution at or above `min_dpi`.
    pub fn choose(caps: &Capabilities, want_format: &str, gray: bool, min_dpi: u32) -> Option<Self> {
        let (source, input) = caps.input()?;
        let has = |f: &str| input.formats.iter().any(|x| x.eq_ignore_ascii_case(f));
        let format = [want_format, "image/png", "image/jpeg", "application/pdf"].iter().find(|f| has(f)).map(|s| s.to_string())
            .or_else(|| input.formats.first().cloned())?;
        let modes = if gray { ["Grayscale8", "RGB24"] } else { ["RGB24", "Grayscale8"] };
        let color_mode = modes.iter().find(|m| input.color_modes.iter().any(|x| x == *m)).map(|s| s.to_string())
            .or_else(|| input.color_modes.first().cloned()).unwrap_or_else(|| "RGB24".into());
        let mut dpis: Vec<u32> = input.resolutions.iter().filter(|r| r.0 == r.1).map(|r| r.0).collect();
        dpis.sort_unstable();
        let resolution = dpis.iter().copied().find(|d| *d >= min_dpi).or_else(|| dpis.last().copied()).unwrap_or(300);
        Some(ScanSettings {
            version: if caps.version.is_empty() { "2.0".into() } else { caps.version.clone() },
            input_source: source.into(), color_mode, format, resolution,
            x_offset: 0, y_offset: 0,
            width: if input.max_width > 0 { input.max_width } else { 2550 },
            height: if input.max_height > 0 { input.max_height } else { 3300 },
        })
    }

    pub fn to_xml(&self) -> String {
        format!(concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<scan:ScanSettings xmlns:scan=\"{}\" xmlns:pwg=\"{}\">\n",
            "  <pwg:Version>{}</pwg:Version>\n",
            "  <pwg:ScanRegions>\n",
            "    <pwg:ScanRegion>\n",
            "      <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>\n",
            "      <pwg:XOffset>{}</pwg:XOffset>\n",
            "      <pwg:YOffset>{}</pwg:YOffset>\n",
            "      <pwg:Width>{}</pwg:Width>\n",
            "      <pwg:Height>{}</pwg:Height>\n",
            "    </pwg:ScanRegion>\n",
            "  </pwg:ScanRegions>\n",
            "  <pwg:InputSource>{}</pwg:InputSource>\n",
            "  <scan:ColorMode>{}</scan:ColorMode>\n",
            "  <pwg:DocumentFormat>{}</pwg:DocumentFormat>\n",
            "  <scan:DocumentFormatExt>{}</scan:DocumentFormatExt>\n",
            "  <scan:XResolution>{}</scan:XResolution>\n",
            "  <scan:YResolution>{}</scan:YResolution>\n",
            "</scan:ScanSettings>\n"),
            NS_SCAN, NS_PWG, xml::escape(&self.version), self.x_offset, self.y_offset, self.width, self.height,
            xml::escape(&self.input_source), xml::escape(&self.color_mode), xml::escape(&self.format), xml::escape(&self.format),
            self.resolution, self.resolution)
    }
}

/// `ScannerStatus`: the scanner's state (Idle, Processing, Stopped, Testing).
pub fn scanner_state(doc: &str) -> Option<String> {
    let root = xml::parse(doc).ok()?;
    root.text_of("State").map(|s| s.to_string())
}

/// The URL paths of the flow, from the eSCL root (`/eSCL`).
pub fn capabilities_path(root: &str) -> String { format!("{}/ScannerCapabilities", root.trim_end_matches('/')) }
pub fn status_path(root: &str) -> String { format!("{}/ScannerStatus", root.trim_end_matches('/')) }
pub fn jobs_path(root: &str) -> String { format!("{}/ScanJobs", root.trim_end_matches('/')) }
pub fn next_document_path(job: &str) -> String { format!("{}/NextDocument", job.trim_end_matches('/')) }
