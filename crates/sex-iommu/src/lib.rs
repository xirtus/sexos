// SPDX-License-Identifier: GPL-3.0-only
//! IOMMU data: firmware tables (Intel DMAR, AMD IVRS), VT-d and AMD-Vi fields,
//! I/O page tables and the device-address window drivers borrow from.
//!
//! No hardware access lives here. The kernel supplies frames and register
//! reads; everything in this crate is plain arithmetic, so the host tests in
//! `tests/` check it without a machine.
#![no_std]

pub mod amdvi;
pub mod dmar;
pub mod ivrs;
pub mod vtd;
pub mod table;
pub mod window;

/// One 4 KiB page, the unit every IOMMU here maps.
pub const PAGE: u64 = 4096;

/// PCI requester id: what the IOMMU sees as the source of a DMA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bdf { pub bus: u8, pub dev: u8, pub func: u8 }

impl Bdf {
    pub const fn new(bus: u8, dev: u8, func: u8) -> Self { Self { bus, dev, func } }
    /// Device and function packed as the low byte of a requester id.
    pub const fn devfn(self) -> u8 { (self.dev << 3) | (self.func & 7) }
    pub const fn sid(self) -> u16 { ((self.bus as u16) << 8) | self.devfn() as u16 }
    pub const fn from_sid(sid: u16) -> Self {
        Self { bus: (sid >> 8) as u8, dev: ((sid >> 3) & 31) as u8, func: (sid & 7) as u8 }
    }
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
pub(crate) fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
pub(crate) fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// ACPI table checksum: all bytes of the table sum to zero.
pub fn checksum_ok(table: &[u8]) -> bool {
    table.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) == 0
}

/// A blocked DMA, decoded from a VT-d fault record or an AMD-Vi event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub source: Bdf,
    /// Page address the device tried to reach.
    pub address: u64,
    /// VT-d fault reason (0x01..), or 0x80 + an AMD-Vi event code.
    pub reason: u8,
    pub write: bool,
}

impl Fault {
    pub fn reason_text(&self) -> &'static str {
        match self.reason {
            0x01 => "root_entry_not_present",
            0x02 => "context_entry_not_present",
            0x03 => "context_entry_invalid",
            0x04 => "address_beyond_width",
            0x05 => "write_not_permitted",
            0x06 => "read_not_permitted",
            0x07 => "table_address_invalid",
            0x08 => "root_table_address_invalid",
            0x09 => "context_table_address_invalid",
            0x0a | 0x0b => "reserved_bits_set",
            amdvi::REASON_ILLEGAL_DTE => "amd_illegal_device_table_entry",
            amdvi::REASON_NOT_PRESENT => "amd_page_not_present",
            amdvi::REASON_PERMISSION => "amd_access_not_permitted",
            amdvi::REASON_HARDWARE => "amd_table_hardware_error",
            amdvi::REASON_COMMAND => "amd_command_error",
            amdvi::REASON_INVALIDATION_TIMEOUT => "amd_iotlb_invalidation_timeout",
            amdvi::REASON_INVALID_REQUEST => "amd_invalid_device_request",
            _ => "other",
        }
    }
}
