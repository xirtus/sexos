// SPDX-License-Identifier: GPL-3.0-only
//! Host commands to the firmware and packets back from it.
//! A command is a "wide" header plus payload in a DMA buffer, described to the
//! chip by a transfer descriptor (TFD) on queue 0; responses and notifications
//! arrive on the RX ring as packets with the same header.

pub const LEGACY_GROUP: u8 = 0;
pub const LONG_GROUP: u8 = 1;
pub const ALIVE: u8 = 0x1;
pub const ALIVE_STATUS_OK: u16 = 0xcafe;
pub const FIRST_TB_SIZE: usize = 20;
pub const WIDE_HEADER: usize = 8;
pub const TFD_BYTES: usize = 256;
pub const NUM_TBS: usize = 25;
const FRAME_SIZE_MASK: u32 = 0x3fff;
const FRAME_INVALID: u32 = 0x5555_0000;
const FRAME_ALIGN: usize = 0x40;

/// Writes a command (header + payload) into `out`; returns its length.
/// Group 0 commands are sent as LONG_GROUP: firmware API 50+ rejects narrow ones.
pub fn encode(out: &mut [u8], group: u8, opcode: u8, version: u8, qid: u8, index: u8, payload: &[u8]) -> Option<usize> {
    let total = WIDE_HEADER + payload.len();
    if out.len() < total || payload.len() > u16::MAX as usize { return None; }
    out[0] = opcode;
    out[1] = if group == LEGACY_GROUP { LONG_GROUP } else { group };
    out[2] = index;
    out[3] = qid;
    out[4..6].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    out[6] = 0;
    out[7] = version;
    out[8..total].copy_from_slice(payload);
    Some(total)
}

/// A TFD for a buffer at bus address `addr` of `len` bytes: the first 20 bytes
/// in TB0 and the rest in TB1, as the scheduler reads the header from TB0.
pub fn tfd_for_command(addr: u64, len: usize) -> [u8; TFD_BYTES] {
    let mut tfd = [0u8; TFD_BYTES];
    let first = len.min(FIRST_TB_SIZE);
    let mut put = |i: usize, l: usize, a: u64| {
        let at = 2 + i * 10;
        tfd[at..at + 2].copy_from_slice(&(l as u16).to_le_bytes());
        tfd[at + 2..at + 10].copy_from_slice(&a.to_le_bytes());
    };
    put(0, first, addr);
    let mut n = 1u16;
    if len > FIRST_TB_SIZE { put(1, len - FIRST_TB_SIZE, addr + FIRST_TB_SIZE as u64); n = 2; }
    tfd[0..2].copy_from_slice(&n.to_le_bytes());
    tfd
}

/// The value written to HBUS_TARG_WRPTR to tell the chip a queue advanced.
pub fn write_pointer(qid: u8, index: u16) -> u32 { (qid as u32) << 16 | index as u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> { pub code: u8, pub group: u8, pub index: u8, pub qid: u8, pub data: &'a [u8] }
impl Packet<'_> {
    /// Responses to our commands carry the queue id without the "unsolicited" bit.
    pub fn is_response(&self) -> bool { self.qid & 0x80 == 0 }
}

/// Splits one RX buffer into packets (a 4 KiB buffer may hold several).
pub fn packets(buffer: &[u8]) -> impl Iterator<Item = Packet<'_>> {
    let mut rest = buffer;
    core::iter::from_fn(move || {
        if rest.len() < 8 { return None; }
        let word = u32::from_le_bytes(rest[..4].try_into().unwrap());
        let len = (word & FRAME_SIZE_MASK) as usize;
        if word == FRAME_INVALID || (rest[4] == 0 && rest[5] == 0 && rest[6] == 0 && rest[7] & 0x7f == 0) || len < 4 || 4 + len > rest.len() { return None; }
        let p = Packet { code: rest[4], group: rest[5], index: rest[6], qid: rest[7], data: &rest[8..4 + len] };
        // Packets are padded to 64-byte boundaries in the buffer (FH_RSCSR_FRAME_ALIGN).
        let step = (4 + len + FRAME_ALIGN - 1) & !(FRAME_ALIGN - 1);
        rest = &rest[step.min(rest.len())..];
        Some(p)
    })
}

/// Firmware versions and status from the ALIVE notification (v4/v5 layouts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alive { pub ok: bool, pub lmac_major: u32, pub lmac_minor: u32, pub umac_major: u32, pub umac_minor: u32, pub error_table: u32, pub umac_error_table: u32 }

pub fn parse_alive(p: &Packet<'_>) -> Option<Alive> {
    if p.code != ALIVE || (p.group != LEGACY_GROUP && p.group != LONG_GROUP) || p.data.len() < 4 + 2 * 48 + 16 { return None; }
    let d = p.data;
    let u = |at: usize| u32::from_le_bytes(d[at..at + 4].try_into().unwrap());
    let umac = 4 + 2 * 48;
    Some(Alive {
        ok: u16::from_le_bytes([d[0], d[1]]) == ALIVE_STATUS_OK,
        lmac_major: u(4), lmac_minor: u(8), error_table: u(4 + 16),
        umac_major: u(umac), umac_minor: u(umac + 4), umac_error_table: u(umac + 8),
    })
}
