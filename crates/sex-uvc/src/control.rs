// SPDX-License-Identifier: GPL-3.0-only
//! UVC Probe / Commit Negotiation and Control Protocol.
//!
//! Handles 26-byte (UVC 1.0) & 34-byte (UVC 1.1/1.5) VideoProbeCommitControl,
//! FPS <-> 100ns interval conversions, and USB control setup packet formatting.

use alloc::vec::Vec;

pub const SET_CUR: u8 = 0x01;
pub const GET_CUR: u8 = 0x81;
pub const GET_MIN: u8 = 0x82;
pub const GET_MAX: u8 = 0x83;
pub const GET_RES: u8 = 0x84;
pub const GET_LEN: u8 = 0x85;
pub const GET_INFO: u8 = 0x86;
pub const GET_DEF: u8 = 0x87;

pub const VS_PROBE_CONTROL: u16 = 0x0100;
pub const VS_COMMIT_CONTROL: u16 = 0x0200;
pub const VS_STILL_PROBE_CONTROL: u16 = 0x0300;
pub const VS_STILL_COMMIT_CONTROL: u16 = 0x0400;

pub const BM_REQ_SET_CLASS_INTERFACE: u8 = 0x21;
pub const BM_REQ_GET_CLASS_INTERFACE: u8 = 0xA1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlError {
    BufferTooShort,
    InvalidInterval,
    InvalidPayloadSize,
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoProbeCommitControl {
    pub bm_hint: u16,
    pub b_format_index: u8,
    pub b_frame_index: u8,
    pub dw_frame_interval: u32, // 100ns units
    pub w_key_frame_rate: u16,
    pub w_p_frame_rate: u16,
    pub w_comp_quality: u16,
    pub w_comp_window_size: u16,
    pub w_delay: u16,
    pub dw_max_video_frame_size: u32,
    pub dw_max_payload_transfer_size: u32,
    // UVC 1.1 / 1.5 fields
    pub dw_clock_frequency: Option<u32>,
    pub bm_framing_info: Option<u8>,
    pub b_prefered_version: Option<u8>,
    pub b_min_version: Option<u8>,
    pub b_max_version: Option<u8>,
}

impl Default for VideoProbeCommitControl {
    fn default() -> Self {
        Self {
            bm_hint: 1, // hint frame interval
            b_format_index: 1,
            b_frame_index: 1,
            dw_frame_interval: 333_333, // 30 fps
            w_key_frame_rate: 0,
            w_p_frame_rate: 0,
            w_comp_quality: 0,
            w_comp_window_size: 0,
            w_delay: 0,
            dw_max_video_frame_size: 614_400, // 640x480 YUY2
            dw_max_payload_transfer_size: 1024,
            dw_clock_frequency: None,
            bm_framing_info: None,
            b_prefered_version: None,
            b_min_version: None,
            b_max_version: None,
        }
    }
}

impl VideoProbeCommitControl {
    pub fn fps_to_interval_100ns(fps: u32) -> u32 {
        if fps == 0 {
            333_333
        } else {
            10_000_000 / fps
        }
    }

    pub fn interval_100ns_to_fps(interval: u32) -> u32 {
        if interval == 0 {
            0
        } else {
            10_000_000 / interval
        }
    }

    pub fn fps(&self) -> u32 {
        Self::interval_100ns_to_fps(self.dw_frame_interval)
    }

    pub fn set_fps(&mut self, fps: u32) {
        self.dw_frame_interval = Self::fps_to_interval_100ns(fps);
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ControlError> {
        if bytes.len() < 26 {
            return Err(ControlError::BufferTooShort);
        }

        let bm_hint = u16::from_le_bytes([bytes[0], bytes[1]]);
        let b_format_index = bytes[2];
        let b_frame_index = bytes[3];
        let dw_frame_interval = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let w_key_frame_rate = u16::from_le_bytes([bytes[8], bytes[9]]);
        let w_p_frame_rate = u16::from_le_bytes([bytes[10], bytes[11]]);
        let w_comp_quality = u16::from_le_bytes([bytes[12], bytes[13]]);
        let w_comp_window_size = u16::from_le_bytes([bytes[14], bytes[15]]);
        let w_delay = u16::from_le_bytes([bytes[16], bytes[17]]);
        let dw_max_video_frame_size = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let dw_max_payload_transfer_size = u32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);

        let (dw_clock_frequency, bm_framing_info, b_prefered_version, b_min_version, b_max_version) =
            if bytes.len() >= 34 {
                (
                    Some(u32::from_le_bytes([bytes[26], bytes[27], bytes[28], bytes[29]])),
                    Some(bytes[30]),
                    Some(bytes[31]),
                    Some(bytes[32]),
                    Some(bytes[33]),
                )
            } else {
                (None, None, None, None, None)
            };

        Ok(Self {
            bm_hint,
            b_format_index,
            b_frame_index,
            dw_frame_interval,
            w_key_frame_rate,
            w_p_frame_rate,
            w_comp_quality,
            w_comp_window_size,
            w_delay,
            dw_max_video_frame_size,
            dw_max_payload_transfer_size,
            dw_clock_frequency,
            bm_framing_info,
            b_prefered_version,
            b_min_version,
            b_max_version,
        })
    }

    pub fn encode(&self, is_uvc_11: bool) -> Vec<u8> {
        let size = if is_uvc_11 { 34 } else { 26 };
        let mut out = alloc::vec![0u8; size];
        let _ = self.encode_to(&mut out);
        out
    }

    pub fn encode_to(&self, out: &mut [u8]) -> Result<usize, ControlError> {
        if out.len() < 26 {
            return Err(ControlError::BufferTooShort);
        }

        out[0..2].copy_from_slice(&self.bm_hint.to_le_bytes());
        out[2] = self.b_format_index;
        out[3] = self.b_frame_index;
        out[4..8].copy_from_slice(&self.dw_frame_interval.to_le_bytes());
        out[8..10].copy_from_slice(&self.w_key_frame_rate.to_le_bytes());
        out[10..12].copy_from_slice(&self.w_p_frame_rate.to_le_bytes());
        out[12..14].copy_from_slice(&self.w_comp_quality.to_le_bytes());
        out[14..16].copy_from_slice(&self.w_comp_window_size.to_le_bytes());
        out[16..18].copy_from_slice(&self.w_delay.to_le_bytes());
        out[18..22].copy_from_slice(&self.dw_max_video_frame_size.to_le_bytes());
        out[22..26].copy_from_slice(&self.dw_max_payload_transfer_size.to_le_bytes());

        if out.len() >= 34 && self.dw_clock_frequency.is_some() {
            out[26..30].copy_from_slice(&self.dw_clock_frequency.unwrap_or(0).to_le_bytes());
            out[30] = self.bm_framing_info.unwrap_or(0);
            out[31] = self.b_prefered_version.unwrap_or(0);
            out[32] = self.b_min_version.unwrap_or(0);
            out[33] = self.b_max_version.unwrap_or(0);
            Ok(34)
        } else {
            Ok(26)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbSetupPacket {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

impl UsbSetupPacket {
    pub fn encode(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[0] = self.bm_request_type;
        out[1] = self.b_request;
        out[2..4].copy_from_slice(&self.w_value.to_le_bytes());
        out[4..6].copy_from_slice(&self.w_index.to_le_bytes());
        out[6..8].copy_from_slice(&self.w_length.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ControlError> {
        if bytes.len() < 8 {
            return Err(ControlError::BufferTooShort);
        }
        Ok(Self {
            bm_request_type: bytes[0],
            b_request: bytes[1],
            w_value: u16::from_le_bytes([bytes[2], bytes[3]]),
            w_index: u16::from_le_bytes([bytes[4], bytes[5]]),
            w_length: u16::from_le_bytes([bytes[6], bytes[7]]),
        })
    }

    pub fn probe_get_cur(interface_num: u8, len: u16) -> Self {
        Self {
            bm_request_type: BM_REQ_GET_CLASS_INTERFACE,
            b_request: GET_CUR,
            w_value: VS_PROBE_CONTROL,
            w_index: interface_num as u16,
            w_length: len,
        }
    }

    pub fn probe_set_cur(interface_num: u8, len: u16) -> Self {
        Self {
            bm_request_type: BM_REQ_SET_CLASS_INTERFACE,
            b_request: SET_CUR,
            w_value: VS_PROBE_CONTROL,
            w_index: interface_num as u16,
            w_length: len,
        }
    }

    pub fn commit_get_cur(interface_num: u8, len: u16) -> Self {
        Self {
            bm_request_type: BM_REQ_GET_CLASS_INTERFACE,
            b_request: GET_CUR,
            w_value: VS_COMMIT_CONTROL,
            w_index: interface_num as u16,
            w_length: len,
        }
    }

    pub fn commit_set_cur(interface_num: u8, len: u16) -> Self {
        Self {
            bm_request_type: BM_REQ_SET_CLASS_INTERFACE,
            b_request: SET_CUR,
            w_value: VS_COMMIT_CONTROL,
            w_index: interface_num as u16,
            w_length: len,
        }
    }
}
