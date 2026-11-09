// SPDX-License-Identifier: GPL-3.0-only
//! What a program needs to be a program: a window that shows text, the keys typed into
//! it, and files in its own folder. Everything here goes through grants the program's
//! BOOT row already holds; this crate adds no authority, only less boilerplate.
#![no_std]

extern crate alloc;
pub mod heap;
pub mod code;
pub mod thread;
pub mod tls;
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
/// refused. HTTP and HTTPS through the network service's checked TLS, and TCP and UDP
/// sockets ([`net::TcpStream`], [`net::TcpListener`], [`net::UdpSocket`]), over IPv4 or IPv6.
pub mod net {
    use super::Window;
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
    use sex_pdx::web as w;

    const NET: u32 = 13;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Error { NotAllowed, Offline, NoName, Connect, Certificate, Secure, TooSlow, Address, Failed, TooLarge, Busy, InUse }
    pub fn message(e: Error) -> &'static [u8] {
        match e {
            Error::NotAllowed => b"This program was not given the network. Start it with run +net.",
            Error::Offline => b"No network.", Error::NoName => b"That name was not found.", Error::Connect => b"Could not connect.",
            Error::Certificate => b"The site's certificate isn't trusted.", Error::Secure => b"The secure connection failed.",
            Error::TooSlow => b"The site took too long.", Error::TooLarge => b"The reply is too large.", Error::Address => b"That address can't be read.", Error::Failed => b"The request failed.",
            Error::Busy => b"Too many connections are open.",
            Error::InUse => b"That port is in use, or not one a program may take.",
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
        match r.0 { w::OK | w::WAIT => Ok(r.1), w::DENIED => Err(Error::NotAllowed), w::OFFLINE => Err(Error::Offline), w::NO_NAME => Err(Error::NoName), w::BUSY => Err(Error::Busy), w::IN_USE => Err(Error::InUse), _ => Err(Error::Failed) }
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
    /// A decoded HTTP/1.1 reply: its head, and the body unframed and decompressed.
    pub struct Response { pub head: sex_fetch::Head, pub body: alloc::vec::Vec<u8> }

    /// An internet address of either family. Data, not authority: holding one reaches
    /// nothing a run not given the network could not.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum IpAddr { V4([u8; 4]), V6([u8; 16]) }
    impl IpAddr {
        /// `10.0.2.2`, `fd00::1` or `[fd00::1]`. Names are for [`resolve`].
        pub fn parse(text: &[u8]) -> Option<IpAddr> {
            let text = text.strip_prefix(b"[").and_then(|t| t.strip_suffix(b"]")).unwrap_or(text);
            if text.contains(&b':') { return parse6(text).map(IpAddr::V6); }
            let mut a = [0u8; 4];
            let mut parts = text.split(|b| *b == b'.');
            for slot in a.iter_mut() {
                let p = parts.next()?;
                if p.is_empty() || p.len() > 3 || !p.iter().all(u8::is_ascii_digit) { return None; }
                *slot = core::str::from_utf8(p).ok()?.parse().ok()?;
            }
            parts.next().is_none().then_some(IpAddr::V4(a))
        }
        pub fn is_ipv6(&self) -> bool { matches!(self, IpAddr::V6(_)) }
    }
    fn parse6(text: &[u8]) -> Option<[u8; 16]> {
        fn groups(part: &[u8], out: &mut [u16; 8]) -> Option<usize> {
            if part.is_empty() { return Some(0); }
            let mut n = 0;
            for g in part.split(|b| *b == b':') {
                if g.is_empty() || g.len() > 4 || n == 8 { return None; }
                out[n] = u16::from_str_radix(core::str::from_utf8(g).ok()?, 16).ok()?; n += 1;
            }
            Some(n)
        }
        let (mut head, mut tail) = ([0u16; 8], [0u16; 8]);
        let split = text.windows(2).position(|w| w == b"::");
        let (h, t) = match split {
            Some(i) => (groups(&text[..i], &mut head)?, groups(&text[i + 2..], &mut tail)?),
            None => (groups(text, &mut head)?, 0),
        };
        if split.is_none() && h != 8 || split.is_some() && h + t > 7 { return None; }
        let mut words = [0u16; 8];
        words[..h].copy_from_slice(&head[..h]);
        words[8 - t..].copy_from_slice(&tail[..t]);
        let mut a = [0u8; 16];
        for (i, w) in words.iter().enumerate() { a[2 * i..2 * i + 2].copy_from_slice(&w.to_be_bytes()); }
        Some(a)
    }
    /// The usual short forms: `10.0.2.2`, and RFC 5952's for IPv6 (`fd00:5e::1`).
    impl core::fmt::Display for IpAddr {
        fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            match self {
                IpAddr::V4(a) => write!(f, "{}.{}.{}.{}", a[0], a[1], a[2], a[3]),
                IpAddr::V6(a) => {
                    let w: [u16; 8] = core::array::from_fn(|i| u16::from_be_bytes([a[2 * i], a[2 * i + 1]]));
                    // The longest run of two or more zero groups (the first of equals) becomes `::`.
                    let (mut best, mut at) = ((0, 0), 0);
                    while at < 8 {
                        let len = w[at..].iter().take_while(|g| **g == 0).count();
                        if len > best.1 { best = (at, len); }
                        at += len.max(1);
                    }
                    let mut i = 0;
                    while i < 8 {
                        if best.1 >= 2 && i == best.0 { f.write_str(if i == 0 { "::" } else { ":" })?; i += best.1; continue; }
                        write!(f, "{:x}", w[i])?;
                        if i < 7 { f.write_str(":")?; }
                        i += 1;
                    }
                    Ok(())
                }
            }
        }
    }
    fn put_ip(out: &mut [u8], a: &IpAddr) {
        match a { IpAddr::V4(b) => w::put_address(out, w::FAMILY_V4, b), IpAddr::V6(b) => w::put_address(out, w::FAMILY_V6, b) }
    }
    fn ip_of(bytes: &[u8]) -> Option<IpAddr> {
        let (family, b) = w::address_of(bytes)?;
        Some(if family == w::FAMILY_V4 { IpAddr::V4(b.try_into().ok()?) } else { IpAddr::V6(b.try_into().ok()?) })
    }
    fn endpoint(bytes: &[u8]) -> Option<(IpAddr, u16)> { Some((ip_of(bytes)?, u16::from_be_bytes([*bytes.get(17)?, *bytes.get(18)?]))) }
    /// Lets a flow number go without telling the service (it holds nothing under it).
    fn release(flow: Flow) { unsafe { (*core::ptr::addr_of_mut!(FLOWS_HELD))[flow.0 as usize / 64] &= !(1 << (flow.0 % 64)); } }
    /// Waits for the network: a DHCP lease or a global IPv6 address. Thirty seconds at most.
    pub fn up(window: &mut Window) -> Result<(), Error> {
        let began = sex_pdx::get_ticks();
        loop {
            if call(window, w::UP, 0).and_then(ok_status)? == w::OK { return Ok(()); }
            if sex_pdx::get_ticks().saturating_sub(began) > 30_000 { return Err(Error::Offline); }
            pause();
        }
    }
    /// `ok`, keeping the status (OK or WAIT).
    fn ok_status(r: (u8, u32)) -> Result<u8, Error> { ok(r).map(|_| r.0) }
    /// The machine's addresses and their prefix lengths: the leased IPv4 address and the
    /// IPv6 ones (link-local and those SLAAC gave).
    pub fn addresses(window: &mut Window) -> Result<alloc::vec::Vec<(IpAddr, u8)>, Error> {
        let n = ok(call(window, w::ADDRESSES, 0)?)? as usize;
        let region = region()?;
        Ok((0..n).filter_map(|i| { let e = &region[i * (w::ADDRESS_BYTES + 1)..]; Some((ip_of(e)?, e[w::ADDRESS_BYTES])) }).collect())
    }
    /// The question a blocking `resolve` holds, apart from every exchange's flow.
    const RESOLVE_HOLDER: Flow = Flow(w::CLIENT_FLOWS);
    /// The addresses of `host`, the one to try first first: IPv6 when the machine has a
    /// global IPv6 address and the name has one, then IPv4. An address is itself.
    pub fn resolve(window: &mut Window, host: &[u8]) -> Result<alloc::vec::Vec<IpAddr>, Error> {
        if let Some(a) = IpAddr::parse(host) { return Ok(alloc::vec![a]); }
        let name = core::str::from_utf8(host).map_err(|_| Error::Address)?;
        if name.is_empty() || name.len() > 253 { return Err(Error::Address); }
        up(window)?;
        if let Some(found) = remembered(name) { return Ok(found.into_iter().flatten().collect()); }
        let began = sex_pdx::get_ticks();
        let held = || unsafe { *core::ptr::addr_of!(RESOLVING) };
        loop {
            if sex_pdx::get_ticks().saturating_sub(began) > 30_000 { return Err(Error::TooSlow); }
            if held().is_none() {
                region()?[..host.len()].copy_from_slice(host);
                let r = call(window, w::RESOLVE_ALL, host.len() as u64)?;
                if r.0 != w::BUSY { ok(r)?; break; }
            }
            pause();
        }
        unsafe { RESOLVING = Some(RESOLVE_HOLDER); }
        let answer = loop {
            let r = call(window, w::RESOLVE_ALL, 0);
            match r {
                Ok((w::WAIT, _)) if sex_pdx::get_ticks().saturating_sub(began) <= 30_000 => pause(),
                Ok((w::WAIT, _)) => break Err(Error::TooSlow),
                Ok(r) => break ok(r),
                Err(e) => break Err(e),
            }
        };
        unsafe { RESOLVING = None; }
        let n = answer? as usize;
        let region = region()?;
        let mut found = [None; 2];
        for (i, slot) in found.iter_mut().enumerate().take(n) { *slot = ip_of(&region[i * w::ADDRESS_BYTES..]); }
        remember(name, found);
        Ok(found.into_iter().flatten().collect())
    }

    /// A TCP connection of this program, over IPv4 or IPv6. Calls that wait have a
    /// non-waiting form, so a program can keep several going and draw between them.
    pub struct TcpStream { flow: Flow, peer: (IpAddr, u16) }
    impl TcpStream {
        /// Starts connecting to `address` and returns at once; `poll_open` says when it is.
        pub fn begin(window: &mut Window, address: &IpAddr, port: u16) -> Result<TcpStream, Error> {
            let flow = take_flow()?;
            put_ip(region().map_err(|e| { release(flow); e })?, address);
            match call_on(window, flow, w::CONNECT_TO, port as u64).and_then(ok) {
                Ok(_) => Ok(TcpStream { flow, peer: (*address, port) }),
                Err(e) => { release(flow); Err(e) }
            }
        }
        /// True once connected (or connected and already closed by the peer).
        pub fn poll_open(&self, window: &mut Window) -> Result<bool, Error> {
            match ok(call_on(window, self.flow, w::STATE, 0)?)? {
                w::STATE_OPENING => Ok(false),
                w::STATE_CLOSED => Err(Error::Connect),
                _ => Ok(true),
            }
        }
        /// Connects to `address`, waiting up to thirty seconds.
        pub fn connect_to(window: &mut Window, address: &IpAddr, port: u16) -> Result<TcpStream, Error> { Self::connect_within(window, address, port, 30_000) }
        fn connect_within(window: &mut Window, address: &IpAddr, port: u16, ms: u64) -> Result<TcpStream, Error> {
            let stream = Self::begin(window, address, port)?;
            let began = sex_pdx::get_ticks();
            loop {
                match stream.poll_open(window) {
                    Ok(true) => return Ok(stream),
                    Ok(false) if sex_pdx::get_ticks().saturating_sub(began) < ms => pause(),
                    Ok(false) => { close(window, stream.flow); return Err(Error::Connect); }
                    Err(e) => { close(window, stream.flow); return Err(e); }
                }
            }
        }
        /// Connects to `host` (a name or an address) on `port`, trying its addresses in
        /// turn: IPv6 first when it has one and the machine can use it, then IPv4.
        pub fn connect(window: &mut Window, host: &[u8], port: u16) -> Result<TcpStream, Error> {
            let found = resolve(window, host)?;
            let mut last = Error::NoName;
            for (i, a) in found.iter().enumerate() {
                // An address with another after it gets less time, so a dead route doesn't hold up the next.
                match Self::connect_within(window, a, port, if i + 1 < found.len() { 3_000 } else { 30_000 }) { Ok(s) => return Ok(s), Err(e) => last = e }
            }
            Err(last)
        }
        /// The address and port at the other end.
        pub fn peer(&self) -> (IpAddr, u16) { self.peer }
        /// Sends what fits now; 0 when the send queue is full.
        pub fn write(&mut self, window: &mut Window, bytes: &[u8]) -> Result<usize, Error> {
            let n = bytes.len().min(w::REGION_BYTES);
            region()?[..n].copy_from_slice(&bytes[..n]);
            Ok(ok(call_on(window, self.flow, w::SEND, n as u64)?)? as usize)
        }
        /// Sends all of `bytes`, waiting for room (thirty seconds at most without any).
        pub fn write_all(&mut self, window: &mut Window, mut bytes: &[u8]) -> Result<(), Error> {
            let mut moved = sex_pdx::get_ticks();
            while !bytes.is_empty() {
                let n = self.write(window, bytes)?;
                bytes = &bytes[n..];
                if n > 0 { moved = sex_pdx::get_ticks(); } else if sex_pdx::get_ticks().saturating_sub(moved) > 30_000 { return Err(Error::TooSlow); } else { pause(); }
            }
            Ok(())
        }
        /// Reads what has arrived; 0 when nothing has yet (see `is_closed`).
        pub fn read(&mut self, window: &mut Window, out: &mut [u8]) -> Result<usize, Error> {
            let room = out.len().min(32 * 1024).max(1);
            let k = (ok(call_on(window, self.flow, w::RECEIVE, room as u64)?)? as usize).min(out.len());
            out[..k].copy_from_slice(&region()?[..k]);
            Ok(k)
        }
        /// True once the peer has closed its side (or the connection is gone).
        pub fn is_closed(&self, window: &mut Window) -> Result<bool, Error> {
            Ok(matches!(ok(call_on(window, self.flow, w::STATE, 0)?)?, w::STATE_PEER_CLOSED | w::STATE_CLOSED))
        }
        /// Waits up to `ms` for bytes: how many came, or 0 once the peer has closed and
        /// everything it sent has been read.
        pub fn read_within(&mut self, window: &mut Window, out: &mut [u8], ms: u64) -> Result<usize, Error> {
            let began = sex_pdx::get_ticks();
            let mut closed = false;
            loop {
                let k = self.read(window, out)?;
                if k > 0 || closed { return Ok(k); }
                // Bytes can land between the two calls, so a closed peer gets one last read.
                closed = self.is_closed(window)?;
                if !closed { if sex_pdx::get_ticks().saturating_sub(began) > ms { return Err(Error::TooSlow); } pause(); }
            }
        }
        /// Sends this side's FIN after what is queued; the stream can still read.
        pub fn shutdown(&mut self, window: &mut Window) -> Result<(), Error> { ok(call_on(window, self.flow, w::SHUTDOWN, 0)?).map(|_| ()) }
        /// Closes this side and lets the connection finish on its own.
        pub fn close(mut self, window: &mut Window) { let _ = self.shutdown(window); close(window, self.flow); }
    }

    /// A TCP port this program takes connections on, over either family. The run must
    /// have the network (`run +net`), and the port must be free and at most
    /// `sex_pdx::web::PROGRAM_PORT_MAX`, not 68.
    pub struct TcpListener { flow: Flow, port: u16 }
    impl TcpListener {
        pub fn bind(window: &mut Window, port: u16) -> Result<TcpListener, Error> {
            let flow = take_flow()?;
            match call_on(window, flow, w::LISTEN, port as u64).and_then(ok) {
                Ok(_) => Ok(TcpListener { flow, port }),
                Err(e) => { release(flow); Err(e) }
            }
        }
        pub fn port(&self) -> u16 { self.port }
        /// A connection a peer made, if one has; the listener keeps listening.
        pub fn accept(&mut self, window: &mut Window) -> Result<Option<TcpStream>, Error> {
            let new = take_flow()?;
            let r = call_on(window, self.flow, w::ACCEPT, new.0 as u64).and_then(ok_status);
            match r {
                Ok(w::OK) => {
                    let peer = endpoint(region()?).unwrap_or((IpAddr::V4([0; 4]), 0));
                    Ok(Some(TcpStream { flow: new, peer }))
                }
                Ok(_) => { release(new); Ok(None) }
                Err(e) => { release(new); Err(e) }
            }
        }
        /// Waits up to `ms` for a connection.
        pub fn accept_within(&mut self, window: &mut Window, ms: u64) -> Result<TcpStream, Error> {
            let began = sex_pdx::get_ticks();
            loop {
                if let Some(s) = self.accept(window)? { return Ok(s); }
                if sex_pdx::get_ticks().saturating_sub(began) > ms { return Err(Error::TooSlow); }
                pause();
            }
        }
        pub fn close(self, window: &mut Window) { close(window, self.flow); }
    }

    /// A UDP socket of this program, over either family.
    pub struct UdpSocket { flow: Flow, port: u16 }
    impl UdpSocket {
        /// On `port`, or on one the service picks when 0. The same rules as `TcpListener`.
        pub fn bind(window: &mut Window, port: u16) -> Result<UdpSocket, Error> {
            let flow = take_flow()?;
            match call_on(window, flow, w::UDP_BIND, port as u64).and_then(ok) {
                Ok(port) => Ok(UdpSocket { flow, port: port as u16 }),
                Err(e) => { release(flow); Err(e) }
            }
        }
        pub fn local_port(&self) -> u16 { self.port }
        /// Queues one datagram; false when the queue has no room yet. Payloads past one
        /// frame (1452 bytes over IPv6, 1472 over IPv4) are refused.
        pub fn send_to(&mut self, window: &mut Window, bytes: &[u8], to: &IpAddr, port: u16) -> Result<bool, Error> {
            if bytes.len() > w::REGION_BYTES - w::ENDPOINT_BYTES { return Err(Error::TooLarge); }
            let region = region()?;
            put_ip(region, to);
            region[17..19].copy_from_slice(&port.to_be_bytes());
            region[w::ENDPOINT_BYTES..w::ENDPOINT_BYTES + bytes.len()].copy_from_slice(bytes);
            let r = call_on(window, self.flow, w::UDP_SEND, bytes.len() as u64)?;
            if r.0 == w::INVALID { return Err(Error::TooLarge); }
            Ok(ok_status(r)? == w::OK)
        }
        /// One datagram that came, if any: its length (cut at `out.len()`), sender and port.
        pub fn recv_from(&mut self, window: &mut Window, out: &mut [u8]) -> Result<Option<(usize, IpAddr, u16)>, Error> {
            let r = call_on(window, self.flow, w::UDP_RECEIVE, out.len() as u64)?;
            if ok_status(r)? != w::OK { return Ok(None); }
            let region = region()?;
            let n = (r.1 as usize).min(out.len());
            out[..n].copy_from_slice(&region[w::ENDPOINT_BYTES..w::ENDPOINT_BYTES + n]);
            let (from, port) = endpoint(region).ok_or(Error::Failed)?;
            Ok(Some((n, from, port)))
        }
        /// Waits up to `ms` for a datagram.
        pub fn recv_from_within(&mut self, window: &mut Window, out: &mut [u8], ms: u64) -> Result<(usize, IpAddr, u16), Error> {
            let began = sex_pdx::get_ticks();
            loop {
                if let Some(d) = self.recv_from(window, out)? { return Ok(d); }
                if sex_pdx::get_ticks().saturating_sub(began) > ms { return Err(Error::TooSlow); }
                pause();
            }
        }
        pub fn close(self, window: &mut Window) { close(window, self.flow); }
    }

    /// One HTTP/1.1 request on a flow of its own, read a step at a time, so a program can
    /// keep several going at once (the service makes flows as they are asked for)
    /// and do other work between steps. `start` connects and sends; `poll` reads what
    /// has arrived; `finish` decodes the reply. One that is given up on is `cancel`led,
    /// or its flow stays held until the program ends.
    pub struct Exchange {
        flow: Flow, get_head: bool, raw: alloc::vec::Vec<u8>, cap: usize, max: usize,
        head_end: Option<usize>, framing: Option<sex_fetch::Framing>, chunks: sex_fetch::ChunkScan,
        began: u64, start: u64, idle: u32, calls: u32, empty: u32, peer_done: bool, whole: bool,
        setup: Setup, host: alloc::string::String, port: u16, tls: bool, out: alloc::vec::Vec<u8>,
        /// Where the name led, in the order to try, and which one is being tried.
        addresses: [Option<IpAddr>; 2], trying: usize, connect_at: u64,
        /// On a connection an earlier exchange left open (keep-alive).
        reused: bool,
        /// Where the reply's body ended exactly, once its framing says so.
        body_end: Option<usize>,
    }
    /// Connections whose last reply ended cleanly and that the site keeps open: host,
    /// port, TLS, the flow, and since when it has been idle.
    static mut IDLE: alloc::vec::Vec<(alloc::string::String, u16, bool, Flow, u64)> = alloc::vec::Vec::new();
    /// Sites close idle connections after a while (often a minute); older ones are not tried.
    const IDLE_KEEP_TICKS: u64 = 30_000;
    const IDLE_MAX: usize = 32;
    fn idle_flows() -> &'static mut alloc::vec::Vec<(alloc::string::String, u16, bool, Flow, u64)> { unsafe { &mut *core::ptr::addr_of_mut!(IDLE) } }
    /// Closes idle connections that are too old to trust.
    fn forget_stale(window: &mut Window) {
        let now = sex_pdx::get_ticks();
        let idle = idle_flows();
        while let Some(i) = idle.iter().position(|e| now.saturating_sub(e.4) >= IDLE_KEEP_TICKS) { let e = idle.remove(i); close(window, e.3); }
    }
    /// Where an exchange is on its way to sending: each `poll` makes one call, so a slow
    /// name, connection or handshake never holds up the others.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Setup { Up, Resolve { asked: bool }, Connect { asked: bool }, Tls { asked: bool }, Send { at: usize }, Done }
    /// The exchange of this program whose question the resolver is answering: it answers
    /// one question per caller at a time.
    static mut RESOLVING: Option<Flow> = None;
    /// Names this program has looked up lately: name, addresses (in the order to try), when.
    static mut NAMES: alloc::vec::Vec<(alloc::string::String, [Option<IpAddr>; 2], u64)> = alloc::vec::Vec::new();
    const NAME_KEEP_TICKS: u64 = 300_000;
    /// How long a connection to one of a name's addresses may take before the next is tried.
    const NEXT_ADDRESS_TICKS: u64 = 3_000;
    fn remembered(host: &str) -> Option<[Option<IpAddr>; 2]> {
        let now = sex_pdx::get_ticks();
        let names = unsafe { &mut *core::ptr::addr_of_mut!(NAMES) };
        names.retain(|(_, _, at)| now.saturating_sub(*at) < NAME_KEEP_TICKS);
        names.iter().find(|(n, _, _)| n == host).map(|(_, a, _)| *a)
    }
    fn remember(host: &str, address: [Option<IpAddr>; 2]) {
        let names = unsafe { &mut *core::ptr::addr_of_mut!(NAMES) };
        names.retain(|(n, _, _)| n != host);
        if names.len() >= 256 { names.remove(0); }
        names.push((alloc::string::String::from(host), address, sex_pdx::get_ticks()));
    }
    fn numeric(host: &[u8]) -> Option<[Option<IpAddr>; 2]> { IpAddr::parse(host).map(|a| [Some(a), None]) }
    impl Exchange {
        /// Takes a flow number for `req` (with `cookie`, the jar's value for its address)
        /// and returns at once: `poll` resolves, connects, encrypts, sends and then reads,
        /// one call per step. Bodies larger than `max` after decoding fail.
        pub fn start(window: &mut Window, req: &sex_fetch::Request, cookie: Option<&str>, max: usize) -> Result<Exchange, Error> {
            let url = sex_fetch::Url::parse(req.url).map_err(|_| Error::Address)?;
            let bytes = sex_fetch::encode(req, &url, cookie).map_err(|_| Error::Address)?;
            // Room for the head, the framing and the body; decoding checks `max` again.
            let cap = max.saturating_add(sex_fetch::HEAD_MAX).saturating_add(max / 8 + 4096);
            Self::raw(window, &url.host, url.port, url.secure, bytes, req.method.eq_ignore_ascii_case("HEAD"), cap, max)
        }
        /// Kept for callers written before setup became stepwise; the same as `start`.
        pub fn start_patiently(window: &mut Window, req: &sex_fetch::Request, cookie: Option<&str>, max: usize) -> Result<Exchange, Error> { Self::start(window, req, cookie, max) }
        fn raw(window: &mut Window, host: &str, port: u16, tls: bool, out: alloc::vec::Vec<u8>, get_head: bool, cap: usize, max: usize) -> Result<Exchange, Error> {
            if host.is_empty() || host.len() > 253 { return Err(Error::Address); }
            forget_stale(window);
            let idle = idle_flows();
            // The newest idle connection to the same place goes first.
            let kept = idle.iter().rposition(|e| e.0 == host && e.1 == port && e.2 == tls).map(|i| idle.remove(i).3);
            let flow = match kept { Some(f) => f, None => take_flow()? };
            let now = sex_pdx::get_ticks();
            Ok(Exchange { flow, get_head, raw: alloc::vec::Vec::new(), cap, max,
                head_end: None, framing: None, chunks: sex_fetch::ChunkScan::new(), began: now, start: now, idle: 0, calls: 0, empty: 0, peer_done: false, whole: false,
                setup: if kept.is_some() { Setup::Send { at: 0 } } else { Setup::Up }, host: alloc::string::String::from(host), port, tls, out, addresses: [None; 2], trying: 0, connect_at: 0,
                reused: kept.is_some(), body_end: None })
        }
        /// A kept connection that failed before any reply byte: the site had closed it.
        /// Start over on a new one (as browsers do), once.
        fn fresh(&mut self, window: &mut Window) -> Result<(), Error> {
            sex_pdx::serial_record!("[net.kept.retry] flow={} sent={}", self.flow.0, matches!(self.setup, Setup::Done) as u8);
            close(window, self.flow);
            self.flow = take_flow()?;
            self.reused = false;
            self.setup = Setup::Up;
            self.peer_done = false;
            self.began = sex_pdx::get_ticks();
            Ok(())
        }
        /// One step of setting up: a single call to the network service.
        fn set_up(&mut self, window: &mut Window) -> Result<(), Error> {
            let flow = self.flow;
            let mut moved = true;
            match self.setup {
                Setup::Up => {
                    let r = call(window, w::UP, 0)?; ok(r)?;
                    if r.0 == w::OK {
                        let known = numeric(self.host.as_bytes()).or_else(|| remembered(&self.host));
                        self.setup = match known { Some(a) => { self.addresses = a; self.trying = 0; Setup::Connect { asked: false } } None => Setup::Resolve { asked: false } };
                    } else { moved = false; }
                }
                Setup::Resolve { asked: false } => {
                    let held = unsafe { *core::ptr::addr_of!(RESOLVING) };
                    if held.is_some_and(|f| f != flow) { moved = false; }
                    else {
                        region()?[..self.host.len()].copy_from_slice(self.host.as_bytes());
                        let r = call(window, w::RESOLVE_ALL, self.host.len() as u64)?;
                        if r.0 == w::BUSY { moved = false; } else { ok(r)?; unsafe { RESOLVING = Some(flow); } self.setup = Setup::Resolve { asked: true }; }
                    }
                }
                Setup::Resolve { asked: true } => {
                    let r = call(window, w::RESOLVE_ALL, 0)?;
                    if r.0 == w::WAIT { moved = false; }
                    else {
                        unsafe { RESOLVING = None; }
                        let n = ok(r)? as usize;
                        let region = region()?;
                        self.addresses = [None; 2];
                        for (i, slot) in self.addresses.iter_mut().enumerate().take(n) { *slot = ip_of(&region[i * w::ADDRESS_BYTES..]); }
                        if self.addresses[0].is_none() { return Err(Error::NoName); }
                        remember(&self.host, self.addresses);
                        self.trying = 0;
                        self.setup = Setup::Connect { asked: false };
                    }
                }
                Setup::Connect { asked: false } => {
                    let address = self.addresses[self.trying].ok_or(Error::NoName)?;
                    put_ip(region()?, &address);
                    let r = call_on(window, flow, w::CONNECT_TO, self.port as u64)?;
                    if r.0 == w::BUSY { moved = false; } else { ok(r)?; self.setup = Setup::Connect { asked: true }; self.connect_at = sex_pdx::get_ticks(); }
                }
                Setup::Connect { asked: true } => {
                    let state = ok(call_on(window, flow, w::STATE, 0)?)?;
                    // A name's next address is tried when the first fails or is slow (the
                    // Happy Eyeballs order: IPv6, then IPv4).
                    let next = self.addresses.get(self.trying + 1).copied().flatten().is_some();
                    let slow = sex_pdx::get_ticks().saturating_sub(self.connect_at) > NEXT_ADDRESS_TICKS;
                    match state {
                        w::STATE_OPEN => self.setup = if self.tls { Setup::Tls { asked: false } } else { Setup::Send { at: 0 } },
                        w::STATE_OPENING if !(next && slow) => moved = false,
                        _ if next => { self.trying += 1; self.setup = Setup::Connect { asked: false }; }
                        _ => return Err(Error::Connect),
                    }
                }
                Setup::Tls { asked: false } => {
                    region()?[..self.host.len()].copy_from_slice(self.host.as_bytes());
                    ok(call_on(window, flow, w::TLS_START, self.host.len() as u64)?)?;
                    self.setup = Setup::Tls { asked: true };
                }
                Setup::Tls { asked: true } => match ok(call_on(window, flow, w::TLS_STAGE, 0)?)? {
                    w::TLS_READY => self.setup = Setup::Send { at: 0 },
                    w::TLS_HANDSHAKE => moved = false,
                    w::TLS_FAILED_CERTIFICATE => return Err(Error::Certificate),
                    _ => return Err(Error::Secure),
                },
                Setup::Send { at } => {
                    if at < self.out.len() {
                        let region = region()?;
                        let n = (self.out.len() - at).min(region.len());
                        region[..n].copy_from_slice(&self.out[at..at + n]);
                        let took = ok(call_on(window, flow, w::SEND, n as u64)?)? as usize;
                        if took == 0 { moved = false; }
                        self.setup = Setup::Send { at: at + took };
                    } else {
                        if self.tls { ok(call_on(window, flow, w::FINISH_REQUEST, 0)?)?; }
                        // A kept connection may turn out closed: keep the request to send again.
                        if !self.reused { self.out = alloc::vec::Vec::new(); }
                        self.setup = Setup::Done;
                    }
                }
                Setup::Done => {}
            }
            if moved { self.idle = 0; self.began = sex_pdx::get_ticks(); } else { self.idle += 1; }
            Ok(())
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
                Some(sex_fetch::Framing::Chunked) => match self.chunks.advance(body) { Ok(None) => false, Ok(Some(end)) => { self.body_end = Some(end); true } Err(_) => true },
                Some(sex_fetch::Framing::Length(n)) => { let done = body.len() as u64 >= n; if done { self.body_end = Some(n as usize); } done }
                Some(sex_fetch::Framing::Empty) => { self.body_end = Some(0); true }
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
            if self.setup != Setup::Done {
                if let Err(e) = self.set_up(window) {
                    if self.reused { self.fresh(window)?; return Ok(false); }
                    self.cancel(window); return Err(e);
                }
                return Ok(false);
            }
            let room = (self.cap - self.raw.len()).min(32 * 1024).max(1) as u64;
            let k = match call_on(window, self.flow, w::RECEIVE, room).and_then(ok) {
                Ok(k) => k as usize,
                Err(_) if self.reused && self.raw.is_empty() => { self.fresh(window)?; return Ok(false); }
                Err(e) => { self.cancel(window); return Err(e); }
            };
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
            if self.peer_done {
                if self.reused && self.raw.is_empty() { self.fresh(window)?; return Ok(false); }
                self.end(window); return Ok(true);
            }
            let state = match call_on(window, self.flow, w::STATE, 0).and_then(ok) { Ok(s) => s, Err(e) => { self.cancel(window); return Err(e); } };
            self.peer_done = matches!(state, w::STATE_PEER_CLOSED | w::STATE_CLOSED);
            self.idle += 1;
            Ok(false)
        }
        fn end(&mut self, window: &mut Window) {
            self.whole = true;
            // Keep the connection when the reply ended exactly where its framing said and
            // the site keeps it open; otherwise let it go.
            let keep = !self.peer_done && self.head_end.is_some_and(|h| self.body_end.is_some_and(|b| h + b == self.raw.len()))
                && sex_fetch::parse_head(&self.raw).ok().flatten().is_some_and(|(head, _)| head.keeps_connection(self.raw.starts_with(b"HTTP/1.0")));
            if keep {
                let idle = idle_flows();
                if idle.len() >= IDLE_MAX { let e = idle.remove(0); close(window, e.3); }
                idle.push((core::mem::take(&mut self.host), self.port, self.tls, self.flow, sex_pdx::get_ticks()));
            } else {
                close(window, self.flow);
            }
            sex_pdx::serial_record!("[net.receive.stats] bytes={} calls={} empty={} ticks={} flow={} reused={} kept={}", self.raw.len(), self.calls, self.empty, sex_pdx::get_ticks() - self.start, self.flow.0, self.reused as u8, keep as u8);
        }
        /// Gives the exchange up and lets its flow go.
        pub fn cancel(&mut self, window: &mut Window) {
            if unsafe { *core::ptr::addr_of!(RESOLVING) } == Some(self.flow) { unsafe { RESOLVING = None; } }
            if !self.whole { self.whole = true; close(window, self.flow); }
        }
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
        let mut ex = Exchange::start(window, req, cookie, max)?;
        wait(window, &mut ex)?;
        ex.finish()
    }

    /// Fetches `url` into `out` and keeps the headers, so a caller can follow redirects
    /// and read the content type.
    pub fn fetch(window: &mut Window, url: &[u8], out: &mut [u8]) -> Result<Reply, Error> {
        let (rest, tls) = match (url.strip_prefix(b"https://"), url.strip_prefix(b"http://")) { (Some(r), _) => (r, true), (_, Some(r)) => (r, false), _ => return Err(Error::Address) };
        let split = rest.iter().position(|b| *b == b'/').unwrap_or(rest.len());
        let (authority, path) = (&rest[..split], if split == rest.len() { &b"/"[..] } else { &rest[split..] });
        // `[v6]:port` keeps its brackets in the host; the colon to split at follows them.
        let colon = match authority.iter().position(|b| *b == b']') { Some(end) => authority[end..].iter().position(|b| *b == b':').map(|i| end + i), None => authority.iter().position(|b| *b == b':') };
        let (host, port) = match colon {
            Some(i) => (&authority[..i], authority[i + 1..].iter().try_fold(0u32, |v, b| b.is_ascii_digit().then(|| v * 10 + (*b - b'0') as u32)).filter(|p| (1..=65535).contains(p)).ok_or(Error::Address)? as u16),
            None => (authority, if tls { 443 } else { 80 }),
        };
        // HTTP/1.0, so the reply needs no chunk decoding and ends when the peer closes.
        let mut request = [0u8; 900];
        let parts: [&[u8]; 5] = [b"GET ", path, b" HTTP/1.0\r\nHost: ", host, b"\r\nUser-Agent: SexOS\r\nAccept: */*\r\nConnection: close\r\n\r\n"];
        let mut n = 0;
        for p in parts { if n + p.len() > request.len() { return Err(Error::Address); } request[n..n + p.len()].copy_from_slice(p); n += p.len(); }
        let host = core::str::from_utf8(host).map_err(|_| Error::Address)?;
        let mut ex = Exchange::raw(window, host, port, tls, request[..n].to_vec(), false, out.len().max(1), out.len())?;
        wait(window, &mut ex)?;
        let got = ex.raw.len().min(out.len());
        out[..got].copy_from_slice(&ex.raw[..got]);
        let end = find(&out[..got], b"\r\n\r\n").ok_or(Error::Failed)?;
        let line = &out[..find(&out[..end + 2], b"\r\n").unwrap_or(end)];
        let status = line.split(|b| *b == b' ').nth(1).and_then(|c| core::str::from_utf8(c).ok()?.parse::<u32>().ok()).unwrap_or(0);
        Ok(Reply { status, body: end + 4, end: got })
    }
}
