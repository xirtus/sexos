// SPDX-License-Identifier: GPL-3.0-only
//! UVC Payload Stream Reassembler.
//!
//! Reassembles incoming USB bulk/isoc packets into complete video frames.
//! Strictly bounds-checked, enforces a hard quota on maximum frame size,
//! handles UVC 1.1/1.5 payload headers (FID toggle, EOF, PTS, SCR, ERR drop).

use alloc::vec::Vec;

pub const HEADER_INFO_FID: u8 = 0x01; // Frame ID
pub const HEADER_INFO_EOF: u8 = 0x02; // End of Frame
pub const HEADER_INFO_PTS: u8 = 0x04; // Presentation Time Stamp
pub const HEADER_INFO_SCR: u8 = 0x08; // Source Clock Reference
pub const HEADER_INFO_RES: u8 = 0x10; // Reserved
pub const HEADER_INFO_STI: u8 = 0x20; // Still Image
pub const HEADER_INFO_ERR: u8 = 0x40; // Error bit
pub const HEADER_INFO_EOH: u8 = 0x80; // End of Header

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    PacketTooShort,
    HeaderLengthInvalid,
    FrameTooLarge,
    CorruptFrame,
    OutOfMemory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvcPayloadHeader {
    pub length: u8,
    pub info: u8,
    pub fid: bool,
    pub eof: bool,
    pub pts: Option<u32>,
    pub scr_time: Option<u32>,
    pub scr_sof: Option<u16>,
    pub still_image: bool,
    pub error: bool,
}

impl UvcPayloadHeader {
    pub fn parse(packet: &[u8]) -> Result<Self, StreamError> {
        if packet.len() < 2 {
            return Err(StreamError::PacketTooShort);
        }
        let length = packet[0];
        if (length as usize) < 2 || (length as usize) > packet.len() {
            return Err(StreamError::HeaderLengthInvalid);
        }
        let info = packet[1];
        let fid = (info & HEADER_INFO_FID) != 0;
        let eof = (info & HEADER_INFO_EOF) != 0;
        let has_pts = (info & HEADER_INFO_PTS) != 0;
        let has_scr = (info & HEADER_INFO_SCR) != 0;
        let still_image = (info & HEADER_INFO_STI) != 0;
        let error = (info & HEADER_INFO_ERR) != 0;

        let mut offset = 2;
        let pts = if has_pts {
            if offset + 4 > length as usize {
                return Err(StreamError::HeaderLengthInvalid);
            }
            let val = u32::from_le_bytes([
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            ]);
            offset += 4;
            Some(val)
        } else {
            None
        };

        let (scr_time, scr_sof) = if has_scr {
            if offset + 6 > length as usize {
                return Err(StreamError::HeaderLengthInvalid);
            }
            let time = u32::from_le_bytes([
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            ]);
            let sof = u16::from_le_bytes([packet[offset + 4], packet[offset + 5]]);
            (Some(time), Some(sof))
        } else {
            (None, None)
        };

        Ok(Self {
            length,
            info,
            fid,
            eof,
            pts,
            scr_time,
            scr_sof,
            still_image,
            error,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteFrame {
    pub data: Vec<u8>,
    pub pts: Option<u32>,
    pub fid: bool,
    pub still_image: bool,
}

pub struct UvcStreamAssembler {
    max_frame_bytes: usize,
    accumulating: Vec<u8>,
    current_fid: Option<bool>,
    current_pts: Option<u32>,
    current_still: bool,
    corrupted_current: bool,

    // Statistics
    pub frames_received: u64,
    pub frames_corrupted: u64,
    pub packets_received: u64,
    pub bytes_received: u64,
}

impl UvcStreamAssembler {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            max_frame_bytes,
            accumulating: Vec::new(),
            current_fid: None,
            current_pts: None,
            current_still: false,
            corrupted_current: false,
            frames_received: 0,
            frames_corrupted: 0,
            packets_received: 0,
            bytes_received: 0,
        }
    }

    pub fn reset(&mut self) {
        self.accumulating.clear();
        self.current_fid = None;
        self.current_pts = None;
        self.current_still = false;
        self.corrupted_current = false;
    }

    /// Feeds one raw UVC packet (starting with the UVC payload header).
    /// Returns `Ok(Some(CompleteFrame))` when a frame is completely reassembled.
    pub fn push_packet(&mut self, packet: &[u8]) -> Result<Option<CompleteFrame>, StreamError> {
        self.packets_received = self.packets_received.wrapping_add(1);
        self.bytes_received = self.bytes_received.wrapping_add(packet.len() as u64);

        let header = UvcPayloadHeader::parse(packet)?;
        let payload = &packet[header.length as usize..];

        // If the hardware marked this packet as having an error, drop the current frame.
        if header.error {
            self.corrupted_current = true;
            self.frames_corrupted = self.frames_corrupted.wrapping_add(1);
            self.accumulating.clear();
            self.current_fid = None;
            return Err(StreamError::CorruptFrame);
        }

        let mut completed_frame = None;

        let new_frame = match self.current_fid {
            None => true,
            Some(cur) => cur != header.fid,
        };

        if new_frame {
            if !self.accumulating.is_empty() {
                if !self.corrupted_current {
                    self.frames_received = self.frames_received.wrapping_add(1);
                    completed_frame = Some(CompleteFrame {
                        data: core::mem::take(&mut self.accumulating),
                        pts: self.current_pts,
                        fid: self.current_fid.unwrap_or(header.fid),
                        still_image: self.current_still,
                    });
                } else {
                    self.accumulating.clear();
                }
            }
            self.corrupted_current = false;
            self.current_fid = Some(header.fid);
        }
        if header.pts.is_some() {
            self.current_pts = header.pts;
        }
        self.current_still = header.still_image;

        if !payload.is_empty() {
            let new_len = self.accumulating.len().checked_add(payload.len()).ok_or(StreamError::FrameTooLarge)?;
            if new_len > self.max_frame_bytes {
                self.corrupted_current = true;
                self.frames_corrupted = self.frames_corrupted.wrapping_add(1);
                self.accumulating.clear();
                return Err(StreamError::FrameTooLarge);
            }
            if self.accumulating.try_reserve(payload.len()).is_err() {
                self.corrupted_current = true;
                self.frames_corrupted = self.frames_corrupted.wrapping_add(1);
                self.accumulating.clear();
                return Err(StreamError::OutOfMemory);
            }
            self.accumulating.extend_from_slice(payload);
        }

        if header.eof {
            if !self.corrupted_current && !self.accumulating.is_empty() {
                self.frames_received = self.frames_received.wrapping_add(1);
                let frame = CompleteFrame {
                    data: core::mem::take(&mut self.accumulating),
                    pts: self.current_pts,
                    fid: header.fid,
                    still_image: self.current_still,
                };
                self.current_fid = None;
                self.corrupted_current = false;
                return Ok(Some(frame));
            } else {
                self.accumulating.clear();
                self.current_fid = None;
                self.corrupted_current = false;
            }
        }

        Ok(completed_frame)
    }
}
