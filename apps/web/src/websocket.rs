// SPDX-License-Identifier: GPL-3.0-only
//! WebSockets for pages: the browser holds each connection (`net::Duplex`, TLS for
//! `wss:`), does the opening handshake and the framing (`sex_fetch::ws`), answers
//! pings, and tells the page that owns the socket what happened. A page's sockets
//! close when it goes away.

use super::{wall, Browser, Doc};
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::net;
use sex_fetch::ws;

/// Sockets one program holds at once.
const MAX_SOCKETS: usize = 64;
/// How often open sockets are looked at, in ms.
pub const POLL_MS: u64 = 10;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum State { Opening, Open, Closing, Closed }

pub struct Conn {
    /// The page (`Document::owner`) the socket belongs to, and its id there.
    owner: u64,
    id: u32,
    link: Option<net::Duplex>,
    key: String,
    protocols: Vec<String>,
    /// The handshake answer until its head is whole.
    head: Vec<u8>,
    reader: ws::Reader,
    state: State,
    close_sent: bool,
    /// What the page has not heard yet: dispatch text, and a message's bytes.
    events: Vec<(String, Option<Vec<u8>>)>,
    mask: u32,
}

fn random_bytes(seed: &mut u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    for b in out.iter_mut() {
        *seed ^= *seed << 13; *seed ^= *seed >> 7; *seed ^= *seed << 17;
        *b = (*seed >> 24) as u8;
    }
    out
}

impl Conn {
    fn next_mask(&mut self) -> [u8; 4] {
        self.mask ^= self.mask << 13; self.mask ^= self.mask >> 17; self.mask ^= self.mask << 5;
        self.mask.to_le_bytes()
    }
    fn fail(&mut self, window: &mut sex_app::Window, why: &str) {
        sex_pdx::serial_record!("[web.ws.failed] id={} why={}", self.id, why);
        if let Some(mut l) = self.link.take() { l.close(window); }
        if self.state != State::Closed {
            self.events.push((alloc::format!("ws\t{}\terror", self.id), None));
            self.events.push((alloc::format!("ws\t{}\tclose\t1006\t0\t", self.id), None));
        }
        self.state = State::Closed;
    }
    fn send_frame(&mut self, window: &mut sex_app::Window, op: u8, payload: &[u8]) {
        let mask = self.next_mask();
        let bytes = ws::frame(op, payload, mask);
        let ok = self.link.as_mut().map_or(false, |l| l.send(window, &bytes).is_ok());
        if !ok { self.fail(window, "send"); }
    }
    /// Takes what arrived and moves the handshake and frames on.
    fn poll(&mut self, window: &mut sex_app::Window) -> bool {
        let Some(link) = self.link.as_mut() else { return false };
        let mut got = Vec::new();
        let mut moved = false;
        let mut gone = false;
        for _ in 0..8 {
            match link.poll(window, &mut got) {
                Ok(true) => moved = true,
                Ok(false) => break,
                Err(_) => { gone = true; break; }
            }
        }
        // What came before the connection ended is read first: a server sends its close
        // frame and then closes.
        if !got.is_empty() { self.take(window, &got); moved = true; }
        if gone {
            self.link = None;
            if self.state != State::Closed { self.fail(window, "connection"); }
            return true;
        }
        moved
    }

    fn take(&mut self, window: &mut sex_app::Window, got: &[u8]) {
        let mut bytes = &got[..];
        if self.state == State::Opening {
            self.head.extend_from_slice(bytes);
            let parsed = sex_fetch::parse_head(&self.head);
            let Ok(Some((head, at))) = parsed else {
                if parsed.is_err() || self.head.len() > 16 << 10 { self.fail(window, "handshake"); }
                return;
            };
            let protocols: Vec<&str> = self.protocols.iter().map(String::as_str).collect();
            match ws::check(&head, &self.key, &protocols) {
                Ok(chosen) => {
                    self.state = State::Open;
                    sex_pdx::serial_record!("[web.ws.open] id={} protocol={}", self.id, if chosen.is_empty() { "-" } else { &chosen });
                    self.events.push((alloc::format!("ws\t{}\topen\t{}", self.id, chosen), None));
                }
                Err(why) => { self.fail(window, why); return; }
            }
            let rest = self.head.split_off(at);
            self.head = Vec::new();
            self.reader.push(&rest);
            bytes = &[];
        }
        self.reader.push(bytes);
        loop {
            match self.reader.next() {
                Ok(None) => break,
                Ok(Some(ws::Event::Text(t))) => { if self.state == State::Open { self.events.push((alloc::format!("ws\t{}\tmessage\tt", self.id), Some(t.into_bytes()))); } }
                Ok(Some(ws::Event::Binary(b))) => { if self.state == State::Open { self.events.push((alloc::format!("ws\t{}\tmessage\tb", self.id), Some(b))); } }
                Ok(Some(ws::Event::Ping(p))) => self.send_frame(window, ws::PONG, &p),
                Ok(Some(ws::Event::Pong)) => {}
                Ok(Some(ws::Event::Close(code, reason))) => {
                    if !self.close_sent { let p = ws::close_payload(if code == 1005 { 1000 } else { code }, ""); self.send_frame(window, ws::CLOSE, &p); self.close_sent = true; }
                    if let Some(mut l) = self.link.take() { l.close(window); }
                    self.state = State::Closed;
                    sex_pdx::serial_record!("[web.ws.closed] id={} code={} clean=1", self.id, code);
                    self.events.push((alloc::format!("ws\t{}\tclose\t{}\t1\t{}", self.id, code, reason.replace('\t', " ")), None));
                    break;
                }
                Err(code) => {
                    let p = ws::close_payload(code, "");
                    self.send_frame(window, ws::CLOSE, &p);
                    self.fail(window, "protocol");
                    break;
                }
            }
            if self.state == State::Closed { break; }
        }
    }
}

impl Browser {
    /// Takes the shown page's WebSocket requests: opens, sends and closes.
    pub(super) fn ws_asks(&mut self) {
        let Doc::Scripted(s) = &mut self.doc else { return };
        let doc = s.page_mut();
        if doc.ws_asks.is_empty() { return; }
        let asks = core::mem::take(&mut doc.ws_asks);
        let owner = doc.owner();
        let page = self.url.clone();
        for ask in asks {
            match ask {
                sex_js::WsAsk::Open { id, url, protocols } => {
                    let mut c = Conn { owner, id, link: None, key: String::new(), protocols: protocols.split(',').filter(|p| !p.is_empty()).map(String::from).collect(),
                        head: Vec::new(), reader: Default::default(), state: State::Opening, close_sent: false, events: Vec::new(), mask: (sex_pdx::get_ticks() as u32) ^ (id.wrapping_mul(0x9E37_79B9)) | 1 };
                    let target = ws::parse(&url);
                    let page_secure = page.starts_with("https://");
                    match target {
                        _ if self.sockets.len() >= MAX_SOCKETS => c.fail(&mut self.window, "too many"),
                        None => c.fail(&mut self.window, "address"),
                        // An https page may not open an insecure socket.
                        Some(t) if page_secure && !t.secure => c.fail(&mut self.window, "insecure"),
                        Some(t) => {
                            let mut seed = sex_pdx::get_ticks() ^ ((id as u64) << 32) ^ 0x2545_F491_4F6C_DD1D;
                            let key = random_bytes(&mut seed);
                            let origin = super::origin_of(&page).unwrap_or_else(|| String::from("null"));
                            let cookie = self.jar.borrow().header(&t, wall());
                            let protocols: Vec<&str> = c.protocols.iter().map(String::as_str).collect();
                            let (req, key) = ws::handshake(&t, &key, &origin, &protocols, cookie.as_deref());
                            c.key = key;
                            match net::Duplex::start(&mut self.window, &t.host, t.port, t.secure, req) {
                                Ok(l) => { c.link = Some(l); sex_pdx::serial_record!("[web.ws.connecting] id={} url={}", id, url); }
                                Err(_) => c.fail(&mut self.window, "connect"),
                            }
                        }
                    }
                    self.sockets.push(c);
                }
                sex_js::WsAsk::Send { id, binary, data } => {
                    let Some(c) = self.sockets.iter_mut().find(|c| c.owner == owner && c.id == id) else { continue };
                    if c.state != State::Open { continue; }
                    c.send_frame(&mut self.window, if binary { ws::BINARY } else { ws::TEXT }, &data);
                }
                sex_js::WsAsk::Close { id, code, reason } => {
                    let Some(c) = self.sockets.iter_mut().find(|c| c.owner == owner && c.id == id) else { continue };
                    match c.state {
                        State::Opening => c.fail(&mut self.window, "closed while opening"),
                        State::Open => { c.state = State::Closing; c.close_sent = true; let p = ws::close_payload(code, &reason); c.send_frame(&mut self.window, ws::CLOSE, &p); }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Moves every socket on and tells the shown page (and its frames) what happened
    /// on theirs. Sockets of pages in the background keep their news until shown.
    pub(super) fn ws_poll(&mut self) {
        if self.sockets.is_empty() { return; }
        for c in self.sockets.iter_mut() { if c.link.is_some() { c.poll(&mut self.window); } }
        self.ws_deliver();
        if self.in_frame.is_none() {
            for i in 0..self.frames.len() {
                if !self.sockets.iter().any(|c| !c.events.is_empty()) { break; }
                self.enter(i);
                self.ws_deliver();
                self.leave(i);
            }
        }
        // Closed sockets whose news was heard go.
        self.sockets.retain(|c| c.state != State::Closed || !c.events.is_empty());
    }

    fn ws_deliver(&mut self) {
        let Doc::Scripted(s) = &mut self.doc else { return };
        let owner = s.page_mut().owner();
        let mut news = Vec::new();
        for c in self.sockets.iter_mut().filter(|c| c.owner == owner) { news.extend(core::mem::take(&mut c.events).into_iter().map(|e| (c.id, e))); }
        if news.is_empty() { return; }
        for (id, (msg, bytes)) in news {
            let Some(script) = self.doc.script() else { return };
            if let Some(b) = bytes { script.page_mut().ws_in.push((id, b)); }
            let _ = script.dispatch(&msg);
            // Bytes the page did not take (no listener read them) don't pile up.
            script.page_mut().ws_in.retain(|(n, _)| *n != id);
            self.after_script(true);
            // The handler may have sent the page elsewhere.
            if self.doc.script().is_none() || self.doc.get().owner() != owner { return; }
            self.ws_asks();
        }
        self.timers_changed();
    }

    /// The page `owner` goes away: its sockets close.
    pub(super) fn ws_close_page(&mut self, owner: u64) {
        let window = &mut self.window;
        self.sockets.retain_mut(|c| {
            if c.owner != owner { return true; }
            if c.state == State::Open { let p = ws::close_payload(1001, ""); c.send_frame(window, ws::CLOSE, &p); }
            if let Some(mut l) = c.link.take() { l.close(window); }
            false
        });
    }

    /// When sockets need looking at next.
    pub(super) fn ws_due(&self) -> Option<u64> {
        self.sockets.iter().any(|c| c.link.is_some() || !c.events.is_empty()).then(|| sex_pdx::get_ticks() + POLL_MS)
    }
}
