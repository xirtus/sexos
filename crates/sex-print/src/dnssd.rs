// SPDX-License-Identifier: GPL-3.0-only
//! DNS-SD over mDNS (RFC 6762, RFC 6763): the query that browses for printers and
//! scanners, and reading the answers into services with their address, port and
//! TXT keys (`rp`, `pdl`, `URF`, `Color`, `Duplex`, `UUID`, `rs`, `ty`, ...).
//!
//! The same packets work as a legacy unicast query sent straight to one host's port
//! 5353 (RFC 6762 6.7), which is how Needle discovers when multicast is unavailable.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const MDNS_PORT: u16 = 5353;
pub const MDNS_V4: [u8; 4] = [224, 0, 0, 251];

pub const IPP: &str = "_ipp._tcp.local";
pub const IPPS: &str = "_ipps._tcp.local";
pub const USCAN: &str = "_uscan._tcp.local";
pub const USCANS: &str = "_uscans._tcp.local";

pub mod rtype {
    pub const A: u16 = 1;
    pub const PTR: u16 = 12;
    pub const TXT: u16 = 16;
    pub const AAAA: u16 = 28;
    pub const SRV: u16 = 33;
    pub const ANY: u16 = 255;
}

/// Labels of a dotted name; `\.` keeps a dot inside a label (instance names).
fn labels(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => if let Some(n) = chars.next() { cur.push(n) },
            '.' => { out.push(core::mem::take(&mut cur)); }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

fn put_name(out: &mut Vec<u8>, name: &str) {
    for l in labels(name) {
        let b = l.as_bytes();
        out.push(b.len().min(63) as u8);
        out.extend_from_slice(&b[..b.len().min(63)]);
    }
    out.push(0);
}

/// A query packet. `unicast_response` sets the QU bit (answer me directly).
pub fn query(id: u16, questions: &[(&str, u16)], unicast_response: bool) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(questions.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    for (name, t) in questions {
        put_name(&mut out, name);
        out.extend_from_slice(&t.to_be_bytes());
        out.extend_from_slice(&(1u16 | if unicast_response { 0x8000 } else { 0 }).to_be_bytes());
    }
    out
}

/// The browse Needle sends: printers (IPP, IPPS) and scanners (eSCL, eSCL over TLS).
pub fn browse_query(id: u16, unicast_response: bool) -> Vec<u8> {
    query(id, &[(IPP, rtype::PTR), (IPPS, rtype::PTR), (USCAN, rtype::PTR), (USCANS, rtype::PTR)], unicast_response)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RData {
    Ptr(String),
    Srv { priority: u16, weight: u16, port: u16, target: String },
    Txt(Vec<Vec<u8>>),
    A([u8; 4]),
    Aaaa([u8; 16]),
    Other(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    /// Without the cache-flush bit.
    pub class: u16,
    pub ttl: u32,
    pub data: RData,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub id: u16,
    pub flags: u16,
    pub questions: Vec<(String, u16)>,
    /// Answers, authority and additional records together.
    pub records: Vec<Record>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Truncated, BadName }

fn u16_at(b: &[u8], at: usize) -> Result<u16, Error> {
    Ok(u16::from_be_bytes([*b.get(at).ok_or(Error::Truncated)?, *b.get(at + 1).ok_or(Error::Truncated)?]))
}

/// A (possibly compressed) name at `at`; returns it and where the name ends.
fn read_name(b: &[u8], mut at: usize) -> Result<(String, usize), Error> {
    let mut name = String::new();
    let mut end = None;
    for _ in 0..128 {
        let len = *b.get(at).ok_or(Error::Truncated)? as usize;
        if len == 0 {
            return Ok((name, end.unwrap_or(at + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let ptr = (u16_at(b, at)? & 0x3FFF) as usize;
            if end.is_none() { end = Some(at + 2); }
            at = ptr;
            continue;
        }
        if len & 0xC0 != 0 { return Err(Error::BadName); }
        let l = b.get(at + 1..at + 1 + len).ok_or(Error::Truncated)?;
        if !name.is_empty() { name.push('.'); }
        for &c in l {
            if c == b'.' || c == b'\\' { name.push('\\'); }
            name.push(c as char);
        }
        at += 1 + len;
    }
    Err(Error::BadName)
}

impl Packet {
    pub fn parse(b: &[u8]) -> Result<Packet, Error> {
        let id = u16_at(b, 0)?;
        let flags = u16_at(b, 2)?;
        let counts = [u16_at(b, 4)?, u16_at(b, 6)?, u16_at(b, 8)?, u16_at(b, 10)?];
        let mut at = 12;
        let mut questions = Vec::new();
        for _ in 0..counts[0] {
            let (n, e) = read_name(b, at)?;
            questions.push((n, u16_at(b, e)?));
            at = e + 4;
        }
        let mut records = Vec::new();
        for _ in 0..counts[1] as usize + counts[2] as usize + counts[3] as usize {
            let (name, e) = read_name(b, at)?;
            let rtype = u16_at(b, e)?;
            let class = u16_at(b, e + 2)? & 0x7FFF;
            let ttl = ((u16_at(b, e + 4)? as u32) << 16) | u16_at(b, e + 6)? as u32;
            let len = u16_at(b, e + 8)? as usize;
            let start = e + 10;
            let rd = b.get(start..start + len).ok_or(Error::Truncated)?;
            let data = match rtype {
                rtype::PTR => RData::Ptr(read_name(b, start)?.0),
                rtype::SRV if len >= 7 => RData::Srv {
                    priority: u16_at(rd, 0)?, weight: u16_at(rd, 2)?, port: u16_at(rd, 4)?, target: read_name(b, start + 6)?.0,
                },
                rtype::TXT => {
                    let mut items = Vec::new();
                    let mut i = 0;
                    while i < rd.len() {
                        let n = rd[i] as usize;
                        items.push(rd.get(i + 1..i + 1 + n).ok_or(Error::Truncated)?.to_vec());
                        i += 1 + n;
                    }
                    RData::Txt(items)
                }
                rtype::A if len == 4 => RData::A([rd[0], rd[1], rd[2], rd[3]]),
                rtype::AAAA if len == 16 => { let mut a = [0u8; 16]; a.copy_from_slice(rd); RData::Aaaa(a) }
                _ => RData::Other(rd.to_vec()),
            };
            records.push(Record { name, rtype, class, ttl, data });
            at = start + len;
        }
        Ok(Packet { id, flags, questions, records })
    }

    pub fn is_response(&self) -> bool { self.flags & 0x8000 != 0 }
}

/// One advertised printer or scanner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Service {
    /// Full instance name, e.g. `Office Printer._ipp._tcp.local`.
    pub instance: String,
    /// `_ipp._tcp.local` ...
    pub service: String,
    pub host: String,
    pub port: u16,
    pub txt: Vec<(String, String)>,
    pub v4: Vec<[u8; 4]>,
    pub v6: Vec<[u8; 16]>,
}

impl Service {
    /// The person-facing name: the instance label, unescaped.
    pub fn name(&self) -> String { labels(&self.instance).into_iter().next().unwrap_or_default() }
    /// A TXT value by key (keys are case-insensitive, RFC 6763 6.4).
    pub fn txt(&self, key: &str) -> Option<&str> { self.txt.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str()) }
    pub fn is_printer(&self) -> bool { self.service.eq_ignore_ascii_case(IPP) || self.service.eq_ignore_ascii_case(IPPS) }
    pub fn is_scanner(&self) -> bool { self.service.eq_ignore_ascii_case(USCAN) || self.service.eq_ignore_ascii_case(USCANS) }

    /// The address to use: the first IPv4 address, else IPv6, else the host name.
    pub fn address(&self) -> String {
        if let Some(a) = self.v4.first() { return format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]); }
        if let Some(a) = self.v6.first() {
            let groups: Vec<String> = a.chunks(2).map(|c| format!("{:x}", u16::from_be_bytes([c[0], c[1]]))).collect();
            return format!("[{}]", groups.join(":"));
        }
        self.host.trim_end_matches('.').to_string()
    }

    /// `ipp://addr:port/rp` for printers, `http://addr:port/rs` for scanners.
    pub fn uri(&self) -> String {
        let s = self.service.to_ascii_lowercase();
        let (scheme, key, default) = if s.starts_with("_ipps") { ("ipps", "rp", "ipp/print") } else if s.starts_with("_ipp") { ("ipp", "rp", "ipp/print") }
            else if s.starts_with("_uscans") { ("https", "rs", "eSCL") } else { ("http", "rs", "eSCL") };
        let path = self.txt(key).unwrap_or(default).trim_start_matches('/');
        format!("{scheme}://{}:{}/{path}", self.address(), self.port)
    }
}

/// Every service of the browsed types the packet describes. Instances whose SRV
/// wasn't included come back with port 0 (ask again for `instance` SRV/TXT).
pub fn services(p: &Packet) -> Vec<Service> {
    let wanted = [IPP, IPPS, USCAN, USCANS];
    let mut out: Vec<Service> = Vec::new();
    for r in &p.records {
        if let RData::Ptr(instance) = &r.data {
            if wanted.iter().any(|w| w.eq_ignore_ascii_case(&r.name)) && !out.iter().any(|s| s.instance.eq_ignore_ascii_case(instance)) {
                out.push(Service { instance: instance.clone(), service: r.name.clone(), ..Default::default() });
            }
        }
    }
    // A packet answering an SRV/TXT question directly, with no PTR.
    for r in &p.records {
        if matches!(r.data, RData::Srv { .. }) && !out.iter().any(|s| s.instance.eq_ignore_ascii_case(&r.name)) {
            if let Some(w) = wanted.iter().find(|w| r.name.to_ascii_lowercase().ends_with(&format!(".{}", w.to_ascii_lowercase()))) {
                out.push(Service { instance: r.name.clone(), service: w.to_string(), ..Default::default() });
            }
        }
    }
    for s in out.iter_mut() { fill(s, p); }
    out
}

/// Fills SRV, TXT and addresses for `s` from `p` (also for a follow-up answer).
pub fn fill(s: &mut Service, p: &Packet) {
    for r in &p.records {
        if !r.name.eq_ignore_ascii_case(&s.instance) { continue; }
        match &r.data {
            RData::Srv { port, target, .. } => { s.port = *port; s.host = target.clone(); }
            RData::Txt(items) => {
                s.txt = items.iter().filter(|i| !i.is_empty()).map(|i| {
                    let t = String::from_utf8_lossy(i);
                    match t.split_once('=') { Some((k, v)) => (k.to_string(), v.to_string()), None => (t.to_string(), String::new()) }
                }).collect();
            }
            _ => {}
        }
    }
    for r in &p.records {
        if s.host.is_empty() || !r.name.eq_ignore_ascii_case(&s.host) { continue; }
        match r.data {
            RData::A(a) if !s.v4.contains(&a) => s.v4.push(a),
            RData::Aaaa(a) if !s.v6.contains(&a) => s.v6.push(a),
            _ => {}
        }
    }
}
