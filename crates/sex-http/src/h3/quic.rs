// SPDX-License-Identifier: GPL-3.0-only
//! RFC 9000 QUIC packet header and frame codec (sans-I/O, `no_std` + `alloc`).

use super::varint::{self, VarintError};
use alloc::vec::Vec;

/// QUIC version 1 (RFC 9000).
pub const QUIC_VERSION_1: u32 = 0x0000_0001;
/// Maximum QUIC connection ID length (RFC 9000 Section 17.2).
pub const MAX_CID_LEN: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuicError {
    Incomplete,
    Varint(VarintError),
    InvalidHeader,
    InvalidCidLength,
    UnknownFrame(u64),
    MalformedFrame,
}

impl From<VarintError> for QuicError {
    fn from(e: VarintError) -> Self {
        match e {
            VarintError::Incomplete => QuicError::Incomplete,
            other => QuicError::Varint(other),
        }
    }
}

/// RFC 9000 Section 17.2 long packet types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LongPacketType {
    Initial = 0,
    ZeroRtt = 1,
    Handshake = 2,
    Retry = 3,
}

impl LongPacketType {
    fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::Initial,
            1 => Self::ZeroRtt,
            2 => Self::Handshake,
            _ => Self::Retry,
        }
    }
}

/// QUIC packet header (Long or Short, RFC 9000 Section 17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PacketHeader {
    Long {
        packet_type: LongPacketType,
        version: u32,
        dcid: Vec<u8>,
        scid: Vec<u8>,
        /// Only present on Initial packets.
        token: Vec<u8>,
        /// Payload length (zero for Retry).
        payload_len: u64,
    },
    Short {
        spin: bool,
        key_phase: bool,
        dcid: Vec<u8>,
    },
}

impl PacketHeader {
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), QuicError> {
        match self {
            PacketHeader::Long { packet_type, version, dcid, scid, token, payload_len } => {
                if dcid.len() > MAX_CID_LEN || scid.len() > MAX_CID_LEN {
                    return Err(QuicError::InvalidCidLength);
                }
                let first = 0x80 | 0x40 | ((*packet_type as u8) << 4);
                out.push(first);
                out.extend_from_slice(&version.to_be_bytes());
                out.push(dcid.len() as u8);
                out.extend_from_slice(dcid);
                out.push(scid.len() as u8);
                out.extend_from_slice(scid);
                if *packet_type == LongPacketType::Initial {
                    varint::encode(token.len() as u64, out)?;
                    out.extend_from_slice(token);
                }
                if *packet_type != LongPacketType::Retry {
                    varint::encode(*payload_len, out)?;
                }
                Ok(())
            }
            PacketHeader::Short { spin, key_phase, dcid } => {
                if dcid.len() > MAX_CID_LEN {
                    return Err(QuicError::InvalidCidLength);
                }
                let first = 0x40
                    | if *spin { 0x20 } else { 0 }
                    | if *key_phase { 0x04 } else { 0 };
                out.push(first);
                out.extend_from_slice(dcid);
                Ok(())
            }
        }
    }

    /// Decodes a QUIC packet header. `short_dcid_len` specifies the expected DCID length
    /// for 1-RTT short headers (RFC 9000 Section 17.3).
    pub fn decode(input: &[u8], short_dcid_len: usize) -> Result<(Self, usize), QuicError> {
        let first = *input.first().ok_or(QuicError::Incomplete)?;
        // Fixed bit (0x40) must be set for QUIC v1 packets.
        if (first & 0x40) == 0 {
            return Err(QuicError::InvalidHeader);
        }
        if (first & 0x80) != 0 {
            if input.len() < 7 {
                return Err(QuicError::Incomplete);
            }
            let packet_type = LongPacketType::from_bits((first >> 4) & 0x03);
            let version = u32::from_be_bytes([input[1], input[2], input[3], input[4]]);
            let mut at = 5usize;
            let dcid_len = *input.get(at).ok_or(QuicError::Incomplete)? as usize;
            at += 1;
            if dcid_len > MAX_CID_LEN {
                return Err(QuicError::InvalidCidLength);
            }
            let dcid = input.get(at..at + dcid_len).ok_or(QuicError::Incomplete)?.to_vec();
            at += dcid_len;
            let scid_len = *input.get(at).ok_or(QuicError::Incomplete)? as usize;
            at += 1;
            if scid_len > MAX_CID_LEN {
                return Err(QuicError::InvalidCidLength);
            }
            let scid = input.get(at..at + scid_len).ok_or(QuicError::Incomplete)?.to_vec();
            at += scid_len;
            let mut token = Vec::new();
            if packet_type == LongPacketType::Initial {
                let (tlen, used) = varint::decode(&input[at..])?;
                at += used;
                let tlen = usize::try_from(tlen).map_err(|_| QuicError::InvalidHeader)?;
                token = input.get(at..at + tlen).ok_or(QuicError::Incomplete)?.to_vec();
                at += tlen;
            }
            let payload_len = if packet_type != LongPacketType::Retry {
                let (plen, used) = varint::decode(&input[at..])?;
                at += used;
                plen
            } else {
                0
            };
            Ok((PacketHeader::Long { packet_type, version, dcid, scid, token, payload_len }, at))
        } else {
            if short_dcid_len > MAX_CID_LEN {
                return Err(QuicError::InvalidCidLength);
            }
            let dcid = input.get(1..1 + short_dcid_len).ok_or(QuicError::Incomplete)?.to_vec();
            Ok((
                PacketHeader::Short {
                    spin: (first & 0x20) != 0,
                    key_phase: (first & 0x04) != 0,
                    dcid,
                },
                1 + short_dcid_len,
            ))
        }
    }
}

/// Core RFC 9000 QUIC transport frames needed for HTTP/3 over QUIC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuicFrame {
    Padding,
    Ping,
    Ack {
        largest_acked: u64,
        ack_delay: u64,
        first_ack_range: u64,
        /// Additional `(gap, ack_range_length)` pairs.
        ranges: Vec<(u64, u64)>,
    },
    Crypto {
        offset: u64,
        data: Vec<u8>,
    },
    Stream {
        stream_id: u64,
        offset: u64,
        fin: bool,
        data: Vec<u8>,
    },
    MaxData {
        max_data: u64,
    },
    MaxStreamData {
        stream_id: u64,
        max_stream_data: u64,
    },
    ConnectionClose {
        is_app: bool,
        error_code: u64,
        frame_type: Option<u64>,
        reason: Vec<u8>,
    },
}

impl QuicFrame {
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), QuicError> {
        match self {
            QuicFrame::Padding => {
                out.push(0x00);
            }
            QuicFrame::Ping => {
                out.push(0x01);
            }
            QuicFrame::Ack { largest_acked, ack_delay, first_ack_range, ranges } => {
                out.push(0x02);
                varint::encode(*largest_acked, out)?;
                varint::encode(*ack_delay, out)?;
                varint::encode(ranges.len() as u64, out)?;
                varint::encode(*first_ack_range, out)?;
                for &(gap, len) in ranges {
                    varint::encode(gap, out)?;
                    varint::encode(len, out)?;
                }
            }
            QuicFrame::Crypto { offset, data } => {
                out.push(0x06);
                varint::encode(*offset, out)?;
                varint::encode(data.len() as u64, out)?;
                out.extend_from_slice(data);
            }
            QuicFrame::Stream { stream_id, offset, fin, data } => {
                let mut kind = 0x08u8 | 0x02; // Always include LEN bit so frames are self-delimiting.
                if *offset != 0 {
                    kind |= 0x04;
                }
                if *fin {
                    kind |= 0x01;
                }
                out.push(kind);
                varint::encode(*stream_id, out)?;
                if *offset != 0 {
                    varint::encode(*offset, out)?;
                }
                varint::encode(data.len() as u64, out)?;
                out.extend_from_slice(data);
            }
            QuicFrame::MaxData { max_data } => {
                out.push(0x10);
                varint::encode(*max_data, out)?;
            }
            QuicFrame::MaxStreamData { stream_id, max_stream_data } => {
                out.push(0x11);
                varint::encode(*stream_id, out)?;
                varint::encode(*max_stream_data, out)?;
            }
            QuicFrame::ConnectionClose { is_app, error_code, frame_type, reason } => {
                out.push(if *is_app { 0x1D } else { 0x1C });
                varint::encode(*error_code, out)?;
                if !*is_app {
                    varint::encode(frame_type.unwrap_or(0), out)?;
                }
                varint::encode(reason.len() as u64, out)?;
                out.extend_from_slice(reason);
            }
        }
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<(Self, usize), QuicError> {
        let (kind, mut at) = varint::decode(input)?;
        match kind {
            0x00 => Ok((QuicFrame::Padding, at)),
            0x01 => Ok((QuicFrame::Ping, at)),
            0x02 | 0x03 => {
                let (largest_acked, u) = varint::decode(&input[at..])?;
                at += u;
                let (ack_delay, u) = varint::decode(&input[at..])?;
                at += u;
                let (range_count, u) = varint::decode(&input[at..])?;
                at += u;
                if range_count > 64 {
                    return Err(QuicError::MalformedFrame);
                }
                let (first_ack_range, u) = varint::decode(&input[at..])?;
                at += u;
                let mut ranges = Vec::with_capacity(range_count as usize);
                for _ in 0..range_count {
                    let (gap, u1) = varint::decode(&input[at..])?;
                    at += u1;
                    let (len, u2) = varint::decode(&input[at..])?;
                    at += u2;
                    ranges.push((gap, len));
                }
                if kind == 0x03 {
                    // Skip 3 ECN count varints (ECT0, ECT1, ECN-CE).
                    for _ in 0..3 {
                        let (_, u) = varint::decode(&input[at..])?;
                        at += u;
                    }
                }
                Ok((QuicFrame::Ack { largest_acked, ack_delay, first_ack_range, ranges }, at))
            }
            0x06 => {
                let (offset, u) = varint::decode(&input[at..])?;
                at += u;
                let (len, u) = varint::decode(&input[at..])?;
                at += u;
                let len = usize::try_from(len).map_err(|_| QuicError::MalformedFrame)?;
                let data = input.get(at..at + len).ok_or(QuicError::Incomplete)?.to_vec();
                at += len;
                Ok((QuicFrame::Crypto { offset, data }, at))
            }
            0x08..=0x0F => {
                let has_off = (kind & 0x04) != 0;
                let has_len = (kind & 0x02) != 0;
                let fin = (kind & 0x01) != 0;
                let (stream_id, u) = varint::decode(&input[at..])?;
                at += u;
                let offset = if has_off {
                    let (o, u) = varint::decode(&input[at..])?;
                    at += u;
                    o
                } else {
                    0
                };
                let data = if has_len {
                    let (len, u) = varint::decode(&input[at..])?;
                    at += u;
                    let len = usize::try_from(len).map_err(|_| QuicError::MalformedFrame)?;
                    let slice = input.get(at..at + len).ok_or(QuicError::Incomplete)?;
                    at += len;
                    slice.to_vec()
                } else {
                    let slice = &input[at..];
                    at = input.len();
                    slice.to_vec()
                };
                Ok((QuicFrame::Stream { stream_id, offset, fin, data }, at))
            }
            0x10 => {
                let (max_data, u) = varint::decode(&input[at..])?;
                at += u;
                Ok((QuicFrame::MaxData { max_data }, at))
            }
            0x11 => {
                let (stream_id, u) = varint::decode(&input[at..])?;
                at += u;
                let (max_stream_data, u) = varint::decode(&input[at..])?;
                at += u;
                Ok((QuicFrame::MaxStreamData { stream_id, max_stream_data }, at))
            }
            0x1C | 0x1D => {
                let is_app = kind == 0x1D;
                let (error_code, u) = varint::decode(&input[at..])?;
                at += u;
                let frame_type = if !is_app {
                    let (ft, u) = varint::decode(&input[at..])?;
                    at += u;
                    Some(ft)
                } else {
                    None
                };
                let (reason_len, u) = varint::decode(&input[at..])?;
                at += u;
                let reason_len = usize::try_from(reason_len).map_err(|_| QuicError::MalformedFrame)?;
                let reason = input.get(at..at + reason_len).ok_or(QuicError::Incomplete)?.to_vec();
                at += reason_len;
                Ok((QuicFrame::ConnectionClose { is_app, error_code, frame_type, reason }, at))
            }
            other => Err(QuicError::UnknownFrame(other)),
        }
    }
}
