// SPDX-License-Identifier: GPL-3.0-only
//! Typed forms of the attributes WebRTC reads and writes. Each one parses from the text
//! after `a=name:` and writes the same text back.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

/// An attribute with a typed value.
pub trait Attr: Sized {
    const NAME: &'static str;
    /// From the value after the colon (None for a flag).
    fn parse(value: Option<&str>) -> Option<Self>;
    /// The value to write after the colon (None for a flag).
    fn value(&self) -> Option<String>;
}

macro_rules! text_attr {
    ($(#[$m:meta])* $t:ident, $name:literal) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $t(pub String);
        impl Attr for $t {
            const NAME: &'static str = $name;
            fn parse(v: Option<&str>) -> Option<Self> { v.map(|v| $t(v.trim().into())) }
            fn value(&self) -> Option<String> { Some(self.0.clone()) }
        }
    };
}
text_attr!(/// `a=mid:0`.
    Mid, "mid");
text_attr!(/// `a=ice-ufrag:`.
    IceUfrag, "ice-ufrag");
text_attr!(/// `a=ice-pwd:`.
    IcePwd, "ice-pwd");

macro_rules! num_attr {
    ($(#[$m:meta])* $t:ident, $ty:ty, $name:literal) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $t(pub $ty);
        impl Attr for $t {
            const NAME: &'static str = $name;
            fn parse(v: Option<&str>) -> Option<Self> { v?.trim().parse().ok().map($t) }
            fn value(&self) -> Option<String> { Some(self.0.to_string()) }
        }
    };
}
num_attr!(/// `a=sctp-port:5000`.
    SctpPort, u16, "sctp-port");
num_attr!(/// `a=max-message-size:262144`.
    MaxMessageSize, u64, "max-message-size");
num_attr!(/// `a=ptime:20`.
    Ptime, u32, "ptime");
num_attr!(/// `a=maxptime:120`.
    MaxPtime, u32, "maxptime");

fn words(v: Option<&str>) -> impl Iterator<Item = &str> { v.unwrap_or("").split_ascii_whitespace() }

/// `a=ice-options:trickle renomination`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IceOptions(pub Vec<String>);
impl IceOptions { pub fn trickle(&self) -> bool { self.0.iter().any(|o| o == "trickle") } }
impl Attr for IceOptions {
    const NAME: &'static str = "ice-options";
    fn parse(v: Option<&str>) -> Option<Self> { Some(IceOptions(words(v).map(Into::into).collect())) }
    fn value(&self) -> Option<String> { Some(self.0.join(" ")) }
}

/// `a=group:BUNDLE 0 1 2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group { pub semantics: String, pub mids: Vec<String> }
impl Attr for Group {
    const NAME: &'static str = "group";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        Some(Group { semantics: w.next()?.into(), mids: w.map(Into::into).collect() })
    }
    fn value(&self) -> Option<String> {
        let mut s = self.semantics.clone();
        for m in &self.mids { s.push(' '); s.push_str(m); }
        Some(s)
    }
}

/// `a=msid-semantic: WMS *` (Chrome) or `a=msid-semantic:WMS *` (Firefox).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MsidSemantic { pub semantics: String, pub ids: Vec<String> }
impl Attr for MsidSemantic {
    const NAME: &'static str = "msid-semantic";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        Some(MsidSemantic { semantics: w.next()?.into(), ids: w.map(Into::into).collect() })
    }
    fn value(&self) -> Option<String> {
        let mut s = format!(" {}", self.semantics);
        for m in &self.ids { s.push(' '); s.push_str(m); }
        Some(s)
    }
}

/// `a=fingerprint:sha-256 AB:CD:...`, the DTLS certificate's hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint { pub algorithm: String, pub bytes: Vec<u8> }
impl Attr for Fingerprint {
    const NAME: &'static str = "fingerprint";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        let algorithm = w.next()?.to_ascii_lowercase();
        let hex = w.next()?;
        let mut bytes = Vec::new();
        for pair in hex.split(':') {
            if pair.len() != 2 { return None; }
            bytes.push(u8::from_str_radix(pair, 16).ok()?);
        }
        Some(Fingerprint { algorithm, bytes })
    }
    fn value(&self) -> Option<String> {
        let mut s = format!("{} ", self.algorithm);
        for (i, b) in self.bytes.iter().enumerate() {
            if i > 0 { s.push(':'); }
            let _ = write!(s, "{:02X}", b);
        }
        Some(s)
    }
}

/// `a=setup:`: which side starts DTLS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup { Active, Passive, Actpass, Holdconn }
impl Attr for Setup {
    const NAME: &'static str = "setup";
    fn parse(v: Option<&str>) -> Option<Self> {
        Some(match v?.trim() { "active" => Setup::Active, "passive" => Setup::Passive, "actpass" => Setup::Actpass, "holdconn" => Setup::Holdconn, _ => return None })
    }
    fn value(&self) -> Option<String> {
        Some(match self { Setup::Active => "active", Setup::Passive => "passive", Setup::Actpass => "actpass", Setup::Holdconn => "holdconn" }.into())
    }
}

/// `a=sendrecv` and the other three flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { SendRecv, SendOnly, RecvOnly, Inactive }
impl Direction {
    pub fn from_name(n: &str) -> Option<Self> {
        Some(match n { "sendrecv" => Direction::SendRecv, "sendonly" => Direction::SendOnly, "recvonly" => Direction::RecvOnly, "inactive" => Direction::Inactive, _ => return None })
    }
    pub fn as_str(self) -> &'static str {
        match self { Direction::SendRecv => "sendrecv", Direction::SendOnly => "sendonly", Direction::RecvOnly => "recvonly", Direction::Inactive => "inactive" }
    }
    /// The direction an answer takes to this offered one.
    pub fn reverse(self) -> Self {
        match self { Direction::SendOnly => Direction::RecvOnly, Direction::RecvOnly => Direction::SendOnly, d => d }
    }
    pub fn sends(self) -> bool { matches!(self, Direction::SendRecv | Direction::SendOnly) }
    pub fn receives(self) -> bool { matches!(self, Direction::SendRecv | Direction::RecvOnly) }
}

/// `a=rtpmap:111 opus/48000/2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rtpmap { pub pt: u8, pub encoding: String, pub clock: u32, pub channels: Option<u16> }
impl Attr for Rtpmap {
    const NAME: &'static str = "rtpmap";
    fn parse(v: Option<&str>) -> Option<Self> {
        let (pt, rest) = v?.trim().split_once(' ')?;
        let mut p = rest.trim().split('/');
        Some(Rtpmap {
            pt: pt.parse().ok()?, encoding: p.next()?.into(), clock: p.next()?.parse().ok()?,
            channels: match p.next() { Some(c) => Some(c.parse().ok()?), None => None },
        })
    }
    fn value(&self) -> Option<String> {
        let mut s = format!("{} {}/{}", self.pt, self.encoding, self.clock);
        if let Some(c) = self.channels { let _ = write!(s, "/{}", c); }
        Some(s)
    }
}

/// `a=fmtp:111 minptime=10;useinbandfec=1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fmtp { pub pt: u8, pub params: String }
impl Fmtp {
    /// The `key=value` pairs; a part with no `=` comes back with an empty value.
    pub fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.split(';').map(str::trim).filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, "")))
    }
    pub fn param(&self, key: &str) -> Option<&str> { self.pairs().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v) }
}
impl Attr for Fmtp {
    const NAME: &'static str = "fmtp";
    fn parse(v: Option<&str>) -> Option<Self> {
        let (pt, rest) = v?.trim().split_once(' ')?;
        Some(Fmtp { pt: pt.parse().ok()?, params: rest.trim().into() })
    }
    fn value(&self) -> Option<String> { Some(format!("{} {}", self.pt, self.params)) }
}

/// `a=rtcp-fb:96 nack pli`; `pt` None for `*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtcpFb { pub pt: Option<u8>, pub kind: String, pub param: Option<String> }
impl Attr for RtcpFb {
    const NAME: &'static str = "rtcp-fb";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        let pt = match w.next()? { "*" => None, p => Some(p.parse().ok()?) };
        let kind = w.next()?.into();
        let rest: Vec<&str> = w.collect();
        Some(RtcpFb { pt, kind, param: if rest.is_empty() { None } else { Some(rest.join(" ")) } })
    }
    fn value(&self) -> Option<String> {
        let mut s = match self.pt { Some(p) => p.to_string(), None => "*".into() };
        s.push(' '); s.push_str(&self.kind);
        if let Some(p) = &self.param { s.push(' '); s.push_str(p); }
        Some(s)
    }
}

/// `a=extmap:3/sendrecv urn:ietf:params:rtp-hdrext:sdes:mid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extmap { pub id: u16, pub direction: Option<Direction>, pub uri: String, pub attributes: Option<String> }
impl Attr for Extmap {
    const NAME: &'static str = "extmap";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        let head = w.next()?;
        let (id, direction) = match head.split_once('/') { Some((i, d)) => (i, Some(Direction::from_name(d)?)), None => (head, None) };
        let uri = w.next()?.into();
        let rest: Vec<&str> = w.collect();
        Some(Extmap { id: id.parse().ok()?, direction, uri, attributes: if rest.is_empty() { None } else { Some(rest.join(" ")) } })
    }
    fn value(&self) -> Option<String> {
        let mut s = self.id.to_string();
        if let Some(d) = self.direction { s.push('/'); s.push_str(d.as_str()); }
        s.push(' '); s.push_str(&self.uri);
        if let Some(a) = &self.attributes { s.push(' '); s.push_str(a); }
        Some(s)
    }
}

/// `a=ssrc:1234 cname:abc` or `a=ssrc:1234 msid:stream track`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ssrc { pub ssrc: u32, pub attribute: String, pub value: Option<String> }
impl Attr for Ssrc {
    const NAME: &'static str = "ssrc";
    fn parse(v: Option<&str>) -> Option<Self> {
        let (id, rest) = v?.trim().split_once(' ')?;
        let (attribute, value) = match rest.split_once(':') { Some((a, b)) => (a.into(), Some(b.into())), None => (rest.into(), None) };
        Some(Ssrc { ssrc: id.parse().ok()?, attribute, value })
    }
    fn value(&self) -> Option<String> {
        Some(match &self.value { Some(v) => format!("{} {}:{}", self.ssrc, self.attribute, v), None => format!("{} {}", self.ssrc, self.attribute) })
    }
}

/// `a=ssrc-group:FID 1234 5678` (RTX) or `SIM` (legacy simulcast).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SsrcGroup { pub semantics: String, pub ssrcs: Vec<u32> }
impl Attr for SsrcGroup {
    const NAME: &'static str = "ssrc-group";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        let semantics = w.next()?.into();
        let ssrcs = w.map(|s| s.parse().ok()).collect::<Option<Vec<u32>>>()?;
        Some(SsrcGroup { semantics, ssrcs })
    }
    fn value(&self) -> Option<String> {
        let mut s = self.semantics.clone();
        for x in &self.ssrcs { let _ = write!(s, " {}", x); }
        Some(s)
    }
}

/// `a=msid:stream track` (`-` for a track in no stream).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Msid { pub stream: String, pub track: Option<String> }
impl Attr for Msid {
    const NAME: &'static str = "msid";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        Some(Msid { stream: w.next()?.into(), track: w.next().map(Into::into) })
    }
    fn value(&self) -> Option<String> {
        Some(match &self.track { Some(t) => format!("{} {}", self.stream, t), None => self.stream.clone() })
    }
}

/// One rid in a simulcast list; `paused` when written with `~`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimulcastId { pub rid: String, pub paused: bool }

/// `a=simulcast:send h;m;l recv r` (RFC 8853). Each stream lists its alternatives.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Simulcast { pub send: Vec<Vec<SimulcastId>>, pub recv: Vec<Vec<SimulcastId>> }
fn streams(s: &str) -> Vec<Vec<SimulcastId>> {
    // Older Firefox wrote `send rid=h;m;l`.
    let s = s.strip_prefix("rid=").unwrap_or(s);
    s.split(';').map(|alt| alt.split(',').map(|r| match r.strip_prefix('~') {
        Some(r) => SimulcastId { rid: r.into(), paused: true },
        None => SimulcastId { rid: r.into(), paused: false },
    }).collect()).collect()
}
fn write_streams(out: &mut String, v: &[Vec<SimulcastId>]) {
    for (i, alt) in v.iter().enumerate() {
        if i > 0 { out.push(';'); }
        for (j, r) in alt.iter().enumerate() {
            if j > 0 { out.push(','); }
            if r.paused { out.push('~'); }
            out.push_str(&r.rid);
        }
    }
}
impl Attr for Simulcast {
    const NAME: &'static str = "simulcast";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut s = Simulcast::default();
        let mut w = words(v);
        while let Some(dir) = w.next() {
            let list = streams(w.next()?);
            match dir { "send" => s.send = list, "recv" => s.recv = list, _ => return None }
        }
        if s.send.is_empty() && s.recv.is_empty() { None } else { Some(s) }
    }
    fn value(&self) -> Option<String> {
        let mut s = String::new();
        if !self.send.is_empty() { s.push_str("send "); write_streams(&mut s, &self.send); }
        if !self.recv.is_empty() {
            if !s.is_empty() { s.push(' '); }
            s.push_str("recv "); write_streams(&mut s, &self.recv);
        }
        Some(s)
    }
}

/// `a=rid:h send pt=96;max-width=1280` (RFC 8851).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rid { pub id: String, pub send: bool, pub restrictions: Option<String> }
impl Rid {
    /// The `pt=` payload types it allows, if it limits them.
    pub fn payload_types(&self) -> Vec<u8> {
        self.restriction("pt").map(|v| v.split(',').filter_map(|p| p.parse().ok()).collect()).unwrap_or_default()
    }
    pub fn restriction(&self, key: &str) -> Option<&str> {
        self.restrictions.as_deref()?.split(';').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v)
    }
}
impl Attr for Rid {
    const NAME: &'static str = "rid";
    fn parse(v: Option<&str>) -> Option<Self> {
        let mut w = words(v);
        let id = w.next()?.into();
        let send = match w.next()? { "send" => true, "recv" => false, _ => return None };
        Some(Rid { id, send, restrictions: w.next().map(Into::into) })
    }
    fn value(&self) -> Option<String> {
        let mut s = format!("{} {}", self.id, if self.send { "send" } else { "recv" });
        if let Some(r) = &self.restrictions { s.push(' '); s.push_str(r); }
        Some(s)
    }
}

/// `a=rtcp:9 IN IP4 0.0.0.0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rtcp { pub port: u16, pub address: Option<String> }
impl Attr for Rtcp {
    const NAME: &'static str = "rtcp";
    fn parse(v: Option<&str>) -> Option<Self> {
        let v = v?.trim();
        let (port, rest) = v.split_once(' ').unwrap_or((v, ""));
        Some(Rtcp { port: port.parse().ok()?, address: if rest.is_empty() { None } else { Some(rest.into()) } })
    }
    fn value(&self) -> Option<String> {
        Some(match &self.address { Some(a) => format!("{} {}", self.port, a), None => self.port.to_string() })
    }
}

/// host, srflx, prflx or relay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateType { Host, ServerReflexive, PeerReflexive, Relay }
impl CandidateType {
    pub fn as_str(self) -> &'static str {
        match self { CandidateType::Host => "host", CandidateType::ServerReflexive => "srflx", CandidateType::PeerReflexive => "prflx", CandidateType::Relay => "relay" }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s { "host" => CandidateType::Host, "srflx" => CandidateType::ServerReflexive, "prflx" => CandidateType::PeerReflexive, "relay" => CandidateType::Relay, _ => return None })
    }
}

/// `a=candidate:...` (RFC 8839). The address may be an mDNS `.local` name, as browsers
/// write host candidates now. Extensions after the known fields are kept in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub foundation: String,
    pub component: u16,
    /// udp or tcp.
    pub transport: String,
    pub priority: u32,
    pub address: String,
    pub port: u16,
    pub kind: CandidateType,
    pub related: Option<(String, u16)>,
    /// active, passive or so (ICE-TCP).
    pub tcptype: Option<String>,
    pub extensions: Vec<(String, String)>,
}
impl Candidate {
    /// From a trickled `candidate:...` string, or an `a=candidate:...` line.
    pub fn parse_line(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix("a=").unwrap_or(s);
        Self::parse(Some(s.strip_prefix("candidate:").unwrap_or(s)))
    }
    /// The `candidate:...` string an RTCIceCandidate carries.
    pub fn to_line(&self) -> String { format!("candidate:{}", self.value().unwrap_or_default()) }
}
impl Attr for Candidate {
    const NAME: &'static str = "candidate";
    fn parse(v: Option<&str>) -> Option<Self> {
        let w: Vec<&str> = words(v).collect();
        if w.len() < 8 || w[6] != "typ" { return None; }
        let mut c = Candidate {
            foundation: w[0].into(), component: w[1].parse().ok()?, transport: w[2].to_ascii_lowercase(),
            priority: w[3].parse().ok()?, address: w[4].into(), port: w[5].parse().ok()?,
            kind: CandidateType::from_str(w[7])?, related: None, tcptype: None, extensions: Vec::new(),
        };
        let mut raddr = None;
        let mut i = 8;
        while i + 1 < w.len() {
            match w[i] {
                "raddr" => raddr = Some(w[i + 1]),
                "rport" => { let p = w[i + 1].parse().ok()?; c.related = Some((raddr.unwrap_or("0.0.0.0").into(), p)); }
                "tcptype" => c.tcptype = Some(w[i + 1].into()),
                k => c.extensions.push((k.into(), w[i + 1].into())),
            }
            i += 2;
        }
        Some(c)
    }
    fn value(&self) -> Option<String> {
        let mut s = format!("{} {} {} {} {} {} typ {}", self.foundation, self.component, self.transport, self.priority, self.address, self.port, self.kind.as_str());
        if let Some((a, p)) = &self.related { let _ = write!(s, " raddr {} rport {}", a, p); }
        if let Some(t) = &self.tcptype { let _ = write!(s, " tcptype {}", t); }
        for (k, v) in &self.extensions { let _ = write!(s, " {} {}", k, v); }
        Some(s)
    }
}
