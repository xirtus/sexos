// SPDX-License-Identifier: GPL-3.0-only
//! Local Service Discovery (BEP 14): finding peers on the same network.
//!
//! Clients multicast a small HTTP-like `BT-SEARCH` message to `239.192.152.143:6771`
//! (IPv4) and `[ff15::efc0:988f]:6771` (IPv6) naming the torrents they have and the port
//! they listen on. Everyone in the group hears it and connects to the sender's address
//! with that port. A random `cookie` lets us recognise (and drop) our own messages when
//! the multicast loops back.
//!
//! [`Lsd`] holds the torrents to advertise. [`Lsd::on_tick`] returns the datagrams to
//! send: each torrent at most once per 5 minutes, and at most one message per minute per
//! group (several torrents share one message, as the BEP allows), so a big library does
//! not flood the network. [`Lsd::on_packet`] turns a received datagram into peers.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use crate::{from_hex, to_hex, Ip, Millis, PeerAddr};

/// The LSD port.
pub const PORT: u16 = 6771;
/// The IPv4 multicast group.
pub const GROUP_V4: PeerAddr = PeerAddr { ip: Ip::V4([239, 192, 152, 143]), port: PORT };
/// The IPv6 multicast group, `ff15::efc0:988f`.
pub const GROUP_V6: PeerAddr = PeerAddr { ip: Ip::V6([0xff, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xef, 0xc0, 0x98, 0x8f]), port: PORT };
/// A torrent is announced at most this often.
pub const TORRENT_INTERVAL: Millis = 5 * 60_000;
/// At most one message per group this often.
pub const SEND_INTERVAL: Millis = 60_000;
/// Info-hashes in one message (keeps it well under one MTU).
pub const MAX_HASHES_PER_MESSAGE: usize = 16;
/// Datagrams bigger than this are ignored.
pub const MAX_MESSAGE: usize = 2048;

/// A decoded `BT-SEARCH` message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Message {
    pub port: u16,
    pub info_hashes: Vec<[u8; 20]>,
    pub cookie: Option<String>,
}

/// Encodes a `BT-SEARCH` message for the IPv4 or IPv6 group.
pub fn encode(v6: bool, port: u16, info_hashes: &[[u8; 20]], cookie: Option<&str>) -> Vec<u8> {
    let mut s = String::from("BT-SEARCH * HTTP/1.1\r\n");
    let _ = write!(s, "Host: {}\r\n", if v6 { "[ff15::efc0:988f]:6771" } else { "239.192.152.143:6771" });
    let _ = write!(s, "Port: {}\r\n", port);
    for h in info_hashes.iter().take(MAX_HASHES_PER_MESSAGE) { let _ = write!(s, "Infohash: {}\r\n", to_hex(h)); }
    if let Some(c) = cookie { let _ = write!(s, "cookie: {}\r\n", c); }
    s.push_str("\r\n\r\n");
    s.into_bytes()
}

/// Decodes a `BT-SEARCH` message. Header names are case-insensitive; unknown headers are
/// skipped; at least one valid info-hash and a non-zero port are required.
pub fn decode(bytes: &[u8]) -> Option<Message> {
    if bytes.len() > MAX_MESSAGE { return None; }
    let text = core::str::from_utf8(bytes).ok()?;
    let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    if lines.next()?.trim() != "BT-SEARCH * HTTP/1.1" { return None; }
    let mut m = Message { port: 0, info_hashes: Vec::new(), cookie: None };
    for line in lines.take(64) {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim();
        if k.eq_ignore_ascii_case("port") {
            m.port = v.parse::<u16>().ok()?;
        } else if k.eq_ignore_ascii_case("infohash") {
            if let Some(h) = from_hex::<20>(v) {
                if !m.info_hashes.contains(&h) && m.info_hashes.len() < MAX_HASHES_PER_MESSAGE * 4 { m.info_hashes.push(h); }
            }
        } else if k.eq_ignore_ascii_case("cookie") && v.len() <= 64 {
            m.cookie = Some(String::from(v));
        }
    }
    if m.port == 0 || m.info_hashes.is_empty() { return None; }
    Some(m)
}

/// A peer heard on the local network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Found { pub info_hash: [u8; 20], pub peer: PeerAddr }

/// The LSD state for one listening port.
#[derive(Clone, Debug)]
pub struct Lsd {
    port: u16,
    cookie: String,
    v4: bool,
    v6: bool,
    /// Torrent → when it was last announced.
    torrents: BTreeMap<[u8; 20], Option<Millis>>,
    last_send: Option<Millis>,
}

impl Lsd {
    /// `port`: our peer port. `v4`/`v6`: which groups to use. `random` makes the cookie.
    pub fn new(port: u16, v4: bool, v6: bool, random: &mut dyn FnMut(&mut [u8])) -> Self {
        let mut c = [0u8; 8];
        random(&mut c);
        Lsd { port, cookie: to_hex(&c), v4, v6, torrents: BTreeMap::new(), last_send: None }
    }
    /// Our cookie (sent in every message).
    pub fn cookie(&self) -> &str { &self.cookie }
    /// Advertise a torrent (announced on the next allowed tick).
    pub fn add(&mut self, info_hash: [u8; 20]) { self.torrents.entry(info_hash).or_insert(None); }
    pub fn remove(&mut self, info_hash: &[u8; 20]) { self.torrents.remove(info_hash); }
    /// The listen port changed: everything is announced again.
    pub fn set_port(&mut self, port: u16) {
        self.port = port;
        for v in self.torrents.values_mut() { *v = None; }
    }

    /// When [`on_tick`](Self::on_tick) next has something to send.
    pub fn next_wakeup(&self) -> Option<Millis> {
        let due = self.torrents.values().map(|t| t.map_or(0, |t| t + TORRENT_INTERVAL)).min()?;
        Some(match self.last_send { Some(l) => due.max(l + SEND_INTERVAL), None => due })
    }

    /// The datagrams to send now: `(group address, bytes)`.
    pub fn on_tick(&mut self, now: Millis) -> Vec<(PeerAddr, Vec<u8>)> {
        let mut out = Vec::new();
        if self.port == 0 || matches!(self.last_send, Some(l) if now < l + SEND_INTERVAL) { return out; }
        let mut due: Vec<([u8; 20], Millis)> = self.torrents.iter().filter_map(|(h, t)| match t { None => Some((*h, 0)), Some(t) if now >= t + TORRENT_INTERVAL => Some((*h, *t)), _ => None }).collect();
        if due.is_empty() { return out; }
        due.sort_by_key(|&(_, t)| t);
        due.truncate(MAX_HASHES_PER_MESSAGE);
        let hashes: Vec<[u8; 20]> = due.iter().map(|d| d.0).collect();
        for h in &hashes { self.torrents.insert(*h, Some(now)); }
        self.last_send = Some(now);
        if self.v4 { out.push((GROUP_V4, encode(false, self.port, &hashes, Some(&self.cookie)))); }
        if self.v6 { out.push((GROUP_V6, encode(true, self.port, &hashes, Some(&self.cookie)))); }
        out
    }

    /// A datagram arrived on the LSD socket from `from`. Returns the peers it names for
    /// torrents we advertise (our own messages are dropped by cookie).
    pub fn on_packet(&self, from: PeerAddr, bytes: &[u8]) -> Vec<Found> {
        let Some(m) = decode(bytes) else { return Vec::new() };
        if m.cookie.as_deref() == Some(self.cookie.as_str()) { return Vec::new(); }
        let peer = PeerAddr { ip: from.ip, port: m.port };
        m.info_hashes.into_iter().filter(|h| self.torrents.contains_key(h)).map(|info_hash| Found { info_hash, peer }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn rnd(v: u8) -> impl FnMut(&mut [u8]) { move |b: &mut [u8]| b.fill(v) }

    #[test]
    fn golden_message() {
        let m = encode(false, 6881, &[[0xab; 20]], Some("c00k1e"));
        let expect = alloc::format!("BT-SEARCH * HTTP/1.1\r\nHost: 239.192.152.143:6771\r\nPort: 6881\r\nInfohash: {}\r\ncookie: c00k1e\r\n\r\n\r\n", "ab".repeat(20));
        assert_eq!(core::str::from_utf8(&m).unwrap(), expect);
        let m6 = encode(true, 1, &[[1; 20]], None);
        assert!(core::str::from_utf8(&m6).unwrap().contains("Host: [ff15::efc0:988f]:6771\r\n"));
        assert_eq!(decode(&m), Some(Message { port: 6881, info_hashes: vec![[0xab; 20]], cookie: Some("c00k1e".into()) }));
    }

    #[test]
    fn decode_variants_and_hostile() {
        let m = decode(b"BT-SEARCH * HTTP/1.1\nHOST: x\nport:  51413 \ninfohash: ABABABABABABABABABABABABABABABABABABABAB\nInfohash: zz\nInfohash: 0101010101010101010101010101010101010101\n\n").unwrap();
        assert_eq!(m.port, 51413);
        assert_eq!(m.info_hashes, vec![[0xab; 20], [1; 20]]);
        for bad in [&b""[..], b"GET / HTTP/1.1\r\nPort: 1\r\n", b"BT-SEARCH * HTTP/1.1\r\nPort: 0\r\nInfohash: 0101010101010101010101010101010101010101\r\n",
            b"BT-SEARCH * HTTP/1.1\r\nPort: 99999\r\nInfohash: 0101010101010101010101010101010101010101\r\n", b"BT-SEARCH * HTTP/1.1\r\nPort: 5\r\n", b"\xff\xfe"] {
            assert_eq!(decode(bad), None);
        }
        assert_eq!(decode(&vec![b'a'; MAX_MESSAGE + 1]), None);
    }

    #[test]
    fn rate_limits_and_cookie() {
        let mut a = Lsd::new(6881, true, true, &mut rnd(1));
        let mut b = Lsd::new(7000, true, false, &mut rnd(2));
        a.add([5; 20]);
        b.add([5; 20]);
        let out = a.on_tick(0);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, GROUP_V4);
        assert_eq!(out[1].0, GROUP_V6);
        // We ignore our own echo; the other side finds us.
        let me = PeerAddr { ip: Ip::V4([192, 168, 1, 2]), port: PORT };
        assert!(a.on_packet(me, &out[0].1).is_empty());
        assert_eq!(b.on_packet(me, &out[0].1), vec![Found { info_hash: [5; 20], peer: PeerAddr { ip: Ip::V4([192, 168, 1, 2]), port: 6881 } }]);
        // Unknown torrents are not reported.
        let other = encode(false, 1, &[[9; 20]], None);
        assert!(b.on_packet(me, &other).is_empty());
        // Nothing for 5 minutes, even when a new torrent shows up (global 1-minute limit).
        assert!(a.on_tick(1_000).is_empty());
        a.add([6; 20]);
        assert!(a.on_tick(59_999).is_empty());
        assert_eq!(a.next_wakeup(), Some(60_000));
        let out = a.on_tick(60_000);
        assert_eq!(decode(&out[0].1).unwrap().info_hashes, vec![[6; 20]]);
        assert!(a.on_tick(200_000).is_empty());
        let out = a.on_tick(300_000);
        assert_eq!(decode(&out[0].1).unwrap().info_hashes, vec![[5; 20]]);
        let out = a.on_tick(360_000);
        assert_eq!(decode(&out[0].1).unwrap().info_hashes, vec![[6; 20]]);
    }

    #[test]
    fn many_torrents_batched() {
        let mut a = Lsd::new(1, true, false, &mut rnd(3));
        for i in 0..40u8 { a.add([i; 20]); }
        let n1 = decode(&a.on_tick(0)[0].1).unwrap().info_hashes.len();
        let n2 = decode(&a.on_tick(60_000)[0].1).unwrap().info_hashes.len();
        let n3 = decode(&a.on_tick(120_000)[0].1).unwrap().info_hashes.len();
        assert_eq!((n1, n2, n3), (16, 16, 8));
        assert!(a.on_tick(180_000).is_empty());
        assert!(a.on_tick(0).len() <= 1);
    }
}
