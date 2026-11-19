// SPDX-License-Identifier: GPL-3.0-only
//! The torrent engine proper: [`Torrent`] runs one torrent's swarm, and [`Session`] runs
//! many of them.
//!
//! A [`Torrent`] ties together the peer connections ([`crate::peer`]), the extensions
//! ([`crate::extension`]), the piece picker ([`crate::picker`]) and storage
//! ([`crate::storage`]). Like everything in this crate it does no I/O: the program feeds
//! it events and runs the [`Action`]s that come back.
//!
//! Events in: [`Torrent::add_peer`] (a connection opened, either way),
//! [`Torrent::on_peer_bytes`], [`Torrent::peer_closed`], [`Torrent::connect_failed`],
//! [`Torrent::on_tick`] (about once a second; [`Torrent::next_wakeup`] says when),
//! [`Torrent::on_disk_read`] / [`Torrent::on_disk_error`], [`Torrent::on_piece_verified`]
//! (when hashing is done by the program), [`Torrent::add_peers`] (from trackers, DHT,
//! LSD), [`Torrent::set_metadata`] (after a magnet link's info dictionary arrived and the
//! program parsed it). Actions out: bytes to send, disk operations, connections to open
//! or close, pieces done, finished, state changes.
//!
//! Choking follows BEP 3's tit-for-tat: every 10 s the 4 interested peers that upload to
//! us fastest are unchoked (when seeding: the ones we upload to fastest), plus one
//! optimistic unchoke rotated every 30 s so newcomers get a chance. Snubbed peers (no
//! data for a minute) do not get regular slots while downloading.
//!
//! Pieces are verified through [`PieceVerifier`] (v1 SHA-1 [`Sha1Pieces`] here; the v2
//! merkle verifier plugs in the same way) or, when none is given, by the program
//! ([`Action::Hash`] then [`Torrent::on_piece_verified`]), so hashing can run on another
//! core.
//!
//! [`Session`] holds torrents keyed by their wire info-hash. It keeps a timer set ordered
//! by each torrent's next wakeup, so a tick touches only torrents that are due: an idle
//! torrent (paused, queued, or with no peers and nobody to call) costs nothing per tick,
//! and every operation is O(log n) in the number of torrents. Queueing (at most N active
//! downloads and M active seeds, in queue order) is recomputed only when a torrent
//! changes between downloading, seeding and inactive.
//!
//! Left for later: BEP 52 hash requests are answered with `hash reject` (the merkle
//! verifier lives in `merkle`), upload/download rate limits are applied by the program
//! (`bandwidth`), and there is no read cache for uploads.
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use sha1::{Digest, Sha1};

use crate::extension::{self, ExtHandshake, MetadataAssembler, MetadataMsg, MetadataProgress, PexMsg, PexPeer, PexState, RemoteExt};
use crate::peer::{allowed_fast_set, peek_info_hash, Bitfield, BlockRef, Handshake, Message, PeerConfig, PeerConn, PeerError, PeerEvent, PeerKey, RateMeter, Reserved, WireError};
use crate::picker::{BlockOutcome, Picker, Playhead};
use crate::resume::ResumeData;
use crate::storage::{Allocation, BlockDone, DiskOp, LayoutError, ReadStart, Storage, TorrentLayout};
use crate::{InfoHashes, Millis, PeerAddr, PeerId};

/// Checks a whole piece against the torrent's hashes.
pub trait PieceVerifier {
    fn verify(&self, piece: u32, data: &[u8]) -> bool;
}

/// v1 verification: the SHA-1 of every piece, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sha1Pieces(pub Vec<[u8; 20]>);

impl PieceVerifier for Sha1Pieces {
    fn verify(&self, piece: u32, data: &[u8]) -> bool {
        self.0.get(piece as usize).is_some_and(|h| Sha1::digest(data).as_slice() == h)
    }
}

impl Sha1Pieces {
    /// Hashes a payload into pieces (for making torrents and tests).
    pub fn of(payload: &[u8], piece_length: u32) -> Sha1Pieces {
        Sha1Pieces(payload.chunks(piece_length.max(1) as usize).map(|c| { let mut h = [0u8; 20]; h.copy_from_slice(&Sha1::digest(c)); h }).collect())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TorrentState {
    /// Hashing existing files.
    Checking,
    /// A magnet link: fetching the info dictionary from peers.
    DownloadingMetadata,
    Downloading,
    /// Every wanted piece is here; uploading.
    Seeding,
    Paused,
    /// Waiting for a download or seed slot ([`Session`] queueing).
    Queued,
    /// A disk write failed; paused until [`Torrent::clear_error`].
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CloseReason {
    Peer(PeerError),
    /// We connected to ourselves.
    SelfConnection,
    /// Already connected to this peer ID.
    Duplicate,
    TooManyPeers,
    /// The torrent is paused, queued, checking, in error or removed.
    Inactive,
    /// The peer sent data that failed the hash too often.
    Banned,
    /// Both sides have everything.
    NotUseful,
}

/// What the program should do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Send { peer: PeerKey, bytes: Vec<u8> },
    Disk(DiskOp),
    /// Close the connection (the torrent has already forgotten it).
    Close { peer: PeerKey, reason: CloseReason },
    /// Open a connection, then call [`Torrent::add_peer`] or [`Torrent::connect_failed`].
    Connect { addr: PeerAddr },
    /// Hash this piece and call [`Torrent::on_piece_verified`] (only without a verifier).
    Hash { piece: u32, data: Vec<u8> },
    PieceDone(u32),
    PieceFailed(u32),
    /// The info dictionary of a magnet link, verified against the info-hash. Parse it
    /// and call [`Torrent::set_metadata`].
    MetadataReceived(Vec<u8>),
    /// A peer's DHT node (BEP 5 `port` message).
    DhtNode(PeerAddr),
    /// Every wanted piece is downloaded.
    Finished,
    StateChanged(TorrentState),
}

/// Tunables for a torrent.
#[derive(Clone, Debug)]
pub struct TorrentConfig {
    pub peer: PeerConfig,
    pub max_connections: usize,
    /// Regular unchoke slots (plus one optimistic).
    pub unchoke_slots: usize,
    pub unchoke_interval: Millis,
    pub optimistic_interval: Millis,
    /// New outgoing connections per tick.
    pub connects_per_tick: usize,
    pub max_known_peers: usize,
    pub listen_port: Option<u16>,
    pub dht_port: Option<u16>,
    pub client: String,
    /// No client name or listen port in handshakes.
    pub anonymous: bool,
    pub cache_bytes: usize,
    pub allocation: Allocation,
    /// Hash failures a peer may cause before it is banned.
    pub hashfail_ban: u32,
    /// Size of the allowed-fast set we give peers.
    pub allowed_fast: usize,
    /// Pieces read in parallel while checking.
    pub check_parallel: usize,
    pub pex: bool,
    /// Timeout of a metadata piece request.
    pub metadata_timeout: Millis,
    pub tick: Millis,
}

impl Default for TorrentConfig {
    fn default() -> Self {
        TorrentConfig {
            peer: PeerConfig::default(), max_connections: 50, unchoke_slots: 4, unchoke_interval: 10_000, optimistic_interval: 30_000,
            connects_per_tick: 5, max_known_peers: 2000, listen_port: None, dht_port: None, client: String::from("Swingers 0.1"),
            anonymous: false, cache_bytes: 16 << 20, allocation: Allocation::Sparse, hashfail_ban: 3, allowed_fast: 10,
            check_parallel: 2, pex: true, metadata_timeout: 15_000, tick: 1_000,
        }
    }
}

/// How to start a torrent.
pub struct TorrentParams {
    pub info_hashes: InfoHashes,
    pub peer_id: PeerId,
    /// `None` for a magnet link (metadata is fetched from peers).
    pub layout: Option<TorrentLayout>,
    /// The raw info dictionary, to serve to magnet-link peers (BEP 9).
    pub info: Option<Vec<u8>>,
    /// `None`: the program hashes ([`Action::Hash`]).
    pub verifier: Option<Box<dyn PieceVerifier>>,
    /// One per file; empty means all 1.
    pub file_priorities: Vec<u8>,
    /// Pieces known good (fast resume or a freshly made torrent).
    pub have: Option<Bitfield>,
    /// Hash the existing files first.
    pub check_files: bool,
    pub resume: Option<ResumeData>,
    pub paused: bool,
    pub config: TorrentConfig,
    /// Seeds the torrent's random choices (tie-breaks, optimistic unchokes).
    pub rng_seed: u64,
}

impl TorrentParams {
    pub fn new(info_hashes: InfoHashes, peer_id: PeerId) -> TorrentParams {
        TorrentParams { info_hashes, peer_id, layout: None, info: None, verifier: None, file_priorities: Vec::new(), have: None, check_files: false, resume: None, paused: false, config: TorrentConfig::default(), rng_seed: 1 }
    }
}

/// A snapshot for the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorrentStats {
    pub state: TorrentState,
    pub num_pieces: u32,
    pub have_pieces: u32,
    /// Of all pieces, in millionths.
    pub progress_ppm: u32,
    /// Bytes of wanted pieces still missing (pieces times piece length, so approximate).
    pub left: u64,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    /// Bytes received that were not used (duplicates, failed hashes).
    pub wasted: u64,
    /// uploaded / downloaded, in thousandths.
    pub ratio_milli: u64,
    pub eta_secs: Option<u64>,
    pub peers: usize,
    pub seeds: usize,
    pub known_peers: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Counted { No, Pieces, Seed }

struct PeerSlot {
    conn: PeerConn,
    addr: PeerAddr,
    ext: RemoteExt,
    listen_port: Option<u16>,
    pex: PexState,
    counted: Counted,
    hashfails: u32,
    upload_only: bool,
}

struct Meta {
    storage: Storage,
    picker: Picker,
    verifier: Option<Box<dyn PieceVerifier>>,
}

#[derive(Clone, Copy, Debug)]
struct Known { failures: u32, next_try: Millis, connected: bool, banned: bool, seed: bool }

#[derive(Clone, Copy, Debug)]
enum ReadPurpose { Upload { peer: PeerKey, block: BlockRef }, Verify { piece: u32 }, Check { piece: u32 } }

#[derive(Clone, Copy, Debug)]
struct Check { next: u32, inflight: usize }

/// Which queue a torrent counts against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueueKind { Download, Seed }

/// One torrent's state machine.
pub struct Torrent {
    hashes: InfoHashes,
    wire_hash: [u8; 20],
    alt_hash: Option<[u8; 20]>,
    our_id: PeerId,
    cfg: TorrentConfig,
    state: TorrentState,
    paused: bool,
    queued: bool,
    error: bool,
    meta: Option<Meta>,
    file_prio: Vec<u8>,
    info: Option<Vec<u8>>,
    metadata: Option<MetadataAssembler>,
    peers: BTreeMap<PeerKey, PeerSlot>,
    known: BTreeMap<PeerAddr, Known>,
    connecting: BTreeMap<PeerAddr, Millis>,
    reads: BTreeMap<u64, ReadPurpose>,
    /// Pieces handed to the program to hash: downloaded data to write (None for checks).
    hashing: BTreeMap<u32, Option<Vec<u8>>>,
    check: Option<Check>,
    out: Vec<Action>,
    rng: u64,
    next_unchoke: Millis,
    next_optimistic: Millis,
    optimistic: Option<PeerKey>,
    last_tick: Millis,
    down: RateMeter,
    up: RateMeter,
    downloaded: u64,
    uploaded: u64,
    wasted: u64,
    finished: bool,
    sequential: bool,
}

fn rand_step(s: &mut u64) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    (s.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as u32
}

impl Torrent {
    /// A new torrent. Initial actions (allocation, checking reads) are returned by the
    /// first event call, or by [`take_actions`](Self::take_actions).
    pub fn new(p: TorrentParams, now: Millis) -> Torrent {
        let wire_hash = p.info_hashes.wire();
        let alt_hash = match p.info_hashes { InfoHashes::Hybrid(_, v2) => Some(v2.truncated()), _ => None };
        let mut t = Torrent {
            hashes: p.info_hashes, wire_hash, alt_hash, our_id: p.peer_id, cfg: p.config, state: TorrentState::Paused,
            paused: p.paused, queued: false, error: false, meta: None, file_prio: p.file_priorities, info: p.info, metadata: None,
            peers: BTreeMap::new(), known: BTreeMap::new(), connecting: BTreeMap::new(), reads: BTreeMap::new(), hashing: BTreeMap::new(),
            check: None, out: Vec::new(), rng: p.rng_seed | 1, next_unchoke: now, next_optimistic: now, optimistic: None, last_tick: now,
            down: RateMeter::default(), up: RateMeter::default(), downloaded: 0, uploaded: 0, wasted: 0, finished: false, sequential: false,
        };
        let mut have = p.have;
        let mut unfinished = Vec::new();
        let mut parts = Vec::new();
        if let Some(r) = &p.resume {
            if t.file_prio.is_empty() { t.file_prio = r.file_priorities.clone(); }
            t.uploaded = r.uploaded;
            t.downloaded = r.downloaded;
            t.sequential = r.sequential;
            t.paused |= r.paused;
            if t.info.is_none() { t.info = r.info.clone(); }
            for a in &r.peers { t.insert_known(*a, false); }
            if let (Some(l), None) = (&p.layout, &have) {
                if r.num_pieces == l.num_pieces() && r.num_pieces > 0 { have = Some(Bitfield::from_bytes_lossy(&r.have, r.num_pieces)); unfinished = r.unfinished.clone(); }
            }
            parts = r.part_slices.clone();
        }
        match p.layout {
            Some(layout) => {
                let _ = t.install(layout, p.verifier, have, now);
                if let Some(m) = t.meta.as_mut() {
                    m.storage.set_part_slices(parts);
                    for (piece, blocks) in unfinished {
                        for (i, got) in blocks.iter().enumerate() {
                            if *got { m.picker.restore_block(piece, i as u32); m.storage.restore_block(piece, i as u32); }
                        }
                    }
                }
                if p.check_files { t.check = Some(Check { next: 0, inflight: 0 }); }
            }
            None => t.metadata = Some(MetadataAssembler::new(t.hashes)),
        }
        t.settle();
        t.pump_check(now);
        t
    }

    fn rand(&mut self) -> u32 { rand_step(&mut self.rng) }

    /// Builds storage and picker for a layout.
    fn install(&mut self, layout: TorrentLayout, verifier: Option<Box<dyn PieceVerifier>>, have: Option<Bitfield>, _now: Millis) -> Result<(), LayoutError> {
        layout.validate()?;
        let n = layout.num_pieces();
        let storage = Storage::new(layout, &self.file_prio, self.cfg.cache_bytes);
        self.file_prio = storage.file_priorities().to_vec();
        let have = have.filter(|h| h.len() == n).unwrap_or_else(|| Bitfield::new(n));
        let mut rng = self.rng;
        let mut picker = Picker::new(n, storage.layout.piece_length, storage.layout.short_pieces(), have, &storage.piece_priorities(), &mut || rand_step(&mut rng));
        self.rng = rng;
        picker.set_sequential(self.sequential);
        self.out.extend(storage.allocate(self.cfg.allocation).into_iter().map(Action::Disk));
        self.finished = picker.is_finished();
        self.meta = Some(Meta { storage, picker, verifier });
        Ok(())
    }

    // ------------------------------------------------------------------ accessors

    pub fn info_hashes(&self) -> InfoHashes { self.hashes }
    /// The 20-byte hash used on the wire.
    pub fn wire_hash(&self) -> [u8; 20] { self.wire_hash }
    /// The second hash a hybrid torrent answers to (truncated v2).
    pub fn alt_hash(&self) -> Option<[u8; 20]> { self.alt_hash }
    pub fn state(&self) -> TorrentState { self.state }
    pub fn has_metadata(&self) -> bool { self.meta.is_some() }
    pub fn have(&self) -> Option<&Bitfield> { self.meta.as_ref().map(|m| m.picker.have()) }
    pub fn layout(&self) -> Option<&TorrentLayout> { self.meta.as_ref().map(|m| &m.storage.layout) }
    pub fn num_peers(&self) -> usize { self.peers.len() }
    pub fn peer_keys(&self) -> impl Iterator<Item = PeerKey> + '_ { self.peers.keys().copied() }
    pub fn is_finished(&self) -> bool { self.meta.as_ref().is_some_and(|m| m.picker.is_finished()) }
    /// Drains actions produced outside an event call (for example by [`Torrent::new`]).
    pub fn take_actions(&mut self) -> Vec<Action> { core::mem::take(&mut self.out) }

    fn active(&self) -> bool { matches!(self.state, TorrentState::Downloading | TorrentState::Seeding | TorrentState::DownloadingMetadata) }

    /// The queue this torrent counts against, or `None` when it is not eligible to run.
    pub fn queue_kind(&self) -> Option<QueueKind> {
        if self.paused || self.error || self.check.is_some() { return None; }
        Some(if self.is_finished() { QueueKind::Seed } else { QueueKind::Download })
    }

    fn settle(&mut self) {
        let s = if self.error { TorrentState::Error } else if self.paused { TorrentState::Paused } else if self.queued { TorrentState::Queued }
            else if self.check.is_some() { TorrentState::Checking } else if self.meta.is_none() { TorrentState::DownloadingMetadata }
            else if self.is_finished() { TorrentState::Seeding } else { TorrentState::Downloading };
        if s != self.state {
            self.state = s;
            self.out.push(Action::StateChanged(s));
            if !self.active() { self.close_all(CloseReason::Inactive); }
        }
    }

    /// End of every event: pump the check, send what peers queued, settle the state.
    fn finish(&mut self, now: Millis) -> Vec<Action> {
        self.pump_check(now);
        self.settle();
        self.flush();
        core::mem::take(&mut self.out)
    }

    fn flush(&mut self) {
        for (k, s) in self.peers.iter_mut() {
            if s.conn.has_outgoing() { self.out.push(Action::Send { peer: *k, bytes: s.conn.take_outgoing() }); }
        }
    }

    fn close_all(&mut self, reason: CloseReason) {
        let keys: Vec<PeerKey> = self.peers.keys().copied().collect();
        for k in keys { self.remove_peer(k, Some(reason)); }
        self.connecting.clear();
    }

    // ------------------------------------------------------------------ peers

    fn insert_known(&mut self, addr: PeerAddr, seed: bool) {
        if addr.port == 0 { return; }
        if let Some(k) = self.known.get_mut(&addr) { k.seed |= seed; return; }
        if self.known.len() >= self.cfg.max_known_peers {
            // Make room by forgetting a peer that keeps failing.
            let worst = self.known.iter().filter(|(_, k)| !k.connected && k.failures > 0).max_by_key(|(_, k)| k.failures).map(|(a, _)| *a);
            match worst { Some(w) => { self.known.remove(&w); } None => return }
        }
        self.known.insert(addr, Known { failures: 0, next_try: 0, connected: false, banned: false, seed });
    }

    /// Peers from a tracker, the DHT, LSD or resume data.
    pub fn add_peers(&mut self, addrs: &[PeerAddr], now: Millis) -> Vec<Action> {
        for a in addrs { self.insert_known(*a, false); }
        self.connect_more(now);
        self.finish(now)
    }

    fn connect_more(&mut self, now: Millis) {
        if !self.active() { return; }
        self.connecting.retain(|_, t| now.saturating_sub(*t) < 60_000);
        let room = self.cfg.max_connections.saturating_sub(self.peers.len() + self.connecting.len()).min(self.cfg.connects_per_tick);
        if room == 0 { return; }
        let seeding = self.is_finished();
        let mut cands: Vec<(u32, PeerAddr)> = self.known.iter()
            .filter(|(a, k)| !k.connected && !k.banned && k.next_try <= now && !(seeding && k.seed) && !self.connecting.contains_key(*a))
            .map(|(a, k)| (k.failures, *a)).collect();
        cands.sort_unstable();
        for (_, a) in cands.into_iter().take(room) {
            self.connecting.insert(a, now);
            if let Some(k) = self.known.get_mut(&a) { k.next_try = now + 60_000; }
            self.out.push(Action::Connect { addr: a });
        }
    }

    /// An outgoing connection could not be opened.
    pub fn connect_failed(&mut self, addr: PeerAddr, now: Millis) -> Vec<Action> {
        self.connecting.remove(&addr);
        if let Some(k) = self.known.get_mut(&addr) {
            k.failures += 1;
            k.next_try = now + (30_000u64 << k.failures.min(6));
            if k.failures >= 5 { self.known.remove(&addr); }
        }
        self.finish(now)
    }

    /// A connection opened: `outgoing` after our [`Action::Connect`], otherwise an
    /// incoming one routed here (see [`Session::route`]).
    pub fn add_peer(&mut self, key: PeerKey, addr: PeerAddr, outgoing: bool, now: Millis) -> Vec<Action> {
        self.connecting.remove(&addr);
        let banned = self.known.get(&addr).is_some_and(|k| k.banned);
        let reason = if !self.active() { Some(CloseReason::Inactive) } else if banned { Some(CloseReason::Banned) }
            else if self.peers.len() >= self.cfg.max_connections || self.peers.contains_key(&key) { Some(CloseReason::TooManyPeers) } else { None };
        if let Some(r) = reason { self.out.push(Action::Close { peer: key, reason: r }); return self.finish(now); }
        let pc = &self.cfg.peer;
        let hs = Handshake { reserved: Reserved::new(pc.extensions, pc.fast, pc.dht && self.cfg.dht_port.is_some(), false), info_hash: self.wire_hash, peer_id: self.our_id.0 };
        let n = self.meta.as_ref().map(|m| m.picker.num_pieces()).unwrap_or(0);
        let conn = PeerConn::new(self.cfg.peer.clone(), hs, self.alt_hash, outgoing, n, now);
        if outgoing { self.insert_known(addr, false); }
        if let Some(k) = self.known.get_mut(&addr) { k.connected = true; }
        self.peers.insert(key, PeerSlot { conn, addr, ext: RemoteExt::default(), listen_port: None, pex: PexState::default(), counted: Counted::No, hashfails: 0, upload_only: false });
        self.finish(now)
    }

    /// The program saw the connection close.
    pub fn peer_closed(&mut self, key: PeerKey, now: Millis) -> Vec<Action> {
        self.remove_peer(key, None);
        self.finish(now)
    }

    fn remove_peer(&mut self, key: PeerKey, reason: Option<CloseReason>) {
        let Some(s) = self.peers.remove(&key) else { return };
        if let Some(m) = self.meta.as_mut() {
            m.picker.peer_gone(key);
            match s.counted { Counted::Seed => m.picker.remove_peer_pieces(&s.conn.have, true), Counted::Pieces => m.picker.remove_peer_pieces(&s.conn.have, false), Counted::No => {} }
        }
        if let Some(a) = self.metadata.as_mut() { a.peer_gone(key); }
        if let Some(k) = self.known.get_mut(&s.addr) {
            k.connected = false;
            if reason == Some(CloseReason::Banned) { k.banned = true; }
            if s.conn.is_seed() { k.seed = true; }
        }
        if self.optimistic == Some(key) { self.optimistic = None; }
        if let Some(r) = reason { self.out.push(Action::Close { peer: key, reason: r }); }
    }

    /// Bytes arrived from a peer.
    pub fn on_peer_bytes(&mut self, key: PeerKey, bytes: &[u8], now: Millis) -> Vec<Action> {
        let Some(s) = self.peers.get_mut(&key) else { return self.finish(now) };
        match s.conn.on_bytes(bytes, now) {
            Err(e) => self.remove_peer(key, Some(CloseReason::Peer(e))),
            Ok(events) => {
                for ev in events {
                    if !self.peers.contains_key(&key) { break; }
                    self.handle(key, ev, now);
                }
                self.fill(key, now);
            }
        }
        self.finish(now)
    }

    fn handle(&mut self, key: PeerKey, ev: PeerEvent, now: Millis) {
        match ev {
            PeerEvent::Handshake(h) => self.on_handshake(key, h, now),
            PeerEvent::Pieces => self.on_pieces(key, now),
            PeerEvent::PiecesPending | PeerEvent::Cancel(_) | PeerEvent::Unchoked | PeerEvent::Suggest(_) | PeerEvent::AllowedFast(_) | PeerEvent::Hashes { .. } | PeerEvent::HashReject(_) => {}
            PeerEvent::Have(i) => {
                let Some(m) = self.meta.as_mut() else { return };
                let Some(s) = self.peers.get_mut(&key) else { return };
                if s.counted == Counted::No { s.counted = Counted::Pieces; }
                m.picker.inc(i);
                if m.picker.wanted(i) { s.conn.set_interested(true, now); }
                if m.picker.is_seed() && s.conn.is_seed() { self.remove_peer(key, Some(CloseReason::NotUseful)); }
            }
            PeerEvent::Block { block, data, .. } => self.on_block(key, block, data, now),
            PeerEvent::Request(b) => self.on_request(key, b, now),
            PeerEvent::Rejected(b) => { if let Some(m) = self.meta.as_mut() { m.picker.on_request_lost(key, &b); } }
            PeerEvent::Choked { dropped } => { if let Some(m) = self.meta.as_mut() { for b in &dropped { m.picker.on_request_lost(key, b); } } }
            PeerEvent::Interested => {
                let unchoked = self.peers.values().filter(|s| !s.conn.am_choking).count();
                if unchoked < self.cfg.unchoke_slots { if let Some(s) = self.peers.get_mut(&key) { s.conn.unchoke(now); } }
            }
            PeerEvent::NotInterested => { if let Some(s) = self.peers.get_mut(&key) { s.conn.choke(now); } }
            PeerEvent::Port(p) => { if let Some(s) = self.peers.get(&key) { if p != 0 { self.out.push(Action::DhtNode(PeerAddr { ip: s.addr.ip, port: p })); } } }
            PeerEvent::Extended { id, payload } => self.on_extended(key, id, &payload, now),
            PeerEvent::HashRequest(r) => { if let Some(s) = self.peers.get_mut(&key) { s.conn.send_message(&Message::HashReject(r), now); } }
        }
    }

    fn on_handshake(&mut self, key: PeerKey, h: Handshake, now: Millis) {
        if h.peer_id == self.our_id.0 { self.remove_peer(key, Some(CloseReason::SelfConnection)); return; }
        if self.peers.iter().any(|(k, s)| *k != key && s.conn.remote.is_some_and(|r| r.peer_id == h.peer_id)) { self.remove_peer(key, Some(CloseReason::Duplicate)); return; }
        let Some(s) = self.peers.get_mut(&key) else { return };
        if s.conn.extensions() {
            let md = self.info.as_ref().map(|i| i.len() as u32);
            let upload_only = self.meta.as_ref().is_some_and(|m| m.picker.is_finished());
            let hs = ExtHandshake::ours(&self.cfg.client, self.cfg.listen_port, Some(s.addr.ip), self.cfg.peer.max_incoming as u32, md, upload_only, self.cfg.anonymous);
            s.conn.send_extended(extension::HANDSHAKE_ID, &hs.encode(), now);
        }
        match self.meta.as_ref() {
            Some(m) => {
                s.conn.send_pieces(m.picker.have(), now);
                if s.conn.fast() && self.cfg.allowed_fast > 0 {
                    for p in allowed_fast_set(s.addr.ip, &self.wire_hash, m.picker.num_pieces(), self.cfg.allowed_fast) {
                        if m.picker.has(p) { s.conn.send_allowed_fast(p, now); }
                    }
                }
            }
            None => { if s.conn.fast() { s.conn.send_message(&Message::HaveNone, now); } }
        }
        if let (Some(port), true) = (self.cfg.dht_port, h.reserved.dht() && self.cfg.peer.dht) { s.conn.send_message(&Message::Port(port), now); }
    }

    fn on_pieces(&mut self, key: PeerKey, now: Millis) {
        let Some(m) = self.meta.as_mut() else { return };
        let Some(s) = self.peers.get_mut(&key) else { return };
        m.picker.add_peer_pieces(&s.conn.have);
        s.counted = if s.conn.have.all() { Counted::Seed } else { Counted::Pieces };
        if m.picker.is_seed() && s.conn.is_seed() { self.remove_peer(key, Some(CloseReason::NotUseful)); return; }
        self.update_interest(key, now);
    }

    fn update_interest(&mut self, key: PeerKey, now: Millis) {
        let Some(s) = self.peers.get_mut(&key) else { return };
        let want = match self.meta.as_ref() { Some(m) => s.conn.have.ones().any(|p| m.picker.wanted(p)), None => false };
        s.conn.set_interested(want, now);
    }

    /// Requests as many blocks as the peer's pipeline has room for.
    fn fill(&mut self, key: PeerKey, now: Millis) {
        if self.state != TorrentState::Downloading { return; }
        let Some(m) = self.meta.as_mut() else { return };
        let Some(s) = self.peers.get_mut(&key) else { return };
        if !s.conn.is_open() || !s.conn.am_interested { return; }
        let only = if s.conn.peer_choking { if s.conn.allowed_fast_in.is_empty() { return; } Some(s.conn.allowed_fast_in.clone()) } else { None };
        let room = s.conn.request_room(now);
        if room == 0 { return; }
        let blocks = m.picker.pick(key, &s.conn.have, room, only.as_deref(), now);
        let none = blocks.is_empty();
        for b in blocks { s.conn.request(b, now); }
        if none && s.conn.outstanding.is_empty() && !s.conn.peer_choking { self.update_interest(key, now); }
    }

    fn fill_all(&mut self, now: Millis) {
        let keys: Vec<PeerKey> = self.peers.keys().copied().collect();
        for k in keys { self.fill(k, now); }
    }

    // ------------------------------------------------------------------ download path

    fn on_block(&mut self, key: PeerKey, block: BlockRef, data: Vec<u8>, now: Millis) {
        let Some(m) = self.meta.as_mut() else { self.wasted += data.len() as u64; return };
        match m.picker.on_block(key, &block) {
            BlockOutcome::NotWanted | BlockOutcome::Duplicate => self.wasted += data.len() as u64,
            BlockOutcome::Accepted { cancel, .. } => {
                self.downloaded += data.len() as u64;
                self.down.add(data.len() as u64, now);
                for c in cancel { if let Some(s) = self.peers.get_mut(&c) { s.conn.cancel(block, now); } }
                let (ops, done) = m.storage.add_block(block.piece, block.begin, &data);
                self.out.extend(ops.into_iter().map(Action::Disk));
                match done {
                    BlockDone::Pending => {}
                    BlockDone::Complete(buf) => self.verify(block.piece, buf, false, now),
                    BlockDone::Reading(id) => { self.reads.insert(id, ReadPurpose::Verify { piece: block.piece }); }
                }
            }
        }
    }

    fn verify(&mut self, piece: u32, data: Vec<u8>, check: bool, now: Millis) {
        let Some(m) = self.meta.as_ref() else { return };
        match &m.verifier {
            Some(v) => { let ok = v.verify(piece, &data); self.piece_result(piece, ok, if check { None } else { Some(data) }, now); }
            None => {
                self.hashing.insert(piece, if check { None } else { Some(data.clone()) });
                self.out.push(Action::Hash { piece, data });
            }
        }
    }

    /// The program finished hashing a piece it got through [`Action::Hash`].
    pub fn on_piece_verified(&mut self, piece: u32, ok: bool, now: Millis) -> Vec<Action> {
        if let Some(data) = self.hashing.remove(&piece) { self.piece_result(piece, ok, data, now); self.fill_all(now); }
        self.finish(now)
    }

    /// `data` is `Some` for a downloaded piece, `None` for a check.
    fn piece_result(&mut self, piece: u32, ok: bool, data: Option<Vec<u8>>, now: Millis) {
        let Some(m) = self.meta.as_mut() else { return };
        let Some(data) = data else {
            if ok { m.picker.piece_passed(piece); }
            if let Some(c) = self.check.as_mut() { c.inflight = c.inflight.saturating_sub(1); }
            return;
        };
        if ok {
            let ops = m.storage.write(piece, 0, &data);
            self.out.extend(ops.into_iter().map(Action::Disk));
            m.picker.piece_passed(piece);
            self.out.push(Action::PieceDone(piece));
            for s in self.peers.values_mut() { if s.conn.is_open() && !s.conn.have.get(piece) { s.conn.send_have(piece, now); } }
            if m.picker.is_finished() && !self.finished { self.on_finished(now); }
        } else {
            m.storage.discard(piece);
            self.wasted += data.len() as u64;
            let who = m.picker.piece_failed(piece);
            self.out.push(Action::PieceFailed(piece));
            for k in who {
                let Some(s) = self.peers.get_mut(&k) else { continue };
                s.hashfails += 1;
                if s.hashfails >= self.cfg.hashfail_ban { self.remove_peer(k, Some(CloseReason::Banned)); }
            }
        }
    }

    fn on_finished(&mut self, now: Millis) {
        self.finished = true;
        self.out.push(Action::Finished);
        let seed = self.meta.as_ref().is_some_and(|m| m.picker.is_seed());
        let keys: Vec<PeerKey> = self.peers.keys().copied().collect();
        for k in keys {
            let Some(s) = self.peers.get_mut(&k) else { continue };
            s.conn.set_interested(false, now);
            if seed && (s.conn.is_seed() || s.upload_only) { self.remove_peer(k, Some(CloseReason::NotUseful)); }
        }
    }

    // ------------------------------------------------------------------ upload path

    fn on_request(&mut self, key: PeerKey, b: BlockRef, now: Millis) {
        let Some(s) = self.peers.get_mut(&key) else { return };
        let Some(m) = self.meta.as_mut() else { s.conn.reject(b, now); return };
        let valid = m.picker.has(b.piece) && b.begin.checked_add(b.len).is_some_and(|e| e <= m.picker.piece_size(b.piece));
        if !valid { s.conn.reject(b, now); return; }
        match m.storage.read(b.piece, b.begin, b.len) {
            ReadStart::Ready(data) => { if s.conn.send_block(b, &data, now) { self.uploaded += data.len() as u64; self.up.add(data.len() as u64, now); } }
            ReadStart::Pending { id, ops } => {
                self.reads.insert(id, ReadPurpose::Upload { peer: key, block: b });
                self.out.extend(ops.into_iter().map(Action::Disk));
            }
        }
    }

    /// A [`DiskOp::Read`] or [`DiskOp::ReadPart`] completed.
    pub fn on_disk_read(&mut self, id: u64, at: u32, data: &[u8], now: Millis) -> Vec<Action> {
        let Some(m) = self.meta.as_mut() else { return self.finish(now) };
        if let Some(buf) = m.storage.on_read(id, at, data) {
            match self.reads.remove(&id) {
                Some(ReadPurpose::Upload { peer, block }) => {
                    if let Some(s) = self.peers.get_mut(&peer) {
                        if s.conn.send_block(block, &buf, now) { self.uploaded += buf.len() as u64; self.up.add(buf.len() as u64, now); }
                    }
                }
                Some(ReadPurpose::Verify { piece }) => { self.verify(piece, buf, false, now); self.fill_all(now); }
                Some(ReadPurpose::Check { piece }) => self.verify(piece, buf, true, now),
                None => {}
            }
        }
        self.finish(now)
    }

    /// A read failed (a missing file while checking, or an I/O error).
    pub fn on_disk_error(&mut self, id: u64, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() { m.storage.read_failed(id); }
        match self.reads.remove(&id) {
            Some(ReadPurpose::Upload { peer, block }) => { if let Some(s) = self.peers.get_mut(&peer) { s.conn.reject(block, now); } }
            Some(ReadPurpose::Verify { piece }) => { if let Some(m) = self.meta.as_mut() { m.picker.piece_failed(piece); } }
            Some(ReadPurpose::Check { .. }) => { if let Some(c) = self.check.as_mut() { c.inflight = c.inflight.saturating_sub(1); } }
            None => {}
        }
        self.finish(now)
    }

    /// A write failed: the torrent stops in [`TorrentState::Error`].
    pub fn on_write_error(&mut self, now: Millis) -> Vec<Action> { self.error = true; self.finish(now) }
    pub fn clear_error(&mut self, now: Millis) -> Vec<Action> { self.error = false; let a = self.finish(now); self.resume_after(a, now) }

    // ------------------------------------------------------------------ checking

    /// Hashes every piece again (pieces that fail are downloaded again).
    pub fn force_recheck(&mut self, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() {
            for p in 0..m.picker.num_pieces() { m.picker.piece_lost(p); }
            self.finished = false;
            self.check = Some(Check { next: 0, inflight: 0 });
        }
        self.finish(now)
    }

    fn pump_check(&mut self, now: Millis) {
        loop {
            let Some(c) = self.check else { return };
            let Some(m) = self.meta.as_mut() else { self.check = None; return };
            let n = m.picker.num_pieces();
            if c.next >= n {
                if c.inflight == 0 && self.hashing.is_empty() {
                    self.check = None;
                    self.finished = m.picker.is_finished();
                    self.settle();
                    self.connect_more(now);
                }
                return;
            }
            if c.inflight >= self.cfg.check_parallel.max(1) { return; }
            let p = c.next;
            self.check = Some(Check { next: p + 1, inflight: c.inflight + 1 });
            match m.storage.read(p, 0, m.picker.piece_size(p)) {
                ReadStart::Ready(buf) => self.verify(p, buf, true, now),
                ReadStart::Pending { id, ops } => { self.reads.insert(id, ReadPurpose::Check { piece: p }); self.out.extend(ops.into_iter().map(Action::Disk)); }
            }
        }
    }

    // ------------------------------------------------------------------ extensions

    fn send_ext(&mut self, key: PeerKey, pick: impl Fn(&RemoteExt) -> u8, payload: &[u8], now: Millis) {
        if let Some(s) = self.peers.get_mut(&key) { let id = pick(&s.ext); if id != 0 { s.conn.send_extended(id, payload, now); } }
    }

    fn on_extended(&mut self, key: PeerKey, id: u8, payload: &[u8], now: Millis) {
        match id {
            extension::HANDSHAKE_ID => {
                let Some(h) = ExtHandshake::decode(payload) else { return };
                let Some(s) = self.peers.get_mut(&key) else { return };
                s.ext.update(&h);
                if let Some(q) = h.reqq { s.conn.peer_reqq = q.min(self.cfg.peer.max_queue); }
                if h.port.is_some() { s.listen_port = h.port; }
                s.upload_only = h.upload_only;
                if let (Some(a), Some(size)) = (self.metadata.as_mut(), h.metadata_size) { a.set_size(size); }
                if self.is_finished() && self.meta.as_ref().is_some_and(|m| m.picker.is_seed()) && h.upload_only { self.remove_peer(key, Some(CloseReason::NotUseful)); return; }
                self.request_metadata(key, now);
            }
            extension::UT_METADATA => {
                let Some(msg) = MetadataMsg::decode(payload) else { return };
                match msg {
                    MetadataMsg::Request(p) => {
                        let reply = match &self.info { Some(info) => extension::metadata_piece(info, p), None => MetadataMsg::Reject(p) };
                        self.send_ext(key, |e| e.ut_metadata, &reply.encode(), now);
                    }
                    MetadataMsg::Reject(p) => { if let Some(a) = self.metadata.as_mut() { a.on_reject(key, p); } }
                    MetadataMsg::Data { piece, total_size, data } => {
                        let Some(a) = self.metadata.as_mut() else { return };
                        match a.on_data(key, piece, total_size, &data) {
                            MetadataProgress::Incomplete => self.request_metadata(key, now),
                            MetadataProgress::Done(info) => {
                                self.metadata = None;
                                self.info = Some(info.clone());
                                self.out.push(Action::MetadataReceived(info));
                            }
                            MetadataProgress::Failed(who) => {
                                for k in who { if let Some(s) = self.peers.get_mut(&k) { s.hashfails += 1; } }
                            }
                        }
                    }
                }
            }
            extension::UT_PEX => {
                if !self.cfg.pex { return; }
                let Some(m) = PexMsg::decode(payload) else { return };
                for p in m.added { self.insert_known(p.addr, p.flags & extension::PEX_SEED != 0); }
                self.connect_more(now);
            }
            extension::LT_DONTHAVE => {
                let (Some(p), Some(m)) = (extension::decode_donthave(payload), self.meta.as_mut()) else { return };
                let Some(s) = self.peers.get_mut(&key) else { return };
                if s.conn.have.clear(p) {
                    if s.counted == Counted::Seed { m.picker.seed_lost_piece(&s.conn.have); s.counted = Counted::Pieces; } else { m.picker.dec(p); }
                }
            }
            extension::UPLOAD_ONLY => {
                if let (Some(v), Some(s)) = (extension::decode_upload_only(payload), self.peers.get_mut(&key)) { s.upload_only = v; }
            }
            _ => {}
        }
    }

    fn request_metadata(&mut self, key: PeerKey, now: Millis) {
        let timeout = self.cfg.metadata_timeout;
        let Some(a) = self.metadata.as_mut() else { return };
        let Some(s) = self.peers.get_mut(&key) else { return };
        if s.ext.ut_metadata == 0 || a.size() == 0 { return; }
        while a.outstanding(key) < 2 {
            let Some(p) = a.next_request(key, now, timeout) else { break };
            s.conn.send_extended(s.ext.ut_metadata, &MetadataMsg::Request(p).encode(), now);
        }
    }

    /// Installs the metadata of a magnet link once the program parsed the info
    /// dictionary from [`Action::MetadataReceived`].
    pub fn set_metadata(&mut self, layout: TorrentLayout, verifier: Option<Box<dyn PieceVerifier>>, now: Millis) -> Result<Vec<Action>, LayoutError> {
        if self.meta.is_some() { return Ok(self.finish(now)); }
        self.install(layout, verifier, None, now)?;
        self.metadata = None;
        let n = self.meta.as_ref().map(|m| m.picker.num_pieces()).unwrap_or(0);
        let keys: Vec<PeerKey> = self.peers.keys().copied().collect();
        for k in keys {
            let Some(s) = self.peers.get_mut(&k) else { continue };
            if let Err(e) = s.conn.set_num_pieces(n) { self.remove_peer(k, Some(CloseReason::Peer(e))); continue; }
            if let Some(m) = self.meta.as_mut() { m.picker.add_peer_pieces(&s.conn.have); }
            s.counted = if s.conn.have.all() { Counted::Seed } else { Counted::Pieces };
            self.update_interest(k, now);
        }
        self.settle();
        self.fill_all(now);
        Ok(self.finish(now))
    }

    // ------------------------------------------------------------------ timers

    /// When [`on_tick`](Self::on_tick) next has work, or `None` while idle (paused,
    /// queued, checking, or no peers and nobody to call).
    pub fn next_wakeup(&self) -> Option<Millis> {
        if !self.active() { return None; }
        if !self.peers.is_empty() || !self.connecting.is_empty() { return Some(self.last_tick + self.cfg.tick); }
        let room = self.peers.len() < self.cfg.max_connections;
        let seeding = self.is_finished();
        let next = self.known.values().filter(|k| room && !k.connected && !k.banned && !(seeding && k.seed)).map(|k| k.next_try).min()?;
        Some(next.max(self.last_tick + self.cfg.tick))
    }

    pub fn on_tick(&mut self, now: Millis) -> Vec<Action> {
        self.last_tick = now;
        if !self.active() { return self.finish(now); }
        let keys: Vec<PeerKey> = self.peers.keys().copied().collect();
        for k in &keys {
            let Some(s) = self.peers.get_mut(k) else { continue };
            match s.conn.on_tick(now) {
                Err(e) => self.remove_peer(*k, Some(CloseReason::Peer(e))),
                Ok(lost) => { if let Some(m) = self.meta.as_mut() { for b in &lost { m.picker.on_request_lost(*k, b); } } }
            }
            self.request_metadata(*k, now);
        }
        self.send_pex(now);
        if now >= self.next_unchoke {
            let rotate = now >= self.next_optimistic;
            self.recalc_unchoke(rotate, now);
            self.next_unchoke = now + self.cfg.unchoke_interval;
            if rotate { self.next_optimistic = now + self.cfg.optimistic_interval; }
        }
        self.connect_more(now);
        self.fill_all(now);
        self.finish(now)
    }

    fn send_pex(&mut self, now: Millis) {
        if !self.cfg.pex || self.peers.values().all(|s| s.ext.ut_pex == 0) { return; }
        let list: Vec<(PeerKey, PexPeer)> = self.peers.iter().filter(|(_, s)| s.conn.is_open()).filter_map(|(k, s)| {
            let port = if s.conn.outgoing { s.addr.port } else { s.listen_port? };
            let flags = if s.conn.is_seed() || s.upload_only { extension::PEX_SEED } else { 0 };
            Some((*k, PexPeer { addr: PeerAddr { ip: s.addr.ip, port }, flags }))
        }).collect();
        for (k, s) in self.peers.iter_mut() {
            if s.ext.ut_pex == 0 { continue; }
            let others: Vec<PexPeer> = list.iter().filter(|(o, _)| o != k).map(|(_, p)| *p).collect();
            if let Some(msg) = s.pex.build(&others, now) { s.conn.send_extended(s.ext.ut_pex, &msg.encode(), now); }
        }
    }

    fn recalc_unchoke(&mut self, rotate: bool, now: Millis) {
        let seeding = self.is_finished() || self.meta.is_none();
        let mut cands: Vec<(u64, u32, PeerKey)> = Vec::new();
        let keys: Vec<PeerKey> = self.peers.keys().copied().collect();
        for k in &keys {
            let r = self.rand();
            let Some(s) = self.peers.get(k) else { continue };
            if !s.conn.is_open() || !s.conn.peer_interested || (!seeding && s.conn.snubbed) { continue; }
            let rate = if seeding { s.conn.up.rate(now) } else { s.conn.down.rate(now) };
            cands.push((rate, r, *k));
        }
        cands.sort_unstable_by(|a, b| b.cmp(a));
        let regular: Vec<PeerKey> = cands.iter().take(self.cfg.unchoke_slots).map(|c| c.2).collect();
        let valid = self.optimistic.is_some_and(|o| cands.iter().any(|c| c.2 == o) && !regular.contains(&o));
        if rotate || !valid {
            let rest: Vec<PeerKey> = cands.iter().map(|c| c.2).filter(|k| !regular.contains(k)).collect();
            self.optimistic = if rest.is_empty() { None } else { Some(rest[self.rand() as usize % rest.len()]) };
        }
        for (k, s) in self.peers.iter_mut() {
            if regular.contains(k) || self.optimistic == Some(*k) { s.conn.unchoke(now); } else { s.conn.choke(now); }
        }
    }

    // ------------------------------------------------------------------ control

    pub fn pause(&mut self, now: Millis) -> Vec<Action> { self.paused = true; self.finish(now) }
    pub fn resume(&mut self, now: Millis) -> Vec<Action> { self.paused = false; let a = self.finish(now); self.resume_after(a, now) }
    /// Used by [`Session`] queueing.
    pub fn set_queued(&mut self, queued: bool, now: Millis) -> Vec<Action> { self.queued = queued; let a = self.finish(now); self.resume_after(a, now) }

    fn resume_after(&mut self, mut a: Vec<Action>, now: Millis) -> Vec<Action> {
        if self.active() { self.connect_more(now); a.extend(self.finish(now)); }
        a
    }

    pub fn set_file_priority(&mut self, file: usize, prio: u8, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() {
            let ops = m.storage.set_file_priority(file, prio);
            self.out.extend(ops.into_iter().map(Action::Disk));
            m.picker.set_priorities(&m.storage.piece_priorities());
            self.file_prio = m.storage.file_priorities().to_vec();
            if !m.picker.is_finished() { self.finished = false; } else if !self.finished { self.on_finished(now); }
            let keys: Vec<PeerKey> = self.peers.keys().copied().collect();
            for k in keys { self.update_interest(k, now); }
        } else if let Some(p) = self.file_prio.get_mut(file) { *p = prio.min(7); }
        self.settle();
        self.fill_all(now);
        self.finish(now)
    }
    pub fn set_sequential(&mut self, on: bool, now: Millis) -> Vec<Action> {
        self.sequential = on;
        if let Some(m) = self.meta.as_mut() { m.picker.set_sequential(on); }
        self.finish(now)
    }
    /// A streaming deadline for one piece (see [`Picker::set_deadline`]).
    pub fn set_deadline(&mut self, piece: u32, at: Millis, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() { m.picker.set_deadline(piece, at); }
        self.fill_all(now);
        self.finish(now)
    }
    pub fn clear_deadlines(&mut self, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() { m.picker.clear_deadlines(); }
        self.finish(now)
    }
    /// Starts streaming a play window (see [`Picker::set_playhead`]).
    pub fn set_playhead(&mut self, ph: Playhead, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() { m.picker.set_playhead(ph, now); }
        self.fill_all(now);
        self.finish(now)
    }
    /// Moves the play position (seek, or playback advancing).
    pub fn seek(&mut self, position: u32, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() { m.picker.seek(position, now); }
        self.fill_all(now);
        self.finish(now)
    }
    /// The playhead piece for a byte offset in a file (for players that seek by byte).
    pub fn piece_at(&self, file: usize, offset: u64) -> Option<u32> {
        let l = self.layout()?;
        let f = l.files.get(file)?;
        Some(((f.offset + offset.min(f.length.saturating_sub(1))) / l.piece_length as u64) as u32)
    }
    /// Writes out the write cache (call before saving resume data).
    pub fn flush_cache(&mut self, now: Millis) -> Vec<Action> {
        if let Some(m) = self.meta.as_mut() { let ops = m.storage.flush(); self.out.extend(ops.into_iter().map(Action::Disk)); }
        self.finish(now)
    }

    /// The engine's part of the resume data (the program adds name, save path, trackers,
    /// labels and times). Call [`flush_cache`](Self::flush_cache) first so the unfinished
    /// blocks it lists are on disk.
    pub fn resume_data(&self) -> ResumeData {
        let mut r = ResumeData {
            version: crate::resume::VERSION,
            info_hash_v1: self.hashes.v1().map(|h| h.0),
            info_hash_v2: self.hashes.v2().map(|h| h.0),
            file_priorities: self.file_prio.clone(),
            uploaded: self.uploaded,
            downloaded: self.downloaded,
            sequential: self.sequential,
            paused: self.paused,
            info: self.info.clone(),
            peers: self.known.iter().filter(|(_, k)| !k.banned).map(|(a, _)| *a).take(200).collect(),
            ..ResumeData::default()
        };
        if let Some(m) = &self.meta {
            r.num_pieces = m.picker.num_pieces();
            r.have = m.picker.have().as_bytes().to_vec();
            r.unfinished = m.picker.unfinished();
            r.part_slices = m.storage.part_slices().collect();
        }
        r
    }

    pub fn stats(&self, now: Millis) -> TorrentStats {
        let (n, have, left) = match &self.meta {
            Some(m) => (m.picker.num_pieces(), m.picker.have().count(), (m.picker.wanted_left() as u64 * m.storage.layout.piece_length as u64).min(m.storage.layout.total_length)),
            None => (0, 0, 0),
        };
        let dr = self.down.rate(now);
        TorrentStats {
            state: self.state, num_pieces: n, have_pieces: have,
            progress_ppm: if n == 0 { 0 } else { (have as u64 * 1_000_000 / n as u64) as u32 },
            left, download_rate: dr, upload_rate: self.up.rate(now), downloaded: self.downloaded, uploaded: self.uploaded, wasted: self.wasted,
            ratio_milli: if self.downloaded == 0 { 0 } else { self.uploaded.saturating_mul(1000) / self.downloaded },
            eta_secs: if left == 0 { Some(0) } else if dr == 0 { None } else { Some(left / dr) },
            peers: self.peers.len(), seeds: self.peers.values().filter(|s| s.conn.is_seed()).count(), known_peers: self.known.len(),
        }
    }
}

// ====================================================================== Session

fn first_queue(t: &mut Torrent, now: Millis) { let a = t.set_queued(true, now); t.out.extend(a); }

/// A torrent's key in a [`Session`]: its wire info-hash.
pub type TorrentKey = [u8; 20];

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub max_active_downloads: usize,
    pub max_active_seeds: usize,
}

impl Default for SessionConfig { fn default() -> Self { SessionConfig { max_active_downloads: 5, max_active_seeds: 10 } } }

/// An action and the torrent it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAction { pub torrent: TorrentKey, pub action: Action }

struct Entry { t: Torrent, pos: u64, wake: Option<Millis> }

/// Many torrents. Peer keys must be unique across the session (the program's
/// connection ids).
pub struct Session {
    pub cfg: SessionConfig,
    torrents: BTreeMap<TorrentKey, Entry>,
    alias: BTreeMap<[u8; 20], TorrentKey>,
    order: BTreeMap<u64, TorrentKey>,
    timers: BTreeSet<(Millis, TorrentKey)>,
    owner: BTreeMap<PeerKey, TorrentKey>,
    next_pos: u64,
    /// How many torrents run per queue, as of the last rebalance.
    running: [usize; 2],
}

impl Session {
    pub fn new(cfg: SessionConfig) -> Session {
        Session { cfg, torrents: BTreeMap::new(), alias: BTreeMap::new(), order: BTreeMap::new(), timers: BTreeSet::new(), owner: BTreeMap::new(), next_pos: 0, running: [0; 2] }
    }
    pub fn len(&self) -> usize { self.torrents.len() }
    pub fn is_empty(&self) -> bool { self.torrents.is_empty() }
    pub fn get(&self, key: &TorrentKey) -> Option<&Torrent> { self.torrents.get(key).map(|e| &e.t) }
    pub fn keys(&self) -> impl Iterator<Item = &TorrentKey> { self.torrents.keys() }
    /// When [`on_tick`](Self::on_tick) next has work.
    pub fn next_wakeup(&self) -> Option<Millis> { self.timers.first().map(|(t, _)| *t) }
    /// How many torrents have a timer (the rest are idle).
    pub fn scheduled(&self) -> usize { self.timers.len() }

    /// Adds a torrent at the end of the queue. Returns `None` if one with the same hash
    /// is already here.
    pub fn add(&mut self, mut t: Torrent, now: Millis) -> Option<Vec<SessionAction>> {
        let key = t.wire_hash();
        if self.torrents.contains_key(&key) { return None; }
        let first = t.take_actions();
        if let Some(a) = t.alt_hash() { self.alias.insert(a, key); }
        let pos = self.next_pos;
        self.next_pos += 1;
        self.order.insert(pos, key);
        // The newcomer is last in the queue: it runs only if its queue has room, which
        // keeps adding O(log n).
        if let Some(kind) = t.queue_kind() {
            let i = kind as usize;
            let limit = [self.cfg.max_active_downloads, self.cfg.max_active_seeds][i];
            if self.running[i] < limit { self.running[i] += 1; } else { first_queue(&mut t, now); }
        }
        let first = { let mut f = first; f.extend(t.take_actions()); f };
        self.torrents.insert(key, Entry { t, pos, wake: None });
        let out: Vec<SessionAction> = first.into_iter().map(|action| SessionAction { torrent: key, action }).collect();
        self.reschedule(&key);
        Some(out)
    }

    /// Removes a torrent, closing its connections.
    pub fn remove(&mut self, key: &TorrentKey, now: Millis) -> Vec<SessionAction> {
        let Some(mut e) = self.torrents.remove(key) else { return Vec::new() };
        if let Some(w) = e.wake { self.timers.remove(&(w, *key)); }
        self.order.remove(&e.pos);
        self.alias.retain(|_, v| v != key);
        let acts = e.t.pause(now);
        self.owner.retain(|_, v| v != key);
        let mut out: Vec<SessionAction> = acts.into_iter().filter(|a| matches!(a, Action::Close { .. })).map(|action| SessionAction { torrent: *key, action }).collect();
        out.extend(self.rebalance(now));
        out
    }

    /// Runs `f` on a torrent and keeps the session's indexes (peers, timers, queue) in
    /// step with what it did.
    pub fn with(&mut self, key: &TorrentKey, now: Millis, f: impl FnOnce(&mut Torrent, Millis) -> Vec<Action>) -> Vec<SessionAction> {
        let Some(e) = self.torrents.get_mut(key) else { return Vec::new() };
        let before = e.t.queue_kind();
        let acts = f(&mut e.t, now);
        let after = e.t.queue_kind();
        let mut out = self.post(key, acts);
        if before != after { out.extend(self.rebalance(now)); }
        self.reschedule(key);
        out
    }

    fn post(&mut self, key: &TorrentKey, acts: Vec<Action>) -> Vec<SessionAction> {
        for a in &acts { if let Action::Close { peer, .. } = a { self.owner.remove(peer); } }
        acts.into_iter().map(|action| SessionAction { torrent: *key, action }).collect()
    }

    fn reschedule(&mut self, key: &TorrentKey) {
        let Some(e) = self.torrents.get_mut(key) else { return };
        let w = e.t.next_wakeup();
        if w == e.wake { return; }
        if let Some(old) = e.wake { self.timers.remove(&(old, *key)); }
        if let Some(new) = w { self.timers.insert((new, *key)); }
        e.wake = w;
    }

    /// Applies the queue limits in queue order. O(torrents), run only when a torrent
    /// changes between downloading, seeding and inactive.
    fn rebalance(&mut self, now: Millis) -> Vec<SessionAction> {
        let (mut downloads, mut seeds) = (0usize, 0usize);
        let mut changes = Vec::new();
        for key in self.order.values() {
            let Some(e) = self.torrents.get(key) else { continue };
            let Some(kind) = e.t.queue_kind() else { continue };
            let slot = match kind { QueueKind::Download => &mut downloads, QueueKind::Seed => &mut seeds };
            let limit = match kind { QueueKind::Download => self.cfg.max_active_downloads, QueueKind::Seed => self.cfg.max_active_seeds };
            let run = *slot < limit;
            if run { *slot += 1; }
            if e.t.queued == run { changes.push((*key, !run)); }
        }
        self.running = [downloads, seeds];
        let mut out = Vec::new();
        for (key, queued) in changes {
            let Some(e) = self.torrents.get_mut(&key) else { continue };
            let acts = e.t.set_queued(queued, now);
            out.extend(self.post(&key, acts));
            self.reschedule(&key);
        }
        out
    }

    /// Moves a torrent in the queue (lower runs first).
    pub fn set_queue_position(&mut self, key: &TorrentKey, pos: u64, now: Millis) -> Vec<SessionAction> {
        let Some(old) = self.torrents.get(key).map(|e| e.pos) else { return Vec::new() };
        self.order.remove(&old);
        if self.order.contains_key(&pos) {
            // Shift the torrents at and after `pos` back by one (a rare, user-driven O(n)).
            let later: Vec<(u64, TorrentKey)> = self.order.range(pos..).map(|(p, k)| (*p, *k)).collect();
            for (p, _) in &later { self.order.remove(p); }
            for (p, k) in later { self.order.insert(p + 1, k); if let Some(e) = self.torrents.get_mut(&k) { e.pos = p + 1; } }
        }
        if let Some(e) = self.torrents.get_mut(key) { e.pos = pos; }
        self.order.insert(pos, *key);
        self.next_pos = self.order.last_key_value().map(|(p, _)| p + 1).unwrap_or(0);
        self.rebalance(now)
    }

    /// The torrent an incoming connection is for, from its first bytes. `Ok(None)`:
    /// fewer than 48 bytes so far, or no such torrent (`Err` when not BitTorrent).
    pub fn route(&self, first_bytes: &[u8]) -> Result<Option<TorrentKey>, WireError> {
        let Some(h) = peek_info_hash(first_bytes)? else { return Ok(None) };
        Ok(if self.torrents.contains_key(&h) { Some(h) } else { self.alias.get(&h).copied() })
    }

    /// An incoming connection routed to `key`; `first_bytes` are the bytes read so far.
    pub fn accept(&mut self, key: &TorrentKey, peer: PeerKey, addr: PeerAddr, first_bytes: &[u8], now: Millis) -> Vec<SessionAction> {
        if !self.torrents.contains_key(key) { return alloc::vec![SessionAction { torrent: *key, action: Action::Close { peer, reason: CloseReason::Inactive } }]; }
        self.owner.insert(peer, *key);
        let mut out = self.with(key, now, |t, now| t.add_peer(peer, addr, false, now));
        if self.owner.contains_key(&peer) { out.extend(self.with(key, now, |t, now| t.on_peer_bytes(peer, first_bytes, now))); }
        out
    }

    /// An outgoing connection (after [`Action::Connect`]) opened.
    pub fn connected(&mut self, key: &TorrentKey, peer: PeerKey, addr: PeerAddr, now: Millis) -> Vec<SessionAction> {
        self.owner.insert(peer, *key);
        self.with(key, now, |t, now| t.add_peer(peer, addr, true, now))
    }

    pub fn on_peer_bytes(&mut self, peer: PeerKey, bytes: &[u8], now: Millis) -> Vec<SessionAction> {
        let Some(key) = self.owner.get(&peer).copied() else { return Vec::new() };
        self.with(&key, now, |t, now| t.on_peer_bytes(peer, bytes, now))
    }

    pub fn peer_closed(&mut self, peer: PeerKey, now: Millis) -> Vec<SessionAction> {
        let Some(key) = self.owner.remove(&peer) else { return Vec::new() };
        self.with(&key, now, |t, now| t.peer_closed(peer, now))
    }

    /// Ticks the torrents that are due, and only those.
    pub fn on_tick(&mut self, now: Millis) -> Vec<SessionAction> {
        let mut out = Vec::new();
        while let Some(&(t, key)) = self.timers.first() {
            if t > now { break; }
            self.timers.remove(&(t, key));
            if let Some(e) = self.torrents.get_mut(&key) { e.wake = None; }
            out.extend(self.with(&key, now, |t, now| t.on_tick(now)));
            // A torrent that stays due (its clock did not move) waits for the next tick.
            if let Some(e) = self.torrents.get_mut(&key) {
                if e.wake.is_some_and(|w| w <= now) { let w = now + 1; if let Some(old) = e.wake { self.timers.remove(&(old, key)); } self.timers.insert((w, key)); e.wake = Some(w); }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InfoHashV1, Ip};
    use alloc::collections::VecDeque;
    use alloc::vec;

    fn payload(n: usize, seed: u64) -> Vec<u8> { let mut s = seed | 1; (0..n).map(|_| rand_step(&mut s) as u8).collect() }

    fn layout_of(sizes: &[u64], plen: u32) -> TorrentLayout {
        TorrentLayout::new(plen, sizes.iter().enumerate().map(|(i, s)| (vec![alloc::format!("f{i}")], *s, false)).collect(), false).unwrap()
    }

    fn split(data: &[u8], l: &TorrentLayout) -> Vec<Vec<u8>> { l.files.iter().map(|f| data[f.offset as usize..(f.offset + f.length) as usize].to_vec()).collect() }

    #[derive(Default)]
    struct Disk { files: Vec<Vec<u8>>, parts: BTreeMap<u32, Vec<u8>>, writes: Vec<DiskOp> }

    impl Disk {
        fn put(buf: &mut Vec<u8>, off: usize, d: &[u8]) { if buf.len() < off + d.len() { buf.resize(off + d.len(), 0); } buf[off..off + d.len()].copy_from_slice(d); }
        fn get(buf: &[u8], off: usize, len: usize) -> Vec<u8> { let mut v = vec![0; len]; for i in 0..len { v[i] = buf.get(off + i).copied().unwrap_or(0); } v }
        /// Runs an op; a read returns (id, at, bytes).
        fn run(&mut self, op: DiskOp) -> Option<(u64, u32, Vec<u8>)> {
            match &op {
                DiskOp::Write { file, offset, data } => { if self.files.len() <= *file { self.files.resize(file + 1, Vec::new()); } Disk::put(&mut self.files[*file], *offset as usize, data); }
                DiskOp::WritePart { piece, offset, data } => Disk::put(self.parts.entry(*piece).or_default(), *offset as usize, data),
                DiskOp::Read { id, file, offset, len, at } => return Some((*id, *at, Disk::get(self.files.get(*file).map(|f| &f[..]).unwrap_or(&[]), *offset as usize, *len as usize))),
                DiskOp::ReadPart { id, piece, offset, len, at } => return Some((*id, *at, Disk::get(self.parts.get(piece).map(|f| &f[..]).unwrap_or(&[]), *offset as usize, *len as usize))),
                DiskOp::Preallocate { file, len } => { if self.files.len() <= *file { self.files.resize(file + 1, Vec::new()); } self.files[*file].resize(*len as usize, 0); }
                DiskOp::PromotePart { piece, part_offset, file, file_offset, len } => {
                    let d = Disk::get(self.parts.get(piece).map(|f| &f[..]).unwrap_or(&[]), *part_offset as usize, *len as usize);
                    if self.files.len() <= *file { self.files.resize(file + 1, Vec::new()); }
                    Disk::put(&mut self.files[*file], *file_offset as usize, &d);
                }
                DiskOp::DiscardPart { piece } => { self.parts.remove(piece); }
            }
            self.writes.push(op);
            None
        }
    }

    struct Node { t: Torrent, disk: Disk, done: Vec<u32>, finished: bool, metadata: Option<Vec<u8>>, closed: Vec<CloseReason>, addr: PeerAddr }

    /// An in-memory network of torrents.
    struct Sim { nodes: Vec<Node>, links: BTreeMap<(usize, PeerKey), (usize, PeerKey)>, q: VecDeque<(usize, Action)>, now: Millis, next_key: PeerKey, budget: usize }

    impl Sim {
        fn new() -> Sim { Sim { nodes: Vec::new(), links: BTreeMap::new(), q: VecDeque::new(), now: 0, next_key: 1, budget: usize::MAX } }
        fn add(&mut self, mut t: Torrent, files: Vec<Vec<u8>>) -> usize {
            let i = self.nodes.len();
            for a in t.take_actions() { self.q.push_back((i, a)); }
            self.nodes.push(Node { t, disk: Disk { files, ..Disk::default() }, done: Vec::new(), finished: false, metadata: None, closed: Vec::new(), addr: PeerAddr { ip: Ip::V4([10, 0, 0, i as u8 + 1]), port: 6881 } });
            self.pump();
            i
        }
        fn push(&mut self, n: usize, acts: Vec<Action>) { for a in acts { self.q.push_back((n, a)); } }
        fn link(&mut self, a: usize, b: usize) {
            let (ka, kb) = (self.next_key, self.next_key + 1);
            self.next_key += 2;
            self.links.insert((a, ka), (b, kb));
            self.links.insert((b, kb), (a, ka));
            let (aa, ba) = (self.nodes[a].addr, self.nodes[b].addr);
            let x = self.nodes[a].t.add_peer(ka, ba, true, self.now);
            self.push(a, x);
            let y = self.nodes[b].t.add_peer(kb, aa, false, self.now);
            self.push(b, y);
            self.pump();
        }
        fn pump(&mut self) {
            let now = self.now;
            // `budget` actions per call model a finite link speed; the rest wait a step.
            let mut left = self.budget;
            while let Some((n, act)) = self.q.pop_front() {
                if left == 0 { self.q.push_front((n, act)); break; }
                left -= 1;
                match act {
                    Action::Send { peer, bytes } => {
                        if let Some(&(m, k)) = self.links.get(&(n, peer)) { let a = self.nodes[m].t.on_peer_bytes(k, &bytes, now); self.push(m, a); }
                    }
                    Action::Disk(op) => {
                        if let Some((id, at, data)) = self.nodes[n].disk.run(op) { let a = self.nodes[n].t.on_disk_read(id, at, &data, now); self.push(n, a); }
                    }
                    Action::Close { peer, reason } => {
                        self.nodes[n].closed.push(reason);
                        if let Some((m, k)) = self.links.remove(&(n, peer)) { self.links.remove(&(m, k)); let a = self.nodes[m].t.peer_closed(k, now); self.push(m, a); }
                    }
                    Action::Connect { addr } => {
                        let Some(m) = self.nodes.iter().position(|x| x.addr == addr) else { let a = self.nodes[n].t.connect_failed(addr, now); self.push(n, a); continue };
                        let (ka, kb) = (self.next_key, self.next_key + 1);
                        self.next_key += 2;
                        self.links.insert((n, ka), (m, kb));
                        self.links.insert((m, kb), (n, ka));
                        let from = self.nodes[n].addr;
                        let a = self.nodes[n].t.add_peer(ka, addr, true, now); self.push(n, a);
                        let b = self.nodes[m].t.add_peer(kb, from, false, now); self.push(m, b);
                    }
                    Action::Hash { piece, data } => {
                        let ok = !data.is_empty();
                        let a = self.nodes[n].t.on_piece_verified(piece, ok, now); self.push(n, a);
                    }
                    Action::PieceDone(p) => self.nodes[n].done.push(p),
                    Action::Finished => self.nodes[n].finished = true,
                    Action::MetadataReceived(info) => self.nodes[n].metadata = Some(info),
                    Action::PieceFailed(_) | Action::DhtNode(_) | Action::StateChanged(_) => {}
                }
            }
        }
        fn step(&mut self) {
            self.now += 100;
            for i in 0..self.nodes.len() { let a = self.nodes[i].t.on_tick(self.now); self.push(i, a); }
            self.pump();
        }
        fn run_until(&mut self, steps: usize, mut done: impl FnMut(&Sim) -> bool) -> bool {
            for _ in 0..steps { if done(self) { return true; } self.step(); }
            done(self)
        }
    }

    fn hashes(data: &[u8]) -> InfoHashes { let mut h = [0u8; 20]; h.copy_from_slice(&Sha1::digest(data)); InfoHashes::V1(InfoHashV1(h)) }

    fn params(ih: InfoHashes, id: u8, layout: &TorrentLayout, data: &[u8], seed: bool) -> TorrentParams {
        let mut p = TorrentParams::new(ih, PeerId([id; 20]));
        p.layout = Some(layout.clone());
        p.verifier = Some(Box::new(Sha1Pieces::of(data, layout.piece_length)));
        if seed { p.have = Some(Bitfield::full(layout.num_pieces())); }
        p.rng_seed = id as u64 * 7919;
        p
    }

    #[test]
    fn end_to_end_seed_to_leech() {
        let layout = layout_of(&[100_000, 70_000, 133_337], 32 * 1024);
        let data = payload(layout.total_length as usize, 42);
        let ih = hashes(b"e2e");
        let mut sim = Sim::new();
        let s = sim.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let l = sim.add(Torrent::new(params(ih, 2, &layout, &data, false), 0), Vec::new());
        assert_eq!(sim.nodes[s].t.state(), TorrentState::Seeding);
        assert_eq!(sim.nodes[l].t.state(), TorrentState::Downloading);
        let sa = sim.nodes[s].addr;
        let a = sim.nodes[l].t.add_peers(&[sa], 0);
        sim.push(l, a);
        sim.pump();
        assert!(sim.run_until(3000, |sim| sim.nodes[l].finished), "download did not finish");
        assert_eq!(sim.nodes[l].disk.files, split(&data, &layout), "payload byte-equal");
        assert_eq!(sim.nodes[l].t.state(), TorrentState::Seeding);
        assert_eq!(sim.nodes[l].done.len(), layout.num_pieces() as usize);
        let st = sim.nodes[l].t.stats(sim.now);
        assert_eq!((st.have_pieces, st.progress_ppm, st.left, st.wasted), (layout.num_pieces(), 1_000_000, 0, 0));
        assert_eq!(st.downloaded, layout.total_length);
        assert_eq!(sim.nodes[s].t.stats(sim.now).uploaded, layout.total_length);
        // Both are seeds now: the connection was closed as useless.
        assert!(sim.nodes[l].closed.contains(&CloseReason::NotUseful));
        assert!(sim.links.is_empty());
    }

    #[test]
    fn deselected_file_is_never_written() {
        let layout = layout_of(&[50_000, 60_000, 40_000], 32 * 1024);
        let data = payload(layout.total_length as usize, 7);
        let ih = hashes(b"desel");
        let mut sim = Sim::new();
        let s = sim.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let mut p = params(ih, 2, &layout, &data, false);
        p.file_priorities = vec![1, 0, 1];
        let l = sim.add(Torrent::new(p, 0), Vec::new());
        sim.link(l, s);
        assert!(sim.run_until(3000, |sim| sim.nodes[l].finished));
        let n = &sim.nodes[l];
        assert!(!n.disk.writes.iter().any(|w| matches!(w, DiskOp::Write { file: 1, .. } | DiskOp::Preallocate { file: 1, .. })), "file 1 was written");
        assert!(n.disk.writes.iter().any(|w| matches!(w, DiskOp::WritePart { .. })), "straddling pieces use the part region");
        let want = split(&data, &layout);
        assert_eq!(n.disk.files[0], want[0]);
        assert_eq!(n.disk.files[2], want[2]);
        // Pieces wholly inside file 1 were not downloaded.
        let (a, b) = layout.file_pieces(1).unwrap();
        let inner: Vec<u32> = (a + 1..b).collect();
        assert!(!inner.is_empty() && inner.iter().all(|p| !n.done.contains(p)));
        assert_eq!(n.t.state(), TorrentState::Seeding, "finished what was selected");
        // Selecting the file later promotes the part bytes and fetches the rest.
        let link_again = sim.nodes[l].t.set_file_priority(1, 1, sim.now);
        assert_eq!(sim.nodes[l].t.state(), TorrentState::Downloading);
        assert!(link_again.iter().any(|a| matches!(a, Action::Disk(DiskOp::PromotePart { file: 1, .. }))));
        sim.push(l, link_again);
        sim.nodes[l].finished = false;
        sim.pump();
        if sim.links.is_empty() { sim.link(l, s); }
        assert!(sim.run_until(3000, |sim| sim.nodes[l].finished));
        assert_eq!(sim.nodes[l].disk.files, want);
    }

    #[test]
    fn streaming_deadlines_arrive_first() {
        let layout = TorrentLayout::single("movie.mkv", 64 * 16384, 16384).unwrap();
        let data = payload(layout.total_length as usize, 99);
        let ih = hashes(b"stream");
        let mut sim = Sim::new();
        let s = sim.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let l = sim.add(Torrent::new(params(ih, 2, &layout, &data, false), 0), Vec::new());
        let ph = Playhead { first: 0, last: 63, header: 2, footer: 2, position: 30, window: 8, step: 1000 };
        let a = sim.nodes[l].t.set_playhead(ph, 0);
        sim.push(l, a);
        sim.link(l, s);
        assert!(sim.run_until(3000, |sim| sim.nodes[l].finished));
        let first: BTreeSet<u32> = sim.nodes[l].done[..12].iter().copied().collect();
        let want: BTreeSet<u32> = [0, 1, 62, 63].into_iter().chain(30..38).collect();
        assert_eq!(first, want, "header, footer and window first: {:?}", sim.nodes[l].done);
        assert_eq!(sim.nodes[l].disk.files, split(&data, &layout));
    }

    #[test]
    fn seek_moves_the_window() {
        let layout = TorrentLayout::single("movie.mkv", 200 * 16384, 16384).unwrap();
        let data = payload(layout.total_length as usize, 5);
        let ih = hashes(b"seek");
        let mut sim = Sim::new();
        let s = sim.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let l = sim.add(Torrent::new(params(ih, 2, &layout, &data, false), 0), Vec::new());
        let a = sim.nodes[l].t.set_playhead(Playhead { first: 0, last: 199, header: 1, footer: 1, position: 10, window: 4, step: 500 }, 0);
        sim.push(l, a);
        sim.link(l, s);
        assert!(sim.run_until(500, |sim| sim.nodes[l].done.len() >= 6));
        let a = sim.nodes[l].t.seek(150, sim.now);
        sim.push(l, a);
        sim.pump();
        let mark = sim.nodes[l].done.len();
        assert!(sim.run_until(3000, |sim| (150..154).all(|p| sim.nodes[l].done.contains(&p))));
        // The new window came within the next few pieces (those already requested drain first).
        let pos: Vec<usize> = (150..154).map(|p| sim.nodes[l].done.iter().position(|x| *x == p).unwrap()).collect();
        assert!(pos.iter().all(|i| *i < mark + 40), "window after seek too late: {pos:?} mark {mark}");
    }

    #[test]
    fn magnet_link_fetches_metadata_then_payload() {
        let layout = layout_of(&[90_000], 16384);
        let data = payload(layout.total_length as usize, 3);
        // A stand-in info dictionary: the hash names it, the program parses it.
        let mut info = b"d6:lengthi90000e4:name2:f012:piece lengthi16384e6:pieces".to_vec();
        let pieces = Sha1Pieces::of(&data, 16384);
        info.extend(alloc::format!("{}:", pieces.0.len() * 20).as_bytes());
        for h in &pieces.0 { info.extend_from_slice(h); }
        info.extend(vec![b'e']);
        let ih = hashes(&info);
        let mut sp = params(ih, 1, &layout, &data, true);
        sp.info = Some(info.clone());
        let mut sim = Sim::new();
        let s = sim.add(Torrent::new(sp, 0), split(&data, &layout));
        let l = sim.add(Torrent::new(TorrentParams { rng_seed: 3, ..TorrentParams::new(ih, PeerId([2; 20])) }, 0), Vec::new());
        assert_eq!(sim.nodes[l].t.state(), TorrentState::DownloadingMetadata);
        sim.link(l, s);
        assert!(sim.run_until(500, |sim| sim.nodes[l].metadata.is_some()));
        assert_eq!(sim.nodes[l].metadata.as_deref(), Some(&info[..]));
        let a = sim.nodes[l].t.set_metadata(layout.clone(), Some(Box::new(pieces)), sim.now).unwrap();
        sim.push(l, a);
        sim.pump();
        assert!(sim.run_until(2000, |sim| sim.nodes[l].finished));
        assert_eq!(sim.nodes[l].disk.files, split(&data, &layout));
        // The leech can now serve the metadata too.
        assert_eq!(sim.nodes[l].t.resume_data().info, Some(info));
    }

    #[test]
    fn two_seeds_one_bad_and_external_hashing() {
        let layout = layout_of(&[300_000], 16384);
        let data = payload(layout.total_length as usize, 11);
        let ih = hashes(b"bad");
        let mut sim = Sim::new();
        let good = sim.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let mut evil = split(&data, &layout);
        for b in evil[0].iter_mut() { *b ^= 0x55; }
        let bad = sim.add(Torrent::new(params(ih, 3, &layout, &data, true), 0), evil);
        let mut lp = params(ih, 2, &layout, &data, false);
        lp.config.hashfail_ban = 1;
        let l = sim.add(Torrent::new(lp, 0), Vec::new());
        sim.link(l, bad);
        sim.link(l, good);
        assert!(sim.run_until(5000, |sim| sim.nodes[l].finished));
        assert_eq!(sim.nodes[l].disk.files, split(&data, &layout));
        assert!(sim.nodes[l].closed.contains(&CloseReason::Banned));
        assert!(sim.nodes[l].t.stats(sim.now).wasted > 0);
        // Without a verifier, the program hashes (the Sim passes everything).
        let mut ep = params(ih, 4, &layout, &data, false);
        ep.verifier = None;
        let e = sim.add(Torrent::new(ep, sim.now), Vec::new());
        sim.link(e, good);
        assert!(sim.run_until(5000, |sim| sim.nodes[e].finished));
        assert_eq!(sim.nodes[e].disk.files, split(&data, &layout));
    }

    #[test]
    fn checking_existing_files() {
        let layout = layout_of(&[40_000, 30_000], 16384);
        let data = payload(layout.total_length as usize, 13);
        let ih = hashes(b"check");
        let mut files = split(&data, &layout);
        files[1][5] ^= 1; // piece 2 or 3 is damaged
        let mut p = params(ih, 1, &layout, &data, false);
        p.check_files = true;
        let mut sim = Sim::new();
        let n = sim.add(Torrent::new(p, 0), files);
        let have = sim.nodes[n].t.have().unwrap().clone();
        let bad = ((40_000 + 5) / 16384) as u32;
        assert!(!have.get(bad));
        assert_eq!(have.count(), layout.num_pieces() - 1);
        assert_eq!(sim.nodes[n].t.state(), TorrentState::Downloading);
        // Missing files: reads fail, nothing is had.
        let mut p = params(ih, 1, &layout, &data, false);
        p.check_files = true;
        let mut t = Torrent::new(p, 0);
        assert_eq!(t.state(), TorrentState::Checking);
        let mut acts = t.take_actions();
        for _ in 0..20 {
            let ids: Vec<u64> = acts.iter().filter_map(|a| if let Action::Disk(DiskOp::Read { id, .. }) = a { Some(*id) } else { None }).collect();
            if ids.is_empty() { break; }
            acts = ids.into_iter().flat_map(|id| t.on_disk_error(id, 0)).collect();
        }
        assert_eq!(t.state(), TorrentState::Downloading);
        assert_eq!(t.have().unwrap().count(), 0);
    }

    #[test]
    fn resume_round_trip_keeps_progress() {
        let layout = layout_of(&[20 * 16384], 2 * 16384);
        let data = payload(layout.total_length as usize, 17);
        let ih = hashes(b"resume");
        let mut sim = Sim::new();
        sim.budget = 40;
        let s = sim.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let l = sim.add(Torrent::new(params(ih, 2, &layout, &data, false), 0), Vec::new());
        sim.link(l, s);
        assert!(sim.run_until(1000, |sim| sim.nodes[l].done.len() >= 3));
        let a = sim.nodes[l].t.pause(sim.now);
        sim.push(l, a);
        sim.budget = usize::MAX;
        sim.pump();
        let a = sim.nodes[l].t.flush_cache(sim.now);
        sim.push(l, a);
        sim.pump();
        let r = ResumeData::decode(&sim.nodes[l].t.resume_data().encode()).unwrap();
        let had = sim.nodes[l].t.have().unwrap().clone();
        assert!(had.count() < layout.num_pieces());
        assert!(r.paused);
        let mut r = r;
        r.paused = false;
        let mut p = params(ih, 2, &layout, &data, false);
        p.resume = Some(r.clone());
        let disk = core::mem::take(&mut sim.nodes[l].disk);
        let t = Torrent::new(p, sim.now);
        assert_eq!(t.have().unwrap(), &had);
        assert_eq!(t.resume_data().unfinished, r.unfinished);
        // The restarted torrent finishes from where it was.
        let mut sim2 = Sim::new();
        let s2 = sim2.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let l2 = sim2.add(t, Vec::new());
        sim2.nodes[l2].disk = disk;
        sim2.link(l2, s2);
        assert!(sim2.run_until(2000, |sim| sim.nodes[l2].finished));
        assert_eq!(sim2.nodes[l2].disk.files, split(&data, &layout));
        assert!(sim2.nodes[l2].done.len() < layout.num_pieces() as usize);
    }

    #[test]
    fn unchoke_slots_and_tit_for_tat() {
        let layout = layout_of(&[64 * 16384], 16384);
        let data = payload(layout.total_length as usize, 23);
        let ih = hashes(b"choke");
        let mut sim = Sim::new();
        sim.budget = 60;
        let s = sim.add(Torrent::new(params(ih, 1, &layout, &data, true), 0), split(&data, &layout));
        let leeches: Vec<usize> = (0..7).map(|i| sim.add(Torrent::new(params(ih, 10 + i, &layout, &data, false), 0), Vec::new())).collect();
        for &l in &leeches { sim.link(l, s); }
        sim.step();
        let unchoked = sim.nodes[s].t.peers.values().filter(|p| !p.conn.am_choking).count();
        assert!(unchoked <= 5, "4 regular + 1 optimistic, got {unchoked}");
        assert!(unchoked >= 4);
        // Everyone gets the payload eventually (optimistic rotation + leeches trading).
        assert!(sim.run_until(20_000, |sim| leeches.iter().all(|l| sim.nodes[*l].finished)));
    }

    #[test]
    fn hostile_bytes_and_self_connections() {
        let layout = layout_of(&[100_000], 16384);
        let data = payload(100_000, 1);
        let ih = hashes(b"hostile");
        let mut t = Torrent::new(params(ih, 1, &layout, &data, true), 0);
        let mut s = 0x1234_5678_9abc_def0u64;
        for k in 0..300u32 {
            t.add_peer(k, PeerAddr { ip: Ip::V4([1, 1, 1, (k % 250) as u8]), port: 1 }, false, 0);
            let mut bytes = Handshake { reserved: Reserved::new(true, k % 2 == 0, true, true), info_hash: ih.wire(), peer_id: [(k as u8).wrapping_add(9); 20] }.encode().to_vec();
            for _ in 0..(rand_step(&mut s) % 300) { bytes.push(rand_step(&mut s) as u8); }
            // Sometimes a real message prefix with garbage.
            if k % 3 == 0 { bytes.extend_from_slice(&[0, 0, 0, 13, 6]); bytes.extend((0..12).map(|_| rand_step(&mut s) as u8)); }
            let _ = t.on_peer_bytes(k, &bytes, k as u64);
            let _ = t.on_tick(k as u64 * 1000);
        }
        // Our own peer ID is refused.
        let mut t = Torrent::new(params(ih, 1, &layout, &data, true), 0);
        t.add_peer(1, PeerAddr { ip: Ip::V4([1, 1, 1, 1]), port: 1 }, false, 0);
        let a = t.on_peer_bytes(1, &Handshake { reserved: Reserved::default(), info_hash: ih.wire(), peer_id: [1; 20] }.encode(), 0);
        assert!(a.contains(&Action::Close { peer: 1, reason: CloseReason::SelfConnection }));
        // Too many peers.
        let mut p = params(ih, 1, &layout, &data, true);
        p.config.max_connections = 1;
        let mut t = Torrent::new(p, 0);
        t.add_peer(1, PeerAddr { ip: Ip::V4([1, 1, 1, 1]), port: 1 }, false, 0);
        assert!(t.add_peer(2, PeerAddr { ip: Ip::V4([1, 1, 1, 2]), port: 1 }, false, 0).contains(&Action::Close { peer: 2, reason: CloseReason::TooManyPeers }));
    }

    fn idle_torrent(i: u32, seed: bool) -> Torrent {
        let layout = layout_of(&[16384], 16384);
        let mut h = [0u8; 20];
        h[..4].copy_from_slice(&i.to_be_bytes());
        let mut p = TorrentParams::new(InfoHashes::V1(InfoHashV1(h)), PeerId([9; 20]));
        p.layout = Some(layout.clone());
        if seed { p.have = Some(Bitfield::full(1)); }
        Torrent::new(p, 0)
    }

    #[test]
    fn session_scales_and_queues() {
        let mut s = Session::new(SessionConfig { max_active_downloads: 3, max_active_seeds: 10_000 });
        for i in 0..10_000u32 { s.add(idle_torrent(i, i % 2 == 0), 0).unwrap(); }
        assert_eq!(s.len(), 10_000);
        assert!(s.add(idle_torrent(5, false), 0).is_none(), "duplicate");
        // Idle torrents have no timers: a tick touches nothing.
        assert_eq!(s.scheduled(), 0);
        assert!(s.on_tick(1_000).is_empty());
        let states = |s: &Session, st: TorrentState| s.keys().filter(|k| s.get(k).unwrap().state() == st).count();
        assert_eq!(states(&s, TorrentState::Downloading), 3);
        assert_eq!(states(&s, TorrentState::Queued), 5_000 - 3);
        assert_eq!(states(&s, TorrentState::Seeding), 5_000);
        // Pausing an active download lets the next one in.
        let first: TorrentKey = { let mut h = [0u8; 20]; h[..4].copy_from_slice(&1u32.to_be_bytes()); h };
        assert_eq!(s.get(&first).unwrap().state(), TorrentState::Downloading);
        let out = s.with(&first, 2_000, |t, now| t.pause(now));
        assert!(out.iter().any(|a| a.action == Action::StateChanged(TorrentState::Paused)));
        assert_eq!(states(&s, TorrentState::Downloading), 3);
        assert_eq!(states(&s, TorrentState::Paused), 1);
        // Moving a queued torrent to the front makes it active.
        let last: TorrentKey = { let mut h = [0u8; 20]; h[..4].copy_from_slice(&9_999u32.to_be_bytes()); h };
        assert_eq!(s.get(&last).unwrap().state(), TorrentState::Queued);
        s.set_queue_position(&last, 0, 3_000);
        assert_eq!(s.get(&last).unwrap().state(), TorrentState::Downloading);
        // Peers make a torrent tick; routing finds it by info-hash.
        let a = s.with(&last, 3_000, |t, now| t.add_peers(&[PeerAddr { ip: Ip::V4([1, 2, 3, 4]), port: 5 }], now));
        assert!(a.iter().any(|x| matches!(x.action, Action::Connect { .. })));
        assert_eq!(s.scheduled(), 1);
        let hs = Handshake { reserved: Reserved::default(), info_hash: last, peer_id: [3; 20] }.encode();
        assert_eq!(s.route(&hs[..30]), Ok(None));
        assert_eq!(s.route(&hs), Ok(Some(last)));
        assert!(s.route(b"\x13BitTorrent protocol\0\0\0\0\0\0\0\0\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff").unwrap().is_none());
        let acts = s.accept(&last, 77, PeerAddr { ip: Ip::V4([5, 5, 5, 5]), port: 9 }, &hs, 3_100);
        assert!(acts.iter().any(|a| matches!(a.action, Action::Send { peer: 77, .. })));
        assert!(s.on_tick(4_200).iter().all(|a| a.torrent == last));
        let gone = s.remove(&last, 5_000);
        assert!(gone.iter().any(|a| a.action == Action::Close { peer: 77, reason: CloseReason::Inactive }));
        assert!(s.on_peer_bytes(77, b"x", 5_000).is_empty());
        assert_eq!(states(&s, TorrentState::Downloading), 3, "the slot was refilled");
    }
}
