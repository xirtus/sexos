// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! NVMe pieces for one I/O queue pair per core (NVMe Base Specification 2.0:
//! Set Features "Number of Queues" (FID 07h), Create I/O Completion Queue (05h),
//! Create I/O Submission Queue (01h), Delete I/O SQ/CQ (00h/04h), doorbell stride).
//!
//! Queue ids start at 1 (0 is the admin queue). Each I/O completion queue gets its
//! own interrupt vector, so the core that owns the pair takes its completions.

pub const OP_DELETE_IO_SQ: u8 = 0x00;
pub const OP_CREATE_IO_SQ: u8 = 0x01;
pub const OP_DELETE_IO_CQ: u8 = 0x04;
pub const OP_CREATE_IO_CQ: u8 = 0x05;
pub const OP_SET_FEATURES: u8 = 0x09;
pub const OP_GET_FEATURES: u8 = 0x0A;
pub const FEATURE_NUMBER_OF_QUEUES: u32 = 0x07;
/// Most I/O queues of each kind a controller may grant (zero-based field of 16 bits).
pub const MAX_IO_QUEUES: u32 = 65535;

/// Command dword 10/11 and the data pointer of an admin command; the driver adds the
/// opcode, command id and PRP2 (unused here: queues are physically contiguous).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admin {
    pub opcode: u8,
    pub prp1: u64,
    pub cdw10: u32,
    pub cdw11: u32,
}

impl Admin {
    /// The 16 dwords of the submission entry with command id `cid`.
    pub fn entry(&self, cid: u16) -> [u32; 16] {
        let mut e = [0u32; 16];
        e[0] = self.opcode as u32 | (cid as u32) << 16;
        e[6] = self.prp1 as u32;
        e[7] = (self.prp1 >> 32) as u32;
        e[10] = self.cdw10;
        e[11] = self.cdw11;
        e
    }
}

/// Set Features: ask for `want` I/O submission and completion queues.
pub fn request_queues(want: u32) -> Admin {
    let n = want.clamp(1, MAX_IO_QUEUES) - 1;
    Admin { opcode: OP_SET_FEATURES, prp1: 0, cdw10: FEATURE_NUMBER_OF_QUEUES, cdw11: n << 16 | n }
}

/// Completion dword 0 of "Number of Queues": the (submission, completion) queues the
/// controller allocated. A controller may grant more than asked; use the smaller.
pub fn granted_queues(dw0: u32, asked: u32) -> u32 {
    let sq = (dw0 & 0xffff) + 1;
    let cq = (dw0 >> 16) + 1;
    sq.min(cq).min(asked.max(1))
}

/// Create I/O Completion Queue `qid` of `entries` slots at physical `base`, raising
/// interrupt vector `vector` (or none: a polled queue).
pub fn create_cq(qid: u16, entries: u16, base: u64, vector: Option<u16>) -> Admin {
    let (ien, iv) = match vector { Some(v) => (1u32, v as u32), None => (0, 0) };
    Admin { opcode: OP_CREATE_IO_CQ, prp1: base,
            cdw10: ((entries.max(2) - 1) as u32) << 16 | qid as u32,
            cdw11: iv << 16 | ien << 1 | 1 /* physically contiguous */ }
}

/// Create I/O Submission Queue `qid` of `entries` slots at `base`, completing on `cqid`.
pub fn create_sq(qid: u16, entries: u16, base: u64, cqid: u16) -> Admin {
    Admin { opcode: OP_CREATE_IO_SQ, prp1: base,
            cdw10: ((entries.max(2) - 1) as u32) << 16 | qid as u32,
            cdw11: (cqid as u32) << 16 | 1 /* physically contiguous, priority urgent=0 */ }
}

/// Delete I/O SQ (delete before its CQ) or CQ `qid`.
pub fn delete_sq(qid: u16) -> Admin { Admin { opcode: OP_DELETE_IO_SQ, prp1: 0, cdw10: qid as u32, cdw11: 0 } }
pub fn delete_cq(qid: u16) -> Admin { Admin { opcode: OP_DELETE_IO_CQ, prp1: 0, cdw10: qid as u32, cdw11: 0 } }

/// Doorbell stride from CAP.DSTRD (bits 35:32): bytes between doorbells.
pub fn stride(cap: u64) -> u64 { 4u64 << ((cap >> 32) & 0xf) }
/// Offset in BAR0 of submission queue `qid`'s tail doorbell.
pub fn sq_tail_doorbell(qid: u16, cap: u64) -> u64 { 0x1000 + 2 * qid as u64 * stride(cap) }
/// Offset in BAR0 of completion queue `qid`'s head doorbell.
pub fn cq_head_doorbell(qid: u16, cap: u64) -> u64 { 0x1000 + (2 * qid as u64 + 1) * stride(cap) }
/// CAP.MQES + 1: the most slots one queue may have.
pub fn max_entries(cap: u64) -> u32 { (cap & 0xffff) as u32 + 1 }
/// Whether CAP.CQR says queues must be physically contiguous (we always make them so).
pub fn contiguous_required(cap: u64) -> bool { cap >> 16 & 1 != 0 }
