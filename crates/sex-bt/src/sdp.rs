// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! SDP, the Service Discovery Protocol (Core 5.4, Vol 3 Part B), on L2CAP PSM 0x0001:
//! data elements, a client that finds a peer's service records (Service Search
//! Attribute, continuation included) and pulls out what A2DP needs (the sink's AVDTP
//! PSM and versions), and a small server that answers peers asking about us (our A2DP
//! Source record, and AVRCP Target and Controller records). Every length and count from the peer is bounded: nesting depth,
//! element count, accumulated response bytes and continuation rounds.

use alloc::vec::Vec;

pub const PSM: u16 = 0x0001;

pub mod uuid {
    pub const L2CAP: u16 = 0x0100;
    pub const AVDTP: u16 = 0x0019;
    pub const AVCTP: u16 = 0x0017;
    pub const PUBLIC_BROWSE_ROOT: u16 = 0x1002;
    pub const AUDIO_SOURCE: u16 = 0x110A;
    pub const AUDIO_SINK: u16 = 0x110B;
    pub const ADVANCED_AUDIO_DISTRIBUTION: u16 = 0x110D;
    pub const AV_REMOTE_CONTROL_TARGET: u16 = 0x110C;
    pub const AV_REMOTE_CONTROL: u16 = 0x110E;
    pub const AV_REMOTE_CONTROL_CONTROLLER: u16 = 0x110F;
}

pub mod attr {
    pub const SERVICE_RECORD_HANDLE: u16 = 0x0000;
    pub const SERVICE_CLASS_ID_LIST: u16 = 0x0001;
    pub const PROTOCOL_DESCRIPTOR_LIST: u16 = 0x0004;
    pub const BROWSE_GROUP_LIST: u16 = 0x0005;
    pub const PROFILE_DESCRIPTOR_LIST: u16 = 0x0009;
    pub const SUPPORTED_FEATURES: u16 = 0x0311;
}

pub mod pdu {
    pub const ERROR_RSP: u8 = 0x01;
    pub const SERVICE_SEARCH_REQ: u8 = 0x02;
    pub const SERVICE_SEARCH_RSP: u8 = 0x03;
    pub const SERVICE_ATTRIBUTE_REQ: u8 = 0x04;
    pub const SERVICE_ATTRIBUTE_RSP: u8 = 0x05;
    pub const SERVICE_SEARCH_ATTRIBUTE_REQ: u8 = 0x06;
    pub const SERVICE_SEARCH_ATTRIBUTE_RSP: u8 = 0x07;
}

/// Error codes.
pub mod error {
    pub const INVALID_RECORD_HANDLE: u16 = 0x0002;
    pub const INVALID_SYNTAX: u16 = 0x0003;
    pub const INVALID_PDU_SIZE: u16 = 0x0004;
    pub const INVALID_CONTINUATION: u16 = 0x0005;
}

/// Bounds on what a peer may make us parse.
const MAX_DEPTH: usize = 8;
const MAX_ELEMENTS: usize = 1024;
/// Response bytes we collect over continuations, and how many rounds.
pub const MAX_RESPONSE: usize = 16 * 1024;
pub const MAX_ROUNDS: usize = 32;

/// The Bluetooth base UUID (0000xxxx-0000-1000-8000-00805F9B34FB) with the short part 0.
const BASE: u128 = 0x0000_0000_0000_1000_8000_0080_5F9B_34FB;

/// A data element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Element {
    Nil,
    /// Unsigned integer and its size in bytes (1, 2, 4, 8; 16-byte values are refused).
    Uint(u64, u8),
    Int(i64, u8),
    /// A UUID in 128-bit form (16- and 32-bit ones are expanded on the base UUID).
    Uuid(u128),
    Text(Vec<u8>),
    Bool(bool),
    Seq(Vec<Element>),
    Alt(Vec<Element>),
    Url(Vec<u8>),
}

pub const fn uuid16(short: u16) -> u128 { BASE | (short as u128) << 96 }

impl Element {
    pub fn u16(v: u16) -> Element { Element::Uint(v as u64, 2) }
    pub fn u32(v: u32) -> Element { Element::Uint(v as u64, 4) }
    pub fn uuid(short: u16) -> Element { Element::Uuid(uuid16(short)) }
    /// The 16-bit form of a base UUID.
    pub fn as_uuid16(&self) -> Option<u16> {
        match *self { Element::Uuid(u) if u & !(0xFFFF_FFFFu128 << 96) == BASE && u >> 112 == 0 => Some((u >> 96) as u16), _ => None }
    }
    pub fn as_uint(&self) -> Option<u64> { match *self { Element::Uint(v, _) => Some(v), _ => None } }
    pub fn items(&self) -> &[Element] { match self { Element::Seq(v) | Element::Alt(v) => v, _ => &[] } }
    /// Whether this element is, or contains, the 16-bit UUID `short`.
    pub fn has_uuid(&self, short: u16) -> bool { self.as_uuid16() == Some(short) || self.items().iter().any(|e| e.has_uuid(short)) }

    /// Append the encoding.
    pub fn encode(&self, out: &mut Vec<u8>) {
        fn sized(out: &mut Vec<u8>, kind: u8, body: &[u8]) {
            let n = body.len();
            if n < 0x100 { out.extend_from_slice(&[kind << 3 | 5, n as u8]); }
            else if n < 0x10000 { out.push(kind << 3 | 6); out.extend_from_slice(&(n as u16).to_be_bytes()); }
            else { out.push(kind << 3 | 7); out.extend_from_slice(&(n as u32).to_be_bytes()); }
            out.extend_from_slice(body);
        }
        fn index(bytes: u8) -> u8 { match bytes { 1 => 0, 2 => 1, 4 => 2, _ => 3 } }
        match self {
            Element::Nil => out.push(0),
            Element::Uint(v, b) => { let b = (*b).clamp(1, 8).next_power_of_two(); out.push(1 << 3 | index(b)); out.extend_from_slice(&v.to_be_bytes()[8 - b as usize..]); }
            Element::Int(v, b) => { let b = (*b).clamp(1, 8).next_power_of_two(); out.push(2 << 3 | index(b)); out.extend_from_slice(&v.to_be_bytes()[8 - b as usize..]); }
            Element::Uuid(u) => {
                if let Some(s) = self.as_uuid16() { out.push(3 << 3 | 1); out.extend_from_slice(&s.to_be_bytes()); }
                else { out.push(3 << 3 | 4); out.extend_from_slice(&u.to_be_bytes()); }
            }
            Element::Text(t) => sized(out, 4, t),
            Element::Bool(b) => out.extend_from_slice(&[5 << 3, *b as u8]),
            Element::Seq(items) | Element::Alt(items) => {
                let mut body = Vec::new();
                for i in items { i.encode(&mut body); }
                sized(out, if matches!(self, Element::Seq(_)) { 6 } else { 7 }, &body);
            }
            Element::Url(t) => sized(out, 8, t),
        }
    }
    pub fn to_bytes(&self) -> Vec<u8> { let mut v = Vec::new(); self.encode(&mut v); v }

    /// Decode one element from the front of `b`: (element, bytes used).
    pub fn decode(b: &[u8]) -> Option<(Element, usize)> {
        let mut budget = MAX_ELEMENTS;
        decode(b, 0, &mut budget)
    }
}

fn decode(b: &[u8], depth: usize, budget: &mut usize) -> Option<(Element, usize)> {
    if depth > MAX_DEPTH || *budget == 0 { return None; }
    *budget -= 1;
    let (&h, rest) = b.split_first()?;
    let (kind, size) = (h >> 3, h & 7);
    // Fixed sizes, or a length that follows.
    let (len, head): (usize, usize) = match size {
        0..=4 if kind == 0 => (0, 1),
        0..=4 => (1usize << size, 1),
        5 => (*rest.first()? as usize, 2),
        6 => (u16::from_be_bytes([*rest.first()?, *rest.get(1)?]) as usize, 3),
        _ => (u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize, 5),
    };
    let body = b.get(head..head.checked_add(len)?)?;
    let be = |n: usize| -> Option<u64> { (n <= 8).then(|| body.iter().fold(0u64, |a, &x| a << 8 | x as u64)) };
    let e = match kind {
        0 if size == 0 => Element::Nil,
        1 if size <= 3 => Element::Uint(be(len)?, len as u8),
        2 if size <= 3 => {
            let v = be(len)?;
            let shift = 64 - 8 * len as u32;
            Element::Int(((v << shift) as i64) >> shift, len as u8)
        }
        3 if matches!(len, 2 | 4) => Element::Uuid(BASE | (be(len)? as u128) << 96),
        3 if len == 16 => Element::Uuid(u128::from_be_bytes(body.try_into().ok()?)),
        4 if size >= 5 => Element::Text(body.to_vec()),
        5 if size == 0 => Element::Bool(body[0] != 0),
        6 | 7 if size >= 5 => {
            let mut items = Vec::new();
            let mut at = 0;
            while at < body.len() {
                let (e, n) = decode(&body[at..], depth + 1, budget)?;
                items.push(e);
                at += n;
            }
            if kind == 6 { Element::Seq(items) } else { Element::Alt(items) }
        }
        8 if size >= 5 => Element::Url(body.to_vec()),
        _ => return None,
    };
    Some((e, head + len))
}

/// One service record: (attribute id, value) pairs.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Record(pub Vec<(u16, Element)>);

impl Record {
    pub fn get(&self, id: u16) -> Option<&Element> { self.0.iter().find(|(a, _)| *a == id).map(|(_, e)| e) }
    pub fn element(&self) -> Element {
        let mut items = Vec::new();
        for (id, e) in &self.0 { items.push(Element::u16(*id)); items.push(e.clone()); }
        Element::Seq(items)
    }
    fn from_element(e: &Element) -> Option<Record> {
        let Element::Seq(items) = e else { return None };
        if items.len() % 2 != 0 { return None; }
        let mut r = Record::default();
        for pair in items.chunks(2) {
            let id = match pair[0] { Element::Uint(v, 2) => v as u16, _ => return None };
            r.0.push((id, pair[1].clone()));
        }
        Some(r)
    }
    /// The PSM and version of `protocol` in the protocol descriptor list
    /// (`((L2CAP, psm), (protocol, version))`).
    pub fn protocol(&self, protocol: u16) -> Option<(u16, u16)> {
        let list = self.get(attr::PROTOCOL_DESCRIPTOR_LIST)?.items();
        let psm = list.iter().find(|d| d.items().first().and_then(|u| u.as_uuid16()) == Some(uuid::L2CAP))
            .and_then(|d| d.items().get(1)?.as_uint()).unwrap_or(0) as u16;
        let version = list.iter().find(|d| d.items().first().and_then(|u| u.as_uuid16()) == Some(protocol))
            .map(|d| d.items().get(1).and_then(|v| v.as_uint()).unwrap_or(0) as u16)?;
        Some((psm, version))
    }
    /// A profile's version from the profile descriptor list.
    pub fn profile_version(&self, profile: u16) -> Option<u16> {
        self.get(attr::PROFILE_DESCRIPTOR_LIST)?.items().iter()
            .find(|d| d.items().first().and_then(|u| u.as_uuid16()) == Some(profile))
            .and_then(|d| d.items().get(1)?.as_uint()).map(|v| v as u16)
    }
}

/// What A2DP needs from a sink's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct A2dpSink { pub psm: u16, pub avdtp_version: u16, pub a2dp_version: u16, pub features: u16 }

/// The first record that is an A2DP Sink with AVDTP on an L2CAP PSM.
pub fn a2dp_sink(records: &[Record]) -> Option<A2dpSink> {
    records.iter().find_map(|r| {
        if !r.get(attr::SERVICE_CLASS_ID_LIST)?.has_uuid(uuid::AUDIO_SINK) { return None; }
        let (psm, avdtp_version) = r.protocol(uuid::AVDTP)?;
        // PSMs are odd with the low bit of the upper byte clear.
        if psm & 0x0101 != 0x0001 { return None; }
        Some(A2dpSink { psm, avdtp_version, a2dp_version: r.profile_version(uuid::ADVANCED_AUDIO_DISTRIBUTION).unwrap_or(0),
            features: r.get(attr::SUPPORTED_FEATURES).and_then(|e| e.as_uint()).unwrap_or(0) as u16 })
    })
}

fn header(id: u8, tid: u16, params: &[u8]) -> Vec<u8> {
    let mut p = alloc::vec![id];
    p.extend_from_slice(&tid.to_be_bytes());
    p.extend_from_slice(&(params.len() as u16).to_be_bytes());
    p.extend_from_slice(params);
    p
}

/// (pdu id, transaction id, parameters) of one SDP PDU.
pub fn parse(p: &[u8]) -> Option<(u8, u16, &[u8])> {
    if p.len() < 5 { return None; }
    let len = u16::from_be_bytes([p[3], p[4]]) as usize;
    Some((p[0], u16::from_be_bytes([p[1], p[2]]), p.get(5..5 + len)?))
}

/// What the client wants next.
#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    /// Send this request (a continuation).
    Send(Vec<u8>),
    Done(Vec<Record>),
    /// The peer answered with an error, or something we will not parse.
    Failed(u16),
}

/// A Service Search Attribute client for one service class.
pub struct Client { class: u16, attrs: Vec<u16>, tid: u16, got: Vec<u8>, rounds: usize, max_bytes: u16 }

impl Client {
    /// Look for records of `class`, asking for `attrs`. `max_bytes` bounds each reply
    /// (keep it under the channel's MTU).
    pub fn new(class: u16, attrs: &[u16], max_bytes: u16) -> Client {
        Client { class, attrs: attrs.to_vec(), tid: 0, got: Vec::new(), rounds: 0, max_bytes: max_bytes.max(7) }
    }
    /// The A2DP Sink search.
    pub fn a2dp_sink() -> Client {
        Client::new(uuid::AUDIO_SINK, &[attr::SERVICE_CLASS_ID_LIST, attr::PROTOCOL_DESCRIPTOR_LIST, attr::PROFILE_DESCRIPTOR_LIST, attr::SUPPORTED_FEATURES], 600)
    }
    fn request(&mut self, continuation: &[u8]) -> Vec<u8> {
        self.tid = self.tid.wrapping_add(1);
        let mut p = Element::Seq(alloc::vec![Element::uuid(self.class)]).to_bytes();
        p.extend_from_slice(&self.max_bytes.to_be_bytes());
        Element::Seq(self.attrs.iter().map(|&a| Element::u16(a)).collect()).encode(&mut p);
        p.push(continuation.len() as u8);
        p.extend_from_slice(continuation);
        header(pdu::SERVICE_SEARCH_ATTRIBUTE_REQ, self.tid, &p)
    }
    pub fn start(&mut self) -> Vec<u8> { self.got.clear(); self.rounds = 0; self.request(&[]) }

    /// One PDU from the peer.
    pub fn on_pdu(&mut self, p: &[u8]) -> Progress {
        let Some((id, tid, params)) = parse(p) else { return Progress::Failed(error::INVALID_PDU_SIZE) };
        if tid != self.tid { return Progress::Failed(error::INVALID_SYNTAX); }
        if id == pdu::ERROR_RSP { return Progress::Failed(params.get(..2).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(error::INVALID_SYNTAX)); }
        if id != pdu::SERVICE_SEARCH_ATTRIBUTE_RSP || params.len() < 3 { return Progress::Failed(error::INVALID_SYNTAX); }
        let count = u16::from_be_bytes([params[0], params[1]]) as usize;
        let Some(lists) = params.get(2..2 + count) else { return Progress::Failed(error::INVALID_PDU_SIZE) };
        let Some(cont) = params.get(2 + count..).and_then(|c| { let n = *c.first()? as usize; if n > 16 { None } else { c.get(1..1 + n) } }) else {
            return Progress::Failed(error::INVALID_CONTINUATION)
        };
        if self.got.len() + lists.len() > MAX_RESPONSE { return Progress::Failed(error::INVALID_PDU_SIZE); }
        self.got.extend_from_slice(lists);
        self.rounds += 1;
        if !cont.is_empty() {
            if self.rounds >= MAX_ROUNDS { return Progress::Failed(error::INVALID_CONTINUATION); }
            let cont = cont.to_vec();
            return Progress::Send(self.request(&cont));
        }
        match Element::decode(&self.got) {
            Some((Element::Seq(records), n)) if n == self.got.len() => {
                match records.iter().map(Record::from_element).collect::<Option<Vec<_>>>() {
                    Some(r) => Progress::Done(r),
                    None => Progress::Failed(error::INVALID_SYNTAX),
                }
            }
            _ => Progress::Failed(error::INVALID_SYNTAX),
        }
    }
}

/// Our own record: an A2DP Source (AVDTP 1.3 on PSM 0x0019, A2DP 1.3, player).
pub fn a2dp_source_record() -> Record {
    use Element as E;
    Record(alloc::vec![
        (attr::SERVICE_RECORD_HANDLE, E::u32(0x0001_0001)),
        (attr::SERVICE_CLASS_ID_LIST, E::Seq(alloc::vec![E::uuid(uuid::AUDIO_SOURCE)])),
        (attr::PROTOCOL_DESCRIPTOR_LIST, E::Seq(alloc::vec![
            E::Seq(alloc::vec![E::uuid(uuid::L2CAP), E::u16(uuid::AVDTP)]),
            E::Seq(alloc::vec![E::uuid(uuid::AVDTP), E::u16(0x0103)]),
        ])),
        (attr::BROWSE_GROUP_LIST, E::Seq(alloc::vec![E::uuid(uuid::PUBLIC_BROWSE_ROOT)])),
        (attr::PROFILE_DESCRIPTOR_LIST, E::Seq(alloc::vec![E::Seq(alloc::vec![E::uuid(uuid::ADVANCED_AUDIO_DISTRIBUTION), E::u16(0x0103)])])),
        (attr::SUPPORTED_FEATURES, E::u16(0x0001)),
    ])
}

/// An AVRCP record: AVCTP 1.4 on PSM 0x0017, AVRCP 1.4 (absolute volume), `features`.
fn avrcp_record(handle: u32, classes: &[u16], features: u16) -> Record {
    use Element as E;
    Record(alloc::vec![
        (attr::SERVICE_RECORD_HANDLE, E::u32(handle)),
        (attr::SERVICE_CLASS_ID_LIST, E::Seq(classes.iter().map(|&c| E::uuid(c)).collect())),
        (attr::PROTOCOL_DESCRIPTOR_LIST, E::Seq(alloc::vec![
            E::Seq(alloc::vec![E::uuid(uuid::L2CAP), E::u16(uuid::AVCTP)]),
            E::Seq(alloc::vec![E::uuid(uuid::AVCTP), E::u16(0x0104)]),
        ])),
        (attr::BROWSE_GROUP_LIST, E::Seq(alloc::vec![E::uuid(uuid::PUBLIC_BROWSE_ROOT)])),
        (attr::PROFILE_DESCRIPTOR_LIST, E::Seq(alloc::vec![E::Seq(alloc::vec![E::uuid(uuid::AV_REMOTE_CONTROL), E::u16(0x0104)])])),
        (attr::SUPPORTED_FEATURES, E::u16(features)),
    ])
}
/// Our AVRCP Target: category 1 (player), for the headset's buttons.
pub fn avrcp_target_record() -> Record { avrcp_record(0x0001_0002, &[uuid::AV_REMOTE_CONTROL_TARGET], 0x0001) }
/// Our AVRCP Controller: category 2 (amplifier), for the headset's absolute volume.
pub fn avrcp_controller_record() -> Record {
    avrcp_record(0x0001_0003, &[uuid::AV_REMOTE_CONTROL, uuid::AV_REMOTE_CONTROL_CONTROLLER], 0x0002)
}

fn error_rsp(tid: u16, code: u16) -> Vec<u8> { header(pdu::ERROR_RSP, tid, &code.to_be_bytes()) }

/// Whether attribute `id` is in an attribute id list (ids and ranges).
fn wanted(list: &Element, id: u16) -> bool {
    list.items().iter().any(|e| match *e {
        Element::Uint(v, 2) => v as u16 == id,
        Element::Uint(v, 4) => (v >> 16) as u16 <= id && id <= v as u16,
        _ => false,
    })
}

/// Answer one request from a peer about `records` (handles from attribute 0). Replies
/// bigger than the peer's maximum go out in parts; our continuation state is the byte
/// offset reached (2 bytes), checked against the reply on every request.
pub fn serve(req: &[u8], records: &[Record]) -> Vec<u8> {
    let Some((id, tid, p)) = parse(req) else { return error_rsp(0, error::INVALID_PDU_SIZE) };
    let mut at = 0;
    let next = |at: &mut usize| -> Option<Element> {
        let (e, n) = Element::decode(p.get(*at..)?)?;
        *at += n;
        Some(e)
    };
    let pattern = match id {
        pdu::SERVICE_SEARCH_REQ | pdu::SERVICE_SEARCH_ATTRIBUTE_REQ => match next(&mut at) {
            Some(Element::Seq(u)) if !u.is_empty() && u.len() <= 12 => Some(u),
            _ => return error_rsp(tid, error::INVALID_SYNTAX),
        },
        pdu::SERVICE_ATTRIBUTE_REQ => None,
        _ => return error_rsp(tid, error::INVALID_SYNTAX),
    };
    let handle = if id == pdu::SERVICE_ATTRIBUTE_REQ {
        let Some(h) = p.get(at..at + 4) else { return error_rsp(tid, error::INVALID_SYNTAX) };
        at += 4;
        Some(u32::from_be_bytes(h.try_into().unwrap()))
    } else { None };
    let Some(m) = p.get(at..at + 2) else { return error_rsp(tid, error::INVALID_SYNTAX) };
    let max = u16::from_be_bytes([m[0], m[1]]) as usize;
    at += 2;
    let matches = |r: &&Record| match &pattern {
        Some(u) => u.iter().all(|want| want.as_uuid16().is_some_and(|w| r.0.iter().any(|(_, e)| e.has_uuid(w)))),
        None => r.get(attr::SERVICE_RECORD_HANDLE).and_then(|e| e.as_uint()) == handle.map(|h| h as u64),
    };
    let found: Vec<&Record> = records.iter().filter(matches).collect();
    if id == pdu::SERVICE_SEARCH_REQ {
        // Handles are counted, not continued: we hold a handful of records.
        let handles: Vec<u32> = found.iter().filter_map(|r| r.get(attr::SERVICE_RECORD_HANDLE)?.as_uint()).map(|h| h as u32).take(max.max(1)).collect();
        let mut b = (handles.len() as u16).to_be_bytes().to_vec();
        b.extend_from_slice(&(handles.len() as u16).to_be_bytes());
        for h in handles { b.extend_from_slice(&h.to_be_bytes()); }
        b.push(0);
        return header(pdu::SERVICE_SEARCH_RSP, tid, &b);
    }
    let (body, rsp) = {
        let Some(ids) = next(&mut at) else { return error_rsp(tid, error::INVALID_SYNTAX) };
        if id == pdu::SERVICE_ATTRIBUTE_REQ && found.is_empty() { return error_rsp(tid, error::INVALID_RECORD_HANDLE); }
        let pick = |r: &Record| Record(r.0.iter().filter(|(a, _)| wanted(&ids, *a)).cloned().collect()).element();
        let body = if id == pdu::SERVICE_ATTRIBUTE_REQ { pick(found[0]) } else { Element::Seq(found.iter().map(|r| pick(r)).collect()) };
        (body.to_bytes(), if id == pdu::SERVICE_ATTRIBUTE_REQ { pdu::SERVICE_ATTRIBUTE_RSP } else { pdu::SERVICE_SEARCH_ATTRIBUTE_RSP })
    };
    // Continuation: where the previous part ended.
    let cont = p.get(at..).and_then(|c| c.get(1..1 + *c.first()? as usize));
    let start = match cont {
        Some([]) => 0,
        Some([hi, lo]) => { let s = u16::from_be_bytes([*hi, *lo]) as usize; if s == 0 || s >= body.len() { return error_rsp(tid, error::INVALID_CONTINUATION); } s }
        _ => return error_rsp(tid, error::INVALID_CONTINUATION),
    };
    if max < 7 || body.len() > u16::MAX as usize { return error_rsp(tid, error::INVALID_SYNTAX); }
    let end = (start + max).min(body.len());
    let mut b = ((end - start) as u16).to_be_bytes().to_vec();
    b.extend_from_slice(&body[start..end]);
    if end < body.len() { b.push(2); b.extend_from_slice(&(end as u16).to_be_bytes()); } else { b.push(0); }
    header(rsp, tid, &b)
}
