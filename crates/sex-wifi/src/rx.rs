// SPDX-License-Identifier: GPL-3.0-only
//! The receive ring: the driver posts empty pages on the "free" ring, the firmware
//! fills them and reports each one on the "used" ring and in the status word. Every
//! firmware notification, ALIVE included, arrives this way. Formats follow OpenBSD
//! iwx(4): 22000-series free entries are the page address with the buffer id in the
//! low 12 bits; AX210 entries are 16 bytes {rbid, reserved, address}. Used entries
//! are 4 bytes (22000, id in the low 12 bits) or 32 bytes with the id at offset 4.
use crate::context::{Dma, DmaRead, Layout, Range, RX_BUFFERS, RX_BUFFER_BYTES, RX_RING_COUNT};
use crate::csr::Registers;
use crate::devices::Family;

/// Free-ring write index of RX queue 0 (a plain CSR on both families).
pub const RFH_Q0_FRBDCB_WIDX_TRG: u32 = 0x1c80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { BadBuffer(u16) }

#[derive(Clone, Debug)]
pub struct Rx {
    family: Family,
    /// Next used-ring entry the driver has not read.
    pub cur: u16,
    /// Next free-ring entry the driver will post a page in.
    pub write: u16,
}

impl Rx {
    pub fn new(family: Family) -> Self { Self { family, cur: 0, write: 0 } }
    fn gen3(&self) -> bool { self.family == Family::Ax210 }

    /// Bytes of buffer `id` in the region.
    pub fn buffer(layout: &Layout, id: u16) -> Range {
        Range { offset: layout.rx_buffers.offset + id as u64 * RX_BUFFER_BYTES, len: RX_BUFFER_BYTES }
    }

    /// Posts page `id` at the next free-ring entry.
    pub fn post(&mut self, dma: &mut impl Dma, layout: &Layout, base: u64, id: u16) {
        let addr = base + Self::buffer(layout, id).offset;
        let at = self.write as u64;
        if self.gen3() {
            let mut d = [0u8; 16];
            d[0..2].copy_from_slice(&id.to_le_bytes());
            d[8..16].copy_from_slice(&addr.to_le_bytes());
            dma.write(layout.rx_free.offset + at * 16, &d);
        } else {
            dma.write(layout.rx_free.offset + at * 8, &(addr | (id as u64 & 0xfff)).to_le_bytes());
        }
        self.write = ((self.write as usize + 1) % RX_RING_COUNT) as u16;
    }

    /// Posts every page once. Called after `Layout::write`, before the CPU starts.
    pub fn fill(&mut self, dma: &mut impl Dma, layout: &Layout, base: u64) {
        for id in 0..RX_BUFFERS as u16 { self.post(dma, layout, base, id); }
    }

    /// The write index the chip is told: a multiple of 8, as the hardware requires.
    pub fn write_index(&self) -> u32 { self.write as u32 & !7 }

    /// Tells the chip how far the free ring is filled.
    pub fn kick(&self, r: &mut impl Registers) { r.write(RFH_Q0_FRBDCB_WIDX_TRG, self.write_index()); }

    /// The last used-ring entry the firmware closed.
    pub fn closed(&self, dma: &impl DmaRead, layout: &Layout) -> u16 {
        let mut b = [0u8; 2];
        dma.read(layout.rx_status.offset, &mut b);
        u16::from_le_bytes(b) & 0xfff
    }

    /// The next page the firmware filled, if any. The caller reads it with
    /// `Rx::buffer`, then hands it back with `post`.
    pub fn next(&mut self, dma: &impl DmaRead, layout: &Layout) -> Option<Result<u16, Error>> {
        let hw = self.closed(dma, layout) as usize % RX_RING_COUNT;
        if self.cur as usize == hw { return None; }
        let at = self.cur as u64;
        let id = if self.gen3() {
            let mut b = [0u8; 2];
            dma.read(layout.rx_used.offset + at * 32 + 4, &mut b);
            u16::from_le_bytes(b)
        } else {
            let mut b = [0u8; 4];
            dma.read(layout.rx_used.offset + at * 4, &mut b);
            (u32::from_le_bytes(b) & 0xfff) as u16
        };
        self.cur = ((self.cur as usize + 1) % RX_RING_COUNT) as u16;
        Some(if (id as usize) < RX_BUFFERS { Ok(id) } else { Err(Error::BadBuffer(id)) })
    }
}
