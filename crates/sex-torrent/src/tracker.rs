// SPDX-License-Identifier: GPL-3.0-only
//! Trackers: the servers a torrent names that hand out lists of other peers.
//!
//! Three layers, all without I/O:
//! - **URLs and HTTP(S) trackers** (BEP 3, 23, 7, 24, 31, 48): [`parse_url`] splits an
//!   announce URL (http, https, udp; IPv6 literals in brackets) into host, port and
//!   path; [`announce_request`] and [`scrape_request`] build the request target
//!   (`/announce?info_hash=...`) that the program sends with its own HTTP client; and
//!   [`parse_announce`] / [`parse_scrape`] read the bencoded body that came back
//!   (failure reason, warning, intervals, swarm counts, peers as a list of dictionaries
//!   or compact strings, `peers6`).
//! - **UDP trackers** (BEP 15, BEP 41 URL data): [`UdpTracker`] is one tracker's client
//!   state machine. It connects (magic `0x41727101980`), keeps the connection id for a
//!   minute, announces or scrapes, and retransmits after 15·2ⁿ seconds (n up to 8). The
//!   program owns the socket: it sends what this returns and feeds back what arrives.
//! - **Tiers** (BEP 12): [`TrackerList`] keeps a torrent's trackers in tiers, shuffles
//!   each tier with the caller's randomness, tries trackers in order, moves one that works
//!   to the front of its tier, keeps the announce interval, backs off after failures, and
//!   sends `started`, `completed` (once) and `stopped` at the right times.
//!
//! Anonymous mode ([`AnnounceParams::anonymous`]) leaves out every address hint
//! (`ipv4`, `ipv6`) so the tracker learns only what the socket shows, and expects a fresh
//! random `key` (the program also generates the peer id with [`crate::PeerId::generate`]
//! in anonymous mode). A v2-only torrent announces with the truncated SHA-256 hash; a
//! hybrid announces both hashes ([`announce_hashes`]).
//!
//! BEP 17 note: old "httpseeds" (Hoffman-style seeds that answer `?info_hash=&piece=`)
//! are obsolete and not supported; BEP 19 web seeds live in [`crate::webseed`].
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use core::fmt::Write as _;

use crate::bencode::{self, Value};
use crate::{InfoHashes, Ip, Millis, PeerAddr};

/// URLs longer than this are refused.
pub const MAX_URL: usize = 4096;
/// At most this many peers are taken from one response (each of `peers` and `peers6`).
pub const MAX_PEERS: usize = 2000;
/// At most this many torrents in one scrape request or response.
pub const MAX_SCRAPE: usize = 74;
/// Strings from trackers (failure reason, warning) are cut to this many bytes.
pub const MAX_TEXT: usize = 1024;

// ---------------------------------------------------------------------------------
// URLs, IP text, percent-encoding
// ---------------------------------------------------------------------------------

/// The transport a tracker URL names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme { Http, Https, Udp }

/// A parsed tracker (or web seed) URL.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrackerUrl {
    pub scheme: Scheme,
    /// A host name or an IP literal, without brackets.
    pub host: String,
    pub port: u16,
    /// Starts with `/`; includes the query (`?...`) if the URL had one, never a fragment.
    pub path_and_query: String,
}

impl TrackerUrl {
    /// The host as an IP address, when it is a literal.
    pub fn host_ip(&self) -> Option<Ip> { parse_ip(&self.host) }
    /// Whether HTTP goes over TLS.
    pub fn tls(&self) -> bool { self.scheme == Scheme::Https }
    /// The host as written in a URL or `Host:` header (`[v6]` in brackets).
    pub fn host_for_header(&self) -> String {
        if matches!(self.host_ip(), Some(Ip::V6(_))) { format!("[{}]", self.host) } else { self.host.clone() }
    }
    /// The URL back as text.
    pub fn to_url(&self) -> String {
        let s = match self.scheme { Scheme::Http => "http", Scheme::Https => "https", Scheme::Udp => "udp" };
        format!("{}://{}:{}{}", s, self.host_for_header(), self.port, self.path_and_query)
    }
}

/// Why a URL could not be used.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UrlError {
    /// Not a URL we understand (scheme, host, port or characters).
    Invalid,
    /// The tracker's URL does not allow scraping (BEP 48: last path part must start with
    /// `announce`).
    NoScrape,
    /// The request is not possible over this transport (e.g. HTTP request for a udp URL).
    WrongScheme,
}

/// Parses `http://`, `https://` and `udp://` URLs. HTTP defaults to port 80, HTTPS to
/// 443; UDP needs an explicit port. User info (`user@`) is refused, fragments dropped.
pub fn parse_url(url: &str) -> Option<TrackerUrl> {
    let url = url.trim();
    if url.len() > MAX_URL || url.bytes().any(|b| b <= b' ' || b >= 0x7f) { return None; }
    let sep = url.find("://")?;
    let scheme = match url[..sep].to_ascii_lowercase().as_str() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        "udp" => Scheme::Udp,
        _ => return None,
    };
    let rest = &url[sep + 3..];
    let rest = rest.split('#').next().unwrap_or("");
    let end = rest.find(|c| c == '/' || c == '?').unwrap_or(rest.len());
    let (auth, path) = rest.split_at(end);
    if auth.contains('@') || auth.is_empty() { return None; }
    let (host, port) = if let Some(r) = auth.strip_prefix('[') {
        let close = r.find(']')?;
        let h = &r[..close];
        if !matches!(parse_ip(h), Some(Ip::V6(_))) { return None; }
        let after = &r[close + 1..];
        let port = if after.is_empty() { None } else { Some(after.strip_prefix(':')?) };
        (h, port)
    } else {
        match auth.rfind(':') {
            Some(i) => (&auth[..i], Some(&auth[i + 1..])),
            None => (auth, None),
        }
    };
    if host.is_empty() || host.len() > 253 { return None; }
    if !auth.starts_with('[') && !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_') {
        return None;
    }
    let port = match port {
        Some(p) => parse_decimal_u16(p).filter(|&p| p != 0)?,
        None => match scheme { Scheme::Http => 80, Scheme::Https => 443, Scheme::Udp => return None },
    };
    let path_and_query = if path.is_empty() { "/".to_string() } else if path.starts_with('?') { format!("/{}", path) } else { path.to_string() };
    Some(TrackerUrl { scheme, host: host.to_ascii_lowercase(), port, path_and_query })
}

fn parse_decimal_u16(s: &str) -> Option<u16> {
    if s.is_empty() || s.len() > 5 || !s.bytes().all(|b| b.is_ascii_digit()) { return None; }
    s.parse::<u32>().ok().and_then(|n| u16::try_from(n).ok())
}

/// Parses an IPv4 dotted quad (no leading zeros) or IPv6 text (RFC 4291: `::`, embedded
/// IPv4 tail; no zone ids).
pub fn parse_ip(s: &str) -> Option<Ip> {
    if s.contains(':') { parse_v6(s).map(Ip::V6) } else { parse_v4(s).map(Ip::V4) }
}

fn parse_v4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) { return None; }
        if part.len() > 1 && part.starts_with('0') { return None; }
        out[n] = part.parse::<u16>().ok().and_then(|v| u8::try_from(v).ok())?;
        n += 1;
    }
    if n == 4 { Some(out) } else { None }
}

fn parse_v6(s: &str) -> Option<[u8; 16]> {
    if s.len() > 45 { return None; }
    fn groups(part: &str, v4_ok: bool, out: &mut Vec<u16>) -> Option<()> {
        if part.is_empty() { return Some(()); }
        let pieces: Vec<&str> = part.split(':').collect();
        for (i, g) in pieces.iter().enumerate() {
            if v4_ok && i + 1 == pieces.len() && g.contains('.') {
                let a = parse_v4(g)?;
                out.push(u16::from_be_bytes([a[0], a[1]]));
                out.push(u16::from_be_bytes([a[2], a[3]]));
            } else {
                if g.is_empty() || g.len() > 4 { return None; }
                out.push(u16::from_str_radix(g, 16).ok()?);
            }
            if out.len() > 8 { return None; }
        }
        Some(())
    }
    let mut head = Vec::new();
    let mut tail = Vec::new();
    match s.find("::") {
        Some(i) => {
            let t = &s[i + 2..];
            if t.contains("::") { return None; }
            groups(&s[..i], false, &mut head)?;
            groups(t, true, &mut tail)?;
            if head.len() + tail.len() > 7 { return None; }
        }
        None => {
            groups(s, true, &mut head)?;
            if head.len() != 8 { return None; }
        }
    }
    let mut words = [0u16; 8];
    words[..head.len()].copy_from_slice(&head);
    words[8 - tail.len()..].copy_from_slice(&tail);
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() { out[2 * i..2 * i + 2].copy_from_slice(&w.to_be_bytes()); }
    Some(out)
}

/// Formats an IP as text: dotted quad, or RFC 5952 IPv6 (lower case, longest zero run
/// compressed).
pub fn fmt_ip(ip: &Ip) -> String {
    let mut s = String::new();
    match ip {
        Ip::V4(a) => { let _ = write!(s, "{}.{}.{}.{}", a[0], a[1], a[2], a[3]); }
        Ip::V6(a) => {
            let w: Vec<u16> = a.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            let (mut best, mut best_len, mut i) = (8, 0, 0);
            while i < 8 {
                if w[i] == 0 {
                    let start = i;
                    while i < 8 && w[i] == 0 { i += 1; }
                    if i - start > best_len { best = start; best_len = i - start; }
                } else { i += 1; }
            }
            if best_len < 2 { best = 8; }
            let mut i = 0;
            while i < 8 {
                if i == best {
                    s.push_str("::");
                    i += best_len;
                    continue;
                }
                if i > 0 && i != best + best_len { s.push(':'); }
                let _ = write!(s, "{:x}", w[i]);
                i += 1;
            }
        }
    }
    s
}

/// Percent-encodes raw bytes for a URL query or path: RFC 3986 unreserved characters
/// (`A-Z a-z 0-9 - . _ ~`) stay, every other byte becomes `%XX`.
pub fn url_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') { s.push(b as char); } else { let _ = write!(s, "%{:02X}", b); }
    }
    s
}

// ---------------------------------------------------------------------------------
// Announce parameters and HTTP requests
// ---------------------------------------------------------------------------------

/// The announce event (BEP 3). UDP codes are the BEP 15 numbers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Event {
    #[default]
    None,
    Completed,
    Started,
    Stopped,
}

impl Event {
    fn http(self) -> Option<&'static str> {
        match self { Event::None => None, Event::Completed => Some("completed"), Event::Started => Some("started"), Event::Stopped => Some("stopped") }
    }
    fn udp(self) -> u32 { match self { Event::None => 0, Event::Completed => 1, Event::Started => 2, Event::Stopped => 3 } }
}

/// What one announce says about us.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AnnounceParams {
    /// The 20 bytes the tracker knows the torrent by (see [`announce_hashes`]).
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub event: Event,
    /// How many peers we want; `None` lets the tracker choose (UDP sends -1).
    pub numwant: Option<u32>,
    /// A random number that stays the same for this torrent's session (lets the tracker
    /// recognise us across IP changes). In anonymous mode, pick it fresh per torrent.
    pub key: u32,
    /// The `tracker id` the tracker gave in an earlier response.
    pub tracker_id: Option<Vec<u8>>,
    /// Our address hints (BEP 7 `ipv4=` / `ipv6=`): sent unless anonymous.
    pub ipv4: Option<[u8; 4]>,
    pub ipv6: Option<[u8; 16]>,
    /// Anonymous mode: no address hints, no `supportcrypto` flag.
    pub anonymous: bool,
    /// Whether we speak protocol encryption (sends `supportcrypto=1`).
    pub supportcrypto: bool,
}

impl AnnounceParams {
    /// Parameters with the common fields filled and the rest empty.
    pub fn new(info_hash: [u8; 20], peer_id: [u8; 20], port: u16, key: u32) -> Self {
        AnnounceParams { info_hash, peer_id, port, uploaded: 0, downloaded: 0, left: 0, event: Event::None, numwant: None, key, tracker_id: None, ipv4: None, ipv6: None, anonymous: false, supportcrypto: false }
    }
}

/// The 20-byte hashes a torrent announces under: the v1 hash, the truncated v2 hash, or
/// both for a hybrid (announced separately, so v2-only peers find us too).
pub fn announce_hashes(ih: &InfoHashes) -> Vec<[u8; 20]> {
    match ih {
        InfoHashes::V1(a) => vec![a.0],
        InfoHashes::V2(b) => vec![b.truncated()],
        InfoHashes::Hybrid(a, b) => vec![a.0, b.truncated()],
    }
}

/// Where to send an HTTP request: the program connects to `host:port` (TLS if `tls`) and
/// sends `GET {path_and_query} HTTP/1.1` with `Host: {host_header}`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpTarget {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub path_and_query: String,
    /// `host` or `[v6]`, plus `:port` when not the scheme's default.
    pub host_header: String,
}

fn http_target(u: &TrackerUrl, path_and_query: String) -> HttpTarget {
    let default = if u.tls() { 443 } else { 80 };
    let host_header = if u.port == default { u.host_for_header() } else { format!("{}:{}", u.host_for_header(), u.port) };
    HttpTarget { host: u.host.clone(), port: u.port, tls: u.tls(), path_and_query, host_header }
}

fn push_param(q: &mut String, k: &str, v: &str) {
    q.push(if q.contains('?') { '&' } else { '?' });
    q.push_str(k);
    q.push('=');
    q.push_str(v);
}

/// Builds the HTTP announce request for `url` (BEP 3, 23 `compact=1`, 7 address hints).
pub fn announce_request(url: &str, p: &AnnounceParams) -> Result<HttpTarget, UrlError> {
    let u = parse_url(url).ok_or(UrlError::Invalid)?;
    if u.scheme == Scheme::Udp { return Err(UrlError::WrongScheme); }
    let mut q = u.path_and_query.clone();
    push_param(&mut q, "info_hash", &url_encode(&p.info_hash));
    push_param(&mut q, "peer_id", &url_encode(&p.peer_id));
    push_param(&mut q, "port", &format!("{}", p.port));
    push_param(&mut q, "uploaded", &format!("{}", p.uploaded));
    push_param(&mut q, "downloaded", &format!("{}", p.downloaded));
    push_param(&mut q, "left", &format!("{}", p.left));
    push_param(&mut q, "key", &format!("{:08X}", p.key));
    if let Some(e) = p.event.http() { push_param(&mut q, "event", e); }
    if let Some(n) = p.numwant { push_param(&mut q, "numwant", &format!("{}", n)); }
    push_param(&mut q, "compact", "1");
    push_param(&mut q, "no_peer_id", "1");
    if p.supportcrypto && !p.anonymous { push_param(&mut q, "supportcrypto", "1"); }
    if let Some(t) = &p.tracker_id { push_param(&mut q, "trackerid", &url_encode(t)); }
    if !p.anonymous {
        if let Some(a) = p.ipv4 { push_param(&mut q, "ipv4", &fmt_ip(&Ip::V4(a))); }
        if let Some(a) = p.ipv6 { push_param(&mut q, "ipv6", &url_encode(fmt_ip(&Ip::V6(a)).as_bytes())); }
    }
    Ok(http_target(&u, q))
}

/// The scrape URL path for an announce URL (BEP 48): the last path component must start
/// with `announce`, which becomes `scrape`.
pub fn scrape_path(path_and_query: &str) -> Option<String> {
    let (path, query) = match path_and_query.find('?') { Some(i) => path_and_query.split_at(i), None => (path_and_query, "") };
    let slash = path.rfind('/')?;
    let last = &path[slash + 1..];
    let rest = last.strip_prefix("announce")?;
    Some(format!("{}scrape{}{}", &path[..slash + 1], rest, query))
}

/// Builds the HTTP scrape request for up to [`MAX_SCRAPE`] torrents.
pub fn scrape_request(url: &str, hashes: &[[u8; 20]]) -> Result<HttpTarget, UrlError> {
    let u = parse_url(url).ok_or(UrlError::Invalid)?;
    if u.scheme == Scheme::Udp { return Err(UrlError::WrongScheme); }
    let mut q = scrape_path(&u.path_and_query).ok_or(UrlError::NoScrape)?;
    for h in hashes.iter().take(MAX_SCRAPE) { push_param(&mut q, "info_hash", &url_encode(h)); }
    Ok(http_target(&u, q))
}

// ---------------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------------

/// A successful announce.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AnnounceResponse {
    /// Seconds until the next regular announce.
    pub interval: Option<u32>,
    /// Seconds we must wait at least before announcing again.
    pub min_interval: Option<u32>,
    /// To send back as `trackerid` next time.
    pub tracker_id: Option<Vec<u8>>,
    /// Seeders.
    pub complete: Option<u32>,
    /// Leechers.
    pub incomplete: Option<u32>,
    /// Times the torrent was completed (non-standard, common).
    pub downloaded: Option<u32>,
    /// IPv4 and IPv6 peers, de-duplicated, without port 0.
    pub peers: Vec<PeerAddr>,
    pub warning: Option<String>,
    /// Our address as the tracker saw it (BEP 24).
    pub external_ip: Option<Ip>,
}

/// Why an announce or scrape did not succeed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TrackerError {
    /// The tracker refused (`failure reason`, or a UDP error action). `retry_in` is the
    /// BEP 31 `retry in` minutes (`None`: use normal back-off; `Some(u32::MAX)`: never).
    Failure { reason: String, retry_in: Option<u32> },
    /// The body was not a valid response.
    Malformed,
    /// No answer (UDP: all retransmissions timed out).
    Timeout,
}

fn text(v: &Value<'_>) -> String {
    let b = v.bytes().unwrap_or(b"");
    let b = &b[..b.len().min(MAX_TEXT)];
    String::from_utf8_lossy(b).into_owned()
}

fn uint(v: Option<&Value<'_>>) -> Option<u32> { v?.int().filter(|&n| n >= 0).map(|n| n.min(u32::MAX as i64) as u32) }

fn push_peer(out: &mut Vec<PeerAddr>, p: PeerAddr) {
    if p.port != 0 && !out.contains(&p) { out.push(p); }
}

/// Parses an HTTP announce response body.
pub fn parse_announce(body: &[u8]) -> Result<AnnounceResponse, TrackerError> {
    let v = bencode::decode(body).map_err(|_| TrackerError::Malformed)?;
    v.dict().ok_or(TrackerError::Malformed)?;
    if let Some(f) = v.get(b"failure reason") {
        let retry_in = match v.get(b"retry in") {
            Some(r) if r.str() == Some("never") => Some(u32::MAX),
            r => uint(r),
        };
        return Err(TrackerError::Failure { reason: text(f), retry_in });
    }
    let mut r = AnnounceResponse {
        interval: uint(v.get(b"interval")),
        min_interval: uint(v.get(b"min interval")),
        tracker_id: v.get(b"tracker id").and_then(|t| t.bytes()).filter(|t| t.len() <= 256).map(|t| t.to_vec()),
        complete: uint(v.get(b"complete")),
        incomplete: uint(v.get(b"incomplete")),
        downloaded: uint(v.get(b"downloaded")),
        warning: v.get(b"warning message").map(text),
        external_ip: v.get(b"external ip").and_then(|e| e.bytes()).and_then(|b| match b.len() {
            4 => Some(Ip::V4([b[0], b[1], b[2], b[3]])),
            16 => { let mut a = [0; 16]; a.copy_from_slice(b); Some(Ip::V6(a)) }
            _ => None,
        }),
        peers: Vec::new(),
    };
    let mut n4 = 0;
    match v.get(b"peers") {
        Some(p) if p.bytes().is_some() => {
            for a in PeerAddr::parse_compact(p.bytes().unwrap_or(b""), false).into_iter().take(MAX_PEERS) { push_peer(&mut r.peers, a); }
        }
        Some(p) if p.list().is_some() => {
            for d in p.list().unwrap_or(&[]) {
                if n4 >= MAX_PEERS { break; }
                n4 += 1;
                let ip = d.get(b"ip").and_then(|i| i.str()).and_then(parse_ip);
                let port = d.get(b"port").and_then(|p| p.int()).and_then(|p| u16::try_from(p).ok());
                if let (Some(ip), Some(port)) = (ip, port) { push_peer(&mut r.peers, PeerAddr { ip, port }); }
            }
        }
        Some(_) => return Err(TrackerError::Malformed),
        None => {}
    }
    if let Some(p6) = v.get(b"peers6").and_then(|p| p.bytes()) {
        for a in PeerAddr::parse_compact(p6, true).into_iter().take(MAX_PEERS) { push_peer(&mut r.peers, a); }
    }
    Ok(r)
}

/// One torrent's counts in a scrape.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ScrapeEntry {
    pub info_hash: [u8; 20],
    pub complete: u32,
    pub incomplete: u32,
    pub downloaded: u32,
}

/// A scrape response: the torrents it covered, and the BEP 48 `min_request_interval`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ScrapeResponse {
    pub files: Vec<ScrapeEntry>,
    pub min_request_interval: Option<u32>,
}

/// Parses an HTTP scrape response body (BEP 48).
pub fn parse_scrape(body: &[u8]) -> Result<ScrapeResponse, TrackerError> {
    let v = bencode::decode(body).map_err(|_| TrackerError::Malformed)?;
    v.dict().ok_or(TrackerError::Malformed)?;
    if let Some(f) = v.get(b"failure reason") { return Err(TrackerError::Failure { reason: text(f), retry_in: None }); }
    let files = v.get(b"files").and_then(|f| f.dict()).ok_or(TrackerError::Malformed)?;
    let mut out = ScrapeResponse { files: Vec::new(), min_request_interval: v.get(b"flags").and_then(|f| uint(f.get(b"min_request_interval"))) };
    for (k, d) in files.iter() {
        if out.files.len() >= MAX_SCRAPE * 4 { break; }
        let Ok(info_hash) = <[u8; 20]>::try_from(*k) else { continue };
        if d.dict().is_none() { continue; }
        out.files.push(ScrapeEntry {
            info_hash,
            complete: uint(d.get(b"complete")).unwrap_or(0),
            incomplete: uint(d.get(b"incomplete")).unwrap_or(0),
            downloaded: uint(d.get(b"downloaded")).unwrap_or(0),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------
// UDP tracker protocol (BEP 15, 41)
// ---------------------------------------------------------------------------------

/// The connect request's magic constant.
pub const PROTOCOL_ID: u64 = 0x41727101980;
pub const ACTION_CONNECT: u32 = 0;
pub const ACTION_ANNOUNCE: u32 = 1;
pub const ACTION_SCRAPE: u32 = 2;
pub const ACTION_ERROR: u32 = 3;
/// A connection id may be used for this long after it was received.
pub const CONNECTION_ID_LIFETIME: Millis = 60_000;
/// Base retransmit timeout: the n-th try waits `15 s · 2ⁿ`.
pub const UDP_TIMEOUT_BASE: Millis = 15_000;
/// Retransmissions before giving up (BEP 15: n goes up to 8, 3840 s).
pub const UDP_MAX_RETRIES: u8 = 8;

/// The 16-byte connect request.
pub fn encode_connect(tid: u32) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&PROTOCOL_ID.to_be_bytes());
    b[8..12].copy_from_slice(&ACTION_CONNECT.to_be_bytes());
    b[12..].copy_from_slice(&tid.to_be_bytes());
    b
}

/// The announce request: 98 bytes, then BEP 41 options (URL data in ≤255-byte chunks, then
/// end-of-options) when `url_data` is not empty.
pub fn encode_announce(connection_id: u64, tid: u32, p: &AnnounceParams, url_data: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(98 + url_data.len() + 8);
    b.extend_from_slice(&connection_id.to_be_bytes());
    b.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
    b.extend_from_slice(&tid.to_be_bytes());
    b.extend_from_slice(&p.info_hash);
    b.extend_from_slice(&p.peer_id);
    b.extend_from_slice(&p.downloaded.to_be_bytes());
    b.extend_from_slice(&p.left.to_be_bytes());
    b.extend_from_slice(&p.uploaded.to_be_bytes());
    b.extend_from_slice(&p.event.udp().to_be_bytes());
    let ip = match (p.anonymous, p.ipv4) { (false, Some(a)) => a, _ => [0; 4] };
    b.extend_from_slice(&ip);
    b.extend_from_slice(&p.key.to_be_bytes());
    let numwant: i32 = p.numwant.map(|n| n.min(i32::MAX as u32) as i32).unwrap_or(-1);
    b.extend_from_slice(&numwant.to_be_bytes());
    b.extend_from_slice(&p.port.to_be_bytes());
    let url_data = &url_data[..url_data.len().min(MAX_URL)];
    if !url_data.is_empty() {
        for chunk in url_data.chunks(255) {
            b.push(2);
            b.push(chunk.len() as u8);
            b.extend_from_slice(chunk);
        }
        b.push(0);
    }
    b
}

/// The scrape request for up to [`MAX_SCRAPE`] hashes.
pub fn encode_scrape(connection_id: u64, tid: u32, hashes: &[[u8; 20]]) -> Vec<u8> {
    let mut b = Vec::with_capacity(16 + 20 * hashes.len().min(MAX_SCRAPE));
    b.extend_from_slice(&connection_id.to_be_bytes());
    b.extend_from_slice(&ACTION_SCRAPE.to_be_bytes());
    b.extend_from_slice(&tid.to_be_bytes());
    for h in hashes.iter().take(MAX_SCRAPE) { b.extend_from_slice(h); }
    b
}

/// A decoded UDP tracker response.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UdpResponse {
    Connect { tid: u32, connection_id: u64 },
    /// `peers` are 6-byte entries on an IPv4 socket, 18-byte on IPv6.
    Announce { tid: u32, interval: u32, leechers: u32, seeders: u32, peers: Vec<PeerAddr> },
    /// (seeders, completed, leechers) per requested hash, in request order.
    Scrape { tid: u32, counts: Vec<(u32, u32, u32)> },
    Error { tid: u32, message: String },
}

fn be32(b: &[u8], at: usize) -> Option<u32> { b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])) }

/// Decodes a UDP tracker response. `v6`: the socket is IPv6, so peers are 18 bytes.
pub fn decode_udp(b: &[u8], v6: bool) -> Option<UdpResponse> {
    let action = be32(b, 0)?;
    let tid = be32(b, 4)?;
    match action {
        ACTION_CONNECT => {
            let s = b.get(8..16)?;
            let mut c = [0u8; 8];
            c.copy_from_slice(s);
            Some(UdpResponse::Connect { tid, connection_id: u64::from_be_bytes(c) })
        }
        ACTION_ANNOUNCE => {
            let interval = be32(b, 8)?;
            let leechers = be32(b, 12)?;
            let seeders = be32(b, 16)?;
            let mut peers = Vec::new();
            for p in PeerAddr::parse_compact(&b[20..], v6).into_iter().take(MAX_PEERS) { push_peer(&mut peers, p); }
            Some(UdpResponse::Announce { tid, interval, leechers, seeders, peers })
        }
        ACTION_SCRAPE => {
            let counts = b[8..].chunks_exact(12).take(MAX_SCRAPE).map(|c| (be32(c, 0).unwrap_or(0), be32(c, 4).unwrap_or(0), be32(c, 8).unwrap_or(0))).collect();
            Some(UdpResponse::Scrape { tid, counts })
        }
        ACTION_ERROR => {
            let m = &b[8..];
            Some(UdpResponse::Error { tid, message: String::from_utf8_lossy(&m[..m.len().min(MAX_TEXT)]).into_owned() })
        }
        _ => None,
    }
}

/// What to ask a UDP tracker.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UdpRequest {
    Announce(AnnounceParams),
    Scrape(Vec<[u8; 20]>),
}

/// What [`UdpTracker`] wants done, or what it learned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UdpEvent {
    /// Send this datagram to the tracker.
    Send(Vec<u8>),
    Announced(AnnounceResponse),
    Scraped(ScrapeResponse),
    /// The request failed (error action or timeout); the job is over.
    Failed(TrackerError),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase { Connecting, Requesting }

#[derive(Clone, Debug)]
struct UdpJob { req: UdpRequest, phase: Phase, tid: u32, attempt: u8, deadline: Millis }

/// One UDP tracker's client state machine (BEP 15). One request at a time.
#[derive(Clone, Debug)]
pub struct UdpTracker {
    url_data: Vec<u8>,
    v6: bool,
    conn: Option<(u64, Millis)>,
    job: Option<UdpJob>,
    max_retries: u8,
}

fn rand_u32(random: &mut dyn FnMut(&mut [u8])) -> u32 { let mut b = [0u8; 4]; random(&mut b); u32::from_be_bytes(b) }

impl UdpTracker {
    /// `url`: the tracker's parsed URL (its path and query go out as BEP 41 URL data,
    /// unless just `/`). `v6`: whether the socket used is IPv6.
    pub fn new(url: &TrackerUrl, v6: bool) -> Self {
        let url_data = if url.path_and_query == "/" { Vec::new() } else { url.path_and_query.as_bytes().to_vec() };
        UdpTracker { url_data, v6, conn: None, job: None, max_retries: UDP_MAX_RETRIES }
    }
    /// Fewer retransmissions than BEP 15's 8 (clients often use 2 or 3).
    pub fn with_max_retries(mut self, n: u8) -> Self { self.max_retries = n.min(UDP_MAX_RETRIES); self }
    /// Whether a request is running.
    pub fn busy(&self) -> bool { self.job.is_some() }
    /// When [`on_tick`](Self::on_tick) next needs calling.
    pub fn next_deadline(&self) -> Option<Millis> { self.job.as_ref().map(|j| j.deadline) }

    fn conn_valid(&self, now: Millis) -> Option<u64> { self.conn.filter(|&(_, exp)| now < exp).map(|(c, _)| c) }

    fn packet(&self, j: &UdpJob, now: Millis) -> Vec<u8> {
        match (j.phase, self.conn_valid(now)) {
            (Phase::Requesting, Some(c)) => match &j.req {
                UdpRequest::Announce(p) => encode_announce(c, j.tid, p, &self.url_data),
                UdpRequest::Scrape(h) => encode_scrape(c, j.tid, h),
            },
            _ => encode_connect(j.tid).to_vec(),
        }
    }

    /// Starts a request (dropping any running one) and returns the first datagram.
    pub fn request(&mut self, req: UdpRequest, now: Millis, random: &mut dyn FnMut(&mut [u8])) -> Vec<u8> {
        let phase = if self.conn_valid(now).is_some() { Phase::Requesting } else { Phase::Connecting };
        let j = UdpJob { req, phase, tid: rand_u32(random), attempt: 0, deadline: now + UDP_TIMEOUT_BASE };
        let p = self.packet(&j, now);
        self.job = Some(j);
        p
    }

    /// Cancels the running request.
    pub fn cancel(&mut self) { self.job = None; }

    /// Feeds a datagram from the tracker's address.
    pub fn on_packet(&mut self, bytes: &[u8], now: Millis, random: &mut dyn FnMut(&mut [u8])) -> Option<UdpEvent> {
        let j = self.job.as_ref()?;
        let resp = decode_udp(bytes, self.v6)?;
        let tid = match &resp { UdpResponse::Connect { tid, .. } | UdpResponse::Announce { tid, .. } | UdpResponse::Scrape { tid, .. } | UdpResponse::Error { tid, .. } => *tid };
        if tid != j.tid { return None; }
        match (j.phase, resp) {
            (_, UdpResponse::Error { message, .. }) => {
                self.job = None;
                self.conn = None;
                Some(UdpEvent::Failed(TrackerError::Failure { reason: message, retry_in: None }))
            }
            (Phase::Connecting, UdpResponse::Connect { connection_id, .. }) => {
                self.conn = Some((connection_id, now + CONNECTION_ID_LIFETIME));
                let mut j = self.job.take()?;
                j.phase = Phase::Requesting;
                j.tid = rand_u32(random);
                j.attempt = 0;
                j.deadline = now + UDP_TIMEOUT_BASE;
                let p = self.packet(&j, now);
                self.job = Some(j);
                Some(UdpEvent::Send(p))
            }
            (Phase::Requesting, UdpResponse::Announce { interval, leechers, seeders, peers, .. }) if matches!(j.req, UdpRequest::Announce(_)) => {
                self.job = None;
                Some(UdpEvent::Announced(AnnounceResponse { interval: Some(interval), complete: Some(seeders), incomplete: Some(leechers), peers, ..Default::default() }))
            }
            (Phase::Requesting, UdpResponse::Scrape { counts, .. }) => {
                let UdpRequest::Scrape(hashes) = &j.req else { return None };
                let files = hashes.iter().zip(counts).map(|(h, (s, c, l))| ScrapeEntry { info_hash: *h, complete: s, incomplete: l, downloaded: c }).collect();
                self.job = None;
                Some(UdpEvent::Scraped(ScrapeResponse { files, min_request_interval: None }))
            }
            _ => None,
        }
    }

    /// Call at [`next_deadline`](Self::next_deadline): retransmits (reconnecting first if
    /// the connection id expired) or gives up with a timeout.
    pub fn on_tick(&mut self, now: Millis, random: &mut dyn FnMut(&mut [u8])) -> Option<UdpEvent> {
        let mut j = self.job.take()?;
        if now < j.deadline { self.job = Some(j); return None; }
        if j.attempt >= self.max_retries { return Some(UdpEvent::Failed(TrackerError::Timeout)); }
        j.attempt += 1;
        if j.phase == Phase::Requesting && self.conn_valid(now).is_none() {
            j.phase = Phase::Connecting;
            j.tid = rand_u32(random);
        }
        j.deadline = now + (UDP_TIMEOUT_BASE << j.attempt);
        let p = self.packet(&j, now);
        self.job = Some(j);
        Some(UdpEvent::Send(p))
    }
}

// ---------------------------------------------------------------------------------
// Tiers (BEP 12)
// ---------------------------------------------------------------------------------

/// Names a tracker in a [`TrackerList`] (stable across reordering).
pub type TrackerId = u32;

/// Regular interval when the tracker gives none, and the floor for any interval.
pub const DEFAULT_INTERVAL: u32 = 1800;
pub const MIN_INTERVAL: u32 = 60;
/// Back-off after failures: 60 s, doubling, at most an hour.
pub const BACKOFF_BASE: Millis = 60_000;
pub const BACKOFF_MAX: Millis = 3_600_000;

/// One tracker and what we know about it.
#[derive(Clone, Debug)]
pub struct TrackerEntry {
    pub id: TrackerId,
    pub url: String,
    pub tier: usize,
    /// Consecutive failures.
    pub fails: u32,
    /// Not to be tried before this time (back-off).
    pub retry_at: Millis,
    /// We announced `started` successfully (so it gets `stopped` later).
    pub started: bool,
    /// The last announce succeeded.
    pub working: bool,
    pub last_error: Option<TrackerError>,
    pub last_warning: Option<String>,
    pub tracker_id: Option<Vec<u8>>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub min_interval: u32,
    in_flight: bool,
}

/// An announce the program should make now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AnnounceJob {
    pub tracker: TrackerId,
    pub url: String,
    pub event: Event,
    /// To fill [`AnnounceParams::tracker_id`].
    pub tracker_id: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
struct Group { next_announce: Millis, last_announce: Option<Millis>, in_flight: bool, completed_pending: bool }

/// A torrent's trackers in tiers (BEP 12).
///
/// Strict mode (default) treats all tiers as one ordered list: the first tracker not in
/// back-off is used, a working one moves to the front of its tier. With `all_tiers`, every
/// tier announces independently (what most modern clients do).
#[derive(Clone, Debug)]
pub struct TrackerList {
    tiers: Vec<Vec<TrackerEntry>>,
    groups: Vec<Group>,
    all_tiers: bool,
    next_id: TrackerId,
    completed_sent: bool,
    stopped: bool,
}

fn shuffle<T>(v: &mut [T], random: &mut dyn FnMut(&mut [u8])) {
    for i in (1..v.len()).rev() {
        let j = (rand_u32(random) as usize) % (i + 1);
        v.swap(i, j);
    }
}

impl TrackerList {
    /// `tiers`: the torrent's `announce-list` (or `[[announce]]`). Unparsable URLs and
    /// duplicates are dropped; each tier is shuffled with `random`.
    pub fn new(tiers: &[Vec<String>], all_tiers: bool, random: &mut dyn FnMut(&mut [u8])) -> Self {
        let mut l = TrackerList { tiers: Vec::new(), groups: Vec::new(), all_tiers, next_id: 0, completed_sent: false, stopped: false };
        for t in tiers.iter().take(64) {
            let mut tier = Vec::new();
            for u in t.iter().take(64) {
                if parse_url(u).is_none() || l.all().any(|e| e.url == *u) || tier.iter().any(|e: &TrackerEntry| e.url == *u) { continue; }
                tier.push(l.entry(u, 0));
            }
            if !tier.is_empty() {
                shuffle(&mut tier, random);
                let n = l.tiers.len();
                for e in &mut tier { e.tier = n; }
                l.tiers.push(tier);
            }
        }
        l.groups = vec![Group::default(); if all_tiers { l.tiers.len() } else { 1 }];
        l
    }

    fn entry(&mut self, url: &str, tier: usize) -> TrackerEntry {
        self.next_id += 1;
        TrackerEntry { id: self.next_id, url: url.to_string(), tier, fails: 0, retry_at: 0, started: false, working: false, last_error: None, last_warning: None, tracker_id: None, seeders: None, leechers: None, min_interval: 0, in_flight: false }
    }

    /// Every tracker, tier by tier in current order.
    pub fn all(&self) -> impl Iterator<Item = &TrackerEntry> { self.tiers.iter().flatten() }

    /// Number of tiers.
    pub fn tier_count(&self) -> usize { self.tiers.len() }

    fn group_of(&self, tier: usize) -> usize { if self.all_tiers { tier } else { 0 } }

    fn find(&mut self, id: TrackerId) -> Option<&mut TrackerEntry> { self.tiers.iter_mut().flatten().find(|e| e.id == id) }

    /// Adds a tracker to a tier (a new tier at the end if `tier` is past the last).
    pub fn add(&mut self, url: &str, tier: usize) -> Option<TrackerId> {
        if parse_url(url).is_none() || self.all().any(|e| e.url == url) { return None; }
        let tier = tier.min(self.tiers.len());
        if tier == self.tiers.len() {
            self.tiers.push(Vec::new());
            if self.all_tiers || self.groups.is_empty() { self.groups.push(Group::default()); }
        }
        let e = self.entry(url, tier);
        let id = e.id;
        self.tiers[tier].push(e);
        Some(id)
    }

    /// Removes a tracker; returns the `stopped` announce it should get if it had started.
    pub fn remove(&mut self, id: TrackerId) -> Option<AnnounceJob> {
        let (t, i) = self.tiers.iter().enumerate().find_map(|(t, tier)| tier.iter().position(|e| e.id == id).map(|i| (t, i)))?;
        let e = self.tiers[t].remove(i);
        if e.in_flight { let g = self.group_of(t); if let Some(g) = self.groups.get_mut(g) { g.in_flight = false; } }
        e.started.then(|| AnnounceJob { tracker: e.id, url: e.url, event: Event::Stopped, tracker_id: e.tracker_id })
    }

    /// The download finished: the next announce of every group carries `completed`
    /// (only ever once per torrent).
    pub fn completed(&mut self, now: Millis) {
        if self.completed_sent || self.stopped { return; }
        self.completed_sent = true;
        for g in &mut self.groups { g.completed_pending = true; g.next_announce = g.next_announce.min(now); }
    }

    /// Announce again as soon as each tracker's `min interval` allows (e.g. user asked).
    pub fn force_reannounce(&mut self, now: Millis) {
        for (gi, g) in self.groups.iter_mut().enumerate() {
            let min = self.tiers.iter().flatten().filter(|e| (if self.all_tiers { e.tier == gi } else { true }) && e.working).map(|e| e.min_interval).max().unwrap_or(0);
            g.next_announce = match g.last_announce { Some(t) => (t + min as Millis * 1000).max(now), None => now };
        }
    }

    /// When [`due`](Self::due) next has something (ignoring in-flight announces).
    pub fn next_wakeup(&self) -> Option<Millis> {
        if self.stopped { return None; }
        self.groups.iter().enumerate().filter(|(_, g)| !g.in_flight).filter_map(|(gi, g)| {
            let retry = self.tiers.iter().flatten().filter(|e| self.group_of(e.tier) == gi).map(|e| e.retry_at).min()?;
            Some(g.next_announce.max(retry))
        }).min()
    }

    /// The announces to make now. Each job must be answered with
    /// [`on_success`](Self::on_success) or [`on_failure`](Self::on_failure).
    pub fn due(&mut self, now: Millis) -> Vec<AnnounceJob> {
        let mut jobs = Vec::new();
        if self.stopped { return jobs; }
        for gi in 0..self.groups.len() {
            let g = &self.groups[gi];
            if g.in_flight || now < g.next_announce { continue; }
            let completed = g.completed_pending;
            let all_tiers = self.all_tiers;
            let Some(e) = self.tiers.iter_mut().flatten().find(|e| (!all_tiers || e.tier == gi) && e.retry_at <= now) else { continue };
            e.in_flight = true;
            let event = if !e.started { Event::Started } else if completed { Event::Completed } else { Event::None };
            jobs.push(AnnounceJob { tracker: e.id, url: e.url.clone(), event, tracker_id: e.tracker_id.clone() });
            self.groups[gi].in_flight = true;
        }
        jobs
    }

    /// The announce to `id` worked. Moves it to the front of its tier.
    pub fn on_success(&mut self, id: TrackerId, event: Event, r: &AnnounceResponse, now: Millis) {
        let all_tiers = self.all_tiers;
        let Some(e) = self.find(id) else { return };
        e.in_flight = false;
        e.fails = 0;
        e.retry_at = 0;
        e.working = true;
        e.last_error = None;
        e.last_warning = r.warning.clone();
        if event == Event::Started || e.started { e.started = true; }
        if r.tracker_id.is_some() { e.tracker_id = r.tracker_id.clone(); }
        if r.complete.is_some() { e.seeders = r.complete; }
        if r.incomplete.is_some() { e.leechers = r.incomplete; }
        e.min_interval = r.min_interval.unwrap_or(0);
        let interval = r.interval.unwrap_or(DEFAULT_INTERVAL).max(r.min_interval.unwrap_or(0)).max(MIN_INTERVAL);
        let tier = e.tier;
        let gi = if all_tiers { tier } else { 0 };
        if let Some(i) = self.tiers[tier].iter().position(|e| e.id == id) {
            let e = self.tiers[tier].remove(i);
            self.tiers[tier].insert(0, e);
        }
        if let Some(g) = self.groups.get_mut(gi) {
            g.in_flight = false;
            g.last_announce = Some(now);
            g.next_announce = now + interval as Millis * 1000;
            if event == Event::Completed || (event == Event::Started && g.completed_pending) {
                // A `started` that went out after completion counts: the tracker sees left=0.
                g.completed_pending = false;
            }
        }
    }

    /// The announce to `id` failed: back off that tracker; the next one in order is tried
    /// at once by the next [`due`](Self::due).
    pub fn on_failure(&mut self, id: TrackerId, err: TrackerError, now: Millis) {
        let all_tiers = self.all_tiers;
        let Some(e) = self.find(id) else { return };
        e.in_flight = false;
        e.working = false;
        e.fails = e.fails.saturating_add(1);
        let mut wait = (BACKOFF_BASE << (e.fails - 1).min(6)).min(BACKOFF_MAX);
        if let TrackerError::Failure { retry_in: Some(m), .. } = err {
            wait = if m == u32::MAX { Millis::MAX / 2 } else { wait.max(m as Millis * 60_000) };
        }
        e.retry_at = now.saturating_add(wait);
        e.last_error = Some(err);
        let gi = if all_tiers { e.tier } else { 0 };
        if let Some(g) = self.groups.get_mut(gi) { g.in_flight = false; }
    }

    /// The torrent stops: `stopped` announces for every tracker that saw `started`. After
    /// this nothing is due.
    pub fn stop(&mut self) -> Vec<AnnounceJob> {
        self.stopped = true;
        let jobs = self.all().filter(|e| e.started).map(|e| AnnounceJob { tracker: e.id, url: e.url.clone(), event: Event::Stopped, tracker_id: e.tracker_id.clone() }).collect();
        for e in self.tiers.iter_mut().flatten() { e.started = false; e.in_flight = false; }
        for g in &mut self.groups { g.in_flight = false; }
        jobs
    }

    /// Starts again after [`stop`](Self::stop): next announces are `started`.
    pub fn restart(&mut self, now: Millis) {
        self.stopped = false;
        for g in &mut self.groups { g.next_announce = now; g.in_flight = false; }
    }

    /// Tracker states keyed by id (for a UI).
    pub fn snapshot(&self) -> BTreeMap<TrackerId, TrackerEntry> { self.all().map(|e| (e.id, e.clone())).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bencode::dict;
    use crate::bencode::Item;

    fn rng(seed: u8) -> impl FnMut(&mut [u8]) {
        let mut s = seed as u32 | 1;
        move |b: &mut [u8]| for x in b { s ^= s << 13; s ^= s >> 17; s ^= s << 5; *x = s as u8; }
    }

    #[test]
    fn urls() {
        let u = parse_url("http://tracker.example.org/announce").unwrap();
        assert_eq!((u.scheme, u.host.as_str(), u.port, u.path_and_query.as_str()), (Scheme::Http, "tracker.example.org", 80, "/announce"));
        let u = parse_url("HTTPS://Tr.Example:8443/a/announce.php?passkey=abc#frag").unwrap();
        assert_eq!((u.scheme, u.host.as_str(), u.port, u.path_and_query.as_str()), (Scheme::Https, "tr.example", 8443, "/a/announce.php?passkey=abc"));
        let u = parse_url("udp://[2001:db8::1]:6969/announce").unwrap();
        assert_eq!(u.host_ip(), Some(Ip::V6([0x20, 1, 0xd, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])));
        assert_eq!(u.to_url(), "udp://[2001:db8::1]:6969/announce");
        let u = parse_url("udp://10.0.0.1:1337").unwrap();
        assert_eq!(u.path_and_query, "/");
        assert_eq!(parse_url("http://h?x=1").unwrap().path_and_query, "/?x=1");
        for bad in ["udp://host/announce", "ftp://x/", "http://", "http://a b/", "http://u@h/", "http://h:0/", "http://h:70000/", "http://[::1/", "http://[1.2.3.4]/", "http://h:x/", "http://a:b:c/", "nonsense", "http://[::1]x/"] {
            assert!(parse_url(bad).is_none(), "{}", bad);
        }
        let long = format!("http://h/{}", "a".repeat(5000));
        assert!(parse_url(&long).is_none());
    }

    #[test]
    fn ip_text() {
        assert_eq!(parse_ip("10.0.2.15"), Some(Ip::V4([10, 0, 2, 15])));
        for bad in ["256.1.1.1", "1.2.3", "1.2.3.4.5", "01.2.3.4", "1..2.3", "", "1:2:3:4:5:6:7:8:9", "::1::", "12345::", "g::", ":::", "1:2:3:4:5:6:7::8:9"] {
            assert!(parse_ip(bad).is_none(), "{}", bad);
        }
        for (s, canon) in [("::", "::"), ("::1", "::1"), ("2001:DB8:0:0:0:0:2:1", "2001:db8::2:1"), ("2001:db8:0:1:1:1:1:1", "2001:db8:0:1:1:1:1:1"), ("::ffff:1.2.3.4", "::ffff:102:304"), ("1:0:0:2:0:0:0:3", "1:0:0:2::3"), ("fe80::", "fe80::")] {
            let ip = parse_ip(s).unwrap();
            assert_eq!(fmt_ip(&ip), canon);
            assert_eq!(parse_ip(canon), Some(ip));
        }
        assert_eq!(fmt_ip(&Ip::V4([1, 2, 3, 4])), "1.2.3.4");
    }

    #[test]
    fn http_announce_request() {
        let mut p = AnnounceParams::new([0x12; 20], *b"-SX0100-abcdefghijkl", 6881, 0xdeadbeef);
        p.info_hash[0] = b'A';
        p.info_hash[1] = b' ';
        p.left = 1000;
        p.event = Event::Started;
        p.numwant = Some(50);
        p.ipv6 = Some([0x20, 1, 0xd, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        p.tracker_id = Some(b"t 1".to_vec());
        let t = announce_request("http://t.example:8080/announce?pk=1", &p).unwrap();
        assert_eq!(t.host, "t.example");
        assert_eq!(t.port, 8080);
        assert_eq!(t.host_header, "t.example:8080");
        assert!(!t.tls);
        let q = &t.path_and_query;
        assert!(q.starts_with("/announce?pk=1&info_hash=A%20%12%12"), "{}", q);
        assert!(q.contains("&peer_id=-SX0100-abcdefghijkl&port=6881&uploaded=0&downloaded=0&left=1000&key=DEADBEEF&event=started&numwant=50&compact=1&no_peer_id=1"));
        assert!(q.contains("&trackerid=t%201"));
        assert!(q.ends_with("&ipv6=2001%3Adb8%3A%3A1"));
        p.anonymous = true;
        let q = announce_request("https://t.example/announce", &p).unwrap();
        assert!(!q.path_and_query.contains("ipv6"));
        assert_eq!(q.host_header, "t.example");
        assert_eq!(announce_request("udp://t:1/", &p), Err(UrlError::WrongScheme));
    }

    #[test]
    fn hybrid_announces_both() {
        let ih = InfoHashes::Hybrid(crate::InfoHashV1([1; 20]), crate::InfoHashV2([2; 32]));
        assert_eq!(announce_hashes(&ih), vec![[1; 20], [2; 20]]);
        assert_eq!(announce_hashes(&InfoHashes::V2(crate::InfoHashV2([3; 32]))), vec![[3; 20]]);
    }

    #[test]
    fn scrape_urls() {
        assert_eq!(scrape_path("/announce").as_deref(), Some("/scrape"));
        assert_eq!(scrape_path("/x/announce.php?pk=1").as_deref(), Some("/x/scrape.php?pk=1"));
        assert_eq!(scrape_path("/a").as_deref(), None);
        assert_eq!(scrape_path("/announce/x").as_deref(), None);
        let t = scrape_request("http://h/announce", &[[0xaa; 20], [b'b'; 20]]).unwrap();
        assert_eq!(t.path_and_query, format!("/scrape?info_hash={}&info_hash={}", "%AA".repeat(20), "b".repeat(20)));
        assert_eq!(scrape_request("http://h/x", &[]), Err(UrlError::NoScrape));
    }

    #[test]
    fn parse_announce_compact_and_dicts() {
        let mut compact = vec![10, 0, 0, 1, 0x1a, 0xe1];
        compact.extend_from_slice(&[10, 0, 0, 2, 0, 0]); // port 0 dropped
        compact.push(9); // trailing partial entry ignored
        let p6 = PeerAddr { ip: Ip::V6([0xfe; 16]), port: 1 }.compact();
        let body = dict([(b"interval", 900.into()), (b"min interval", 60.into()), (b"complete", 3.into()), (b"incomplete", (-1).into()),
            (b"peers", compact.into()), (b"peers6", p6.into()), (b"tracker id", "abc".into()), (b"warning message", "careful".into()),
            (b"external ip", vec![1u8, 2, 3, 4].into())]).encode();
        let r = parse_announce(&body).unwrap();
        assert_eq!(r.interval, Some(900));
        assert_eq!(r.min_interval, Some(60));
        assert_eq!(r.complete, Some(3));
        assert_eq!(r.incomplete, None);
        assert_eq!(r.peers, vec![PeerAddr { ip: Ip::V4([10, 0, 0, 1]), port: 6881 }, PeerAddr { ip: Ip::V6([0xfe; 16]), port: 1 }]);
        assert_eq!(r.tracker_id.as_deref(), Some(&b"abc"[..]));
        assert_eq!(r.warning.as_deref(), Some("careful"));
        assert_eq!(r.external_ip, Some(Ip::V4([1, 2, 3, 4])));

        let peer = |ip: &str, port: i64| dict([(b"ip", ip.into()), (b"port", port.into()), (b"peer id", vec![0u8; 20].into())]);
        let list = Item::List(vec![peer("1.2.3.4", 80), peer("::1", 81), peer("host.name", 82), peer("1.2.3.4", 70000), Item::Int(5), peer("1.2.3.4", 80)]);
        let r = parse_announce(&dict([(b"interval", 60.into()), (b"peers", list)]).encode()).unwrap();
        assert_eq!(r.peers.len(), 2);
    }

    #[test]
    fn parse_announce_failures() {
        let body = dict([(b"failure reason", "unregistered torrent".into()), (b"retry in", "never".into())]).encode();
        assert_eq!(parse_announce(&body), Err(TrackerError::Failure { reason: "unregistered torrent".into(), retry_in: Some(u32::MAX) }));
        let body = dict([(b"failure reason", "busy".into()), (b"retry in", 5.into())]).encode();
        assert_eq!(parse_announce(&body), Err(TrackerError::Failure { reason: "busy".into(), retry_in: Some(5) }));
        for bad in [&b""[..], b"<html>", b"le", b"i5e", b"d5:peersi5ee", b"d8:intervali1e"] {
            assert_eq!(parse_announce(bad), Err(TrackerError::Malformed), "{:?}", bad);
        }
        let huge = dict([(b"failure reason", vec![b'x'; 5000].into())]).encode();
        match parse_announce(&huge) { Err(TrackerError::Failure { reason, .. }) => assert_eq!(reason.len(), MAX_TEXT), e => panic!("{:?}", e) }
        // Many peers are capped.
        let many = vec![1u8; 6 * (MAX_PEERS + 100)];
        let r = parse_announce(&dict([(b"peers", many.into())]).encode()).unwrap();
        assert!(r.peers.len() <= MAX_PEERS);
    }

    #[test]
    fn parse_scrape_bodies() {
        let files = Item::Dict([(vec![7u8; 20], dict([(b"complete", 5.into()), (b"downloaded", 50.into()), (b"incomplete", 10.into())])), (vec![1u8; 3], dict([]))].into_iter().collect());
        let body = dict([(b"files", files), (b"flags", dict([(b"min_request_interval", 900.into())]))]).encode();
        let r = parse_scrape(&body).unwrap();
        assert_eq!(r.files, vec![ScrapeEntry { info_hash: [7; 20], complete: 5, incomplete: 10, downloaded: 50 }]);
        assert_eq!(r.min_request_interval, Some(900));
        assert_eq!(parse_scrape(b"d5:filesi1ee"), Err(TrackerError::Malformed));
        assert!(matches!(parse_scrape(b"d14:failure reason3:no"), Err(TrackerError::Malformed)));
        assert!(matches!(parse_scrape(b"d14:failure reason4:nopee"), Err(TrackerError::Failure { .. })));
    }

    #[test]
    fn udp_golden_layouts() {
        assert_eq!(encode_connect(0x01020304), [0, 0, 4, 0x17, 0x27, 0x10, 0x19, 0x80, 0, 0, 0, 0, 1, 2, 3, 4]);
        let mut p = AnnounceParams::new([0xaa; 20], [0xbb; 20], 0x1ae1, 0x11223344);
        p.downloaded = 1;
        p.left = 2;
        p.uploaded = 3;
        p.event = Event::Stopped;
        let b = encode_announce(0x0102030405060708, 9, &p, b"");
        assert_eq!(b.len(), 98);
        assert_eq!(&b[0..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&b[8..12], &[0, 0, 0, 1]);
        assert_eq!(&b[12..16], &[0, 0, 0, 9]);
        assert_eq!(&b[16..36], &[0xaa; 20]);
        assert_eq!(&b[36..56], &[0xbb; 20]);
        assert_eq!(&b[56..64], &1u64.to_be_bytes());
        assert_eq!(&b[64..72], &2u64.to_be_bytes());
        assert_eq!(&b[72..80], &3u64.to_be_bytes());
        assert_eq!(&b[80..84], &[0, 0, 0, 3]);
        assert_eq!(&b[84..88], &[0; 4]);
        assert_eq!(&b[88..92], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(&b[92..96], &[0xff; 4]);
        assert_eq!(&b[96..98], &[0x1a, 0xe1]);
        // BEP 41: "/announce?x" as URL data.
        let b = encode_announce(1, 2, &p, b"/ann?x");
        assert_eq!(&b[98..], &[2, 6, b'/', b'a', b'n', b'n', b'?', b'x', 0]);
        let long = vec![b'a'; 300];
        let b = encode_announce(1, 2, &p, &long);
        assert_eq!((b[98], b[99], b[98 + 2 + 255], b[98 + 2 + 255 + 1], *b.last().unwrap()), (2, 255, 2, 45, 0));
        let s = encode_scrape(5, 6, &[[1; 20], [2; 20]]);
        assert_eq!(s.len(), 56);
        assert_eq!(&s[8..12], &[0, 0, 0, 2]);
    }

    #[test]
    fn udp_decode_hostile() {
        for n in 0..40 { let _ = decode_udp(&vec![0u8; n], false); let _ = decode_udp(&vec![0xffu8; n], true); }
        assert_eq!(decode_udp(&[0, 0, 0, 0, 0, 0, 0, 1], false), None);
        assert_eq!(decode_udp(&[0, 0, 0, 9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0], false), None);
        let mut a = vec![0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 60, 0, 0, 0, 2, 0, 0, 0, 3];
        a.extend_from_slice(&[0x20; 18]);
        match decode_udp(&a, true).unwrap() { UdpResponse::Announce { peers, seeders, leechers, interval, .. } => { assert_eq!((peers.len(), seeders, leechers, interval), (1, 3, 2, 60)); } r => panic!("{:?}", r) }
        assert_eq!(decode_udp(&[0, 0, 0, 3, 0, 0, 0, 7, b'n', b'o'], false), Some(UdpResponse::Error { tid: 7, message: "no".into() }));
    }

    /// A fake UDP tracker answering the client's datagrams.
    fn server(req: &[u8], conn_id: u64) -> Vec<u8> {
        if req.len() == 16 && req[..8] == PROTOCOL_ID.to_be_bytes() {
            let mut r = vec![0, 0, 0, 0];
            r.extend_from_slice(&req[12..16]);
            r.extend_from_slice(&conn_id.to_be_bytes());
            return r;
        }
        assert_eq!(&req[..8], &conn_id.to_be_bytes());
        let action = be32(req, 8).unwrap();
        let mut r = action.to_be_bytes().to_vec();
        r.extend_from_slice(&req[12..16]);
        if action == 1 {
            r.extend_from_slice(&[0, 0, 7, 8, 0, 0, 0, 1, 0, 0, 0, 2, 9, 9, 9, 9, 0x1a, 0xe1]);
        } else {
            for i in 0..(req.len() - 16) / 20 { r.extend_from_slice(&[0, 0, 0, i as u8, 0, 0, 0, 5, 0, 0, 0, 6]); }
        }
        r
    }

    #[test]
    fn udp_client_round_trip_and_expiry() {
        let mut rnd = rng(3);
        let u = parse_url("udp://t.example:6969/announce").unwrap();
        let mut c = UdpTracker::new(&u, false);
        let p = AnnounceParams::new([1; 20], [2; 20], 6881, 5);
        let connect = c.request(UdpRequest::Announce(p.clone()), 0, &mut rnd);
        assert_eq!(connect.len(), 16);
        // A reply with the wrong transaction id is ignored.
        let mut wrong = server(&connect, 77);
        wrong[7] ^= 1;
        assert_eq!(c.on_packet(&wrong, 1, &mut rnd), None);
        let UdpEvent::Send(ann) = c.on_packet(&server(&connect, 77), 1, &mut rnd).unwrap() else { panic!() };
        assert_eq!(&ann[98..], &[2, 9, b'/', b'a', b'n', b'n', b'o', b'u', b'n', b'c', b'e', 0]);
        let UdpEvent::Announced(r) = c.on_packet(&server(&ann, 77), 2, &mut rnd).unwrap() else { panic!() };
        assert_eq!(r.interval, Some(0x708));
        assert_eq!(r.peers, vec![PeerAddr { ip: Ip::V4([9; 4]), port: 6881 }]);
        assert!(!c.busy());
        // Within a minute the connection id is reused: no connect.
        let s = c.request(UdpRequest::Scrape(vec![[1; 20], [2; 20]]), 30_000, &mut rnd);
        assert_eq!(s.len(), 56);
        let UdpEvent::Scraped(r) = c.on_packet(&server(&s, 77), 30_001, &mut rnd).unwrap() else { panic!() };
        assert_eq!(r.files[1], ScrapeEntry { info_hash: [2; 20], complete: 1, downloaded: 5, incomplete: 6 });
        // After 60 s it connects again.
        assert_eq!(c.request(UdpRequest::Scrape(vec![[1; 20]]), 61_000, &mut rnd).len(), 16);
    }

    #[test]
    fn udp_retransmit_schedule_and_error() {
        let mut rnd = rng(4);
        let u = parse_url("udp://t:1").unwrap();
        let mut c = UdpTracker::new(&u, false);
        let first = c.request(UdpRequest::Scrape(vec![[1; 20]]), 0, &mut rnd);
        let mut now = 0;
        let mut waits = Vec::new();
        loop {
            let d = c.next_deadline().unwrap();
            waits.push((d - now) / 1000);
            now = d;
            assert_eq!(c.on_tick(now - 1, &mut rnd), None);
            match c.on_tick(now, &mut rnd).unwrap() {
                UdpEvent::Send(p) => assert_eq!(p, first),
                UdpEvent::Failed(e) => { assert_eq!(e, TrackerError::Timeout); break; }
                e => panic!("{:?}", e),
            }
        }
        assert_eq!(waits, vec![15, 30, 60, 120, 240, 480, 960, 1920, 3840]);
        assert!(!c.busy());
        // Error action ends the job.
        let req = c.request(UdpRequest::Scrape(vec![[1; 20]]), now, &mut rnd);
        let mut err = vec![0, 0, 0, 3];
        err.extend_from_slice(&req[12..16]);
        err.extend_from_slice(b"banned");
        assert_eq!(c.on_packet(&err, now, &mut rnd), Some(UdpEvent::Failed(TrackerError::Failure { reason: "banned".into(), retry_in: None })));
    }

    #[test]
    fn udp_reconnects_when_id_expires_during_retries() {
        let mut rnd = rng(5);
        let mut c = UdpTracker::new(&parse_url("udp://t:1").unwrap(), true).with_max_retries(2);
        let conn = c.request(UdpRequest::Announce(AnnounceParams::new([1; 20], [2; 20], 1, 1)), 0, &mut rnd);
        let UdpEvent::Send(ann) = c.on_packet(&server(&conn, 9), 0, &mut rnd).unwrap() else { panic!() };
        assert_eq!(ann.len(), 98);
        let UdpEvent::Send(re) = c.on_tick(15_000, &mut rnd).unwrap() else { panic!() };
        assert_eq!(re.len(), 98);
        let UdpEvent::Send(re) = c.on_tick(45_000 + 30_000, &mut rnd).unwrap() else { panic!() };
        assert_eq!(re.len(), 16, "expired id means connect again");
        assert_eq!(c.on_tick(10_000_000, &mut rnd), Some(UdpEvent::Failed(TrackerError::Timeout)));
    }

    fn tiers() -> Vec<Vec<String>> {
        vec![vec!["http://a/announce".into(), "http://b/announce".into(), "bogus".into()], vec!["udp://c:1".into()]]
    }

    #[test]
    fn tiers_try_in_order_and_promote() {
        let mut rnd = rng(9);
        let mut l = TrackerList::new(&tiers(), false, &mut rnd);
        assert_eq!(l.all().count(), 3);
        let jobs = l.due(0);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].event, Event::Started);
        let first = jobs[0].tracker;
        assert!(l.due(0).is_empty(), "one in flight per group");
        l.on_failure(first, TrackerError::Timeout, 0);
        let j2 = l.due(0);
        assert_eq!(j2.len(), 1);
        assert_ne!(j2[0].tracker, first);
        let second = j2[0].tracker;
        l.on_success(second, Event::Started, &AnnounceResponse { interval: Some(100), ..Default::default() }, 0);
        assert_eq!(l.all().next().unwrap().id, second, "promoted to front");
        assert!(l.due(99_000).is_empty());
        assert_eq!(l.next_wakeup(), Some(100_000));
        let j = l.due(100_000);
        assert_eq!((j[0].tracker, j[0].event), (second, Event::None));
        l.on_success(second, Event::None, &AnnounceResponse::default(), 100_000);
        // completed once.
        l.completed(200_000);
        let j = l.due(200_000);
        assert_eq!(j[0].event, Event::Completed);
        l.on_success(second, Event::Completed, &AnnounceResponse::default(), 200_000);
        l.completed(300_000);
        assert!(l.due(300_000).is_empty());
        let j = l.due(200_000 + 1_800_000);
        assert_eq!(j[0].event, Event::None);
        l.on_failure(j[0].tracker, TrackerError::Malformed, 2_000_000);
        let stops = l.stop();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].event, Event::Stopped);
        assert!(l.due(u64::MAX / 4).is_empty());
    }

    #[test]
    fn all_tiers_and_backoff() {
        let mut rnd = rng(1);
        let mut l = TrackerList::new(&tiers(), true, &mut rnd);
        let jobs = l.due(0);
        assert_eq!(jobs.len(), 2, "one per tier");
        for j in &jobs { l.on_failure(j.tracker, TrackerError::Timeout, 0); }
        let jobs = l.due(0);
        assert_eq!(jobs.len(), 1, "tier 1 has a second tracker; tier 2 is backing off");
        l.on_failure(jobs[0].tracker, TrackerError::Failure { reason: "x".into(), retry_in: Some(10) }, 0);
        assert!(l.due(59_000).is_empty());
        assert_eq!(l.due(60_000).len(), 2);
        let first = l.all().next().unwrap().id;
        let removed = l.remove(first);
        assert_eq!(removed, None, "never started, no stopped needed");
        assert!(l.add("http://d/announce", 5).is_some());
        assert_eq!(l.tier_count(), 3);
        assert!(l.add("http://d/announce", 0).is_none());
    }
}
