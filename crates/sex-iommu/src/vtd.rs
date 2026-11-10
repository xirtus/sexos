// SPDX-License-Identifier: GPL-3.0-only
//! Intel VT-d register fields and table entry encodings (legacy, not scalable mode).

pub const VER: usize = 0x00;
pub const CAP: usize = 0x08;
pub const ECAP: usize = 0x10;
pub const GCMD: usize = 0x18;
pub const GSTS: usize = 0x1c;
pub const RTADDR: usize = 0x20;
pub const CCMD: usize = 0x28;
pub const FSTS: usize = 0x34;
pub const FECTL: usize = 0x38;
pub const PMEN: usize = 0x64;

// Global command / status bits (same positions in both registers).
pub const TE: u32 = 1 << 31;
pub const SRTP: u32 = 1 << 30;
pub const WBF: u32 = 1 << 27;
pub const QIE: u32 = 1 << 26;
pub const IRE: u32 = 1 << 25;
/// Status bits that persist and must be echoed into GCMD with each command;
/// the others are one-shot and must be written as zero.
pub const GSTS_PERSISTENT: u32 = 0x96ff_ffff;

/// Next GCMD value that keeps the current state and adds or drops `bit`.
pub const fn command(status: u32, bit: u32, on: bool) -> u32 {
    let keep = status & GSTS_PERSISTENT;
    if on { keep | bit } else { keep & !bit }
}

// Context-command register.
pub const CCMD_ICC: u64 = 1 << 63;
pub const CCMD_GLOBAL: u64 = 1 << 61;
/// Domain-selective context invalidation.
pub const fn ccmd_domain(did: u16) -> u64 { CCMD_ICC | (2 << 61) | did as u64 }

// IOTLB invalidate register (at ECAP.IRO * 16 + 8).
pub const IOTLB_IVT: u64 = 1 << 63;
pub const IOTLB_GLOBAL: u64 = 1 << 60;
pub const IOTLB_DRAIN: u64 = (1 << 49) | (1 << 48);
pub const fn iotlb_domain(did: u16) -> u64 { IOTLB_IVT | (2 << 60) | IOTLB_DRAIN | ((did as u64) << 32) }
pub const fn iotlb_page(did: u16) -> u64 { IOTLB_IVT | (3 << 60) | IOTLB_DRAIN | ((did as u64) << 32) }
/// IVA register value for a page-selective flush of 2^order pages at `iova`.
pub const fn iva(iova: u64, order: u8) -> u64 { (iova & !0xfff) | (order as u64 & 0x3f) }

// Fault status.
pub const FSTS_PFO: u32 = 1 << 0;
pub const FSTS_PPF: u32 = 1 << 1;
pub const FECTL_IM: u32 = 1 << 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cap(pub u64);

impl Cap {
    /// Domain ids the unit supports.
    pub fn domains(self) -> u32 { 1 << (4 + 2 * (self.0 & 7) as u32) }
    pub fn required_write_buffer_flush(self) -> bool { self.0 & (1 << 4) != 0 }
    /// Caching mode: not-present entries may be cached too (virtual IOMMUs).
    pub fn caching_mode(self) -> bool { self.0 & (1 << 7) != 0 }
    /// SAGAW: bit 1 = 39-bit (3-level), bit 2 = 48-bit (4-level), bit 3 = 57-bit.
    pub fn sagaw(self) -> u8 { ((self.0 >> 8) & 0x1f) as u8 }
    pub fn mgaw(self) -> u8 { ((self.0 >> 16) & 0x3f) as u8 + 1 }
    pub fn fault_offset(self) -> usize { (((self.0 >> 24) & 0x3ff) * 16) as usize }
    pub fn page_selective(self) -> bool { self.0 & (1 << 39) != 0 }
    pub fn fault_records(self) -> usize { ((self.0 >> 40) & 0xff) as usize + 1 }
    pub fn max_flush_order(self) -> u8 { ((self.0 >> 48) & 0x3f) as u8 }

    /// Page-table levels and context AW value to use: 4 levels when offered.
    pub fn levels(self) -> Option<(u8, u64)> {
        if self.sagaw() & 4 != 0 { Some((4, 2)) }
        else if self.sagaw() & 2 != 0 { Some((3, 1)) }
        else { None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ecap(pub u64);

impl Ecap {
    /// Page walks snoop CPU caches; otherwise each table write needs clflush.
    pub fn coherent(self) -> bool { self.0 & 1 != 0 }
    pub fn queued_invalidation(self) -> bool { self.0 & (1 << 1) != 0 }
    pub fn interrupt_remapping(self) -> bool { self.0 & (1 << 3) != 0 }
    pub fn pass_through(self) -> bool { self.0 & (1 << 6) != 0 }
    pub fn iotlb_offset(self) -> usize { (((self.0 >> 8) & 0x3ff) * 16) as usize }
    pub fn scalable_mode(self) -> bool { self.0 & (1 << 43) != 0 }
}

/// Root entry low word: present, pointing at a bus's context table.
pub const fn root_entry(context_table: u64) -> u64 { (context_table & !0xfff) | 1 }

/// Context entry (low, high) for second-level translation of one device.
pub const fn context_entry(page_table: u64, aw: u64, did: u16) -> (u64, u64) {
    ((page_table & !0xfff) | 1, (aw & 7) | ((did as u64) << 8))
}

pub const fn context_did(high: u64) -> u16 { (high >> 8) as u16 }

pub use crate::Fault;

pub const FAULT_F: u64 = 1 << 63;

/// One VT-d fault record (128 bits) decoded.
pub fn decode_fault(low: u64, high: u64) -> Option<Fault> {
    if high & FAULT_F == 0 { return None; }
    Some(Fault {
        source: crate::Bdf::from_sid(high as u16),
        address: low & !0xfff,
        reason: (high >> 32) as u8,
        // T1 = 0 means a write request.
        write: high & (1 << 62) == 0,
    })
}
