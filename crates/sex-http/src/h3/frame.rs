// SPDX-License-Identifier: GPL-3.0-only
//! RFC 9114 HTTP/3 frame and unidirectional stream type codec.

use super::varint::{self, VarintError};
use alloc::vec::Vec;

/// RFC 9114 Section 7.2 HTTP/3 frame types.
pub const FRAME_DATA: u64 = 0x00;
pub const FRAME_HEADERS: u64 = 0x01;
pub const FRAME_CANCEL_PUSH: u64 = 0x03;
pub const FRAME_SETTINGS: u64 = 0x04;
pub const FRAME_PUSH_PROMISE: u64 = 0x05;
pub const FRAME_GOAWAY: u64 = 0x07;
pub const FRAME_MAX_PUSH_ID: u64 = 0x0D;

/// RFC 9114 Section 6.2 unidirectional stream types.
pub const STREAM_CONTROL: u64 = 0x00;
pub const STREAM_PUSH: u64 = 0x01;
pub const STREAM_QPACK_ENCODER: u64 = 0x02;
pub const STREAM_QPACK_DECODER: u64 = 0x03;

/// RFC 9114 / RFC 9204 SETTINGS identifiers.
pub const SETTINGS_QPACK_MAX_TABLE_CAPACITY: u64 = 0x01;
pub const SETTINGS_MAX_FIELD_SECTION_SIZE: u64 = 0x06;
pub const SETTINGS_QPACK_BLOCKED_STREAMS: u64 = 0x07;

/// RFC 9114 Section 8.1 HTTP/3 error codes.
pub const H3_NO_ERROR: u64 = 0x0100;
pub const H3_GENERAL_PROTOCOL_ERROR: u64 = 0x0101;
pub const H3_STREAM_CREATION_ERROR: u64 = 0x0103;
pub const H3_FRAME_UNEXPECTED: u64 = 0x0105;
pub const H3_FRAME_ERROR: u64 = 0x0106;
pub const H3_SETTINGS_ERROR: u64 = 0x0109;
pub const H3_MISSING_SETTINGS: u64 = 0x010A;

/// Maximum single HTTP/3 frame payload accepted by our decoder (4 MiB).
pub const MAX_H3_FRAME_PAYLOAD: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    Incomplete,
    Varint(VarintError),
    TooLarge,
    Malformed,
    ReservedType(u64),
}

impl From<VarintError> for FrameError {
    fn from(e: VarintError) -> Self {
        match e {
            VarintError::Incomplete => FrameError::Incomplete,
            other => FrameError::Varint(other),
        }
    }
}

/// HTTP/3 frame (RFC 9114 Section 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum H3Frame {
    Data(Vec<u8>),
    Headers(Vec<u8>),
    CancelPush(u64),
    Settings(Vec<(u64, u64)>),
    PushPromise { push_id: u64, header_block: Vec<u8> },
    GoAway(u64),
    MaxPushId(u64),
    /// Unknown/GREASE extension frames are ignored per RFC 9114 Section 7.2.8.
    Unknown { frame_type: u64, payload: Vec<u8> },
}

impl H3Frame {
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), FrameError> {
        let mut payload = Vec::new();
        let kind = match self {
            H3Frame::Data(b) => {
                payload.extend_from_slice(b);
                FRAME_DATA
            }
            H3Frame::Headers(b) => {
                payload.extend_from_slice(b);
                FRAME_HEADERS
            }
            H3Frame::CancelPush(id) => {
                varint::encode(*id, &mut payload)?;
                FRAME_CANCEL_PUSH
            }
            H3Frame::Settings(pairs) => {
                for &(k, v) in pairs {
                    varint::encode(k, &mut payload)?;
                    varint::encode(v, &mut payload)?;
                }
                FRAME_SETTINGS
            }
            H3Frame::PushPromise { push_id, header_block } => {
                varint::encode(*push_id, &mut payload)?;
                payload.extend_from_slice(header_block);
                FRAME_PUSH_PROMISE
            }
            H3Frame::GoAway(id) => {
                varint::encode(*id, &mut payload)?;
                FRAME_GOAWAY
            }
            H3Frame::MaxPushId(id) => {
                varint::encode(*id, &mut payload)?;
                FRAME_MAX_PUSH_ID
            }
            H3Frame::Unknown { frame_type, payload: p } => {
                payload.extend_from_slice(p);
                *frame_type
            }
        };
        varint::encode(kind, out)?;
        varint::encode(payload.len() as u64, out)?;
        out.extend_from_slice(&payload);
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<(Self, usize), FrameError> {
        let (kind, mut at) = varint::decode(input)?;
        // RFC 9114 Section 7.2.8: HTTP/2 reserved frame types (0x02, 0x06, 0x08, 0x09) MUST fail.
        if matches!(kind, 0x02 | 0x06 | 0x08 | 0x09) {
            return Err(FrameError::ReservedType(kind));
        }
        let (len, used) = varint::decode(&input[at..])?;
        at += used;
        let len = usize::try_from(len).map_err(|_| FrameError::TooLarge)?;
        if len > MAX_H3_FRAME_PAYLOAD {
            return Err(FrameError::TooLarge);
        }
        let body = input.get(at..at + len).ok_or(FrameError::Incomplete)?;
        let total = at + len;
        let frame = match kind {
            FRAME_DATA => H3Frame::Data(body.to_vec()),
            FRAME_HEADERS => H3Frame::Headers(body.to_vec()),
            FRAME_CANCEL_PUSH => {
                let (id, u) = varint::decode(body).map_err(|_| FrameError::Malformed)?;
                if u != body.len() { return Err(FrameError::Malformed); }
                H3Frame::CancelPush(id)
            }
            FRAME_SETTINGS => {
                let mut pairs = Vec::new();
                let mut pos = 0;
                while pos < body.len() {
                    let (id, u1) = varint::decode(&body[pos..]).map_err(|_| FrameError::Malformed)?;
                    pos += u1;
                    // RFC 9114 Section 7.2.4.1: HTTP/2 settings 0x02..=0x05 are forbidden.
                    if matches!(id, 0x02 | 0x03 | 0x04 | 0x05) {
                        return Err(FrameError::ReservedType(id));
                    }
                    let (val, u2) = varint::decode(&body[pos..]).map_err(|_| FrameError::Malformed)?;
                    pos += u2;
                    pairs.push((id, val));
                }
                H3Frame::Settings(pairs)
            }
            FRAME_PUSH_PROMISE => {
                let (push_id, u) = varint::decode(body).map_err(|_| FrameError::Malformed)?;
                H3Frame::PushPromise { push_id, header_block: body[u..].to_vec() }
            }
            FRAME_GOAWAY => {
                let (id, u) = varint::decode(body).map_err(|_| FrameError::Malformed)?;
                if u != body.len() { return Err(FrameError::Malformed); }
                H3Frame::GoAway(id)
            }
            FRAME_MAX_PUSH_ID => {
                let (id, u) = varint::decode(body).map_err(|_| FrameError::Malformed)?;
                if u != body.len() { return Err(FrameError::Malformed); }
                H3Frame::MaxPushId(id)
            }
            other => H3Frame::Unknown { frame_type: other, payload: body.to_vec() },
        };
        Ok((frame, total))
    }
}
