// SPDX-License-Identifier: GPL-3.0-only
//! Port mapping: asking the home router to forward a port to us, so other peers can
//! connect in (which matters a lot for how well a torrent client does).
//!
//! Three protocols, tried in this order:
//! - PCP, the Port Control Protocol (RFC 6887), UDP to the gateway's port 5351.
//! - NAT-PMP (RFC 6886), its predecessor, on the same port. A NAT-PMP-only router answers
//!   a PCP request with "unsupported version", which is how we fall back quickly.
//! - UPnP IGD: an SSDP M-SEARCH multicast finds the router, an HTTP GET fetches its
//!   device description (an XML file we scan for the WANIPConnection or WANPPPConnection
//!   service and its controlURL), then SOAP calls over HTTP add, renew and delete
//!   mappings and fetch the external address.
//!
//! The encoders and parsers for each are plain functions. [`PortMapper`] is the state
//! machine on top, sans-IO: it asks the program to send UDP datagrams or make HTTP
//! requests ([`Action`]), takes their answers ([`PortMapper::on_udp`],
//! [`PortMapper::on_http_response`]), and reports [`Event`]s: a port mapped (with the
//! external address), failed, removed. It renews mappings at half their lifetime, notices
//! a router that rebooted (PCP and NAT-PMP epoch), and deletes everything on shutdown.

use crate::{Ip, PeerAddr};
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The gateway's PCP / NAT-PMP port, and the port announcements are multicast to.
pub const SERVER_PORT: u16 = 5351;
pub const CLIENT_PORT: u16 = 5350;
/// The SSDP multicast address and port.
pub const SSDP: PeerAddr = PeerAddr { ip: Ip::V4([239, 255, 255, 250]), port: 1900 };
/// The search target we multicast.
pub const IGD_ST: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:1";
/// Services that can map ports, best first.
pub const WAN_SERVICES: [&str; 3] = [
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];
/// Biggest XML / HTTP response we look at.
pub const MAX_HTTP: usize = 256 * 1024;

/// Transport protocol of a mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Protocol { Udp, Tcp }

impl Protocol {
    /// IANA protocol number (PCP).
    pub fn number(self) -> u8 { match self { Protocol::Udp => 17, Protocol::Tcp => 6 } }
    /// NAT-PMP opcode.
    pub fn natpmp_op(self) -> u8 { match self { Protocol::Udp => 1, Protocol::Tcp => 2 } }
    /// UPnP name.
    pub fn upnp(self) -> &'static str { match self { Protocol::Udp => "UDP", Protocol::Tcp => "TCP" } }
}

// ---------------------------------------------------------------------------------------
// NAT-PMP (RFC 6886)

/// Request for the external address (opcode 0).
pub fn natpmp_external_request() -> [u8; 2] { [0, 0] }

/// Map request; lifetime 0 with external port 0 deletes the mapping.
pub fn natpmp_map_request(proto: Protocol, internal: u16, external: u16, lifetime_s: u32) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[1] = proto.natpmp_op();
    b[4..6].copy_from_slice(&internal.to_be_bytes());
    b[6..8].copy_from_slice(&external.to_be_bytes());
    b[8..12].copy_from_slice(&lifetime_s.to_be_bytes());
    b
}

/// A NAT-PMP response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NatPmpResponse {
    External { epoch: u32, ip: [u8; 4] },
    Mapping { proto: Protocol, epoch: u32, internal: u16, external: u16, lifetime_s: u32 },
    /// Result code not 0: 1 unsupported version, 2 not authorized, 3 network failure,
    /// 4 out of resources, 5 unsupported opcode.
    Error { opcode: u8, result: u16, epoch: u32 },
}

fn be16(b: &[u8], at: usize) -> Option<u16> { Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?])) }
fn be32(b: &[u8], at: usize) -> Option<u32> { Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?])) }

/// Parses a NAT-PMP response.
pub fn parse_natpmp(b: &[u8]) -> Option<NatPmpResponse> {
    if *b.first()? != 0 { return None; }
    let op = *b.get(1)?;
    if op < 128 { return None; }
    let result = be16(b, 2)?;
    let epoch = be32(b, 4).unwrap_or(0);
    if result != 0 { return Some(NatPmpResponse::Error { opcode: op - 128, result, epoch }); }
    match op - 128 {
        0 => Some(NatPmpResponse::External { epoch, ip: [*b.get(8)?, *b.get(9)?, *b.get(10)?, *b.get(11)?] }),
        1 | 2 => Some(NatPmpResponse::Mapping {
            proto: if op == 129 { Protocol::Udp } else { Protocol::Tcp },
            epoch, internal: be16(b, 8)?, external: be16(b, 10)?, lifetime_s: be32(b, 12)?,
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// PCP (RFC 6887)

/// PCP result codes.
pub mod pcp_result {
    pub const SUCCESS: u8 = 0;
    pub const UNSUPP_VERSION: u8 = 1;
    pub const NOT_AUTHORIZED: u8 = 2;
    pub const MALFORMED_REQUEST: u8 = 3;
    pub const UNSUPP_OPCODE: u8 = 4;
    pub const UNSUPP_OPTION: u8 = 5;
    pub const MALFORMED_OPTION: u8 = 6;
    pub const NETWORK_FAILURE: u8 = 7;
    pub const NO_RESOURCES: u8 = 8;
    pub const UNSUPP_PROTOCOL: u8 = 9;
    pub const USER_EX_QUOTA: u8 = 10;
    pub const CANNOT_PROVIDE_EXTERNAL: u8 = 11;
    pub const ADDRESS_MISMATCH: u8 = 12;
    pub const EXCESSIVE_REMOTE_PEERS: u8 = 13;
}

const PCP_VERSION: u8 = 2;
const OP_ANNOUNCE: u8 = 0;
const OP_MAP: u8 = 1;

/// An address in PCP's 16-byte form (IPv4 as ::ffff:a.b.c.d).
fn pcp_addr(ip: Ip) -> [u8; 16] {
    match ip {
        Ip::V6(a) => a,
        Ip::V4(a) => { let mut o = [0u8; 16]; o[10] = 0xff; o[11] = 0xff; o[12..].copy_from_slice(&a); o }
    }
}

fn from_pcp_addr(a: &[u8]) -> Option<Ip> {
    let a: [u8; 16] = a.try_into().ok()?;
    if a[..10].iter().all(|&x| x == 0) && a[10] == 0xff && a[11] == 0xff { Some(Ip::V4([a[12], a[13], a[14], a[15]])) } else { Some(Ip::V6(a)) }
}

fn pcp_header(opcode: u8, lifetime_s: u32, client: Ip) -> Vec<u8> {
    let mut v = Vec::with_capacity(60);
    v.extend_from_slice(&[PCP_VERSION, opcode & 0x7f, 0, 0]);
    v.extend_from_slice(&lifetime_s.to_be_bytes());
    v.extend_from_slice(&pcp_addr(client));
    v
}

/// ANNOUNCE request (24 bytes): asks whether a PCP server is there.
pub fn pcp_announce_request(client: Ip) -> Vec<u8> { pcp_header(OP_ANNOUNCE, 0, client) }

/// MAP request (60 bytes). Lifetime 0 deletes. `suggested_ip` `None` means "any".
pub fn pcp_map_request(client: Ip, nonce: [u8; 12], proto: Protocol, internal: u16, suggested_port: u16, suggested_ip: Option<Ip>, lifetime_s: u32) -> Vec<u8> {
    let mut v = pcp_header(OP_MAP, lifetime_s, client);
    v.extend_from_slice(&nonce);
    v.extend_from_slice(&[proto.number(), 0, 0, 0]);
    v.extend_from_slice(&internal.to_be_bytes());
    v.extend_from_slice(&suggested_port.to_be_bytes());
    v.extend_from_slice(&suggested_ip.map(pcp_addr).unwrap_or_else(|| pcp_addr(Ip::V4([0; 4]))));
    v
}

/// A PCP response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcpResponse {
    Announce { result: u8, epoch: u32 },
    Map { result: u8, lifetime_s: u32, epoch: u32, nonce: [u8; 12], protocol: u8, internal: u16, external_port: u16, external_ip: Ip },
    /// Another opcode we do not use.
    Other { opcode: u8, result: u8, epoch: u32 },
}

/// Parses a PCP response (version 2, R bit set).
pub fn parse_pcp(b: &[u8]) -> Option<PcpResponse> {
    if b.len() < 24 || b.len() > 1100 || b.len() % 4 != 0 || b[0] != PCP_VERSION || b[1] & 0x80 == 0 { return None; }
    let opcode = b[1] & 0x7f;
    let result = b[3];
    let lifetime_s = be32(b, 4)?;
    let epoch = be32(b, 8)?;
    match opcode {
        OP_ANNOUNCE => Some(PcpResponse::Announce { result, epoch }),
        OP_MAP => {
            if b.len() < 60 { return None; }
            let nonce: [u8; 12] = b.get(24..36)?.try_into().ok()?;
            Some(PcpResponse::Map {
                result, lifetime_s, epoch, nonce, protocol: b[36],
                internal: be16(b, 40)?, external_port: be16(b, 42)?, external_ip: from_pcp_addr(b.get(44..60)?)?,
            })
        }
        _ => Some(PcpResponse::Other { opcode, result, epoch }),
    }
}

/// RFC 6887 section 8.5 (also RFC 6886 3.6): did the server lose its state (reboot)?
/// Times in seconds: previous and current server epoch, and the client time between.
pub fn epoch_lost_state(prev_epoch: u32, prev_client_s: u64, epoch: u32, client_s: u64) -> bool {
    if (epoch as u64) + 1 < prev_epoch as u64 { return true; }
    let server_delta = epoch.saturating_sub(prev_epoch) as u64;
    let client_delta = client_s.saturating_sub(prev_client_s);
    client_delta + 2 < server_delta - server_delta / 16 || server_delta + 2 < client_delta - client_delta / 16
}

// ---------------------------------------------------------------------------------------
// UPnP IGD: SSDP, URLs, XML scanning, HTTP and SOAP.

/// An SSDP M-SEARCH request for `st`.
pub fn ssdp_msearch(st: &str) -> Vec<u8> {
    format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nST: {}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\n\r\n", st).into_bytes()
}

/// The parts of an SSDP answer we use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SsdpResponse { pub location: String, pub st: String }

/// Iterates HTTP header lines as (lower-cased name, trimmed value).
fn headers(head: &str) -> impl Iterator<Item = (String, &str)> {
    head.split("\r\n").skip(1).filter_map(|l| {
        let (k, v) = l.split_once(':')?;
        Some((k.trim().to_ascii_lowercase(), v.trim()))
    })
}

/// Parses an SSDP search response (HTTP/1.1 200 OK with LOCATION and ST).
pub fn parse_ssdp(b: &[u8]) -> Option<SsdpResponse> {
    let text = core::str::from_utf8(b.get(..b.len().min(4096))?).ok()?;
    let first = text.split("\r\n").next()?;
    let mut parts = first.split_whitespace();
    if !parts.next()?.starts_with("HTTP/1.") || parts.next()? != "200" { return None; }
    let (mut location, mut st) = (None, None);
    for (k, v) in headers(text) {
        match k.as_str() { "location" => location = Some(v.to_string()), "st" => st = Some(v.to_string()), _ => {} }
    }
    let st = st?;
    let interesting = st.contains("InternetGatewayDevice") || st.contains("WANIPConnection") || st.contains("WANPPPConnection");
    interesting.then_some(SsdpResponse { location: location?, st })
}

/// An http:// URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url { pub host: String, pub port: u16, pub path: String }

impl Url {
    /// Parses `http://host[:port][/path]` (no https: routers do not use it here).
    pub fn parse(s: &str) -> Option<Url> {
        let s = s.trim();
        let rest = s.get(..7).filter(|p| p.eq_ignore_ascii_case("http://")).and(s.get(7..))?;
        let (hostport, path) = match rest.find('/') { Some(i) => (&rest[..i], &rest[i..]), None => (rest, "/") };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.contains(']') || h.ends_with(']') => (h, p.parse().ok()?),
            _ => (hostport, 80),
        };
        if host.is_empty() || host.len() > 255 || host.chars().any(|c| c.is_whitespace() || c == '@') { return None; }
        Some(Url { host: host.to_string(), port, path: path.to_string() })
    }
    /// Resolves a (possibly relative) reference against this URL.
    pub fn join(&self, r: &str) -> Option<Url> {
        let r = r.trim();
        if r.len() >= 7 && r[..7].eq_ignore_ascii_case("http://") { return Url::parse(r); }
        let path = if r.starts_with('/') {
            r.to_string()
        } else {
            let dir = self.path.rfind('/').map_or("/", |i| &self.path[..=i]);
            format!("{}{}", dir, r)
        };
        Some(Url { host: self.host.clone(), port: self.port, path })
    }
    fn host_header(&self) -> String { if self.port == 80 { self.host.clone() } else { format!("{}:{}", self.host, self.port) } }
}

/// Finds the local name of the tag starting at `<` (skipping a namespace prefix).
fn tag_at(s: &str, lt: usize) -> Option<(bool, &str, usize)> {
    let rest = s.get(lt + 1..)?;
    let (closing, rest, skip) = match rest.strip_prefix('/') { Some(r) => (true, r, 2), None => (false, rest, 1) };
    let end = rest.find(|c: char| c == '>' || c == '/' || c.is_whitespace())?;
    let name = &rest[..end];
    let local = name.rsplit(':').next().unwrap_or(name);
    let gt = rest.find('>')?;
    Some((closing, local, lt + skip + gt + 1))
}

/// Byte ranges of the contents of each `<tag>`..`</tag>` (local name, any case), in order.
/// Elements of the same name are assumed not to nest.
fn elements<'a>(xml: &'a str, tag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let mut pos = 0;
    core::iter::from_fn(move || {
        loop {
            let lt = pos + xml.get(pos..)?.find('<')?;
            let (closing, name, after) = tag_at(xml, lt)?;
            pos = after;
            if closing || !name.eq_ignore_ascii_case(tag) || xml.get(..after)?.ends_with("/>") { continue; }
            // Find the matching close tag.
            let mut p = after;
            loop {
                let lt2 = p + xml.get(p..)?.find('<')?;
                let (c2, n2, after2) = tag_at(xml, lt2)?;
                if c2 && n2.eq_ignore_ascii_case(tag) { pos = after2; return xml.get(after..lt2); }
                p = after2;
            }
        }
    })
}

/// Text of the first `<tag>` element, entities decoded, trimmed.
pub fn xml_text(xml: &str, tag: &str) -> Option<String> {
    elements(xml, tag).next().map(|t| unescape(t.trim()))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The service to send SOAP calls to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IgdService { pub service_type: String, pub control_url: Url }

/// Scans a device description for the best WAN connection service.
pub fn parse_description(xml: &str, location: &Url) -> Option<IgdService> {
    let xml = &xml[..floor_char(xml, MAX_HTTP)];
    let base = xml_text(xml, "URLBase").and_then(|b| Url::parse(&b)).unwrap_or_else(|| location.clone());
    let mut best: Option<(usize, IgdService)> = None;
    for svc in elements(xml, "service") {
        let Some(st) = xml_text(svc, "serviceType") else { continue };
        let Some(rank) = WAN_SERVICES.iter().position(|w| st.eq_ignore_ascii_case(w)) else { continue };
        let Some(ctrl) = xml_text(svc, "controlURL").and_then(|c| base.join(&c)) else { continue };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) { best = Some((rank, IgdService { service_type: st, control_url: ctrl })); }
    }
    best.map(|(_, s)| s)
}

fn floor_char(s: &str, max: usize) -> usize {
    let mut i = s.len().min(max);
    while !s.is_char_boundary(i) { i -= 1; }
    i
}

/// An HTTP GET request.
pub fn http_get(url: &Url) -> Vec<u8> {
    format!("GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: SexOS/1.0 UPnP/1.1 Swingers/1.0\r\n\r\n", url.path, url.host_header()).into_bytes()
}

/// A SOAP action request (complete HTTP POST).
pub fn soap_request(svc: &IgdService, action: &str, args: &[(&str, String)]) -> Vec<u8> {
    let mut body = format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{} xmlns:u=\"{}\">",
        action, svc.service_type);
    for (k, v) in args { body.push_str(&format!("<{}>{}</{}>", k, escape(v), k)); }
    body.push_str(&format!("</u:{}></s:Body></s:Envelope>\r\n", action));
    let u = &svc.control_url;
    let mut req = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{}#{}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        u.path, u.host_header(), svc.service_type, action, body.len()).into_bytes();
    req.extend_from_slice(body.as_bytes());
    req
}

/// AddPortMapping arguments.
pub fn add_port_mapping(svc: &IgdService, proto: Protocol, external: u16, internal: u16, client: [u8; 4], description: &str, lease_s: u32) -> Vec<u8> {
    soap_request(svc, "AddPortMapping", &[
        ("NewRemoteHost", String::new()),
        ("NewExternalPort", external.to_string()),
        ("NewProtocol", proto.upnp().to_string()),
        ("NewInternalPort", internal.to_string()),
        ("NewInternalClient", format!("{}.{}.{}.{}", client[0], client[1], client[2], client[3])),
        ("NewEnabled", "1".to_string()),
        ("NewPortMappingDescription", description.to_string()),
        ("NewLeaseDuration", lease_s.to_string()),
    ])
}

/// DeletePortMapping arguments.
pub fn delete_port_mapping(svc: &IgdService, proto: Protocol, external: u16) -> Vec<u8> {
    soap_request(svc, "DeletePortMapping", &[
        ("NewRemoteHost", String::new()),
        ("NewExternalPort", external.to_string()),
        ("NewProtocol", proto.upnp().to_string()),
    ])
}

/// GetExternalIPAddress.
pub fn get_external_ip(svc: &IgdService) -> Vec<u8> { soap_request(svc, "GetExternalIPAddress", &[]) }

/// Splits an HTTP response into status code and body (Content-Length or chunked; else
/// everything up to the end, as the connection was closed).
pub fn parse_http_response(b: &[u8]) -> Option<(u16, Vec<u8>)> {
    let b = &b[..b.len().min(MAX_HTTP)];
    let split = b.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = core::str::from_utf8(&b[..split]).ok()?;
    let body = &b[split + 4..];
    let mut first = head.split("\r\n").next()?.split_whitespace();
    if !first.next()?.starts_with("HTTP/1.") { return None; }
    let status: u16 = first.next()?.parse().ok()?;
    let (mut chunked, mut len) = (false, None);
    for (k, v) in headers(head) {
        if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") { chunked = true; }
        if k == "content-length" { len = v.parse::<usize>().ok(); }
    }
    if chunked { return Some((status, dechunk(body)?)); }
    let n = len.unwrap_or(body.len()).min(body.len());
    Some((status, body[..n].to_vec()))
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let line = core::str::from_utf8(&b[..eol]).ok()?;
        let size = usize::from_str_radix(line.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 { return Some(out); }
        let chunk = b.get(..size)?;
        out.extend_from_slice(chunk);
        b = b.get(size..)?;
        b = b.strip_prefix(b"\r\n").unwrap_or(b);
    }
}

/// What a SOAP call came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoapResult {
    /// 200: the response body.
    Ok(String),
    /// A UPnP error: 718 ConflictInMappingEntry, 725 OnlyPermanentLeasesSupported,
    /// 714 NoSuchEntryInArray, 606 Action not authorized, ...
    Fault { code: u16, description: String },
    /// Some other HTTP failure (or garbage).
    Http(u16),
}

pub const ERR_CONFLICT: u16 = 718;
pub const ERR_ONLY_PERMANENT: u16 = 725;

/// Interprets a SOAP response (status and body from [`parse_http_response`]).
pub fn parse_soap(status: u16, body: &[u8]) -> SoapResult {
    let text = core::str::from_utf8(&body[..body.len().min(MAX_HTTP)]).unwrap_or("");
    if status == 200 { return SoapResult::Ok(text.to_string()); }
    if let Some(code) = xml_text(text, "errorCode").and_then(|c| c.parse().ok()) {
        return SoapResult::Fault { code, description: xml_text(text, "errorDescription").unwrap_or_default() };
    }
    SoapResult::Http(status)
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut o = [0u8; 4];
    let mut it = s.trim().split('.');
    for b in o.iter_mut() { *b = it.next()?.parse().ok()?; }
    it.next().is_none().then_some(o)
}

// ---------------------------------------------------------------------------------------
// The state machine.

/// Settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// The default gateway (for PCP and NAT-PMP); `None` goes straight to UPnP.
    pub gateway: Option<[u8; 4]>,
    /// Our LAN address (the mapping's internal client).
    pub local_ip: [u8; 4],
    /// Requested lifetime / lease, seconds.
    pub lifetime_s: u32,
    /// Shown in the router's UPnP table.
    pub description: String,
    pub pcp: bool,
    pub natpmp: bool,
    pub upnp: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { gateway: None, local_ip: [0; 4], lifetime_s: 3600, description: "Swingers".to_string(), pcp: true, natpmp: true, upnp: true }
    }
}

/// Which protocol a mapping was made with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method { Pcp, NatPmp, Upnp }

/// Names a mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(pub u32);

/// What the program should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Send this UDP datagram (to the gateway's 5351, or the SSDP multicast group).
    /// Answers come back through [`PortMapper::on_udp`]; also listen on 224.0.0.1:5350
    /// for PCP / NAT-PMP announcements.
    Udp { to: PeerAddr, data: Vec<u8> },
    /// Connect over TCP to host:port, send `request`, read until the server closes, and
    /// hand the bytes (or `None` on failure) to [`PortMapper::on_http_response`].
    Http { token: u32, host: String, port: u16, request: Vec<u8> },
}

/// Why a mapping failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No PCP, NAT-PMP or UPnP gateway answered.
    NoGateway,
    /// The gateway said no (PCP result code).
    Pcp(u8),
    /// The gateway said no (NAT-PMP result code).
    NatPmp(u16),
    /// The gateway said no (UPnP error code, or HTTP status).
    Upnp(u16),
    /// The gateway stopped answering.
    Timeout,
}

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A port is mapped (or its external side changed).
    Mapped { id: MapId, protocol: Protocol, internal_port: u16, external: PeerAddr, method: Method, lifetime_s: u32 },
    /// The router's external address.
    ExternalIp(Ip),
    Failed { id: MapId, reason: Failure },
    /// Removed (deletion sent to the router).
    Unmapped { id: MapId },
}

const UDP_FIRST_RETRY_US: u64 = 250_000;
const PROBE_TRIES: u32 = 4;
const MAP_TRIES: u32 = 6;
const SSDP_TRIES: u32 = 3;
const SSDP_WAIT_US: u64 = 2_000_000;
const HTTP_TRIES: u32 = 3;
const MAX_CONFLICT_TRIES: u32 = 8;
const MAX_MAPPINGS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Backend {
    ProbePcp { tries: u32, next_us: u64 },
    ProbeNatPmp { tries: u32, next_us: u64 },
    Ssdp { tries: u32, next_us: u64 },
    Describe { token: u32, url: Url },
    Pcp,
    NatPmp,
    Upnp(IgdService),
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MState {
    /// Waiting for a backend.
    Pending,
    /// Request out; resend at `next_us` (UDP) or awaiting HTTP `token`.
    Requesting { tries: u32, next_us: u64, token: Option<u32> },
    Mapped { renew_us: Option<u64> },
}

#[derive(Clone, Debug)]
struct Mapping {
    id: MapId,
    proto: Protocol,
    internal: u16,
    external_port: u16,
    lease_s: u32,
    nonce: [u8; 12],
    state: MState,
    external: Option<PeerAddr>,
    conflicts: u32,
}

/// The port-mapping state machine.
#[derive(Debug)]
pub struct PortMapper {
    cfg: Config,
    backend: Backend,
    maps: Vec<Mapping>,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
    next_id: u32,
    next_token: u32,
    rng: u64,
    epoch: Option<(u32, u64)>,
    external_ip: Option<[u8; 4]>,
    ext_ip_token: Option<u32>,
    shutting_down: bool,
}

impl PortMapper {
    /// Starts probing at once. `seed` (from the program's random source) makes PCP nonces.
    pub fn new(cfg: Config, seed: u64, now_us: u64) -> Self {
        let mut m = PortMapper {
            cfg, backend: Backend::None, maps: Vec::new(), actions: VecDeque::new(), events: VecDeque::new(),
            next_id: 1, next_token: 1, rng: seed, epoch: None, external_ip: None, ext_ip_token: None, shutting_down: false,
        };
        m.restart(now_us);
        m
    }

    /// Probes again from PCP (after a network change, say); existing mappings are redone.
    pub fn restart(&mut self, now_us: u64) {
        for m in &mut self.maps { m.state = MState::Pending; }
        self.epoch = None;
        self.backend = if self.cfg.gateway.is_some() && self.cfg.pcp {
            Backend::ProbePcp { tries: 0, next_us: now_us }
        } else {
            self.after_natpmp_probe_start(now_us)
        };
        if self.cfg.gateway.is_some() && !self.cfg.pcp && self.cfg.natpmp { self.backend = Backend::ProbeNatPmp { tries: 0, next_us: now_us }; }
    }

    fn after_natpmp_probe_start(&self, now_us: u64) -> Backend {
        if self.cfg.gateway.is_some() && self.cfg.natpmp { Backend::ProbeNatPmp { tries: 0, next_us: now_us } } else { self.ssdp_or_none(now_us) }
    }

    fn ssdp_or_none(&self, now_us: u64) -> Backend {
        if self.cfg.upnp { Backend::Ssdp { tries: 0, next_us: now_us } } else { Backend::None }
    }

    /// The method in use, once one answered.
    pub fn method(&self) -> Option<Method> {
        match self.backend { Backend::Pcp => Some(Method::Pcp), Backend::NatPmp => Some(Method::NatPmp), Backend::Upnp(_) => Some(Method::Upnp), _ => None }
    }

    /// Asks for `internal_port` to be reachable from outside, preferably as `external_port`.
    pub fn add(&mut self, proto: Protocol, internal_port: u16, external_port: u16) -> MapId {
        let id = MapId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        if self.maps.len() >= MAX_MAPPINGS || self.shutting_down {
            self.events.push_back(Event::Failed { id, reason: Failure::NoGateway });
            return id;
        }
        let mut nonce = [0u8; 12];
        for c in nonce.chunks_mut(8) { let r = self.random().to_le_bytes(); c.copy_from_slice(&r[..c.len()]); }
        self.maps.push(Mapping {
            id, proto, internal: internal_port, external_port: if external_port == 0 { internal_port } else { external_port },
            lease_s: self.cfg.lifetime_s, nonce, state: MState::Pending, external: None, conflicts: 0,
        });
        if self.backend == Backend::None { self.fail_all_pending(); }
        id
    }

    /// Removes a mapping (sends the deletion if it was mapped).
    pub fn remove(&mut self, id: MapId) {
        let Some(i) = self.maps.iter().position(|m| m.id == id) else { return };
        let m = self.maps.remove(i);
        if matches!(m.state, MState::Mapped { .. }) || matches!(m.state, MState::Requesting { .. }) { self.send_delete(&m); }
        self.events.push_back(Event::Unmapped { id });
    }

    /// Removes every mapping; after this, [`PortMapper::is_idle`] once the actions are sent.
    pub fn shutdown(&mut self) {
        self.shutting_down = true;
        let ids: Vec<MapId> = self.maps.iter().map(|m| m.id).collect();
        for id in ids { self.remove(id); }
        self.backend = Backend::None;
    }

    /// Nothing mapped and nothing left to send.
    pub fn is_idle(&self) -> bool { self.maps.is_empty() && self.actions.is_empty() }

    pub fn poll_event(&mut self) -> Option<Event> { self.events.pop_front() }

    /// The next thing to do. Also runs the timers.
    pub fn poll_action(&mut self, now_us: u64) -> Option<Action> {
        if self.actions.is_empty() { self.run(now_us); }
        self.actions.pop_front()
    }

    /// When `poll_action` next needs calling for timers.
    pub fn next_timeout(&self) -> Option<u64> {
        let b = match self.backend {
            Backend::ProbePcp { next_us, .. } | Backend::ProbeNatPmp { next_us, .. } | Backend::Ssdp { next_us, .. } => Some(next_us),
            _ => None,
        };
        let m = self.maps.iter().filter_map(|m| match m.state {
            MState::Requesting { next_us, token: None, .. } => Some(next_us),
            MState::Mapped { renew_us } => renew_us,
            _ => None,
        }).min();
        [b, m].into_iter().flatten().min()
    }

    fn random(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn token(&mut self) -> u32 { let t = self.next_token; self.next_token = self.next_token.wrapping_add(1).max(1); t }

    fn gateway(&self) -> PeerAddr { PeerAddr { ip: Ip::V4(self.cfg.gateway.unwrap_or([0; 4])), port: SERVER_PORT } }

    fn udp(&mut self, data: Vec<u8>) { let to = self.gateway(); self.actions.push_back(Action::Udp { to, data }); }

    fn http(&mut self, url: &Url, request: Vec<u8>) -> u32 {
        let token = self.token();
        self.actions.push_back(Action::Http { token, host: url.host.clone(), port: url.port, request });
        token
    }

    fn fail_all_pending(&mut self) {
        let failed: Vec<MapId> = self.maps.iter().filter(|m| !matches!(m.state, MState::Mapped { .. })).map(|m| m.id).collect();
        self.maps.retain(|m| matches!(m.state, MState::Mapped { .. }));
        for id in failed { self.events.push_back(Event::Failed { id, reason: Failure::NoGateway }); }
    }

    fn fail(&mut self, i: usize, reason: Failure) {
        if i < self.maps.len() {
            let m = self.maps.remove(i);
            self.events.push_back(Event::Failed { id: m.id, reason });
        }
    }

    fn send_delete(&mut self, m: &Mapping) {
        let client = Ip::V4(self.cfg.local_ip);
        match &self.backend {
            Backend::Pcp => {
                let ext = m.external.map(|e| e.ip);
                let d = pcp_map_request(client, m.nonce, m.proto, m.internal, m.external_port, ext, 0);
                self.udp(d);
            }
            Backend::NatPmp => { let d = natpmp_map_request(m.proto, m.internal, 0, 0).to_vec(); self.udp(d); }
            Backend::Upnp(svc) => {
                let (svc, req) = (svc.clone(), delete_port_mapping(svc, m.proto, m.external_port));
                self.http(&svc.control_url, req);
            }
            _ => {}
        }
    }

    /// Runs timers: probes and mapping requests that are due.
    fn run(&mut self, now_us: u64) {
        let backoff = |tries: u32| UDP_FIRST_RETRY_US << tries.min(10);
        match self.backend.clone() {
            Backend::ProbePcp { tries, next_us } if now_us >= next_us => {
                if tries >= PROBE_TRIES { self.backend = self.after_natpmp_probe_start(now_us); return self.run(now_us); }
                let d = pcp_announce_request(Ip::V4(self.cfg.local_ip));
                self.udp(d);
                self.backend = Backend::ProbePcp { tries: tries + 1, next_us: now_us + backoff(tries) };
            }
            Backend::ProbeNatPmp { tries, next_us } if now_us >= next_us => {
                if tries >= PROBE_TRIES { self.backend = self.ssdp_or_none(now_us); return self.run(now_us); }
                self.udp(natpmp_external_request().to_vec());
                self.backend = Backend::ProbeNatPmp { tries: tries + 1, next_us: now_us + backoff(tries) };
            }
            Backend::Ssdp { tries, next_us } if now_us >= next_us => {
                if tries >= SSDP_TRIES { self.backend = Backend::None; self.fail_all_pending(); return; }
                self.actions.push_back(Action::Udp { to: SSDP, data: ssdp_msearch(IGD_ST) });
                self.backend = Backend::Ssdp { tries: tries + 1, next_us: now_us + SSDP_WAIT_US };
            }
            Backend::None => self.fail_all_pending(),
            _ => {}
        }
        if !matches!(self.backend, Backend::Pcp | Backend::NatPmp | Backend::Upnp(_)) { return; }
        for i in 0..self.maps.len() {
            let Some(m) = self.maps.get(i) else { break };
            let due = match m.state {
                MState::Pending => Some(0),
                MState::Requesting { tries, next_us, token: None } if now_us >= next_us => Some(tries),
                MState::Mapped { renew_us: Some(r) } if now_us >= r => Some(0),
                _ => None,
            };
            if let Some(tries) = due { self.request(i, tries, now_us); }
        }
        // Drop the ones that ran out of tries (marked by `request`).
        let mut i = 0;
        while i < self.maps.len() {
            if matches!(self.maps[i].state, MState::Requesting { tries, token: None, .. } if tries > MAP_TRIES) { self.fail(i, Failure::Timeout); } else { i += 1; }
        }
    }

    /// Sends (or resends) the map request for mapping `i`.
    fn request(&mut self, i: usize, tries: u32, now_us: u64) {
        let client = Ip::V4(self.cfg.local_ip);
        let Some(m) = self.maps.get(i).cloned() else { return };
        if tries >= MAP_TRIES {
            if let Some(mm) = self.maps.get_mut(i) { mm.state = MState::Requesting { tries: MAP_TRIES + 1, next_us: now_us, token: None }; }
            return;
        }
        let next_us = now_us + (UDP_FIRST_RETRY_US << tries.min(10));
        let token = match &self.backend {
            Backend::Pcp => {
                let d = pcp_map_request(client, m.nonce, m.proto, m.internal, m.external.map_or(m.external_port, |e| e.port), m.external.map(|e| e.ip), m.lease_s);
                self.udp(d);
                None
            }
            Backend::NatPmp => {
                let d = natpmp_map_request(m.proto, m.internal, m.external.map_or(m.external_port, |e| e.port), m.lease_s).to_vec();
                self.udp(d);
                None
            }
            Backend::Upnp(svc) => {
                let svc = svc.clone();
                let req = add_port_mapping(&svc, m.proto, m.external_port, m.internal, self.cfg.local_ip, &self.cfg.description, m.lease_s);
                Some(self.http(&svc.control_url, req))
            }
            _ => return,
        };
        if let Some(mm) = self.maps.get_mut(i) { mm.state = MState::Requesting { tries: tries + 1, next_us, token }; }
    }

    fn check_epoch(&mut self, epoch: u32, now_us: u64) {
        let now_s = now_us / 1_000_000;
        if let Some((prev, at)) = self.epoch {
            if epoch_lost_state(prev, at, epoch, now_s) {
                // The router rebooted: map everything again.
                for m in &mut self.maps { if matches!(m.state, MState::Mapped { .. }) { m.state = MState::Pending; } }
            }
        }
        self.epoch = Some((epoch, now_s));
    }

    fn mapped(&mut self, i: usize, external: PeerAddr, lifetime_s: u32, now_us: u64, method: Method) {
        let Some(m) = self.maps.get_mut(i) else { return };
        let renew_us = (lifetime_s > 0).then(|| now_us + lifetime_s as u64 * 500_000);
        m.state = MState::Mapped { renew_us };
        if m.external != Some(external) {
            m.external = Some(external);
            let ev = Event::Mapped { id: m.id, protocol: m.proto, internal_port: m.internal, external, method, lifetime_s };
            self.events.push_back(ev);
        }
    }

    /// A UDP datagram from `from` (the gateway's 5351, an announcement to 5350, or an
    /// SSDP answer).
    pub fn on_udp(&mut self, from: PeerAddr, data: &[u8], now_us: u64) {
        if data.first() == Some(&b'H') {
            if let (Backend::Ssdp { .. }, Some(r)) = (&self.backend, parse_ssdp(data)) {
                if let Some(url) = Url::parse(&r.location) {
                    let token = self.http(&url, http_get(&url));
                    self.backend = Backend::Describe { token, url };
                }
            }
            return;
        }
        if self.cfg.gateway.map(Ip::V4) != Some(from.ip) { return; }
        if let Some(r) = parse_pcp(data) { self.on_pcp(r, now_us); return; }
        if let Some(r) = parse_natpmp(data) { self.on_natpmp(r, now_us); }
    }

    fn on_pcp(&mut self, r: PcpResponse, now_us: u64) {
        match r {
            PcpResponse::Announce { result, epoch } => {
                if let Backend::ProbePcp { .. } = self.backend {
                    if result == pcp_result::SUCCESS {
                        self.backend = Backend::Pcp;
                        self.check_epoch(epoch, now_us);
                    } else {
                        self.backend = self.after_natpmp_probe_start(now_us);
                    }
                } else if self.backend == Backend::Pcp {
                    self.check_epoch(epoch, now_us);
                }
            }
            PcpResponse::Map { result, lifetime_s, epoch, nonce, protocol, internal, external_port, external_ip } => {
                if self.backend != Backend::Pcp { return; }
                self.check_epoch(epoch, now_us);
                let Some(i) = self.maps.iter().position(|m| m.nonce == nonce && m.proto.number() == protocol && m.internal == internal) else { return };
                if !matches!(self.maps[i].state, MState::Requesting { .. }) { return; }
                if result == pcp_result::SUCCESS {
                    let ext = PeerAddr { ip: external_ip, port: external_port };
                    if let Ip::V4(a) = external_ip { self.note_external(a); }
                    self.mapped(i, ext, lifetime_s, now_us, Method::Pcp);
                } else if result == pcp_result::CANNOT_PROVIDE_EXTERNAL && self.maps[i].external.is_some() {
                    // Our old external port is gone: ask for any.
                    if let Some(m) = self.maps.get_mut(i) { m.external = None; m.state = MState::Pending; }
                } else {
                    self.fail(i, Failure::Pcp(result));
                }
            }
            PcpResponse::Other { .. } => {}
        }
    }

    fn note_external(&mut self, ip: [u8; 4]) {
        if self.external_ip != Some(ip) {
            self.external_ip = Some(ip);
            self.events.push_back(Event::ExternalIp(Ip::V4(ip)));
        }
    }

    fn on_natpmp(&mut self, r: NatPmpResponse, now_us: u64) {
        match r {
            NatPmpResponse::Error { opcode, result, epoch } => match self.backend {
                // A NAT-PMP router answering our PCP probe: "unsupported version".
                Backend::ProbePcp { .. } => self.backend = Backend::ProbeNatPmp { tries: 0, next_us: now_us },
                Backend::ProbeNatPmp { .. } if opcode == 0 => self.backend = self.ssdp_or_none(now_us),
                Backend::NatPmp if opcode == 1 || opcode == 2 => {
                    self.check_epoch(epoch, now_us);
                    let proto = if opcode == 1 { Protocol::Udp } else { Protocol::Tcp };
                    // Error responses do not say which port: fail the oldest outstanding one.
                    if let Some(i) = self.maps.iter().position(|m| m.proto == proto && matches!(m.state, MState::Requesting { .. })) {
                        self.fail(i, Failure::NatPmp(result));
                    }
                }
                _ => {}
            },
            NatPmpResponse::External { epoch, ip } => {
                if matches!(self.backend, Backend::ProbeNatPmp { .. } | Backend::ProbePcp { .. }) { self.backend = Backend::NatPmp; }
                if self.backend != Backend::NatPmp { return; }
                self.check_epoch(epoch, now_us);
                self.note_external(ip);
            }
            NatPmpResponse::Mapping { proto, epoch, internal, external, lifetime_s } => {
                if self.backend != Backend::NatPmp { return; }
                self.check_epoch(epoch, now_us);
                let Some(i) = self.maps.iter().position(|m| m.proto == proto && m.internal == internal && matches!(m.state, MState::Requesting { .. })) else { return };
                let ip = Ip::V4(self.external_ip.unwrap_or([0; 4]));
                self.mapped(i, PeerAddr { ip, port: external }, lifetime_s, now_us, Method::NatPmp);
            }
        }
    }

    /// The answer to an [`Action::Http`]: everything read until the server closed, or
    /// `None` if the connection failed.
    pub fn on_http_response(&mut self, token: u32, response: Option<&[u8]>, now_us: u64) {
        let parsed = response.and_then(parse_http_response);
        if let Backend::Describe { token: t, url } = &self.backend {
            if *t == token {
                let svc = parsed.as_ref().filter(|(s, _)| *s == 200)
                    .and_then(|(_, body)| core::str::from_utf8(body).ok().and_then(|x| parse_description(x, url)));
                match svc {
                    Some(svc) => {
                        let req = get_external_ip(&svc);
                        self.ext_ip_token = Some(self.http(&svc.control_url, req));
                        self.backend = Backend::Upnp(svc);
                    }
                    None => { self.backend = Backend::None; self.fail_all_pending(); }
                }
                return;
            }
        }
        let Backend::Upnp(_) = &self.backend else { return };
        if self.ext_ip_token == Some(token) {
            self.ext_ip_token = None;
            if let Some((s, body)) = &parsed {
                if let SoapResult::Ok(text) = parse_soap(*s, body) {
                    if let Some(ip) = xml_text(&text, "NewExternalIPAddress").and_then(|t| parse_ipv4(&t)) {
                        self.note_external(ip);
                        // Mappings made before we knew the address: report them again.
                        let ext = Ip::V4(ip);
                        let done: Vec<usize> = (0..self.maps.len()).filter(|&i| self.maps[i].external.is_some_and(|e| e.ip != ext)).collect();
                        for i in done {
                            let m = &self.maps[i];
                            let (port, lease) = (m.external.map_or(0, |e| e.port), m.lease_s);
                            if let Some(mm) = self.maps.get_mut(i) { mm.external = None; }
                            self.mapped(i, PeerAddr { ip: ext, port }, lease, now_us, Method::Upnp);
                        }
                    }
                }
            }
            return;
        }
        let Some(i) = self.maps.iter().position(|m| matches!(m.state, MState::Requesting { token: Some(t), .. } if t == token)) else { return };
        let MState::Requesting { tries, .. } = self.maps[i].state else { return };
        let result = parsed.map(|(s, b)| parse_soap(s, &b));
        match result {
            Some(SoapResult::Ok(_)) => {
                let m = &self.maps[i];
                let ext = PeerAddr { ip: Ip::V4(self.external_ip.unwrap_or([0; 4])), port: m.external_port };
                let lease = m.lease_s;
                self.mapped(i, ext, lease, now_us, Method::Upnp);
            }
            Some(SoapResult::Fault { code: ERR_CONFLICT, .. }) => {
                let m = &mut self.maps[i];
                m.conflicts += 1;
                if m.conflicts > MAX_CONFLICT_TRIES { return self.fail(i, Failure::Upnp(ERR_CONFLICT)); }
                // Someone else has that external port: try the next one.
                m.external_port = if m.external_port >= 65535 { 1025 } else { m.external_port + 1 };
                m.state = MState::Pending;
                self.run(now_us);
            }
            Some(SoapResult::Fault { code: ERR_ONLY_PERMANENT, .. }) if self.maps[i].lease_s != 0 => {
                self.maps[i].lease_s = 0;
                self.maps[i].state = MState::Pending;
                self.run(now_us);
            }
            Some(SoapResult::Fault { code, .. }) => self.fail(i, Failure::Upnp(code)),
            Some(SoapResult::Http(code)) => self.fail(i, Failure::Upnp(code)),
            None if tries < HTTP_TRIES => { self.maps[i].state = MState::Requesting { tries, next_us: now_us + 1_000_000, token: None }; }
            None => self.fail(i, Failure::Timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const GW: [u8; 4] = [192, 168, 1, 1];
    const ME: [u8; 4] = [192, 168, 1, 23];
    const GWA: PeerAddr = PeerAddr { ip: Ip::V4(GW), port: SERVER_PORT };

    #[test]
    fn natpmp_goldens() {
        assert_eq!(natpmp_external_request(), [0, 0]);
        assert_eq!(natpmp_map_request(Protocol::Tcp, 6881, 6881, 7200), [0, 2, 0, 0, 0x1a, 0xe1, 0x1a, 0xe1, 0, 0, 0x1c, 0x20]);
        assert_eq!(natpmp_map_request(Protocol::Udp, 6881, 0, 0), [0, 1, 0, 0, 0x1a, 0xe1, 0, 0, 0, 0, 0, 0]);
        let ext = [0, 128, 0, 0, 0, 0, 0x01, 0x2c, 203, 0, 113, 7];
        assert_eq!(parse_natpmp(&ext), Some(NatPmpResponse::External { epoch: 300, ip: [203, 0, 113, 7] }));
        let map = [0, 130, 0, 0, 0, 0, 0, 5, 0x1a, 0xe1, 0x1a, 0xe2, 0, 0, 0x1c, 0x20];
        assert_eq!(parse_natpmp(&map), Some(NatPmpResponse::Mapping { proto: Protocol::Tcp, epoch: 5, internal: 6881, external: 6882, lifetime_s: 7200 }));
        assert_eq!(parse_natpmp(&[0, 129, 0, 3, 0, 0, 0, 1]), Some(NatPmpResponse::Error { opcode: 1, result: 3, epoch: 1 }));
        // Hostile: truncated, request opcodes, wrong version, unknown opcode.
        assert_eq!(parse_natpmp(&map[..15]), None);
        assert_eq!(parse_natpmp(&[0, 1]), None);
        assert_eq!(parse_natpmp(&[1, 128, 0, 0]), None);
        assert_eq!(parse_natpmp(&[0, 200, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(parse_natpmp(&[]), None);
    }

    #[test]
    fn pcp_goldens() {
        let a = pcp_announce_request(Ip::V4(ME));
        assert_eq!(a.len(), 24);
        assert_eq!(&a[..8], &[2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&a[8..24], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 1, 23]);
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let m = pcp_map_request(Ip::V4(ME), nonce, Protocol::Tcp, 6881, 6881, None, 3600);
        assert_eq!(m.len(), 60);
        assert_eq!(&m[..8], &[2, 1, 0, 0, 0, 0, 0x0e, 0x10]);
        assert_eq!(&m[24..36], &nonce);
        assert_eq!(&m[36..44], &[6, 0, 0, 0, 0x1a, 0xe1, 0x1a, 0xe1]);
        assert_eq!(&m[44..60], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]);
        // A response: the request with R set, result, lifetime, epoch, assigned address.
        let mut r = m.clone();
        r[1] |= 0x80;
        r[3] = 0;
        r[4..8].copy_from_slice(&1800u32.to_be_bytes());
        r[8..12].copy_from_slice(&42u32.to_be_bytes());
        r[12..24].fill(0);
        r[42..44].copy_from_slice(&40000u16.to_be_bytes());
        r[56..60].copy_from_slice(&[203, 0, 113, 7]);
        assert_eq!(parse_pcp(&r), Some(PcpResponse::Map {
            result: 0, lifetime_s: 1800, epoch: 42, nonce, protocol: 6, internal: 6881, external_port: 40000, external_ip: Ip::V4([203, 0, 113, 7]),
        }));
        let mut ann = vec![2, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9];
        ann.resize(24, 0);
        assert_eq!(parse_pcp(&ann), Some(PcpResponse::Announce { result: 0, epoch: 9 }));
        // Hostile.
        assert_eq!(parse_pcp(&ann[..20]), None);
        assert_eq!(parse_pcp(&r[..56]), None);
        let mut req = a.clone();
        req[1] = 0; // a request, not a response
        assert_eq!(parse_pcp(&req), None);
        assert_eq!(parse_pcp(&[2, 0x81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None); // not /4
    }

    #[test]
    fn epoch_rules() {
        assert!(!epoch_lost_state(100, 1000, 160, 1060));
        assert!(epoch_lost_state(100, 1000, 5, 1060)); // went backwards: rebooted
        assert!(epoch_lost_state(100, 1000, 110, 1300)); // server time ran far slower
        assert!(!epoch_lost_state(100, 1000, 100, 1001));
        assert!(!epoch_lost_state(100, 1000, 99, 1000)); // one second of slack
    }

    const DESC: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
    <friendlyName>Home Router</friendlyName>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:L3Forwarding1</serviceId>
        <controlURL>/ctl/L3F</controlURL>
      </service>
    </serviceList>
    <deviceList>
      <device>
        <deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>
        <serviceList>
          <service>
            <serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType>
            <controlURL>/ctl/CmnIfCfg</controlURL>
          </service>
        </serviceList>
        <deviceList>
          <device>
            <deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>
            <serviceList>
              <service>
                <serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType>
                <controlURL>/ctl/PPPConn</controlURL>
              </service>
              <service>
                <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
                <serviceId>urn:upnp-org:serviceId:WANIPConn1</serviceId>
                <SCPDURL>/WANIPCn.xml</SCPDURL>
                <controlURL>ctl/IPConn?a=1&amp;b=2</controlURL>
                <eventSubURL>/evt/IPConn</eventSubURL>
              </service>
            </serviceList>
          </device>
        </deviceList>
      </device>
    </deviceList>
  </device>
</root>"#;

    #[test]
    fn ssdp_and_description() {
        let q = ssdp_msearch(IGD_ST);
        assert!(q.starts_with(b"M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\n"));
        assert!(q.ends_with(b"MX: 2\r\n\r\n"));
        let r = b"HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nst: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nUSN: uuid:x::urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nEXT:\r\nSERVER: Router UPnP/1.1 MiniUPnPd/2.2\r\nLocation: http://192.168.1.1:5000/rootDesc.xml\r\n\r\n";
        let s = parse_ssdp(r).unwrap();
        assert_eq!(s.location, "http://192.168.1.1:5000/rootDesc.xml");
        assert!(parse_ssdp(b"HTTP/1.1 200 OK\r\nST: urn:dial-multiscreen-org:service:dial:1\r\nLOCATION: http://x/\r\n\r\n").is_none());
        assert!(parse_ssdp(b"NOTIFY * HTTP/1.1\r\n\r\n").is_none());
        assert!(parse_ssdp(&[0xff, 0xfe]).is_none());
        let loc = Url::parse(&s.location).unwrap();
        assert_eq!(loc, Url { host: "192.168.1.1".into(), port: 5000, path: "/rootDesc.xml".into() });
        let svc = parse_description(DESC, &loc).unwrap();
        assert_eq!(svc.service_type, "urn:schemas-upnp-org:service:WANIPConnection:1");
        assert_eq!(svc.control_url, Url { host: "192.168.1.1".into(), port: 5000, path: "/ctl/IPConn?a=1&b=2".into() });
        // Namespaced tags, URLBase, and only a PPP service.
        let x = "<r:root><URLBase>http://10.0.0.1:49000/</URLBase><r:service><r:serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</r:serviceType><r:controlURL>upnp/control/WANPPPConn1</r:controlURL></r:service></r:root>";
        let svc = parse_description(x, &loc).unwrap();
        assert_eq!(svc.control_url, Url { host: "10.0.0.1".into(), port: 49000, path: "/upnp/control/WANPPPConn1".into() });
        // Hostile XML: unterminated, no services, garbage.
        assert!(parse_description("<service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1", &loc).is_none());
        assert!(parse_description("<<<<>>>></</", &loc).is_none());
        assert!(parse_description("", &loc).is_none());
        let big = "<service>".repeat(50_000);
        assert!(parse_description(&big, &loc).is_none());
    }

    #[test]
    fn urls() {
        assert_eq!(Url::parse("http://h/x").unwrap(), Url { host: "h".into(), port: 80, path: "/x".into() });
        assert_eq!(Url::parse("HTTP://h:81").unwrap().path, "/");
        assert!(Url::parse("https://h/").is_none());
        assert!(Url::parse("http://:80/").is_none());
        assert!(Url::parse("http://h:99999/").is_none());
        assert!(Url::parse("http://u@h/").is_none());
        let b = Url::parse("http://h:1/a/b.xml").unwrap();
        assert_eq!(b.join("c").unwrap().path, "/a/c");
        assert_eq!(b.join("/c").unwrap().path, "/c");
        assert_eq!(b.join("http://o:2/z").unwrap(), Url { host: "o".into(), port: 2, path: "/z".into() });
    }

    #[test]
    fn soap_and_http() {
        let svc = IgdService { service_type: WAN_SERVICES[1].into(), control_url: Url::parse("http://192.168.1.1:5000/ctl/IPConn").unwrap() };
        let req = add_port_mapping(&svc, Protocol::Tcp, 6881, 6881, ME, "Swingers <&>", 3600);
        let text = core::str::from_utf8(&req).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("POST /ctl/IPConn HTTP/1.1\r\nHost: 192.168.1.1:5000\r\n"));
        assert!(head.contains("SOAPAction: \"urn:schemas-upnp-org:service:WANIPConnection:1#AddPortMapping\""));
        assert!(head.contains(&format!("Content-Length: {}", body.len())));
        assert!(body.contains("<u:AddPortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">"));
        assert!(body.contains("<NewExternalPort>6881</NewExternalPort><NewProtocol>TCP</NewProtocol><NewInternalPort>6881</NewInternalPort><NewInternalClient>192.168.1.23</NewInternalClient>"));
        assert!(body.contains("<NewPortMappingDescription>Swingers &lt;&amp;&gt;</NewPortMappingDescription>"));
        assert!(core::str::from_utf8(&delete_port_mapping(&svc, Protocol::Udp, 7000)).unwrap().contains("<NewExternalPort>7000</NewExternalPort><NewProtocol>UDP</NewProtocol>"));
        let fault = b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/xml\r\nContent-Length: 400\r\n\r\n<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>718</errorCode><errorDescription>ConflictInMappingEntry</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>";
        let (st, b) = parse_http_response(fault).unwrap();
        assert_eq!(parse_soap(st, &b), SoapResult::Fault { code: 718, description: "ConflictInMappingEntry".into() });
        let ok = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n<s:Envelope><u:G\r\n2a\r\netExternalIPAddressResponse><NewExternalIP\r\n2e\r\nAddress>203.0.113.7</NewExternalIPAddress></u:\r\n1a\r\nGetExternalIPAddressRespon\r\n12\r\nse></s:Envelope>\r\n\r\n0\r\n\r\n";
        let (st, b) = parse_http_response(ok).unwrap();
        let SoapResult::Ok(t) = parse_soap(st, &b) else { panic!() };
        assert_eq!(xml_text(&t, "NewExternalIPAddress").as_deref(), Some("203.0.113.7"));
        assert_eq!(parse_soap(404, b"nope"), SoapResult::Http(404));
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\n").is_none());
        assert!(parse_http_response(b"garbage\r\n\r\n").is_none());
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").is_none());
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffff\r\nab").is_none());
        assert_eq!(parse_http_response(b"HTTP/1.0 200 OK\r\nContent-Length: 99\r\n\r\nshort").unwrap().1, b"short");
    }

    fn drain(pm: &mut PortMapper, now: u64) -> Vec<Action> { let mut v = Vec::new(); while let Some(a) = pm.poll_action(now) { v.push(a); } v }
    fn events(pm: &mut PortMapper) -> Vec<Event> { let mut v = Vec::new(); while let Some(e) = pm.poll_event() { v.push(e); } v }
    fn cfg() -> Config { Config { gateway: Some(GW), local_ip: ME, ..Config::default() } }

    /// A PCP server: answers ANNOUNCE and MAP.
    fn pcp_answer(req: &[u8], epoch: u32, ext_port: u16) -> Vec<u8> {
        let mut r = req.to_vec();
        r[1] |= 0x80;
        r[3] = 0;
        r[8..12].copy_from_slice(&epoch.to_be_bytes());
        r[12..24].fill(0);
        if r.len() >= 60 {
            let life = u32::from_be_bytes([req[4], req[5], req[6], req[7]]).min(1800);
            r[4..8].copy_from_slice(&life.to_be_bytes());
            r[42..44].copy_from_slice(&ext_port.to_be_bytes());
            r[44..60].copy_from_slice(&pcp_addr(Ip::V4([203, 0, 113, 7])));
        }
        r
    }

    #[test]
    fn pcp_flow_with_renewal_reboot_and_shutdown() {
        let mut pm = PortMapper::new(cfg(), 7, 0);
        let id = pm.add(Protocol::Tcp, 6881, 6881);
        let acts = drain(&mut pm, 0);
        assert_eq!(acts.len(), 1);
        let Action::Udp { to, data } = &acts[0] else { panic!() };
        assert_eq!((*to, data.len()), (GWA, 24));
        pm.on_udp(GWA, &pcp_answer(data, 1000, 0), 10_000);
        assert_eq!(pm.method(), Some(Method::Pcp));
        let acts = drain(&mut pm, 10_000);
        let Action::Udp { data: map, .. } = &acts[0] else { panic!() };
        assert_eq!(map.len(), 60);
        // A response with the wrong nonce is ignored.
        let mut bad = pcp_answer(map, 1000, 40000);
        bad[24] ^= 1;
        pm.on_udp(GWA, &bad, 20_000);
        assert!(events(&mut pm).is_empty());
        // Responses from anyone but the gateway are ignored.
        pm.on_udp(PeerAddr { ip: Ip::V4([6, 6, 6, 6]), port: SERVER_PORT }, &pcp_answer(map, 1000, 1), 20_000);
        assert!(events(&mut pm).is_empty());
        pm.on_udp(GWA, &pcp_answer(map, 1000, 40000), 20_000);
        let ev = events(&mut pm);
        assert!(ev.contains(&Event::ExternalIp(Ip::V4([203, 0, 113, 7]))));
        assert!(ev.contains(&Event::Mapped { id, protocol: Protocol::Tcp, internal_port: 6881, external: PeerAddr { ip: Ip::V4([203, 0, 113, 7]), port: 40000 }, method: Method::Pcp, lifetime_s: 1800 }));
        // Renewal at half the lifetime asks for the same external port.
        assert_eq!(pm.next_timeout(), Some(20_000 + 900_000_000));
        assert!(drain(&mut pm, 800_000_000).is_empty());
        let acts = drain(&mut pm, 900_020_000);
        let Action::Udp { data: renew, .. } = &acts[0] else { panic!() };
        assert_eq!(&renew[42..44], &40000u16.to_be_bytes());
        pm.on_udp(GWA, &pcp_answer(renew, 1000 + 900, 40000), 900_030_000);
        assert!(events(&mut pm).is_empty()); // same external address: nothing new
        // Router reboot: epoch goes back to 3, the mapping is redone.
        let mut ann = vec![2, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3];
        ann.resize(24, 0);
        pm.on_udp(GWA, &ann, 950_000_000);
        let acts = drain(&mut pm, 950_000_000);
        assert_eq!(acts.len(), 1);
        let Action::Udp { data: again, .. } = &acts[0] else { panic!() };
        pm.on_udp(GWA, &pcp_answer(again, 4, 40000), 950_010_000);
        // Shutdown deletes with lifetime 0.
        pm.shutdown();
        let acts = drain(&mut pm, 960_000_000);
        let Action::Udp { data: del, .. } = &acts[0] else { panic!() };
        assert_eq!(&del[4..8], &[0, 0, 0, 0]);
        assert!(events(&mut pm).contains(&Event::Unmapped { id }));
        assert!(pm.is_idle());
    }

    #[test]
    fn falls_back_to_natpmp_on_version_reply() {
        let mut pm = PortMapper::new(cfg(), 7, 0);
        let id = pm.add(Protocol::Udp, 6881, 0);
        drain(&mut pm, 0);
        // NAT-PMP router: "unsupported version" to our PCP announce.
        pm.on_udp(GWA, &[0, 128, 0, 1, 0, 0, 0, 50], 1000);
        let acts = drain(&mut pm, 1000);
        assert_eq!(acts, vec![Action::Udp { to: GWA, data: vec![0, 0] }]);
        pm.on_udp(GWA, &[0, 128, 0, 0, 0, 0, 0, 50, 198, 51, 100, 9], 2000);
        assert_eq!(pm.method(), Some(Method::NatPmp));
        let acts = drain(&mut pm, 2000);
        assert_eq!(acts, vec![Action::Udp { to: GWA, data: natpmp_map_request(Protocol::Udp, 6881, 6881, 3600).to_vec() }]);
        pm.on_udp(GWA, &[0, 129, 0, 0, 0, 0, 0, 51, 0x1a, 0xe1, 0x1a, 0xe1, 0, 0, 0x0e, 0x10], 3000);
        let ev = events(&mut pm);
        assert!(ev.contains(&Event::ExternalIp(Ip::V4([198, 51, 100, 9]))));
        assert!(ev.contains(&Event::Mapped { id, protocol: Protocol::Udp, internal_port: 6881, external: PeerAddr { ip: Ip::V4([198, 51, 100, 9]), port: 6881 }, method: Method::NatPmp, lifetime_s: 3600 }));
        pm.remove(id);
        assert_eq!(drain(&mut pm, 4000), vec![Action::Udp { to: GWA, data: natpmp_map_request(Protocol::Udp, 6881, 0, 0).to_vec() }]);
    }

    #[test]
    fn natpmp_refusal_fails_mapping() {
        let c = Config { pcp: false, ..cfg() };
        let mut pm = PortMapper::new(c, 1, 0);
        let id = pm.add(Protocol::Tcp, 80, 80);
        assert_eq!(drain(&mut pm, 0), vec![Action::Udp { to: GWA, data: vec![0, 0] }]);
        pm.on_udp(GWA, &[0, 128, 0, 0, 0, 0, 0, 1, 1, 2, 3, 4], 0);
        drain(&mut pm, 0);
        pm.on_udp(GWA, &[0, 130, 0, 2, 0, 0, 0, 1], 0);
        assert!(events(&mut pm).contains(&Event::Failed { id, reason: Failure::NatPmp(2) }));
    }

    /// Times out PCP and NAT-PMP, then does UPnP with a conflict and a permanent-only router.
    #[test]
    fn upnp_flow_after_timeouts() {
        let mut pm = PortMapper::new(cfg(), 3, 0);
        let id = pm.add(Protocol::Tcp, 6881, 6881);
        let mut now = 0;
        let mut ssdp = None;
        for _ in 0..200 {
            for a in drain(&mut pm, now) {
                if let Action::Udp { to, data } = a { if to == SSDP { ssdp = Some(data); } }
            }
            if ssdp.is_some() { break; }
            now = pm.next_timeout().unwrap();
        }
        assert!(now < 10_000_000, "fallback took {} us", now);
        assert!(ssdp.unwrap().starts_with(b"M-SEARCH"));
        pm.on_udp(PeerAddr { ip: Ip::V4(GW), port: 1900 }, b"HTTP/1.1 200 OK\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLOCATION: http://192.168.1.1:5000/rootDesc.xml\r\n\r\n", now);
        let acts = drain(&mut pm, now);
        let Action::Http { token, host, port, request } = &acts[0] else { panic!() };
        assert_eq!((host.as_str(), *port), ("192.168.1.1", 5000));
        assert!(request.starts_with(b"GET /rootDesc.xml HTTP/1.1\r\nHost: 192.168.1.1:5000\r\n"));
        let mut resp = b"HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\n\r\n".to_vec();
        resp.extend_from_slice(DESC.as_bytes());
        pm.on_http_response(*token, Some(&resp), now);
        assert_eq!(pm.method(), Some(Method::Upnp));
        let acts = drain(&mut pm, now);
        assert_eq!(acts.len(), 2); // GetExternalIPAddress and AddPortMapping
        let tok = |a: &Action| match a { Action::Http { token, request, .. } => (*token, String::from_utf8(request.clone()).unwrap()), _ => panic!() };
        let (t_ip, r_ip) = tok(&acts[0]);
        assert!(r_ip.contains("#GetExternalIPAddress"));
        let (t_add, r_add) = tok(&acts[1]);
        assert!(r_add.contains("<NewExternalPort>6881</NewExternalPort>"));
        pm.on_http_response(t_ip, Some(b"HTTP/1.1 200 OK\r\n\r\n<NewExternalIPAddress>203.0.113.7</NewExternalIPAddress>"), now);
        // Conflict: the next external port is tried.
        let fault = |code: u16| format!("HTTP/1.1 500 Internal Server Error\r\n\r\n<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>{}</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>", code);
        pm.on_http_response(t_add, Some(fault(718).as_bytes()), now);
        let acts = drain(&mut pm, now);
        let (t2, r2) = tok(&acts[0]);
        assert!(r2.contains("<NewExternalPort>6882</NewExternalPort>"));
        // Only permanent leases: retried with lease 0.
        pm.on_http_response(t2, Some(fault(725).as_bytes()), now);
        let acts = drain(&mut pm, now);
        let (t3, r3) = tok(&acts[0]);
        assert!(r3.contains("<NewLeaseDuration>0</NewLeaseDuration>"));
        pm.on_http_response(t3, Some(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"), now);
        let ev = events(&mut pm);
        assert!(ev.contains(&Event::Mapped { id, protocol: Protocol::Tcp, internal_port: 6881, external: PeerAddr { ip: Ip::V4([203, 0, 113, 7]), port: 6882 }, method: Method::Upnp, lifetime_s: 0 }), "{:?}", ev);
        assert_eq!(pm.next_timeout(), None); // permanent: no renewal
        // A different error fails a second mapping.
        let id2 = pm.add(Protocol::Udp, 6881, 6881);
        let acts = drain(&mut pm, now);
        let (t4, _) = tok(&acts[0]);
        pm.on_http_response(t4, Some(fault(606).as_bytes()), now);
        assert!(events(&mut pm).contains(&Event::Failed { id: id2, reason: Failure::Upnp(606) }));
        // Shutdown sends DeletePortMapping for the external port actually used.
        pm.shutdown();
        let acts = drain(&mut pm, now);
        let (_, del) = tok(&acts[0]);
        assert!(del.contains("#DeletePortMapping") && del.contains("<NewExternalPort>6882</NewExternalPort>"));
        assert!(pm.is_idle());
    }

    #[test]
    fn nothing_answers() {
        let mut pm = PortMapper::new(cfg(), 3, 0);
        let id = pm.add(Protocol::Tcp, 1, 1);
        let mut now = 0;
        for _ in 0..100 {
            drain(&mut pm, now);
            match pm.next_timeout() { Some(t) => now = t, None => break }
        }
        assert!(events(&mut pm).contains(&Event::Failed { id, reason: Failure::NoGateway }));
        assert!(now < 20_000_000);
        // Later additions fail at once.
        let id2 = pm.add(Protocol::Tcp, 2, 2);
        assert!(events(&mut pm).contains(&Event::Failed { id: id2, reason: Failure::NoGateway }));
        // No gateway known and UPnP off: fails without sending anything.
        let mut pm = PortMapper::new(Config { gateway: None, upnp: false, ..cfg() }, 1, 0);
        let id = pm.add(Protocol::Udp, 1, 1);
        assert!(drain(&mut pm, 0).is_empty());
        assert!(events(&mut pm).contains(&Event::Failed { id, reason: Failure::NoGateway }));
    }

    #[test]
    fn hostile_udp_and_http_never_panic() {
        let mut pm = PortMapper::new(cfg(), 3, 0);
        pm.add(Protocol::Tcp, 1, 1);
        let mut r: u64 = 1;
        for i in 0..5000u64 {
            r ^= r << 13; r ^= r >> 7; r ^= r << 17;
            let n = (r % 80) as usize;
            let mut v: Vec<u8> = (0..n).map(|k| (r >> (k % 56)) as u8).collect();
            if let Some(f) = v.first_mut() { *f = [0u8, 2, b'H'][(i % 3) as usize]; }
            pm.on_udp(GWA, &v, i);
            pm.on_http_response((r % 5) as u32, Some(&v), i);
            let _ = parse_http_response(&v);
            let _ = parse_description(core::str::from_utf8(&v).unwrap_or(""), &Url::parse("http://a/").unwrap());
            drain(&mut pm, i * 1000);
        }
    }
}
