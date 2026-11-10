// SPDX-License-Identifier: GPL-3.0-only
//! The host command queue (TX queue 0, set up by the context info). A command is
//! a wide header and payload in one of the command buffers, described by a TFD
//! on the ring; the chip is told with HBUS_TARG_WRPTR. The answer comes back on
//! the receive ring as a packet carrying the same queue and index, without the
//! "unsolicited" bit. As in iwx(4) (`iwx_send_cmd`, `iwx_cmd_done`) one command
//! is in flight at a time; the caller waits for its answer between steps.
use crate::context::{Dma, Layout, Range, CMD_BUFFER_BYTES, CMD_SLOTS, TFD_BYTES, TX_RING_COUNT};
use crate::csr::{Registers, TARG_WRPTR};
use crate::devices::Family;
use crate::hcmd::{self, Packet, WIDE_HEADER};

pub const QUEUE: u8 = 0;
pub const MAX_PAYLOAD: usize = CMD_BUFFER_BYTES as usize - WIDE_HEADER;
/// Bytes of a response kept (NVM_GET_INFO, the largest one read at init, is 468).
pub const RESPONSE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Busy, TooLong }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sent { pub group: u8, pub opcode: u8, pub index: u8 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer { pub sent: Sent, pub failed: bool, pub len: usize }

pub struct CommandQueue {
    family: Family,
    /// Next ring entry (0..TX_RING_COUNT).
    pub cur: u16,
    /// The write pointer the chip is told: it counts past the ring size on the
    /// AX210 family (wrapping at 65536), and wraps with the ring before that.
    pub hw: u32,
    pub pending: Option<Sent>,
    pub answer: Option<Answer>,
    pub response: [u8; RESPONSE_BYTES],
    pub sent: u32,
}

impl CommandQueue {
    pub fn new(family: Family) -> Self {
        Self { family, cur: 0, hw: 0, pending: None, answer: None, response: [0; RESPONSE_BYTES], sent: 0 }
    }

    fn hw_wrap(&self) -> u32 { if self.family == Family::Ax210 { 65536 } else { TX_RING_COUNT as u32 } }

    /// Command buffer used by ring entry `index`.
    pub fn buffer(layout: &Layout, index: u16) -> Range {
        Range { offset: layout.cmd_buffers.offset + (index as usize % CMD_SLOTS) as u64 * CMD_BUFFER_BYTES, len: CMD_BUFFER_BYTES }
    }

    /// Queues one command and tells the chip. Its answer is picked up by `on_packet`.
    pub fn send(&mut self, dma: &mut impl Dma, r: &mut impl Registers, layout: &Layout, base: u64,
        group: u8, opcode: u8, version: u8, payload: &[u8]) -> Result<Sent, Error>
    {
        if self.pending.is_some() { return Err(Error::Busy); }
        if payload.len() > MAX_PAYLOAD { return Err(Error::TooLong); }
        let index = self.cur as u8;
        let buf = Self::buffer(layout, self.cur);
        dma.write(buf.offset, &hcmd::header(group, opcode, version, QUEUE, index, payload.len() as u16));
        dma.write(buf.offset + WIDE_HEADER as u64, payload);
        let tfd = hcmd::tfd_for_command(base + buf.offset, WIDE_HEADER + payload.len());
        dma.write(layout.cmd_ring.offset + self.cur as u64 * TFD_BYTES, &tfd);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.cur = ((self.cur as usize + 1) % TX_RING_COUNT) as u16;
        self.hw = (self.hw + 1) % self.hw_wrap();
        r.write(TARG_WRPTR, hcmd::write_pointer(QUEUE, self.hw as u16));
        let sent = Sent { group, opcode, index };
        self.pending = Some(sent);
        self.answer = None;
        self.sent = self.sent.wrapping_add(1);
        Ok(sent)
    }

    /// Takes a packet from the receive ring; true when it answers the pending command.
    pub fn on_packet(&mut self, p: &Packet<'_>) -> bool {
        let Some(sent) = self.pending else { return false };
        if !p.is_response() || p.qid != QUEUE || p.index != sent.index || !p.is(sent.group, sent.opcode) { return false; }
        let n = p.data.len().min(RESPONSE_BYTES);
        self.response[..n].copy_from_slice(&p.data[..n]);
        self.answer = Some(Answer { sent, failed: p.failed(), len: p.data.len() });
        self.pending = None;
        true
    }

    /// The answer's payload (truncated to `RESPONSE_BYTES`).
    pub fn response(&self) -> &[u8] { &self.response[..self.answer.map_or(0, |a| a.len.min(RESPONSE_BYTES))] }

    /// Gives up on the pending command (it timed out).
    pub fn abandon(&mut self) { self.pending = None; }
}
