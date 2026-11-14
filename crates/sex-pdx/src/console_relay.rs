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
/// The attached run's state, at the start of TAKE's data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status { pub state: u8, pub code: u32, pub target: u32 }
impl Status {
    pub fn encode(self, out: &mut [u8]) {
        out[..STATUS_BYTES].fill(0);
        out[0] = self.state;
        out[4..8].copy_from_slice(&self.code.to_le_bytes());
        out[8..12].copy_from_slice(&self.target.to_le_bytes());
    }
    pub fn decode(d: &[u8]) -> Status {
        if d.len() < STATUS_BYTES { return Status::default(); }
        Status { state: d[0], code: u32::from_le_bytes(d[4..8].try_into().unwrap()), target: u32::from_le_bytes(d[8..12].try_into().unwrap()) }
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
