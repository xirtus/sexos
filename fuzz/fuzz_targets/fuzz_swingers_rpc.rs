// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_torrent::magnet::Magnet;
use sex_torrent::metainfo::Metainfo;
use sex_torrent::rpc::json::{self, Json};
use sex_torrent::rpc::{
    base64_decode, base64_encode, parse_request, AddOutcome, AddRequest, AddSource, FileInfo,
    HttpOutcome, RpcBackend, RpcConfig, RpcServer, SessionChanges, SessionSettings, SessionStats,
    StatsBlock, TorrentAction, TorrentChanges, TorrentInfo, TorrentStatus, TrackerInfo,
};
use sex_torrent::session::{Torrent, TorrentParams};
use sex_torrent::{webui, PeerId};

const MAX_HEAD_BYTES: usize = 8 * 1024;

struct FuzzRpcBackend {
    session: SessionSettings,
    torrents: Vec<TorrentInfo>,
    removed: Vec<u64>,
    next_id: u64,
    strict: bool,
}

impl FuzzRpcBackend {
    fn new(strict: bool) -> Self {
        let session = SessionSettings {
            download_dir: String::from("/downloads"),
            peer_port: 51413,
            ..SessionSettings::default()
        };
        let seed_torrent = TorrentInfo {
            id: 1,
            name: String::from("SexOS-Live.iso"),
            hash_string: String::from("0123456789abcdef0123456789abcdef01234567"),
            status: TorrentStatus::Download,
            percent_done: 0.5,
            percent_complete: 0.5,
            metadata_percent_complete: 1.0,
            eta: 120,
            total_size: 65536,
            size_when_done: 65536,
            left_until_done: 32768,
            have_valid: 32768,
            piece_count: 4,
            piece_size: 16384,
            pieces: vec![0b1010_0000],
            download_dir: String::from("/downloads"),
            files: vec![
                FileInfo {
                    name: String::from("SexOS-Live/boot.img"),
                    length: 32768,
                    bytes_completed: 32768,
                    wanted: true,
                    priority: 1,
                    begin_piece: 0,
                    end_piece: 2,
                },
                FileInfo {
                    name: String::from("SexOS-Live/README.txt"),
                    length: 32768,
                    bytes_completed: 0,
                    wanted: false,
                    priority: -1,
                    begin_piece: 2,
                    end_piece: 4,
                },
            ],
            trackers: vec![TrackerInfo {
                id: 0,
                tier: 0,
                announce: String::from("https://tracker.example.org/announce"),
                scrape: String::from("https://tracker.example.org/scrape"),
                host: String::from("tracker.example.org"),
                has_announced: true,
                last_announce_succeeded: true,
                last_announce_peer_count: 4,
                last_announce_time: 1_700_000_000,
                next_announce_time: 1_700_001_800,
                seeder_count: 10,
                leecher_count: 2,
                download_count: 50,
                last_announce_result: String::from("Success"),
                ..TrackerInfo::default()
            }],
            labels: vec![String::from("iso")],
            ..TorrentInfo::default()
        };
        Self {
            session,
            torrents: vec![seed_torrent],
            removed: vec![99],
            next_id: 2,
            strict,
        }
    }
}

impl RpcBackend for FuzzRpcBackend {
    fn session(&self) -> SessionSettings {
        self.session.clone()
    }

    fn set_session(&mut self, c: &SessionChanges) -> Result<(), String> {
        self.session.apply(c);
        Ok(())
    }

    fn stats(&self) -> SessionStats {
        let active_torrent_count = self
            .torrents
            .iter()
            .filter(|t| t.status != TorrentStatus::Stopped)
            .count() as u32;
        let paused_torrent_count = self.torrents.len() as u32 - active_torrent_count;
        let b = StatsBlock {
            uploaded_bytes: 1024,
            downloaded_bytes: 2048,
            files_added: self.torrents.len() as u64,
            session_count: 1,
            seconds_active: 60,
        };
        SessionStats {
            active_torrent_count,
            paused_torrent_count,
            torrent_count: self.torrents.len() as u32,
            download_speed: 0,
            upload_speed: 0,
            current: b,
            cumulative: b,
        }
    }

    fn torrents(&self, ids: Option<&[u64]>, _detail: bool) -> Vec<TorrentInfo> {
        match ids {
            None => self.torrents.clone(),
            Some(want) => self
                .torrents
                .iter()
                .filter(|t| want.contains(&t.id))
                .cloned()
                .collect(),
        }
    }

    fn recently_active(&self) -> Vec<u64> {
        self.torrents.iter().map(|t| t.id).collect()
    }

    fn recently_removed(&self) -> Vec<u64> {
        self.removed.clone()
    }

    fn add(&mut self, req: AddRequest) -> Result<AddOutcome, String> {
        match req.source {
            AddSource::Metainfo(raw) => {
                let meta = Metainfo::parse(&raw).map_err(|e| format!("bad torrent: {:?}", e))?;
                let layout = meta.layout().map_err(|e| format!("bad layout: {:?}", e))?;
                let hash = sex_torrent::to_hex(&meta.info_hashes.wire());
                if let Some(existing) = self.torrents.iter().find(|t| t.hash_string == hash) {
                    return Ok(AddOutcome::Duplicate {
                        id: existing.id,
                        name: existing.name.clone(),
                        hash,
                    });
                }
                // Mirror `apps/swingers/src/main.rs` + `remote.rs` (`Torrent::new` + `(0..s.num_pieces).collect()`),
                // capping piece count in non-strict mode so fuzz campaigns stay well below RSS limits.
                if self.strict || layout.num_pieces() <= 4096 {
                    let mut p = TorrentParams::new(meta.info_hashes, PeerId(*b"-SW0001-000000000000"));
                    p.layout = Some(layout.clone());
                    p.verifier = Some(meta.verifier());
                    p.info = Some(meta.info_bytes.clone());
                    let t = Torrent::new(p, 0);
                    let s = t.stats(0);
                    let _priorities: Vec<u32> = (0..s.num_pieces).collect();
                }
                if self.torrents.len() >= 16 {
                    return Err(String::from("fuzz cap"));
                }
                let id = self.next_id;
                self.next_id += 1;
                let files: Vec<FileInfo> = layout
                    .files
                    .iter()
                    .take(32)
                    .enumerate()
                    .filter(|(_, f)| !f.pad)
                    .map(|(idx, f)| FileInfo {
                        name: f.path.join("/"),
                        length: f.length,
                        bytes_completed: 0,
                        wanted: !req.files_unwanted.contains(&(idx as u32)),
                        priority: if req.priority_high.contains(&(idx as u32)) {
                            1
                        } else if req.priority_low.contains(&(idx as u32)) {
                            -1
                        } else {
                            0
                        },
                        begin_piece: 0,
                        end_piece: 1,
                    })
                    .collect();
                let piece_count = layout.num_pieces().min(4096);
                let tv = TorrentInfo {
                    id,
                    name: meta.name.clone(),
                    hash_string: hash.clone(),
                    status: if req.paused.unwrap_or(false) {
                        TorrentStatus::Stopped
                    } else {
                        TorrentStatus::Download
                    },
                    total_size: layout.total_length,
                    size_when_done: layout.total_length,
                    left_until_done: layout.total_length,
                    piece_count,
                    piece_size: layout.piece_length as u64,
                    pieces: vec![0u8; (piece_count as usize).div_ceil(8)],
                    download_dir: req
                        .download_dir
                        .unwrap_or_else(|| self.session.download_dir.clone()),
                    files,
                    labels: req.labels,
                    ..TorrentInfo::default()
                };
                self.torrents.push(tv);
                Ok(AddOutcome::Added {
                    id,
                    name: meta.name,
                    hash,
                })
            }
            AddSource::Magnet(uri) => {
                if self.strict {
                    // `apps/swingers/src/main.rs:105` dispatches `source.starts_with("magnet:")`
                    // case-sensitively, falling through to `std::fs::read(&source)` otherwise!
                    assert!(
                        uri.starts_with("magnet:"),
                        "AddSource::Magnet preserved non-lowercase scheme {:?}, causing swingers main.rs to read local file path",
                        uri
                    );
                }
                let m = Magnet::parse(&uri).map_err(|e| format!("bad magnet: {:?}", e))?;
                let hash = sex_torrent::to_hex(&m.info_hashes.wire());
                let name = m.display_name.unwrap_or_else(|| hash.clone());
                let id = self.next_id;
                self.next_id += 1;
                Ok(AddOutcome::Added { id, name, hash })
            }
            AddSource::Url(_) | AddSource::Path(_) => {
                Err(String::from("unsupported source in fuzz"))
            }
        }
    }

    fn action(&mut self, ids: &[u64], action: TorrentAction) -> Result<(), String> {
        match action {
            TorrentAction::Start | TorrentAction::StartNow => {
                for t in &mut self.torrents {
                    if ids.contains(&t.id) {
                        t.status = TorrentStatus::Download;
                    }
                }
            }
            TorrentAction::Stop => {
                for t in &mut self.torrents {
                    if ids.contains(&t.id) {
                        t.status = TorrentStatus::Stopped;
                    }
                }
            }
            TorrentAction::Remove { .. } => {
                self.torrents.retain(|t| {
                    if ids.contains(&t.id) {
                        self.removed.push(t.id);
                        false
                    } else {
                        true
                    }
                });
            }
            TorrentAction::Verify
            | TorrentAction::Reannounce
            | TorrentAction::QueueTop
            | TorrentAction::QueueUp
            | TorrentAction::QueueDown
            | TorrentAction::QueueBottom => {}
        }
        Ok(())
    }

    fn set(&mut self, ids: &[u64], c: &TorrentChanges) -> Result<(), String> {
        for t in &mut self.torrents {
            if !ids.contains(&t.id) {
                continue;
            }
            if let Some(labels) = &c.labels {
                t.labels = labels.clone();
            }
            if let Some(want) = &c.files_wanted {
                for &i in want {
                    if let Some(f) = t.files.get_mut(i as usize) {
                        f.wanted = true;
                    }
                }
            }
            if let Some(unwant) = &c.files_unwanted {
                for &i in unwant {
                    if let Some(f) = t.files.get_mut(i as usize) {
                        f.wanted = false;
                    }
                }
            }
        }
        Ok(())
    }

    fn set_location(
        &mut self,
        ids: &[u64],
        location: &str,
        _move_data: bool,
    ) -> Result<(), String> {
        for t in &mut self.torrents {
            if ids.contains(&t.id) {
                t.download_dir = String::from(location);
            }
        }
        Ok(())
    }

    fn free_space(&self, path: &str) -> Option<(u64, u64)> {
        if path.is_empty() {
            None
        } else {
            Some((10_000_000_000, 100_000_000_000))
        }
    }

    fn port_test(&mut self) -> Option<bool> {
        Some(true)
    }
}

fn is_valid_local_host_header(h: &str) -> bool {
    let h = h.trim();
    if let Some(rest) = h.strip_prefix('[') {
        let Some((inside, after)) = rest.split_once(']') else {
            return false;
        };
        if !after.is_empty() {
            let Some(port) = after.strip_prefix(':') else {
                return false;
            };
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
        }
        // Bracketed host must actually be an IPv6 literal (hex digits & colons only, containing `:`)
        return !inside.is_empty()
            && inside.contains(':')
            && inside
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    }
    let host_part = if let Some((left, port)) = h.rsplit_once(':') {
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        left
    } else {
        h
    };
    let lower = host_part.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    !lower.is_empty()
        && lower.split('.').count() == 4
        && lower.split('.').all(|p| p.parse::<u8>().is_ok())
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let strict = std::env::var_os("SEXOS_FUZZ_STRICT_SECURITY").is_some();

    // 1. JSON codec parse -> encode -> re-parse invariant.
    if let Ok(parsed) = json::parse(data) {
        let encoded = parsed.encode();
        let reparsed = json::parse(encoded.as_bytes()).expect("Json::encode produced invalid JSON");
        let _ = reparsed == parsed;
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = webui::serve("GET", s);
        let _ = webui::serve("HEAD", s);
    }

    // 2. Base64 decode / encode round-trip.
    let _ = base64_decode(data);
    if data.len() <= 4096 {
        let b64 = base64_encode(data);
        let decoded = base64_decode(b64.as_bytes()).expect("base64_encode must decode cleanly");
        assert_eq!(decoded.as_slice(), data);
    }

    // 3. Raw HTTP/1.1 stream parsing (`parse_request` + `RpcServer::handle_http` pipelining).
    if let Ok((req, consumed)) = parse_request(data) {
        assert!(consumed <= data.len());
        if strict {
            let overhead = consumed.saturating_sub(req.body.len());
            assert!(
                overhead <= MAX_HEAD_BYTES + 1024,
                "parse_request accepted {} bytes of HTTP header/chunk-trailer overhead (> MAX_HEAD)",
                overhead
            );
        }
    }

    let mut backend = FuzzRpcBackend::new(strict);
    let mut cfg = RpcConfig::default();
    if (data[0] & 0x80) != 0 && !strict {
        cfg.auth_required = true;
        cfg.username = String::from("admin");
        cfg.password = String::from("secret");
    }
    let mut zero_rng = |b: &mut [u8]| b.fill(0);
    let mut server = RpcServer::new(cfg, &mut zero_rng);

    let mut cursor = 0usize;
    let mut http_reqs = 0usize;
    while cursor < data.len() && http_reqs < 8 {
        http_reqs += 1;
        match server.handle_http(&data[cursor..], &mut backend) {
            HttpOutcome::NeedMore => break,
            HttpOutcome::Reply {
                bytes,
                close,
                consumed,
            } => {
                if strict {
                    if let Ok((req, _)) = parse_request(&data[cursor..cursor + consumed]) {
                        let not_421 = !bytes.starts_with(b"HTTP/1.1 421");
                        let not_400 = !bytes.starts_with(b"HTTP/1.1 400");
                        if not_421 && not_400 {
                            let host_headers: Vec<&str> = req
                                .headers
                                .iter()
                                .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
                                .map(|(_, v)| v.as_str())
                                .collect();
                            if !host_headers.is_empty() {
                                assert!(
                                    host_headers.len() == 1
                                        && is_valid_local_host_header(host_headers[0]),
                                    "DNS-rebinding Host check bypassed with Host headers {:?} -> HTTP reply {:?}",
                                    host_headers,
                                    core::str::from_utf8(&bytes[..bytes.len().min(48)])
                                );
                            }
                        }
                    }
                }
                if consumed == 0 || close {
                    break;
                }
                cursor += consumed;
            }
        }
    }

    // 4. Direct authenticated JSON-RPC / Legacy Transmission RPC dispatch (`handle_rpc`)
    // so fuzzing immediately exercises all 20+ RPC methods without needing to guess the
    // 48-char CSRF token or HTTP framing.
    for msg in data.split(|&b| b == 0).take(8) {
        if !msg.is_empty() {
            if let Ok(reply_json) = server.handle_rpc(msg, &mut backend) {
                let encoded = reply_json.encode();
                let _ = json::parse(encoded.as_bytes());
            }
        }
    }

    // 5. Structured RPC envelope synthesis around arbitrary payload bytes (base64 metainfo,
    // magnet links, field selectors, ids, session-set / torrent-set / jsonrpc 2.0).
    if data.len() >= 2 {
        let kind = data[0] % 8;
        let rest = &data[1..];
        let synthesized = match kind {
            0 => {
                let b64 = base64_encode(&rest[..rest.len().min(2048)]);
                format!(
                    r#"{{"method":"torrent-add","arguments":{{"metainfo":"{}","paused":true,"files-unwanted":[0],"priority-high":[1],"labels":["fuzz"]}}}}"#,
                    b64
                )
            }
            1 => {
                let s = String::from_utf8_lossy(&rest[..rest.len().min(512)]);
                let escaped: String = s
                    .chars()
                    .filter(|&c| c != '"' && c != '\\' && c >= ' ')
                    .collect();
                format!(
                    r#"{{"method":"torrent-add","arguments":{{"filename":"{}"}}}}"#,
                    escaped
                )
            }
            2 => {
                let table = if (data[0] & 0x10) != 0 {
                    r#""format":"table","#
                } else {
                    ""
                };
                format!(
                    r#"{{"method":"torrent-get","arguments":{{{}"ids":"recently-active","fields":["id","name","status","totalSize","percentDone","files","fileStats","priorities","wanted","trackers","trackerStats","trackerList","pieces","labels","magnetLink","eta","uploadRatio"]}},"tag":7}}"#,
                    table
                )
            }
            3 => {
                let s = String::from_utf8_lossy(&rest[..rest.len().min(256)]);
                format!(
                    r#"{{"jsonrpc":"2.0","method":"torrent_get","params":{{"ids":[1],"fields":["hash_string","percent_done","files"]}},"id":{}}}"#,
                    Json::Str(s.into_owned()).encode()
                )
            }
            4 => {
                let lim = rest.first().copied().unwrap_or(0) as u64 * 10;
                format!(
                    r#"{{"method":"torrent-set","arguments":{{"ids":[1],"downloadLimit":{},"downloadLimited":true,"seedRatioLimit":1.25,"seedRatioMode":1,"files-wanted":[0],"files-unwanted":[1]}}}}"#,
                    lim
                )
            }
            5 => {
                let port = u16::from_le_bytes([rest[0], *rest.get(1).unwrap_or(&0)]) as u64;
                format!(
                    r#"{{"method":"session-set","arguments":{{"peer-port":{},"speed-limit-down":500,"speed-limit-down-enabled":true,"encryption":"preferred"}}}}"#,
                    port
                )
            }
            6 => String::from(r#"{"method":"session-get"}"#),
            _ => String::from(r#"{"method":"session-stats"}"#),
        };
        let _ = server.handle_rpc(synthesized.as_bytes(), &mut backend);
    }
});
