// SPDX-License-Identifier: GPL-3.0-only
//! Where everything the firmware reads lives in the driver's DMA memory, and the
//! "context info" tables that tell the chip. Intel chips since the 22000 series
//! load their own firmware: the driver copies the sections into DRAM, lists their
//! addresses, and starts the CPU. Layouts follow OpenBSD iwx(4) byte for byte.
use crate::devices::Family;
use crate::ucode::{Image, Section};

pub const PAGE: u64 = 4096;
pub const MAX_DRAM_ENTRY: usize = 64;
pub const TX_RING_COUNT: usize = 256;
pub const RX_RING_COUNT: usize = 512;
pub const TFD_BYTES: u64 = 256;
pub const CONTEXT_INFO_BYTES: usize = 1792;
pub const CONTEXT_INFO_GEN3_BYTES: usize = 104;
pub const PRPH_SCRATCH_BYTES: usize = 1660;
/// log2(entries) as the chip wants ring sizes.
pub const RX_CB_SIZE: u32 = 9;
pub const TFD_CB_SIZE: u32 = 5;

const CTXT_INFO_TFD_FORMAT_LONG: u32 = 1 << 8;
const CTXT_INFO_RB_CB_SIZE_POS: u32 = 4;
const CTXT_INFO_RB_SIZE_POS: u32 = 9;
const CTXT_INFO_RB_SIZE_4K: u32 = 4;
const PRPH_SCRATCH_RB_SIZE_4K: u32 = 1 << 16;
const PRPH_SCRATCH_MTR_MODE: u32 = 1 << 17;
const PRPH_MTR_FORMAT_256B: u32 = 0xc0000;
const PRPH_SCRATCH_IMR_DEBUG_EN: u32 = 1 << 1;

/// Writes into the driver's DMA region (byte offsets from its start).
pub trait Dma {
    fn write(&mut self, offset: u64, bytes: &[u8]);
    fn zero(&mut self, offset: u64, len: u64);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range { pub offset: u64, pub len: u64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { TooManySections, TooSmall { needed: u64 } }

/// Placement of every structure in one DMA region. Offsets are page aligned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub family: Family,
    pub context: Range,
    pub prph_scratch: Range,
    pub prph_info: Range,
    pub rx_status: Range,
    pub rx_free: Range,
    pub rx_used: Range,
    pub cmd_ring: Range,
    pub iml: Range,
    pub lmac: [Range; MAX_DRAM_ENTRY],
    pub umac: [Range; MAX_DRAM_ENTRY],
    pub paging: [Range; MAX_DRAM_ENTRY],
    pub counts: (usize, usize, usize),
    /// First free byte after the firmware; RX buffers and the data queues go here.
    pub end: u64,
}

fn align(x: u64) -> u64 { (x + PAGE - 1) & !(PAGE - 1) }

impl Layout {
    pub fn plan(family: Family, image: &Image<'_, '_>, iml_len: usize, region_bytes: u64) -> Result<Self, Error> {
        if image.lmac.len() > MAX_DRAM_ENTRY || image.umac.len() > MAX_DRAM_ENTRY || image.paging.len() > MAX_DRAM_ENTRY {
            return Err(Error::TooManySections);
        }
        let gen3 = family == Family::Ax210;
        let mut at = 0u64;
        let mut take = |len: u64| { let r = Range { offset: at, len }; at = align(at + len.max(1)); r };
        let none = Range { offset: 0, len: 0 };
        let context = take(if gen3 { CONTEXT_INFO_GEN3_BYTES } else { CONTEXT_INFO_BYTES } as u64);
        let prph_scratch = if gen3 { take(PRPH_SCRATCH_BYTES as u64) } else { none };
        let prph_info = if gen3 { take(PAGE) } else { none };
        let rx_status = take(if gen3 { 2 } else { 12 });
        let rx_free = take(RX_RING_COUNT as u64 * if gen3 { 16 } else { 8 });
        let rx_used = take(RX_RING_COUNT as u64 * if gen3 { 32 } else { 4 });
        let cmd_ring = take(TX_RING_COUNT as u64 * TFD_BYTES);
        let iml = if gen3 { take(iml_len as u64) } else { none };
        let mut place = |sections: &[Section<'_>]| {
            let mut out = [none; MAX_DRAM_ENTRY];
            for (slot, s) in out.iter_mut().zip(sections) { *slot = take(s.data.len() as u64); }
            out
        };
        let lmac = place(image.lmac);
        let umac = place(image.umac);
        let paging = place(image.paging);
        let end = at;
        if end > region_bytes { return Err(Error::TooSmall { needed: end }); }
        Ok(Layout { family, context, prph_scratch, prph_info, rx_status, rx_free, rx_used, cmd_ring, iml,
            lmac, umac, paging, counts: (image.lmac.len(), image.umac.len(), image.paging.len()), end })
    }

    /// Copies the firmware sections and the image loader into place, writes the
    /// context info for this family, and returns the bytes of the context tables.
    /// `base` is the bus address of offset 0 of the DMA region.
    pub fn write(&self, dma: &mut impl Dma, base: u64, image: &Image<'_, '_>, iml: &[u8], hw_rev: u32, imr: bool) {
        dma.zero(0, self.end);
        for (r, s) in self.lmac.iter().zip(image.lmac).chain(self.umac.iter().zip(image.umac)).chain(self.paging.iter().zip(image.paging)) {
            dma.write(r.offset, s.data);
        }
        let mut dram = [0u8; 3 * MAX_DRAM_ENTRY * 8];
        let put = |dram: &mut [u8], table: usize, ranges: &[Range], n: usize| {
            for (i, r) in ranges.iter().take(n).enumerate() {
                let at = table * MAX_DRAM_ENTRY * 8 + i * 8;
                dram[at..at + 8].copy_from_slice(&(base + r.offset).to_le_bytes());
            }
        };
        // Order in the table is UMAC, LMAC, virtual (paging).
        put(&mut dram, 0, &self.umac, self.counts.1);
        put(&mut dram, 1, &self.lmac, self.counts.0);
        put(&mut dram, 2, &self.paging, self.counts.2);
        if self.family == Family::Ax210 {
            dma.write(self.iml.offset, iml);
            let mut scratch = [0u8; PRPH_SCRATCH_BYTES];
            scratch[0..2].copy_from_slice(&(hw_rev as u16).to_le_bytes());
            scratch[4..6].copy_from_slice(&((PRPH_SCRATCH_BYTES / 4) as u16).to_le_bytes());
            let mut flags = PRPH_SCRATCH_RB_SIZE_4K | PRPH_SCRATCH_MTR_MODE | PRPH_MTR_FORMAT_256B;
            if imr { flags |= PRPH_SCRATCH_IMR_DEBUG_EN; }
            scratch[8..12].copy_from_slice(&flags.to_le_bytes());
            scratch[48..56].copy_from_slice(&(base + self.rx_free.offset).to_le_bytes());
            scratch[124..].copy_from_slice(&dram);
            dma.write(self.prph_scratch.offset, &scratch);
            let mut c = [0u8; CONTEXT_INFO_GEN3_BYTES];
            let info = base + self.prph_info.offset;
            c[8..16].copy_from_slice(&info.to_le_bytes());
            c[16..24].copy_from_slice(&(base + self.rx_status.offset).to_le_bytes());
            c[24..32].copy_from_slice(&(info + PAGE / 2).to_le_bytes());
            c[32..40].copy_from_slice(&(info + 3 * PAGE / 4).to_le_bytes());
            c[52..60].copy_from_slice(&(base + self.cmd_ring.offset).to_le_bytes());
            c[60..68].copy_from_slice(&(base + self.rx_used.offset).to_le_bytes());
            c[68..70].copy_from_slice(&(TFD_CB_SIZE as u16).to_le_bytes());
            c[70..72].copy_from_slice(&(RX_CB_SIZE as u16).to_le_bytes());
            c[88..96].copy_from_slice(&(base + self.prph_scratch.offset).to_le_bytes());
            c[96..100].copy_from_slice(&(PRPH_SCRATCH_BYTES as u32).to_le_bytes());
            dma.write(self.context.offset, &c);
        } else {
            let mut c = [0u8; CONTEXT_INFO_BYTES];
            c[0..2].copy_from_slice(&(hw_rev as u16).to_le_bytes());
            c[4..6].copy_from_slice(&((CONTEXT_INFO_BYTES / 4) as u16).to_le_bytes());
            let flags = CTXT_INFO_TFD_FORMAT_LONG | (RX_CB_SIZE << CTXT_INFO_RB_CB_SIZE_POS) | (CTXT_INFO_RB_SIZE_4K << CTXT_INFO_RB_SIZE_POS);
            c[8..12].copy_from_slice(&flags.to_le_bytes());
            c[24..32].copy_from_slice(&(base + self.rx_free.offset).to_le_bytes());
            c[32..40].copy_from_slice(&(base + self.rx_used.offset).to_le_bytes());
            c[40..48].copy_from_slice(&(base + self.rx_status.offset).to_le_bytes());
            c[48..56].copy_from_slice(&(base + self.cmd_ring.offset).to_le_bytes());
            c[56] = TFD_CB_SIZE as u8;
            c[192..192 + dram.len()].copy_from_slice(&dram);
            dma.write(self.context.offset, &c);
        }
    }
}
