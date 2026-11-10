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
/// Receive buffers actually posted: the ring has 512 descriptors, but 64 pages are
/// plenty for firmware notifications and keep the region small.
pub const RX_BUFFERS: usize = 64;
pub const RX_BUFFER_BYTES: u64 = 4096;
pub const TFD_BYTES: u64 = 256;
/// Host command buffers on queue 0. One command is in flight at a time, so a few
/// pages, reused round the ring, are enough; each holds the largest command.
pub const CMD_SLOTS: usize = 16;
pub const CMD_BUFFER_BYTES: u64 = 4096;
/// Platform NVM for AX210-class chips: one page of segment addresses (used when
/// the firmware wants a fragmented image) and room for one SKU's sections
/// (about 14 KiB in today's files).
pub const PNVM_BYTES: u64 = 64 * 1024;
/// Frame buffers for the management/data TX queue (stage W5): each holds a TX
/// command and one frame; reused round the ring like the command buffers.
pub const TX_SLOTS: usize = 32;
pub const TX_BUFFER_BYTES: u64 = 4096;
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

/// Reads what the device wrote into the DMA region (RX status, completions, buffers).
pub trait DmaRead {
    fn read(&self, offset: u64, out: &mut [u8]);
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
    /// `RX_BUFFERS` receive pages, one after another.
    pub rx_buffers: Range,
    /// `CMD_SLOTS` host command buffers.
    pub cmd_buffers: Range,
    /// Platform NVM table page and sections (AX210 family; empty otherwise).
    pub pnvm: Range,
    /// The station's TX queue (W5): its TFD ring, byte-count table and frame buffers.
    pub tx_ring: Range,
    pub tx_byte_counts: Range,
    pub tx_buffers: Range,
    /// First free byte after the RX buffers; the data queues go here.
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
        let rx_buffers = take(RX_BUFFERS as u64 * RX_BUFFER_BYTES);
        let cmd_buffers = take(CMD_SLOTS as u64 * CMD_BUFFER_BYTES);
        let pnvm = if gen3 { take(PAGE + PNVM_BYTES) } else { none };
        let tx_ring = take(TX_RING_COUNT as u64 * TFD_BYTES);
        let tx_byte_counts = take(PAGE);
        let tx_buffers = take(TX_SLOTS as u64 * TX_BUFFER_BYTES);
        let end = at;
        if end > region_bytes { return Err(Error::TooSmall { needed: end }); }
        Ok(Layout { family, context, prph_scratch, prph_info, rx_status, rx_free, rx_used, cmd_ring, iml,
            lmac, umac, paging, counts: (image.lmac.len(), image.umac.len(), image.paging.len()), rx_buffers, cmd_buffers, pnvm, tx_ring, tx_byte_counts, tx_buffers, end })
    }

    /// Puts one SKU's platform NVM sections in the region and points the
    /// peripheral scratch at them (`iwx_pnvm_setup`, `iwx_ctxt_info_gen3_set_pnvm`).
    /// A fragmented image is a table of segment addresses, each segment on its own
    /// pages; otherwise the sections are joined. Returns (address, bytes) given to
    /// the chip, or None when this family has no PNVM or the sections do not fit.
    pub fn write_pnvm(&self, dma: &mut impl Dma, base: u64, sections: &[&[u8]], fragmented: bool) -> Option<(u64, u32)> {
        if self.family != Family::Ax210 || sections.is_empty() || sections.len() > MAX_DRAM_ENTRY { return None; }
        let data = self.pnvm.offset + PAGE;
        let limit = self.pnvm.offset + self.pnvm.len;
        let mut at = data;
        let mut table = [0u8; MAX_DRAM_ENTRY * 8];
        let mut total = 0u64;
        for (i, s) in sections.iter().enumerate() {
            if at + s.len() as u64 > limit { return None; }
            dma.write(at, s);
            table[i * 8..i * 8 + 8].copy_from_slice(&(base + at).to_le_bytes());
            total += s.len() as u64;
            at += s.len() as u64;
            if fragmented { at = align(at); }
        }
        let addr = if fragmented { dma.write(self.pnvm.offset, &table); base + self.pnvm.offset } else { base + data };
        // prph_scratch.ctrl_cfg.pnvm_cfg: base address at 16, size at 24.
        dma.write(self.prph_scratch.offset + 16, &addr.to_le_bytes());
        dma.write(self.prph_scratch.offset + 24, &(total as u32).to_le_bytes());
        Some((addr, total as u32))
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
