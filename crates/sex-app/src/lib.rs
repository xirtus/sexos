// SPDX-License-Identifier: GPL-3.0-only
//! What a program needs to be a program: a window that shows text, the keys typed into
//! it, and files in its own folder. Everything here goes through grants the program's
//! BOOT row already holds; this crate adds no authority, only less boilerplate.
#![no_std]

extern crate alloc;
pub mod heap;
pub mod sound;

/// Runs `main` on a stack of `bytes` taken from the heap, for programs that need more
/// than the 64 KiB a run starts with (deep layout, script engines, decoders).
#[cfg(all(feature = "heap", target_arch = "x86_64"))]
pub fn with_stack(bytes: usize, main: extern "C" fn() -> !) -> ! {
    let stack: &'static mut [u8] = alloc::vec![0u8; bytes].leak();
    let top = (stack.as_mut_ptr() as usize + bytes) & !15;
    // A call from a 16-byte aligned rsp leaves `main` the alignment the ABI expects.
    unsafe { core::arch::asm!("mov rsp, {top}", "xor ebp, ebp", "call {main}", "ud2", top = in(reg) top, main = in(reg) main, options(noreturn)) }
}

use sex_pdx::app_key::Key;
use sex_pdx::text_batch::{hash, Request};
use sex_pdx::{app_lifecycle as lifecycle, pdx_call_checked, SLOT_DISPLAY, SLOT_SHELL};

const TITLE_ID: u64 = 300;
const SHELL: u32 = 3;
const FILES: u32 = 11;

pub enum Event {
    Key(Key),
    Close,
    /// The window's pixel area is now this many pixels wide and high: redraw to fit.
    Resize { width: usize, height: usize },
    /// The mouse moved, clicked or scrolled over the window's pixel area (positions in
    /// pixels from its top-left; see [`sex_pdx::app_pointer`]).
    Pointer(sex_pdx::app_pointer::Pointer),
}

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
    /// The pixel area Shell last reported.
    size: (usize, usize),
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
        let (w, h) = sex_pdx::pixel_region::area(640, 400);
        let mut window = Self { target: identity.target_pd, task: identity.task_id, surface, region, sequence: 0, size: (w as usize, h as usize), held: [const { None }; 16] };
        window.show(b"");
        call(SLOT_DISPLAY, lifecycle::OP_APP_PRESENT, surface, 1, 0);
        window
    }
    /// The window's pixel surface, 0x00RRGGBB, row-major at the width you show. It holds
    /// at least 800 by 600 pixels and enough for the whole screen; what you don't show is
    /// the program's own scratch.
    pub fn pixels(&self) -> &'static mut [u32] {
        let (base, bytes) = sex_pdx::pixel_region_client().unwrap_or_else(|_| sex_pdx::task_finish(3));
        unsafe { core::slice::from_raw_parts_mut(base as *mut u32, (bytes / 4) as usize) }
    }
    /// The width and height of the window's pixel area, as Shell last reported it
    /// (see [`Event::Resize`]). Pixels shown beyond it are cut off.
    pub fn size(&self) -> (usize, usize) { self.size }
    /// Shows the first `width` by `height` pixels in the window instead of its text.
    pub fn show_pixels(&mut self, width: usize, height: usize) {
        call(SLOT_DISPLAY, sex_pdx::pixel_region::OP, self.surface, ((width as u64) << 32) | height as u64, 0);
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
    fn event(&mut self, m: &sex_pdx::PdxMessage) -> Option<Event> {
        if m.caller_pd != SHELL { return None; }
        if m.type_id == 0x202 {
            if let Some((w, h)) = sex_pdx::pixel_region::resize_decode(m.arg0, m.arg1, m.arg2) {
                self.size = (w as usize, h as usize);
                return Some(Event::Resize { width: w as usize, height: h as usize });
            }
        }
        if m.type_id == 0x202 {
            if let Some(p) = sex_pdx::app_pointer::Pointer::decode(m.arg0, m.arg1, m.arg2) { return Some(Event::Pointer(p)); }
        }
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
    /// The next key or close if one is already waiting, otherwise `None` at once.
    pub fn poll(&mut self) -> Option<Event> {
        if let Some(slot) = self.held.iter_mut().find(|e| e.is_some()) { return slot.take(); }
        while let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
            if let Some(event) = self.event(&m) { return Some(event); }
        }
        None
    }
    /// Like [`Window::next`], but gives up after about `ms` milliseconds.
    pub fn next_within(&mut self, ms: u64) -> Option<Event> {
        let end = sex_pdx::get_ticks().saturating_add(ms);
        loop {
            if let Some(e) = self.poll() { return Some(e); }
            let now = sex_pdx::get_ticks();
            if now >= end { return None; }
            let _ = sex_pdx::sys_wait_self_ticks((end - now).clamp(1, 64));
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
        read_at(window, path, 0, out)
    }
    /// Reads up to `out.len()` bytes of the file from byte `offset`; fewer at its end.
    pub fn read_at(window: &mut Window, path: &[u8], offset: u64, out: &mut [u8]) -> Result<usize, Error> {
        let mut done = 0;
        while done < out.len() {
            let want = (out.len() - done).min(w::IO_MAX);
            let got = request(window, w::READ, path, offset + done as u64, want as u64)? as usize;
            out[done..done + got].copy_from_slice(&region()?[w::PATH_BYTES..w::PATH_BYTES + got]);
            done += got;
            if got < want { break; }
        }
        Ok(done)
    }
    /// Replaces the file with `data`, creating it if needed.
    /// Replaces the file with `data`, of any size: it goes over in pieces as big as the
    /// shared region holds.
    pub fn write(window: &mut Window, path: &[u8], data: &[u8]) -> Result<(), Error> {
        match request(window, w::STAT, path, 0, 0) {
            Ok(_) => { request(window, w::TRUNCATE, path, 0, 0)?; }
            Err(w::NOT_FOUND) => { request(window, w::CREATE, path, 0, 0)?; }
            Err(e) => return Err(e),
        }
        let mut done = 0;
        while done < data.len() {
            let n = (data.len() - done).min(w::IO_MAX);
            region()?[w::PATH_BYTES..w::PATH_BYTES + n].copy_from_slice(&data[done..done + n]);
            let wrote = request(window, w::WRITE, path, done as u64, n as u64)? as usize;
            if wrote < n { return Err(w::NO_SPACE); }
            done += n;
        }
        Ok(())
    }
    /// The file's size, or why there is none.
    pub fn size(window: &mut Window, path: &[u8]) -> Result<u64, Error> {
        request(window, w::STAT, path, 0, 0)?;
        let r = region()?;
        Ok(u64::from_le_bytes(r[w::PATH_BYTES + 8..w::PATH_BYTES + 16].try_into().unwrap_or([0; 8])))
    }
    /// Makes everything written so far durable.
    pub fn sync(window: &mut Window) -> Result<(), Error> {
        request(window, w::SYNC, b"", 0, 0).map(|_| ())
    }
}

/// Movement keys, whichever keyboard code Shell passed (PS/2 set 1 or the normalized one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav { Left, Right, Up, Down, Home, End, Delete }
pub fn nav(key: Key) -> Option<Nav> {
    Some(match key.code {
        0x4B | 0x69 => Nav::Left, 0x4D | 0x6A => Nav::Right, 0x48 | 0x67 => Nav::Up, 0x50 | 0x6C => Nav::Down,
        0x47 | 0x66 => Nav::Home, 0x4F | 0x6B => Nav::End, 0x53 => Nav::Delete,
        _ => return None,
    })
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
    pub enum Error { NotAllowed, Offline, NoName, Connect, Certificate, Secure, TooSlow, Address, Failed, TooLarge, Busy }
    pub fn message(e: Error) -> &'static [u8] {
        match e {
            Error::NotAllowed => b"This program was not given the network. Start it with run +net.",
            Error::Offline => b"No network.", Error::NoName => b"That name was not found.", Error::Connect => b"Could not connect.",
            Error::Certificate => b"The site's certificate isn't trusted.", Error::Secure => b"The secure connection failed.",
            Error::TooSlow => b"The site took too long.", Error::TooLarge => b"The reply is too large.", Error::Address => b"That address can't be read.", Error::Failed => b"The request failed.",
            Error::Busy => b"Too many connections are open.",
        }
    }
    fn region() -> Result<&'static mut [u8], Error> {
        let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).map_err(|_| Error::NotAllowed)?;
        Ok(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
    }
    static mut SEQ: u32 = 0;
    /// This program's flow numbers in use, one bit each.
    static mut FLOWS_HELD: [u64; 4] = [0; 4];
    /// One open flow of this program, by its own number for it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Flow(u8);
    fn take_flow() -> Result<Flow, Error> {
        let held = unsafe { &mut *core::ptr::addr_of_mut!(FLOWS_HELD) };
        let n = (0..w::CLIENT_FLOWS).find(|n| held[*n as usize / 64] & (1 << (n % 64)) == 0).ok_or(Error::Busy)?;
        held[n as usize / 64] |= 1 << (n % 64);
        Ok(Flow(n))
    }
    fn close(window: &mut Window, flow: Flow) {
        let _ = call_on(window, flow, w::CLOSE, 0);
        unsafe { (*core::ptr::addr_of_mut!(FLOWS_HELD))[flow.0 as usize / 64] &= !(1 << (flow.0 % 64)); }
    }
    fn call(window: &mut Window, kind: u8, value: u64) -> Result<(u8, u32), Error> { call_on(window, Flow(0), kind, value) }
    fn call_on(window: &mut Window, flow: Flow, kind: u8, value: u64) -> Result<(u8, u32), Error> {
        let seq = unsafe { SEQ = (SEQ + 1) & 0x7F_FFFF; SEQ };
        loop {
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, w::OP, w::request_on(seq, kind, flow.0), value, 0) {
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
        match r.0 { w::OK | w::WAIT => Ok(r.1), w::DENIED => Err(Error::NotAllowed), w::OFFLINE => Err(Error::Offline), w::NO_NAME => Err(Error::NoName), w::BUSY => Err(Error::Busy), _ => Err(Error::Failed) }
    }
    fn pause() { let until = sex_pdx::get_ticks() + 5; while sex_pdx::get_ticks() < until { sex_pdx::sys_yield(); } }
    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> { hay.windows(needle.len()).position(|w| w == needle) }
    /// Fetches `url` into `out`: the HTTP status and the body's length, cut at `out.len()`.
    pub fn get(window: &mut Window, url: &[u8], out: &mut [u8]) -> Result<(u32, usize), Error> {
        let r = fetch(window, url, out)?;
        out.copy_within(r.body..r.end, 0);
        Ok((r.status, r.end - r.body))
    }
    /// A reply as it arrived in `out`: headers in `..body`, the body in `body..end`.
    #[derive(Clone, Copy, Debug)]
    pub struct Reply { pub status: u32, pub body: usize, pub end: usize }
    impl Reply {
        /// The value of header `name` (ASCII case-insensitive), from the bytes `fetch` filled.
        pub fn header<'a>(&self, out: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
            out[..self.body].split(|b| *b == b'\n').skip(1).find_map(|line| {
                let colon = line.iter().position(|b| *b == b':')?;
                line[..colon].eq_ignore_ascii_case(name).then(|| {
                    let v = &line[colon + 1..];
                    let start = v.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(v.len());
                    let end = v.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(start, |e| e + 1);
                    &v[start..end.max(start)]
                })
            })
        }
    }
    /// Calls `kind` until the service stops answering `BUSY` (its flows or its resolver
    /// are someone else's for now), for up to 30 seconds.
    fn call_patiently(window: &mut Window, flow: Flow, kind: u8, value: u64) -> Result<(u8, u32), Error> {
        let began = sex_pdx::get_ticks();
        loop {
            let r = call_on(window, flow, kind, value)?;
            if r.0 != w::BUSY || sex_pdx::get_ticks().saturating_sub(began) > 30_000 { return Ok(r); }
            pause();
        }
    }
    /// Opens a flow to `host:port`, encrypted when `tls`, and gives back the region the
    /// flow's bytes pass through.
    fn open(window: &mut Window, host: &[u8], port: u16, tls: bool, patient: bool) -> Result<(Flow, &'static mut [u8]), Error> {
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
                ok(call_patiently(window, Flow(0), w::RESOLVE, host.len() as u64)?)?;
                loop { let r = call(window, w::RESOLVE, 0)?; ok(r)?; if r.0 == w::OK { break; } pause(); }
                [region[0], region[1], region[2], region[3]]
            }
        };
        let flow = take_flow()?;
        let fail = |window: &mut Window, e: Error| { close(window, flow); Err(e) };
        match (if patient { call_patiently(window, flow, w::CONNECT, w::address_port(address, port)) } else { call_on(window, flow, w::CONNECT, w::address_port(address, port)) }).and_then(ok) { Ok(_) => {}, Err(e) => return fail(window, e) }
        loop {
            let state = match call_on(window, flow, w::STATE, 0).and_then(ok) { Ok(s) => s, Err(e) => return fail(window, e) };
            if state == w::STATE_OPEN { break; }
            if state != w::STATE_OPENING { return fail(window, Error::Connect); }
            pause();
        }
        if tls {
            region[..host.len()].copy_from_slice(host);
            if let Err(e) = call_on(window, flow, w::TLS_START, host.len() as u64).and_then(ok) { return fail(window, e); }
            loop {
                match call_on(window, flow, w::TLS_STAGE, 0).and_then(ok) {
                    Ok(w::TLS_READY) => break,
                    Ok(w::TLS_HANDSHAKE) => pause(),
                    Ok(w::TLS_FAILED_CERTIFICATE) => return fail(window, Error::Certificate),
                    Ok(_) => return fail(window, Error::Secure),
                    Err(e) => return fail(window, e),
                }
            }
        }
        Ok((flow, region))
    }
    /// Sends all of `bytes` on the open flow, then says the request is whole.
    fn send_all(window: &mut Window, flow: Flow, region: &mut [u8], bytes: &[u8], tls: bool) -> Result<(), Error> {
        let mut sent = 0;
        while sent < bytes.len() {
            let n = (bytes.len() - sent).min(region.len());
            region[..n].copy_from_slice(&bytes[sent..sent + n]);
            let took = ok(call_on(window, flow, w::SEND, n as u64)?)? as usize;
            sent += took;
            if took < n { pause(); }
        }
        if tls { ok(call_on(window, flow, w::FINISH_REQUEST, 0)?)?; }
        Ok(())
    }
    /// A decoded HTTP/1.1 reply: its head, and the body unframed and decompressed.
    pub struct Response { pub head: sex_fetch::Head, pub body: alloc::vec::Vec<u8> }

    /// One HTTP/1.1 request on a flow of its own, read a step at a time, so a program can
    /// keep several going at once (the service makes flows as they are asked for)
    /// and do other work between steps. `start` connects and sends; `poll` reads what
    /// has arrived; `finish` decodes the reply. One that is given up on is `cancel`led,
    /// or its flow stays held until the program ends.
    pub struct Exchange {
        flow: Flow, get_head: bool, raw: alloc::vec::Vec<u8>, cap: usize, max: usize,
        head_end: Option<usize>, framing: Option<sex_fetch::Framing>, chunks: sex_fetch::ChunkScan,
        began: u64, start: u64, idle: u32, calls: u32, empty: u32, peer_done: bool, whole: bool,
    }
    impl Exchange {
        /// Opens a flow, sends `req` (with `cookie`, the jar's value for its address) and
        /// returns once it is sent. Bodies larger than `max` after decoding fail. While
        /// every flow of the service is held, this fails at once with `Busy`, so the
        /// caller can finish what it has going and try again.
        pub fn start(window: &mut Window, req: &sex_fetch::Request, cookie: Option<&str>, max: usize) -> Result<Exchange, Error> { Self::begin(window, req, cookie, max, false) }
        /// `start`, but waiting up to 30 seconds for a flow while others hold them all.
        pub fn start_patiently(window: &mut Window, req: &sex_fetch::Request, cookie: Option<&str>, max: usize) -> Result<Exchange, Error> { Self::begin(window, req, cookie, max, true) }
        fn begin(window: &mut Window, req: &sex_fetch::Request, cookie: Option<&str>, max: usize, patient: bool) -> Result<Exchange, Error> {
            let url = sex_fetch::Url::parse(req.url).map_err(|_| Error::Address)?;
            let bytes = sex_fetch::encode(req, &url, cookie).map_err(|_| Error::Address)?;
            let (flow, region) = open(window, url.host.as_bytes(), url.port, url.secure, patient)?;
            if let Err(e) = send_all(window, flow, region, &bytes, url.secure) { close(window, flow); return Err(e); }
            // Room for the head, the framing and the body; decoding checks `max` again.
            let cap = max.saturating_add(sex_fetch::HEAD_MAX).saturating_add(max / 8 + 4096);
            Ok(Self::sent(flow, req.method.eq_ignore_ascii_case("HEAD"), cap, max))
        }
        fn sent(flow: Flow, get_head: bool, cap: usize, max: usize) -> Exchange {
            let now = sex_pdx::get_ticks();
            Exchange { flow, get_head, raw: alloc::vec::Vec::new(), cap, max,
                head_end: None, framing: None, chunks: sex_fetch::ChunkScan::new(), began: now, start: now, idle: 0, calls: 0, empty: 0, peer_done: false, whole: false }
        }
        /// Polls in a row that found nothing new; a caller with nothing else to do
        /// yields while this is small and sleeps once it grows.
        pub fn idle(&self) -> u32 { self.idle }
        /// Bytes of the reply so far, before decoding.
        pub fn received(&self) -> usize { self.raw.len() }
        fn enough(&mut self) -> bool {
            let seen = &self.raw[..];
            if self.head_end.is_none() {
                match sex_fetch::parse_head(seen) {
                    Ok(Some((h, at))) => { self.head_end = Some(at); self.framing = h.framing(if self.get_head { "HEAD" } else { "GET" }).ok(); }
                    Ok(None) => return false,
                    Err(_) => return true,
                }
            }
            let body = &seen[self.head_end.unwrap_or(0)..];
            match self.framing {
                Some(sex_fetch::Framing::Chunked) => !matches!(self.chunks.advance(body), Ok(None)),
                Some(f) => sex_fetch::complete(f, body).unwrap_or(true),
                None => true,
            }
        }
        /// One step: takes what has arrived. True once the reply is whole (or the peer
        /// closed); the flow is closed then. An error closes it too.
        pub fn poll(&mut self, window: &mut Window) -> Result<bool, Error> {
            if self.whole { return Ok(true); }
            // Sixty seconds without a byte is too slow; a long download that keeps moving is not.
            if sex_pdx::get_ticks().saturating_sub(self.began) > 60_000 { self.cancel(window); return Err(Error::TooSlow); }
            let room = (self.cap - self.raw.len()).min(32 * 1024).max(1) as u64;
            let k = match call_on(window, self.flow, w::RECEIVE, room).and_then(ok) { Ok(k) => k as usize, Err(e) => { self.cancel(window); return Err(e); } };
            self.calls += 1;
            if k > 0 {
                let take = k.min(self.cap - self.raw.len());
                let region = region().map_err(|e| { self.cancel(window); e })?;
                self.raw.extend_from_slice(&region[..take]);
                self.began = sex_pdx::get_ticks();
                self.idle = 0;
                if self.raw.len() == self.cap || self.enough() { self.end(window); }
                return Ok(self.whole);
            }
            self.empty += 1;
            // Bytes can land between the two calls, so a closed peer gets one last read.
            if self.peer_done { self.end(window); return Ok(true); }
            let state = match call_on(window, self.flow, w::STATE, 0).and_then(ok) { Ok(s) => s, Err(e) => { self.cancel(window); return Err(e); } };
            self.peer_done = matches!(state, w::STATE_PEER_CLOSED | w::STATE_CLOSED);
            self.idle += 1;
            Ok(false)
        }
        fn end(&mut self, window: &mut Window) {
            self.whole = true;
            close(window, self.flow);
            sex_pdx::serial_record!("[net.receive.stats] bytes={} calls={} empty={} ticks={} flow={}", self.raw.len(), self.calls, self.empty, sex_pdx::get_ticks() - self.start, self.flow.0);
        }
        /// Gives the exchange up and lets its flow go.
        pub fn cancel(&mut self, window: &mut Window) { if !self.whole { self.whole = true; close(window, self.flow); } }
        /// The reply, unframed and decompressed. Call once `poll` said true.
        pub fn finish(self) -> Result<Response, Error> {
            let (head, at) = match sex_fetch::parse_head(&self.raw) { Ok(Some(h)) => h, _ => return Err(Error::Failed) };
            let method = if self.get_head { "HEAD" } else { "GET" };
            match sex_fetch::decode(&head, method, &self.raw[at..], self.max) {
                Ok(body) => Ok(Response { head, body }),
                Err(sex_fetch::Error::TooLarge) => Err(Error::TooLarge),
                Err(_) => Err(Error::Failed),
            }
        }
    }
    /// Waits on one exchange to the end, yielding while bytes are flowing.
    fn wait(window: &mut Window, ex: &mut Exchange) -> Result<(), Error> {
        // While bytes are flowing the next frame is usually a scheduler turn away:
        // yield first, and sleep a tick only once the flow has gone quiet.
        while !ex.poll(window)? { if ex.idle() > 64 { pause(); } else if ex.idle() > 0 { sex_pdx::sys_yield(); } }
        Ok(())
    }

    /// Sends one HTTP/1.1 request (any method, headers, body) and reads the reply,
    /// undoing chunked framing and gzip/deflate. `cookie` is the jar's `Cookie` value
    /// for this address. Redirects are the caller's to follow (each hop may set
    /// cookies). Bodies larger than `max` after decoding fail.
    pub fn request(window: &mut Window, req: &sex_fetch::Request, cookie: Option<&str>, max: usize) -> Result<Response, Error> {
        let mut ex = Exchange::begin(window, req, cookie, max, true)?;
        wait(window, &mut ex)?;
        ex.finish()
    }

    /// Fetches `url` into `out` and keeps the headers, so a caller can follow redirects
    /// and read the content type.
    pub fn fetch(window: &mut Window, url: &[u8], out: &mut [u8]) -> Result<Reply, Error> {
        let (rest, tls) = match (url.strip_prefix(b"https://"), url.strip_prefix(b"http://")) { (Some(r), _) => (r, true), (_, Some(r)) => (r, false), _ => return Err(Error::Address) };
        let split = rest.iter().position(|b| *b == b'/').unwrap_or(rest.len());
        let (authority, path) = (&rest[..split], if split == rest.len() { &b"/"[..] } else { &rest[split..] });
        let (host, port) = match authority.iter().position(|b| *b == b':') {
            Some(i) => (&authority[..i], authority[i + 1..].iter().try_fold(0u32, |v, b| b.is_ascii_digit().then(|| v * 10 + (*b - b'0') as u32)).filter(|p| (1..=65535).contains(p)).ok_or(Error::Address)? as u16),
            None => (authority, if tls { 443 } else { 80 }),
        };
        // HTTP/1.0, so the reply needs no chunk decoding and ends when the peer closes.
        let mut request = [0u8; 900];
        let parts: [&[u8]; 5] = [b"GET ", path, b" HTTP/1.0\r\nHost: ", host, b"\r\nUser-Agent: SexOS\r\nAccept: */*\r\nConnection: close\r\n\r\n"];
        let mut n = 0;
        for p in parts { if n + p.len() > request.len() { return Err(Error::Address); } request[n..n + p.len()].copy_from_slice(p); n += p.len(); }
        let (flow, region) = open(window, host, port, tls, true)?;
        if let Err(e) = send_all(window, flow, region, &request[..n], tls) { close(window, flow); return Err(e); }
        let mut ex = Exchange::sent(flow, false, out.len().max(1), out.len());
        wait(window, &mut ex)?;
        let got = ex.raw.len().min(out.len());
        out[..got].copy_from_slice(&ex.raw[..got]);
        let end = find(&out[..got], b"\r\n\r\n").ok_or(Error::Failed)?;
        let line = &out[..find(&out[..end + 2], b"\r\n").unwrap_or(end)];
        let status = line.split(|b| *b == b' ').nth(1).and_then(|c| core::str::from_utf8(c).ok()?.parse::<u32>().ok()).unwrap_or(0);
        Ok(Reply { status, body: end + 4, end: got })
    }
}
