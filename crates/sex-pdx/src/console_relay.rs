// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A program's console attached to a terminal program: one more request kind on the
//! Files protocol (`fs`), with the operation in `offset` and bytes in the data area.
//!
//! The person opens a terminal window with Spindle's `spindle` command (`run +launch term`): its run then
//! holds [`GRANT_TOKEN`], which only Spindle can put in a run's arguments. Such a run may
//! PREPARE `/bin/NAME` with arguments, as Spindle does; Files drops any `+` grant or `@`
//! path the terminal put among them, and the run gets no network. Shell starts the ticket
//! only for the program that prepared it. Files then pairs the new run with the terminal:
//! the run's console goes through Files to the terminal and never opens a window.
//!
//! Files keeps the bytes (an input ring and an output ring per terminal), so neither side
//! maps the other's memory and no kernel API is involved. A full ring takes fewer bytes,
//! which holds a fast writer back; both sides poll.
//!
//! A terminal holds up to [`SLOTS`] attached runs at once (one per tab or pane): the
//! terminal's requests carry the slot in `offset` above the operation ([`at_slot`]), and
//! its PREPARE carries it the same way, so slot 0 is what a caller that never names one
//! gets. An attached run never names a slot: Files knows which one it is.
//!
//! Beyond bytes, the run and its terminal share the window's size in cells (SET_SIZE from
//! the terminal, SIZE from the run, with a count of changes a program polls to notice a
//! resize) and the run's terminal mode (SET_MODE from the run, in TAKE's [`Status`]):
//! cooked or raw, echo on or off, Ctrl+C an interrupt or a byte. Ctrl+C reaches a run
//! the next time it asks Files anything about its console (READ, WRITE or POLL); a run
//! that computes without asking is stopped by the terminal's second Ctrl+C through Shell
//! ([`crate::program_store::OP_STOP_ATTACHED`]), never by a kernel signal.

/// The request kind (`fs::Request::kind`), next to `package::KIND`.
pub const KIND: u8 = 41;
/// The run argument Spindle adds for `spindle`: this run may start programs attached to it.
pub const GRANT_TOKEN: &[u8] = b"+launch";

// Operations, in `offset`. The reply's value is a byte count unless said otherwise.
/// Attached run: is this run's console attached? OK if so, `UNSUPPORTED` if not.
pub const ATTACHED: u64 = 0;
/// Attached run: `length` bytes of output from the data area; the value is how many were
/// taken (fewer when the terminal is behind). `UNSUPPORTED` once the terminal is gone.
pub const WRITE: u64 = 1;
/// Attached run: up to `length` bytes of input into the data area. Status [`INTERRUPTED`]
/// when the person pressed Ctrl+C since the last call, [`HUNG_UP`] once the terminal closed
/// its input (Ctrl+D on an empty line) or went away.
pub const READ: u64 = 2;
/// Attached run: it ends, with `length` as its exit status.
pub const EXIT: u64 = 3;
/// Terminal: `length` bytes of input for the attached run, from the data area.
pub const PUT: u64 = 4;
/// Terminal: up to `length` bytes of the run's output into the data area. The data area
/// starts with a [`Status`] (`STATUS_BYTES`), the bytes follow; the value counts the bytes.
pub const TAKE: u64 = 5;
/// Terminal: Ctrl+C. The run's next READ or WRITE answers [`INTERRUPTED`].
pub const INTERRUPT: u64 = 6;
/// Terminal: the end of input (Ctrl+D). Reads answer [`HUNG_UP`] once input runs dry.
pub const HANG_UP: u64 = 7;
/// Terminal: forget the run that ended (or never started), so another can be prepared.
pub const RELEASE: u64 = 8;

/// Attached run: the terminal's size into the data area, as a [`Size`] ([`SIZE_BYTES`]).
/// A size of 0 by 0: the terminal has not said yet.
pub const SIZE: u64 = 9;
/// Attached run: its terminal mode becomes `length` ([`RAW`], [`NO_ECHO`],
/// [`NO_SIGNALS`]); the value is the mode before.
pub const SET_MODE: u64 = 10;
/// Attached run: a look without taking anything. The value is how many input bytes wait;
/// status [`INTERRUPTED`] (once) after Ctrl+C, [`HUNG_UP`] at the end of input.
pub const POLL: u64 = 11;
/// Terminal: its size in cells, `length` = [`size_word`]`(cols, rows)`. It stays across
/// the runs of that slot.
pub const SET_SIZE: u64 = 12;

/// Mode bits (SET_MODE, [`Status::mode`]). 0 is a cooked line with echo, as a terminal
/// starts: the terminal edits the line and sends it on Enter.
/// Keys go to the run as they are typed, encoded as a terminal sends them; no local editing.
pub const RAW: u8 = 1;
/// The terminal does not show what is typed (passwords; or the run echoes itself).
pub const NO_ECHO: u8 = 2;
/// Ctrl+C is the byte 0x03 for the run, not an interrupt (an editor's own Ctrl+C).
pub const NO_SIGNALS: u8 = 4;
pub const MODES: u8 = RAW | NO_ECHO | NO_SIGNALS;

/// Attached runs per terminal: one per tab or pane.
pub const SLOTS: usize = 8;
/// Where the slot sits in a terminal request's `offset` (and in its PREPARE's).
pub const SLOT_SHIFT: u32 = 8;
/// A terminal's request `op` for its run in `slot`. Slot 0 is `op` itself.
pub const fn at_slot(op: u64, slot: usize) -> u64 { op | (slot as u64) << SLOT_SHIFT }
/// The operation and slot of a request's `offset`; None for a slot past [`SLOTS`] or
/// bits beyond it.
pub const fn split(offset: u64) -> Option<(u64, usize)> {
    let slot = offset >> SLOT_SHIFT;
    if slot >= SLOTS as u64 { return None; }
    Some((offset & ((1 << SLOT_SHIFT) - 1), slot as usize))
}
/// SET_SIZE's `length`.
pub const fn size_word(cols: u16, rows: u16) -> u64 { cols as u64 | (rows as u64) << 16 }
/// The columns and rows of SET_SIZE's `length`; None beyond 32 bits.
pub const fn split_size(word: u64) -> Option<(u16, u16)> {
    if word >> 32 != 0 { return None; }
    Some((word as u16, (word >> 16) as u16))
}

pub const SIZE_BYTES: usize = 8;
/// The terminal's size as SIZE reports it, and how many times it has changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size { pub cols: u16, pub rows: u16, pub changes: u32 }
impl Size {
    pub fn encode(self, out: &mut [u8]) {
        out[0..2].copy_from_slice(&self.cols.to_le_bytes());
        out[2..4].copy_from_slice(&self.rows.to_le_bytes());
        out[4..8].copy_from_slice(&self.changes.to_le_bytes());
    }
    pub fn decode(d: &[u8]) -> Size {
        if d.len() < SIZE_BYTES { return Size::default(); }
        Size { cols: u16::from_le_bytes([d[0], d[1]]), rows: u16::from_le_bytes([d[2], d[3]]), changes: u32::from_le_bytes(d[4..8].try_into().unwrap()) }
    }
    /// The size it becomes: the change count moves only when it changes.
    pub fn set(&mut self, cols: u16, rows: u16) {
        if (cols, rows) != (self.cols, self.rows) { self.cols = cols; self.rows = rows; self.changes = self.changes.wrapping_add(1); }
    }
    pub fn known(self) -> bool { self.cols != 0 && self.rows != 0 }
}

/// Reply statuses beyond the `fs` ones.
pub const INTERRUPTED: u8 = 60;
pub const HUNG_UP: u8 = 61;

/// The input ring per terminal.
pub const INPUT_BYTES: usize = 4096;
/// The output ring per terminal.
pub const OUTPUT_BYTES: usize = 32 * 1024;

/// Where the attached run is, as TAKE reports it.
pub const NONE: u8 = 0;
/// Prepared: Shell has not started it yet.
pub const STARTING: u8 = 1;
pub const RUNNING: u8 = 2;
/// It ended: `code` is its exit status ([`CRASHED`] when it never said).
pub const ENDED: u8 = 3;
pub const CRASHED: u32 = 0xFFFF;
/// It never started: no such program, or nothing could run it.
pub const NOT_STARTED: u32 = 0xFFFE;

pub const STATUS_BYTES: usize = 16;
/// The attached run's state, at the start of TAKE's data: byte 0 the state, byte 1 the
/// mode (it was 0 before modes existed, which is cooked with echo), bytes 4..8 the code,
/// 8..12 the run's target, 12..16 its task (0 before it runs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status { pub state: u8, pub mode: u8, pub code: u32, pub target: u32, pub task: u32 }
impl Status {
    pub fn encode(self, out: &mut [u8]) {
        out[..STATUS_BYTES].fill(0);
        out[0] = self.state;
        out[1] = self.mode;
        out[4..8].copy_from_slice(&self.code.to_le_bytes());
        out[8..12].copy_from_slice(&self.target.to_le_bytes());
        out[12..16].copy_from_slice(&self.task.to_le_bytes());
    }
    pub fn decode(d: &[u8]) -> Status {
        if d.len() < STATUS_BYTES { return Status::default(); }
        let word = |i: usize| u32::from_le_bytes(d[i..i + 4].try_into().unwrap());
        Status { state: d[0], mode: d[1] & MODES, code: word(4), target: word(8), task: word(12) }
    }
}

/// The arguments a terminal may give a run: its own words, without grants (`+…`) or
/// granted paths (`@…`), which only Spindle may give. Returns the length kept in `out`.
pub fn plain_args(args: &[u8], out: &mut [u8]) -> usize {
    let mut n = 0;
    for word in args.split(|b| *b == b' ').filter(|w| !w.is_empty() && w[0] != b'+' && w[0] != b'@') {
        let sep = (n > 0) as usize;
        if n + sep + word.len() > out.len() { break; }
        if sep == 1 { out[n] = b' '; }
        out[n + sep..n + sep + word.len()].copy_from_slice(word);
        n += sep + word.len();
    }
    n
}

/// Files' rings, shared by every terminal's slots: a run attached anywhere holds one
/// pair from PREPARE until RELEASE. There are only as many pairs as program rows, since
/// no more runs than that can be attached at once, so the slots cost no ring memory.
/// This keeps which pairs are taken; `N` is at most 64.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pool<const N: usize> { taken: u64 }
impl<const N: usize> Pool<N> {
    pub const fn new() -> Self { Pool { taken: 0 } }
    /// A free pair, now taken; None when all are.
    pub fn take(&mut self) -> Option<usize> {
        let i = (0..N).find(|i| self.taken & 1 << i == 0)?;
        self.taken |= 1 << i;
        Some(i)
    }
    pub fn give(&mut self, i: usize) { if i < N { self.taken &= !(1 << i); } }
    pub fn free(&self) -> usize { N - self.taken.count_ones() as usize }
}

/// A byte ring, as Files keeps one per direction.
pub struct Ring<const N: usize> { buf: [u8; N], head: usize, len: usize }
impl<const N: usize> Ring<N> {
    pub const fn new() -> Self { Ring { buf: [0; N], head: 0, len: 0 } }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn clear(&mut self) { self.head = 0; self.len = 0; }
    /// As many of `bytes` as fit; returns how many.
    pub fn push(&mut self, bytes: &[u8]) -> usize {
        let n = bytes.len().min(N - self.len);
        for (i, b) in bytes[..n].iter().enumerate() { self.buf[(self.head + self.len + i) % N] = *b; }
        self.len += n;
        n
    }
    /// Up to `out.len()` bytes, oldest first; returns how many.
    pub fn pop(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len);
        for (i, o) in out[..n].iter_mut().enumerate() { *o = self.buf[(self.head + i) % N]; }
        self.head = (self.head + n) % N;
        self.len -= n;
        n
    }
}
impl<const N: usize> Default for Ring<N> { fn default() -> Self { Self::new() } }
