// SPDX-License-Identifier: GPL-3.0-only
//! macOS Messages Noir Chat Client GUI for Flirt (`apps/talk`).
//!
//! Pure `#![no_std]` + `alloc` pixel renderer using `sex-font` (Geist Sans / Bold / Mono),
//! `sex-theme` rounded primitives, Obsidian Noir (`#0D1017` / `#151924` / `#252C3D`),
//! and SexOS Rose (`#F43F5E` / `#E11D48`) outgoing message bubbles.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write as _;

use sex_app::net::{self, TcpListener, TcpStream, UdpSocket};
use sex_app::{clipboard, nav, text, Event, Nav, Window};
use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_pdx::app_key::Key;
use sex_pdx::app_pointer::Kind as PKind;
use sex_theme::candy_ui::{fill_rounded_rect, mix_rgb};

const BG: u32 = 0x000D_1017;
const SIDEBAR: u32 = 0x0015_1924;
const SURFACE: u32 = 0x0015_1924;
const CARD_HOVER: u32 = 0x001B_2130;
const CARD_ACTIVE: u32 = 0x0021_283B;
const BORDER: u32 = 0x0025_2C3D;
const BUBBLE_IN: u32 = 0x001C_2230;
const BUBBLE_OUT: u32 = 0x00E1_1D48;
const ACCENT: u32 = 0x00F4_3F5E;
const ACCENT_HOVER: u32 = 0x00FB_7185;
const ACCENT_PRESSED: u32 = 0x00BE_123C;
const TEXT: u32 = 0x00F8_FAFC;
const MUTED: u32 = 0x0094_A3B8;
const DIM: u32 = 0x0064_748B;
const ONLINE: u32 = 0x0034_D399;
const WARN: u32 = 0x00FB_BF24;
const SELECT_BG: u32 = 0x0052_1B36;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    Tcp,
    Udp,
    Loopback,
}

impl Transport {
    fn label(self) -> &'static str {
        match self {
            Transport::Tcp => "TCP",
            Transport::Udp => "UDP",
            Transport::Loopback => "ECHO",
        }
    }

    fn toggle(self) -> Self {
        match self {
            Transport::Loopback => Transport::Tcp,
            Transport::Tcp => Transport::Udp,
            Transport::Udp => Transport::Loopback,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PeerStatus {
    Online,
    Ready,
    Listening,
    Offline,
}

impl PeerStatus {
    fn color(self) -> u32 {
        match self {
            PeerStatus::Online => ONLINE,
            PeerStatus::Ready => WARN,
            PeerStatus::Listening => ACCENT,
            PeerStatus::Offline => DIM,
        }
    }

    fn label(self) -> &'static str {
        match self {
            PeerStatus::Online => "Online",
            PeerStatus::Ready => "Ready",
            PeerStatus::Listening => "Listening",
            PeerStatus::Offline => "Standby",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MsgKind {
    Incoming,
    Outgoing,
    System,
}

#[derive(Clone)]
struct ChatMsg {
    kind: MsgKind,
    sender: String,
    body: String,
    time: String,
    meta: String,
}

struct Peer {
    name: String,
    initials: &'static str,
    avatar_color: u32,
    host: String,
    port: u16,
    transport: Transport,
    status: PeerStatus,
    messages: Vec<ChatMsg>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAction {
    SelectPeer(usize),
    SelectMsg(usize),
    ToggleTransport,
    ToggleListen,
    ShowAddresses,
    FocusComposer,
    Send,
    ClearInput,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    action: HitAction,
}

impl HitBox {
    fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

struct Fonts {
    sans: Option<Font<'static>>,
    bold: Option<Font<'static>>,
    mono: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
}

impl Fonts {
    fn new() -> Self {
        Self {
            sans: Font::new(sex_font::GEIST_SANS),
            bold: Font::new(sex_font::GEIST_SANS_BOLD).or_else(|| Font::new(sex_font::GEIST_SANS)),
            mono: Font::new(sex_font::GEIST_MONO).or_else(|| Font::new(sex_font::GEIST_SANS)),
            acc: Vec::new(),
            glyph: Vec::new(),
        }
    }

    fn font(&self, kind: FontKind) -> Option<&Font<'static>> {
        match kind {
            FontKind::Sans => self.sans.as_ref(),
            FontKind::Bold => self.bold.as_ref(),
            FontKind::Mono => self.mono.as_ref(),
        }
    }

    fn measure(&self, kind: FontKind, px_h: i32, s: &str) -> i32 {
        let Some(font) = self.font(kind) else {
            return (s.chars().count() as i32) * (px_h / 2).max(6);
        };
        let size = (px_h.max(10) as u32) * 64;
        let mut pen = 0i32;
        let mut prev = None;
        for ch in s.chars() {
            let Some(id) = font.glyph(ch as u32) else {
                pen += (px_h / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            pen += font.advance(id, size);
            prev = Some(id);
        }
        pen / FX
    }

    fn char_index_at_x(&self, kind: FontKind, px_h: i32, s: &str, rel_x: i32) -> usize {
        if rel_x <= 0 {
            return 0;
        }
        let Some(font) = self.font(kind) else {
            let cw = (px_h / 2).max(6);
            return ((rel_x + cw / 2) / cw).max(0) as usize;
        };
        let size = (px_h.max(10) as u32) * 64;
        let mut pen = 0i32;
        let mut prev = None;
        for (byte_idx, ch) in s.char_indices() {
            let Some(id) = font.glyph(ch as u32) else {
                let adv = (px_h / 2) * FX;
                if rel_x * FX < pen + adv / 2 {
                    return byte_idx;
                }
                pen += adv;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            let adv = font.advance(id, size);
            if rel_x * FX < pen + adv / 2 {
                return byte_idx;
            }
            pen += adv;
            prev = Some(id);
        }
        s.len()
    }

    fn wrap_lines(&self, kind: FontKind, px_h: i32, s: &str, max_w: i32) -> Vec<String> {
        let mut lines = Vec::new();
        let max_w = max_w.max(48);
        for raw_line in s.split('\n') {
            if raw_line.is_empty() {
                lines.push(String::new());
                continue;
            }
            let mut cur = String::new();
            for word in raw_line.split_inclusive(' ') {
                let candidate = if cur.is_empty() {
                    String::from(word)
                } else {
                    format!("{cur}{word}")
                };
                if self.measure(kind, px_h, candidate.trim_end()) <= max_w {
                    cur = candidate;
                } else {
                    if !cur.is_empty() {
                        lines.push(String::from(cur.trim_end()));
                        cur.clear();
                    }
                    // Break ultra-long tokens by character if needed.
                    let w_trim = word.trim_end();
                    if self.measure(kind, px_h, w_trim) <= max_w {
                        cur.push_str(word);
                    } else {
                        for ch in word.chars() {
                            let mut next = cur.clone();
                            next.push(ch);
                            if self.measure(kind, px_h, &next) > max_w && !cur.is_empty() {
                                lines.push(cur);
                                cur = String::new();
                            }
                            cur.push(ch);
                        }
                    }
                }
            }
            if !cur.is_empty() {
                lines.push(String::from(cur.trim_end()));
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        kind: FontKind,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: i32,
        text: &str,
        color: u32,
    ) -> i32 {
        let font = match kind {
            FontKind::Sans => self.sans.as_ref(),
            FontKind::Bold => self.bold.as_ref(),
            FontKind::Mono => self.mono.as_ref(),
        };
        let Some(font) = font else {
            return x;
        };
        let px_size = px_h.max(10) as usize;
        let size = (px_size * 64) as u32;
        let asc = font.ascender(size);
        let line_h = ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
        let limit = (x + max_w).min(bw as i32);
        let cr = (color >> 16) & 0xFF;
        let cg = (color >> 8) & 0xFF;
        let cb = color & 0xFF;
        let pad = 2usize;
        let mut pen = x * FX;
        let mut prev = None;
        for ch in text.chars() {
            if pen / FX >= limit - 4 {
                break;
            }
            let Some(id) = font.glyph(ch as u32) else {
                pen += (px_h / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            let adv = font.advance(id, size);
            let gx0 = (pen / FX) - pad as i32;
            let gw = ((adv / FX).max(0) as usize + 2 * pad + 2).min(64);
            if gw > 0 && line_h > 0 && line_h <= 64 {
                let slen = scratch_len(gw, line_h);
                if self.acc.len() < slen {
                    self.acc.resize(slen, 0);
                }
                if self.glyph.len() < gw * line_h {
                    self.glyph.resize(gw * line_h, 0);
                }
                self.glyph[..gw * line_h].fill(0);
                let sub = pen - (pen / FX) * FX + pad as i32 * FX;
                font.draw(
                    id,
                    size,
                    sub,
                    asc + FX,
                    gw,
                    line_h,
                    &mut self.acc[..slen],
                    &mut self.glyph[..gw * line_h],
                );
                for row in 0..line_h {
                    let py = y + row as i32;
                    if py < 0 || py >= bh as i32 {
                        continue;
                    }
                    let row_off = py as usize * bw;
                    for col in 0..gw {
                        let px = gx0 + col as i32;
                        if px < 0 || px >= limit {
                            continue;
                        }
                        let a = self.glyph[row * gw + col] as u32;
                        if a == 0 {
                            continue;
                        }
                        let dst = buf[row_off + px as usize];
                        let dr = (dst >> 16) & 0xFF;
                        let dg = (dst >> 8) & 0xFF;
                        let db = dst & 0xFF;
                        let inv = 255 - a;
                        buf[row_off + px as usize] = (((cr * a + dr * inv) / 255) << 16)
                            | (((cg * a + dg * inv) / 255) << 8)
                            | ((cb * a + db * inv) / 255);
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
        pen / FX
    }
}

#[derive(Clone, Copy)]
enum FontKind {
    Sans,
    Bold,
    Mono,
}

fn fill_bordered_card(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    fill: u32,
    border: u32,
) {
    fill_rounded_rect(buf, bw, bh, x, y, w, h, r, border);
    if w > 2 && h > 2 {
        fill_rounded_rect(buf, bw, bh, x + 1, y + 1, w - 2, h - 2, (r - 1).max(0), fill);
    }
}

fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w).clamp(0, bw as i32) as usize;
    let y1 = (y + h).clamp(0, bh as i32) as usize;
    if x0 >= x1 {
        return;
    }
    for row in y0..y1 {
        buf[row * bw + x0..row * bw + x1].fill(color);
    }
}

fn format_tick_time() -> String {
    let secs = sex_pdx::get_ticks() / 1000;
    let mins = (secs / 60) % 60;
    let rem = secs % 60;
    let mut s = String::new();
    let _ = write!(s, "{:02}:{:02}", mins, rem);
    s
}

struct ChatApp {
    peers: Vec<Peer>,
    active_peer: usize,
    selected_msg: Option<usize>,
    scroll_px: i32,
    input: String,
    cursor: usize,
    sel_anchor: Option<usize>,
    dragging_input: bool,
    input_box: (i32, i32, i32, i32),
    input_text_x: i32,
    input_scroll_x: i32,
    hover_pos: (i32, i32),
    pressed_action: Option<HitAction>,
    listener: Option<TcpListener>,
    status: String,
    hits: Vec<HitBox>,
    fonts: Fonts,
}

impl ChatApp {
    fn new() -> Self {
        let peers = vec![
            Peer {
                name: String::from("Echo Loopback"),
                initials: "EL",
                avatar_color: 0x00E1_1D48,
                host: String::from("127.0.0.1"),
                port: 7,
                transport: Transport::Loopback,
                status: PeerStatus::Online,
                messages: vec![
                    ChatMsg {
                        kind: MsgKind::System,
                        sender: String::from("System"),
                        body: String::from("Flirt Noir Messages ready. Local loopback active; toggle TCP/UDP for live sockets."),
                        time: String::from("00:00"),
                        meta: String::from("ECHO"),
                    },
                    ChatMsg {
                        kind: MsgKind::Incoming,
                        sender: String::from("Echo Loopback"),
                        body: String::from("Hey! Type a message below, paste with Ctrl+V, or switch to a TCP/UDP peer in the sidebar."),
                        time: String::from("00:01"),
                        meta: String::from("ECHO"),
                    },
                ],
            },
            Peer {
                name: String::from("QEMU Host Gateway"),
                initials: "HG",
                avatar_color: 0x0063_66F1,
                host: String::from("10.0.2.2"),
                port: 7,
                transport: Transport::Tcp,
                status: PeerStatus::Ready,
                messages: vec![ChatMsg {
                    kind: MsgKind::System,
                    sender: String::from("System"),
                    body: String::from("Target 10.0.2.2:7 (TCP echo). Requires `run +net flirt` to reach host network."),
                    time: String::from("00:00"),
                    meta: String::from("TCP"),
                }],
            },
            Peer {
                name: String::from("IPv6 Neighbor"),
                initials: "V6",
                avatar_color: 0x000EA5_E9,
                host: String::from("fd00:5e:0:1::2"),
                port: 7,
                transport: Transport::Udp,
                status: PeerStatus::Ready,
                messages: vec![ChatMsg {
                    kind: MsgKind::System,
                    sender: String::from("System"),
                    body: String::from("Target [fd00:5e:0:1::2]:7 (UDP datagram)."),
                    time: String::from("00:00"),
                    meta: String::from("UDP"),
                }],
            },
            Peer {
                name: String::from("Listener :7070"),
                initials: "L7",
                avatar_color: 0x0010_B981,
                host: String::from("0.0.0.0"),
                port: 7070,
                transport: Transport::Tcp,
                status: PeerStatus::Offline,
                messages: vec![ChatMsg {
                    kind: MsgKind::System,
                    sender: String::from("System"),
                    body: String::from("Click [Listen :7070] in the header to accept incoming TCP connections live."),
                    time: String::from("00:00"),
                    meta: String::from("TCP"),
                }],
            },
        ];

        Self {
            peers,
            active_peer: 0,
            selected_msg: None,
            scroll_px: 0,
            input: String::new(),
            cursor: 0,
            sel_anchor: None,
            dragging_input: false,
            input_box: (0, 0, 0, 0),
            input_text_x: 0,
            input_scroll_x: 0,
            hover_pos: (-1, -1),
            pressed_action: None,
            listener: None,
            status: String::from("Ready · Enter sends · Ctrl+A/C/V/X · Wheel scrolls"),
            hits: Vec::new(),
            fonts: Fonts::new(),
        }
    }

    fn sel_range(&self) -> Option<(usize, usize)> {
        let anchor = self.sel_anchor?;
        if anchor == self.cursor {
            None
        } else if anchor < self.cursor {
            Some((anchor, self.cursor))
        } else {
            Some((self.cursor, anchor))
        }
    }

    fn clear_selection(&mut self) {
        self.sel_anchor = None;
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.sel_range() else {
            return false;
        };
        self.input.replace_range(a..b, "");
        self.cursor = a;
        self.sel_anchor = None;
        true
    }

    fn insert_text(&mut self, s: &str) {
        self.delete_selection();
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() || self.input.len() + clean.len() > 1024 {
            return;
        }
        self.input.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    fn prev_char_boundary(&self, idx: usize) -> usize {
        if idx == 0 {
            return 0;
        }
        let mut p = idx - 1;
        while p > 0 && !self.input.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn next_char_boundary(&self, idx: usize) -> usize {
        if idx >= self.input.len() {
            return self.input.len();
        }
        let mut p = idx + 1;
        while p < self.input.len() && !self.input.is_char_boundary(p) {
            p += 1;
        }
        p
    }

    fn move_cursor(&mut self, next: usize, extend_sel: bool) {
        if extend_sel {
            if self.sel_anchor.is_none() {
                self.sel_anchor = Some(self.cursor);
            }
        } else {
            self.sel_anchor = None;
        }
        self.cursor = next.min(self.input.len());
    }

    fn select_peer(&mut self, idx: usize) {
        if idx < self.peers.len() {
            self.active_peer = idx;
            self.selected_msg = None;
            self.scroll_px = 0;
            let name = self.peers[idx].name.clone();
            self.status = format!("Active conversation: {name}");
        }
    }

    fn push_msg(&mut self, peer_idx: usize, kind: MsgKind, sender: &str, body: String, meta: &str) {
        let time = format_tick_time();
        self.peers[peer_idx].messages.push(ChatMsg {
            kind,
            sender: String::from(sender),
            body,
            time,
            meta: String::from(meta),
        });
        if peer_idx == self.active_peer {
            self.scroll_px = 0;
        }
    }

    fn send_current(&mut self, window: &mut Window) {
        let text_out = self.input.trim().to_string();
        if text_out.is_empty() {
            return;
        }
        self.input.clear();
        self.cursor = 0;
        self.sel_anchor = None;
        self.selected_msg = None;

        let idx = self.active_peer;
        let transport = self.peers[idx].transport;
        let host = self.peers[idx].host.clone();
        let port = self.peers[idx].port;
        let peer_name = self.peers[idx].name.clone();

        self.push_msg(idx, MsgKind::Outgoing, "You", text_out.clone(), transport.label());

        match transport {
            Transport::Loopback => {
                self.push_msg(idx, MsgKind::Incoming, &peer_name, text_out, "ECHO");
                self.status = String::from("Echoed locally.");
            }
            Transport::Tcp => {
                let mut reply = [0u8; 2048];
                let mut payload = text_out.as_bytes().to_vec();
                payload.push(b'\n');
                match TcpStream::connect(window, host.as_bytes(), port) {
                    Ok(mut stream) => {
                        let res = stream.write_all(window, &payload).and_then(|()| {
                            let mut got = 0;
                            while got < payload.len().min(reply.len()) {
                                let n = stream.read_within(window, &mut reply[got..], 2500)?;
                                if n == 0 {
                                    break;
                                }
                                got += n;
                            }
                            Ok(got)
                        });
                        stream.close(window);
                        match res {
                            Ok(n) if n > 0 => {
                                let msg = core::str::from_utf8(&reply[..n])
                                    .unwrap_or("<binary>")
                                    .trim_end_matches(['\r', '\n'])
                                    .to_string();
                                self.push_msg(idx, MsgKind::Incoming, &peer_name, msg, "TCP");
                                self.peers[idx].status = PeerStatus::Online;
                                self.status = format!("TCP reply ({n} bytes) from {host}:{port}");
                            }
                            Ok(_) => {
                                self.status = format!("TCP sent to {host}:{port} (0-byte reply)");
                            }
                            Err(e) => {
                                let err = core::str::from_utf8(net::message(e)).unwrap_or("TCP error");
                                self.push_msg(idx, MsgKind::System, "System", String::from(err), "TCP");
                                self.status = String::from(err);
                            }
                        }
                    }
                    Err(e) => {
                        let err = core::str::from_utf8(net::message(e)).unwrap_or("Connect failed");
                        self.push_msg(idx, MsgKind::System, "System", String::from(err), "TCP");
                        self.status = String::from(err);
                    }
                }
            }
            Transport::Udp => {
                let mut reply = [0u8; 2048];
                match net::resolve(window, host.as_bytes())
                    .and_then(|addrs| addrs.first().copied().ok_or(net::Error::NoName))
                    .and_then(|to| {
                        let mut sock = UdpSocket::bind(window, 0)?;
                        let began = sex_pdx::get_ticks();
                        while !sock.send_to(window, text_out.as_bytes(), &to, port)? {
                            if sex_pdx::get_ticks() - began > 2500 {
                                sock.close(window);
                                return Err(net::Error::TooSlow);
                            }
                            sex_pdx::sys_yield();
                        }
                        let r = sock.recv_from_within(window, &mut reply, 2500);
                        sock.close(window);
                        r
                    }) {
                    Ok((n, from, from_port)) => {
                        let msg = core::str::from_utf8(&reply[..n])
                            .unwrap_or("<binary>")
                            .trim_end_matches(['\r', '\n'])
                            .to_string();
                        self.push_msg(
                            idx,
                            MsgKind::Incoming,
                            &format!("[{from}]:{from_port}"),
                            msg,
                            "UDP",
                        );
                        self.peers[idx].status = PeerStatus::Online;
                        self.status = format!("UDP reply ({n} bytes) from [{from}]:{from_port}");
                    }
                    Err(e) => {
                        let err = core::str::from_utf8(net::message(e)).unwrap_or("UDP error");
                        self.push_msg(idx, MsgKind::System, "System", String::from(err), "UDP");
                        self.status = String::from(err);
                    }
                }
            }
        }
    }

    fn toggle_listen(&mut self, window: &mut Window) {
        if let Some(l) = self.listener.take() {
            l.close(window);
            if let Some(p) = self.peers.get_mut(3) {
                p.status = PeerStatus::Offline;
            }
            self.push_msg(
                self.active_peer,
                MsgKind::System,
                "System",
                String::from("Stopped TCP listener on port 7070."),
                "TCP",
            );
            self.status = String::from("Listener stopped.");
            return;
        }
        match net::up(window).and_then(|()| TcpListener::bind(window, 7070)) {
            Ok(l) => {
                let port = l.port();
                sex_pdx::serial_record!("[talk.listening] port={}", port);
                self.listener = Some(l);
                self.active_peer = 3.min(self.peers.len() - 1);
                if let Some(p) = self.peers.get_mut(self.active_peer) {
                    p.status = PeerStatus::Listening;
                }
                self.push_msg(
                    self.active_peer,
                    MsgKind::System,
                    "System",
                    format!("Listening for incoming TCP connections on port {port}."),
                    "LISTEN",
                );
                self.status = format!("Listening on TCP :{port}");
            }
            Err(e) => {
                let err = core::str::from_utf8(net::message(e)).unwrap_or("Cannot listen");
                self.push_msg(
                    self.active_peer,
                    MsgKind::System,
                    "System",
                    String::from(err),
                    "LISTEN",
                );
                self.status = String::from(err);
            }
        }
    }

    fn query_addresses(&mut self, window: &mut Window) {
        match net::up(window).and_then(|()| net::addresses(window)) {
            Ok(list) => {
                if list.is_empty() {
                    self.push_msg(
                        self.active_peer,
                        MsgKind::System,
                        "System",
                        String::from("No IP addresses assigned yet."),
                        "NET",
                    );
                } else {
                    let mut out = String::from("Local addresses: ");
                    for (i, (ip, prefix)) in list.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        let _ = write!(out, "{}/{}", ip, prefix);
                    }
                    self.push_msg(self.active_peer, MsgKind::System, "System", out, "NET");
                }
                self.status = String::from("Network interfaces queried.");
            }
            Err(e) => {
                let err = core::str::from_utf8(net::message(e)).unwrap_or("Network unavailable");
                self.push_msg(
                    self.active_peer,
                    MsgKind::System,
                    "System",
                    String::from(err),
                    "NET",
                );
                self.status = String::from(err);
            }
        }
    }

    fn poll_listener(&mut self, window: &mut Window) -> bool {
        let Some(ref mut listener) = self.listener else {
            return false;
        };
        let Ok(Some(mut stream)) = listener.accept(window) else {
            return false;
        };
        let (peer, peer_port) = stream.peer();
        let mut buf = [0u8; 2048];
        let mut msg = String::new();
        if let Ok(n) = stream.read_within(window, &mut buf, 1000) {
            if n > 0 {
                let _ = stream.write_all(window, &buf[..n]);
                msg = core::str::from_utf8(&buf[..n])
                    .unwrap_or("<binary>")
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
            }
        }
        stream.close(window);
        if !msg.is_empty() {
            let idx = 3.min(self.peers.len() - 1);
            self.push_msg(
                idx,
                MsgKind::Incoming,
                &format!("[{peer}]:{peer_port}"),
                msg,
                "TCP",
            );
            self.status = format!("Incoming message from [{peer}]:{peer_port}");
            return true;
        }
        false
    }

    fn handle_key(&mut self, window: &mut Window, key: Key) {
        let ctrl_or_meta = key.control || key.alt;
        match key.code {
            // Ctrl/Super+A: Select all composer text
            0x1E if ctrl_or_meta => {
                if !self.input.is_empty() {
                    self.sel_anchor = Some(0);
                    self.cursor = self.input.len();
                    self.status = String::from("Selected all composer text.");
                }
            }
            // Ctrl/Super+C: Copy selection, whole input, or selected message bubble
            0x2E if ctrl_or_meta => {
                if let Some((a, b)) = self.sel_range() {
                    let clip = self.input[a..b].to_string();
                    let _ = clipboard::copy(window, &clip);
                    self.status = format!("Copied {} bytes.", clip.len());
                } else if !self.input.is_empty() {
                    let _ = clipboard::copy(window, &self.input);
                    self.status = format!("Copied composer ({} bytes).", self.input.len());
                } else if let Some(mi) = self.selected_msg {
                    if let Some(m) = self.peers[self.active_peer].messages.get(mi) {
                        let _ = clipboard::copy(window, &m.body);
                        self.status = String::from("Copied selected message.");
                    }
                }
            }
            // Ctrl/Super+X: Cut selection
            0x2D if ctrl_or_meta => {
                if let Some((a, b)) = self.sel_range() {
                    let clip = self.input[a..b].to_string();
                    let _ = clipboard::copy(window, &clip);
                    self.delete_selection();
                    self.status = format!("Cut {} bytes.", clip.len());
                }
            }
            // Ctrl/Super+V: Paste from clipboard
            0x2F if ctrl_or_meta => {
                if let Ok(clip) = clipboard::paste(window) {
                    let s = clip.to_string();
                    self.insert_text(&s);
                    self.status = format!("Pasted {} bytes.", s.len());
                }
            }
            // Enter: send
            0x1C => self.send_current(window),
            // Backspace
            0x0E => {
                if !self.delete_selection() && self.cursor > 0 {
                    let prev = self.prev_char_boundary(self.cursor);
                    self.input.replace_range(prev..self.cursor, "");
                    self.cursor = prev;
                }
            }
            // Tab: cycle peers
            0x0F => {
                let next = (self.active_peer + 1) % self.peers.len();
                self.select_peer(next);
            }
            _ => match nav(key) {
                Some(Nav::Left) => {
                    let p = self.prev_char_boundary(self.cursor);
                    self.move_cursor(p, key.shift);
                }
                Some(Nav::Right) => {
                    let n = self.next_char_boundary(self.cursor);
                    self.move_cursor(n, key.shift);
                }
                Some(Nav::Home) => self.move_cursor(0, key.shift),
                Some(Nav::End) => {
                    let end = self.input.len();
                    self.move_cursor(end, key.shift);
                }
                Some(Nav::Delete) => {
                    if !self.delete_selection() && self.cursor < self.input.len() {
                        let next = self.next_char_boundary(self.cursor);
                        self.input.replace_range(self.cursor..next, "");
                    }
                }
                Some(Nav::Up) => {
                    if key.control {
                        let prev = self.active_peer.saturating_sub(1);
                        self.select_peer(prev);
                    } else {
                        self.scroll_px = self.scroll_px.saturating_add(48);
                    }
                }
                Some(Nav::Down) => {
                    if key.control {
                        let next = (self.active_peer + 1).min(self.peers.len() - 1);
                        self.select_peer(next);
                    } else {
                        self.scroll_px = (self.scroll_px - 48).max(0);
                    }
                }
                None => {
                    if let Some(ch) = text(key) {
                        if !ch.is_control() {
                            let mut buf = [0u8; 4];
                            let s = ch.encode_utf8(&mut buf);
                            self.insert_text(s);
                        }
                    }
                }
            },
        }
    }

    fn hit_at(&self, px: i32, py: i32) -> Option<HitAction> {
        self.hits
            .iter()
            .rev()
            .find(|h| h.contains(px, py))
            .map(|h| h.action)
    }

    fn handle_pointer(&mut self, window: &mut Window, p: sex_pdx::app_pointer::Pointer) -> bool {
        let px = p.x as i32;
        let py = p.y as i32;
        match p.kind {
            PKind::Move => {
                self.hover_pos = (px, py);
                if self.dragging_input {
                    let rel = px - self.input_text_x + self.input_scroll_x;
                    self.cursor = self
                        .fonts
                        .char_index_at_x(FontKind::Sans, 13, &self.input, rel)
                        .min(self.input.len());
                    return true;
                }
                false
            }
            PKind::Leave => {
                self.hover_pos = (-1, -1);
                self.dragging_input = false;
                true
            }
            PKind::Wheel { steps } => {
                if steps > 0 {
                    self.scroll_px = self.scroll_px.saturating_add(steps as i32 * 40);
                } else if steps < 0 {
                    self.scroll_px = (self.scroll_px + steps as i32 * 40).max(0);
                }
                true
            }
            PKind::Down { button: 0 } => {
                let act = self.hit_at(px, py);
                self.pressed_action = act;
                if let Some(HitAction::FocusComposer) = act {
                    let rel = px - self.input_text_x + self.input_scroll_x;
                    let idx = self
                        .fonts
                        .char_index_at_x(FontKind::Sans, 13, &self.input, rel)
                        .min(self.input.len());
                    self.cursor = idx;
                    self.sel_anchor = Some(idx);
                    self.dragging_input = true;
                }
                true
            }
            PKind::Up { button: 0 } => {
                self.dragging_input = false;
                let released_on = self.hit_at(px, py);
                let pressed = self.pressed_action.take();
                if let Some(HitAction::FocusComposer) = pressed {
                    if self.sel_anchor == Some(self.cursor) {
                        self.sel_anchor = None;
                    }
                    return true;
                }
                if pressed.is_some() && pressed == released_on {
                    match pressed.unwrap() {
                        HitAction::SelectPeer(i) => self.select_peer(i),
                        HitAction::SelectMsg(i) => {
                            self.selected_msg = if self.selected_msg == Some(i) { None } else { Some(i) };
                            self.status = String::from("Message selected (Ctrl+C to copy).");
                        }
                        HitAction::ToggleTransport => {
                            let peer = &mut self.peers[self.active_peer];
                            peer.transport = peer.transport.toggle();
                            let lbl = peer.transport.label();
                            self.status = format!("Transport switched to {lbl}.");
                        }
                        HitAction::ToggleListen => self.toggle_listen(window),
                        HitAction::ShowAddresses => self.query_addresses(window),
                        HitAction::Send => self.send_current(window),
                        HitAction::ClearInput => {
                            self.input.clear();
                            self.cursor = 0;
                            self.clear_selection();
                        }
                        HitAction::FocusComposer => {}
                    }
                    return true;
                }
                pressed.is_some()
            }
            _ => false,
        }
    }

    fn draw_pill(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        h: i32,
        label: &str,
        font: FontKind,
        primary: bool,
        active: bool,
        action: HitAction,
    ) -> i32 {
        let tw = self.fonts.measure(font, 12, label);
        let w = (tw + 22).max(52);
        let hovered = x <= self.hover_pos.0
            && self.hover_pos.0 < x + w
            && y <= self.hover_pos.1
            && self.hover_pos.1 < y + h;
        let pressed = self.pressed_action == Some(action);
        let (fill, border, fg) = if primary {
            let f = if pressed {
                ACCENT_PRESSED
            } else if hovered {
                ACCENT_HOVER
            } else {
                ACCENT
            };
            (f, ACCENT, 0x00FF_FFFF)
        } else if active {
            (0x002B_1524, ACCENT, ACCENT)
        } else {
            let f = if pressed {
                CARD_ACTIVE
            } else if hovered {
                CARD_HOVER
            } else {
                SURFACE
            };
            (f, BORDER, TEXT)
        };
        fill_bordered_card(buf, bw, bh, x, y, w, h, h / 2, fill, border);
        self.fonts
            .draw(font, buf, bw, bh, x + (w - tw) / 2, y + (h - 14) / 2, w - 8, 12, label, fg);
        self.hits.push(HitBox { x, y, w, h, action });
        w
    }

    fn render(&mut self, window: &mut Window) {
        if !window.ready() {
            return;
        }
        let (w, h) = window.size();
        if w < 240 || h < 180 {
            return;
        }
        let buf = window.frame(w, h);
        buf[..w * h].fill(BG);
        self.hits.clear();

        let wi = w as i32;
        let hi = h as i32;
        let sidebar_w = (wi * 27 / 100).clamp(185, 250);

        // Left Sidebar background + right hairline border
        fill_rect(buf, w, h, 0, 0, sidebar_w, hi, SIDEBAR);
        fill_rect(buf, w, h, sidebar_w - 1, 0, 1, hi, BORDER);

        // Sidebar Title & Pill
        self.fonts
            .draw(FontKind::Bold, buf, w, h, 16, 16, sidebar_w - 86, 16, "Messages", TEXT);
        let peer_cnt = format!("{} peers", self.peers.len());
        let cnt_w = self.fonts.measure(FontKind::Mono, 11, &peer_cnt) + 14;
        fill_bordered_card(
            buf,
            w,
            h,
            sidebar_w - cnt_w - 14,
            15,
            cnt_w,
            22,
            11,
            0x001C_2230,
            BORDER,
        );
        self.fonts.draw(
            FontKind::Mono,
            buf,
            w,
            h,
            sidebar_w - cnt_w - 7,
            19,
            cnt_w,
            11,
            &peer_cnt,
            MUTED,
        );
        fill_rect(buf, w, h, 0, 52, sidebar_w - 1, 1, BORDER);

        // Sidebar Peer Rows
        let mut sy = 62;
        let row_h = 58;
        for i in 0..self.peers.len() {
            if sy + row_h > hi - 26 {
                break;
            }
            let rx = 8;
            let rw = sidebar_w - 17;
            let active = i == self.active_peer;
            let hovered = rx <= self.hover_pos.0
                && self.hover_pos.0 < rx + rw
                && sy <= self.hover_pos.1
                && self.hover_pos.1 < sy + row_h;
            let (row_fill, row_border) = if active {
                (CARD_ACTIVE, ACCENT)
            } else if hovered {
                (CARD_HOVER, BORDER)
            } else {
                (SIDEBAR, SIDEBAR)
            };
            if active || hovered {
                fill_bordered_card(buf, w, h, rx, sy, rw, row_h - 4, 10, row_fill, row_border);
            }
            // Avatar circle + status dot
            let av_x = rx + 10;
            let av_y = sy + 10;
            fill_rounded_rect(buf, w, h, av_x, av_y, 34, 34, 17, self.peers[i].avatar_color);
            let init = self.peers[i].initials;
            let iw = self.fonts.measure(FontKind::Bold, 12, init);
            self.fonts
                .draw(FontKind::Bold, buf, w, h, av_x + (34 - iw) / 2, av_y + 10, 32, 12, init, 0x00FF_FFFF);
            let dot_col = self.peers[i].status.color();
            fill_rounded_rect(buf, w, h, av_x + 24, av_y + 24, 11, 11, 5, SIDEBAR);
            fill_rounded_rect(buf, w, h, av_x + 26, av_y + 26, 7, 7, 3, dot_col);

            let tx = av_x + 42;
            let tw = rw - 54;
            let tlabel = self.peers[i].transport.label();
            let tbadge_w = self.fonts.measure(FontKind::Mono, 10, tlabel) + 10;
            self.fonts.draw(
                FontKind::Bold,
                buf,
                w,
                h,
                tx,
                sy + 10,
                tw - tbadge_w - 4,
                13,
                &self.peers[i].name,
                TEXT,
            );
            fill_rounded_rect(
                buf,
                w,
                h,
                rx + rw - tbadge_w - 8,
                sy + 10,
                tbadge_w,
                17,
                5,
                0x0013_1722,
            );
            self.fonts.draw(
                FontKind::Mono,
                buf,
                w,
                h,
                rx + rw - tbadge_w - 3,
                sy + 12,
                tbadge_w,
                10,
                tlabel,
                if active { ACCENT } else { MUTED },
            );

            let preview = self.peers[i]
                .messages
                .last()
                .map(|m| m.body.as_str())
                .unwrap_or("No messages");
            self.fonts
                .draw(FontKind::Sans, buf, w, h, tx, sy + 29, tw, 11, preview, MUTED);

            self.hits.push(HitBox {
                x: rx,
                y: sy,
                w: rw,
                h: row_h - 4,
                action: HitAction::SelectPeer(i),
            });
            sy += row_h;
        }

        // Sidebar footer: Flirt brand
        self.fonts
            .draw(FontKind::Mono, buf, w, h, 14, hi - 22, sidebar_w - 24, 11, "FLIRT · NOIR", DIM);

        // Right Pane Header Bar
        let cx = sidebar_w;
        let cw = wi - sidebar_w;
        fill_rect(buf, w, h, cx, 0, cw, 52, 0x0011_151F);
        fill_rect(buf, w, h, cx, 52, cw, 1, BORDER);

        let peer_name = self.peers[self.active_peer].name.clone();
        let peer_addr = format!(
            "{}:{} · {}",
            self.peers[self.active_peer].host,
            self.peers[self.active_peer].port,
            self.peers[self.active_peer].status.label()
        );
        let peer_dot = self.peers[self.active_peer].status.color();
        let peer_init = self.peers[self.active_peer].initials;
        let peer_col = self.peers[self.active_peer].avatar_color;
        let peer_transport = self.peers[self.active_peer].transport.label();

        fill_rounded_rect(buf, w, h, cx + 16, 11, 30, 30, 15, peer_col);
        let piw = self.fonts.measure(FontKind::Bold, 11, peer_init);
        self.fonts.draw(
            FontKind::Bold,
            buf,
            w,
            h,
            cx + 16 + (30 - piw) / 2,
            19,
            28,
            11,
            peer_init,
            0x00FF_FFFF,
        );
        fill_rounded_rect(buf, w, h, cx + 38, 33, 8, 8, 4, peer_dot);

        // Action pills on top right
        let mut pill_x = wi - 14;
        let addrs_lbl = "Addrs";
        let addrs_w = self.fonts.measure(FontKind::Sans, 12, addrs_lbl) + 22;
        pill_x -= addrs_w;
        self.draw_pill(
            buf,
            w,
            h,
            pill_x,
            13,
            26,
            addrs_lbl,
            FontKind::Sans,
            false,
            false,
            HitAction::ShowAddresses,
        );

        let listen_lbl = if self.listener.is_some() {
            "Listening :7070"
        } else {
            "Listen :7070"
        };
        let listen_w = self.fonts.measure(FontKind::Sans, 12, listen_lbl) + 22;
        pill_x -= listen_w + 8;
        self.draw_pill(
            buf,
            w,
            h,
            pill_x,
            13,
            26,
            listen_lbl,
            FontKind::Sans,
            false,
            self.listener.is_some(),
            HitAction::ToggleListen,
        );

        let tr_w = self.fonts.measure(FontKind::Mono, 12, peer_transport) + 22;
        pill_x -= tr_w + 8;
        self.draw_pill(
            buf,
            w,
            h,
            pill_x,
            13,
            26,
            peer_transport,
            FontKind::Mono,
            false,
            true,
            HitAction::ToggleTransport,
        );

        let title_max = (pill_x - (cx + 58)).max(60);
        self.fonts
            .draw(FontKind::Bold, buf, w, h, cx + 54, 10, title_max, 14, &peer_name, TEXT);
        self.fonts
            .draw(FontKind::Mono, buf, w, h, cx + 54, 29, title_max, 11, &peer_addr, MUTED);

        // Chat Timeline Area
        let composer_h = 80;
        let feed_top = 56;
        let feed_bot = hi - composer_h;
        let max_bubble_w = (cw * 64 / 100).clamp(160, 480);

        // Pre-layout bubbles bottom-up
        struct LaidMsg {
            idx: usize,
            kind: MsgKind,
            lines: Vec<String>,
            box_w: i32,
            box_h: i32,
            meta_line: String,
        }

        let msgs = self.peers[self.active_peer].messages.clone();
        let mut laid = Vec::with_capacity(msgs.len());
        let mut total_h = 16i32;
        for (mi, m) in msgs.iter().enumerate() {
            let font_kind = FontKind::Sans;
            let wrap_w = if m.kind == MsgKind::System {
                (cw - 64).max(120)
            } else {
                max_bubble_w - 28
            };
            let lines = self.fonts.wrap_lines(font_kind, 13, &m.body, wrap_w);
            let mut text_w = 0i32;
            for line in &lines {
                text_w = text_w.max(self.fonts.measure(font_kind, 13, line));
            }
            let meta_line = if m.kind == MsgKind::System {
                format!("{} · {}", m.meta, m.time)
            } else {
                format!("{} · {} · {}", m.sender, m.meta, m.time)
            };
            let meta_w = self.fonts.measure(FontKind::Mono, 10, &meta_line);
            let box_w = if m.kind == MsgKind::System {
                (text_w + 28).clamp(120, cw - 36)
            } else {
                (text_w.max(meta_w) + 28).clamp(88, max_bubble_w)
            };
            let box_h = if m.kind == MsgKind::System {
                lines.len() as i32 * 18 + 14
            } else {
                lines.len() as i32 * 19 + 30
            };
            total_h += box_h + 10;
            laid.push(LaidMsg {
                idx: mi,
                kind: m.kind,
                lines,
                box_w,
                box_h,
                meta_line,
            });
        }

        let visible_h = (feed_bot - feed_top).max(40);
        let max_scroll = (total_h - visible_h).max(0);
        self.scroll_px = self.scroll_px.clamp(0, max_scroll);
        let mut cur_y = feed_bot - 12 + self.scroll_px;

        for item in laid.iter().rev() {
            cur_y -= item.box_h;
            let top_y = cur_y;
            let bot_y = cur_y + item.box_h;
            cur_y -= 10;

            if bot_y <= feed_top || top_y >= feed_bot {
                continue;
            }
            // Clip vertically inside feed region
            let draw_y = top_y.max(feed_top);
            let draw_h = (bot_y.min(feed_bot) - draw_y).max(0);
            if draw_h < 8 {
                continue;
            }

            let bx = match item.kind {
                MsgKind::Incoming => cx + 18,
                MsgKind::Outgoing => wi - 18 - item.box_w,
                MsgKind::System => cx + (cw - item.box_w) / 2,
            };
            let selected = self.selected_msg == Some(item.idx);
            let (fill, border, fg, meta_fg) = match item.kind {
                MsgKind::Incoming => (
                    BUBBLE_IN,
                    if selected { ACCENT } else { BORDER },
                    TEXT,
                    MUTED,
                ),
                MsgKind::Outgoing => (
                    BUBBLE_OUT,
                    if selected { 0x00FF_FFFF } else { ACCENT },
                    0x00FF_FFFF,
                    0x00FD_A4AF,
                ),
                MsgKind::System => (
                    SURFACE,
                    if selected { ACCENT } else { BORDER },
                    MUTED,
                    DIM,
                ),
            };
            if top_y >= feed_top && bot_y <= feed_bot {
                fill_bordered_card(buf, w, h, bx, top_y, item.box_w, item.box_h, 14, fill, border);
                let mut ly = top_y + 9;
                for line in &item.lines {
                    self.fonts.draw(
                        FontKind::Sans,
                        buf,
                        w,
                        h,
                        bx + 14,
                        ly,
                        item.box_w - 28,
                        13,
                        line,
                        fg,
                    );
                    ly += if item.kind == MsgKind::System { 18 } else { 19 };
                }
                if item.kind != MsgKind::System {
                    self.fonts.draw(
                        FontKind::Mono,
                        buf,
                        w,
                        h,
                        bx + 14,
                        top_y + item.box_h - 17,
                        item.box_w - 28,
                        10,
                        &item.meta_line,
                        meta_fg,
                    );
                }
            }
            self.hits.push(HitBox {
                x: bx,
                y: draw_y,
                w: item.box_w,
                h: draw_h,
                action: HitAction::SelectMsg(item.idx),
            });
        }

        // Bottom Rounded Composer Bar
        fill_rect(buf, w, h, cx, feed_bot, cw, composer_h, BG);
        fill_rect(buf, w, h, cx, feed_bot, cw, 1, BORDER);

        let send_w = 78;
        let clear_w = if self.input.is_empty() { 0 } else { 54 };
        let well_x = cx + 16;
        let well_y = feed_bot + 10;
        let well_h = 38;
        let well_w = (cw - 32 - send_w - 10 - if clear_w > 0 { clear_w + 8 } else { 0 }).max(100);
        self.input_box = (well_x, well_y, well_w, well_h);
        self.input_text_x = well_x + 14;

        let well_border = if !self.input.is_empty() || self.dragging_input {
            ACCENT
        } else {
            BORDER
        };
        fill_bordered_card(buf, w, h, well_x, well_y, well_w, well_h, 19, SURFACE, well_border);

        let inner_w = (well_w - 28).max(20);
        let cursor_px = self
            .fonts
            .measure(FontKind::Sans, 13, &self.input[..self.cursor.min(self.input.len())]);
        if cursor_px - self.input_scroll_x > inner_w - 8 {
            self.input_scroll_x = cursor_px - inner_w + 8;
        } else if cursor_px < self.input_scroll_x {
            self.input_scroll_x = cursor_px;
        }
        if self.input_scroll_x < 0 {
            self.input_scroll_x = 0;
        }

        let text_y = well_y + 11;
        if self.input.is_empty() {
            let ph = format!("Message {peer_name} ({})...", peer_transport);
            self.fonts
                .draw(FontKind::Sans, buf, w, h, self.input_text_x, text_y, inner_w, 13, &ph, DIM);
        } else {
            // Selection highlight
            if let Some((a, b)) = self.sel_range() {
                let ax = self.fonts.measure(FontKind::Sans, 13, &self.input[..a]) - self.input_scroll_x;
                let bx = self.fonts.measure(FontKind::Sans, 13, &self.input[..b]) - self.input_scroll_x;
                let sx0 = (self.input_text_x + ax).max(self.input_text_x);
                let sx1 = (self.input_text_x + bx).min(self.input_text_x + inner_w);
                if sx1 > sx0 {
                    fill_rounded_rect(buf, w, h, sx0, well_y + 7, sx1 - sx0, well_h - 14, 4, SELECT_BG);
                }
            }
            // Visible slice from scroll_x
            let start_idx = if self.input_scroll_x == 0 {
                0
            } else {
                self.fonts
                    .char_index_at_x(FontKind::Sans, 13, &self.input, self.input_scroll_x)
            };
            let visible = &self.input[start_idx.min(self.input.len())..];
            self.fonts
                .draw(FontKind::Sans, buf, w, h, self.input_text_x, text_y, inner_w, 13, visible, TEXT);
        }

        // Caret
        let caret_x = self.input_text_x + cursor_px - self.input_scroll_x;
        if caret_x >= self.input_text_x && caret_x < self.input_text_x + inner_w {
            fill_rect(buf, w, h, caret_x, well_y + 9, 2, well_h - 18, ACCENT);
        }

        self.hits.push(HitBox {
            x: well_x,
            y: well_y,
            w: well_w,
            h: well_h,
            action: HitAction::FocusComposer,
        });

        let mut btn_x = well_x + well_w + 8;
        if clear_w > 0 {
            self.draw_pill(
                buf,
                w,
                h,
                btn_x,
                well_y + 2,
                34,
                "Clear",
                FontKind::Sans,
                false,
                false,
                HitAction::ClearInput,
            );
            btn_x += clear_w + 8;
        }
        self.draw_pill(
            buf,
            w,
            h,
            btn_x,
            well_y + 2,
            34,
            "Send",
            FontKind::Bold,
            true,
            false,
            HitAction::Send,
        );

        // Status line under composer
        self.fonts.draw(
            FontKind::Mono,
            buf,
            w,
            h,
            cx + 18,
            hi - 22,
            cw - 36,
            11,
            &self.status,
            DIM,
        );

        // Subtle focus highlight on hover border if needed (already handled per control).
        let _ = mix_rgb(BG, SURFACE, 128);
        window.show_pixels(w, h);
    }
}

pub fn run_messages_ui(mut window: Window) -> ! {
    let mut app = ChatApp::new();
    app.render(&mut window);
    loop {
        let mut dirty = app.poll_listener(&mut window);
        if let Some(ev) = window.next_within(60) {
            if handle_event(&mut app, &mut window, ev) {
                dirty = true;
            }
            for _ in 0..32 {
                let Some(ev) = window.poll() else { break };
                if handle_event(&mut app, &mut window, ev) {
                    dirty = true;
                }
            }
        }
        if dirty {
            app.render(&mut window);
        }
    }
}

fn handle_event(app: &mut ChatApp, window: &mut Window, ev: Event) -> bool {
    match ev {
        Event::Close => {
            if let Some(l) = app.listener.take() {
                l.close(window);
            }
            sex_pdx::serial_record!("[talk.done] target={} task={}", window.target, window.task);
            sex_app::launch::exit(0);
        }
        Event::Resize { .. } => true,
        Event::Key(k) => {
            app.handle_key(window, k);
            true
        }
        Event::Pointer(p) => app.handle_pointer(window, p),
        Event::Drop { .. } | Event::DragEnded { .. } | Event::KeyUp(_) => false,
    }
}
