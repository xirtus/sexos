// SPDX-License-Identifier: GPL-3.0-only
//! Tether v2 wire framing: length-prefixed multiplexed binary frames.
//!
//! Stream allocation:
//! - `0` (`STREAM_ACP`): Bidirectional ACP JSON-RPC 2.0 between SexOS Tether GUI and host `agy acp`
//! - `1` (`STREAM_CONTROL`): Handshake, PIN auth, capability sync, heartbeats, telemetry, kill-switch
//! - `2` (`STREAM_LOGS`): Live kernel/serial log events pushed from SexOS to host
//! - `3..` (`STREAM_RPC_START`): Ephemeral per-RPC request/response and binary transfer streams

use alloc::vec::Vec;

pub const MAGIC: [u8; 4] = *b"TTH2";
pub const PROTOCOL_VERSION: u16 = 2;
pub const DEFAULT_PORT: u16 = 7178;
pub const DEFAULT_CONTROL_PORT: u16 = 7179;
pub const HEADER_LEN: usize = 10;

pub const STREAM_ACP: u32 = 0;
pub const STREAM_CONTROL: u32 = 1;
pub const STREAM_LOGS: u32 = 2;
pub const STREAM_RPC_START: u32 = 3;

/// Default maximum payload per frame (4 MiB — safe for SexOS RAM & large binary chunks).
pub const DEFAULT_MAX_PAYLOAD: usize = 4 * 1024 * 1024;

pub const FLAG_END_STREAM: u8 = 1 << 0;
pub const FLAG_ERROR: u8 = 1 << 1;
pub const FLAG_REQUIRES_APPROVAL: u8 = 1 << 2;
pub const FLAG_BINARY: u8 = 1 << 3;

/// Heartbeat payload tag: Ping (receiver replies with `HEARTBEAT_PONG` + remaining bytes).
pub const HEARTBEAT_PING: u8 = 0;
/// Heartbeat payload tag: Pong (receiver updates liveness timestamp and does NOT echo).
pub const HEARTBEAT_PONG: u8 = 1;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    Hello = 1,
    AuthChallenge = 2,
    AuthResponse = 3,
    AuthOk = 4,
    AuthErr = 5,
    Heartbeat = 6,
    AcpJson = 10,
    RpcRequest = 20,
    RpcResponse = 21,
    RpcStreamChunk = 22,
    RpcStreamEnd = 23,
    LogEvent = 30,
    Telemetry = 31,
    WaveEvent = 32,
    SnapshotEvent = 33,
    CapabilitySync = 34,
    Cancel = 40,
    KillSwitch = 41,
}

impl FrameKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Hello),
            2 => Some(Self::AuthChallenge),
            3 => Some(Self::AuthResponse),
            4 => Some(Self::AuthOk),
            5 => Some(Self::AuthErr),
            6 => Some(Self::Heartbeat),
            10 => Some(Self::AcpJson),
            20 => Some(Self::RpcRequest),
            21 => Some(Self::RpcResponse),
            22 => Some(Self::RpcStreamChunk),
            23 => Some(Self::RpcStreamEnd),
            30 => Some(Self::LogEvent),
            31 => Some(Self::Telemetry),
            32 => Some(Self::WaveEvent),
            33 => Some(Self::SnapshotEvent),
            34 => Some(Self::CapabilitySync),
            40 => Some(Self::Cancel),
            41 => Some(Self::KillSwitch),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub stream_id: u32,
    pub kind: FrameKind,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(stream_id: u32, kind: FrameKind, flags: u8, payload: Vec<u8>) -> Self {
        Self {
            stream_id,
            kind,
            flags,
            payload,
        }
    }

    pub fn control(kind: FrameKind, payload: Vec<u8>) -> Self {
        Self::new(STREAM_CONTROL, kind, 0, payload)
    }

    pub fn acp(json: &[u8]) -> Self {
        Self::new(STREAM_ACP, FrameKind::AcpJson, 0, json.to_vec())
    }

    pub fn heartbeat_ping(seq_or_ticks: u64) -> Self {
        let mut payload = Vec::with_capacity(9);
        payload.push(HEARTBEAT_PING);
        payload.extend_from_slice(&seq_or_ticks.to_le_bytes());
        Self::control(FrameKind::Heartbeat, payload)
    }

    pub fn heartbeat_pong(ping_payload: &[u8]) -> Self {
        let mut payload = Vec::with_capacity(ping_payload.len().max(1));
        payload.push(HEARTBEAT_PONG);
        if ping_payload.len() > 1 {
            payload.extend_from_slice(&ping_payload[1..]);
        }
        Self::control(FrameKind::Heartbeat, payload)
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.reserve(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.stream_id.to_le_bytes());
        out.push(self.kind as u8);
        out.push(self.flags);
        out.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.payload);
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        self.encode(&mut out);
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    UnknownKind(u8),
    PayloadTooLarge(usize),
    BufferOverflow,
}

/// Incremental, zero-panic frame decoder for streaming TCP connections.
pub struct FrameDecoder {
    buf: Vec<u8>,
    max_payload: usize,
    expect_magic: bool,
}

impl FrameDecoder {
    pub fn new(expect_magic: bool) -> Self {
        Self::with_max_payload(expect_magic, DEFAULT_MAX_PAYLOAD)
    }

    pub fn with_max_payload(expect_magic: bool, max_payload: usize) -> Self {
        Self {
            buf: Vec::new(),
            max_payload,
            expect_magic,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<(), DecodeError> {
        let limit = self.max_payload.saturating_add(HEADER_LEN + 64 * 1024);
        if self.buf.len().saturating_add(bytes.len()) > limit {
            return Err(DecodeError::BufferOverflow);
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, DecodeError> {
        if self.expect_magic {
            if self.buf.len() < MAGIC.len() {
                return Ok(None);
            }
            if self.buf[..MAGIC.len()] != MAGIC {
                return Err(DecodeError::BadMagic);
            }
            self.buf.drain(..MAGIC.len());
            self.expect_magic = false;
        }

        if self.buf.len() < HEADER_LEN {
            return Ok(None);
        }

        let stream_id = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        let kind_raw = self.buf[4];
        let kind = FrameKind::from_u8(kind_raw).ok_or(DecodeError::UnknownKind(kind_raw))?;
        let flags = self.buf[5];
        let len = u32::from_le_bytes([self.buf[6], self.buf[7], self.buf[8], self.buf[9]]) as usize;

        if len > self.max_payload {
            return Err(DecodeError::PayloadTooLarge(len));
        }

        let total = HEADER_LEN + len;
        if self.buf.len() < total {
            return Ok(None);
        }

        let payload = self.buf[HEADER_LEN..total].to_vec();
        self.buf.drain(..total);
        Ok(Some(Frame {
            stream_id,
            kind,
            flags,
            payload,
        }))
    }

    pub fn buffered_len(&self) -> usize {
        self.buf.len()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }
}
