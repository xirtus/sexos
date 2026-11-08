// SPDX-License-Identifier: GPL-3.0-only
//! What a program needs to be a program: a window that shows text, the keys typed into
//! it, and files in its own folder. Everything here goes through grants the program's
//! BOOT row already holds; this crate adds no authority, only less boilerplate.
#![no_std]

use sex_pdx::app_key::Key;
use sex_pdx::text_batch::{hash, Request};
use sex_pdx::{app_lifecycle as lifecycle, pdx_call_checked, SLOT_DISPLAY, SLOT_SHELL};

const TITLE_ID: u64 = 300;
const SHELL: u32 = 3;
const FILES: u32 = 11;

pub enum Event { Key(Key), Close }

/// Retries only while the service queue is full; anything else ends the program.
fn call(slot: u64, op: u64, a: u64, b: u64, c: u64) -> u64 {
    loop {
        match pdx_call_checked(slot, op, a, b, c) {
            Ok(value) => return value,
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => sex_pdx::sys_yield(),
            Err(_) => sex_pdx::task_finish(2),
        }
    }
}

pub struct Window {
    pub target: u32,
    pub task: u32,
    surface: u64,
    region: u64,
    sequence: u32,
    /// Events that arrived while a file request waited for its answer.
    held: [Option<Event>; 16],
}

impl Window {
    /// The row's own window, framed by Shell and on screen.
    pub fn open() -> Self {
        let identity = lifecycle::catalog(lifecycle::SELF_CATALOG_SLOT).unwrap_or_else(|_| sex_pdx::task_finish(3));
        let region = sex_pdx::text_region_client().unwrap_or_else(|_| sex_pdx::task_finish(3));
        let surface = identity.surface_id;
        // The surface must be ours before Shell frames it and before text goes to it.
        call(SLOT_DISPLAY, 0xEC, surface, (56u64 << 32) | 200, (400u64 << 32) | 640);
        call(SLOT_SHELL, 0xFA, surface, TITLE_ID, (identity.target_pd as u64) << 8);
        let mut window = Self { target: identity.target_pd, task: identity.task_id, surface, region, sequence: 0, held: [const { None }; 16] };
        window.show(b"");
        call(SLOT_DISPLAY, lifecycle::OP_APP_PRESENT, surface, 1, 0);
        window
    }
    /// The name in the window's title bar, up to sixteen characters.
    pub fn title(&mut self, name: &[u8]) {
        let mut bytes = [0u8; 16];
        let n = name.len().min(16);
        bytes[..n].copy_from_slice(&name[..n]);
        let words = (u64::from_le_bytes(bytes[..8].try_into().unwrap()), u64::from_le_bytes(bytes[8..].try_into().unwrap()));
        call(SLOT_DISPLAY, lifecycle::OP_APP_TITLE, self.surface, words.0, words.1);
    }
    /// Replaces the window's text. Longer text than the region holds is cut.
    pub fn show(&mut self, text: &[u8]) {
        let text = &text[..text.len().min(sex_pdx::text_region::BYTES as usize)];
        for (i, byte) in text.iter().enumerate() {
            unsafe { core::ptr::write_volatile((self.region as *mut u8).add(i), *byte) };
        }
        self.sequence = self.sequence.wrapping_add(1).max(1);
        let sequence = self.sequence;
        let begin = Request::Begin { sequence, len: text.len(), color: sex_pdx::desktop_style::TEXT };
        let commit = Request::Commit { sequence, hash: hash(text) };
        for request in [begin, Request::Fill { sequence }, commit] {
            let (op, a, b) = request.wire();
            call(SLOT_DISPLAY, op, self.surface, a, b);
        }
    }
    fn event(&self, m: &sex_pdx::PdxMessage) -> Option<Event> {
        if m.caller_pd != SHELL { return None; }
        if lifecycle::close_requested(m.type_id, m.caller_pd, [m.arg0, m.arg1, m.arg2], self.target, self.task) { return Some(Event::Close); }
        if m.type_id == 0x202 { return Key::decode(m.arg0, m.arg1, m.arg2).map(Event::Key); }
        None
    }
    /// The next key or close, waiting for one.
    pub fn next(&mut self) -> Event {
        if let Some(slot) = self.held.iter_mut().find(|e| e.is_some()) { return slot.take().unwrap(); }
        loop {
            let m = sex_pdx::pdx_listen_raw(0);
            if let Some(event) = self.event(&m) { return event; }
        }
    }
    /// Ends the program. Nothing it saved is lost: files are on disk once written.
    pub fn close(self) -> ! { sex_pdx::task_finish(0) }
}

/// Files in the program's own folder. Paths are relative to it; `..` is refused.
pub mod files {
    use super::{Window, FILES};
    use sex_pdx::fs as w;
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

    pub type Error = u8;
    pub use w::{message, NOT_FOUND};

    fn region() -> Result<&'static mut [u8], Error> {
        let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).map_err(|_| w::UNAVAILABLE)?;
        Ok(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
    }
    static mut SEQ: u32 = 0;
    /// Shell tells Files where a new run lives just as the run starts, so a first request
    /// can beat it there. A refusal is retried briefly; a program never bound stays refused.
    fn request(window: &mut Window, kind: u8, path: &[u8], offset: u64, length: u64) -> Result<u32, Error> {
        let mut tries = 0;
        loop {
            match once(window, kind, path, offset, length) {
                Err(w::DENIED) if tries < 50 => { tries += 1; for _ in 0..20 { sex_pdx::sys_yield(); } }
                other => return other,
            }
        }
    }
    /// One request, answered before it returns. Keys and closes that arrive meanwhile wait.
    fn once(window: &mut Window, kind: u8, path: &[u8], offset: u64, length: u64) -> Result<u32, Error> {
        let region = region()?;
        if path.len() > w::PATH_BYTES { return Err(w::NAME); }
        region[..path.len()].copy_from_slice(path);
        let seq = unsafe { SEQ = (SEQ + 1) & 0xFF_FFFF; SEQ };
        let r = w::Request { seq, kind, path_len: path.len() as u16, offset, length };
        let a = r.args();
        loop {
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, w::OP, a[0], a[1], a[2]) {
                Ok(_) => break,
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => sex_pdx::sys_yield(),
                Err(_) => return Err(w::UNAVAILABLE),
            }
        }
        loop {
            let m = sex_pdx::pdx_listen_raw(0);
            if m.type_id == 1 && m.caller_pd == FILES {
                if let Some((got, status, value)) = w::decode_reply(m.arg0) {
                    if got == seq { return if status == w::OK { Ok(value) } else { Err(status) }; }
                }
                continue;
            }
            if let Some(event) = window.event(&m) {
                if let Some(slot) = window.held.iter_mut().find(|e| e.is_none()) { *slot = Some(event); }
            }
        }
    }
    /// The words typed after the program's name, if it was started with any.
    pub fn args<'a>(window: &mut Window, out: &'a mut [u8; w::ARGS_MAX]) -> &'a [u8] {
        let Ok(n) = request(window, w::ARGS, b"", 0, 0) else { return &out[..0] };
        let n = (n as usize).min(w::ARGS_MAX);
        let Ok(region) = region() else { return &out[..0] };
        out[..n].copy_from_slice(&region[w::PATH_BYTES..w::PATH_BYTES + n]);
        &out[..n]
    }
    /// Reads up to `out.len()` bytes of the file from its start.
    pub fn read(window: &mut Window, path: &[u8], out: &mut [u8]) -> Result<usize, Error> {
        let mut done = 0;
        while done < out.len() {
            let want = (out.len() - done).min(w::DATA_MAX);
            let got = request(window, w::READ, path, done as u64, want as u64)? as usize;
            out[done..done + got].copy_from_slice(&region()?[w::PATH_BYTES..w::PATH_BYTES + got]);
            done += got;
            if got < want { break; }
        }
        Ok(done)
    }
    /// Replaces the file with `data`, creating it if needed.
    pub fn write(window: &mut Window, path: &[u8], data: &[u8]) -> Result<(), Error> {
        if data.len() > w::DATA_MAX { return Err(w::NO_SPACE); }
        match request(window, w::STAT, path, 0, 0) {
            Ok(_) => { request(window, w::TRUNCATE, path, 0, 0)?; }
            Err(w::NOT_FOUND) => { request(window, w::CREATE, path, 0, 0)?; }
            Err(e) => return Err(e),
        }
        region()?[w::PATH_BYTES..w::PATH_BYTES + data.len()].copy_from_slice(data);
        let wrote = request(window, w::WRITE, path, 0, data.len() as u64)? as usize;
        if wrote < data.len() { return Err(w::NO_SPACE); }
        Ok(())
    }
}

/// The character a key types, if any, on the keyboard codes Shell passes through.
pub fn typed(key: Key) -> Option<u8> {
    let plain: &[u8; 58] = b"\0\x001234567890-=\0\0qwertyuiop[]\0\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
    let upper: &[u8; 58] = b"\0\0!@#$%^&*()_+\0\0QWERTYUIOP{}\0\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";
    if key.control { return None; }
    let c = *if key.shift { upper } else { plain }.get(key.code as usize)?;
    (c != 0).then_some(c)
}

/// The network, for a run the person started with `run +net`. Otherwise every call is
/// refused. HTTP and HTTPS, one page at a time, through the network service's checked TLS.
pub mod net {
    use super::Window;
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
    use sex_pdx::web as w;

    const NET: u32 = 13;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Error { NotAllowed, Offline, NoName, Connect, Certificate, Secure, TooSlow, Address, Failed }
    pub fn message(e: Error) -> &'static [u8] {
        match e {
            Error::NotAllowed => b"This program was not given the network. Start it with run +net.",
            Error::Offline => b"No network.", Error::NoName => b"That name was not found.", Error::Connect => b"Could not connect.",
            Error::Certificate => b"The site's certificate isn't trusted.", Error::Secure => b"The secure connection failed.",
            Error::TooSlow => b"The site took too long.", Error::Address => b"That address can't be read.", Error::Failed => b"The request failed.",
        }
    }
    fn region() -> Result<&'static mut [u8], Error> {
        let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).map_err(|_| Error::NotAllowed)?;
        Ok(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
    }
    static mut SEQ: u32 = 0;
    fn call(window: &mut Window, kind: u8, value: u64) -> Result<(u8, u32), Error> {
        let seq = unsafe { SEQ = (SEQ + 1) & 0x7F_FFFF; SEQ };
        loop {
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, w::OP, w::request(seq, kind), value, 0) {
                Ok(_) => break,
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => sex_pdx::sys_yield(),
                Err(_) => return Err(Error::NotAllowed),
            }
        }
        let began = sex_pdx::get_ticks();
        loop {
            if sex_pdx::get_ticks().saturating_sub(began) > 30_000 { return Err(Error::TooSlow); }
            let Some(m) = sex_pdx::pdx_try_listen_raw(0) else { sex_pdx::sys_yield(); continue };
            if m.type_id == 1 && m.caller_pd == NET {
                if let Some((got, status, value)) = w::decode_reply(m.arg0) { if got == seq { return Ok((status, value)); } }
                continue;
            }
            if let Some(event) = window.event(&m) {
                if let Some(slot) = window.held.iter_mut().find(|e| e.is_none()) { *slot = Some(event); }
            }
        }
    }
    fn ok(r: (u8, u32)) -> Result<u32, Error> {
        match r.0 { w::OK | w::WAIT => Ok(r.1), w::DENIED => Err(Error::NotAllowed), w::OFFLINE => Err(Error::Offline), w::NO_NAME => Err(Error::NoName), _ => Err(Error::Failed) }
    }
    fn pause() { let until = sex_pdx::get_ticks() + 5; while sex_pdx::get_ticks() < until { sex_pdx::sys_yield(); } }
    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> { hay.windows(needle.len()).position(|w| w == needle) }
    /// Fetches `url` into `out`: the HTTP status and the body's length, cut at `out.len()`.
    pub fn get(window: &mut Window, url: &[u8], out: &mut [u8]) -> Result<(u32, usize), Error> {
        let (rest, tls) = match (url.strip_prefix(b"https://"), url.strip_prefix(b"http://")) { (Some(r), _) => (r, true), (_, Some(r)) => (r, false), _ => return Err(Error::Address) };
        let split = rest.iter().position(|b| *b == b'/').unwrap_or(rest.len());
        let (authority, path) = (&rest[..split], if split == rest.len() { &b"/"[..] } else { &rest[split..] });
        let (host, port) = match authority.iter().position(|b| *b == b':') {
            Some(i) => (&authority[..i], authority[i + 1..].iter().try_fold(0u32, |v, b| b.is_ascii_digit().then(|| v * 10 + (*b - b'0') as u32)).filter(|p| (1..=65535).contains(p)).ok_or(Error::Address)? as u16),
            None => (authority, if tls { 443 } else { 80 }),
        };
        if host.is_empty() || host.len() > 253 { return Err(Error::Address); }
        let region = region()?;
        loop { let r = call(window, w::UP, 0)?; ok(r)?; if r.0 == w::OK { break; } pause(); }
        let numeric = {
            let mut a = [0u8; 4]; let mut parts = host.split(|b| *b == b'.'); let mut good = true;
            for slot in a.iter_mut() { match parts.next().and_then(|p| core::str::from_utf8(p).ok()?.parse::<u8>().ok()) { Some(v) => *slot = v, None => good = false } }
            (good && parts.next().is_none()).then_some(a)
        };
        let address = match numeric {
            Some(a) => a,
            None => {
                region[..host.len()].copy_from_slice(host);
                ok(call(window, w::RESOLVE, host.len() as u64)?)?;
                loop { let r = call(window, w::RESOLVE, 0)?; ok(r)?; if r.0 == w::OK { break; } pause(); }
                [region[0], region[1], region[2], region[3]]
            }
        };
        ok(call(window, w::CONNECT, w::address_port(address, port))?)?;
        loop {
            let state = ok(call(window, w::STATE, 0)?)?;
            if state == w::STATE_OPEN { break; }
            if state != w::STATE_OPENING { let _ = call(window, w::CLOSE, 0); return Err(Error::Connect); }
            pause();
        }
        if tls {
            region[..host.len()].copy_from_slice(host);
            ok(call(window, w::TLS_START, host.len() as u64)?)?;
            loop {
                match ok(call(window, w::TLS_STAGE, 0)?)? {
                    w::TLS_READY => break,
                    w::TLS_HANDSHAKE => pause(),
                    w::TLS_FAILED_CERTIFICATE => { let _ = call(window, w::CLOSE, 0); return Err(Error::Certificate); }
                    _ => { let _ = call(window, w::CLOSE, 0); return Err(Error::Secure); }
                }
            }
        }
        // HTTP/1.0, so the reply needs no chunk decoding and ends when the peer closes.
        let mut request = [0u8; 900];
        let parts: [&[u8]; 5] = [b"GET ", path, b" HTTP/1.0\r\nHost: ", host, b"\r\nUser-Agent: SexOS\r\nAccept: */*\r\nConnection: close\r\n\r\n"];
        let mut n = 0;
        for p in parts { if n + p.len() > request.len() { return Err(Error::Address); } request[n..n + p.len()].copy_from_slice(p); n += p.len(); }
        let mut sent = 0;
        while sent < n { region[..n - sent].copy_from_slice(&request[sent..n]); sent += ok(call(window, w::SEND, (n - sent) as u64)?)? as usize; if sent < n { pause(); } }
        if tls { ok(call(window, w::FINISH_REQUEST, 0)?)?; }
        let (mut got, mut done) = (0usize, false);
        let began = sex_pdx::get_ticks();
        loop {
            if sex_pdx::get_ticks().saturating_sub(began) > 60_000 { let _ = call(window, w::CLOSE, 0); return Err(Error::TooSlow); }
            let k = ok(call(window, w::RECEIVE, (out.len() - got).min(32 * 1024).max(1) as u64)?)? as usize;
            if k > 0 { let take = k.min(out.len() - got); out[got..got + take].copy_from_slice(&region[..take]); got += take; if got == out.len() { break; } continue; }
            if done { break; }
            // Bytes can land between the two calls, so a closed peer gets one last read.
            let state = ok(call(window, w::STATE, 0)?)?;
            done = matches!(state, w::STATE_PEER_CLOSED | w::STATE_CLOSED);
            if !done { pause(); }
        }
        let _ = call(window, w::CLOSE, 0);
        let end = find(&out[..got], b"\r\n\r\n").ok_or(Error::Failed)?;
        let line = &out[..find(&out[..end + 2], b"\r\n").unwrap_or(end)];
        let status = line.split(|b| *b == b' ').nth(1).and_then(|c| core::str::from_utf8(c).ok()?.parse::<u32>().ok()).unwrap_or(0);
        out.copy_within(end + 4..got, 0);
        Ok((status, got - end - 4))
    }
}
