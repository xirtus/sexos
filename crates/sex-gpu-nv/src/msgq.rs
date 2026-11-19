// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The shared-memory RPC queues between the driver and GSP-RM.
//!
//! One contiguous buffer: a page table of its own pages, the command queue
//! (driver to GSP, 256 KiB) and the message queue (GSP to driver, 256 KiB).
//! Each queue's first page is its header (`msgqTxHeader`, then the other
//! side's `msgqRxHeader` read pointer); entries are 4 KiB pages. A message is
//! a 48-byte element header (checksum, sequence, page count) followed by a
//! 32-byte RPC header and the payload, possibly wrapping around the end.

use crate::{wr32, wr64, Dma, Error};

/// Shared memory changes under us (GSP-RM writes it by DMA), so every read
/// of queue state goes through a volatile access the compiler cannot cache.
fn rd32(s: &[u8], o: usize) -> Option<u32> {
    if o.checked_add(4)? > s.len() { return None; }
    // SAFETY: bounds checked above; unaligned volatile read of plain bytes.
    Some(u32::from_le_bytes(unsafe { core::ptr::read_volatile(s.as_ptr().add(o) as *const [u8; 4]) }))
}
fn rd8v(s: &[u8], o: usize) -> u8 {
    // SAFETY: callers index within `s` (checked by the slice index below first).
    let _ = s[o];
    unsafe { core::ptr::read_volatile(s.as_ptr().add(o)) }
}
fn wr32v(s: &mut [u8], o: usize, v: u32) {
    let _ = &s[o..o + 4];
    // SAFETY: bounds checked by the slice expression above.
    unsafe { core::ptr::write_volatile(s.as_mut_ptr().add(o) as *mut [u8; 4], v.to_le_bytes()) }
}

pub const PAGE: usize = 0x1000;
pub const QUEUE_SIZE: usize = 0x4_0000;
/// Bytes before the RPC header in each element.
pub const ELEM_HDR: usize = 48;
pub const RPC_HDR: usize = 32;
/// GSP-RM accepts messages of up to 16 pages; bigger ones need continuation records.
pub const MAX_MSG: usize = PAGE * 16;
pub const RPC_SIGNATURE: u32 = u32::from_le_bytes(*b"VRPC");
pub const RPC_HEADER_VERSION: u32 = 0x0300_0000;

// Offsets in a queue's header page.
const TX_SIZE: usize = 4;
const TX_MSG_SIZE: usize = 8;
const TX_MSG_COUNT: usize = 12;
const TX_WRITE_PTR: usize = 16;
const TX_FLAGS: usize = 20;
const TX_RX_HDR_OFF: usize = 24;
const TX_ENTRY_OFF: usize = 28;
/// `rxHdrOff`: the read pointer sits right after the 32-byte tx header.
const RX_READ_PTR: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedLayout { pub pte_count: u32, pub ptes_size: usize, pub cmdq_off: usize, pub msgq_off: usize, pub total: usize }

pub const fn shared_layout() -> SharedLayout {
    let mut nr = (2 * QUEUE_SIZE) / PAGE;
    nr += (nr * 8).div_ceil(PAGE);
    let ptes_size = (nr * 8).div_ceil(PAGE) * PAGE;
    SharedLayout { pte_count: nr as u32, ptes_size, cmdq_off: ptes_size, msgq_off: ptes_size + QUEUE_SIZE, total: ptes_size + 2 * QUEUE_SIZE }
}

/// Entries per queue (the first page is the header).
pub const ENTRIES: u32 = ((QUEUE_SIZE - PAGE) / PAGE) as u32;

/// Lay out a fresh shared buffer: its page table and the command queue header.
/// GSP-RM writes the message queue header itself.
pub fn init_shared(shm: &mut Dma) -> Result<SharedLayout, Error> {
    let l = shared_layout();
    if shm.len() < l.total { return Err(Error::BufferTooSmall { need: l.total, have: shm.len() }); }
    shm.cpu[..l.total].fill(0);
    for i in 0..l.pte_count as usize { wr64(shm.cpu, i * 8, shm.bus + (i * PAGE) as u64); }
    let q = l.cmdq_off;
    wr32(shm.cpu, q, 0);
    wr32(shm.cpu, q + TX_SIZE, QUEUE_SIZE as u32);
    wr32(shm.cpu, q + TX_ENTRY_OFF, PAGE as u32);
    wr32(shm.cpu, q + TX_MSG_SIZE, PAGE as u32);
    wr32(shm.cpu, q + TX_MSG_COUNT, ENTRIES);
    wr32(shm.cpu, q + TX_WRITE_PTR, 0);
    wr32(shm.cpu, q + TX_FLAGS, 1);
    wr32(shm.cpu, q + TX_RX_HDR_OFF, RX_READ_PTR as u32);
    Ok(l)
}

/// The XOR-fold checksum that makes a whole element sum to zero.
pub fn checksum(elem: &[u8]) -> u32 {
    let mut c = 0u64;
    for w in elem.chunks(8) {
        let mut b = [0u8; 8];
        b[..w.len()].copy_from_slice(w);
        c ^= u64::from_le_bytes(b);
    }
    (c >> 32) as u32 ^ c as u32
}

/// A received RPC: function, result, and the payload length copied out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rpc { pub function: u32, pub result: u32, pub sequence: u32, pub len: usize }

/// The driver's side of both queues over the shared buffer.
pub struct Queues { pub l: SharedLayout, pub seq: u32 }

impl Queues {
    pub fn new(l: SharedLayout) -> Self { Queues { l, seq: 0 } }

    fn cmdq_wptr(&self, s: &[u8]) -> u32 { rd32(s, self.l.cmdq_off + TX_WRITE_PTR).unwrap_or(0) }
    /// GSP's read pointer of the command queue lives in the message queue's page.
    fn cmdq_rptr(&self, s: &[u8]) -> u32 { rd32(s, self.l.msgq_off + RX_READ_PTR).unwrap_or(0) }
    fn msgq_wptr(&self, s: &[u8]) -> u32 { rd32(s, self.l.msgq_off + TX_WRITE_PTR).unwrap_or(0) }
    /// Our read pointer of the message queue lives in the command queue's page.
    fn msgq_rptr(&self, s: &[u8]) -> u32 { rd32(s, self.l.cmdq_off + RX_READ_PTR).unwrap_or(0) }

    /// Free entries in the command queue.
    pub fn cmdq_free(&self, s: &[u8]) -> u32 {
        let (r, w) = (self.cmdq_rptr(s) % ENTRIES, self.cmdq_wptr(s) % ENTRIES);
        (r + ENTRIES - w - 1) % ENTRIES
    }

    /// Queue an RPC. The caller rings the doorbell (`QUEUE_HEAD0` on the GSP
    /// falcon) afterwards. `scratch` holds the assembled element (>= 16 pages).
    pub fn push(&mut self, s: &mut [u8], scratch: &mut [u8], function: u32, payload: &[u8]) -> Result<(), Error> {
        let rpc_len = RPC_HDR + payload.len();
        let len = (ELEM_HDR + crate::align_up(rpc_len as u64, 8) as usize).div_ceil(PAGE) * PAGE;
        if len > MAX_MSG || scratch.len() < len { return Err(Error::QueueFull); }
        let pages = (len / PAGE) as u32;
        if self.cmdq_free(s) < pages { return Err(Error::QueueFull); }
        let e = &mut scratch[..len];
        e.fill(0);
        wr32(e, 36, self.seq);
        wr32(e, 40, pages);
        let r = ELEM_HDR;
        wr32(e, r, RPC_HEADER_VERSION);
        wr32(e, r + 4, RPC_SIGNATURE);
        wr32(e, r + 8, rpc_len as u32);
        wr32(e, r + 12, function);
        wr32(e, r + 16, 0xffff_ffff);
        wr32(e, r + 20, 0xffff_ffff);
        e[r + RPC_HDR..r + rpc_len].copy_from_slice(payload);
        let c = checksum(e);
        wr32(e, 32, c);
        self.seq = self.seq.wrapping_add(1);
        let mut w = self.cmdq_wptr(s) % ENTRIES;
        for p in 0..pages as usize {
            let at = self.l.cmdq_off + PAGE + w as usize * PAGE;
            s[at..at + PAGE].copy_from_slice(&e[p * PAGE..(p + 1) * PAGE]);
            w = (w + 1) % ENTRIES;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        wr32v(s, self.l.cmdq_off + TX_WRITE_PTR, w);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    /// Whether GSP-RM has written a message we have not read.
    pub fn pending(&self, s: &[u8]) -> bool { self.msgq_wptr(s) % ENTRIES != self.msgq_rptr(s) % ENTRIES }

    /// Take the next message: copies its payload into `out` (truncated to
    /// `out.len()`) and advances our read pointer past all its pages.
    pub fn recv(&mut self, s: &mut [u8], out: &mut [u8]) -> Result<Option<Rpc>, Error> {
        if !self.pending(s) { return Ok(None); }
        let r = self.msgq_rptr(s) % ENTRIES;
        let entry = |i: u32| self.l.msgq_off + PAGE + (i % ENTRIES) as usize * PAGE;
        let at = entry(r);
        let rpc = at + ELEM_HDR;
        let length = rd32(s, rpc + 8).unwrap_or(0) as usize;
        if !(RPC_HDR..=MAX_MSG - ELEM_HDR).contains(&length) { return Err(Error::Gsp("msgq: bad message length")); }
        if rd32(s, rpc + 4) != Some(RPC_SIGNATURE) { return Err(Error::Gsp("msgq: bad RPC signature")); }
        let pages = (ELEM_HDR + length).div_ceil(PAGE) as u32;
        let avail = (self.msgq_wptr(s) % ENTRIES + ENTRIES - r) % ENTRIES;
        if avail < pages { return Ok(None); } // still being written
        let msg = Rpc { function: rd32(s, rpc + 12).unwrap_or(0), result: rd32(s, rpc + 16).unwrap_or(0), sequence: rd32(s, rpc + 24).unwrap_or(0), len: (length - RPC_HDR).min(out.len()) };
        // Copy the payload byte-wise through the wrap (logical offset -> page).
        for (i, b) in out[..msg.len].iter_mut().enumerate() {
            let logical = ELEM_HDR + RPC_HDR + i;
            let page = r + (logical / PAGE) as u32;
            *b = rd8v(s, entry(page) + logical % PAGE);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        wr32v(s, self.l.cmdq_off + RX_READ_PTR, (r + pages) % ENTRIES);
        Ok(Some(msg))
    }
}
