// SPDX-License-Identifier: GPL-3.0-only
//! WebSocket (RFC 6455), client side, without I/O: the opening handshake, its check,
//! and frames both ways. The browser carries the bytes over a connection that stays
//! open (`sex_app::net::Duplex`), with TLS for `wss:`.

use crate::Url;
use alloc::string::String;
use alloc::vec::Vec;

/// Largest message a page may receive; a bigger one closes the connection (1009).
pub const MAX_MESSAGE: usize = 16 << 20;

/// `ws://` and `wss://` addresses, as the http(s) [`Url`] they connect like.
pub fn parse(url: &str) -> Option<Url> {
    let http = if let Some(r) = url.strip_prefix("ws://") { alloc::format!("http://{r}") } else if let Some(r) = url.strip_prefix("wss://") { alloc::format!("https://{r}") } else { return None };
    if http.contains('#') { return None; }
    Url::parse(&http).ok()
}

/// SHA-1, only for `Sec-WebSocket-Accept`.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = Vec::from(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 { msg.push(0); }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 { w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]); }
        for i in 16..80 { w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1); }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i { 0..=19 => ((b & c) | (!b & d), 0x5A82_7999), 20..=39 => (b ^ c ^ d, 0x6ED9_EBA1), 40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC), _ => (b ^ c ^ d, 0xCA62_C1D6) };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d; d = c; c = b.rotate_left(30); b = a; a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) { *x = x.wrapping_add(y); }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() { out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes()); }
    out
}

pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { if i <= c.len() { out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char); } else { out.push('='); } }
    }
    out
}

/// What the server must answer for `key`.
pub fn accept_for(key: &str) -> String {
    let mut s = String::from(key);
    s.push_str("258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64(&sha1(s.as_bytes()))
}

/// The opening request. `key` is 16 random bytes; `origin` the page's.
pub fn handshake(url: &Url, key: &[u8; 16], origin: &str, protocols: &[&str], cookie: Option<&str>) -> (Vec<u8>, String) {
    let key = base64(key);
    let mut r = alloc::format!("GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\nOrigin: {}\r\nUser-Agent: {}\r\n",
        url.target, url.authority(), key, origin, crate::USER_AGENT);
    if !protocols.is_empty() { r.push_str(&alloc::format!("Sec-WebSocket-Protocol: {}\r\n", protocols.join(", "))); }
    if let Some(c) = cookie { r.push_str(&alloc::format!("Cookie: {c}\r\n")); }
    r.push_str("\r\n");
    (r.into_bytes(), key)
}

/// Checks the server's answer to the handshake: the protocol it chose (maybe empty).
pub fn check(head: &crate::Head, key: &str, protocols: &[&str]) -> Result<String, &'static str> {
    if head.status != 101 { return Err("the server did not switch to WebSocket"); }
    if !head.get("upgrade").is_some_and(|u| u.trim().eq_ignore_ascii_case("websocket")) { return Err("no Upgrade: websocket"); }
    if !head.get("connection").is_some_and(|c| c.split(',').any(|t| t.trim().eq_ignore_ascii_case("upgrade"))) { return Err("no Connection: Upgrade"); }
    if head.get("sec-websocket-accept").map(str::trim) != Some(accept_for(key).as_str()) { return Err("wrong Sec-WebSocket-Accept"); }
    let chosen = head.get("sec-websocket-protocol").map(|p| String::from(p.trim())).unwrap_or_default();
    if !chosen.is_empty() && !protocols.contains(&chosen.as_str()) { return Err("a protocol that was not asked for"); }
    Ok(chosen)
}

pub const TEXT: u8 = 1;
pub const BINARY: u8 = 2;
pub const CLOSE: u8 = 8;
pub const PING: u8 = 9;
pub const PONG: u8 = 10;

/// One client frame (always masked with `mask`, always final).
pub fn frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    match payload.len() {
        n if n < 126 => out.push(0x80 | n as u8),
        n if n <= 0xFFFF => { out.push(0x80 | 126); out.extend_from_slice(&(n as u16).to_be_bytes()); }
        n => { out.push(0x80 | 127); out.extend_from_slice(&(n as u64).to_be_bytes()); }
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// A close frame's payload: code and reason.
pub fn close_payload(code: u16, reason: &str) -> Vec<u8> {
    let mut p = Vec::from(code.to_be_bytes());
    p.extend_from_slice(&reason.as_bytes()[..reason.len().min(123)]);
    p
}

/// Something that arrived from the server.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    /// The server closes: code (1005 when none) and reason.
    Close(u16, String),
}

/// Reads frames out of the bytes as they come, joining fragmented messages.
#[derive(Default)]
pub struct Reader {
    buf: Vec<u8>,
    /// A fragmented message being joined: its opcode and the bytes so far.
    partial: Option<(u8, Vec<u8>)>,
}

impl Reader {
    pub fn push(&mut self, bytes: &[u8]) { self.buf.extend_from_slice(bytes); }

    /// The next whole event, `Ok(None)` while more bytes are needed, or the close code
    /// the connection must end with (1002 protocol error, 1007 bad text, 1009 too big).
    pub fn next(&mut self) -> Result<Option<Event>, u16> {
        loop {
            let b = &self.buf;
            if b.len() < 2 { return Ok(None); }
            let (fin, rsv, op) = (b[0] & 0x80 != 0, b[0] & 0x70, b[0] & 0x0F);
            if rsv != 0 { return Err(1002); }
            // A server never masks.
            if b[1] & 0x80 != 0 { return Err(1002); }
            let (len, at) = match b[1] & 0x7F {
                126 => { if b.len() < 4 { return Ok(None); } (u16::from_be_bytes([b[2], b[3]]) as u64, 4) }
                127 => { if b.len() < 10 { return Ok(None); } (u64::from_be_bytes(b[2..10].try_into().unwrap()), 10) }
                n => (n as u64, 2),
            };
            if len > MAX_MESSAGE as u64 { return Err(1009); }
            let end = at + len as usize;
            if b.len() < end { return Ok(None); }
            let payload: Vec<u8> = b[at..end].to_vec();
            self.buf.drain(..end);
            let control = op >= 8;
            if control && (!fin || payload.len() > 125) { return Err(1002); }
            let whole = match op {
                0 => {
                    let Some((kind, mut sofar)) = self.partial.take() else { return Err(1002) };
                    if sofar.len() + payload.len() > MAX_MESSAGE { return Err(1009); }
                    sofar.extend_from_slice(&payload);
                    if !fin { self.partial = Some((kind, sofar)); continue; }
                    (kind, sofar)
                }
                TEXT | BINARY => {
                    if self.partial.is_some() { return Err(1002); }
                    if !fin { self.partial = Some((op, payload)); continue; }
                    (op, payload)
                }
                CLOSE => {
                    if payload.len() == 1 { return Err(1002); }
                    let code = if payload.len() >= 2 { u16::from_be_bytes([payload[0], payload[1]]) } else { 1005 };
                    let reason = core::str::from_utf8(payload.get(2..).unwrap_or(&[])).map_err(|_| 1007u16)?;
                    return Ok(Some(Event::Close(code, String::from(reason))));
                }
                PING => return Ok(Some(Event::Ping(payload))),
                PONG => return Ok(Some(Event::Pong)),
                _ => return Err(1002),
            };
            return match whole.0 {
                TEXT => String::from_utf8(whole.1).map(|t| Some(Event::Text(t))).map_err(|_| 1007),
                _ => Ok(Some(Event::Binary(whole.1))),
            };
        }
    }
}
