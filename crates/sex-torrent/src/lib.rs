// SPDX-License-Identifier: GPL-3.0-only
//! The SexOS BitTorrent engine: the library behind Swingers, and the one Qupid and
//! Kaleidoscope embed for streaming and magnet links.
//!
//! No I/O. Every protocol here is a state machine: the program that embeds it (Swingers,
//! a confined run) owns the sockets, the files and the clock, hands this crate the bytes
//! and the time that arrived, and sends what comes back. That keeps the engine testable on
//! the host, and keeps the network grant (for example "only through the WireGuard tunnel")
//! with the program, where it cannot be bypassed by a code path that opens its own socket.
//!
//! Map of the crate (BEP numbers are the BitTorrent Enhancement Proposals):
//! - `bencode`: the wire and file encoding (BEP 3), with exact byte spans for info-hashes.
//! - `metainfo`, `merkle`, `magnet`, `create`: torrents v1, v2 and hybrid (BEP 3, 52, 47),
//!   magnet links (BEP 9, 53), making torrents.
//! - `peer`, `extension`, `picker`, `storage`, `resume`, `session`: the peer wire protocol
//!   (BEP 3, 6, 10, 11, 9, 21, 52 hash requests), piece picking (rarest first, sequential,
//!   streaming deadlines), the file layout, resume data, and the session that runs them.
//! - `tracker`, `dht`, `lsd`, `webseed`: finding peers (BEP 3, 15, 23, 7, 5, 32, 14, 19).
//! - `utp`, `ledbat`, `mse`, `portmap`: transport (BEP 29, RFC 6817), protocol encryption,
//!   NAT-PMP, PCP and UPnP IGD port mapping.
//! - `rss`, `torznab`, `rules`: automation (feeds, indexers, episode rules).
//! - `rpc`, `webui`: remote control (Transmission-compatible RPC).
//! - `bandwidth`, `anonymous`: limits, schedules, ratio rules and anonymous mode.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod bencode;

pub mod metainfo;
pub mod merkle;
pub mod magnet;
pub mod create;

pub mod peer;
pub mod extension;
pub mod picker;
pub mod storage;
pub mod resume;
pub mod session;

pub mod tracker;
pub mod dht;
pub mod lsd;
pub mod webseed;

pub mod utp;
pub mod ledbat;
pub mod mse;
pub mod portmap;

pub mod rss;
pub mod torznab;
pub mod rules;

pub mod rpc;
pub mod webui;

pub mod bandwidth;
pub mod anonymous;

use core::fmt;

/// A SHA-1 info-hash (v1 torrents, and the v1 half of a hybrid).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct InfoHashV1(pub [u8; 20]);

/// A SHA-256 info-hash (v2 torrents). On the wire (handshake, DHT, trackers) a v2 torrent
/// is named by the first 20 bytes of it ([`InfoHashV2::truncated`]).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct InfoHashV2(pub [u8; 32]);

impl InfoHashV2 {
    pub fn truncated(&self) -> [u8; 20] {
        let mut out = [0; 20];
        out.copy_from_slice(&self.0[..20]);
        out
    }
}

/// What names a torrent: v1, v2 or both (a hybrid).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum InfoHashes {
    V1(InfoHashV1),
    V2(InfoHashV2),
    Hybrid(InfoHashV1, InfoHashV2),
}

impl InfoHashes {
    pub fn v1(&self) -> Option<InfoHashV1> {
        match *self { InfoHashes::V1(a) | InfoHashes::Hybrid(a, _) => Some(a), InfoHashes::V2(_) => None }
    }
    pub fn v2(&self) -> Option<InfoHashV2> {
        match *self { InfoHashes::V2(b) | InfoHashes::Hybrid(_, b) => Some(b), InfoHashes::V1(_) => None }
    }
    /// The 20 bytes used in handshakes, trackers and the DHT: the v1 hash when there is
    /// one, otherwise the truncated v2 hash.
    pub fn wire(&self) -> [u8; 20] {
        match self.v1() { Some(a) => a.0, None => self.v2().map(|b| b.truncated()).unwrap_or([0; 20]) }
    }
}

fn hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for b in bytes { write!(f, "{:02x}", b)?; }
    Ok(())
}
impl fmt::Debug for InfoHashV1 { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { hex(f, &self.0) } }
impl fmt::Display for InfoHashV1 { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { hex(f, &self.0) } }
impl fmt::Debug for InfoHashV2 { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { hex(f, &self.0) } }
impl fmt::Display for InfoHashV2 { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { hex(f, &self.0) } }

/// Lower-case hex of any bytes.
pub fn to_hex(bytes: &[u8]) -> alloc::string::String {
    let mut s = alloc::string::String::with_capacity(bytes.len() * 2);
    for b in bytes { s.push(char::from_digit((b >> 4) as u32, 16).unwrap()); s.push(char::from_digit((b & 15) as u32, 16).unwrap()); }
    s
}

/// Parses hex (either case) of exactly `N` bytes.
pub fn from_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let b = s.as_bytes();
    if b.len() != N * 2 { return None; }
    let mut out = [0u8; N];
    for i in 0..N {
        let hi = (b[2 * i] as char).to_digit(16)?;
        let lo = (b[2 * i + 1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// Our 20-byte peer ID. Normally Azureus style (`-SX0100-` plus 12 random bytes); in
/// anonymous mode the prefix is random too, so no client string is sent.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PeerId(pub [u8; 20]);

pub const CLIENT_PREFIX: &[u8; 8] = b"-SX0100-";

impl PeerId {
    /// `random` fills the bytes it is given (the program's hardware generator).
    pub fn generate(anonymous: bool, random: &mut dyn FnMut(&mut [u8])) -> PeerId {
        let mut id = [0u8; 20];
        random(&mut id);
        if !anonymous {
            id[..8].copy_from_slice(CLIENT_PREFIX);
        }
        PeerId(id)
    }
}

/// An IPv4 or IPv6 peer address. Same shape as `sex_app::net::IpAddr`, kept here so the
/// crate builds on the host without the SexOS runtime.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Ip { V4([u8; 4]), V6([u8; 16]) }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PeerAddr { pub ip: Ip, pub port: u16 }

impl PeerAddr {
    /// Compact form (BEP 23 / BEP 7): 6 bytes for IPv4, 18 for IPv6.
    pub fn compact(&self) -> alloc::vec::Vec<u8> {
        let mut v = alloc::vec::Vec::with_capacity(18);
        match self.ip { Ip::V4(a) => v.extend_from_slice(&a), Ip::V6(a) => v.extend_from_slice(&a) }
        v.extend_from_slice(&self.port.to_be_bytes());
        v
    }
    /// Parses a run of compact addresses; `six` picks 18-byte entries.
    pub fn parse_compact(bytes: &[u8], six: bool) -> alloc::vec::Vec<PeerAddr> {
        let n = if six { 18 } else { 6 };
        bytes.chunks_exact(n).map(|c| {
            let ip = if six { let mut a = [0; 16]; a.copy_from_slice(&c[..16]); Ip::V6(a) } else { Ip::V4([c[0], c[1], c[2], c[3]]) };
            PeerAddr { ip, port: u16::from_be_bytes([c[n - 2], c[n - 1]]) }
        }).collect()
    }
}

/// Milliseconds on the program's monotonic clock. Every timer in the crate uses it.
pub type Millis = u64;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_round_trip() {
        let h = from_hex::<4>("DEADbeef").unwrap();
        assert_eq!(h, [0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(to_hex(&h), "deadbeef");
    }
    #[test]
    fn peer_id_prefix_only_when_not_anonymous() {
        let mut fill = |b: &mut [u8]| b.fill(7);
        assert_eq!(&PeerId::generate(false, &mut fill).0[..8], CLIENT_PREFIX);
        assert!(!PeerId::generate(true, &mut fill).0.windows(2).any(|w| w == b"SX"));
    }
    #[test]
    fn compact_round_trip() {
        let a = PeerAddr { ip: Ip::V4([10, 0, 2, 15]), port: 6881 };
        assert_eq!(PeerAddr::parse_compact(&a.compact(), false), alloc::vec![a]);
        let b = PeerAddr { ip: Ip::V6([1; 16]), port: 51413 };
        assert_eq!(PeerAddr::parse_compact(&b.compact(), true), alloc::vec![b]);
    }
}
