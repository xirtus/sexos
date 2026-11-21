// SPDX-License-Identifier: GPL-3.0-only
//! Control frames on ring 0: 32-bit words sent big-endian, then a CRC-32C of
//! those bytes (also big-endian). The frame's type ("PDF") rides in the ring
//! descriptor's SOF and EOF fields, not in the frame.

use crate::regs::FRAME_SIZE;

/// Frame types (protocol defined fields).
pub const PDF_READ: u8 = 1;
pub const PDF_WRITE: u8 = 2;
pub const PDF_ERROR: u8 = 3;
pub const PDF_NOTIFY_ACK: u8 = 4;
pub const PDF_EVENT: u8 = 5;
pub const PDF_XDOMAIN_REQUEST: u8 = 6;
pub const PDF_XDOMAIN_RESPONSE: u8 = 7;
pub const PDF_OVERRIDE: u8 = 8;
pub const PDF_RESET: u8 = 9;
pub const PDF_ICM_EVENT: u8 = 10;
pub const PDF_ICM_COMMAND: u8 = 11;
pub const PDF_ICM_RESPONSE: u8 = 12;

/// Most words a frame holds (256 bytes less the CRC).
pub const MAX_WORDS: usize = (FRAME_SIZE - 4) / 4;

/// CRC-32C (Castagnoli), as the control channel uses it.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 { c = if c & 1 != 0 { (c >> 1) ^ 0x82f6_3b78 } else { c >> 1 }; }
    }
    !c
}

/// Writes `words` as a frame into `out`; returns its length with the CRC, or None
/// if it does not fit.
pub fn encode(words: &[u32], out: &mut [u8]) -> Option<usize> {
    let n = words.len() * 4;
    if words.len() > MAX_WORDS || out.len() < n + 4 { return None; }
    for (i, w) in words.iter().enumerate() { out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes()); }
    let crc = crc32c(&out[..n]);
    out[n..n + 4].copy_from_slice(&crc.to_be_bytes());
    Some(n + 4)
}

/// Whether frames of this type carry a CRC the receiver checks (the firmware's
/// own ICM events and responses are not checked).
pub fn checked(pdf: u8) -> bool {
    matches!(pdf, PDF_READ | PDF_WRITE | PDF_ERROR | PDF_OVERRIDE | PDF_RESET | PDF_EVENT | PDF_XDOMAIN_REQUEST | PDF_XDOMAIN_RESPONSE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Size, Crc }

/// A received frame's words.
#[derive(Clone, Copy)]
pub struct Words { pub pdf: u8, pub len: usize, pub w: [u32; MAX_WORDS] }
impl Words {
    pub fn words(&self) -> &[u32] { &self.w[..self.len] }
    pub fn word(&self, i: usize) -> u32 { if i < self.len { self.w[i] } else { 0 } }
}

/// Reads a received frame (`bytes` includes its CRC).
pub fn decode(pdf: u8, bytes: &[u8]) -> Result<Words, Error> {
    if bytes.len() < 4 || bytes.len() % 4 != 0 || bytes.len() > FRAME_SIZE { return Err(Error::Size); }
    let n = bytes.len() - 4;
    if checked(pdf) && crc32c(&bytes[..n]).to_be_bytes() != bytes[n..] { return Err(Error::Crc); }
    let mut out = Words { pdf, len: n / 4, w: [0; MAX_WORDS] };
    for i in 0..out.len { out.w[i] = u32::from_be_bytes([bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3]]); }
    Ok(out)
}

/// Config spaces a control READ/WRITE addresses.
pub const SPACE_HOPS: u8 = 0;
pub const SPACE_PORT: u8 = 1;
pub const SPACE_SWITCH: u8 = 2;
pub const SPACE_COUNTERS: u8 = 3;

/// A config read of `length` dwords at `offset` in a router's `space`: route
/// (two words) then the address word.
pub fn config_read(route: u64, space: u8, port: u8, offset: u16, length: u8, seq: u8) -> [u32; 3] {
    let address = (offset as u32 & 0x1fff) | (length as u32 & 0x3f) << 13 | (port as u32 & 0x3f) << 19
        | (space as u32 & 3) << 25 | (seq as u32 & 3) << 27;
    [(route >> 32) as u32 & 0x003f_ffff, route as u32, address]
}

/// The route a control frame names (replies set the top bit of the first word).
pub fn route_of(w: &Words) -> u64 { ((w.word(0) & 0x003f_ffff) as u64) << 32 | w.word(1) as u64 }
