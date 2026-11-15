// SPDX-License-Identifier: GPL-3.0-only
//! Magnet links (BEP 9, BEP 53): a torrent named by its info-hash, without the .torrent.
//!
//! `magnet:?xt=urn:btih:<40 hex or 32 base32>&xt=urn:btmh:1220<64 hex>&dn=<name>&tr=<url>
//! &ws=<web seed>&x.pe=<host:port>&so=0,2,4-6&xl=<bytes>`
//!
//! - `xt=urn:btih:` is the v1 (SHA-1) info-hash, `xt=urn:btmh:` the v2 one as a
//!   multihash (`12` = SHA-256, `20` = 32 bytes). Both together name a hybrid.
//! - `tr`, `ws` and `x.pe` may repeat. `x.pe` peers are `ip:port`, `[ipv6]:port` or
//!   `host:port`.
//! - `so` ("select only", BEP 53) lists file indices and inclusive ranges.
//!
//! Values are percent-decoded (`+` means a space only in `dn`); unknown parameters are
//! ignored. [`Magnet::to_uri`] writes a link back, percent-encoding everything except
//! the unreserved characters, so parse and build round trip.
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

use crate::metainfo::Metainfo;
use crate::{from_hex, to_hex, InfoHashV1, InfoHashV2, InfoHashes, Ip, PeerAddr};

/// Longest magnet link accepted.
pub const MAX_URI_LEN: usize = 64 * 1024;
/// Most parameters read from one link.
pub const MAX_PARAMS: usize = 512;
/// Most `so` entries.
pub const MAX_SELECT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Does not start with `magnet:?`.
    NotMagnet,
    TooLong,
    /// No usable `xt` with a BitTorrent hash.
    NoInfoHash,
    /// Two different hashes of the same kind.
    ConflictingHash,
    /// A malformed `xt`, `so` or `xl`.
    BadValue(&'static str),
}

/// A peer from `x.pe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MagnetPeer {
    Addr(PeerAddr),
    /// A hostname to resolve, and a port.
    Host(String, u16),
}

/// A parsed magnet link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Magnet {
    pub info_hashes: InfoHashes,
    /// `dn`: a name to show until the metadata arrives.
    pub display_name: Option<String>,
    /// `tr`, in order, without duplicates.
    pub trackers: Vec<String>,
    /// `ws` web seeds.
    pub web_seeds: Vec<String>,
    /// `x.pe` peers to fetch the metadata from directly.
    pub peers: Vec<MagnetPeer>,
    /// `so` file selection as inclusive ranges; empty means everything.
    pub select_only: Vec<(u32, u32)>,
    /// `xl`: the total size, if given.
    pub exact_length: Option<u64>,
}

impl Magnet {
    /// A link with just the hashes.
    pub fn new(info_hashes: InfoHashes) -> Magnet {
        Magnet { info_hashes, display_name: None, trackers: Vec::new(), web_seeds: Vec::new(), peers: Vec::new(), select_only: Vec::new(), exact_length: None }
    }

    /// The link for a torrent: hashes, name, all trackers, web seeds and size.
    pub fn from_metainfo(m: &Metainfo) -> Magnet {
        let mut g = Magnet::new(m.info_hashes);
        g.display_name = Some(m.name.clone());
        for t in m.trackers.iter().flatten() { if !g.trackers.contains(t) { g.trackers.push(t.clone()); } }
        g.web_seeds = m.web_seeds.clone();
        g.exact_length = Some(m.total_length);
        g
    }

    /// Parses a magnet URI.
    pub fn parse(uri: &str) -> Result<Magnet, Error> {
        if uri.len() > MAX_URI_LEN { return Err(Error::TooLong); }
        let uri = uri.trim();
        let query = strip_prefix_ci(uri, "magnet:?").ok_or(Error::NotMagnet)?;
        let (mut v1, mut v2): (Option<InfoHashV1>, Option<InfoHashV2>) = (None, None);
        let mut m = Magnet::new(InfoHashes::V1(InfoHashV1::default()));
        for param in query.split('&').filter(|p| !p.is_empty()).take(MAX_PARAMS) {
            let (key, raw) = param.split_once('=').unwrap_or((param, ""));
            let key = key.to_ascii_lowercase();
            let base = key.split('.').next().unwrap_or("");
            match key.as_str() {
                _ if base == "xt" => {
                    let v = decode(raw, false);
                    if let Some(h) = strip_prefix_ci(&v, "urn:btih:") {
                        let h = parse_btih(h).ok_or(Error::BadValue("xt"))?;
                        if v1.is_some_and(|o| o != h) { return Err(Error::ConflictingHash); }
                        v1 = Some(h);
                    } else if let Some(h) = strip_prefix_ci(&v, "urn:btmh:") {
                        let h = strip_prefix_ci(h, "1220").and_then(from_hex::<32>).ok_or(Error::BadValue("xt"))?;
                        if v2.is_some_and(|o| o.0 != h) { return Err(Error::ConflictingHash); }
                        v2 = Some(InfoHashV2(h));
                    }
                }
                "dn" => m.display_name = Some(decode(raw, true)),
                "tr" => push_unique(&mut m.trackers, decode(raw, false)),
                "ws" => push_unique(&mut m.web_seeds, decode(raw, false)),
                "x.pe" => {
                    if let Some(p) = parse_peer(&decode(raw, false)) { if !m.peers.contains(&p) { m.peers.push(p); } }
                }
                "so" => m.select_only = parse_select(&decode(raw, false)).ok_or(Error::BadValue("so"))?,
                "xl" => m.exact_length = Some(decode(raw, false).parse().map_err(|_| Error::BadValue("xl"))?),
                _ => {}
            }
        }
        m.info_hashes = match (v1, v2) {
            (Some(a), Some(b)) => InfoHashes::Hybrid(a, b),
            (Some(a), None) => InfoHashes::V1(a),
            (None, Some(b)) => InfoHashes::V2(b),
            (None, None) => return Err(Error::NoInfoHash),
        };
        Ok(m)
    }

    /// Whether file `index` is selected (`so` absent selects all).
    pub fn is_selected(&self, index: u32) -> bool {
        self.select_only.is_empty() || self.select_only.iter().any(|&(a, b)| a <= index && index <= b)
    }

    /// Writes the link.
    pub fn to_uri(&self) -> String {
        let mut s = String::from("magnet:?");
        let mut sep = "";
        let mut add = |s: &mut String, k: &str, v: &str| { s.push_str(sep); s.push_str(k); s.push('='); encode_into(s, v); sep = "&"; };
        if let Some(a) = self.info_hashes.v1() { add(&mut s, "xt", &(String::from("urn:btih:") + &to_hex(&a.0))); }
        if let Some(b) = self.info_hashes.v2() { add(&mut s, "xt", &(String::from("urn:btmh:1220") + &to_hex(&b.0))); }
        if let Some(n) = &self.display_name { add(&mut s, "dn", n); }
        if let Some(l) = self.exact_length { add(&mut s, "xl", &l.to_string()); }
        for t in &self.trackers { add(&mut s, "tr", t); }
        for w in &self.web_seeds { add(&mut s, "ws", w); }
        for p in &self.peers { add(&mut s, "x.pe", &format_peer(p)); }
        if !self.select_only.is_empty() {
            let mut v = String::new();
            for (i, &(a, b)) in self.select_only.iter().enumerate() {
                if i > 0 { v.push(','); }
                if a == b { let _ = write!(v, "{}", a); } else { let _ = write!(v, "{}-{}", a, b); }
            }
            add(&mut s, "so", &v);
        }
        s
    }
}

fn push_unique(list: &mut Vec<String>, v: String) {
    if !v.is_empty() && !list.contains(&v) { list.push(v); }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) { s.get(prefix.len()..) } else { None }
}

/// 40 hex characters or 32 base32 (RFC 4648) characters.
fn parse_btih(s: &str) -> Option<InfoHashV1> {
    match s.len() {
        40 => from_hex::<20>(s).map(InfoHashV1),
        32 => {
            let mut out = [0u8; 20];
            let (mut acc, mut bits, mut n) = (0u64, 0u32, 0usize);
            for c in s.bytes() {
                let v = match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a', b'2'..=b'7' => c - b'2' + 26, _ => return None };
                acc = (acc << 5) | v as u64;
                bits += 5;
                if bits >= 8 { bits -= 8; *out.get_mut(n)? = (acc >> bits) as u8; n += 1; acc &= (1 << bits) - 1; }
            }
            Some(InfoHashV1(out))
        }
        _ => None,
    }
}

/// Percent-decodes; malformed escapes are kept as written; invalid UTF-8 is replaced.
pub fn decode(s: &str, plus_is_space: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) { out.push((h * 16 + l) as u8); i += 3; continue; }
                out.push(b'%');
            }
            b'+' if plus_is_space => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes everything but `A-Z a-z 0-9 - . _ ~`.
pub fn encode_into(out: &mut String, s: &str) {
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || b"-._~".contains(&c) { out.push(c as char); } else { let _ = write!(out, "%{:02X}", c); }
    }
}

/// `0,2,4-6`: sorted, merged inclusive ranges.
fn parse_select(s: &str) -> Option<Vec<(u32, u32)>> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if out.len() >= MAX_SELECT { return None; }
        let (a, b) = match part.split_once('-') { Some((a, b)) => (a.trim().parse().ok()?, b.trim().parse().ok()?), None => { let n = part.parse().ok()?; (n, n) } };
        if a > b { return None; }
        out.push((a, b));
    }
    out.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(out.len());
    for (a, b) in out {
        match merged.last_mut() {
            Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    Some(merged)
}

/// `1.2.3.4:5`, `[::1]:5` or `host:5`.
pub fn parse_peer(s: &str) -> Option<MagnetPeer> {
    if let Some(rest) = s.strip_prefix('[') {
        let (ip, port) = rest.split_once("]:")?;
        return Some(MagnetPeer::Addr(PeerAddr { ip: Ip::V6(parse_ipv6(ip)?), port: port.parse().ok()? }));
    }
    let (host, port) = s.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    if let Some(v4) = parse_ipv4(host) { return Some(MagnetPeer::Addr(PeerAddr { ip: Ip::V4(v4), port })); }
    let ok = !host.is_empty() && host.len() <= 253 && host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.');
    ok.then(|| MagnetPeer::Host(host.to_string(), port))
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for o in out.iter_mut() {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) { return None; }
        *o = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

fn parse_ipv6(s: &str) -> Option<[u8; 16]> {
    fn groups(s: &str) -> Option<Vec<u16>> {
        if s.is_empty() { return Some(Vec::new()); }
        s.split(':').map(|g| if g.is_empty() || g.len() > 4 { None } else { u16::from_str_radix(g, 16).ok() }).collect()
    }
    let (head, tail) = match s.split_once("::") { Some((h, t)) => (groups(h)?, Some(groups(t)?)), None => (groups(s)?, None) };
    let mut all = head;
    match tail {
        Some(t) => {
            if all.len() + t.len() > 7 { return None; }
            all.resize(8 - t.len(), 0);
            all.extend(t);
        }
        None if all.len() != 8 => return None,
        None => {}
    }
    let mut out = [0u8; 16];
    for (i, g) in all.iter().enumerate() { out[2 * i..2 * i + 2].copy_from_slice(&g.to_be_bytes()); }
    Some(out)
}

fn format_peer(p: &MagnetPeer) -> String {
    let mut s = String::new();
    match p {
        MagnetPeer::Host(h, port) => { let _ = write!(s, "{}:{}", h, port); }
        MagnetPeer::Addr(PeerAddr { ip: Ip::V4(a), port }) => { let _ = write!(s, "{}.{}.{}.{}:{}", a[0], a[1], a[2], a[3], port); }
        MagnetPeer::Addr(PeerAddr { ip: Ip::V6(a), port }) => {
            s.push('[');
            for i in 0..8 { if i > 0 { s.push(':'); } let _ = write!(s, "{:x}", u16::from_be_bytes([a[2 * i], a[2 * i + 1]])); }
            let _ = write!(s, "]:{}", port);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const HEX: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
    const V2HEX: &str = "d8dd32ac93357c368556af3ac1d95c9d76bd0dff6fa9833ecdac3d53134efabb";

    #[test]
    fn v1_hex_and_base32_agree() {
        let a = Magnet::parse(&(String::from("magnet:?xt=urn:btih:") + HEX)).unwrap();
        // The same hash in base32 (RFC 4648), lower and upper case.
        let b32 = "YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKEK";
        let b = Magnet::parse(&(String::from("magnet:?xt=urn:btih:") + b32)).unwrap();
        let c = Magnet::parse(&(String::from("MAGNET:?XT=urn:BTIH:") + &b32.to_lowercase())).unwrap();
        assert_eq!(a.info_hashes, b.info_hashes);
        assert_eq!(b.info_hashes, c.info_hashes);
        assert_eq!(to_hex(&a.info_hashes.v1().unwrap().0), HEX);
    }

    #[test]
    fn full_link_round_trip() {
        let uri = String::from("magnet:?xt=urn:btih:") + HEX + "&xt=urn:btmh:1220" + V2HEX
            + "&dn=Big+Buck%20Bunny%E2%9C%93&tr=udp%3A%2F%2Ftracker.example%3A1337%2Fannounce&tr=http://t2/a?x=1"
            + "&tr=udp%3A%2F%2Ftracker.example%3A1337%2Fannounce&ws=https%3A%2F%2Fseed.example%2F&x.pe=10.0.0.2:6881"
            + "&x.pe=[2001:db8::1]:51413&x.pe=peer.example:7000&so=4-6,0,2,5,7&xl=1234567&foo=bar";
        let m = Magnet::parse(&uri).unwrap();
        assert!(matches!(m.info_hashes, InfoHashes::Hybrid(..)));
        assert_eq!(to_hex(&m.info_hashes.v2().unwrap().0), V2HEX);
        assert_eq!(m.display_name.as_deref(), Some("Big Buck Bunny\u{2713}"));
        assert_eq!(m.trackers, vec![String::from("udp://tracker.example:1337/announce"), String::from("http://t2/a?x=1")]);
        assert_eq!(m.web_seeds, vec![String::from("https://seed.example/")]);
        let v6 = parse_ipv6("2001:db8::1").unwrap();
        assert_eq!(v6[0..2], [0x20, 0x01]);
        assert_eq!(v6[15], 1);
        assert_eq!(m.peers, vec![
            MagnetPeer::Addr(PeerAddr { ip: Ip::V4([10, 0, 0, 2]), port: 6881 }),
            MagnetPeer::Addr(PeerAddr { ip: Ip::V6(v6), port: 51413 }),
            MagnetPeer::Host("peer.example".into(), 7000),
        ]);
        assert_eq!(m.select_only, vec![(0, 0), (2, 2), (4, 7)]);
        assert!(m.is_selected(5) && m.is_selected(0) && !m.is_selected(1) && !m.is_selected(8));
        assert_eq!(m.exact_length, Some(1234567));
        let again = Magnet::parse(&m.to_uri()).unwrap();
        assert_eq!(again, m);
        assert!(m.to_uri().contains("so=0%2C2%2C4-7"));
    }

    #[test]
    fn from_torrent_and_back() {
        use crate::create::{Creator, FileSpec, Options, Version};
        let mut o = Options::new("movie.mkv", Version::Hybrid);
        o.trackers = vec![vec!["udp://a:1/announce".into()], vec!["http://b/announce".into()]];
        o.web_seeds = vec!["https://ws/".into()];
        let mut c = Creator::new(o, vec![FileSpec::new(&["movie.mkv"], 50_000)]).unwrap();
        c.feed(&[3u8; 50_000]).unwrap();
        let m = Metainfo::parse(&c.finish().unwrap()).unwrap();
        let g = Magnet::from_metainfo(&m);
        let back = Magnet::parse(&g.to_uri()).unwrap();
        assert_eq!(back.info_hashes, m.info_hashes);
        assert_eq!(back.trackers, vec![String::from("udp://a:1/announce"), String::from("http://b/announce")]);
        assert_eq!(back.display_name.as_deref(), Some("movie.mkv"));
        assert_eq!(back.exact_length, Some(50_000));
        // The metadata a peer would send for this magnet names the same torrent.
        assert_eq!(Metainfo::from_info_bytes(&m.info_bytes).unwrap().info_hashes, back.info_hashes);
    }

    #[test]
    fn v2_only() {
        let m = Magnet::parse(&(String::from("magnet:?xt=urn:btmh:1220") + V2HEX + "&dn=x")).unwrap();
        assert!(matches!(m.info_hashes, InfoHashes::V2(_)));
        assert_eq!(m.to_uri(), String::from("magnet:?xt=urn%3Abtmh%3A1220") + V2HEX + "&dn=x");
    }

    #[test]
    fn hostile_links() {
        assert_eq!(Magnet::parse("http://x"), Err(Error::NotMagnet));
        assert_eq!(Magnet::parse("magnet:?dn=x"), Err(Error::NoInfoHash));
        assert_eq!(Magnet::parse("magnet:?xt=urn:btih:zz"), Err(Error::BadValue("xt")));
        assert_eq!(Magnet::parse("magnet:?xt=urn:btmh:1114aaaa"), Err(Error::BadValue("xt")));
        let two = String::from("magnet:?xt=urn:btih:") + HEX + "&xt=urn:btih:" + &"0".repeat(40);
        assert_eq!(Magnet::parse(&two), Err(Error::ConflictingHash));
        let ok = String::from("magnet:?xt=urn:btih:") + HEX;
        assert_eq!(Magnet::parse(&(ok.clone() + "&so=5-2")), Err(Error::BadValue("so")));
        assert_eq!(Magnet::parse(&(ok.clone() + "&so=1,a")), Err(Error::BadValue("so")));
        assert_eq!(Magnet::parse(&(ok.clone() + "&xl=-1")), Err(Error::BadValue("xl")));
        assert_eq!(Magnet::parse(&"a".repeat(MAX_URI_LEN + 1)), Err(Error::TooLong));
        // Bad escapes and peers are tolerated, not trusted.
        let m = Magnet::parse(&(ok.clone() + "&dn=%&dn=%4&dn=%zz%C3&x.pe=[::1&x.pe=1.2.3.4:99999&x.pe=bad host:1&x.pe=::1:5")).unwrap();
        assert_eq!(m.display_name.as_deref(), Some("%zz\u{fffd}"));
        assert!(m.peers.is_empty());
        assert_eq!(decode("%", false), "%");
        assert_eq!(decode("%4", false), "%4");
        assert_eq!(decode("a%41", false), "aA");
        assert_eq!(parse_ipv6("1:2:3:4:5:6:7:8:9"), None);
        assert_eq!(parse_ipv6("1::2::3"), None);
        assert_eq!(parse_ipv6("::").unwrap(), [0; 16]);
        assert_eq!(parse_ipv4("1.2.3"), None);
        assert_eq!(parse_ipv4("256.1.1.1"), None);
        // Byte noise never panics.
        let mut x: u32 = 7;
        for _ in 0..3000 {
            let mut s = ok.clone() + "&";
            for _ in 0..20 { x ^= x << 13; x ^= x >> 17; x ^= x << 5; s.push(b"%&=:[]-,.0123456789abcxtsodnpe+"[x as usize % 31] as char); }
            let _ = Magnet::parse(&s);
        }
    }
}
