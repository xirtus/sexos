// SPDX-License-Identifier: GPL-3.0-only
//! Remote control: Transmission's RPC (phone apps, transmission-remote, Flood and the
//! like) and the web UI, served from the daemon's event loop. The protocol is
//! `sex_torrent::rpc`; this file maps it onto the running torrents.
use crate::net::Client;
use crate::say;
use sex_torrent::rpc::{
    AddOutcome, AddRequest, AddSource, FileInfo, HttpOutcome, RpcBackend, RpcConfig, RpcServer, SessionChanges, SessionSettings, SessionStats,
    TorrentAction, TorrentChanges, TorrentInfo, TorrentStatus,
};
use sex_torrent::session::{TorrentKey, TorrentState};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

/// One client connection: what it sent that is not answered yet, and what is still to go.
struct Conn { stream: TcpStream, inbuf: Vec<u8>, out: Vec<u8>, close: bool }

pub struct Rpc {
    listener: TcpListener,
    server: RpcServer,
    conns: Vec<Conn>,
    settings: SessionSettings,
    removed: Vec<u64>,
}

impl Rpc {
    pub fn new(port: u16, user: &str, password: &str, download_dir: &str, peer_port: u16) -> Result<Rpc, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("cannot listen on port {port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let config = RpcConfig { auth_required: !password.is_empty(), username: user.into(), password: password.into(), ..RpcConfig::default() };
        let server = RpcServer::new(config, &mut |b: &mut [u8]| crate::random(b));
        let settings = SessionSettings {
            download_dir: download_dir.into(), peer_port: peer_port as u64, pex_enabled: true, encryption: "preferred".into(),
            start_added_torrents: true, peer_limit_global: 500, peer_limit_per_torrent: 80, seed_ratio_limit: 2.0, ..SessionSettings::default()
        };
        Ok(Rpc { listener, server, conns: Vec::new(), settings, removed: Vec::new() })
    }
}

/// Serves the remote-control connections once. Returns whether anything moved.
pub fn pump(c: &mut Client) -> bool {
    let Some(mut rpc) = c.rpc.take() else { return false };
    let mut any = false;
    while let Ok((s, _)) = rpc.listener.accept() {
        any = true;
        if rpc.conns.len() >= 64 { continue; }
        let _ = s.set_nonblocking(true);
        let _ = s.set_nodelay(true);
        rpc.conns.push(Conn { stream: s, inbuf: Vec::new(), out: Vec::new(), close: false });
    }
    let mut buf = [0u8; 16 * 1024];
    let Rpc { server, conns, settings, removed, .. } = &mut rpc;
    conns.retain_mut(|conn| {
        if !conn.close {
            loop {
                match conn.stream.read(&mut buf) {
                    Ok(0) => { conn.close = true; break; }
                    Ok(n) => { any = true; conn.inbuf.extend_from_slice(&buf[..n]); if conn.inbuf.len() > 64 << 20 { return false; } }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => return false,
                }
            }
            let mut backend = Backend { c, settings, removed };
            while !conn.inbuf.is_empty() {
                match server.handle_http(&conn.inbuf, &mut backend) {
                    HttpOutcome::NeedMore => break,
                    HttpOutcome::Reply { bytes, consumed, close } => {
                        any = true;
                        conn.out.extend_from_slice(&bytes);
                        conn.inbuf.drain(..consumed.min(conn.inbuf.len()));
                        if close { conn.close = true; conn.inbuf.clear(); }
                    }
                }
            }
        }
        while !conn.out.is_empty() {
            match conn.stream.write(&conn.out) {
                Ok(0) => return false,
                Ok(n) => { any = true; conn.out.drain(..n); }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        !(conn.close && conn.out.is_empty())
    });
    c.rpc = Some(rpc);
    any
}

/// The running torrents as the RPC sees them.
struct Backend<'a> { c: &'a mut Client, settings: &'a mut SessionSettings, removed: &'a mut Vec<u64> }

impl Backend<'_> {
    fn key_of(&self, id: u64) -> Option<TorrentKey> { self.c.jobs.iter().find(|(_, j)| j.id == id).map(|(k, _)| *k) }

    fn keys(&self, ids: &[u64]) -> Vec<TorrentKey> {
        if ids.is_empty() { self.c.jobs.keys().copied().collect() } else { ids.iter().filter_map(|&i| self.key_of(i)).collect() }
    }

    fn info(&self, key: &TorrentKey, detail: bool) -> Option<TorrentInfo> {
        let job = self.c.jobs.get(key)?;
        let t = self.c.session.get(key)?;
        let now = self.c.now();
        let s = t.stats(now);
        let status = match s.state {
            TorrentState::Checking => TorrentStatus::Check,
            TorrentState::DownloadingMetadata | TorrentState::Downloading => TorrentStatus::Download,
            TorrentState::Seeding => TorrentStatus::Seed,
            TorrentState::Queued => if t.is_finished() { TorrentStatus::SeedWait } else { TorrentStatus::DownloadWait },
            TorrentState::Paused | TorrentState::Error => TorrentStatus::Stopped,
        };
        let layout = t.layout();
        let total = layout.map(|l| l.total_length).unwrap_or(0);
        let left = s.left.min(total);
        let mut info = TorrentInfo {
            id: job.id,
            hash_string: sex_torrent::to_hex(key),
            name: job.name.clone(),
            status,
            error: if s.state == TorrentState::Error { 3 } else { 0 },
            error_string: if s.state == TorrentState::Error { "disk write failed".into() } else { String::new() },
            percent_done: s.progress_ppm as f64 / 1e6,
            percent_complete: s.progress_ppm as f64 / 1e6,
            metadata_percent_complete: if layout.is_some() { 1.0 } else { 0.0 },
            rate_download: s.download_rate,
            rate_upload: s.upload_rate,
            eta: if t.is_finished() { -1 } else { s.eta_secs.map(|e| e as i64).unwrap_or(-1) },
            eta_idle: -1,
            total_size: total,
            size_when_done: total,
            left_until_done: left,
            have_valid: total - left,
            desired_available: left,
            corrupt_ever: s.wasted,
            uploaded_ever: s.uploaded,
            downloaded_ever: s.downloaded,
            upload_ratio: if s.downloaded == 0 { -1.0 } else { s.ratio_milli as f64 / 1000.0 },
            peers_connected: s.peers as u32,
            peers_sending_to_us: s.seeds as u32,
            added_date: job.added,
            download_dir: job.dir.display().to_string(),
            is_finished: job.finished_at.is_some() && s.state == TorrentState::Paused,
            magnet_link: job.magnet.clone(),
            piece_count: s.num_pieces,
            piece_size: layout.map(|l| l.piece_length as u64).unwrap_or(0),
            max_connected_peers: self.settings.peer_limit_per_torrent as u32,
            ..TorrentInfo::default()
        };
        if detail {
            if let (Some(l), Some(have)) = (layout, t.have()) {
                let pl = l.piece_length as u64;
                for f in l.files.iter().filter(|f| !f.pad) {
                    let mut done = 0u64;
                    if f.length > 0 {
                        let (first, last) = (f.offset / pl, (f.offset + f.length - 1) / pl);
                        for p in first..=last {
                            if have.get(p as u32) {
                                let (a, b) = ((p * pl).max(f.offset), ((p + 1) * pl).min(f.offset + f.length));
                                done += b - a;
                            }
                        }
                    }
                    let name = core::iter::once(job.name.as_str()).chain(f.path.iter().map(String::as_str).skip(if l.files.len() == 1 { 1 } else { 0 })).collect::<Vec<_>>().join("/");
                    info.files.push(FileInfo { name, length: f.length, bytes_completed: done, wanted: true, priority: 0, begin_piece: (f.offset / pl) as u32, end_piece: ((f.offset + f.length).div_ceil(pl)) as u32 });
                }
                // The piece bitfield, base64 like Transmission's `pieces`.
                info.pieces = (0..s.num_pieces).collect::<Vec<u32>>().chunks(8).map(|c| c.iter().fold(0u8, |b, &i| b | ((have.get(i) as u8) << (7 - i % 8)))).collect();
            }
            info.trackers = job.trackers.iter().enumerate().map(|(i, u)| sex_torrent::rpc::TrackerInfo { id: i as u32, announce: u.clone(), tier: i as u32, ..Default::default() }).collect();
        }
        Some(info)
    }
}

impl RpcBackend for Backend<'_> {
    fn session(&self) -> SessionSettings { self.settings.clone() }

    fn set_session(&mut self, changes: &SessionChanges) -> Result<(), String> {
        self.settings.apply(changes);
        if let Some(d) = &changes.download_dir { self.c.dir = PathBuf::from(d); }
        Ok(())
    }

    fn stats(&self) -> SessionStats {
        let now = self.c.now();
        let mut out = SessionStats { torrent_count: self.c.jobs.len() as u32, ..SessionStats::default() };
        for k in self.c.jobs.keys() {
            let Some(t) = self.c.session.get(k) else { continue };
            let s = t.stats(now);
            if s.state == TorrentState::Paused { out.paused_torrent_count += 1 } else { out.active_torrent_count += 1 }
            out.download_speed += s.download_rate;
            out.upload_speed += s.upload_rate;
            out.current.downloaded_bytes += s.downloaded;
            out.current.uploaded_bytes += s.uploaded;
        }
        out.current.seconds_active = now / 1000;
        out.current.session_count = 1;
        out.current.files_added = self.c.next_id - 1;
        out.cumulative = out.current;
        out
    }

    fn torrents(&self, ids: Option<&[u64]>, detail: bool) -> Vec<TorrentInfo> {
        let mut keys = self.keys(ids.unwrap_or(&[]));
        if ids.is_some_and(|i| !i.is_empty()) { keys.retain(|k| self.c.jobs.contains_key(k)); }
        let mut out: Vec<TorrentInfo> = keys.iter().filter_map(|k| self.info(k, detail)).collect();
        out.sort_by_key(|i| i.id);
        out
    }

    fn recently_active(&self) -> Vec<u64> { self.c.jobs.values().map(|j| j.id).collect() }

    fn recently_removed(&self) -> Vec<u64> { self.removed.clone() }

    fn add(&mut self, req: AddRequest) -> Result<AddOutcome, String> {
        let dir = req.download_dir.map(PathBuf::from);
        let (id, key, dup) = match req.source {
            AddSource::Magnet(m) => self.c.add_source(&m, None, Vec::new(), dir)?,
            AddSource::Path(p) => self.c.add_source(&p, None, Vec::new(), dir)?,
            AddSource::Metainfo(b) => self.c.add_source("", Some(b), Vec::new(), dir)?,
            AddSource::Url(u) => return Err(format!("fetching .torrent URLs is not supported yet: {u}")),
        };
        let name = self.c.jobs[&key].name.clone();
        let hash = sex_torrent::to_hex(&key);
        if dup { return Ok(AddOutcome::Duplicate { id, name, hash }); }
        if req.paused == Some(true) || (req.paused.is_none() && !self.settings.start_added_torrents) {
            self.action(&[id], TorrentAction::Stop)?;
        }
        Ok(AddOutcome::Added { id, name, hash })
    }

    fn action(&mut self, ids: &[u64], action: TorrentAction) -> Result<(), String> {
        let now = self.c.now();
        for key in self.keys(ids) {
            match action {
                TorrentAction::Start | TorrentAction::StartNow => {
                    let a = self.c.session.with(&key, now, |t, now| t.resume(now));
                    self.c.queue.extend(a);
                    if let Some(j) = self.c.jobs.get_mut(&key) { j.paused = false; j.next_announce = 0; }
                }
                TorrentAction::Stop => {
                    let a = self.c.session.with(&key, now, |t, now| t.pause(now));
                    self.c.queue.extend(a);
                    if let Some(j) = self.c.jobs.get_mut(&key) { j.paused = true; }
                }
                TorrentAction::Verify => {
                    let a = self.c.session.with(&key, now, |t, now| t.force_recheck(now));
                    self.c.queue.extend(a);
                    if let Some(j) = self.c.jobs.get_mut(&key) { j.finished_at = None; }
                }
                TorrentAction::Reannounce => { if let Some(j) = self.c.jobs.get_mut(&key) { j.next_announce = 1; } }
                TorrentAction::Remove { delete_local_data } => {
                    let a = self.c.session.remove(&key, now);
                    self.c.queue.extend(a);
                    let Some(mut job) = self.c.jobs.remove(&key) else { continue };
                    self.removed.push(job.id);
                    say(&format!("[swingers.removed] {} {}", sex_torrent::to_hex(&key), job.name));
                    if let Some(d) = job.disk.as_mut() {
                        d.close();
                        if delete_local_data {
                            for i in 0..d.layout().files.len() { if let Some(p) = d.path_of(i) { let _ = std::fs::remove_file(p); } }
                            let _ = std::fs::remove_file(d.root.join(".swingers").join(format!("{}.parts", sex_torrent::to_hex(&key))));
                            // Folders the torrent made, if they are now empty.
                            if d.layout().files.len() > 1 { let _ = remove_empty_dirs(&d.root.join(&job.name)); }
                        }
                    }
                }
                TorrentAction::QueueTop | TorrentAction::QueueUp | TorrentAction::QueueDown | TorrentAction::QueueBottom => {}
            }
        }
        Ok(())
    }

    fn set(&mut self, _ids: &[u64], _changes: &TorrentChanges) -> Result<(), String> { Ok(()) }

    fn set_location(&mut self, ids: &[u64], location: &str, move_data: bool) -> Result<(), String> {
        if move_data { return Err("moving data is not supported yet".into()); }
        for key in self.keys(ids) {
            if let Some(j) = self.c.jobs.get_mut(&key) {
                j.dir = PathBuf::from(location);
                if let Some(d) = j.disk.take() { j.disk = Some(crate::disk::Disk::new(&j.dir, d.layout().clone(), &sex_torrent::to_hex(&key))); }
            }
        }
        Ok(())
    }

    fn free_space(&self, _path: &str) -> Option<(u64, u64)> { None }
}

fn remove_empty_dirs(dir: &std::path::Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() { let _ = remove_empty_dirs(&p); }
    }
    std::fs::remove_dir(dir)
}
