// SPDX-License-Identifier: GPL-3.0-only
//! USB network adapters, from SexUSB to SexNet: Ethernet frames through one shared region.
//!
//! BOOT gives SexNet and SexUSB the same region at [`SLOT`]. SexUSB drives the adapter
//! (CDC Ethernet) and writes the adapter's state and address into the header; frames
//! it receives go into the receive ring, frames SexNet sends come out of the send ring.
//! Each ring has one producer and one consumer and both sides poll it, so no call or
//! lock is needed: a producer writes the frame, then publishes its index (release); a
//! consumer reads the index (acquire), then the frame, then publishes its own index.

use core::sync::atomic::{AtomicU32, Ordering};

/// The shared region, in SexNet and in SexUSB.
pub const SLOT: u64 = 1002;
pub const REGION_ID: u32 = 0x4401;
pub const BYTES: u64 = 128 * 1024;
/// Frames each ring holds.
pub const SLOTS: u32 = 30;
/// Bytes a ring slot takes: a 4-byte length, then the frame.
pub const SLOT_BYTES: usize = 2048;
/// The largest frame without its CRC (no jumbo frames).
pub const MAX_FRAME: usize = 1514;
const MAGIC: u32 = 0x5553_424E; // "USBN"
const HEADER: usize = 4096;
const RX: usize = HEADER;
const TX: usize = HEADER + SLOTS as usize * SLOT_BYTES;

// Header words (byte offsets).
const W_MAGIC: usize = 0;
const W_LINK: usize = 4;
const W_MAC_LO: usize = 8;
const W_MAC_HI: usize = 12;
const W_RX_PROD: usize = 16;
const W_RX_CONS: usize = 20;
const W_TX_PROD: usize = 24;
const W_TX_CONS: usize = 28;
const W_RX_DROPPED: usize = 32;
const W_GENERATION: usize = 36;

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

    /// SexUSB: an adapter is ready (`Some(mac)`) or gone (`None`). Rings start empty.
    pub fn set_link(&self, mac: Option<[u8; 6]>) {
        self.store(W_LINK, 0);
        for w in [W_RX_PROD, W_RX_CONS, W_TX_PROD, W_TX_CONS] { self.store(w, 0); }
        if let Some(m) = mac {
            self.store(W_MAC_LO, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
            self.store(W_MAC_HI, u16::from_le_bytes([m[4], m[5]]) as u32);
            self.store(W_GENERATION, self.load(W_GENERATION).wrapping_add(1));
            self.store(W_MAGIC, MAGIC);
            self.store(W_LINK, 1);
        }
    }
    /// The adapter's address while one is ready, and the adapter's generation (it changes
    /// when an adapter is plugged in again, and the rings are then new).
    pub fn link(&self) -> Option<([u8; 6], u32)> {
        if self.load(W_MAGIC) != MAGIC || self.load(W_LINK) != 1 { return None; }
        let (lo, hi) = (self.load(W_MAC_LO).to_le_bytes(), self.load(W_MAC_HI).to_le_bytes());
        Some(([lo[0], lo[1], lo[2], lo[3], hi[0], hi[1]], self.load(W_GENERATION)))
    }
    pub fn rx_dropped(&self) -> u32 { self.load(W_RX_DROPPED) }

    fn slot(&self, ring: usize, index: u32) -> *mut u8 { unsafe { self.base.add(ring + (index % SLOTS) as usize * SLOT_BYTES) } }
    fn push(&self, ring: usize, prod: usize, cons: usize, frame: &[u8]) -> bool {
        if frame.is_empty() || frame.len() > MAX_FRAME { return false; }
        let (p, c) = (self.load(prod), self.load(cons));
        if p.wrapping_sub(c) >= SLOTS { return false; }
        let s = self.slot(ring, p);
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), s.add(4), frame.len());
            (s as *mut u32).write_volatile(frame.len() as u32);
        }
        self.store(prod, p.wrapping_add(1));
        true
    }
    fn pop(&self, ring: usize, prod: usize, cons: usize, out: &mut [u8]) -> Option<usize> {
        let (p, c) = (self.load(prod), self.load(cons));
        if p == c { return None; }
        let s = self.slot(ring, c);
        let len = unsafe { (s as *const u32).read_volatile() } as usize;
        let n = len.min(MAX_FRAME).min(out.len());
        unsafe { core::ptr::copy_nonoverlapping(s.add(4), out.as_mut_ptr(), n); }
        self.store(cons, c.wrapping_add(1));
        Some(n)
    }
    fn space(&self, prod: usize, cons: usize) -> u32 { SLOTS - self.load(prod).wrapping_sub(self.load(cons)).min(SLOTS) }

    /// SexUSB: a frame from the wire. False (and counted) when SexNet has not kept up.
    pub fn rx_push(&self, frame: &[u8]) -> bool {
        let ok = self.push(RX, W_RX_PROD, W_RX_CONS, frame);
        if !ok { self.store(W_RX_DROPPED, self.load(W_RX_DROPPED).wrapping_add(1)); }
        ok
    }
    /// SexNet: the next frame from the wire, copied into `out`; its length.
    pub fn rx_pop(&self, out: &mut [u8]) -> Option<usize> { self.pop(RX, W_RX_PROD, W_RX_CONS, out) }
    /// SexNet: queue a frame to send. False when the ring is full.
    pub fn tx_push(&self, frame: &[u8]) -> bool { self.push(TX, W_TX_PROD, W_TX_CONS, frame) }
    pub fn tx_space(&self) -> u32 { self.space(W_TX_PROD, W_TX_CONS) }
    /// SexUSB: the next frame to send, copied into `out`; its length.
    pub fn tx_pop(&self, out: &mut [u8]) -> Option<usize> { self.pop(TX, W_TX_PROD, W_TX_CONS, out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn region() -> Vec<u32> { vec![0u32; BYTES as usize / 4] }
    #[test]
    fn link_and_rings() {
        let mut mem = region();
        let s = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert!(s.link().is_none());
        s.set_link(Some([0x52, 0x54, 0, 0x12, 0x34, 0x56]));
        let (mac, generation) = s.link().unwrap();
        assert_eq!(mac, [0x52, 0x54, 0, 0x12, 0x34, 0x56]);
        assert_eq!(generation, 1);
        let mut out = [0u8; MAX_FRAME];
        assert!(s.rx_pop(&mut out).is_none());
        for n in 0..SLOTS { assert!(s.rx_push(&[n as u8; 60])); }
        assert!(!s.rx_push(&[0; 60]));
        assert_eq!(s.rx_dropped(), 1);
        for n in 0..SLOTS { assert_eq!(s.rx_pop(&mut out), Some(60)); assert_eq!(out[0], n as u8); }
        assert!(s.rx_pop(&mut out).is_none());
        // Wraps past the slot count.
        for n in 0..3 * SLOTS { assert!(s.tx_push(&[n as u8; 1514])); assert_eq!(s.tx_pop(&mut out), Some(1514)); assert_eq!(out[1513], n as u8); }
        assert_eq!(s.tx_space(), SLOTS);
        assert!(!s.tx_push(&[0; 1515]) && !s.tx_push(&[]));
        s.set_link(None);
        assert!(s.link().is_none());
        s.set_link(Some([1, 2, 3, 4, 5, 6]));
        assert_eq!(s.link().unwrap().1, 2);
    }
}
