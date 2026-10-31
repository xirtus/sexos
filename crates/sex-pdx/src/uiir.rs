// SPDX-License-Identifier: GPL-3.0-only
//! UiIR: one BOOT-granted, single-producer/single-consumer data stream.
//!
//! UiIR is a transport, not IPC and not authority. BOOT pairs two exact
//! domains with two regions: a record ring the producer writes and the
//! consumer may only read, and a cursor page the consumer writes and the
//! producer may only read. Nothing is minted at runtime.
//!
//! Ordering and visibility:
//! - Every record carries the producer's per-sender sequence (1, 2, 3, ...)
//!   and the epoch it belongs to. Sequences are contiguous and epochs never
//!   decrease; a consumer rejects anything else.
//! - Records become visible only when the producer commits an epoch, which
//!   publishes the committed sequence with release ordering. A consumer never
//!   observes part of an uncommitted epoch.
//! - The producer never overwrites a record the consumer has not consumed.

use core::sync::atomic::{AtomicU64, Ordering};

pub const MAGIC: u64 = 0x5249_4975_5845_5300; // "\0SEXuIIR"
pub const VERSION: u64 = 1;
pub const RING_BYTES: usize = 4096;
pub const CURSOR_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 64;
pub const RECORD_BYTES: usize = 32;
pub const CAPACITY: u64 = ((RING_BYTES - HEADER_BYTES) / RECORD_BYTES) as u64;

/// The first BOOT stream: Shell (producer) to Display (consumer). Each side
/// holds the ring and the cursor at the same two slots.
pub const SHELL_PD: u32 = 3;
pub const DISPLAY_PD: u32 = 1;
pub const RING_SLOT: u64 = 96;
pub const CURSOR_SLOT: u64 = 97;
pub const RING_REGION_ID: u32 = 9;
pub const CURSOR_REGION_ID: u32 = 10;
/// Record kinds on the Shell → Display stream.
pub const KIND_FOCUS: u64 = 1;

/// Ring header, written only by the producer.
#[repr(C, align(64))]
pub struct Header {
    pub magic: AtomicU64,
    pub version: AtomicU64,
    /// Highest committed sequence; records up to it are visible.
    pub committed: AtomicU64,
    /// Epoch of the last commit.
    pub epoch: AtomicU64,
    _reserved: [u64; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub sequence: u64,
    pub epoch: u64,
    pub kind: u64,
    pub payload: u64,
}

/// Consumer cursor page, written only by the consumer.
#[repr(C, align(64))]
pub struct Cursor {
    pub consumed: AtomicU64,
}

const _: () = assert!(core::mem::size_of::<Header>() == HEADER_BYTES);
const _: () = assert!(core::mem::size_of::<Record>() == RECORD_BYTES);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The consumer has not consumed enough to free a slot.
    Full,
    /// Epochs never move backwards.
    EpochRegressed,
    /// The ring is not an initialized stream of this version.
    Uninitialized,
    /// A record's sequence or epoch breaks the stream.
    Order { expected: u64, got: u64 },
    /// Sequence space exhausted.
    Exhausted,
}

/// Producer side. `ring` is the producer's writable view of the ring
/// region; `cursor` its read-only view of the consumer's cursor page.
pub struct Producer {
    header: *const Header,
    records: *mut Record,
    cursor: *const Cursor,
    next: u64,
    epoch: u64,
}

impl Producer {
    /// # Safety
    /// `ring` must be a writable, exclusively produced RING_BYTES mapping
    /// and `cursor` a readable CURSOR_BYTES mapping, both live for the
    /// producer's lifetime. BOOT zeroes both before granting them.
    pub unsafe fn attach(ring: *mut u8, cursor: *const u8) -> Self {
        let header = ring as *const Header;
        (*header).version.store(VERSION, Ordering::Relaxed);
        (*header).magic.store(MAGIC, Ordering::Release);
        Self { header, records: ring.add(HEADER_BYTES) as *mut Record, cursor: cursor as *const Cursor,
            next: (*header).committed.load(Ordering::Relaxed) + 1, epoch: (*header).epoch.load(Ordering::Relaxed) }
    }

    /// Stage one record in the current (uncommitted) epoch.
    pub fn push(&mut self, kind: u64, payload: u64) -> Result<u64, Error> {
        let consumed = unsafe { (*self.cursor).consumed.load(Ordering::Acquire) };
        if self.next.checked_add(1).is_none() { return Err(Error::Exhausted); }
        // A consumer cursor beyond what was ever committed is not trusted.
        let committed = unsafe { (*self.header).committed.load(Ordering::Relaxed) };
        let consumed = consumed.min(committed);
        if self.next - consumed > CAPACITY { return Err(Error::Full); }
        let record = Record { sequence: self.next, epoch: self.epoch + 1, kind, payload };
        unsafe { self.records.add(((self.next - 1) % CAPACITY) as usize).write_volatile(record); }
        self.next += 1;
        Ok(record.sequence)
    }

    /// Make every staged record visible as one epoch.
    pub fn commit(&mut self) -> u64 {
        let last = self.next - 1;
        let committed = unsafe { (*self.header).committed.load(Ordering::Relaxed) };
        if last == committed { return self.epoch; }
        self.epoch += 1;
        unsafe {
            (*self.header).epoch.store(self.epoch, Ordering::Relaxed);
            (*self.header).committed.store(last, Ordering::Release);
        }
        self.epoch
    }

    pub fn staged(&self) -> u64 {
        self.next - 1 - unsafe { (*self.header).committed.load(Ordering::Relaxed) }
    }
}

/// Consumer side. `ring` is the consumer's read-only view of the ring;
/// `cursor` its writable view of its own cursor page.
pub struct Consumer {
    header: *const Header,
    records: *const Record,
    cursor: *const Cursor,
    expected: u64,
    epoch: u64,
}

impl Consumer {
    /// # Safety
    /// `ring` must be a readable RING_BYTES mapping of the paired producer's
    /// ring, and `cursor` this consumer's writable CURSOR_BYTES page.
    pub unsafe fn attach(ring: *const u8, cursor: *mut u8) -> Self {
        let cursor = cursor as *const Cursor;
        Self { header: ring as *const Header, records: ring.add(HEADER_BYTES) as *const Record, cursor,
            expected: (*cursor).consumed.load(Ordering::Relaxed) + 1, epoch: 0 }
    }

    /// The next committed record, in order. A record that breaks sequence or
    /// epoch order is reported and not consumed.
    pub fn poll(&mut self) -> Result<Option<Record>, Error> {
        let header = unsafe { &*self.header };
        if header.magic.load(Ordering::Acquire) != MAGIC { return Ok(None); }
        if header.version.load(Ordering::Relaxed) != VERSION { return Err(Error::Uninitialized); }
        let committed = header.committed.load(Ordering::Acquire);
        if self.expected > committed { return Ok(None); }
        let record = unsafe { self.records.add(((self.expected - 1) % CAPACITY) as usize).read_volatile() };
        if record.sequence != self.expected { return Err(Error::Order { expected: self.expected, got: record.sequence }); }
        if record.epoch < self.epoch { return Err(Error::EpochRegressed); }
        self.epoch = record.epoch;
        unsafe { (*self.cursor).consumed.store(self.expected, Ordering::Release); }
        self.expected += 1;
        Ok(Some(record))
    }

    pub fn consumed(&self) -> u64 { self.expected - 1 }
    pub fn epoch(&self) -> u64 { self.epoch }
}
