// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bookkeeping for many NVMe commands in flight on one I/O queue pair.
//!
//! A request longer than one command goes to the drive as several commands at
//! once: as many SQEs as fit, one doorbell, then completions reaped in whatever
//! order the drive finishes them. This module holds only the arithmetic: ring
//! indices and phase, which command IDs are in flight and which piece of the
//! request each one carries, and when the submission queue is full. It touches
//! no memory the device sees, so the rules run on the host
//! (tests/sexdrive_ioq.rs). DMA custody stays with `dma_transfer`: a buffer is
//! reused only after the CQE carrying its own CID and the expected phase.

/// I/O queue entries we ask for (bounded by CAP.MQES). 64 SQEs of 64 bytes are
/// exactly one 4 KiB page, so the queue still fits the page it always had.
pub const QUEUE_ENTRIES: u32 = 64;
/// Most commands one request keeps in flight.
pub const MAX_IN_FLIGHT: usize = 32;

/// Entries for the I/O queue pair: QUEUE_ENTRIES, or fewer when CAP.MQES
/// (0-based) says so. The spec's minimum queue is 2 entries.
pub fn queue_entries(mqes: u16) -> u32 {
    (mqes as u32 + 1).clamp(2, QUEUE_ENTRIES)
}

/// Status field of a CQE (DW3 bits 31:17); 0 is success.
pub fn status(dw3: u32) -> u32 { (dw3 >> 17) & 0x7FFF }
pub fn cid(dw3: u32) -> u16 { (dw3 & 0xFFFF) as u16 }
pub fn phase(dw3: u32) -> u32 { (dw3 >> 16) & 1 }

/// How many commands of at most `run` bytes carry `size` bytes.
pub fn pieces(size: u64, run: u64) -> u32 {
    if run == 0 { return 0; }
    size.div_ceil(run) as u32
}

/// Byte offset and length of piece `i`.
pub fn piece(i: u32, size: u64, run: u64) -> (u64, u64) {
    let at = i as u64 * run;
    (at, size.saturating_sub(at).min(run))
}

/// One SQ/CQ pair's indices. The SQ is full when one more entry would make
/// tail equal head; the device reports how far it has read in each CQE (SQHD).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring {
    pub entries: u32,
    pub sq_tail: u32,
    pub sq_head: u32,
    pub cq_head: u32,
    pub phase: u32,
}

impl Ring {
    /// A queue with nothing outstanding: everything up to the tail was consumed.
    pub fn idle(entries: u32, sq_tail: u32, cq_head: u32, phase: u32) -> Self {
        Ring { entries, sq_tail, sq_head: sq_tail, cq_head, phase }
    }
    pub fn sq_used(&self) -> u32 { (self.sq_tail + self.entries - self.sq_head) % self.entries }
    pub fn sq_free(&self) -> u32 { self.entries - 1 - self.sq_used() }
    /// The SQ index for one more command, or None when the queue is full.
    pub fn push(&mut self) -> Option<u32> {
        if self.sq_free() == 0 { return None; }
        let at = self.sq_tail;
        self.sq_tail = (self.sq_tail + 1) % self.entries;
        Some(at)
    }
    /// Is the CQE at the head new (its phase is the one we expect)?
    pub fn fresh(&self, dw3: u32) -> bool { phase(dw3) == self.phase }
    /// Consume the CQE at the head: the head moves on, the phase flips when it
    /// wraps, and the device's SQ head is taken when it lies between our head
    /// and tail (a value outside that span is ignored, which only keeps the
    /// queue fuller than it is).
    pub fn consume(&mut self, dw2: u32) {
        let sqhd = dw2 & 0xFFFF;
        if sqhd < self.entries && (sqhd + self.entries - self.sq_head) % self.entries <= self.sq_used() {
            self.sq_head = sqhd;
        }
        self.cq_head = (self.cq_head + 1) % self.entries;
        if self.cq_head == 0 { self.phase ^= 1; }
    }
}

/// Command IDs in flight, each in a slot that also names its DMA buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InFlight {
    cids: [u16; MAX_IN_FLIGHT],
    pieces: [u32; MAX_IN_FLIGHT],
    used: u32,
    limit: usize,
}

impl InFlight {
    pub fn new(limit: usize) -> Self {
        InFlight { cids: [0; MAX_IN_FLIGHT], pieces: [0; MAX_IN_FLIGHT], used: 0, limit: limit.min(MAX_IN_FLIGHT) }
    }
    pub fn len(&self) -> usize { self.used.count_ones() as usize }
    pub fn is_empty(&self) -> bool { self.used == 0 }
    pub fn limit(&self) -> usize { self.limit }
    /// The lowest free slot below the limit.
    pub fn free_slot(&self) -> Option<usize> {
        (0..self.limit).find(|&slot| self.used & (1 << slot) == 0)
    }
    pub fn find(&self, cid: u16) -> Option<usize> {
        (0..self.limit).find(|&slot| self.used & (1 << slot) != 0 && self.cids[slot] == cid)
    }
    /// Refuses a used slot, one past the limit, or a CID already in flight.
    pub fn insert(&mut self, slot: usize, cid: u16, piece: u32) -> bool {
        if slot >= self.limit || self.used & (1 << slot) != 0 || self.find(cid).is_some() { return false; }
        self.cids[slot] = cid;
        self.pieces[slot] = piece;
        self.used |= 1 << slot;
        true
    }
    /// The CID and piece of a used slot, which becomes free.
    pub fn take(&mut self, slot: usize) -> Option<(u16, u32)> {
        if slot >= self.limit || self.used & (1 << slot) == 0 { return None; }
        self.used &= !(1 << slot);
        Some((self.cids[slot], self.pieces[slot]))
    }
}

/// What one reaped CQE settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Done {
    pub slot: usize,
    pub cid: u16,
    pub piece: u32,
    pub status: u32,
    /// The phase the CQE carried (the one expected before it was consumed).
    pub phase: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The CQE at the head still carries the old phase: nothing new there.
    Stale,
    /// A new CQE for a command this request did not submit.
    Foreign(u16),
}

/// One request cut into `pieces` commands, at most `limit` in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub ring: Ring,
    pub flight: InFlight,
    pieces: u32,
    next: u32,
    stopped: bool,
    errors: u32,
    first_error: Option<(u16, u32)>,
    pub most_in_flight: usize,
}

impl Batch {
    pub fn new(ring: Ring, pieces: u32, limit: usize) -> Self {
        Batch { ring, flight: InFlight::new(limit), pieces, next: 0, stopped: false, errors: 0, first_error: None, most_in_flight: 0 }
    }
    /// The next piece and the slot it goes in, when the request has one left,
    /// a slot is free and the SQ has room. Nothing changes until `submitted`.
    pub fn next_submit(&self) -> Option<(u32, usize)> {
        if self.stopped || self.next >= self.pieces || self.ring.sq_free() == 0 { return None; }
        Some((self.next, self.flight.free_slot()?))
    }
    /// Record the SQE written for `next_submit`'s answer; returns its SQ index.
    pub fn submitted(&mut self, slot: usize, cid: u16) -> Option<u32> {
        if self.stopped || self.next >= self.pieces || self.ring.sq_free() == 0 { return None; }
        if !self.flight.insert(slot, cid, self.next) { return None; }
        let at = self.ring.push()?;
        self.next += 1;
        self.most_in_flight = self.most_in_flight.max(self.flight.len());
        Some(at)
    }
    /// No more submissions; what is in flight is still reaped.
    pub fn stop(&mut self) { self.stopped = true; }
    pub fn accepts(&self, cid: u16) -> bool { self.flight.find(cid).is_some() }
    /// Reap the CQE at the head. A failed command stops further submissions
    /// (its data is not trusted and the queue will be stopped), but the others
    /// still in flight are reaped: their buffers are only free after their CQEs.
    pub fn complete(&mut self, dw2: u32, dw3: u32) -> Result<Done, Refused> {
        if !self.ring.fresh(dw3) { return Err(Refused::Stale); }
        let cid = cid(dw3);
        let slot = self.flight.find(cid).ok_or(Refused::Foreign(cid))?;
        let (_, piece) = self.flight.take(slot).ok_or(Refused::Foreign(cid))?;
        let phase = self.ring.phase;
        self.ring.consume(dw2);
        // Nothing left in flight means the device read every SQE we wrote.
        if self.flight.is_empty() { self.ring.sq_head = self.ring.sq_tail; }
        let status = status(dw3);
        if status != 0 {
            self.errors += 1;
            if self.first_error.is_none() { self.first_error = Some((cid, status)); }
            self.stopped = true;
        }
        Ok(Done { slot, cid, piece, status, phase })
    }
    pub fn finished(&self) -> bool { self.flight.is_empty() && (self.stopped || self.next >= self.pieces) }
    /// Every piece went out and came back without an error.
    pub fn succeeded(&self) -> bool { self.finished() && !self.stopped && self.errors == 0 && self.next == self.pieces }
    pub fn errors(&self) -> u32 { self.errors }
    pub fn first_error(&self) -> Option<(u16, u32)> { self.first_error }
    pub fn submitted_pieces(&self) -> u32 { self.next }
}
