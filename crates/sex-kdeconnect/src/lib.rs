// SPDX-License-Identifier: GPL-3.0-only
//! The KDE Connect protocol, as its public protocol description lays it out, for
//! Hookup (apps/hookup). Written here from that description; no code is taken from
//! the KDE implementation.
//!
//! * [`Packet`]: one JSON object per line, `{"id", "type", "body"}`, with
//!   `payloadSize` and `payloadTransferInfo` when bytes follow on a side connection.
//! * [`Identity`]: who a device is. Sent as a UDP datagram to port 1716 to be found,
//!   and as the first line of every TCP link.
//! * [`Pairing`]: the pair handshake (`kdeconnect.pair`, `{"pair": true|false}`),
//!   with its 30 second timeout.
//! * [`plugins`]: the packets Hookup speaks: notifications, clipboard, share (files,
//!   text and links) and media control (MPRIS).
//! * [`Framer`]: bytes from a link into lines, bounded.
//! * [`Trusted`]: the paired devices file.
//! * `net` (feature `std`): the device runtime over std::net.
//!
//! Encryption: the protocol runs each link and payload connection under TLS with
//! self-signed certificates pinned at pairing. SexOS's TLS crate is a client of the
//! public web only (no server role, no self-signed certificates of its own), so this
//! crate runs the same packets over plain TCP for now; the [`net`] runtime marks
//! every link `encrypted: false` and the place TLS goes is the link's stream.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
pub use serde_json::{json, Map, Value};

pub mod plugins;
#[cfg(feature = "std")]
pub mod net;

/// The port a device listens on for identity datagrams and links.
pub const PORT: u16 = 1716;
/// Payload connections are offered on a port in this range.
pub const PAYLOAD_PORTS: core::ops::RangeInclusive<u16> = 1739..=1764;
/// The protocol version spoken (identity sent in the clear before encryption).
pub const PROTOCOL_VERSION: i64 = 7;
/// A line longer than this ends the link.
pub const MAX_LINE: usize = 512 * 1024;
/// How long a pair request waits for an answer, in milliseconds.
pub const PAIR_TIMEOUT_MS: u64 = 30_000;

pub const IDENTITY: &str = "kdeconnect.identity";
pub const PAIR: &str = "kdeconnect.pair";
pub const PING: &str = "kdeconnect.ping";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a JSON object, or a field has the wrong type.
    Malformed,
    /// A line over [`MAX_LINE`].
    TooLong,
    /// A packet of an unexpected type where an identity must come.
    NotIdentity,
    /// A device id the protocol does not allow.
    BadDeviceId,
}

/// One protocol packet.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub id: i64,
    pub kind: String,
    pub body: Map<String, Value>,
    /// Bytes that follow on a payload connection.
    pub payload_size: Option<u64>,
    /// Where to fetch them: `{"port": N}`.
    pub payload_port: Option<u16>,
}

impl Packet {
    pub fn new(kind: &str, body: Value, id: i64) -> Packet {
        let body = match body { Value::Object(m) => m, _ => Map::new() };
        Packet { id, kind: kind.to_string(), body, payload_size: None, payload_port: None }
    }

    /// The packet as one line, newline included.
    pub fn to_line(&self) -> String {
        let mut m = Map::new();
        m.insert("id".into(), Value::from(self.id));
        m.insert("type".into(), Value::from(self.kind.as_str()));
        m.insert("body".into(), Value::Object(self.body.clone()));
        if let Some(size) = self.payload_size {
            m.insert("payloadSize".into(), Value::from(size));
            let mut info = Map::new();
            if let Some(port) = self.payload_port { info.insert("port".into(), Value::from(port)); }
            m.insert("payloadTransferInfo".into(), Value::Object(info));
        }
        let mut s = serde_json::to_string(&Value::Object(m)).unwrap_or_default();
        s.push('\n');
        s
    }

    /// Parses one line (without or with its newline).
    pub fn parse(line: &[u8]) -> Result<Packet, Error> {
        let line = trim(line);
        let v: Value = serde_json::from_slice(line).map_err(|_| Error::Malformed)?;
        let Value::Object(mut m) = v else { return Err(Error::Malformed) };
        // Some peers send the id as a string of digits.
        let id = match m.get("id") {
            Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
            Some(Value::String(s)) => s.parse().unwrap_or(0),
            _ => 0,
        };
        let kind = match m.remove("type") { Some(Value::String(s)) => s, _ => return Err(Error::Malformed) };
        let body = match m.remove("body") { Some(Value::Object(b)) => b, None => Map::new(), _ => return Err(Error::Malformed) };
        let payload_size = m.get("payloadSize").and_then(|v| v.as_i64()).filter(|n| *n >= 0).map(|n| n as u64);
        let payload_port = m.get("payloadTransferInfo").and_then(|v| v.get("port")).and_then(|v| v.as_u64())
            .and_then(|p| u16::try_from(p).ok());
        Ok(Packet { id, kind, body, payload_size, payload_port })
    }

    pub fn str(&self, key: &str) -> Option<&str> { self.body.get(key).and_then(|v| v.as_str()) }
    pub fn bool(&self, key: &str) -> Option<bool> { self.body.get(key).and_then(|v| v.as_bool()) }
    pub fn int(&self, key: &str) -> Option<i64> { self.body.get(key).and_then(|v| v.as_i64()) }
}

fn trim(mut b: &[u8]) -> &[u8] {
    while let [rest @ .., b'\n' | b'\r' | b' ' | b'\t'] = b { b = rest; }
    while let [b' ' | b'\t', rest @ ..] = b { b = rest; }
    b
}

/// Who a device is.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub device_id: String,
    pub device_name: String,
    /// `desktop`, `laptop`, `phone`, `tablet` or `tv`.
    pub device_type: String,
    pub protocol_version: i64,
    pub incoming: Vec<String>,
    pub outgoing: Vec<String>,
    /// Where it takes links; absent in an identity sent over a link.
    pub tcp_port: Option<u16>,
}

/// A device id: 32 to 38 characters of letters, digits and `_` (older peers used
/// UUIDs with `-`, which are taken too).
pub fn valid_device_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A fresh device id from some unpredictable bytes (32 hex characters).
pub fn device_id_from(seed: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in seed { s.push(char::from_digit((b >> 4) as u32, 16).unwrap()); s.push(char::from_digit((b & 15) as u32, 16).unwrap()); }
    s
}

impl Identity {
    pub fn packet(&self, id: i64) -> Packet {
        let mut body = json!({
            "deviceId": self.device_id,
            "deviceName": self.device_name,
            "deviceType": self.device_type,
            "protocolVersion": self.protocol_version,
            "incomingCapabilities": self.incoming,
            "outgoingCapabilities": self.outgoing,
        });
        if let Some(p) = self.tcp_port { body["tcpPort"] = Value::from(p); }
        Packet::new(IDENTITY, body, id)
    }

    pub fn from_packet(p: &Packet) -> Result<Identity, Error> {
        if p.kind != IDENTITY { return Err(Error::NotIdentity); }
        let device_id = p.str("deviceId").ok_or(Error::Malformed)?.to_string();
        if !valid_device_id(&device_id) { return Err(Error::BadDeviceId); }
        let list = |k: &str| -> Vec<String> {
            p.body.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
        };
        // Names are shown: keep them printable and short.
        let name: String = p.str("deviceName").unwrap_or("Unnamed device").chars().filter(|c| !c.is_control()).take(64).collect();
        Ok(Identity {
            device_id,
            device_name: name,
            device_type: p.str("deviceType").unwrap_or("phone").to_string(),
            protocol_version: p.int("protocolVersion").unwrap_or(0),
            incoming: list("incomingCapabilities"),
            outgoing: list("outgoingCapabilities"),
            tcp_port: p.int("tcpPort").and_then(|n| u16::try_from(n).ok()),
        })
    }

    /// What Hookup says about itself.
    pub fn hookup(device_id: &str, name: &str, tcp_port: Option<u16>) -> Identity {
        let caps = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        Identity {
            device_id: device_id.to_string(),
            device_name: name.to_string(),
            device_type: "desktop".into(),
            protocol_version: PROTOCOL_VERSION,
            incoming: caps(plugins::INCOMING),
            outgoing: caps(plugins::OUTGOING),
            tcp_port,
        }
    }
}

/// What to do about an identity datagram from the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discovery {
    /// Our own announcement coming back.
    Ourselves,
    /// Already linked: nothing to do.
    AlreadyLinked,
    /// The peer speaks a protocol older than any this side knows.
    TooOld,
    /// Open a link to it on this port.
    Connect(u16),
}

/// Decides on a datagram from `peer` (`linked` says whether a link to that device id
/// exists).
pub fn discovered(own_id: &str, peer: &Identity, linked: bool) -> Discovery {
    if peer.device_id == own_id { return Discovery::Ourselves; }
    if linked { return Discovery::AlreadyLinked; }
    if peer.protocol_version < 6 { return Discovery::TooOld; }
    Discovery::Connect(peer.tcp_port.unwrap_or(PORT))
}

/// Lines out of a byte stream.
#[derive(Default)]
pub struct Framer { buf: Vec<u8> }

impl Framer {
    pub fn new() -> Framer { Framer::default() }
    /// Adds bytes; returns every complete line in them (without the newline).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).take(i).collect();
            if !trim(&line).is_empty() { out.push(line); }
        }
        if self.buf.len() > MAX_LINE { return Err(Error::TooLong); }
        Ok(out)
    }
}

/// Where a pairing stands with one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairState {
    NotPaired,
    /// We asked; waiting for the peer's answer since `at` (ms).
    Requested { at: u64 },
    /// The peer asked; waiting for the person since `at` (ms).
    RequestedByPeer { at: u64 },
    Paired,
}

/// What a step of the pairing means for the person and the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairOutcome {
    /// Send this `{"pair": ...}` answer.
    Send(bool),
    /// Ask the person whether to pair.
    Ask,
    /// Now paired (keep the device as trusted).
    NowPaired,
    /// Now not paired (forget the device); the reason is for the status line.
    NowUnpaired(&'static str),
}

/// The pair handshake with one device, from the protocol description: either side
/// sends `{"pair": true}`; the other answers `true` to accept or `false` to refuse;
/// `false` at any time unpairs; an unanswered request lapses after 30 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pairing { pub state: PairState }

impl Pairing {
    pub fn new(trusted: bool) -> Pairing { Pairing { state: if trusted { PairState::Paired } else { PairState::NotPaired } } }
    pub fn paired(&self) -> bool { self.state == PairState::Paired }

    /// The person asks to pair.
    pub fn request(&mut self, now: u64) -> Vec<PairOutcome> {
        match self.state {
            PairState::NotPaired => { self.state = PairState::Requested { at: now }; alloc::vec![PairOutcome::Send(true)] }
            // Asking back is accepting.
            PairState::RequestedByPeer { .. } => self.accept(),
            _ => Vec::new(),
        }
    }
    /// The person accepts the peer's request.
    pub fn accept(&mut self) -> Vec<PairOutcome> {
        if let PairState::RequestedByPeer { .. } = self.state {
            self.state = PairState::Paired;
            return alloc::vec![PairOutcome::Send(true), PairOutcome::NowPaired];
        }
        Vec::new()
    }
    /// The person refuses the peer's request.
    pub fn reject(&mut self) -> Vec<PairOutcome> {
        if let PairState::RequestedByPeer { .. } = self.state {
            self.state = PairState::NotPaired;
            return alloc::vec![PairOutcome::Send(false), PairOutcome::NowUnpaired("refused")];
        }
        Vec::new()
    }
    /// The person unpairs.
    pub fn unpair(&mut self) -> Vec<PairOutcome> {
        let was = self.state;
        self.state = PairState::NotPaired;
        match was {
            PairState::NotPaired => Vec::new(),
            _ => alloc::vec![PairOutcome::Send(false), PairOutcome::NowUnpaired("unpaired")],
        }
    }
    /// A `kdeconnect.pair` packet arrived.
    pub fn received(&mut self, pair: bool, now: u64) -> Vec<PairOutcome> {
        match (self.state, pair) {
            (PairState::NotPaired, true) => { self.state = PairState::RequestedByPeer { at: now }; alloc::vec![PairOutcome::Ask] }
            (PairState::Requested { .. }, true) => { self.state = PairState::Paired; alloc::vec![PairOutcome::NowPaired] }
            // Already paired: say so again (the peer may have lost its record of it).
            (PairState::Paired, true) => alloc::vec![PairOutcome::Send(true)],
            (PairState::RequestedByPeer { .. }, true) => Vec::new(),
            (PairState::NotPaired, false) => Vec::new(),
            (PairState::Requested { .. }, false) => { self.state = PairState::NotPaired; alloc::vec![PairOutcome::NowUnpaired("refused by the device")] }
            (PairState::RequestedByPeer { .. }, false) => { self.state = PairState::NotPaired; alloc::vec![PairOutcome::NowUnpaired("withdrawn by the device")] }
            (PairState::Paired, false) => { self.state = PairState::NotPaired; alloc::vec![PairOutcome::NowUnpaired("unpaired by the device")] }
        }
    }
    /// Time passes: an unanswered request lapses.
    pub fn tick(&mut self, now: u64) -> Vec<PairOutcome> {
        match self.state {
            PairState::Requested { at } | PairState::RequestedByPeer { at } if now.saturating_sub(at) >= PAIR_TIMEOUT_MS => {
                self.state = PairState::NotPaired;
                alloc::vec![PairOutcome::NowUnpaired("timed out")]
            }
            _ => Vec::new(),
        }
    }
}

/// The pair packet for an answer or request.
pub fn pair_packet(pair: bool, id: i64) -> Packet { Packet::new(PAIR, json!({ "pair": pair }), id) }

/// Paired devices, kept as lines `device id <TAB> name`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trusted { pub devices: Vec<(String, String)> }

impl Trusted {
    pub fn parse(text: &str) -> Trusted {
        let devices = text.lines().filter_map(|l| {
            let (id, name) = l.split_once('\t').unwrap_or((l, ""));
            valid_device_id(id.trim()).then(|| (id.trim().to_string(), name.trim().to_string()))
        }).collect();
        Trusted { devices }
    }
    pub fn serialize(&self) -> String {
        let mut s = String::new();
        for (id, name) in &self.devices { s.push_str(id); s.push('\t'); s.push_str(&name.replace(['\t', '\n'], " ")); s.push('\n'); }
        s
    }
    pub fn contains(&self, id: &str) -> bool { self.devices.iter().any(|d| d.0 == id) }
    pub fn add(&mut self, id: &str, name: &str) {
        if let Some(d) = self.devices.iter_mut().find(|d| d.0 == id) { d.1 = name.to_string(); } else { self.devices.push((id.to_string(), name.to_string())); }
    }
    pub fn remove(&mut self, id: &str) { self.devices.retain(|d| d.0 != id); }
}

/// A file name from a peer, made safe to create in a folder: no directories, no
/// leading dots, nothing unprintable, at most 120 bytes.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base.chars().filter(|c| !c.is_control() && *c != ':').collect();
    while s.starts_with('.') { s.remove(0); }
    let s = s.trim().to_string();
    let mut s = if s.is_empty() { String::from("received") } else { s };
    while s.len() > 120 { s.pop(); }
    s
}

#[cfg(test)]
mod tests;
