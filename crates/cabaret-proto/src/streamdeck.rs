//! Native Elgato Stream Deck USB HID Protocol.
//!
//! Supports Stream Deck Original, Mini, Mk2, XL, and Plus (VID `0x0fd9`):
//! - Model metadata (PID, key grid, rotary dials, icon dimensions, image format)
//! - HID input report decoding (key bitmasks, edge events, Plus rotary dials & touch strip)
//! - HID output packet chunking (brightness feature report, reset, BMP/JPEG key icon framing)

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

/// USB Vendor ID for all Elgato Stream Deck devices.
pub const ELGATO_VID: u16 = 0x0fd9;

/// Known Elgato Stream Deck USB Product IDs.
pub mod pid {
    pub const ORIGINAL: u16 = 0x0060;
    pub const ORIGINAL_V2: u16 = 0x006d;
    pub const MINI: u16 = 0x0063;
    pub const MINI_V2: u16 = 0x0090;
    pub const XL: u16 = 0x006c;
    pub const XL_V2: u16 = 0x008f;
    pub const MK2: u16 = 0x0080;
    pub const PLUS: u16 = 0x0084;
}

/// Image encoding expected by a Stream Deck hardware revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDeckImageFormat {
    Bmp,
    Jpeg,
}

/// Supported Elgato Stream Deck hardware models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDeckModel {
    Original,
    Mini,
    Mk2,
    Xl,
    Plus,
}

impl StreamDeckModel {
    /// USB Vendor ID (`0x0fd9`).
    pub const fn vid(self) -> u16 {
        ELGATO_VID
    }

    /// Primary USB Product ID for this model.
    pub const fn pid(self) -> u16 {
        match self {
            Self::Original => pid::ORIGINAL,
            Self::Mini => pid::MINI,
            Self::Mk2 => pid::MK2,
            Self::Xl => pid::XL,
            Self::Plus => pid::PLUS,
        }
    }

    /// Identifies a Stream Deck model from USB `(vid, pid)`.
    pub const fn from_vid_pid(vid: u16, product_id: u16) -> Option<Self> {
        if vid != ELGATO_VID {
            return None;
        }
        match product_id {
            pid::ORIGINAL | pid::ORIGINAL_V2 => Some(Self::Original),
            pid::MINI | pid::MINI_V2 => Some(Self::Mini),
            pid::MK2 => Some(Self::Mk2),
            pid::XL | pid::XL_V2 => Some(Self::Xl),
            pid::PLUS => Some(Self::Plus),
            _ => None,
        }
    }

    /// Number of key columns.
    pub const fn cols(self) -> u8 {
        match self {
            Self::Original | Self::Mk2 => 5,
            Self::Mini => 3,
            Self::Xl => 8,
            Self::Plus => 4,
        }
    }

    /// Number of key rows.
    pub const fn rows(self) -> u8 {
        match self {
            Self::Original | Self::Mk2 => 3,
            Self::Mini | Self::Plus => 2,
            Self::Xl => 4,
        }
    }

    /// Total number of programmable LCD keys.
    pub const fn key_count(self) -> usize {
        (self.cols() as usize) * (self.rows() as usize)
    }

    /// Number of rotary push dials (4 on Stream Deck Plus, 0 on others).
    pub const fn dial_count(self) -> usize {
        match self {
            Self::Plus => 4,
            _ => 0,
        }
    }

    /// Key icon dimensions `(width_px, height_px)`.
    pub const fn icon_dimensions(self) -> (u16, u16) {
        match self {
            Self::Original | Self::Mk2 => (72, 72),
            Self::Mini => (80, 80),
            Self::Xl => (96, 96),
            Self::Plus => (120, 120),
        }
    }

    /// Key icon image format expected by the device firmware.
    pub const fn image_format(self) -> StreamDeckImageFormat {
        match self {
            Self::Original | Self::Mini => StreamDeckImageFormat::Bmp,
            Self::Mk2 | Self::Xl | Self::Plus => StreamDeckImageFormat::Jpeg,
        }
    }

    /// HID output packet size in bytes for key image transfers.
    pub const fn packet_size(self) -> usize {
        match self {
            Self::Original => 8191,
            Self::Mini | Self::Mk2 | Self::Xl | Self::Plus => 1024,
        }
    }

    /// Returns `true` if this model uses v1 HID framing (Original) rather than Gen2 HID framing.
    pub const fn is_v1_hid(self) -> bool {
        matches!(self, Self::Original)
    }
}

/// Errors produced by Stream Deck HID encoding or decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamDeckError {
    InvalidReportId(u8),
    ReportTooShort { expected: usize, actual: usize },
    UnsupportedInputType { model: StreamDeckModel, input_type: u8 },
    InvalidKeyIndex { key_index: u8, key_count: usize },
    EmptyImagePayload,
    InvalidRgbBufferLen { expected: usize, actual: usize },
    PayloadTooLarge,
}

impl fmt::Display for StreamDeckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReportId(id) => write!(f, "invalid HID report id 0x{id:02x}"),
            Self::ReportTooShort { expected, actual } => {
                write!(f, "HID report too short: expected >= {expected} bytes, got {actual}")
            }
            Self::UnsupportedInputType { model, input_type } => {
                write!(f, "unsupported input type 0x{input_type:02x} for {model:?}")
            }
            Self::InvalidKeyIndex {
                key_index,
                key_count,
            } => {
                write!(f, "key index {key_index} out of bounds (max {key_count})")
            }
            Self::EmptyImagePayload => f.write_str("key image payload must not be empty"),
            Self::InvalidRgbBufferLen { expected, actual } => {
                write!(f, "invalid RGB24 buffer length: expected {expected} bytes, got {actual}")
            }
            Self::PayloadTooLarge => f.write_str("image payload exceeds 65535 HID packets"),
        }
    }
}

/// Decoded single HID input report from a Stream Deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamDeckInputReport {
    Keys {
        states: Vec<bool>,
        pressed_mask: u64,
    },
    DialsRotate {
        deltas: [i8; 4],
    },
    DialsPress {
        states: [bool; 4],
        pressed_mask: u8,
    },
    TouchTap {
        x: u16,
        y: u16,
        long: bool,
    },
    TouchSwipe {
        start_x: u16,
        start_y: u16,
        end_x: u16,
        end_y: u16,
    },
}

/// Edge-triggered high-level Stream Deck input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDeckEvent {
    KeyDown(u8),
    KeyUp(u8),
    DialRotate { dial: u8, delta: i8 },
    DialDown(u8),
    DialUp(u8),
    TouchTap { x: u16, y: u16, long: bool },
    TouchSwipe {
        start_x: u16,
        start_y: u16,
        end_x: u16,
        end_y: u16,
    },
}

/// Packs a slice of boolean key states into a `u64` bitmask (bit `i` = key `i`).
pub fn key_bitmask(states: &[bool]) -> u64 {
    let mut mask = 0u64;
    for (idx, &pressed) in states.iter().take(64).enumerate() {
        if pressed {
            mask |= 1u64 << idx;
        }
    }
    mask
}

/// Computes `(newly_pressed_mask, newly_released_mask)` from two key bitmasks.
pub const fn diff_bitmask(prev: u64, next: u64) -> (u64, u64) {
    let changed = prev ^ next;
    (changed & next, changed & prev)
}

/// Decodes a raw USB HID input report for `model`.
pub fn decode_input_report(
    model: StreamDeckModel,
    report: &[u8],
) -> Result<StreamDeckInputReport, StreamDeckError> {
    if report.is_empty() {
        return Err(StreamDeckError::ReportTooShort {
            expected: 1,
            actual: 0,
        });
    }
    if report[0] != 0x01 {
        return Err(StreamDeckError::InvalidReportId(report[0]));
    }

    let key_count = model.key_count();
    if model.is_v1_hid() {
        let expected = 1 + key_count;
        if report.len() < expected {
            return Err(StreamDeckError::ReportTooShort {
                expected,
                actual: report.len(),
            });
        }
        let states: Vec<bool> = report[1..1 + key_count].iter().map(|&b| b != 0).collect();
        let pressed_mask = key_bitmask(&states);
        return Ok(StreamDeckInputReport::Keys {
            states,
            pressed_mask,
        });
    }

    // Gen2 HID input reports (Mini v2 / Mk2 / XL / Plus):
    // [0x01, input_type, _, _, payload...]
    if report.len() < 4 {
        return Err(StreamDeckError::ReportTooShort {
            expected: 4,
            actual: report.len(),
        });
    }
    let input_type = report[1];
    match input_type {
        0x00 => {
            let expected = 4 + key_count;
            if report.len() < expected {
                return Err(StreamDeckError::ReportTooShort {
                    expected,
                    actual: report.len(),
                });
            }
            let states: Vec<bool> = report[4..4 + key_count].iter().map(|&b| b != 0).collect();
            let pressed_mask = key_bitmask(&states);
            Ok(StreamDeckInputReport::Keys {
                states,
                pressed_mask,
            })
        }
        0x02 if model == StreamDeckModel::Plus => {
            if report.len() < 10 {
                return Err(StreamDeckError::ReportTooShort {
                    expected: 10,
                    actual: report.len(),
                });
            }
            let subtype = report[4];
            let x = u16::from_le_bytes([report[6], report[7]]);
            let y = u16::from_le_bytes([report[8], report[9]]);
            match subtype {
                0x01 => Ok(StreamDeckInputReport::TouchTap { x, y, long: false }),
                0x02 => Ok(StreamDeckInputReport::TouchTap { x, y, long: true }),
                0x03 => {
                    if report.len() < 14 {
                        return Err(StreamDeckError::ReportTooShort {
                            expected: 14,
                            actual: report.len(),
                        });
                    }
                    let end_x = u16::from_le_bytes([report[10], report[11]]);
                    let end_y = u16::from_le_bytes([report[12], report[13]]);
                    Ok(StreamDeckInputReport::TouchSwipe {
                        start_x: x,
                        start_y: y,
                        end_x,
                        end_y,
                    })
                }
                _ => Err(StreamDeckError::UnsupportedInputType { model, input_type }),
            }
        }
        0x03 if model == StreamDeckModel::Plus => {
            if report.len() < 9 {
                return Err(StreamDeckError::ReportTooShort {
                    expected: 9,
                    actual: report.len(),
                });
            }
            let subtype = report[4];
            match subtype {
                0x00 => {
                    let states = [
                        report[5] != 0,
                        report[6] != 0,
                        report[7] != 0,
                        report[8] != 0,
                    ];
                    let mut pressed_mask = 0u8;
                    for (i, &st) in states.iter().enumerate() {
                        if st {
                            pressed_mask |= 1u8 << i;
                        }
                    }
                    Ok(StreamDeckInputReport::DialsPress {
                        states,
                        pressed_mask,
                    })
                }
                0x01 => {
                    let deltas = [
                        report[5] as i8,
                        report[6] as i8,
                        report[7] as i8,
                        report[8] as i8,
                    ];
                    Ok(StreamDeckInputReport::DialsRotate { deltas })
                }
                _ => Err(StreamDeckError::UnsupportedInputType { model, input_type }),
            }
        }
        _ => Err(StreamDeckError::UnsupportedInputType { model, input_type }),
    }
}

/// Stateful decoder that converts raw HID input reports into edge-triggered [`StreamDeckEvent`]s.
#[derive(Debug, Clone)]
pub struct StreamDeckInputDecoder {
    model: StreamDeckModel,
    prev_key_mask: u64,
    prev_dial_mask: u8,
}

impl StreamDeckInputDecoder {
    pub fn new(model: StreamDeckModel) -> Self {
        Self {
            model,
            prev_key_mask: 0,
            prev_dial_mask: 0,
        }
    }

    pub fn model(&self) -> StreamDeckModel {
        self.model
    }

    pub fn key_mask(&self) -> u64 {
        self.prev_key_mask
    }

    pub fn dial_mask(&self) -> u8 {
        self.prev_dial_mask
    }

    /// Feeds one HID input report and returns any edge-triggered events.
    pub fn process_report(&mut self, report: &[u8]) -> Result<Vec<StreamDeckEvent>, StreamDeckError> {
        let decoded = decode_input_report(self.model, report)?;
        let mut events = Vec::new();
        match decoded {
            StreamDeckInputReport::Keys { pressed_mask, .. } => {
                let (down, up) = diff_bitmask(self.prev_key_mask, pressed_mask);
                for idx in 0..self.model.key_count() {
                    let bit = 1u64 << idx;
                    if (down & bit) != 0 {
                        events.push(StreamDeckEvent::KeyDown(idx as u8));
                    }
                    if (up & bit) != 0 {
                        events.push(StreamDeckEvent::KeyUp(idx as u8));
                    }
                }
                self.prev_key_mask = pressed_mask;
            }
            StreamDeckInputReport::DialsPress { pressed_mask, .. } => {
                let changed = self.prev_dial_mask ^ pressed_mask;
                for idx in 0..self.model.dial_count() {
                    let bit = 1u8 << idx;
                    if (changed & bit) != 0 {
                        if (pressed_mask & bit) != 0 {
                            events.push(StreamDeckEvent::DialDown(idx as u8));
                        } else {
                            events.push(StreamDeckEvent::DialUp(idx as u8));
                        }
                    }
                }
                self.prev_dial_mask = pressed_mask;
            }
            StreamDeckInputReport::DialsRotate { deltas } => {
                for (idx, &delta) in deltas.iter().enumerate() {
                    if delta != 0 {
                        events.push(StreamDeckEvent::DialRotate {
                            dial: idx as u8,
                            delta,
                        });
                    }
                }
            }
            StreamDeckInputReport::TouchTap { x, y, long } => {
                events.push(StreamDeckEvent::TouchTap { x, y, long });
            }
            StreamDeckInputReport::TouchSwipe {
                start_x,
                start_y,
                end_x,
                end_y,
            } => {
                events.push(StreamDeckEvent::TouchSwipe {
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                });
            }
        }
        Ok(events)
    }
}

/// Builds the USB HID feature report to set backlight brightness (`0..=100` percent).
pub fn set_brightness(model: StreamDeckModel, percent: u8) -> Vec<u8> {
    let clamped = percent.min(100);
    if model.is_v1_hid() {
        let mut buf = vec![0u8; 17];
        buf[0] = 0x05;
        buf[1] = 0x55;
        buf[2] = 0xaa;
        buf[3] = 0xd1;
        buf[4] = 0x01;
        buf[5] = clamped;
        buf
    } else {
        let mut buf = vec![0u8; 32];
        buf[0] = 0x03;
        buf[1] = 0x08;
        buf[2] = clamped;
        buf
    }
}

/// Builds the USB HID feature report to reset the Stream Deck display to the logo screen.
pub fn reset_display(model: StreamDeckModel) -> Vec<u8> {
    if model.is_v1_hid() {
        let mut buf = vec![0u8; 17];
        buf[0] = 0x0b;
        buf[1] = 0x63;
        buf
    } else {
        let mut buf = vec![0u8; 32];
        buf[0] = 0x03;
        buf[1] = 0x02;
        buf
    }
}

/// Encodes a top-down RGB24 pixel buffer (`width * height * 3` bytes) into a
/// 24-bit uncompressed Windows BMP (bottom-up BGR with 4-byte row alignment).
pub fn encode_bmp_24(
    width: u16,
    height: u16,
    rgb24: &[u8],
) -> Result<Vec<u8>, StreamDeckError> {
    let w = usize::from(width);
    let h = usize::from(height);
    let expected = w * h * 3;
    if rgb24.len() != expected {
        return Err(StreamDeckError::InvalidRgbBufferLen {
            expected,
            actual: rgb24.len(),
        });
    }

    let row_bytes = w * 3;
    let stride = (row_bytes + 3) & !3;
    let pixel_data_size = stride * h;
    let file_size = 54 + pixel_data_size;

    let mut out = Vec::with_capacity(file_size);
    // BITMAPFILEHEADER (14 bytes)
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]); // reserved
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel offset

    // BITMAPINFOHEADER (40 bytes)
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&24u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(pixel_data_size as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes()); // ~72 DPI X
    out.extend_from_slice(&2835u32.to_le_bytes()); // ~72 DPI Y
    out.extend_from_slice(&0u32.to_le_bytes()); // colors used
    out.extend_from_slice(&0u32.to_le_bytes()); // important colors

    let pad = stride - row_bytes;
    for y in (0..h).rev() {
        let row_start = y * row_bytes;
        for x in 0..w {
            let px = row_start + x * 3;
            out.push(rgb24[px + 2]); // B
            out.push(rgb24[px + 1]); // G
            out.push(rgb24[px]); // R
        }
        for _ in 0..pad {
            out.push(0);
        }
    }
    Ok(out)
}

/// Chunks an already-encoded key icon (`BMP` for Original/Mini, `JPEG` for Mk2/XL/Plus)
/// into fixed-size USB HID output packets with Stream Deck header framing.
pub fn encode_key_image_packets(
    model: StreamDeckModel,
    key_index: u8,
    image_bytes: &[u8],
) -> Result<Vec<Vec<u8>>, StreamDeckError> {
    let key_count = model.key_count();
    if usize::from(key_index) >= key_count {
        return Err(StreamDeckError::InvalidKeyIndex {
            key_index,
            key_count,
        });
    }
    if image_bytes.is_empty() {
        return Err(StreamDeckError::EmptyImagePayload);
    }

    let packet_size = model.packet_size();
    let header_len = if model.is_v1_hid() { 16 } else { 8 };
    let max_chunk = packet_size - header_len;
    let total_chunks = image_bytes.len().div_ceil(max_chunk);
    if total_chunks > usize::from(u16::MAX) {
        return Err(StreamDeckError::PayloadTooLarge);
    }

    let mut packets = Vec::with_capacity(total_chunks);
    for (seq, chunk) in image_bytes.chunks(max_chunk).enumerate() {
        let is_last = u8::from(seq + 1 == total_chunks);
        let mut pkt = vec![0u8; packet_size];
        if model.is_v1_hid() {
            let page = (seq + 1) as u16;
            let [p_lo, p_hi] = page.to_le_bytes();
            pkt[0] = 0x02;
            pkt[1] = 0x01;
            pkt[2] = p_lo;
            pkt[3] = p_hi;
            pkt[4] = 0x00;
            pkt[5] = is_last;
            pkt[6] = key_index + 1;
        } else {
            let [len_lo, len_hi] = (chunk.len() as u16).to_le_bytes();
            let [seq_lo, seq_hi] = (seq as u16).to_le_bytes();
            pkt[0] = 0x02;
            pkt[1] = 0x07;
            pkt[2] = key_index;
            pkt[3] = is_last;
            pkt[4] = len_lo;
            pkt[5] = len_hi;
            pkt[6] = seq_lo;
            pkt[7] = seq_hi;
        }
        pkt[header_len..header_len + chunk.len()].copy_from_slice(chunk);
        packets.push(pkt);
    }
    Ok(packets)
}

/// Frames a raw RGB24 buffer as a 24-bit BMP icon sized for `model` and chunks it
/// into USB HID output packets.
pub fn encode_key_rgb_bmp_packets(
    model: StreamDeckModel,
    key_index: u8,
    rgb24: &[u8],
) -> Result<Vec<Vec<u8>>, StreamDeckError> {
    let (w, h) = model.icon_dimensions();
    let bmp = encode_bmp_24(w, h, rgb24)?;
    encode_key_image_packets(model, key_index, &bmp)
}
