// SPDX-License-Identifier: GPL-3.0-only
//! The event loop: sockets for peers (TCP, non-blocking), connects and tracker requests
//! on short-lived threads that report back over channels, and the disk. Everything the
//! engine asks for (`SessionAction`) is carried out here, in order.
use crate::disk::{Disk, ReadDone};
use crate::say;
use sha2::{Digest, Sha256};
use sex_torrent::metainfo::Metainfo;
use sex_torrent::mse::{req2_hash, Cipher, Handshake, Policy, Progress};
use sex_torrent::session::{Action, Session, SessionAction, SessionConfig, Torrent, TorrentKey, TorrentParams, TorrentState};
use sex_torrent::tracker::{announce_request, parse_announce, AnnounceParams, Event as AnnounceEvent};
use sex_torrent::{Ip, Millis, PeerAddr, PeerId};
use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

pub fn to_peer(a: SocketAddr) -> PeerAddr {
    let ip = match a.ip() { IpAddr::V4(v) => Ip::V4(v.octets()), IpAddr::V6(v) => Ip::V6(v.octets()) };
    PeerAddr { ip, port: a.port() }
}
fn to_socket(a: PeerAddr) -> SocketAddr {
    match a.ip { Ip::V4(v) => SocketAddr::from((v, a.port)), Ip::V6(v) => SocketAddr::from((v, a.port)) }
}

/// A peer connection. `torrent` is unknown for an incoming one until its handshake names it.
struct Conn { stream: TcpStream, addr: PeerAddr, torrent: Option<TorrentKey>, first: Vec<u8>, out: VecDeque<u8>, mse: Option<Handshake>, cipher: Option<Cipher> }

/// What a helper thread found.
enum Done {
    Connected { peer: u32, torrent: TorrentKey, addr: PeerAddr, stream: Option<TcpStream> },
    Announced { torrent: TorrentKey, url: String, peers: Vec<PeerAddr>, interval: u64, error: Option<String> },
}

pub(crate) struct Job { pub trackers: Vec<String>, pub disk: Option<Disk>, pub next_announce: Millis, pub announced_done: bool, pub finished_at: Option<Millis>, pub name: String, pub id: u64, pub added: i64, pub dir: PathBuf, pub paused: bool, pub magnet: String }

pub struct Client {
    pub(crate) session: Session,
    pub(crate) jobs: HashMap<TorrentKey, Job>,
    pub(crate) rpc: Option<crate::remote::Rpc>,
    pub(crate) next_id: u64,
    conns: HashMap<u32, Conn>,
    listener: Option<TcpListener>,
    port: u16,
    pub(crate) dir: PathBuf,
    peer_id: PeerId,
    anonymous: bool,
    next_peer: u32,
    pub(crate) started: Instant,
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
    pub(crate) queue: VecDeque<SessionAction>,
    key: u32,
}

const MAX_CONNECTING: usize = 64;

impl Client {
    pub fn new(port: u16, dir: PathBuf, peer_id: PeerId, anonymous: bool) -> Client {
        let listener = if anonymous { None } else {
            TcpListener::bind(("0.0.0.0", port)).ok().inspect(|l| { let _ = l.set_nonblocking(true); })
        };
        if listener.is_none() && !anonymous { say(&format!("[swingers.note] cannot listen on port {port}; outgoing connections only")); }
        let (done_tx, done_rx) = channel();
        let mut k = [0u8; 4];
        crate::random(&mut k);
        Client { session: Session::new(SessionConfig::default()), jobs: HashMap::new(), rpc: None, next_id: 1, conns: HashMap::new(), listener, port, dir, peer_id, anonymous,
            next_peer: 1, started: Instant::now(), done_tx, done_rx, queue: VecDeque::new(), key: u32::from_le_bytes(k) }
    }

    /// Serves Transmission's RPC and the web UI on `port` (daemon mode).
    pub fn listen_rpc(&mut self, port: u16, user: &str, password: &str) -> Result<(), String> {
        self.rpc = Some(crate::remote::Rpc::new(port, user, password, &self.dir.display().to_string(), self.port)?);
        Ok(())
    }

    pub(crate) fn now(&self) -> Millis { self.started.elapsed().as_millis() as Millis }

    /// Adds a torrent from a magnet link, info-hash or .torrent. Returns its RPC id, key
    /// and whether it was already here.
    pub fn add_source(&mut self, source: &str, bytes: Option<Vec<u8>>, extra_peers: Vec<PeerAddr>, dir: Option<PathBuf>) -> Result<(u64, TorrentKey, bool), String> {
        let prep = crate::prepare(source, bytes, self.peer_id, self.port, self.anonymous)?;
        let key = prep.params.info_hashes.wire();
        if let Some(j) = self.jobs.get(&key) { return Ok((j.id, key, true)); }
        let magnet = prep.meta.as_ref().map(|m| sex_torrent::magnet::Magnet::from_metainfo(m).to_uri()).unwrap_or_else(|| format!("magnet:?xt=urn:btih:{}", sex_torrent::to_hex(&key)));
        say(&format!("[swingers.added] {} {}", sex_torrent::to_hex(&key), prep.name));
        let mut peers = prep.peers;
        peers.extend(extra_peers);
        self.add(prep.params, prep.name, prep.trackers, peers, dir.unwrap_or_else(|| self.dir.clone()), magnet);
        Ok((self.jobs[&key].id, key, false))
    }

    fn add(&mut self, params: TorrentParams, name: String, trackers: Vec<String>, peers: Vec<PeerAddr>, dir: PathBuf, magnet: String) {
        let now = self.now();
        let key = params.info_hashes.wire();
        let disk = params.layout.clone().map(|l| Disk::new(&dir, l, &sex_torrent::to_hex(&key)));
        let t = Torrent::new(params, now);
        let id = self.next_id; self.next_id += 1;
        let added = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.jobs.insert(key, Job { trackers, disk, next_announce: 0, announced_done: false, finished_at: None, name, id, added, dir, paused: false, magnet });
        if let Some(a) = self.session.add(t, now) { self.queue.extend(a); }
        let more = self.session.with(&key, now, |t, now| t.add_peers(&peers, now));
        self.queue.extend(more);
    }

    /// Runs until every torrent is done and has seeded `seed_secs`.
    /// `seed_secs: None` runs until stopped (daemon mode).
    pub fn run(&mut self, seed_secs: Option<u64>) {
        let mut last_tick = 0;
        let mut last_report = 0;
        loop {
            let now = self.now();
            self.drain();
            let mut busy = self.accept(now);
            busy |= self.pump(now);
            while let Ok(d) = self.done_rx.try_recv() { self.done(d); busy = true; }
            busy |= crate::remote::pump(self);
            if now >= last_tick + 50 { last_tick = now; let a = self.session.on_tick(now); self.queue.extend(a); self.announce(now); }
            if now >= last_report + 2000 { last_report = now; self.report(now); }
            self.drain();
            if self.check_finished(now, seed_secs.unwrap_or(0)) && seed_secs.is_some() { break; }
            if !busy { std::thread::sleep(Duration::from_millis(2)); }
        }
        let now = self.now();
        let keys: Vec<TorrentKey> = self.jobs.keys().copied().collect();
        for k in keys { self.stop_announce(&k, now); }
    }

    fn report(&mut self, now: Millis) {
        for (k, _) in self.jobs.iter() {
            if let Some(t) = self.session.get(k) {
                let s = t.stats(now);
                say(&format!("[swingers.progress] {} {}.{}% down={}B/s up={}B/s peers={} state={:?}", sex_torrent::to_hex(k), s.progress_ppm / 10_000, (s.progress_ppm / 1000) % 10, s.download_rate, s.upload_rate, t.num_peers(), s.state));
            }
        }
    }

    fn check_finished(&mut self, now: Millis, seed_secs: u64) -> bool {
        let mut all = !self.jobs.is_empty();
        let keys: Vec<TorrentKey> = self.jobs.keys().copied().collect();
        for k in keys {
            let finished = self.session.get(&k).is_some_and(|t| t.is_finished() && t.state() != TorrentState::Checking);
            let job = self.jobs.get_mut(&k).expect("job");
            if finished && job.finished_at.is_none() {
                job.finished_at = Some(now);
                let a = self.session.with(&k, now, |t, now| t.flush_cache(now));
                self.queue.extend(a);
                self.drain();
                let daemon = self.rpc.is_some();
                let job = self.jobs.get_mut(&k).expect("job");
                if let Some(d) = job.disk.as_mut() {
                    d.close();
                    // The gates check every file; a daemon's files can be huge, so it skips this.
                    for (i, f) in d.layout().files.clone().iter().enumerate() {
                        if f.pad || daemon { continue; }
                        let path = d.path_of(i).expect("file");
                        let sum = std::fs::read(&path).map(|b| sex_torrent::to_hex(&Sha256::digest(&b))).unwrap_or_else(|e| format!("unreadable:{e}"));
                        say(&format!("[swingers.file] {} {}", path.display(), sum));
                    }
                }
                say(&format!("[swingers.done] {} {}", sex_torrent::to_hex(&k), job.name));
                job.next_announce = 0; // tell trackers we completed
            }
            let job = &self.jobs[&k];
            all &= job.finished_at.is_some_and(|t| now >= t + seed_secs * 1000);
        }
        all
    }

    /// Carries out queued engine actions.
    fn drain(&mut self) {
        while let Some(SessionAction { torrent, action }) = self.queue.pop_front() {
            let now = self.now();
            match action {
                Action::Send { peer, mut bytes } => if let Some(c) = self.conns.get_mut(&peer) { if let Some(k) = c.cipher.as_mut() { k.encrypt(&mut bytes); } c.out.extend(bytes); },
                Action::Close { peer, reason } => { if let Some(c) = self.conns.remove(&peer) { if std::env::var_os("SWINGERS_DEBUG").is_some() { say(&format!("[swingers.debug] close {:?} {:?}", c.addr, reason)); } } }
                Action::Connect { addr } => {
                    if self.conns.len() >= MAX_CONNECTING * 4 { let a = self.session.with(&torrent, now, |t, now| t.connect_failed(addr, now)); self.queue.extend(a); continue; }
                    let peer = self.next_peer; self.next_peer += 1;
                    let tx = self.done_tx.clone();
                    std::thread::spawn(move || {
                        let stream = TcpStream::connect_timeout(&to_socket(addr), Duration::from_secs(8)).ok();
                        let _ = tx.send(Done::Connected { peer, torrent, addr, stream });
                    });
                }
                Action::Disk(op) => {
                    let Some(job) = self.jobs.get_mut(&torrent) else { continue };
                    let Some(disk) = job.disk.as_mut() else { continue };
                    match disk.run(op) {
                        Ok(Some(ReadDone { id, at, data: Ok(d) })) => { let a = self.session.with(&torrent, now, |t, now| t.on_disk_read(id, at, &d, now)); self.queue.extend(a); }
                        Ok(Some(ReadDone { id, data: Err(()), .. })) => { let a = self.session.with(&torrent, now, |t, now| t.on_disk_error(id, now)); self.queue.extend(a); }
                        Ok(None) => {}
                        Err(e) => { say(&format!("[swingers.failed] disk: {e}")); let a = self.session.with(&torrent, now, |t, now| t.on_write_error(now)); self.queue.extend(a); }
                    }
                }
                Action::Hash { piece, data } => { let _ = (piece, data); }
                Action::MetadataReceived(info) => self.metadata(torrent, &info, now),
                Action::Finished | Action::PieceDone(_) | Action::PieceFailed(_) | Action::DhtNode(_) => {}
                Action::StateChanged(s) => say(&format!("[swingers.state] {} {:?}", sex_torrent::to_hex(&torrent), s)),
            }
        }
    }

    fn metadata(&mut self, torrent: TorrentKey, info: &[u8], now: Millis) {
        match Metainfo::from_info_bytes(info) {
            Ok(m) => match m.layout() {
                Ok(l) => {
                    if let Some(job) = self.jobs.get_mut(&torrent) {
                        job.name = m.name.clone();
                        job.disk = Some(Disk::new(&job.dir.clone(), l.clone(), &sex_torrent::to_hex(&torrent)));
                    }
                    say(&format!("[swingers.metadata] {} {} files={}", sex_torrent::to_hex(&torrent), m.name, m.files.iter().filter(|f| !f.attr.pad).count()));
                    let v = m.verifier();
                    let a = self.session.with(&torrent, now, move |t, now| t.set_metadata(l, Some(v), now).unwrap_or_default());
                    self.queue.extend(a);
                }
                Err(e) => say(&format!("[swingers.failed] metadata layout: {e:?}")),
            },
            Err(e) => say(&format!("[swingers.failed] metadata: {e:?}")),
        }
    }

    fn done(&mut self, d: Done) {
        let now = self.now();
        match d {
            Done::Connected { peer, torrent, addr, stream: Some(s) } => {
                let _ = s.set_nonblocking(true);
                let _ = s.set_nodelay(true);
                self.conns.insert(peer, Conn { stream: s, addr, torrent: Some(torrent), first: Vec::new(), out: VecDeque::new(), mse: None, cipher: None });
                let a = self.session.connected(&torrent, peer, addr, now);
                self.queue.extend(a);
            }
            Done::Connected { torrent, addr, stream: None, .. } => { let a = self.session.with(&torrent, now, |t, now| t.connect_failed(addr, now)); self.queue.extend(a); }
            Done::Announced { torrent, url, peers, interval, error } => {
                if let Some(e) = error { say(&format!("[swingers.tracker] {url} failed: {e}")); }
                else { say(&format!("[swingers.tracker] {url} peers={}", peers.len())); }
                if let Some(job) = self.jobs.get_mut(&torrent) { job.next_announce = now + interval.clamp(60, 3600) * 1000; }
                let a = self.session.with(&torrent, now, |t, now| t.add_peers(&peers, now));
                self.queue.extend(a);
            }
        }
    }

    fn accept(&mut self, _now: Millis) -> bool {
        let Some(l) = self.listener.as_ref() else { return false };
        let mut any = false;
        while let Ok((s, a)) = l.accept() {
            any = true;
            if self.conns.len() >= 500 { continue; }
            let _ = s.set_nonblocking(true);
            let peer = self.next_peer; self.next_peer += 1;
            let mse = Some(Handshake::responder(Policy::PreferEncryption, &mut |b: &mut [u8]| crate::random(b)));
            self.conns.insert(peer, Conn { stream: s, addr: to_peer(a), torrent: None, first: Vec::new(), out: VecDeque::new(), mse, cipher: None });
        }
        any
    }

    /// Moves bytes between sockets and the engine. Returns whether anything moved.
    fn pump(&mut self, now: Millis) -> bool {
        let mut any = false;
        let mut closed = Vec::new();
        let mut actions = Vec::new();
        let peers: Vec<u32> = self.conns.keys().copied().collect();
        let mut buf = vec![0u8; 64 * 1024];
        let req2: HashMap<[u8; 20], [u8; 20]> = self.jobs.keys().map(|k| (req2_hash(k), *k)).collect();
        for peer in peers {
            let c = self.conns.get_mut(&peer).expect("conn");
            // Write what is queued.
            while !c.out.is_empty() {
                let (a, _) = c.out.as_slices();
                match c.stream.write(a) {
                    Ok(0) => { closed.push(peer); break; }
                    Ok(n) => { c.out.drain(..n); any = true; }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => { closed.push(peer); break; }
                }
            }
            // Read what arrived (bounded per round so one peer cannot starve the rest).
            for _ in 0..8 {
                if c.out.len() > 4 << 20 { break; }
                match c.stream.read(&mut buf) {
                    Ok(0) => { closed.push(peer); break; }
                    Ok(n) => {
                        any = true;
                        // Protocol encryption (or a plain handshake) on incoming connections.
                        let mut data = buf[..n].to_vec();
                        if let Some(h) = c.mse.as_mut() {
                            match h.feed(&data, &mut |r: &[u8; 20]| req2.get(r).copied()) {
                                Ok(Progress::NeedMore) => { c.out.extend(h.take_output()); continue; }
                                Ok(Progress::Done(est)) => { c.out.extend(h.take_output()); c.mse = None; c.cipher = Some(est.cipher); data = est.payload; }
                                Err(e) => { if std::env::var_os("SWINGERS_DEBUG").is_some() { say(&format!("[swingers.debug] mse {:?} {e:?}", c.addr)); } closed.push(peer); break; }
                            }
                        } else if let Some(k) = c.cipher.as_mut() { k.decrypt(&mut data); }
                        if data.is_empty() { continue; }
                        match c.torrent {
                            Some(_) => actions.extend(self.session.on_peer_bytes(peer, &data, now)),
                            None => {
                                c.first.extend_from_slice(&data);
                                match self.session.route(&c.first) {
                                    Ok(Some(k)) => { c.torrent = Some(k); let first = std::mem::take(&mut c.first); actions.extend(self.session.accept(&k, peer, c.addr, &first, now)); }
                                    Ok(None) if c.first.len() < 68 => {}
                                    _ => { closed.push(peer); }
                                }
                            }
                        }
                        if closed.last() == Some(&peer) { break; }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => { closed.push(peer); break; }
                }
            }
        }
        self.queue.extend(actions);
        for p in closed {
            if let Some(c) = self.conns.remove(&p) {
                if c.torrent.is_some() { let a = self.session.peer_closed(p, now); self.queue.extend(a); }
            }
        }
        any
    }

    fn announce_params(&self, key: &TorrentKey, event: AnnounceEvent, now: Millis) -> AnnounceParams {
        let mut p = AnnounceParams::new(*key, self.peer_id.0, self.port, self.key);
        if let Some(t) = self.session.get(key) { let s = t.stats(now); p.uploaded = s.uploaded; p.downloaded = s.downloaded; p.left = s.left; }
        p.event = event;
        p.anonymous = self.anonymous;
        p
    }

    fn announce(&mut self, now: Millis) {
        let keys: Vec<TorrentKey> = self.jobs.keys().copied().collect();
        for k in keys {
            let job = &self.jobs[&k];
            if job.trackers.is_empty() || now < job.next_announce { continue; }
            let event = if job.finished_at.is_some() && !job.announced_done { AnnounceEvent::Completed } else if job.next_announce == 0 && job.finished_at.is_none() { AnnounceEvent::Started } else { AnnounceEvent::None };
            let params = self.announce_params(&k, event, now);
            let job = self.jobs.get_mut(&k).expect("job");
            if event == AnnounceEvent::Completed { job.announced_done = true; }
            job.next_announce = now + 120_000; // until the answer sets the tracker's interval
            for url in job.trackers.clone() {
                let tx = self.done_tx.clone();
                let p = params.clone();
                std::thread::spawn(move || {
                    let (peers, interval, error) = match http_announce(&url, &p) { Ok((a, i)) => (a, i, None), Err(e) => (Vec::new(), 300, Some(e)) };
                    let _ = tx.send(Done::Announced { torrent: k, url, peers, interval, error });
                });
            }
        }
    }

    fn stop_announce(&mut self, key: &TorrentKey, now: Millis) {
        let p = self.announce_params(key, AnnounceEvent::Stopped, now);
        for url in self.jobs[key].trackers.clone() { let _ = http_announce(&url, &p); }
    }
}

/// One HTTP announce (plain `http://` trackers; `udp://` and `https://` come next).
fn http_announce(url: &str, p: &AnnounceParams) -> Result<(Vec<PeerAddr>, u64), String> {
    let target = announce_request(url, p).map_err(|e| format!("{e:?}"))?;
    if target.tls { return Err("https trackers are not supported yet".into()); }
    let addr = crate::resolve(&target.host, target.port).into_iter().next().ok_or("no address")?;
    let mut s = TcpStream::connect_timeout(&to_socket(addr), Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(15)));
    let ua = if p.anonymous { String::new() } else { "User-Agent: Swingers/0.1\r\n".into() };
    write!(s, "GET {} HTTP/1.0\r\nHost: {}\r\n{ua}Connection: close\r\n\r\n", target.path_and_query, target.host_header).map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    s.take(4 << 20).read_to_end(&mut body).map_err(|e| e.to_string())?;
    let split = body.windows(4).position(|w| w == b"\r\n\r\n").ok_or("no HTTP header")?;
    let r = parse_announce(&body[split + 4..]).map_err(|e| format!("{e:?}"))?;
    Ok((r.peers, r.interval.unwrap_or(1800) as u64))
}
