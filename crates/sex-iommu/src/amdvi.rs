// SPDX-License-Identifier: GPL-3.0-only
//! AMD-Vi (AMD I/O Virtualization, spec 48882) register fields, device table
//! entries, commands and event log entries.
//!
//! One device table, indexed by requester id, says for every device which
//! page table (and domain id) its DMA goes through. The kernel talks to the
//! IOMMU through a ring of 16-byte commands (invalidations, completion waits)
//! and reads blocked DMA from a ring of 16-byte events.

pub const DEVICE_TABLE: usize = 0x00;
pub const COMMAND_BUFFER: usize = 0x08;
pub const EVENT_LOG: usize = 0x10;
pub const CONTROL: usize = 0x18;
pub const EXCLUSION_BASE: usize = 0x20;
pub const EXCLUSION_LIMIT: usize = 0x28;
pub const EXTENDED_FEATURES: usize = 0x30;
pub const COMMAND_HEAD: usize = 0x2000;
pub const COMMAND_TAIL: usize = 0x2008;
pub const EVENT_HEAD: usize = 0x2010;
pub const EVENT_TAIL: usize = 0x2018;
pub const STATUS: usize = 0x2020;

pub const CONTROL_IOMMU: u64 = 1 << 0;
pub const CONTROL_EVENT_LOG: u64 = 1 << 2;
pub const CONTROL_COHERENT: u64 = 1 << 10;
pub const CONTROL_COMMAND_BUFFER: u64 = 1 << 12;
/// Invalidation timeout field (bits 7:5): 1 = 1 ms.
pub const CONTROL_TIMEOUT_1MS: u64 = 1 << 5;

pub const STATUS_EVENT_OVERFLOW: u64 = 1 << 0;
pub const STATUS_EVENT_LOG_RUN: u64 = 1 << 3;
pub const STATUS_COMMAND_RUN: u64 = 1 << 4;

/// Extended feature: INVALIDATE_IOMMU_ALL is supported.
pub const FEATURE_INVALIDATE_ALL: u64 = 1 << 6;

/// 256 entries of 16 bytes: one 4 KiB page for the command ring and the event log.
pub const RING_BYTES: u64 = 4096;
pub const RING_ENTRIES: u64 = RING_BYTES / 16;
/// Every requester id of one PCI segment, 32 bytes each: 2 MiB.
pub const DEVICE_TABLE_BYTES: u64 = 65536 * 32;

/// Device table base register: address and size in 4 KiB pages minus one.
pub const fn device_table(base: u64, bytes: u64) -> u64 { (base & !0xfff) | ((bytes / 4096 - 1) & 0x1ff) }

/// Command buffer or event log base register: address and log2(entries).
pub const fn ring(base: u64) -> u64 { (base & !0xfff) | (8u64 << 56) }

const ADDRESS: u64 = 0x000f_ffff_ffff_f000;
pub const DTE_VALID: u64 = 1 << 0;
pub const DTE_TRANSLATION_VALID: u64 = 1 << 1;
pub const DTE_READ: u64 = 1 << 61;
pub const DTE_WRITE: u64 = 1 << 62;

/// Device table entry (four words). With a page table the device's DMA is
/// translated through it under `domain`; without one every DMA is refused
/// and logged. Interrupts stay unremapped (IV = 0), so MSI/MSI-X keep working.
pub const fn dte(page_table: Option<u64>, levels: u8, domain: u16) -> [u64; 4] {
    let word0 = match page_table {
        Some(root) => (root & ADDRESS) | ((levels as u64 & 7) << 9) | DTE_VALID | DTE_TRANSLATION_VALID | DTE_READ | DTE_WRITE,
        // Mode 0 without IR/IW: no translation and no access.
        None => DTE_VALID | DTE_TRANSLATION_VALID,
    };
    [word0, domain as u64, 0, 0]
}

pub const fn dte_domain(words: &[u64; 4]) -> u16 { words[1] as u16 }

const fn opcode(op: u64) -> u64 { op << 60 }

/// COMPLETION_WAIT: when every earlier command is done, store `data` at `store`.
pub const fn completion_wait(store: u64, data: u64) -> [u64; 2] {
    [(store & 0x000f_ffff_ffff_fff8) | 1 | opcode(1), data]
}

/// INVALIDATE_DEVTAB_ENTRY: forget the cached device table entry of `device`.
pub const fn invalidate_device(device: u16) -> [u64; 2] { [device as u64 | opcode(2), 0] }

/// INVALIDATE_IOMMU_PAGES for every address of `domain`, page tables included.
pub const fn invalidate_domain(domain: u16) -> [u64; 2] {
    [((domain as u64) << 32) | opcode(3), 0x7fff_ffff_ffff_f000 | 0b11]
}

/// INVALIDATE_IOMMU_ALL (only when `FEATURE_INVALIDATE_ALL`).
pub const fn invalidate_all() -> [u64; 2] { [opcode(8), 0] }

// Event codes, reported as Fault::reason 0x80 + code.
pub const EVENT_ILLEGAL_DTE: u8 = 0x1;
pub const EVENT_PAGE_FAULT: u8 = 0x2;
pub const REASON_ILLEGAL_DTE: u8 = 0x81;
/// IO_PAGE_FAULT on a page that is not present.
pub const REASON_NOT_PRESENT: u8 = 0x82;
/// IO_PAGE_FAULT on a present page the access is not allowed on (or a blocked device).
pub const REASON_PERMISSION: u8 = 0x8a;
pub const REASON_HARDWARE: u8 = 0x83;
pub const REASON_COMMAND: u8 = 0x85;
pub const REASON_INVALIDATION_TIMEOUT: u8 = 0x87;
pub const REASON_INVALID_REQUEST: u8 = 0x88;

const FLAG_PRESENT: u64 = 0x010;
const FLAG_WRITE: u64 = 0x020;

/// One event log entry (two words) as a fault; None for an empty slot.
pub fn decode_event(low: u64, high: u64) -> Option<crate::Fault> {
    let code = (low >> 60) as u8;
    if code == 0 { return None; }
    let flags = (low >> 48) & 0xfff;
    let reason = match code {
        EVENT_PAGE_FAULT if flags & FLAG_PRESENT != 0 => REASON_PERMISSION,
        EVENT_PAGE_FAULT => REASON_NOT_PRESENT,
        3 | 4 | 6 => REASON_HARDWARE,
        c => 0x80 | c,
    };
    // Hardware and command errors carry no requester id.
    let source = if matches!(code, 1 | 2 | 7 | 8) { crate::Bdf::from_sid(low as u16) } else { crate::Bdf::new(0, 0, 0) };
    Some(crate::Fault { source, address: high & !0xfff, reason, write: flags & FLAG_WRITE != 0 })
}
