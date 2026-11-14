// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A terminal stream: a pair of byte rings in one shared piece of memory, the SexOS
//! answer to a pseudo-terminal. It is a protocol over memory the two sides already
//! share, not a kernel object: the kernel never sees it, and nothing here adds authority.
//!
//! The *host* is the terminal (Term): it writes what the person types into the input
//! ring and reads what the program prints from the output ring. The *guest* is the
//! program (a wasm guest on a Term worker today; a program given the region tomorrow):
//! it reads input and writes output. Each ring has one writer and one reader, so plain
//! atomic head and tail counters are enough; no locks, no syscalls.
//!
//! Besides bytes the header carries the window size in cells (the guest's `TIOCGWINSZ`),
//! a count of interrupts (Ctrl+C, which the guest may treat as a stop request), and
//! which side has hung up.
//!
//! Layout, little-endian `u32` words, then the two rings:
//!
//! | Offset | Field |
//! |---|---|
//! | 0 | magic `SXST` |
//! | 4 | version (1) |
//! | 8 | bytes in each ring (a power of two) |
//! | 12 | flags: host hung up, guest hung up |
//! | 16, 20 | columns, rows |
//! | 24, 28 | input ring head (bytes written), tail (bytes read) |
//! | 32, 36 | output ring head, tail |
//! | 40 | interrupts sent |
//! | 44 | size changes |
//! | 64 | input ring, then output ring |
use core::sync::atomic::{AtomicU32, Ordering::{Acquire, Release, Relaxed}};

pub const MAGIC: u32 = u32::from_le_bytes(*b"SXST");
pub const VERSION: u32 = 1;
pub const HEADER: usize = 64;
const HOST_GONE: u32 = 1;
const GUEST_GONE: u32 = 2;

/// Which end of the stream this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side { Host, Guest }

/// Bytes of memory a stream with `ring` bytes in each direction needs.
pub const fn bytes_needed(ring: usize) -> usize { HEADER + 2 * ring }

/// A view of a stream in memory both sides can reach.
#[derive(Clone, Copy)]
pub struct Stream { base: *mut u8, ring: u32 }
// The memory is shared on purpose; every field is reached through atomics or through
// ring bytes whose ownership the head and tail counters hand over.
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

impl Stream {
    /// Lays out a fresh stream in `len` bytes at `base`, with rings as large as fit
    /// (a power of two). The host does this once, before the guest attaches.
    ///
    /// # Safety
    /// `base..base+len` must be writable memory that stays valid, 4-byte aligned, for as
    /// long as any `Stream` over it is used.
    pub unsafe fn init(base: *mut u8, len: usize, cols: u32, rows: u32) -> Option<Stream> {
        if base.is_null() || base as usize % 4 != 0 || len < bytes_needed(16) { return None; }
        let mut ring = 1usize << (usize::BITS - 1 - ((len - HEADER) / 2).leading_zeros());
        ring = ring.min(1 << 30);
        let s = Stream { base, ring: ring as u32 };
        for off in (0..HEADER).step_by(4) { s.word(off).store(0, Relaxed); }
        s.word(4).store(VERSION, Relaxed);
        s.word(8).store(ring as u32, Relaxed);
        s.word(16).store(cols, Relaxed);
        s.word(20).store(rows, Relaxed);
        s.word(0).store(MAGIC, Release);
        Some(s)
    }
    /// A stream another side laid out. `None` if the memory does not hold one.
    ///
    /// # Safety
    /// As for [`Stream::init`].
    pub unsafe fn attach(base: *mut u8, len: usize) -> Option<Stream> {
        if base.is_null() || base as usize % 4 != 0 || len < HEADER { return None; }
        let probe = Stream { base, ring: 0 };
        if probe.word(0).load(Acquire) != MAGIC || probe.word(4).load(Relaxed) != VERSION { return None; }
        let ring = probe.word(8).load(Relaxed);
        if !ring.is_power_of_two() || bytes_needed(ring as usize) > len { return None; }
        Some(Stream { base, ring })
    }
    fn word(&self, off: usize) -> &AtomicU32 { unsafe { &*(self.base.add(off) as *const AtomicU32) } }
    /// (head, tail, ring offset) of the ring this side writes, or reads.
    fn ring_of(&self, side: Side, writing: bool) -> (usize, usize, usize) {
        let input = (side == Side::Host) == writing;
        if input { (24, 28, HEADER) } else { (32, 36, HEADER + self.ring as usize) }
    }

    /// Copies as much of `data` as fits; returns how many bytes went. Zero once the
    /// other side has hung up, so a writer never waits on a reader that is gone.
    pub fn write(&self, side: Side, data: &[u8]) -> usize {
        if self.gone(other(side)) { return 0; }
        let (h, t, at) = self.ring_of(side, true);
        let head = self.word(h).load(Relaxed);
        let tail = self.word(t).load(Acquire);
        let free = self.ring - head.wrapping_sub(tail);
        let n = (data.len() as u32).min(free);
        let mask = self.ring - 1;
        for (i, b) in data[..n as usize].iter().enumerate() {
            let pos = (head.wrapping_add(i as u32) & mask) as usize;
            unsafe { self.base.add(at + pos).write_volatile(*b) };
        }
        self.word(h).store(head.wrapping_add(n), Release);
        n as usize
    }
    /// Copies waiting bytes into `out`; returns how many.
    pub fn read(&self, side: Side, out: &mut [u8]) -> usize {
        let (h, t, at) = self.ring_of(side, false);
        let tail = self.word(t).load(Relaxed);
        let head = self.word(h).load(Acquire);
        let n = head.wrapping_sub(tail).min(out.len() as u32);
        let mask = self.ring - 1;
        for (i, b) in out[..n as usize].iter_mut().enumerate() {
            let pos = (tail.wrapping_add(i as u32) & mask) as usize;
            *b = unsafe { self.base.add(at + pos).read_volatile() };
        }
        self.word(t).store(tail.wrapping_add(n), Release);
        n as usize
    }
    /// Bytes waiting for this side to read.
    pub fn readable(&self, side: Side) -> usize {
        let (h, t, _) = self.ring_of(side, false);
        self.word(h).load(Acquire).wrapping_sub(self.word(t).load(Relaxed)) as usize
    }
    /// Room this side has to write.
    pub fn writable(&self, side: Side) -> usize {
        let (h, t, _) = self.ring_of(side, true);
        (self.ring - self.word(h).load(Relaxed).wrapping_sub(self.word(t).load(Acquire))) as usize
    }
    /// This side is done: the guest exited, or the person closed the terminal.
    pub fn hang_up(&self, side: Side) { self.word(12).fetch_or(flag(side), Release); }
    pub fn gone(&self, side: Side) -> bool { self.word(12).load(Acquire) & flag(side) != 0 }

    /// The host's window changed size, in cells.
    pub fn set_size(&self, cols: u32, rows: u32) {
        self.word(16).store(cols, Relaxed);
        self.word(20).store(rows, Relaxed);
        self.word(44).fetch_add(1, Release);
    }
    pub fn size(&self) -> (u32, u32) { (self.word(16).load(Relaxed), self.word(20).load(Relaxed)) }
    /// How many times the size changed; a guest compares it with what it saw last.
    pub fn size_changes(&self) -> u32 { self.word(44).load(Acquire) }
    /// The person pressed Ctrl+C. The byte 3 also goes into the input ring; this count
    /// reaches a guest that is busy and not reading.
    pub fn interrupt(&self) { self.word(40).fetch_add(1, Release); }
    pub fn interrupts(&self) -> u32 { self.word(40).load(Acquire) }
}

const fn other(side: Side) -> Side { match side { Side::Host => Side::Guest, Side::Guest => Side::Host } }
const fn flag(side: Side) -> u32 { match side { Side::Host => HOST_GONE, Side::Guest => GUEST_GONE } }

// Host tests: crates/sex-pdx/tests/stream.rs (an echo program across threads).
