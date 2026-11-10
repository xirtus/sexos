// SPDX-License-Identifier: GPL-3.0-only
//! Stage W5b: sending 802.11 frames on the station's TX queue (queue 1, set up by
//! the join), as iwx(4)'s `iwx_tx` does. Each frame lives in one of the queue's
//! buffers: a 4-byte command header, the TX command (`iwx_tx_cmd_gen3` on the
//! AX210 family, `iwx_tx_cmd_gen2` before it) holding the 802.11 header, padding
//! to four bytes, then the frame body. The TFD points at the first 20 bytes
//! (TB0, which the hardware writes back to), the rest of the command (TB1) and
//! the body (TB2). The byte-count table tells the scheduler the frame's length.
//! The firmware answers each frame with a TX response (`iwx_rx_tx_cmd`) that
//! carries the status and how far the ring may be reused.
use crate::context::{Dma, Layout, TFD_BYTES, TX_BUFFER_BYTES, TX_RING_COUNT, TX_SLOTS};
use crate::csr::{Registers, TARG_WRPTR};
use crate::devices::Family;
use crate::hcmd::{self as h, Packet, FIRST_TB_SIZE};
use crate::join::{Target, MGMT_QUEUE};

/// The command header in front of a TX command (`iwx_cmd_header`).
pub const CMD_HEADER: usize = 4;
pub const GEN3_BYTES: usize = 28;
pub const GEN2_BYTES: usize = 20;
/// The TX response's size (`iwx_tx_resp` v6) and the SSN word after it.
pub const RESPONSE_BYTES: usize = 44;

pub const FLAG_CMD_RATE: u16 = 1 << 0;
pub const FLAG_ENCRYPT_DIS: u16 = 1 << 1;
pub const FLAG_HIGH_PRI: u16 = 1 << 2;
const OFFLD_PAD: u32 = 1 << 13;
const RATE_ANT_A: u32 = 1 << 14;
const RATE_LEGACY_OFDM: u32 = 1 << 8;
pub const STATUS_SUCCESS: u16 = 0x01;
pub const STATUS_DIRECT_DONE: u16 = 0x02;

/// The rate for management frames (`iwx_tx_fill_cmd`, rate_n_flags v2): the
/// lowest basic rate of the AP's band, on antenna A. CCK 1 Mb/s on 2.4 GHz
/// unless the AP's basic rates are all OFDM; OFDM 6 Mb/s or the AP's lowest on 5 GHz.
pub fn management_rate(t: &Target) -> u32 {
    let lowest = |bits: u8| if bits == 0 { 0 } else { bits.trailing_zeros() };
    if t.two_ghz() && (t.basic_cck & 0xf != 0 || t.basic_ofdm == 0) {
        RATE_ANT_A | lowest(t.basic_cck & 0xf)
    } else {
        RATE_ANT_A | RATE_LEGACY_OFDM | lowest(t.basic_ofdm)
    }
}

/// The TX command's bytes for a frame: `len` = whole 802.11 frame, `header` its
/// 802.11 header, which is copied in. Returns the command's length with the pad.
pub fn command(out: &mut [u8], family: Family, len: usize, header: &[u8], flags: u16, rate: u32) -> usize {
    let pad = (4 - header.len() % 4) % 4;
    let offload = ((header.len() / 2) as u32 & 0x1f) << 8 | if pad != 0 { OFFLD_PAD } else { 0 };
    let size = if family == Family::Ax210 { GEN3_BYTES } else { GEN2_BYTES };
    out[..size + header.len() + pad].fill(0);
    out[0..2].copy_from_slice(&(len as u16).to_le_bytes());
    if family == Family::Ax210 {
        out[2..4].copy_from_slice(&flags.to_le_bytes());
        out[4..8].copy_from_slice(&offload.to_le_bytes());
    } else {
        out[2..4].copy_from_slice(&(offload as u16).to_le_bytes());
        out[4..8].copy_from_slice(&(flags as u32).to_le_bytes());
    }
    // dram_info (8 bytes) stays 0; rate_n_flags follows it.
    out[16..20].copy_from_slice(&rate.to_le_bytes());
    out[size..size + header.len()].copy_from_slice(header);
    size + header.len() + pad
}

/// A byte-count table entry (`iwx_tx_update_byte_tbl`): bytes on the AX210 family,
/// dwords before it, and the number of 64-byte TFD chunks to fetch, less one.
pub fn byte_count(family: Family, len: usize, tbs: usize) -> u16 {
    let chunks = (2 + tbs * 10).div_ceil(64) as u16 - 1;
    if family == Family::Ax210 { len as u16 | chunks << 14 } else { len.div_ceil(4) as u16 | chunks << 12 }
}

/// What the firmware said about one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status { pub index: u8, pub status: u16, pub retries: u8, pub ssn: u32 }
impl Status {
    pub fn ok(&self) -> bool { matches!(self.status & 0xff, STATUS_SUCCESS | STATUS_DIRECT_DONE) }
}

/// Reads a TX response for our queue, if `p` is one.
pub fn parse_response(p: &Packet<'_>) -> Option<Status> {
    if p.code != h::TX_CMD || p.qid & 0x7f != MGMT_QUEUE as u8 || p.data.len() < RESPONSE_BYTES + 4 { return None; }
    let d = p.data;
    if d[0] != 1 { return None; } // one frame: aggregation is not used
    Some(Status {
        index: p.index,
        status: u16::from_le_bytes([d[40], d[41]]),
        retries: d[3],
        ssn: u32::from_le_bytes(d[44..48].try_into().unwrap()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Full, TooLong }

pub struct TxQueue {
    family: Family,
    /// Next ring entry, and the write pointer the chip is told (as `cmdq`).
    pub cur: u16,
    pub hw: u32,
    /// Ring entry and write pointer of the oldest frame not yet answered.
    tail: u16,
    tail_hw: u32,
    pub queued: u16,
    pub sent: u32,
    pub last: Option<Status>,
}

impl TxQueue {
    pub fn new(family: Family) -> Self {
        Self { family, cur: 0, hw: 0, tail: 0, tail_hw: 0, queued: 0, sent: 0, last: None }
    }
    fn hw_wrap(&self) -> u32 { if self.family == Family::Ax210 { 65536 } else { TX_RING_COUNT as u32 } }

    /// Queues one 802.11 frame of which the first `header_len` bytes are the header.
    pub fn send(&mut self, dma: &mut impl Dma, r: &mut impl Registers, layout: &Layout, base: u64,
        frame: &[u8], header_len: usize, flags: u16, rate: u32) -> Result<u8, Error>
    {
        if self.queued as usize >= TX_SLOTS { return Err(Error::Full); }
        let index = self.cur;
        let slot = layout.tx_buffers.offset + (index as usize % TX_SLOTS) as u64 * TX_BUFFER_BYTES;
        let mut cmd = [0u8; CMD_HEADER + GEN3_BYTES + 32 + 4];
        if header_len > 32 || frame.len() < header_len { return Err(Error::TooLong); }
        cmd[0] = h::TX_CMD;
        cmd[2] = index as u8;
        cmd[3] = MGMT_QUEUE as u8;
        let n = CMD_HEADER + command(&mut cmd[CMD_HEADER..], self.family, frame.len(), &frame[..header_len], flags, rate);
        let body = &frame[header_len..];
        if n + body.len() > TX_BUFFER_BYTES as usize { return Err(Error::TooLong); }
        dma.write(slot, &cmd[..n]);
        dma.write(slot + n as u64, body);
        let addr = base + slot;
        let mut tfd = [0u8; TFD_BYTES as usize];
        let mut put = |i: usize, l: usize, a: u64| {
            let at = 2 + i * 10;
            tfd[at..at + 2].copy_from_slice(&(l as u16).to_le_bytes());
            tfd[at + 2..at + 10].copy_from_slice(&a.to_le_bytes());
        };
        put(0, FIRST_TB_SIZE, addr);
        put(1, n - FIRST_TB_SIZE, addr + FIRST_TB_SIZE as u64);
        let tbs = if body.is_empty() { 2 } else { put(2, body.len(), addr + n as u64); 3 };
        tfd[0..2].copy_from_slice(&(tbs as u16).to_le_bytes());
        dma.write(layout.tx_ring.offset + index as u64 * TFD_BYTES, &tfd);
        dma.write(layout.tx_byte_counts.offset + index as u64 * 2, &byte_count(self.family, frame.len(), tbs).to_le_bytes());
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.cur = ((self.cur as usize + 1) % TX_RING_COUNT) as u16;
        self.hw = (self.hw + 1) % self.hw_wrap();
        self.queued += 1;
        self.sent = self.sent.wrapping_add(1);
        r.write(TARG_WRPTR, h::write_pointer(MGMT_QUEUE as u8, self.hw as u16));
        Ok(index as u8)
    }

    /// Takes a TX response (`parse_response`): frees the ring up to its SSN
    /// (`iwx_txq_advance`).
    pub fn release(&mut self, dma: &mut impl Dma, layout: &Layout, s: Status) {
        if s.ssn < self.hw_wrap() {
            while self.tail_hw != s.ssn && self.queued > 0 {
                dma.write(layout.tx_byte_counts.offset + self.tail as u64 * 2, &[0, 0]);
                self.tail = ((self.tail as usize + 1) % TX_RING_COUNT) as u16;
                self.tail_hw = (self.tail_hw + 1) % self.hw_wrap();
                self.queued -= 1;
            }
        }
        self.last = Some(s);
    }
}
