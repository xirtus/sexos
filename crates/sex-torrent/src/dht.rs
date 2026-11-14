// SPDX-License-Identifier: GPL-3.0-only
//! The Mainline DHT (BEP 5) over IPv4 and IPv6 (BEP 32): a distributed table, spread over
//! every BitTorrent client online, that maps an info-hash to the peers in its swarm.
//!
//! Every node has a random 160-bit id. A node keeps a routing table of other nodes, many
//! close to its own id and few far away (buckets of [`K`] = 8, the bucket holding our own
//! id splits when full). To find peers for an info-hash, a node asks the nodes it knows
//! closest to that hash for closer nodes, [`ALPHA`] = 3 at a time, until the 8 closest
//! have all answered ("iterative lookup"); those nodes return the peers that announced
//! there, and a write token with which we announce ourselves.
//!
//! What is here:
//! - KRPC messages ([`Krpc`]): ping, find_node, get_peers, announce_peer (with
//!   `implied_port`), errors, BEP 32 `nodes6` / `want`, BEP 42 `ip`, BEP 43 `ro`.
//! - The routing table ([`Table`]): good / questionable / bad nodes, replacement cache,
//!   pings for questionable nodes, refresh of buckets idle for 15 minutes.
//! - Tokens: `sha1(secret ‖ ip)`, secret rotated every 5 minutes, the previous one still
//!   accepted.
//! - A bounded peer store for announces we receive, entries expire after 30 minutes.
//! - BEP 42: node ids derived from the external IP with CRC32-C ([`bep42_node_id`],
//!   [`bep42_valid`]); enforcement is optional ([`Config::enforce_bep42`]).
//! - BEP 43 read-only mode ([`Config::read_only`]): we mark queries `ro=1`, answer no
//!   queries, and never put nodes that say `ro=1` in our table.
//!
//! Sans-IO: [`Dht::on_packet`] takes a datagram, [`Dht::on_tick`] the time (call it about
//! once a second); both return [`Out`]: datagrams to send and [`Event`]s. The IPv4 and
//! IPv6 tables are separate (each with its own node id); a datagram's family picks one.
//! Bootstrap host names (router.bittorrent.com, dht.transmissionbt.com, ...) are resolved
//! by the program, which passes the addresses to [`Dht::bootstrap`]. Randomness comes from
//! the 32-byte seed given to [`Dht::new`] (stretched with SHA-256).
//!
//! Not done: BEP 33 scrapes, BEP 44 (put/get), BEP 51 (sample_infohashes).
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use sha1::{Digest, Sha1};
use sha2::Sha256;

use crate::bencode::{self, Item, Value};
use crate::{Ip, Millis, PeerAddr};

/// A node id (also the key space of info-hashes).
pub type NodeId = [u8; 20];
/// Names a lookup started with [`Dht::get_peers`] or [`Dht::find_node`].
pub type LookupId = u32;

/// Bucket size and the number of closest nodes a lookup converges on.
pub const K: usize = 8;
/// Parallel queries per lookup and family.
pub const ALPHA: usize = 3;
/// A query unanswered after this long has failed.
pub const RPC_TIMEOUT: Millis = 5_000;
/// Nodes are good for 15 minutes after a reply; idle buckets are refreshed after that.
pub const NODE_FRESH: Millis = 15 * 60_000;
/// Token secrets rotate this often.
pub const TOKEN_ROTATE: Millis = 5 * 60_000;
/// Announced peers are kept this long.
pub const PEER_TTL: Millis = 30 * 60_000;
/// Datagrams bigger than this are dropped.
pub const MAX_PACKET: usize = 4096;
/// Peer store bounds.
pub const MAX_STORED_TORRENTS: usize = 4096;
pub const MAX_PEERS_PER_TORRENT: usize = 256;
/// Peers returned in one get_peers answer (IPv4, IPv6) so it fits one datagram.
pub const MAX_VALUES_V4: usize = 50;
pub const MAX_VALUES_V6: usize = 30;
/// Candidates kept per lookup and family, queries per lookup and family, lookups at once.
pub const MAX_CANDIDATES: usize = 64;
pub const MAX_QUERIES: usize = 128;
pub const MAX_LOOKUPS: usize = 128;
/// Outstanding queries at once (beyond this, new queries are not sent).
pub const MAX_RPCS: usize = 2048;

fn fam(a: &PeerAddr) -> usize { match a.ip { Ip::V4(_) => 0, Ip::V6(_) => 1 } }

fn xor(a: &NodeId, b: &NodeId) -> NodeId {
    let mut o = [0u8; 20];
    for i in 0..20 { o[i] = a[i] ^ b[i]; }
    o
}

/// Compares the XOR distances of `a` and `b` to `target`.
pub fn distance_cmp(target: &NodeId, a: &NodeId, b: &NodeId) -> Ordering { xor(target, a).cmp(&xor(target, b)) }

/// Number of leading bits `a` and `b` share (160 when equal).
pub fn common_prefix(a: &NodeId, b: &NodeId) -> usize {
    for i in 0..20 {
        let x = a[i] ^ b[i];
        if x != 0 { return i * 8 + x.leading_zeros() as usize; }
    }
    160
}

// ---------------------------------------------------------------------------------
// BEP 42: node ids bound to the external IP
// ---------------------------------------------------------------------------------

const CRC32C_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 { c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 }; k += 1; }
        t[i] = c;
        i += 1;
    }
    t
};

/// CRC32-C (Castagnoli), as BEP 42 uses it.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data { c = CRC32C_TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8); }
    !c
}

fn bep42_prefix(ip: &Ip, r: u8) -> u32 {
    let mut buf = [0u8; 8];
    let n = match ip {
        Ip::V4(a) => { for i in 0..4 { buf[i] = a[i] & [0x03, 0x0f, 0x3f, 0xff][i]; } 4 }
        Ip::V6(a) => { for i in 0..8 { buf[i] = a[i] & [0x01, 0x03, 0x07, 0x0f, 0x1f, 0x3f, 0x7f, 0xff][i]; } 8 }
    };
    buf[0] |= (r & 7) << 5;
    crc32c(&buf[..n])
}

/// A BEP 42 node id for our external `ip`: `r` is a random byte (kept in the last byte),
/// `rand` fills the free bits.
pub fn bep42_node_id(ip: &Ip, r: u8, rand: &[u8; 20]) -> NodeId {
    let crc = bep42_prefix(ip, r);
    let mut id = *rand;
    id[0] = (crc >> 24) as u8;
    id[1] = (crc >> 16) as u8;
    id[2] = ((crc >> 8) as u8 & 0xf8) | (rand[2] & 7);
    id[19] = r;
    id
}

/// Addresses BEP 42 exempts (private, loopback, link-local).
pub fn is_local(ip: &Ip) -> bool {
    match ip {
        Ip::V4(a) => a[0] == 10 || a[0] == 127 || (a[0] == 172 && a[1] & 0xf0 == 16) || (a[0] == 192 && a[1] == 168) || (a[0] == 169 && a[1] == 254),
        Ip::V6(a) => a[0] & 0xfe == 0xfc || (a[0] == 0xfe && a[1] & 0xc0 == 0x80) || *a == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    }
}

/// Whether `id` is a valid BEP 42 id for `ip` (always true for local addresses).
pub fn bep42_valid(id: &NodeId, ip: &Ip) -> bool {
    if is_local(ip) { return true; }
    let crc = bep42_prefix(ip, id[19]);
    id[0] == (crc >> 24) as u8 && id[1] == (crc >> 16) as u8 && id[2] & 0xf8 == (crc >> 8) as u8 & 0xf8
}

// ---------------------------------------------------------------------------------
// KRPC messages
// ---------------------------------------------------------------------------------

/// A query's method and arguments.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Query {
    Ping,
    FindNode { target: NodeId },
    GetPeers { info_hash: NodeId },
    AnnouncePeer { info_hash: NodeId, port: u16, implied_port: bool, token: Vec<u8> },
}

/// A response's fields (which ones appear depends on the query).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Reply {
    pub id: NodeId,
    pub nodes: Vec<(NodeId, PeerAddr)>,
    pub nodes6: Vec<(NodeId, PeerAddr)>,
    pub values: Vec<PeerAddr>,
    pub token: Option<Vec<u8>>,
}

/// The three KRPC message kinds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Body {
    /// `want_v4` / `want_v6`: BEP 32 `want` (both false when absent).
    Query { id: NodeId, query: Query, want_v4: bool, want_v6: bool },
    Response(Reply),
    Error { code: i64, message: String },
}

/// One KRPC message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Krpc {
    /// Transaction id (echoed in the answer).
    pub t: Vec<u8>,
    pub body: Body,
    /// BEP 43 read-only flag.
    pub ro: bool,
    /// BEP 42: the requester's address as the responder sees it.
    pub ip: Option<PeerAddr>,
    /// Client version (4 bytes by convention).
    pub v: Option<Vec<u8>>,
}

/// Error codes (BEP 5).
pub const ERR_GENERIC: i64 = 201;
pub const ERR_SERVER: i64 = 202;
pub const ERR_PROTOCOL: i64 = 203;
pub const ERR_METHOD: i64 = 204;

/// Caps on hostile fields.
const MAX_T: usize = 64;
const MAX_TOKEN: usize = 64;
const MAX_NODES: usize = 32;
const MAX_VALUES_IN: usize = 256;

fn compact_nodes(nodes: &[(NodeId, PeerAddr)]) -> Vec<u8> {
    let mut v = Vec::new();
    for (id, a) in nodes { v.extend_from_slice(id); v.extend_from_slice(&a.compact()); }
    v
}

fn parse_nodes(b: &[u8], six: bool) -> Vec<(NodeId, PeerAddr)> {
    let n = if six { 38 } else { 26 };
    b.chunks_exact(n).take(MAX_NODES).filter_map(|c| {
        let mut id = [0u8; 20];
        id.copy_from_slice(&c[..20]);
        let a = PeerAddr::parse_compact(&c[20..], six).into_iter().next()?;
        (a.port != 0).then_some((id, a))
    }).collect()
}

fn parse_addr(b: &[u8]) -> Option<PeerAddr> {
    match b.len() { 6 => PeerAddr::parse_compact(b, false).into_iter().next(), 18 => PeerAddr::parse_compact(b, true).into_iter().next(), _ => None }
}

fn id20(v: Option<&Value<'_>>) -> Option<NodeId> { v?.bytes().and_then(|b| <[u8; 20]>::try_from(b).ok()) }

impl Krpc {
    /// Encodes canonically (sorted keys), the form in BEP 5's examples.
    pub fn encode(&self) -> Vec<u8> {
        let mut top: BTreeMap<Vec<u8>, Item> = BTreeMap::new();
        top.insert(b"t".to_vec(), Item::Bytes(self.t.clone()));
        match &self.body {
            Body::Query { id, query, want_v4, want_v6 } => {
                let mut a: BTreeMap<Vec<u8>, Item> = BTreeMap::new();
                a.insert(b"id".to_vec(), Item::Bytes(id.to_vec()));
                let method: &str = match query {
                    Query::Ping => "ping",
                    Query::FindNode { target } => { a.insert(b"target".to_vec(), Item::Bytes(target.to_vec())); "find_node" }
                    Query::GetPeers { info_hash } => { a.insert(b"info_hash".to_vec(), Item::Bytes(info_hash.to_vec())); "get_peers" }
                    Query::AnnouncePeer { info_hash, port, implied_port, token } => {
                        a.insert(b"info_hash".to_vec(), Item::Bytes(info_hash.to_vec()));
                        a.insert(b"port".to_vec(), Item::Int(*port as i64));
                        if *implied_port { a.insert(b"implied_port".to_vec(), Item::Int(1)); }
                        a.insert(b"token".to_vec(), Item::Bytes(token.clone()));
                        "announce_peer"
                    }
                };
                if *want_v4 || *want_v6 {
                    let mut w = Vec::new();
                    if *want_v4 { w.push(Item::from("n4")); }
                    if *want_v6 { w.push(Item::from("n6")); }
                    a.insert(b"want".to_vec(), Item::List(w));
                }
                top.insert(b"y".to_vec(), "q".into());
                top.insert(b"q".to_vec(), method.into());
                top.insert(b"a".to_vec(), Item::Dict(a));
            }
            Body::Response(r) => {
                let mut d: BTreeMap<Vec<u8>, Item> = BTreeMap::new();
                d.insert(b"id".to_vec(), Item::Bytes(r.id.to_vec()));
                if !r.nodes.is_empty() { d.insert(b"nodes".to_vec(), Item::Bytes(compact_nodes(&r.nodes))); }
                if !r.nodes6.is_empty() { d.insert(b"nodes6".to_vec(), Item::Bytes(compact_nodes(&r.nodes6))); }
                if !r.values.is_empty() { d.insert(b"values".to_vec(), Item::List(r.values.iter().map(|p| Item::Bytes(p.compact())).collect())); }
                if let Some(t) = &r.token { d.insert(b"token".to_vec(), Item::Bytes(t.clone())); }
                top.insert(b"y".to_vec(), "r".into());
                top.insert(b"r".to_vec(), Item::Dict(d));
            }
            Body::Error { code, message } => {
                top.insert(b"y".to_vec(), "e".into());
                top.insert(b"e".to_vec(), Item::List(vec![Item::Int(*code), message.as_str().into()]));
            }
        }
        if self.ro { top.insert(b"ro".to_vec(), Item::Int(1)); }
        if let Some(ip) = &self.ip { top.insert(b"ip".to_vec(), Item::Bytes(ip.compact())); }
        if let Some(v) = &self.v { top.insert(b"v".to_vec(), Item::Bytes(v.clone())); }
        Item::Dict(top).encode()
    }

    /// Decodes strictly. On failure returns the transaction id (only for a query, which
    /// deserves an error answer) and the KRPC error code to answer with.
    pub fn decode(bytes: &[u8]) -> Result<Krpc, (Option<Vec<u8>>, i64)> {
        let bad = (None, ERR_PROTOCOL);
        if bytes.len() > MAX_PACKET { return Err(bad); }
        let v = bencode::decode(bytes).map_err(|_| bad.clone())?;
        v.dict().ok_or(bad.clone())?;
        let t = v.get(b"t").and_then(|t| t.bytes()).filter(|t| t.len() <= MAX_T).ok_or(bad.clone())?.to_vec();
        let y = v.get(b"y").and_then(|y| y.bytes()).ok_or(bad.clone())?;
        let ro = v.get(b"ro").and_then(|r| r.int()) == Some(1);
        let ip = v.get(b"ip").and_then(|i| i.bytes()).and_then(parse_addr);
        let ver = v.get(b"v").and_then(|x| x.bytes()).filter(|x| x.len() <= 16).map(|x| x.to_vec());
        let body = match y {
            b"q" => {
                let qerr = |code| (Some(t.clone()), code);
                let method = v.get(b"q").and_then(|q| q.bytes()).ok_or(qerr(ERR_PROTOCOL))?;
                let a = v.get(b"a").filter(|a| a.dict().is_some()).ok_or(qerr(ERR_PROTOCOL))?;
                let id = id20(a.get(b"id")).ok_or(qerr(ERR_PROTOCOL))?;
                let (mut want_v4, mut want_v6) = (false, false);
                for w in a.get(b"want").and_then(|w| w.list()).unwrap_or(&[]).iter().take(8) {
                    match w.bytes() { Some(b"n4") => want_v4 = true, Some(b"n6") => want_v6 = true, _ => {} }
                }
                let query = match method {
                    b"ping" => Query::Ping,
                    b"find_node" => Query::FindNode { target: id20(a.get(b"target")).ok_or(qerr(ERR_PROTOCOL))? },
                    b"get_peers" => Query::GetPeers { info_hash: id20(a.get(b"info_hash")).ok_or(qerr(ERR_PROTOCOL))? },
                    b"announce_peer" => {
                        let info_hash = id20(a.get(b"info_hash")).ok_or(qerr(ERR_PROTOCOL))?;
                        let implied_port = a.get(b"implied_port").and_then(|i| i.int()).is_some_and(|i| i != 0);
                        let port = a.get(b"port").and_then(|p| p.int()).and_then(|p| u16::try_from(p).ok());
                        let port = match (port, implied_port) { (Some(p), _) => p, (None, true) => 0, (None, false) => return Err(qerr(ERR_PROTOCOL)) };
                        let token = a.get(b"token").and_then(|t| t.bytes()).filter(|t| t.len() <= MAX_TOKEN).ok_or(qerr(ERR_PROTOCOL))?.to_vec();
                        Query::AnnouncePeer { info_hash, port, implied_port, token }
                    }
                    _ => return Err(qerr(ERR_METHOD)),
                };
                Body::Query { id, query, want_v4, want_v6 }
            }
            b"r" => {
                let r = v.get(b"r").filter(|r| r.dict().is_some()).ok_or(bad.clone())?;
                let id = id20(r.get(b"id")).ok_or(bad.clone())?;
                let nodes = r.get(b"nodes").and_then(|n| n.bytes()).map(|b| parse_nodes(b, false)).unwrap_or_default();
                let nodes6 = r.get(b"nodes6").and_then(|n| n.bytes()).map(|b| parse_nodes(b, true)).unwrap_or_default();
                let mut values = Vec::new();
                for x in r.get(b"values").and_then(|x| x.list()).unwrap_or(&[]).iter().take(MAX_VALUES_IN) {
                    if let Some(a) = x.bytes().and_then(parse_addr) { if a.port != 0 && !values.contains(&a) { values.push(a); } }
                }
                let token = r.get(b"token").and_then(|t| t.bytes()).filter(|t| t.len() <= MAX_TOKEN).map(|t| t.to_vec());
                Body::Response(Reply { id, nodes, nodes6, values, token })
            }
            b"e" => {
                let e = v.get(b"e").and_then(|e| e.list()).unwrap_or(&[]);
                let code = e.first().and_then(|c| c.int()).unwrap_or(ERR_GENERIC);
                let msg = e.get(1).and_then(|m| m.bytes()).unwrap_or(b"");
                Body::Error { code, message: String::from_utf8_lossy(&msg[..msg.len().min(256)]).into_owned() }
            }
            _ => return Err(bad),
        };
        Ok(Krpc { t, body, ro, ip, v: ver })
    }
}

// ---------------------------------------------------------------------------------
// Routing table
// ---------------------------------------------------------------------------------

/// A node in the routing table.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NodeEntry {
    pub id: NodeId,
    pub addr: PeerAddr,
    /// When it last answered one of our queries (`None`: never, it only queried us).
    pub last_reply: Option<Millis>,
    /// Consecutive unanswered queries.
    pub fails: u8,
    pinged: Option<Millis>,
}

impl NodeEntry {
    /// Answered within 15 minutes.
    pub fn good(&self, now: Millis) -> bool { self.fails == 0 && self.last_reply.is_some_and(|t| now.saturating_sub(t) < NODE_FRESH) }
    /// Failed to answer twice in a row: may be replaced.
    pub fn bad(&self) -> bool { self.fails >= 2 }
}

#[derive(Clone, Debug)]
struct Bucket { nodes: Vec<NodeEntry>, repl: Vec<NodeEntry>, changed: Millis }

/// A routing table: buckets indexed by how many leading bits a node shares with our id;
/// the last bucket holds everything closer and is the only one that splits.
#[derive(Clone, Debug)]
pub struct Table { own: NodeId, buckets: Vec<Bucket> }

impl Table {
    pub fn new(own: NodeId, now: Millis) -> Self { Table { own, buckets: vec![Bucket { nodes: Vec::new(), repl: Vec::new(), changed: now }] } }
    pub fn own_id(&self) -> NodeId { self.own }
    /// Nodes in the table (not counting replacements).
    pub fn len(&self) -> usize { self.buckets.iter().map(|b| b.nodes.len()).sum() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    pub fn bucket_count(&self) -> usize { self.buckets.len() }
    /// Every node.
    pub fn nodes(&self) -> impl Iterator<Item = &NodeEntry> { self.buckets.iter().flat_map(|b| b.nodes.iter()) }

    fn idx(&self, id: &NodeId) -> usize { common_prefix(&self.own, id).min(self.buckets.len() - 1) }

    /// Adds or refreshes a node. `verified`: it answered our query.
    pub fn insert(&mut self, id: NodeId, addr: PeerAddr, now: Millis, verified: bool) {
        if id == self.own || addr.port == 0 { return; }
        let i = self.idx(&id);
        if let Some(e) = self.buckets[i].nodes.iter_mut().find(|e| e.id == id) {
            if e.addr != addr { return; }
            if verified { e.last_reply = Some(now); e.fails = 0; e.pinged = None; self.buckets[i].changed = now; }
            return;
        }
        if self.nodes().any(|e| e.addr == addr) { return; }
        let entry = NodeEntry { id, addr, last_reply: verified.then_some(now), fails: 0, pinged: None };
        loop {
            let len = self.buckets.len();
            let i = self.idx(&id);
            let b = &mut self.buckets[i];
            if b.nodes.len() < K {
                b.repl.retain(|e| e.id != id);
                b.nodes.push(entry);
                b.changed = now;
                return;
            }
            if i == len - 1 && len < 160 { self.split(now); continue; }
            if let Some(p) = b.nodes.iter().position(|e| e.bad()) {
                b.nodes[p] = entry;
                b.changed = now;
                return;
            }
            b.repl.retain(|e| e.id != id && e.addr != addr);
            if b.repl.len() >= K { b.repl.remove(0); }
            b.repl.push(entry);
            return;
        }
    }

    fn split(&mut self, now: Millis) {
        let n = self.buckets.len();
        let own = self.own;
        let Some(last) = self.buckets.last_mut() else { return };
        let (go, stay): (Vec<_>, Vec<_>) = core::mem::take(&mut last.nodes).into_iter().partition(|e| common_prefix(&own, &e.id) >= n);
        let (rgo, rstay): (Vec<_>, Vec<_>) = core::mem::take(&mut last.repl).into_iter().partition(|e| common_prefix(&own, &e.id) >= n);
        last.nodes = stay;
        last.repl = rstay;
        self.buckets.push(Bucket { nodes: go, repl: rgo, changed: now });
    }

    /// A query to this node went unanswered.
    pub fn on_timeout(&mut self, id: &NodeId, addr: &PeerAddr) {
        let i = self.idx(id);
        let b = &mut self.buckets[i];
        let Some(p) = b.nodes.iter().position(|e| e.id == *id && e.addr == *addr) else { return };
        b.nodes[p].fails = b.nodes[p].fails.saturating_add(1);
        let f = b.nodes[p].fails;
        if f >= 3 && !b.repl.is_empty() {
            let r = b.repl.pop().unwrap_or_else(|| b.nodes[p].clone());
            b.nodes[p] = r;
        } else if f >= 5 {
            b.nodes.remove(p);
        }
    }

    /// Up to `n` non-bad nodes closest to `target`.
    pub fn closest(&self, target: &NodeId, n: usize) -> Vec<(NodeId, PeerAddr)> {
        let mut v: Vec<(NodeId, PeerAddr)> = self.nodes().filter(|e| !e.bad()).map(|e| (e.id, e.addr)).collect();
        v.sort_by(|a, b| distance_cmp(target, &a.0, &b.0));
        v.truncate(n);
        v
    }

    /// A random id falling in bucket `i` (for refreshes).
    fn random_in_bucket(&self, i: usize, rand: &[u8; 20]) -> NodeId {
        let mut id = *rand;
        for bit in 0..i.min(160) {
            let (byte, mask) = (bit / 8, 0x80u8 >> (bit % 8));
            id[byte] = (id[byte] & !mask) | (self.own[byte] & mask);
        }
        if i < 159 && i + 1 < self.buckets.len() {
            let (byte, mask) = (i / 8, 0x80u8 >> (i % 8));
            id[byte] = (id[byte] & !mask) | (!self.own[byte] & mask);
        }
        id
    }
}

// ---------------------------------------------------------------------------------
// The DHT node
// ---------------------------------------------------------------------------------

/// Settings.
#[derive(Clone, Debug)]
pub struct Config {
    pub v4: bool,
    pub v6: bool,
    /// BEP 43: never answer queries, ask others not to add us.
    pub read_only: bool,
    /// BEP 42: keep only nodes whose id matches their IP (local addresses exempt).
    pub enforce_bep42: bool,
    /// Announce with `implied_port` (the port the datagram came from; for uTP / NAT).
    pub implied_port: bool,
    /// The `v` field (e.g. `b"SX01"`); `None` in anonymous mode.
    pub version: Option<[u8; 4]>,
}

impl Default for Config {
    fn default() -> Self { Config { v4: true, v6: true, read_only: false, enforce_bep42: false, implied_port: false, version: Some(*b"SX01") } }
}

/// What the DHT tells the program.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// New peers for a get_peers lookup (each reported once per lookup).
    Peers { lookup: LookupId, peers: Vec<PeerAddr> },
    /// The lookup finished; `announced` = nodes we sent announce_peer to.
    LookupDone { lookup: LookupId, peers_found: usize, announced: usize, closest: Vec<(NodeId, PeerAddr)> },
    /// A node told us how it sees our address (BEP 42 `ip`); the program may vote on these
    /// and call [`Dht::set_external_ip`].
    ExternalAddress { reported_by: PeerAddr, addr: PeerAddr },
}

/// Output of the state machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Out {
    Send { to: PeerAddr, bytes: Vec<u8> },
    Event(Event),
}

/// A SHA-256 based generator stretched from the caller's seed.
#[derive(Clone)]
struct Drbg { key: [u8; 32], counter: u64 }

impl Drbg {
    fn fill(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(32) {
            let mut h = Sha256::new();
            h.update(self.key);
            h.update(self.counter.to_be_bytes());
            self.counter += 1;
            let block = h.finalize();
            chunk.copy_from_slice(&block[..chunk.len()]);
        }
    }
    fn bytes<const N: usize>(&mut self) -> [u8; N] { let mut b = [0u8; N]; self.fill(&mut b); b }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CState { Fresh, Wait, Done, Failed }

#[derive(Clone, Debug)]
struct Cand { id: NodeId, addr: PeerAddr, state: CState, token: Option<Vec<u8>> }

#[derive(Clone, Debug, Default)]
struct FamLookup { cands: Vec<Cand>, inflight: usize, queries: usize, done: bool }

#[derive(Clone, Debug)]
struct Lookup { target: NodeId, get_peers: bool, announce: Option<u16>, user: bool, fams: [FamLookup; 2], found: BTreeSet<PeerAddr> }

#[derive(Clone, Copy, Debug)]
enum PendKind { Lookup(LookupId, usize), Ping, Announce }

#[derive(Clone, Debug)]
struct Rpc { to: PeerAddr, id: Option<NodeId>, sent: Millis, kind: PendKind }

/// A DHT node (one per program; it serves both address families).
pub struct Dht {
    cfg: Config,
    tables: [Table; 2],
    routers: [Vec<PeerAddr>; 2],
    rpcs: BTreeMap<u16, Rpc>,
    lookups: BTreeMap<LookupId, Lookup>,
    next_lookup: LookupId,
    secrets: [[u8; 20]; 2],
    secret_at: Millis,
    store: BTreeMap<NodeId, Vec<(PeerAddr, Millis)>>,
    rng: Drbg,
    out: Vec<Out>,
    last_maint: Millis,
    last_bootstrap: Millis,
}

impl Dht {
    /// A node with random ids (call [`set_external_ip`](Self::set_external_ip) once the
    /// external address is known, for BEP 42 ids). `seed`: 32 bytes from the program's
    /// random generator.
    pub fn new(cfg: Config, seed: [u8; 32], now: Millis) -> Self {
        let mut rng = Drbg { key: seed, counter: 0 };
        let id4 = rng.bytes::<20>();
        let id6 = rng.bytes::<20>();
        let secrets = [rng.bytes::<20>(), rng.bytes::<20>()];
        Dht {
            cfg, tables: [Table::new(id4, now), Table::new(id6, now)], routers: [Vec::new(), Vec::new()],
            rpcs: BTreeMap::new(), lookups: BTreeMap::new(), next_lookup: 1, secrets, secret_at: now,
            store: BTreeMap::new(), rng, out: Vec::new(), last_maint: now, last_bootstrap: now,
        }
    }

    /// Our node id for a family.
    pub fn node_id(&self, v6: bool) -> NodeId { self.tables[v6 as usize].own }
    /// The routing table for a family.
    pub fn table(&self, v6: bool) -> &Table { &self.tables[v6 as usize] }
    pub fn config(&self) -> &Config { &self.cfg }

    /// Sets our id (e.g. restored from saved state); the table is rebuilt around it.
    pub fn set_node_id(&mut self, v6: bool, id: NodeId, now: Millis) {
        let f = v6 as usize;
        let old = core::mem::replace(&mut self.tables[f], Table::new(id, now));
        for e in old.nodes() { self.tables[f].insert(e.id, e.addr, now, e.last_reply.is_some()); }
    }

    /// Our external address is known: take a BEP 42 id for it (unless ours already is).
    pub fn set_external_ip(&mut self, ip: Ip, now: Millis) {
        let v6 = matches!(ip, Ip::V6(_));
        if bep42_valid(&self.node_id(v6), &ip) && !is_local(&ip) { return; }
        let r = self.rng.bytes::<1>()[0];
        let rand = self.rng.bytes::<20>();
        self.set_node_id(v6, bep42_node_id(&ip, r, &rand), now);
    }

    /// Drains pending output (after [`get_peers`](Self::get_peers) and friends).
    pub fn poll(&mut self) -> Vec<Out> { core::mem::take(&mut self.out) }

    fn enabled(&self, f: usize) -> bool { if f == 0 { self.cfg.v4 } else { self.cfg.v6 } }

    /// Joins the network through these nodes (resolved router names, saved nodes): each
    /// is asked for nodes near our id. They are remembered for re-bootstrapping while the
    /// table is small, and never put in the table themselves.
    pub fn bootstrap(&mut self, nodes: &[PeerAddr], now: Millis) {
        for n in nodes.iter().take(32) {
            let f = fam(n);
            if !self.routers[f].contains(n) && self.routers[f].len() < 32 { self.routers[f].push(*n); }
        }
        self.last_bootstrap = now;
        for f in 0..2 {
            if self.enabled(f) && !self.routers[f].is_empty() {
                let own = self.tables[f].own;
                self.start_lookup(own, false, None, false, now);
            }
        }
    }

    /// Pings a node we heard of (e.g. a peer's `port` message, saved state); it joins the
    /// table if it answers.
    pub fn add_node(&mut self, addr: PeerAddr, now: Millis) {
        if self.enabled(fam(&addr)) { self.send_query(addr, None, Query::Ping, PendKind::Ping, now); }
    }

    /// Looks up peers for `info_hash` (both families); with `announce`, announces us on
    /// that port to the closest nodes when done. Output comes from [`poll`](Self::poll)
    /// and later calls.
    pub fn get_peers(&mut self, info_hash: [u8; 20], announce: Option<u16>, now: Millis) -> LookupId {
        self.start_lookup(info_hash, true, announce, true, now)
    }

    /// Looks up the nodes closest to `target`.
    pub fn find_node(&mut self, target: NodeId, now: Millis) -> LookupId { self.start_lookup(target, false, None, true, now) }

    /// Stops a lookup without a [`Event::LookupDone`].
    pub fn cancel(&mut self, lookup: LookupId) { self.lookups.remove(&lookup); }

    /// Number of running lookups.
    pub fn lookups_running(&self) -> usize { self.lookups.len() }

    fn start_lookup(&mut self, target: NodeId, get_peers: bool, announce: Option<u16>, user: bool, now: Millis) -> LookupId {
        let lid = self.next_lookup;
        self.next_lookup = self.next_lookup.wrapping_add(1).max(1);
        if self.lookups.len() >= MAX_LOOKUPS {
            if user { self.out.push(Out::Event(Event::LookupDone { lookup: lid, peers_found: 0, announced: 0, closest: Vec::new() })); }
            return lid;
        }
        let mut lk = Lookup { target, get_peers, announce, user, fams: [FamLookup::default(), FamLookup::default()], found: BTreeSet::new() };
        let mut router_sends = Vec::new();
        for f in 0..2 {
            if !self.enabled(f) { lk.fams[f].done = true; continue; }
            let own = self.tables[f].own;
            for (id, addr) in self.tables[f].closest(&target, K * 2) { add_cand(&mut lk.fams[f], &target, &own, id, addr); }
            if self.tables[f].len() < K {
                for r in &self.routers[f] { router_sends.push((f, *r)); }
            }
        }
        self.lookups.insert(lid, lk);
        for (f, r) in router_sends {
            if self.send_query(r, None, lookup_query(get_peers, &target), PendKind::Lookup(lid, f), now) {
                if let Some(l) = self.lookups.get_mut(&lid) { l.fams[f].inflight += 1; }
            }
        }
        self.step(lid, now);
        lid
    }

    fn new_tid(&mut self) -> Option<u16> {
        if self.rpcs.len() >= MAX_RPCS { return None; }
        loop {
            let t = u16::from_be_bytes(self.rng.bytes::<2>());
            if !self.rpcs.contains_key(&t) { return Some(t); }
        }
    }

    fn send_query(&mut self, to: PeerAddr, id: Option<NodeId>, query: Query, kind: PendKind, now: Millis) -> bool {
        let Some(t) = self.new_tid() else { return false };
        let f = fam(&to);
        let m = Krpc { t: t.to_be_bytes().to_vec(), body: Body::Query { id: self.tables[f].own, query, want_v4: false, want_v6: false }, ro: self.cfg.read_only, ip: None, v: self.cfg.version.map(|v| v.to_vec()) };
        self.rpcs.insert(t, Rpc { to, id, sent: now, kind });
        self.out.push(Out::Send { to, bytes: m.encode() });
        true
    }

    fn reply(&mut self, to: PeerAddr, t: Vec<u8>, body: Body) {
        let m = Krpc { t, body, ro: false, ip: Some(to), v: self.cfg.version.map(|v| v.to_vec()) };
        self.out.push(Out::Send { to, bytes: m.encode() });
    }

    fn step(&mut self, lid: LookupId, now: Millis) {
        let Some(lk) = self.lookups.get_mut(&lid) else { return };
        let mut sends = Vec::new();
        for f in 0..2 {
            let fl = &mut lk.fams[f];
            if fl.done { continue; }
            let (mut considered, mut all_done) = (0, true);
            for c in fl.cands.iter_mut() {
                if c.state == CState::Failed { continue; }
                if considered == K { break; }
                considered += 1;
                match c.state {
                    CState::Done | CState::Failed => {}
                    CState::Wait => all_done = false,
                    CState::Fresh => {
                        all_done = false;
                        if fl.inflight < ALPHA && fl.queries < MAX_QUERIES {
                            c.state = CState::Wait;
                            fl.inflight += 1;
                            fl.queries += 1;
                            sends.push((f, c.id, c.addr));
                        }
                    }
                }
            }
            if (considered > 0 && all_done) || fl.inflight == 0 { fl.done = true; }
        }
        let (target, gp) = (lk.target, lk.get_peers);
        let finished = lk.fams.iter().all(|f| f.done);
        for (f, id, addr) in sends {
            if !self.send_query(addr, Some(id), lookup_query(gp, &target), PendKind::Lookup(lid, f), now) {
                if let Some(l) = self.lookups.get_mut(&lid) {
                    l.fams[f].inflight -= 1;
                    if let Some(c) = l.fams[f].cands.iter_mut().find(|c| c.addr == addr) { c.state = CState::Failed; }
                }
            }
        }
        if finished { self.finish(lid, now); }
    }

    fn finish(&mut self, lid: LookupId, now: Millis) {
        let Some(lk) = self.lookups.remove(&lid) else { return };
        let mut announced = 0;
        let mut closest = Vec::new();
        for f in 0..2 {
            let best: Vec<&Cand> = lk.fams[f].cands.iter().filter(|c| c.state == CState::Done).take(K).collect();
            closest.extend(best.iter().map(|c| (c.id, c.addr)));
            if let (Some(port), true) = (lk.announce, lk.get_peers) {
                let jobs: Vec<(NodeId, PeerAddr, Vec<u8>)> = best.iter().filter_map(|c| Some((c.id, c.addr, c.token.clone()?))).collect();
                for (id, addr, token) in jobs {
                    let q = Query::AnnouncePeer { info_hash: lk.target, port, implied_port: self.cfg.implied_port, token };
                    if self.send_query(addr, Some(id), q, PendKind::Announce, now) { announced += 1; }
                }
            }
        }
        if lk.user { self.out.push(Out::Event(Event::LookupDone { lookup: lid, peers_found: lk.found.len(), announced, closest })); }
    }

    fn lookup_reply(&mut self, lid: LookupId, f: usize, from: PeerAddr, reply: Option<&Reply>, now: Millis) {
        let own = self.tables[f].own;
        let Some(lk) = self.lookups.get_mut(&lid) else { return };
        let target = lk.target;
        let fl = &mut lk.fams[f];
        fl.inflight = fl.inflight.saturating_sub(1);
        if let Some(c) = fl.cands.iter_mut().find(|c| c.addr == from) {
            match reply {
                Some(r) => { c.state = CState::Done; c.token = r.token.clone(); }
                None => c.state = CState::Failed,
            }
        }
        if let Some(r) = reply {
            if !fl.done {
                let nodes = if f == 0 { &r.nodes } else { &r.nodes6 };
                for (id, addr) in nodes { if fam(addr) == f { add_cand(fl, &target, &own, *id, *addr); } }
            }
            if lk.get_peers && lk.user && !r.values.is_empty() {
                let fresh: Vec<PeerAddr> = r.values.iter().filter(|p| lk.found.len() < 4096 && lk.found.insert(**p)).copied().collect();
                if !fresh.is_empty() { self.out.push(Out::Event(Event::Peers { lookup: lid, peers: fresh })); }
            }
        }
        self.step(lid, now);
    }

    /// A datagram arrived from `from`.
    pub fn on_packet(&mut self, from: PeerAddr, bytes: &[u8], now: Millis) -> Vec<Out> {
        let f = fam(&from);
        if !self.enabled(f) || bytes.len() > MAX_PACKET || from.port == 0 { return self.poll(); }
        match Krpc::decode(bytes) {
            Err((Some(t), code)) => {
                if !self.cfg.read_only { self.reply(from, t, Body::Error { code, message: String::from(if code == ERR_METHOD { "Method Unknown" } else { "Protocol Error" }) }); }
            }
            Err((None, _)) => {}
            Ok(m) => match m.body {
                Body::Query { id, query, want_v4, want_v6 } => self.on_query(from, m.t, m.ro, id, query, want_v4, want_v6, now),
                Body::Response(r) => self.on_response(from, &m.t, m.ro, m.ip, Some(r), now),
                Body::Error { .. } => self.on_response(from, &m.t, m.ro, None, None, now),
            },
        }
        self.poll()
    }

    fn token(&self, ip: &Ip, which: usize) -> [u8; 8] {
        let mut h = Sha1::new();
        h.update(self.secrets[which]);
        match ip { Ip::V4(a) => h.update(a), Ip::V6(a) => h.update(a) }
        let d = h.finalize();
        let mut t = [0u8; 8];
        t.copy_from_slice(&d[..8]);
        t
    }

    #[allow(clippy::too_many_arguments)]
    fn on_query(&mut self, from: PeerAddr, t: Vec<u8>, ro: bool, id: NodeId, query: Query, want_v4: bool, want_v6: bool, now: Millis) {
        if self.cfg.read_only { return; }
        let f = fam(&from);
        if !ro && (!self.cfg.enforce_bep42 || bep42_valid(&id, &from.ip)) { self.tables[f].insert(id, from, now, false); }
        let own = self.tables[f].own;
        let (w4, w6) = if want_v4 || want_v6 { (want_v4 && self.cfg.v4, want_v6 && self.cfg.v6) } else { (f == 0, f == 1) };
        let nodes_for = |dht: &Dht, target: &NodeId| -> (Vec<(NodeId, PeerAddr)>, Vec<(NodeId, PeerAddr)>) {
            (if w4 { dht.tables[0].closest(target, K) } else { Vec::new() }, if w6 { dht.tables[1].closest(target, K) } else { Vec::new() })
        };
        let body = match query {
            Query::Ping => Body::Response(Reply { id: own, ..Default::default() }),
            Query::FindNode { target } => {
                let (nodes, nodes6) = nodes_for(self, &target);
                Body::Response(Reply { id: own, nodes, nodes6, ..Default::default() })
            }
            Query::GetPeers { info_hash } => {
                let (nodes, nodes6) = nodes_for(self, &info_hash);
                let max = if f == 0 { MAX_VALUES_V4 } else { MAX_VALUES_V6 };
                let values = self.store.get(&info_hash).map(|v| v.iter().rev().filter(|(p, exp)| fam(p) == f && *exp > now).map(|(p, _)| *p).take(max).collect()).unwrap_or_default();
                Body::Response(Reply { id: own, nodes, nodes6, values, token: Some(self.token(&from.ip, 0).to_vec()) })
            }
            Query::AnnouncePeer { info_hash, port, implied_port, token } => {
                let ok = token == self.token(&from.ip, 0) || token == self.token(&from.ip, 1);
                let port = if implied_port { from.port } else { port };
                if !ok || port == 0 {
                    Body::Error { code: ERR_PROTOCOL, message: String::from(if ok { "Bad Port" } else { "Bad Token" }) }
                } else {
                    self.store_peer(info_hash, PeerAddr { ip: from.ip, port }, now);
                    Body::Response(Reply { id: own, ..Default::default() })
                }
            }
        };
        self.reply(from, t, body);
    }

    fn store_peer(&mut self, info_hash: NodeId, peer: PeerAddr, now: Millis) {
        if !self.store.contains_key(&info_hash) && self.store.len() >= MAX_STORED_TORRENTS {
            self.expire_store(now);
            if self.store.len() >= MAX_STORED_TORRENTS { return; }
        }
        let v = self.store.entry(info_hash).or_default();
        v.retain(|(p, _)| *p != peer);
        if v.len() >= MAX_PEERS_PER_TORRENT { v.remove(0); }
        v.push((peer, now + PEER_TTL));
    }

    fn expire_store(&mut self, now: Millis) {
        self.store.retain(|_, v| { v.retain(|(_, exp)| *exp > now); !v.is_empty() });
    }

    /// Peers stored for an info-hash (announced to us).
    pub fn stored_peers(&self, info_hash: &NodeId) -> Vec<PeerAddr> { self.store.get(info_hash).map(|v| v.iter().map(|p| p.0).collect()).unwrap_or_default() }

    fn on_response(&mut self, from: PeerAddr, t: &[u8], ro: bool, ip: Option<PeerAddr>, reply: Option<Reply>, now: Millis) {
        let Ok(tb) = <[u8; 2]>::try_from(t) else { return };
        let key = u16::from_be_bytes(tb);
        match self.rpcs.get(&key) { Some(r) if r.to == from => {} _ => return }
        let Some(rpc) = self.rpcs.remove(&key) else { return };
        let f = fam(&from);
        let mut reply = reply;
        if let Some(r) = &reply {
            if rpc.id.is_some_and(|id| id != r.id) {
                // A different node at that address now: not the one we asked.
                reply = None;
            } else {
                let router = self.routers[f].contains(&from);
                if !ro && !router && (!self.cfg.enforce_bep42 || bep42_valid(&r.id, &from.ip)) { self.tables[f].insert(r.id, from, now, true); }
                if let Some(addr) = ip { if fam(&addr) == f { self.out.push(Out::Event(Event::ExternalAddress { reported_by: from, addr })); } }
            }
        }
        if let PendKind::Lookup(lid, lf) = rpc.kind { self.lookup_reply(lid, lf, from, reply.as_ref(), now); }
    }

    /// Timers: query timeouts, token rotation, peer expiry, pings, bucket refresh,
    /// re-bootstrap. Call about once a second.
    pub fn on_tick(&mut self, now: Millis) -> Vec<Out> {
        let expired: Vec<u16> = self.rpcs.iter().filter(|(_, r)| now >= r.sent + RPC_TIMEOUT).map(|(k, _)| *k).collect();
        for k in expired {
            let Some(rpc) = self.rpcs.remove(&k) else { continue };
            let f = fam(&rpc.to);
            if let Some(id) = rpc.id { self.tables[f].on_timeout(&id, &rpc.to); }
            if let PendKind::Lookup(lid, lf) = rpc.kind { self.lookup_reply(lid, lf, rpc.to, None, now); }
        }
        if now >= self.secret_at + TOKEN_ROTATE {
            self.secrets[1] = self.secrets[0];
            self.secrets[0] = self.rng.bytes::<20>();
            self.secret_at = now;
        }
        // Ping questionable nodes, a few per tick.
        for f in 0..2 {
            if !self.enabled(f) { continue; }
            let mut pings = Vec::new();
            for b in self.tables[f].buckets.iter_mut() {
                for e in b.nodes.iter_mut() {
                    if pings.len() >= 4 { break; }
                    let stale = e.last_reply.map_or(true, |t| now.saturating_sub(t) >= NODE_FRESH);
                    if stale && e.pinged.map_or(true, |p| now.saturating_sub(p) >= 60_000) {
                        e.pinged = Some(now);
                        pings.push((e.id, e.addr));
                    }
                }
            }
            for (id, addr) in pings { self.send_query(addr, Some(id), Query::Ping, PendKind::Ping, now); }
        }
        if now >= self.last_maint + 60_000 {
            self.last_maint = now;
            self.expire_store(now);
            for f in 0..2 {
                if !self.enabled(f) { continue; }
                // Refresh the first bucket idle for 15 minutes (one per family per minute).
                let idle = self.tables[f].buckets.iter().position(|b| now.saturating_sub(b.changed) >= NODE_FRESH);
                if let Some(i) = idle {
                    let rand = self.rng.bytes::<20>();
                    let target = self.tables[f].random_in_bucket(i, &rand);
                    self.tables[f].buckets[i].changed = now;
                    if !self.tables[f].is_empty() { self.start_lookup(target, false, None, false, now); }
                }
                if self.tables[f].len() < K && !self.routers[f].is_empty() && now >= self.last_bootstrap + 60_000 {
                    self.last_bootstrap = now;
                    let own = self.tables[f].own;
                    self.start_lookup(own, false, None, false, now);
                }
            }
        }
        self.poll()
    }
}

fn lookup_query(get_peers: bool, target: &NodeId) -> Query {
    if get_peers { Query::GetPeers { info_hash: *target } } else { Query::FindNode { target: *target } }
}

fn add_cand(fl: &mut FamLookup, target: &NodeId, own: &NodeId, id: NodeId, addr: PeerAddr) {
    if id == *own || addr.port == 0 || fl.cands.iter().any(|c| c.id == id || c.addr == addr) { return; }
    let pos = fl.cands.partition_point(|c| distance_cmp(target, &c.id, &id) == Ordering::Less);
    if pos >= MAX_CANDIDATES { return; }
    fl.cands.insert(pos, Cand { id, addr, state: CState::Fresh, token: None });
    fl.cands.truncate(MAX_CANDIDATES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::VecDeque;
    use crate::from_hex;

    #[test]
    fn crc32c_check_value() { assert_eq!(crc32c(b"123456789"), 0xE306_9283); }

    #[test]
    fn bep42_vectors() {
        let v: [([u8; 4], u8, &str); 5] = [
            ([124, 31, 75, 21], 1, "5fbfbff10c5d6a4ec8a88e4c6ab4c28b95eee401"),
            ([21, 75, 31, 124], 86, "5a3ce9c14e7a08645677bbd1cfe7d8f956d53256"),
            ([65, 23, 51, 170], 22, "a5d43220bc8f112a3d426c84764f8c2a1150e616"),
            ([84, 124, 73, 14], 65, "1b0321dd1bb1fe518101ceef99462b947a01ff41"),
            ([43, 213, 53, 83], 90, "e56f6cbf5b7c4be0237986d5243b87aa6d51305a"),
        ];
        for (ip, r, hex) in v {
            let expect = from_hex::<20>(hex).unwrap();
            let ip = Ip::V4(ip);
            assert!(bep42_valid(&expect, &ip), "{:?}", ip);
            let id = bep42_node_id(&ip, r, &expect);
            assert_eq!(id, expect);
            let mut other = expect;
            other[0] ^= 0x80;
            assert!(!bep42_valid(&other, &ip));
        }
        assert!(bep42_valid(&[0; 20], &Ip::V4([192, 168, 0, 1])));
        let ip6 = Ip::V6([0x20, 1, 0xd, 0xb8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert!(bep42_valid(&bep42_node_id(&ip6, 200, &[7; 20]), &ip6));
    }

    fn rt(bytes: &[u8]) -> Krpc {
        let m = Krpc::decode(bytes).unwrap();
        assert_eq!(m.encode(), bytes, "{:?}", m);
        m
    }

    #[test]
    fn bep5_golden_messages() {
        let m = rt(b"d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe");
        assert_eq!(m.body, Body::Query { id: *b"abcdefghij0123456789", query: Query::Ping, want_v4: false, want_v6: false });
        let m = rt(b"d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re");
        assert_eq!(m.body, Body::Response(Reply { id: *b"mnopqrstuvwxyz123456", ..Default::default() }));
        let m = rt(b"d1:eli201e23:A Generic Error Ocurrede1:t2:aa1:y1:ee");
        assert_eq!(m.body, Body::Error { code: 201, message: "A Generic Error Ocurred".into() });
        let m = rt(b"d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t2:aa1:y1:qe");
        assert!(matches!(m.body, Body::Query { query: Query::FindNode { .. }, .. }));
        rt(b"d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t2:aa1:y1:qe");
        let m = rt(b"d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re");
        let Body::Response(r) = m.body else { panic!() };
        assert_eq!(r.values, vec![PeerAddr { ip: Ip::V4(*b"axje"), port: u16::from_be_bytes(*b".u") }, PeerAddr { ip: Ip::V4(*b"idht"), port: u16::from_be_bytes(*b"nm") }]);
        assert_eq!(r.token.as_deref(), Some(&b"aoeusnth"[..]));
        let m = rt(b"d1:ad2:id20:abcdefghij012345678912:implied_porti1e9:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoeusnthe1:q13:announce_peer1:t2:aa1:y1:qe");
        assert_eq!(m.body, Body::Query { id: *b"abcdefghij0123456789", query: Query::AnnouncePeer { info_hash: *b"mnopqrstuvwxyz123456", port: 6881, implied_port: true, token: b"aoeusnth".to_vec() }, want_v4: false, want_v6: false });
        let m = rt(b"d2:ip6:\x01\x02\x03\x04\x00\x051:rd2:id20:0123456789abcdefghij5:nodes26:mnopqrstuvwxyz123456\x0a\x00\x00\x01\x1a\xe16:nodes638:mnopqrstuvwxyz123456\xfe\x80\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x01e2:roi1e1:t2:aa1:v4:SX011:y1:re");
        assert!(m.ro);
        assert_eq!(m.ip, Some(PeerAddr { ip: Ip::V4([1, 2, 3, 4]), port: 5 }));
        let Body::Response(r) = m.body else { panic!() };
        assert_eq!(r.nodes[0].1, PeerAddr { ip: Ip::V4([10, 0, 0, 1]), port: 6881 });
        assert_eq!(r.nodes6.len(), 1);
        let m = rt(b"d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz1234564:wantl2:n42:n6ee1:q9:find_node1:t2:aa1:y1:qe");
        assert!(matches!(m.body, Body::Query { want_v4: true, want_v6: true, .. }));
    }

    #[test]
    fn hostile_krpc() {
        assert_eq!(Krpc::decode(b"").unwrap_err(), (None, ERR_PROTOCOL));
        assert_eq!(Krpc::decode(b"le").unwrap_err(), (None, ERR_PROTOCOL));
        assert_eq!(Krpc::decode(b"d1:y1:qe").unwrap_err(), (None, ERR_PROTOCOL));
        assert_eq!(Krpc::decode(b"d1:ad2:id20:abcdefghij0123456789e1:q4:pong1:t2:aa1:y1:qe").unwrap_err(), (Some(b"aa".to_vec()), ERR_METHOD));
        assert_eq!(Krpc::decode(b"d1:ad2:id3:abce1:q4:ping1:t2:aa1:y1:qe").unwrap_err(), (Some(b"aa".to_vec()), ERR_PROTOCOL));
        assert_eq!(Krpc::decode(b"d1:ad2:id20:abcdefghij01234567894:porti99999e9:info_hash20:mnopqrstuvwxyz1234565:token1:xe1:q13:announce_peer1:t2:aa1:y1:qe").unwrap_err().1, ERR_PROTOCOL);
        assert!(Krpc::decode(b"d1:rd2:id20:abcdefghij01234567896:valuesl3:abci5ee5:nodes27:mnopqrstuvwxyz123456\x0a\x00\x00\x01\x1a\xe1\x07e1:t2:aa1:y1:re").is_ok());
        let deep: Vec<u8> = core::iter::repeat(b'l').take(200).chain(core::iter::repeat(b'e').take(200)).collect();
        assert!(Krpc::decode(&deep).is_err());
        assert!(Krpc::decode(&vec![b'0'; MAX_PACKET + 1]).is_err());
        // A pseudo-random fuzz: never panics.
        let mut s = 0x1234_5678u32;
        let base = b"d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re".to_vec();
        for _ in 0..3000 {
            let mut m = base.clone();
            for _ in 0..3 { s ^= s << 13; s ^= s >> 17; s ^= s << 5; let i = s as usize % m.len(); m[i] = (s >> 8) as u8; }
            let _ = Krpc::decode(&m);
        }
    }

    fn addr(i: usize) -> PeerAddr { PeerAddr { ip: Ip::V4([10, 0, (i >> 8) as u8, i as u8 + 1]), port: 6881 } }

    #[test]
    fn table_splits_and_replaces() {
        let own = [0u8; 20];
        let mut t = Table::new(own, 0);
        // Far nodes (first bit 1): only K fit.
        for i in 0..20u8 { let mut id = [0xff; 20]; id[19] = i; t.insert(id, addr(i as usize), 0, true); }
        assert_eq!(t.len(), K);
        // Near nodes split the own bucket repeatedly.
        for i in 0..40u8 { let mut id = [0u8; 20]; id[1] = 0x80 >> (i % 8); id[19] = i + 1; t.insert(id, addr(100 + i as usize), 0, true); }
        assert!(t.bucket_count() > 2);
        assert!(t.len() > 2 * K);
        // Same address with another id is refused; same id refreshed.
        let n = t.len();
        let mut id = [0xff; 20]; id[19] = 99; t.insert(id, addr(0), 1, true);
        assert_eq!(t.len(), n);
        // Bad nodes are replaced from the cache.
        let first = t.nodes().find(|e| e.id[0] == 0xff).unwrap().clone();
        for _ in 0..3 { t.on_timeout(&first.id, &first.addr); }
        assert!(!t.nodes().any(|e| e.id == first.id));
        assert_eq!(t.nodes().filter(|e| e.id[0] == 0xff).count(), K);
        let c = t.closest(&[0xff; 20], 3);
        assert_eq!(c.len(), 3);
        assert!(c.windows(2).all(|w| distance_cmp(&[0xff; 20], &w[0].0, &w[1].0) != Ordering::Greater));
        for i in 0..6 { let r = t.random_in_bucket(i, &[0x5a; 20]); assert_eq!(common_prefix(&own, &r), i); }
    }

    #[test]
    fn tokens_rotate() {
        let mut d = Dht::new(Config::default(), [1; 32], 0);
        let peer = PeerAddr { ip: Ip::V4([1, 2, 3, 4]), port: 7000 };
        let gp = Krpc { t: b"t1".to_vec(), body: Body::Query { id: [9; 20], query: Query::GetPeers { info_hash: [5; 20] }, want_v4: false, want_v6: false }, ro: false, ip: None, v: None };
        let out = d.on_packet(peer, &gp.encode(), 0);
        let Out::Send { bytes, .. } = &out[0] else { panic!() };
        let Body::Response(r) = Krpc::decode(bytes).unwrap().body else { panic!() };
        let token = r.token.unwrap();
        let ann = |tok: Vec<u8>, implied: bool| Krpc { t: b"t2".to_vec(), body: Body::Query { id: [9; 20], query: Query::AnnouncePeer { info_hash: [5; 20], port: 1234, implied_port: implied, token: tok }, want_v4: false, want_v6: false }, ro: false, ip: None, v: None }.encode();
        // Still valid after one rotation, not after two; wrong IP is refused.
        d.on_tick(TOKEN_ROTATE);
        let out = d.on_packet(peer, &ann(token.clone(), false), TOKEN_ROTATE);
        let Out::Send { bytes, .. } = &out[0] else { panic!() };
        assert!(matches!(Krpc::decode(bytes).unwrap().body, Body::Response(_)));
        assert_eq!(d.stored_peers(&[5; 20]), vec![PeerAddr { ip: Ip::V4([1, 2, 3, 4]), port: 1234 }]);
        let other = PeerAddr { ip: Ip::V4([1, 2, 3, 5]), port: 7000 };
        let Out::Send { bytes, .. } = &d.on_packet(other, &ann(token.clone(), false), TOKEN_ROTATE)[0] else { panic!() };
        assert!(matches!(Krpc::decode(bytes).unwrap().body, Body::Error { code: ERR_PROTOCOL, .. }));
        d.on_tick(2 * TOKEN_ROTATE);
        let Out::Send { bytes, .. } = &d.on_packet(peer, &ann(token, true), 2 * TOKEN_ROTATE)[0] else { panic!() };
        assert!(matches!(Krpc::decode(bytes).unwrap().body, Body::Error { .. }));
        // Peers expire.
        d.on_tick(TOKEN_ROTATE + PEER_TTL + 60_000);
        assert!(d.stored_peers(&[5; 20]).is_empty());
    }

    #[test]
    fn read_only_and_errors() {
        let mut d = Dht::new(Config { read_only: true, ..Config::default() }, [2; 32], 0);
        let ping = Krpc { t: b"x".to_vec(), body: Body::Query { id: [9; 20], query: Query::Ping, want_v4: false, want_v6: false }, ro: false, ip: None, v: None };
        assert!(d.on_packet(addr(1), &ping.encode(), 0).is_empty());
        d.add_node(addr(1), 0);
        let Out::Send { bytes, .. } = &d.poll()[0] else { panic!() };
        assert!(Krpc::decode(bytes).unwrap().ro);
        let mut d = Dht::new(Config::default(), [3; 32], 0);
        let out = d.on_packet(addr(1), b"d1:ad2:id20:abcdefghij0123456789e1:q3:foo1:t2:zz1:y1:qe", 0);
        let Out::Send { bytes, .. } = &out[0] else { panic!() };
        assert!(matches!(Krpc::decode(bytes).unwrap().body, Body::Error { code: ERR_METHOD, .. }));
        // A node marked ro is answered but not added.
        let mut ro_ping = ping.clone();
        ro_ping.ro = true;
        assert_eq!(d.on_packet(addr(2), &ro_ping.encode(), 0).len(), 1);
        assert!(d.table(false).is_empty());
        d.on_packet(addr(3), &ping.encode(), 0);
        assert_eq!(d.table(false).len(), 1);
        // Unsolicited responses are ignored.
        assert!(d.on_packet(addr(4), b"d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re", 0).is_empty());
    }

    /// An in-memory network of DHT nodes.
    struct Net { nodes: Vec<Dht>, addrs: Vec<PeerAddr>, queue: VecDeque<(PeerAddr, PeerAddr, Vec<u8>)>, events: Vec<Vec<Event>>, now: Millis, delivered: usize }

    impl Net {
        fn take(&mut self, i: usize, out: Vec<Out>) {
            for o in out {
                match o {
                    Out::Send { to, bytes } => self.queue.push_back((self.addrs[i], to, bytes)),
                    Out::Event(e) => self.events[i].push(e),
                }
            }
        }
        fn run(&mut self, ms: Millis) {
            let end = self.now + ms;
            while self.now < end {
                while let Some((from, to, bytes)) = self.queue.pop_front() {
                    self.delivered += 1;
                    if let Some(j) = self.addrs.iter().position(|a| *a == to) {
                        let out = self.nodes[j].on_packet(from, &bytes, self.now);
                        self.take(j, out);
                    }
                }
                self.now += 1000;
                for j in 0..self.nodes.len() { let out = self.nodes[j].on_tick(self.now); self.take(j, out); }
            }
        }
    }

    #[test]
    fn fifty_node_network_finds_announced_peer() {
        let n = 50;
        let mut net = Net { nodes: Vec::new(), addrs: Vec::new(), queue: VecDeque::new(), events: Vec::new(), now: 0, delivered: 0 };
        for i in 0..n {
            let mut seed = [0u8; 32];
            seed[0] = i as u8;
            seed[1] = 0x42;
            net.nodes.push(Dht::new(Config { v6: false, ..Config::default() }, seed, 0));
            net.addrs.push(addr(i));
            net.events.push(Vec::new());
        }
        let (announcer, searcher) = (n - 2, n - 1);
        // Everyone but the last two joins through node 0, one after another.
        for i in 1..announcer {
            net.nodes[i].bootstrap(&[net.addrs[0]], net.now);
            let out = net.nodes[i].poll();
            net.take(i, out);
            net.run(2000);
        }
        net.run(30_000);
        assert!(net.nodes[0].table(false).len() >= K);
        // The announcer joins through node 5 and announces.
        let ih = [0x77u8; 20];
        net.nodes[announcer].bootstrap(&[net.addrs[5]], net.now);
        let out = net.nodes[announcer].poll();
        net.take(announcer, out);
        net.run(10_000);
        let lid = net.nodes[announcer].get_peers(ih, Some(7777), net.now);
        let out = net.nodes[announcer].poll();
        net.take(announcer, out);
        net.run(20_000);
        let done = net.events[announcer].iter().find_map(|e| match e { Event::LookupDone { lookup, announced, closest, .. } if *lookup == lid => Some((*announced, closest.len())), _ => None }).expect("announce lookup finished");
        assert!(done.0 >= K / 2, "announced to {} nodes", done.0);
        assert_eq!(done.1, K);
        let holders = (0..n).filter(|&i| !net.nodes[i].stored_peers(&ih).is_empty()).count();
        assert!(holders >= K / 2, "{} nodes store the peer", holders);
        // The searcher knows only node 7 and finds the announcer through the lookup.
        net.nodes[searcher].bootstrap(&[net.addrs[7]], net.now);
        let out = net.nodes[searcher].poll();
        net.take(searcher, out);
        net.run(10_000);
        let lid = net.nodes[searcher].get_peers(ih, None, net.now);
        let out = net.nodes[searcher].poll();
        net.take(searcher, out);
        net.run(20_000);
        let want = PeerAddr { ip: net.addrs[announcer].ip, port: 7777 };
        let found: Vec<PeerAddr> = net.events[searcher].iter().filter_map(|e| match e { Event::Peers { lookup, peers } if *lookup == lid => Some(peers.clone()), _ => None }).flatten().collect();
        assert!(found.contains(&want), "found {:?}", found);
        assert!(net.events[searcher].iter().any(|e| matches!(e, Event::LookupDone { lookup, peers_found, .. } if *lookup == lid && *peers_found >= 1)));
        // The network keeps running without a storm: refreshes and pings for an hour.
        let before = net.delivered;
        net.run(3_600_000);
        let per_node_per_min = (net.delivered - before) as f64 / n as f64 / 60.0;
        assert!(per_node_per_min < 50.0, "{} packets per node per minute", per_node_per_min);
        assert!(net.nodes.iter().all(|d| d.lookups_running() <= 4));
    }

    #[test]
    fn lookup_on_empty_table_finishes() {
        let mut d = Dht::new(Config::default(), [4; 32], 0);
        let lid = d.get_peers([1; 20], Some(1), 0);
        assert_eq!(d.poll(), vec![Out::Event(Event::LookupDone { lookup: lid, peers_found: 0, announced: 0, closest: Vec::new() })]);
        // With an unreachable bootstrap node it times out and finishes.
        d.bootstrap(&[addr(9)], 0);
        assert_eq!(d.poll().len(), 1);
        let lid = d.get_peers([1; 20], None, 0);
        assert_eq!(d.poll().len(), 1, "query to the router");
        let out = d.on_tick(RPC_TIMEOUT);
        assert!(out.iter().any(|o| matches!(o, Out::Event(Event::LookupDone { lookup, .. }) if *lookup == lid)));
    }

    #[test]
    fn external_ip_sets_bep42_id() {
        let mut d = Dht::new(Config::default(), [5; 32], 0);
        let ip = Ip::V4([124, 31, 75, 21]);
        d.set_external_ip(ip, 0);
        assert!(bep42_valid(&d.node_id(false), &ip));
        let ip6 = Ip::V6([0x2a, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 1]);
        d.set_external_ip(ip6, 0);
        assert!(bep42_valid(&d.node_id(true), &ip6));
    }
}
