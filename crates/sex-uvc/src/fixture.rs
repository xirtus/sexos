// SPDX-License-Identifier: GPL-3.0-only
//! UVC Fixture Wire Protocol and Synthetic Test Device.
//!
//! Provides a framed packet protocol for network and loopback camera streaming,
//! plus a deterministic SyntheticUvcDevice emitting valid UVC descriptors and
//! multi-packet YUY2 / MJPEG video frames.

use crate::control::VideoProbeCommitControl;
use crate::descriptors::*;
use crate::stream::{
    HEADER_INFO_EOF, HEADER_INFO_FID, HEADER_INFO_PTS,
};
use alloc::vec;
use alloc::vec::Vec;

pub const WIRE_MAGIC: [u8; 4] = *b"SUVC";

pub const MSG_DESCRIPTORS: u8 = 1;
pub const MSG_PROBE_REQ: u8 = 2;
pub const MSG_PROBE_RESP: u8 = 3;
pub const MSG_COMMIT_REQ: u8 = 4;
pub const MSG_COMMIT_ACK: u8 = 5;
pub const MSG_START_STREAM: u8 = 6;
pub const MSG_STOP_STREAM: u8 = 7;
pub const MSG_UVC_PACKET: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    InvalidMagic,
    UnknownMsgType(u8),
    PayloadLengthExceeded,
    BufferTooShort,
    ControlError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireMessage {
    Descriptors(Vec<u8>),
    ProbeReq(VideoProbeCommitControl),
    ProbeResp(VideoProbeCommitControl),
    CommitReq(VideoProbeCommitControl),
    CommitAck(u8),
    StartStream,
    StopStream,
    UvcPacket(Vec<u8>),
}

impl WireMessage {
    pub fn encode(&self) -> Vec<u8> {
        let (msg_type, payload) = match self {
            WireMessage::Descriptors(data) => (MSG_DESCRIPTORS, data.clone()),
            WireMessage::ProbeReq(ctrl) => (MSG_PROBE_REQ, ctrl.encode(false)),
            WireMessage::ProbeResp(ctrl) => (MSG_PROBE_RESP, ctrl.encode(false)),
            WireMessage::CommitReq(ctrl) => (MSG_COMMIT_REQ, ctrl.encode(false)),
            WireMessage::CommitAck(status) => (MSG_COMMIT_ACK, vec![*status]),
            WireMessage::StartStream => (MSG_START_STREAM, Vec::new()),
            WireMessage::StopStream => (MSG_STOP_STREAM, Vec::new()),
            WireMessage::UvcPacket(pkt) => (MSG_UVC_PACKET, pkt.clone()),
        };

        let len = payload.len() as u16;
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&WIRE_MAGIC);
        out.push(msg_type);
        out.push(0); // reserved
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    pub fn decode_one(buf: &[u8]) -> Result<Option<(WireMessage, usize)>, WireError> {
        if buf.len() < 8 {
            return Ok(None);
        }
        if &buf[0..4] != &WIRE_MAGIC {
            return Err(WireError::InvalidMagic);
        }

        let msg_type = buf[4];
        let payload_len = u16::from_be_bytes([buf[6], buf[7]]) as usize;
        let total_len = 8 + payload_len;

        if buf.len() < total_len {
            return Ok(None);
        }

        let payload = &buf[8..total_len];
        let msg = match msg_type {
            MSG_DESCRIPTORS => WireMessage::Descriptors(payload.to_vec()),
            MSG_PROBE_REQ => {
                let ctrl = VideoProbeCommitControl::decode(payload).map_err(|_| WireError::ControlError)?;
                WireMessage::ProbeReq(ctrl)
            }
            MSG_PROBE_RESP => {
                let ctrl = VideoProbeCommitControl::decode(payload).map_err(|_| WireError::ControlError)?;
                WireMessage::ProbeResp(ctrl)
            }
            MSG_COMMIT_REQ => {
                let ctrl = VideoProbeCommitControl::decode(payload).map_err(|_| WireError::ControlError)?;
                WireMessage::CommitReq(ctrl)
            }
            MSG_COMMIT_ACK => {
                let status = payload.first().copied().unwrap_or(0);
                WireMessage::CommitAck(status)
            }
            MSG_START_STREAM => WireMessage::StartStream,
            MSG_STOP_STREAM => WireMessage::StopStream,
            MSG_UVC_PACKET => WireMessage::UvcPacket(payload.to_vec()),
            other => return Err(WireError::UnknownMsgType(other)),
        };

        Ok(Some((msg, total_len)))
    }
}

/// A deterministic synthetic UVC device generating valid descriptors and frames.
pub struct SyntheticUvcDevice {
    pub width: u16,
    pub height: u16,
    pub fps: u32,
    pub packet_size: usize,
    pub frame_counter: u64,
    pub committed: Option<VideoProbeCommitControl>,
    pub streaming: bool,
}

impl SyntheticUvcDevice {
    pub fn new(width: u16, height: u16, fps: u32) -> Self {
        Self {
            width,
            height,
            fps,
            packet_size: 1024,
            frame_counter: 0,
            committed: None,
            streaming: false,
        }
    }

    /// Emits complete standard USB/UVC binary descriptors for this device.
    pub fn binary_descriptors(&self) -> Vec<u8> {
        let mut d = Vec::new();

        // 1. Interface Association Descriptor (IAD)
        d.extend_from_slice(&[
            0x08,                      // bLength
            DESC_INTERFACE_ASSOCIATION, // 0x0B
            0x00,                      // bFirstInterface (VideoControl)
            0x02,                      // bInterfaceCount (VC + VS)
            CC_VIDEO,                  // 0x0E
            SC_VIDEO_INTERFACE_COLLECTION, // 0x03
            PC_PROTOCOL_UNDEFINED,     // 0x00
            0x00,                      // iFunction
        ]);

        // 2. VideoControl Standard Interface Descriptor
        d.extend_from_slice(&[
            0x09,           // bLength
            DESC_INTERFACE, // 0x04
            0x00,           // bInterfaceNumber
            0x00,           // bAlternateSetting
            0x00,           // bNumEndpoints
            CC_VIDEO,       // 0x0E
            SC_VIDEOCONTROL,// 0x01
            0x00,           // bInterfaceProtocol
            0x00,           // iInterface
        ]);

        // 3. Class-Specific VC Header Descriptor
        // Header length: 12 + 1 = 13 bytes
        let vc_total = 13 + 17 + 11 + 9; // VC Header + Camera IT + PU + OT
        d.extend_from_slice(&[
            0x0D,               // bLength (13)
            DESC_CS_INTERFACE,  // 0x24
            VC_HEADER,          // 0x01
            0x10, 0x01,         // bcdUVC = 1.10
        ]);
        d.extend_from_slice(&(vc_total as u16).to_le_bytes()); // wTotalLength
        d.extend_from_slice(&48_000_000u32.to_le_bytes());     // dwClockFrequency (48 MHz)
        d.push(0x01);                                         // bInCollection (1 VS interface)
        d.push(0x01);                                         // baInterfaceNr[0] = 1

        // 4. Input Terminal Descriptor (Camera IT)
        d.extend_from_slice(&[
            0x11,              // bLength (17)
            DESC_CS_INTERFACE, // 0x24
            VC_INPUT_TERMINAL, // 0x02
            0x01,              // bTerminalID = 1
            0x01, 0x02,        // wTerminalType = ITT_CAMERA (0x0201)
            0x00,              // bAssocTerminal
            0x00,              // iTerminal
            0x00, 0x00,        // wObjectiveFocalLengthMin
            0x00, 0x00,        // wObjectiveFocalLengthMax
            0x00, 0x00,        // wOcularFocalLength
            0x02,              // bControlSize = 2
            0x00, 0x00,        // bmControls
        ]);

        // 5. Processing Unit Descriptor
        d.extend_from_slice(&[
            0x0B,              // bLength (11)
            DESC_CS_INTERFACE, // 0x24
            VC_PROCESSING_UNIT,// 0x05
            0x02,              // bUnitID = 2
            0x01,              // bSourceID = 1 (Camera IT)
            0x00, 0x00,        // wMaxMultiplier
            0x02,              // bControlSize = 2
            0x01, 0x00,        // bmControls (Brightness)
            0x00,              // iProcessing
        ]);

        // 6. Output Terminal Descriptor
        d.extend_from_slice(&[
            0x09,              // bLength (9)
            DESC_CS_INTERFACE, // 0x24
            VC_OUTPUT_TERMINAL,// 0x03
            0x03,              // bTerminalID = 3
            0x01, 0x01,        // wTerminalType = TT_STREAMING (0x0101)
            0x00,              // bAssocTerminal
            0x02,              // bSourceID = 2 (Processing Unit)
            0x00,              // iTerminal
        ]);

        // 7. VideoStreaming Standard Interface Descriptor
        d.extend_from_slice(&[
            0x09,              // bLength
            DESC_INTERFACE,    // 0x04
            0x01,              // bInterfaceNumber = 1
            0x00,              // bAlternateSetting = 0
            0x01,              // bNumEndpoints = 1
            CC_VIDEO,          // 0x0E
            SC_VIDEOSTREAMING, // 0x02
            0x00,              // bInterfaceProtocol
            0x00,              // iInterface
        ]);

        // 8. Class-Specific VS Input Header Descriptor
        // Header length: 13 + 1 * 1 = 14 bytes
        let vs_total = 14 + 27 + 30; // Input Header + Format Uncompressed + Frame Uncompressed
        d.extend_from_slice(&[
            0x0E,              // bLength (14)
            DESC_CS_INTERFACE, // 0x24
            VS_INPUT_HEADER,   // 0x01
            0x01,              // bNumFormats = 1
        ]);
        d.extend_from_slice(&(vs_total as u16).to_le_bytes()); // wTotalLength
        d.extend_from_slice(&[
            0x81,              // bEndpointAddress (EP 1 IN)
            0x00,              // bmInfo
            0x03,              // bTerminalLink (Output Terminal ID 3)
            0x00,              // bStillCaptureMethod
            0x00,              // bTriggerSupport
            0x00,              // bTriggerUsage
            0x01,              // bControlSize = 1
            0x00,              // bmaControls[0]
        ]);

        // 9. VS Format Uncompressed Descriptor (YUY2)
        d.extend_from_slice(&[
            0x1B,                  // bLength (27)
            DESC_CS_INTERFACE,     // 0x24
            VS_FORMAT_UNCOMPRESSED,// 0x04
            0x01,                  // bFormatIndex = 1
            0x01,                  // bNumFrameDescriptors = 1
        ]);
        d.extend_from_slice(&GUID_YUY2); // 16 bytes
        d.extend_from_slice(&[
            0x10,              // bBitsPerPixel = 16
            0x01,              // bDefaultFrameIndex = 1
            0x04,              // bAspectRatioX = 4
            0x03,              // bAspectRatioY = 3
            0x00,              // bmInterlaceFlags
            0x00,              // bCopyProtect
        ]);

        // 10. VS Frame Uncompressed Descriptor
        // 26 + 1 interval (4 bytes) = 30 bytes
        let interval_100ns = VideoProbeCommitControl::fps_to_interval_100ns(self.fps);
        let frame_size = (self.width as u32) * (self.height as u32) * 2;
        d.extend_from_slice(&[
            0x1E,                 // bLength (30)
            DESC_CS_INTERFACE,    // 0x24
            VS_FRAME_UNCOMPRESSED,// 0x05
            0x01,                 // bFrameIndex = 1
            0x00,                 // bmCapabilities
        ]);
        d.extend_from_slice(&self.width.to_le_bytes());
        d.extend_from_slice(&self.height.to_le_bytes());
        d.extend_from_slice(&(frame_size * self.fps * 8).to_le_bytes()); // dwMinBitRate
        d.extend_from_slice(&(frame_size * self.fps * 8).to_le_bytes()); // dwMaxBitRate
        d.extend_from_slice(&frame_size.to_le_bytes());                  // dwMaxVideoFrameBufferSize
        d.extend_from_slice(&interval_100ns.to_le_bytes());             // dwDefaultFrameInterval
        d.push(0x01);                                                   // bFrameIntervalType = 1 discrete
        d.extend_from_slice(&interval_100ns.to_le_bytes());             // interval 0

        // 11. Standard Endpoint Descriptor
        d.extend_from_slice(&[
            0x07,          // bLength
            DESC_ENDPOINT, // 0x05
            0x81,          // bEndpointAddress (EP 1 IN)
            0x02,          // bmAttributes (Bulk)
            0x00, 0x04,    // wMaxPacketSize (1024)
            0x00,          // bInterval
        ]);

        d
    }

    /// Handles a probe request and returns a negotiated probe response.
    pub fn handle_probe(&mut self, req: &VideoProbeCommitControl) -> VideoProbeCommitControl {
        let mut resp = *req;
        resp.b_format_index = 1;
        resp.b_frame_index = 1;
        resp.dw_frame_interval = VideoProbeCommitControl::fps_to_interval_100ns(self.fps);
        resp.dw_max_video_frame_size = (self.width as u32) * (self.height as u32) * 2;
        resp.dw_max_payload_transfer_size = self.packet_size as u32;
        resp
    }

    /// Handles a commit request.
    pub fn handle_commit(&mut self, req: &VideoProbeCommitControl) {
        self.committed = Some(*req);
    }

    /// Generates a synthetic raw YUY2 frame buffer for the given frame index `n`.
    pub fn generate_raw_yuy2_frame(&self, frame_num: u64) -> Vec<u8> {
        let total_pixels = (self.width as usize) * (self.height as usize);
        let mut out = vec![0u8; total_pixels * 2];

        // Animate color bars / gradients based on frame_num
        let shift = (frame_num * 8) as u8;
        for y in 0..self.height as usize {
            for x in 0..self.width as usize / 2 {
                let idx = (y * (self.width as usize) + x * 2) * 2;
                let color_idx = ((x * 8) / (self.width as usize / 2).max(1)) as u8;

                // Alternate colors: Red, Green, Blue, White, etc.
                let (y_val, u_val, v_val): (u8, u8, u8) = match (color_idx + (shift / 32)) % 4 {
                    0 => (128, 90, 240),  // Reddish
                    1 => (180, 54, 34),   // Greenish
                    2 => (80, 240, 110),  // Blueish
                    _ => (220, 128, 128), // Bright White/Grey
                };

                out[idx] = y_val.wrapping_add((y % 16) as u8);
                out[idx + 1] = u_val;
                out[idx + 2] = y_val.wrapping_add((y % 16) as u8);
                out[idx + 3] = v_val;
            }
        }

        out
    }

    /// Generates packetized UVC packets for the next frame.
    pub fn next_frame_packets(&mut self) -> Vec<Vec<u8>> {
        let frame_num = self.frame_counter;
        self.frame_counter = self.frame_counter.wrapping_add(1);

        let raw = self.generate_raw_yuy2_frame(frame_num);
        let fid = (frame_num % 2) != 0;
        let pts = (frame_num as u32).wrapping_mul(VideoProbeCommitControl::fps_to_interval_100ns(self.fps));

        let payload_per_packet = self.packet_size.saturating_sub(12).max(64);
        let mut packets = Vec::new();

        let mut offset = 0;
        while offset < raw.len() {
            let chunk_len = (raw.len() - offset).min(payload_per_packet);
            let is_eof = offset + chunk_len >= raw.len();

            let mut info = 0u8;
            if fid {
                info |= HEADER_INFO_FID;
            }
            if is_eof {
                info |= HEADER_INFO_EOF;
            }
            info |= HEADER_INFO_PTS;

            let header_len = 6u8; // 2 byte base + 4 byte PTS
            let mut pkt = Vec::with_capacity(header_len as usize + chunk_len);
            pkt.push(header_len);
            pkt.push(info);
            pkt.extend_from_slice(&pts.to_le_bytes());
            pkt.extend_from_slice(&raw[offset..offset + chunk_len]);

            packets.push(pkt);
            offset += chunk_len;
        }

        packets
    }
}
