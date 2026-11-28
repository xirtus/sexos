// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Screen sharing (a web page's `getDisplayMedia`, or any program that wants to show
//! the desktop): the program only asks, and the Shell decides with the person.
//!
//! - **Ask.** The program sends Shell [`OP_SHARE`] [`ASK`]. Shell takes it only from
//!   the focused program right after the person pressed a key or clicked in it (as
//!   the clipboard does). Shell then shows its own picker: the window overview, every
//!   window of the workspace as a live thumbnail, with a hint pill over it that names
//!   the asking program and holds an "Entire screen" button ([`Picker`]). A click on a
//!   window, or Enter on the selected one, shares that window; the button, or E,
//!   shares the whole screen; Escape says no. Programs cannot draw over the picker.
//! - **Share.** Shell tells Display ([`OP_SHARE_VIEW`]) what is shared and with whom.
//!   Display copies that window's own pixels (or the whole screen), fitted into
//!   [`MAX_W`] x [`MAX_H`], into the read-only [`Bank`] each time it publishes a frame
//!   (at most every [`INTERVAL_MS`]), and rings what is shared in red with a "Stop
//!   sharing" pill at the bottom of the screen ([`stop_pill`]).
//! - **Answer.** Shell answers [`OP_SHARED`]: arg0 = [`STARTED`] with arg1 = [`SCREEN`]
//!   or [`WINDOW`] and arg2 = the session; [`DENIED`] (the person said no, or the ask
//!   was not right after a key or click), [`BUSY`] (another program is picking), or
//!   [`STOPPED`] (the person pressed "Stop sharing", or the window went away).
//! - **Stop.** The program sends [`OP_SHARE`] [`STOP`] when it no longer wants it.
//!   Display then zeroes the bank.
//!
//! The bank is one 4 MiB region the kernel makes at boot: Display writes it (slot
//! [`WRITER_SLOT`]) and every program row maps it read-only (slot [`CLIENT_SLOT`]),
//! like the webcam's frame bank. Its header names the program it is shared with
//! ([`Info::target`]); a program shown someone else's share must not read it. Display
//! writes pixels only while a share is on, and zeroes them when it ends.

use core::sync::atomic::{AtomicU32, Ordering};

/// Program -> Shell: arg0 = [`ASK`] or [`STOP`].
pub const OP_SHARE: u64 = 0x330;
/// Shell -> program: arg0 = status, arg1 = kind, arg2 = session.
pub const OP_SHARED: u64 = 0x331;
/// Shell -> Display: arg0 = [`View`] word, arg1 = surface (the window shared, or the
/// asking program's window while picking), arg2 = target pd << 32 | session.
pub const OP_SHARE_VIEW: u64 = 0x332;

pub const ASK: u64 = 1;
pub const STOP: u64 = 2;

pub const STARTED: u64 = 1;
pub const DENIED: u64 = 2;
pub const BUSY: u64 = 3;
pub const STOPPED: u64 = 4;

/// Sends one screen sharing word (`op` [`OP_SHARE`], [`OP_SHARED`] or
/// [`OP_SHARE_VIEW`]) on `route`, waiting a little while the other side is busy.
/// False when it could not be told.
pub fn send(route: u64, op: u64, w: [u64; 3]) -> bool {
    for _ in 0..1024 {
        match crate::pdx_call_checked(route, op, w[0], w[1], w[2]) {
            Ok(_) => return true,
            Err(crate::ERR_SERVICE_NOT_READY) => crate::sys_yield(),
            Err(_) => return false,
        }
    }
    false
}

/// What is shared.
pub const SCREEN: u64 = 1;
pub const WINDOW: u64 = 2;

/// Display's write-capable slot for the bank.
pub const WRITER_SLOT: u64 = 1015;
/// Program rows' read-only slot for the bank (`< 1024` for `program_tenancy`).
pub const CLIENT_SLOT: u64 = 264;
pub const REGION_ID: u32 = 0x4409;
pub const BYTES: u64 = 4 * 1024 * 1024;
/// The biggest picture shared. A bigger window or screen is fitted, keeping its shape.
pub const MAX_W: usize = 960;
pub const MAX_H: usize = 540;
pub const MAX_PIXELS: usize = MAX_W * MAX_H;
pub const HEADER: usize = 4096;
pub const SLOTS: usize = 2;
pub const SLOT_BYTES: usize = MAX_PIXELS * 4;
const _: () = assert!(HEADER + SLOTS * SLOT_BYTES <= BYTES as usize);
/// Display copies at most this often.
pub const INTERVAL_MS: u64 = 66;
pub const TITLE_BYTES: usize = 64;

const MAGIC_VAL: u32 = 0x5348_5253; // "SRHS"
const MAGIC: usize = 0;
const ACTIVE: usize = 4;
const SEQ: usize = 8;
const ACTIVE_SLOT: usize = 12;
const TARGET: usize = 16;
const KIND: usize = 20;
const SESSION: usize = 24;
const TITLE_LEN: usize = 28;
const TITLE: usize = 64;
const SLOT_META: usize = 256;
const SLOT_META_STRIDE: usize = 16;

/// What Display shows and copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Hidden,
    /// Shell's picker is open for the program whose window is `asker`.
    Picking { asker: u64 },
    /// `kind` ([`SCREEN`] or [`WINDOW`]); `surface` is the window shared.
    Sharing { kind: u64, surface: u64, target: u32, session: u32 },
}

impl View {
    pub fn wire(self) -> [u64; 3] {
        match self {
            View::Hidden => [0, 0, 0],
            View::Picking { asker } => [1, asker, 0],
            View::Sharing { kind, surface, target, session } => [1 + kind, surface, (target as u64) << 32 | session as u64],
        }
    }
    pub fn decode(w: [u64; 3]) -> Option<View> {
        Some(match w[0] {
            0 if w[1] == 0 && w[2] == 0 => View::Hidden,
            1 if w[2] == 0 => View::Picking { asker: w[1] },
            k @ (2 | 3) => {
                let kind = k - 1;
                if kind == WINDOW && w[1] == 0 { return None; }
                let (target, session) = ((w[2] >> 32) as u32, w[2] as u32);
                if target == 0 || session == 0 { return None; }
                View::Sharing { kind, surface: if kind == SCREEN { 0 } else { w[1] }, target, session }
            }
            _ => return None,
        })
    }
}

/// The picker's hint pill and its "Entire screen" button, over the overview's top
/// margin, for a `width` screen with a `bar` tall bar at interface scale `scale`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picker {
    pub pill: (usize, usize, usize, usize),
    pub button: (usize, usize, usize, usize),
}

impl Picker {
    pub fn new(width: usize, bar: usize, scale: u32) -> Option<Picker> {
        let s = scale.max(1) as usize;
        let w = (640 * s).min(width.saturating_sub(16 * s));
        let (h, bw) = (26 * s, 130 * s);
        if w < bw + 40 * s { return None; }
        let (x, y) = ((width - w) / 2, bar + 3 * s);
        Some(Picker { pill: (x, y, w, h), button: (x + w - bw - 3 * s, y + 3 * s, bw, h - 6 * s) })
    }
    pub fn on_button(&self, x: usize, y: usize) -> bool { inside(self.button, x, y) }
}

/// The "Stop sharing" pill at the bottom middle of a `width` x `height` screen.
pub fn stop_pill(width: usize, height: usize, scale: u32) -> (usize, usize, usize, usize) {
    let s = scale.max(1) as usize;
    let (w, h) = ((300 * s).min(width), 28 * s);
    ((width - w) / 2, height.saturating_sub(h + 16 * s), w, h)
}

pub fn inside(r: (usize, usize, usize, usize), x: usize, y: usize) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The size a `w` x `h` picture is shared at: at most [`MAX_W`] x [`MAX_H`], same shape.
pub fn fit(w: usize, h: usize) -> (usize, usize) {
    if w == 0 || h == 0 { return (0, 0); }
    if w <= MAX_W && h <= MAX_H { return (w, h); }
    if w * MAX_H >= h * MAX_W { (MAX_W, (h * MAX_W / w).max(1)) } else { ((w * MAX_H / h).max(1), MAX_H) }
}

/// Who is sharing what, as the bank's header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub target: u32,
    pub kind: u64,
    pub session: u32,
    pub title: [u8; TITLE_BYTES],
    pub title_len: usize,
}

impl Info {
    pub fn title(&self) -> &[u8] { &self.title[..self.title_len] }
}

/// One picture read from the bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub seq: u32,
    pub width: usize,
    pub height: usize,
}

/// The shared pictures: Display writes the slot not shown and flips; readers check
/// the slot's even/odd sequence word around their copy, so they never keep a torn one.
#[derive(Clone, Copy)]
pub struct Bank {
    base: *mut u8,
}

impl Bank {
    /// # Safety
    /// `base` must point to a valid, 4-byte-aligned mapping of at least [`BYTES`] bytes.
    pub unsafe fn at(base: *mut u8) -> Bank { Bank { base } }

    /// Display: the writable view.
    #[cfg(target_os = "none")]
    pub fn open_writer() -> Option<Bank> {
        crate::memory_region_full_view(WRITER_SLOT, crate::memory_region::QUERY_VIEW | crate::memory_region::READ | crate::memory_region::WRITE)
            .ok().filter(|v| v.length >= BYTES && v.virtual_address != 0)
            .map(|v| unsafe { Bank::at(v.virtual_address as *mut u8) })
    }

    /// A program: the read-only view.
    #[cfg(target_os = "none")]
    pub fn open_reader() -> Option<Bank> {
        crate::memory_region_full_view(CLIENT_SLOT, crate::memory_region::QUERY_VIEW | crate::memory_region::READ)
            .ok().filter(|v| v.length >= BYTES && v.virtual_address != 0)
            .map(|v| unsafe { Bank::at(v.virtual_address as *mut u8) })
    }

    fn word(&self, at: usize) -> &AtomicU32 { unsafe { &*(self.base.add(at) as *const AtomicU32) } }
    fn load(&self, at: usize) -> u32 { self.word(at).load(Ordering::Acquire) }
    fn store(&self, at: usize, v: u32) { self.word(at).store(v, Ordering::Release) }
    fn pixels(&self, slot: usize) -> *mut u32 { unsafe { self.base.add(HEADER + (slot & 1) * SLOT_BYTES) as *mut u32 } }

    /// Display: a share begins (`title` is the window's, or "Screen").
    pub fn start(&self, target: u32, kind: u64, session: u32, title: &[u8]) {
        self.store(ACTIVE, 0);
        let n = title.len().min(TITLE_BYTES);
        for i in 0..TITLE_BYTES { unsafe { core::ptr::write_volatile(self.base.add(TITLE + i), if i < n { title[i] } else { 0 }); } }
        self.store(TITLE_LEN, n as u32);
        self.store(TARGET, target);
        self.store(KIND, kind as u32);
        self.store(SESSION, session);
        self.store(MAGIC, MAGIC_VAL);
        self.store(ACTIVE, 1);
    }

    /// Display: the share ended; nothing of it stays.
    pub fn stop(&self) {
        self.store(ACTIVE, 0);
        self.store(TARGET, 0);
        self.store(SESSION, 0);
        for slot in 0..SLOTS {
            let meta = SLOT_META + slot * SLOT_META_STRIDE;
            self.store(meta, 0);
            self.store(meta + 4, 0);
            unsafe { core::ptr::write_bytes(self.pixels(slot), 0, MAX_PIXELS); }
        }
        self.store(SEQ, 0);
    }

    pub fn info(&self) -> Option<Info> {
        if self.load(MAGIC) != MAGIC_VAL || self.load(ACTIVE) != 1 { return None; }
        let mut title = [0u8; TITLE_BYTES];
        for (i, b) in title.iter_mut().enumerate() { *b = unsafe { core::ptr::read_volatile(self.base.add(TITLE + i)) }; }
        Some(Info { target: self.load(TARGET), kind: self.load(KIND) as u64, session: self.load(SESSION),
            title, title_len: (self.load(TITLE_LEN) as usize).min(TITLE_BYTES) })
    }

    /// Display: writes a `w` x `h` picture (`fill` gets the slot's pixels, row by row).
    /// Returns the new sequence.
    pub fn publish(&self, w: usize, h: usize, fill: impl FnOnce(&mut [u32])) -> Option<u32> {
        if w == 0 || h == 0 || w > MAX_W || h > MAX_H || self.load(ACTIVE) != 1 { return None; }
        let slot = ((self.load(ACTIVE_SLOT) ^ 1) & 1) as usize;
        let meta = SLOT_META + slot * SLOT_META_STRIDE;
        let next = self.load(SEQ).wrapping_add(1).max(1);
        // Odd while writing.
        self.store(meta, next.wrapping_mul(2) | 1);
        core::sync::atomic::fence(Ordering::Release);
        fill(unsafe { core::slice::from_raw_parts_mut(self.pixels(slot), w * h) });
        self.store(meta + 4, (w as u32) | (h as u32) << 16);
        self.store(meta, next.wrapping_mul(2));
        self.store(ACTIVE_SLOT, slot as u32);
        self.store(SEQ, next);
        Some(next)
    }

    /// A program: copies the newest picture into `out` when it is newer than `last`.
    pub fn read(&self, last: u32, out: &mut [u32]) -> Option<Frame> {
        if self.load(ACTIVE) != 1 { return None; }
        let seq = self.load(SEQ);
        if seq == 0 || seq == last { return None; }
        for _ in 0..4 {
            let slot = (self.load(ACTIVE_SLOT) & 1) as usize;
            let meta = SLOT_META + slot * SLOT_META_STRIDE;
            let s0 = self.load(meta);
            if s0 == 0 || s0 & 1 != 0 { continue; }
            let dims = self.load(meta + 4);
            let (w, h) = ((dims & 0xFFFF) as usize, (dims >> 16) as usize);
            if w == 0 || h == 0 || w > MAX_W || h > MAX_H || out.len() < w * h { return None; }
            unsafe { core::ptr::copy_nonoverlapping(self.pixels(slot), out.as_mut_ptr(), w * h); }
            core::sync::atomic::fence(Ordering::Acquire);
            if self.load(meta) == s0 {
                let seq = s0 / 2;
                if seq == last { return None; }
                return Some(Frame { seq, width: w, height: h });
            }
        }
        None
    }
}

/// One share Shell holds: who, what, and its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Share<S> {
    pub who: S,
    pub target: u32,
    pub kind: u64,
    pub surface: u64,
    pub session: u32,
}

impl<S> Share<S> {
    pub fn view(&self) -> View {
        View::Sharing { kind: self.kind, surface: if self.kind == SCREEN { 0 } else { self.surface }, target: self.target, session: self.session }
    }
}

/// What Shell does after an event: answer `S` with ([`OP_SHARED`] arg0, arg1, arg2),
/// and show the `View`.
pub type Effect<S> = (Option<(S, [u64; 3])>, Option<View>);

/// Shell's side: who asked, who shares, and what each event answers. `S` names a
/// program run (pd, task).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Custody<S: Copy + PartialEq> {
    pub picking: Option<S>,
    pub sharing: Option<Share<S>>,
    session: u32,
}

impl<S: Copy + PartialEq> Custody<S> {
    pub const fn new() -> Self { Self { picking: None, sharing: None, session: 0 } }

    /// `who` asked; `used`: right after a key or click in its focused window. True
    /// when the picker opens.
    pub fn ask(&mut self, who: S, used: bool) -> (bool, Option<(S, [u64; 3])>) {
        if !used { return (false, Some((who, [DENIED, 0, 0]))); }
        // One share at a time: another program picking or sharing makes this one wait.
        if self.picking.is_some_and(|p| p != who) || self.sharing.is_some_and(|s| s.who != who) { return (false, Some((who, [BUSY, 0, 0]))); }
        self.picking = Some(who);
        (true, None)
    }

    /// The person picked (`Some((kind, surface))`) or said no (`None`). `target` is
    /// the asking program's pd. Asking again while sharing picks anew: the new share
    /// takes the old one's place.
    pub fn picked(&mut self, choice: Option<(u64, u64)>, target: u32) -> Effect<S> {
        let Some(who) = self.picking.take() else { return (None, None) };
        let Some((kind, surface)) = choice else {
            return (Some((who, [DENIED, 0, 0])), Some(self.view()));
        };
        self.session = self.session.wrapping_add(1).max(1);
        let share = Share { who, target, kind, surface: if kind == SCREEN { 0 } else { surface }, session: self.session };
        self.sharing = Some(share);
        (Some((who, [STARTED, kind, self.session as u64])), Some(share.view()))
    }

    /// The share ended. `tell`: answer the program [`STOPPED`] (not when it asked
    /// to stop itself, or ended).
    pub fn stop(&mut self, tell: bool) -> Effect<S> {
        let Some(s) = self.sharing.take() else { return (None, None) };
        (tell.then_some((s.who, [STOPPED, s.kind, s.session as u64])), Some(View::Hidden))
    }

    /// What Display shows once the picker closed: the share still on, else nothing.
    pub fn view(&self) -> View { self.sharing.map_or(View::Hidden, |s| s.view()) }
}
