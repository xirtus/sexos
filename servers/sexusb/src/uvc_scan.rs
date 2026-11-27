// SPDX-License-Identifier: GPL-3.0-only
//! Finds a USB Video Class camera in a configuration descriptor, without allocating: the
//! control and streaming interfaces, the streaming alternate settings with their
//! isochronous (or bulk) IN endpoint, and the formats and frame sizes offered.
//!
//! sexusb logs this when a camera is plugged in (`[sexusb.uvc]`), so the first boot on a
//! laptop shows its built-in webcam; streaming builds on it (`isoch.rs`, `servers/sexuvc`).
//! Pure: `tests/sexusb_isoch.rs` runs this exact file.

use super::isoch::Endpoint;

pub const CLASS_VIDEO: u8 = 0x0E;
pub const SUB_CONTROL: u8 = 1;
pub const SUB_STREAMING: u8 = 2;

pub const MAX_ALTS: usize = 12;
pub const MAX_FORMATS: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Alt { pub alt: u8, pub ep: u8, pub isoch: bool, pub w_max_packet: u16, pub b_interval: u8, pub companion: Option<(u8, u8)> }

impl Alt {
    pub fn endpoint(&self, speed: u32) -> Endpoint { Endpoint::new(speed, self.w_max_packet, self.b_interval, self.companion) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format { Mjpeg, Yuy2, Nv12, Other }

impl Format {
    pub fn name(self) -> &'static str { match self { Format::Mjpeg => "mjpeg", Format::Yuy2 => "yuy2", Format::Nv12 => "nv12", Format::Other => "other" } }
}

/// One format and its largest frame size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatInfo { pub index: u8, pub format: Format, pub frames: u8, pub max_w: u16, pub max_h: u16 }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Camera {
    pub control_iface: u8,
    pub stream_iface: u8,
    /// UVC specification version in BCD from the VC_HEADER descriptor (e.g. 0x0100, 0x0110, 0x0150).
    pub bcd_uvc: u16,
    pub alts: [Alt; MAX_ALTS],
    pub alt_count: usize,
    pub formats: [Option<FormatInfo>; MAX_FORMATS],
    /// Streaming interfaces beyond the first (an IR camera is often a second function).
    pub more_streams: u8,
}

impl Camera {
    pub fn alts(&self) -> &[Alt] { &self.alts[..self.alt_count] }
    pub fn formats(&self) -> impl Iterator<Item = &FormatInfo> { self.formats.iter().flatten() }
    /// Bytes per interval of the largest alternate setting at `speed`.
    pub fn max_payload(&self, speed: u32) -> u32 { self.alts().iter().map(|a| a.endpoint(speed).esit_payload()).max().unwrap_or(0) }
    /// Bulk streaming (alt 0 carries the endpoint) needs no isochronous bandwidth.
    pub fn bulk(&self) -> bool { self.alts().iter().any(|a| !a.isoch) }
    /// VS_PROBE / VS_COMMIT control block size per UVC spec: 26 (1.0), 34 (1.1), 48 (1.5).
    pub fn probe_len(&self) -> u16 {
        if self.bcd_uvc >= 0x0150 { 48 }
        else if self.bcd_uvc >= 0x0110 { 34 }
        else { 26 }
    }
}

const GUID_YUY2: [u8; 4] = *b"YUY2";
const GUID_NV12: [u8; 4] = *b"NV12";

/// The first camera in `cfg` (a whole configuration descriptor).
pub fn scan(cfg: &[u8]) -> Option<Camera> {
    let mut cam = Camera::default();
    let (mut found_control, mut found_stream) = (false, false);
    let (mut iface, mut alt, mut class, mut sub) = (0u8, 0u8, 0u8, 0u8);
    let mut last_alt: Option<usize> = None;
    let mut fmt_at: Option<usize> = None;
    let mut at = 0;
    while at + 2 <= cfg.len() {
        let len = cfg[at] as usize;
        if len < 2 || at + len > cfg.len() { break; }
        let d = &cfg[at..at + len];
        match d[1] {
            4 if len >= 9 => {
                (iface, alt, class, sub) = (d[2], d[3], d[5], d[6]);
                last_alt = None;
                if class == CLASS_VIDEO && sub == SUB_CONTROL && !found_control { cam.control_iface = iface; found_control = true; }
                if class == CLASS_VIDEO && sub == SUB_STREAMING {
                    if !found_stream { cam.stream_iface = iface; found_stream = true; }
                    else if iface != cam.stream_iface && alt == 0 { cam.more_streams = cam.more_streams.saturating_add(1); }
                }
            }
            5 if len >= 7 && class == CLASS_VIDEO && sub == SUB_STREAMING && iface == cam.stream_iface && d[2] & 0x80 != 0 => {
                let kind = d[3] & 3;
                if (kind == 1 || kind == 2) && cam.alt_count < MAX_ALTS {
                    cam.alts[cam.alt_count] = Alt { alt, ep: d[2], isoch: kind == 1, w_max_packet: u16::from_le_bytes([d[4], d[5]]), b_interval: d[6], companion: None };
                    last_alt = Some(cam.alt_count);
                    cam.alt_count += 1;
                }
            }
            0x30 if len >= 4 => {
                if let Some(k) = last_alt { cam.alts[k].companion = Some((d[2], d[3])); }
            }
            // Class-specific VideoControl: VC_HEADER (0x01) carries bcdUVC at bytes 3..5.
            0x24 if len >= 5 && class == CLASS_VIDEO && sub == SUB_CONTROL && iface == cam.control_iface && d[2] == 0x01 => {
                cam.bcd_uvc = u16::from_le_bytes([d[3], d[4]]);
            }
            // Class-specific VideoStreaming: formats (uncompressed 0x04, MJPEG 0x06) and frames.
            0x24 if len >= 3 && class == CLASS_VIDEO && sub == SUB_STREAMING && iface == cam.stream_iface => match d[2] {
                0x04 | 0x06 if len >= 5 => {
                    let format = if d[2] == 0x06 { Format::Mjpeg } else if len >= 9 && d[5..9] == GUID_YUY2 { Format::Yuy2 }
                        else if len >= 9 && d[5..9] == GUID_NV12 { Format::Nv12 } else { Format::Other };
                    if let Some(slot) = cam.formats.iter().position(|f| f.is_none()) {
                        cam.formats[slot] = Some(FormatInfo { index: d[3], format, frames: d[4], max_w: 0, max_h: 0 });
                        fmt_at = Some(slot);
                    } else { fmt_at = None; }
                }
                0x05 | 0x07 if len >= 9 => {
                    if let Some(Some(f)) = fmt_at.map(|k| &mut cam.formats[k]) {
                        let (w, h) = (u16::from_le_bytes([d[5], d[6]]), u16::from_le_bytes([d[7], d[8]]));
                        if w as u32 * h as u32 > f.max_w as u32 * f.max_h as u32 { f.max_w = w; f.max_h = h; }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        at += len;
    }
    (found_control && found_stream).then_some(cam)
}
