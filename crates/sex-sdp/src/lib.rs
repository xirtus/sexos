// SPDX-License-Identifier: GPL-3.0-only
//! SDP (RFC 8866) as WebRTC uses it (JSEP, RFC 8829): the offers and answers a
//! peer connection trades with a signalling server.
//!
//! No I/O. `Session::parse` reads a description; `Display` writes one back with CRLF line
//! ends. Every attribute is kept in its place as text, so a parsed description writes back
//! line for line. The attributes WebRTC reads have typed forms in `attr`
//! (rtpmap, fmtp, rtcp-fb, extmap, ssrc, ssrc-group, msid, simulcast, rid, candidate,
//! fingerprint, setup, group, ice-ufrag/pwd/options, sctp-port, direction): read them with
//! `Attributes::get` and `Attributes::all`, add them with `push`.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod attr;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
pub use attr::*;

/// Why a description was refused: the line (from 1) and what was wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error { pub line: usize, pub what: &'static str }
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "SDP line {}: {}", self.line, self.what) }
}

/// `o=`: who made the description, and its version (raised on each new offer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin { pub username: String, pub session_id: u64, pub session_version: u64, pub net: String, pub family: String, pub address: String }
impl Default for Origin {
    fn default() -> Self {
        Origin { username: "-".into(), session_id: 0, session_version: 2, net: "IN".into(), family: "IP4".into(), address: "127.0.0.1".into() }
    }
}

/// `c=`: network, family and address (with any `/ttl` kept as written).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection { pub net: String, pub family: String, pub address: String }
impl Connection {
    /// The `c=IN IP4 0.0.0.0` every WebRTC section carries.
    pub fn none() -> Self { Connection { net: "IN".into(), family: "IP4".into(), address: "0.0.0.0".into() } }
}

/// `b=AS:2000`, `b=TIAS:...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bandwidth { pub kind: String, pub value: u64 }

/// `t=` and the `r=` lines after it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Timing { pub start: u64, pub stop: u64, pub repeats: Vec<String> }

/// One `a=` line: `a=name:value`, or `a=name` for a flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute { pub name: String, pub value: Option<String> }
impl Attribute {
    pub fn new(name: &str, value: Option<&str>) -> Self { Attribute { name: name.into(), value: value.map(Into::into) } }
    pub fn flag(name: &str) -> Self { Attribute { name: name.into(), value: None } }
    fn parse(s: &str) -> Self {
        match s.split_once(':') {
            Some((n, v)) => Attribute { name: n.into(), value: Some(v.into()) },
            None => Attribute { name: s.into(), value: None },
        }
    }
}

/// The whole description.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Session {
    pub version: u32,
    pub origin: Origin,
    pub name: String,
    pub info: Option<String>,
    pub uri: Option<String>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub connection: Option<Connection>,
    pub bandwidths: Vec<Bandwidth>,
    pub timing: Vec<Timing>,
    pub zone: Option<String>,
    pub key: Option<String>,
    pub attributes: Vec<Attribute>,
    pub media: Vec<Media>,
}

/// One `m=` section and its lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media {
    /// audio, video or application.
    pub kind: String,
    pub port: u16,
    /// The `/n` after the port, if written.
    pub ports: Option<u16>,
    /// UDP/TLS/RTP/SAVPF for media, UDP/DTLS/SCTP for data channels.
    pub proto: String,
    /// Payload types (or `webrtc-datachannel`).
    pub formats: Vec<String>,
    pub info: Option<String>,
    pub connections: Vec<Connection>,
    pub bandwidths: Vec<Bandwidth>,
    pub key: Option<String>,
    pub attributes: Vec<Attribute>,
}

impl Media {
    /// A section on the discard port 9 with `c=IN IP4 0.0.0.0`, as JSEP writes them.
    pub fn new(kind: &str, proto: &str, formats: &[&str]) -> Self {
        Media {
            kind: kind.into(), port: 9, ports: None, proto: proto.into(),
            formats: formats.iter().map(|f| (*f).into()).collect(),
            info: None, connections: alloc::vec![Connection::none()], bandwidths: Vec::new(), key: None, attributes: Vec::new(),
        }
    }
    /// The section's `a=mid`.
    pub fn mid(&self) -> Option<&str> { self.value("mid") }
    /// Port 0: a section that was rejected or stopped.
    pub fn rejected(&self) -> bool { self.port == 0 }
    /// The payload types as numbers (RTP sections).
    pub fn payload_types(&self) -> Vec<u8> { self.formats.iter().filter_map(|f| f.parse().ok()).collect() }
    /// Each payload type with its rtpmap, fmtp and rtcp-fb lines, in `m=` order.
    pub fn codecs(&self) -> Vec<Codec> {
        let maps: Vec<Rtpmap> = self.all();
        let fmtps: Vec<Fmtp> = self.all();
        let fbs: Vec<RtcpFb> = self.all();
        self.payload_types().into_iter().map(|pt| Codec {
            pt,
            rtpmap: maps.iter().find(|m| m.pt == pt).cloned(),
            fmtp: fmtps.iter().find(|f| f.pt == pt).map(|f| f.params.clone()),
            feedback: fbs.iter().filter(|f| f.pt.map_or(true, |p| p == pt)).cloned().collect(),
        }).collect()
    }
}

/// One payload type of a section, gathered from its lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Codec { pub pt: u8, pub rtpmap: Option<Rtpmap>, pub fmtp: Option<String>, pub feedback: Vec<RtcpFb> }
impl Codec {
    pub fn name(&self) -> Option<&str> { self.rtpmap.as_ref().map(|r| r.encoding.as_str()) }
}

/// Reading and adding attributes, the same at session and media level.
pub trait Attributes {
    fn attributes(&self) -> &[Attribute];
    fn attributes_mut(&mut self) -> &mut Vec<Attribute>;
    /// The first `a=name:value`'s value.
    fn value(&self, name: &str) -> Option<&str> {
        self.attributes().iter().find(|a| a.name == name).and_then(|a| a.value.as_deref())
    }
    fn has(&self, name: &str) -> bool { self.attributes().iter().any(|a| a.name == name) }
    /// The first attribute of this type that parses.
    fn get<T: Attr>(&self) -> Option<T> {
        self.attributes().iter().filter(|a| a.name == T::NAME).find_map(|a| T::parse(a.value.as_deref()))
    }
    /// Every attribute of this type that parses, in order.
    fn all<T: Attr>(&self) -> Vec<T> {
        self.attributes().iter().filter(|a| a.name == T::NAME).filter_map(|a| T::parse(a.value.as_deref())).collect()
    }
    /// sendrecv, sendonly, recvonly or inactive, if one is written here.
    fn direction(&self) -> Option<Direction> { self.attributes().iter().find_map(|a| Direction::from_name(&a.name)) }
    /// Sets the direction, replacing any written one.
    fn set_direction(&mut self, d: Direction) {
        let v = self.attributes_mut();
        match v.iter().position(|a| Direction::from_name(&a.name).is_some()) {
            Some(i) => v[i] = Attribute::flag(d.as_str()),
            None => v.push(Attribute::flag(d.as_str())),
        }
    }
    fn push<T: Attr>(&mut self, a: T) { let value = a.value(); self.attributes_mut().push(Attribute { name: T::NAME.into(), value }); }
    fn push_flag(&mut self, name: &str) { self.attributes_mut().push(Attribute::flag(name)); }
    /// Drops every attribute named `name`.
    fn remove(&mut self, name: &str) { self.attributes_mut().retain(|a| a.name != name); }
}
impl Attributes for Session {
    fn attributes(&self) -> &[Attribute] { &self.attributes }
    fn attributes_mut(&mut self) -> &mut Vec<Attribute> { &mut self.attributes }
}
impl Attributes for Media {
    fn attributes(&self) -> &[Attribute] { &self.attributes }
    fn attributes_mut(&mut self) -> &mut Vec<Attribute> { &mut self.attributes }
}

fn num<T: core::str::FromStr>(s: Option<&str>, line: usize, what: &'static str) -> Result<T, Error> {
    s.and_then(|s| s.parse().ok()).ok_or(Error { line, what })
}

fn connection(v: &str, line: usize) -> Result<Connection, Error> {
    let mut w = v.split_ascii_whitespace();
    match (w.next(), w.next(), w.next()) {
        (Some(n), Some(f), Some(a)) => Ok(Connection { net: n.into(), family: f.into(), address: a.into() }),
        _ => Err(Error { line, what: "c= needs network, family and address" }),
    }
}
fn bandwidth(v: &str, line: usize) -> Result<Bandwidth, Error> {
    let (k, n) = v.split_once(':').ok_or(Error { line, what: "b= needs type:value" })?;
    Ok(Bandwidth { kind: k.into(), value: num(Some(n.trim()), line, "b= value is not a number")? })
}

impl Session {
    /// An empty description as JSEP starts one: `v=0`, `s=-`, `t=0 0`.
    pub fn new(session_id: u64) -> Self {
        Session {
            origin: Origin { session_id, ..Origin::default() }, name: "-".into(),
            timing: alloc::vec![Timing::default()], ..Session::default()
        }
    }

    /// Reads a description. Lines may end in CRLF or LF. Unknown attributes are kept.
    pub fn parse(text: &str) -> Result<Session, Error> {
        let mut s = Session { name: String::new(), ..Session::default() };
        let mut seen_v = false;
        let mut seen_o = false;
        for (i, raw) in text.split('\n').enumerate() {
            let line = i + 1;
            let l = raw.strip_suffix('\r').unwrap_or(raw);
            if l.trim().is_empty() { continue; }
            let b = l.as_bytes();
            if b.len() < 2 || b[1] != b'=' { return Err(Error { line, what: "a line is type=value" }); }
            let v = &l[2..];
            if let Some(m) = s.media.last_mut() {
                match b[0] {
                    b'i' => m.info = Some(v.into()),
                    b'c' => m.connections.push(connection(v, line)?),
                    b'b' => m.bandwidths.push(bandwidth(v, line)?),
                    b'k' => m.key = Some(v.into()),
                    b'a' => m.attributes.push(Attribute::parse(v)),
                    b'm' => s.media.push(media(v, line)?),
                    _ => return Err(Error { line, what: "this line can't be in a media section" }),
                }
                continue;
            }
            match b[0] {
                b'v' => { s.version = num(Some(v), line, "v= is not a number")?; seen_v = true; }
                b'o' => { s.origin = origin(v, line)?; seen_o = true; }
                b's' => s.name = v.into(),
                b'i' => s.info = Some(v.into()),
                b'u' => s.uri = Some(v.into()),
                b'e' => s.emails.push(v.into()),
                b'p' => s.phones.push(v.into()),
                b'c' => s.connection = Some(connection(v, line)?),
                b'b' => s.bandwidths.push(bandwidth(v, line)?),
                b't' => {
                    let mut w = v.split_ascii_whitespace();
                    let start = num(w.next(), line, "t= needs start and stop")?;
                    let stop = num(w.next(), line, "t= needs start and stop")?;
                    s.timing.push(Timing { start, stop, repeats: Vec::new() });
                }
                b'r' => s.timing.last_mut().ok_or(Error { line, what: "r= before t=" })?.repeats.push(v.into()),
                b'z' => s.zone = Some(v.into()),
                b'k' => s.key = Some(v.into()),
                b'a' => s.attributes.push(Attribute::parse(v)),
                b'm' => s.media.push(media(v, line)?),
                _ => return Err(Error { line, what: "unknown line type" }),
            }
        }
        if !seen_v || !seen_o { return Err(Error { line: 1, what: "v= and o= are required" }); }
        Ok(s)
    }

    /// The groups (`a=group:BUNDLE 0 1 2`).
    pub fn groups(&self) -> Vec<Group> { self.all() }
    /// The first BUNDLE group.
    pub fn bundle(&self) -> Option<Group> { self.groups().into_iter().find(|g| g.semantics == "BUNDLE") }
    pub fn media_by_mid(&self, mid: &str) -> Option<&Media> { self.media.iter().find(|m| m.mid() == Some(mid)) }
    /// A section's ICE username fragment and password, falling back to the session's.
    pub fn ice_credentials(&self, m: &Media) -> Option<(String, String)> {
        let u = m.get::<IceUfrag>().or_else(|| self.get())?;
        let p = m.get::<IcePwd>().or_else(|| self.get())?;
        Some((u.0, p.0))
    }
    /// A section's DTLS fingerprint, falling back to the session's.
    pub fn fingerprint(&self, m: &Media) -> Option<Fingerprint> { m.get().or_else(|| self.get()) }
    /// A section's DTLS role, falling back to the session's.
    pub fn setup(&self, m: &Media) -> Option<Setup> { m.get().or_else(|| self.get()) }
    /// True when the session says it is an ICE lite agent (as most SFUs are).
    pub fn ice_lite(&self) -> bool { self.has("ice-lite") }
}

fn origin(v: &str, line: usize) -> Result<Origin, Error> {
    let w: Vec<&str> = v.split_ascii_whitespace().collect();
    if w.len() != 6 { return Err(Error { line, what: "o= needs six fields" }); }
    Ok(Origin {
        username: w[0].into(), session_id: num(Some(w[1]), line, "o= session id is not a number")?,
        session_version: num(Some(w[2]), line, "o= version is not a number")?,
        net: w[3].into(), family: w[4].into(), address: w[5].into(),
    })
}

fn media(v: &str, line: usize) -> Result<Media, Error> {
    let mut w = v.split_ascii_whitespace();
    let kind = w.next().ok_or(Error { line, what: "m= needs media, port and protocol" })?;
    let port = w.next().ok_or(Error { line, what: "m= needs media, port and protocol" })?;
    let proto = w.next().ok_or(Error { line, what: "m= needs media, port and protocol" })?;
    let (port, ports) = match port.split_once('/') {
        Some((p, n)) => (num(Some(p), line, "m= port")?, Some(num(Some(n), line, "m= port count")?)),
        None => (num(Some(port), line, "m= port")?, None),
    };
    Ok(Media {
        kind: kind.into(), port, ports, proto: proto.into(), formats: w.map(|f| f.to_string()).collect(),
        info: None, connections: Vec::new(), bandwidths: Vec::new(), key: None, attributes: Vec::new(),
    })
}

impl fmt::Display for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.value { Some(v) => write!(f, "a={}:{}\r\n", self.name, v), None => write!(f, "a={}\r\n", self.name) }
    }
}
impl fmt::Display for Connection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "c={} {} {}\r\n", self.net, self.family, self.address) }
}
impl fmt::Display for Bandwidth {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "b={}:{}\r\n", self.kind, self.value) }
}
impl fmt::Display for Media {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "m={} {}", self.kind, self.port)?;
        if let Some(n) = self.ports { write!(f, "/{}", n)?; }
        write!(f, " {}", self.proto)?;
        for x in &self.formats { write!(f, " {}", x)?; }
        f.write_str("\r\n")?;
        if let Some(i) = &self.info { write!(f, "i={}\r\n", i)?; }
        for c in &self.connections { c.fmt(f)?; }
        for b in &self.bandwidths { b.fmt(f)?; }
        if let Some(k) = &self.key { write!(f, "k={}\r\n", k)?; }
        for a in &self.attributes { a.fmt(f)?; }
        Ok(())
    }
}
impl fmt::Display for Session {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let o = &self.origin;
        write!(f, "v={}\r\no={} {} {} {} {} {}\r\ns={}\r\n", self.version, o.username, o.session_id, o.session_version, o.net, o.family, o.address, self.name)?;
        if let Some(i) = &self.info { write!(f, "i={}\r\n", i)?; }
        if let Some(u) = &self.uri { write!(f, "u={}\r\n", u)?; }
        for e in &self.emails { write!(f, "e={}\r\n", e)?; }
        for p in &self.phones { write!(f, "p={}\r\n", p)?; }
        if let Some(c) = &self.connection { c.fmt(f)?; }
        for b in &self.bandwidths { b.fmt(f)?; }
        for t in &self.timing {
            write!(f, "t={} {}\r\n", t.start, t.stop)?;
            for r in &t.repeats { write!(f, "r={}\r\n", r)?; }
        }
        if let Some(z) = &self.zone { write!(f, "z={}\r\n", z)?; }
        if let Some(k) = &self.key { write!(f, "k={}\r\n", k)?; }
        for a in &self.attributes { a.fmt(f)?; }
        for m in &self.media { m.fmt(f)?; }
        Ok(())
    }
}
