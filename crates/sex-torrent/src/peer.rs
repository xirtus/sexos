// SPDX-License-Identifier: GPL-3.0-only
//! The peer wire protocol (BEP 3) and the per-connection state machine.
//!
//! A connection starts with a 68-byte handshake (protocol string, 8 reserved bytes that
//! advertise extensions, the info-hash and the peer ID), then carries length-prefixed
//! messages. This module has:
//! - [`Handshake`] and [`Reserved`] (extension protocol BEP 10, fast extension BEP 6,
//!   DHT BEP 5, v2 upgrade BEP 52),
//! - [`Message`] with every message we speak: the BEP 3 core, the BEP 6 fast extension,
//!   the BEP 52 hash request/hashes/hash reject framing and the BEP 10 extended message,
//! - a streaming [`Decoder`] that checks each message's length against a per-type cap
//!   *before* buffering it, so a hostile peer cannot make us allocate much,
//! - [`allowed_fast_set`] (BEP 6's canonical generator, with its test vector),
//! - [`Bitfield`] and [`RateMeter`], used by the rest of the engine,
//! - [`PeerConn`]: one connection's state (choke/interest flags, the peer's pieces, our
//!   outstanding requests with timeouts, a request pipeline whose depth follows the
//!   measured rate, snub detection, keepalives, idle timeout). It does not pick pieces or
//!   touch storage; it turns bytes into [`PeerEvent`]s for the session and queues the
//!   bytes the session asks it to send.
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use sha1::{Digest, Sha1};

use crate::{Ip, Millis};

/// The protocol string with its length prefix.
pub const PROTOCOL: &[u8; 20] = b"\x13BitTorrent protocol";
pub const HANDSHAKE_LEN: usize = 68;
/// The block size everyone requests (16 KiB); larger blocks are refused.
pub const BLOCK_SIZE: u32 = 16 * 1024;
/// Cap on an extended message (BEP 10 payload). ut_metadata pieces are 16 KiB.
pub const MAX_EXTENDED: usize = 256 * 1024;
/// Cap on the number of hashes in one BEP 52 `hashes` message.
pub const MAX_HASHES: usize = 2048;
/// Cap on a message with an id we do not know (skipped).
pub const MAX_UNKNOWN: usize = 64 * 1024;
/// Default cap on a bitfield when the piece count is not yet known (magnet links).
pub const MAX_BITFIELD: usize = 1 << 20;

/// A connection, as named by the embedding program (unique within a torrent).
pub type PeerKey = u32;

/// The 8 reserved handshake bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Reserved(pub [u8; 8]);

impl Reserved {
    /// The reserved bytes for the features we enable.
    pub fn new(extensions: bool, fast: bool, dht: bool, v2: bool) -> Reserved {
        let mut r = [0u8; 8];
        if extensions { r[5] |= 0x10; }
        if fast { r[7] |= 0x04; }
        if dht { r[7] |= 0x01; }
        if v2 { r[7] |= 0x10; }
        Reserved(r)
    }
    /// BEP 10 extension protocol (bit 20 from the right).
    pub fn extensions(&self) -> bool { self.0[5] & 0x10 != 0 }
    /// BEP 6 fast extension.
    pub fn fast(&self) -> bool { self.0[7] & 0x04 != 0 }
    /// BEP 5 DHT (the peer sends `port`).
    pub fn dht(&self) -> bool { self.0[7] & 0x01 != 0 }
    /// BEP 52 upgrade to v2 is supported.
    pub fn v2(&self) -> bool { self.0[7] & 0x10 != 0 }
}

/// The handshake both sides send first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Handshake {
    pub reserved: Reserved,
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl Handshake {
    pub fn encode(&self) -> [u8; HANDSHAKE_LEN] {
        let mut b = [0u8; HANDSHAKE_LEN];
        b[..20].copy_from_slice(PROTOCOL);
        b[20..28].copy_from_slice(&self.reserved.0);
        b[28..48].copy_from_slice(&self.info_hash);
        b[48..68].copy_from_slice(&self.peer_id);
        b
    }
    /// Parses a full 68-byte handshake from the start of `b`.
    pub fn parse(b: &[u8]) -> Result<Handshake, WireError> {
        let b = b.get(..HANDSHAKE_LEN).ok_or(WireError::Truncated)?;
        if &b[..20] != PROTOCOL { return Err(WireError::BadProtocol); }
        let mut h = Handshake { reserved: Reserved([0; 8]), info_hash: [0; 20], peer_id: [0; 20] };
        h.reserved.0.copy_from_slice(&b[20..28]);
        h.info_hash.copy_from_slice(&b[28..48]);
        h.peer_id.copy_from_slice(&b[48..68]);
        Ok(h)
    }
}

/// The info-hash of an incoming handshake, once its first 48 bytes have arrived, so the
/// session can route the connection to the right torrent. `Ok(None)`: need more bytes.
pub fn peek_info_hash(b: &[u8]) -> Result<Option<[u8; 20]>, WireError> {
    let n = b.len().min(20);
    if b[..n] != PROTOCOL[..n] { return Err(WireError::BadProtocol); }
    match b.get(28..48) {
        Some(h) => { let mut out = [0; 20]; out.copy_from_slice(h); Ok(Some(out)) }
        None => Ok(None),
    }
}

/// A block of a piece: what `request`, `cancel` and `reject` name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockRef { pub piece: u32, pub begin: u32, pub len: u32 }

/// A BEP 52 hash request (also the header of `hashes` and `hash reject`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HashRequest {
    pub pieces_root: [u8; 32],
    pub base_layer: u32,
    pub index: u32,
    pub length: u32,
    pub proof_layers: u32,
}

/// Every peer wire message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    /// Raw bitfield bytes (validated against the piece count by [`Bitfield::from_bytes`]).
    Bitfield(Vec<u8>),
    Request(BlockRef),
    Piece { piece: u32, begin: u32, data: Vec<u8> },
    Cancel(BlockRef),
    /// BEP 5: the peer's DHT port.
    Port(u16),
    /// BEP 6 fast extension.
    Suggest(u32),
    HaveAll,
    HaveNone,
    Reject(BlockRef),
    AllowedFast(u32),
    /// BEP 10: extended message; id 0 is the extended handshake.
    Extended { id: u8, payload: Vec<u8> },
    /// BEP 52.
    HashRequest(HashRequest),
    Hashes { req: HashRequest, hashes: Vec<[u8; 32]> },
    HashReject(HashRequest),
    /// A message id we do not know (already skipped).
    Unknown(u8),
}

/// Why bytes from a peer were refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireError {
    /// Not the BitTorrent protocol string.
    BadProtocol,
    /// Not enough bytes (only from [`Handshake::parse`]).
    Truncated,
    /// A message whose length is wrong for its type, or over its cap.
    BadLength { id: u8, len: u32 },
}

fn be32(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 4) { Some(s) => u32::from_be_bytes([s[0], s[1], s[2], s[3]]), None => 0 }
}

fn block_at(b: &[u8], at: usize) -> BlockRef { BlockRef { piece: be32(b, at), begin: be32(b, at + 4), len: be32(b, at + 8) } }

fn hash_req_at(b: &[u8]) -> HashRequest {
    let mut root = [0u8; 32];
    if let Some(s) = b.get(..32) { root.copy_from_slice(s); }
    HashRequest { pieces_root: root, base_layer: be32(b, 32), index: be32(b, 36), length: be32(b, 40), proof_layers: be32(b, 44) }
}

/// Checks a message's declared length (id byte included) against its type.
fn length_ok(id: u8, len: u32, max_bitfield: usize) -> bool {
    let len = len as usize;
    match id {
        0..=3 | 14 | 15 => len == 1,
        4 | 13 | 17 => len == 5,
        5 => len >= 2 && len - 1 <= max_bitfield,
        6 | 8 | 16 => len == 13,
        7 => (9..=9 + BLOCK_SIZE as usize).contains(&len),
        9 => len == 3,
        20 => (2..=1 + MAX_EXTENDED).contains(&len),
        21 | 23 => len == 49,
        22 => len >= 49 && (len - 49) % 32 == 0 && (len - 49) / 32 <= MAX_HASHES,
        _ => len <= 1 + MAX_UNKNOWN,
    }
}

/// Parses one message from the start of `buf`. `Ok(None)` means more bytes are needed;
/// an error is returned as soon as the length prefix and id show the message is bad,
/// without waiting for its body.
pub fn parse_message(buf: &[u8], max_bitfield: usize) -> Result<Option<(Message, usize)>, WireError> {
    if buf.len() < 4 { return Ok(None); }
    let len = be32(buf, 0);
    if len == 0 { return Ok(Some((Message::KeepAlive, 4))); }
    let Some(&id) = buf.get(4) else { return Ok(None) };
    if !length_ok(id, len, max_bitfield) { return Err(WireError::BadLength { id, len }); }
    let total = 4 + len as usize;
    let Some(body) = buf.get(5..total) else { return Ok(None) };
    let m = match id {
        0 => Message::Choke,
        1 => Message::Unchoke,
        2 => Message::Interested,
        3 => Message::NotInterested,
        4 => Message::Have(be32(body, 0)),
        5 => Message::Bitfield(body.to_vec()),
        6 => Message::Request(block_at(body, 0)),
        7 => Message::Piece { piece: be32(body, 0), begin: be32(body, 4), data: body.get(8..).unwrap_or(&[]).to_vec() },
        8 => Message::Cancel(block_at(body, 0)),
        9 => Message::Port(u16::from_be_bytes([body.first().copied().unwrap_or(0), body.get(1).copied().unwrap_or(0)])),
        13 => Message::Suggest(be32(body, 0)),
        14 => Message::HaveAll,
        15 => Message::HaveNone,
        16 => Message::Reject(block_at(body, 0)),
        17 => Message::AllowedFast(be32(body, 0)),
        20 => Message::Extended { id: body.first().copied().unwrap_or(0), payload: body.get(1..).unwrap_or(&[]).to_vec() },
        21 => Message::HashRequest(hash_req_at(body)),
        22 => {
            let hashes = body.get(48..).unwrap_or(&[]).chunks_exact(32).map(|c| { let mut h = [0u8; 32]; h.copy_from_slice(c); h }).collect();
            Message::Hashes { req: hash_req_at(body), hashes }
        }
        23 => Message::HashReject(hash_req_at(body)),
        other => Message::Unknown(other),
    };
    Ok(Some((m, total)))
}

fn put_block(out: &mut Vec<u8>, id: u8, b: &BlockRef) {
    out.extend_from_slice(&13u32.to_be_bytes());
    out.push(id);
    for v in [b.piece, b.begin, b.len] { out.extend_from_slice(&v.to_be_bytes()); }
}

fn put_hash_req(out: &mut Vec<u8>, id: u8, r: &HashRequest, extra: usize) {
    out.extend_from_slice(&((49 + extra) as u32).to_be_bytes());
    out.push(id);
    out.extend_from_slice(&r.pieces_root);
    for v in [r.base_layer, r.index, r.length, r.proof_layers] { out.extend_from_slice(&v.to_be_bytes()); }
}

impl Message {
    /// Appends the wire encoding to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let simple = |out: &mut Vec<u8>, id: u8| { out.extend_from_slice(&1u32.to_be_bytes()); out.push(id); };
        let with_u32 = |out: &mut Vec<u8>, id: u8, v: u32| { out.extend_from_slice(&5u32.to_be_bytes()); out.push(id); out.extend_from_slice(&v.to_be_bytes()); };
        match self {
            Message::KeepAlive => out.extend_from_slice(&[0; 4]),
            Message::Choke => simple(out, 0),
            Message::Unchoke => simple(out, 1),
            Message::Interested => simple(out, 2),
            Message::NotInterested => simple(out, 3),
            Message::Have(i) => with_u32(out, 4, *i),
            Message::Bitfield(b) => { out.extend_from_slice(&(1 + b.len() as u32).to_be_bytes()); out.push(5); out.extend_from_slice(b); }
            Message::Request(b) => put_block(out, 6, b),
            Message::Piece { piece, begin, data } => {
                out.extend_from_slice(&(9 + data.len() as u32).to_be_bytes());
                out.push(7);
                out.extend_from_slice(&piece.to_be_bytes());
                out.extend_from_slice(&begin.to_be_bytes());
                out.extend_from_slice(data);
            }
            Message::Cancel(b) => put_block(out, 8, b),
            Message::Port(p) => { out.extend_from_slice(&3u32.to_be_bytes()); out.push(9); out.extend_from_slice(&p.to_be_bytes()); }
            Message::Suggest(i) => with_u32(out, 13, *i),
            Message::HaveAll => simple(out, 14),
            Message::HaveNone => simple(out, 15),
            Message::Reject(b) => put_block(out, 16, b),
            Message::AllowedFast(i) => with_u32(out, 17, *i),
            Message::Extended { id, payload } => { out.extend_from_slice(&(2 + payload.len() as u32).to_be_bytes()); out.push(20); out.push(*id); out.extend_from_slice(payload); }
            Message::HashRequest(r) => put_hash_req(out, 21, r, 0),
            Message::Hashes { req, hashes } => { put_hash_req(out, 22, req, hashes.len() * 32); for h in hashes { out.extend_from_slice(h); } }
            Message::HashReject(r) => put_hash_req(out, 23, r, 0),
            Message::Unknown(id) => simple(out, *id),
        }
    }
    pub fn encode(&self) -> Vec<u8> { let mut v = Vec::new(); self.encode_into(&mut v); v }
}

/// Buffers bytes from one connection and yields whole messages (or the handshake).
#[derive(Debug, Default)]
pub struct Decoder { buf: Vec<u8>, pos: usize, max_bitfield: usize }

impl Decoder {
    pub fn new(max_bitfield: usize) -> Decoder { Decoder { buf: Vec::new(), pos: 0, max_bitfield } }
    pub fn set_max_bitfield(&mut self, n: usize) { self.max_bitfield = n; }
    pub fn feed(&mut self, data: &[u8]) {
        if self.pos > 0 && self.pos * 2 >= self.buf.len() { self.buf.drain(..self.pos); self.pos = 0; }
        self.buf.extend_from_slice(data);
    }
    /// Bytes buffered and not yet consumed.
    pub fn buffered(&self) -> usize { self.buf.len() - self.pos }
    /// The handshake, once 68 bytes are in.
    pub fn handshake(&mut self) -> Result<Option<Handshake>, WireError> {
        let rest = &self.buf[self.pos..];
        let n = rest.len().min(20);
        if rest[..n] != PROTOCOL[..n] { return Err(WireError::BadProtocol); }
        if rest.len() < HANDSHAKE_LEN { return Ok(None); }
        let h = Handshake::parse(rest)?;
        self.pos += HANDSHAKE_LEN;
        Ok(Some(h))
    }
    /// The next message, if a whole one is buffered.
    pub fn next(&mut self) -> Result<Option<Message>, WireError> {
        match parse_message(&self.buf[self.pos..], self.max_bitfield)? {
            Some((m, used)) => { self.pos += used; Ok(Some(m)) }
            None => Ok(None),
        }
    }
}

/// The BEP 6 allowed-fast set: `k` piece indices derived from the peer's address and
/// the info-hash. BEP 6 defines it for IPv4 (the /24 of the address); for IPv6 we apply
/// the same procedure to the /48 (the receiver honours whatever set it is sent).
pub fn allowed_fast_set(ip: Ip, info_hash: &[u8; 20], num_pieces: u32, k: usize) -> Vec<u32> {
    let k = k.min(num_pieces as usize);
    let mut x: Vec<u8> = match ip {
        Ip::V4(a) => alloc::vec![a[0], a[1], a[2], 0],
        Ip::V6(a) => { let mut v = alloc::vec![0u8; 16]; v[..6].copy_from_slice(&a[..6]); v }
    };
    x.extend_from_slice(info_hash);
    let mut out: Vec<u32> = Vec::with_capacity(k);
    while out.len() < k {
        x = Sha1::digest(&x).to_vec();
        for i in 0..5 {
            if out.len() >= k { break; }
            let y = be32(&x, i * 4);
            let index = y % num_pieces;
            if !out.contains(&index) { out.push(index); }
        }
    }
    out
}

/// A set of pieces, one bit each, high bit of the first byte = piece 0 (BEP 3).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Bitfield { bits: Vec<u8>, len: u32, ones: u32 }

impl Bitfield {
    pub fn new(len: u32) -> Bitfield { Bitfield { bits: alloc::vec![0; len.div_ceil(8) as usize], len, ones: 0 } }
    pub fn full(len: u32) -> Bitfield {
        let mut b = Bitfield { bits: alloc::vec![0xff; len.div_ceil(8) as usize], len, ones: len };
        b.clear_spare();
        b
    }
    fn clear_spare(&mut self) {
        let spare = (self.bits.len() as u32 * 8 - self.len) as u8;
        if let (Some(last), true) = (self.bits.last_mut(), spare > 0) { *last &= 0xffu8 << spare; }
    }
    /// From wire bytes: must be exactly `ceil(len/8)` bytes with the spare bits clear.
    pub fn from_bytes(bytes: &[u8], len: u32) -> Option<Bitfield> {
        if bytes.len() != len.div_ceil(8) as usize { return None; }
        let mut b = Bitfield { bits: bytes.to_vec(), len, ones: 0 };
        let before = b.bits.last().copied();
        b.clear_spare();
        if b.bits.last().copied() != before { return None; }
        b.ones = b.bits.iter().map(|x| x.count_ones()).sum();
        Some(b)
    }
    /// Like [`from_bytes`](Self::from_bytes) but lenient: wrong sizes are padded or cut
    /// and spare bits cleared (for resume data).
    pub fn from_bytes_lossy(bytes: &[u8], len: u32) -> Bitfield {
        let mut b = Bitfield::new(len);
        let n = b.bits.len().min(bytes.len());
        b.bits[..n].copy_from_slice(&bytes[..n]);
        b.clear_spare();
        b.ones = b.bits.iter().map(|x| x.count_ones()).sum();
        b
    }
    pub fn len(&self) -> u32 { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn get(&self, i: u32) -> bool {
        i < self.len && self.bits.get((i / 8) as usize).is_some_and(|b| b & (0x80 >> (i % 8)) != 0)
    }
    /// Sets bit `i`; returns whether it changed.
    pub fn set(&mut self, i: u32) -> bool {
        if i >= self.len || self.get(i) { return false; }
        if let Some(b) = self.bits.get_mut((i / 8) as usize) { *b |= 0x80 >> (i % 8); self.ones += 1; return true; }
        false
    }
    /// Clears bit `i`; returns whether it changed.
    pub fn clear(&mut self, i: u32) -> bool {
        if !self.get(i) { return false; }
        if let Some(b) = self.bits.get_mut((i / 8) as usize) { *b &= !(0x80 >> (i % 8)); self.ones -= 1; return true; }
        false
    }
    pub fn count(&self) -> u32 { self.ones }
    pub fn all(&self) -> bool { self.ones == self.len }
    pub fn none(&self) -> bool { self.ones == 0 }
    pub fn as_bytes(&self) -> &[u8] { &self.bits }
    /// Indices of set bits.
    pub fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.bits.iter().enumerate().filter(|(_, b)| **b != 0).flat_map(move |(i, b)| {
            (0..8u32).filter(move |j| b & (0x80 >> j) != 0).map(move |j| i as u32 * 8 + j)
        }).filter(move |&i| i < self.len)
    }
}

/// Bytes per second over a sliding window of the last few seconds.
#[derive(Clone, Debug, Default)]
pub struct RateMeter { buckets: [(u64, u64); RATE_WINDOW], start: Option<u64>, total: u64 }
const RATE_WINDOW: usize = 5;

impl RateMeter {
    pub fn add(&mut self, bytes: u64, now: Millis) {
        let sec = now / 1000;
        self.start.get_or_insert(sec);
        let slot = &mut self.buckets[(sec % RATE_WINDOW as u64) as usize];
        if slot.0 != sec { *slot = (sec, 0); }
        slot.1 += bytes;
        self.total = self.total.saturating_add(bytes);
    }
    /// Average rate over the window (shorter while the meter is young).
    pub fn rate(&self, now: Millis) -> u64 {
        let sec = now / 1000;
        let Some(start) = self.start else { return 0 };
        let sum: u64 = self.buckets.iter().filter(|(s, _)| *s + (RATE_WINDOW as u64) > sec && *s <= sec).map(|b| b.1).sum();
        let span = (sec.saturating_sub(start) + 1).min(RATE_WINDOW as u64);
        sum / span
    }
    pub fn total(&self) -> u64 { self.total }
}

/// Tunables for [`PeerConn`].
#[derive(Clone, Debug)]
pub struct PeerConfig {
    /// Request pipeline bounds, in blocks.
    pub min_queue: u32,
    pub max_queue: u32,
    /// The pipeline holds this many milliseconds' worth of data at the measured rate.
    pub queue_time: Millis,
    /// A request older than this with no block arriving meanwhile is timed out.
    pub request_timeout: Millis,
    /// No block for this long while requests are outstanding: the peer is snubbed.
    pub snub_after: Millis,
    /// Send a keepalive when nothing was sent for this long.
    pub keepalive: Millis,
    /// Close when nothing was received for this long.
    pub idle_timeout: Millis,
    /// Close if the handshake takes longer than this.
    pub handshake_timeout: Millis,
    /// Cap on queued requests from the peer (what we advertise as `reqq`).
    pub max_incoming: usize,
    /// Fast extension and extension protocol, as we advertise them.
    pub fast: bool,
    pub extensions: bool,
    pub dht: bool,
}

impl Default for PeerConfig {
    fn default() -> Self {
        PeerConfig {
            min_queue: 2, max_queue: 500, queue_time: 3_000, request_timeout: 30_000, snub_after: 60_000,
            keepalive: 60_000, idle_timeout: 180_000, handshake_timeout: 15_000, max_incoming: 250,
            fast: true, extensions: true, dht: true,
        }
    }
}

/// What a message from the peer means for the session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PeerEvent {
    /// The peer's handshake arrived (info-hash already checked).
    Handshake(Handshake),
    Have(u32),
    /// The peer sent its pieces (bitfield, have all, have none); read [`PeerConn::have`].
    Pieces,
    /// The peer's pieces for a torrent whose piece count is not known yet; they are kept
    /// until [`PeerConn::set_num_pieces`].
    PiecesPending,
    /// A block arrived; `requested` is false if we had not (or no longer) requested it.
    Block { block: BlockRef, data: Vec<u8>, requested: bool },
    /// The peer wants a block from us (already checked: not choked or allowed fast).
    Request(BlockRef),
    Cancel(BlockRef),
    /// The peer rejected our request (fast extension).
    Rejected(BlockRef),
    /// The peer choked us; without the fast extension our requests are dropped.
    Choked { dropped: Vec<BlockRef> },
    Unchoked,
    Interested,
    NotInterested,
    Port(u16),
    Suggest(u32),
    AllowedFast(u32),
    Extended { id: u8, payload: Vec<u8> },
    HashRequest(HashRequest),
    Hashes { req: HashRequest, hashes: Vec<[u8; 32]> },
    HashReject(HashRequest),
}

/// Why a connection must be closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeerError {
    Wire(WireError),
    WrongInfoHash,
    /// A message that is not allowed here (bitfield twice, fast message without fast...).
    Protocol,
    /// A piece index out of range, or a bitfield of the wrong size.
    BadPiece,
    HandshakeTimeout,
    Idle,
}

impl From<WireError> for PeerError { fn from(e: WireError) -> Self { PeerError::Wire(e) } }

/// One of our requests waiting for its block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Outstanding { pub block: BlockRef, pub sent: Millis }

#[derive(Clone, Debug, Default)]
struct PendingPieces { bitfield: Option<Vec<u8>>, have_all: bool, haves: Vec<u32> }

const MAX_PENDING_HAVES: usize = 1 << 16;
const MAX_HINTS: usize = 64;

/// One peer connection's protocol state.
#[derive(Debug)]
pub struct PeerConn {
    pub cfg: PeerConfig,
    our: Handshake,
    alt_hash: Option<[u8; 20]>,
    pub outgoing: bool,
    /// The peer's handshake, once it arrived.
    pub remote: Option<Handshake>,
    dec: Decoder,
    out: Vec<u8>,
    pub am_choking: bool,
    pub am_interested: bool,
    pub peer_choking: bool,
    pub peer_interested: bool,
    /// The peer's pieces (empty until the piece count is known).
    pub have: Bitfield,
    num_pieces: u32,
    pending: PendingPieces,
    got_pieces: bool,
    /// Our requests in flight, oldest first.
    pub outstanding: Vec<Outstanding>,
    /// The peer's requests we have accepted and not yet served.
    pub incoming: VecDeque<BlockRef>,
    /// Pieces the peer lets us request while choked (BEP 6).
    pub allowed_fast_in: Vec<u32>,
    /// Pieces we let the peer request while we choke it.
    pub allowed_fast_out: Vec<u32>,
    pub suggested: Vec<u32>,
    /// Payload rates.
    pub down: RateMeter,
    pub up: RateMeter,
    pub connected_at: Millis,
    pub last_recv: Millis,
    pub last_sent: Millis,
    last_block: Millis,
    pub snubbed: bool,
    /// The peer's request queue limit (`reqq` from its extended handshake).
    pub peer_reqq: u32,
}

impl PeerConn {
    /// A new connection. An outgoing one sends our handshake at once; an incoming one
    /// answers when the peer's arrives. `num_pieces` is 0 while metadata is missing.
    /// `alt_hash` is a second info-hash we answer to (the truncated v2 hash of a hybrid).
    pub fn new(cfg: PeerConfig, our: Handshake, alt_hash: Option<[u8; 20]>, outgoing: bool, num_pieces: u32, now: Millis) -> PeerConn {
        let max_bf = if num_pieces > 0 { num_pieces.div_ceil(8) as usize } else { MAX_BITFIELD };
        let mut c = PeerConn {
            cfg, our, alt_hash, outgoing, remote: None, dec: Decoder::new(max_bf), out: Vec::new(),
            am_choking: true, am_interested: false, peer_choking: true, peer_interested: false,
            have: Bitfield::new(num_pieces), num_pieces, pending: PendingPieces::default(), got_pieces: false,
            outstanding: Vec::new(), incoming: VecDeque::new(), allowed_fast_in: Vec::new(), allowed_fast_out: Vec::new(),
            suggested: Vec::new(), down: RateMeter::default(), up: RateMeter::default(),
            connected_at: now, last_recv: now, last_sent: now, last_block: now, snubbed: false, peer_reqq: 250,
        };
        if outgoing { c.out.extend_from_slice(&our.encode()); }
        c
    }

    /// Both sides speak the fast extension.
    pub fn fast(&self) -> bool { self.cfg.fast && self.remote.is_some_and(|h| h.reserved.fast()) }
    /// Both sides speak the extension protocol.
    pub fn extensions(&self) -> bool { self.cfg.extensions && self.remote.is_some_and(|h| h.reserved.extensions()) }
    pub fn is_open(&self) -> bool { self.remote.is_some() }
    pub fn is_seed(&self) -> bool { self.num_pieces > 0 && self.have.all() }

    /// Bytes waiting to be sent.
    pub fn take_outgoing(&mut self) -> Vec<u8> { core::mem::take(&mut self.out) }
    pub fn has_outgoing(&self) -> bool { !self.out.is_empty() }

    fn send(&mut self, m: &Message, now: Millis) { m.encode_into(&mut self.out); self.last_sent = now; }

    /// Feeds bytes from the socket.
    pub fn on_bytes(&mut self, data: &[u8], now: Millis) -> Result<Vec<PeerEvent>, PeerError> {
        self.dec.feed(data);
        self.last_recv = now;
        let mut events = Vec::new();
        if self.remote.is_none() {
            let Some(h) = self.dec.handshake()? else { return Ok(events) };
            if h.info_hash != self.our.info_hash {
                if Some(h.info_hash) == self.alt_hash && !self.outgoing { self.our.info_hash = h.info_hash; } else { return Err(PeerError::WrongInfoHash); }
            }
            self.remote = Some(h);
            if !self.outgoing { let hs = self.our.encode(); self.out.extend_from_slice(&hs); self.last_sent = now; }
            events.push(PeerEvent::Handshake(h));
        }
        while let Some(m) = self.dec.next()? {
            self.on_message(m, now, &mut events)?;
        }
        Ok(events)
    }

    fn on_message(&mut self, m: Message, now: Millis, ev: &mut Vec<PeerEvent>) -> Result<(), PeerError> {
        let fast = self.fast();
        match m {
            Message::KeepAlive | Message::Unknown(_) => {}
            Message::Choke => {
                self.peer_choking = true;
                let dropped = if fast { Vec::new() } else { self.outstanding.drain(..).map(|o| o.block).collect() };
                ev.push(PeerEvent::Choked { dropped });
            }
            Message::Unchoke => { self.peer_choking = false; self.last_block = now; ev.push(PeerEvent::Unchoked); }
            Message::Interested => { self.peer_interested = true; ev.push(PeerEvent::Interested); }
            Message::NotInterested => { self.peer_interested = false; ev.push(PeerEvent::NotInterested); }
            Message::Have(i) => {
                self.got_pieces = true;
                if self.num_pieces == 0 {
                    if self.pending.haves.len() < MAX_PENDING_HAVES { self.pending.haves.push(i); }
                    ev.push(PeerEvent::PiecesPending);
                } else if i >= self.num_pieces {
                    return Err(PeerError::BadPiece);
                } else if self.have.set(i) {
                    ev.push(PeerEvent::Have(i));
                }
            }
            Message::Bitfield(b) => {
                // Some clients (aria2) send `have none` and later a bitfield of what they
                // got since; treat a late bitfield as a run of `have`s instead of failing.
                if self.got_pieces && self.num_pieces > 0 {
                    let late = Bitfield::from_bytes(&b, self.num_pieces).ok_or(PeerError::BadPiece)?;
                    let new: Vec<u32> = late.ones().collect();
                    for i in new { if self.have.set(i) { ev.push(PeerEvent::Have(i)); } }
                    return Ok(());
                }
                if self.got_pieces { return Err(PeerError::Protocol); }
                self.got_pieces = true;
                if self.num_pieces == 0 { self.pending.bitfield = Some(b); ev.push(PeerEvent::PiecesPending); }
                else { self.have = Bitfield::from_bytes(&b, self.num_pieces).ok_or(PeerError::BadPiece)?; ev.push(PeerEvent::Pieces); }
            }
            Message::HaveAll | Message::HaveNone => {
                if !fast { return Err(PeerError::Protocol); }
                let all = m == Message::HaveAll;
                // Late: `have none` adds nothing; `have all` is every piece.
                if self.got_pieces && self.num_pieces > 0 {
                    if all { for i in 0..self.num_pieces { if self.have.set(i) { ev.push(PeerEvent::Have(i)); } } }
                    return Ok(());
                }
                if self.got_pieces { return Err(PeerError::Protocol); }
                self.got_pieces = true;
                if self.num_pieces == 0 { self.pending.have_all = all; ev.push(PeerEvent::PiecesPending); }
                else { self.have = if all { Bitfield::full(self.num_pieces) } else { Bitfield::new(self.num_pieces) }; ev.push(PeerEvent::Pieces); }
            }
            Message::Request(b) => {
                let allowed = !self.am_choking || (fast && self.allowed_fast_out.contains(&b.piece));
                if b.len == 0 || b.len > BLOCK_SIZE || !allowed || self.incoming.len() >= self.cfg.max_incoming {
                    if fast { self.send(&Message::Reject(b), now); }
                } else if !self.incoming.contains(&b) {
                    self.incoming.push_back(b);
                    ev.push(PeerEvent::Request(b));
                }
            }
            Message::Piece { piece, begin, data } => {
                let block = BlockRef { piece, begin, len: data.len() as u32 };
                let pos = self.outstanding.iter().position(|o| o.block == block);
                if let Some(i) = pos { self.outstanding.remove(i); }
                self.down.add(data.len() as u64, now);
                self.last_block = now;
                self.snubbed = false;
                ev.push(PeerEvent::Block { block, data, requested: pos.is_some() });
            }
            Message::Cancel(b) => {
                if let Some(i) = self.incoming.iter().position(|x| *x == b) { self.incoming.remove(i); ev.push(PeerEvent::Cancel(b)); }
            }
            Message::Reject(b) => {
                if !fast { return Err(PeerError::Protocol); }
                if let Some(i) = self.outstanding.iter().position(|o| o.block == b) { self.outstanding.remove(i); ev.push(PeerEvent::Rejected(b)); }
            }
            Message::Port(p) => ev.push(PeerEvent::Port(p)),
            Message::Suggest(i) => {
                if !fast { return Err(PeerError::Protocol); }
                if self.suggested.len() < MAX_HINTS && !self.suggested.contains(&i) { self.suggested.push(i); ev.push(PeerEvent::Suggest(i)); }
            }
            Message::AllowedFast(i) => {
                if !fast { return Err(PeerError::Protocol); }
                if self.allowed_fast_in.len() < MAX_HINTS && !self.allowed_fast_in.contains(&i) { self.allowed_fast_in.push(i); ev.push(PeerEvent::AllowedFast(i)); }
            }
            Message::Extended { id, payload } => { if self.extensions() { ev.push(PeerEvent::Extended { id, payload }); } }
            Message::HashRequest(r) => ev.push(PeerEvent::HashRequest(r)),
            Message::Hashes { req, hashes } => ev.push(PeerEvent::Hashes { req, hashes }),
            Message::HashReject(r) => ev.push(PeerEvent::HashReject(r)),
        }
        Ok(())
    }

    /// Sets the piece count once metadata is known, applying the pieces the peer sent
    /// before. Errors if they do not fit (the connection should be closed).
    pub fn set_num_pieces(&mut self, n: u32) -> Result<(), PeerError> {
        if self.num_pieces == n { return Ok(()); }
        self.num_pieces = n;
        self.dec.set_max_bitfield(n.div_ceil(8) as usize);
        let p = core::mem::take(&mut self.pending);
        self.have = if p.have_all { Bitfield::full(n) } else if let Some(b) = p.bitfield { Bitfield::from_bytes(&b, n).ok_or(PeerError::BadPiece)? } else { Bitfield::new(n) };
        for i in p.haves { if i >= n { return Err(PeerError::BadPiece); } self.have.set(i); }
        Ok(())
    }

    /// How many more blocks we should request now: the pipeline depth follows the rate.
    pub fn request_room(&self, now: Millis) -> usize {
        let target = if self.snubbed { 1 } else {
            let by_rate = self.down.rate(now).saturating_mul(self.cfg.queue_time) / 1000 / BLOCK_SIZE as u64;
            (by_rate as u32).clamp(self.cfg.min_queue, self.cfg.max_queue.min(self.peer_reqq.max(1)))
        };
        (target as usize).saturating_sub(self.outstanding.len())
    }

    // Things the session asks us to send.

    pub fn choke(&mut self, now: Millis) -> Vec<BlockRef> {
        if self.am_choking { return Vec::new(); }
        self.am_choking = true;
        self.send(&Message::Choke, now);
        let fast = self.fast();
        let mut dropped = Vec::new();
        let keep: VecDeque<BlockRef> = self.incoming.drain(..).filter(|b| {
            let keep = fast && self.allowed_fast_out.contains(&b.piece);
            if !keep { dropped.push(*b); }
            keep
        }).collect();
        self.incoming = keep;
        if fast { for b in &dropped { Message::Reject(*b).encode_into(&mut self.out); } }
        dropped
    }
    pub fn unchoke(&mut self, now: Millis) { if self.am_choking { self.am_choking = false; self.send(&Message::Unchoke, now); } }
    pub fn set_interested(&mut self, yes: bool, now: Millis) {
        if self.am_interested != yes { self.am_interested = yes; self.send(if yes { &Message::Interested } else { &Message::NotInterested }, now); }
    }
    pub fn send_have(&mut self, piece: u32, now: Millis) { self.send(&Message::Have(piece), now); }
    /// Sends our pieces: have all / have none when both speak fast, otherwise a bitfield
    /// (skipped when empty, which BEP 3 allows).
    pub fn send_pieces(&mut self, have: &Bitfield, now: Millis) {
        if self.fast() && have.all() { self.send(&Message::HaveAll, now); }
        else if self.fast() && have.none() { self.send(&Message::HaveNone, now); }
        else if !have.none() { self.send(&Message::Bitfield(have.as_bytes().to_vec()), now); }
    }
    pub fn request(&mut self, b: BlockRef, now: Millis) {
        if self.outstanding.is_empty() { self.last_block = self.last_block.max(now); }
        self.outstanding.push(Outstanding { block: b, sent: now });
        self.send(&Message::Request(b), now);
    }
    /// Cancels one of our requests (end game, or a timeout).
    pub fn cancel(&mut self, b: BlockRef, now: Millis) {
        if let Some(i) = self.outstanding.iter().position(|o| o.block == b) { self.outstanding.remove(i); self.send(&Message::Cancel(b), now); }
    }
    /// Sends a block the peer asked for, if the request still stands.
    pub fn send_block(&mut self, b: BlockRef, data: &[u8], now: Millis) -> bool {
        let Some(i) = self.incoming.iter().position(|x| *x == b) else { return false };
        self.incoming.remove(i);
        self.out.extend_from_slice(&(9 + data.len() as u32).to_be_bytes());
        self.out.push(7);
        self.out.extend_from_slice(&b.piece.to_be_bytes());
        self.out.extend_from_slice(&b.begin.to_be_bytes());
        self.out.extend_from_slice(data);
        self.last_sent = now;
        self.up.add(data.len() as u64, now);
        true
    }
    /// Refuses one of the peer's requests (rejects it if fast, else drops it silently).
    pub fn reject(&mut self, b: BlockRef, now: Millis) {
        if let Some(i) = self.incoming.iter().position(|x| *x == b) { self.incoming.remove(i); }
        if self.fast() { self.send(&Message::Reject(b), now); }
    }
    pub fn send_allowed_fast(&mut self, piece: u32, now: Millis) {
        if self.fast() && !self.allowed_fast_out.contains(&piece) { self.allowed_fast_out.push(piece); self.send(&Message::AllowedFast(piece), now); }
    }
    pub fn send_extended(&mut self, id: u8, payload: &[u8], now: Millis) {
        self.out.extend_from_slice(&(2 + payload.len() as u32).to_be_bytes());
        self.out.push(20);
        self.out.push(id);
        self.out.extend_from_slice(payload);
        self.last_sent = now;
    }
    pub fn send_message(&mut self, m: &Message, now: Millis) { self.send(m, now); }

    /// Timers: returns requests that timed out (already cancelled with the peer), or an
    /// error if the connection should be closed.
    pub fn on_tick(&mut self, now: Millis) -> Result<Vec<BlockRef>, PeerError> {
        if self.remote.is_none() && now.saturating_sub(self.connected_at) > self.cfg.handshake_timeout { return Err(PeerError::HandshakeTimeout); }
        if now.saturating_sub(self.last_recv) > self.cfg.idle_timeout { return Err(PeerError::Idle); }
        let mut timed_out = Vec::new();
        if !self.outstanding.is_empty() {
            let quiet = now.saturating_sub(self.last_block);
            if quiet > self.cfg.snub_after { self.snubbed = true; }
            if quiet > self.cfg.request_timeout {
                let limit = self.cfg.request_timeout;
                let (old, keep): (Vec<Outstanding>, Vec<Outstanding>) = self.outstanding.drain(..).partition(|o| now.saturating_sub(o.sent) > limit);
                self.outstanding = keep;
                for o in old { Message::Cancel(o.block).encode_into(&mut self.out); timed_out.push(o.block); }
                if !timed_out.is_empty() { self.last_sent = now; }
            }
        }
        if self.remote.is_some() && now.saturating_sub(self.last_sent) >= self.cfg.keepalive { self.send(&Message::KeepAlive, now); }
        Ok(timed_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn hs(fast: bool) -> Handshake { Handshake { reserved: Reserved::new(true, fast, true, false), info_hash: [7; 20], peer_id: [1; 20] } }

    #[test]
    fn reserved_bits() {
        let r = Reserved::new(true, true, true, true);
        assert_eq!(r.0, [0, 0, 0, 0, 0, 0x10, 0, 0x15]);
        assert!(r.extensions() && r.fast() && r.dht() && r.v2());
        assert!(!Reserved::default().fast());
    }

    #[test]
    fn handshake_round_trip_and_errors() {
        let h = hs(true);
        let b = h.encode();
        assert_eq!(&b[..20], b"\x13BitTorrent protocol");
        assert_eq!(Handshake::parse(&b), Ok(h));
        assert_eq!(Handshake::parse(&b[..67]), Err(WireError::Truncated));
        let mut bad = b; bad[3] = b'X';
        assert_eq!(Handshake::parse(&bad), Err(WireError::BadProtocol));
        assert_eq!(peek_info_hash(&b[..40]), Ok(None));
        assert_eq!(peek_info_hash(&b[..48]), Ok(Some([7; 20])));
        assert_eq!(peek_info_hash(b"GET / HTTP/1.1"), Err(WireError::BadProtocol));
    }

    #[test]
    fn every_message_round_trips() {
        let r = HashRequest { pieces_root: [9; 32], base_layer: 1, index: 2, length: 3, proof_layers: 4 };
        let b = BlockRef { piece: 1, begin: 16384, len: 16384 };
        let all = vec![
            Message::KeepAlive, Message::Choke, Message::Unchoke, Message::Interested, Message::NotInterested,
            Message::Have(42), Message::Bitfield(vec![0xf0, 0x80]), Message::Request(b),
            Message::Piece { piece: 3, begin: 0, data: vec![5; 100] }, Message::Cancel(b), Message::Port(6881),
            Message::Suggest(4), Message::HaveAll, Message::HaveNone, Message::Reject(b), Message::AllowedFast(9),
            Message::Extended { id: 0, payload: b"d1:md11:ut_metadatai1eee".to_vec() },
            Message::HashRequest(r), Message::Hashes { req: r, hashes: vec![[1; 32], [2; 32]] }, Message::HashReject(r),
        ];
        let mut stream = Vec::new();
        for m in &all { m.encode_into(&mut stream); }
        let mut d = Decoder::new(MAX_BITFIELD);
        // Byte by byte, to exercise the streaming path.
        let mut got = Vec::new();
        for byte in &stream { d.feed(&[*byte]); while let Some(m) = d.next().unwrap() { got.push(m); } }
        assert_eq!(got, all);
        assert_eq!(d.buffered(), 0);
    }

    #[test]
    fn goldens() {
        assert_eq!(Message::Have(1).encode(), [0, 0, 0, 5, 4, 0, 0, 0, 1]);
        assert_eq!(Message::Request(BlockRef { piece: 0, begin: 0, len: 0x4000 }).encode(), [0, 0, 0, 13, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0]);
        assert_eq!(Message::HaveAll.encode(), [0, 0, 0, 1, 14]);
        assert_eq!(Message::Port(0x1ae1).encode(), [0, 0, 0, 3, 9, 0x1a, 0xe1]);
    }

    #[test]
    fn hostile_lengths_refused_before_buffering() {
        // A piece message claiming 1 GiB: refused from the 5-byte header alone.
        let mut d = Decoder::new(MAX_BITFIELD);
        d.feed(&[0x40, 0, 0, 0, 7]);
        assert!(matches!(d.next(), Err(WireError::BadLength { id: 7, .. })));
        for (id, len) in [(0u8, 2u32), (4, 4), (6, 12), (9, 5), (20, 1), (5, 1), (22, 50), (21, 48), (99, 1 << 20)] {
            let mut b = len.to_be_bytes().to_vec(); b.push(id);
            assert!(parse_message(&b, 16).is_err(), "id {id} len {len}");
        }
        // Bitfield longer than the piece count allows.
        let mut b = 6u32.to_be_bytes().to_vec(); b.push(5);
        assert!(parse_message(&b, 4).is_err());
        // Unknown ids within the cap are skipped.
        let mut u = 3u32.to_be_bytes().to_vec(); u.extend_from_slice(&[200, 1, 2]);
        assert_eq!(parse_message(&u, 4).unwrap(), Some((Message::Unknown(200), 7)));
        // Random garbage never panics.
        let mut seed = 0x1234_5678u32;
        for _ in 0..2000 {
            let mut buf = Vec::new();
            for _ in 0..(seed % 64) { seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345); buf.push((seed >> 16) as u8); }
            let _ = parse_message(&buf, 64);
            let mut c = PeerConn::new(PeerConfig::default(), hs(true), None, false, 10, 0);
            let mut x = hs(true).encode().to_vec(); x.extend_from_slice(&buf);
            let _ = c.on_bytes(&x, 1);
        }
    }

    #[test]
    fn bep6_allowed_fast_vector() {
        let ip = Ip::V4([80, 4, 4, 200]);
        assert_eq!(allowed_fast_set(ip, &[0xaa; 20], 1313, 7), vec![1059, 431, 808, 1217, 287, 376, 1188]);
        assert_eq!(allowed_fast_set(ip, &[0xaa; 20], 1313, 9), vec![1059, 431, 808, 1217, 287, 376, 1188, 353, 508]);
        // More than there are pieces terminates.
        assert_eq!(allowed_fast_set(ip, &[0xaa; 20], 3, 10).len(), 3);
        assert_eq!(allowed_fast_set(Ip::V6([1; 16]), &[0xaa; 20], 50, 5).len(), 5);
    }

    #[test]
    fn bitfield_rules() {
        let b = Bitfield::from_bytes(&[0b1010_0000], 3).unwrap();
        assert!(b.get(0) && !b.get(1) && b.get(2) && !b.get(3) && !b.get(100));
        assert_eq!(b.count(), 2);
        assert_eq!(b.ones().collect::<Vec<_>>(), vec![0, 2]);
        assert!(Bitfield::from_bytes(&[0b1011_0000], 3).is_none(), "spare bit set");
        assert!(Bitfield::from_bytes(&[0, 0], 3).is_none(), "wrong size");
        let mut f = Bitfield::full(10);
        assert!(f.all() && f.as_bytes() == [0xff, 0xc0]);
        assert!(f.clear(9) && !f.clear(9) && !f.all());
        assert!(f.set(9) && !f.set(10));
        assert_eq!(Bitfield::from_bytes_lossy(&[0xff, 0xff, 0xff], 10).count(), 10);
    }

    #[test]
    fn rate_meter_window() {
        let mut r = RateMeter::default();
        assert_eq!(r.rate(0), 0);
        r.add(1000, 0);
        assert_eq!(r.rate(500), 1000);
        for s in 1..5 { r.add(1000, s * 1000); }
        assert_eq!(r.rate(4_500), 1000);
        assert_eq!(r.rate(20_000), 0, "old buckets expire");
        assert_eq!(r.total(), 5000);
    }

    /// Two connections wired back to back.
    fn pair(fast: bool) -> (PeerConn, PeerConn) {
        let mut cfg = PeerConfig::default();
        cfg.fast = fast;
        let mut a = PeerConn::new(cfg.clone(), Handshake { peer_id: [1; 20], ..hs(true) }, None, true, 10, 0);
        let mut b = PeerConn::new(cfg, Handshake { peer_id: [2; 20], ..hs(true) }, None, false, 10, 0);
        let ev = b.on_bytes(&a.take_outgoing(), 0).unwrap();
        assert!(matches!(ev[0], PeerEvent::Handshake(h) if h.peer_id == [1; 20]));
        let ev = a.on_bytes(&b.take_outgoing(), 0).unwrap();
        assert!(matches!(ev[0], PeerEvent::Handshake(h) if h.peer_id == [2; 20]));
        (a, b)
    }

    #[test]
    fn conversation() {
        let (mut a, mut b) = pair(true);
        assert!(a.fast() && b.fast());
        b.send_pieces(&Bitfield::full(10), 0);
        assert_eq!(a.on_bytes(&b.take_outgoing(), 1).unwrap(), vec![PeerEvent::Pieces]);
        assert!(a.is_seed());
        a.set_interested(true, 1);
        assert_eq!(b.on_bytes(&a.take_outgoing(), 1).unwrap(), vec![PeerEvent::Interested]);
        // A request while choked is rejected (fast).
        let blk = BlockRef { piece: 2, begin: 0, len: 16384 };
        a.request(blk, 2);
        assert!(b.on_bytes(&a.take_outgoing(), 2).unwrap().is_empty());
        assert_eq!(a.on_bytes(&b.take_outgoing(), 2).unwrap(), vec![PeerEvent::Rejected(blk)]);
        assert!(a.outstanding.is_empty());
        // Unchoke, request, serve.
        b.unchoke(3);
        a.on_bytes(&b.take_outgoing(), 3).unwrap();
        assert!(!a.peer_choking);
        a.request(blk, 3);
        assert_eq!(b.on_bytes(&a.take_outgoing(), 3).unwrap(), vec![PeerEvent::Request(blk)]);
        assert!(b.send_block(blk, &[9; 16384], 4));
        let ev = a.on_bytes(&b.take_outgoing(), 4).unwrap();
        assert!(matches!(&ev[0], PeerEvent::Block { block, requested: true, data } if *block == blk && data.len() == 16384));
        assert_eq!(a.down.total(), 16384);
        // A late `have none` or bitfield (aria2 sends both) is tolerated: a late bitfield
        // counts as `have`s, a late `have none` changes nothing.
        b.send_message(&Message::HaveNone, 5);
        assert_eq!(a.on_bytes(&b.take_outgoing(), 5), Ok(vec![]));
        let mut late = Bitfield::new(a.have.len());
        late.set(1);
        b.send_message(&Message::Bitfield(late.as_bytes().to_vec()), 5);
        let ev = a.on_bytes(&b.take_outgoing(), 5).unwrap();
        assert!(ev.contains(&PeerEvent::Have(1)) || a.have.get(1));
    }

    #[test]
    fn choke_without_fast_drops_requests() {
        let (mut a, mut b) = pair(false);
        assert!(!a.fast());
        b.unchoke(0);
        a.on_bytes(&b.take_outgoing(), 0).unwrap();
        let blk = BlockRef { piece: 0, begin: 0, len: 100 };
        a.request(blk, 0);
        b.on_bytes(&a.take_outgoing(), 0).unwrap();
        assert_eq!(b.incoming.len(), 1);
        assert_eq!(b.choke(1), vec![blk]);
        assert_eq!(a.on_bytes(&b.take_outgoing(), 1).unwrap(), vec![PeerEvent::Choked { dropped: vec![blk] }]);
        // Fast-only messages without fast close the connection.
        b.send_message(&Message::Reject(blk), 2);
        assert_eq!(a.on_bytes(&b.take_outgoing(), 2), Err(PeerError::Protocol));
    }

    #[test]
    fn wrong_info_hash_and_alt_hash() {
        let mut a = PeerConn::new(PeerConfig::default(), hs(true), None, false, 10, 0);
        let other = Handshake { info_hash: [8; 20], ..hs(true) };
        assert_eq!(a.on_bytes(&other.encode(), 0), Err(PeerError::WrongInfoHash));
        let mut b = PeerConn::new(PeerConfig::default(), hs(true), Some([8; 20]), false, 10, 0);
        b.on_bytes(&other.encode(), 0).unwrap();
        assert_eq!(&b.take_outgoing()[28..48], &[8; 20]);
    }

    #[test]
    fn pending_pieces_until_metadata() {
        let mut a = PeerConn::new(PeerConfig::default(), hs(true), None, true, 0, 0);
        let mut x = hs(true).encode().to_vec();
        Message::Bitfield(vec![0xc0]).encode_into(&mut x);
        Message::Have(5).encode_into(&mut x);
        let ev = a.on_bytes(&x, 0).unwrap();
        assert_eq!(ev.iter().filter(|e| **e == PeerEvent::PiecesPending).count(), 2);
        a.set_num_pieces(6).unwrap();
        assert_eq!(a.have.ones().collect::<Vec<_>>(), vec![0, 1, 5]);
        let mut b = PeerConn::new(PeerConfig::default(), hs(true), None, true, 0, 0);
        let mut y = hs(true).encode().to_vec();
        Message::Have(50).encode_into(&mut y);
        b.on_bytes(&y, 0).unwrap();
        assert_eq!(b.set_num_pieces(6), Err(PeerError::BadPiece));
    }

    #[test]
    fn timers_and_pipeline() {
        let (mut a, mut b) = pair(true);
        b.unchoke(0);
        a.on_bytes(&b.take_outgoing(), 0).unwrap();
        assert_eq!(a.request_room(0), 2, "min queue when nothing measured");
        // Measured 1 MiB/s for 3 s of queue time: 192 blocks.
        for s in 0..5 { a.down.add(1 << 20, s * 1000); }
        assert_eq!(a.request_room(4000), 192);
        a.request(BlockRef { piece: 0, begin: 0, len: 16384 }, 5000);
        assert!(a.on_tick(20_000).unwrap().is_empty());
        let t = a.on_tick(36_000).unwrap();
        assert_eq!(t.len(), 1);
        assert!(a.outstanding.is_empty());
        a.request(BlockRef { piece: 1, begin: 0, len: 16384 }, 36_000);
        a.on_tick(36_000 + 61_000).unwrap();
        assert!(a.snubbed);
        assert_eq!(a.request_room(100_000), 1, "a snubbed peer gets one request at a time");
        // Keepalive went out, then idle timeout closes.
        assert!(!a.take_outgoing().is_empty());
        assert_eq!(a.on_tick(500_000), Err(PeerError::Idle));
        let mut c = PeerConn::new(PeerConfig::default(), hs(true), None, true, 1, 0);
        assert_eq!(c.on_tick(16_000), Err(PeerError::HandshakeTimeout));
    }
}
