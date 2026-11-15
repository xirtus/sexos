// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bluetooth on the desktop (Cufflinks W2b): the Shell's Bluetooth panel and its
//! pairing prompt.
//!
//! **Cufflinks <-> Shell.** One 4 KiB region, paired by BOOT between the Shell and
//! Cufflinks (kernel `boot_memory::init_bluetooth`), plus one route, Cufflinks -> Shell
//! ([`NOTIFY_SLOT`], op [`OP_EVENT`]), so a sleeping Shell wakes for a pairing prompt.
//! No Shell -> Cufflinks route: Cufflinks polls the region every tick while a
//! controller is up, and with no controller there is nothing to command, so an idle
//! Cufflinks is never woken by the desktop.
//!
//! - Cufflinks writes a [`Snapshot`] (controller phase, scanning, the devices it saw or
//!   has bonded, a numeric comparison waiting for the user, the last pairing outcome)
//!   under a sequence lock; the Shell reads it whole ([`Shared::read`]) and retries a
//!   torn read.
//! - The Shell writes [`Command`]s (scan on/off, pair, forget, confirm) into a small
//!   single-producer ring; Cufflinks takes them ([`Shared::take_command`]).
//! - The Shell marks itself attached ([`Shared::shell_attach`]) and says whether its
//!   panel is open. Cufflinks asks the user only while a Shell is attached and the wake
//!   reaches it; otherwise it falls back to its bounded first-setup rule.
//! - [`OP_EVENT`] (Cufflinks -> Shell, arg0 = [`EVENT_CHANGED`], [`EVENT_COMPARE`] or
//!   [`EVENT_OUTCOME`]) only means "look at the region"; the Shell does not reply.
//!   Cufflinks sends `EVENT_CHANGED` only while the panel is open.
//!
//! **Shell -> Display.** The Shell decides what the panel says ([`Ui`]); Display draws
//! it over the desktop like the tray menus. [`OP_PANEL`] carries [`Lines`] in 16-byte
//! chunks ([`chunks`], [`Assembler`]); both sides place it with [`Panel`], so a click
//! lands on what is drawn. Only the Shell (domain [`SHELL_PD`]) may send it.
//!
//! Everything a peer named (device names) is untrusted: names are cut to [`NAME_MAX`]
//! printable ASCII bytes before they reach the region, and every count and length read
//! back is checked.

use core::sync::atomic::{fence, AtomicU32, Ordering};

/// The region, in the Shell and in Cufflinks.
pub const SLOT: u64 = 1007;
pub const REGION_ID: u32 = 0x4405;
pub const BYTES: u64 = 4096;
/// Cufflinks's route to the Shell.
pub const NOTIFY_SLOT: u64 = 1008;
/// Cufflinks -> Shell: the region changed (arg0 = an `EVENT_*`). No reply.
pub const OP_EVENT: u64 = 0x2CA;
/// Shell -> Display: one chunk of the panel ([`Chunk`] in arg0, 16 text bytes in arg1/arg2).
pub const OP_PANEL: u64 = 0x2CB;
/// The Shell's BOOT domain (`shell_ready::SHELL_PD_ID`).
pub const SHELL_PD: u32 = 3;

pub const EVENT_CHANGED: u64 = 1;
pub const EVENT_COMPARE: u64 = 2;
pub const EVENT_OUTCOME: u64 = 3;

/// Controller phases.
pub const UNAVAILABLE: u8 = 0; // Cufflinks has not written the region (not running)
pub const NO_CONTROLLER: u8 = 1;
pub const STARTING: u8 = 2; // firmware load and bring-up
pub const READY: u8 = 3;
pub const FAILED: u8 = 4;

/// Device kinds.
pub const OTHER: u8 = 0;
pub const KEYBOARD: u8 = 1;
pub const MOUSE: u8 = 2;
/// A keyboard with a pointer, or an input device that does not say which.
pub const INPUT: u8 = 3;
pub const HEADPHONES: u8 = 4;

/// Device flags.
pub const PAIRED: u8 = 1; // bonded: its keys are kept
pub const CONNECTED: u8 = 2;
pub const IN_USE: u8 = 4; // input or audio is up
pub const PAIRING: u8 = 8; // we are connecting or pairing it now
pub const LE: u8 = 16;

pub const MAX_DEVICES: usize = 12;
pub const NAME_MAX: usize = 40;

/// What kind of device a scan result is, from its class of device (classic) or
/// appearance (LE). Same rules as `sex_bt::host::Host::is_hid` and `is_audio`.
pub fn kind_of(class: Option<u32>, appearance: Option<u16>) -> u8 {
    if let Some(c) = class {
        if (c >> 8) & 0x1F == 0x04 && c & (1 << 18 | 1 << 21) != 0 { return HEADPHONES; }
        if (c >> 8) & 0x1F == 0x05 {
            return match (c >> 6) & 3 { 1 => KEYBOARD, 2 => MOUSE, _ => INPUT };
        }
    }
    match appearance {
        Some(0x03C1) => KEYBOARD,
        Some(0x03C2) => MOUSE,
        Some(0x03C0) => INPUT,
        _ => OTHER,
    }
}

pub const fn kind_name(kind: u8) -> &'static str {
    match kind { KEYBOARD => "keyboard", MOUSE => "mouse", INPUT => "input device", HEADPHONES => "headphones", _ => "other" }
}

/// Whether Cufflinks can pair this kind (HID and A2DP only).
pub const fn supported(kind: u8) -> bool { matches!(kind, KEYBOARD | MOUSE | INPUT | HEADPHONES) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device { pub addr: [u8; 6], pub kind: u8, pub flags: u8, pub rssi: i8, pub name_len: u8, pub name: [u8; NAME_MAX] }

impl Device {
    pub const EMPTY: Device = Device { addr: [0; 6], kind: OTHER, flags: 0, rssi: 127, name_len: 0, name: [0; NAME_MAX] };
    pub fn new(addr: [u8; 6]) -> Device { Device { addr, ..Device::EMPTY } }
    pub fn name(&self) -> &[u8] { &self.name[..(self.name_len as usize).min(NAME_MAX)] }
    pub fn has(&self, flags: u8) -> bool { self.flags & flags != 0 }
    /// A peer's name (UTF-8) as printable ASCII: each other character becomes one '?',
    /// cut to `NAME_MAX`. True if it changed.
    pub fn set_name(&mut self, raw: &[u8]) -> bool {
        let mut out = [0u8; NAME_MAX];
        let mut n = 0;
        for &b in raw {
            if n == NAME_MAX { break; }
            if (0x80..0xC0).contains(&b) { continue; } // UTF-8 continuation byte
            out[n] = if (0x20..0x7F).contains(&b) { b } else { b'?' };
            n += 1;
        }
        while n > 0 && out[n - 1] == b' ' { n -= 1; }
        if n == 0 || (out == self.name && n == self.name_len as usize) { return false; }
        self.name = out;
        self.name_len = n as u8;
        true
    }
}

/// "AA:BB:CC:DD:EE:FF" (the address bytes as HCI carries them, little-endian).
pub fn addr_text(addr: &[u8; 6], out: &mut [u8; 17]) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in 0..6 {
        let b = addr[5 - i];
        out[i * 3] = HEX[(b >> 4) as usize];
        out[i * 3 + 1] = HEX[(b & 15) as usize];
        if i < 5 { out[i * 3 + 2] = b':'; }
    }
}

/// A numeric comparison waiting for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compare { pub ticket: u32, pub addr: [u8; 6], pub value: u32 }

/// How the last pairing ended. `serial` grows with each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome { pub serial: u32, pub addr: [u8; 6], pub ok: bool, pub reason: u8 }

/// Pairing failure reasons Cufflinks adds to the controller's and SMP's own codes.
pub const REASON_REFUSED: u8 = 0xF0; // the user said no
pub const REASON_TIMEOUT: u8 = 0xF1; // nobody answered in time
pub const REASON_UNSUPPORTED: u8 = 0xF2; // not a keyboard, mouse or headset
pub const REASON_NOT_FOUND: u8 = 0xF3; // not seen by this scan

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: u8,
    pub scanning: bool,
    pub count: u8,
    pub devices: [Device; MAX_DEVICES],
    pub compare: Option<Compare>,
    pub outcome: Option<Outcome>,
}

const HEAD_BYTES: usize = 32;
const DEVICE_BYTES: usize = 6 + 4 + NAME_MAX;
pub const SNAPSHOT_BYTES: usize = HEAD_BYTES + MAX_DEVICES * DEVICE_BYTES;

impl Snapshot {
    pub const EMPTY: Snapshot = Snapshot { phase: UNAVAILABLE, scanning: false, count: 0, devices: [Device::EMPTY; MAX_DEVICES], compare: None, outcome: None };

    pub fn devices(&self) -> &[Device] { &self.devices[..(self.count as usize).min(MAX_DEVICES)] }
    pub fn find(&self, addr: &[u8; 6]) -> Option<&Device> { self.devices().iter().find(|d| d.addr == *addr) }
    fn index(&self, addr: &[u8; 6]) -> Option<usize> { self.devices().iter().position(|d| d.addr == *addr) }

    /// Cufflinks: add or refresh a device. True if anything the panel shows changed
    /// (signal strength alone does not count). When the table is full, the oldest
    /// device that is neither paired, connected nor pairing makes room; if every
    /// device is one of those, the new one is not listed.
    pub fn put(&mut self, addr: [u8; 6], kind: u8, name: Option<&[u8]>, rssi: Option<i8>, le: bool) -> bool {
        let at = match self.index(&addr) {
            Some(i) => i,
            None => {
                if self.count as usize == MAX_DEVICES {
                    let Some(old) = self.devices().iter().position(|d| !d.has(PAIRED | CONNECTED | PAIRING)) else { return false };
                    self.devices.copy_within(old + 1..MAX_DEVICES, old);
                    self.count -= 1;
                }
                let i = self.count as usize;
                self.devices[i] = Device::new(addr);
                self.count += 1;
                if le { self.devices[i].flags |= LE; }
                if let Some(n) = name { self.devices[i].set_name(n); }
                self.devices[i].kind = kind;
                if let Some(r) = rssi { self.devices[i].rssi = r; }
                return true;
            }
        };
        let d = &mut self.devices[at];
        let mut changed = false;
        if kind != OTHER && d.kind != kind { d.kind = kind; changed = true; }
        if let Some(n) = name { changed |= d.set_name(n); }
        if let Some(r) = rssi { d.rssi = r; }
        changed
    }

    /// Cufflinks: set or clear flags of a listed device. True if they changed.
    pub fn set(&mut self, addr: &[u8; 6], flags: u8, on: bool) -> bool {
        let Some(i) = self.index(addr) else { return false };
        let d = &mut self.devices[i];
        let before = d.flags;
        if on { d.flags |= flags } else { d.flags &= !flags }
        before != d.flags
    }

    pub fn remove(&mut self, addr: &[u8; 6]) -> bool {
        let Some(i) = self.index(addr) else { return false };
        self.devices.copy_within(i + 1..MAX_DEVICES, i);
        self.devices[MAX_DEVICES - 1] = Device::EMPTY;
        self.count -= 1;
        true
    }

    pub fn encode(&self, out: &mut [u8; SNAPSHOT_BYTES]) {
        out.fill(0);
        out[0] = self.phase;
        out[1] = self.scanning as u8;
        out[2] = (self.count as usize).min(MAX_DEVICES) as u8;
        if let Some(c) = self.compare {
            out[3] = 1;
            out[4..8].copy_from_slice(&c.ticket.to_le_bytes());
            out[8..14].copy_from_slice(&c.addr);
            out[14..18].copy_from_slice(&c.value.to_le_bytes());
        }
        if let Some(o) = self.outcome {
            out[18] = 1 | (o.ok as u8) << 1;
            out[19] = o.reason;
            out[20..24].copy_from_slice(&o.serial.to_le_bytes());
            out[24..30].copy_from_slice(&o.addr);
        }
        for (i, d) in self.devices().iter().enumerate() {
            let at = HEAD_BYTES + i * DEVICE_BYTES;
            out[at..at + 6].copy_from_slice(&d.addr);
            out[at + 6] = d.kind;
            out[at + 7] = d.flags;
            out[at + 8] = d.rssi as u8;
            out[at + 9] = d.name_len;
            out[at + 10..at + 10 + NAME_MAX].copy_from_slice(&d.name);
        }
    }

    /// Checked: a bad phase, count or name length is refused, and names are made
    /// printable again (the bytes came from another domain).
    pub fn decode(b: &[u8; SNAPSHOT_BYTES]) -> Option<Snapshot> {
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let addr_at = |i: usize| { let mut a = [0u8; 6]; a.copy_from_slice(&b[i..i + 6]); a };
        if b[0] > FAILED || b[2] as usize > MAX_DEVICES || b[1] > 1 { return None; }
        let mut s = Snapshot { phase: b[0], scanning: b[1] == 1, count: b[2], ..Snapshot::EMPTY };
        if b[3] == 1 { s.compare = Some(Compare { ticket: u32_at(4), addr: addr_at(8), value: u32_at(14) % 1_000_000 }); }
        if b[18] & 1 == 1 { s.outcome = Some(Outcome { serial: u32_at(20), addr: addr_at(24), ok: b[18] & 2 != 0, reason: b[19] }); }
        for i in 0..s.count as usize {
            let at = HEAD_BYTES + i * DEVICE_BYTES;
            let len = b[at + 9] as usize;
            if len > NAME_MAX || b[at + 6] > HEADPHONES { return None; }
            let mut d = Device { addr: addr_at(at), kind: b[at + 6], flags: b[at + 7], rssi: b[at + 8] as i8, ..Device::EMPTY };
            d.set_name(&b[at + 10..at + 10 + len]);
            s.devices[i] = d;
        }
        Some(s)
    }
}

/// What the Shell asks Cufflinks to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Start (true) or stop (false) looking for new devices.
    Scan(bool),
    /// Connect this device (pairing it if it is new): as a keyboard/mouse or a headset by its kind.
    Pair([u8; 6]),
    /// Forget the device's keys and drop its link.
    Forget([u8; 6]),
    /// The user's answer to the numeric comparison with this ticket.
    Confirm { ticket: u32, yes: bool },
}

pub const COMMAND_BYTES: usize = 16;

impl Command {
    pub fn encode(&self) -> [u8; COMMAND_BYTES] {
        let mut b = [0u8; COMMAND_BYTES];
        match *self {
            Command::Scan(on) => { b[0] = 1; b[1] = on as u8; }
            Command::Pair(a) => { b[0] = 2; b[2..8].copy_from_slice(&a); }
            Command::Forget(a) => { b[0] = 3; b[2..8].copy_from_slice(&a); }
            Command::Confirm { ticket, yes } => { b[0] = 4; b[1] = yes as u8; b[8..12].copy_from_slice(&ticket.to_le_bytes()); }
        }
        b
    }
    pub fn decode(b: &[u8; COMMAND_BYTES]) -> Option<Command> {
        let mut a = [0u8; 6];
        a.copy_from_slice(&b[2..8]);
        match (b[0], b[1]) {
            (1, f @ 0..=1) => Some(Command::Scan(f == 1)),
            (2, 0) => Some(Command::Pair(a)),
            (3, 0) => Some(Command::Forget(a)),
            (4, f @ 0..=1) => Some(Command::Confirm { ticket: u32::from_le_bytes([b[8], b[9], b[10], b[11]]), yes: f == 1 }),
            _ => None,
        }
    }
}

const MAGIC: u32 = 0x5546_5543; // "CUFU"
const SHELL_MAGIC: u32 = 0x4C45_4853; // "SHEL"
const W_MAGIC: usize = 0; // Cufflinks
const W_SEQ: usize = 4; // Cufflinks: odd while writing the snapshot
const W_TAKEN: usize = 8; // Cufflinks: commands taken
const W_SHELL: usize = 16; // Shell: SHELL_MAGIC once attached
const W_OPEN: usize = 20; // Shell: the panel shows
const W_WRITTEN: usize = 24; // Shell: commands written
const SNAP_AT: usize = 64;
const CMD_AT: usize = 1024;
/// Commands the ring holds.
pub const COMMANDS: u32 = 8;
const _: () = assert!(SNAP_AT + SNAPSHOT_BYTES <= CMD_AT && CMD_AT + COMMANDS as usize * COMMAND_BYTES <= BYTES as usize);

/// The region, seen from either side. `base` must be [`BYTES`] long and 4-byte aligned.
#[derive(Clone, Copy)]
pub struct Shared { base: *mut u8 }

impl Shared {
    /// # Safety
    /// `base` points at a live mapping of at least [`BYTES`] bytes for as long as this is used.
    pub unsafe fn at(base: *mut u8) -> Shared { Shared { base } }
    fn word(&self, at: usize) -> &AtomicU32 { unsafe { &*(self.base.add(at) as *const AtomicU32) } }
    fn load(&self, at: usize) -> u32 { self.word(at).load(Ordering::Acquire) }
    fn store(&self, at: usize, v: u32) { self.word(at).store(v, Ordering::Release) }

    // ── Cufflinks ──

    /// Cufflinks (re)started: commands from before are dropped (a stale "pair" must not
    /// run later), then the first snapshot makes the region valid.
    pub fn cufflinks_start(&self, first: &Snapshot) {
        self.store(W_TAKEN, self.load(W_WRITTEN));
        self.publish(first);
        self.store(W_MAGIC, MAGIC);
    }
    pub fn publish(&self, s: &Snapshot) {
        let mut bytes = [0u8; SNAPSHOT_BYTES];
        s.encode(&mut bytes);
        let seq = self.load(W_SEQ) | 1;
        self.store(W_SEQ, seq);
        fence(Ordering::Release);
        for (i, b) in bytes.iter().enumerate() { unsafe { self.base.add(SNAP_AT + i).write_volatile(*b) } }
        fence(Ordering::Release);
        self.store(W_SEQ, seq.wrapping_add(1));
    }
    /// A Shell has attached (it may still be asleep; the wake reaches it).
    pub fn shell_attached(&self) -> bool { self.load(W_SHELL) == SHELL_MAGIC }
    pub fn shell_open(&self) -> bool { self.shell_attached() && self.load(W_OPEN) != 0 }
    /// The next command, if any; a damaged one is skipped.
    pub fn take_command(&self) -> Option<Command> {
        loop {
            let taken = self.load(W_TAKEN);
            let written = self.load(W_WRITTEN);
            if written == taken { return None; }
            // A writer more than a ring ahead went wrong: drop what it wrote.
            if written.wrapping_sub(taken) > COMMANDS { self.store(W_TAKEN, written); return None; }
            let at = CMD_AT + (taken % COMMANDS) as usize * COMMAND_BYTES;
            let mut b = [0u8; COMMAND_BYTES];
            for (i, o) in b.iter_mut().enumerate() { *o = unsafe { self.base.add(at + i).read_volatile() }; }
            self.store(W_TAKEN, taken.wrapping_add(1));
            if let Some(c) = Command::decode(&b) { return Some(c); }
        }
    }

    // ── Shell ──

    /// Whether Cufflinks has written the region.
    pub fn available(&self) -> bool { self.load(W_MAGIC) == MAGIC }
    pub fn shell_attach(&self) { self.store(W_SHELL, SHELL_MAGIC); }
    pub fn set_open(&self, open: bool) { self.store(W_OPEN, open as u32); }
    /// The whole snapshot, or None if Cufflinks has not written one or kept writing
    /// while we read.
    pub fn read(&self) -> Option<Snapshot> {
        if !self.available() { return None; }
        let mut bytes = [0u8; SNAPSHOT_BYTES];
        for _ in 0..16 {
            let before = self.load(W_SEQ);
            if before & 1 == 1 { core::hint::spin_loop(); continue; }
            for (i, b) in bytes.iter_mut().enumerate() { *b = unsafe { self.base.add(SNAP_AT + i).read_volatile() }; }
            fence(Ordering::Acquire);
            if self.load(W_SEQ) == before { return Snapshot::decode(&bytes); }
        }
        None
    }
    /// False when the ring is full (Cufflinks has not taken the last `COMMANDS`).
    pub fn push_command(&self, c: Command) -> bool {
        let written = self.load(W_WRITTEN);
        if written.wrapping_sub(self.load(W_TAKEN)) >= COMMANDS { return false; }
        let at = CMD_AT + (written % COMMANDS) as usize * COMMAND_BYTES;
        for (i, b) in c.encode().iter().enumerate() { unsafe { self.base.add(at + i).write_volatile(*b) } }
        self.store(W_WRITTEN, written.wrapping_add(1));
        true
    }
}

// ── The panel: Shell decides, Display draws ──

pub const LINES: usize = 14;
pub const LINE_BYTES: usize = 48;
pub const CHUNK_BYTES: usize = 16;
pub const PARTS: usize = LINE_BYTES / CHUNK_BYTES;

/// Line styles. A line may hold one '\t': what follows is drawn right-aligned.
pub const TEXT: u8 = 0;
pub const TITLE: u8 = 1;
pub const DIM: u8 = 2;
/// A row the user can pick (highlighted when selected).
pub const BUTTON: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lines { pub text: [[u8; LINE_BYTES]; LINES], pub len: [u8; LINES], pub style: [u8; LINES], pub count: u8 }

impl Lines {
    pub const EMPTY: Lines = Lines { text: [[0; LINE_BYTES]; LINES], len: [0; LINES], style: [0; LINES], count: 0 };
    pub fn line(&self, i: usize) -> &[u8] { &self.text[i][..(self.len[i] as usize).min(LINE_BYTES)] }
    /// Adds a line made of `parts`, cut to `LINE_BYTES`; false when there is no room.
    pub fn push(&mut self, style: u8, parts: &[&[u8]]) -> bool {
        let i = self.count as usize;
        if i == LINES { return false; }
        let mut n = 0;
        for p in parts {
            for &b in p.iter() {
                if n == LINE_BYTES { break; }
                self.text[i][n] = if b == b'\t' || (0x20..0x7F).contains(&b) { b } else { b'?' };
                n += 1;
            }
        }
        self.len[i] = n as u8;
        self.style[i] = style;
        self.count += 1;
        true
    }
    /// The line, split at its '\t' into (left, right).
    pub fn columns(&self, i: usize) -> (&[u8], &[u8]) {
        let l = self.line(i);
        match l.iter().position(|b| *b == b'\t') { Some(t) => (&l[..t], &l[t + 1..]), None => (l, &[]) }
    }
}

/// arg0 of `OP_PANEL`: which line and part this chunk is, that line's style, the
/// panel's line count and selected line, and whether it is the last chunk (Display
/// shows the panel then). A count of zero hides it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk { pub line: u8, pub part: u8, pub style: u8, pub count: u8, pub selected: Option<u8>, pub last: bool }

const CHUNK_TAG: u64 = 0xB1;

impl Chunk {
    pub fn word(&self) -> u64 {
        self.line as u64 | (self.part as u64) << 8 | (self.style as u64 & 0xF) << 12 | (self.count as u64) << 16
            | (self.selected.unwrap_or(0xFF) as u64) << 24 | (self.last as u64) << 32 | CHUNK_TAG << 56
    }
    pub fn decode(w: u64) -> Option<Chunk> {
        if w >> 56 != CHUNK_TAG || w >> 33 & 0x7F_FFFF != 0 { return None; }
        let c = Chunk { line: w as u8, part: (w >> 8) as u8 & 0xF, style: (w >> 12) as u8 & 0xF, count: (w >> 16) as u8,
            selected: match (w >> 24) as u8 { 0xFF => None, s => Some(s) }, last: w >> 32 & 1 == 1 };
        let lines_ok = c.count as usize <= LINES && (c.line < c.count || c.count == 0) && (c.part as usize) < PARTS;
        let selected_ok = c.selected.map_or(true, |s| s < c.count);
        (lines_ok && selected_ok && c.style <= BUTTON).then_some(c)
    }
}

/// The calls that show `lines` with `selected` (or hide the panel when `lines` is empty).
pub fn chunks(lines: &Lines, selected: Option<usize>) -> impl Iterator<Item = (u64, u64, u64)> + '_ {
    let count = lines.count.min(LINES as u8);
    let selected = selected.filter(|s| *s < count as usize).map(|s| s as u8);
    let total = (count as usize * PARTS).max(1);
    (0..total).map(move |k| {
        let (line, part) = ((k / PARTS) as u8, (k % PARTS) as u8);
        let c = Chunk { line, part, style: if count == 0 { 0 } else { lines.style[line as usize] }, count, selected, last: k + 1 == total };
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        if count != 0 {
            let at = part as usize * CHUNK_BYTES;
            let t = &lines.text[line as usize];
            let n = lines.len[line as usize] as usize;
            for i in 0..8 { if at + i < n { a[i] = t[at + i]; } if at + 8 + i < n { b[i] = t[at + 8 + i]; } }
        }
        (c.word(), u64::from_le_bytes(a), u64::from_le_bytes(b))
    })
}

/// Display's side: collects chunks; the last one gives the panel to draw.
pub struct Assembler { staging: Lines }

impl Default for Assembler { fn default() -> Self { Self::new() } }

impl Assembler {
    pub const fn new() -> Self { Self { staging: Lines::EMPTY } }
    /// One `OP_PANEL` call: `Some((lines, selected))` when the panel is complete
    /// (an empty `lines` hides it); None while more chunks are due or the chunk is bad.
    pub fn push(&mut self, word: u64, a: u64, b: u64) -> Option<(Lines, Option<usize>)> {
        let c = Chunk::decode(word)?;
        if c.line == 0 && c.part == 0 { self.staging = Lines::EMPTY; }
        if c.count != 0 {
            let (l, at) = (c.line as usize, c.part as usize * CHUNK_BYTES);
            let t = &mut self.staging.text[l];
            t[at..at + 8].copy_from_slice(&a.to_le_bytes());
            t[at + 8..at + 16].copy_from_slice(&b.to_le_bytes());
            for x in t[at..at + 16].iter_mut() { if *x != 0 && *x != b'\t' && !(0x20..0x7F).contains(x) { *x = b'?'; } }
            self.staging.len[l] = t.iter().position(|x| *x == 0).unwrap_or(LINE_BYTES) as u8;
            self.staging.style[l] = c.style;
        }
        if !c.last { return None; }
        self.staging.count = c.count;
        let out = (self.staging, c.selected.map(|s| s as usize));
        self.staging = Lines::EMPTY;
        Some(out)
    }
}

pub type Rect = (usize, usize, usize, usize);
fn inside(r: Rect, x: usize, y: usize) -> bool { x >= r.0 && y >= r.1 && x < r.0 + r.2 && y < r.1 + r.3 }

/// Where the panel goes: under the bar at the right, one row per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panel { pub panel: Rect, pub pad: usize, pub row: usize, pub count: usize }

impl Panel {
    pub fn new(width: usize, bar: usize, scale: usize, count: usize) -> Option<Panel> {
        let s = scale.max(1);
        let (w, pad, row) = (420 * s, 12 * s, 26 * s);
        if width < w + 16 * s || count == 0 { return None; }
        let (x, y) = (width - w - 8 * s, bar + 6 * s);
        Some(Panel { panel: (x, y, w, 2 * pad + count.min(LINES) * row), pad, row, count: count.min(LINES) })
    }
    pub fn row(&self, i: usize) -> Rect {
        let (x, y, w, _) = self.panel;
        (x + self.pad, y + self.pad + i * self.row, w - 2 * self.pad, self.row)
    }
    pub fn inside(&self, x: usize, y: usize) -> bool { inside(self.panel, x, y) }
    pub fn row_at(&self, x: usize, y: usize) -> Option<usize> { (0..self.count).find(|&i| inside(self.row(i), x, y)) }
}

// ── The Shell's panel model (what it says, what keys and clicks do) ──

/// What the Shell should do after a key or click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Show the panel again (the selection or text changed).
    Redraw,
    Close,
    Send(Command),
}

/// Key codes the panel reads (the Shell's scancodes; Up/Down also as USB delivers them).
pub mod key {
    pub const ESC: u8 = 0x01;
    pub const ENTER: u8 = 0x1C;
    pub const ENTER_PAD: u8 = 0x9C;
    pub const SPACE: u8 = 0x39;
    pub const UP: u8 = 0x48;
    pub const DOWN: u8 = 0x50;
    pub const UP_USB: u8 = 0x67;
    pub const DOWN_USB: u8 = 0x6C;
    pub const DELETE: u8 = 0x53;
    pub const BACKSPACE: u8 = 0x0E;
    pub const Y: u8 = 0x15;
    pub const N: u8 = 0x31;
    pub const S: u8 = 0x1F;
    pub const F: u8 = 0x21;
}

/// What a selectable row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item { Scan, Device([u8; 6]), Yes, No }

/// The panel as the user sees it. `selected` counts selectable rows only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ui {
    pub open: bool,
    pub selected: usize,
    /// The comparison on screen (its ticket and the device's name, which may leave
    /// the list while the dialog shows).
    pub dialog: Option<Compare>,
    /// The last comparison answered or dismissed, so it is not asked again.
    pub answered: u32,
    /// The last outcome shown, and whether it still shows.
    pub outcome_seen: u32,
    pub notice: Option<Outcome>,
}

impl Default for Ui { fn default() -> Self { Self::new() } }

impl Ui {
    pub const fn new() -> Ui { Ui { open: false, selected: 0, dialog: None, answered: 0, outcome_seen: 0, notice: None } }

    /// Devices in the order shown: paired or connected ones first.
    fn order(s: &Snapshot, out: &mut [usize; MAX_DEVICES]) -> usize {
        let mut n = 0;
        for pass in 0..2 {
            for (i, d) in s.devices().iter().enumerate() {
                if d.has(PAIRED | CONNECTED) == (pass == 0) { out[n] = i; n += 1; }
            }
        }
        n
    }
    /// The devices shown (at most what fits under the fixed rows).
    pub const MAX_SHOWN: usize = LINES - 5;

    fn items(&self, s: Option<&Snapshot>, out: &mut [Item; MAX_DEVICES + 1]) -> usize {
        if self.dialog.is_some() { out[0] = Item::Yes; out[1] = Item::No; return 2; }
        let Some(s) = s.filter(|s| s.phase == READY) else { return 0 };
        out[0] = Item::Scan;
        let mut order = [0usize; MAX_DEVICES];
        let n = Self::order(s, &mut order).min(Self::MAX_SHOWN);
        for k in 0..n { out[1 + k] = Item::Device(s.devices[order[k]].addr); }
        1 + n
    }

    /// Follows a new snapshot: a waiting comparison opens the dialog (and the panel);
    /// one that went away (answered elsewhere, timed out) closes it. True if the
    /// panel should be shown again.
    pub fn update(&mut self, s: Option<&Snapshot>) -> bool {
        let mut redraw = false;
        let compare = s.and_then(|s| s.compare);
        match (self.dialog, compare) {
            (Some(d), Some(c)) if d.ticket == c.ticket => {}
            (_, Some(c)) if c.ticket != self.answered => {
                self.dialog = Some(c);
                self.open = true;
                self.selected = 0;
                redraw = true;
            }
            (Some(_), _) => { self.dialog = None; self.selected = 0; redraw = true; }
            _ => {}
        }
        if let Some(o) = s.and_then(|s| s.outcome) {
            if o.serial != self.outcome_seen {
                self.outcome_seen = o.serial;
                self.notice = Some(o);
                redraw = true;
            }
        }
        let mut items = [Item::Scan; MAX_DEVICES + 1];
        let n = self.items(s, &mut items);
        if self.selected >= n.max(1) { self.selected = n.saturating_sub(1); redraw = true; }
        redraw && self.open
    }

    pub fn toggle(&mut self) -> Action {
        self.open = !self.open;
        self.selected = 0;
        self.notice = None;
        if self.open { Action::Redraw } else { Action::Close }
    }

    fn answer(&mut self, yes: bool) -> Action {
        let Some(d) = self.dialog.take() else { return Action::None };
        self.answered = d.ticket;
        self.selected = 0;
        Action::Send(Command::Confirm { ticket: d.ticket, yes })
    }

    fn activate(&mut self, item: Item, s: Option<&Snapshot>) -> Action {
        self.notice = None;
        match item {
            Item::Yes => self.answer(true),
            Item::No => self.answer(false),
            Item::Scan => Action::Send(Command::Scan(!s.is_some_and(|s| s.scanning))),
            Item::Device(a) => Action::Send(Command::Pair(a)),
        }
    }

    /// A key while the panel shows (presses and repeats only).
    pub fn key(&mut self, code: u8, s: Option<&Snapshot>) -> Action {
        let mut items = [Item::Scan; MAX_DEVICES + 1];
        let n = self.items(s, &mut items);
        if self.dialog.is_some() {
            match code {
                key::Y => return self.answer(true),
                key::N | key::ESC => return self.answer(false),
                _ => {}
            }
        }
        match code {
            key::ESC => { self.open = false; self.notice = None; Action::Close }
            key::UP | key::UP_USB => { if self.selected > 0 { self.selected -= 1; } Action::Redraw }
            key::DOWN | key::DOWN_USB => { if self.selected + 1 < n { self.selected += 1; } Action::Redraw }
            key::ENTER | key::ENTER_PAD | key::SPACE if self.selected < n => self.activate(items[self.selected], s),
            key::S if self.dialog.is_none() && n > 0 => self.activate(Item::Scan, s),
            key::DELETE | key::BACKSPACE | key::F if self.dialog.is_none() => match items.get(self.selected) {
                Some(Item::Device(a)) if self.selected < n && s.and_then(|s| s.find(a)).is_some_and(|d| d.has(PAIRED)) => {
                    self.notice = None;
                    Action::Send(Command::Forget(*a))
                }
                _ => Action::None,
            },
            _ => Action::None,
        }
    }

    /// A click on line `line` of what [`Ui::lines`] made (None: outside the panel).
    pub fn click(&mut self, line: Option<usize>, s: Option<&Snapshot>) -> Action {
        let Some(line) = line else {
            // Outside closes the list; a question waits for its answer.
            if self.dialog.is_some() { return Action::None; }
            self.open = false;
            return Action::Close;
        };
        let (lines, _) = self.lines(s);
        let mut items = [Item::Scan; MAX_DEVICES + 1];
        let n = self.items(s, &mut items);
        let picked = (0..=line).filter(|&i| lines.style[i] == BUTTON).count();
        if line >= lines.count as usize || lines.style[line] != BUTTON || picked == 0 || picked > n { return Action::None; }
        self.selected = picked - 1;
        self.activate(items[picked - 1], s)
    }

    /// The panel's lines and which line is selected.
    pub fn lines(&self, s: Option<&Snapshot>) -> (Lines, Option<usize>) {
        let mut l = Lines::EMPTY;
        let name_of = |addr: &[u8; 6], buf: &mut [u8; 17]| -> usize {
            match s.and_then(|s| s.find(addr)).filter(|d| d.name_len != 0) {
                Some(d) => { let n = d.name(); buf[..n.len().min(17)].copy_from_slice(&n[..n.len().min(17)]); n.len().min(17) }
                None => { addr_text(addr, buf); 17 }
            }
        };
        if let Some(d) = self.dialog {
            let mut name = [0u8; 17];
            let n = name_of(&d.addr, &mut name);
            let mut digits = [0u8; 6];
            for (i, x) in digits.iter_mut().enumerate() { *x = b'0' + (d.value / 10u32.pow(5 - i as u32) % 10) as u8; }
            l.push(TITLE, &[b"Pair with ", &name[..n], b"?"]);
            l.push(TEXT, &[b"Does it show ", &digits, b"?"]);
            l.push(BUTTON, &[b"Yes, the numbers match"]);
            l.push(BUTTON, &[b"No"]);
            l.push(DIM, &[b"Y: yes   N or Esc: no"]);
            return (l, Some(2 + self.selected.min(1)));
        }
        let phase = s.map_or(UNAVAILABLE, |s| s.phase);
        l.push(TITLE, &[b"Bluetooth\t", match phase {
            READY if s.is_some_and(|s| s.scanning) => b"On, scanning".as_slice(),
            READY => b"On",
            NO_CONTROLLER => b"Off",
            STARTING => b"Starting",
            FAILED => b"Failed",
            _ => b"Unavailable",
        }]);
        let mut status_name = [0u8; 17];
        match (phase, self.notice) {
            (_, Some(o)) => {
                let n = name_of(&o.addr, &mut status_name);
                if o.ok { l.push(TEXT, &[b"Paired with ", &status_name[..n]]); }
                else if o.reason == REASON_UNSUPPORTED { l.push(TEXT, &[b"Cannot pair ", &status_name[..n], b": not supported"]); }
                else { l.push(TEXT, &[b"Pairing with ", &status_name[..n], b" failed"]); }
            }
            (NO_CONTROLLER, _) => { l.push(TEXT, &[b"No Bluetooth controller"]); }
            (STARTING, _) => { l.push(TEXT, &[b"Starting the controller"]); }
            (FAILED, _) => { l.push(TEXT, &[b"The controller did not start"]); }
            (UNAVAILABLE, _) => { l.push(TEXT, &[b"Bluetooth service is not running"]); }
            _ => {}
        }
        let mut selected_line = None;
        if let Some(s) = s.filter(|s| s.phase == READY) {
            if self.selected == 0 { selected_line = Some(l.count as usize); }
            l.push(BUTTON, &[if s.scanning { b"Stop scanning".as_slice() } else { b"Scan for devices" }]);
            let mut order = [0usize; MAX_DEVICES];
            let n = Self::order(s, &mut order).min(Self::MAX_SHOWN);
            if n == 0 { l.push(DIM, &[b"No devices found yet"]); }
            for (k, &i) in order[..n].iter().enumerate() {
                let d = &s.devices[i];
                let mut addr = [0u8; 17];
                addr_text(&d.addr, &mut addr);
                let name = if d.name_len == 0 { &addr[..] } else { d.name() };
                let state: &[u8] = if d.has(PAIRING) { b", pairing" } else if d.has(IN_USE) { b", in use" }
                    else if d.has(CONNECTED) { b", connected" } else if d.has(PAIRED) { b", paired" } else { b"" };
                let kind = kind_name(d.kind).as_bytes();
                // The name is cut so the right column always fits.
                let room = LINE_BYTES.saturating_sub(kind.len() + state.len() + 2);
                if self.selected == k + 1 { selected_line = Some(l.count as usize); }
                l.push(BUTTON, &[&name[..name.len().min(room)], b"\t", kind, state]);
            }
            l.push(DIM, &[b"Enter: pair or connect  Del: forget  Esc: close"]);
        } else {
            l.push(DIM, &[b"Esc: close"]);
        }
        (l, selected_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: [u8; 6] = [1, 2, 3, 4, 5, 6];
    const HP: [u8; 6] = [9, 9, 9, 9, 9, 9];

    fn ready() -> Snapshot {
        let mut s = Snapshot { phase: READY, ..Snapshot::EMPTY };
        assert!(s.put(KB, KEYBOARD, Some(b"Silk Keyboard"), Some(-40), true));
        assert!(s.put(HP, HEADPHONES, Some("Cuff Héadphones".as_bytes()), None, false));
        s
    }

    #[test]
    fn kinds_follow_class_and_appearance() {
        assert_eq!(kind_of(Some(0x002540), None), KEYBOARD);
        assert_eq!(kind_of(Some(0x002580), None), MOUSE);
        assert_eq!(kind_of(Some(0x0025C0), None), INPUT);
        assert_eq!(kind_of(Some(0x240404), None), HEADPHONES);
        assert_eq!(kind_of(Some(0x000404), None), OTHER); // audio class without the service bits
        assert_eq!(kind_of(Some(0x5A020C), None), OTHER); // a phone
        assert_eq!(kind_of(None, Some(0x03C1)), KEYBOARD);
        assert_eq!(kind_of(None, Some(0x03C2)), MOUSE);
        assert_eq!(kind_of(None, Some(0x0040)), OTHER);
        assert!(supported(HEADPHONES) && !supported(OTHER));
    }

    #[test]
    fn names_are_printable_and_bounded() {
        let s = ready();
        assert_eq!(s.find(&HP).unwrap().name(), b"Cuff H?adphones");
        let mut d = Device::new(KB);
        assert!(d.set_name(&[b'a'; 100]));
        assert_eq!(d.name().len(), NAME_MAX);
        assert!(!d.set_name(&[b'a'; 100]));
        assert!(!d.set_name(b"   "));
        let mut t = [0u8; 17];
        addr_text(&[0x66, 0x55, 0x44, 0x33, 0x22, 0x11], &mut t);
        assert_eq!(&t, b"11:22:33:44:55:66");
    }

    #[test]
    fn table_updates_evicts_and_round_trips() {
        let mut s = ready();
        assert!(!s.put(KB, OTHER, None, Some(-80), true), "rssi alone is not a change");
        assert!(s.put(KB, KEYBOARD, Some(b"Silk Keyboard 2"), None, true));
        assert!(s.set(&KB, PAIRED | CONNECTED, true) && !s.set(&KB, PAIRED, true));
        for i in 0..20u8 { s.put([i, 0, 0, 0, 0, 0x77], OTHER, None, None, false); }
        assert_eq!(s.count as usize, MAX_DEVICES);
        assert!(s.find(&KB).is_some(), "a paired device is never evicted");
        assert!(s.find(&HP).is_none(), "the oldest other device made room");
        s.compare = Some(Compare { ticket: 7, addr: KB, value: 42 });
        s.outcome = Some(Outcome { serial: 3, addr: HP, ok: false, reason: REASON_REFUSED });
        s.scanning = true;
        let mut b = [0u8; SNAPSHOT_BYTES];
        s.encode(&mut b);
        assert_eq!(Snapshot::decode(&b), Some(s));
        assert!(s.remove(&KB) && !s.remove(&KB));
        // Damage is refused, never trusted.
        let mut bad = b; bad[2] = 200; assert_eq!(Snapshot::decode(&bad), None);
        let mut bad = b; bad[0] = 9; assert_eq!(Snapshot::decode(&bad), None);
        let mut bad = b; bad[HEAD_BYTES + 9] = 200; assert_eq!(Snapshot::decode(&bad), None);
        let mut odd = b; odd[HEAD_BYTES + 10] = 0x07;
        assert_eq!(Snapshot::decode(&odd).unwrap().devices[0].name()[0], b'?');
    }

    #[test]
    fn commands_round_trip() {
        for c in [Command::Scan(true), Command::Scan(false), Command::Pair(KB), Command::Forget(HP), Command::Confirm { ticket: 0xDEAD_BEEF, yes: true }, Command::Confirm { ticket: 1, yes: false }] {
            assert_eq!(Command::decode(&c.encode()), Some(c));
        }
        assert_eq!(Command::decode(&[0; COMMAND_BYTES]), None);
        let mut b = Command::Scan(true).encode(); b[1] = 2; assert_eq!(Command::decode(&b), None);
    }

    #[test]
    fn region_snapshot_commands_and_attach() {
        let mut mem = vec![0u32; BYTES as usize / 4];
        let r = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert!(!r.available() && r.read().is_none() && !r.shell_attached());
        // Shell commands before Cufflinks starts are dropped.
        assert!(r.push_command(Command::Pair(KB)));
        let s = ready();
        r.cufflinks_start(&s);
        assert!(r.available());
        assert_eq!(r.take_command(), None);
        assert_eq!(r.read(), Some(s));
        r.shell_attach();
        assert!(r.shell_attached() && !r.shell_open());
        r.set_open(true);
        assert!(r.shell_open());
        for i in 0..COMMANDS { assert!(r.push_command(Command::Confirm { ticket: i, yes: true })); }
        assert!(!r.push_command(Command::Scan(true)), "full ring refuses");
        for i in 0..COMMANDS { assert_eq!(r.take_command(), Some(Command::Confirm { ticket: i, yes: true })); }
        assert_eq!(r.take_command(), None);
        // A torn write (odd sequence) is never read.
        let mut t = s; t.scanning = true;
        r.publish(&t);
        assert_eq!(r.read(), Some(t));
        let seq = r.load(W_SEQ);
        r.store(W_SEQ, seq | 1);
        assert_eq!(r.read(), None);
        r.store(W_SEQ, seq);
        // A writer that ran ahead of the ring is dropped, not replayed.
        r.store(W_WRITTEN, r.load(W_TAKEN).wrapping_add(50));
        assert_eq!(r.take_command(), None);
        assert!(r.push_command(Command::Scan(false)));
        assert_eq!(r.take_command(), Some(Command::Scan(false)));
    }

    fn show(ui: &Ui, s: Option<&Snapshot>) -> Lines {
        let (lines, selected) = ui.lines(s);
        // Through the wire as Display gets it.
        let mut a = Assembler::new();
        let mut got = None;
        let calls: Vec<_> = chunks(&lines, selected).collect();
        assert_eq!(calls.len(), lines.count as usize * PARTS);
        for (i, (w, x, y)) in calls.iter().enumerate() {
            let r = a.push(*w, *x, *y);
            assert_eq!(r.is_some(), i + 1 == calls.len());
            got = got.or(r);
        }
        let (back, sel) = got.unwrap();
        assert_eq!(sel, selected);
        for i in 0..lines.count as usize { assert_eq!(back.line(i), lines.line(i)); assert_eq!(back.style[i], lines.style[i]); }
        assert_eq!(back.count, lines.count);
        lines
    }
    fn text(l: &Lines) -> String { (0..l.count as usize).map(|i| String::from_utf8_lossy(l.line(i)).into_owned()).collect::<Vec<_>>().join("\n") }

    #[test]
    fn panel_says_no_controller_and_unavailable() {
        let ui = Ui { open: true, ..Ui::new() };
        let none = Snapshot { phase: NO_CONTROLLER, ..Snapshot::EMPTY };
        let t = text(&show(&ui, Some(&none)));
        assert!(t.contains("No Bluetooth controller") && t.starts_with("Bluetooth\tOff"), "{t}");
        assert!(text(&show(&ui, None)).contains("Bluetooth service is not running"));
        // Hiding is one chunk with no lines.
        let hide: Vec<_> = chunks(&Lines::EMPTY, None).collect();
        assert_eq!(hide.len(), 1);
        assert_eq!(Assembler::new().push(hide[0].0, 0, 0).unwrap().0.count, 0);
    }

    #[test]
    fn panel_lists_pairs_forgets_and_scans() {
        let mut s = ready();
        s.set(&HP, PAIRED, true);
        let mut ui = Ui::new();
        assert_eq!(ui.toggle(), Action::Redraw);
        let l = show(&ui, Some(&s));
        let t = text(&l);
        // Paired first.
        let hp = t.find("Cuff H?adphones\theadphones, paired").unwrap();
        let kb = t.find("Silk Keyboard\tkeyboard").unwrap();
        assert!(hp < kb, "{t}");
        assert_eq!(ui.key(key::ENTER, Some(&s)), Action::Send(Command::Scan(true)));
        assert_eq!(ui.key(key::DOWN, Some(&s)), Action::Redraw);
        assert_eq!(ui.key(key::DELETE, Some(&s)), Action::Send(Command::Forget(HP)));
        assert_eq!(ui.key(key::DOWN_USB, Some(&s)), Action::Redraw);
        assert_eq!(ui.key(key::DOWN, Some(&s)), Action::Redraw, "stays on the last row");
        assert_eq!(ui.selected, 2);
        assert_eq!(ui.key(key::DELETE, Some(&s)), Action::None, "an unpaired device has nothing to forget");
        assert_eq!(ui.key(key::ENTER, Some(&s)), Action::Send(Command::Pair(KB)));
        // Clicks land on the line drawn there.
        let (lines, _) = ui.lines(Some(&s));
        let p = Panel::new(1280, 28, 1, lines.count as usize).unwrap();
        let (x, y, _, _) = p.row(1);
        let line = p.row_at(x + 3, y + 3);
        assert_eq!(line, Some(1));
        assert_eq!(ui.click(line, Some(&s)), Action::Send(Command::Scan(true)));
        assert_eq!(ui.click(Some(2), Some(&s)), Action::Send(Command::Pair(HP)));
        assert_eq!(ui.selected, 1);
        assert_eq!(ui.click(Some(0), Some(&s)), Action::None, "the title is not a button");
        assert!(p.row_at(p.panel.0, p.panel.1).is_none() && p.inside(p.panel.0, p.panel.1));
        assert_eq!(ui.click(None, Some(&s)), Action::Close);
        assert!(!ui.open);
        s.scanning = true;
        ui.toggle();
        assert_eq!(ui.key(key::S, Some(&s)), Action::Send(Command::Scan(false)));
        assert!(text(&show(&ui, Some(&s))).contains("On, scanning"));
        assert_eq!(ui.key(key::ESC, Some(&s)), Action::Close);
    }

    #[test]
    fn comparison_dialog_opens_answers_once_and_follows_cufflinks() {
        let mut s = ready();
        let mut ui = Ui::new();
        assert!(!ui.update(Some(&s)));
        s.compare = Some(Compare { ticket: 5, addr: KB, value: 42 });
        s.set(&KB, PAIRING, true);
        assert!(ui.update(Some(&s)), "a comparison opens the panel");
        assert!(ui.open);
        let t = text(&show(&ui, Some(&s)));
        assert!(t.starts_with("Pair with Silk Keyboard?\nDoes it show 000042?"), "{t}");
        assert_eq!(ui.lines(Some(&s)).1, Some(2));
        assert!(!ui.update(Some(&s)), "the same ticket changes nothing");
        assert_eq!(ui.key(key::DOWN, Some(&s)), Action::Redraw);
        assert_eq!(ui.lines(Some(&s)).1, Some(3));
        assert_eq!(ui.click(None, Some(&s)), Action::None, "a question waits for its answer");
        assert_eq!(ui.key(key::ENTER, Some(&s)), Action::Send(Command::Confirm { ticket: 5, yes: false }));
        assert!(ui.dialog.is_none());
        assert!(!ui.update(Some(&s)), "an answered ticket is not asked again");
        // A new one: Y answers yes, from anywhere.
        s.compare = Some(Compare { ticket: 6, addr: HP, value: 999_999 });
        ui.open = false;
        assert!(ui.update(Some(&s)));
        assert!(text(&show(&ui, Some(&s))).contains("Does it show 999999?"));
        assert_eq!(ui.key(key::Y, Some(&s)), Action::Send(Command::Confirm { ticket: 6, yes: true }));
        // Cufflinks gave up (timeout): the dialog goes with it.
        s.compare = Some(Compare { ticket: 7, addr: KB, value: 1 });
        assert!(ui.update(Some(&s)));
        s.compare = None;
        s.outcome = Some(Outcome { serial: 1, addr: KB, ok: false, reason: REASON_TIMEOUT });
        assert!(ui.update(Some(&s)));
        assert!(ui.dialog.is_none());
        assert!(text(&show(&ui, Some(&s))).contains("Pairing with Silk Keyboard failed"));
        s.outcome = Some(Outcome { serial: 2, addr: KB, ok: true, reason: 0 });
        ui.update(Some(&s));
        assert!(text(&show(&ui, Some(&s))).contains("Paired with Silk Keyboard"));
    }

    #[test]
    fn chunks_are_checked() {
        let good = Chunk { line: 1, part: 2, style: BUTTON, count: 3, selected: Some(2), last: true };
        assert_eq!(Chunk::decode(good.word()), Some(good));
        assert_eq!(Chunk::decode(good.word() & !(0xFF << 56)), None, "untagged");
        assert_eq!(Chunk::decode(Chunk { line: 3, ..good }.word()), None, "line past count");
        assert_eq!(Chunk::decode(Chunk { part: 3, ..good }.word()), None);
        assert_eq!(Chunk::decode(Chunk { selected: Some(3), ..good }.word()), None);
        assert_eq!(Chunk::decode(Chunk { count: LINES as u8 + 1, line: 0, selected: None, ..good }.word()), None);
        // Control bytes from a sender become '?'.
        let mut a = Assembler::new();
        let w = Chunk { line: 0, part: 0, style: TEXT, count: 1, selected: None, last: false }.word();
        assert_eq!(a.push(w, u64::from_le_bytes(*b"ab\x07d\tefg"), 0), None);
        let w2 = Chunk { line: 0, part: 2, style: TEXT, count: 1, selected: None, last: true }.word();
        let (l, _) = a.push(w2, 0, 0).unwrap();
        assert_eq!(l.line(0), b"ab?d\tefg");
        assert_eq!(l.columns(0), (&b"ab?d"[..], &b"efg"[..]));
    }
}
