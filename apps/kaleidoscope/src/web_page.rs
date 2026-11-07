//! Open web pages by address, through the open-internet stream. Self-contained beside
//! the fixed-endpoint panels: one page at a time, driven one step per Browser turn.
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::web as w;

pub const SOURCE_BYTES: usize = 512 * 1024;
pub const TEXT_BYTES: usize = crate::document_reader::LONG_BODY;
const URL_MAX: usize = 1024;
const HISTORY: usize = 16;
const MAX_REDIRECTS: u8 = 5;
const POLL: u64 = 5;
static mut SOURCE: [u8; SOURCE_BYTES] = [0; SOURCE_BYTES];
static mut TEXT: [u8; TEXT_BYTES] = [0; TEXT_BYTES];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase { Idle, Up, ResolveAsk, ResolveWait, Connect, Opening, TlsStart, TlsWait, Send, FinishRequest, Receive, CheckPeer, Close, Convert }

#[derive(Clone, Copy)]
struct Url { bytes: [u8; URL_MAX], len: usize }
impl Url {
    const EMPTY: Url = Url { bytes: [0; URL_MAX], len: 0 };
    fn set(&mut self, value: &[u8]) -> bool {
        if value.len() > URL_MAX { return false; }
        self.bytes[..value.len()].copy_from_slice(value);
        self.len = value.len();
        true
    }
    fn get(&self) -> &[u8] { &self.bytes[..self.len] }
}
pub struct Parts<'a> { pub tls: bool, pub host: &'a [u8], pub port: u16, pub path: &'a [u8] }
pub fn parse(url: &[u8]) -> Option<Parts<'_>> {
    let (rest, tls) = match (url.strip_prefix(b"https://"), url.strip_prefix(b"http://")) {
        (Some(r), _) => (r, true),
        (_, Some(r)) => (r, false),
        _ => return None,
    };
    let end = rest.iter().position(|b| matches!(*b, b'/' | b'?' | b'#')).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let path = &tail[..tail.iter().position(|b| *b == b'#').unwrap_or(tail.len())];
    let (host, port) = match authority.iter().position(|b| *b == b':') {
        Some(i) => {
            let mut port = 0u32;
            for b in &authority[i + 1..] {
                if !b.is_ascii_digit() { return None; }
                port = port * 10 + (*b - b'0') as u32;
                if port > 65535 { return None; }
            }
            if port == 0 { return None; }
            (&authority[..i], port as u16)
        }
        None => (authority, if tls { 443 } else { 80 }),
    };
    if host.is_empty() || host.len() > 253 { return None; }
    Some(Parts { tls, host, port, path: if path.is_empty() { b"/" } else { path } })
}
/// `target` as a whole address, seen from the page at `base`.
pub fn resolve(base: &[u8], target: &[u8], out: &mut [u8; URL_MAX]) -> Option<usize> {
    let mut put = |parts: &[&[u8]]| -> Option<usize> {
        let mut at = 0;
        for part in parts {
            if at + part.len() > URL_MAX { return None; }
            out[at..at + part.len()].copy_from_slice(part);
            at += part.len();
        }
        Some(at)
    };
    let target = &target[..target.iter().position(|b| *b == b'#').unwrap_or(target.len())];
    if target.starts_with(b"http://") || target.starts_with(b"https://") { return put(&[target]); }
    let scheme_end = base.windows(3).position(|w| w == b"://")? + 3;
    let origin_end = base[scheme_end..].iter().position(|b| matches!(*b, b'/' | b'?' | b'#')).map_or(base.len(), |i| scheme_end + i);
    if target.starts_with(b"//") { return put(&[&base[..scheme_end - 2], target]); }
    if target.first() == Some(&b'/') { return put(&[&base[..origin_end], target]); }
    if target.iter().take_while(|b| **b != b'/').any(|b| *b == b':') { return None; }
    let path_end = base[origin_end..].iter().position(|b| matches!(*b, b'?' | b'#')).map_or(base.len(), |i| origin_end + i);
    let dir_end = base[origin_end..path_end].iter().rposition(|b| *b == b'/').map_or(origin_end, |i| origin_end + i + 1);
    if dir_end == origin_end { return put(&[&base[..origin_end], b"/", target]); }
    // Collapse `.` and `..` so the server sees a plain path.
    let mut path = [0u8; URL_MAX];
    let mut len = 0;
    for piece in base[origin_end..dir_end].split(|b| *b == b'/').chain(target.split(|b| *b == b'/')) {
        if piece == b"." || (piece.is_empty()) { continue; }
        if piece == b".." {
            while len > 0 { len -= 1; if path[len] == b'/' { break; } }
            continue;
        }
        if len + 1 + piece.len() > URL_MAX { return None; }
        path[len] = b'/';
        path[len + 1..len + 1 + piece.len()].copy_from_slice(piece);
        len += 1 + piece.len();
    }
    let trailing: &[u8] = if target.last() == Some(&b'/') { b"/" } else { b"" };
    put(&[&base[..origin_end], &path[..len], trailing])
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
    parts.next().is_none().then_some(out)
}
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> { hay.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle)) }
/// What a key types, on the keyboard codes the Shell passes through.
pub fn typed(code: u8, shift: bool) -> Option<u8> {
    let plain: &[u8; 58] = b"\0\x001234567890-=\0\0qwertyuiop[]\0\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
    let upper: &[u8; 58] = b"\0\0!@#$%^&*()_+\0\0QWERTYUIOP{}\0\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";
    let c = *if shift { upper } else { plain }.get(code as usize)?;
    (c != 0).then_some(c)
}
pub static mut WEB: Web = Web::new();
pub struct Links { pub targets: [(usize, usize); crate::html_text::MAX_LINKS], pub count: usize }
pub struct Web {
    pub shown: bool, pub editing: bool, phase: Phase, seq: u32, sent: bool, next_at: u64, deadline: u64,
    url: Url, typed: Url, number: u32, history: [Url; HISTORY], depth: usize, redirects: u8, address: [u8; 4],
    request_sent: usize, request_len: usize, source_len: usize, header_end: usize, text_len: usize, pub dirty: bool, status: u32,
    message: [u8; 96], message_len: usize, pub links: Links, truncated: bool, logged: Phase, peer_done: bool,
}
impl Web {
    pub const fn new() -> Self {
        Self { shown: false, editing: false, phase: Phase::Idle, seq: 0, sent: false, next_at: 0, deadline: 0, url: Url::EMPTY, typed: Url::EMPTY, number: 0,
            history: [Url::EMPTY; HISTORY], depth: 0, redirects: 0, address: [0; 4], request_sent: 0, request_len: 0, source_len: 0, header_end: 0, text_len: 0,
            dirty: false, status: 0, message: [0; 96], message_len: 0, links: Links { targets: [(0, 0); crate::html_text::MAX_LINKS], count: 0 }, truncated: false, logged: Phase::Idle, peer_done: false }
    }
    pub fn busy(&self) -> bool { self.phase != Phase::Idle }
    pub fn text(&self) -> &[u8] { unsafe { &(&*core::ptr::addr_of!(TEXT))[..self.text_len] } }
    pub fn url(&self) -> &[u8] { self.url.get() }
    pub fn typed(&self) -> &[u8] { self.typed.get() }
    pub fn message(&self) -> &[u8] { &self.message[..self.message_len] }
    fn say(&mut self, text: &[u8]) {
        let n = text.len().min(self.message.len());
        self.message[..n].copy_from_slice(&text[..n]);
        self.message_len = n;
        self.dirty = true;
        sex_pdx::serial_record!("[browser.web.status] {}", core::str::from_utf8(&self.message[..n]).unwrap_or("?"));
    }
    fn region() -> Option<&'static mut [u8]> {
        let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
        Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
    }
    fn call(&mut self, kind: u8, value: u64) {
        self.seq = (self.seq + 1) & 0xFF_FFFF;
        if sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, w::OP, w::request(self.seq, kind), value, 0).is_ok() { self.sent = true; }
    }
    /// Start a key press in the address bar.
    pub fn begin_edit(&mut self) { self.shown = true; self.editing = true; self.typed = self.url; self.dirty = true; }
    pub fn key(&mut self, code: u8, shift: bool, now: u64) -> bool {
        if self.editing {
            match code {
                0x01 => { self.editing = false; if self.url.len == 0 && self.text_len == 0 { self.shown = false; } }
                0x1C => { self.editing = false; self.submit(now); }
                0x0E => self.typed.len = self.typed.len.saturating_sub(1),
                _ => if let Some(c) = typed(code, shift) { if self.typed.len < URL_MAX { self.typed.bytes[self.typed.len] = c; self.typed.len += 1; } },
            }
            self.dirty = true;
            return true;
        }
        match code {
            0x01 => { self.shown = false; self.dirty = true; true }
            // Type a link's number, then Enter. Backspace goes back.
            0x02..=0x0B => { let digit = if code == 0x0B { 0 } else { code as u32 - 1 }; self.number = (self.number * 10 + digit) % 1000; self.dirty = true; true }
            0x1C => { let n = core::mem::take(&mut self.number); if n > 0 { self.follow(n as usize, now); } self.dirty = true; true }
            0x0E => { if self.number > 0 { self.number /= 10; self.dirty = true; } else { self.back(now); } true }
            _ => false,
        }
    }
    pub fn number(&self) -> u32 { self.number }
    fn submit(&mut self, now: u64) {
        let mut line = self.typed;
        let value = line.get();
        if value.is_empty() { return; }
        // A bare name means the secure site.
        if !value.starts_with(b"http://") && !value.starts_with(b"https://") {
            let mut full = Url::EMPTY;
            full.bytes[..8].copy_from_slice(b"https://");
            let n = value.len().min(URL_MAX - 8);
            full.bytes[8..8 + n].copy_from_slice(&value[..n]);
            full.len = 8 + n;
            line = full;
        }
        if self.url.len > 0 { self.push_history(); }
        self.url = line;
        self.redirects = 0;
        self.open(now);
    }
    fn push_history(&mut self) {
        if self.depth == HISTORY { self.history.copy_within(1.., 0); self.depth -= 1; }
        self.history[self.depth] = self.url;
        self.depth += 1;
    }
    fn follow(&mut self, n: usize, now: u64) {
        if self.busy() { return; }
        if n == 0 || n > self.links.count { self.say(b"No link with that number."); return; }
        let (from, to) = self.links.targets[n - 1];
        let mut out = [0u8; URL_MAX];
        let text = self.text();
        if to > text.len() || from >= to { return; }
        let Some(len) = resolve(self.url.get(), &text[from..to], &mut out) else { self.say(b"Can't open that kind of link."); return };
        self.push_history();
        self.url.set(&out[..len]);
        self.redirects = 0;
        self.open(now);
    }
    fn back(&mut self, now: u64) {
        if self.busy() || self.depth == 0 { return; }
        self.depth -= 1;
        self.url = self.history[self.depth];
        self.redirects = 0;
        self.open(now);
    }
    fn open(&mut self, now: u64) {
        let copy = self.url;
        let Some(parts) = parse(copy.get()) else { self.say(b"That isn't a web address."); return };
        let Some(region) = Self::region() else { self.say(b"This window has no network."); return };
        let _ = region;
        sex_pdx::serial_record!("[browser.web.open] url={}", core::str::from_utf8(copy.get()).unwrap_or("?"));
        self.request_len = 0;
        let (host, path) = (parts.host, parts.path);
        self.address = numeric(host).unwrap_or([0; 4]);
        let _ = (host, path);
        self.source_len = 0; self.header_end = 0; self.request_sent = 0; self.status = 0; self.peer_done = false; self.truncated = false;
        self.phase = Phase::Up; self.sent = false; self.next_at = 0; self.deadline = now + 60_000;
        self.say(b"Loading...");
    }
    fn fail(&mut self, why: &[u8]) { self.say(why); self.phase = Phase::Close; self.sent = false; }
    fn build_request(&mut self, region: &mut [u8]) -> usize {
        let copy = self.url;
        let Some(p) = parse(copy.get()) else { return 0 };
        let parts: [&[u8]; 5] = [b"GET ", p.path, b" HTTP/1.0\r\nHost: ", p.host, b"\r\nUser-Agent: SexOS Browser\r\nAccept: text/html, text/plain\r\nConnection: close\r\n\r\n"];
        let mut at = 0;
        for part in parts { region[at..at + part.len()].copy_from_slice(part); at += part.len(); }
        at
    }
    /// Sends the next request when one is due, and does the one big step, converting.
    pub fn step(&mut self, now: u64, convert: impl FnOnce(&[u8], &mut [u8]) -> crate::html_text::Converted) {
        if self.phase == Phase::Convert { self.finish(convert); return; }
        if self.phase == Phase::Idle || self.sent || now < self.next_at { return; }
        if now > self.deadline && self.phase != Phase::Close { self.fail(b"The site took too long."); }
        let Some(region) = Self::region() else { self.phase = Phase::Idle; return };
        let copy = self.url;
        let Some(p) = parse(copy.get()) else { self.phase = Phase::Idle; return };
        match self.phase {
            Phase::Up => self.call(w::UP, 0),
            Phase::ResolveAsk => { region[..p.host.len()].copy_from_slice(p.host); self.call(w::RESOLVE, p.host.len() as u64); }
            Phase::ResolveWait => self.call(w::RESOLVE, 0),
            Phase::Connect => self.call(w::CONNECT, w::address_port(self.address, p.port)),
            Phase::Opening => self.call(w::STATE, 0),
            Phase::TlsStart => { region[..p.host.len()].copy_from_slice(p.host); self.call(w::TLS_START, p.host.len() as u64); }
            Phase::TlsWait => self.call(w::TLS_STAGE, 0),
            Phase::Send => {
                let len = self.build_request(region);
                self.request_len = len;
                region.copy_within(self.request_sent..len, 0);
                self.call(w::SEND, (len - self.request_sent) as u64);
            }
            Phase::FinishRequest => self.call(w::FINISH_REQUEST, 0),
            Phase::Receive => self.call(w::RECEIVE, 32 * 1024),
            Phase::CheckPeer => self.call(w::STATE, 0),
            Phase::Close => self.call(w::CLOSE, 0),
            _ => {}
        }
    }
    fn finish(&mut self, convert: impl FnOnce(&[u8], &mut [u8]) -> crate::html_text::Converted) {
        let source = unsafe { &(&*core::ptr::addr_of!(SOURCE))[self.header_end..self.source_len] };
        let text = unsafe { &mut *core::ptr::addr_of_mut!(TEXT) };
        let head = unsafe { &(&*core::ptr::addr_of!(SOURCE))[..self.header_end] };
        let is_html = find(head, b"text/html").is_some() || crate::html_text::looks_like_html(source);
        if is_html {
            let converted = convert(source, &mut text[..]);
            self.text_len = converted.len.min(TEXT_BYTES);
            self.links.targets = converted.targets;
            self.links.count = converted.links;
        } else {
            let n = source.len().min(TEXT_BYTES);
            text[..n].copy_from_slice(&source[..n]);
            self.text_len = n;
            self.links.count = 0;
        }
        sex_pdx::serial_record!("[browser.web.page] source={} text={} links={} html={} truncated={}", source.len(), self.text_len, self.links.count, is_html as u8, self.truncated as u8);
        self.message_len = 0;
        self.phase = Phase::Idle;
        self.dirty = true;
    }
    fn headers_done(&mut self, now: u64) -> bool {
        let source = unsafe { &(&*core::ptr::addr_of!(SOURCE))[..self.source_len] };
        let Some(end) = find(source, b"\r\n\r\n") else { return false };
        self.header_end = end + 4;
        let head = &source[..end];
        let line_end = find(head, b"\r\n").unwrap_or(head.len());
        let code = head[..line_end].split(|b| *b == b' ').nth(1).map_or(0, |c| c.iter().fold(0u32, |v, b| v.wrapping_mul(10).wrapping_add((*b as u32).wrapping_sub(48))));
        self.status = code;
        sex_pdx::serial_record!("[browser.web.status.code] {}", code);
        if (300..400).contains(&code) {
            if let Some(at) = find(head, b"\r\nLocation:") {
                let v = &head[at + 11..];
                let v = &v[..find(v, b"\r\n").unwrap_or(v.len())];
                let v = &v[v.iter().position(|b| *b != b' ').unwrap_or(v.len())..];
                let mut out = [0u8; URL_MAX];
                if self.redirects < MAX_REDIRECTS {
                    if let Some(len) = resolve(self.url.get(), v, &mut out) {
                        self.redirects += 1;
                        self.url.set(&out[..len]);
                        self.phase = Phase::Close;
                        self.status = 1;
                        self.next_at = now;
                        return true;
                    }
                }
            }
            self.fail(b"Too many redirects.");
        }
        true
    }
    /// True if `word` answered the request in flight.
    pub fn receive(&mut self, word: u64, now: u64) -> bool {
        let Some((seq, status, value)) = w::decode_reply(word) else { return false };
        if !self.sent || seq != self.seq { sex_pdx::serial_record!("[browser.web.reply.stale] seq={} want={} sent={}", seq, self.seq, self.sent as u8); return true; }
        self.sent = false;
        if self.phase != self.logged { self.logged = self.phase; sex_pdx::serial_record!("[browser.web.phase] {:?} status={} value={}", self.phase, status, value); }
        if self.phase == Phase::Close {
            if self.status == 1 { self.open(now); return true; }
            if self.source_len > 0 && self.status != 0 && self.header_end > 0 { self.phase = Phase::Convert; }
            else { self.phase = Phase::Idle; if self.message_len == 0 || self.message.starts_with(b"Loading") { self.say(b"The site closed without answering."); } }
            return true;
        }
        if !matches!(status, w::OK | w::WAIT) {
            self.fail(match status { w::OFFLINE => b"No network.", w::NO_NAME => b"Can't find that site.", w::BUSY => b"The network is busy. Try again.", w::DENIED => b"Not allowed.", _ => b"Network error." });
            return true;
        }
        let Some(region) = Self::region() else { self.phase = Phase::Idle; return true };
        match self.phase {
            Phase::Up => {
                if status == w::WAIT { self.next_at = now + POLL * 20; return true; }
                let copy = self.url;
                let host_is_numeric = parse(copy.get()).is_some_and(|p| numeric(p.host).is_some());
                self.phase = if host_is_numeric { Phase::Connect } else { Phase::ResolveAsk };
            }
            Phase::ResolveAsk => { self.phase = Phase::ResolveWait; self.next_at = now + POLL; }
            Phase::ResolveWait => {
                if status == w::WAIT { self.next_at = now + POLL; return true; }
                self.address.copy_from_slice(&region[..4]);
                self.phase = Phase::Connect;
            }
            Phase::Connect => { self.phase = Phase::Opening; self.next_at = now + POLL; }
            Phase::Opening => match value {
                w::STATE_OPEN => {
                    let copy = self.url;
                    self.phase = if parse(copy.get()).is_some_and(|p| p.tls) { Phase::TlsStart } else { Phase::Send };
                }
                w::STATE_OPENING => self.next_at = now + POLL,
                _ => self.fail(b"Can't connect to that site."),
            },
            Phase::TlsStart => { self.phase = Phase::TlsWait; self.next_at = now + POLL; }
            Phase::TlsWait => match value {
                w::TLS_READY => self.phase = Phase::Send,
                w::TLS_HANDSHAKE => self.next_at = now + POLL,
                w::TLS_FAILED_CERTIFICATE => self.fail(b"This site's certificate isn't trusted."),
                w::TLS_FAILED_SIGNATURE => self.fail(b"This site's signature isn't valid."),
                _ => self.fail(b"Secure connection failed."),
            },
            Phase::Send => {
                self.request_sent += value as usize;
                if self.request_sent < self.request_len { self.next_at = now + POLL; }
                else {
                    let copy = self.url;
                    self.phase = if parse(copy.get()).is_some_and(|p| p.tls) { Phase::FinishRequest } else { Phase::Receive };
                }
            }
            Phase::FinishRequest => self.phase = Phase::Receive,
            Phase::Receive => {
                if value == 0 {
                    // Bytes can land between two calls, so a closed peer gets one last read.
                    self.phase = if self.peer_done { Phase::Close } else { Phase::CheckPeer };
                    return true;
                }
                let n = value as usize;
                let source = unsafe { &mut *core::ptr::addr_of_mut!(SOURCE) };
                let take = n.min(SOURCE_BYTES - self.source_len);
                source[self.source_len..self.source_len + take].copy_from_slice(&region[..take]);
                self.source_len += take;
                if take < n { self.truncated = true; }
                self.deadline = now + 60_000;
                if self.header_end == 0 { self.headers_done(now); }
                if self.source_len == SOURCE_BYTES { self.phase = Phase::Close; }
            }
            Phase::CheckPeer => {
                self.peer_done = matches!(value, w::STATE_PEER_CLOSED | w::STATE_CLOSED);
                self.phase = Phase::Receive;
                if !self.peer_done { self.next_at = now + POLL; }
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(base: &str, t: &str) -> String { let mut o = [0; URL_MAX]; let n = resolve(base.as_bytes(), t.as_bytes(), &mut o).unwrap_or(0); String::from_utf8(o[..n].to_vec()).unwrap() }
    #[test] fn resolves_links() {
        assert_eq!(r("https://a.org/x/y.html", "z.html"), "https://a.org/x/z.html");
        assert_eq!(r("https://a.org/x/y.html", "../z"), "https://a.org/z");
        assert_eq!(r("https://a.org/x/y.html", "/z"), "https://a.org/z");
        assert_eq!(r("https://a.org", "z"), "https://a.org/z");
        assert_eq!(r("http://a.org/", "//b.org/c"), "http://b.org/c");
        assert_eq!(r("http://a.org/", "https://b.org/c#d"), "https://b.org/c");
        assert_eq!(r("http://a.org/", "mailto:x@y"), "");
    }
}
