// SPDX-License-Identifier: GPL-3.0-only
//! The open-internet streams: TCP flows and UDP sockets made as programs ask for them, over
//! IPv4 or IPv6, reached by name through whatever
//! the network's DHCP and DNS say. Holding the BOOT route to the network service and
//! the region shared with it is the right. An address is data, not authority.

pub const OP: u64 = 0x211;
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
/// Encrypt the flow just connected. The region holds the host name to check against.
pub const TLS_START: u8 = 8;
pub const TLS_STAGE: u8 = 9;
pub const FINISH_REQUEST: u8 = 10;

pub const TLS_HANDSHAKE: u32 = 0;
pub const TLS_READY: u32 = 1;
pub const TLS_FAILED_CERTIFICATE: u32 = 2;
pub const TLS_FAILED_SIGNATURE: u32 = 3;
pub const TLS_FAILED_OTHER: u32 = 4;

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
/// The same on the caller's flow `flow` (its own number, below `CLIENT_FLOWS`). A
/// request without one is on flow 0.
pub const fn request_on(seq: u32, kind: u8, flow: u8) -> u64 { request(seq, kind) | (flow as u64) << 24 }
pub const fn flow_of(a: u64) -> u8 { (a >> 24) as u8 }
/// Flows one caller may number at once. The service makes flows as callers ask for
/// them; a `CONNECT` answers `BUSY` only when memory is short.
pub const CLIENT_FLOWS: u8 = 255;
pub const fn address_port(address: [u8; 4], port: u16) -> u64 { (u32::from_be_bytes(address) as u64) << 16 | port as u64 }
/// Shell's route to the network service, for binding program runs alone.
pub const SHELL_SLOT: u64 = 262;
/// Program rows follow the Spindles and Browsers in the provider slots and regions.
pub const PROGRAM_INDEX_BASE: usize = crate::app_catalog::SPINDLE_TARGETS.len() + crate::browser_sources::PLANS.len();

// Kinds that name addresses of either family. An endpoint in the region is
// `ENDPOINT_BYTES`: the family (`FAMILY_V4` or `FAMILY_V6`), sixteen address bytes
// (an IPv4 address in the first four) and the port, big-endian.
/// Look a name up in both families. value: the name's length to ask, 0 for the answer.
/// The answer's value is how many addresses (1 or 2) the region holds, `ADDRESS_BYTES`
/// each, the one to try first first: IPv6 when the machine has a global IPv6 address
/// and the name has one, then IPv4.
pub const RESOLVE_ALL: u8 = 11;
/// Connect the flow to the address in the region (`ADDRESS_BYTES`); value: the port.
pub const CONNECT_TO: u8 = 12;
/// Make the flow a UDP socket on a local port (value; 0 takes any). Answers the port.
pub const UDP_BIND: u8 = 13;
/// Send one datagram: the region holds the endpoint, then the payload; value: the
/// payload's length. `WAIT` when the send queue has no room for it yet.
pub const UDP_SEND: u8 = 14;
/// Take one datagram, if one came: the region gets its sender's endpoint, then the
/// payload; the value is the payload's length. `WAIT` when none has. value: room.
pub const UDP_RECEIVE: u8 = 15;
/// Make the flow a TCP listener on a local port (value). `STATE` says opening until a
/// peer connects.
pub const LISTEN: u8 = 16;
/// On a listener a peer connected to: the connection becomes the caller's flow
/// numbered by value, and the listener listens again. The region gets the peer's
/// endpoint. `WAIT` while no peer has connected.
pub const ACCEPT: u8 = 17;
/// The machine's addresses: the value is how many, each `ADDRESS_BYTES` and a prefix
/// length byte in the region.
pub const ADDRESSES: u8 = 18;
/// Send the flow's FIN after the bytes already queued; the flow can still read. A
/// `CLOSE` after this lets the connection finish on its own instead of resetting it.
pub const SHUTDOWN: u8 = 19;

/// The port is held by another flow, or is one the service keeps for itself.
pub const IN_USE: u8 = 8;

pub const FAMILY_V4: u8 = 4;
pub const FAMILY_V6: u8 = 6;
pub const ADDRESS_BYTES: usize = 17;
pub const ENDPOINT_BYTES: usize = 19;
/// Programs may bind and listen on ports 1 to this, except 68 (the DHCP client's).
/// The ports above it are the service's own.
pub const PROGRAM_PORT_MAX: u16 = 39_998;
pub fn put_address(out: &mut [u8], family: u8, address: &[u8]) {
    out[0] = family;
    out[1..17].fill(0);
    out[1..1 + address.len()].copy_from_slice(address);
}
/// The family and its address bytes (4 or 16), or None for an unknown family.
pub fn address_of(bytes: &[u8]) -> Option<(u8, &[u8])> {
    match *bytes.first()? {
        FAMILY_V4 => Some((FAMILY_V4, bytes.get(1..5)?)),
        FAMILY_V6 => Some((FAMILY_V6, bytes.get(1..17)?)),
        _ => None,
    }
}
pub fn put_endpoint(out: &mut [u8], family: u8, address: &[u8], port: u16) {
    put_address(out, family, address);
    out[17..19].copy_from_slice(&port.to_be_bytes());
}
pub fn endpoint_of(bytes: &[u8]) -> Option<(u8, &[u8], u16)> {
    let (family, address) = address_of(bytes)?;
    Some((family, address, u16::from_be_bytes([*bytes.get(17)?, *bytes.get(18)?])))
}
