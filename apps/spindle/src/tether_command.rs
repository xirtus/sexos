// SPDX-License-Identifier: GPL-3.0-only
//! `tether HOST[:PORT]`: lets a helper on another computer watch and drive this Spindle
//! during the first starts on new hardware (the Alienware x17 R1 has no serial port).
//!
//! Spindle connects *out* to HOST (default port 7177) over the ordinary network: nothing
//! listens on SexOS, and only the person at the keyboard can start it. Over that one TCP
//! flow it sends, as plain text lines:
//!
//! 1. `SEXOS-TETHER 1` and the kernel's copy of the boot log head (the first 256 KiB,
//!    the same text `hw all` reads), between `--- boot log head ---` markers;
//! 2. every line Spindle has shown (what is still in its scrollback), then every new line
//!    as it appears.
//!
//! Each line the other side sends back is run as if typed, and shown first as
//! `remote> LINE`, so the person sees every command. A few commands that write disks or
//! boot entries (`install go`, `update`, `format`) are refused from the tether: those stay
//! the person's to type. `tether off` or closing Spindle ends it; `tether`
//! alone says whether it is up. The host side is `scripts/tether_host.py`.
//!
//! The flow is the network service's flow `FLOW` of this Spindle, so it never collides
//! with `fetch` (flow 0); the shared region still carries one call at a time, so `fetch`,
//! `update` and `pkg` wait for `tether off`.

use crate::Scrollback;
use sex_pdx::crash_log as log;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::web as w;

const FLOW: u8 = 1;
/// `tether listen`: the listening flow (the connection it accepts becomes `FLOW`).
const LISTEN_FLOW: u8 = 2;
/// Between asks whether the Mac has connected.
const ACCEPT_MS: u64 = 250;
pub const DEFAULT_PORT: u16 = 7177;
/// Between receives while nothing arrives.
const POLL_MS: u64 = 40;
const OUT_MAX: usize = 16 * 1024;
const IN_MAX: usize = 2048;
const QUEUE: usize = 16;
pub const CMD_MAX: usize = 256;
/// Empty receives between asks whether the other side is still there.
const STATE_EVERY: u32 = 25;
/// Connecting gives up after this long.
const CONNECT_MS: u64 = 20_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase { Idle, Up, ResolveAsk, ResolveWait, Connect, Opening, Live, Close,
    /// `tether listen`: make the listener, say the address, wait for the Mac, and drop a
    /// finished connection to wait for the next one.
    Listen, Addresses, Accept, Drop, CloseListener }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Call { Plain, Send, Receive, State, Close }

pub struct Tether {
    phase: Phase,
    seq: u32,
    sent: bool,
    call: Call,
    next_at: u64,
    deadline: u64,
    host: [u8; 128],
    host_len: usize,
    port: u16,
    address: [u8; 4],
    out: [u8; OUT_MAX],
    out_len: usize,
    /// The boot log head's next offset to send; `usize::MAX` once it all went.
    log_at: usize,
    log_total: usize,
    /// Scrollback lines (counted since Spindle started) already sent.
    lines_sent: u32,
    inbound: [u8; IN_MAX],
    in_len: usize,
    queue: [[u8; CMD_MAX]; QUEUE],
    queue_len: [usize; QUEUE],
    queue_head: usize,
    queue_count: usize,
    empty_receives: u32,
    recv_turn: bool,
    bytes_out: u64,
    commands: u64,
    /// `tether listen`: SexOS waits and the Mac connects (no typing needed to aim it).
    listening: bool,
}

pub static mut TETHER: Tether = Tether {
    phase: Phase::Idle, seq: 0, sent: false, call: Call::Plain, next_at: 0, deadline: 0,
    host: [0; 128], host_len: 0, port: DEFAULT_PORT, address: [0; 4],
    out: [0; OUT_MAX], out_len: 0, log_at: 0, log_total: 0, lines_sent: 0,
    inbound: [0; IN_MAX], in_len: 0, queue: [[0; CMD_MAX]; QUEUE], queue_len: [0; QUEUE],
    queue_head: 0, queue_count: 0, empty_receives: 0, recv_turn: false, bytes_out: 0, commands: 0, listening: false,
};

fn net() -> Option<&'static mut [u8]> {
    let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
}

/// `a.b.c.d` -> its four bytes.
pub fn numeric(host: &[u8]) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = host.split(|b| *b == b'.');
    for slot in out.iter_mut() {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.iter().all(u8::is_ascii_digit) { return None; }
        let value = part.iter().fold(0u32, |v, b| v * 10 + (*b - b'0') as u32);
        if value > 255 { return None; }
        *slot = value as u8;
    }
    if parts.next().is_some() { return None; }
    Some(out)
}

/// `HOST[:PORT]` -> (host, port).
pub fn parse_target(arg: &[u8]) -> Option<(&[u8], u16)> {
    let (host, port) = match arg.iter().rposition(|b| *b == b':') {
        Some(i) => {
            let digits = &arg[i + 1..];
            if digits.is_empty() || digits.len() > 5 || !digits.iter().all(u8::is_ascii_digit) { return None; }
            let p = digits.iter().fold(0u32, |v, b| v * 10 + (*b - b'0') as u32);
            if !(1..=65535).contains(&p) { return None; }
            (&arg[..i], p as u16)
        }
        None => (arg, DEFAULT_PORT),
    };
    if host.is_empty() || host.len() > 127 || host.iter().any(|b| *b <= b' ') { return None; }
    Some((host, port))
}

fn parse_port(digits: &[u8]) -> Option<u16> {
    if digits.is_empty() || digits.len() > 5 || !digits.iter().all(u8::is_ascii_digit) { return None; }
    let p = digits.iter().fold(0u32, |v, b| v * 10 + (*b - b'0') as u32);
    if (1..=65535).contains(&p) { Some(p as u16) } else { None }
}

/// Commands the tether may not run: they write disks or boot entries.
pub fn refused(line: &[u8]) -> bool {
    let mut words = line.split(|b| b.is_ascii_whitespace()).filter(|w| !w.is_empty());
    match words.next() {
        // Files runs the install when `go` is any of its words.
        Some(b"install") => words.any(|w| w == b"go"),
        Some(b"update") | Some(b"format") | Some(b"mkfs") => true,
        _ => false,
    }
}

struct Text { buf: [u8; 160], len: usize }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
macro_rules! say { ($sb:expr, $($arg:tt)*) => {{
    let mut t = Text { buf: [0; 160], len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!($($arg)*));
    $sb.push(&t.buf[..t.len]);
    sex_pdx::serial_record!("[spindle.tether.line] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}

impl Tether {
    /// Connecting or connected.
    pub fn active(&self) -> bool { self.phase != Phase::Idle }
    /// Has a network call out (the shared region is in use).
    pub fn in_call(&self) -> bool { self.sent }

    pub fn command(&mut self, args: &[u8], sb: &mut Scrollback, fetch_busy: bool) {
        let args = trim(args);
        if args.is_empty() {
            if self.phase == Phase::Live {
                say!(sb, "tether: up to {}:{}, {} bytes sent, {} commands run", core::str::from_utf8(&self.host[..self.host_len]).unwrap_or("?"), self.port, self.bytes_out, self.commands);
            } else if self.active() {
                sb.push(b"tether: connecting.");
            } else {
                sb.push(b"Usage: tether HOST[:PORT] (default port 7177), tether off");
                sb.push(b"On the other computer: python3 scripts/tether_host.py");
            }
            return;
        }
        if args == b"off" || args == b"stop" {
            if !self.active() { sb.push(b"tether: not running."); return; }
            sb.push(b"tether: closing.");
            self.end();
            return;
        }
        if self.active() { sb.push(b"tether: already running (tether off first)."); return; }
        if fetch_busy { sb.push(b"tether: a download is using the network; try again when it ends."); return; }
        if args == b"listen" || args.starts_with(b"listen ") {
            let rest = trim(&args[6..]);
            let port = if rest.is_empty() { Some(DEFAULT_PORT) } else { parse_port(rest) };
            let Some(port) = port else { sb.push(b"Usage: tether listen [PORT]"); return };
            self.reset(sb);
            self.listening = true;
            self.port = port;
            self.host_len = 0;
            self.phase = Phase::Up;
            say!(sb, "tether: waiting for the network, then listening on port {}", port);
            return;
        }
        let Some((host, port)) = parse_target(args) else { sb.push(b"Usage: tether HOST[:PORT] | tether listen [PORT]"); return };
        self.listening = false;
        self.host[..host.len()].copy_from_slice(host);
        self.host_len = host.len();
        self.port = port;
        self.reset(sb);
        self.phase = Phase::Up;
        say!(sb, "tether: connecting to {}:{}", core::str::from_utf8(host).unwrap_or("?"), port);
    }

    /// A fresh session: the boot log head and Spindle's retained lines go first.
    fn reset(&mut self, sb: &Scrollback) {
        self.out_len = 0;
        self.in_len = 0;
        self.queue_count = 0;
        self.queue_head = 0;
        self.log_at = 0;
        self.log_total = 0;
        self.bytes_out = 0;
        self.commands = 0;
        self.empty_receives = 0;
        self.lines_sent = sb.total_lines.saturating_sub(sb.retained() as u32);
        self.deadline = sex_pdx::get_ticks() + CONNECT_MS;
        self.next_at = 0;
        self.sent = false;
    }

    fn end(&mut self) {
        // A call still out is answered and dropped by `receive` (phase Close).
        self.phase = Phase::Close;
        self.next_at = 0;
    }

    fn fail(&mut self, sb: &mut Scrollback, why: &str) {
        say!(sb, "tether: {}", why);
        sex_pdx::serial_record!("[spindle.tether.end] reason={} bytes={} commands={}", why, self.bytes_out, self.commands);
        // Listening: a session that ends drops its connection and waits for the next.
        if self.listening && self.phase == Phase::Live { self.phase = Phase::Drop; self.next_at = 0; return; }
        self.end();
    }

    fn net_call(&mut self, kind: u8, value: u64, call: Call) { self.net_call_on(FLOW, kind, value, call) }
    fn net_call_on(&mut self, flow: u8, kind: u8, value: u64, call: Call) {
        self.seq = (self.seq + 1) & 0x7F_FFFF;
        if sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, w::OP, w::request_on(self.seq, kind, flow), value, 0).is_ok() {
            self.sent = true;
            self.call = call;
        } else {
            self.next_at = sex_pdx::get_ticks() + POLL_MS;
        }
    }

    /// Fills `out` with the next text to send: the boot log head, then new scrollback lines.
    fn fill(&mut self, sb: &Scrollback) {
        if self.log_at != usize::MAX {
            if self.log_at == 0 && self.out_len == 0 {
                self.put(b"SEXOS-TETHER 1\n--- boot log head ---\n");
            }
            let mut chunk = [0u8; log::LOG_CHUNK];
            while self.out_len + log::LOG_CHUNK + 64 <= OUT_MAX {
                match log::log_head(self.log_at, &mut chunk) {
                    Some((n, total)) if n > 0 => { self.log_total = total; self.put(&chunk[..n]); self.log_at += n; if self.log_at >= total { break; } }
                    _ => break,
                }
            }
            let done = log::log_head(self.log_at, &mut chunk).map_or(true, |(n, _)| n == 0);
            if done {
                self.put(b"\n--- end of boot log head ---\n--- spindle ---\n");
                sex_pdx::serial_record!("[spindle.tether.log] bytes={}", self.log_at);
                self.log_at = usize::MAX;
            }
            return;
        }
        let kept = sb.retained() as u32;
        let oldest = sb.total_lines.saturating_sub(kept);
        if self.lines_sent < oldest {
            let mut t = Text { buf: [0; 160], len: 0 };
            let _ = core::fmt::Write::write_fmt(&mut t, format_args!("[tether] {} lines scrolled away before they could be sent\n", oldest - self.lines_sent));
            let n = t.len;
            let b = t.buf;
            self.put(&b[..n]);
            self.lines_sent = oldest;
        }
        while self.lines_sent < sb.total_lines {
            let line = sb.get_retained((self.lines_sent - oldest) as usize);
            if self.out_len + line.len() + 1 > OUT_MAX { break; }
            self.put(line);
            self.put(b"\n");
            self.lines_sent += 1;
        }
    }

    fn put(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(OUT_MAX - self.out_len);
        self.out[self.out_len..self.out_len + n].copy_from_slice(&bytes[..n]);
        self.out_len += n;
    }

    /// Sends the next call when one is due. True when the screen may need drawing.
    pub fn step(&mut self, sb: &mut Scrollback) -> bool {
        if self.phase == Phase::Idle || self.sent { return false; }
        let now = sex_pdx::get_ticks();
        if now < self.next_at { return false; }
        if !self.listening && self.phase != Phase::Live && self.phase != Phase::Close && now > self.deadline {
            self.fail(sb, "could not reach the other computer (is tether_host.py running, and the firewall open?)");
            return true;
        }
        let Some(region) = net() else { self.phase = Phase::Idle; sb.push(b"tether: no network here."); return true };
        match self.phase {
            Phase::Up => self.net_call(w::UP, 0, Call::Plain),
            Phase::ResolveAsk => { region[..self.host_len].copy_from_slice(&self.host[..self.host_len]); self.net_call(w::RESOLVE, self.host_len as u64, Call::Plain); }
            Phase::ResolveWait => self.net_call(w::RESOLVE, 0, Call::Plain),
            Phase::Connect => self.net_call(w::CONNECT, w::address_port(self.address, self.port), Call::Plain),
            Phase::Opening => self.net_call(w::STATE, 0, Call::Plain),
            Phase::Close => self.net_call(w::CLOSE, 0, Call::Close),
            Phase::Listen => self.net_call_on(LISTEN_FLOW, w::LISTEN, self.port as u64, Call::Plain),
            Phase::Addresses => self.net_call_on(LISTEN_FLOW, w::ADDRESSES, 0, Call::Plain),
            Phase::Accept => self.net_call_on(LISTEN_FLOW, w::ACCEPT, FLOW as u64, Call::Plain),
            Phase::Drop => self.net_call(w::CLOSE, 0, Call::Plain),
            Phase::CloseListener => self.net_call_on(LISTEN_FLOW, w::CLOSE, 0, Call::Close),
            Phase::Live => {
                if self.out_len == 0 { self.fill(sb); }
                // Receives and sends take turns, so commands get through during the log dump.
                self.recv_turn = !self.recv_turn;
                if self.out_len > 0 && !self.recv_turn {
                    let n = self.out_len.min(w::REGION_BYTES);
                    region[..n].copy_from_slice(&self.out[..n]);
                    self.net_call(w::SEND, n as u64, Call::Send);
                } else if self.empty_receives >= STATE_EVERY {
                    self.empty_receives = 0;
                    self.net_call(w::STATE, 0, Call::State);
                } else {
                    let room = (IN_MAX - self.in_len).max(1).min(w::REGION_BYTES);
                    self.net_call(w::RECEIVE, room as u64, Call::Receive);
                }
            }
            Phase::Idle => {}
        }
        true
    }

    /// True if `word` was the answer to our call.
    pub fn receive(&mut self, caller: u32, word: u64, sb: &mut Scrollback) -> bool {
        if !self.sent || self.phase == Phase::Idle || caller != 13 { return false; }
        let Some((seq, status, value)) = w::decode_reply(word) else { return false };
        if seq != self.seq { return false; }
        self.sent = false;
        let now = sex_pdx::get_ticks();
        if self.phase == Phase::Close {
            // The answer to a call made before the end: the flow still needs its CLOSE.
            if self.call != Call::Close { return true; }
            // Listening: the listener closes too.
            if self.listening { self.phase = Phase::CloseListener; self.next_at = 0; return true; }
            self.phase = Phase::Idle;
            sex_pdx::serial_record!("[spindle.tether.closed] bytes={} commands={}", self.bytes_out, self.commands);
            sb.push(b"tether: closed.");
            return true;
        }
        if self.phase == Phase::CloseListener {
            self.phase = Phase::Idle;
            self.listening = false;
            sex_pdx::serial_record!("[spindle.tether.closed] bytes={} commands={}", self.bytes_out, self.commands);
            sb.push(b"tether: closed.");
            return true;
        }
        if self.phase == Phase::Drop {
            // The finished connection is closed whatever the answer; wait for the next.
            self.phase = Phase::Accept;
            self.next_at = now + ACCEPT_MS;
            return true;
        }
        // Listening, the network may come up late (DHCP on a cable plugged in later).
        if self.listening && self.phase == Phase::Up && status == w::OFFLINE { self.next_at = now + 2_000; return true; }
        if !matches!(status, w::OK | w::WAIT) {
            self.fail(sb, match status { w::OFFLINE => "no network (join Wi-Fi with `wifi join N PASSWORD`, or plug in Ethernet)", w::NO_NAME => "name not found (use the computer's IP address)", w::BUSY => "the network is busy", w::DENIED => "not allowed to use the network", _ => "network error" });
            return true;
        }
        let Some(region) = net() else { self.phase = Phase::Idle; return true };
        match self.phase {
            Phase::Up => {
                if status == w::WAIT { self.next_at = now + POLL_MS * 5; return true; }
                if self.listening { self.phase = Phase::Listen; return true; }
                match numeric(&self.host[..self.host_len]) {
                    Some(a) => { self.address = a; self.phase = Phase::Connect; }
                    None => self.phase = Phase::ResolveAsk,
                }
            }
            Phase::ResolveAsk => { self.phase = Phase::ResolveWait; self.next_at = now + POLL_MS; }
            Phase::ResolveWait => {
                if status == w::WAIT { self.next_at = now + POLL_MS; return true; }
                self.address.copy_from_slice(&region[..4]);
                self.phase = Phase::Connect;
            }
            Phase::Connect => { self.phase = Phase::Opening; self.next_at = now + POLL_MS; }
            Phase::Opening => match value {
                w::STATE_OPEN => {
                    self.phase = Phase::Live;
                    let a = self.address;
                    say!(sb, "tether: up to {}.{}.{}.{}:{}. Commands from there show as remote> lines; tether off ends it.", a[0], a[1], a[2], a[3], self.port);
                    sex_pdx::serial_record!("[spindle.tether.up] port={}", self.port);
                }
                w::STATE_OPENING => self.next_at = now + POLL_MS,
                _ => self.fail(sb, "the other computer refused the connection (is tether_host.py running?)"),
            },
            Phase::Listen => { self.phase = Phase::Addresses; }
            Phase::Addresses => {
                // Say this machine's IPv4 addresses, so a photo of the screen names it too.
                let mut shown = 0;
                for k in 0..(value as usize).min(4) {
                    let at = k * (w::ADDRESS_BYTES + 1);
                    if region[at] == w::FAMILY_V4 {
                        let a = &region[at + 1..at + 5];
                        say!(sb, "tether: listening at {}.{}.{}.{}:{}. On the Mac: python3 tether_host.py connect --scan", a[0], a[1], a[2], a[3], self.port);
                        shown += 1;
                    }
                }
                if shown == 0 { say!(sb, "tether: listening on port {} (no IPv4 address yet)", self.port); }
                sex_pdx::serial_record!("[spindle.tether.listening] port={} v4={}", self.port, shown);
                self.phase = Phase::Accept;
                self.next_at = now + ACCEPT_MS;
            }
            Phase::Accept => {
                if status == w::WAIT { self.next_at = now + ACCEPT_MS; return true; }
                self.reset(sb);
                self.phase = Phase::Live;
                let a = &region[1..5];
                say!(sb, "tether: {}.{}.{}.{} connected. Commands from there show as remote> lines; tether off ends it.", a[0], a[1], a[2], a[3]);
                sex_pdx::serial_record!("[spindle.tether.up] port={} accepted=1", self.port);
            }
            Phase::Live => match self.call {
                Call::Send => {
                    let n = (value as usize).min(self.out_len);
                    self.out.copy_within(n..self.out_len, 0);
                    self.out_len -= n;
                    self.bytes_out += n as u64;
                    if n == 0 { self.next_at = now + POLL_MS; }
                }
                Call::Receive => {
                    let n = (value as usize).min(IN_MAX - self.in_len);
                    if n == 0 {
                        self.empty_receives += 1;
                        if self.out_len == 0 { self.next_at = now + POLL_MS; }
                    } else {
                        self.empty_receives = 0;
                        self.inbound[self.in_len..self.in_len + n].copy_from_slice(&region[..n]);
                        self.in_len += n;
                        self.split_lines(sb);
                    }
                }
                Call::State => {
                    if matches!(value, w::STATE_PEER_CLOSED | w::STATE_CLOSED) { self.fail(sb, "the other computer closed the connection"); }
                }
                Call::Plain | Call::Close => {}
            },
            _ => {}
        }
        true
    }

    fn split_lines(&mut self, sb: &mut Scrollback) {
        while let Some(end) = self.inbound[..self.in_len].iter().position(|b| *b == b'\n') {
            let mut line = [0u8; CMD_MAX];
            let raw = &self.inbound[..end];
            let raw = if let [rest @ .., b'\r'] = raw { rest } else { raw };
            let n = raw.len().min(CMD_MAX);
            line[..n].copy_from_slice(&raw[..n]);
            self.inbound.copy_within(end + 1..self.in_len, 0);
            self.in_len -= end + 1;
            let text = trim(&line[..n]);
            if text.is_empty() { continue; }
            if self.queue_count == QUEUE { sb.push(b"tether: too many commands waiting; one dropped."); continue; }
            let slot = (self.queue_head + self.queue_count) % QUEUE;
            self.queue[slot][..text.len()].copy_from_slice(text);
            self.queue_len[slot] = text.len();
            self.queue_count += 1;
        }
        // A line longer than the buffer: drop what came so far.
        if self.in_len == IN_MAX { self.in_len = 0; sb.push(b"tether: a command line was too long."); }
    }

    /// The next command that arrived, to run as if typed.
    pub fn next_command(&mut self, out: &mut [u8; CMD_MAX]) -> Option<usize> {
        if self.queue_count == 0 { return None; }
        let slot = self.queue_head;
        let n = self.queue_len[slot];
        out[..n].copy_from_slice(&self.queue[slot][..n]);
        self.queue_head = (self.queue_head + 1) % QUEUE;
        self.queue_count -= 1;
        self.commands += 1;
        Some(n)
    }
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = s { s = rest; }
    while let [rest @ .., b' ' | b'\t'] = s { s = rest; }
    s
}
