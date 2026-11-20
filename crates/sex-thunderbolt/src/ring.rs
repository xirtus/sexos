// SPDX-License-Identifier: GPL-3.0-only
//! NHI DMA rings in raw mode (the driver adds and checks the CRC itself), as the
//! control channel uses them. A descriptor is 16 bytes: the buffer's bus address,
//! then length (11:0), EOF (15:12), SOF (19:16) and flags (31:20), then a time
//! word written as zero. Each slot owns one 256-byte buffer.

use crate::regs::*;
use crate::Nhi;

/// Slots per control ring (Linux uses 10; one always stays empty).
pub const SLOTS: u16 = 10;
pub const DESCRIPTOR_BYTES: u64 = 16;

/// Where ring 0's descriptors and buffers sit in the DMA pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout { pub bus: u64, pub tx_descriptors: u64, pub rx_descriptors: u64, pub tx_buffers: u64, pub rx_buffers: u64, pub end: u64 }
impl Layout {
    /// Ring 0 at the start of a pool whose bus address is `bus`.
    pub const fn control(bus: u64) -> Self {
        let tx_descriptors = 0;
        let rx_descriptors = 256;
        let tx_buffers = 512;
        let rx_buffers = tx_buffers + SLOTS as u64 * FRAME_SIZE as u64;
        Layout { bus, tx_descriptors, rx_descriptors, tx_buffers, rx_buffers, end: rx_buffers + SLOTS as u64 * FRAME_SIZE as u64 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Full, TooLong }

/// One ring. `head` is the next slot the driver fills, `tail` the oldest the
/// controller still owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring { pub hop: u16, pub tx: bool, pub size: u16, pub descriptors: u64, pub buffers: u64, pub bus: u64, pub head: u16, pub tail: u16, pub running: bool }

impl Ring {
    pub fn tx(layout: &Layout, hop: u16) -> Self {
        Ring { hop, tx: true, size: SLOTS, descriptors: layout.tx_descriptors, buffers: layout.tx_buffers, bus: layout.bus, head: 0, tail: 0, running: false }
    }
    pub fn rx(layout: &Layout, hop: u16) -> Self {
        Ring { hop, tx: false, size: SLOTS, descriptors: layout.rx_descriptors, buffers: layout.rx_buffers, bus: layout.bus, head: 0, tail: 0, running: false }
    }
    fn base(&self) -> u32 { (if self.tx { TX_RING_BASE } else { RX_RING_BASE }) + self.hop as u32 * 16 }
    fn options(&self) -> u32 { (if self.tx { TX_OPTIONS_BASE } else { RX_OPTIONS_BASE }) + self.hop as u32 * 32 }
    fn descriptor(&self, slot: u16) -> u64 { self.descriptors + slot as u64 * DESCRIPTOR_BYTES }
    fn buffer(&self, slot: u16) -> u64 { self.buffers + slot as u64 * FRAME_SIZE as u64 }
    pub fn full(&self) -> bool { (self.head + 1) % self.size == self.tail }
    pub fn empty(&self) -> bool { self.head == self.tail }

    fn write_descriptor(&self, n: &mut impl Nhi, slot: u16, length: u32, pdf: u8) {
        let mut d = [0u8; 16];
        d[0..8].copy_from_slice(&(self.bus + self.buffer(slot)).to_le_bytes());
        let word = (length & 0xfff) | (pdf as u32 & 0xf) << 12 | (pdf as u32 & 0xf) << 16 | DESC_POSTED << 20;
        d[8..12].copy_from_slice(&word.to_le_bytes());
        n.store(self.descriptor(slot), &d);
    }

    /// The flags/length word of a slot's descriptor.
    fn status(&self, n: &impl Nhi, slot: u16) -> u32 {
        let mut w = [0u8; 4];
        n.load(self.descriptor(slot) + 8, &mut w);
        u32::from_le_bytes(w)
    }

    /// Points the controller at the ring and enables it (Linux `tb_ring_start`,
    /// raw mode, no interrupts: the server polls). RX rings take every frame type
    /// and get all but one slot posted.
    pub fn start(&mut self, n: &mut impl Nhi) {
        self.head = 0;
        self.tail = 0;
        n.store(self.descriptors, &[0u8; SLOTS as usize * DESCRIPTOR_BYTES as usize]);
        let at = self.bus + self.descriptors;
        let (base, options) = (self.base(), self.options());
        n.write(base, at as u32);
        n.write(base + 4, (at >> 32) as u32);
        if self.tx {
            n.write(base + 12, self.size as u32);
            n.write(options + 4, 0);
        } else {
            n.write(base + 12, (FRAME_SIZE as u32) << 16 | self.size as u32);
            n.write(options + 4, 0xffff << 16 | 0xffff);
        }
        n.write(options, RING_FLAG_ENABLE | RING_FLAG_RAW);
        self.running = true;
        if !self.tx {
            while !self.full() { self.write_descriptor(n, self.head, 0, 0); self.head = (self.head + 1) % self.size; }
            n.write(base + 8, self.head as u32);
        }
    }

    /// Disables the ring (Linux `tb_ring_stop`).
    pub fn stop(&mut self, n: &mut impl Nhi) {
        let (base, options) = (self.base(), self.options());
        n.write(options, 0);
        n.write(base, 0);
        n.write(base + 4, 0);
        n.write(base + 8, 0);
        n.write(base + 12, 0);
        self.running = false;
        self.head = 0;
        self.tail = 0;
    }

    /// Frees TX slots the controller has finished with.
    pub fn reap(&mut self, n: &impl Nhi) -> u16 {
        let mut done = 0;
        while self.tx && !self.empty() && (self.status(n, self.tail) >> 20) & DESC_COMPLETED != 0 {
            self.tail = (self.tail + 1) % self.size;
            done += 1;
        }
        done
    }

    /// Queues one frame (already CRC'd) of type `pdf` on a TX ring.
    pub fn send(&mut self, n: &mut impl Nhi, frame: &[u8], pdf: u8) -> Result<(), Error> {
        if frame.len() > FRAME_SIZE { return Err(Error::TooLong); }
        self.reap(n);
        if self.full() { return Err(Error::Full); }
        let slot = self.head;
        n.store(self.buffer(slot), frame);
        self.write_descriptor(n, slot, frame.len() as u32, pdf);
        self.head = (self.head + 1) % self.size;
        n.write(self.base() + 8, (self.head as u32) << 16);
        Ok(())
    }

    /// Takes the oldest completed RX frame into `out`: (type, bytes with CRC,
    /// descriptor flags). Reposts the slot.
    pub fn receive(&mut self, n: &mut impl Nhi, out: &mut [u8; FRAME_SIZE]) -> Option<(u8, usize, u32)> {
        if self.tx || !self.running || self.empty() { return None; }
        let status = self.status(n, self.tail);
        let flags = status >> 20;
        if flags & DESC_COMPLETED == 0 { return None; }
        let length = ((status & 0xfff) as usize).min(FRAME_SIZE);
        let pdf = ((status >> 12) & 0xf) as u8;
        n.load(self.buffer(self.tail), &mut out[..length]);
        self.tail = (self.tail + 1) % self.size;
        // The slot before the tail was the empty one: post it.
        self.write_descriptor(n, self.head, 0, 0);
        self.head = (self.head + 1) % self.size;
        n.write(self.base() + 8, self.head as u32);
        Some((pdf, length, flags))
    }
}
