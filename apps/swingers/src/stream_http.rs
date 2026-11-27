// SPDX-License-Identifier: GPL-3.0-only
//! Local HTTP video streaming server (`swingers stream <hash-or-magnet> [--port 8990]`).
//!
//! Prioritizes the largest video file in the torrent using `sex_torrent::stream::Planner`
//! (first 2 MiB + last 1 MiB for MP4/MKV metadata, then sequential forward ahead of the
//! player's read position) and serves `GET /stream` / `HEAD /stream` on `127.0.0.1:<port>`
//! with `Accept-Ranges: bytes` and `206 Partial Content`.
use crate::net::Client;
use crate::say;
use sex_torrent::session::TorrentKey;
use sex_torrent::storage::TorrentLayout;
use sex_torrent::stream::{FileSpan, Planner};
use sex_torrent::Millis;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};

const MAX_CONNS: usize = 16;
const CHUNK_BYTES: usize = 64 * 1024;
const OUT_HIGH_WATER: usize = 256 * 1024;
const META_WAIT_TIMEOUT_MS: Millis = 90_000;

enum ConnState {
    ReadingRequest,
    WaitingForMetadata { head_only: bool, range_hdr: Option<String>, since: Millis },
    Streaming { start: u64, end: u64, sent: u64, partial: bool, headers_sent: bool },
    Draining,
}

struct StreamConn {
    stream: TcpStream,
    inbuf: Vec<u8>,
    out: Vec<u8>,
    state: ConnState,
}

pub struct HttpStreamServer {
    listener: TcpListener,
    pub target: TorrentKey,
    configured: bool,
    file_idx: Option<usize>,
    file_len: u64,
    content_type: &'static str,
    planner: Option<Planner>,
    last_deadlines: Millis,
    conns: Vec<StreamConn>,
}

impl HttpStreamServer {
    pub fn new(port: u16, target: TorrentKey) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self {
            listener,
            target,
            configured: false,
            file_idx: None,
            file_len: 0,
            content_type: "application/octet-stream",
            planner: None,
            last_deadlines: 0,
            conns: Vec::new(),
        })
    }
}

fn video_content_type(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".mp4") || lower.ends_with(".m4v") {
        Some("video/mp4")
    } else if lower.ends_with(".mkv") {
        Some("video/x-matroska")
    } else if lower.ends_with(".webm") {
        Some("video/webm")
    } else if lower.ends_with(".mov") {
        Some("video/quicktime")
    } else if lower.ends_with(".ts") || lower.ends_with(".m2ts") {
        Some("video/mp2t")
    } else if lower.ends_with(".avi") {
        Some("video/x-msvideo")
    } else {
        None
    }
}

/// Picks the largest video file in `layout`, falling back to the largest non-pad file.
fn pick_video_file(layout: &TorrentLayout) -> Option<(usize, String, u64, &'static str)> {
    let mut best_video: Option<(usize, String, u64, &'static str)> = None;
    let mut best_any: Option<(usize, String, u64)> = None;
    for (idx, f) in layout.files.iter().enumerate() {
        if f.pad || f.length == 0 {
            continue;
        }
        let name = f.path.last().cloned().unwrap_or_default();
        if let Some(ct) = video_content_type(&name) {
            if best_video.as_ref().is_none_or(|b| f.length > b.2) {
                best_video = Some((idx, name.clone(), f.length, ct));
            }
        }
        if best_any.as_ref().is_none_or(|b| f.length > b.2) {
            best_any = Some((idx, name, f.length));
        }
    }
    best_video.or_else(|| best_any.map(|(i, n, l)| (i, n, l, "application/octet-stream")))
}

/// Parses an HTTP `Range: bytes=...` header into an inclusive `(start, end)` range.
fn parse_range(hdr: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = hdr.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() {
        let suffix: u64 = b.trim().parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        Some((start, total - 1))
    } else {
        let start: u64 = a.trim().parse().ok()?;
        if start >= total {
            return None;
        }
        let end = if b.trim().is_empty() {
            total - 1
        } else {
            b.trim().parse::<u64>().ok()?.min(total - 1)
        };
        if end < start {
            return None;
        }
        Some((start, end))
    }
}

fn push_deadlines(c: &mut Client, key: &TorrentKey, planner: &Planner, now: Millis) {
    let deadlines = {
        let Some(t) = c.session.get(key) else { return };
        let Some(have) = t.have() else { return };
        planner.deadlines(&|p| have.get(p))
    };
    if deadlines.is_empty() {
        return;
    }
    let actions = c.session.with(key, now, |t, now| {
        let mut out = Vec::new();
        for d in deadlines.iter().take(64) {
            out.extend(t.set_deadline(d.piece, d.due, now));
        }
        out
    });
    c.queue.extend(actions);
}

fn configure_target(c: &mut Client, srv: &mut HttpStreamServer, now: Millis) {
    if srv.configured {
        return;
    }
    let layout = match c.session.get(&srv.target).and_then(|t| t.layout().cloned()) {
        Some(l) => l,
        None => return,
    };
    let Some((file_idx, name, file_len, content_type)) = pick_video_file(&layout) else {
        return;
    };
    let file_offset = layout.files[file_idx].offset;
    let span = FileSpan {
        offset: file_offset,
        length: file_len,
        piece_length: layout.piece_length,
    };
    let planner = Planner::new(span, now);

    // Prioritize only the chosen video file, enable sequential mode, and pin header/footer pieces.
    let file_count = layout.files.len();
    let actions = c.session.with(&srv.target, now, |t, now| {
        let mut out = Vec::new();
        for i in 0..file_count {
            let prio = if i == file_idx { 1 } else { 0 };
            out.extend(t.set_file_priority(i, prio, now));
        }
        out.extend(t.set_sequential(true, now));
        out
    });
    c.queue.extend(actions);

    srv.configured = true;
    srv.file_idx = Some(file_idx);
    srv.file_len = file_len;
    srv.content_type = content_type;
    srv.planner = Some(planner);
    srv.last_deadlines = now;
    if let Some(p) = srv.planner.as_ref() {
        push_deadlines(c, &srv.target, p, now);
    }
    say(&format!(
        "[swingers.stream.ready] file={} bytes={} content_type={}",
        name, file_len, content_type
    ));
}

fn parse_http_request(buf: &[u8]) -> Option<(String, String, Option<String>)> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_ascii_uppercase();
    let path = parts.next()?.to_string();
    let mut range = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }
    Some((method, path, range))
}

fn write_error(out: &mut Vec<u8>, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    out.extend_from_slice(resp.as_bytes());
}

fn begin_request(
    c: &mut Client,
    srv_target: &TorrentKey,
    planner: &mut Option<Planner>,
    file_len: u64,
    content_type: &'static str,
    head_only: bool,
    range_hdr: Option<&str>,
    out: &mut Vec<u8>,
    now: Millis,
) -> ConnState {
    if head_only {
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Length: {file_len}\r\nConnection: close\r\n\r\n"
        );
        out.extend_from_slice(resp.as_bytes());
        return ConnState::Draining;
    }
    let (start, end, partial) = match range_hdr {
        Some(hdr) => match parse_range(hdr, file_len) {
            Some((s, e)) => (s, e, true),
            None => {
                let resp = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{file_len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                out.extend_from_slice(resp.as_bytes());
                return ConnState::Draining;
            }
        },
        None => {
            if file_len == 0 {
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                out.extend_from_slice(resp.as_bytes());
                return ConnState::Draining;
            }
            (0, file_len - 1, false)
        }
    };
    if let Some(p) = planner.as_mut() {
        p.set_position(start, 0, now);
        push_deadlines(c, srv_target, p, now);
    }
    ConnState::Streaming {
        start,
        end,
        sent: start,
        partial,
        headers_sent: false,
    }
}

/// Pumps the local HTTP streaming server once. Returns whether anything moved.
pub fn pump(c: &mut Client) -> bool {
    let Some(mut srv) = c.stream_http.take() else { return false };
    let now = c.now();
    let mut any = false;

    configure_target(c, &mut srv, now);

    // Periodically refresh streaming deadlines so newly-arrived pieces keep the window moving.
    if srv.configured && now >= srv.last_deadlines + 1000 {
        srv.last_deadlines = now;
        if let Some(p) = srv.planner.as_ref() {
            push_deadlines(c, &srv.target, p, now);
        }
    }

    while let Ok((s, _)) = srv.listener.accept() {
        any = true;
        if srv.conns.len() >= MAX_CONNS {
            continue;
        }
        let _ = s.set_nonblocking(true);
        let _ = s.set_nodelay(true);
        srv.conns.push(StreamConn {
            stream: s,
            inbuf: Vec::new(),
            out: Vec::new(),
            state: ConnState::ReadingRequest,
        });
    }

    let mut rbuf = [0u8; 4096];
    let target = srv.target;
    let configured = srv.configured;
    let file_idx = srv.file_idx;
    let file_len = srv.file_len;
    let content_type = srv.content_type;

    srv.conns.retain_mut(|conn| {
        if matches!(conn.state, ConnState::ReadingRequest) {
            loop {
                match conn.stream.read(&mut rbuf) {
                    Ok(0) => return false,
                    Ok(n) => {
                        any = true;
                        conn.inbuf.extend_from_slice(&rbuf[..n]);
                        if conn.inbuf.len() > 16 * 1024 {
                            return false;
                        }
                        if conn.inbuf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => return false,
                }
            }
            if let Some((method, path, range_hdr)) = parse_http_request(&conn.inbuf) {
                conn.inbuf.clear();
                let clean_path = path.split('?').next().unwrap_or(&path);
                if clean_path != "/stream" && clean_path != "/" {
                    write_error(&mut conn.out, "404 Not Found", "not found");
                    conn.state = ConnState::Draining;
                } else if method != "GET" && method != "HEAD" {
                    write_error(&mut conn.out, "405 Method Not Allowed", "method not allowed");
                    conn.state = ConnState::Draining;
                } else {
                    let head_only = method == "HEAD";
                    if configured {
                        conn.state = begin_request(
                            c,
                            &target,
                            &mut srv.planner,
                            file_len,
                            content_type,
                            head_only,
                            range_hdr.as_deref(),
                            &mut conn.out,
                            now,
                        );
                    } else {
                        conn.state = ConnState::WaitingForMetadata {
                            head_only,
                            range_hdr,
                            since: now,
                        };
                    }
                }
            }
        }

        if let ConnState::WaitingForMetadata { head_only, ref range_hdr, since } = conn.state {
            if configured {
                let rh = range_hdr.clone();
                conn.state = begin_request(
                    c,
                    &target,
                    &mut srv.planner,
                    file_len,
                    content_type,
                    head_only,
                    rh.as_deref(),
                    &mut conn.out,
                    now,
                );
            } else if now.saturating_sub(since) >= META_WAIT_TIMEOUT_MS {
                write_error(&mut conn.out, "503 Service Unavailable", "torrent metadata timeout");
                conn.state = ConnState::Draining;
            }
        }

        if let ConnState::Streaming { start, end, ref mut sent, partial, ref mut headers_sent } = conn.state {
            if conn.out.len() < OUT_HIGH_WATER {
                let readable = match (srv.planner.as_ref(), c.session.get(&target).and_then(|t| t.have())) {
                    (Some(p), Some(have)) => p.readable(*sent, &|piece| have.get(piece)),
                    _ => 0,
                };
                if readable > 0 {
                    if !*headers_sent {
                        let content_len = end - start + 1;
                        let hdr = if partial {
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{file_len}\r\nContent-Length: {content_len}\r\nConnection: close\r\n\r\n"
                            )
                        } else {
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Length: {content_len}\r\nConnection: close\r\n\r\n"
                            )
                        };
                        conn.out.extend_from_slice(hdr.as_bytes());
                        *headers_sent = true;
                    }
                    let remaining = (end - *sent) + 1;
                    let take = readable.min(remaining).min(CHUNK_BYTES as u64) as usize;
                    if let (Some(fidx), Some(disk)) = (file_idx, c.jobs.get_mut(&target).and_then(|j| j.disk.as_mut())) {
                        match disk.read_file_range(fidx, *sent, take) {
                            Ok(bytes) if !bytes.is_empty() => {
                                any = true;
                                conn.out.extend_from_slice(&bytes);
                                *sent += bytes.len() as u64;
                                if *sent > end {
                                    conn.state = ConnState::Draining;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        while !conn.out.is_empty() {
            match conn.stream.write(&conn.out) {
                Ok(0) => return false,
                Ok(n) => {
                    any = true;
                    conn.out.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }

        !(matches!(conn.state, ConnState::Draining) && conn.out.is_empty())
    });

    c.stream_http = Some(srv);
    any
}
