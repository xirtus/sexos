//! `fetch URL FILE`: HTTP and HTTPS into the filesystem. One step at a time, driven from the
//! main loop, so typing keeps working while a download runs.
use crate::Scrollback;
use sex_pdx::fs;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::web as w;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase { Idle, Up, ResolveAsk, ResolveWait, Connect, Opening, TlsStart, TlsWait, Send, FinishRequest, CreateFile, CutFile, Receive, CheckPeer, Store, Close }
const HEAD_MAX: usize = 16 * 1024;
const TICK_POLL: u64 = 5;
/// A download that drops part way (a Wi-Fi stall past the flow's timeout, a reset) is
/// resumed from where it stopped this many times, with `Range` when the server honours it
/// and from the start when it does not.
const MAX_RESUMES: u8 = 10;
/// Unknown length: the server sent no Content-Length.
const NO_LENGTH: u64 = u64::MAX;
pub struct Fetch {
    phase: Phase, seq: u32, sent: bool, to_files: bool, next_at: u64, deadline: u64, host: [u8; 256], host_len: usize, port: u16, address: [u8; 4],
    request: [u8; 900], request_len: usize, request_sent: usize, path_len: usize, head: [u8; HEAD_MAX], head_len: usize, in_body: bool,
    written: u64, chunk: usize, fs_seq: u32, peer_done: bool, status: u32, tls: bool,
    /// Network's run when the pending call went out, and when it did: a call the old
    /// run took is never answered after a restart, so the download ends instead of waiting.
    net_epoch: u64, sent_at: u64,
    /// Where a redirect said to go next ("URL FILE"), and how many were followed.
    redirect: [u8; 1024], redirect_len: usize, hops: u8,
    /// How the last download ended, for `update`: Some(true) saved, Some(false) failed.
    pub result: Option<bool>,
    /// The file's whole length when the server said it (Content-Length, or a 206's
    /// Content-Range total), else `NO_LENGTH`. A body that ends short of it is cut off.
    expected: u64,
    /// Where this attempt's body lands in the file: 0, or the resume point of a `Range` ask.
    from: u64,
    /// The flow ended by a reset or timeout (STATE_CLOSED), not by the peer's FIN.
    aborted: bool,
    /// Resumes used, and whether the Close in flight is followed by one.
    resumes: u8, resume_next: bool,
    /// Received bytes wait in `net()` while the file is cut first (a server that ignored
    /// `Range` sends the file from its start).
    chunk_pending: bool,
    /// The "URL FILE" line the current attempt runs (after any redirect), for a resume.
    again: [u8; 1024], again_len: usize,
}
pub static mut FETCH: Fetch = Fetch { phase: Phase::Idle, seq: 0, sent: false, to_files: false, next_at: 0, deadline: 0, host: [0; 256], host_len: 0, port: 80, address: [0; 4], request: [0; 900], request_len: 0, request_sent: 0, path_len: 0, head: [0; HEAD_MAX], head_len: 0, in_body: false, written: 0, chunk: 0, fs_seq: 0x80_0000, peer_done: false, status: 0, tls: false, net_epoch: 0, sent_at: 0, redirect: [0; 1024], redirect_len: 0, hops: 0, result: None, expected: NO_LENGTH, from: 0, aborted: false, resumes: 0, resume_next: false, chunk_pending: false, again: [0; 1024], again_len: 0 };

fn net() -> Option<&'static mut [u8]> {
    let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
}
fn files() -> Option<&'static mut [u8]> {
    let view = sex_pdx::memory_region_view(fs::CLIENT_SLOT, 0, fs::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, fs::REGION_BYTES) })
}
fn numeric(host: &[u8]) -> Option<[u8; 4]> {
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
    sex_pdx::serial_record!("[spindle.fetch.line] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}
/// Network's restart epoch (0 when it cannot be read).
fn net_epoch() -> u64 { sex_pdx::crash_log::peer_state(sex_pdx::SLOT_NET).map_or(0, |p| p.epoch) }
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> { hay.windows(needle.len()).position(|w| w == needle) }
/// A header's value in `head` (up to the blank line), its name matched without case.
fn header<'a>(head: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    head.split(|b| *b == b'\n').skip(1).find_map(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let colon = line.iter().position(|b| *b == b':')?;
        if !line[..colon].eq_ignore_ascii_case(name) { return None; }
        Some(line[colon + 1..].trim_ascii())
    })
}
fn number(text: &[u8]) -> Option<u64> {
    if text.is_empty() || text.len() > 19 || !text.iter().all(u8::is_ascii_digit) { return None; }
    Some(text.iter().fold(0u64, |v, b| v * 10 + (*b - b'0') as u64))
}

impl Fetch {
    pub fn busy(&self) -> bool { self.phase != Phase::Idle }
    /// True while `fetch` holds Spindle's shared network region (an in-flight `SLOT_NET`
    /// call, or received bytes waiting in `net()` for `Phase::Store` to copy to `files()`).
    pub fn region_in_use(&self) -> bool { (self.sent && !self.to_files) || self.phase == Phase::Store || self.chunk_pending }
    fn end(&mut self) { self.phase = Phase::Close; self.sent = false; self.to_files = false; }
    fn fail(&mut self, sb: &mut Scrollback, why: &str) {
        // While resuming, a failed reconnect (Wi-Fi still coming back) is one more try.
        if self.resumes > 0 && self.resumes < MAX_RESUMES && self.written > 0 && !self.resume_next { self.cut_off(sb, why); return; }
        say!(sb, "fetch: {}", why); self.written = 0; self.redirect_len = 0; self.resume_next = false; self.chunk_pending = false; self.result = Some(false); self.end(); }
    /// The body stopped before its end: resume it, or fail saying how much arrived.
    fn cut_off(&mut self, sb: &mut Scrollback, why: &str) {
        if self.resumes < MAX_RESUMES && self.again_len > 0 {
            self.resumes += 1;
            sex_pdx::serial_record!("[spindle.fetch.resume] why={} at={} of={} try={}", why, self.written, self.expected, self.resumes);
            if self.expected != NO_LENGTH { say!(sb, "fetch: {} at {} of {} bytes; resuming ({}/{})", why, self.written, self.expected, self.resumes, MAX_RESUMES); }
            else { say!(sb, "fetch: {} at {} bytes; resuming ({}/{})", why, self.written, self.resumes, MAX_RESUMES); }
            self.resume_next = true;
            self.chunk_pending = false;
            self.end();
            return;
        }
        sex_pdx::serial_record!("[spindle.fetch.incomplete] why={} got={} of={}", why, self.written, self.expected);
        let mut t = Text { buf: [0; 160], len: 0 };
        if self.expected != NO_LENGTH { let _ = core::fmt::Write::write_fmt(&mut t, format_args!("download incomplete ({}): got {} of {} bytes", why, self.written, self.expected)); }
        else { let _ = core::fmt::Write::write_fmt(&mut t, format_args!("download incomplete ({}): got {} bytes", why, self.written)); }
        let mut msg = [0u8; 160];
        msg[..t.len].copy_from_slice(&t.buf[..t.len]);
        self.fail(sb, core::str::from_utf8(&msg[..t.len]).unwrap_or("download incomplete"));
    }
    /// The body is in the file when the peer closed after all of it: what the length said,
    /// or (no length) a clean FIN rather than a reset or timeout.
    fn complete(&self) -> bool { if self.expected != NO_LENGTH { self.written >= self.expected } else { !self.aborted } }

    pub fn start(&mut self, args: &[u8], sb: &mut Scrollback, join: impl Fn(&[u8], &mut [u8]) -> Option<usize>) {
        if self.busy() { sb.push(b"A download is already running."); return; }
        self.hops = 0;
        self.result = None;
        self.resumes = 0;
        self.resume_next = false;
        self.begin(args, sb, join, 0);
    }
    fn begin(&mut self, args: &[u8], sb: &mut Scrollback, join: impl Fn(&[u8], &mut [u8]) -> Option<usize>, from: u64) {
        self.redirect_len = 0;
        if args.len() <= self.again.len() && !core::ptr::eq(args.as_ptr(), self.again.as_ptr()) { self.again[..args.len()].copy_from_slice(args); self.again_len = args.len(); }
        let mut words = args.splitn(2, |b| *b == b' ');
        let (url, file) = (words.next().unwrap_or(&[]), words.next().unwrap_or(&[]));
        let (rest, tls) = match (url.strip_prefix(b"https://"), url.strip_prefix(b"http://")) {
            (Some(rest), _) => (rest, true),
            (_, Some(rest)) => (rest, false),
            _ => { sb.push(b"Usage: fetch URL FILE"); self.result = Some(false); return; }
        };
        if file.is_empty() { sb.push(b"Usage: fetch URL FILE"); self.result = Some(false); return; }
        let split = rest.iter().position(|b| *b == b'/').unwrap_or(rest.len());
        let (authority, path) = (&rest[..split], if split == rest.len() { &b"/"[..] } else { &rest[split..] });
        let (host, port) = match authority.iter().position(|b| *b == b':') {
            Some(i) => (&authority[..i], authority[i + 1..].iter().try_fold(0u32, |v, b| if b.is_ascii_digit() { Some(v * 10 + (*b - b'0') as u32) } else { None }).filter(|p| (1..=65535).contains(p)).map(|p| p as u16)),
            None => (authority, Some(if tls { 443 } else { 80 })),
        };
        let Some(port) = port else { sb.push(b"fetch: bad port"); return };
        if host.is_empty() || host.len() > 253 { sb.push(b"fetch: bad host"); return; }
        let Some(region) = files() else { sb.push(b"fetch: no filesystem"); return };
        let mut joined = [0u8; 512];
        let Some(n) = join(file, &mut joined) else { sb.push(b"Path too long."); return };
        region[..n].copy_from_slice(&joined[..n]);
        self.path_len = n;
        self.host_len = host.len();
        self.host[..host.len()].copy_from_slice(host);
        self.port = port;
        self.tls = tls;
        // HTTP/1.0 so the reply needs no chunk decoding, and ends when the peer closes.
        // A resume asks for the rest with `Range`.
        let mut range = Text { buf: [0; 160], len: 0 };
        if from > 0 { let _ = core::fmt::Write::write_fmt(&mut range, format_args!("Range: bytes={}-\r\n", from)); }
        let parts: [&[u8]; 6] = [b"GET ", path, b" HTTP/1.0\r\nHost: ", host, b"\r\nUser-Agent: SexOS\r\nAccept: */*\r\nConnection: close\r\n", &range.buf[..range.len]];
        let total: usize = parts.iter().map(|p| p.len()).sum();
        if total > self.request.len() { sb.push(b"fetch: address too long"); return; }
        let mut at = 0;
        if total + 2 > self.request.len() { sb.push(b"fetch: address too long"); return; }
        for p in parts { self.request[at..at + p.len()].copy_from_slice(p); at += p.len(); }
        self.request[at..at + 2].copy_from_slice(b"\r\n"); at += 2;
        self.request_len = at;
        self.request_sent = 0; self.head_len = 0; self.in_body = false; self.written = from; self.from = from; self.peer_done = false; self.status = 0;
        self.aborted = false; self.chunk_pending = false; if from == 0 { self.expected = NO_LENGTH; }
        self.next_at = 0;
        self.deadline = sex_pdx::get_ticks() + 60_000;
        self.phase = Phase::Up;
        self.sent = false;
        say!(sb, "Connecting to {}:{}", core::str::from_utf8(host).unwrap_or("?"), port);
    }
    fn net_call(&mut self, kind: u8, value: u64) {
        self.seq = (self.seq + 1) & 0x7F_FFFF;
        self.net_epoch = net_epoch();
        if sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, w::OP, w::request(self.seq, kind), value, 0).is_ok() { self.sent = true; self.to_files = false; self.sent_at = sex_pdx::get_ticks(); }
    }
    /// A call Network took before it restarted gets no answer: give up on it (and on one
    /// that has gone unanswered far past the download's deadline).
    fn lost_call(&mut self, sb: &mut Scrollback) -> bool {
        if !self.sent || self.to_files { return false; }
        let now = sex_pdx::get_ticks();
        let restarted = self.net_epoch != 0 && matches!(net_epoch(), e if e != 0 && e != self.net_epoch);
        if !restarted && now < self.deadline.max(self.sent_at) + 10_000 { return false; }
        self.sent = false;
        sex_pdx::serial_record!("[spindle.fetch.lost] restarted={} phase={:?}", restarted as u8, self.phase);
        if self.phase == Phase::Close { self.phase = Phase::Idle; } else { self.fail(sb, if restarted { "network restarted" } else { "no answer" }); self.phase = Phase::Idle; }
        true
    }
    fn fs_call(&mut self, kind: u8, offset: u64, length: u64) {
        self.fs_seq = 0x80_0000 | ((self.fs_seq + 1) & 0x7F_FFFF);
        let r = fs::Request { seq: self.fs_seq, kind, path_len: self.path_len as u16, offset, length };
        let a = r.args();
        if sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, fs::OP, a[0], a[1], a[2]).is_ok() { self.sent = true; self.to_files = true; }
    }
    /// Sends the next request when one is due.
    pub fn step(&mut self, sb: &mut Scrollback, region_busy: bool) -> bool {
        if self.lost_call(sb) { return true; }
        if self.phase == Phase::Idle || self.sent { return false; }
        let now = sex_pdx::get_ticks();
        if now < self.next_at { return false; }
        if now > self.deadline && self.phase != Phase::Close {
            if self.in_body { self.cut_off(sb, "timed out"); } else { self.fail(sb, "timed out"); }
        }
        if region_busy && !matches!(self.phase, Phase::CreateFile | Phase::CutFile | Phase::Store) { return false; }
        let Some(region) = net() else { self.phase = Phase::Idle; return true };
        match self.phase {
            Phase::Up => self.net_call(w::UP, 0),
            Phase::ResolveAsk => { region[..self.host_len].copy_from_slice(&self.host[..self.host_len]); self.net_call(w::RESOLVE, self.host_len as u64); }
            Phase::ResolveWait => self.net_call(w::RESOLVE, 0),
            Phase::Connect => self.net_call(w::CONNECT, w::address_port(self.address, self.port)),
            Phase::Opening => self.net_call(w::STATE, 0),
            Phase::TlsStart => { region[..self.host_len].copy_from_slice(&self.host[..self.host_len]); self.net_call(w::TLS_START, self.host_len as u64); }
            Phase::TlsWait => self.net_call(w::TLS_STAGE, 0),
            Phase::FinishRequest => self.net_call(w::FINISH_REQUEST, 0),
            Phase::Send => {
                let left = &self.request[self.request_sent..self.request_len];
                region[..left.len()].copy_from_slice(left);
                self.net_call(w::SEND, left.len() as u64);
            }
            Phase::CreateFile => self.fs_call(fs::CREATE, 0, 0),
            Phase::CutFile => self.fs_call(fs::TRUNCATE, 0, 0),
            Phase::Receive => self.net_call(w::RECEIVE, 32 * 1024),
            Phase::CheckPeer => self.net_call(w::STATE, 0),
            Phase::Store => {
                let Some(file) = files() else { self.phase = Phase::Idle; return true };
                file[fs::PATH_BYTES..fs::PATH_BYTES + self.chunk].copy_from_slice(&region[..self.chunk]);
                self.fs_call(fs::WRITE, self.written, self.chunk as u64);
            }
            Phase::Close => self.net_call(w::CLOSE, 0),
            Phase::Idle => {}
        }
        true
    }
    fn after_headers(&mut self, sb: &mut Scrollback) -> bool {
        let Some(end) = find(&self.head[..self.head_len], b"\r\n\r\n") else {
            if self.head_len == HEAD_MAX { self.fail(sb, "reply headers too long"); }
            return false;
        };
        let line = &self.head[..find(&self.head[..end + 2], b"\r\n").unwrap_or(end)];
        let code = line.split(|b| *b == b' ').nth(1).map_or(0, |c| c.iter().fold(0u32, |v, b| v.wrapping_mul(10).wrapping_add((*b as u32).wrapping_sub(48))));
        self.status = code;
        sex_pdx::serial_record!("[spindle.fetch.status] code={}", code);
        if matches!(code, 301 | 302 | 303 | 307 | 308) && self.hops < 8 {
            // Follow it (GitHub release downloads redirect to their storage): same file.
            let location = find(&self.head[..end], b"ocation: ").map(|at| { let v = &self.head[at + 9..end]; &v[..find(v, b"\r\n").unwrap_or(v.len())] });
            if let (Some(loc), Some(region)) = (location, files()) {
                let mut at = 0;
                let mut put = |bytes: &[u8], buf: &mut [u8; 1024]| { let ok = at + bytes.len() <= buf.len(); if ok { buf[at..at + bytes.len()].copy_from_slice(bytes); at += bytes.len(); } ok };
                let mut port = [0u8; 6];
                let port_len = { let mut t = Text { buf: [0; 160], len: 0 }; let _ = core::fmt::Write::write_fmt(&mut t, format_args!(":{}", self.port)); port[..t.len.min(6)].copy_from_slice(&t.buf[..t.len.min(6)]); t.len.min(6) };
                let mut ok = true;
                if loc.starts_with(b"/") {
                    ok &= put(if self.tls { b"https://" } else { b"http://" }, &mut self.redirect);
                    ok &= put(&self.host[..self.host_len], &mut self.redirect);
                    ok &= put(&port[..port_len], &mut self.redirect);
                }
                ok &= put(loc, &mut self.redirect);
                ok &= put(b" ", &mut self.redirect);
                ok &= put(&region[..self.path_len], &mut self.redirect);
                if ok && (loc.starts_with(b"/") || loc.starts_with(b"http://") || loc.starts_with(b"https://")) {
                    self.redirect_len = at;
                    self.hops += 1;
                    sex_pdx::serial_record!("[spindle.fetch.redirect] code={} hop={}", code, self.hops);
                    self.end();
                    return false;
                }
            }
        }
        if !(200..300).contains(&code) {
            let location = find(&self.head[..end], b"ocation: ").map(|at| { let v = &self.head[at + 9..end]; &v[..find(v, b"\r\n").unwrap_or(v.len())] });
            let mut t = Text { buf: [0; 160], len: 0 };
            let _ = core::fmt::Write::write_fmt(&mut t, format_args!("HTTP {}{}{}", code, if location.is_some() { " to " } else { "" }, location.and_then(|l| core::str::from_utf8(l).ok()).unwrap_or("")));
            self.fail(sb, core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
            return false;
        }
        let head = &self.head[..end + 2];
        let length = header(head, b"content-length").and_then(number);
        if code == 206 && self.from > 0 {
            // "bytes FIRST-LAST/TOTAL": it must start where the file stops.
            let range = header(head, b"content-range").and_then(|v| v.strip_prefix(b"bytes ")).unwrap_or(&[]);
            let dash = range.iter().position(|b| *b == b'-');
            let slash = range.iter().position(|b| *b == b'/');
            let first = dash.and_then(|d| number(&range[..d]));
            let total = slash.and_then(|s| number(&range[s + 1..]));
            if first != Some(self.from) { self.fail(sb, "the server resumed at the wrong place"); return false; }
            self.expected = total.or(length.map(|l| self.from + l)).unwrap_or(NO_LENGTH);
        } else {
            // A whole file: from its start, even when a resume asked for the rest.
            self.expected = length.unwrap_or(NO_LENGTH);
            if self.from > 0 { self.from = 0; self.written = 0; self.chunk_pending = true; }
        }
        sex_pdx::serial_record!("[spindle.fetch.length] code={} expected={} from={}", code, self.expected, self.from);
        true
    }
    /// True if `word` was ours.
    pub fn receive(&mut self, caller: u32, word: u64, sb: &mut Scrollback) -> bool {
        if !self.sent || self.phase == Phase::Idle { return false; }
        if self.to_files {
            let Some((seq, status, value)) = fs::decode_reply(word) else { return false };
            if caller != 11 || seq != self.fs_seq { return false; }
            self.sent = false;
            match self.phase {
                Phase::CreateFile => {
                    if status == fs::EXISTS { self.phase = Phase::CutFile; }
                    else if status == fs::OK { self.phase = Phase::Receive; }
                    else { let m = core::str::from_utf8(fs::message(status)).unwrap_or("?"); self.fail(sb, m); }
                }
                Phase::CutFile => {
                    if status == fs::OK { self.phase = if self.chunk_pending { self.chunk_pending = false; Phase::Store } else { Phase::Receive }; } else { let m = core::str::from_utf8(fs::message(status)).unwrap_or("?"); self.fail(sb, m); }
                }
                Phase::Store => {
                    if status != fs::OK { let m = core::str::from_utf8(fs::message(status)).unwrap_or("?"); self.fail(sb, m); return true; }
                    self.written += value as u64;
                    if (value as usize) < self.chunk { self.fail(sb, "disk full"); return true; }
                    self.phase = Phase::Receive;
                }
                _ => {}
            }
            return true;
        }
        let Some((seq, status, value)) = w::decode_reply(word) else { return false };
        if caller != 13 || seq != self.seq { return false; }
        self.sent = false;
        let now = sex_pdx::get_ticks();
        if self.phase == Phase::Close {
            self.phase = Phase::Idle;
            let same = |p: &[u8], o: &mut [u8]| { let k = p.len(); if k > o.len() { return None; } o[..k].copy_from_slice(p); Some(k) };
            if self.redirect_len > 0 {
                let line = self.redirect;
                let n = self.redirect_len;
                self.begin(&line[..n], sb, same, 0);
                if self.phase == Phase::Idle && self.result.is_none() { self.result = Some(false); }
                return true;
            }
            if self.resume_next {
                self.resume_next = false;
                let line = self.again;
                let n = self.again_len;
                let from = self.written;
                self.begin(&line[..n], sb, same, from);
                // Give the link a moment before asking again, longer each time.
                self.next_at = now + 1_000 * self.resumes as u64;
                if self.phase == Phase::Idle && self.result.is_none() { self.result = Some(false); }
                return true;
            }
            if self.status != 0 && (200..300).contains(&self.status) {
                say!(sb, "Saved {} bytes", self.written);
                sex_pdx::serial_record!("[spindle.fetch.done] bytes={}", self.written);
                self.result = Some(true);
            } else if self.result.is_none() { self.result = Some(false); }
            return true;
        }
        if status == w::BUSY && self.phase == Phase::ResolveAsk { self.next_at = now + TICK_POLL * 10; return true; }
        if !matches!(status, w::OK | w::WAIT) && self.in_body && matches!(self.phase, Phase::Receive | Phase::CheckPeer) { self.cut_off(sb, "network error"); return true; }
        if !matches!(status, w::OK | w::WAIT) { self.fail(sb, match status { w::OFFLINE => "no network", w::NO_NAME => "name not found", w::BUSY => "the network is busy", w::DENIED => "not allowed", _ => "network error" }); return true; }
        let Some(region) = net() else { self.phase = Phase::Idle; return true };
        match self.phase {
            Phase::Up => {
                if status == w::WAIT { self.next_at = now + TICK_POLL * 20; return true; }
                match numeric(&self.host[..self.host_len]) {
                    Some(a) => { self.address = a; self.phase = Phase::Connect; }
                    None => self.phase = Phase::ResolveAsk,
                }
            }
            Phase::ResolveAsk => { self.phase = Phase::ResolveWait; self.next_at = now + TICK_POLL; }
            Phase::ResolveWait => {
                if status == w::WAIT { self.next_at = now + TICK_POLL; return true; }
                self.address.copy_from_slice(&region[..4]);
                self.phase = Phase::Connect;
            }
            Phase::Connect => { self.phase = Phase::Opening; self.next_at = now + TICK_POLL; }
            Phase::CheckPeer => {
                // Bytes can land between the two calls, so a closed peer gets one last read.
                self.peer_done = matches!(value, w::STATE_PEER_CLOSED | w::STATE_CLOSED);
                self.aborted |= value == w::STATE_CLOSED;
                if !self.peer_done { self.next_at = now + TICK_POLL; }
                self.phase = Phase::Receive;
            }
            Phase::Opening => match value {
                w::STATE_OPEN => self.phase = if self.tls { Phase::TlsStart } else { Phase::Send },
                w::STATE_OPENING => self.next_at = now + TICK_POLL,
                _ => self.fail(sb, "could not connect"),
            },
            Phase::TlsStart => { self.phase = Phase::TlsWait; self.next_at = now + TICK_POLL; }
            Phase::TlsWait => match value {
                w::TLS_READY => { sex_pdx::serial_record!("[spindle.fetch.tls] verified=1"); self.phase = Phase::Send; }
                w::TLS_HANDSHAKE => self.next_at = now + TICK_POLL,
                w::TLS_FAILED_CERTIFICATE => self.fail(sb, "the site's certificate is not trusted"),
                w::TLS_FAILED_SIGNATURE => self.fail(sb, "the site's signature is not valid"),
                _ => self.fail(sb, "secure connection failed"),
            },
            Phase::Send => {
                self.request_sent += value as usize;
                if self.request_sent < self.request_len { self.next_at = now + TICK_POLL; }
                else if self.tls { self.phase = Phase::FinishRequest; }
                else { self.phase = if self.from > 0 { Phase::Receive } else { Phase::CreateFile }; }
            }
            // A resume keeps the file: what the server answers decides whether it is cut.
            Phase::FinishRequest => self.phase = if self.from > 0 { Phase::Receive } else { Phase::CreateFile },
            Phase::Receive => {
                if value == 0 {
                    if self.peer_done {
                        if self.in_body && !self.complete() { self.cut_off(sb, if self.aborted { "connection lost" } else { "connection closed early" }); return true; }
                        if !self.in_body && self.head_len > 0 { self.fail(sb, "reply cut off in its headers"); return true; }
                        self.end(); return true;
                    }
                    // Nothing yet. Ask whether the peer has finished.
                    self.phase = Phase::CheckPeer;
                    return true;
                }
                let mut bytes = value as usize;
                // A download that keeps arriving may take as long as it needs; only silence ends it.
                self.deadline = now + 60_000;
                if !self.in_body {
                    let take = bytes.min(HEAD_MAX - self.head_len);
                    self.head[self.head_len..self.head_len + take].copy_from_slice(&region[..take]);
                    self.head_len += take;
                    if !self.after_headers(sb) { return true; }
                    self.in_body = true;
                    let end = find(&self.head[..self.head_len], b"\r\n\r\n").unwrap_or(0) + 4;
                    // Whatever followed the headers in this chunk is the start of the file.
                    let body_in_head = self.head_len - end;
                    let chunk_start = take - body_in_head;
                    region.copy_within(chunk_start..bytes, 0);
                    bytes -= chunk_start;
                    if bytes == 0 { if self.chunk_pending { self.chunk_pending = false; self.phase = Phase::CutFile; } return true; }
                }
                self.chunk = bytes;
                if self.chunk_pending { self.phase = Phase::CutFile; return true; }
                self.phase = Phase::Store;
            }
            _ => {}
        }
        true
    }
}
