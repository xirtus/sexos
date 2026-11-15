// SPDX-License-Identifier: GPL-3.0-only
//! The extension protocol (BEP 10) and the extensions we speak over it.
//!
//! After the BitTorrent handshake, peers that both set the extension bit exchange an
//! "extended handshake": a bencoded dictionary whose `m` maps extension names to the
//! message ids the *sender* wants to receive them under. Each side then sends extension
//! messages using the id the other side chose. This module has:
//! - [`ExtHandshake`] (`m`, `v` client name (omitted when anonymous), `p`, `yourip`,
//!   `reqq`, `metadata_size`, `ipv4`/`ipv6`, `upload_only`) and [`RemoteExt`], the ids a
//!   peer assigned;
//! - `ut_metadata` (BEP 9): [`MetadataMsg`] and [`MetadataAssembler`], which fetches the
//!   info dictionary of a magnet link in 16 KiB pieces from many peers and verifies it
//!   against the info-hash (SHA-1 for v1, SHA-256 for v2, both for hybrids);
//! - `ut_pex` (BEP 11): [`PexMsg`] and [`PexState`], which sends at most one message a
//!   minute per peer with at most 50 added and 50 dropped peers;
//! - `lt_donthave` (a piece is no longer available) and `upload_only` (BEP 21).
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::bencode::{self, dict, Item};
use crate::peer::PeerKey;
use crate::{InfoHashes, Ip, Millis, PeerAddr};

/// The extended message id of the extended handshake.
pub const HANDSHAKE_ID: u8 = 0;
/// The ids we ask peers to use for extensions sent to us.
pub const UT_METADATA: u8 = 1;
pub const UT_PEX: u8 = 2;
pub const LT_DONTHAVE: u8 = 3;
pub const UPLOAD_ONLY: u8 = 4;
/// The names we advertise, with our ids.
pub const OURS: [(&str, u8); 4] = [("ut_metadata", UT_METADATA), ("ut_pex", UT_PEX), ("lt_donthave", LT_DONTHAVE), ("upload_only", UPLOAD_ONLY)];

/// Caps on hostile handshakes.
const MAX_M_ENTRIES: usize = 64;
const MAX_NAME: usize = 64;
const MAX_CLIENT: usize = 128;

/// The BEP 10 extended handshake.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ExtHandshake {
    /// Extension name to the id the sender receives it under (0 = disabled).
    pub m: Vec<(String, u8)>,
    /// `v`: client name and version.
    pub client: Option<String>,
    /// `p`: the sender's listen port.
    pub port: Option<u16>,
    /// `yourip`: the address the sender sees us at.
    pub yourip: Option<Ip>,
    /// `reqq`: how many outstanding requests the sender accepts.
    pub reqq: Option<u32>,
    /// BEP 9 `metadata_size`: bytes of the info dictionary.
    pub metadata_size: Option<u32>,
    pub ipv4: Option<[u8; 4]>,
    pub ipv6: Option<[u8; 16]>,
    /// BEP 21: the sender only uploads (a seed, or a partial seed).
    pub upload_only: bool,
}

fn ip_bytes(ip: &Ip) -> Vec<u8> { match ip { Ip::V4(a) => a.to_vec(), Ip::V6(a) => a.to_vec() } }
fn ip_from(b: &[u8]) -> Option<Ip> {
    match b.len() {
        4 => Some(Ip::V4([b[0], b[1], b[2], b[3]])),
        16 => { let mut a = [0; 16]; a.copy_from_slice(b); Some(Ip::V6(a)) }
        _ => None,
    }
}

impl ExtHandshake {
    /// Our handshake. In anonymous mode no client name, listen port or own addresses go out.
    pub fn ours(client: &str, port: Option<u16>, yourip: Option<Ip>, reqq: u32, metadata_size: Option<u32>, upload_only: bool, anonymous: bool) -> ExtHandshake {
        ExtHandshake {
            m: OURS.iter().map(|(n, i)| (String::from(*n), *i)).collect(),
            client: if anonymous { None } else { Some(String::from(client)) },
            port: if anonymous { None } else { port },
            yourip, reqq: Some(reqq), metadata_size, ipv4: None, ipv6: None, upload_only,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut d = alloc::collections::BTreeMap::new();
        d.insert(b"m".to_vec(), Item::Dict(self.m.iter().map(|(n, i)| (n.as_bytes().to_vec(), Item::Int(*i as i64))).collect()));
        if let Some(v) = &self.client { d.insert(b"v".to_vec(), v.as_str().into()); }
        if let Some(p) = self.port { d.insert(b"p".to_vec(), Item::Int(p as i64)); }
        if let Some(ip) = &self.yourip { d.insert(b"yourip".to_vec(), ip_bytes(ip).into()); }
        if let Some(q) = self.reqq { d.insert(b"reqq".to_vec(), Item::Int(q as i64)); }
        if let Some(s) = self.metadata_size { d.insert(b"metadata_size".to_vec(), Item::Int(s as i64)); }
        if let Some(a) = self.ipv4 { d.insert(b"ipv4".to_vec(), a.to_vec().into()); }
        if let Some(a) = self.ipv6 { d.insert(b"ipv6".to_vec(), a.to_vec().into()); }
        if self.upload_only { d.insert(b"upload_only".to_vec(), Item::Int(1)); }
        Item::Dict(d).encode()
    }

    /// Decodes tolerantly: unknown keys and badly typed values are ignored, but the
    /// payload must be a dictionary.
    pub fn decode(payload: &[u8]) -> Option<ExtHandshake> {
        let v = bencode::decode(payload).ok()?;
        v.dict()?;
        let mut h = ExtHandshake::default();
        if let Some(m) = v.get(b"m").and_then(|m| m.dict()) {
            for (k, val) in m.iter().take(MAX_M_ENTRIES) {
                let (Ok(name), Some(id)) = (core::str::from_utf8(k), val.int()) else { continue };
                if name.len() > MAX_NAME || !(0..=255).contains(&id) { continue; }
                h.m.push((String::from(name), id as u8));
            }
        }
        h.client = v.get(b"v").and_then(|x| x.bytes()).map(|b| String::from_utf8_lossy(&b[..b.len().min(MAX_CLIENT)]).into_owned());
        h.port = v.get(b"p").and_then(|x| x.int()).filter(|p| (1..=65535).contains(p)).map(|p| p as u16);
        h.yourip = v.get(b"yourip").and_then(|x| x.bytes()).and_then(ip_from);
        h.reqq = v.get(b"reqq").and_then(|x| x.int()).filter(|q| *q > 0).map(|q| q.min(u32::MAX as i64) as u32);
        h.metadata_size = v.get(b"metadata_size").and_then(|x| x.int()).filter(|s| *s > 0 && *s <= MAX_METADATA as i64).map(|s| s as u32);
        h.ipv4 = v.get(b"ipv4").and_then(|x| x.bytes()).and_then(|b| b.try_into().ok());
        h.ipv6 = v.get(b"ipv6").and_then(|x| x.bytes()).and_then(|b| b.try_into().ok());
        h.upload_only = v.get(b"upload_only").and_then(|x| x.int()).is_some_and(|x| x != 0);
        Some(h)
    }

    /// The id the sender assigned to `name`, if enabled.
    pub fn id_of(&self, name: &str) -> Option<u8> { self.m.iter().find(|(n, _)| n == name).map(|(_, i)| *i).filter(|i| *i != 0) }
}

/// The ids a peer wants our extension messages sent under (0 = not supported).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RemoteExt { pub ut_metadata: u8, pub ut_pex: u8, pub lt_donthave: u8, pub upload_only: u8 }

impl RemoteExt {
    /// Applies a (possibly later, partial) handshake: names present update the id, an id
    /// of 0 disables, names absent keep their previous id (BEP 10).
    pub fn update(&mut self, h: &ExtHandshake) {
        for (n, i) in &h.m {
            match n.as_str() {
                "ut_metadata" => self.ut_metadata = *i,
                "ut_pex" => self.ut_pex = *i,
                "lt_donthave" => self.lt_donthave = *i,
                "upload_only" => self.upload_only = *i,
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------- ut_metadata (BEP 9)

/// Metadata is exchanged in pieces of this size.
pub const METADATA_PIECE: u32 = 16 * 1024;
/// We refuse info dictionaries larger than this.
pub const MAX_METADATA: u32 = 64 << 20;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MetadataMsg {
    Request(u32),
    Data { piece: u32, total_size: u32, data: Vec<u8> },
    Reject(u32),
}

impl MetadataMsg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            MetadataMsg::Request(p) => dict([(b"msg_type", 0.into()), (b"piece", (*p as i64).into())]).encode(),
            MetadataMsg::Data { piece, total_size, data } => {
                let mut v = dict([(b"msg_type", 1.into()), (b"piece", (*piece as i64).into()), (b"total_size", (*total_size as i64).into())]).encode();
                v.extend_from_slice(data);
                v
            }
            MetadataMsg::Reject(p) => dict([(b"msg_type", 2.into()), (b"piece", (*p as i64).into())]).encode(),
        }
    }
    pub fn decode(payload: &[u8]) -> Option<MetadataMsg> {
        let (v, used) = bencode::decode_prefix(payload).ok()?;
        let piece = v.get(b"piece")?.int().filter(|p| (0..=(MAX_METADATA / METADATA_PIECE) as i64).contains(p))? as u32;
        match v.get(b"msg_type")?.int()? {
            0 => Some(MetadataMsg::Request(piece)),
            1 => {
                let total = v.get(b"total_size")?.int().filter(|s| *s > 0 && *s <= MAX_METADATA as i64)? as u32;
                let data = payload.get(used..)?;
                if data.len() > METADATA_PIECE as usize { return None; }
                Some(MetadataMsg::Data { piece, total_size: total, data: data.to_vec() })
            }
            2 => Some(MetadataMsg::Reject(piece)),
            _ => None,
        }
    }
}

/// The answer to a metadata request when we have the info dictionary.
pub fn metadata_piece(info: &[u8], piece: u32) -> MetadataMsg {
    let start = piece as usize * METADATA_PIECE as usize;
    match info.get(start..(start + METADATA_PIECE as usize).min(info.len())) {
        Some(d) if !d.is_empty() => MetadataMsg::Data { piece, total_size: info.len() as u32, data: d.to_vec() },
        _ => MetadataMsg::Reject(piece),
    }
}

/// Checks an info dictionary against every hash the torrent is named by.
pub fn verify_info(hashes: &InfoHashes, info: &[u8]) -> bool {
    let v1 = hashes.v1().map(|h| Sha1::digest(info).as_slice() == h.0);
    let v2 = hashes.v2().map(|h| Sha256::digest(info).as_slice() == h.0);
    v1.unwrap_or(true) && v2.unwrap_or(true) && (v1.is_some() || v2.is_some())
}

/// Progress of a metadata download.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MetadataProgress {
    Incomplete,
    /// The verified info dictionary.
    Done(Vec<u8>),
    /// All pieces arrived but the hash did not match; the download restarted. The peers
    /// that sent pieces are listed so the caller can distrust them.
    Failed(Vec<PeerKey>),
}

/// Fetches the info dictionary of a magnet link from peers.
#[derive(Clone, Debug)]
pub struct MetadataAssembler {
    hashes: InfoHashes,
    size: u32,
    pieces: Vec<Option<(Vec<u8>, PeerKey)>>,
    requested: Vec<Option<(PeerKey, Millis)>>,
}

impl MetadataAssembler {
    pub fn new(hashes: InfoHashes) -> MetadataAssembler { MetadataAssembler { hashes, size: 0, pieces: Vec::new(), requested: Vec::new() } }
    pub fn size(&self) -> u32 { self.size }
    /// Adopts a size a peer announced (the first plausible one wins until a failure).
    pub fn set_size(&mut self, size: u32) -> bool {
        if self.size != 0 { return self.size == size; }
        if size == 0 || size > MAX_METADATA { return false; }
        self.size = size;
        let n = size.div_ceil(METADATA_PIECE) as usize;
        self.pieces = alloc::vec![None; n];
        self.requested = alloc::vec![None; n];
        true
    }
    fn piece_len(&self, p: u32) -> usize { (self.size - p * METADATA_PIECE).min(METADATA_PIECE) as usize }
    /// The next piece to ask `peer` for: a missing one not requested, or requested from
    /// someone else longer than `timeout` ago.
    pub fn next_request(&mut self, peer: PeerKey, now: Millis, timeout: Millis) -> Option<u32> {
        let i = (0..self.pieces.len()).find(|&i| self.pieces[i].is_none() && match self.requested[i] {
            None => true,
            Some((p, at)) => p != peer && now.saturating_sub(at) > timeout,
        })?;
        self.requested[i] = Some((peer, now));
        Some(i as u32)
    }
    /// How many pieces are requested from `peer` right now.
    pub fn outstanding(&self, peer: PeerKey) -> usize {
        self.requested.iter().zip(&self.pieces).filter(|(r, p)| p.is_none() && r.is_some_and(|(k, _)| k == peer)).count()
    }
    pub fn on_reject(&mut self, peer: PeerKey, piece: u32) {
        if let Some(r) = self.requested.get_mut(piece as usize) { if r.is_some_and(|(k, _)| k == peer) { *r = None; } }
    }
    pub fn peer_gone(&mut self, peer: PeerKey) {
        for r in self.requested.iter_mut() { if r.is_some_and(|(k, _)| k == peer) { *r = None; } }
    }
    pub fn on_data(&mut self, peer: PeerKey, piece: u32, total_size: u32, data: &[u8]) -> MetadataProgress {
        if self.size == 0 { self.set_size(total_size); }
        if total_size != self.size || piece as usize >= self.pieces.len() || data.len() != self.piece_len(piece) { return MetadataProgress::Incomplete; }
        let slot = &mut self.pieces[piece as usize];
        if slot.is_none() { *slot = Some((data.to_vec(), peer)); }
        if self.pieces.iter().any(|p| p.is_none()) { return MetadataProgress::Incomplete; }
        let mut info = Vec::with_capacity(self.size as usize);
        let mut who = Vec::new();
        for (d, k) in self.pieces.iter().flatten() { info.extend_from_slice(d); if !who.contains(k) { who.push(*k); } }
        if verify_info(&self.hashes, &info) { return MetadataProgress::Done(info); }
        *self = MetadataAssembler::new(self.hashes);
        MetadataProgress::Failed(who)
    }
}

// ---------------------------------------------------------------- ut_pex (BEP 11)

/// `added.f` flags.
pub const PEX_ENCRYPTION: u8 = 0x01;
pub const PEX_SEED: u8 = 0x02;
pub const PEX_UTP: u8 = 0x04;
pub const PEX_HOLEPUNCH: u8 = 0x08;
pub const PEX_REACHABLE: u8 = 0x10;
/// At most this many added and dropped peers go in one message (BEP 11).
pub const PEX_MAX: usize = 50;
/// Messages to one peer are at least this far apart.
pub const PEX_INTERVAL: Millis = 60_000;
/// We read at most this many entries of each list from a message.
const PEX_MAX_IN: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PexPeer { pub addr: PeerAddr, pub flags: u8 }

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PexMsg { pub added: Vec<PexPeer>, pub dropped: Vec<PeerAddr> }

impl PexMsg {
    pub fn encode(&self) -> Vec<u8> {
        let (mut a4, mut f4, mut a6, mut f6, mut d4, mut d6) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for p in &self.added {
            match p.addr.ip { Ip::V4(_) => { a4.extend(p.addr.compact()); f4.push(p.flags); } Ip::V6(_) => { a6.extend(p.addr.compact()); f6.push(p.flags); } }
        }
        for a in &self.dropped { match a.ip { Ip::V4(_) => d4.extend(a.compact()), Ip::V6(_) => d6.extend(a.compact()) } }
        dict([(b"added", a4.into()), (b"added.f", f4.into()), (b"added6", a6.into()), (b"added6.f", f6.into()), (b"dropped", d4.into()), (b"dropped6", d6.into())]).encode()
    }
    /// Decodes tolerantly (missing keys are empty lists), keeping at most 200 of each.
    pub fn decode(payload: &[u8]) -> Option<PexMsg> {
        let v = bencode::decode(payload).ok()?;
        v.dict()?;
        let list = |key: &[u8], six: bool| -> Vec<PeerAddr> {
            let n = if six { 18 } else { 6 };
            let b = v.get(key).and_then(|x| x.bytes()).unwrap_or(&[]);
            PeerAddr::parse_compact(&b[..b.len().min(PEX_MAX_IN * n)], six)
        };
        let flags = |key: &[u8]| v.get(key).and_then(|x| x.bytes()).unwrap_or(&[]).to_vec();
        let mut m = PexMsg::default();
        for (key, fkey, six) in [(&b"added"[..], &b"added.f"[..], false), (b"added6", b"added6.f", true)] {
            let f = flags(fkey);
            for (i, addr) in list(key, six).into_iter().enumerate() {
                if addr.port != 0 { m.added.push(PexPeer { addr, flags: f.get(i).copied().unwrap_or(0) }); }
            }
        }
        m.dropped = list(b"dropped", false);
        m.dropped.extend(list(b"dropped6", true));
        Some(m)
    }
}

/// What we have told one peer, so the next message carries only the difference.
#[derive(Clone, Debug, Default)]
pub struct PexState { last_sent: Option<Millis>, advertised: BTreeSet<PeerAddr> }

impl PexState {
    /// The message to send now given our current peers, or `None` when it is too soon
    /// (one per minute) or nothing changed.
    pub fn build(&mut self, current: &[PexPeer], now: Millis) -> Option<PexMsg> {
        if self.last_sent.is_some_and(|t| now.saturating_sub(t) < PEX_INTERVAL) { return None; }
        let cur: BTreeSet<PeerAddr> = current.iter().map(|p| p.addr).collect();
        let added: Vec<PexPeer> = current.iter().filter(|p| !self.advertised.contains(&p.addr)).take(PEX_MAX).copied().collect();
        let dropped: Vec<PeerAddr> = self.advertised.iter().filter(|a| !cur.contains(a)).take(PEX_MAX).copied().collect();
        if added.is_empty() && dropped.is_empty() { return None; }
        for a in &added { self.advertised.insert(a.addr); }
        for d in &dropped { self.advertised.remove(d); }
        self.last_sent = Some(now);
        Some(PexMsg { added, dropped })
    }
}

// ---------------------------------------------------------------- lt_donthave, upload_only

/// `lt_donthave`: the sender no longer has `piece` (4-byte big-endian payload).
pub fn encode_donthave(piece: u32) -> [u8; 4] { piece.to_be_bytes() }
pub fn decode_donthave(payload: &[u8]) -> Option<u32> { Some(u32::from_be_bytes(payload.try_into().ok()?)) }
/// `upload_only` message (libtorrent's form of BEP 21): one byte, nonzero = upload only.
pub fn encode_upload_only(yes: bool) -> [u8; 1] { [yes as u8] }
pub fn decode_upload_only(payload: &[u8]) -> Option<bool> { payload.first().map(|b| *b != 0) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InfoHashV1, InfoHashV2};
    use alloc::vec;

    #[test]
    fn handshake_round_trip() {
        let h = ExtHandshake::ours("Swingers 0.1", Some(6881), Some(Ip::V4([1, 2, 3, 4])), 250, Some(31235), true, false);
        let enc = h.encode();
        assert!(bencode::decode(&enc).unwrap().canonical());
        let d = ExtHandshake::decode(&enc).unwrap();
        // `m` comes back sorted by name.
        assert_eq!(d.id_of("ut_metadata"), Some(UT_METADATA));
        assert_eq!(d.id_of("ut_pex"), Some(UT_PEX));
        assert_eq!((d.client.as_deref(), d.port, d.yourip, d.reqq, d.metadata_size, d.upload_only), (Some("Swingers 0.1"), Some(6881), Some(Ip::V4([1, 2, 3, 4])), Some(250), Some(31235), true));
        let anon = ExtHandshake::ours("Swingers 0.1", Some(6881), None, 250, None, false, true).encode();
        assert!(!anon.windows(8).any(|w| w == b"Swingers"));
        assert!(!anon.windows(3).any(|w| w == b"1:p"));
    }

    #[test]
    fn bep10_example_and_hostile() {
        // The example from BEP 10.
        let d = ExtHandshake::decode(b"d1:md11:LT_metadatai1e6:ut_pexi2ee1:pi6881e1:v13:\xc2\xb5Torrent 1.2e").unwrap();
        assert_eq!(d.id_of("LT_metadata"), Some(1));
        assert_eq!(d.port, Some(6881));
        assert_eq!(d.client.as_deref(), Some("\u{b5}Torrent 1.2"));
        let mut r = RemoteExt::default();
        r.update(&d);
        assert_eq!(r.ut_pex, 2);
        r.update(&ExtHandshake::decode(b"d1:md6:ut_pexi0eee").unwrap());
        assert_eq!(r.ut_pex, 0, "id 0 disables");
        assert!(ExtHandshake::decode(b"le").is_none());
        assert!(ExtHandshake::decode(b"d1:pi99999999e1:mi5e4:reqqi-1ee").is_some_and(|h| h.port.is_none() && h.m.is_empty() && h.reqq.is_none()));
        assert!(ExtHandshake::decode(b"d13:metadata_sizei999999999999ee").unwrap().metadata_size.is_none());
    }

    #[test]
    fn metadata_messages() {
        assert_eq!(MetadataMsg::Request(0).encode(), b"d8:msg_typei0e5:piecei0ee");
        let data = MetadataMsg::Data { piece: 1, total_size: 20000, data: vec![7; 3616] };
        assert_eq!(MetadataMsg::decode(&data.encode()), Some(data));
        assert_eq!(MetadataMsg::decode(&MetadataMsg::Reject(3).encode()), Some(MetadataMsg::Reject(3)));
        assert_eq!(MetadataMsg::decode(b"d8:msg_typei9e5:piecei0ee"), None);
        assert_eq!(MetadataMsg::decode(b"d8:msg_typei0e5:piecei-1ee"), None);
        let mut big = MetadataMsg::Data { piece: 0, total_size: 40000, data: vec![] }.encode();
        big.extend(vec![0; 16385]);
        assert_eq!(MetadataMsg::decode(&big), None);
    }

    fn info_of(n: usize) -> Vec<u8> {
        let mut info = b"d6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces".to_vec();
        let pad = n.saturating_sub(info.len() + 8);
        info.extend(alloc::format!("{}:", pad).as_bytes());
        info.extend(vec![b'z'; pad]);
        info.push(b'e');
        info
    }

    #[test]
    fn assemble_from_two_peers_and_verify() {
        let info = info_of(40_000);
        let mut h = [0u8; 20];
        h.copy_from_slice(&Sha1::digest(&info));
        let hashes = InfoHashes::V1(InfoHashV1(h));
        let mut a = MetadataAssembler::new(hashes);
        assert!(a.set_size(info.len() as u32));
        assert_eq!(a.next_request(1, 0, 10_000), Some(0));
        assert_eq!(a.next_request(2, 0, 10_000), Some(1));
        assert_eq!(a.next_request(1, 0, 10_000), Some(2));
        assert_eq!(a.next_request(2, 0, 10_000), None);
        assert_eq!(a.outstanding(1), 2);
        a.on_reject(1, 2);
        assert_eq!(a.next_request(2, 0, 10_000), Some(2));
        let mut got = MetadataProgress::Incomplete;
        for (p, peer) in [(0u32, 1u32), (1, 2), (2, 2)] {
            if let MetadataMsg::Data { piece, total_size, data } = metadata_piece(&info, p) { got = a.on_data(peer, piece, total_size, &data); }
        }
        assert_eq!(got, MetadataProgress::Done(info.clone()));
        // A v2 hash and a hybrid are checked too.
        let mut h2 = [0u8; 32];
        h2.copy_from_slice(&Sha256::digest(&info));
        assert!(verify_info(&InfoHashes::V2(InfoHashV2(h2)), &info));
        assert!(verify_info(&InfoHashes::Hybrid(InfoHashV1(h), InfoHashV2(h2)), &info));
        assert!(!verify_info(&InfoHashes::Hybrid(InfoHashV1(h), InfoHashV2([0; 32])), &info));
        assert_eq!(metadata_piece(&info, 3), MetadataMsg::Reject(3));
    }

    #[test]
    fn assembler_rejects_bad_data() {
        let info = info_of(20_000);
        let mut a = MetadataAssembler::new(InfoHashes::V1(InfoHashV1([0; 20])));
        assert!(!a.set_size(MAX_METADATA + 1));
        assert!(a.set_size(info.len() as u32));
        assert!(!a.set_size(5));
        assert_eq!(a.on_data(1, 0, 999, &info[..16384]), MetadataProgress::Incomplete, "wrong total size");
        assert_eq!(a.on_data(1, 1, info.len() as u32, &info[..5]), MetadataProgress::Incomplete, "wrong length");
        assert_eq!(a.on_data(1, 9, info.len() as u32, &info[..5]), MetadataProgress::Incomplete, "out of range");
        a.on_data(1, 0, info.len() as u32, &info[..16384]);
        assert_eq!(a.on_data(2, 1, info.len() as u32, &info[16384..]), MetadataProgress::Failed(vec![1, 2]));
        assert_eq!(a.size(), 0, "restarted");
        // Timed-out requests go to another peer.
        a.set_size(100);
        assert_eq!(a.next_request(1, 0, 5000), Some(0));
        assert_eq!(a.next_request(2, 1000, 5000), None);
        assert_eq!(a.next_request(2, 6000, 5000), Some(0));
        a.peer_gone(2);
        assert_eq!(a.outstanding(2), 0);
    }

    fn addr(i: u8) -> PeerAddr { PeerAddr { ip: Ip::V4([10, 0, 0, i]), port: 6881 } }

    #[test]
    fn pex_round_trip_and_rate_limit() {
        let v6 = PeerAddr { ip: Ip::V6([2; 16]), port: 1 };
        let m = PexMsg { added: vec![PexPeer { addr: addr(1), flags: PEX_SEED }, PexPeer { addr: v6, flags: PEX_UTP }], dropped: vec![addr(9), v6] };
        assert_eq!(PexMsg::decode(&m.encode()), Some(m));
        assert_eq!(PexMsg::decode(b"d5:added6:\x01\x02\x03\x04\x1a\xe1e").unwrap().added, vec![PexPeer { addr: PeerAddr { ip: Ip::V4([1, 2, 3, 4]), port: 6881 }, flags: 0 }]);
        assert_eq!(PexMsg::decode(b"de"), Some(PexMsg::default()));
        assert!(PexMsg::decode(b"i1e").is_none());
        // 300 hostile entries are cut to 200.
        let mut big = b"d5:added1800:".to_vec();
        big.extend(vec![1; 1800]); big.push(b'e');
        assert_eq!(PexMsg::decode(&big).unwrap().added.len(), 200);

        let mut s = PexState::default();
        let peers: Vec<PexPeer> = (0..60).map(|i| PexPeer { addr: addr(i), flags: 0 }).collect();
        let first = s.build(&peers, 0).unwrap();
        assert_eq!(first.added.len(), PEX_MAX);
        assert!(s.build(&peers, 30_000).is_none(), "once per minute");
        let second = s.build(&peers[5..], 60_000).unwrap();
        assert_eq!(second.added.len(), 10);
        assert_eq!(second.dropped.len(), 5);
        assert!(s.build(&peers[5..], 200_000).is_none(), "nothing changed");
    }

    #[test]
    fn donthave_upload_only() {
        assert_eq!(decode_donthave(&encode_donthave(77)), Some(77));
        assert_eq!(decode_donthave(&[1, 2, 3]), None);
        assert_eq!(decode_upload_only(&encode_upload_only(true)), Some(true));
        assert_eq!(decode_upload_only(&[]), None);
    }
}
