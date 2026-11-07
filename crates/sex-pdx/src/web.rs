// SPDX-License-Identifier: GPL-3.0-only
//! The open-internet stream: one TCP flow at a time, reached by name through whatever
//! the network's DHCP and DNS say. Holding the BOOT route to the network service and
//! the region shared with it is the right. An address is data, not authority.

pub const OP: u64 = 0x20E;
pub const CLIENT_SLOT: u64 = 259;
pub const PROVIDER_SLOT_BASE: u64 = 340;
pub const REGION_ID_BASE: u32 = 0x5100;
pub const REGION_BYTES: usize = 64 * 1024;

pub const UP: u8 = 1;
pub const RESOLVE: u8 = 2;
pub const CONNECT: u8 = 3;
pub const STATE: u8 = 4;
pub const SEND: u8 = 5;
pub const RECEIVE: u8 = 6;
pub const CLOSE: u8 = 7;

pub const OK: u8 = 0;
pub const WAIT: u8 = 1;
pub const DENIED: u8 = 2;
pub const OFFLINE: u8 = 3;
pub const NO_NAME: u8 = 4;
pub const BUSY: u8 = 5;
pub const FAILED: u8 = 6;
pub const INVALID: u8 = 7;

pub const STATE_OPENING: u32 = 0;
pub const STATE_OPEN: u32 = 1;
pub const STATE_PEER_CLOSED: u32 = 2;
pub const STATE_CLOSED: u32 = 3;

const TAG: u64 = 0xF6;
pub const fn reply(seq: u32, status: u8, value: u32) -> u64 {
    TAG << 56 | ((seq as u64) & 0xFF_FFFF) << 32 | (status as u64) << 24 | (value as u64 & 0xFF_FFFF)
}
pub fn decode_reply(word: u64) -> Option<(u32, u8, u32)> {
    if word >> 56 != TAG { return None; }
    Some((((word >> 32) & 0xFF_FFFF) as u32, (word >> 24) as u8, (word & 0xFF_FFFF) as u32))
}
/// args: seq and kind; then a value (name length, address and port, or byte count).
pub const fn request(seq: u32, kind: u8) -> u64 { (seq as u64 & 0xFF_FFFF) | (kind as u64) << 32 }
pub const fn address_port(address: [u8; 4], port: u16) -> u64 { (u32::from_be_bytes(address) as u64) << 16 | port as u64 }
