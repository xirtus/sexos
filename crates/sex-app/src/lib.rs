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
