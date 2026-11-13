// SPDX-License-Identifier: GPL-3.0-only
//! An HTTP/2 client (RFC 9113) without I/O. The caller moves bytes: `output()` gives
//! what to send, `receive()` takes what came. Many requests share one connection as
//! streams; flow control, SETTINGS, PING, GOAWAY and HPACK are handled here, so the
//! whole protocol is tested on the host. The caller picks h2 by ALPN (or prior
//! knowledge on cleartext) and sends nothing on the connection before `Client::new`'s
//! preface.
//!
//! A hostile server is bounded: frames by our SETTINGS_MAX_FRAME_SIZE, header lists by
//! `HEADER_LIST_MAX`, buffered body bytes by the windows we grant.

pub mod hpack;
mod huffman_table;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
pub use hpack::Header;

pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
/// Frame types (section 6).
pub const DATA: u8 = 0;
pub const HEADERS: u8 = 1;
pub const PRIORITY: u8 = 2;
pub const RST_STREAM: u8 = 3;
pub const SETTINGS: u8 = 4;
pub const PUSH_PROMISE: u8 = 5;
pub const PING: u8 = 6;
pub const GOAWAY: u8 = 7;
pub const WINDOW_UPDATE: u8 = 8;
pub const CONTINUATION: u8 = 9;
const END_STREAM: u8 = 1;
const ACK: u8 = 1;
const END_HEADERS: u8 = 4;
const PADDED: u8 = 8;
const PRIORITY_FLAG: u8 = 0x20;
/// Error codes (section 7).
pub const NO_ERROR: u32 = 0;
pub const PROTOCOL_ERROR: u32 = 1;
pub const FLOW_CONTROL_ERROR: u32 = 3;
pub const FRAME_SIZE_ERROR: u32 = 6;
pub const REFUSED_STREAM: u32 = 7;
pub const CANCEL: u32 = 8;
pub const COMPRESSION_ERROR: u32 = 9;

/// What we tell the server: frames up to 64 KiB, 4 MiB per stream in flight and
/// 16 MiB for the connection, header lists up to 256 KiB, no server push.
pub const OUR_MAX_FRAME: usize = 65536;
pub const OUR_STREAM_WINDOW: u32 = 4 << 20;
pub const OUR_CONNECTION_WINDOW: u32 = 16 << 20;
pub const HEADER_LIST_MAX: usize = 256 << 10;
const DEFAULT_WINDOW: i64 = 65535;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The connection is broken: send nothing more on it. The code was sent in GOAWAY.
    Connection(u32),
    /// The server is going away and takes no new streams; open another connection.
    GoingAway,
    /// No such stream, or it already ended.
    Stream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The response head (and trailers, when `trailers`).
    Headers { stream: u32, status: u16, headers: Vec<Header>, trailers: bool },
    /// Body bytes, in order.
    Data { stream: u32, bytes: Vec<u8> },
    /// The response is complete.
    End { stream: u32 },
    /// The stream was reset (by the server, or by us for a protocol error).
    Reset { stream: u32, code: u32 },
    /// The server will process no stream above `last` (those may be retried elsewhere).
    GoAway { last: u32, code: u32 },
}

struct Stream {
    send_window: i64,
    /// Request body bytes still to send, and whether the request ends after them.
    body: VecDeque<u8>,
    end_after_body: bool,
    sent_end: bool,
    got_head: bool,
    /// Body bytes taken by the caller since the last WINDOW_UPDATE.
    unacked: u32,
    recv_window: i64,
}

struct Pending { headers: Vec<u8>, stream: u32 }

pub struct Client {
    out: Vec<u8>,
    input: Vec<u8>,
    events: VecDeque<Event>,
    streams: BTreeMap<u32, Stream>,
    /// Requests waiting for the server's stream limit to have room.
    queued: VecDeque<Pending>,
    next_stream: u32,
    decoder: hpack::Decoder,
    send_window: i64,
    recv_window: i64,
    conn_unacked: u32,
    peer_max_frame: usize,
    peer_initial_window: i64,
    peer_max_streams: usize,
    /// A header block in pieces: stream, end-stream flag, bytes so far.
    continuation: Option<(u32, bool, Vec<u8>)>,
    going_away: Option<u32>,
    broken: Option<u32>,
    got_settings: bool,
}

fn frame(out: &mut Vec<u8>, kind: u8, flags: u8, stream: u32, payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&(stream & 0x7fff_ffff).to_be_bytes());
    out.extend_from_slice(payload);
}

impl Default for Client { fn default() -> Self { Self::new() } }

impl Client {
    /// A connection: queues the preface, our SETTINGS and the connection window.
    pub fn new() -> Self {
        let mut c = Self {
            out: Vec::new(), input: Vec::new(), events: VecDeque::new(), streams: BTreeMap::new(), queued: VecDeque::new(),
            next_stream: 1, decoder: hpack::Decoder::new(4096, HEADER_LIST_MAX), send_window: DEFAULT_WINDOW,
            recv_window: OUR_CONNECTION_WINDOW as i64, conn_unacked: 0, peer_max_frame: 16384, peer_initial_window: DEFAULT_WINDOW,
            peer_max_streams: 100, continuation: None, going_away: None, broken: None, got_settings: false,
        };
        c.out.extend_from_slice(PREFACE);
        let mut s = Vec::new();
        for (id, v) in [(2u16, 0u32), (3, 1000), (4, OUR_STREAM_WINDOW), (5, OUR_MAX_FRAME as u32), (6, HEADER_LIST_MAX as u32)] {
            s.extend_from_slice(&id.to_be_bytes());
            s.extend_from_slice(&v.to_be_bytes());
        }
        frame(&mut c.out, SETTINGS, 0, 0, &s);
        frame(&mut c.out, WINDOW_UPDATE, 0, 0, &(OUR_CONNECTION_WINDOW - DEFAULT_WINDOW as u32).to_be_bytes());
        c
    }

    /// Bytes to send to the server now.
    pub fn output(&mut self) -> Vec<u8> { core::mem::take(&mut self.out) }
    pub fn has_output(&self) -> bool { !self.out.is_empty() }
    pub fn event(&mut self) -> Option<Event> { self.events.pop_front() }
    /// Streams open or waiting to open.
    pub fn active(&self) -> usize { self.streams.len() + self.queued.len() }
    pub fn broken(&self) -> Option<u32> { self.broken }
    /// The server's SETTINGS came: its limits are known (until then, RFC 9113's
    /// defaults are assumed, which allow 100 streams).
    pub fn settled(&self) -> bool { self.got_settings }

    /// Starts a request. `authority` is the host (and port); `headers` are lower-case
    /// regular headers (no connection-specific ones). Gives the stream number. With
    /// `body`, the request carries it, sent as the windows allow.
    pub fn request(&mut self, method: &[u8], scheme: &[u8], authority: &[u8], path: &[u8], headers: &[(&[u8], &[u8])], body: Option<&[u8]>) -> Result<u32, Error> {
        if let Some(code) = self.broken { return Err(Error::Connection(code)); }
        if self.going_away.is_some() { return Err(Error::GoingAway); }
        if self.next_stream > 0x7fff_ffff { return Err(Error::GoingAway); }
        let stream = self.next_stream;
        self.next_stream += 2;
        let mut block = Vec::new();
        let mut all: Vec<(&[u8], &[u8])> = alloc::vec![(b":method", method), (b":scheme", scheme), (b":authority", authority), (b":path", path)];
        all.extend(headers.iter().filter(|(n, _)| !matches!(*n, b"connection" | b"keep-alive" | b"proxy-connection" | b"transfer-encoding" | b"upgrade" | b"host")));
        hpack::encode(&mut block, &all);
        let s = Stream {
            send_window: self.peer_initial_window, body: body.map(|b| b.iter().copied().collect()).unwrap_or_default(), end_after_body: true,
            sent_end: false, got_head: false, unacked: 0, recv_window: OUR_STREAM_WINDOW as i64,
        };
        self.streams.insert(stream, s);
        self.queued.push_back(Pending { headers: block, stream });
        self.start_queued();
        Ok(stream)
    }

    /// Cancels a stream: RST_STREAM(CANCEL); nothing more is reported for it.
    pub fn cancel(&mut self, stream: u32) {
        if let Some(i) = self.queued.iter().position(|p| p.stream == stream) { self.queued.remove(i); self.streams.remove(&stream); return; }
        if self.streams.remove(&stream).is_some() { frame(&mut self.out, RST_STREAM, 0, stream, &CANCEL.to_be_bytes()); self.start_queued(); }
    }

    /// Sends a PING; its ACK shows the connection is alive.
    pub fn ping(&mut self, data: [u8; 8]) { frame(&mut self.out, PING, 0, 0, &data); }

    /// Closes the connection politely.
    pub fn close(&mut self) {
        let mut p = Vec::new();
        p.extend_from_slice(&0u32.to_be_bytes());
        p.extend_from_slice(&NO_ERROR.to_be_bytes());
        frame(&mut self.out, GOAWAY, 0, 0, &p);
        self.going_away = Some(0);
    }

    fn open_streams(&self) -> usize { self.streams.len() - self.queued.len() }

    fn start_queued(&mut self) {
        while self.open_streams() < self.peer_max_streams {
            let Some(p) = self.queued.pop_front() else { break };
            let no_body = self.streams.get(&p.stream).is_none_or(|s| s.body.is_empty());
            // HEADERS, then CONTINUATION frames for a block bigger than a frame.
            let mut chunks = p.headers.chunks(self.peer_max_frame).peekable();
            let first = chunks.next().unwrap_or(&[]);
            let last = chunks.peek().is_none();
            frame(&mut self.out, HEADERS, if last { END_HEADERS } else { 0 } | if no_body { END_STREAM } else { 0 }, p.stream, first);
            while let Some(c) = chunks.next() { frame(&mut self.out, CONTINUATION, if chunks.peek().is_none() { END_HEADERS } else { 0 }, p.stream, c); }
            if no_body { if let Some(s) = self.streams.get_mut(&p.stream) { s.sent_end = true; } }
        }
        self.send_bodies();
    }

    fn send_bodies(&mut self) {
        let queued: Vec<u32> = self.queued.iter().map(|p| p.stream).collect();
        for (id, s) in self.streams.iter_mut() {
            if s.sent_end || queued.contains(id) { continue; }
            while !s.body.is_empty() && s.send_window > 0 && self.send_window > 0 {
                let n = s.body.len().min(self.peer_max_frame).min(s.send_window as usize).min(self.send_window as usize);
                let chunk: Vec<u8> = s.body.drain(..n).collect();
                s.send_window -= n as i64;
                self.send_window -= n as i64;
                let end = s.body.is_empty() && s.end_after_body;
                frame(&mut self.out, DATA, if end { END_STREAM } else { 0 }, *id, &chunk);
                if end { s.sent_end = true; }
            }
        }
    }

    fn fail(&mut self, code: u32) -> Error {
        if self.broken.is_none() {
            let mut p = Vec::new();
            p.extend_from_slice(&0u32.to_be_bytes());
            p.extend_from_slice(&code.to_be_bytes());
            frame(&mut self.out, GOAWAY, 0, 0, &p);
            self.broken = Some(code);
        }
        Error::Connection(code)
    }

    fn reset(&mut self, stream: u32, code: u32) {
        if self.streams.remove(&stream).is_some() {
            frame(&mut self.out, RST_STREAM, 0, stream, &code.to_be_bytes());
            self.events.push_back(Event::Reset { stream, code });
        }
    }

    /// `n` bytes of `stream` were delivered: once half a window is used, the server
    /// is told it may send more.
    fn consumed(&mut self, stream: u32, n: u32) {
        self.conn_unacked += n;
        if self.conn_unacked >= OUR_CONNECTION_WINDOW / 2 {
            frame(&mut self.out, WINDOW_UPDATE, 0, 0, &self.conn_unacked.to_be_bytes());
            self.recv_window += self.conn_unacked as i64;
            self.conn_unacked = 0;
        }
        if let Some(s) = self.streams.get_mut(&stream) {
            s.unacked += n;
            if s.unacked >= OUR_STREAM_WINDOW / 2 {
                let u = s.unacked;
                s.recv_window += u as i64;
                s.unacked = 0;
                frame(&mut self.out, WINDOW_UPDATE, 0, stream, &u.to_be_bytes());
            }
        }
    }

    /// Takes bytes from the server. Events come out of `event()`. An error means the
    /// connection is unusable (the GOAWAY saying why is in `output()`).
    pub fn receive(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if let Some(code) = self.broken { return Err(Error::Connection(code)); }
        self.input.extend_from_slice(bytes);
        let mut at = 0;
        while self.input.len() - at >= 9 {
            let h = &self.input[at..at + 9];
            let len = u32::from_be_bytes([0, h[0], h[1], h[2]]) as usize;
            let (kind, flags) = (h[3], h[4]);
            let stream = u32::from_be_bytes([h[5], h[6], h[7], h[8]]) & 0x7fff_ffff;
            if len > OUR_MAX_FRAME { self.input.clear(); return Err(self.fail(FRAME_SIZE_ERROR)); }
            if self.input.len() - at < 9 + len { break; }
            let payload = self.input[at + 9..at + 9 + len].to_vec();
            at += 9 + len;
            if let Err(e) = self.frame_in(kind, flags, stream, &payload) { self.input.clear(); return Err(e); }
        }
        self.input.drain(..at);
        self.start_queued();
        Ok(())
    }

    fn frame_in(&mut self, kind: u8, flags: u8, stream: u32, p: &[u8]) -> Result<(), Error> {
        // The first frame from the server is its SETTINGS (section 3.4).
        if !self.got_settings && kind != SETTINGS { return Err(self.fail(PROTOCOL_ERROR)); }
        // Nothing may come between a header block's frames (section 6.10).
        if let Some((s, _, _)) = &self.continuation { if kind != CONTINUATION || stream != *s { return Err(self.fail(PROTOCOL_ERROR)); } }
        match kind {
            SETTINGS => {
                if stream != 0 { return Err(self.fail(PROTOCOL_ERROR)); }
                if flags & ACK != 0 { if !p.is_empty() { return Err(self.fail(FRAME_SIZE_ERROR)); } return Ok(()); }
                if p.len() % 6 != 0 { return Err(self.fail(FRAME_SIZE_ERROR)); }
                for s in p.chunks_exact(6) {
                    let v = u32::from_be_bytes([s[2], s[3], s[4], s[5]]);
                    match u16::from_be_bytes([s[0], s[1]]) {
                        3 => self.peer_max_streams = v as usize,
                        4 => {
                            if v > 0x7fff_ffff { return Err(self.fail(FLOW_CONTROL_ERROR)); }
                            let delta = v as i64 - self.peer_initial_window;
                            self.peer_initial_window = v as i64;
                            for st in self.streams.values_mut() { st.send_window += delta; }
                        }
                        5 => {
                            if !(16384..=16_777_215).contains(&v) { return Err(self.fail(PROTOCOL_ERROR)); }
                            self.peer_max_frame = v as usize;
                        }
                        2 if v > 1 => return Err(self.fail(PROTOCOL_ERROR)),
                        _ => {}
                    }
                }
                self.got_settings = true;
                frame(&mut self.out, SETTINGS, ACK, 0, &[]);
            }
            PING => {
                if stream != 0 { return Err(self.fail(PROTOCOL_ERROR)); }
                if p.len() != 8 { return Err(self.fail(FRAME_SIZE_ERROR)); }
                if flags & ACK == 0 { frame(&mut self.out, PING, ACK, 0, p); }
            }
            GOAWAY => {
                if p.len() < 8 { return Err(self.fail(FRAME_SIZE_ERROR)); }
                let last = u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0x7fff_ffff;
                let code = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
                self.going_away = Some(last);
                // Streams above `last` were never processed: report them reset (refused).
                let gone: Vec<u32> = self.streams.keys().copied().filter(|s| *s > last).collect();
                for s in gone { self.streams.remove(&s); self.events.push_back(Event::Reset { stream: s, code: REFUSED_STREAM }); }
                self.queued.clear();
                self.events.push_back(Event::GoAway { last, code });
            }
            WINDOW_UPDATE => {
                if p.len() != 4 { return Err(self.fail(FRAME_SIZE_ERROR)); }
                let n = (u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0x7fff_ffff) as i64;
                if stream == 0 {
                    if n == 0 { return Err(self.fail(PROTOCOL_ERROR)); }
                    self.send_window += n;
                    if self.send_window > 0x7fff_ffff { return Err(self.fail(FLOW_CONTROL_ERROR)); }
                } else if let Some(s) = self.streams.get_mut(&stream) {
                    s.send_window += n;
                    if n == 0 { self.reset(stream, PROTOCOL_ERROR); }
                    else if s.send_window > 0x7fff_ffff { self.reset(stream, FLOW_CONTROL_ERROR); }
                }
            }
            RST_STREAM => {
                if p.len() != 4 { return Err(self.fail(FRAME_SIZE_ERROR)); }
                if stream == 0 { return Err(self.fail(PROTOCOL_ERROR)); }
                if self.streams.remove(&stream).is_some() {
                    self.events.push_back(Event::Reset { stream, code: u32::from_be_bytes([p[0], p[1], p[2], p[3]]) });
                }
            }
            PUSH_PROMISE => return Err(self.fail(PROTOCOL_ERROR)), // we said ENABLE_PUSH=0
            PRIORITY => if p.len() != 5 { self.reset(stream, FRAME_SIZE_ERROR); },
            HEADERS => {
                if stream == 0 || stream % 2 == 0 { return Err(self.fail(PROTOCOL_ERROR)); }
                let mut body = unpad(flags, p).ok_or_else(|| self.fail(PROTOCOL_ERROR))?;
                if flags & PRIORITY_FLAG != 0 { body = body.get(5..).ok_or_else(|| self.fail(FRAME_SIZE_ERROR))?; }
                let block = body.to_vec();
                if flags & END_HEADERS != 0 { self.header_block(stream, flags & END_STREAM != 0, &block)?; }
                else { self.continuation = Some((stream, flags & END_STREAM != 0, block)); }
            }
            CONTINUATION => {
                let Some((s, end, mut block)) = self.continuation.take() else { return Err(self.fail(PROTOCOL_ERROR)) };
                block.extend_from_slice(p);
                if block.len() > HEADER_LIST_MAX * 2 { return Err(self.fail(PROTOCOL_ERROR)); }
                if flags & END_HEADERS != 0 { self.header_block(s, end, &block)?; } else { self.continuation = Some((s, end, block)); }
            }
            DATA => {
                if stream == 0 { return Err(self.fail(PROTOCOL_ERROR)); }
                // The whole frame, padding included, counts against the windows.
                self.recv_window -= p.len() as i64;
                if self.recv_window < 0 { return Err(self.fail(FLOW_CONTROL_ERROR)); }
                let body = unpad(flags, p).ok_or_else(|| self.fail(PROTOCOL_ERROR))?.to_vec();
                let pad = (p.len() - body.len()) as u32;
                let Some(s) = self.streams.get_mut(&stream) else {
                    // A stream we cancelled may still have data in flight: give its window back.
                    self.consumed(stream, p.len() as u32);
                    return Ok(());
                };
                s.recv_window -= p.len() as i64;
                if !s.got_head || s.recv_window < 0 { let code = if s.got_head { FLOW_CONTROL_ERROR } else { PROTOCOL_ERROR }; self.reset(stream, code); self.consumed(stream, p.len() as u32); return Ok(()); }
                if !body.is_empty() { self.events.push_back(Event::Data { stream, bytes: body.clone() }); }
                self.consumed(stream, body.len() as u32 + pad);
                if flags & END_STREAM != 0 { self.streams.remove(&stream); self.events.push_back(Event::End { stream }); }
            }
            _ => {} // Unknown frame types are ignored (section 5.5).
        }
        Ok(())
    }

    fn header_block(&mut self, stream: u32, end: bool, block: &[u8]) -> Result<(), Error> {
        // The block must be decoded even for a stream we no longer want: it changes the table.
        let headers = self.decoder.decode(block).map_err(|_| self.fail(COMPRESSION_ERROR))?;
        let Some(s) = self.streams.get_mut(&stream) else { return Ok(()) };
        let status = headers.iter().find(|(n, _)| n == b":status").and_then(|(_, v)| core::str::from_utf8(v).ok()?.parse::<u16>().ok());
        let trailers = s.got_head;
        // An informational (1xx) head is followed by the real one.
        let informational = status.is_some_and(|c| (100..200).contains(&c));
        if !trailers && status.is_none() { self.reset(stream, PROTOCOL_ERROR); return Ok(()); }
        if trailers && !end { self.reset(stream, PROTOCOL_ERROR); return Ok(()); }
        if !informational { s.got_head = true; }
        let headers: Vec<Header> = headers.into_iter().filter(|(n, _)| !n.starts_with(b":")).collect();
        self.events.push_back(Event::Headers { stream, status: status.unwrap_or(0), headers, trailers });
        if end { self.streams.remove(&stream); self.events.push_back(Event::End { stream }); }
        Ok(())
    }
}

fn unpad(flags: u8, p: &[u8]) -> Option<&[u8]> {
    if flags & PADDED == 0 { return Some(p); }
    let pad = *p.first()? as usize;
    if pad >= p.len() { return None; }
    Some(&p[1..p.len() - pad])
}

#[cfg(test)]
mod tests;
