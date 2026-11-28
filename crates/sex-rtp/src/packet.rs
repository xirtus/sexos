// SPDX-License-Identifier: GPL-3.0-only
//! RTP packets (RFC 3550 5.1) and header extensions (RFC 8285 one- and two-byte forms).
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Malformed,
    TooLarge,
}

/// Header extension URIs WebRTC negotiates in SDP a=extmap.
pub mod uri {
    pub const ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
    pub const TRANSPORT_CC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
    pub const MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
    pub const RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
    pub const REPAIRED_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";
    pub const AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packet {
    pub marker: bool,
    pub payload_type: u8,
    pub seq: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub csrc: Vec<u32>,
    /// (id, data) pairs, ids 1..=14 for one-byte, 1..=255 for two-byte.
    pub extensions: Vec<(u8, Vec<u8>)>,
    pub payload: Vec<u8>,
    /// Padding bytes removed on parse; added on write when nonzero.
    pub padding: u8,
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl Packet {
    pub fn parse(b: &[u8]) -> Result<Packet, Error> {
        if b.len() < 12 || b[0] >> 6 != 2 {
            return Err(Error::Malformed);
        }
        let cc = (b[0] & 15) as usize;
        let mut off = 12 + cc * 4;
        if b.len() < off {
            return Err(Error::Malformed);
        }
        let csrc = (0..cc).map(|i| be32(&b[12 + 4 * i..])).collect();
        let mut extensions = Vec::new();
        if b[0] & 0x10 != 0 {
            if b.len() < off + 4 {
                return Err(Error::Malformed);
            }
            let profile = be16(&b[off..]);
            let len = be16(&b[off + 2..]) as usize * 4;
            let start = off + 4;
            if b.len() < start + len {
                return Err(Error::Malformed);
            }
            extensions = parse_extensions(profile, &b[start..start + len])?;
            off = start + len;
        }
        let mut end = b.len();
        let mut padding = 0;
        if b[0] & 0x20 != 0 {
            padding = *b.last().unwrap();
            if padding == 0 || end - off < padding as usize {
                return Err(Error::Malformed);
            }
            end -= padding as usize;
        }
        Ok(Packet {
            marker: b[1] & 0x80 != 0,
            payload_type: b[1] & 0x7f,
            seq: be16(&b[2..]),
            timestamp: be32(&b[4..]),
            ssrc: be32(&b[8..]),
            csrc,
            extensions,
            payload: b[off..end].into(),
            padding,
        })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(12 + self.payload.len() + 32);
        let x = !self.extensions.is_empty();
        v.push(0x80 | if self.padding > 0 { 0x20 } else { 0 } | if x { 0x10 } else { 0 } | self.csrc.len() as u8);
        v.push(if self.marker { 0x80 } else { 0 } | self.payload_type);
        v.extend_from_slice(&self.seq.to_be_bytes());
        v.extend_from_slice(&self.timestamp.to_be_bytes());
        v.extend_from_slice(&self.ssrc.to_be_bytes());
        for c in &self.csrc {
            v.extend_from_slice(&c.to_be_bytes());
        }
        if x {
            write_extensions(&mut v, &self.extensions);
        }
        v.extend_from_slice(&self.payload);
        if self.padding > 0 {
            v.extend(core::iter::repeat(0).take(self.padding as usize - 1));
            v.push(self.padding);
        }
        v
    }

    pub fn extension(&self, id: u8) -> Option<&[u8]> {
        self.extensions.iter().find(|e| e.0 == id).map(|e| e.1.as_slice())
    }

    pub fn set_extension(&mut self, id: u8, data: Vec<u8>) {
        match self.extensions.iter_mut().find(|e| e.0 == id) {
            Some(e) => e.1 = data,
            None => self.extensions.push((id, data)),
        }
    }
}

fn parse_extensions(profile: u16, d: &[u8]) -> Result<Vec<(u8, Vec<u8>)>, Error> {
    let mut v = Vec::new();
    let mut i = 0;
    if profile == 0xbede {
        while i < d.len() {
            let b = d[i];
            if b == 0 {
                i += 1;
                continue;
            }
            let (id, len) = (b >> 4, (b & 15) as usize + 1);
            if id == 15 {
                break;
            }
            if i + 1 + len > d.len() {
                return Err(Error::Malformed);
            }
            v.push((id, d[i + 1..i + 1 + len].into()));
            i += 1 + len;
        }
    } else if profile & 0xfff0 == 0x1000 {
        while i < d.len() {
            if d[i] == 0 {
                i += 1;
                continue;
            }
            if i + 2 > d.len() {
                return Err(Error::Malformed);
            }
            let (id, len) = (d[i], d[i + 1] as usize);
            if i + 2 + len > d.len() {
                return Err(Error::Malformed);
            }
            v.push((id, d[i + 2..i + 2 + len].into()));
            i += 2 + len;
        }
    }
    Ok(v)
}

/// One-byte form when every id is 1..=14 and every value 1..=16 bytes, else two-byte.
fn write_extensions(v: &mut Vec<u8>, exts: &[(u8, Vec<u8>)]) {
    let one = exts.iter().all(|(id, d)| (1..=14).contains(id) && (1..=16).contains(&d.len()));
    let start = v.len();
    v.extend_from_slice(&if one { 0xbedeu16 } else { 0x1000 }.to_be_bytes());
    v.extend_from_slice(&[0, 0]);
    for (id, d) in exts {
        if one {
            v.push(id << 4 | (d.len() - 1) as u8);
        } else {
            v.push(*id);
            v.push(d.len() as u8);
        }
        v.extend_from_slice(d);
    }
    while (v.len() - start) % 4 != 0 {
        v.push(0);
    }
    let words = ((v.len() - start - 4) / 4) as u16;
    v[start + 2..start + 4].copy_from_slice(&words.to_be_bytes());
}

/// abs-send-time: 6.18 fixed-point seconds, 24 bits, from microseconds.
pub fn abs_send_time(now_us: u64) -> [u8; 3] {
    let v = ((now_us << 18) / 1_000_000) as u32 & 0xff_ffff;
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// abs-send-time back to microseconds within its 64-second wrap.
pub fn abs_send_time_us(b: &[u8]) -> Option<u64> {
    if b.len() != 3 {
        return None;
    }
    let v = (b[0] as u64) << 16 | (b[1] as u64) << 8 | b[2] as u64;
    Some(v * 1_000_000 >> 18)
}

/// transport-wide-cc extension: a 16-bit transport sequence number.
pub fn twcc_seq(b: &[u8]) -> Option<u16> {
    (b.len() == 2).then(|| be16(b))
}

/// Extends 16-bit sequence numbers to 64 bits across wraps, relative to the newest seen.
#[derive(Clone, Copy, Debug, Default)]
pub struct SeqUnwrapper {
    last: Option<u64>,
}

impl SeqUnwrapper {
    pub fn unwrap(&mut self, seq: u16) -> u64 {
        let v = self.peek(seq);
        if self.last.map_or(true, |l| v > l) {
            self.last = Some(v);
        }
        v
    }
    pub fn peek(&self, seq: u16) -> u64 {
        match self.last {
            None => seq as u64 + 0x1_0000,
            Some(l) => {
                let d = seq.wrapping_sub(l as u16) as i16 as i64;
                (l as i64 + d).max(0) as u64
            }
        }
    }
}
