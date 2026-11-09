// SPDX-License-Identifier: GPL-3.0-only
//! AMD "I/O Virtualization Reporting Structure" ACPI table (AMD IOMMU spec 5.2).
//!
//! Only the hardware unit descriptions (IVHD) are read today: enough to report
//! an AMD-Vi machine and plan its domains. Device-table programming follows
//! the VT-d path and is tracked as its own stage.
use crate::{u16_at, u32_at, u64_at};

pub const SIGNATURE: &[u8; 4] = b"IVRS";
const FIRST_BLOCK: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Short, Signature, Length, Block }

#[derive(Clone, Copy, Debug)]
pub struct Ivrs<'a> { bytes: &'a [u8], pub info: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    /// 0x10, 0x11 or 0x40: newer types carry more feature words.
    pub kind: u8,
    pub flags: u8,
    /// The IOMMU's own PCI requester id.
    pub device_id: u16,
    pub capability_offset: u16,
    pub register_base: u64,
    pub segment: u16,
}

impl<'a> Ivrs<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < FIRST_BLOCK { return Err(Error::Short); }
        if &bytes[0..4] != SIGNATURE { return Err(Error::Signature); }
        let length = u32_at(bytes, 4).ok_or(Error::Short)? as usize;
        if length < FIRST_BLOCK || length > bytes.len() { return Err(Error::Length); }
        let bytes = &bytes[..length];
        let mut at = FIRST_BLOCK;
        while at < bytes.len() {
            let len = u16_at(bytes, at + 2).ok_or(Error::Block)? as usize;
            if len < 4 || at + len > bytes.len() { return Err(Error::Block); }
            at += len;
        }
        Ok(Self { bytes, info: u32_at(bytes, 36).ok_or(Error::Short)? })
    }

    /// Highest virtual address bits the units translate (IVinfo VASize).
    pub fn virtual_address_bits(&self) -> u8 { ((self.info >> 15) & 0x7f) as u8 }

    pub fn units(&self) -> impl Iterator<Item = Unit> + 'a {
        let bytes = self.bytes;
        let mut at = FIRST_BLOCK;
        core::iter::from_fn(move || loop {
            let kind = *bytes.get(at)?;
            let len = u16_at(bytes, at + 2)? as usize;
            let b = bytes.get(at..at + len)?;
            at += len;
            if matches!(kind, 0x10 | 0x11 | 0x40) && len >= 24 {
                return Some(Unit { kind, flags: b[1], device_id: u16_at(b, 4)?,
                    capability_offset: u16_at(b, 6)?, register_base: u64_at(b, 8)?, segment: u16_at(b, 16)? });
            }
        })
    }
}
