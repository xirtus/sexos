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
pub mod mailbox;

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
const NET: u32 = 13;

/// An answer from Files or the network service: it belongs to whichever task asked.
/// How long the main task sleeps before it looks in the mailbox again; anything that
/// reaches its inbox wakes it at once.
const MAILBOX_LOOK_TICKS: u64 = 4;

fn is_answer(m: &sex_pdx::PdxMessage) -> bool { m.type_id == 1 && (m.caller_pd == FILES || m.caller_pd == NET) }
/// A message from Shell: keys, pointer, resizes and closes for the main task's window.
fn is_shell(m: &sex_pdx::PdxMessage) -> bool { m.caller_pd == SHELL }

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
        sex_pdx::system_stats::set_name(name);
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
        // Another task's answer, taken while this one waited: it looks in the mailbox.
        if is_answer(m) { mailbox::put(*m); return None; }
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
        // A worker waiting for Files may take a key meant for this task and leave it in
        // the mailbox, which wakes no one: so sleep only briefly, then look there again.
        loop {
            if let Some(event) = self.poll() { return event; }
            let _ = sex_pdx::sys_wait_self_ticks(MAILBOX_LOOK_TICKS);
        }
    }
    /// The next key or close if one is already waiting, otherwise `None` at once.
    pub fn poll(&mut self) -> Option<Event> {
        if let Some(slot) = self.held.iter_mut().find(|e| e.is_some()) { return slot.take(); }
        while let Some(m) = mailbox::take(is_shell) {
            if let Some(event) = self.event(&m) { return Some(event); }
        }
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
///
/// The functions that take a `Window` keep the keys and closes that arrive while they
/// wait for it. Those in [`files::direct`] need no window: any task of the program may
/// call them, and what arrives meanwhile waits in the [`mailbox`] for its owner. One
/// request is in flight at a time per program; others wait their turn.
pub mod files {
    use super::{is_answer, is_shell, mailbox, Window, FILES};
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use sex_pdx::fs as w;
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

    pub type Error = u8;
    pub use w::{message, NOT_FOUND};

    fn region() -> Result<&'static mut [u8], Error> {
        let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).map_err(|_| w::UNAVAILABLE)?;
        Ok(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
    }
    /// The region's address, asked once: it is this run's for the run's whole life.
    static REGION_AT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    /// What Files last said about `path`, if nothing has changed since: no call at all.
    fn cached(path: &[u8]) -> Option<direct::Stat> {
        let mut at = REGION_AT.load(Ordering::Relaxed);
        if at == 0 { at = region().ok()?.as_ptr() as u64; REGION_AT.store(at, Ordering::Relaxed); }
        let cache = unsafe { core::slice::from_raw_parts((at as usize + w::CACHE_AT) as *const u8, w::CACHE_BYTES) };
        let (dir, size, modified, id) = w::stat_cache::read(cache, path)?;
        Some(direct::Stat { dir, size, modified, id })
    }
    /// Held while a request owns the shared region, from writing the path to reading the
    /// answer.
    static BUSY: AtomicBool = AtomicBool::new(false);
    struct Turn;
    impl Turn {
        fn take() -> Turn {
            while BUSY.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { sex_pdx::sys_yield(); }
            Turn
        }
    }
    impl Drop for Turn { fn drop(&mut self) { BUSY.store(false, Ordering::Release); } }
    static SEQ: AtomicU32 = AtomicU32::new(0);
    /// Where keys and closes that arrive during a request go.
    type Hold<'a> = Option<&'a mut Window>;

    /// Shell tells Files where a new run lives just as the run starts, so a first request
    /// can beat it there. A refusal is retried briefly; a program never bound stays refused.
    /// `fill` writes the data area before the request goes; `take` reads it after the answer.
    fn exchange<R>(mut hold: Hold, kind: u8, path: &[u8], offset: u64, length: u64,
                   fill: impl Fn(&mut [u8]), take: impl FnOnce(u32, &[u8]) -> R) -> Result<R, Error> {
        if path.len() > w::PATH_BYTES { return Err(w::NAME); }
        let _turn = Turn::take();
        let mut tries = 0;
        loop {
            let region = region()?;
            region[..path.len()].copy_from_slice(path);
            fill(&mut region[w::PATH_BYTES..]);
            match once(&mut hold, kind, path.len(), offset, length) {
                Err(w::DENIED) if tries < 50 => { tries += 1; for _ in 0..20 { sex_pdx::sys_yield(); } }
                Ok(value) => return Ok(take(value, &region[w::PATH_BYTES..])),
                Err(e) => return Err(e),
            }
        }
    }
    fn mine(m: &sex_pdx::PdxMessage, seq: u32) -> bool {
        m.type_id == 1 && m.caller_pd == FILES && w::decode_reply(m.arg0).is_some_and(|r| r.0 == seq)
    }
    /// One request, answered before it returns.
    fn once(hold: &mut Hold, kind: u8, path_len: usize, offset: u64, length: u64) -> Result<u32, Error> {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1) & 0xFF_FFFF;
        let r = w::Request { seq, kind, path_len: path_len as u16, offset, length };
        let a = r.args();
        loop {
            // Waits for the answer in the same kernel entry; Files runs next.
            match sex_pdx::pdx_call_wait(sex_pdx::SLOT_STORAGE, w::OP, a[0], a[1], a[2]) {
                (0, _) => break,
                (sex_pdx::ERR_SERVICE_NOT_READY, _) => sex_pdx::sys_yield(),
                _ => return Err(w::UNAVAILABLE),
            }
        }
        // The main task sleeps until its inbox has something (or briefly, to see the
        // mailbox); a worker cannot sleep that way, so it looks, and lets others run.
        let worker = crate::thread::is_worker();
        let mut idle = 0u32;
        loop {
            let m = match mailbox::take(|m| mine(m, seq)) {
                Some(m) => m,
                None => match sex_pdx::pdx_try_listen_raw(0) {
                    Some(m) => { idle = 0; m }
                    // Another task may move this answer into the mailbox meanwhile,
                    // which wakes no one: the main task sleeps briefly and looks again.
                    None if !worker => { let _ = sex_pdx::sys_wait_self_ticks(super::MAILBOX_LOOK_TICKS); continue; }
                    None => { idle += 1; crate::thread::pause(idle); continue; }
                },
            };
            if mine(&m, seq) {
                let (_, status, value) = w::decode_reply(m.arg0).unwrap_or((0, w::INVALID, 0));
                return if status == w::OK { Ok(value) } else { Err(status) };
            }
            if is_answer(&m) { mailbox::put(m); continue; }
            match hold {
                Some(window) => if let Some(event) = window.event(&m) {
                    if let Some(slot) = window.held.iter_mut().find(|e| e.is_none()) { *slot = Some(event); }
                },
                None => if is_shell(&m) { mailbox::put(m) },
            }
        }
    }
    /// The words typed after the program's name, if it was started with any.
    pub fn args<'a>(window: &mut Window, out: &'a mut [u8; w::ARGS_MAX]) -> &'a [u8] {
        let n = direct::args_into(Some(window), out);
        &out[..n]
    }
    /// Reads up to `out.len()` bytes of the file from its start.
    pub fn read(window: &mut Window, path: &[u8], out: &mut [u8]) -> Result<usize, Error> {
        read_at(window, path, 0, out)
    }
    /// File-payload device I/O caused by this run's READ requests, excluding metadata.
    /// A new run or Files restart resets the counters. This does not bypass any
    /// cache and does not report other clients' I/O or hardware cache misses.
    pub fn read_io_stats(window: &mut Window) -> Result<w::ReadIoStats, Error> {
        exchange(Some(window), w::READ_IO_STATS, b"", 0, 0, |_| {}, |n, data| {
            w::ReadIoStats::decode(data.get(..n as usize).unwrap_or(&[])).ok_or(w::INVALID)
        })?
    }
    /// Reads up to `out.len()` bytes of the file from byte `offset`; fewer at its end.
    pub fn read_at(window: &mut Window, path: &[u8], offset: u64, out: &mut [u8]) -> Result<usize, Error> {
        direct::read_at_held(Some(window), path, offset, out)
    }
    /// Replaces the file with `data`, of any size: it goes over in pieces as big as the
    /// shared region holds. Creates it if needed.
    pub fn write(window: &mut Window, path: &[u8], data: &[u8]) -> Result<(), Error> {
        match exchange(Some(&mut *window), w::STAT, path, 0, 0, |_| {}, |_, _| ()) {
            Ok(_) => { exchange(Some(&mut *window), w::TRUNCATE, path, 0, 0, |_| {}, |_, _| ())?; }
            Err(w::NOT_FOUND) => { exchange(Some(&mut *window), w::CREATE, path, 0, 0, |_| {}, |_, _| ())?; }
            Err(e) => return Err(e),
        }
        let wrote = direct::write_at_held(Some(&mut *window), path, 0, data)?;
        if wrote < data.len() { return Err(w::NO_SPACE); }
        Ok(())
    }
    /// The file's size, or why there is none.
    pub fn size(window: &mut Window, path: &[u8]) -> Result<u64, Error> {
        if let Some(st) = cached(path) { return Ok(st.size); }
        exchange(Some(&mut *window), w::STAT, path, 0, 0, |_| {}, |_, d| direct::Stat::decode(d).size)
    }
    /// One call to Files and its answer, carrying nothing: what a round trip costs.
    pub fn ping(window: &mut Window) -> Result<(), Error> {
        request(window, w::PING, b"", 0, 0).map(|_| ())
    }
    /// Makes everything written so far durable.
    pub fn sync(window: &mut Window) -> Result<(), Error> {
        exchange(Some(&mut *window), w::SYNC, b"", 0, 0, |_| {}, |_, _| ())
    }

    /// The same files with no window: for any task, and for libraries (Rust `std`, the
    /// C library) that have none to hand. Keys and closes that arrive meanwhile wait in
    /// the mailbox, where `Window::next` finds them.
    pub mod direct {
        use super::{exchange, w, Error, Hold};

        /// What Files knows of a file or folder.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct Stat { pub dir: bool, pub size: u64, pub modified: u64, pub id: u64 }
        impl Stat {
            pub(crate) fn decode(d: &[u8]) -> Stat {
                let word = |i: usize| u64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
                Stat { dir: word(0) != 0, size: word(1), modified: word(2), id: word(3) }
            }
        }
        /// One entry of a folder.
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct Entry { pub dir: bool, pub size: u64, pub name: alloc::vec::Vec<u8> }

        pub(super) fn args_into(hold: Hold, out: &mut [u8; w::ARGS_MAX]) -> usize {
            exchange(hold, w::ARGS, b"", 0, 0, |_| {}, |n, d| {
                let n = (n as usize).min(w::ARGS_MAX);
                out[..n].copy_from_slice(&d[..n]);
                n
            }).unwrap_or(0)
        }
        pub(super) fn read_at_held(mut hold: Hold, path: &[u8], offset: u64, out: &mut [u8]) -> Result<usize, Error> {
            let mut done = 0;
            while done < out.len() {
                let want = (out.len() - done).min(w::IO_MAX);
                let piece = &mut out[done..done + want];
                let got = exchange(hold.as_deref_mut(), w::READ, path, offset + done as u64, want as u64, |_| {}, |n, d| {
                    let n = (n as usize).min(want);
                    piece[..n].copy_from_slice(&d[..n]);
                    n
                })?;
                done += got;
                if got < want { break; }
            }
            Ok(done)
        }
        pub(super) fn write_at_held(mut hold: Hold, path: &[u8], offset: u64, data: &[u8]) -> Result<usize, Error> {
            let mut done = 0;
            while done < data.len() {
                let n = (data.len() - done).min(w::IO_MAX);
                let piece = &data[done..done + n];
                let wrote = exchange(hold.as_deref_mut(), w::WRITE, path, offset + done as u64, n as u64,
                    |d| d[..n].copy_from_slice(piece), |n, _| n as usize)?;
                done += wrote;
                if wrote < n { break; }
            }
            Ok(done)
        }
        fn simple(kind: u8, path: &[u8], offset: u64) -> Result<(), Error> {
            exchange(None, kind, path, offset, 0, |_| {}, |_, _| ())
        }

        /// The words typed after the program's name, if it was started with any.
        pub fn args(out: &mut [u8; w::ARGS_MAX]) -> usize { args_into(None, out) }
        /// Reads up to `out.len()` bytes from byte `offset`; fewer at the file's end.
        pub fn read_at(path: &[u8], offset: u64, out: &mut [u8]) -> Result<usize, Error> { read_at_held(None, path, offset, out) }
        /// Writes `data` at byte `offset`, growing the file as needed. Returns how much
        /// was written: less than all only when the disk is full.
        pub fn write_at(path: &[u8], offset: u64, data: &[u8]) -> Result<usize, Error> { write_at_held(None, path, offset, data) }
        pub fn stat(path: &[u8]) -> Result<Stat, Error> {
            if let Some(st) = super::cached(path) { return Ok(st); }
            exchange(None, w::STAT, path, 0, 0, |_| {}, |_, d| Stat::decode(d))
        }
        /// Makes an empty file; `EXISTS` if there is one.
        pub fn create(path: &[u8]) -> Result<(), Error> { simple(w::CREATE, path, 0) }
        /// Cuts or extends the file to `size` bytes.
        pub fn truncate(path: &[u8], size: u64) -> Result<(), Error> { simple(w::TRUNCATE, path, size) }
        pub fn mkdir(path: &[u8]) -> Result<(), Error> { simple(w::MKDIR, path, 0) }
        /// Removes a file or an empty folder.
        pub fn remove(path: &[u8]) -> Result<(), Error> { simple(w::REMOVE, path, 0) }
        pub fn rename(from: &[u8], to: &[u8]) -> Result<(), Error> {
            let mut both = alloc::vec::Vec::with_capacity(from.len() + 1 + to.len());
            both.extend_from_slice(from);
            both.push(0);
            both.extend_from_slice(to);
            simple(w::RENAME, &both, 0)
        }
        /// Makes everything written so far durable.
        pub fn sync() -> Result<(), Error> { simple(w::SYNC, b"", 0) }
        /// Everything in the folder `path` (`b""` for the program's own folder).
        pub fn list(path: &[u8]) -> Result<alloc::vec::Vec<Entry>, Error> {
            let mut all = alloc::vec::Vec::new();
            let mut cursor = 0u64;
            loop {
                let next = exchange(None, w::LIST, path, cursor, 0, |_| {}, |used, d| {
                    let used = (used as usize).min(d.len());
                    let mut at = w::LIST_HEAD;
                    while at + w::ENTRY_HEAD <= used {
                        let len = d[at + 1] as usize;
                        if at + w::ENTRY_HEAD + len > used { break; }
                        let size = u64::from_le_bytes(d[at + 2..at + 10].try_into().unwrap_or([0; 8]));
                        all.push(Entry { dir: d[at] == 1, size, name: d[at + w::ENTRY_HEAD..at + w::ENTRY_HEAD + len].to_vec() });
                        at += w::ENTRY_HEAD + len;
                    }
                    u64::from_le_bytes(d[..8].try_into().unwrap_or([0; 8]))
                })?;
                if next == 0 { return Ok(all); }
                cursor = next;
            }
        }
    }
    /// One request with no data either way, for the window's task, keeping keys and
    /// closes that arrive meanwhile. Requests with data use [`exchange`]'s `fill` and
    /// `take`, so another task's request cannot replace the data in between.
    fn request(window: &mut Window, kind: u8, path: &[u8], offset: u64, length: u64) -> Result<u32, Error> {
        exchange(Some(window), kind, path, offset, length, |_| {}, |value, _| value)
    }
    fn word(d: &[u8], i: usize) -> u64 { u64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8])) }
    /// What a path is.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Stat { pub dir: bool, pub size: u64, pub mtime: u64 }
    pub fn stat(window: &mut Window, path: &[u8]) -> Result<Stat, Error> {
        if let Some(st) = cached(path) { return Ok(Stat { dir: st.dir, size: st.size, mtime: st.modified }); }
        exchange(Some(window), w::STAT, path, 0, 0, |_| {}, |_, d| Stat { dir: word(d, 0) != 0, size: word(d, 1), mtime: word(d, 2) })
    }
    /// One entry of a folder.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Entry { pub name: alloc::vec::Vec<u8>, pub dir: bool, pub size: u64 }
    /// Everything in a folder, in the order the disk keeps it.
    pub fn list(window: &mut Window, path: &[u8]) -> Result<alloc::vec::Vec<Entry>, Error> {
        let mut out = alloc::vec::Vec::new();
        let mut cursor = 0u64;
        loop {
            let next = exchange(Some(&mut *window), w::LIST, path, cursor, 0, |_| {}, |used, r| {
                let used = (used as usize).min(r.len());
                let mut at = w::LIST_HEAD;
                while at + w::ENTRY_HEAD <= used {
                    let len = r[at + 1] as usize;
                    if at + w::ENTRY_HEAD + len > used { break; }
                    let size = u64::from_le_bytes(r[at + 2..at + 10].try_into().unwrap_or([0; 8]));
                    out.push(Entry { name: r[at + w::ENTRY_HEAD..at + w::ENTRY_HEAD + len].to_vec(), dir: r[at] == 1, size });
                    at += w::ENTRY_HEAD + len;
                }
                word(r, 0)
            })?;
            if next == 0 { return Ok(out); }
            cursor = next;
        }
    }
    pub fn mkdir(window: &mut Window, path: &[u8]) -> Result<(), Error> { request(window, w::MKDIR, path, 0, 0).map(|_| ()) }
    /// Writes out and forgets the volume `/media/NAME`; for a USB stick every volume on it
    /// (Eject). Grants into it end with it.
    pub fn eject(window: &mut Window, media_path: &[u8]) -> Result<(), Error> { request(window, w::UNMOUNT, media_path, 0, 0).map(|_| ()) }
    pub fn create(window: &mut Window, path: &[u8]) -> Result<(), Error> { request(window, w::CREATE, path, 0, 0).map(|_| ()) }
    /// A file, or an empty folder.
    pub fn remove(window: &mut Window, path: &[u8]) -> Result<(), Error> { request(window, w::REMOVE, path, 0, 0).map(|_| ()) }
    pub fn truncate(window: &mut Window, path: &[u8], len: u64) -> Result<(), Error> { request(window, w::TRUNCATE, path, len, 0).map(|_| ()) }
    /// Moves or renames. Between two disks Files answers `OTHER_DISK`: copy, then remove.
    pub fn rename(window: &mut Window, from: &[u8], to: &[u8]) -> Result<(), Error> {
        if from.len() + 1 + to.len() > w::PATH_BYTES { return Err(w::NAME); }
        let mut both = [0u8; w::PATH_BYTES];
        both[..from.len()].copy_from_slice(from);
        both[from.len() + 1..from.len() + 1 + to.len()].copy_from_slice(to);
        request(window, w::RENAME, &both[..from.len() + 1 + to.len()], 0, 0).map(|_| ())
    }
    /// Writes `data` at byte `offset`, growing the file if it must. Returns bytes written.
    pub fn write_at(window: &mut Window, path: &[u8], offset: u64, data: &[u8]) -> Result<usize, Error> {
        let mut done = 0;
        while done < data.len() {
            let n = (data.len() - done).min(w::IO_MAX);
            let wrote = direct::write_at_held(Some(&mut *window), path, offset + done as u64, &data[done..done + n])?;
            done += wrote;
            if wrote < n { return Err(w::NO_SPACE); }
        }
        Ok(done)
    }
    /// (total, free) bytes of the disk holding `path`.
    pub fn space(window: &mut Window, path: &[u8]) -> Result<(u64, u64), Error> {
        exchange(Some(window), w::STATFS, path, 0, 0, |_| {}, |_, d| (word(d, 0) * 4096, word(d, 1) * 4096))
    }

    /// What the picker asks the person for.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Pick { Open, Save, Folder }
    /// Asks the person to choose a file (or a folder, or where to save) in the Files
    /// app, and waits. The answer is a path this run may now use, `@/...`, kept in `out`.
    /// Closing the window meanwhile cancels. Errors: DENIED when the person cancelled.
    pub fn pick<'a>(window: &mut Window, mode: Pick, suggestion: &[u8], out: &'a mut [u8; w::PATH_BYTES]) -> Result<&'a [u8], Error> {
        let n = suggestion.len().min(w::PICK_HINT_MAX);
        let code = match mode { Pick::Open => w::PICK_OPEN, Pick::Save => w::PICK_SAVE, Pick::Folder => w::PICK_FOLDER };
        let ticket = exchange(Some(&mut *window), w::PICK, b"", 0, n as u64, |r| {
            r[0] = code;
            r[1..1 + n].copy_from_slice(&suggestion[..n]);
        }, |ticket, _| ticket)?;
        loop {
            let picked = exchange(Some(&mut *window), w::PICKED, b"", ticket as u64, 0, |_| {}, |len, d| {
                let len = (len as usize).min(w::PATH_BYTES).min(d.len());
                out[..len].copy_from_slice(&d[..len]);
                len
            });
            match picked {
                Ok(len) => return Ok(&out[..len]),
                Err(w::PENDING) => {}
                Err(e) => return Err(e),
            }
            // Wait a little. Keys wait for later; a close ends the wait.
            let end = sex_pdx::get_ticks().saturating_add(100);
            while sex_pdx::get_ticks() < end {
                while let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
                    if let Some(event) = window.event(&m) {
                        let close = matches!(event, super::Event::Close);
                        if let Some(slot) = window.held.iter_mut().find(|e| e.is_none()) { *slot = Some(event); }
                        if close { return Err(w::DENIED); }
                    }
                }
                let _ = sex_pdx::sys_wait_self_ticks(10);
            }
        }
    }
    /// One waiting question, for the program that shows the picker.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Question { pub row: u8, pub mode: Pick, pub ticket: u32, pub program: alloc::vec::Vec<u8>, pub suggestion: alloc::vec::Vec<u8> }
    /// Questions waiting for the person. Only a run given the whole filesystem may ask.
    pub fn questions(window: &mut Window) -> Result<alloc::vec::Vec<Question>, Error> {
        exchange(Some(window), w::PICKS, b"", 0, 0, |_| {}, |used, r| {
            let used = (used as usize).min(r.len());
            let mut out = alloc::vec::Vec::new();
            let mut at = 0;
            while at + 7 <= used {
                let row = r[at];
                let mode = match r[at + 1] { w::PICK_SAVE => Pick::Save, w::PICK_FOLDER => Pick::Folder, _ => Pick::Open };
                let ticket = u32::from_le_bytes(r[at + 2..at + 6].try_into().unwrap_or([0; 4]));
                let name_len = r[at + 6] as usize;
                let program = r[at + 7..at + 7 + name_len].to_vec();
                at += 7 + name_len;
                let hint_len = r[at] as usize;
                let suggestion = r[at + 1..at + 1 + hint_len].to_vec();
                at += 1 + hint_len;
                out.push(Question { row, mode, ticket, program, suggestion });
            }
            out
        })
    }
    /// Answers a question with `path` (`@/...`), or cancels it with `None`.
    pub fn answer(window: &mut Window, q: &Question, path: Option<&[u8]>) -> Result<(), Error> {
        request(window, w::ANSWER, path.unwrap_or(b""), q.row as u64, q.ticket as u64).map(|_| ())
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
/// The ASCII character a key types, if any. Programs that take any text use [`text`].
pub fn typed(key: Key) -> Option<u8> {
    key.char().filter(char::is_ascii).map(|c| c as u8)
}
/// The character a key typed under the person's keyboard layout, if any: `ü` on a
/// German keyboard, `é` after a dead `´`, `€` from Compose `=` `e`.
pub fn text(key: Key) -> Option<char> { key.char() }

/// The network, for a run the person started with `run +net`. Otherwise every call is
/// refused. HTTP and HTTPS through the network service's checked TLS, and TCP and UDP
/// sockets ([`net::TcpStream`], [`net::TcpListener`], [`net::UdpSocket`]), over IPv4 or IPv6.
pub mod net {
    use super::Window;
    use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
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
    static SEQ: AtomicU32 = AtomicU32::new(0);
    /// This program's flow numbers in use, one bit each.
    static FLOWS_HELD: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
    /// The task holding the shared region, by its thread pointer (`usize::MAX`: none),
    /// from filling it until the answer has been read back. The holder may take it
    /// again (a call inside a call), so a function holds it across its whole use.
    static OWNER: AtomicUsize = AtomicUsize::new(usize::MAX);
    static DEPTH: AtomicU32 = AtomicU32::new(0);
    struct Turn;
    impl Turn {
        fn take() -> Turn {
            let me = crate::tls::pointer();
            if OWNER.load(Ordering::Acquire) == me { DEPTH.fetch_add(1, Ordering::Relaxed); return Turn; }
            while OWNER.compare_exchange_weak(usize::MAX, me, Ordering::Acquire, Ordering::Relaxed).is_err() { sex_pdx::sys_yield(); }
            DEPTH.store(1, Ordering::Relaxed);
            Turn
        }
    }
    impl Drop for Turn {
        fn drop(&mut self) { if DEPTH.fetch_sub(1, Ordering::Relaxed) == 1 { OWNER.store(usize::MAX, Ordering::Release); } }
    }
    /// One open flow of this program, by its own number for it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Flow(u8);
    fn take_flow() -> Result<Flow, Error> {
        for n in 0..w::CLIENT_FLOWS {
            let bit = 1u64 << (n % 64);
            if FLOWS_HELD[n as usize / 64].fetch_or(bit, Ordering::AcqRel) & bit == 0 { return Ok(Flow(n)); }
        }
        Err(Error::Busy)
    }
    fn close(window: &mut Window, flow: Flow) {
        let _ = call_on(window, flow, w::CLOSE, 0);
        release(flow);
    }
    fn call(window: &mut Window, kind: u8, value: u64) -> Result<(u8, u32), Error> { call_on(window, Flow(0), kind, value) }
    fn call_on(window: &mut Window, flow: Flow, kind: u8, value: u64) -> Result<(u8, u32), Error> {
        let _turn = Turn::take();
        call_held(Some(window), flow, kind, value)
    }
    fn mine(m: &sex_pdx::PdxMessage, seq: u32) -> bool {
        m.type_id == 1 && m.caller_pd == NET && w::decode_reply(m.arg0).is_some_and(|r| r.0 == seq)
    }
    /// One request and its answer, under the turn. What else arrives meanwhile goes to
    /// the window, or to the mailbox for the task it belongs to.
    fn call_held(mut hold: Option<&mut Window>, flow: Flow, kind: u8, value: u64) -> Result<(u8, u32), Error> {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1) & 0x7F_FFFF;
        loop {
            match sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, w::OP, w::request_on(seq, kind, flow.0), value, 0) {
                Ok(_) => break,
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => sex_pdx::sys_yield(),
                Err(_) => return Err(Error::NotAllowed),
            }
        }
        let began = sex_pdx::get_ticks();
        let worker = crate::thread::is_worker();
        let mut idle = 0u32;
        loop {
            if sex_pdx::get_ticks().saturating_sub(began) > 30_000 { return Err(Error::TooSlow); }
            let m = match super::mailbox::take(|m| mine(m, seq)) {
                Some(m) => m,
                None => match sex_pdx::pdx_try_listen_raw(0) {
                    Some(m) => { idle = 0; m }
                    None if worker => { idle += 1; crate::thread::pause(idle); continue; }
                    None => { sex_pdx::sys_yield(); continue; }
                },
            };
            if mine(&m, seq) {
                let (_, status, value) = w::decode_reply(m.arg0).unwrap_or((0, w::INVALID, 0));
                return Ok((status, value));
            }
            match hold.as_deref_mut() {
                Some(window) => if let Some(event) = window.event(&m) {
                    if let Some(slot) = window.held.iter_mut().find(|e| e.is_none()) { *slot = Some(event); }
                },
                // Another task's answer, a key or a close: its owner looks in the mailbox.
                // A late answer to this program's own earlier question is dropped.
                None => if (super::is_answer(&m) && m.caller_pd != NET) || super::is_shell(&m) { super::mailbox::put(m) },
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
    fn release(flow: Flow) { FLOWS_HELD[flow.0 as usize / 64].fetch_and(!(1u64 << (flow.0 % 64)), Ordering::AcqRel); }
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
        let _turn = Turn::take();
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
                let _turn = Turn::take();
                region()?[..host.len()].copy_from_slice(host);
                let r = call(window, w::RESOLVE_ALL, host.len() as u64)?;
                if r.0 != w::BUSY { ok(r)?; break; }
            }
            pause();
        }
        unsafe { RESOLVING = Some(RESOLVE_HOLDER); }
        let answer = loop {
            let turn = Turn::take();
            let r = call(window, w::RESOLVE_ALL, 0);
            match r {
                Ok((w::WAIT, _)) if sex_pdx::get_ticks().saturating_sub(began) <= 30_000 => { drop(turn); pause() }
                Ok((w::WAIT, _)) => break Err(Error::TooSlow),
                // The addresses are read before the turn goes.
                Ok(r) => break ok(r).and_then(|n| {
                    let region = region()?;
                    let mut found = [None; 2];
                    for (i, slot) in found.iter_mut().enumerate().take(n as usize) { *slot = ip_of(&region[i * w::ADDRESS_BYTES..]); }
                    Ok(found)
                }),
                Err(e) => break Err(e),
            }
        };
        unsafe { RESOLVING = None; }
        let found = answer?;
        remember(name, found);
        Ok(found.into_iter().flatten().collect())
    }

    /// The same sockets with no window, for any task of the program and for libraries
    /// (Rust `std`, the C library) that have none: each call returns at once, and the
    /// caller decides how to wait. Keys and closes that arrive meanwhile wait in the
    /// mailbox for `Window::next`. Flows are this program's, as with the window calls.
    pub mod direct {
        use super::{call_held, endpoint, ip_of, ok, ok_status, put_ip, region, release, take_flow, w, Error, Flow, IpAddr, Turn};
        use core::sync::atomic::{AtomicBool, Ordering};

        /// One request: `fill` writes the shared region before it goes, `take` reads
        /// the answer from it, all under the turn.
        fn exchange<R>(flow: Flow, kind: u8, value: u64, fill: impl FnOnce(&mut [u8]), take: impl FnOnce((u8, u32), &[u8]) -> R) -> Result<R, Error> {
            let _turn = Turn::take();
            let region = region()?;
            fill(region);
            let r = call_held(None, flow, kind, value)?;
            Ok(take(r, region))
        }
        fn simple(flow: Flow, kind: u8, value: u64) -> Result<(u8, u32), Error> { exchange(flow, kind, value, |_| {}, |r, _| r) }

        /// True once the network is up (a DHCP lease or a global IPv6 address).
        pub fn up() -> Result<bool, Error> { Ok(simple(Flow(0), w::UP, 0).and_then(ok_status)? == w::OK) }

        static RESOLVING: AtomicBool = AtomicBool::new(false);
        /// Asks for the addresses of `host`: `None` while the answer is on its way (ask
        /// again), then the addresses, the one to try first first. One question at a
        /// time per program: a second caller sees `Busy` until the first has its answer.
        pub fn resolve_step(host: &[u8], asked: &mut bool) -> Result<Option<alloc::vec::Vec<IpAddr>>, Error> {
            if let Some(a) = IpAddr::parse(host) { return Ok(Some(alloc::vec![a])); }
            if host.is_empty() || host.len() > 253 || core::str::from_utf8(host).is_err() { return Err(Error::Address); }
            if !*asked {
                if RESOLVING.swap(true, Ordering::AcqRel) { return Err(Error::Busy); }
                let r = exchange(Flow(0), w::RESOLVE_ALL, host.len() as u64, |d| d[..host.len()].copy_from_slice(host), |r, _| r);
                match r {
                    Ok((w::BUSY, _)) => { RESOLVING.store(false, Ordering::Release); return Err(Error::Busy); }
                    Ok(r) => if let Err(e) = ok(r) { RESOLVING.store(false, Ordering::Release); return Err(e); },
                    Err(e) => { RESOLVING.store(false, Ordering::Release); return Err(e); }
                }
                *asked = true;
                return Ok(None);
            }
            let r = exchange(Flow(0), w::RESOLVE_ALL, 0, |_| {}, |r, d| {
                let n = (r.1 as usize).min(2);
                (r, (0..n).filter_map(|i| ip_of(&d[i * w::ADDRESS_BYTES..])).collect::<alloc::vec::Vec<_>>())
            });
            match r {
                Ok(((w::WAIT, _), _)) => Ok(None),
                Ok((r, found)) => {
                    RESOLVING.store(false, Ordering::Release);
                    ok(r)?;
                    if found.is_empty() { Err(Error::NoName) } else { Ok(Some(found)) }
                }
                Err(e) => { RESOLVING.store(false, Ordering::Release); Err(e) }
            }
        }

        /// Gives up a question `resolve_step` asked, so the next caller may ask.
        pub fn resolve_cancel(asked: &mut bool) {
            if core::mem::take(asked) { RESOLVING.store(false, Ordering::Release); }
        }

        /// A TCP connection, or a listener, or a UDP socket, by its flow.
        #[derive(Debug)]
        pub struct Socket { flow: Flow }
        /// Where a TCP connection is.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum State { Opening, Open, PeerClosed, Closed }
        impl Socket {
            /// Starts connecting; `state` says when it is open.
            pub fn connect(address: &IpAddr, port: u16) -> Result<Socket, Error> {
                let flow = take_flow()?;
                let address = *address;
                match exchange(flow, w::CONNECT_TO, port as u64, |d| put_ip(d, &address), |r, _| r).and_then(ok) {
                    Ok(_) => Ok(Socket { flow }),
                    Err(e) => { release(flow); Err(e) }
                }
            }
            pub fn listen(port: u16) -> Result<Socket, Error> {
                let flow = take_flow()?;
                match simple(flow, w::LISTEN, port as u64).and_then(ok) {
                    Ok(_) => Ok(Socket { flow }),
                    Err(e) => { release(flow); Err(e) }
                }
            }
            /// A UDP socket on `port` (0: one the service picks), and the port it got.
            pub fn udp(port: u16) -> Result<(Socket, u16), Error> {
                let flow = take_flow()?;
                match simple(flow, w::UDP_BIND, port as u64).and_then(ok) {
                    Ok(port) => Ok((Socket { flow }, port as u16)),
                    Err(e) => { release(flow); Err(e) }
                }
            }
            pub fn state(&self) -> Result<State, Error> {
                Ok(match ok(simple(self.flow, w::STATE, 0)?)? {
                    w::STATE_OPENING => State::Opening,
                    w::STATE_PEER_CLOSED => State::PeerClosed,
                    w::STATE_CLOSED => State::Closed,
                    _ => State::Open,
                })
            }
            /// A connection a peer made, and from where, if one is waiting.
            pub fn accept(&self) -> Result<Option<(Socket, IpAddr, u16)>, Error> {
                let new = take_flow()?;
                let r = exchange(self.flow, w::ACCEPT, new.0 as u64, |_| {}, |r, d| (r, endpoint(d)));
                match r {
                    Ok((r, peer)) => match ok_status(r) {
                        Ok(w::OK) => { let (ip, port) = peer.unwrap_or((IpAddr::V4([0; 4]), 0)); Ok(Some((Socket { flow: new }, ip, port))) }
                        Ok(_) => { release(new); Ok(None) }
                        Err(e) => { release(new); Err(e) }
                    },
                    Err(e) => { release(new); Err(e) }
                }
            }
            /// Sends what fits now: 0 when the send queue is full.
            pub fn send(&self, bytes: &[u8]) -> Result<usize, Error> {
                let n = bytes.len().min(w::REGION_BYTES);
                Ok(ok(exchange(self.flow, w::SEND, n as u64, |d| d[..n].copy_from_slice(&bytes[..n]), |r, _| r)?)? as usize)
            }
            /// Reads what has arrived: 0 when nothing has yet (see `state`).
            pub fn receive(&self, out: &mut [u8]) -> Result<usize, Error> {
                let room = out.len().min(32 * 1024).max(1);
                exchange(self.flow, w::RECEIVE, room as u64, |_| {}, |r, d| ok(r).map(|k| {
                    let k = (k as usize).min(out.len());
                    out[..k].copy_from_slice(&d[..k]);
                    k
                }))?
            }
            /// Sends this side's FIN after what is queued.
            pub fn shutdown(&self) -> Result<(), Error> { ok(simple(self.flow, w::SHUTDOWN, 0)?).map(|_| ()) }
            /// Queues one datagram: false when there is no room yet.
            pub fn send_to(&self, bytes: &[u8], to: &IpAddr, port: u16) -> Result<bool, Error> {
                if bytes.len() > w::REGION_BYTES - w::ENDPOINT_BYTES { return Err(Error::TooLarge); }
                let to = *to;
                let r = exchange(self.flow, w::UDP_SEND, bytes.len() as u64, |d| {
                    put_ip(d, &to);
                    d[17..19].copy_from_slice(&port.to_be_bytes());
                    d[w::ENDPOINT_BYTES..w::ENDPOINT_BYTES + bytes.len()].copy_from_slice(bytes);
                }, |r, _| r)?;
                if r.0 == w::INVALID { return Err(Error::TooLarge); }
                Ok(ok_status(r)? == w::OK)
            }
            /// One datagram that came, if any: its length (cut at `out.len()`), sender, port.
            pub fn receive_from(&self, out: &mut [u8]) -> Result<Option<(usize, IpAddr, u16)>, Error> {
                let len = out.len();
                exchange(self.flow, w::UDP_RECEIVE, len as u64, |_| {}, |r, d| {
                    if ok_status(r)? != w::OK { return Ok(None); }
                    let n = (r.1 as usize).min(len);
                    out[..n].copy_from_slice(&d[w::ENDPOINT_BYTES..w::ENDPOINT_BYTES + n]);
                    let (from, port) = endpoint(d).ok_or(Error::Failed)?;
                    Ok(Some((n, from, port)))
                })?
            }
        }
        impl Drop for Socket {
            fn drop(&mut self) { let _ = simple(self.flow, w::CLOSE, 0); release(self.flow); }
        }
    }

    /// A TCP connection of this program, over IPv4 or IPv6. Calls that wait have a
    /// non-waiting form, so a program can keep several going and draw between them.
    pub struct TcpStream { flow: Flow, peer: (IpAddr, u16) }
    impl TcpStream {
        /// Starts connecting to `address` and returns at once; `poll_open` says when it is.
        pub fn begin(window: &mut Window, address: &IpAddr, port: u16) -> Result<TcpStream, Error> {
            let flow = take_flow()?;
            let _turn = Turn::take();
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
            let _turn = Turn::take();
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
            let _turn = Turn::take();
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
            let _turn = Turn::take();
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
            let _turn = Turn::take();
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
            let _turn = Turn::take();
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
        /// Body bytes `stream` has handed out (and taken from `raw`).
        drained: usize,
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
                reused: kept.is_some(), body_end: None, drained: 0 })
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
            // One call per step; its region use stays inside the turn.
            let _turn = Turn::take();
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
                Some(sex_fetch::Framing::Length(n)) => { let done = (body.len() + self.drained) as u64 >= n; if done { self.body_end = Some(n as usize); } done }
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
            let room = self.cap.saturating_sub(self.raw.len() + self.drained).min(32 * 1024).max(1) as u64;
            let turn = Turn::take();
            let k = match call_on(window, self.flow, w::RECEIVE, room).and_then(ok) {
                Ok(k) => k as usize,
                Err(_) if self.reused && self.raw.is_empty() => { self.fresh(window)?; return Ok(false); }
                Err(e) => { self.cancel(window); return Err(e); }
            };
            self.calls += 1;
            if k > 0 {
                let take = k.min(self.cap.saturating_sub(self.raw.len() + self.drained));
                let region = region().map_err(|e| { self.cancel(window); e })?;
                self.raw.extend_from_slice(&region[..take]);
                drop(turn);
                self.began = sex_pdx::get_ticks();
                self.idle = 0;
                if self.raw.len() + self.drained >= self.cap || self.enough() { self.end(window); }
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
            let keep = !self.peer_done && self.head_end.is_some_and(|h| self.body_end.is_some_and(|b| h + b == self.raw.len() + self.drained))
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
        /// The reply's head, once it has arrived.
        pub fn head(&self) -> Option<sex_fetch::Head> {
            self.head_end?;
            sex_fetch::parse_head(&self.raw).ok().flatten().map(|(h, _)| h)
        }
        /// For a reply whose body can be used as it arrives (a length or an until-close
        /// body, not compressed, not chunked): moves the body bytes that came since the
        /// last call to the end of `into` and gives the body's length, if the head said
        /// it. `None` while the head is still coming or when the body must be read whole
        /// (then `finish` as usual). Once bytes are taken this way, `finish` has no body.
        pub fn stream(&mut self, into: &mut alloc::vec::Vec<u8>) -> Option<Option<u64>> {
            let at = self.head_end?;
            let total = match self.framing? { sex_fetch::Framing::Length(n) => Some(n), sex_fetch::Framing::UntilClose => None, _ => return None };
            let head = sex_fetch::parse_head(&self.raw).ok().flatten()?.0;
            if head.get("content-encoding").is_some_and(|e| !e.trim().eq_ignore_ascii_case("identity")) { return None; }
            // A length body ends there: bytes past it belong to no one.
            let end = total.map_or(self.raw.len(), |n| self.raw.len().min(at + (n as usize).saturating_sub(self.drained)));
            if end > at {
                if into.try_reserve(end - at).is_err() { return None; }
                into.extend_from_slice(&self.raw[at..end]);
                self.drained += end - at;
                self.raw.drain(at..end);
            }
            Some(total)
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
    /// A connection that stays open both ways (a WebSocket after its handshake): the
    /// same setup as an [`Exchange`] (name, Happy Eyeballs, TLS), then bytes in and out
    /// for as long as both ends like. Every call is one step and returns at once.
    pub struct Duplex { ex: Exchange, sent_first: bool }
    impl Duplex {
        /// Starts connecting to `host`:`port` (TLS when `tls`), sending `first` once up.
        pub fn start(window: &mut Window, host: &str, port: u16, tls: bool, first: alloc::vec::Vec<u8>) -> Result<Duplex, Error> {
            if host.is_empty() || host.len() > 253 { return Err(Error::Address); }
            forget_stale(window);
            let flow = take_flow()?;
            let now = sex_pdx::get_ticks();
            let ex = Exchange { flow, get_head: false, raw: alloc::vec::Vec::new(), cap: usize::MAX, max: usize::MAX,
                head_end: None, framing: None, chunks: sex_fetch::ChunkScan::new(), began: now, start: now, idle: 0, calls: 0, empty: 0, peer_done: false, whole: false,
                setup: Setup::Up, host: alloc::string::String::from(host), port, tls, out: first, addresses: [None; 2], trying: 0, connect_at: 0,
                reused: false, body_end: None, drained: 0 };
            Ok(Duplex { ex, sent_first: false })
        }
        /// True once connected and the first bytes are sent.
        pub fn ready(&self) -> bool { self.ex.setup == Setup::Done }
        /// One step: moves setup on, or takes what arrived into `into`. True when
        /// anything happened. An error means the connection is gone.
        pub fn poll(&mut self, window: &mut Window, into: &mut alloc::vec::Vec<u8>) -> Result<bool, Error> {
            if self.ex.whole { return Err(Error::Failed); }
            if self.ex.setup != Setup::Done {
                // Setting up may not take more than thirty seconds.
                if sex_pdx::get_ticks().saturating_sub(self.ex.start) > 30_000 { self.close(window); return Err(Error::TooSlow); }
                if let Err(e) = self.ex.set_up(window) { self.close(window); return Err(e); }
                if self.ex.setup == Setup::Done { self.sent_first = true; }
                return Ok(self.ex.idle == 0);
            }
            let turn = Turn::take();
            let k = match call_on(window, self.ex.flow, w::RECEIVE, 32 * 1024).and_then(ok) { Ok(k) => k as usize, Err(e) => { drop(turn); self.close(window); return Err(e); } };
            if k > 0 { into.extend_from_slice(&region()?[..k]); return Ok(true); }
            drop(turn);
            let state = call_on(window, self.ex.flow, w::STATE, 0).and_then(ok)?;
            if matches!(state, w::STATE_PEER_CLOSED | w::STATE_CLOSED) { self.close(window); return Err(Error::Failed); }
            Ok(false)
        }
        /// Sends `bytes` (once ready), waiting for room up to ten seconds.
        pub fn send(&mut self, window: &mut Window, mut bytes: &[u8]) -> Result<(), Error> {
            if !self.ready() || self.ex.whole { return Err(Error::Failed); }
            let mut moved = sex_pdx::get_ticks();
            while !bytes.is_empty() {
                let _turn = Turn::take();
                let region = region()?;
                let n = bytes.len().min(region.len());
                region[..n].copy_from_slice(&bytes[..n]);
                let took = ok(call_on(window, self.ex.flow, w::SEND, n as u64)?)? as usize;
                bytes = &bytes[took..];
                if took > 0 { moved = sex_pdx::get_ticks(); } else if sex_pdx::get_ticks().saturating_sub(moved) > 10_000 { return Err(Error::TooSlow); } else { drop(_turn); pause(); }
            }
            if self.ex.tls { ok(call_on(window, self.ex.flow, w::FINISH_REQUEST, 0)?)?; }
            Ok(())
        }
        /// Lets the connection go.
        pub fn close(&mut self, window: &mut Window) { self.ex.cancel(window); }
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
